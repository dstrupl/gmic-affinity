//! Standalone driver for the picker panel.
//!
//! Run with:
//!
//! ```sh
//! cargo run --release --example picker --features live
//! ```
//!
//! IMPORTANT: must be `--release`. The picker's modal run loop goes
//! through `NSApplication::beginModalSessionForWindow:` /
//! `runModalSession:`, whose binding in `objc2-app-kit` 0.2 declares
//! a typed `NSModalSession` struct pointer while the AppKit runtime
//! actually returns plain `void *`. `objc2`'s debug-build
//! type-encoding check panics on that mismatch. Release builds (and
//! the `make install`-produced plugin) disable the check via
//! `debug_assertions = off`, so the same code path works fine inside
//! Affinity Photo.
//!
//! Lets us iterate on the AppKit UI in seconds without installing the
//! plugin and restarting Affinity Photo 2. Behaves like a normal Cocoa
//! app: shows the picker modally, prints the chosen filter (command +
//! arg vector) to stdout on OK or `CANCEL` on Cancel/Esc, and exits
//! when the panel closes. This is the dev loop for T15 — round-trip
//! the show_picker -> ChosenFilter API without a live Affinity host.
//!
//! Debug helper: setting the environment variable `GMIC_PRESELECT` to
//! a G'MIC command (e.g. `fx_frame_cube`, `gcd_depth_blur`) causes
//! the panel to open with that filter's form pane already populated,
//! which is the fastest way to QA the parameter-form layout for a
//! specific filter without driving the outline view by hand. Unknown
//! commands are silently ignored.

// Gated through `required-features = ["live"]` in `Cargo.toml`; cargo
// silently skips this binary when the feature is off rather than
// requiring a noop stub here.
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
        let cat = GmicFilter::catalogue::builtin();
        let settings = GmicFilter::settings::Settings::load();
        match GmicFilter::ui::picker::show_picker(cat, settings.last.as_ref()) {
            Some(chosen) => {
                println!("OK");
                println!("command: {}", chosen.command);
                for (i, arg) in chosen.args.iter().enumerate() {
                    println!("arg[{i}]:  {arg}");
                }
            }
            None => {
                println!("CANCEL");
            }
        }
    });

    eprintln!("picker closed");
}
