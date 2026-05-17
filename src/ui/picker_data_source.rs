//! `NSOutlineViewDataSource` for the T7 static-tree milestone.
//!
//! Hard-codes one folder ("Artistic") containing one leaf
//! ("Paint Brush"). The tree is identified to AppKit by `NSNumber`
//! tags: `0` = folder, `1` = leaf. AppKit retains the items we return
//! from `child:ofItem:` and passes those same pointers back to the
//! other selectors, so we never have to fight pointer-identity issues.
//!
//! T8 swaps this out for a real catalogue-backed data source.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{declare_class, msg_send_id, mutability, ClassType, DeclaredClass};
use objc2_app_kit::{NSOutlineView, NSOutlineViewDataSource, NSTableColumn};
use objc2_foundation::{
    MainThreadMarker, NSInteger, NSNumber, NSObject, NSObjectProtocol, NSString,
};

const TAG_FOLDER: i64 = 0;
const TAG_LEAF: i64 = 1;

declare_class!(
    pub(crate) struct StaticDataSource;

    unsafe impl ClassType for StaticDataSource {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "GmicAffinityStaticDataSource";
    }

    impl DeclaredClass for StaticDataSource {
        type Ivars = ();
    }

    unsafe impl NSObjectProtocol for StaticDataSource {}

    unsafe impl NSOutlineViewDataSource for StaticDataSource {
        #[method(outlineView:numberOfChildrenOfItem:)]
        unsafe fn number_of_children(
            &self,
            _view: &NSOutlineView,
            item: Option<&AnyObject>,
        ) -> NSInteger {
            match tag_of(item) {
                None => 1,           // root has one folder
                Some(t) if t == TAG_FOLDER => 1,  // folder has one leaf
                _ => 0,              // leaf has no children
            }
        }

        #[method_id(outlineView:child:ofItem:)]
        unsafe fn child(
            &self,
            _view: &NSOutlineView,
            _index: NSInteger,
            item: Option<&AnyObject>,
        ) -> Retained<AnyObject> {
            let tag = match tag_of(item) {
                None => TAG_FOLDER,
                Some(t) if t == TAG_FOLDER => TAG_LEAF,
                _ => TAG_LEAF, // defensive: AppKit shouldn't ask
            };
            let number = NSNumber::new_i64(tag);
            // Upcast Retained<NSNumber> -> Retained<AnyObject>.
            // SAFETY: AnyObject is the root of every Objective-C class.
            unsafe { Retained::cast::<AnyObject>(number) }
        }

        #[method(outlineView:isItemExpandable:)]
        unsafe fn is_expandable(
            &self,
            _view: &NSOutlineView,
            item: &AnyObject,
        ) -> bool {
            tag_of(Some(item)) == Some(TAG_FOLDER)
        }

        #[method_id(outlineView:objectValueForTableColumn:byItem:)]
        unsafe fn value_for(
            &self,
            _view: &NSOutlineView,
            _column: Option<&NSTableColumn>,
            item: Option<&AnyObject>,
        ) -> Option<Retained<AnyObject>> {
            let label = match tag_of(item) {
                Some(t) if t == TAG_FOLDER => "Artistic",
                Some(t) if t == TAG_LEAF => "Paint Brush",
                _ => "?",
            };
            let s: Retained<NSString> = NSString::from_str(label);
            // SAFETY: AnyObject is the root of every Objective-C class.
            Some(unsafe { Retained::cast::<AnyObject>(s) })
        }
    }
);

impl StaticDataSource {
    pub(crate) fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(());
        unsafe { msg_send_id![super(this), init] }
    }

    /// Type-erase to the protocol object that `NSOutlineView::setDataSource`
    /// wants.
    pub(crate) fn as_protocol(&self) -> &ProtocolObject<dyn NSOutlineViewDataSource> {
        ProtocolObject::from_ref(self)
    }
}

/// Read the i64 tag carried by an item, or `None` for the root (nil).
///
/// # Safety
/// The caller asserts that any non-nil item came out of our `child:ofItem:`
/// and is therefore an `NSNumber`. AppKit never invents items, so this
/// holds by construction.
unsafe fn tag_of(item: Option<&AnyObject>) -> Option<i64> {
    let obj = item?;
    let number: &NSNumber = unsafe { &*(obj as *const AnyObject as *const NSNumber) };
    Some(number.as_i64())
}
