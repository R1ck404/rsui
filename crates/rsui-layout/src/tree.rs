use std::fmt::Write;

use rsui_core::{Element, Node, Patch};
use thin_vec::ThinVec;
use crate::hit_region::{HitProbe, NoProbe, RegionProbe};

use crate::engine::{self, Axis, Key};
use crate::measure::Measure;
use crate::style::{Style, is_layout_prop, is_paint_only_prop};

/// A width and a height, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// A position and size, in pixels. `x` and `y` are from the top left of the window.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// This box has changed, so its results can't be reused.
pub(crate) const SELF_DIRTY: u32 = 1 << 0;
/// A box somewhere below this one has changed.
pub(crate) const CHILD_DIRTY: u32 = 1 << 1;
pub(crate) const HAS_LAYOUT: u32 = 1 << 2;
pub(crate) const HAS_MEASURE: u32 = 1 << 3;
pub(crate) const LAYOUT_WIDTH_EXACT: u32 = 1 << 4;
pub(crate) const LAYOUT_HEIGHT_EXACT: u32 = 1 << 5;
pub(crate) const MEASURE_WIDTH_EXACT: u32 = 1 << 6;
pub(crate) const MEASURE_HEIGHT_EXACT: u32 = 1 << 7;
/// This box's parent asked it more than one question that depends on its content, and
/// it only remembers one. So if its content changes, it can't tell whether the parent
/// would see a difference, and must assume so.
pub(crate) const FORGOT_A_QUESTION: u32 = 1 << 8;
/// More than one child has changed, so they all need checking.
pub(crate) const MANY_CHILDREN_DIRTY: u32 = 1 << 9;
/// This box's content can be bigger than it is, as a scrolling area or a list.
pub(crate) const SCROLLS: u32 = 1 << 10;
/// This box is on top of everything, out of its parent's way.
pub(crate) const IS_OVERLAY: u32 = 1 << 11;
/// A box below this one is on top of everything, which is how to find them quickly.
pub(crate) const CONTAINS_OVERLAY: u32 = 1 << 12;
/// What's on top of everything that the pointer goes straight through. See [`crate::is_inert`].
pub(crate) const OVERLAY_IS_INERT: u32 = 1 << 13;
/// What's inside this is the parts of a paragraph, which aren't visited or hit as boxes.
pub(crate) const PARAGRAPH: u32 = 1 << 14;
/// Placed by its own `top` and `left` and so on, so that it takes up no room among what's around it.
pub(crate) const IS_ABSOLUTE: u32 = 1 << 15;
/// Some of its children say how far up they go, so they're drawn and found in that order rather than the order they're in.
pub(crate) const STACKED: u32 = 1 << 16;
/// The margin before what's first in it goes out of it, to what's around it, as a block's does in CSS, so it leaves that out.
pub(crate) const TOP_ESCAPES: u32 = 1 << 17;
/// The same for the margin after what's last in it.
pub(crate) const BOTTOM_ESCAPES: u32 = 1 << 18;
/// It has no box of its own, as CSS's `display: contents` says: what's in it is laid out as part of what it's in. It covers what
/// it's in, at its top left, so what's in it is where that put it, and it isn't anything itself, to find at a point or to reach.
pub(crate) const CONTENTS: u32 = 1 << 19;
/// Ordinary paint only, throughout this subtree. Recomputed with dirty layout nodes;
/// a clear bit means the painter must retain the original traversal.
pub(crate) const CLIPPED_PAINT_SAFE: u32 = 1 << 20;
/// A strict descendant is absolute along a path with no overlay boxes. This
/// describes lifted discovery, not geometry, and does not include this box itself.
pub(crate) const CONTAINS_ABSOLUTE: u32 = 1 << 21;
/// Placed by itself and inert, so the pointer goes through it and what's in it, as through what's drawn over an editor.
pub(crate) const ABSOLUTE_IS_INERT: u32 = 1 << 22;
/// Placed by itself and said to take the pointer (CSS's `pointer-events: auto` or `all`), which it does even in what's inert, as
/// input-otp's `<input>` does in its `pointer-events: none` box.
pub(crate) const ABSOLUTE_TAKES_POINTER: u32 = 1 << 27;
/// What a scroller's content is laid out in took a share of how tall the scroller shows (see `VIEW_HEIGHT` in style.rs), which
/// isn't in what it was asked, so what it answered is worked out again each time the scroller is.
pub(crate) const USES_VIEW_HEIGHT: u32 = 1 << 28;
/// How tall it's asked to be is definite, as CSS has it (CSS 2.1 §10.5, CSS Flexbox §9.8): it says how tall it is, or it's
/// flexed in a flex box that's as tall as that says, or stretched across a row that is. One in a block, as tall as what's in it,
/// isn't, though it's laid out at that height. Said by what it's in, as it starts it (`start_item`).
pub(crate) const DEFINITE_HEIGHT: u32 = 1 << 29;
/// When this scrolling area was last laid out, a scrollbar that takes room (`scrollbar_room`) was down its side, or along its
/// bottom. Laying it out again starts from those, as a browser keeps the scrollbars it had: starting with none each time laid
/// what's in it out at its full width, found it too tall, and laid all of it out again at the width the scrollbar leaves, so
/// that each change of a long list's scroll area laid out every row in it twice.
pub(crate) const BAR_DOWN: u32 = 1 << 30;
pub(crate) const BAR_ACROSS: u32 = 1 << 31;
/// All that's changed about this box itself since it was laid out is that children have been put in it or taken out, so
/// where the rest of them go can follow from where they were: see `solve_moved` in the engine.
pub(crate) const SPLICED: u32 = 1 << 23;
/// Something just before this was taken out of what it's in, so where it goes has to be worked out again, with what's around it.
pub(crate) const SHIFTED: u32 = 1 << 24;
/// Everything in this, itself included, draws only where its own box is, if at all, as far as what's drawn is concerned: nothing
/// in it is turned, faded, filtered, shadowed, outlined or placed by itself, and nothing in it is on top of everything. So when
/// none of their boxes is where anything shows, none of it needs visiting: see [`LayoutVisitor::draws_nothing_at`]. Worked out
/// with [`CLIPPED_PAINT_SAFE`], so it's as up to date as that is; a box that hasn't been laid out doesn't have it.
pub(crate) const PAINTS_IN_OWN_BOXES: u32 = 1 << 25;
/// When this was last laid out, its node had at least one of [`ONE_PIECE_PROPS`], so it was drawn as one piece. What layout
/// makes of those is only whether any is there (in [`drawn_as_one`], and as names in the flags above), never what they say, so
/// while one is there, changing any of them to another value changes nothing that layout worked out: that's what an animated
/// spinner or a fade does at every frame, and laying the page out again for it was most of what each frame cost.
pub(crate) const ONE_PIECE: u32 = 1 << 26;

/// The props that have something drawn as one piece, turned, resized or faded: see [`drawn_as_one`] and [`ONE_PIECE`].
const ONE_PIECE_PROPS: [&str; 6] = ["opacity", "scale", "scale_x", "scale_y", "rotate", "transform"];

/// Whether this is one of [`ONE_PIECE_PROPS`].
pub(crate) fn is_one_piece_prop(name: rsui_core::Symbol) -> bool {
    static NAMES: std::sync::LazyLock<[rsui_core::Symbol; 6]> =
        std::sync::LazyLock::new(|| ONE_PIECE_PROPS.map(rsui_core::Symbol::new));
    NAMES.contains(&name)
}

/// Whether a prop can make something draw outside its own box, or change how what's in it is drawn, or where or when that's
/// visited: [`PAINTS_IN_OWN_BOXES`] isn't for what has any of these. rsui-render checks that each of the painter's own props of
/// that kind is here, so that a new one can't be left out.
pub fn moves_paint(name: rsui_core::Symbol) -> bool {
    static NAMES: std::sync::LazyLock<Vec<rsui_core::Symbol>> = std::sync::LazyLock::new(|| {
        MOVES_PAINT.iter().map(|name| rsui_core::Symbol::new(name)).collect()
    });
    NAMES.contains(&name)
}

/// The names [`moves_paint`] says yes to.
pub const MOVES_PAINT: &[&str] = &[
    // Turned or resized, which is drawn wherever that puts it.
    "scale",
    "scale_x",
    "scale_y",
    "rotate",
    "transform",
    // Faded, or filtered, which what's in it is drawn as one piece for.
    "opacity",
    "disabled",
    "css_filter",
    // Drawn around it, past its edges, or cut to a shape.
    "shadows",
    "shadow_blur",
    "text_shadow",
    "outline_width",
    "clip_path",
    // Placed by itself, or raised, which is drawn after the rest, with what it's in cutting it off differently.
    "position",
    "sticky_top",
    "sticky_bottom",
    "z_index",
    "overlay",
];

/// Whether something that scrolls across lays out what's in it as wide as itself, as CSS has it, rather than as wide as that wants.
pub(crate) fn fills_across(element: &Element) -> bool {
    matches!(
        element.props.get_symbol(crate::style::NAMES.fill_across),
        Some(rsui_core::PropValue::Bool(true))
    )
}

/// Whether this is what a page's scroller lays out what's in it in, which is no box of its own on a page (see web.js): a height
/// that's a share, in it, is of how tall the scroller shows, as it would be of the scroller's content box.
pub(crate) fn is_scroll_content(element: &rsui_core::Element) -> bool {
    matches!(element.props.get_symbol(crate::style::NAMES.scroll_content), Some(rsui_core::PropValue::Bool(true)))
}

/// How far right and down what's in a box with none of its own (`display: contents`) reaches, from where it is.
fn contents_reach(current: &LayoutBox) -> (f32, f32) {
    current
        .children
        .iter()
        .filter(|child| !child.has(IS_OVERLAY))
        .fold((0.0, 0.0), |(right, bottom): (f32, f32), child| {
            let (width, height) = if child.has(CONTENTS) {
                contents_reach(child)
            } else {
                (child.width, child.height)
            };
            (right.max(child.x + width), bottom.max(child.y + height))
        })
}

/// How far right anything in this reaches, from its left, as what's in something can be wider than it is. `element` is what
/// `current` is the box of.
pub(crate) fn furthest_right(current: &LayoutBox, element: &rsui_core::Element) -> f32 {
    current
        .children
        .iter()
        .zip(&element.children)
        .filter(|(child, _)| !child.has(IS_OVERLAY))
        // What scrolls keeps what's in it inside it, however wide that is, and so does what cuts it off at its edges, as
        // `overflow: hidden` does: a carousel's slides past its end make the page no wider (shadcn/ui's Carousel).
        .map(|(child, child_element)| {
            // Something with nothing inside its edges, as a box with no width or height, doesn't reach anywhere itself, as a
            // browser has it, though what's in it can.
            let own = if child.width > 0.0 && child.height > 0.0 && !child.has(CONTENTS) {
                child.width
            } else {
                0.0
            };
            let reach = if child.has(SCROLLS) || clips(child_element) {
                own
            } else {
                own.max(furthest_right(child, child_element))
            };
            if reach > 0.0 { child.x + reach } else { 0.0 }
        })
        .fold(0.0, f32::max)
}

/// How far left anything in this reaches past its left, as what's in something wider than it is that starts at its right does
/// where text goes from right to left: less than 0 then, and 0 if nothing does. As [`furthest_right`] has it otherwise.
pub(crate) fn furthest_left(current: &LayoutBox, element: &rsui_core::Element) -> f32 {
    leftmost(current, element).min(0.0)
}

/// How far left anything in this reaches, from its left: infinity if there's nothing in it.
fn leftmost(current: &LayoutBox, element: &rsui_core::Element) -> f32 {
    current
        .children
        .iter()
        .zip(&element.children)
        .filter(|(child, _)| !child.has(IS_OVERLAY))
        .map(|(child, child_element)| {
            let own = if child.width > 0.0 && child.height > 0.0 && !child.has(CONTENTS) { 0.0 } else { f32::INFINITY };
            let inside = if child.has(SCROLLS) || clips(child_element) { f32::INFINITY } else { leftmost(child, child_element) };
            let reach = own.min(inside);
            if reach.is_finite() { child.x + reach } else { f32::INFINITY }
        })
        .fold(f32::INFINITY, f32::min)
}

/// How many things a box has to have in it for [`Layout::near_view`] to look in only those of them that are near being seen.
const LONG_NEAR_FROM: usize = 64;

/// How many boxes [`Layout::draws_nothing`] looks at, at most, to find that something can't be seen.
const UNSEEN_BOXES_LOOKED_AT: u32 = 64;

/// What's placed by itself while the tree's being visited, to be visited after everything else: where each is, the path to where
/// the visit's up to, and how many of what that's in are drawn as one piece, inside which nothing's left for later.
#[derive(Default)]
struct Lift {
    later: Vec<Vec<u32>>,
    path: Vec<u32>,
    apart: u32,
}

/// Whether something cuts off what's in it at its edges, as CSS's `overflow: hidden` does.
/// A point taken through what `undo` says, as `[a, b, c, d, e, f]`, or as it is if it says nothing.
fn undone(undo: Option<[f32; 6]>, x: f32, y: f32) -> (f32, f32) {
    match undo {
        Some([a, b, c, d, e, f]) => (a * x + c * y + e, b * x + d * y + f),
        None => (x, y),
    }
}

/// Whether this is an element of a page, as `rsui/web` makes them: one with a `dom_key`, or whose own key is a page's. What
/// holds a page (its window, with the system's rounded corners) isn't.
fn is_page_element(node: &Element) -> bool {
    static DOM_KEY: std::sync::LazyLock<rsui_core::Symbol> = std::sync::LazyLock::new(|| rsui_core::Symbol::new("dom_key"));
    node.props.get_symbol(*DOM_KEY).is_some()
        || matches!(&node.key, Some(rsui_core::Key::Str(key)) if key.starts_with("__dom"))
}

/// An element's rounded corners, as `[across, down]` from the top left going round, made to fit a box this big as CSS has it
/// (CSS Backgrounds 3, 5.5: all of them made smaller by as much as the two along the side they overlap most on need), from its
/// `corner_radii` (in pixels or shares of the box) or its `radius`. `None` if it has none.
fn corners_of(node: &Element, w: f32, h: f32) -> Option<[[f32; 2]; 4]> {
    use rsui_core::PropValue;
    static RADII: std::sync::LazyLock<rsui_core::Symbol> = std::sync::LazyLock::new(|| rsui_core::Symbol::new("corner_radii"));
    static RADIUS: std::sync::LazyLock<rsui_core::Symbol> = std::sync::LazyLock::new(|| rsui_core::Symbol::new("radius"));
    static CORNER: std::sync::LazyLock<rsui_core::Symbol> = std::sync::LazyLock::new(|| rsui_core::Symbol::new("corner_radius"));
    let mut corners = match node.props.get_symbol(*RADII) {
        Some(PropValue::Str(said)) => {
            let mut parts = [0f32; 8];
            let mut count = 0;
            for (at, word) in said.split_whitespace().take(8).enumerate() {
                let whole = if at % 2 == 0 { w } else { h };
                parts[at] = match word.strip_suffix('%') {
                    Some(share) => share.parse::<f32>().map_or(0.0, |share| share / 100.0 * whole),
                    None => word.trim_end_matches("px").parse::<f32>().unwrap_or(0.0),
                };
                count += 1;
            }
            if count < 8 {
                return None;
            }
            [[parts[0], parts[1]], [parts[2], parts[3]], [parts[4], parts[5]], [parts[6], parts[7]]]
        }
        _ => {
            let radius = match node.props.get_symbol(*RADIUS).or_else(|| node.props.get_symbol(*CORNER)) {
                Some(PropValue::Float(radius)) => *radius as f32,
                Some(PropValue::Int(radius)) => *radius as f32,
                _ => return None,
            };
            [[radius; 2]; 4]
        }
    };
    let mut times = 1f32;
    for (one, two, along, whole) in [(0, 1, 0, w), (3, 2, 0, w), (0, 3, 1, h), (1, 2, 1, h)] {
        let sum = corners[one][along] + corners[two][along];
        if sum > whole && sum > 0.0 {
            times = times.min(whole / sum);
        }
    }
    for corner in &mut corners {
        corner[0] = (corner[0] * times).max(0.0);
        corner[1] = (corner[1] * times).max(0.0);
    }
    corners.iter().any(|corner| corner[0] > 0.0 && corner[1] > 0.0).then_some(corners)
}

