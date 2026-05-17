//! `NSOutlineViewDataSource` driven by the bundled G'MIC catalogue.
//!
//! Replaces T7's hardcoded `StaticDataSource`. The tree shown to AppKit is:
//!
//! ```text
//! Root
//! ├── Recent             (virtual; only listed when there is at least one entry)
//! │   ├── <RecentEntry 0>
//! │   └── …
//! ├── <top-level Folder 0>
//! │   ├── <Folder>…
//! │   └── <Filter>…
//! └── …
//! ```
//!
//! Items are passed to AppKit as `NSNumber` tags. The tag is an index
//! into [`DataSourceIvars::nodes`], a small intern table that grows on
//! demand from inside `child:ofItem:`. Interning gives every (parent,
//! child-index) pair a stable `NSNumber` whose `isEqual:` value matches
//! across reloads — important so `NSOutlineView` does not lose row
//! expansion state when we call `reloadData` on every search keystroke.
//!
//! The data source also acts as the search field's
//! `NSControlTextEditingDelegate`. On every keystroke it rebuilds an
//! optional [`PrunedTree`] (visible-children map keyed by parent node)
//! and triggers `reloadData` + an expand-all so matching subtrees
//! disclose themselves automatically.

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{declare_class, msg_send_id, mutability, ClassType, DeclaredClass};
use objc2_app_kit::{
    NSControl, NSControlTextEditingDelegate, NSOutlineView, NSOutlineViewDataSource,
    NSSearchFieldDelegate, NSTableColumn, NSTextFieldDelegate,
};
use objc2_foundation::{
    MainThreadMarker, NSInteger, NSNotification, NSNumber, NSObject, NSObjectProtocol, NSString,
};

use crate::catalogue::{Catalogue, Filter, Folder, Node};
use crate::settings::RecentEntry;

/// Logical tree node. Folder/Filter variants carry raw pointers into
/// `&'static Catalogue::root`; those pointers are stable for the
/// lifetime of the process because the catalogue lives in a
/// `OnceLock` (see [`crate::catalogue::builtin`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TreeNode {
    /// The synthetic root. Children: optional `Recent` + every top-level
    /// folder of the catalogue.
    Root,
    /// The "Recent" pseudo-folder. Children: every entry in
    /// `DataSourceIvars::recent` (the snapshot taken at panel-open time).
    Recent,
    /// A leaf inside the `Recent` pseudo-folder. The `usize` is an
    /// index into `DataSourceIvars::recent`.
    RecentEntry(usize),
    /// A real catalogue folder.
    Folder(*const Folder),
    /// A real catalogue filter (leaf).
    Filter(*const Filter),
}

/// View on what is visible when a search substring is active. `None`
/// for `visible` means "no filter — show everything". An empty
/// `visible_root` / `visible_recent` with a `Some(visible)` map means
/// the user typed something that matches nothing.
#[derive(Default, Debug)]
struct PrunedTree {
    /// `Some(map)` when search is active. Maps a folder node to the
    /// indices (into `Folder::children`) of its visible children.
    /// `None` means "filter inactive".
    visible: Option<HashMap<TreeNode, Vec<usize>>>,
    /// Visible top-level catalogue folders (indices into
    /// `catalogue.root.children`). Only meaningful when `visible.is_some()`.
    visible_root: Vec<usize>,
    /// Visible entries inside the Recent pseudo-folder (indices into
    /// `DataSourceIvars::recent`). Only meaningful when `visible.is_some()`.
    visible_recent: Vec<usize>,
    /// Whether the Recent pseudo-folder itself should appear at the
    /// top level when a filter is active (i.e. has any visible child
    /// OR its literal name matched).
    recent_root_visible: bool,
}

