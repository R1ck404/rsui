//! What the app looks like to assistive technology, apart from how any platform asks about it.
//!
//! This costs nothing until something asks. A screen reader then gets a plain copy of the tree that's
//! safe to read from any thread, as they can ask from any thread at any time, and what it asks to
//! have done comes back to the window's thread as a command.

use std::collections::HashMap;

use rsui_core::{ActionKind, Actions, Element, Node, PropValue, Role, Text};
use rsui_layout::{Layout, ScrollInfo};

/// Something assistive technology wants done to a node.
#[derive(Debug, Clone, PartialEq)]
pub enum AccessCommand {
    /// Does what clicking it does.
    Invoke,
    Toggle,
    SetValue(String),
    Focus,
    /// Scrolls to this far along, from 0 to 1, in whichever direction it goes.
    ScrollTo(f64),
    /// Scrolls by this many small steps, which are about an arrow key.
    ScrollSteps(i32),
    /// Scrolls by this many pages.
    ScrollPages(i32),
    /// Scrolls whatever it's inside so that it shows.
    Reveal,
}

/// One node, as assistive technology sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessNode {
    /// Stays the same as long as its keys do, however the tree around it changes.
    pub id: u64,
    pub parent: Option<u32>,
    pub children: Vec<u32>,
    pub path: Vec<u32>,
    pub role: Role,
    pub name: Text,
    /// Its key, as a stable name for it that isn't meant for people.
    pub key: Option<String>,
    pub value: Option<Text>,
    /// Explains what it's for.
    pub help: Option<Text>,
    /// Left, top, width and height, in physical pixels, from the top left of the window.
    pub rect: [f32; 4],
    pub actions: Actions,
    pub checked: Option<bool>,
    pub scroll: Option<ScrollInfo>,
    /// Whether it's on top of everything, and so isn't cut off by what it's inside.
    pub overlay: bool,
    /// Whether it means something to someone, rather than only arranging what's inside it.
    pub meaningful: bool,
    /// Whether it's beyond the edges of what's around it.
    pub offscreen: bool,
}

impl AccessNode {
    pub fn offers(&self, action: ActionKind) -> bool {
        self.actions.contains(action)
    }

    /// Whether the keyboard can get to it, by the same rule as for everything else.
    pub fn focusable(&self) -> bool {
        self.role == Role::TextInput
            || self.offers(ActionKind::Click)
            || self.offers(ActionKind::Toggle)
            || self.offers(ActionKind::Focus)
            || self.offers(ActionKind::Scroll)
    }

    fn contains(&self, x: f32, y: f32) -> bool {
        let [left, top, width, height] = self.rect;
        x >= left && x < left + width && y >= top && y < top + height
    }
}

/// The whole tree, as assistive technology sees it. The root is the first node.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccessTree {
    pub nodes: Vec<AccessNode>,
    pub focused: Option<u32>,
    by_id: HashMap<u64, u32>,
}

impl AccessTree {
    pub fn find(&self, id: u64) -> Option<&AccessNode> {
        self.by_id
            .get(&id)
            .and_then(|at| self.nodes.get(*at as usize))
    }

    /// The first node with this key, which is what tests and agents tend to know something by.
    pub fn by_key(&self, key: &str) -> Option<&AccessNode> {
        self.nodes
            .iter()
            .find(|node| node.key.as_deref() == Some(key))
    }

    /// What's at this position, in physical pixels from the top left of the window.
    pub fn at_point(&self, x: f32, y: f32) -> Option<&AccessNode> {
        let root = self.nodes.first()?;
        self.deepest(0, x, y, false)
            .or_else(|| root.contains(x, y).then_some(root))
    }

    /// The deepest node at a position, with what's drawn last getting the first chance. What's on top of
    /// everything can be outside what it's in, so it has to be looked for even if that isn't here.
    fn deepest(&self, index: u32, x: f32, y: f32, only_overlays: bool) -> Option<&AccessNode> {
        let node = &self.nodes[index as usize];
        let inside = node.contains(x, y);
        for &child in node.children.iter().rev() {
            if self.nodes[child as usize].overlay {
                if let Some(found) = self.deepest(child, x, y, false) {
                    return Some(found);
                }
            } else if let Some(found) = self.deepest(child, x, y, true) {
                if found.overlay {
                    return Some(found);
                }
            }
        }
        if !inside || only_overlays && !node.overlay {
            return None;
        }
        for &child in node.children.iter().rev() {
            if let Some(found) = self.deepest(child, x, y, false) {
                return Some(found);
            }
        }
        Some(node)
    }

    /// Whether the same things are in the same places in the tree, which says nothing about their names.
    pub fn same_structure(&self, other: &Self) -> bool {
        self.nodes.len() == other.nodes.len()
            && self
                .nodes
                .iter()
                .zip(&other.nodes)
                .all(|(a, b)| a.id == b.id && a.children == b.children)
    }
}

