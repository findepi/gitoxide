use bstr::ByteSlice;
use git_hash::ObjectId;

use crate::util::split_at_pos;
use crate::{
    extension::{Signature, Tree},
    util::split_at_byte_exclusive,
};

pub const SIGNATURE: Signature = *b"TREE";

pub mod verify {
    use bstr::BString;
    use quick_error::quick_error;

    quick_error! {
        #[derive(Debug)]
        pub enum Error {
            RootWithName { name: BString } {
                display("The root tree was named '{}', even though it should be empty", name)
            }
            EntriesCount {actual: u32, expected: u32 } {
                display("Expected not more than {} entries to be reachable from the top-level, but actual count was {}", expected, actual)
            }
        }
    }
}

impl Tree {
    pub fn verify(&self) -> Result<(), verify::Error> {
        fn verify_recursive(children: &[Tree]) -> Result<Option<u32>, verify::Error> {
            if children.is_empty() {
                return Ok(None);
            }
            let mut entries = 0;
            for child in children {
                let actual_num_entries = verify_recursive(&child.children)?;
                if let Some(actual) = actual_num_entries {
                    if actual > child.num_entries {
                        return Err(verify::Error::EntriesCount {
                            actual,
                            expected: child.num_entries,
                        });
                    }
                }
                entries += child.num_entries;
            }
            Ok(entries.into())
        }

        if !self.name.is_empty() {
            return Err(verify::Error::RootWithName {
                name: self.name.as_bstr().into(),
            });
        }

        let declared_entries = verify_recursive(&self.children)?;
        if let Some(actual) = declared_entries {
            if actual > self.num_entries {
                return Err(verify::Error::EntriesCount {
                    actual,
                    expected: self.num_entries,
                });
            }
        }

        Ok(())
    }
}

/// A recursive data structure
pub fn decode(data: &[u8], object_hash: git_hash::Kind) -> Option<Tree> {
    let (tree, data) = one_recursive(data, object_hash.len_in_bytes())?;
    assert!(
        data.is_empty(),
        "BUG: should fully consume the entire tree extension chunk, got {} left",
        data.len()
    );
    Some(tree)
}

pub fn one_recursive(data: &[u8], hash_len: usize) -> Option<(Tree, &[u8])> {
    let (path, data) = split_at_byte_exclusive(data, 0)?;

    let (entry_count, data) = split_at_byte_exclusive(data, b' ')?;
    let num_entries: u32 = atoi::atoi(entry_count)?;

    let (subtree_count, data) = split_at_byte_exclusive(data, b'\n')?;
    let subtree_count: usize = atoi::atoi(subtree_count)?;

    let (hash, mut data) = split_at_pos(data, hash_len)?;
    let id = ObjectId::from(hash);

    let mut subtrees = Vec::with_capacity(subtree_count);
    for _ in 0..subtree_count {
        let (tree, rest) = one_recursive(data, hash_len)?;
        subtrees.push(tree);
        data = rest;
    }

    Some((
        Tree {
            id,
            num_entries,
            name: path.into(),
            children: subtrees,
        },
        data,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_of_tree() {
        assert_eq!(std::mem::size_of::<Tree>(), 80);
    }
}
