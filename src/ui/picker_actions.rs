//! Target/action controller wired to the picker's bottom button bar
//! and the outline view's double-click action.
//!
//! AppKit-side wiring (assembled in [`crate::ui::picker::show_picker`]):
//!
//! - **OK button**            → action = `onOk:`        → stop modal with `OK`
//! - **Cancel button**        → action = `onCancel:`    → stop modal with `Cancel`
//! - **Reset Defaults**       → action = `onReset:`     → rebuild form with stdlib defaults
//! - **Outline double-click** → action = `onDoubleClick:`
//!   → if a leaf is currently selected, behaves identically to OK.
//!   → if a folder is selected, the call is a no-op (AppKit's own
//!   default behaviour for double-click in the disclosure column
//!   toggles expand/collapse, and we don't want to override that).
//!
//! All actions also invoke `NSApp.stopModalWithCode:` so the manual
//! modal pump in [`crate::ui::runloop::run_modal_window`] returns
//! cleanly. The Rust-side wrapper in `picker.rs` then collects the
//! current form values and packages them into [`ChosenFilter`].
//!
//! Identity / retain notes:
//!
//! - The actions controller is declared `MainThreadOnly` because every
//!   AppKit target/action callback fires on the main thread and the
//!   ivars borrow `Retained<…>` which are `!Send + !Sync`.
//! - `setTarget:` on `NSControl` does NOT retain its argument. The
//!   caller in `picker.rs` keeps the controller alive in a local
//!   binding that is dropped only after the modal session has fully
//!   ended.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, Sel};
use objc2::{declare_class, msg_send_id, mutability, sel, ClassType, DeclaredClass};
use objc2_app_kit::{
    NSApplication, NSButton, NSModalResponseCancel, NSModalResponseOK, NSOutlineView,
};
use objc2_foundation::MainThreadMarker;

use crate::ui::picker_catalogue_data_source::CatalogueDataSource;
use crate::ui::picker_form::FormController;

pub(crate) struct PickerActionIvars {
    pub(crate) outline: Retained<NSOutlineView>,
    pub(crate) form: Retained<FormController>,
    pub(crate) data_source: Retained<CatalogueDataSource>,
    /// The OK button. We keep it for two reasons:
    /// 1. `onReset:` may need to refresh enabled state after a rebuild,
    ///    though in practice Reset never changes the selection.
    /// 2. The standalone debug-preselect path in `picker.rs` updates
    ///    OK-enabled outside of a regular selection event.
    pub(crate) ok_button: Retained<NSButton>,
    /// Captured at OK time so the Rust-side caller of `show_picker`
    /// can read it back after the modal pump exits.
    pub(crate) chosen_values: RefCell<Option<Vec<String>>>,
    /// Captured at OK time so the caller knows which filter the user
    /// landed on. Stored as a raw pointer because `*const Filter`
    /// implements `Copy` and AppKit callbacks borrow `&self`. The
    /// catalogue lives for `'static`, so the pointer never dangles.
    pub(crate) chosen_filter: Cell<Option<*const crate::catalogue::Filter>>,
}

declare_class!(
    pub(crate) struct PickerActions;

    unsafe impl ClassType for PickerActions {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "GmicAffinityPickerActions";
    }

    impl DeclaredClass for PickerActions {
        type Ivars = PickerActionIvars;
    }

    unsafe impl NSObjectProtocol for PickerActions {}

    unsafe impl PickerActions {
        #[method(onOk:)]
        unsafe fn on_ok(&self, _sender: Option<&AnyObject>) {
            self.commit_ok();
        }

        #[method(onCancel:)]
        unsafe fn on_cancel(&self, _sender: Option<&AnyObject>) {
            let mtm = MainThreadMarker::new()
                .expect("PickerActions::on_cancel must run on main thread");
            let app = NSApplication::sharedApplication(mtm);
            crate::logging::log("picker: Cancel pressed");
            unsafe { app.stopModalWithCode(NSModalResponseCancel) };
        }

        #[method(onReset:)]
        unsafe fn on_reset(&self, _sender: Option<&AnyObject>) {
            if let Some(filter) = self.currently_selected_filter() {
                crate::logging::log(&format!(
                    "picker: Reset Defaults for '{}'",
                    filter.command,
                ));
                // Rebuild the form with no prefill → kind defaults only.
                self.ivars().form.show_filter(filter);
            }
        }

        #[method(onDoubleClick:)]
        unsafe fn on_double_click(&self, _sender: Option<&AnyObject>) {
            // Only OK on double-click of a leaf row. Double-click on a
            // folder row falls through to AppKit's default
            // expand/collapse behaviour because we don't call
            // stopModal here.
            if self.currently_selected_filter().is_some() {
                self.commit_ok();
            }
        }
    }
);

