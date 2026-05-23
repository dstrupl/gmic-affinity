//! Wrapper around AppKit's modal window loop so the rest of the UI
//! never has to touch `objc2` directly for the run-loop dance.
//!
//! The first milestone of the picker design is "open an empty `NSPanel`
//! from `SELECTOR_PARAMETERS`, close it with the title bar's close button,
//! return cleanly to Affinity". This file is the surface that proves that works. If
//! `runModal` misbehaves inside Affinity's run loop, this is where
//! we will find out and decide on the sheet-based fallback.

use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSModalResponse, NSWindow};
use objc2_foundation::{MainThreadMarker, NSDate, NSDefaultRunLoopMode, NSRunLoop};

/// Show `window` modally and block until it is dismissed. Returns the
/// AppKit modal response code (`.OK` / `.cancel` / a custom int).
///
/// MUST be called on the main thread. `PluginMain` is always invoked
/// on the main thread per the Photoshop SDK contract.
///
/// Use `runModalForWindow:` rather than a manual modal session. In
/// Affinity Photo 2, the manual pump caused heavy host window-update
/// notification churn while scrolling the picker; letting AppKit own
/// the modal loop removes that extra work.
pub fn run_modal_window(window: &Retained<NSWindow>) -> NSModalResponse {
    let mtm = MainThreadMarker::new()
        .expect("ui::runloop::run_modal_window must be called on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    window.makeKeyAndOrderFront(None);

    let response = unsafe { app.runModalForWindow(window) };
    window.orderOut(None);
    // `orderOut` only marks the window for removal; the actual frame
    // server compositing happens on the next run-loop cycle in
    // NSDefaultRunLoopMode. When PluginMain returns from
    // SELECTOR_PARAMETERS the host immediately fires SELECTOR_CONTINUE
    // and our gmic subprocess blocks the main thread for several
    // seconds, so without a flush here the dismissed picker stays
    // visibly on screen until gmic finishes — exactly the
    // "still-open dialog while the busy cursor spins" UX the user
    // reported. Pumping the default-mode run loop briefly hands the
    // window server its chance to actually remove the window.
    flush_main_runloop(mtm);
    response
}

/// Run the main run loop in [`NSDefaultRunLoopMode`] just long
/// enough for AppKit to drain pending Core Animation transactions
/// (window order-out, cursor changes, etc.). Bounded to ~50 ms so
/// we never hold up the host meaningfully even if no draws are
/// pending.
fn flush_main_runloop(_mtm: MainThreadMarker) {
    unsafe {
        let rl = NSRunLoop::mainRunLoop();
        let until = NSDate::dateWithTimeIntervalSinceNow(0.05);
        // `runMode:beforeDate:` returns when either an input source
        // fires or the deadline elapses, so we loop until the date
        // is reached. In practice one or two iterations is enough.
        loop {
            let _ = rl.runMode_beforeDate(NSDefaultRunLoopMode, &until);
            if NSDate::now().timeIntervalSinceDate(&until) >= 0.0 {
                break;
            }
        }
    }
}
