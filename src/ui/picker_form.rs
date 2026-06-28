//! Right-pane form controller for the picker dialog (T9).
//!
//! Responsibilities:
//!
//! - Owns the right-pane root view (an `NSScrollView` wrapping a
//!   flipped [`FlippedView`] we lay out manually).
//! - Acts as the [`NSOutlineViewDelegate`] for the left-pane outline
//!   view: whenever the selection changes we rebuild the form to
//!   match the newly-selected filter.
//! - Builds one row per [`crate::catalogue::Param`] using a control
//!   appropriate to the [`crate::catalogue::ParamKind`].
//! - Retains every interactive control in a `Vec<FormCell>` so T10
//!   can read every control's current value back into the
//!   gmic-arg `Vec<String>` when the user clicks OK.
//!
//! We layout rows by absolute frame instead of via `NSStackView` +
//! Auto Layout because the latter requires extra `objc2-app-kit`
//! features and interacts awkwardly with our scroll view. A flipped
//! `NSView` document with manual `(x, y, w, h)` placement is enough
//! for this much.

use std::cell::{Cell, OnceCell, RefCell};

use objc2::declare_class;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{msg_send_id, mutability, sel, ClassType, DeclaredClass};
use objc2_app_kit::{
    NSBox, NSBoxType, NSButton, NSButtonType, NSColor, NSColorWell, NSControlStateValueOff,
    NSControlStateValueOn, NSControlTextEditingDelegate, NSOutlineView, NSOutlineViewDelegate,
    NSPopUpButton, NSScrollView, NSSlider, NSTextField, NSView, NSViewFrameDidChangeNotification,
};
use objc2_foundation::{
    CGFloat, CGPoint, CGRect, CGSize, MainThreadMarker, NSNotification, NSNotificationCenter,
    NSString,
};

use crate::catalogue::{Filter, Param, ParamKind};
use crate::previews::format_float;
use crate::ui::picker_catalogue_data_source::CatalogueDataSource;

/// Horizontal margin inside the form pane.
const FORM_HMARGIN: CGFloat = 14.0;
/// Top margin above the first row.
const FORM_TOP_MARGIN: CGFloat = 14.0;
/// Vertical gap between rows.
const FORM_ROW_GAP: CGFloat = 6.0;
/// Default row height for a single-line label + control. Wrapping
/// label rows (descriptions, notes) compute their own height via
/// `cellSizeForBounds:` and ignore this minimum once they exceed it.
const FORM_ROW_HEIGHT: CGFloat = 22.0;
/// Width of the label column on a "label + control" row. Anything
/// wider feels awkward at the default pane width; wider labels just
/// wrap inside this column.
const FORM_LABEL_WIDTH: CGFloat = 130.0;
/// Gap between the label column and the control column.
const FORM_LABEL_GAP: CGFloat = 8.0;
/// Initial document-view height. The width is *always* slaved to the
/// enclosing clip view's bounds width — we never use AppKit's
/// `NSViewWidthSizable` autoresizing on the document because it is
/// delta-based and starts from the (0, 0) clip-view bounds the scroll
/// view is built with, so the first split-view resize would grow the
/// document by the entire pane width (e.g. 400 + 600 = 1000) and
/// push every control off the right edge of the visible area. We
/// instead listen for `NSViewFrameDidChangeNotification` on the clip
/// view and rebuild the form deterministically against the new
/// width. Height stays zero until the first build computes the real
/// stack height.
const FORM_INITIAL_DOC_HEIGHT: CGFloat = 0.0;

declare_class! {
    /// Flipped subclass of `NSView` used as the form's document view.
    /// Overrides `isFlipped` to return `YES`, which makes y=0 sit at
    /// the top of the view and rows grow downward — the same
    /// coordinate convention every other GUI uses, and the only way
    /// our absolute-positioned form rows render in the expected
    /// order without each `add_*` having to subtract from
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
        let this: Retained<Self> =
            unsafe { msg_send_id![mtm.alloc::<Self>(), initWithFrame: frame] };
        this
    }
}

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
    remembered: std::collections::BTreeMap<String, Vec<String>>,
) -> FormPane {
    // Outer scroll view. Size 0×0 is fine: the split view will resize
    // us as soon as `adjustSubviews` runs in `build_split_view`.
    let scroll: Retained<NSScrollView> = unsafe {
        NSScrollView::initWithFrame(
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
    unsafe {
        scroll.setHasVerticalScroller(true);
        scroll.setHasHorizontalScroller(false);
        scroll.setAutohidesScrollers(true);
        scroll.setDrawsBackground(false);
        let scroll_view: &NSView = &scroll;
        // Same scroll-perf tricks as the tree pane: layer-backed +
        // collapse subtree into one CALayer.
        scroll_view.setWantsLayer(true);
        scroll_view.setCanDrawSubviewsIntoLayer(true);
    }

    let document = FlippedView::new_with_frame(
        mtm,
        CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: 0.0,
                height: FORM_INITIAL_DOC_HEIGHT,
            },
        },
    );
    let document_as_view: Retained<NSView> = unsafe { Retained::cast(document.clone()) };
    // Intentionally no `setAutoresizingMask` on the document view —
    // see the FORM_INITIAL_DOC_HEIGHT comment. The clip view's
    // frame-changed notification drives every resize instead.
    unsafe {
        scroll.setDocumentView(Some(&document_as_view));
    }

    let controller = FormController::new(mtm, document_as_view, data_source, remembered);

    // Subscribe to the clip view's frame-changed notification so we
    // can keep the document width in lockstep with the visible pane
    // and rebuild the parameter rows. `setPostsFrameChangedNotifications`
    // defaults to YES on NSClipView but we set it explicitly here to
    // be defensive against future AppKit changes.
    unsafe {
        let clip = scroll.contentView();
        let clip_as_view: &NSView = &clip;
        clip_as_view.setPostsFrameChangedNotifications(true);
        let center = NSNotificationCenter::defaultCenter();
        center.addObserver_selector_name_object(
            &controller,
            sel!(clipViewFrameDidChange:),
            Some(NSViewFrameDidChangeNotification),
            Some(clip_as_view),
        );
    }

    controller.show_empty_placeholder();

    FormPane {
        controller,
        root_view: scroll,
    }
}

