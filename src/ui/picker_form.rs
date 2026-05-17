//! Right-pane form controller for the picker dialog (T9).
//!
//! Responsibilities:
//!
//! - Owns the right-pane root view (an `NSScrollView` wrapping a
//!   flipped `NSView` we lay out manually).
//! - Acts as the [`NSOutlineViewDelegate`] for the left-pane outline
//!   view: whenever the selection changes we rebuild the form to
//!   match the newly-selected filter.
//!
//! Sub-commit A (this file's first cut) only renders the filter's
//! `command`, `description`, and parameter count as plain
//! `NSTextField` rows — enough to prove the split-pane wiring and
//! selection delegate work end to end inside Affinity Photo 2
//! before we add the full [`crate::catalogue::ParamKind`] → control
//! mapping in sub-commit B.
//!
//! We deliberately layout rows by absolute frame instead of via
//! `NSStackView` + Auto Layout: the latter requires extra
//! `objc2-app-kit` features (`NSLayoutConstraint` API surface) and
//! interacts awkwardly with our scroll view's manual sizing. A flat
//! flipped `NSView` with manual `(x, y, w, h)` placement is enough
//! until the form grows into a real per-`ParamKind` widget tree
//! (sub-commit B will revisit).

use std::cell::RefCell;

use objc2::declare_class;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{msg_send_id, mutability, ClassType, DeclaredClass};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSControlTextEditingDelegate, NSOutlineView, NSOutlineViewDelegate,
    NSScrollView, NSTextField, NSView,
};
use objc2_foundation::{
    CGFloat, CGPoint, CGRect, CGSize, MainThreadMarker, NSNotification, NSString,
};

declare_class! {
    /// Flipped subclass of `NSView` used as the form's document view.
    /// Overrides `isFlipped` to return `YES`, which makes y=0 sit at
    /// the top of the view and rows grow downward — the same
    /// coordinate convention every other GUI uses, and the only way
    /// our absolute-positioned form rows render in the expected
    /// order without each `add_label` having to subtract from
    /// `document.frame().size.height`.
    pub(crate) struct FlippedView;

    unsafe impl ClassType for FlippedView {
        type Super = NSView;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "GmicPickerFormDocumentView";
    }

    impl DeclaredClass for FlippedView {}

    unsafe impl NSObjectProtocol for FlippedView {}

    unsafe impl FlippedView {
        #[method(isFlipped)]
        fn is_flipped(&self) -> bool {
            true
        }
    }
}

impl FlippedView {
    fn new_with_frame(mtm: MainThreadMarker, frame: CGRect) -> Retained<Self> {
        let this: Retained<Self> = unsafe { msg_send_id![mtm.alloc::<Self>(), initWithFrame: frame] };
        this
    }
}

use crate::catalogue::Filter;
use crate::ui::picker_catalogue_data_source::CatalogueDataSource;

/// Horizontal margin inside the form pane.
const FORM_HMARGIN: CGFloat = 14.0;
/// Top margin above the first row.
const FORM_TOP_MARGIN: CGFloat = 14.0;
/// Vertical gap between rows.
const FORM_ROW_GAP: CGFloat = 6.0;
/// Default row height for a single-line label.
const FORM_ROW_HEIGHT: CGFloat = 22.0;
/// Default row height reserved for the wrapping description row. The
/// wrap height is hard to compute without doing a layout pass; we
/// reserve a couple of lines and let the cell wrap inside.
const FORM_DESC_ROW_HEIGHT: CGFloat = 80.0;
/// Initial document-view width / height. We resize the document view
/// to match the clip view after every form rebuild so the content
/// fills the visible area.
const FORM_INITIAL_DOC_WIDTH: CGFloat = 400.0;
const FORM_INITIAL_DOC_HEIGHT: CGFloat = 600.0;

