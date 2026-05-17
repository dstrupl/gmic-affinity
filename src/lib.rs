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
unsafe fn dispatch(selector: i16, filter_record: *mut c_void, _data: *mut isize) -> i16 {
    if filter_record.is_null() {
        log("null FilterRecord pointer; refusing");
        return USER_CANCEL;
    }
    let fr = &mut *(filter_record as *mut FilterRecord);

    match selector {
        SELECTOR_ABOUT => NO_ERR,
        SELECTOR_PARAMETERS => {
            log("PARAMETERS: opening picker");
            match ui::picker::show_empty() {
                Some(()) => NO_ERR,
                None => USER_CANCEL,
            }
        }
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
                    if rc != NO_ERR { return rc; }
                    NO_ERR
                }
                None => {
                    log("advance_state pointer is NULL; host did not provide it");
                    NO_ERR
                }
            }
        }
        SELECTOR_CONTINUE => {
            // M4: real gmic round-trip. The M3 pass-through is still
            // available via `filter::run_passthrough` for debugging the
            // pointer path in isolation; we don't expose a runtime switch
            // in v1 because Affinity has no parameter UI yet.
            log(&format!(
                "CONTINUE: invoking gmic::run_filter (in_data={:p} in_row_bytes={} out_data={:p} out_row_bytes={})",
                fr.in_data, fr.in_row_bytes, fr.out_data, fr.out_row_bytes
            ));
            match gmic::run_filter(fr) {
                Ok(()) => {
                    log("CONTINUE: gmic::run_filter returned OK");
                    // Signal "done, no more tiles" by zeroing the rects.
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
                    log(&format!("CONTINUE: filter failed: {e}"));
                    USER_CANCEL
                }
            }
        }
        SELECTOR_FINISH => NO_ERR,
        _ => NO_ERR,
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
