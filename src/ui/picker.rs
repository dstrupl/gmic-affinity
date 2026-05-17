//! The user-facing picker dialog. For Wave 1 this is a stub that opens
//! an empty `NSPanel` with OK / Cancel buttons. Each subsequent UI
//! task (T7 → T8 → T9 → T10) layers in real content here.

use objc2::rc::Retained;
use objc2_app_kit::{
    NSBackingStoreType, NSPanel, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{CGRect, CGPoint, CGSize, MainThreadMarker};

use crate::ui::runloop::run_modal_window;

/// Open an empty panel with a title bar and a close button. Blocks
/// until the user dismisses it. Returns `Some(())` on close, `None`
/// on any internal error — for the stub these are indistinguishable;
/// real return types come in T10.
pub fn show_empty() -> Option<()> {
    let mtm = MainThreadMarker::new()?;

    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Resizable;
    let content_rect = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize { width: 720.0, height: 520.0 },
    };

    let panel: Retained<NSPanel> = unsafe {
        NSPanel::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            content_rect,
            style,
            NSBackingStoreType::NSBackingStoreBuffered,
            false,
        )
    };
    let window: Retained<NSWindow> = Retained::into_super(panel);
    window.setTitle(&objc2_foundation::NSString::from_str("G'MIC (stub)"));

    let _response = run_modal_window(&window);
    Some(())
}