/// Whether a point, from a box's top left, is inside that box with these corners (see [`corners_of`]).
fn inside_corners(corners: &[[f32; 2]; 4], x: f32, y: f32, w: f32, h: f32) -> bool {
    // Each corner's ellipse, its middle, and which side of it the corner is on.
    let at = [
        (corners[0], corners[0][0], corners[0][1], -1.0, -1.0),
        (corners[1], w - corners[1][0], corners[1][1], 1.0, -1.0),
        (corners[2], w - corners[2][0], h - corners[2][1], 1.0, 1.0),
        (corners[3], corners[3][0], h - corners[3][1], -1.0, 1.0),
    ];
    for ([rx, ry], cx, cy, sx, sy) in at {
        if rx <= 0.0 || ry <= 0.0 {
            continue;
        }
        let (dx, dy) = (x - cx, y - cy);
        // Only the corner's own square, past the middle towards it, is curved.
        if dx * sx > 0.0 && dy * sy > 0.0 {
            let (nx, ny) = (dx / rx, dy / ry);
            if nx * nx + ny * ny > 1.0 {
                return false;
            }
        }
    }
    true
}

pub fn clips(node: &rsui_core::Element) -> bool {
    static CLIP: std::sync::LazyLock<rsui_core::Symbol> =
        std::sync::LazyLock::new(|| rsui_core::Symbol::new("clip"));
    node.props.get_symbol(*CLIP) == Some(&rsui_core::PropValue::Bool(true))
}

/// Whether something's placed where it would be, but drawn with what's placed by itself, as CSS's `position: relative` and
/// `sticky` are.
fn placed(node: &Node) -> bool {
    if node.props.is_empty() {
        return false;
    }
    let names = &*crate::style::NAMES;
    matches!(
        node.props.get_symbol(names.position),
        Some(rsui_core::PropValue::Str(how)) if matches!(how.as_str(), "relative" | "sticky")
    ) || node.props.get_symbol(names.sticky_top).is_some()
        || node.props.get_symbol(names.sticky_bottom).is_some()
}

/// Which of what's around what's placed by itself at `path` cut it off at their edges, as CSS's `overflow: hidden` does, one for
/// each step of the path (what it's in, outermost first). Something `position: relative` (or sticky) is always cut off by what
/// it's in, as it's drawn where it would be. Something `position: absolute` isn't by what's around the box it's placed in (its
/// containing block, the nearest positioned thing around it): a box that clips cuts it off only when that box, or something
/// positioned inside it, is what it's placed in, as CSS has it.
fn cut_off_by(root_box: &LayoutBox, root: &Node, path: &[u32]) -> Vec<bool> {
    // Each step's box and node, and then the one at the end.
    let mut chain: Vec<(&LayoutBox, &Node)> = Vec::with_capacity(path.len() + 1);
    let (mut current, mut node) = (root_box, root);
    chain.push((current, node));
    for &index in path {
        let (Some(child_box), Some(child)) = (
            current.children.get(index as usize),
            node.children.get(index as usize),
        ) else {
            return Vec::new();
        };
        (current, node) = (child_box, child);
        chain.push((current, node));
    }
    let positioned = |(one_box, one): &(&LayoutBox, &Node)| one_box.has(IS_ABSOLUTE) || placed(one);
    (0..path.len())
        .map(|at| {
            let (_, around) = chain[at];
            if around.props.is_empty() || !clips(around) {
                return false;
            }
            // An absolutely placed box after it whose containing block is outside it (nothing positioned from it down to that
            // box) isn't cut off by it, nor is what's in that box.
            let mut contained = positioned(&chain[at]);
            for step in &chain[at + 1..] {
                if step.0.has(IS_ABSOLUTE) && !contained {
                    return false;
                }
                contained = contained || positioned(step);
            }
            true
        })
        .collect()
}

/// Whether something is drawn as one piece, faded or turned, which what's in it has to be drawn with.
fn drawn_as_one(node: &Element) -> bool {
    ONE_PIECE_PROPS
        .iter()
        .any(|name| node.props.get(name).is_some())
}

/// Whether the node at `path` in `root` has any of [`ONE_PIECE_PROPS`].
fn was_drawn_as_one(root: &Node, path: &[u32]) -> bool {
    let mut node = root;
    for &index in path {
        match node.children.get(index as usize) {
            Some(child) => node = child,
            None => return false,
        }
    }
    drawn_as_one(node)
}

/// Whether anything in a scrolling area sticks to it as it's scrolled, apart from in what scrolls by itself inside it, which has
/// its own view, as `stick` goes through them.
fn sticks_inside(element: &Element) -> bool {
    let names = &*crate::style::NAMES;
    element.children.iter().any(|child| {
        child.props.get_symbol(names.sticky_top).is_some()
            || child.props.get_symbol(names.sticky_bottom).is_some()
            || (child.props.get_symbol(names.scroll_y).is_none() && sticks_inside(child))
    })
}

/// The `z_index` of what's at `path`, if it says one.
fn z_index_at(root: &Node, path: &[u32]) -> Option<f64> {
    let mut node = root;
    for &index in path {
        node = node.children.get(index as usize)?;
    }
    match node.props.get_symbol(crate::style::NAMES.z_index) {
        Some(rsui_core::PropValue::Int(z)) => Some(*z as f64),
        Some(rsui_core::PropValue::Float(z)) => Some(*z),
        _ => None,
    }
}

/// A number a node says by this name, if it says one.
fn number_prop(node: &Node, name: rsui_core::Symbol) -> Option<f64> {
    match node.props.get_symbol(name) {
        Some(rsui_core::PropValue::Int(z)) => Some(*z as f64),
        Some(rsui_core::PropValue::Float(z)) => Some(*z),
        _ => None,
    }
}

/// Whether something says it goes below what's around it, with a `z_index` under nothing.
fn below_the_rest(node: &Node) -> bool {
    match node.props.get_symbol(crate::style::NAMES.z_index) {
        Some(rsui_core::PropValue::Int(z)) => *z < 0,
        Some(rsui_core::PropValue::Float(z)) => *z < 0.0,
        _ => false,
    }
}

/// The order to draw what's in `node`: by its `z_index`, lowest first, and otherwise in the order it's in.
/// The order a box's children get the chance to be what's at a point, the one drawn on top first, as CSS paints them: what's
/// placed by itself (`position: absolute`) over what's in the flow, the last of those on top, then the rest as they come. A
/// sheet's close button, placed in its top corner, is over its header, which is in the flow under it.
fn hit_order(current: &LayoutBox) -> impl Iterator<Item = usize> + '_ {
    let placed = current.has(CONTAINS_ABSOLUTE);
    let lifted = current.children.iter().enumerate().rev().filter(move |(_, child)| placed && child.has(IS_ABSOLUTE));
    let flowing = current.children.iter().enumerate().filter(move |(_, child)| !placed || !child.has(IS_ABSOLUTE));
    lifted.chain(flowing).map(|(index, _)| index)
}

fn stacking_order(node: &Node) -> Vec<usize> {
    let name = crate::style::NAMES.z_index;
    let z = |child: &Node| match child.props.get_symbol(name) {
        Some(rsui_core::PropValue::Int(z)) => *z as f64,
        Some(rsui_core::PropValue::Float(z)) => *z,
        _ => 0.0,
    };
    let mut order: Vec<usize> = (0..node.children.len()).collect();
    order.sort_by(|&a, &b| z(&node.children[a]).total_cmp(&z(&node.children[b])));
    order
}

/// The layout of one node. A box doesn't refer to its node, so holding on to boxes
/// doesn't keep nodes alive, or stop the runtime from updating them in place.
#[derive(Default, Clone)]
pub(crate) struct LayoutBox {
    /// Relative to the parent, so moving a box doesn't touch anything inside it.
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    /// The available space that produced `width` and `height`.
    pub(crate) layout_key: [f32; 2],
    /// The most recent question about this box's size, and its answer. A parent asks
    /// this before deciding a child's final size, so keeping both answers stops the
    /// work from doubling at every level of nesting.
    pub(crate) measure_key: [f32; 2],
    pub(crate) measured: [f32; 2],
    /// Which child changed, unless [`MANY_CHILDREN_DIRTY`] is set. Usually just one has,
    /// and this saves looking through all of them to find it.
    pub(crate) dirty_child: u32,
    pub(crate) flags: u32,
    pub(crate) children: ThinVec<LayoutBox>,
    /// Other questions about its size it's been asked and their answers, beyond the two above, kept from one layout to the next
    /// until it changes: what a flex or grid box is in asks it several (how wide at least, at most, then the size it's given). With
    /// only one kept, a change anywhere inside had to be taken to change the answer to a question that was forgotten, so what it's
    /// in laid out everything in it again, and that went on up to the window: a click in the Mantine kitchen sink laid out all of
    /// its 2,700 boxes, twice. Empty for most boxes, which takes no more than a pointer.
    pub(crate) answers: ThinVec<Answer>,
}

/// A question about a box's size, as a [`Key`]'s two lengths and whether each is exact, and its answer.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Answer {
    key: [f32; 2],
    exact: [bool; 2],
    size: [f32; 2],
}

impl Answer {
    pub(crate) fn key(&self) -> Key {
        Key {
            width: Axis { value: self.key[0], exact: self.exact[0] },
            height: Axis { value: self.key[1], exact: self.exact[1] },
        }
    }

    pub(crate) fn size(&self) -> Size {
        Size::new(self.size[0], self.size[1])
    }
}

/// How many more answers a box keeps. One that's been asked more than this is taken to have forgotten a question, as before.
const MOST_ANSWERS: usize = 8;

impl LayoutBox {
    pub(crate) fn has(&self, flag: u32) -> bool {
        self.flags & flag != 0
    }

    /// Whether this is placed by itself and lets the pointer through to what's under it, with what's in it, as
    /// `pointer-events: none` on Mantine's input sections (a Select's chevron, a left icon) has it. It's passed over wherever
    /// a point is looked for, whether what's beside it is tried first or, with a `z-index`, it is.
    pub(crate) fn lets_pointer_through(&self) -> bool {
        self.flags & (IS_ABSOLUTE | ABSOLUTE_IS_INERT) == IS_ABSOLUTE | ABSOLUTE_IS_INERT
    }

    pub(crate) fn contributes_absolute(&self) -> bool {
        !self.has(IS_OVERLAY) && self.has(IS_ABSOLUTE | CONTAINS_ABSOLUTE)
    }

    /// Recomputed after children and their parent-owned positioning flags settle.
    /// Overlay subtrees are excluded even when the overlay is pointer-inert.
    pub(crate) fn update_contains_absolute(&mut self) {
        if self.children.iter().any(Self::contributes_absolute) {
            self.flags |= CONTAINS_ABSOLUTE;
        } else {
            self.flags &= !CONTAINS_ABSOLUTE;
        }
    }

    /// Notes whether any box below this one is on top of everything.
    pub(crate) fn update_contains_overlay(&mut self) {
        if self
            .children
            .iter()
            .any(|child| child.has(IS_OVERLAY | CONTAINS_OVERLAY))
        {
            self.flags |= CONTAINS_OVERLAY;
        } else {
            self.flags &= !CONTAINS_OVERLAY;
        }
    }

    /// A positive, deliberately narrow proof. Unknown paint/resource/positioning
    /// properties keep their callbacks, including future features. No geometry is
    /// inferred from a parent's box: the visitor must also prove an empty clip.
    pub(crate) fn update_clipped_paint(&mut self, node: &Element) {
        use rsui_core::Role;
        let safe = !self.has(SCROLLS | PARAGRAPH | CONTENTS | CONTAINS_OVERLAY)
            && matches!(node.role, Role::Group | Role::Text | Role::Button)
            && node.props.iter().all(|(name, _)| {
                matches!(
                    name.as_str(),
                    "width"
                        | "height"
                        | "min_width"
                        | "max_width"
                        | "min_height"
                        | "max_height"
                        | "flex"
                        | "flex_basis"
                        | "flex_shrink"
                        | "direction"
                        | "align"
                        | "justify"
                        | "gap"
                        | "padding"
                        | "padding_left"
                        | "padding_right"
                        | "padding_top"
                        | "padding_bottom"
                        | "margin"
                        | "margin_left"
                        | "margin_right"
                        | "margin_top"
                        | "margin_bottom"
                        | "clip"
                        | "background"
                        | "hover_background"
                        | "pressed_background"
                        | "text_color"
                        | "font_size"
                        | "font_family"
                        | "font_weight"
                        | "italic"
                        | "line_height"
                        | "text_align"
                        | "no_wrap"
                        | "ellipsis"
                        | "letter_spacing"
                        | "word_spacing"
                        | "border_width"
                        | "border_color"
                        | "corner_radius"
                )
            })
            && self
                .children
                .iter()
                .all(|child| child.has(CLIPPED_PAINT_SAFE) && !child.has(IS_ABSOLUTE | IS_OVERLAY));
        if safe {
            self.flags |= CLIPPED_PAINT_SAFE;
        } else {
            self.flags &= !CLIPPED_PAINT_SAFE;
        }
        // What's in a paragraph is drawn with it, as parts of its text, so only the paragraph's own box counts.
        let in_own_boxes = !self.has(CONTAINS_OVERLAY | CONTAINS_ABSOLUTE)
            && node.props.iter().all(|(name, _)| !moves_paint(name))
            && (self.has(PARAGRAPH)
                || self.children.iter().all(|child| {
                    child.flags & (PAINTS_IN_OWN_BOXES | IS_ABSOLUTE | IS_OVERLAY) == PAINTS_IN_OWN_BOXES
                }));
        if in_own_boxes {
            self.flags |= PAINTS_IN_OWN_BOXES;
        } else {
            self.flags &= !PAINTS_IN_OWN_BOXES;
        }
        if !node.props.is_empty() && drawn_as_one(node) {
            self.flags |= ONE_PIECE;
        } else {
            self.flags &= !ONE_PIECE;
        }
    }