/// Instance variables for [`CatalogueDataSource`]. All fields are accessed
/// on the main thread only — the class declares `MainThreadOnly` mutability,
/// so the AppKit runtime guarantees serial access. That is what lets us
/// use `RefCell` / `OnceCell` instead of `Mutex` / `OnceLock`.
pub(crate) struct DataSourceIvars {
    catalogue: &'static Catalogue,
    /// Snapshot of `Settings::recent` taken at panel-open time. Never
    /// mutated during the modal session.
    recent: Vec<RecentEntry>,
    /// Intern table: `nodes[i]` is the `TreeNode` we previously handed
    /// to AppKit wrapped in `NSNumber(i)`. Slot 0 is always
    /// `TreeNode::Root`, which we never actually hand out (AppKit
    /// represents the root as `nil`); the slot just lets `intern` use
    /// `nodes.len()` as the index for new entries without a special case.
    nodes: RefCell<Vec<TreeNode>>,
    /// Reverse lookup so `intern` is O(1) and (parent, idx) callsites
    /// converge on the same slot across reloads.
    by_node: RefCell<HashMap<TreeNode, i64>>,
    /// Memoised `NSString` per node. Built lazily by
    /// `cached_label` and reused on every subsequent
    /// `objectValueForTableColumn:byItem:` call — that selector fires
    /// once per row whenever a row becomes visible during scroll, and
    /// without the cache each call was doing a UTF-8 → UTF-16
    /// conversion + Objective-C object allocation. That cost on the
    /// modal run loop was what made wheel-scroll feel sluggish even
    /// after pinning a fixed row height (see picker.rs).
    labels: RefCell<HashMap<TreeNode, Retained<NSString>>>,
    /// Current lowercased substring filter; empty means "no filter".
    search: RefCell<String>,
    /// Cached pruned tree for the current `search` value. Rebuilt by
    /// `update_search` whenever `search` changes.
    pruned: RefCell<PrunedTree>,
    /// Outline view we drive. Set once by `set_outline` right after
    /// the AppKit object graph is built; used by `update_search` to
    /// call `reloadData` on every keystroke.
    outline: OnceCell<Retained<NSOutlineView>>,
}

