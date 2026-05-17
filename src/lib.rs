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
//! M1 (current): a no-op `PluginMain` that returns `NO_ERR` for every
//! selector and never dereferences the `FilterRecord`. This is intentional:
//! it lets us verify the bundle loads, the symbol resolves, and the plugin
//! appears in Affinity's Filters menu without any risk of crashing the host
//! due to a layout mismatch in `FilterRecord` (see `ps_types.rs`).

pub mod ps_types;

use ps_types::{
    NO_ERR, SELECTOR_ABOUT, SELECTOR_CONTINUE, SELECTOR_FINISH, SELECTOR_PARAMETERS,
    SELECTOR_PREPARE, SELECTOR_START,
};
use std::ffi::c_void;

/// Photoshop-compatible filter entry point.
///
/// # Safety
/// All four pointers are supplied by the host. We do not dereference
/// `filter_record` or `data` in M1; we only write a result code through
/// `result` if it is non-null.
#[no_mangle]
pub unsafe extern "C" fn PluginMain(
    selector:      i16,
    _filter_record: *mut c_void,
    _data:          *mut isize,
    result:         *mut i16,
) {
    log_selector(selector);

    let code: i16 = match selector {
        SELECTOR_ABOUT
        | SELECTOR_PARAMETERS
        | SELECTOR_PREPARE
        | SELECTOR_START
        | SELECTOR_CONTINUE
        | SELECTOR_FINISH => NO_ERR,
        _ => NO_ERR,
    };

    if !result.is_null() {
        *result = code;
    }
}

fn log_selector(selector: i16) {
    // M1 instrumentation: anything we log to stderr lands in Console.app when
    // Affinity loads the plugin. Replace with `os_log` later if we want
    // structured logging.
    let name = match selector {
        SELECTOR_ABOUT      => "About",
        SELECTOR_PARAMETERS => "Parameters",
        SELECTOR_PREPARE    => "Prepare",
        SELECTOR_START      => "Start",
        SELECTOR_CONTINUE   => "Continue",
        SELECTOR_FINISH     => "Finish",
        _                    => "Unknown",
    };
    eprintln!("[gmic-affinity] PluginMain selector={} ({})", selector, name);
}
