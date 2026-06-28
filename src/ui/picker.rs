//! The user-facing picker dialog.
//!
//! T10 milestone: the panel is the production picker. Layout (top to
//! bottom of the content view):
//!
//! - `NSSearchField` — types filter the tree in real time.
//! - `NSSplitView`:
//!   - left pane:  `NSScrollView` + `NSOutlineView` of catalogue folders / filters
//!   - right pane: `NSScrollView` + flipped `NSView` form rebuilt on
//!     every selection change by
//!     [`crate::ui::picker_form::FormController`].
//! - Button bar: `Reset Defaults` (left), `Cancel` and `OK` (right).
//!   Keyboard shortcuts: Return → OK, Escape → Cancel, double-click
//!   on a leaf row → OK.
//!
//! Public entry point [`show_picker`] returns:
//! - `Some(ChosenFilter)` when the user clicks OK on a leaf row.
//! - `None` when the user cancels (Esc / Cancel / window close) or
//!   when called off the main thread (a contract violation in
//!   `PluginMain`, defended against rather than handled).

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{declare_class, msg_send, msg_send_id, mutability, ClassType, DeclaredClass};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBackingStoreType, NSBezelStyle, NSButton, NSButtonType, NSEvent,
    NSModalResponseOK, NSOutlineView, NSPanel, NSScrollElasticity, NSScrollView, NSSearchField,
    NSSplitView, NSTableColumn, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    CGFloat, CGPoint, CGRect, CGSize, MainThreadMarker, NSIndexSet, NSSize, NSString,
};

use crate::catalogue::{self, Catalogue, ChosenFilter};
use crate::settings::{LastChoice, Settings};
use crate::ui::modal_close_delegate::ModalCloseDelegate;
use crate::ui::picker_actions::{
    sel_on_cancel, sel_on_double_click, sel_on_ok, sel_on_reset, PickerActions,
};
use crate::ui::picker_catalogue_data_source::CatalogueDataSource;
use crate::ui::picker_form::{build_form_pane, FormController, FormPane};
use crate::ui::runloop::run_modal_window;

/// Height reserved at the top of the panel for the search field.
const SEARCH_BAR_HEIGHT: f64 = 28.0;
/// Margin between the search field and the outline view below it.
const SEARCH_BAR_GAP: f64 = 4.0;
/// Margin around the search field horizontally.
const SEARCH_BAR_HMARGIN: f64 = 8.0;
/// Height of the bottom button bar (OK / Cancel / Reset).
const BUTTON_BAR_HEIGHT: f64 = 36.0;
/// Vertical margin between the button bar and the split view above it.
const BUTTON_BAR_GAP: f64 = 6.0;
/// Horizontal margin around the button bar.
const BUTTON_BAR_HMARGIN: f64 = 12.0;
/// Width of an individual action button (OK / Cancel / Reset).
const BUTTON_WIDTH: f64 = 100.0;
/// Height of an individual action button.
const BUTTON_HEIGHT: f64 = 24.0;
/// Horizontal gap between two adjacent buttons in the trailing group.
const BUTTON_GAP: f64 = 8.0;
/// Initial fraction of the split-view width allocated to the tree
/// pane on the left. The form pane gets the rest. The user can drag
/// the divider after the panel opens; we restore the resulting
/// position from the window's autosave name via `setAutosaveName:` on
/// the split view itself.
// Tree column is a fixed list of filter names that rarely needs more
// than ~280pt; give the rest to the form pane so parameter
// descriptions have room to wrap without forcing the user to drag.
const TREE_PANE_WIDTH_FRACTION: CGFloat = 0.38;
/// Minimum tree-pane width in points so the user can't accidentally
/// drag the divider so far right that the tree disappears.
const TREE_PANE_MIN_WIDTH: CGFloat = 200.0;
/// Minimum form-pane width — keeps slider + label rows readable even
/// when the divider is dragged hard against the right edge.
const FORM_PANE_MIN_WIDTH: CGFloat = 220.0;
/// Initial width of the rightmost preview pane, in points.
const PREVIEW_PANE_WIDTH: CGFloat = 280.0;
/// Minimum preview-pane width so the image stays legible.
const PREVIEW_PANE_MIN_WIDTH: CGFloat = 220.0;