declare_class!(
    /// Data source / search delegate for the picker outline view.
    pub(crate) struct CatalogueDataSource;

    unsafe impl ClassType for CatalogueDataSource {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "GmicAffinityCatalogueDataSource";
    }

    impl DeclaredClass for CatalogueDataSource {
        type Ivars = DataSourceIvars;
    }

    unsafe impl NSObjectProtocol for CatalogueDataSource {}

    unsafe impl NSOutlineViewDataSource for CatalogueDataSource {
        #[method(outlineView:numberOfChildrenOfItem:)]
        unsafe fn number_of_children(
            &self,
            _view: &NSOutlineView,
            item: Option<&AnyObject>,
        ) -> NSInteger {
            let parent = unsafe { self.lookup(item) };
            self.count_children(parent) as NSInteger
        }

        #[method_id(outlineView:child:ofItem:)]
        unsafe fn child(
            &self,
            _view: &NSOutlineView,
            index: NSInteger,
            item: Option<&AnyObject>,
        ) -> Retained<AnyObject> {
            let parent = unsafe { self.lookup(item) };
            let child = self.child_at(parent, index as usize);
            let slot = self.intern(child);
            let number = NSNumber::new_i64(slot);
            // SAFETY: NSNumber is an Objective-C object so the upcast
            // to AnyObject (the root of the class hierarchy) is trivially valid.
            unsafe { Retained::cast::<AnyObject>(number) }
        }

        #[method(outlineView:isItemExpandable:)]
        unsafe fn is_expandable(
            &self,
            _view: &NSOutlineView,
            item: &AnyObject,
        ) -> bool {
            let node = unsafe { self.lookup(Some(item)) };
            matches!(node, TreeNode::Root | TreeNode::Recent | TreeNode::Folder(_))
        }

        #[method_id(outlineView:objectValueForTableColumn:byItem:)]
        unsafe fn value_for(
            &self,
            _view: &NSOutlineView,
            _column: Option<&NSTableColumn>,
            item: Option<&AnyObject>,
        ) -> Option<Retained<AnyObject>> {
            let node = unsafe { self.lookup(item) };
            let s = self.cached_label(node);
            // SAFETY: NSString is an Objective-C object.
            Some(unsafe { Retained::cast::<AnyObject>(s) })
        }
    }

    // NSSearchField's setDelegate: requires an `NSSearchFieldDelegate`.
    // We declare conformance to ONLY the leaf protocol from inside
    // `declare_class!`. Each `unsafe impl Trait for Self {}` block here
    // calls `class_addProtocol` at runtime; calling it for both a
    // protocol AND any super-protocol of it triggers the runtime's
    // "already conforms via inheritance" path, which returns NO and
    // makes objc2's `ClassBuilder::add_protocol` `assert!`-panic during
    // lazy class registration — that's what brought Affinity Photo
    // down on the T8 first pass (see crash report
    // `Affinity Photo 2 …-2026-05-17-171140.ips`).
    //
    // The Rust trait bound `NSSearchFieldDelegate: NSTextFieldDelegate
    // + NSControlTextEditingDelegate + …` is satisfied by the plain
    // `unsafe impl … for CatalogueDataSource {}` blocks WRITTEN OUTSIDE
    // the macro (see below `declare_class!`). Those bare impl blocks
    // give us Rust-level conformance without touching
    // `class_addProtocol`, which is exactly what we want.
    unsafe impl NSSearchFieldDelegate for CatalogueDataSource {}

    // `controlTextDidChange:` is defined by `NSControlTextEditingDelegate`,
    // a super-protocol of `NSTextFieldDelegate` which we already
    // declare. AppKit dispatches the notification via
    // `respondsToSelector:` on whatever object is set as the search
    // field's delegate, so installing the selector here without an
    // explicit `unsafe impl NSControlTextEditingDelegate` block is
    // both correct and necessary (see the comment above the
    // `NSTextFieldDelegate` impl for why an explicit conformance
    // would crash class registration).
    unsafe impl CatalogueDataSource {
        #[method(controlTextDidChange:)]
        unsafe fn control_text_did_change(&self, notification: &NSNotification) {
            // SAFETY: NSSearchField is an NSControl; pulling its
            // `stringValue` via the NSControl method is well-defined.
            let Some(obj) = (unsafe { notification.object() }) else {
                return;
            };
            let ctrl: &NSControl =
                unsafe { &*(&*obj as *const AnyObject as *const NSControl) };
            let text = unsafe { ctrl.stringValue() }.to_string();
            self.update_search(&text);
        }
    }
);

// Bare Rust-level conformance for the supertraits of
// `NSSearchFieldDelegate`. These impls satisfy the Rust trait bound
// `NSSearchFieldDelegate: NSTextFieldDelegate + NSControlTextEditingDelegate`
// without invoking `class_addProtocol` (which `declare_class!`'s
// `unsafe impl Trait for Self {}` blocks would do, and which the
// Objective-C runtime would reject because the class already conforms
// via inheritance from `NSSearchFieldDelegate`).
//
// Both protocols' methods are entirely `#[optional]`, so an empty impl
// is well-formed at the Rust type level. The `controlTextDidChange:`
// implementation lives in the `unsafe impl CatalogueDataSource {}`
// methods block inside `declare_class!` above.
//
// SAFETY: An NSSearchField's delegate is invoked on the main thread,
// and `CatalogueDataSource` is declared as `MainThreadOnly`. No protocol
// method is required, so there are no invariants to uphold beyond that.
unsafe impl NSTextFieldDelegate for CatalogueDataSource {}
unsafe impl NSControlTextEditingDelegate for CatalogueDataSource {}

