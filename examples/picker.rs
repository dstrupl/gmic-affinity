//! Standalone driver for the picker panel.
//!
//! Run with:
//!
//! ```sh
//! cargo run --example picker --features live
//! ```
//!
//! Lets us iterate on the AppKit UI in seconds without installing the
//! plugin and restarting Affinity Photo 2. Behaves like a normal Cocoa
//! app: shows the picker modally, prints whatever the user picked
//! (currently nothing — T8 has no return value yet), and exits when
//! the panel closes.

#[cfg(not(feature = "live"))]
fn main() {
    eprintln!("This example requires the `live` feature.");
    eprintln!("Run: cargo run --example picker --features live");
    std::process::exit(2);
}

#[cfg(feature = "live")]
fn main() {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new().expect("examples/picker.rs runs on main thread");
    let app = NSApplication::sharedApplication(mtm);
    // Promote this unbundled cargo example to a Regular foreground
    // app; without this macOS treats us as background-only and the
    // panel never receives focus or even becomes visible above other
    // apps. The plugin build runs inside Affinity Photo, which has
    // already done this for us.
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    // SAFETY: NSApplication::activate is documented to be safe on the
    // main thread, which we verified via `MainThreadMarker`. It is
    // marked `unsafe` in objc2 only because most NSApplication methods
    // are.
    unsafe {
        app.activate();
    }

    autoreleasepool(|_| {
        GmicFilter::ui::picker::show_empty();
    });

    eprintln!("picker closed");
}