impl PickerActions {
    pub(crate) fn new(
        mtm: MainThreadMarker,
        outline: Retained<NSOutlineView>,
        form: Retained<FormController>,
        data_source: Retained<CatalogueDataSource>,
        ok_button: Retained<NSButton>,
    ) -> Retained<Self> {
        let ivars = PickerActionIvars {
            outline,
            form,
            data_source,
            ok_button,
            chosen_values: RefCell::new(None),
            chosen_filter: Cell::new(None),
        };
        let this = mtm.alloc::<Self>().set_ivars(ivars);
        unsafe { msg_send_id![super(this), init] }
    }

    /// Currently-selected leaf filter, or `None` for a folder / no
    /// selection.
    pub(crate) fn currently_selected_filter(&self) -> Option<&'static crate::catalogue::Filter> {
        let row = unsafe { self.ivars().outline.selectedRow() };
        if row < 0 {
            return None;
        }
        let item = unsafe { self.ivars().outline.itemAtRow(row) }?;
        self.ivars().data_source.resolve_filter(&item)
    }

    /// Snapshot the form, end the modal session with `.OK`.
    fn commit_ok(&self) {
        let Some(filter) = self.currently_selected_filter() else {
            crate::logging::log("picker: OK with no leaf selected, ignored");
            return;
        };
        let values = self.ivars().form.collect_values();
        crate::logging::log(&format!(
            "picker: OK for '{}' (args={})",
            filter.command,
            values.len(),
        ));
        *self.ivars().chosen_values.borrow_mut() = Some(values);
        self.ivars().chosen_filter.set(Some(filter as *const _));

        let mtm =
            MainThreadMarker::new().expect("PickerActions::commit_ok must run on main thread");
        let app = NSApplication::sharedApplication(mtm);
        unsafe { app.stopModalWithCode(NSModalResponseOK) };
    }

    /// Take ownership of the captured filter + values after the modal
    /// pump exits. Returns `None` if the user cancelled.
    pub(crate) fn take_chosen(&self) -> Option<(&'static crate::catalogue::Filter, Vec<String>)> {
        let values = self.ivars().chosen_values.borrow_mut().take()?;
        let p = self.ivars().chosen_filter.take()?;
        // SAFETY: `p` was captured from a `&'static Filter` produced
        // by `resolve_filter`, which is itself rooted in
        // `catalogue::builtin()`'s `OnceLock`. The catalogue is never
        // dropped during process lifetime.
        let filter: &'static crate::catalogue::Filter = unsafe { &*p };
        Some((filter, values))
    }

    /// Refresh the OK button's enabled state based on the current
    /// selection. Called by the form controller whenever the
    /// outline view's selection changes.
    pub(crate) fn refresh_ok_enabled(&self) {
        let enabled = self.currently_selected_filter().is_some();
        self.ivars().ok_button.setEnabled(enabled);
    }
}

/// Selectors exposed to AppKit. Kept here so `picker.rs` can wire
/// them via `setAction:` / `setDoubleAction:` without depending on
/// `sel!` macros further afield.
pub(crate) fn sel_on_ok() -> Sel {
    sel!(onOk:)
}
pub(crate) fn sel_on_cancel() -> Sel {
    sel!(onCancel:)
}
pub(crate) fn sel_on_reset() -> Sel {
    sel!(onReset:)
}
pub(crate) fn sel_on_double_click() -> Sel {
    sel!(onDoubleClick:)
}