/// Open the picker panel and run it modally until the user dismisses it.
///
/// The data source is populated from `catalogue` plus a snapshot of
/// `Settings::recent`. When `last_choice` is `Some`, the corresponding
/// filter row is pre-selected, scrolled into view, and its parameter
/// form pre-filled with the saved (reconciled) values.
///
/// Returns:
/// - `Some(ChosenFilter)` when the user clicks OK on a leaf row
///   (or double-clicks a leaf, or presses Return with a leaf
///   selected).
/// - `None` when the user cancels (Esc, the Cancel button, or the
///   window's close control) or when called off the main thread (a
///   contract violation for `PluginMain`, defended against here rather
///   than handled).
pub fn show_picker(
    catalogue: &'static Catalogue,
    last_choice: Option<&LastChoice>,
) -> Option<ChosenFilter> {
    let mtm = MainThreadMarker::new()?;

    let window = build_picker_window(mtm);

    let delegate = ModalCloseDelegate::new(mtm);
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    let settings = Settings::load();
    let recent = settings.recent.clone();
    let remembered = settings.remembered_args.clone();
    let data_source = CatalogueDataSource::new(mtm, catalogue, recent);

    // Use the contentView's actual (post-autosave-restore) bounds as
    // the basis for subview frames; the autoresizing masks set on
    // each subview then handle every subsequent live resize.
    let content_view = window.contentView()?;
    let content_bounds = content_view.bounds();
    log_content_bounds(content_bounds);

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

    // Tree (left pane) — same scroll view we have shipped since T8,
    // just smaller because the split view now owns its frame.
    let tree_scroll = build_scroll_view(mtm, content_bounds, &outline);
    // Form (right pane). Builds its own scroll view + flipped view and
    // attaches the form controller to the outline view's delegate
    // slot so every selection change repopulates the form.
    let FormPane {
        controller: form_controller,
        root_view: form_scroll,
    } = build_form_pane(mtm, data_source.clone(), remembered);
    unsafe {
        outline.setDelegate(Some(form_controller.as_outline_view_delegate()));
    }

    // Preview (right-most pane). A plain Rust struct wrapped in `Rc`:
    // the form controller holds one clone and updates it on every
    // selection change; we keep another clone alive here until after
    // the modal pump exits so the AppKit views it owns aren't freed
    // out from under the live panel.
    let preview = std::rc::Rc::new(crate::ui::preview_pane::build_preview_view(mtm));
    form_controller.set_preview(preview.clone());

    // Split view sits between the search bar and the button bar.
    let split = build_split_view(
        mtm,
        content_bounds,
        &tree_scroll,
        &form_scroll,
        &preview.root,
    );

    // Bottom button bar: Reset Defaults (leading) + Cancel/OK (trailing).
    let buttons = build_button_bar(mtm, content_bounds);

    attach_picker_views(&content_view, &search, &split, &buttons);
    configure_button_shortcuts(&buttons);
    // OK is disabled until a leaf row is selected. The action
    // controller flips this every time the outline view's selection
    // changes (see `FormController::outlineViewSelectionDidChange:` →
    // `PickerActions::refresh_ok_enabled`).
    buttons.ok.setEnabled(false);

    // Build the action controller and wire targets / actions. The
    // controller holds strong refs to the outline view, form
    // controller, data source, and OK button so they all stay alive
    // until we drop `actions` after the modal pump exits.
    let actions = build_picker_actions(mtm, &outline, &form_controller, &data_source, &buttons);
    form_controller.set_actions(actions.clone());
    wire_picker_actions(&outline, &buttons, &actions);

    // The search field receives focus immediately so typing filters
    // the tree without an extra click.
    let search_view: &NSView = &search;
    window.setInitialFirstResponder(Some(search_view));

    // Pre-selection order: GMIC_PRESELECT (debug-only env var) wins
    // over `last_choice` so developers can iterate on a specific
    // filter from the command line.
    let preselect_command: Option<String> = std::env::var("GMIC_PRESELECT")
        .ok()
        .or_else(|| last_choice.map(|l| l.command.clone()));

    preselect_filter(&data_source, &outline, preselect_command.as_deref());
    log_picker_layout(&search, &split);

    let response = run_modal_window(&window);

    // Translate the modal response into the public return shape. The
    // action controller has already captured the leaf + values it
    // observed at the moment OK fired; we just move them out here.
    let chosen = if response == NSModalResponseOK {
        actions.take_chosen().map(|(filter, args)| ChosenFilter {
            command: filter.command.clone(),
            args,
        })
    } else {
        None
    };

    // Keep delegates alive until after `run_modal_window` returns.
    // `NSOutlineView::setDataSource:`, `setDelegate:`,
    // `NSSearchField::setDelegate:`, `setTarget:`, `setAction:`,
    // and the window-delegate slot all store WEAK references. We
    // explicitly drop them here so the order is unambiguous: the
    // modal pump has already exited and AppKit will not call back
    // into them.
    // The form controller registered itself as an
    // NSViewFrameDidChangeNotification observer on the scroll view's
    // clip view in `build_form_pane`. NSNotificationCenter keeps a
    // raw, non-retaining reference to its observers, so dropping the
    // controller without first removing it would leave the clip view
    // posting notifications into a freed Objective-C object the next
    // time something resizes the right pane (e.g. during a Cmd-W
    // close animation). Unregister first, then drop.
    unsafe {
        objc2_foundation::NSNotificationCenter::defaultCenter().removeObserver(&form_controller);
    }
    drop(actions);
    drop(form_controller);
    drop(data_source);
    // The preview view owns AppKit views still parented in the split
    // view's hierarchy; drop it only after the modal pump has exited
    // and the controller (its other `Rc` holder) is gone.
    drop(preview);

    chosen
}