/// One row's interactive (or non-interactive) state, stored on the
/// form controller so T10 can read the user's final values back as
/// CLI args when OK is clicked.
///
/// Non-interactive params (notes, separators, links, unknowns) use
/// the `Static` variant: they contribute no arg and the controller
/// keeps strong refs to their views via [`FormControllerIvars::extra_views`].
#[allow(dead_code)] // T10 will read these fields back when collecting args
pub(crate) enum FormCell {
    /// Integer scalar; `slider.intValue()` is the chosen value.
    Int {
        slider: Retained<NSSlider>,
        min: i64,
        max: i64,
    },
    /// Float scalar; `slider.doubleValue()` is the chosen value.
    Float {
        slider: Retained<NSSlider>,
        min: f64,
        max: f64,
    },
    /// Bool; `button.state() == NSControlStateValueOn`.
    Bool { button: Retained<NSButton> },
    /// Choice index into `choices`; `popup.indexOfSelectedItem()`.
    Choice {
        popup: Retained<NSPopUpButton>,
        choices: Vec<String>,
    },
    /// sRGB colour (alpha discarded). `well.color()` returns the
    /// NSColor we then decompose into 0..=255 bytes.
    Color { well: Retained<NSColorWell> },
    /// Free text; `field.stringValue()` is the value.
    Text { field: Retained<NSTextField> },
    /// Non-interactive cell (note, separator, link, unknown). The
    /// view itself is held by [`FormControllerIvars::extra_views`].
    Static,
    /// Hidden parameter whose user-visible row is just a read-only
    /// label, but which still contributes its `default` verbatim to
    /// the gmic argv (for `value(...)`, `button(...)`, etc.).
    /// Stored, not rendered, so a filter with eight `value(0)`
    /// internals doesn't crowd the form with eight identical rows.
    Internal { default: String },
}

#[derive(Clone, Copy)]
struct RowLayout {
    width: CGFloat,
    cell_x: CGFloat,
    cell_w: CGFloat,
    y: CGFloat,
}

enum RowOutcome {
    Rendered { cell: FormCell, row_height: CGFloat },
    Hidden(FormCell),
}

#[derive(Clone, Copy)]
struct IntRowSpec {
    default: i64,
    min: i64,
    max: i64,
}

#[derive(Clone, Copy)]
struct FloatRowSpec {
    default: f64,
    min: f64,
    max: f64,
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
    /// One cell per [`Param`] of the currently-selected filter, in
    /// declaration order. Used by T10 to read the chosen values back
    /// into the gmic arg list.
    cells: RefCell<Vec<FormCell>>,
    /// Strong refs to the *non-cell* views attached to the document
    /// (headers, label fields, separator boxes, note text fields,
    /// the placeholder). These are not part of `cells` but must
    /// outlive AppKit's superview retain to avoid a use-after-free
    /// when AppKit re-renders.
    extra_views: RefCell<Vec<Retained<NSView>>>,
    /// Back-reference to the actions controller (T10) so the
    /// selection-changed delegate can flip the OK button's enabled
    /// state in addition to rebuilding the form. Lazy-set right
    /// after the actions controller is built — they reference each
    /// other, hence the [`OnceCell`].
    actions: OnceCell<Retained<crate::ui::picker_actions::PickerActions>>,
    /// Remembered-args lookup used when the user clicks a new
    /// filter: the saved `remembered_args[command]` becomes the
    /// initial values for the form rather than the hard-coded
    /// kind defaults. Captured at panel-open time so live mutations
    /// during the session don't surprise the user; the [`RefCell`]
    /// allows `show_filter_with_prefill` to fold an explicit
    /// per-call prefill into the lookup so subsequent relayouts
    /// (driven by clip-view frame-change notifications, which can't
    /// see the original per-call prefill argument) still rebuild
    /// the form with the user's saved values rather than stdlib
    /// defaults.
    remembered: RefCell<std::collections::BTreeMap<String, Vec<String>>>,
    /// Raw pointer to the currently-displayed filter (or `None` for
    /// the empty placeholder). Used by the clip-view frame-change
    /// handler to rebuild the same form against a new width without
    /// the controller needing a back-channel into the outline view.
    /// Safe to store as a raw pointer because every `Filter` we get
    /// here comes from the `'static` `Catalogue` interned in
    /// [`crate::catalogue::builtin`].
    current_filter: Cell<Option<*const Filter>>,
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

    unsafe impl FormController {
        // Notification handler: fires when the scroll view's clip
        // view changes size (window resize, split-divider drag,
        // initial layout pass). We snap the document view's width to
        // the new clip width and re-lay the rows against it so
        // sliders / popups stay flush with the visible pane edge.
        #[method(clipViewFrameDidChange:)]
        fn clip_view_frame_did_change(&self, _notification: &NSNotification) {
            self.relayout_for_current_width();
        }
    }

