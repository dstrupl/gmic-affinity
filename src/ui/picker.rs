//! The user-facing picker dialog.
//!
//! T7 milestone: the panel now hosts an `NSOutlineView` inside an
//! `NSScrollView`, backed by a hardcoded data source from
//! [`super::picker_data_source::StaticDataSource`] — one folder
//! ("Artistic") containing one leaf ("Paint Brush"). The point is to
//! prove the `objc2::declare_class!` data-source pattern works inside
//! Affinity's run loop before we plug the real catalogue in for T8.
//!
//! The public entry point is still [`show_empty`] so T1's wiring
//! through `lib.rs` keeps compiling; T10 renames it to `show_picker`.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSBackingStoreType, NSOutlineView, NSPanel, NSScrollView, NSTableColumn,
    NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{CGPoint, CGRect, CGSize, MainThreadMarker, NSString};

use crate::ui::modal_close_delegate::ModalCloseDelegate;
use crate::ui::picker_data_source::StaticDataSource;
use crate::ui::runloop::run_modal_window;

/// Open the picker panel and run it modally until the user dismisses
/// it. Currently the data source is a hardcoded one-folder one-leaf
/// tree; the panel has no OK / Cancel actions yet.
///
/// Returns `None` only when [`MainThreadMarker::new()`] fails, meaning
/// this was called off the main thread — a contract violation for
/// `PluginMain`, not something the end user can trigger through the
/// panel.
pub fn show_empty() -> Option<()> {
    let mtm = MainThreadMarker::new()?;

    let content_rect = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize { width: 720.0, height: 520.0 },
    };
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Resizable;

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
    window.setTitle(&NSString::from_str("G'MIC (static tree)"));

    let delegate = ModalCloseDelegate::new(mtm);
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    let outline = build_outline_view(mtm);
    let data_source = StaticDataSource::new(mtm);
    unsafe {
        outline.setDataSource(Some(data_source.as_protocol()));
        outline.reloadData();
    }

    let scroll = build_scroll_view(mtm, content_rect, &outline);
    if let Some(content) = window.contentView() {
        unsafe { content.addSubview(&scroll) };
    }

    let _response = run_modal_window(&window);

    // Keep the data source alive for the life of the modal session.
    // `NSOutlineView::setDataSource:` does NOT retain its argument; if
    // we let `data_source` drop while the panel is still up AppKit
    // would crash. Dropping it here is correct because by the time we
    // get past `run_modal_window` the panel has been ordered out and
    // AppKit will not call back into the data source again.
    drop(data_source);
    Some(())
}

fn build_outline_view(mtm: MainThreadMarker) -> Retained<NSOutlineView> {
    let frame = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize { width: 720.0, height: 520.0 },
    };
    let outline: Retained<NSOutlineView> =
        unsafe { NSOutlineView::initWithFrame(mtm.alloc(), frame) };
    let column: Retained<NSTableColumn> = unsafe {
        NSTableColumn::initWithIdentifier(mtm.alloc(), &NSString::from_str("name"))
    };
    unsafe {
        outline.addTableColumn(&column);
        outline.setOutlineTableColumn(Some(&column));
    }
    outline
}

fn build_scroll_view(
    mtm: MainThreadMarker,
    frame: CGRect,
    outline: &Retained<NSOutlineView>,
) -> Retained<NSScrollView> {
    let scroll: Retained<NSScrollView> =
        unsafe { NSScrollView::initWithFrame(mtm.alloc(), frame) };
    unsafe {
        scroll.setHasVerticalScroller(true);
        // NSOutlineView : NSTableView : NSControl : NSView — hop up via
        // the inherited Deref chain to satisfy setDocumentView's &NSView.
        let view: &NSView = outline;
        scroll.setDocumentView(Some(view));
    }
    scroll
}
