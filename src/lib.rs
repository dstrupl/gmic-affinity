// The crate's library name is `GmicFilter` (PascalCase) because the
// Photoshop plugin format ties the executable name to `CFBundleExecutable`
// in `Info.plist`, and convention there is the BundleCase name.
#![allow(non_snake_case)]

//! `GmicFilter.plugin` entry point.
//!
//! Affinity (and any Photoshop-compatible host) loads our `.plugin` bundle
//! and looks up the exported `PluginMain` symbol. It then calls us with one
//! of the selectors defined in `ps_types.rs`.
//!
//! Two build modes:
//!
//! - Default build (`cargo build`): a no-op `PluginMain` that returns
//!   `NO_ERR` for every selector and never dereferences `FilterRecord`.
//!   Safe to install in Affinity even before the SDK-verified struct layout
//!   has been reconciled.
//! - `--features live`: the real M3+ `PluginMain` that reads pixels and
//!   (M4+) shells out to gmic. Only install this once `cargo test --
//!   --ignored` confirms `FilterRecord` offsets match `PIFilter.h`.

pub mod catalogue;
pub mod filter;
pub mod gmic;
pub mod logging;
pub mod ps_data;
pub mod ps_types;
pub mod settings;
pub mod tiff_io;

pub mod ui;

use logging::log;
use ps_types::{
    NO_ERR, SELECTOR_ABOUT, SELECTOR_CONTINUE, SELECTOR_FINISH, SELECTOR_PARAMETERS,
    SELECTOR_PREPARE, SELECTOR_START,
};
use std::ffi::c_void;

#[cfg(feature = "live")]
use ps_types::{FilterRecord, VRect, USER_CANCEL};

/// Photoshop-compatible filter entry point.
///
/// # Safety
/// All four pointers are supplied by the host. The default build does not
/// dereference `filter_record`; the `live` build does, and assumes the
/// caller (the host) has provided a valid pointer to a `FilterRecord`.
#[no_mangle]
pub unsafe extern "C" fn PluginMain(
    selector: i16,
    filter_record: *mut c_void,
    data: *mut isize,
    result: *mut i16,
) {
    log_selector(selector);

    let code: i16 = dispatch(selector, filter_record, data);

    if !result.is_null() {
        *result = code;
    }
}

#[cfg(not(feature = "live"))]
unsafe fn dispatch(_selector: i16, _filter_record: *mut c_void, _data: *mut isize) -> i16 {
    NO_ERR
}

#[cfg(feature = "live")]
unsafe fn dispatch(selector: i16, filter_record: *mut c_void, data: *mut isize) -> i16 {
    if filter_record.is_null() {
        log("null FilterRecord pointer; refusing");
        return USER_CANCEL;
    }
    let fr = &mut *(filter_record as *mut FilterRecord);

    match selector {
        SELECTOR_ABOUT => NO_ERR,
        SELECTOR_PARAMETERS => parameters_selector(data),
        SELECTOR_PREPARE => {
            fr.buffer_space = 0;
            fr.max_space = 0;
            NO_ERR
        }
        SELECTOR_START => {
            // Tell the host we want the whole filter rect, all planes,
            // then call back into the host's `advanceState` so it
            // populates `in_data` / `in_row_bytes` and allocates
            // `out_data` matching the rects. Without this call Affinity
            // leaves both at zero and CONTINUE has no pixels to work
            // with (this is exactly what bit us on first install).
            fr.in_rect = fr.filter_rect;
            fr.in_lo_plane = 0;
            fr.in_hi_plane = fr.planes - 1;
            fr.out_rect = fr.filter_rect;
            fr.out_lo_plane = 0;
            fr.out_hi_plane = fr.planes - 1;
            log(&format!(
                "START rect=({},{})-({},{}) planes={}",
                fr.filter_rect.left,
                fr.filter_rect.top,
                fr.filter_rect.right,
                fr.filter_rect.bottom,
                fr.planes,
            ));
            match fr.advance_state {
                Some(advance) => {
                    let rc = advance();
                    log(&format!(
                        "advance_state -> rc={} in_data={:p} in_row_bytes={} out_data={:p} out_row_bytes={}",
                        rc, fr.in_data, fr.in_row_bytes, fr.out_data, fr.out_row_bytes
                    ));
                    if rc != NO_ERR {
                        return rc;
                    }
                    NO_ERR
                }
                None => {
                    log("advance_state pointer is NULL; host did not provide it");
                    NO_ERR
                }
            }
        }
        SELECTOR_CONTINUE => continue_selector(fr, data),
        SELECTOR_FINISH => {
            // Drop the Boxed ChosenFilter from *data (T13). If
            // PARAMETERS never ran (Last-Filter path: *data is null),
            // `take_and_drop` is a no-op.
            crate::ps_data::take_and_drop::<crate::catalogue::ChosenFilter>(data);
            log("FINISH: *data reclaimed");
            NO_ERR
        }
        _ => NO_ERR,
    }
}