fn build_picker_window(mtm: MainThreadMarker) -> Retained<NSWindow> {
    // Initial content rect used only if no autosaved frame exists.
    let initial_rect = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize {
            width: 1040.0,
            height: 520.0,
        },
    };
    let style =
        NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable;

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
    // subviews.
    unsafe {
        window.setFrameAutosaveName(&NSString::from_str("GmicPickerPanel"));
    }
    // Stop the user from dragging the panel down to nothing while
    // they're trying to resize it. ~520x360 still shows a useful
    // amount of the tree plus the search field.
    window.setMinSize(NSSize {
        width: 760.0,
        height: 360.0,
    });

    window
}

fn log_content_bounds(content_bounds: CGRect) {
    crate::logging::log(&format!(
        "picker: contentView bounds = {}x{} (origin {},{})",
        content_bounds.size.width,
        content_bounds.size.height,
        content_bounds.origin.x,
        content_bounds.origin.y,
    ));
}

fn attach_picker_views(
    content_view: &NSView,
    search: &Retained<NSSearchField>,
    split: &Retained<NSSplitView>,
    buttons: &ButtonBar,
) {
    unsafe {
        // NOTE: do NOT set `wantsLayer` on the contentView here. It
        // breaks AppKit's autoresizing of immediate subviews (search
        // field stops pinning to the top, scroll view loses its
        // scroller). Layer-backing the scroll view directly is both
        // safe and sufficient.
        content_view.addSubview(search);
        content_view.addSubview(split);
        let reset_view: &NSView = &buttons.reset;
        let cancel_view: &NSView = &buttons.cancel;
        let ok_view: &NSView = &buttons.ok;
        content_view.addSubview(reset_view);
        content_view.addSubview(cancel_view);
        content_view.addSubview(ok_view);
    }
}

fn configure_button_shortcuts(buttons: &ButtonBar) {
    // AppKit dispatches both keystrokes and clicks through the same
    // target/action, so nothing else has to special-case them.
    unsafe {
        buttons.ok.setKeyEquivalent(&NSString::from_str("\r"));
        buttons
            .cancel
            .setKeyEquivalent(&NSString::from_str("\u{1B}"));
    }
}