    pub(crate) fn layout_key(&self) -> Key {
        Key {
            width: Axis {
                value: self.layout_key[0],
                exact: self.has(LAYOUT_WIDTH_EXACT),
            },
            height: Axis {
                value: self.layout_key[1],
                exact: self.has(LAYOUT_HEIGHT_EXACT),
            },
        }
    }

    pub(crate) fn measure_key(&self) -> Key {
        Key {
            width: Axis {
                value: self.measure_key[0],
                exact: self.has(MEASURE_WIDTH_EXACT),
            },
            height: Axis {
                value: self.measure_key[1],
                exact: self.has(MEASURE_HEIGHT_EXACT),
            },
        }
    }

    pub(crate) fn store_layout(&mut self, key: Key, size: Size) {
        // This answer may also have served as the answer to a question about size, and
        // it depended on the content unless it was given its size in both directions.
        if self.has(HAS_LAYOUT) {
            let old = self.layout_key();
            if old != key && !(old.width.exact && old.height.exact) {
                self.keep_answer(old, Size::new(self.width, self.height));
            }
        }
        self.width = size.width;
        self.height = size.height;
        self.layout_key = [key.width.value, key.height.value];
        self.flags &= !(LAYOUT_WIDTH_EXACT | LAYOUT_HEIGHT_EXACT);
        self.flags |= HAS_LAYOUT;
        if key.width.exact {
            self.flags |= LAYOUT_WIDTH_EXACT;
        }
        if key.height.exact {
            self.flags |= LAYOUT_HEIGHT_EXACT;
        }
    }

    pub(crate) fn store_measure(&mut self, key: Key, size: Size) {
        self.measured = [size.width, size.height];
        self.measure_key = [key.width.value, key.height.value];
        self.flags &= !(MEASURE_WIDTH_EXACT | MEASURE_HEIGHT_EXACT);
        self.flags |= HAS_MEASURE;
        if key.width.exact {
            self.flags |= MEASURE_WIDTH_EXACT;
        }
        if key.height.exact {
            self.flags |= MEASURE_HEIGHT_EXACT;
        }
    }

    pub(crate) fn clear_results(&mut self) {
        self.flags &= !(HAS_LAYOUT | HAS_MEASURE | FORGOT_A_QUESTION);
        if !self.answers.is_empty() {
            self.answers = ThinVec::new();
        }
    }

    /// Keeps the answer to a question that's about to be put out of one of the two places for them, as it may be asked again.
    pub(crate) fn keep_answer(&mut self, key: Key, size: Size) {
        let answer = Answer {
            key: [key.width.value, key.height.value],
            exact: [key.width.exact, key.height.exact],
            size: [size.width, size.height],
        };
        if let Some(kept) = self.answers.iter_mut().find(|kept| kept.key() == key) {
            *kept = answer;
            return;
        }
        if self.answers.len() >= MOST_ANSWERS {
            self.answers.remove(0);
            self.flags |= FORGOT_A_QUESTION;
        }
        self.answers.push(answer);
    }

    /// The answer kept to this question, if there is one.
    #[inline]
    pub(crate) fn kept_answer(&self, key: Key) -> Option<Size> {
        self.answers.iter().find(|kept| kept.key() == key).map(Answer::size)
    }

    /// Its old results stay until the next computation, which needs to know what this
    /// box was asked before, to find out whether the answers have changed.
    fn mark_dirty(&mut self) {
        self.flags = (self.flags | SELF_DIRTY) & !SPLICED;
    }

    /// The same, for children having been put in it or taken out, which, if that's all that's changed about it, is noted.
    fn mark_spliced(&mut self) {
        if !self.has(SELF_DIRTY) {
            self.flags |= SPLICED;
        }
        self.flags |= SELF_DIRTY;
    }
}

// Every node on screen has a box, so a larger one costs real memory. The kept answers' pointer is 8 bytes of it, which saves laying out
// a whole page again when something deep in a flex or grid box changes.
const _: () = assert!(size_of::<LayoutBox>() == 64);

/// How far something's content goes, and which part of it shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollInfo {
    /// How much of the content shows at once.
    pub view: Size,
    /// How big all of it is, which is never less than the view.
    pub content: Size,
    /// How far it's moved along each direction.
    pub x: f32,
    pub y: f32,
}

impl ScrollInfo {
    /// How far it can go down.
    pub fn furthest_y(&self) -> f32 {
        (self.content.height - self.view.height).max(0.0)
    }

    /// How far it can go across.
    pub fn furthest_x(&self) -> f32 {
        (self.content.width - self.view.width).max(0.0)
    }
}

/// Receives each node from [`Layout::visit`].
pub trait LayoutVisitor {
    /// Reaching a node, before any of its children.
    fn enter(&mut self, element: &Element, rect: Rect);
    /// Right after [`enter`](Self::enter), for something whose content can be bigger than it is.
    fn enter_scrolled(&mut self, _rect: Rect, _element: &Element) {}
    /// After all the children of what [`enter_scrolled`](Self::enter_scrolled) was called for.
    fn exit_scrolled(&mut self, _rect: Rect, _info: ScrollInfo, _element: &Element) {}
    /// On the way to something on top of everything, going through what it's in, which isn't being drawn this time.
    fn passing(&mut self, _element: &Element) {}
    /// Coming back out of what [`passing`](Self::passing) was called for.
    fn passed(&mut self, _element: &Element) {}
    /// Right after [`enter`](Self::enter), for text made of parts, whose children aren't visited.
    fn paragraph(&mut self, _element: &Element, _rect: Rect) {}
    /// What's in `element`, which is at `rect`, is cut off at its edges, as CSS's `overflow: hidden` has it: right after
    /// [`enter`](Self::enter), or on the way to something that's placed by itself, going through something that scrolls or
    /// cuts it off, without that being drawn this time.
    fn clip_to(&mut self, _rect: Rect, _element: &Element) {}
    /// Coming back out of what [`clip_to`](Self::clip_to) was called for.
    fn unclip(&mut self) {}
    /// Opt-in paint-only pruning after an explicit clip has been pushed. Return
    /// true only if the effective clip is empty and inherited effects cannot move
    /// paint out of it. Layout additionally proves the subtree has ordinary paint
    /// and no lifted content. Other visitors retain every existing callback.
    fn clipped_children_are_invisible(&self) -> bool {
        false
    }
    /// Whether a box at `rect` would draw nothing here, for leaving out what can't be seen. Layout only asks about something
    /// whose every box draws only inside itself, if at all, nothing in which is placed by itself or on top of everything; and
    /// leaves it out, not calling anything for it, when this says so for each of its boxes that would be visited. It's to be
    /// exactly what drawing it would find, or err towards false, which the default always does.
    #[inline(always)]
    fn draws_nothing_at(&self, _rect: Rect) -> bool {
        false
    }
    /// Leaving a node, after all of its children.
    fn exit(&mut self, element: &Element);
}

/// Where everything in a tree goes. Keep one for as long as the UI exists, so that
/// each update only pays for what changed. A copy goes on from where this one is, for laying out another version of the
/// same tree without laying out what the two have in common again.
#[derive(Default, Clone)]
pub struct Layout {
    root: LayoutBox,
    last_solved: usize,
    last_looked_at: usize,
    list_sizes: Vec<(usize, f32, f32)>,
    paragraphs: bool,
    /// Says that this has gone when it does, for [`Layout::info_of`].
    gone: Gone,
}

#[derive(Default, Clone)]
struct Gone;

impl Drop for Gone {
    fn drop(&mut self) {
        layouts_changed();
    }
}

