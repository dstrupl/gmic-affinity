//! Wrapper around `NSApp.runModal(for:)` so the rest of the UI never
//! has to touch `objc2` directly for the run-loop dance.
//!
//! The first milestone of the picker design is "open an empty `NSPanel`
//! from `SELECTOR_PARAMETERS`, close it with the title bar's close button,
//! return cleanly to Affinity". This file is the surface that proves that works. If
//! `runModal` misbehaves inside Affinity's run loop, this is where
//! we will find out and decide on the sheet-based fallback.

use objc2::rc::{autoreleasepool, Retained};
use objc2_app_kit::{
    NSApplication, NSEventMask, NSEventTrackingRunLoopMode, NSModalResponse,
    NSModalResponseContinue, NSWindow,
};
use objc2_foundation::{MainThreadMarker, NSDate};

/// Show `window` modally and block until it is dismissed. Returns the
/// AppKit modal response code (`.OK` / `.cancel` / a custom int).
///
/// MUST be called on the main thread. `PluginMain` is always invoked
/// on the main thread per the Photoshop SDK contract.
///
/// Implementation note: uses `beginModalSessionForWindow:` +
/// `runModalSession:` in a manual loop rather than the simpler
/// `runModalForWindow:`. The reason is that inside Affinity Photo
/// 2's busy modal run loop, plain `runModalForWindow:` services
/// scroll-wheel events with very noticeable latency (the in-modal
/// `NSEventTrackingRunLoopMode` ends up starved by the host's
/// frequent redraw work). Pumping the modal session ourselves and
/// wrapping each iteration in an autorelease pool keeps wheel/scroll
/// events flowing at the same cadence as scrollbar drags — which
/// turned out to be the dominant cause of the "wheel feels stuck
/// while scrollbar feels fine" gap observed during T8 verification.
pub fn run_modal_window(window: &Retained<NSWindow>) -> NSModalResponse {
    let mtm = MainThreadMarker::new()
        .expect("ui::runloop::run_modal_window must be called on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    window.makeKeyAndOrderFront(None);

    let session = unsafe { app.beginModalSessionForWindow(window) };
    let mut response: NSModalResponse = NSModalResponseContinue;
    while response == NSModalResponseContinue {
        // A fresh pool per iteration keeps temporary AppKit objects
        // (the autoreleased event objects, NSStrings the data source
        // hands us, ...) from accumulating across the lifetime of the
        // modal session.
        autoreleasepool(|_| {
            response = unsafe { app.runModalSession(session) };

            // Drain whatever is sitting in NSEventTrackingRunLoopMode
            // and dispatch it to the panel. Wheel-scroll inertia ticks
            // and trackpad gesture phases arrive in this mode; inside
            // Affinity Photo 2's modal context they otherwise queue up
            // while the modal-panel mode is busy, which the user sees
            // as choppy wheel scroll even though the renderer is fast
            // (the scrollbar drag stays smooth because it dispatches
            // through the normal mouse-tracking path).
            unsafe {
                drain_tracking_events(&app);
            }
        });
    }
    unsafe { app.endModalSession(session) };
    window.orderOut(None);
    response
}

/// Pull every event currently waiting in
/// [`NSEventTrackingRunLoopMode`] and forward it via
/// [`NSApplication::sendEvent`]. `NSDate::distantPast` makes the call
/// non-blocking: if no event is queued the loop exits immediately.
///
/// # Safety
/// Must be called on the main thread (the caller in
/// [`run_modal_window`] holds a [`MainThreadMarker`]).
unsafe fn drain_tracking_events(app: &NSApplication) {
    loop {
        let event = unsafe {
            app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&NSDate::distantPast()),
                NSEventTrackingRunLoopMode,
                true,
            )
        };
        match event {
            Some(e) => unsafe { app.sendEvent(&e) },
            None => break,
        }
    }
}