    unsafe impl NSOutlineViewDelegate for FormController {
        // `outlineViewSelectionDidChange:` — fires every time the
        // user (or our own programmatic `selectRowIndexes:` from
        // searching) changes the highlighted row.
        #[method(outlineViewSelectionDidChange:)]
        fn outline_view_selection_did_change(&self, notification: &NSNotification) {
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
                Some(filter) => {
                    // `render_filter` already reads the remembered map
                    // itself; calling it through `show_filter_with_prefill`
                    // with `None` keeps `current_filter` updated and
                    // routes through the single source of truth for
                    // prefill values.
                    self.show_filter_with_prefill(filter, None);
                }
                None => self.show_empty_placeholder(),
            }
            if let Some(actions) = self.ivars().actions.get() {
                actions.refresh_ok_enabled();
            }
        }
    }
}

impl FormController {
    pub(crate) fn new(
        mtm: MainThreadMarker,
        document: Retained<NSView>,
        data_source: Retained<CatalogueDataSource>,
        remembered: std::collections::BTreeMap<String, Vec<String>>,
    ) -> Retained<Self> {
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(FormControllerIvars {
            document,
            data_source,
            cells: RefCell::new(Vec::new()),
            extra_views: RefCell::new(Vec::new()),
            actions: OnceCell::new(),
            remembered: RefCell::new(remembered),
            current_filter: Cell::new(None),
        });
        unsafe { msg_send_id![super(this), init] }
    }

    /// Wire the back-reference to the [`PickerActions`] controller.
    /// Called once, after both objects have been constructed; the
    /// `OnceCell` semantics prevent surprise re-registration.
    pub(crate) fn set_actions(&self, actions: Retained<crate::ui::picker_actions::PickerActions>) {
        let _ = self.ivars().actions.set(actions);
    }

    /// Expose the controller as an `NSOutlineViewDelegate` protocol
    /// object for `NSOutlineView::setDelegate:`.
    pub(crate) fn as_outline_view_delegate(&self) -> &ProtocolObject<dyn NSOutlineViewDelegate> {
        ProtocolObject::from_ref(self)
    }

    /// Walk the current rows and produce one CLI string per *interactive*
    /// parameter, in declaration order. Non-interactive cells (notes,
    /// separators, links, unknowns) contribute nothing — they don't
    /// take a value in gmic's argv either.
    ///
    /// Called by the OK button's action handler in T10.
    pub(crate) fn collect_values(&self) -> Vec<String> {
        let cells = self.ivars().cells.borrow();
        cells
            .iter()
            .filter_map(|cell| match cell {
                FormCell::Int { slider, min, max } => {
                    let v = unsafe { slider.doubleValue() }.round() as i64;
                    Some(v.clamp(*min, *max).to_string())
                }
                FormCell::Float { slider, min, max } => {
                    let v = unsafe { slider.doubleValue() }.clamp(*min, *max);
                    Some(format_float(v))
                }
                FormCell::Bool { button } => {
                    let on = unsafe { button.state() } == NSControlStateValueOn;
                    Some(if on { "1" } else { "0" }.to_string())
                }
                FormCell::Choice { popup, .. } => {
                    let idx = unsafe { popup.indexOfSelectedItem() }.max(0);
                    Some(idx.to_string())
                }
                FormCell::Color { well } => {
                    let raw = unsafe { well.color() };
                    // NSColorWell can return a colour in *any* colour
                    // space; component accessors panic outside RGB
                    // family. Convert to sRGB first so a wide-gamut
                    // picker doesn't crash us at OK time.
                    let rgb = unsafe {
                        let cs: Retained<objc2::runtime::AnyObject> =
                            objc2::msg_send_id![objc2::class!(NSColorSpace), sRGBColorSpace];
                        let converted: Option<Retained<NSColor>> =
                            objc2::msg_send_id![&raw, colorUsingColorSpace: &*cs];
                        converted.unwrap_or(raw)
                    };
                    let r = unsafe { rgb.redComponent() };
                    let g = unsafe { rgb.greenComponent() };
                    let b = unsafe { rgb.blueComponent() };
                    let to_byte =
                        |c: CGFloat| -> u8 { (c * 255.0).round().clamp(0.0, 255.0) as u8 };
                    Some(format!("{},{},{}", to_byte(r), to_byte(g), to_byte(b)))
                }
                FormCell::Text { field } => {
                    let s = unsafe { field.stringValue() };
                    Some(s.to_string())
                }
                FormCell::Static => None,
                // Internal params are not user-editable but still need
                // to appear in argv at their declared position, so we
                // emit the stored default verbatim. This is how
                // `value(0)` / `button(2)` filter declarations stay
                // round-trippable through the picker even though no
                // NSControl is bound to them.
                FormCell::Internal { default } => Some(default.clone()),
            })
            .collect()
    }

    /// Replace the form contents with the empty-selection
    /// placeholder.
    pub(crate) fn show_empty_placeholder(&self) {
        self.ivars().current_filter.set(None);
        self.render_empty_placeholder();
    }

    /// Replace the form contents with the selected filter's name,
    /// description, and one row per [`Param`] using a control that
    /// matches the [`ParamKind`]. Uses each ParamKind's compiled-in
    /// defaults for the row values.
    pub(crate) fn show_filter(&self, filter: &Filter) {
        self.show_filter_with_prefill(filter, None);
    }