/// Handle `SELECTOR_PARAMETERS`: open the picker, persist the user's
/// pick to `settings.json`, and stash the [`ChosenFilter`] on the
/// plugin-private `*data` slot so `SELECTOR_CONTINUE` can recover it.
///
/// Panics inside the AppKit code path are isolated via
/// `catch_unwind` (T14) so a UI crash translates to "user cancel"
/// plus an alert, never a host-killing unwind across the FFI
/// boundary.
#[cfg(feature = "live")]
unsafe fn parameters_selector(data: *mut isize) -> i16 {
    use crate::catalogue::{self, ChosenFilter};
    use std::panic::{catch_unwind, AssertUnwindSafe};

    log("PARAMETERS: opening picker");

    // Catalogue lookup can panic only if the bundled snapshot is
    // corrupt — extremely unlikely in shipped builds because LFS
    // hydration is required to even produce the bundle.
    let cat: &'static catalogue::Catalogue = match catch_unwind(catalogue::builtin) {
        Ok(c) => c,
        Err(_) => {
            log("PARAMETERS: catalogue::builtin panicked");
            crate::ui::alert::alert_error(
                &crate::ui::alert::NsAlertSink,
                "G'MIC",
                "G'MIC filter list is unreadable — this build of the plugin may be corrupted.",
                true,
            );
            return USER_CANCEL;
        }
    };

    let mut settings = crate::settings::Settings::load();
    let chosen = match catch_unwind(AssertUnwindSafe(|| {
        crate::ui::picker::show_picker(cat, settings.last.as_ref())
    })) {
        Ok(opt) => opt,
        Err(_) => {
            log("PARAMETERS: picker panicked");
            crate::ui::alert::alert_error(
                &crate::ui::alert::NsAlertSink,
                "G'MIC",
                "Couldn't open the G'MIC dialog.",
                true,
            );
            return USER_CANCEL;
        }
    };

    match chosen {
        Some(chosen) => {
            let ts = crate::logging::iso8601_now();
            let display_path = catalogue::lookup_display_path(cat, &chosen.command)
                .unwrap_or_else(|| chosen.command.clone());
            settings.record_pick(&chosen.command, chosen.args.clone(), &display_path, &ts);
            settings.save();
            crate::ps_data::leak::<ChosenFilter>(chosen, data);
            log("PARAMETERS: ChosenFilter leaked into *data, settings saved");
            NO_ERR
        }
        None => {
            log("PARAMETERS: user cancelled");
            USER_CANCEL
        }
    }
}

/// Handle `SELECTOR_CONTINUE`: recover the [`ChosenFilter`] stashed
/// by PARAMETERS (or fall back to `settings.last` for the Last-Filter
/// menu item) and run the gmic pipeline against it.
#[cfg(feature = "live")]
unsafe fn continue_selector(fr: &mut FilterRecord, data: *mut isize) -> i16 {
    use crate::catalogue::ChosenFilter;

    let chosen_owned: Option<ChosenFilter> = crate::ps_data::borrow::<ChosenFilter>(data).cloned();

    let chosen = match chosen_owned {
        Some(c) => c,
        None => {
            let settings = crate::settings::Settings::load();
            match settings.last {
                Some(last) => ChosenFilter {
                    command: last.command,
                    args: last.args,
                },
                None => {
                    crate::ui::alert::alert_error(
                        &crate::ui::alert::NsAlertSink,
                        "G'MIC",
                        "No previous G'MIC filter to repeat. Pick one from Filters > Plugins > G'MIC > G'MIC….",
                        false,
                    );
                    log("CONTINUE: no *data, no settings.last — USER_CANCEL");
                    return USER_CANCEL;
                }
            }
        }
    };

    log(&format!(
        "CONTINUE: invoking gmic::run_filter_with (cmd={} argc={}) in_data={:p} out_data={:p}",
        chosen.command,
        chosen.args.len(),
        fr.in_data,
        fr.out_data,
    ));
    match gmic::run_filter_with(fr, &chosen) {
        Ok(()) => {
            log("CONTINUE: gmic::run_filter_with returned OK");
            fr.in_rect = VRect {
                top: 0,
                left: 0,
                bottom: 0,
                right: 0,
            };
            fr.out_rect = VRect {
                top: 0,
                left: 0,
                bottom: 0,
                right: 0,
            };
            NO_ERR
        }
        Err(e) => {
            // T14 NSAlert matrix — translate the GmicError variant
            // into a user-actionable message. Everything also goes to
            // the log for support diagnosis.
            let sink = &crate::ui::alert::NsAlertSink;
            match &e {
                gmic::GmicError::NotFound => {
                    crate::ui::alert::alert_error(
                        sink,
                        "G'MIC",
                        "G'MIC isn't installed. Install it with: brew install gmic and try again.",
                        false,
                    );
                }
                gmic::GmicError::Failed { status } => {
                    let status_str = status
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "signal".into());
                    crate::ui::alert::alert_error(
                        sink,
                        "G'MIC",
                        &format!(
                            "G'MIC reported an error running `{}` (exit {}).",
                            chosen.command, status_str,
                        ),
                        true,
                    );
                }
                gmic::GmicError::Tiff(_) => {
                    crate::ui::alert::alert_error(
                        sink,
                        "G'MIC",
                        "G'MIC produced an image we couldn't read back.",
                        true,
                    );
                }
                _ => {
                    crate::ui::alert::alert_error(
                        sink,
                        "G'MIC",
                        &format!("G'MIC pipeline failed: {e}"),
                        true,
                    );
                }
            }
            log(&format!("CONTINUE: gmic failed: {e}"));
            USER_CANCEL
        }
    }
}

fn log_selector(selector: i16) {
    // Side-channel log (Affinity drops stderr, so eprintln! is invisible).
    // See src/logging.rs for the rationale and file location.
    let name = match selector {
        SELECTOR_ABOUT => "About",
        SELECTOR_PARAMETERS => "Parameters",
        SELECTOR_PREPARE => "Prepare",
        SELECTOR_START => "Start",
        SELECTOR_CONTINUE => "Continue",
        SELECTOR_FINISH => "Finish",
        _ => "Unknown",
    };
    log(&format!("PluginMain selector={} ({})", selector, name));
}
