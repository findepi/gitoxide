use crate::tree::EntryKind;
use crate::{tree, Tree};
use bstr::{BStr, BString, ByteSlice, ByteVec};
use gix_hash::ObjectId;
use gix_hashtable::hash_map::Entry;
use std::cmp::Ordering;

/// The state needed to apply edits instantly to in-memory trees.
///
/// It's made so that each tree is looked at in the object database at most once, and held in memory for
/// all edits until everything is flushed to write all changed trees.
///
/// The editor is optimized to edit existing trees, but can deal with building entirely new trees as well
/// with some penalties.
///
/// ### Note
///
/// For reasons of efficiency, internally a SHA1 based hashmap is used to avoid having to store full paths
/// to each edited tree. The chance of collision is low, but could be engineered to overwrite or write into
/// an unintended tree.
pub struct Editor<'a> {
    /// A way to lookup trees.
    find: &'a dyn crate::FindExt,
    /// All trees we currently hold in memory. Each of these may change while adding and removing entries.
    /// null-object-ids mark tree-entries whose value we don't know yet, they are placeholders that will be
    /// dropped when writing at the latest.
    trees: gix_hashtable::HashMap<ObjectId, Tree>,
    /// A buffer to build up paths when finding the tree to edit.
    path_buf: BString,
}

/// Lifecycle
impl<'a> Editor<'a> {
    /// Create a new editor that uses `root` as base for all edits. Use `find` to lookup existing
    /// trees when edits are made. Each tree will only be looked-up once and then edited in place from
    /// that point on.
    pub fn new(root: Tree, find: &'a dyn crate::FindExt) -> Self {
        Editor {
            find,
            trees: gix_hashtable::HashMap::from_iter(Some((empty_path_hash(), root))),
            path_buf: Vec::with_capacity(512).into(),
        }
    }
}

/// Operations
impl<'a> Editor<'a> {
    /// Write the entire in-memory state of all changed trees (and only changed trees) to `out`.
    ///
    /// The last call to `out` will be the changed root tree, whose object-id will also be returned.
    /// `out` is free to do any kind of additional validation, like to assure that all entries in the tree exist.
    /// We don't assure that as there is no validation that inserted entries are valid object ids.
    ///
    /// Future calls to [`upsert`](Self::upsert) or similar will keep working on the last seen state of the
    /// just-written root-tree.
    /// If this is not desired, use [set_root()](Self::set_root()).
    pub fn write<E>(&mut self, mut out: impl FnMut(&Tree) -> Result<ObjectId, E>) -> Result<ObjectId, E> {
        assert_ne!(self.trees.len(), 0, "there is at least the root tree");

        // back is for children, front is for parents.
        let mut parents = vec![(
            None::<usize>,
            BString::default(),
            self.trees
                .remove(&empty_path_hash())
                .expect("root tree is always present"),
        )];
        let mut children = Vec::new();
        while let Some((parent_idx, mut rela_path, mut tree)) = children.pop().or_else(|| parents.pop()) {
            let mut all_entries_unchanged_or_written = true;
            for entry in &tree.entries {
                if entry.mode.is_tree() {
                    let prev_len = push_path_component(&mut rela_path, &entry.filename);
                    if let Some(sub_tree) = self.trees.remove(&path_hash(&rela_path)) {
                        all_entries_unchanged_or_written = false;
                        let next_parent_idx = parents.len();
                        children.push((Some(next_parent_idx), rela_path.clone(), sub_tree));
                    }
                    rela_path.truncate(prev_len);
                }
            }
            if all_entries_unchanged_or_written {
                tree.entries.retain(|e| !e.oid.is_null());
                let tree_id = out(&tree)?;
                if let Some((_, _, parent_to_adjust)) =
                    parent_idx.map(|idx| parents.get_mut(idx).expect("always present, pointing towards zero"))
                {
                    let name = filename(rela_path.as_bstr());
                    let entry_idx = parent_to_adjust
                        .entries
                        .binary_search_by(|e| cmp_entry_with_name(e, name, true))
                        .expect("the parent always knows us by name");
                    if tree.entries.is_empty() {
                        parent_to_adjust.entries.remove(entry_idx);
                    } else {
                        parent_to_adjust.entries[entry_idx].oid = tree_id;
                    }
                }
                if parents.is_empty() {
                    debug_assert!(children.is_empty(), "we consume children before parents");
                    debug_assert!(rela_path.is_empty(), "this should always be the root tree");

                    // There may be left-over trees if they are replaced with blobs for example.
                    self.trees.clear();
                    self.trees.insert(empty_path_hash(), tree);
                    return Ok(tree_id);
                }
            } else {
                parents.push((parent_idx, rela_path, tree));
            }
        }

        unreachable!("we exit as soon as everything is consumed")
    }

