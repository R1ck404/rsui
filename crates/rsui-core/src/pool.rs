use triomphe::Arc;

use crate::element::{Element, Node, Role};

/// Reuses the allocations of dead trees, which is much cheaper than asking the
/// allocator for new ones when large parts of the UI are rebuilt.
///
/// Nodes that something else still holds are left alone, so recycling is always
/// safe, including for trees that share subtrees with the current one.
///
/// Free nodes still occupy memory. Call [`shrink_to`](Self::shrink_to) once a
/// large rebuild is over to give it back.
#[derive(Default)]
pub struct NodePool {
    free: Vec<Node>,
}

impl NodePool {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many nodes are ready to be reused.
    pub fn len(&self) -> usize {
        self.free.len()
    }

    pub fn is_empty(&self) -> bool {
        self.free.is_empty()
    }

    /// Takes back every node of `root` that nothing else holds.
    pub fn recycle(&mut self, mut root: Node) {
        if let Some(element) = Arc::get_mut(&mut root) {
            for child in element.children.drain(..) {
                self.recycle(child);
            }
            element.reset();
            self.free.push(root);
        }
    }

    /// Makes a node, reusing an allocation if one is free. `fill` receives an
    /// element with only its role set, as [`Element::new`] would make it.
    pub fn node(&mut self, role: Role, fill: impl FnOnce(&mut Element)) -> Node {
        let mut node = self
            .free
            .pop()
            .unwrap_or_else(|| Arc::new(Element::new(Role::Group)));
        let element = Arc::get_mut(&mut node).expect("nothing else holds a free node");
        element.role = role;
        fill(element);
        element.shrink_to_fit();
        node
    }

    /// Frees all but `count` of the free nodes, and the pool's spare memory.
    pub fn shrink_to(&mut self, count: usize) {
        self.free.truncate(count);
        self.free.shrink_to_fit();
    }
}
