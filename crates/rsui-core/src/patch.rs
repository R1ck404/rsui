use std::fmt;
use std::ops::Deref;
use std::sync::Arc as StdArc;

use serde::{Serialize, Serializer};
use triomphe::Arc;

use crate::Text;
use crate::element::{Actions, Node, PropValue};
use crate::symbol::Symbol;

/// How many indices a [`Path`] holds without allocating.
const INLINE_DEPTH: usize = 5;

/// Child indices leading from the root to a node. Empty means the root.
///
/// Most changes are near the root, so short paths need no allocation.
#[derive(Clone)]
pub struct Path(PathRepr);

#[derive(Clone)]
enum PathRepr {
    Inline {
        len: u8,
        indices: [u32; INLINE_DEPTH],
    },
    Shared(StdArc<[u32]>),
}

impl Deref for Path {
    type Target = [u32];

    fn deref(&self) -> &[u32] {
        match &self.0 {
            PathRepr::Inline { len, indices } => &indices[..*len as usize],
            PathRepr::Shared(indices) => indices,
        }
    }
}

impl From<&[u32]> for Path {
    fn from(indices: &[u32]) -> Self {
        if indices.len() <= INLINE_DEPTH {
            let mut inline = [0; INLINE_DEPTH];
            inline[..indices.len()].copy_from_slice(indices);
            Self(PathRepr::Inline {
                len: indices.len() as u8,
                indices: inline,
            })
        } else {
            Self(PathRepr::Shared(StdArc::from(indices)))
        }
    }
}

impl PartialEq for Path {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for Path {}

impl fmt::Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl Serialize for Path {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (**self).serialize(serializer)
    }
}

/// One edit to a tree.
///
/// Patches apply in order, and each index refers to the tree as the earlier
/// patches left it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Patch {
    /// Swap the node at `path` for `node`.
    Replace {
        path: Path,
        node: Node,
    },
    SetName {
        path: Path,
        name: Text,
    },
    SetProp {
        path: Path,
        name: Symbol,
        value: PropValue,
    },
    RemoveProp {
        path: Path,
        name: Symbol,
    },
    SetActions {
        path: Path,
        actions: Actions,
    },
    /// Insert `node` so that it ends up at `index` among the children of `parent`.
    Insert {
        parent: Path,
        index: u32,
        node: Node,
    },
    Remove {
        parent: Path,
        index: u32,
    },
    /// Take the child at `from` out, then put it back so that it ends up at `to`.
    Move {
        parent: Path,
        from: u32,
        to: u32,
    },
    /// Rearrange many children at once, for reorders too large for individual
    /// moves to be worthwhile. The child at `start + order[i]` ends up at
    /// `start + i`. `order` is a permutation of `0..order.len()`.
    Reorder {
        parent: Path,
        start: u32,
        order: Box<[u32]>,
    },
}

/// A patch pointed at a child that does not exist, so it was not made for this tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyError {
    /// Position of the failing patch in the slice passed to [`apply`].
    pub patch: usize,
    /// The child index that was out of bounds.
    pub index: u32,
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "patch {} refers to child index {}, which does not exist",
            self.patch, self.index
        )
    }
}

impl std::error::Error for ApplyError {}

/// Applies `patches` to `root` and returns the updated tree.
///
/// Nodes are copied only if something else still holds them. Keep a clone of
/// `root` if you need the old tree, or need to recover from an error.
pub fn apply(mut root: Node, patches: &[Patch]) -> Result<Node, ApplyError> {
    for (position, patch) in patches.iter().enumerate() {
        apply_one(&mut root, patch).map_err(|index| ApplyError {
            patch: position,
            index,
        })?;
    }
    Ok(root)
}

/// Fails with the child index that was out of bounds.
fn apply_one(root: &mut Node, patch: &Patch) -> Result<(), u32> {
    match patch {
        Patch::Replace { path, node } => *locate(root, path)? = node.clone(),
        Patch::SetName { path, name } => {
            Arc::make_mut(locate(root, path)?).name = name.clone();
        }
        Patch::SetProp { path, name, value } => {
            Arc::make_mut(locate(root, path)?)
                .props
                .insert(*name, value.clone());
        }
        Patch::RemoveProp { path, name } => {
            Arc::make_mut(locate(root, path)?)
                .props
                .remove(name.as_str());
        }
        Patch::SetActions { path, actions } => {
            Arc::make_mut(locate(root, path)?).actions = *actions;
        }
        Patch::Insert {
            parent,
            index,
            node,
        } => {
            let children = &mut Arc::make_mut(locate(root, parent)?).children;
            if *index as usize > children.len() {
                return Err(*index);
            }
            children.insert(*index as usize, node.clone());
        }
        Patch::Remove { parent, index } => {
            let children = &mut Arc::make_mut(locate(root, parent)?).children;
            if *index as usize >= children.len() {
                return Err(*index);
            }
            children.remove(*index as usize);
        }
        Patch::Move { parent, from, to } => {
            let children = &mut Arc::make_mut(locate(root, parent)?).children;
            let (from, to) = (*from as usize, *to as usize);
            for index in [from, to] {
                if index >= children.len() {
                    return Err(index as u32);
                }
            }
            // Rotating touches only the span between the two positions, unlike
            // a remove and insert, which both shift the whole tail.
            if from < to {
                children[from..=to].rotate_left(1);
            } else {
                children[to..=from].rotate_right(1);
            }
        }
        Patch::Reorder {
            parent,
            start,
            order,
        } => {
            let children = &mut Arc::make_mut(locate(root, parent)?).children;
            let start = *start as usize;
            let end = start + order.len();
            if end > children.len() {
                return Err(end as u32);
            }
            let mut taken: Vec<Option<Node>> = children.drain(start..end).map(Some).collect();
            let mut reordered = Vec::with_capacity(order.len());
            for &from in order {
                let child = taken.get_mut(from as usize).and_then(Option::take);
                reordered.push(child.ok_or(from)?);
            }
            children.splice(start..start, reordered);
        }
    }
    Ok(())
}

fn locate<'a>(root: &'a mut Node, path: &[u32]) -> Result<&'a mut Node, u32> {
    let mut node = root;
    for &index in path {
        node = Arc::make_mut(node)
            .children
            .get_mut(index as usize)
            .ok_or(index)?;
    }
    Ok(node)
}
