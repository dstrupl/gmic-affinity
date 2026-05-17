//! The user-facing picker dialog.
//!
//! T8 milestone: the panel hosts a search field on top of an
//! `NSOutlineView` (inside an `NSScrollView`), backed by
//! [`super::picker_catalogue_data_source::CatalogueDataSource`]. The
//! data source reads the bundled catalogue and the `Settings::recent`
//! snapshot taken at panel-open time, and also serves as the search
//! field's `NSTextFieldDelegate` so every keystroke filters the tree.
//!
//! The public entry point is still [`show_empty`] so T1's wiring
//! through `lib.rs` keeps compiling; T10 renames it to `show_picker`
//! and adds an `Option<ChosenFilter>` return value.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBackingStoreType, NSOutlineView, NSPanel, NSScrollElasticity,
    NSScrollView, NSSearchField, NSTableColumn, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{CGPoint, CGRect, CGSize, MainThreadMarker, NSSize, NSString};

use crate::catalogue;
use crate::settings::Settings;
use crate::ui::modal_close_delegate::ModalCloseDelegate;
use crate::ui::picker_catalogue_data_source::CatalogueDataSource;
use crate::ui::runloop::run_modal_window;

/// Height reserved at the top of the panel for the search field.
const SEARCH_BAR_HEIGHT: f64 = 28.0;
/// Margin between the search field and the outline view below it.
const SEARCH_BAR_GAP: f64 = 4.0;
/// Margin around the search field horizontally.
const SEARCH_BAR_HMARGIN: f64 = 8.0;

/// Open the picker panel and run it modally until the user dismisses
/// it. The data source is populated from
/// [`crate::catalogue::builtin`] plus a snapshot of
/// `Settings::recent`. There is still no OK / Cancel action — T10 adds
/// that.
///
/// Returns `None` only when [`MainThreadMarker::new`] fails, meaning
/// this was called off the main thread — a contract violation for
/// `PluginMain`, not something the end user can trigger through the
/// panel.
pub fn show_empty() -> Option<()> {
    let mtm = MainThreadMarker::new()?;

    // Initial content rect used only if no autosaved frame exists.
    let initial_rect = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize { width: 720.0, height: 520.0 },
    };
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Resizable;

    let panel: Retained<NSPanel> = unsafe {
        NSPanel::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            initial_rect,
            style,
            NSBackingStoreType::NSBackingStoreBuffered,
            false,
        )
    };
    let window: Retained<NSWindow> = Retained::into_super(panel);
    window.setTitle(&NSString::from_str("G'MIC Filters"));
    // Centre the window first; if a saved frame exists, the next
    // call's autosave-restore will overwrite this position. Without
    // it, NSPanel created at `origin=(0,0)` lands at the bottom-left
    // of the screen (hidden behind the Dock).
    window.center();
    let fr = window.frame();
    let wid = unsafe { window.windowNumber() };
    crate::logging::log(&format!(
        "picker: windowNumber={} frame after center = {}x{} at ({},{})",
        wid, fr.size.width, fr.size.height, fr.origin.x, fr.origin.y
    ));
    // Persist window frame across sessions. AppKit stores this under
    // the host app's preferences ("Affinity Photo 2"), keyed by the
    // autosave name; subsequent opens come up at the user's last size
    // and position. IMPORTANT: this call may resize the contentView
    // synchronously to whatever was last saved, so we must read the
    // contentView's *current* bounds AFTER this call to size our
    // subviews — using the initial 720×520 here is what was leaving
    // a grey gap at the top of larger restored windows.
    unsafe {
        window.setFrameAutosaveName(&NSString::from_str("GmicPickerPanel"));
    }
    // Stop the user from dragging the panel down to nothing while
    // they're trying to resize it. ~520x360 still shows a useful
    // amount of the tree plus the search field.
    window.setMinSize(NSSize { width: 520.0, height: 360.0 });

    let delegate = ModalCloseDelegate::new(mtm);
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    let catalogue = catalogue::builtin();
    let recent = Settings::load().recent;
    let data_source = CatalogueDataSource::new(mtm, catalogue, recent);

    // Use the contentView's actual (post-autosave-restore) bounds as
    // the basis for subview frames; the autoresizing masks set on
    // each subview then handle every subsequent live resize.
    let content_view = window.contentView()?;
    let content_bounds = content_view.bounds();
    crate::logging::log(&format!(
        "picker: contentView bounds = {}x{} (origin {},{})",
        content_bounds.size.width,
        content_bounds.size.height,
        content_bounds.origin.x,
        content_bounds.origin.y,
    ));

    let outline = build_outline_view(mtm);
    unsafe {
        outline.setDataSource(Some(data_source.as_data_source()));
        outline.reloadData();
    }
    data_source.set_outline(outline.clone());

    let search = build_search_field(mtm, content_bounds);
    unsafe {
        search.setDelegate(Some(data_source.as_search_field_delegate()));
    }
    let sf = search.frame();
    crate::log(&format!(
        "picker: search frame = {}x{} at ({}, {})",
        sf.size.width, sf.size.height, sf.origin.x, sf.origin.y
    ));

    let scroll = build_scroll_view(mtm, content_bounds, &outline);
    let scf = scroll.frame();
    crate::log(&format!(
        "picker: scroll frame = {}x{} at ({}, {})",
        scf.size.width, scf.size.height, scf.origin.x, scf.origin.y
    ));
    unsafe {
        // NOTE: do NOT set `wantsLayer` on the contentView here. It
        // breaks AppKit's autoresizing of immediate subviews (search
        // field stops pinning to the top, scroll view loses its
        // scroller) — confirmed by manual QA on the second T8 pass.
        // Layer-backing the scroll view directly (see
        // `build_scroll_view`) is both safe and sufficient.
        content_view.addSubview(&search);
        content_view.addSubview(&scroll);
    }
    // Log post-add frames in case AppKit auto-adjusts.
    let sf2 = search.frame();
    let scf2 = scroll.frame();
    crate::log(&format!(
        "picker: after addSubview, search={}x{}@({},{}), scroll={}x{}@({},{})",
        sf2.size.width, sf2.size.height, sf2.origin.x, sf2.origin.y,
        scf2.size.width, scf2.size.height, scf2.origin.x, scf2.origin.y,
    ));

    let _response = run_modal_window(&window);

    // Keep the data source alive for the life of the modal session.
    // `NSOutlineView::setDataSource:` and `NSTextField::setDelegate:`
    // both store WEAK references. If we let `data_source` drop while
    // the panel is still up AppKit would crash. Dropping it here is
    // correct because by the time we get past `run_modal_window` the
    // panel has been ordered out and AppKit will not call back into
    // it again.
    drop(data_source);
    Some(())
}

