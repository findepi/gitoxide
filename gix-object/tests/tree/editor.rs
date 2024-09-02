use gix_object::tree::EntryKind;
use gix_object::{find, Tree};

#[test]
fn from_empty_add() -> crate::Result {
    let mut edit = gix_object::tree::Editor::new(Tree::default(), &find::Never);

    let (storage, mut write) = new_inmemory_writes();
    let actual = edit.write(&mut write).expect("no changes are fine");
    assert_eq!(actual, empty_tree(), "empty stays empty");
    assert_eq!(storage.borrow().len(), 1, "the empty tree was written");
    assert_eq!(
        display_tree(actual, &storage),
        "4b825dc642cb6eb9a060e54bf8d69288fbee4904\n"
    );

    edit.upsert(Some("hi"), EntryKind::Blob, gix_hash::Kind::Sha1.null())?;
    let actual = edit.write(&mut write).expect("effectively no changes are fine");
    assert_eq!(
        actual,
        empty_tree(),
        "null-ids are dropped automatically, they act as placeholders"
    );
    assert_eq!(storage.borrow().len(), 1, "the empty tree was written, nothing new");

    edit.upsert(["a", "b", "c"], EntryKind::Blob, gix_hash::Kind::Sha1.null())?
        .upsert(["a", "b", "d", "e"], EntryKind::Blob, gix_hash::Kind::Sha1.null())?;
    let actual = edit.write(&mut write).expect("effectively no changes are fine");
    assert_eq!(
        actual,
        empty_tree(),
        "null-ids are dropped automatically, recursively, they act as placeholders"
    );
    assert_eq!(storage.borrow().len(), 1, "still nothing but empty trees");

    edit.upsert(["a", "b"], EntryKind::Tree, empty_tree())?
        .upsert(["a", "b", "c"], EntryKind::Tree, empty_tree())?
        .upsert(["a", "b", "d", "e"], EntryKind::Tree, empty_tree())?;
    let actual = edit.write(&mut write).expect("it's OK to write empty trees");
    assert_eq!(
        display_tree(actual, &storage),
        "bf91a94ae659ac8a9da70d26acf42df1a36adb6e
└── a
    └── b
        ├── c (empty)
        └── d
            └── e (empty)
",
        "one can write through trees, and empty trees are also fine"
    );

    edit.upsert(["a"], EntryKind::Blob, any_blob())?
        .upsert(["a", "b"], EntryKind::Blob, any_blob())?
        .upsert(["a", "b", "c"], EntryKind::Blob, any_blob())?
        .upsert(["b", "d"], EntryKind::Blob, any_blob())?;

    let actual = edit.write(&mut write).expect("writing made-up blobs is fine");
    assert_eq!(
        display_tree(actual, &storage),
        "bf18e0ec42a5a96e16b312e04a7a67a9710a54a3
├── a
│   └── b
│       └── c bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.100644
└── b
    └── d bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.100644
",
        "it's possible to write through previously added blobs"
    );

    edit.upsert(["a", "b", "c"], EntryKind::Blob, any_blob())?
        .upsert(["a"], EntryKind::Blob, any_blob())?;

    let actual = edit.write(&mut write)?;
    assert_eq!(
        display_tree(actual, &storage),
        "835a710bc8a649148c9094f6cad1f309ce33a4fa
├── a bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.100644
└── b
    └── d bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.100644
",
        "note that `b/d` is from the previouly written root-tree, which may be confusing"
    );

    edit.set_root(Tree::default())
        .upsert(["a", "b", "c"], EntryKind::Blob, any_blob())?
        .upsert(["a"], EntryKind::Blob, any_blob())?;
    let actual = edit.write(&mut write)?;
    assert_eq!(
        display_tree(actual, &storage),
        "077c77c8214a54bdaf8cafcc36c2f7f0e61a2e43
└── a bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.100644
",
        "now the root is back to a well-known state, so edits are more intuitive"
    );
    Ok(())
}

mod utils {
    use bstr::{BStr, ByteSlice};
    use gix_hash::ObjectId;
    use gix_object::{Tree, WriteTo};
    use std::cell::RefCell;
    use std::rc::Rc;

    type TreeStore = Rc<RefCell<gix_hashtable::HashMap<ObjectId, Tree>>>;

    pub(super) fn new_inmemory_writes() -> (
        TreeStore,
        impl FnMut(&Tree) -> Result<ObjectId, std::convert::Infallible>,
    ) {
        let store = TreeStore::default();
        let write_tree = {
            let store = store.clone();
            let mut buf = Vec::with_capacity(512);
            move |tree: &Tree| {
                buf.clear();
                tree.write_to(&mut buf)
                    .expect("write to memory can't fail and tree is valid");
                let header = gix_object::encode::loose_header(gix_object::Kind::Tree, buf.len() as u64);
                let mut hasher = gix_features::hash::hasher(gix_hash::Kind::Sha1);
                hasher.update(&header);
                hasher.update(&buf);
                let id = hasher.digest().into();
                store.borrow_mut().insert(id, tree.clone());
                Ok(id)
            }
        };
        (store, write_tree)
    }

    fn display_tree_recursive(tree_id: ObjectId, storage: &TreeStore, name: Option<&BStr>) -> termtree::Tree<String> {
        let borrow = storage.borrow();
        let tree = borrow
            .get(&tree_id)
            .unwrap_or_else(|| panic!("tree {tree_id} is always present"));

        let mut termtree = termtree::Tree::new(if let Some(name) = name {
            if tree.entries.is_empty() {
                format!("{name} (empty)")
            } else {
                name.to_string()
            }
        } else {
            tree_id.to_string()
        });

        for entry in &tree.entries {
            if entry.mode.is_tree() {
                termtree.push(display_tree_recursive(
                    entry.oid,
                    storage,
                    Some(entry.filename.as_bstr()),
                ));
            } else {
                termtree.push(format!(
                    "{} {}.{}",
                    entry.filename,
                    entry.oid,
                    entry.mode.kind().as_octal_str()
                ));
            }
        }
        termtree
    }

    pub(super) fn display_tree(tree_id: ObjectId, storage: &TreeStore) -> String {
        display_tree_recursive(tree_id, storage, None).to_string()
    }

    pub(super) fn empty_tree() -> ObjectId {
        ObjectId::empty_tree(gix_hash::Kind::Sha1)
    }

    pub(super) fn any_blob() -> ObjectId {
        ObjectId::from_hex("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".as_bytes()).unwrap()
    }
}
use utils::{any_blob, display_tree, empty_tree, new_inmemory_writes};
