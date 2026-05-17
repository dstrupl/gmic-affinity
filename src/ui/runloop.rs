//! Wrapper around `NSApp.runModal(for:)` so the rest of the UI never
//! has to touch `objc2` directly for the run-loop dance.
//!
//! The first milestone of the picker design is "open an empty `NSPanel`
//! from `SELECTOR_PARAMETERS`, close it with the title bar's close button,
//! return cleanly to Affinity". This file is the surface that proves that works. If
//! `runModal` misbehaves inside Affinity's run loop, this is where
//! we will find out and decide on the sheet-based fallback.

use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSModalResponse, NSWindow};
use objc2_foundation::MainThreadMarker;

/// Show `window` modally and block until it is dismissed. Returns the
/// AppKit modal response code (`.OK` / `.cancel` / a custom int).
///
/// MUST be called on the main thread. `PluginMain` is always invoked
/// on the main thread per the Photoshop SDK contract.
pub fn run_modal_window(window: &Retained<NSWindow>) -> NSModalResponse {
    let mtm = MainThreadMarker::new()
        .expect("ui::runloop::run_modal_window must be called on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    window.makeKeyAndOrderFront(None);
    let response = unsafe { app.runModalForWindow(window) };
    window.orderOut(None);
    response
}