fn build_outline_view(mtm: MainThreadMarker) -> Retained<NSOutlineView> {
    // The frame supplied here is irrelevant: the scroll view sets it
    // to its own contentSize via setDocumentView:. Use NSZeroRect so
    // future readers don't wonder which content-rect we meant.
    let outline: Retained<NSOutlineView> = unsafe {
        NSOutlineView::initWithFrame(
            mtm.alloc(),
            CGRect {
                origin: CGPoint { x: 0.0, y: 0.0 },
                size: CGSize { width: 0.0, height: 0.0 },
            },
        )
    };
    let column: Retained<NSTableColumn> = unsafe {
        NSTableColumn::initWithIdentifier(mtm.alloc(), &NSString::from_str("name"))
    };
    unsafe {
        column.setTitle(&NSString::from_str("Filter"));
        // Generous default width so a typical screen shows the full
        // path of all but the most deeply-nested entries without
        // forcing a horizontal scroll. The column resizes with the
        // panel (the scroll view's autoresizing mask handles the
        // outer frame; the column gets the slack).
        column.setWidth(680.0);
        column.setMinWidth(200.0);
        outline.addTableColumn(&column);
        outline.setOutlineTableColumn(Some(&column));
        // Pin a stable row height so AppKit does not call back into
        // the delegate to measure each row on every scroll tick —
        // that is what was making wheel-scroll feel sluggish in the
        // first cut. 18pt matches the standard NSOutlineView row
        // height on macOS.
        outline.setRowHeight(18.0);
        outline.setUsesAlternatingRowBackgroundColors(true);
        // Reduce per-frame layout work during scroll: do not let the
        // outline column rebalance on every scroll tick, and skip
        // the header view (we already title the column "Filter"
        // statically through `setTitle:` for use in any future
        // multi-column layout).
        outline.setAutoresizesOutlineColumn(false);
        outline.setHeaderView(None);
    }
    outline
}