    /// Like [`show_filter`] but with optional per-param starting
    /// values. The slice indexes into `filter.params` and is fed
    /// through [`reconcile`] before display, so a stale or partial
    /// vector falls back to defaults per-row without misaligning
    /// the form when a non-interactive param sits between two
    /// interactive ones.
    pub(crate) fn show_filter_with_prefill(&self, filter: &Filter, prefill: Option<&[String]>) {
        // Remember the filter so clip-view frame-change notifications
        // can rebuild it against a new width. Safe to store as a raw
        // pointer because every `Filter` comes from the `'static`
        // built-in catalogue.
        self.ivars()
            .current_filter
            .set(Some(filter as *const Filter));
        // Persist remembered prefill *into* the lookup map so that a
        // later relayout (driven by the clip view's frame-change
        // notification, which doesn't know about the per-call prefill)
        // still uses the correct starting values rather than falling
        // back to stdlib defaults.
        if let Some(values) = prefill {
            self.ivars()
                .remembered
                .borrow_mut()
                .insert(filter.command.clone(), values.to_vec());
        }
        self.render_filter(filter);
    }

    /// Snap the document view's width to the current clip-view width
    /// and re-lay the form rows. Called from the clip-view frame-
    /// change handler so resizes (initial layout, window drag, split
    /// divider drag) keep every control inside the visible pane.
    fn relayout_for_current_width(&self) {
        let clip_width = self.clip_view_width();
        let doc = &self.ivars().document;
        let doc_frame = doc.frame();
        // Width must always be >= some minimum so `row_width` doesn't
        // saturate at the 60px clamp; height keeps whatever the last
        // `fit_document_height` set so the scroll view's thumb stays
        // put across the relayout.
        let new_width = clip_width.max(0.0);
        if (doc_frame.size.width - new_width).abs() > 0.5 {
            let new_frame = CGRect {
                origin: doc_frame.origin,
                size: CGSize {
                    width: new_width,
                    height: doc_frame.size.height,
                },
            };
            unsafe { doc.setFrame(new_frame) };
        }
        match self.ivars().current_filter.get() {
            Some(p) => {
                let filter: &Filter = unsafe { &*p };
                self.render_filter(filter);
            }
            None => self.render_empty_placeholder(),
        }
    }

    /// Width of the document view's superview (the scroll view's
    /// `NSClipView`). Returns 0 when the document hasn't been added
    /// yet (defensive: callers clamp with `.max(MIN)`).
    fn clip_view_width(&self) -> CGFloat {
        let doc = &self.ivars().document;
        let superview: Option<Retained<NSView>> = unsafe { doc.superview() };
        match superview {
            Some(clip) => clip.frame().size.width,
            None => 0.0,
        }
    }

    fn render_empty_placeholder(&self) {
        let mtm = MainThreadMarker::from(&*self.ivars().document);
        self.clear_rows();
        let width = self.row_width();
        let mut cursor_y = FORM_TOP_MARGIN;
        let h = self.add_label(
            mtm,
            "Select a filter on the left to see its parameters.",
            13.0,
            FORM_HMARGIN,
            cursor_y,
            width,
        );
        cursor_y += h + FORM_ROW_GAP;
        self.fit_document_height(cursor_y);
    }

    /// Internal entry point used by both the user-driven
    /// `show_filter_with_prefill` and the clip-view frame-change
    /// relayout. Reads the prefill from the remembered map so both
    /// callers go through one code path.
    fn render_filter(&self, filter: &Filter) {
        let mtm = MainThreadMarker::from(&*self.ivars().document);
        self.clear_rows();
        let width = self.row_width();
        let mut cursor_y = FORM_TOP_MARGIN;
        // Snapshot the remembered prefill so the borrow on the ivars
        // map doesn't outlast any of the `self.add_*` calls below
        // (which themselves borrow other ivars).
        let prefill: Option<Vec<String>> = self
            .ivars()
            .remembered
            .borrow()
            .get(&filter.command)
            .cloned();

        let h = self.add_label(
            mtm,
            &filter.display_name,
            14.0,
            FORM_HMARGIN,
            cursor_y,
            width,
        );
        cursor_y += h + FORM_ROW_GAP;
        let h = self.add_label(
            mtm,
            &format!("gmic command: {}", filter.command),
            11.0,
            FORM_HMARGIN,
            cursor_y,
            width,
        );
        cursor_y += h + FORM_ROW_GAP;

        if let Some(desc) = &filter.description {
            if !desc.trim().is_empty() {
                let h = self.add_label(mtm, desc, 12.0, FORM_HMARGIN, cursor_y, width);
                cursor_y += h + FORM_ROW_GAP;
            }
        }

        cursor_y += FORM_ROW_GAP;

        // Reconcile the prefill against the current param list. Out-
        // of-range or wrong-type entries silently fall back to the
        // kind's default — same contract as `reconcile::reconcile`.
        let reconciled: Vec<String> = match prefill {
            Some(saved) => crate::catalogue::reconcile::reconcile(&saved, &filter.params),
            None => Vec::new(),
        };
        let starting = if reconciled.is_empty() {
            None
        } else {
            Some(reconciled.as_slice())
        };

        if filter.params.is_empty() {
            let h = self.add_label(mtm, "(no parameters)", 11.0, FORM_HMARGIN, cursor_y, width);
            cursor_y += h + FORM_ROW_GAP;
        } else {
            for (i, param) in filter.params.iter().enumerate() {
                let start = starting.and_then(|s| s.get(i)).map(String::as_str);
                cursor_y = self.add_param_row(mtm, param, cursor_y, start);
            }
        }

        self.fit_document_height(cursor_y);
    }