/// External handle to the form pane: the scroll view that gets added
/// to the right side of the split, plus the controller that owns it
/// and serves as the outline-view delegate.
///
/// Keep the [`FormController`] alive for the lifetime of the modal
/// session (its delegate registration is a weak reference, same
/// gotcha as the data source).
pub(crate) struct FormPane {
    pub controller: Retained<FormController>,
    pub root_view: Retained<NSScrollView>,
}

/// Build the form pane: an `NSScrollView` whose document view is a
/// flipped `NSView` we layout manually. Initial content is the
/// empty-selection placeholder.
pub(crate) fn build_form_pane(
    mtm: MainThreadMarker,
    data_source: Retained<CatalogueDataSource>,
) -> FormPane {
    // Outer scroll view. Size 0×0 is fine: the split view will resize
    // us on `addArrangedSubview` and every live divider drag.
    let scroll: Retained<NSScrollView> = unsafe {
        NSScrollView::initWithFrame(
            mtm.alloc(),
            CGRect {
                origin: CGPoint { x: 0.0, y: 0.0 },
                size: CGSize { width: 0.0, height: 0.0 },
            },
        )
    };
    unsafe {
        scroll.setHasVerticalScroller(true);
        scroll.setHasHorizontalScroller(false);
        scroll.setAutohidesScrollers(true);
        scroll.setDrawsBackground(false);
        let scroll_view: &NSView = &scroll;
        // Same scroll-perf tricks as the tree pane: layer-backed +
        // collapse subtree into one CALayer. The form is mostly static
        // text so this is pure compositing win.
        scroll_view.setWantsLayer(true);
        scroll_view.setCanDrawSubviewsIntoLayer(true);
    }

    // Document view: a FlippedView so y=0 is at the top and rows
    // grow downward like every other GUI in the universe. Width grows
    // with the scroll view; height is sized by the controller after
    // each rebuild based on how many rows the filter produced.
    let document = FlippedView::new_with_frame(
        mtm,
        CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: FORM_INITIAL_DOC_WIDTH,
                height: FORM_INITIAL_DOC_HEIGHT,
            },
        },
    );
    let document_as_view: Retained<NSView> = unsafe { Retained::cast(document.clone()) };
    unsafe {
        // Width-sizable: when the divider moves, the document grows
        // horizontally and our row layout follows the new width. We
        // do not autoresize the height — the controller sets it
        // explicitly so the scroll view knows how tall the content
        // really is.
        document_as_view.setAutoresizingMask(NSAutoresizingMaskOptions::NSViewWidthSizable);
        scroll.setDocumentView(Some(&document_as_view));
    }

    let controller = FormController::new(mtm, document_as_view, data_source);
    controller.show_empty_placeholder();

    FormPane {
        controller,
        root_view: scroll,
    }
}

/// Per-controller mutable state. Held inside the `declare_class!`
/// type via `MainThreadOnly` so the AppKit delegate callbacks can
/// borrow it without locks.
pub(crate) struct FormControllerIvars {
    /// The flipped document view we add rows into. We resize this on
    /// every rebuild so the enclosing scroll view's vertical
    /// scrollbar reflects the true content height.
    document: Retained<NSView>,
    /// Strong ref to the left-pane data source. Held so the form
    /// controller can call `resolve_filter` to translate an outline
    /// view item into a `&Filter` without having to re-implement the
    /// `NSNumber` intern-table decoding. There is no ownership cycle
    /// because the data source does not hold the controller.
    data_source: Retained<CatalogueDataSource>,
    /// Strong refs to every form-row view we currently display, so
    /// the controller (not just AppKit's view hierarchy) keeps them
    /// alive while the form is on screen.
    ///
    /// In sub-commit B this becomes `Vec<FormCell>` with typed value
    /// readers; for now we only need them retained.
    rows: RefCell<Vec<Retained<NSView>>>,
}