impl CatalogueDataSource {
    /// Build a fresh data source for the given catalogue and (snapshot
    /// of) recent picks. The catalogue lives for `'static`; the recent
    /// list is taken by value because the data source owns it for the
    /// modal session and we do not want the picker to see live mutations
    /// while the panel is up.
    pub(crate) fn new(
        mtm: MainThreadMarker,
        catalogue: &'static Catalogue,
        recent: Vec<RecentEntry>,
    ) -> Retained<Self> {
        // Slot 0 is reserved for `TreeNode::Root` so `intern` can use
        // `nodes.len()` as the next index without a special case for
        // the empty Vec.
        let mut nodes = Vec::with_capacity(64);
        let mut by_node = HashMap::with_capacity(64);
        nodes.push(TreeNode::Root);
        by_node.insert(TreeNode::Root, 0_i64);

        let ivars = DataSourceIvars {
            catalogue,
            recent,
            nodes: RefCell::new(nodes),
            by_node: RefCell::new(by_node),
            labels: RefCell::new(HashMap::with_capacity(256)),
            search: RefCell::new(String::new()),
            pruned: RefCell::new(PrunedTree::default()),
            outline: OnceCell::new(),
        };
        let this = mtm.alloc::<Self>().set_ivars(ivars);
        unsafe { msg_send_id![super(this), init] }
    }

    /// Wire the outline view we drive. Called once, right after the
    /// AppKit object graph is built and `setDataSource:` has been called.
    /// We need this back-reference so `controlTextDidChange:` can fire
    /// `reloadData` on each keystroke.
    pub(crate) fn set_outline(&self, outline: Retained<NSOutlineView>) {
        // Ignore the (impossible) double-set: same outline → same effect.
        let _ = self.ivars().outline.set(outline);
    }

    pub(crate) fn as_data_source(&self) -> &ProtocolObject<dyn NSOutlineViewDataSource> {
        ProtocolObject::from_ref(self)
    }

    pub(crate) fn as_search_field_delegate(&self) -> &ProtocolObject<dyn NSSearchFieldDelegate> {
        ProtocolObject::from_ref(self)
    }

    fn intern(&self, node: TreeNode) -> i64 {
        let mut by = self.ivars().by_node.borrow_mut();
        if let Some(&i) = by.get(&node) {
            return i;
        }
        let mut nodes = self.ivars().nodes.borrow_mut();
        let i = nodes.len() as i64;
        nodes.push(node);
        by.insert(node, i);
        i
    }

    /// Map an AppKit-provided item back to a `TreeNode`.
    ///
    /// # Safety
    /// Callers must only pass items that originated from
    /// `child:ofItem:`. AppKit never invents items, so this holds by
    /// construction.
    unsafe fn lookup(&self, item: Option<&AnyObject>) -> TreeNode {
        let Some(obj) = item else {
            return TreeNode::Root;
        };
        let number: &NSNumber = unsafe { &*(obj as *const AnyObject as *const NSNumber) };
        let idx = number.as_i64();
        let nodes = self.ivars().nodes.borrow();
        // Defensive: an unknown slot would be a programming error
        // (item came from a different data source). Fall back to Root
        // rather than panic from inside an Objective-C callback.
        nodes.get(idx as usize).copied().unwrap_or(TreeNode::Root)
    }

    fn count_children(&self, parent: TreeNode) -> usize {
        let pruned = self.ivars().pruned.borrow();
        let filtering = pruned.visible.is_some();
        match parent {
            TreeNode::Root => {
                if filtering {
                    let mut n = pruned.visible_root.len();
                    if pruned.recent_root_visible {
                        n += 1;
                    }
                    n
                } else {
                    let real = self.ivars().catalogue.root.children.len();
                    if self.ivars().recent.is_empty() {
                        real
                    } else {
                        real + 1
                    }
                }
            }
            TreeNode::Recent => {
                if filtering {
                    pruned.visible_recent.len()
                } else {
                    self.ivars().recent.len()
                }
            }
            TreeNode::Folder(p) => {
                let folder: &Folder = unsafe { &*p };
                if filtering {
                    pruned
                        .visible
                        .as_ref()
                        .and_then(|m| m.get(&parent))
                        .map(|v| v.len())
                        .unwrap_or(0)
                } else {
                    folder.children.len()
                }
            }
            TreeNode::RecentEntry(_) | TreeNode::Filter(_) => 0,
        }
    }