    /// Add one row for a single [`Param`]. Returns the new cursor-y.
    /// Always records exactly one entry in `cells` so the index
    /// matches the param index — even for non-interactive kinds
    /// (we store a `FormCell::Static` for those).
    fn add_param_row(
        &self,
        mtm: MainThreadMarker,
        param: &Param,
        y: CGFloat,
        prefill: Option<&str>,
    ) -> CGFloat {
        let width = self.row_width();
        let (cell_x, cell_w) = self.control_column(width);
        let layout = RowLayout {
            width,
            cell_x,
            cell_w,
            y,
        };
        let outcome = self.render_param_row(mtm, param, layout, prefill);

        match outcome {
            RowOutcome::Rendered { cell, row_height } => {
                self.ivars().cells.borrow_mut().push(cell);
                y + row_height + FORM_ROW_GAP
            }
            RowOutcome::Hidden(cell) => {
                self.ivars().cells.borrow_mut().push(cell);
                y
            }
        }
    }

    fn render_param_row(
        &self,
        mtm: MainThreadMarker,
        param: &Param,
        layout: RowLayout,
        prefill: Option<&str>,
    ) -> RowOutcome {
        match &param.kind {
            ParamKind::Int { default, min, max } => self.add_int_row(
                mtm,
                &param.label,
                layout,
                prefill,
                IntRowSpec {
                    default: *default,
                    min: *min,
                    max: *max,
                },
            ),
            ParamKind::Float { default, min, max } => self.add_float_row(
                mtm,
                &param.label,
                layout,
                prefill,
                FloatRowSpec {
                    default: *default,
                    min: *min,
                    max: *max,
                },
            ),
            ParamKind::Bool { default } => {
                self.add_bool_row(mtm, &param.label, layout, prefill, *default)
            }
            ParamKind::Choice { choices, default } => {
                self.add_choice_row(mtm, &param.label, layout, prefill, choices, *default)
            }
            ParamKind::Color { default_rgb } => {
                self.add_color_row(mtm, &param.label, layout, prefill, *default_rgb)
            }
            ParamKind::Text { default } => {
                self.add_text_row(mtm, &param.label, layout, prefill, default)
            }
            ParamKind::Note(body) => self.add_note_row(mtm, layout, body),
            ParamKind::Separator => self.add_separator_row(mtm, layout),
            ParamKind::Link { label, url } => self.add_link_row(mtm, layout, label, url),
            ParamKind::Internal { label: _, default } => {
                self.add_internal_row(mtm, param, layout, default)
            }
            ParamKind::Unknown(raw) => self.add_unknown_row(mtm, param, layout, raw),
        }
    }

    fn add_int_row(
        &self,
        mtm: MainThreadMarker,
        label: &str,
        layout: RowLayout,
        prefill: Option<&str>,
        spec: IntRowSpec,
    ) -> RowOutcome {
        let label_h = self.add_control_label(mtm, label, layout);
        let starting = prefill
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(spec.default);
        let slider = self.add_slider(
            mtm,
            layout.cell_x,
            layout.y,
            layout.cell_w,
            spec.min as f64,
            spec.max as f64,
            starting as f64,
            /* integer = */ true,
        );
        RowOutcome::Rendered {
            cell: FormCell::Int {
                slider,
                min: spec.min,
                max: spec.max,
            },
            row_height: label_h.max(FORM_ROW_HEIGHT),
        }
    }

    fn add_float_row(
        &self,
        mtm: MainThreadMarker,
        label: &str,
        layout: RowLayout,
        prefill: Option<&str>,
        spec: FloatRowSpec,
    ) -> RowOutcome {
        let label_h = self.add_control_label(mtm, label, layout);
        let starting = prefill
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(spec.default);
        let slider = self.add_slider(
            mtm,
            layout.cell_x,
            layout.y,
            layout.cell_w,
            spec.min,
            spec.max,
            starting,
            /* integer = */ false,
        );
        RowOutcome::Rendered {
            cell: FormCell::Float {
                slider,
                min: spec.min,
                max: spec.max,
            },
            row_height: label_h.max(FORM_ROW_HEIGHT),
        }
    }

    fn add_bool_row(
        &self,
        mtm: MainThreadMarker,
        label: &str,
        layout: RowLayout,
        prefill: Option<&str>,
        default: bool,
    ) -> RowOutcome {
        // Bool gets a checkbox with the param label as its title; no
        // separate left-side label.
        let starting = match prefill {
            Some("1") | Some("true") => true,
            Some("0") | Some("false") => false,
            _ => default,
        };
        let button = self.add_checkbox(mtm, label, starting, FORM_HMARGIN, layout.y, layout.width);
        RowOutcome::Rendered {
            cell: FormCell::Bool { button },
            row_height: FORM_ROW_HEIGHT,
        }
    }

    fn add_choice_row(
        &self,
        mtm: MainThreadMarker,
        label: &str,
        layout: RowLayout,
        prefill: Option<&str>,
        choices: &[String],
        default: usize,
    ) -> RowOutcome {
        let label_h = self.add_control_label(mtm, label, layout);
        let starting = choice_starting_index(prefill, choices, default);
        let popup = self.add_popup(
            mtm,
            layout.cell_x,
            layout.y,
            layout.cell_w,
            choices,
            starting,
        );
        RowOutcome::Rendered {
            cell: FormCell::Choice {
                popup,
                choices: choices.to_vec(),
            },
            row_height: label_h.max(FORM_ROW_HEIGHT),
        }
    }