fn build_picker_actions(
    mtm: MainThreadMarker,
    outline: &Retained<NSOutlineView>,
    form_controller: &Retained<FormController>,
    data_source: &Retained<CatalogueDataSource>,
    buttons: &ButtonBar,
) -> Retained<PickerActions> {
    PickerActions::new(
        mtm,
        outline.clone(),
        form_controller.clone(),
        data_source.clone(),
        buttons.ok.clone(),
    )
}

fn wire_picker_actions(
    outline: &Retained<NSOutlineView>,
    buttons: &ButtonBar,
    actions: &Retained<PickerActions>,
) {
    let actions_obj: &AnyObject = actions;
    unsafe {
        let ok_ctrl: &objc2_app_kit::NSControl = &buttons.ok;
        let cancel_ctrl: &objc2_app_kit::NSControl = &buttons.cancel;
        let reset_ctrl: &objc2_app_kit::NSControl = &buttons.reset;
        ok_ctrl.setTarget(Some(actions_obj));
        ok_ctrl.setAction(Some(sel_on_ok()));
        cancel_ctrl.setTarget(Some(actions_obj));
        cancel_ctrl.setAction(Some(sel_on_cancel()));
        reset_ctrl.setTarget(Some(actions_obj));
        reset_ctrl.setAction(Some(sel_on_reset()));

        let outline_ctrl: &objc2_app_kit::NSControl = outline;
        outline_ctrl.setTarget(Some(actions_obj));
        outline.setDoubleAction(Some(sel_on_double_click()));
    }
}

fn preselect_filter(
    data_source: &Retained<CatalogueDataSource>,
    outline: &Retained<NSOutlineView>,
    command: Option<&str>,
) {
    let Some(cmd) = command else {
        return;
    };

    if let Some(row) = data_source.expand_to_filter(outline, cmd) {
        unsafe {
            let indexes = NSIndexSet::indexSetWithIndex(row as usize);
            outline.selectRowIndexes_byExtendingSelection(&indexes, false);
            outline.scrollRowToVisible(row);
        }
        // `selectRowIndexes:` posts an
        // `NSOutlineViewSelectionDidChangeNotification`, so the form
        // controller's delegate has already populated the right pane.
        crate::logging::log(&format!("picker: pre-selected '{cmd}' at row {row}",));
    } else {
        crate::logging::log(&format!(
            "picker: pre-select '{cmd}' missing from catalogue, skipping"
        ));
    }
}

fn log_picker_layout(search: &Retained<NSSearchField>, split: &Retained<NSSplitView>) {
    let sf2 = search.frame();
    let spf = split.frame();
    crate::log(&format!(
        "picker: after addSubview, search={}x{}@({},{}), split={}x{}@({},{})",
        sf2.size.width,
        sf2.size.height,
        sf2.origin.x,
        sf2.origin.y,
        spf.size.width,
        spf.size.height,
        spf.origin.x,
        spf.origin.y,
    ));
}

/// Wrapper around [`show_picker`] kept for the standalone
/// `examples/picker` runner from earlier in the project, which still
/// passes no arguments and ignores the return value. New callers
/// should prefer [`show_picker`].
///
/// The default catalogue + no last-choice path means the panel opens
/// with the form pane in its empty-selection placeholder state.
pub fn show_empty() -> Option<()> {
    show_picker(catalogue::builtin(), None).map(|_| ())
}

declare_class! {
    /// Scroll view used for the picker tree. Affinity's plugin modal
    /// context scrolls this outline smoothly only when wheel events
    /// are handled on our subclass boundary before AppKit continues
    /// through the normal `NSScrollView` implementation.
    struct TreeScrollView;

    unsafe impl ClassType for TreeScrollView {
        type Super = NSScrollView;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "GmicPickerTreeScrollView";
    }

    impl DeclaredClass for TreeScrollView {}

    unsafe impl NSObjectProtocol for TreeScrollView {}

    unsafe impl TreeScrollView {
        #[method(scrollWheel:)]
        unsafe fn scroll_wheel(&self, event: &NSEvent) {
            let _: () = unsafe { msg_send![super(self), scrollWheel: event] };
        }
    }
}