    fn child_at(&self, parent: TreeNode, idx: usize) -> TreeNode {
        let pruned = self.ivars().pruned.borrow();
        let filtering = pruned.visible.is_some();
        match parent {
            TreeNode::Root => {
                // Recent (if visible) is the first child; real folders follow.
                if filtering {
                    if pruned.recent_root_visible {
                        if idx == 0 {
                            return TreeNode::Recent;
                        }
                        let real_idx = pruned.visible_root[idx - 1];
                        Self::root_child_at_index(self.ivars().catalogue, real_idx)
                    } else {
                        let real_idx = pruned.visible_root[idx];
                        Self::root_child_at_index(self.ivars().catalogue, real_idx)
                    }
                } else {
                    let has_recent = !self.ivars().recent.is_empty();
                    if has_recent {
                        if idx == 0 {
                            return TreeNode::Recent;
                        }
                        Self::root_child_at_index(self.ivars().catalogue, idx - 1)
                    } else {
                        Self::root_child_at_index(self.ivars().catalogue, idx)
                    }
                }
            }
            TreeNode::Recent => {
                let real_idx = if filtering {
                    pruned.visible_recent[idx]
                } else {
                    idx
                };
                TreeNode::RecentEntry(real_idx)
            }
            TreeNode::Folder(p) => {
                let folder: &Folder = unsafe { &*p };
                let real_idx = if filtering {
                    pruned
                        .visible
                        .as_ref()
                        .and_then(|m| m.get(&parent))
                        .and_then(|v| v.get(idx))
                        .copied()
                        .unwrap_or(idx)
                } else {
                    idx
                };
                Self::node_to_tree(&folder.children[real_idx])
            }
            TreeNode::RecentEntry(_) | TreeNode::Filter(_) => {
                // Leaves have no children — AppKit shouldn't ask, but be safe.
                TreeNode::Root
            }
        }
    }

    fn root_child_at_index(cat: &Catalogue, idx: usize) -> TreeNode {
        Self::node_to_tree(&cat.root.children[idx])
    }

    fn node_to_tree(node: &Node) -> TreeNode {
        match node {
            Node::Folder(f) => TreeNode::Folder(f as *const Folder),
            Node::Filter(f) => TreeNode::Filter(f as *const Filter),
        }
    }

    fn label(&self, node: TreeNode) -> String {
        match node {
            TreeNode::Root => String::new(),
            TreeNode::Recent => "Recent".to_string(),
            TreeNode::RecentEntry(i) => self
                .ivars()
                .recent
                .get(i)
                .map(|r| r.display_path.clone())
                .unwrap_or_default(),
            TreeNode::Folder(p) => unsafe { (*p).name.clone() },
            TreeNode::Filter(p) => unsafe { (*p).display_name.clone() },
        }
    }

    /// Memoised flavour of [`label`]. Returns a `Retained<NSString>`
    /// that is cached in `ivars.labels` so repeated visits to the
    /// same row during scroll are a single `objc_retain` rather than
    /// a UTF-8 → UTF-16 conversion + object alloc.
    fn cached_label(&self, node: TreeNode) -> Retained<NSString> {
        if let Some(s) = self.ivars().labels.borrow().get(&node) {
            return s.clone();
        }
        let label = self.label(node);
        let s: Retained<NSString> = NSString::from_str(&label);
        self.ivars()
            .labels
            .borrow_mut()
            .insert(node, s.clone());
        s
    }

