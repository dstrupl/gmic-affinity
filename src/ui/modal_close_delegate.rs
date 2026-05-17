//! [`NSWindowDelegate`] that ends an app-modal session when the window closes.
//!
//! `NSApplication::runModalForWindow` only returns after `stopModal` /
//! `stopModalWithCode`. The title-bar close control does not call those by
//! default, so we stop the modal from `windowWillClose:`.

use objc2::rc::Retained;
use objc2::{declare_class, msg_send_id, mutability, ClassType, DeclaredClass};
use objc2_app_kit::{NSApplication, NSWindowDelegate};
use objc2_foundation::{MainThreadMarker, NSNotification, NSObject, NSObjectProtocol};

declare_class!(
    pub(crate) struct ModalCloseDelegate;

    unsafe impl ClassType for ModalCloseDelegate {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "GmicAffinityModalCloseDelegate";
    }

    impl DeclaredClass for ModalCloseDelegate {
        type Ivars = ();
    }

    unsafe impl NSObjectProtocol for ModalCloseDelegate {}

    unsafe impl NSWindowDelegate for ModalCloseDelegate {
        #[method(windowWillClose:)]
        unsafe fn windowWillClose(&self, _notification: &NSNotification) {
            let mtm = MainThreadMarker::new()
                .expect("ModalCloseDelegate::windowWillClose must run on main thread");
            let app = NSApplication::sharedApplication(mtm);
            unsafe { app.stopModal() };
        }
    }
);

impl ModalCloseDelegate {
    pub(crate) fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc();
        let this = this.set_ivars(());
        unsafe { msg_send_id![super(this), init] }
    }
}