    /// Insert a new entry of `kind` with `id` at `rela_path`, an iterator over each path component in the tree,
    /// like `a/b/c`. Names are matched case-sensitively.
    ///
    /// Existing leaf-entries will be overwritten unconditionally, and it is assumed that `id` is available in the object database
    /// or will be made available at a later point to assure the integrity of the produced tree.
    ///
    /// Intermediate trees will be created if they don't exist in the object database, otherwise they will be loaded and entries
    /// will be inserted into them instead.
    ///
    /// Note that `id` can be [null](ObjectId::null()) to create a placeholder. These will not be written, and paths leading
    /// through them will not be considered a problem.
    ///
    /// `id` can also be an empty tree, along with [the respective `kind`](EntryKind::Tree), even though that's normally not allowed
    /// in Git trees.
    pub fn upsert<I, C>(
        &mut self,
        rela_path: I,
        kind: EntryKind,
        id: ObjectId,
    ) -> Result<&mut Self, crate::find::existing_object::Error>
    where
        I: IntoIterator<Item = C>,
        C: AsRef<BStr>,
    {
        let mut cursor = self.trees.get_mut(&empty_path_hash()).expect("root is always present");
        self.path_buf.clear();
        let mut rela_path = rela_path.into_iter().peekable();
        while let Some(name) = rela_path.next() {
            let name = name.as_ref();
            let is_last = rela_path.peek().is_none();
            match cursor
                .entries
                .binary_search_by(|e| cmp_entry_with_name(e, name, false))
                .or_else(|_| cursor.entries.binary_search_by(|e| cmp_entry_with_name(e, name, true)))
            {
                Ok(idx) => {
                    let entry = &mut cursor.entries[idx];
                    if is_last {
                        entry.oid = id;
                        entry.mode = kind.into();
                        break;
                    } else {
                        // TODO: lookup existing tree if it is one, otherwise overwrite.
                        entry.oid = id.kind().null();
                        entry.mode = EntryKind::Tree.into();
                    }
                }
                Err(insertion_idx) => {
                    cursor.entries.insert(
                        insertion_idx,
                        tree::Entry {
                            filename: name.into(),
                            mode: if is_last { kind.into() } else { EntryKind::Tree.into() },
                            oid: if is_last { id } else { id.kind().null() },
                        },
                    );
                    if is_last {
                        break;
                    }
                }
            }
            debug_assert!(
                !is_last,
                "adding new trees makes sense only for intermediate components"
            );
            push_path_component(&mut self.path_buf, name);
            let path_id = path_hash(&self.path_buf);
            cursor = match self.trees.entry(path_id) {
                Entry::Occupied(e) => e.into_mut(),
                Entry::Vacant(e) => e.insert(Tree::default()),
            };
        }
        Ok(self)
    }

    /// Set the root tree of the modification to `root`, assuring it has a well-known state.
    ///
    /// Note that this erases all previous edits.
    ///
    /// This is useful if the same editor is re-used for various trees.
    pub fn set_root(&mut self, root: Tree) -> &mut Self {
        self.trees.clear();
        self.trees.insert(empty_path_hash(), root);
        self
    }
}

fn cmp_entry_with_name(a: &tree::Entry, filename: &BStr, is_tree: bool) -> Ordering {
    let common = a.filename.len().min(filename.len());
    a.filename[..common].cmp(&filename[..common]).then_with(|| {
        let a = a.filename.get(common).or_else(|| a.mode.is_tree().then_some(&b'/'));
        let b = filename.get(common).or_else(|| is_tree.then_some(&b'/'));
        a.cmp(&b)
    })
}

fn filename(path: &BStr) -> &BStr {
    path.rfind_byte(b'/').map_or(path, |pos| &path[pos + 1..])
}

fn empty_path_hash() -> ObjectId {
    gix_features::hash::hasher(gix_hash::Kind::Sha1).digest().into()
}

fn path_hash(path: &[u8]) -> ObjectId {
    let mut hasher = gix_features::hash::hasher(gix_hash::Kind::Sha1);
    hasher.update(path);
    hasher.digest().into()
}

fn push_path_component(base: &mut BString, component: &[u8]) -> usize {
    let prev_len = base.len();
    debug_assert!(base.last() != Some(&b'/'));
    if !base.is_empty() {
        base.push_byte(b'/');
    }
    base.push_str(component);
    prev_len
}
