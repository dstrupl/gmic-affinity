//! The user-facing picker dialog. Task 1 ships a stub only: an empty
//! `NSPanel` with a titled, closable, resizable chrome (no custom content
//! and no OK / Cancel actions yet). The only entry point is [`show_empty`],
//! returning [`Option<()>`]. Tasks T7 → T8 → T9 → T10 grow this into the
//! full G'MIC picker.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSBackingStoreType, NSPanel, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{CGRect, CGPoint, CGSize, MainThreadMarker};

use crate::ui::modal_close_delegate::ModalCloseDelegate;
use crate::ui::runloop::run_modal_window;

/// Open the stub panel and run it modally until the user dismisses it (e.g.
/// via the title bar close button). On success the stub always returns
/// `Some(())` after the modal session completes.
///
/// Returns `None` only when [`MainThreadMarker::new()`] fails, meaning this
/// was called off the main thread—a contract violation for [`PluginMain`],
/// not something the end user can trigger through the panel.
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

    let delegate = ModalCloseDelegate::new(mtm);
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    let _response = run_modal_window(&window);
    // T10: map NSModalResponse OK vs cancel (and other codes) into a meaningful Option / result.
    Some(())
}