impl TreeScrollView {
    fn new_with_frame(mtm: MainThreadMarker, frame: CGRect) -> Retained<Self> {
        unsafe { msg_send_id![mtm.alloc::<Self>(), initWithFrame: frame] }
    }
}

/// Build the side-by-side split view that hosts the tree on the left
/// and the parameter form on the right. The frame slots beneath the
/// search bar and grows with the window.
fn build_split_view(
    mtm: MainThreadMarker,
    content_bounds: CGRect,
    tree: &NSScrollView,
    form: &NSScrollView,
    preview: &NSView,
) -> Retained<NSSplitView> {
    // The split view sits between the search bar (top) and the
    // button bar (bottom). Cocoa's coordinate origin is bottom-left,
    // so the split view's y origin is the height of the button bar
    // and its height excludes both bars.
    let split_height = content_bounds.size.height
        - SEARCH_BAR_HEIGHT
        - SEARCH_BAR_GAP * 2.0
        - BUTTON_BAR_HEIGHT
        - BUTTON_BAR_GAP;
    let frame = CGRect {
        origin: CGPoint {
            x: 0.0,
            y: BUTTON_BAR_HEIGHT + BUTTON_BAR_GAP,
        },
        size: CGSize {
            width: content_bounds.size.width,
            height: split_height,
        },
    };
    let split: Retained<NSSplitView> = unsafe { NSSplitView::initWithFrame(mtm.alloc(), frame) };
    unsafe {
        // Vertical = left/right layout (the divider is a vertical
        // line). AppKit's vocabulary here is the opposite of CSS:
        // `setVertical(true)` puts panes side-by-side, not stacked.
        split.setVertical(true);
        split.setDividerStyle(objc2_app_kit::NSSplitViewDividerStyle::Thin);
        split.setAutoresizingMask(
            NSAutoresizingMaskOptions::NSViewWidthSizable
                | NSAutoresizingMaskOptions::NSViewHeightSizable,
        );
        // Subviews go in left-to-right order. The split view sizes
        // them according to its own layout pass once we add them.
        let tree_view: &NSView = tree;
        let form_view: &NSView = form;
        split.addSubview(tree_view);
        split.addSubview(form_view);
        split.addSubview(preview);
        // NSSplitView only lays out its subviews when explicitly
        // told to. Without this call AppKit leaves the panes at
        // their initial untouched 0×height frames and the panel
        // looks completely empty.
        split.adjustSubviews();

        // Three panes, two dividers. The tree takes its fraction of
        // the width, the preview takes a fixed slice on the right, and
        // the form gets the remainder — each clamped so no pane falls
        // below its minimum.
        let total_width = frame.size.width;
        let mut tree_width = (total_width * TREE_PANE_WIDTH_FRACTION).max(TREE_PANE_MIN_WIDTH);
        let preview_width = PREVIEW_PANE_WIDTH
            .min(total_width - TREE_PANE_MIN_WIDTH - FORM_PANE_MIN_WIDTH)
            .max(PREVIEW_PANE_MIN_WIDTH);
        if total_width - tree_width - preview_width < FORM_PANE_MIN_WIDTH {
            tree_width = total_width - preview_width - FORM_PANE_MIN_WIDTH;
        }
        // Divider 0 between tree and form; divider 1 between form and
        // preview.
        split.setPosition_ofDividerAtIndex(tree_width, 0);
        split.setPosition_ofDividerAtIndex(
            tree_width + (total_width - tree_width - preview_width),
            1,
        );
    }
    split
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
                size: CGSize {
                    width: 0.0,
                    height: 0.0,
                },
            },
        )
    };
    let column: Retained<NSTableColumn> =
        unsafe { NSTableColumn::initWithIdentifier(mtm.alloc(), &NSString::from_str("name")) };
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
) -> Retained<TreeScrollView> {
    // Outline view sits below the search bar.
    let scroll_height = content_bounds.size.height - SEARCH_BAR_HEIGHT - SEARCH_BAR_GAP * 2.0;
    let frame = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize {
            width: content_bounds.size.width,
            height: scroll_height,
        },
    };
    let scroll = TreeScrollView::new_with_frame(mtm, frame);
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