    fn add_color_row(
        &self,
        mtm: MainThreadMarker,
        label: &str,
        layout: RowLayout,
        prefill: Option<&str>,
        default_rgb: [u8; 3],
    ) -> RowOutcome {
        let label_h = self.add_control_label(mtm, label, layout);
        let starting = prefill.and_then(parse_rgb_triple).unwrap_or(default_rgb);
        let well = self.add_color_well(mtm, layout.cell_x, layout.y, layout.cell_w, starting);
        RowOutcome::Rendered {
            cell: FormCell::Color { well },
            row_height: label_h.max(FORM_ROW_HEIGHT),
        }
    }

    fn add_text_row(
        &self,
        mtm: MainThreadMarker,
        label: &str,
        layout: RowLayout,
        prefill: Option<&str>,
        default: &str,
    ) -> RowOutcome {
        let label_h = self.add_control_label(mtm, label, layout);
        let starting = prefill.unwrap_or(default);
        let field = self.add_textfield(mtm, layout.cell_x, layout.y, layout.cell_w, starting);
        RowOutcome::Rendered {
            cell: FormCell::Text { field },
            row_height: label_h.max(FORM_ROW_HEIGHT),
        }
    }

    fn add_note_row(&self, mtm: MainThreadMarker, layout: RowLayout, body: &str) -> RowOutcome {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            // Don't waste a row on a `note("")` placeholder.
            return RowOutcome::Hidden(FormCell::Static);
        }
        let row_height = self.add_label(mtm, trimmed, 11.0, FORM_HMARGIN, layout.y, layout.width);
        RowOutcome::Rendered {
            cell: FormCell::Static,
            row_height,
        }
    }

    fn add_separator_row(&self, mtm: MainThreadMarker, layout: RowLayout) -> RowOutcome {
        self.add_separator(mtm, FORM_HMARGIN, layout.y, layout.width);
        RowOutcome::Rendered {
            cell: FormCell::Static,
            row_height: 12.0,
        }
    }

    fn add_link_row(
        &self,
        mtm: MainThreadMarker,
        layout: RowLayout,
        label: &str,
        url: &str,
    ) -> RowOutcome {
        let display = if url.is_empty() {
            label.to_owned()
        } else {
            format!("{label}  ({url})")
        };
        let row_height = self.add_label(mtm, &display, 12.0, FORM_HMARGIN, layout.y, layout.width);
        RowOutcome::Rendered {
            cell: FormCell::Static,
            row_height,
        }
    }

    fn add_internal_row(
        &self,
        mtm: MainThreadMarker,
        param: &Param,
        layout: RowLayout,
        default: &str,
    ) -> RowOutcome {
        // Hidden internal decls still contribute their default to argv,
        // but empty labels should not crowd the form.
        let user_label = param.label.trim();
        let cell = FormCell::Internal {
            default: default.to_owned(),
        };
        if user_label.is_empty() {
            return RowOutcome::Hidden(cell);
        }

        let label_h = self.add_control_label(mtm, user_label, layout);
        let display = format!("(internal: {default})");
        let value_h = self.add_label(mtm, &display, 11.0, layout.cell_x, layout.y, layout.cell_w);
        RowOutcome::Rendered {
            cell,
            row_height: label_h.max(value_h).max(FORM_ROW_HEIGHT),
        }
    }

    fn add_unknown_row(
        &self,
        mtm: MainThreadMarker,
        param: &Param,
        layout: RowLayout,
        raw: &str,
    ) -> RowOutcome {
        let label = param.label.trim();
        if label.is_empty() {
            return RowOutcome::Hidden(FormCell::Static);
        }

        let label_h = self.add_control_label(mtm, label, layout);
        let display = format!("(unsupported: {})", raw.trim());
        let value_h = self.add_label(mtm, &display, 11.0, layout.cell_x, layout.y, layout.cell_w);
        RowOutcome::Rendered {
            cell: FormCell::Static,
            row_height: label_h.max(value_h).max(FORM_ROW_HEIGHT),
        }
    }

    fn add_control_label(&self, mtm: MainThreadMarker, label: &str, layout: RowLayout) -> CGFloat {
        self.add_label(mtm, label, 12.0, FORM_HMARGIN, layout.y, FORM_LABEL_WIDTH)
    }

    fn clear_rows(&self) {
        // Detach every retained view from the document so the next
        // rebuild starts with a clean slate. We hold strong refs in
        // `cells` (via the controls) and in `extra_views` (labels,
        // notes, separators); both need explicit detach because
        // AppKit only removes them when they're released.
        for cell in self.ivars().cells.borrow_mut().drain(..) {
            match cell {
                FormCell::Int { slider, .. } | FormCell::Float { slider, .. } => unsafe {
                    let v: &NSView = &slider;
                    v.removeFromSuperview();
                },
                FormCell::Bool { button } => unsafe {
                    let v: &NSView = &button;
                    v.removeFromSuperview();
                },
                FormCell::Choice { popup, .. } => unsafe {
                    let v: &NSView = &popup;
                    v.removeFromSuperview();
                },
                FormCell::Color { well } => unsafe {
                    let v: &NSView = &well;
                    v.removeFromSuperview();
                },
                FormCell::Text { field } => unsafe {
                    let v: &NSView = &field;
                    v.removeFromSuperview();
                },
                FormCell::Static | FormCell::Internal { .. } => {}
            }
        }
        for v in self.ivars().extra_views.borrow_mut().drain(..) {
            unsafe { v.removeFromSuperview() };
        }
    }

    fn row_width(&self) -> CGFloat {
        let doc_width = self.ivars().document.frame().size.width;
        (doc_width - 2.0 * FORM_HMARGIN).max(60.0)
    }

    /// Returns (cell_x, cell_w) for a "label + control" row given the
    /// total row width.
    fn control_column(&self, row_width: CGFloat) -> (CGFloat, CGFloat) {
        let cell_x = FORM_HMARGIN + FORM_LABEL_WIDTH + FORM_LABEL_GAP;
        let cell_w = (row_width - FORM_LABEL_WIDTH - FORM_LABEL_GAP).max(60.0);
        (cell_x, cell_w)
    }

    fn fit_document_height(&self, used_y: CGFloat) {
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

    /// Add one wrapping label at `(x, y)` with width `w` and a
    /// measured (auto-sized) height. Returns the height the label
    /// actually occupied so the caller can advance `cursor_y`
    /// correctly. Caller is responsible for adding `FORM_ROW_GAP`.
    fn add_label(
        &self,
        mtm: MainThreadMarker,
        text: &str,
        font_size: CGFloat,
        x: CGFloat,
        y: CGFloat,
        w: CGFloat,
    ) -> CGFloat {
        let initial_frame = CGRect {
            origin: CGPoint { x, y },
            size: CGSize {
                width: w,
                height: FORM_ROW_HEIGHT,
            },
        };
        let (label, height) = build_label_view(mtm, text, font_size, initial_frame, w);
        unsafe {
            // No autoresizing mask: the form is fully rebuilt by
            // `relayout_for_current_width` whenever the clip view's
            // frame changes, so the label's frame is always set
            // explicitly relative to the live `row_width()`.
            self.ivars().document.addSubview(&label);
        }
        self.ivars().extra_views.borrow_mut().push(label);
        height
    }

    #[allow(clippy::too_many_arguments)]
    fn add_slider(
        &self,
        mtm: MainThreadMarker,
        x: CGFloat,
        y: CGFloat,
        w: CGFloat,
        min: f64,
        max: f64,
        default: f64,
        integer: bool,
    ) -> Retained<NSSlider> {
        let frame = CGRect {
            origin: CGPoint { x, y },
            size: CGSize {
                width: w,
                height: FORM_ROW_HEIGHT,
            },
        };
        let slider: Retained<NSSlider> = unsafe { NSSlider::initWithFrame(mtm.alloc(), frame) };
        unsafe {
            // No autoresizing mask: see `add_label` for the rationale.
            slider.setMinValue(min);
            slider.setMaxValue(max);
            slider.setDoubleValue(default.clamp(min, max));
            if integer {
                // Ticks at every integer step keep the slider snapping
                // to whole values; required for ParamKind::Int.
                let count = ((max - min).round() as i64).max(1) + 1;
                slider.setNumberOfTickMarks(count as isize);
                slider.setAllowsTickMarkValuesOnly(true);
            }
            let v: &NSView = &slider;
            self.ivars().document.addSubview(v);
        }
        slider
    }

    fn add_checkbox(
        &self,
        mtm: MainThreadMarker,
        title: &str,
        default: bool,
        x: CGFloat,
        y: CGFloat,
        w: CGFloat,
    ) -> Retained<NSButton> {
        let frame = CGRect {
            origin: CGPoint { x, y },
            size: CGSize {
                width: w,
                height: FORM_ROW_HEIGHT,
            },
        };
        let button: Retained<NSButton> = unsafe { NSButton::initWithFrame(mtm.alloc(), frame) };
        unsafe {
            button.setButtonType(NSButtonType::Switch);
            button.setTitle(&NSString::from_str(title));
            button.setState(if default {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            let v: &NSView = &button;
            self.ivars().document.addSubview(v);
        }
        button
    }

    fn add_popup(
        &self,
        mtm: MainThreadMarker,
        x: CGFloat,
        y: CGFloat,
        w: CGFloat,
        choices: &[String],
        default: usize,
    ) -> Retained<NSPopUpButton> {
        let frame = CGRect {
            origin: CGPoint { x, y },
            size: CGSize {
                width: w,
                height: FORM_ROW_HEIGHT + 4.0,
            },
        };
        let popup: Retained<NSPopUpButton> =
            unsafe { NSPopUpButton::initWithFrame_pullsDown(mtm.alloc(), frame, false) };
        unsafe {
            for choice in choices {
                popup.addItemWithTitle(&NSString::from_str(choice));
            }
            if !choices.is_empty() {
                let idx = default.min(choices.len() - 1) as isize;
                popup.selectItemAtIndex(idx);
            }
            let v: &NSView = &popup;
            self.ivars().document.addSubview(v);
        }
        popup
    }

    fn add_color_well(
        &self,
        mtm: MainThreadMarker,
        x: CGFloat,
        y: CGFloat,
        w: CGFloat,
        default_rgb: [u8; 3],
    ) -> Retained<NSColorWell> {
        // Color well is square-ish but takes the available width so
        // a divider drag does not squash it against the slider.
        let frame = CGRect {
            origin: CGPoint { x, y },
            size: CGSize {
                width: w.min(80.0),
                height: FORM_ROW_HEIGHT,
            },
        };
        let well: Retained<NSColorWell> = unsafe { NSColorWell::initWithFrame(mtm.alloc(), frame) };
        unsafe {
            let color = NSColor::colorWithSRGBRed_green_blue_alpha(
                default_rgb[0] as CGFloat / 255.0,
                default_rgb[1] as CGFloat / 255.0,
                default_rgb[2] as CGFloat / 255.0,
                1.0,
            );
            well.setColor(&color);
            let v: &NSView = &well;
            self.ivars().document.addSubview(v);
        }
        well
    }

    fn add_textfield(
        &self,
        mtm: MainThreadMarker,
        x: CGFloat,
        y: CGFloat,
        w: CGFloat,
        default: &str,
    ) -> Retained<NSTextField> {
        let frame = CGRect {
            origin: CGPoint { x, y },
            size: CGSize {
                width: w,
                height: FORM_ROW_HEIGHT,
            },
        };
        let field: Retained<NSTextField> =
            unsafe { NSTextField::initWithFrame(mtm.alloc(), frame) };
        unsafe {
            field.setStringValue(&NSString::from_str(default));
            field.setEditable(true);
            field.setBezeled(true);
            field.setDrawsBackground(true);
            let v: &NSView = &field;
            self.ivars().document.addSubview(v);
        }
        field
    }

    fn add_separator(&self, mtm: MainThreadMarker, x: CGFloat, y: CGFloat, w: CGFloat) {
        let frame = CGRect {
            origin: CGPoint { x, y: y + 4.0 },
            size: CGSize {
                width: w,
                height: 1.0,
            },
        };
        let box_view: Retained<NSBox> = unsafe { NSBox::initWithFrame(mtm.alloc(), frame) };
        unsafe {
            box_view.setBoxType(NSBoxType::NSBoxSeparator);
            let v: &NSView = &box_view;
            self.ivars().document.addSubview(v);
        }
        // Retain in extra_views via NSView cast so it lives as long
        // as the form is on screen.
        let v: Retained<NSView> = unsafe { Retained::cast(box_view) };
        self.ivars().extra_views.borrow_mut().push(v);
    }
}

/// Decode an `"r,g,b"` triple of byte components, returning `None` if
/// the string isn't well-formed. Whitespace around each component is
/// allowed because the saved-args file is hand-editable.
fn parse_rgb_triple(s: &str) -> Option<[u8; 3]> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 3 {
        return None;
    }
    let r = parts[0].trim().parse::<u8>().ok()?;
    let g = parts[1].trim().parse::<u8>().ok()?;
    let b = parts[2].trim().parse::<u8>().ok()?;
    Some([r, g, b])
}

fn choice_starting_index(prefill: Option<&str>, choices: &[String], default: usize) -> usize {
    // Saved values are integer indices, but legacy paths may have
    // stored the literal choice label. Accept both shapes so a
    // downgrade doesn't lose state.
    match prefill {
        Some(s) => match s.parse::<usize>() {
            Ok(idx) if idx < choices.len() => idx,
            _ => choices.iter().position(|c| c == s).unwrap_or(default),
        },
        None => default,
    }
}

/// Build a single label-style `NSTextField`, measure its wrapped
/// content height for the given width, then return both the view and
/// the height it actually needs. The caller uses the height to lay
/// out the next row.
///
/// The auto-measure step is what fixes the "long descriptions get
/// clipped" bug: AppKit's `NSTextField` does not auto-size to its
/// content. We have to ask its `NSTextFieldCell` for
/// `cellSizeForBounds:` with the desired width and a huge height,
/// then resize the field's frame to the height the cell reports.
fn build_label_view(
    mtm: MainThreadMarker,
    text: &str,
    font_size: CGFloat,
    initial_frame: CGRect,
    width: CGFloat,
) -> (Retained<NSView>, CGFloat) {
    let field: Retained<NSTextField> =
        unsafe { NSTextField::initWithFrame(mtm.alloc(), initial_frame) };
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
            // NSLineBreakMode is NSUInteger; passing isize trips the
            // objc2 runtime type-encoding check (`Q` vs `q`).
            let _: () = objc2::msg_send![&cell, setLineBreakMode: 0_usize /* byWordWrapping */];
        }
        let font: Retained<objc2::runtime::AnyObject> = objc2::msg_send_id![
            objc2::class!(NSFont),
            systemFontOfSize: font_size
        ];
        let _: () = objc2::msg_send![&field, setFont: &*font];
    }

    // Measure required height for the configured width.
    // `cellSizeForBounds:` honours the font, wraps the string, and
    // returns the exact size the cell would draw in.
    let measured_height = unsafe {
        let cell: Option<Retained<objc2::runtime::AnyObject>> = msg_send_id![&field, cell];
        match cell {
            Some(cell) => {
                let probe = CGRect {
                    origin: CGPoint { x: 0.0, y: 0.0 },
                    size: CGSize {
                        width,
                        height: CGFloat::MAX,
                    },
                };
                let size: CGSize = objc2::msg_send![&cell, cellSizeForBounds: probe];
                size.height.ceil().max(FORM_ROW_HEIGHT)
            }
            None => FORM_ROW_HEIGHT,
        }
    };

    // Apply the measured height to the field's frame so the wrapped
    // glyphs actually render.
    let final_frame = CGRect {
        origin: initial_frame.origin,
        size: CGSize {
            width: initial_frame.size.width,
            height: measured_height,
        },
    };
    unsafe { field.setFrame(final_frame) };

    // Safe: NSTextField is a subclass of NSView.
    let view: Retained<NSView> = unsafe { Retained::cast(field) };
    (view, measured_height)
}