fn build_scroll_view(
    mtm: MainThreadMarker,
    content_bounds: CGRect,
    outline: &Retained<NSOutlineView>,
) -> Retained<NSScrollView> {
    // Outline view sits below the search bar.
    let scroll_height =
        content_bounds.size.height - SEARCH_BAR_HEIGHT - SEARCH_BAR_GAP * 2.0;
    let frame = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize {
            width: content_bounds.size.width,
            height: scroll_height,
        },
    };
    let scroll: Retained<NSScrollView> =
        unsafe { NSScrollView::initWithFrame(mtm.alloc(), frame) };
    unsafe {
        scroll.setHasVerticalScroller(true);
        // Grow with the window: width + height fully flexible so a
        // resize feels like dragging the tree's edge, not the panel's
        // outline.
        scroll.setAutoresizingMask(
            NSAutoresizingMaskOptions::NSViewWidthSizable
                | NSAutoresizingMaskOptions::NSViewHeightSizable,
        );
        // Layer-backed scrolling is dramatically smoother than the
        // default CPU-only path, especially for an outline view with
        // a thousand-plus rows — fixes the "wheel feels stuck while
        // the scrollbar feels fine" gap.
        let view: &NSView = &scroll;
        view.setWantsLayer(true);
        // Collapse the entire scroll subtree (clip view + outline view
        // + every row + every cell) into a single CALayer for scroll
        // compositing. The tree is static text, never animates per
        // cell, so this is a pure win: the compositor moves one bitmap
        // instead of asking AppKit to redraw the row hierarchy on
        // every frame. Crucial inside Affinity Photo 2's modal context
        // where the host is competing for compositor time.
        view.setCanDrawSubviewsIntoLayer(true);
        // Disable predominant-axis biasing: the heuristic adds
        // per-event delay deciding "is this scroll horizontal or
        // vertical" and inside Affinity Photo 2's busy modal run
        // loop that delay accumulates into the very visible "wheel
        // scroll lag" you saw even after the rendering path was
        // already smooth (drag-the-scrollbar was fast).
        scroll.setUsesPredominantAxisScrolling(false);
        // Disable rubber-band overscroll. The bounce animation needs
        // CoreAnimation transactions to land on every frame, and
        // inside a busy host that becomes a real per-scroll-event
        // cost. The tree never has horizontal content so the
        // horizontal axis would never bounce anyway; we just turn
        // the feature off everywhere for symmetry.
        scroll.setVerticalScrollElasticity(NSScrollElasticity::None);
        scroll.setHorizontalScrollElasticity(NSScrollElasticity::None);
        // NSOutlineView : NSTableView : NSControl : NSView — hop up via
        // the inherited Deref chain to satisfy setDocumentView's &NSView.
        let doc_view: &NSView = outline;
        scroll.setDocumentView(Some(doc_view));
    }
    scroll
}

fn build_search_field(mtm: MainThreadMarker, content_bounds: CGRect) -> Retained<NSSearchField> {
    // Cocoa coordinates: origin is bottom-left, so the search field
    // goes at the *top* of the content rect.
    let frame = CGRect {
        origin: CGPoint {
            x: SEARCH_BAR_HMARGIN,
            y: content_bounds.size.height - SEARCH_BAR_HEIGHT - SEARCH_BAR_GAP,
        },
        size: CGSize {
            width: content_bounds.size.width - SEARCH_BAR_HMARGIN * 2.0,
            height: SEARCH_BAR_HEIGHT,
        },
    };
    let search: Retained<NSSearchField> =
        unsafe { NSSearchField::initWithFrame(mtm.alloc(), frame) };
    unsafe {
        search.setRecentsAutosaveName(Some(&NSString::from_str("GmicPickerSearch")));
        search.setPlaceholderString(Some(&NSString::from_str("Search filters…")));
        // Stay glued to the top edge and stretch horizontally as the
        // window resizes (`ViewMinYMargin` keeps the *bottom* gap
        // fixed, anchoring the field at top).
        let view: &NSView = &search;
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::NSViewWidthSizable
                | NSAutoresizingMaskOptions::NSViewMinYMargin,
        );
    }
    search
}