// `NSOutlineViewDelegate` inherits from `NSControlTextEditingDelegate`
// (which in turn inherits from `NSTextDelegate`, `NSObjectProtocol`).
// `declare_class!` registers each `unsafe impl Protocol for Self`
// inside the block as `class_addProtocol(...)` at class-load time. We
// only want one `class_addProtocol` call per Objective-C protocol —
// otherwise the runtime's "duplicate protocol" assertion fires the
// first time AppKit looks us up. So declare the super-protocols here,
// outside the macro, with bare `unsafe impl` blocks: Rust's type
// system is satisfied, but no runtime registration happens.
unsafe impl NSControlTextEditingDelegate for FormController {}

declare_class! {
    pub(crate) struct FormController;

    unsafe impl ClassType for FormController {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "GmicPickerFormController";
    }

    impl DeclaredClass for FormController {
        type Ivars = FormControllerIvars;
    }

    unsafe impl NSObjectProtocol for FormController {}

    unsafe impl NSOutlineViewDelegate for FormController {
        // `outlineViewSelectionDidChange:` — fires every time the
        // user (or our own programmatic `selectRowIndexes:` from
        // searching) changes the highlighted row.
        #[method(outlineViewSelectionDidChange:)]
        fn outline_view_selection_did_change(&self, notification: &NSNotification) {
            // Pull the outline view back out of the notification so
            // we can ask it for the selected row's item — that round
            // trip is the documented AppKit pattern.
            let outline: Option<Retained<NSOutlineView>> = unsafe {
                msg_send_id![notification, object]
            };
            let Some(outline) = outline else {
                crate::logging::log("form: selection notification without object");
                return;
            };
            let selected_row: isize = unsafe { outline.selectedRow() };
            if selected_row < 0 {
                self.show_empty_placeholder();
                return;
            }
            let item: Option<Retained<NSObject>> =
                unsafe { msg_send_id![&outline, itemAtRow: selected_row] };
            let Some(item) = item else {
                self.show_empty_placeholder();
                return;
            };
            let item_obj: &objc2::runtime::AnyObject = &item;
            match self.ivars().data_source.resolve_filter(item_obj) {
                Some(filter) => self.show_filter(filter),
                None => self.show_empty_placeholder(),
            }
        }
    }
}

impl FormController {
    pub(crate) fn new(
        mtm: MainThreadMarker,
        document: Retained<NSView>,
        data_source: Retained<CatalogueDataSource>,
    ) -> Retained<Self> {
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(FormControllerIvars {
            document,
            data_source,
            rows: RefCell::new(Vec::new()),
        });
        unsafe { msg_send_id![super(this), init] }
    }

    /// Expose the controller as an `NSOutlineViewDelegate` protocol
    /// object for `NSOutlineView::setDelegate:`.
    pub(crate) fn as_outline_view_delegate(
        &self,
    ) -> &ProtocolObject<dyn NSOutlineViewDelegate> {
        ProtocolObject::from_ref(self)
    }

    /// Replace the form contents with the empty-selection
    /// placeholder.
    pub(crate) fn show_empty_placeholder(&self) {
        let mtm = MainThreadMarker::from(&*self.ivars().document);
        self.clear_rows();
        let mut cursor_y = FORM_TOP_MARGIN;
        let width = self.row_width();
        cursor_y = self.add_label(
            mtm,
            "Select a filter on the left to see its parameters.",
            13.0,
            cursor_y,
            width,
            FORM_DESC_ROW_HEIGHT,
        );
        self.fit_document_height(cursor_y);
    }

    /// Replace the form contents with the selected filter's name,
    /// command, and description. Sub-commit B replaces this body
    /// with the full `ParamKind` → control mapping.
    fn show_filter(&self, filter: &Filter) {
        let mtm = MainThreadMarker::from(&*self.ivars().document);
        self.clear_rows();
        let width = self.row_width();
        let mut cursor_y = FORM_TOP_MARGIN;

        cursor_y = self.add_label(
            mtm,
            &filter.display_name,
            14.0,
            cursor_y,
            width,
            FORM_ROW_HEIGHT,
        );
        cursor_y = self.add_label(
            mtm,
            &format!("gmic command: {}", filter.command),
            11.0,
            cursor_y,
            width,
            FORM_ROW_HEIGHT,
        );

        if let Some(desc) = &filter.description {
            cursor_y =
                self.add_label(mtm, desc, 12.0, cursor_y, width, FORM_DESC_ROW_HEIGHT);
        }

        let param_count = filter.params.len();
        let params_summary = if param_count == 0 {
            "(no parameters)".to_string()
        } else {
            format!("{param_count} parameters — controls coming in T9-B")
        };
        cursor_y = self.add_label(
            mtm,
            &params_summary,
            11.0,
            cursor_y,
            width,
            FORM_ROW_HEIGHT,
        );

        self.fit_document_height(cursor_y);
    }