/// How many times any layout has been changed, or one has gone, which says when what [`Layout::info_of`] kept may not hold.
static LAYOUTS_CHANGED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn layouts_changed() {
    LAYOUTS_CHANGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// What scrolls with at least this many things in it has how far they go kept, as working that out looks at each of them.
const KEEP_INFO_FROM: usize = 64;
/// How many of those are kept.
const KEPT_INFOS: usize = 8;

/// Which box, which node and which layout an answer of [`Layout::info_of`] is for.
type InfoAt = (usize, usize, usize, usize, u64);

thread_local! {
    /// What [`Layout::info_of`] last worked out for the scrolling areas that have the most in them, by [`InfoAt`]. How far a long
    /// list's rows go was worked out by looking at each of them, as it was drawn and each time code asked how far it scrolls, which
    /// it does three times as `scrollIntoView()` brings a row into view: 15 ms of each key that moved the selection in a list of
    /// 25,000 rows.
    static KEPT_INFO: std::cell::RefCell<Vec<(InfoAt, Option<ScrollInfo>)>> = const { std::cell::RefCell::new(Vec::new()) };
}


#[cfg(test)]
mod scroll_info_tests {
    use super::*;
    use rsui_core::{Element, Role, diff};
    use crate::FixedMetrics;

    fn page(x: f32, y: f32, width: f32, margin: f32, color: &str, rtl: bool) -> Node {
        let inside = Element::new(Role::Group)
            .prop("width", f64::from(width))
            .prop("height", 900.0)
            .prop("margin_right", f64::from(margin))
            .prop("background", color)
            .child(Element::new(Role::Group).prop("width", f64::from(width + 75.0)).prop("height", 15.0))
            .child(Element::new(Role::Group).prop("overlay", "fill").prop("width", 2000.0).prop("height", 2000.0));
        Element::new(Role::Window)
            .child(Element::new(Role::Group)
                .prop("scroll_x", f64::from(x))
                .prop("scroll_y", f64::from(y))
                .prop("height", 120.0)
                .prop("fill_across", true)
                .prop("text_direction", if rtl { "rtl" } else { "ltr" })
                .prop("scrollbar_room", 17.0)
                .child(inside))
            .into_node()
    }

    #[test]
    fn queries_stay_current_after_scroll_style_structure_resize_and_clone() {
        let mut layout = Layout::new();
        let mut before = page(0.0, 0.0, 500.0, 10.0, "red", false);
        let mut size = Size::new(400.0, 300.0);
        layout.compute(&before, size, &mut FixedMetrics::default());
        for (x, y, width, margin, color, rtl, resized) in [
            (0.0, 35.0, 500.0, 10.0, "red", false, false),
            (20.0, 70.0, 500.0, 10.0, "red", false, false),
            (20.0, 70.0, 500.0, 10.0, "blue", false, false),
            (20.0, 70.0, 650.0, 40.0, "blue", false, false),
            (20.0, 70.0, 650.0, 40.0, "blue", true, false),
            (20.0, 70.0, 650.0, 40.0, "blue", true, true),
            (0.0, 0.0, 300.0, 0.0, "red", false, false),
        ] {
            // Populate before every update, so each transition checks invalidation.
            assert_eq!(layout.scroll_info(&[0], &before.children[0]), layout.scroll_info(&[0], &before.children[0]));
            let after = page(x, y, width, margin, color, rtl);
            let needs_layout = layout.apply_needs_layout_since(&diff(&before, &after), Some(&before));
            if resized {
                size = Size::new(450.0, 350.0);
            }
            if needs_layout || resized {
                layout.compute(&after, size, &mut FixedMetrics::default());
            }
            let mut fresh = Layout::new();
            fresh.compute(&after, size, &mut FixedMetrics::default());
            let expected = fresh.scroll_info(&[0], &after.children[0]);
            for _ in 0..3 {
                assert_eq!(layout.scroll_info(&[0], &after.children[0]), expected, "{x} {y} {width} {margin} {rtl}");
            }
            let copied = layout.clone();
            assert_eq!(copied.scroll_info(&[0], &after.children[0]), expected);
            before = after;
        }
    }

    #[test]
    fn painting_observes_the_same_scroll_position_as_queries() {
        #[derive(Default)]
        struct Observed(Vec<ScrollInfo>);
        impl LayoutVisitor for Observed {
            fn enter(&mut self, _: &Element, _: Rect) {}
            fn exit(&mut self, _: &Element) {}
            fn exit_scrolled(&mut self, _: Rect, info: ScrollInfo, _: &Element) { self.0.push(info); }
        }
        let mut layout = Layout::new();
        let before = page(0.0, 0.0, 500.0, 10.0, "red", false);
        layout.compute(&before, Size::new(400.0, 300.0), &mut FixedMetrics::default());
        let mut observed = Observed::default();
        layout.visit(&before, &mut observed);
        assert_eq!(observed.0, [layout.scroll_info(&[0], &before.children[0]).unwrap()]);
        let after = page(0.0, 45.0, 500.0, 10.0, "red", false);
        assert!(!layout.apply_needs_layout_since(&diff(&before, &after), Some(&before)));
        observed.0.clear();
        layout.visit(&after, &mut observed);
        assert_eq!(observed.0, [layout.scroll_info(&[0], &after.children[0]).unwrap()]);
        assert_eq!(observed.0[0].y, 45.0);
    }
}

#[cfg(test)]
mod ancestor_rect_tests {
    use super::*;
    use crate::FixedMetrics;
    use rsui_core::{Element, Role, diff};

    fn compare_prefixes(layout: &Layout, path: &[u32]) {
        let expected: Vec<_> = (0..=path.len())
            .map_while(|depth| layout.rect(&path[..depth]))
            .collect();
        let mut actual = layout.rects_along(path);
        assert_eq!(actual.by_ref().collect::<Vec<_>>(), expected, "{path:?}");
        assert_eq!(actual.next(), None);
        assert_eq!(actual.next(), None);
    }

    #[test]
    fn linear_rectangles_match_prefix_reads_after_scroll_structure_and_resize() {
        let make = |scroll: f64, extra: bool| {
            let mut inside = Element::new(Role::Group).prop("width", 600.0).prop("height", 900.0).prop("padding", 7.25);
            for index in 0..12 {
                let one = Element::new(Role::Group).prop("height", 25.0 + index as f64).prop("margin_left", index as f64 / 3.0)
                    .child(Element::new(Role::Group).prop("height", 12.25).prop("width", 40.5));
                inside = inside.child(one);
            }
            if extra { inside = inside.child(Element::new(Role::Group).prop("height", 23.75)); }
            Element::new(Role::Window).child(Element::new(Role::Group)
                .prop("scroll_x", scroll / 2.0).prop("scroll_y", scroll).prop("fill_across", true)
                .prop("height", 120.0).child(inside)).into_node()
        };
        let mut layout = Layout::new();
        let mut before = make(0.0, false);
        for (scroll, extra, size) in [
            (0.0, false, Size::new(400.0, 300.0)),
            (30.25, false, Size::new(400.0, 300.0)),
            (150.5, true, Size::new(375.0, 280.0)),
            (0.0, false, Size::new(500.0, 350.0)),
        ] {
            let root = make(scroll, extra);
            layout.apply(&diff(&before, &root));
            layout.compute(&root, size, &mut FixedMetrics::default());
            let mut paths = Vec::new();
            layout.walk(|path, _| paths.push(path.to_vec()));
            for path in &paths { compare_prefixes(&layout, path); }
            for path in [&[99][..], &[0, 99, 0][..], &[0, 0, 0, u32::MAX][..]] {
                compare_prefixes(&layout, path);
            }
            before = root;
        }
    }

    #[test]
    fn linear_rectangles_preserve_unlaid_root_and_partial_child_behavior() {
        compare_prefixes(&Layout::new(), &[]);
        compare_prefixes(&Layout::new(), &[0, 0]);
        // `rect` gates the root only; an allocated child without HAS_LAYOUT still
        // supplies its stored box. Do not introduce a new per-child validity gate.
        let mut layout = Layout::new();
        layout.root.flags = HAS_LAYOUT;
        layout.root.x = -0.0;
        layout.root.y = 0.25;
        layout.root.width = 10.0;
        layout.root.children.push(LayoutBox {
            x: -1.25, y: 2.5, width: 3.75, height: 4.0, ..Default::default()
        });
        for path in [&[][..], &[0][..], &[0, 0][..], &[1][..]] { compare_prefixes(&layout, path); }
        let first = layout.rects_along(&[]).next().unwrap();
        assert_eq!(first.x.to_bits(), layout.rect(&[]).unwrap().x.to_bits());
    }
}

/// How many patches from the start of these are the same change to one parent's children, in the order
/// that lets them be done together: adding from the front, or removing from the back.
fn run_of_one_kind(patches: &[Patch]) -> usize {
    let mut run = 1;
    match &patches[0] {
        Patch::Insert { parent, index, .. } => {
            let mut last = *index;
            while let Some(Patch::Insert {
                parent: next_parent,
                index: next,
                ..
            }) = patches.get(run)
            {
                if next_parent != parent || *next <= last {
                    break;
                }
                last = *next;
                run += 1;
            }
        }
        Patch::Remove { parent, index } => {
            let mut last = *index;
            while let Some(Patch::Remove {
                parent: next_parent,
                index: next,
            }) = patches.get(run)
            {
                if next_parent != parent || *next >= last {
                    break;
                }
                last = *next;
                run += 1;
            }
        }
        _ => {}
    }
    run
}

impl Layout {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records what changed in the tree. Call this with the patches from each update,
    /// in order, before the next [`compute`](Self::compute).
    pub fn apply(&mut self, patches: &[Patch]) {
        self.apply_needs_layout(patches);
    }

    /// Records patches like [`apply`](Self::apply), returning whether this batch
    /// requires layout work. `false` does not cancel earlier invalidation or a
    /// changed available size; visual changes still require painting.
    pub fn apply_needs_layout(&mut self, patches: &[Patch]) -> bool {
        self.apply_needs_layout_since(patches, None)
    }

    /// The same as [`apply_needs_layout`](Self::apply_needs_layout), given the tree as it was before these patches, which
    /// says more about what they change: a box that's laid out as a part of a paragraph has no layout of its own to say it
    /// was drawn as one piece, but if it was turned or faded before, another value for that still changes nothing layout
    /// worked out, as layout only ever looks at whether one of [`ONE_PIECE_PROPS`] is there. A spinner inside a button's
    /// text is one, and laying the page out again for it at every frame had the page's `ResizeObserver`s looked at every
    /// frame as well, which made garbage that the window, never quiet while it moves, only collected now and then.
    pub fn apply_needs_layout_since(&mut self, patches: &[Patch], before: Option<&Node>) -> bool {
        layouts_changed();
        let needs_layout = self.apply_now(patches, before);
        // Scrolling moves what's in what scrolls, while this is going on as well as before it.
        layouts_changed();
        needs_layout
    }

    fn apply_now(&mut self, patches: &[Patch], before: Option<&Node>) -> bool {
        let mut needs_layout = false;
        let mut rest = patches;
        while let Some(patch) = rest.first() {
            match patch {
                Patch::SetActions { .. } => {
                    rest = &rest[1..];
                    continue;
                }
                Patch::SetProp { name, .. } | Patch::RemoveProp { name, .. }
                    if is_paint_only_prop(*name) =>
                {
                    rest = &rest[1..];
                    continue;
                }
                // Another place for a scrolling area to be scrolled to, which only moves what's in it.
                Patch::SetProp { path, name, value } if self.scrolled_without_layout(path, *name, value, before) => {
                    rest = &rest[1..];
                    continue;
                }
                // Another value for what turns or fades something that was already turned or faded when it was laid out.
                Patch::SetProp { path, name, .. }
                    if is_one_piece_prop(*name)
                        && (self.laid_out_in_one_piece(path)
                            || before.is_some_and(|before| was_drawn_as_one(before, path))) =>
                {
                    rest = &rest[1..];
                    continue;
                }
                _ => {}
            }
            needs_layout = true;
            let run = run_of_one_kind(rest);
            if run > 1 {
                self.apply_run(&rest[..run]);
            } else {
                self.apply_one(patch);
            }
            rest = &rest[run..];
        }
        needs_layout
    }

    /// Scrolls the scrolling area at `path` to where `value` says, if that's all that's changed about it, by moving what's in it
    /// by as much as it's moved, as boxes are placed from their parents: nothing in it changes size, so nothing needs laying out
    /// again. Laying it out again found the same sizes for over a thousand boxes on each frame of scrolling the Mantine kitchen
    /// sink, 3-4 ms a frame, and told the page's `ResizeObserver`s to look at everything they watch each time. Says
    /// whether it did that; when it didn't, the area's laid out again as for any other change. What's placed in the window rather
    /// than in what it's in (on top of everything, or fixed) is placed again. It isn't done for a list, which builds its rows
    /// from where it is; for what sticks, which is placed from where the area is; or for an area that something placed by itself
    /// inside is placed from outside of.
    fn scrolled_without_layout(&mut self, path: &[u32], name: rsui_core::Symbol, value: &rsui_core::PropValue, before: Option<&Node>) -> bool {
        let names = &*crate::style::NAMES;
        let down = name == names.scroll_y;
        if !down && name != names.scroll_x {
            return false;
        }
        let Some(before) = before else {
            return false;
        };
        let wanted = match value {
            rsui_core::PropValue::Float(value) => *value as f32,
            rsui_core::PropValue::Int(value) => *value as f32,
            _ => return false,
        };
        if !wanted.is_finite() {
            return false;
        }
        let mut element: &Element = before;
        for &index in path {
            match element.children.get(index as usize) {
                Some(child) => element = child,
                None => return false,
            }
        }
        // Only what it says about where it's scrolled to is different.
        if element.props.get_symbol(name).is_none() {
            return false;
        }
        let window = Size::new(self.root.width, self.root.height);
        let (mut x, mut y) = (self.root.x, self.root.y);
        let mut current = &mut self.root;
        for &index in path {
            if !current.has(HAS_LAYOUT) {
                return false;
            }
            let Some(child) = current.children.get_mut(index as usize) else {
                return false;
            };
            current = child;
            x += current.x;
            y += current.y;
        }
        // Something in it that's changed doesn't stop that: it's laid out again by itself, where it's been moved to, and should that
        // change its size, this is laid out again for it, from where it says it's scrolled to. A long list's row that's changed as it
        // was scrolled to, as the selection that moves into view is, laid out all of the list again with it.
        const WAITING: u32 = SELF_DIRTY | SPLICED | FORGOT_A_QUESTION;
        if current.flags & (HAS_LAYOUT | SCROLLS | WAITING) != HAS_LAYOUT | SCROLLS {
            return false;
        }
        // What's placed by itself in it from something around it stays where that puts it, which only laying out says.
        if current.has(CONTAINS_ABSOLUTE)
            && !engine::places_absolutes(element)
            && current.children.iter().zip(&element.children).any(|(child_box, child)| {
                (child_box.has(IS_ABSOLUTE) && !child_box.has(IS_OVERLAY) && engine::places_absolutes(child))
                    || engine::holds_passed(child_box, child)
            })
        {
            return false;
        }
        let (style, _) = Style::read(&element.props);
        if style.row_height.is_some()
            || (crate::style::ANY_STICKY.load(std::sync::atomic::Ordering::Relaxed) && sticks_inside(element))
        {
            return false;
        }
        let Some(info) = Self::info_of(current, element) else {
            return false;
        };
        let (was, furthest) = if down { (info.y, info.furthest_y()) } else { (info.x, info.furthest_x()) };
        // As the layout has it: what isn't past the start is at the start, and nothing goes past the end. How far the end is was
        // worked out from where what's in it is, as `solve_scroll` has it, so the far end is where laying it out would put it
        // too: going to the end of a long list from code (`scrollIntoView()` on its last row) laid all of it out again for that.
        let to = if wanted > 0.0 { wanted.min(furthest) } else { 0.0 };
        let by = to - was;
        for child in &mut current.children {
            if down {
                child.y -= by;
            } else {
                child.x -= by;
            }
        }
        if current.has(CONTAINS_OVERLAY) {
            engine::place_overlays(current, element, x, y, window);
        }
        true
    }

    /// Whether the box at `path` has been laid out, and was drawn as one piece then, with nothing waiting to lay it out again.
    /// See [`ONE_PIECE`].
    fn laid_out_in_one_piece(&self, path: &[u32]) -> bool {
        let mut current = &self.root;
        for &index in path {
            match current.children.get(index as usize) {
                Some(child) => current = child,
                None => return false,
            }
        }
        current.flags & (HAS_LAYOUT | ONE_PIECE | SELF_DIRTY) == HAS_LAYOUT | ONE_PIECE
    }

    /// Applies patches that add or remove many children of one parent, all at once. One at a time would
    /// move everything after each of them, which took tens of milliseconds to filter a long list.
    fn apply_run(&mut self, run: &[Patch]) {
        let (Patch::Insert { parent, .. } | Patch::Remove { parent, .. }) = &run[0] else {
            unreachable!("only these make runs");
        };
        let Some(target) = Self::mark_path(&mut self.root, parent) else {
            return;
        };
        target.mark_spliced();
        let children = &mut target.children;
        if matches!(run[0], Patch::Remove { .. }) {
            // These go from the back, so none of them moves one still to come. An index past the end is
            // for a box that was never made.
            let mut gone = run
                .iter()
                .rev()
                .filter_map(|patch| match patch {
                    Patch::Remove { index, .. } => Some(*index as usize),
                    _ => None,
                })
                .peekable();
            let mut position = 0;
            let mut after_one = false;
            children.retain_mut(|child| {
                let remove = gone.peek() == Some(&position);
                if remove {
                    gone.next();
                } else if after_one {
                    child.flags |= SHIFTED;
                }
                after_one = remove;
                position += 1;
                !remove
            });
            if after_one && let Some(last) = children.last_mut() {
                last.flags |= SHIFTED;
            }
            return;
        }
        let old = std::mem::take(children);
        let mut kept = old.into_iter();
        children.reserve_exact(kept.len() + run.len());
        for patch in run {
            let Patch::Insert { index, .. } = patch else {
                continue;
            };
            let index = *index as usize;
            while children.len() < index {
                let Some(next) = kept.next() else { break };
                children.push(next);
            }
            // Just as with one at a time, nothing's made past the end of what's there.
            if children.len() == index {
                children.push(LayoutBox::default());
            }
        }
        children.extend(kept);
    }

    fn apply_one(&mut self, patch: &Patch) {
        let path: &[u32] = match patch {
            // Actions have no effect on layout.
            Patch::SetActions { .. } => return,
            Patch::Replace { path, .. }
            | Patch::SetName { path, .. }
            | Patch::SetProp { path, .. }
            | Patch::RemoveProp { path, .. } => path,
            Patch::Insert { parent, .. }
            | Patch::Remove { parent, .. }
            | Patch::Move { parent, .. }
            | Patch::Reorder { parent, .. } => parent,
        };
        // A child's style feeds into its parent's decisions, so changing it can move
        // things even if the child's own size stays the same.
        let affects_parent = match patch {
            Patch::Replace { .. } => true,
            Patch::SetProp { name, .. } | Patch::RemoveProp { name, .. } => is_layout_prop(*name),
            _ => false,
        };
        if affects_parent {
            if let Some((_, parent_path)) = path.split_last() {
                if let Some(parent) = Self::mark_path(&mut self.root, parent_path) {
                    parent.mark_dirty();
                }
            }
        }
        let target = Self::mark_path(&mut self.root, path);
        // A box that doesn't exist yet was never laid out, so there's nothing to keep
        // in step. The next computation builds it.
        let Some(target) = target else { return };
        if matches!(patch, Patch::Insert { .. } | Patch::Remove { .. }) {
            target.mark_spliced();
        } else {
            target.mark_dirty();
        }
        let children = &mut target.children;
        match patch {
            Patch::Replace { .. } => *children = ThinVec::new(),
            Patch::Insert { index, .. } => {
                let index = *index as usize;
                if index <= children.len() {
                    children.insert(index, LayoutBox::default());
                }
            }
            Patch::Remove { index, .. } => {
                let index = *index as usize;
                if index < children.len() {
                    children.remove(index);
                    // What came after it is where it was, or, if it was last, what's now last is where it ended.
                    let next = index.min(children.len().saturating_sub(1));
                    if let Some(next) = children.get_mut(next) {
                        next.flags |= SHIFTED;
                    }
                }
            }
            Patch::Move { from, to, .. } => {
                let (from, to) = (*from as usize, *to as usize);
                if from < children.len() && to < children.len() {
                    if from < to {
                        children[from..=to].rotate_left(1);
                    } else {
                        children[to..=from].rotate_right(1);
                    }
                }
            }
            Patch::Reorder { start, order, .. } => {
                let start = *start as usize;
                let end = start + order.len();
                if end <= children.len() {
                    let mut taken: Vec<Option<LayoutBox>> =
                        children.drain(start..).map(Some).collect();
                    for &from in order {
                        children.push(taken[from as usize].take().unwrap_or_default());
                    }
                    children.extend(taken.drain(order.len()..).flatten());
                }
            }
            Patch::SetName { .. } | Patch::SetProp { .. } | Patch::RemoveProp { .. } => {}
            Patch::SetActions { .. } => unreachable!("handled above"),
        }
    }

    /// Finds the box at `path`, noting on the way that something below has changed.
    fn mark_path<'a>(root: &'a mut LayoutBox, path: &[u32]) -> Option<&'a mut LayoutBox> {
        let mut current = root;
        for &index in path {
            if !current.has(CHILD_DIRTY) {
                current.dirty_child = index;
            } else if current.dirty_child != index {
                current.flags |= MANY_CHILDREN_DIRTY;
            }
            current.flags |= CHILD_DIRTY;
            current = current.children.get_mut(index as usize)?;
        }
        Some(current)
    }

    /// Works out where everything goes, for a window of size `available`. Parts of the
    /// tree that haven't changed since the last time aren't visited.
    pub fn compute(&mut self, root: &Node, available: Size, measure: &mut impl Measure) {
        layouts_changed();
        (self.last_solved, self.last_looked_at) = engine::compute(
            &mut self.root,
            root,
            available,
            measure,
            &mut self.list_sizes,
            &mut self.paragraphs,
        );
        layouts_changed();
    }

    /// Whether any text made of parts has been laid out here, as most programs never have any, and
    /// anything looking for one can stop there.
    pub fn has_paragraphs(&self) -> bool {
        self.paragraphs
    }

    /// The virtual lists that the last [`compute`](Self::compute) found have the wrong idea of how
    /// many rows fit in them, as the address of each one's element and how many do, including part
    /// of a row, and for a grid, how many items fit across (0 for a list that isn't one). Tell them with their `resize` action,
    /// then lay out again. This is usually empty.
    pub fn list_sizes(&self) -> &[(usize, f32, f32)] {
        &self.list_sizes
    }

    /// How many times the last [`compute`](Self::compute) had to work out a node's
    /// size, rather than reuse an earlier answer. This shows how much of the tree an
    /// update really touched, which is otherwise invisible, since a layout that does
    /// too much work still comes out right.
    pub fn last_solved(&self) -> usize {
        self.last_solved
    }

    /// How many things laid out one after another the last [`compute`](Self::compute) looked at, laid out again or not, as each
    /// one in what was laid out again is: a long list that's laid out again, with none of its rows, still looks at each of them.
    pub fn last_looked_at(&self) -> usize {
        self.last_looked_at
    }

    /// Where the node at `path` is, from the top left of the window. `None` if there's
    /// no such node, or it hasn't been laid out.
    pub fn rect(&self, path: &[u32]) -> Option<Rect> {
        let mut current = &self.root;
        if !current.has(HAS_LAYOUT) {
            return None;
        }
        let (mut x, mut y) = (current.x, current.y);
        for &index in path {
            current = current.children.get(index as usize)?;
            x += current.x;
            y += current.y;
        }
        Some(Rect::new(x, y, current.width, current.height))
    }

    /// The root's rectangle, then each rectangle along `path`, from the top left
    /// of the window. Stops at the first missing box, or immediately if the root
    /// has not been laid out, just as successive [`rect`](Self::rect) prefix
    /// reads do. Walks each box once and borrows both layout and path.
    pub fn rects_along<'a>(&'a self, path: &'a [u32]) -> impl Iterator<Item = Rect> + 'a {
        let mut current = self.root.has(HAS_LAYOUT).then_some(&self.root);
        let mut indices = path.iter();
        let (mut x, mut y) = (self.root.x, self.root.y);
        std::iter::from_fn(move || {
            let one = current?;
            let rect = Rect::new(x, y, one.width, one.height);
            current = indices.next().and_then(|&index| one.children.get(index as usize));
            if let Some(next) = current {
                x += next.x;
                y += next.y;
            }
            Some(rect)
        })
    }

    /// How far the content of the node at `path` goes, if it's a scrolling area or a list. `element`
    /// must be that node.
    pub fn scroll_info(&self, path: &[u32], element: &Element) -> Option<ScrollInfo> {
        let mut current = &self.root;
        for &index in path {
            current = current.children.get(index as usize)?;
        }
        Self::info_of(current, element)
    }

    /// Only the current scroll offsets, without measuring the content's extent.
    /// `element` must be the node at `path`, as for [`Self::scroll_info`].
    pub fn scroll_offsets(&self, path: &[u32], element: &Element) -> Option<[f32; 2]> {
        let mut current = &self.root;
        for &index in path {
            current = current.children.get(index as usize)?;
        }
        if !current.has(SCROLLS) || !current.has(HAS_LAYOUT) {
            return None;
        }
        let (style, list) = Style::read(&element.props);
        if let Some(row_height) = style.row_height {
            return Some([0.0, list.first_visible * row_height]);
        }
        // RTL fill-across content can reach left of its immediate children;
        // retain the full calculation for that less common geometry case.
        if fills_across(element) && crate::engine::right_to_left(element) {
            return Self::info_of(current, element).map(|info| [info.x, info.y]);
        }
        let (mut x, y, _, bottom) = Self::scroll_bounds(current, element);
        if let Some(room) = Self::left_scrollbar_room(current, element, &style, bottom) {
            x -= room;
        }
        Some([(style.padding_left - x).max(0.0), (style.padding_top - y).max(0.0)])
    }

    /// How big what's in the box at `path` is, as a page's `scrollWidth` and `scrollHeight` say, for a box that doesn't scroll:
    /// its own size, or as far as what's in it reaches past that, with its padding, as when it clips them, or is no taller
    /// than nothing while what's in it isn't. A collapsed section is measured this way before it opens, to open it to that.
    /// `None` for a box that scrolls (see [`Self::scroll_info`]) or hasn't been laid out.
    pub fn content_size(&self, path: &[u32], element: &Element) -> Option<Size> {
        let mut current = &self.root;
        for &index in path {
            current = current.children.get(index as usize)?;
        }
        if current.has(SCROLLS) || !current.has(HAS_LAYOUT) {
            return None;
        }
        let (style, _) = Style::read(&element.props);
        let (mut right, mut bottom) = (current.width, current.height);
        // Children of a list are mostly given the same props, which is one set, so what was read from the last one is what the
        // next one has too.
        let mut last_margins: Option<(usize, [f32; 2])> = None;
        // What has no box of its own (`display: contents`, as what passes an element's variables down is) reaches as far as
        // what's in it does, as `furthest_right` has it: a Collapse with a Mantine `Box` in it opened to nothing.
        for (child, child_element) in current
            .children
            .iter()
            .zip(&element.children)
            .filter(|(child, _)| !child.has(IS_OVERLAY) && (child.has(HAS_LAYOUT) || child.has(CONTENTS)))
        {
            let (width, height) = if child.has(CONTENTS) {
                contents_reach(child)
            } else {
                (child.width, child.height)
            };
            let which = child_element.props.identity();
            let [margin_right, margin_bottom] = match last_margins {
                Some((same, margins)) if same == which => margins,
                _ => {
                    let margins = Style::read_far_margins(&child_element.props);
                    last_margins = Some((which, margins));
                    margins
                }
            };
            right = right.max(child.x + width + margin_right.max(0.0) + style.padding_right);
            bottom = bottom.max(child.y + height + margin_bottom.max(0.0) + style.padding_bottom);
        }
        Some(Size::new(right, bottom))
    }

    /// How far what's in a scrolling area has been moved by its scroll, across and down, from where it's laid out.
    pub(crate) fn scrolled_by(current: &LayoutBox, element: &Element) -> (f32, f32) {
        Self::info_of(current, element).map_or((0.0, 0.0), |info| (info.x, info.y))
    }

    fn info_of(current: &LayoutBox, element: &Element) -> Option<ScrollInfo> {
        if !current.has(SCROLLS) || !current.has(HAS_LAYOUT) {
            return None;
        }
        if current.children.len() < KEEP_INFO_FROM {
            return Self::info_now(current, element);
        }
        // Nothing about a box changes without a layout changing, and a node is never changed, so the same box and node with
        // nothing changed since come to the same.
        let at: InfoAt = (
            std::ptr::from_ref(current) as usize,
            std::ptr::from_ref(element) as usize,
            element.props.identity(),
            element.children.len(),
            LAYOUTS_CHANGED.load(std::sync::atomic::Ordering::Relaxed),
        );
        let kept = KEPT_INFO.with(|kept| kept.borrow().iter().find(|(said, _)| *said == at).map(|(_, info)| *info));
        if let Some(info) = kept {
            return info;
        }
        let info = Self::info_now(current, element);
        KEPT_INFO.with(|kept| {
            let mut kept = kept.borrow_mut();
            kept.retain(|(said, _)| said.4 == at.4);
            if kept.len() >= KEPT_INFOS {
                kept.remove(0);
            }
            kept.push((at, info));
        });
        info
    }

    fn info_now(current: &LayoutBox, element: &Element) -> Option<ScrollInfo> {
        let (style, list) = Style::read(&element.props);
        let view = Size::new(current.width, current.height);
        if let Some(row_height) = style.row_height {
            let count = match element.props.get("count") {
                Some(rsui_core::PropValue::Int(count)) => *count as f32,
                _ => element.children.len() as f32,
            };
            let padding = style.padding_top + style.padding_bottom;
            return Some(ScrollInfo {
                view,
                content: Size::new(view.width, (count * row_height + padding).max(view.height)),
                x: 0.0,
                y: list.first_visible * row_height,
            });
        }
        let (mut x, y, mut right, bottom) = Self::scroll_bounds(current, element);
        if fills_across(element) {
            // What's in it is as wide as it is, and it goes across as far as anything in that reaches: to its left too where text
            // goes from right to left, which is what there is to scroll to there.
            right = right.max(furthest_right(current, element));
            if crate::engine::right_to_left(element) {
                x = x.min(furthest_left(current, element));
            }
        }

        // What's in it starts after a scrollbar at its left (see `solve_scroll`), which isn't how far it's moved or how wide it is.
        if let Some(room) = Self::left_scrollbar_room(current, element, &style, bottom) {
            x -= room;
            right -= room;
        }
        let moved_x = (style.padding_left - x).max(0.0);
        let moved_y = (style.padding_top - y).max(0.0);
        let (wide, tall) = (
            right + moved_x + style.padding_right,
            bottom + moved_y + style.padding_bottom,
        );
        // Scrollbars that take room, as a page's do, leave less to see, and one can make the other needed, as the layout found.
        let room = match element.props.get_symbol(crate::style::NAMES.scrollbar_room) {
            Some(rsui_core::PropValue::Float(room)) => *room as f32,
            Some(rsui_core::PropValue::Int(room)) => *room as f32,
            _ => 0.0,
        };
        let view = if room > 0.0 {
            let (mut down, mut across) = (false, false);
            for _ in 0..2 {
                down = style.scroll_y.is_some()
                    && tall > view.height - if across { room } else { 0.0 } + 0.5;
                across = style.scroll_x.is_some()
                    && wide > view.width - if down { room } else { 0.0 } + 0.5;
            }
            Size::new(
                (view.width - if down { room } else { 0.0 }).max(0.0),
                (view.height - if across { room } else { 0.0 }).max(0.0),
            )
        } else {
            view
        };
        let content = |far: f32, moved: f32, padding: f32, view: f32, scrolls: bool| {
            if scrolls {
                (far + moved + padding).max(view)
            } else {
                view
            }
        };
        Some(ScrollInfo {
            view,
            content: Size::new(
                content(
                    right,
                    moved_x,
                    style.padding_right,
                    view.width,
                    style.scroll_x.is_some(),
                ),
                content(
                    bottom,
                    moved_y,
                    style.padding_bottom,
                    view.height,
                    style.scroll_y.is_some(),
                ),
            ),
            x: moved_x,
            y: moved_y,
        })
    }

    /// Positions are already clamped by layout. Immediate child origins determine
    /// their LTR offsets; recursive content widths are needed only by size reads.
    fn scroll_bounds(current: &LayoutBox, element: &Element) -> (f32, f32, f32, f32) {
        let (mut x, mut y) = (f32::INFINITY, f32::INFINITY);
        let (mut right, mut bottom) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        // Children of a list are mostly given the same props, which is one set, so what was read from the last one is what the
        // next one has too.
        let mut last_margins: Option<(usize, [f32; 2])> = None;
        for (child, child_element) in current
            .children
            .iter()
            .zip(&element.children)
            .filter(|(child, _)| !child.has(IS_OVERLAY))
        {
            let which = child_element.props.identity();
            let [margin_right, margin_bottom] = match last_margins {
                Some((same, margins)) if same == which => margins,
                _ => {
                    let margins = Style::read_far_margins(&child_element.props);
                    last_margins = Some((which, margins));
                    margins
                }
            };
            x = x.min(child.x);
            y = y.min(child.y);
            right = right.max(child.x + child.width + margin_right.max(0.0));
            bottom = bottom.max(child.y + child.height + margin_bottom.max(0.0));
        }
        if !right.is_finite() {
            (0.0, 0.0, 0.0, 0.0)
        } else {
            (x, y, right, bottom)
        }
    }

    fn left_scrollbar_room(current: &LayoutBox, element: &Element, style: &Style, bottom: f32) -> Option<f32> {
        if style.scroll_y.is_some()
            && bottom + style.padding_bottom > current.height + 0.5
            && matches!(element.props.get_symbol(crate::style::NAMES.scrollbar_left), Some(rsui_core::PropValue::Bool(true)))
        {
            match element.props.get_symbol(crate::style::NAMES.scrollbar_room) {
                Some(rsui_core::PropValue::Float(room)) => Some(*room as f32),
                Some(rsui_core::PropValue::Int(room)) => Some(*room as f32),
                _ => None,
            }
        } else {
            None
        }
    }

    /// Calls `visit` with the path and position of every node, parents before their
    /// children, which is the order to draw them in.
    /// Each node `wanted` says, as far as it goes into, that's within `margin` of being seen: inside the window and each area
    /// around it that scrolls, each made `margin` bigger on every side, as CSS's `content-visibility: auto` asks of what it skips.
    /// What's in something that scrolls is only looked at when its own box is near what shows of that, as it's drawn (see
    /// `visit_scrolled`), so a long list costs a look at each of its rows and no more; what's laid out outside its box isn't
    /// found from there. `root` must be what this was laid out from. `found` is given each one's path and node.
    pub fn near_view(
        &self,
        root: &Node,
        margin: f32,
        wanted: &mut impl FnMut(&Element) -> bool,
        found: &mut impl FnMut(&[u32], &Element),
    ) {
        let window = Rect::new(-margin, -margin, self.root.width + margin * 2.0, self.root.height + margin * 2.0);
        Self::near_box(&self.root, root, 0.0, 0.0, window, margin, &mut Vec::new(), wanted, found);
    }

    #[allow(clippy::too_many_arguments)]
    fn near_box(
        current: &LayoutBox,
        node: &Element,
        parent_x: f32,
        parent_y: f32,
        area: Rect,
        margin: f32,
        path: &mut Vec<u32>,
        wanted: &mut impl FnMut(&Element) -> bool,
        found: &mut impl FnMut(&[u32], &Element),
    ) {
        let (x, y) = (parent_x + current.x, parent_y + current.y);
        let meets = |rect: Rect, area: Rect| {
            rect.x < area.x + area.width && rect.y < area.y + area.height && rect.x + rect.width > area.x && rect.y + rect.height > area.y
        };
        let own = Rect::new(x, y, current.width, current.height);
        if wanted(node) {
            if meets(own, area) {
                found(path, node);
            }
            // What's skipped has nothing in it to look at.
            return;
        }
        // What's in something that scrolls shows only inside it, and is near only as far as `margin` past that.
        let scrolls = current.has(SCROLLS);
        let inner = if scrolls {
            let left = area.x.max(x - margin);
            let top = area.y.max(y - margin);
            let right = (area.x + area.width).min(x + current.width + margin);
            let bottom = (area.y + area.height).min(y + current.height + margin);
            Rect::new(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
        } else {
            area
        };
        // And so is each of a long list's rows, as what's in what scrolls is most often in one box that holds them: what's in a row
        // that's laid out outside it isn't found from there either. Looking in each of 25,000 rows took a millisecond each time the
        // list was laid out or scrolled.
        let long = current.children.len() >= LONG_NEAR_FROM;
        for (index, (child_box, child)) in current.children.iter().zip(&node.children).enumerate() {
            if (scrolls || long)
                && !child_box.has(IS_OVERLAY)
                && !meets(Rect::new(x + child_box.x, y + child_box.y, child_box.width, child_box.height), inner)
            {
                continue;
            }
            path.push(index as u32);
            rsui_core::stack::deep(|| Self::near_box(child_box, child, x, y, inner, margin, path, wanted, found));
            path.pop();
        }
    }

    pub fn walk(&self, mut visit: impl FnMut(&[u32], Rect)) {
        let mut path = Vec::new();
        Self::walk_box(&self.root, 0.0, 0.0, &mut path, &mut visit);
    }

    fn walk_box(
        current: &LayoutBox,
        parent_x: f32,
        parent_y: f32,
        path: &mut Vec<u32>,
        visit: &mut impl FnMut(&[u32], Rect),
    ) {
        let (x, y) = (parent_x + current.x, parent_y + current.y);
        visit(path, Rect::new(x, y, current.width, current.height));
        for (index, child) in current.children.iter().enumerate() {
            path.push(index as u32);
            Self::walk_box(child, x, y, path, visit);
            path.pop();
        }
    }

    /// Visits every node with its position, parents before their children, which is the
    /// order to draw them in. What's scrolled entirely out of view is left out, as a scrolling area
    /// cuts off what's past its edges anyway. `root` must be the tree last given to
    /// [`compute`](Self::compute).
    pub fn visit(&self, root: &Node, visitor: &mut impl LayoutVisitor) {
        let mut lift = Lift::default();
        Self::visit_box(&self.root, root, 0.0, 0.0, visitor, &mut lift);
        // What's placed, by itself or where it would be, is drawn after everything that isn't, as CSS has it, in the order it's
        // in, with what's placed in each right after it.
        let later = std::mem::take(&mut lift.later);
        self.visit_all_lifted(root, later, visitor, &mut lift);
        if self.root.has(CONTAINS_OVERLAY) {
            let stacked = self.stacked_overlays(root);
            if stacked.is_none() {
                Self::visit_overlays(&self.root, root, self.root.x, self.root.y, visitor);
            }
            for path in stacked.unwrap_or_default() {
                self.visit_overlay_at(root, &path, visitor);
            }
        }
    }

    /// Where each thing on top of everything stands among the rest: by the `z_index` of the outermost one it's in (itself, if
    /// it's in none), and then by when the portal into the page's `body` it's in was opened (`portal_order`, which a page's
    /// `createPortal` gives the group it puts what's in it in), as each portal goes at the end of `body` when it opens, so what
    /// opened later is drawn over what opened before. What's in no portal comes before every portal, as what the app drew is
    /// before them in `body`.
    fn overlay_rank(&self, root: &Node, path: &[u32]) -> (f64, f64) {
        let names = &crate::style::NAMES;
        let order = rsui_core::symbol!("portal_order");
        let (mut current, mut node) = (&self.root, root);
        let mut z = None;
        let mut portal = 0.0;
        for &index in path {
            let (Some(child_box), Some(child)) = (current.children.get(index as usize), node.children.get(index as usize)) else {
                break;
            };
            (current, node) = (child_box, child);
            if z.is_some() {
                break;
            }
            if let Some(opened) = number_prop(node, order) {
                portal = opened;
            }
            if current.has(IS_OVERLAY) {
                z = Some(number_prop(node, names.z_index).unwrap_or(0.0));
            }
        }
        (z.unwrap_or(0.0), portal)
    }

    /// Everything on top of everything else in the order it's drawn, with what's on top last, when that isn't the order they're
    /// in, which [`overlays`](Self::overlays) gives: see `overlay_rank`. `None` when it's that order.
    fn stacked_overlays(&self, root: &Node) -> Option<Vec<Vec<u32>>> {
        let mut found = self.overlays();
        if found.len() < 2 {
            return None;
        }
        let ranks: Vec<(f64, f64)> = found.iter().map(|path| self.overlay_rank(root, path)).collect();
        if ranks.windows(2).all(|two| two[0].0 <= two[1].0 && (two[0].0 < two[1].0 || two[0].1 <= two[1].1)) {
            return None;
        }
        let mut order: Vec<usize> = (0..found.len()).collect();
        // Sorting keeps the order they're in for what ranks the same, which keeps what's in one after it.
        order.sort_by(|&a, &b| ranks[a].0.total_cmp(&ranks[b].0).then(ranks[a].1.total_cmp(&ranks[b].1)));
        Some(order.into_iter().map(|at| std::mem::take(&mut found[at])).collect())
    }

    /// Everything on top of everything else, with what's on top last, as it's drawn: see `overlay_rank`.
    pub fn overlays_in(&self, root: &Node) -> Vec<Vec<u32>> {
        self.stacked_overlays(root).unwrap_or_else(|| self.overlays())
    }

    /// Draws what's on top of everything at `path`, as `visit_overlays` draws each, without what's in it that's on top too.
    fn visit_overlay_at(&self, root: &Node, path: &[u32], visitor: &mut impl LayoutVisitor) {
        let (mut current, mut node) = (&self.root, root);
        let (mut x, mut y) = (self.root.x, self.root.y);
        let mut around: Vec<&Node> = Vec::with_capacity(path.len());
        for (depth, &index) in path.iter().enumerate() {
            visitor.passing(node);
            around.push(node);
            let (Some(child_box), Some(child)) = (current.children.get(index as usize), node.children.get(index as usize)) else {
                break;
            };
            if depth + 1 == path.len() {
                let mut lift = Lift {
                    apart: 1,
                    ..Lift::default()
                };
                Self::visit_box(child_box, child, x, y, visitor, &mut lift);
            } else {
                (x, y) = (x + child_box.x, y + child_box.y);
            }
            (current, node) = (child_box, child);
        }
        for element in around.into_iter().rev() {
            visitor.passed(element);
        }
    }

    /// What's placed by itself, in what it's drawn over, as CSS has it: what has a `z_index` above nothing comes after the
    /// rest, lowest first, wherever it is in what doesn't have one of its own.
    fn visit_all_lifted(
        &self,
        root: &Node,
        paths: Vec<Vec<u32>>,
        visitor: &mut impl LayoutVisitor,
        lift: &mut Lift,
    ) {
        let mut raised = Vec::new();
        self.visit_lifted_list(root, paths, visitor, lift, &mut raised);
        if raised.is_empty() {
            return;
        }
        // In the order they're in, for what's as high as each other.
        raised.sort_by(|a: &(f64, Vec<u32>), b| a.0.total_cmp(&b.0));
        for (_, path) in raised {
            self.visit_lifted(root, &path, visitor, lift, None);
        }
    }

    /// Draws each of `paths` now, apart from what's raised above the rest, which goes in `raised` to be drawn after them.
    fn visit_lifted_list(
        &self,
        root: &Node,
        paths: Vec<Vec<u32>>,
        visitor: &mut impl LayoutVisitor,
        lift: &mut Lift,
        raised: &mut Vec<(f64, Vec<u32>)>,
    ) {
        for path in paths {
            match z_index_at(root, &path) {
                Some(z) if z > 0.0 => raised.push((z, path)),
                _ => self.visit_lifted(root, &path, visitor, lift, Some(raised)),
            }
        }
    }

    /// Something placed by itself, at `path`, after everything else: what it's in isn't drawn again, but cuts it off where it
    /// would have, and gives it its color.
    fn visit_lifted(
        &self,
        root: &Node,
        path: &[u32],
        visitor: &mut impl LayoutVisitor,
        lift: &mut Lift,
        outer: Option<&mut Vec<(f64, Vec<u32>)>>,
    ) {
        let (mut current, mut node) = (&self.root, root);
        let (mut x, mut y) = (0.0, 0.0);
        let mut around: Vec<(&Node, bool)> = Vec::with_capacity(path.len());
        let cut = cut_off_by(&self.root, root, path);
        for (at, &index) in path.iter().enumerate() {
            let (inner_x, inner_y) = (x + current.x, y + current.y);
            visitor.passing(node);
            let clips = current.has(SCROLLS) || cut.get(at).copied().unwrap_or(false);
            if clips {
                visitor.clip_to(Rect::new(inner_x, inner_y, current.width, current.height), node);
            }
            around.push((node, clips));
            (x, y) = (inner_x, inner_y);
            let (Some(child_box), Some(child)) = (
                current.children.get(index as usize),
                node.children.get(index as usize),
            ) else {
                break;
            };
            (current, node) = (child_box, child);
        }
        if around.len() == path.len() {
            lift.path.clear();
            lift.path.extend_from_slice(path);
            lift.apart = 0;
            let others = std::mem::take(&mut lift.later);
            Self::visit_box(current, node, x, y, visitor, lift);
            let inside = std::mem::replace(&mut lift.later, others);
            // What's placed inside it comes right after it, with what scrolls around it still cutting it off. Without a
            // `z_index` of its own, what's raised in it is raised among what's around it, as it is on a page.
            match outer {
                Some(outer) if z_index_at(root, path).is_none() => {
                    self.visit_lifted_list(root, inside, visitor, lift, outer);
                }
                _ => self.visit_all_lifted(root, inside, visitor, lift),
            }
        }
        for (element, clips) in around.into_iter().rev() {
            if clips {
                visitor.unclip();
            }
            visitor.passed(element);
        }
    }

    fn visit_box(
        current: &LayoutBox,
        node: &Node,
        parent_x: f32,
        parent_y: f32,
        visitor: &mut impl LayoutVisitor,
        lift: &mut Lift,
    ) {
        let (x, y) = (parent_x + current.x, parent_y + current.y);
        let rect = Rect::new(x, y, current.width, current.height);
        visitor.enter(node, rect);
        // What's in something that says it's cut off at its edges, as CSS's `overflow: hidden` is, doesn't show past them.
        let clips = !node.props.is_empty() && clips(node);
        if clips {
            visitor.clip_to(rect, node);
            if current.has(CLIPPED_PAINT_SAFE)
                && !current.children.is_empty()
                && visitor.clipped_children_are_invisible()
            {
                visitor.unclip();
                visitor.exit(node);
                return;
            }
        }
        // What's faded or turned is drawn as one piece, so what's placed by itself inside it is drawn with it, where it is.
        let one_piece = !node.props.is_empty() && drawn_as_one(node);
        lift.apart += u32::from(one_piece);
        if current.has(SCROLLS | CONTAINS_OVERLAY | PARAGRAPH) {
            if current.has(PARAGRAPH) {
                visitor.paragraph(node, rect);
            } else if current.has(SCROLLS) {
                Self::visit_scrolled(current, node, rect, visitor, lift);
            } else {
                for (index, (child_box, child)) in
                    current.children.iter().zip(&node.children).enumerate()
                {
                    if !child_box.has(IS_OVERLAY) {
                        Self::visit_child(child_box, child, index, x, y, visitor, lift);
                    }
                }
            }
        } else if current.has(STACKED) {
            for index in stacking_order(node) {
                Self::visit_child(
                    &current.children[index],
                    &node.children[index],
                    index,
                    x,
                    y,
                    visitor,
                    lift,
                );
            }
        } else {
            for (index, (child_box, child)) in
                current.children.iter().zip(&node.children).enumerate()
            {
                Self::visit_child(child_box, child, index, x, y, visitor, lift);
            }
        }
        lift.apart -= u32::from(one_piece);
        if clips {
            visitor.unclip();
        }
        visitor.exit(node);
    }

    /// A child, now or, if it's placed by itself, later, as [`visit`](Self::visit) says.
    #[inline(always)]
    fn visit_child(
        child_box: &LayoutBox,
        child: &Node,
        index: usize,
        x: f32,
        y: f32,
        visitor: &mut impl LayoutVisitor,
        lift: &mut Lift,
    ) {
        // What can't be seen is left out. `PAINTS_IN_OWN_BOXES` says nothing in it is placed by itself, so nothing's left out
        // that would have been visited later.
        if child_box.flags & (PAINTS_IN_OWN_BOXES | IS_ABSOLUTE | IS_OVERLAY) == PAINTS_IN_OWN_BOXES
            && Self::draws_nothing(child_box, x, y, visitor, &mut { UNSEEN_BOXES_LOOKED_AT })
        {
            return;
        }
        lift.path.push(index as u32);
        // What's placed by itself below what it's in, as Radix's card's panel is with `z-index: -1`, is drawn now, under what
        // comes after it, which `stacking_order` puts it before, rather than over everything later.
        if lift.apart == 0 && (child_box.has(IS_ABSOLUTE) || placed(child)) && !below_the_rest(child) {
            lift.later.push(lift.path.clone());
        } else {
            // A tree deeper than the stack has room for goes on with another: see `rsui_core::stack`.
            rsui_core::stack::deep(|| Self::visit_box(child_box, child, x, y, visitor, lift));
        }
        lift.path.pop();
    }

    /// Whether `current`, where `x` and `y` put what it's in, and everything in it that would be visited, draws nothing, as
    /// the visitor says of each box. It's only asked of what has [`PAINTS_IN_OWN_BOXES`], as what's in that has too. Gives up
    /// after looking at `budget` boxes, as then it's quicker to visit them, so that a tall tree that's partly seen isn't looked
    /// through again for each thing in it.
    fn draws_nothing(
        current: &LayoutBox,
        x: f32,
        y: f32,
        visitor: &impl LayoutVisitor,
        budget: &mut u32,
    ) -> bool {
        let (x, y) = (x + current.x, y + current.y);
        if !visitor.draws_nothing_at(Rect::new(x, y, current.width, current.height)) {
            return false;
        }
        // A paragraph's parts are drawn with it, and what scrolls cuts off what's in it inside its own box.
        if current.children.is_empty() || current.has(PARAGRAPH | SCROLLS) {
            return true;
        }
        let Some(left) = budget.checked_sub(current.children.len() as u32) else {
            return false;
        };
        *budget = left;
        current
            .children
            .iter()
            .all(|child| Self::draws_nothing(child, x, y, visitor, budget))
    }

    /// Visits what's on top of everything, after all the rest, with nothing cut off by what it's in.
    fn visit_overlays(
        current: &LayoutBox,
        node: &Node,
        x: f32,
        y: f32,
        visitor: &mut impl LayoutVisitor,
    ) {
        visitor.passing(node);
        for (child_box, child) in current.children.iter().zip(&node.children) {
            let (child_x, child_y) = (x + child_box.x, y + child_box.y);
            if child_box.has(IS_OVERLAY) {
                // What's on top of everything has what's placed by itself in it drawn where it is.
                let mut lift = Lift {
                    apart: 1,
                    ..Lift::default()
                };
                Self::visit_box(child_box, child, x, y, visitor, &mut lift);
            }
            if child_box.has(CONTAINS_OVERLAY) {
                Self::visit_overlays(child_box, child, child_x, child_y, visitor);
            }
        }
        visitor.passed(node);
    }

    /// Whether anything is on top of everything right now.
    pub fn has_overlays(&self) -> bool {
        self.root.has(CONTAINS_OVERLAY)
    }

    /// Whether the node at `path` is on top of everything.
    pub fn is_overlay(&self, path: &[u32]) -> bool {
        let mut current = &self.root;
        for &index in path {
            match current.children.get(index as usize) {
                Some(child) => current = child,
                None => return false,
            }
        }
        current.has(IS_OVERLAY)
    }

    /// The path of everything on top of everything else, with what's on top last.
    pub fn overlays(&self) -> Vec<Vec<u32>> {
        fn walk(current: &LayoutBox, path: &mut Vec<u32>, found: &mut Vec<Vec<u32>>) {
            for (index, child) in current.children.iter().enumerate() {
                path.push(index as u32);
                if child.has(IS_OVERLAY) {
                    found.push(path.clone());
                }
                if child.has(CONTAINS_OVERLAY) {
                    walk(child, path, found);
                }
                path.pop();
            }
        }
        let mut found = Vec::new();
        if self.root.has(CONTAINS_OVERLAY) {
            walk(&self.root, &mut Vec::new(), &mut found);
        }
        found
    }

    /// The children of something that scrolls, of which only some show. This is apart from
    /// [`visit_box`](Self::visit_box) to leave that small, as it runs for every node.
    #[inline(never)]
    fn visit_scrolled(
        current: &LayoutBox,
        node: &Node,
        rect: Rect,
        visitor: &mut impl LayoutVisitor,
        lift: &mut Lift,
    ) {
        let Some(info) = Self::info_of(current, node) else {
            return;
        };
        visitor.enter_scrolled(rect, node);
        for (index, (child_box, child)) in current.children.iter().zip(&node.children).enumerate() {
            if child_box.has(IS_OVERLAY) {
                continue;
            }
            let shows = child_box.x < current.width
                && child_box.y < current.height
                && child_box.x + child_box.width > 0.0
                && child_box.y + child_box.height > 0.0;
            if shows {
                Self::visit_child(child_box, child, index, rect.x, rect.y, visitor, lift);
            }
        }
        visitor.exit_scrolled(rect, info, node);
    }

    /// The path of the deepest node at a point, for finding what a click landed on.
    /// A child only counts inside its parent, since a parent's edge is where a list
    /// or a scrolling area stops showing it. Siblings can't overlap, so at most one
    /// child can contain the point.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<Vec<u32>> {
        let mut path = Vec::new();
        if self.root.has(CONTAINS_OVERLAY)
            && Self::hit_overlays(&self.root, x - self.root.x, y - self.root.y, &mut path)
        {
            return Some(path);
        }
        if let Some(found) = self.hit_lifted(x, y) {
            return Some(found);
        }
        Self::hit_box(&self.root, x, y, &mut path).then_some(path)
    }

    /// What's placed by itself at a point, the last drawn first, as that's on top, and as far as what scrolls around it shows it.
    /// It counts outside what it's in, as a menu below a button does.
    fn hit_lifted(&self, x: f32, y: f32) -> Option<Vec<u32>> {
        if !self.root.has(CONTAINS_ABSOLUTE) {
            return None;
        }
        'each: for path in self.lifted().iter().rev() {
            let mut current = &self.root;
            let (mut left, mut top) = (0.0, 0.0);
            for &index in path {
                let (inner_x, inner_y) = (left + current.x, top + current.y);
                if current.has(SCROLLS)
                    && !Rect::new(inner_x, inner_y, current.width, current.height).contains(x, y)
                {
                    continue 'each;
                }
                (left, top) = (inner_x, inner_y);
                current = &current.children[index as usize];
            }
            let mut deeper = path.clone();
            if Self::hit_box(current, x - left, y - top, &mut deeper) {
                return Some(deeper);
            }
        }
        None
    }

    /// [`hit_lifted`](Self::hit_lifted) where things can have been turned or moved: the point is taken through what each thing on
    /// the way to what's placed by itself undoes, as well as that thing's own, as a Radix menu is `position: fixed` at 0, 0 and
    /// moved by `transform: translate()` to where it's shown, which is where it's clicked.
    fn hit_lifted_turned_observed(
        &self,
        root: &Node,
        x: f32,
        y: f32,
        undo: &impl Fn(&Element, Rect) -> Option<[f32; 6]>,
        probe: &mut impl HitProbe,
    ) -> Option<Vec<u32>> {
        static FUSED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if !*FUSED.get_or_init(|| std::env::var_os("RSUI_FUSED_HITS").is_none_or(|value| value != "0")) {
            return self.hit_lifted_turned_materialized_observed(root, x, y, undo, probe);
        }
        self.hit_lifted_turned_fused_observed(root, x, y, undo, probe)
    }

    #[cfg(test)]
    fn hit_lifted_turned_fused(
        &self,
        root: &Node,
        x: f32,
        y: f32,
        undo: &impl Fn(&Element, Rect) -> Option<[f32; 6]>,
    ) -> Option<Vec<u32>> {
        self.hit_lifted_turned_fused_observed(root, x, y, undo, &mut NoProbe)
    }

    fn hit_lifted_turned_fused_observed(
        &self,
        root: &Node,
        x: f32,
        y: f32,
        undo: &impl Fn(&Element, Rect) -> Option<[f32; 6]>,
        probe: &mut impl HitProbe,
    ) -> Option<Vec<u32>> {
        if !self.root.has(CONTAINS_ABSOLUTE) {
            return None;
        }
        let mut path = Vec::new();
        Self::hit_lifted_children_turned_observed(&self.root, root, x, y, undo, false, &mut path, probe).then_some(path)
    }

    /// Reverse preorder is the exact reverse of lifted(): descendants precede
    /// their absolute parent, and later siblings precede earlier siblings.
    /// Coordinates here precede current's own transform, just as each prefix
    /// step in the materialized walk does. A candidate keeps that convention
    /// when passed to hit_box_turned, which applies its own transform itself.
    fn hit_lifted_children_turned_observed(
        current: &LayoutBox,
        node: &Node,
        x: f32,
        y: f32,
        undo: &impl Fn(&Element, Rect) -> Option<[f32; 6]>,
        muted: bool,
        path: &mut Vec<u32>,
        probe: &mut impl HitProbe,
    ) -> bool {
        let rect = Rect::new(current.x, current.y, current.width, current.height);
        let (x, y) = {
            let matrix = undo(node, rect);
            probe.transform(matrix, rect, x, y, path.len());
            undone(matrix, x, y)
        };
        if current.has(SCROLLS) && !probe.contains(rect, x, y, path.len()) {
            return false;
        }
        let (x, y) = (x - current.x, y - current.y);
        for (index, child) in current.children.iter().enumerate().rev() {
            #[cfg(test)]
            lifted_tests::note_gather_visit();
            if !child.contributes_absolute() {
                continue;
            }
            let inert = child.has(ABSOLUTE_IS_INERT);
            if inert && !child.has(CONTAINS_ABSOLUTE) {
                continue;
            }
            let Some(child_node) = node.children.get(index) else {
                continue;
            };
            let muted = inert || (muted && !child.has(ABSOLUTE_TAKES_POINTER));
            path.push(index as u32);
            if child.has(CONTAINS_ABSOLUTE)
                && Self::hit_lifted_children_turned_observed(child, child_node, x, y, undo, muted, path, probe)
            {
                return true;
            }
            if child.has(IS_ABSOLUTE) && !muted && Self::hit_box_turned_observed(child, child_node, x, y, undo, path, probe) {
                return true;
            }
            path.pop();
        }
        false
    }

    /// Preserved materialized algorithm for same-binary comparisons and exact
    /// differential tests of the traversal above.
    #[cfg(test)]
    fn hit_lifted_turned_materialized(
        &self,
        root: &Node,
        x: f32,
        y: f32,
        undo: &impl Fn(&Element, Rect) -> Option<[f32; 6]>,
    ) -> Option<Vec<u32>> {
        self.hit_lifted_turned_materialized_observed(root, x, y, undo, &mut NoProbe)
    }

    fn hit_lifted_turned_materialized_observed(
        &self,
        root: &Node,
        x: f32,
        y: f32,
        undo: &impl Fn(&Element, Rect) -> Option<[f32; 6]>,
        probe: &mut impl HitProbe,
    ) -> Option<Vec<u32>> {
        if !self.root.has(CONTAINS_ABSOLUTE) {
            return None;
        }
        'each: for path in self.lifted().iter().rev() {
            let mut current = &self.root;
            let mut node = root;
            // The point, where `current`'s box is said from.
            let (mut x, mut y) = (x, y);
            for (depth, &index) in path.iter().enumerate() {
                let rect = Rect::new(current.x, current.y, current.width, current.height);
                (x, y) = {
                    let matrix = undo(node, rect);
                    probe.transform(matrix, rect, x, y, depth);
                    undone(matrix, x, y)
                };
                if current.has(SCROLLS) && !probe.contains(rect, x, y, depth) {
                    continue 'each;
                }
                (x, y) = (x - current.x, y - current.y);
                let (Some(child), Some(child_node)) =
                    (current.children.get(index as usize), node.children.get(index as usize))
                else {
                    continue 'each;
                };
                current = child;
                node = child_node;
            }
            let mut deeper = path.clone();
            if Self::hit_box_turned_observed(current, node, x, y, undo, &mut deeper, probe) {
                return Some(deeper);
            }
        }
        None
    }

    /// The paths of what's placed by itself, in the order it's drawn.
    fn lifted(&self) -> Vec<Vec<u32>> {
        // What's in something inert that's placed by itself only counts once something in it says it takes the pointer again.
        fn gather(current: &LayoutBox, path: &mut Vec<u32>, found: &mut Vec<Vec<u32>>, muted: bool) {
            for (index, child) in current.children.iter().enumerate() {
                #[cfg(test)]
                lifted_tests::note_gather_visit();
                if !child.contributes_absolute() {
                    continue;
                }
                let takes = child.has(ABSOLUTE_TAKES_POINTER);
                let inert = child.has(ABSOLUTE_IS_INERT);
                if inert && !child.has(CONTAINS_ABSOLUTE) {
                    continue;
                }
                let muted = inert || (muted && !takes);
                path.push(index as u32);
                if child.has(IS_ABSOLUTE) && !muted {
                    found.push(path.clone());
                }
                if child.has(CONTAINS_ABSOLUTE) {
                    gather(child, path, found, muted);
                }
                path.pop();
            }
        }
        let mut found = Vec::new();
        gather(&self.root, &mut Vec::new(), &mut found, false);
        found
    }

    /// [`hit_test`](Self::hit_test), when what's there can have been turned or resized. `undo` says what takes a point from around
    /// something to where it would be if it hadn't been, as `[a, b, c, d, e, f]`, for an element laid out at this place. Each point in
    /// the middle of this tree is looked at, so it's only for a tree that has such things.
    pub fn hit_test_turned(
        &self,
        root: &Node,
        x: f32,
        y: f32,
        undo: &impl Fn(&Element, Rect) -> Option<[f32; 6]>,
    ) -> Option<Vec<u32>> {
        self.hit_test_turned_observed(root, x, y, undo, &mut NoProbe)
    }

    /// The normal raw hit plus an optional bounded region with the same raw
    /// result, for an unchanged tree/layout and translation-only transforms.
    /// Callers must still apply rounded corners, inert ownership and text hits.
    pub fn hit_test_turned_certified(
        &self,
        root: &Node,
        x: f32,
        y: f32,
        undo: &impl Fn(&Element, Rect) -> Option<[f32; 6]>,
        bounds: Rect,
    ) -> (Option<Vec<u32>>, Option<crate::HitRegion>) {
        let mut probe = RegionProbe::new(x, y, bounds);
        let hit = self.hit_test_turned_observed(root, x, y, undo, &mut probe);
        (hit, probe.finish())
    }

    fn hit_test_turned_observed(
        &self,
        root: &Node,
        x: f32,
        y: f32,
        undo: &impl Fn(&Element, Rect) -> Option<[f32; 6]>,
        probe: &mut impl HitProbe,
    ) -> Option<Vec<u32>> {
        let mut path = Vec::new();
        if self.root.has(CONTAINS_OVERLAY) {
            let rect = Rect::new(self.root.x, self.root.y, self.root.width, self.root.height);
            let (x, y) = {
                let matrix = undo(root, rect);
                probe.transform(matrix, rect, x, y, 0);
                undone(matrix, x, y)
            };
            match self.stacked_overlays(root) {
                None => {
                    if Self::hit_overlays_turned_observed(&self.root, root, x - self.root.x, y - self.root.y, undo, &mut path, probe) {
                        return Some(path);
                    }
                }
                Some(stacked) => {
                    // What's drawn last gets the first chance.
                    for overlay in stacked.iter().rev() {
                        if let Some(found) = self.hit_overlay_at_observed(root, overlay, x - self.root.x, y - self.root.y, undo, probe) {
                            return Some(found);
                        }
                    }
                }
            }
        }
        if let Some(found) = self.hit_lifted_turned_observed(root, x, y, undo, probe) {
            return Some(found);
        }
        Self::hit_box_turned_observed(&self.root, root, x, y, undo, &mut path, probe).then_some(path)
    }

    #[cfg(test)]
    fn hit_box_turned(
        current: &LayoutBox,
        node: &Node,
        x: f32,
        y: f32,
        undo: &impl Fn(&Element, Rect) -> Option<[f32; 6]>,
        path: &mut Vec<u32>,
    ) -> bool {
        Self::hit_box_turned_observed(current, node, x, y, undo, path, &mut NoProbe)
    }

    fn hit_box_turned_observed(
        current: &LayoutBox,
        node: &Node,
        x: f32,
        y: f32,
        undo: &impl Fn(&Element, Rect) -> Option<[f32; 6]>,
        path: &mut Vec<u32>,
        probe: &mut impl HitProbe,
    ) -> bool {
        let rect = Rect::new(current.x, current.y, current.width, current.height);
        let (x, y) = {
            let matrix = undo(node, rect);
            probe.transform(matrix, rect, x, y, path.len());
            undone(matrix, x, y)
        };
        if !probe.contains(rect, x, y, path.len()) {
            // What's placed by itself in it can stand out of it, as a toast does from sonner's list, which is a fixed box
            // with nothing in its flow and so 0px tall. So can what's in its flow, where it doesn't cut what's in it off, as
            // Embla's slides do from its track, a row as wide as one slide, moved along by its transform.
            if current.has(SCROLLS) || current.children.is_empty() {
                return false;
            }
            let flowing_out = node.props.is_empty() || !clips(node);
            if !flowing_out && !current.has(CONTAINS_ABSOLUTE) {
                return false;
            }
            let (width, height) = (current.width, current.height);
            let (inner_x, inner_y) = (x - current.x, y - current.y);
            // What's drawn on top gets the first chance, by `z-index` where something says one.
            let order = current.has(STACKED).then(|| stacking_order(node));
            let count = order.as_ref().map_or(current.children.len().min(node.children.len()), Vec::len);
            for position in (0..count).rev() {
                let index = order.as_ref().map_or(position, |order| order[position]);
                let (child, child_node) = (&current.children[index], &node.children[index]);
                let stands_out = flowing_out
                    && !child.has(IS_OVERLAY)
                    && (child.x < 0.0 || child.y < 0.0 || child.x + child.width > width || child.y + child.height > height);
                if !(child.contributes_absolute() || stands_out) || child.lets_pointer_through() {
                    continue;
                }
                path.push(index as u32);
                if Self::hit_box_turned_observed(child, child_node, inner_x, inner_y, undo, path, probe) {
                    return true;
                }
                path.pop();
            }
            return false;
        }
        if current.has(PARAGRAPH) {
            return true;
        }
        let (inner_x, inner_y) = (x - current.x, y - current.y);
        if current.has(STACKED) {
            // What's drawn on top gets the first chance.
            for index in stacking_order(node).into_iter().rev() {
                let child = &current.children[index];
                if child.has(IS_OVERLAY) || child.lets_pointer_through() {
                    continue;
                }
                path.push(index as u32);
                if Self::hit_box_turned_observed(child, &node.children[index], inner_x, inner_y, undo, path, probe)
                {
                    return true;
                }
                path.pop();
            }
            return !current.has(CONTENTS);
        }
        for index in hit_order(current) {
            let (Some(child), Some(child_node)) = (current.children.get(index), node.children.get(index)) else {
                continue;
            };
            if child.has(IS_OVERLAY) || child.lets_pointer_through() {
                continue;
            }
            path.push(index as u32);
            if Self::hit_box_turned_observed(child, child_node, inner_x, inner_y, undo, path, probe) {
                return true;
            }
            path.pop();
        }
        !current.has(CONTENTS)
    }

    /// How much of `path`, found at this point by [`hit_test`](Self::hit_test) or [`hit_test_turned`](Self::hit_test_turned) (with
    /// the same `undo`), is hit once rounded corners count, as CSS has it (Chromium's `HitTestClippedOutByBorder`): a box with
    /// rounded corners isn't hit outside their curves, made to fit the box as they're drawn (CSS Backgrounds 3, 5.5), and nor is
    /// what it clips (`clip`); what's in it that isn't clipped still is. What isn't hit leaves what it's in hit, as that's under it.
    pub fn kept_inside_corners(
        &self,
        root: &Node,
        path: &[u32],
        x: f32,
        y: f32,
        undo: &impl Fn(&Element, Rect) -> Option<[f32; 6]>,
    ) -> usize {
        let mut current = &self.root;
        let mut node = root;
        let rect = Rect::new(current.x, current.y, current.width, current.height);
        let (mut x, mut y) = undone(undo(node, rect), x, y);
        (x, y) = (x - current.x, y - current.y);
        // Whether the point is outside each one's corners, from the first step of the path on, and how far down the path what
        // clips what's in it is missed. What's on top of everything (`position: fixed`, an overlay here) isn't clipped by what
        // it's in, nor is it under it, so only what's from the last of those on counts.
        let mut outside = Vec::new();
        let mut clipped_at = None;
        for (depth, &index) in path.iter().enumerate() {
            let (Some(child), Some(child_node)) =
                (current.children.get(index as usize), node.children.get(index as usize))
            else {
                break;
            };
            let rect = Rect::new(child.x, child.y, child.width, child.height);
            (x, y) = undone(undo(child_node, rect), x, y);
            if child.has(IS_OVERLAY) {
                outside.iter_mut().for_each(|one| *one = false);
                clipped_at = None;
            }
            let missed = is_page_element(child_node)
                && corners_of(child_node, rect.width, rect.height)
                    .is_some_and(|corners| !inside_corners(&corners, x - rect.x, y - rect.y, rect.width, rect.height));
            if missed && clips(child_node) && clipped_at.is_none() {
                clipped_at = Some(depth);
            }
            outside.push(missed);
            (x, y) = (x - child.x, y - child.y);
            current = child;
            node = child_node;
        }
        // Nothing in what clips is hit outside its corners either, and what isn't hit leaves what it's in.
        let mut kept = clipped_at.unwrap_or(outside.len());
        while kept > 0 && outside[kept - 1] {
            kept -= 1;
        }
        kept
    }

    /// Whether anything has said how far up it goes among what's next to it, which takes
    /// [`hit_test_turned`](Self::hit_test_turned) to find what's at a point, as that needs the tree.
    pub fn has_stacking(&self) -> bool {
        crate::style::ANY_Z.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether what's on top of everything is at this point, which is relative to `current`. What's
    /// drawn last gets the first chance.
    /// [`hit_overlays`](Self::hit_overlays) where things can have been turned or moved, as a Radix menu, which is `position: fixed`
    /// at 0, 0 (an overlay here) and moved by its transform to where it's shown, is clicked where it's shown.
    #[cfg(test)]
    pub(crate) fn hit_overlays_turned(
        current: &LayoutBox,
        node: &Node,
        x: f32,
        y: f32,
        undo: &impl Fn(&Element, Rect) -> Option<[f32; 6]>,
        path: &mut Vec<u32>,
    ) -> bool {
        Self::hit_overlays_turned_observed(current, node, x, y, undo, path, &mut NoProbe)
    }

    fn hit_overlays_turned_observed(
        current: &LayoutBox,
        node: &Node,
        x: f32,
        y: f32,
        undo: &impl Fn(&Element, Rect) -> Option<[f32; 6]>,
        path: &mut Vec<u32>,
        probe: &mut impl HitProbe,
    ) -> bool {
        for (index, child) in current.children.iter().enumerate().rev() {
            let Some(child_node) = node.children.get(index) else {
                continue;
            };
            path.push(index as u32);
            if child.has(CONTAINS_OVERLAY) {
                let rect = Rect::new(child.x, child.y, child.width, child.height);
                let (inner_x, inner_y) = {
                    let matrix = undo(child_node, rect);
                    probe.transform(matrix, rect, x, y, path.len());
                    undone(matrix, x, y)
                };
                if Self::hit_overlays_turned_observed(child, child_node, inner_x - child.x, inner_y - child.y, undo, path, probe) {
                    return true;
                }
            }
            if child.has(IS_OVERLAY)
                && !child.has(OVERLAY_IS_INERT)
                && Self::hit_box_turned_observed(child, child_node, x, y, undo, path, probe)
            {
                return true;
            }
            path.pop();
        }
        false
    }

    /// What's at this point in what's on top of everything at `overlay`, as `hit_overlays_turned` finds it there, without
    /// what's in it that's on top too, which is asked about by itself.
    fn hit_overlay_at_observed(
        &self,
        root: &Node,
        overlay: &[u32],
        mut x: f32,
        mut y: f32,
        undo: &impl Fn(&Element, Rect) -> Option<[f32; 6]>,
        probe: &mut impl HitProbe,
    ) -> Option<Vec<u32>> {
        let (mut current, mut node) = (&self.root, root);
        let (&last, above) = overlay.split_last()?;
        for (depth, &index) in above.iter().enumerate() {
            let child = current.children.get(index as usize)?;
            let child_node = node.children.get(index as usize)?;
            let rect = Rect::new(child.x, child.y, child.width, child.height);
            (x, y) = {
                let matrix = undo(child_node, rect);
                probe.transform(matrix, rect, x, y, depth + 1);
                undone(matrix, x, y)
            };
            (x, y) = (x - child.x, y - child.y);
            (current, node) = (child, child_node);
        }
        let child = current.children.get(last as usize)?;
        let child_node = node.children.get(last as usize)?;
        let mut path = overlay.to_vec();
        (child.has(IS_OVERLAY) && !child.has(OVERLAY_IS_INERT) && Self::hit_box_turned_observed(child, child_node, x, y, undo, &mut path, probe))
            .then_some(path)
    }

    fn hit_overlays(current: &LayoutBox, x: f32, y: f32, path: &mut Vec<u32>) -> bool {
        for (index, child) in current.children.iter().enumerate().rev() {
            path.push(index as u32);
            if child.has(CONTAINS_OVERLAY)
                && Self::hit_overlays(child, x - child.x, y - child.y, path)
            {
                return true;
            }
            if child.has(IS_OVERLAY)
                && !child.has(OVERLAY_IS_INERT)
                && Self::hit_box(child, x, y, path)
            {
                return true;
            }
            path.pop();
        }
        false
    }

    /// Whether `x`, `y` is in `current`, with the path to the innermost thing it's in added to `path` if so. It goes down with a
    /// list of where it's got to in each level rather than a call for each, so a tree of any depth takes no more of the stack.
    fn hit_box(current: &LayoutBox, x: f32, y: f32, path: &mut Vec<u32>) -> bool {
        let inside = |at: &LayoutBox, x: f32, y: f32| Rect::new(at.x, at.y, at.width, at.height).contains(x, y);
        if !inside(current, x, y) {
            // What's placed by itself in it can stand out of it (see `hit_box_turned`).
            if !current.has(CONTAINS_ABSOLUTE) || current.has(SCROLLS) {
                return false;
            }
            let (inner_x, inner_y) = (x - current.x, y - current.y);
            for (index, child) in current.children.iter().enumerate().rev() {
                if !child.contributes_absolute() || child.lets_pointer_through() {
                    continue;
                }
                path.push(index as u32);
                if Self::hit_box(child, inner_x, inner_y, path) {
                    return true;
                }
                path.pop();
            }
            return false;
        }
        // Which part of a paragraph it is takes knowing where its words went, so it's for what knows that.
        if current.has(PARAGRAPH) {
            return true;
        }
        // Each level's box, where the point is inside it, the next of its children to try, as a place in the order they're tried
        // in (`hit_order`: what's placed by itself first, as it's drawn over the flow), and that order where it isn't theirs.
        let order_of = |at: &LayoutBox| at.has(CONTAINS_ABSOLUTE).then(|| hit_order(at).collect::<Vec<usize>>());
        let mut levels: Vec<(&LayoutBox, f32, f32, usize, Option<Vec<usize>>)> =
            vec![(current, x - current.x, y - current.y, 0, order_of(current))];
        while let Some(&(at, x, y, next, _)) = levels.last() {
            let tried = {
                let order = levels.last().and_then(|level| level.4.as_deref());
                let index_at = |place: usize| match order {
                    Some(order) => order.get(place).copied(),
                    None => (place < at.children.len()).then_some(place),
                };
                let mut place = next;
                loop {
                    let Some(index) = index_at(place) else {
                        break None;
                    };
                    let child = &at.children[index];
                    if !child.has(IS_OVERLAY) && !child.lets_pointer_through() {
                        break Some((place, index));
                    }
                    place += 1;
                }
            };
            if let Some((place, index)) = tried {
                if let Some(level) = levels.last_mut() {
                    level.3 = place + 1;
                }
                let child = &at.children[index];
                if inside(child, x, y) {
                    path.push(index as u32);
                    if child.has(PARAGRAPH) {
                        return true;
                    }
                    levels.push((child, x - child.x, y - child.y, 0, order_of(child)));
                }
                continue;
            }
            // In none of its children: it's what's hit, unless it's no box of its own.
            if !at.has(CONTENTS) {
                return true;
            }
            levels.pop();
            if levels.is_empty() {
                return false;
            }
            path.pop();
        }
        false
    }

    /// The tree as text, with one line per node showing its position, for reading a
    /// layout and for checking one in a test:
    ///
    /// ```text
    /// window 0,0 400x300
    ///   button "Save" 0,0 400x16
    /// ```
    pub fn outline(&self, root: &Node) -> String {
        let mut out = String::new();
        Self::outline_box(&self.root, root, 0.0, 0.0, 0, &mut out);
        out
    }

    fn outline_box(
        current: &LayoutBox,
        node: &Node,
        parent_x: f32,
        parent_y: f32,
        level: usize,
        out: &mut String,
    ) {
        let (x, y) = (parent_x + current.x, parent_y + current.y);
        let _ = write!(
            out,
            "{:indent$}{}",
            "",
            node.role.name(),
            indent = level * 2
        );
        if !node.name.is_empty() {
            let _ = write!(out, " {:?}", node.name.as_str());
        }
        let _ = writeln!(out, " {x},{y} {}x{}", current.width, current.height);
        for (child_box, child) in current.children.iter().zip(&node.children) {
            Self::outline_box(child_box, child, x, y, level + 1, out);
        }
    }
}