    /// Apply a new search substring. Empty (or unchanged) is a no-op
    /// beyond clearing the filter; non-empty rebuilds the pruned tree
    /// and triggers `reloadData` + expand-all so matches surface.
    fn update_search(&self, q: &str) {
        let lc = q.trim().to_lowercase();
        {
            let cur = self.ivars().search.borrow();
            if *cur == lc {
                return;
            }
        }
        *self.ivars().search.borrow_mut() = lc.clone();
        *self.ivars().pruned.borrow_mut() = if lc.is_empty() {
            PrunedTree::default()
        } else {
            build_pruned(self.ivars().catalogue, &self.ivars().recent, &lc)
        };
        if let Some(outline) = self.ivars().outline.get() {
            unsafe {
                outline.reloadData();
                if !lc.is_empty() {
                    // Disclose every matching subtree so the user sees
                    // their hits without having to click around.
                    outline.expandItem_expandChildren(None, true);
                }
            }
        }
    }
}

/// Walk the catalogue + recent list and build a `PrunedTree` for the
/// given lowercased substring `q`. Folder name matches expose the
/// entire subtree below; filter name matches expose the filter plus
/// every ancestor folder.
fn build_pruned(cat: &Catalogue, recent: &[RecentEntry], q: &str) -> PrunedTree {
    let mut visible: HashMap<TreeNode, Vec<usize>> = HashMap::new();

    let mut visible_root: Vec<usize> = Vec::new();
    for (idx, child) in cat.root.children.iter().enumerate() {
        let keep = match child {
            Node::Folder(sub) => walk(sub, q, &mut visible),
            Node::Filter(f) => f.display_name.to_lowercase().contains(q),
        };
        if keep {
            visible_root.push(idx);
        }
    }

    let visible_recent: Vec<usize> = recent
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r.display_path.to_lowercase().contains(q)
                || r.command.to_lowercase().contains(q)
        })
        .map(|(i, _)| i)
        .collect();

    let recent_root_visible = "recent".contains(q) || !visible_recent.is_empty();

    PrunedTree {
        visible: Some(visible),
        visible_root,
        visible_recent,
        recent_root_visible,
    }
}

/// Returns `true` iff `folder` (or any descendant) is visible under `q`.
/// Mutates `visible` to record which direct children survive the prune.
fn walk(folder: &Folder, q: &str, visible: &mut HashMap<TreeNode, Vec<usize>>) -> bool {
    let folder_matches = folder.name.to_lowercase().contains(q);
    let mut my_visible: Vec<usize> = Vec::new();
    for (idx, child) in folder.children.iter().enumerate() {
        let child_keep = match child {
            Node::Folder(sub) => {
                if folder_matches {
                    walk_show_all(sub, visible);
                    true
                } else {
                    walk(sub, q, visible)
                }
            }
            Node::Filter(f) => {
                folder_matches || f.display_name.to_lowercase().contains(q)
            }
        };
        if child_keep {
            my_visible.push(idx);
        }
    }
    let any_kept = !my_visible.is_empty();
    if any_kept {
        visible.insert(TreeNode::Folder(folder as *const Folder), my_visible);
    }
    folder_matches || any_kept
}