const FNV_OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

fn mix(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// A node's id, from its parent's, and its own key if it has one, or else its position.
fn id_of(parent: u64, element: &Element, position: u32) -> u64 {
    match &element.key {
        Some(key) => mix(mix(parent, b"k"), key.to_string().as_bytes()),
        None => mix(mix(parent, b"i"), &position.to_le_bytes()),
    }
}

fn is_meaningful(element: &Element) -> bool {
    !matches!(element.role, Role::Group | Role::Window)
        || !element.name.is_empty()
        || !element.actions.is_empty()
}

fn prop_text(element: &Element, name: &str) -> Option<Text> {
    match element.props.get(name) {
        Some(PropValue::Str(text)) => Some(Text::clone(text)),
        _ => None,
    }
}

/// Copies a tree, with where everything is. `focused` is the path of what has the keyboard.
pub(crate) fn build(
    root: &Node,
    layout: &Layout,
    scale: f32,
    window: (u32, u32),
    focused: Option<&[u32]>,
) -> AccessTree {
    let mut tree = AccessTree::default();
    let mut path = Vec::new();
    let screen = [0.0, 0.0, window.0 as f32, window.1 as f32];
    add(
        &mut tree, root, layout, scale, focused, None, FNV_OFFSET, 0, &mut path, screen,
    );
    tree.by_id = tree
        .nodes
        .iter()
        .enumerate()
        .map(|(at, node)| (node.id, at as u32))
        .collect();
    tree
}

#[allow(clippy::too_many_arguments)]
fn add(
    tree: &mut AccessTree,
    element: &Node,
    layout: &Layout,
    scale: f32,
    focused: Option<&[u32]>,
    parent: Option<u32>,
    parent_id: u64,
    position: u32,
    path: &mut Vec<u32>,
    visible: [f32; 4],
) -> u32 {
    let rect = layout.rect(path).map_or([0.0; 4], |rect| {
        [
            rect.x * scale,
            rect.y * scale,
            rect.width * scale,
            rect.height * scale,
        ]
    });
    let overlay = layout.is_overlay(path);
    let scroll = layout.scroll_info(path, element);
    let visible = if overlay {
        [0.0, 0.0, f32::MAX, f32::MAX]
    } else {
        visible
    };
    let offscreen = rect[0] >= visible[0] + visible[2]
        || rect[1] >= visible[1] + visible[3]
        || rect[0] + rect[2] <= visible[0]
        || rect[1] + rect[3] <= visible[1];
    let inner = if scroll.is_some() {
        // What's inside only shows within this.
        let left = rect[0].max(visible[0]);
        let top = rect[1].max(visible[1]);
        let right = (rect[0] + rect[2]).min(visible[0] + visible[2]);
        let bottom = (rect[1] + rect[3]).min(visible[1] + visible[3]);
        [left, top, (right - left).max(0.0), (bottom - top).max(0.0)]
    } else {
        visible
    };
    let id = id_of(parent_id, element, position);
    let index = tree.nodes.len() as u32;
    if focused == Some(path.as_slice()) {
        tree.focused = Some(index);
    }
    // This shares the text rather than copying it, as it happens on every key, and it could be long.
    let value = (element.role == Role::TextInput).then(|| {
        // What reads this out loud is told what shows, rather than what a password is.
        if let Some(mask) = rsui_render::password_mask(element) {
            return Text::from(mask);
        }
        match element.props.get("value") {
            Some(PropValue::Str(value)) => (**value).clone(),
            _ => Text::new(),
        }
    });
    tree.nodes.push(AccessNode {
        id,
        parent,
        children: Vec::new(),
        path: path.clone(),
        role: element.role,
        name: element.name.clone(),
        key: element.key.as_ref().map(ToString::to_string),
        value,
        help: prop_text(element, "tooltip"),
        rect,
        actions: element.actions,
        checked: match element.props.get("checked") {
            Some(PropValue::Bool(checked)) => Some(*checked),
            _ => None,
        },
        scroll,
        overlay,
        meaningful: is_meaningful(element),
        offscreen,
    });
    let mut children = Vec::with_capacity(element.children.len());
    for (position, child) in element.children.iter().enumerate() {
        // On its way out, and it's already gone as far as anyone using it is concerned.
        if rsui_layout::is_inert(child) {
            continue;
        }
        path.push(position as u32);
        children.push(add(
            tree,
            child,
            layout,
            scale,
            focused,
            Some(index),
            id,
            position as u32,
            path,
            inner,
        ));
        path.pop();
    }
    tree.nodes[index as usize].children = children;
    index
}