#[cfg(test)]
#[path = "lifted_tests.rs"]
mod lifted_tests;
#[cfg(test)]
#[path = "hit_region_tests.rs"]
mod hit_region_tests;

#[cfg(test)]
mod scroll_offsets_tests {
    use super::*;
    use crate::FixedMetrics;
    use rsui_core::{Element, Role, diff};

    fn page(x: f64, y: f64, rtl: bool, left_bar: bool, fill: bool, padding: f64) -> Node {
        let content = Element::new(Role::Group)
            .prop("width", 300.0).prop("height", 900.0)
            .prop("margin_right", 13.0).prop("margin_bottom", 9.0)
            .child(Element::new(Role::Group).prop("width", 850.0).prop("height", 20.0))
            .child(Element::new(Role::Group).prop("width", 90.0).prop("height", 40.0)
                .prop("scroll_x", 3.0).prop("scroll_y", 11.0)
                .child(Element::new(Role::Group).prop("width", 250.0).prop("height", 180.0)));
        Element::new(Role::Window)
            .child(Element::new(Role::Group).prop("width", 300.0).prop("height", 180.0)
                .prop("scroll_x", x).prop("scroll_y", y)
                .prop("fill_across", fill).prop("padding", padding)
                .prop("text_direction", if rtl { "rtl" } else { "ltr" })
                .prop("scrollbar_room", 17.0).prop("scrollbar_left", left_bar)
                .child(content)
                .child(Element::new(Role::Group).prop("overlay", "fill")
                    .prop("width", 2000.0).prop("height", 2000.0)))
            .into_node()
    }