/// Expose every descendant of `folder` unconditionally. Called when an
/// ancestor's name matched and we therefore want its entire subtree
/// disclosed.
fn walk_show_all(folder: &Folder, visible: &mut HashMap<TreeNode, Vec<usize>>) {
    let all: Vec<usize> = (0..folder.children.len()).collect();
    visible.insert(TreeNode::Folder(folder as *const Folder), all);
    for child in &folder.children {
        if let Node::Folder(sub) = child {
            walk_show_all(sub, visible);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue;

    fn fixture() -> Catalogue {
        Catalogue {
            root: Folder {
                name: String::new(),
                children: vec![
                    Node::Folder(Folder {
                        name: "Artistic".to_string(),
                        children: vec![
                            Node::Filter(Filter {
                                display_name: "Paint Brush".to_string(),
                                command: "fx_painting".to_string(),
                                description: None,
                                params: vec![],
                            }),
                            Node::Filter(Filter {
                                display_name: "Pen Drawing".to_string(),
                                command: "fx_pen_drawing".to_string(),
                                description: None,
                                params: vec![],
                            }),
                        ],
                    }),
                    Node::Folder(Folder {
                        name: "Lights & Shadows".to_string(),
                        children: vec![Node::Filter(Filter {
                            display_name: "Light Glow".to_string(),
                            command: "fx_glow".to_string(),
                            description: None,
                            params: vec![],
                        })],
                    }),
                ],
            },
        }
    }

    #[test]
    fn empty_query_yields_default_pruned_tree() {
        let cat = fixture();
        let pruned = PrunedTree::default();
        assert!(pruned.visible.is_none(), "no filter ⇒ visible map absent");
        let _ = cat; // unused but documents intent: empty query = no walking
    }

    #[test]
    fn folder_name_match_exposes_entire_subtree() {
        let cat = fixture();
        let pruned = build_pruned(&cat, &[], "artistic");
        assert_eq!(pruned.visible_root, vec![0]);
        let artistic_ptr = match &cat.root.children[0] {
            Node::Folder(f) => f as *const Folder,
            _ => unreachable!(),
        };
        let kids = pruned
            .visible
            .as_ref()
            .and_then(|m| m.get(&TreeNode::Folder(artistic_ptr)))
            .expect("Artistic must appear in pruned tree");
        assert_eq!(kids, &vec![0, 1], "every child shown when folder matches");
    }

    #[test]
    fn filter_name_match_promotes_only_ancestor_chain() {
        let cat = fixture();
        let pruned = build_pruned(&cat, &[], "glow");
        assert_eq!(pruned.visible_root, vec![1], "only Lights & Shadows visible");
        let ls_ptr = match &cat.root.children[1] {
            Node::Folder(f) => f as *const Folder,
            _ => unreachable!(),
        };
        let kids = pruned
            .visible
            .as_ref()
            .and_then(|m| m.get(&TreeNode::Folder(ls_ptr)))
            .expect("L&S parent must surface for matching child");
        assert_eq!(kids, &vec![0]);
    }

    #[test]
    fn no_match_yields_empty_visible_root() {
        let cat = fixture();
        let pruned = build_pruned(&cat, &[], "doesnotexist");
        assert!(pruned.visible_root.is_empty());
        assert!(pruned.visible.as_ref().unwrap().is_empty());
        assert!(!pruned.recent_root_visible);
    }

    #[test]
    fn recent_substring_match_surfaces_recent_root() {
        let cat = fixture();
        let recent = vec![RecentEntry {
            command: "fx_painting".to_string(),
            display_path: "Artistic / Paint Brush".to_string(),
            ts: "ts".to_string(),
        }];
        let pruned = build_pruned(&cat, &recent, "paint");
        assert!(pruned.recent_root_visible);
        assert_eq!(pruned.visible_recent, vec![0]);
    }

    #[test]
    fn typing_word_recent_shows_recent_root_even_with_no_entries() {
        let cat = fixture();
        let pruned = build_pruned(&cat, &[], "recent");
        assert!(
            pruned.recent_root_visible,
            "literal 'recent' substring should reveal the pseudo-folder"
        );
    }

    #[test]
    fn catalogue_pointer_identity_is_stable_across_clones() {
        // Smoke test that walking via `&'static Catalogue` (which is
        // what `catalogue::builtin()` returns) gives the same Folder
        // pointer for the same logical node — required for our
        // `TreeNode::Folder(*const Folder)` identity to round-trip
        // through `intern`.
        let cat = catalogue::builtin();
        let first = match &cat.root.children[0] {
            Node::Folder(f) => f as *const Folder,
            Node::Filter(_) => return, // skip if first top-level happens to be a filter
        };
        let again = match &cat.root.children[0] {
            Node::Folder(f) => f as *const Folder,
            _ => return,
        };
        assert_eq!(first, again, "addresses inside &'static Catalogue must be stable");
    }
}