    fn clear_rows(&self) {
        let mut rows = self.ivars().rows.borrow_mut();
        for row in rows.drain(..) {
            unsafe { row.removeFromSuperview() };
        }
    }

    fn row_width(&self) -> CGFloat {
        let doc_width = self.ivars().document.frame().size.width;
        (doc_width - 2.0 * FORM_HMARGIN).max(60.0)
    }

    fn fit_document_height(&self, used_y: CGFloat) {
        // Grow / shrink the document view so the scroll view's
        // content size reflects the form's actual height. Adding a
        // small bottom pad keeps the last row clear of the scrolling
        // chrome.
        let new_height = used_y + FORM_TOP_MARGIN;
        let doc_frame = self.ivars().document.frame();
        let new_frame = CGRect {
            origin: doc_frame.origin,
            size: CGSize {
                width: doc_frame.size.width,
                height: new_height,
            },
        };
        unsafe { self.ivars().document.setFrame(new_frame) };
    }

    /// Add one row at (FORM_HMARGIN, y), height `h`, width `w`.
    /// Returns the next cursor-y (i.e. the y to use for the next row).
    fn add_label(
        &self,
        mtm: MainThreadMarker,
        text: &str,
        font_size: CGFloat,
        y: CGFloat,
        w: CGFloat,
        h: CGFloat,
    ) -> CGFloat {
        let frame = CGRect {
            origin: CGPoint { x: FORM_HMARGIN, y },
            size: CGSize { width: w, height: h },
        };
        let label = build_label(mtm, text, font_size, frame);
        unsafe {
            // Width-sizable so labels follow divider drags.
            label.setAutoresizingMask(NSAutoresizingMaskOptions::NSViewWidthSizable);
            self.ivars().document.addSubview(&label);
        }
        self.ivars().rows.borrow_mut().push(label);
        y + h + FORM_ROW_GAP
    }
}

/// Build a single label-style `NSTextField` with the given frame.
fn build_label(
    mtm: MainThreadMarker,
    text: &str,
    font_size: CGFloat,
    frame: CGRect,
) -> Retained<NSView> {
    let field: Retained<NSTextField> = unsafe { NSTextField::initWithFrame(mtm.alloc(), frame) };
    unsafe {
        field.setStringValue(&NSString::from_str(text));
        field.setEditable(false);
        field.setSelectable(true);
        field.setBezeled(false);
        field.setDrawsBackground(false);
        field.setBordered(false);
        // Wrap long descriptions instead of truncating with an
        // ellipsis: ParamKind::Note bodies can be 2-3 sentences.
        let cell: Option<Retained<objc2::runtime::AnyObject>> = msg_send_id![&field, cell];
        if let Some(cell) = cell {
            let _: () = objc2::msg_send![&cell, setWraps: true];
            let _: () = objc2::msg_send![&cell, setLineBreakMode: 0_isize /* byWordWrapping */];
        }
        let font: Retained<objc2::runtime::AnyObject> = objc2::msg_send_id![
            objc2::class!(NSFont),
            systemFontOfSize: font_size
        ];
        let _: () = objc2::msg_send![&field, setFont: &*font];
    }
    // Safe: NSTextField is a subclass of NSView; the cast just
    // narrows our static type to match the form-row vector.
    unsafe { Retained::cast(field) }
}