    fn compare_tree(layout: &Layout, root: &Node) {
        fn visit(layout: &Layout, node: &Node, path: &mut Vec<u32>) {
            let full = layout.scroll_info(path, node).map(|info| [info.x, info.y]);
            assert_eq!(layout.scroll_offsets(path, node), full, "path {path:?}");
            for (index, child) in node.children.iter().enumerate() {
                path.push(index as u32);
                visit(layout, child, path);
                path.pop();
            }
        }
        visit(layout, root, &mut Vec::new());
    }

    #[test]
    fn offsets_match_full_info_for_nested_horizontal_rtl_padding_and_scrollbars() {
        for rtl in [false, true] {
            for left in [false, true] {
                for fill in [false, true] {
                    for padding in [0.0, 7.25] {
                        let mut before = page(0.0, 0.0, rtl, left, fill, padding);
                        let mut layout = Layout::new();
                        layout.compute(&before, Size::new(500.0, 400.0), &mut FixedMetrics::default());
                        compare_tree(&layout, &before);
                        for (x, y) in [(31.5, 82.25), (10000.0, 10000.0), (0.0, 0.0)] {
                            let after = page(x, y, rtl, left, fill, padding);
                            if layout.apply_needs_layout_since(&diff(&before, &after), Some(&before)) {
                                layout.compute(&after, Size::new(500.0, 400.0), &mut FixedMetrics::default());
                            }
                            compare_tree(&layout, &after);
                            before = after;
                        }
                        layout.compute(&before, Size::new(420.0, 350.0), &mut FixedMetrics::default());
                        compare_tree(&layout, &before);
                    }
                }
            }
        }
    }

    #[test]
    fn lists_empty_scrollers_and_missing_layout_keep_the_full_offset_semantics() {
        let root = Element::new(Role::Window)
            .child(Element::new(Role::List).prop("width", 240.0).prop("height", 90.0)
                .prop("row_height", 18.5).prop("count", 37).prop("first_visible", 2.25)
                .prop("text_direction", "rtl").prop("fill_across", true))
            .child(Element::new(Role::Group).prop("height", 40.0)
                .prop("scroll_y", 10.0).prop("padding", 7.0))
            .into_node();
        let mut layout = Layout::new();
        assert_eq!(layout.scroll_offsets(&[], &root), None);
        layout.compute(&root, Size::new(500.0, 400.0), &mut FixedMetrics::default());
        compare_tree(&layout, &root);
        assert_eq!(layout.scroll_offsets(&[0], &root.children[0]), Some([0.0, 2.25 * 18.5]));
        assert_eq!(layout.scroll_offsets(&[0, 99], &root.children[0]), None);
    }
}