/// Bottom button bar: Reset Defaults (leading) + Cancel/OK (trailing).
/// All three buttons are owned by [`PickerActions`] via target/action
/// once the caller wires them in.
struct ButtonBar {
    reset: Retained<NSButton>,
    cancel: Retained<NSButton>,
    ok: Retained<NSButton>,
}

fn build_button_bar(mtm: MainThreadMarker, content_bounds: CGRect) -> ButtonBar {
    // Y origin: the bottom row of the content view, minus a half-gap
    // so the buttons sit visually above the panel chrome.
    let y = (BUTTON_BAR_HEIGHT - BUTTON_HEIGHT) / 2.0;
    let ok = make_text_button(mtm, "OK");
    let cancel = make_text_button(mtm, "Cancel");
    let reset = make_text_button(mtm, "Reset Defaults");
    // OK on the right edge, Cancel left of OK. Reset on the left edge.
    let total_w = content_bounds.size.width;
    let ok_x = total_w - BUTTON_BAR_HMARGIN - BUTTON_WIDTH;
    let cancel_x = ok_x - BUTTON_GAP - BUTTON_WIDTH;
    let reset_x = BUTTON_BAR_HMARGIN;
    unsafe {
        // The leading button stays glued to the left, the trailing
        // pair stays glued to the right. The combination is what
        // gives the button bar its "Mac-native" behaviour on resize.
        ok.setFrame(CGRect {
            origin: CGPoint { x: ok_x, y },
            size: CGSize {
                width: BUTTON_WIDTH,
                height: BUTTON_HEIGHT,
            },
        });
        ok.setAutoresizingMask(
            NSAutoresizingMaskOptions::NSViewMinXMargin
                | NSAutoresizingMaskOptions::NSViewMaxYMargin,
        );
        cancel.setFrame(CGRect {
            origin: CGPoint { x: cancel_x, y },
            size: CGSize {
                width: BUTTON_WIDTH,
                height: BUTTON_HEIGHT,
            },
        });
        cancel.setAutoresizingMask(
            NSAutoresizingMaskOptions::NSViewMinXMargin
                | NSAutoresizingMaskOptions::NSViewMaxYMargin,
        );
        reset.setFrame(CGRect {
            origin: CGPoint { x: reset_x, y },
            size: CGSize {
                width: BUTTON_WIDTH + 30.0,
                height: BUTTON_HEIGHT,
            },
        });
        reset.setAutoresizingMask(
            NSAutoresizingMaskOptions::NSViewMaxXMargin
                | NSAutoresizingMaskOptions::NSViewMaxYMargin,
        );
    }
    ButtonBar { reset, cancel, ok }
}

/// Build a plain rounded push button with the given title and the
/// standard system control size. Used by [`build_button_bar`] for OK,
/// Cancel, and Reset. The frame is irrelevant — the caller resets it
/// with the final layout coordinates immediately after.
fn make_text_button(mtm: MainThreadMarker, title: &str) -> Retained<NSButton> {
    let btn: Retained<NSButton> = unsafe {
        NSButton::initWithFrame(
            mtm.alloc(),
            CGRect {
                origin: CGPoint { x: 0.0, y: 0.0 },
                size: CGSize {
                    width: BUTTON_WIDTH,
                    height: BUTTON_HEIGHT,
                },
            },
        )
    };
    unsafe {
        btn.setButtonType(NSButtonType::MomentaryLight);
        // `NSBezelStyle::Rounded` is the documented "default Mac
        // push button" look; the symbol is an alias for `Push` in
        // modern AppKit and the `Rounded` form has been marked
        // deprecated in `objc2-app-kit` to mirror the SDK headers.
        btn.setBezelStyle(NSBezelStyle::Push);
        btn.setTitle(&NSString::from_str(title));
    }
    btn
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
