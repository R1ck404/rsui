//! The layout algorithm. A parent asks each child how big it wants to be, decides the
//! final sizes, then lays each child out at its final size.

use rsui_core::{Element, Node};

use crate::measure::Measure;
use crate::share::Mix;
use crate::style::{
    Align, Direction, Justify, Length, NAMES, Placement, Style, TablePart, is_contents, is_inert,
    is_paragraph, read_align, read_length, shows_own_text, sized_by_parent_height,
};

#[path = "table.rs"]
mod table;
use crate::tree::{
    BOTTOM_ESCAPES, CHILD_DIRTY, CONTAINS_ABSOLUTE, CONTAINS_OVERLAY, CONTENTS, FORGOT_A_QUESTION,
    HAS_LAYOUT, HAS_MEASURE, IS_ABSOLUTE, IS_OVERLAY, LayoutBox, MANY_CHILDREN_DIRTY,
    OVERLAY_IS_INERT, PARAGRAPH, Rect, SCROLLS, SELF_DIRTY, SHIFTED, SPLICED, STACKED, Size,
    TOP_ESCAPES,
};

/// How many children a block needs before what's after a change in it is moved rather than laid out again: see
/// [`Pass::solve_moved`].
const MOVED_FROM: usize = 8;
/// How many of them, from the one before what changed to the one after, are laid out again at most, before it's quicker to lay
/// them all out.
const MOST_MOVED: usize = 64;

/// The space available in one direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Axis {
    pub(crate) value: f32,
    /// The size must be exactly `value`. Otherwise, `value` is the most it should be,
    /// and infinity means there's no limit.
    pub(crate) exact: bool,
}

impl Axis {
    const UNBOUNDED: Self = Self::at_most(f32::INFINITY);
    /// As narrow (or short) as it can be, which isn't held to any room, as CSS's min-content is. It's less than nothing, which no
    /// room ever is, so that it's kept apart from a question about no room at all.
    const MIN_CONTENT: Self = Self::at_most(-1.0);

    const fn exact(value: f32) -> Self {
        Self { value, exact: true }
    }

    const fn at_most(value: f32) -> Self {
        Self {
            value,
            exact: false,
        }
    }
}

/// The question put to a node: how big is it, given this space?
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Key {
    pub(crate) width: Axis,
    pub(crate) height: Axis,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Only the size is needed, so children's positions aren't touched.
    Measure,
    /// Decide this node's size, and place everything inside it.
    Layout,
}

/// What a parent knows that decides what its children are asked.
#[derive(Clone, Copy)]
struct FlexRoom {
    row: bool,
    align: Align,
    definite_main: Option<f32>,
    definite_cross: Option<f32>,
    /// What a size along that's a percentage is of: `definite_main`, or, in a scroller or what it lays its content out in, how
    /// tall it shows, though what's in it can be taller (see `VIEW_HEIGHT` in style.rs).
    percent_main: Option<f32>,
    cross_limit: f32,
    /// It's being asked how narrow it can be along, as CSS's min-content is.
    least_main: bool,
    /// The margins of what's in it go together, as a CSS block's do, which a group with nothing said about it, as a component's
    /// fragment is, passes through, as it isn't a box on a page.
    collapse: bool,
}

/// A group that nothing's said anything about, or that's no box, as `display: contents` says, which is what a component's fragment
/// is: in a block, the margins of what's in it go through it, as if it weren't there.
fn is_bare_group(element: &Element) -> bool {
    element.role == rsui_core::Role::Group && (element.props.is_empty() || is_contents(element))
}

/// What's next for a child, after [`start_item`].
enum ItemStart {
    /// It's placed by what it says, so it takes no room here.
    Absolute,
    /// It's on top of everything, so it takes no room here.
    Overlay,
    /// Its size is known without asking it.
    Decided,
    /// It has to be asked this.
    Ask(Key),
}

/// One child, while its parent works out the sizes.
struct Item {
    style: Style,
    /// The child's size along the parent's direction, once decided.
    main: f32,
    /// The size it starts from, before any space is shared out.
    base: f32,
    min_main: f32,
    max_main: f32,
    frozen: bool,
    cross: f32,
    /// The space across the parent's direction to ask the child about.
    cross_axis: Axis,
    /// What the child already said it wants, if it was asked: its size along the
    /// parent's direction, and across it. If the child gets just what it asked for,
    /// there's no need to ask it again.
    wanted: Option<(f32, f32)>,
    /// Which of the parent's children this is.
    index: usize,
}

impl Item {
    /// How much of what there's too little room for it gives up, against the others: its `flex-shrink` times how big it starts
    /// inside its padding and border, as CSS has it (Flexbox 1 §9.7), so that what's mostly padding gives up little of it.
    fn shrink_factor(&self, row: bool) -> f32 {
        let (across, down) = padding_totals(&self.style);
        self.style.shrink * (self.base - if row { across } else { down }).max(0.0)
    }
}

pub(crate) fn compute<M: Measure>(
    root: &mut LayoutBox,
    node: &Node,
    available: Size,
    measure: &mut M,
    list_sizes: &mut Vec<(usize, f32, f32)>,
    paragraphs: &mut bool,
) -> (usize, usize) {
    list_sizes.clear();
    let mut pass = Pass {
        measure,
        solved: 0,
        looked_at: 0,
        paragraphs,
        window_width: available.width,
        window_height: available.height,
        list_sizes,
    };
    if root.has(HAS_LAYOUT) {
        pass.refresh(root, node);
    }
    let key = Key {
        width: Axis::exact(available.width),
        height: Axis::exact(available.height),
    };
    pass.solve(root, node, key, Mode::Layout);
    let counts = (pass.solved, pass.looked_at);
    if root.has(CONTAINS_OVERLAY) {
        place_overlays(root, node, root.x, root.y, available);
    }
    counts
}

/// Works out where everything on top of everything else goes. That depends on where its parent has
/// ended up in the window, which isn't known until everything's been placed, as boxes only know where
/// they are in their parents. This only goes down where there's something to find.
pub(crate) fn place_overlays(current: &mut LayoutBox, node: &Element, x: f32, y: f32, window: Size) {
    let (width, height) = (current.width, current.height);
    for (child_box, child) in current.children.iter_mut().zip(&node.children) {
        if child_box.has(IS_OVERLAY) {
            let (style, _) = Style::read(&child.props);
            let placement = style.overlay.unwrap_or(Placement::Below);
            if placement == Placement::Inset {
                // From the window's top left, or where what it's in starts along a way it says nothing about.
                let sides = Sides::read(child, &style, window);
                let (left, top) = sides.offsets(window, (child_box.width, child_box.height));
                child_box.x = left.unwrap_or(x) - x;
                child_box.y = top.unwrap_or(y) - y;
                if child_box.has(CONTAINS_OVERLAY) {
                    let (child_x, child_y) = (x + child_box.x, y + child_box.y);
                    place_overlays(child_box, child, child_x, child_y, window);
                }
                continue;
            }
            let (left, top) = overlay_position(
                placement,
                (x, y, width, height),
                (child_box.width, child_box.height),
                window,
                style.overlay_fixed,
            );
            child_box.x = left - x;
            child_box.y = top - y;
        }
        if child_box.has(CONTAINS_OVERLAY) {
            let (child_x, child_y) = (x + child_box.x, y + child_box.y);
            place_overlays(child_box, child, child_x, child_y, window);
        }
    }
}

/// Notes whether what's placed by itself lets the pointer through, as what's drawn over an editor does.
pub(crate) fn mark_absolute_inert(child_box: &mut LayoutBox, child: &Element) {
    // What a `clip-path` cuts all of away shows nothing to be pointed at, as a page's `sr-only` text (`clip-path: inset(50%)`)
    // in an icon button: the press is the button's. It can still have the keyboard, as that isn't about where it is.
    if is_inert(child) || clipped_away(child) {
        child_box.flags |= crate::tree::ABSOLUTE_IS_INERT;
    } else {
        child_box.flags &= !crate::tree::ABSOLUTE_IS_INERT;
    }
    // Said not to be, as CSS's `pointer-events: auto` says it in something that's `none`: see `lifted` in the tree.
    if crate::style::takes_pointer(child) {
        child_box.flags |= crate::tree::ABSOLUTE_TAKES_POINTER;
    } else {
        child_box.flags &= !crate::tree::ABSOLUTE_TAKES_POINTER;
    }
}

/// Whether `clip_path` cuts away all of something, as `inset(50%)` does: insets of a share of it that meet across or down.
fn clipped_away(child: &Element) -> bool {
    let Some(rsui_core::PropValue::Str(said)) = child.props.get("clip_path") else {
        return false;
    };
    let Some(inside) = said.as_str().trim().strip_prefix("inset(").and_then(|rest| rest.strip_suffix(')')) else {
        return false;
    };
    let inside = inside.split(" round ").next().unwrap_or("");
    let shares: Vec<f32> = inside
        .split_whitespace()
        .map_while(|word| word.strip_suffix('%').and_then(|share| share.parse().ok()))
        .collect();
    let [top, right, bottom, left] = match shares[..] {
        [all] => [all, all, all, all],
        [down, across] => [down, across, down, across],
        [top, across, bottom] => [top, across, bottom, across],
        [top, right, bottom, left] => [top, right, bottom, left],
        _ => return false,
    };
    top + bottom >= 100.0 || left + right >= 100.0
}

/// Notes that this child is on top of everything, and what the pointer does about that.
fn mark_overlay(child_box: &mut LayoutBox, child: &Element) {
    child_box.flags |= IS_OVERLAY;
    if is_inert(child) {
        child_box.flags |= OVERLAY_IS_INERT;
    } else {
        child_box.flags &= !OVERLAY_IS_INERT;
    }
}

/// What `Placement::At` says less than, a place it's from the far side of the window by what's past it: what's fixed by its right
/// or bottom side, as CSS's `position: fixed; right: 16px` is, whose size isn't known until it's laid out.
pub const FROM_END: f32 = 1e6;

/// Where something of this size goes, from the top left of the window.
fn overlay_position(
    placement: Placement,
    (x, y, width, height): (f32, f32, f32, f32),
    (own_width, own_height): (f32, f32),
    window: Size,
    fixed: bool,
) -> (f32, f32) {
    // Whichever side has room, starting with the one asked for, then whichever has more.
    let along = |near: f32, far: f32, own: f32, limit: f32, first_after: bool| {
        let after = far;
        let before = near - own;
        let after_fits = after + own <= limit;
        let before_fits = before >= 0.0;
        match (first_after, after_fits, before_fits) {
            (true, true, _) | (false, true, false) => after,
            (false, _, true) | (true, false, true) => before,
            _ if limit - far >= near => after,
            _ => before,
        }
    };
    let (left, top) = match placement {
        Placement::Below => (x, along(y, y + height, own_height, window.height, true)),
        Placement::Above => (x, along(y, y + height, own_height, window.height, false)),
        Placement::Right => (along(x, x + width, own_width, window.width, true), y),
        Placement::Left => (along(x, x + width, own_width, window.width, false), y),
        Placement::Center => (
            (window.width - own_width) / 2.0,
            (window.height - own_height) / 2.0,
        ),
        Placement::Fill | Placement::Inset => (0.0, 0.0),
        // Far past the start is from the far side, by as much again: see `FROM_END`.
        Placement::At(at_x, at_y) => {
            let at = (
                if at_x <= -FROM_END { window.width - own_width - (-at_x - FROM_END) } else { at_x },
                if at_y <= -FROM_END { window.height - own_height - (-at_y - FROM_END) } else { at_y },
            );
            // A page's `position: fixed` is where it's said to be, even past the window's edge, as CSS has it: a dialog at
            // `left: 50%` that's wider than the half of the window that's left is still there, to be moved back by its
            // `translate(-50%)`, and what's put far off to the left to hide it stays hidden. A popup of rsui's own is kept in.
            if fixed {
                return at;
            }
            at
        }
    };
    // Off the far edge moves it back, and being too big for the window shows its start.
    (
        left.min(window.width - own_width).max(0.0),
        top.min(window.height - own_height).max(0.0),
    )
}

struct Pass<'a, M> {
    measure: &'a mut M,
    /// How many times a node's size has been worked out in this pass.
    solved: usize,
    /// How many things in what's laid out one after another have been looked at in this pass, whether or not they were laid out.
    looked_at: usize,
    /// Set if there's ever been a paragraph, which most programs never have.
    paragraphs: &'a mut bool,
    window_width: f32,
    window_height: f32,
    /// Lists that have the wrong idea of how many rows fit in them, or for a grid, how many items fit across, by the address
    /// of their element, and how many do (0 across for a list that isn't a grid).
    list_sizes: &'a mut Vec<(usize, f32, f32)>,
}

impl<M: Measure> Pass<'_, M> {
    /// Updates what has changed below `current`, and returns whether its size is
    /// different as a result. If it isn't, nothing above it needs to change.
    fn refresh(&mut self, current: &mut LayoutBox, node: &Element) -> bool {
        if !current.has(SELF_DIRTY | CHILD_DIRTY) {
            return false;
        }
        // Which children changed size, from the first to the last, when that's all that's changed.
        let mut resized: Option<(usize, usize)> = None;
        let needs_solving = if current.has(SELF_DIRTY) {
            true
        } else {
            let many = current.has(MANY_CHILDREN_DIRTY);
            current.flags &= !(CHILD_DIRTY | MANY_CHILDREN_DIRTY);
            let mut changed = false;
            let mut overlays_found = false;
            let mut absolutes_found = false;
            if many {
                for (index, (child_box, child)) in current.children.iter_mut().zip(&node.children).enumerate() {
                    if self.refresh(child_box, child) {
                        changed = true;
                        resized = Some(resized.map_or((index, index), |(first, _)| (first, index)));
                    }
                }
            } else {
                let index = current.dirty_child as usize;
                if let (Some(child_box), Some(child)) =
                    (current.children.get_mut(index), node.children.get(index))
                {
                    changed = self.refresh(child_box, child);
                    overlays_found = child_box.has(IS_OVERLAY | CONTAINS_OVERLAY);
                    absolutes_found = child_box.contributes_absolute();
                    if changed {
                        resized = Some((index, index));
                    }
                }
            }
            // A popup can come and go inside something, without changing the size of anything.
            if many || overlays_found != current.has(CONTAINS_OVERLAY) {
                current.update_contains_overlay();
            }
            // Like overlays, lifted targets can change without changing a size.
            // If one dirty child agrees with the old OR, no sibling scan is needed.
            if many || absolutes_found != current.has(CONTAINS_ABSOLUTE) {
                current.update_contains_absolute();
            }
            changed
        };
        if !needs_solving {
            // Paint eligibility can change without changing any box dimensions.
            current.update_clipped_paint(node);
            // What's placed by itself in what changed, from this, is placed again, as what it's in placed it from itself.
            let size = Size::new(current.width, current.height);
            self.place_passed_absolutes(current, node, size);
            return false;
        }
        // Children that have only been put in, taken out or changed size move what's after them, which is all that's done if it can be.
        let spliced = current.has(SPLICED);
        current.flags &= !SPLICED;
        if (spliced || resized.is_some())
            && let Some(changed) = self.solve_moved(current, node, resized)
        {
            let size = Size::new(current.width, current.height);
            self.place_passed_absolutes(current, node, size);
            return changed;
        }
        if spliced {
            for child_box in &mut current.children {
                child_box.flags &= !SHIFTED;
            }
        }
        let had_layout = current.has(HAS_LAYOUT);
        let had_measure = current.has(HAS_MEASURE);
        let forgot_a_question = current.has(FORGOT_A_QUESTION);
        let old_size = Size::new(current.width, current.height);
        let old_measured = current.measured;
        let layout_key = current.layout_key();
        let measure_key = current.measure_key();
        // The other questions it's been asked, each to be asked again, as what it's in may ask any of them.
        let asked = std::mem::take(&mut current.answers);
        current.clear_results();
        current.flags &= !SELF_DIRTY;
        // Its parent has only ever asked about its size, if anything, and may need to
        // ask again.
        if !had_layout {
            return true;
        }
        let new_size = self.solve(current, node, layout_key, Mode::Layout);
        let mut changed = forgot_a_question || new_size != old_size;
        if had_measure {
            let measured = self.solve(current, node, measure_key, Mode::Measure);
            changed |= [measured.width, measured.height] != old_measured;
        }
        for answer in &asked {
            if changed {
                break;
            }
            changed |= self.solve(current, node, answer.key(), Mode::Measure) != answer.size();
        }
        changed
    }

    /// What's in a block of things one under another, as a page's paragraphs are, after some of them have been put in or taken out,
    /// or have changed size: what's from the one before the first of them to the one after the last is laid out again, as what's in
    /// a block of its own with the same width and none of its own room above and below, which places them as they'd be placed here,
    /// from the one before. What's after them moves down by as much as the one after did, without being laid out again, and the
    /// block is that much taller. Gives back whether its size changed, as [`refresh`](Self::refresh) does.
    ///
    /// It's only when where each goes follows from where the one before it went: one under another, from the top, with no wrapping,
    /// and nothing that shares out room between them, which needs the block's height to be what's in it, rather than one it says or
    /// a limit; and nothing placed by itself, moved, or on top of everything among them, as where that goes is from the block. It's
    /// `None` otherwise, with nothing changed that laying it out as usual won't put right.
    #[inline(never)]
    fn solve_moved(&mut self, current: &mut LayoutBox, node: &Element, resized: Option<(usize, usize)>) -> Option<bool> {
        let count = node.children.len();
        if count < MOVED_FROM
            || current.children.len() != count
            || !current.has(HAS_LAYOUT)
            || current.has(FORGOT_A_QUESTION | SCROLLS | PARAGRAPH | CONTENTS)
            || !current.answers.is_empty()
        {
            return None;
        }
        let in_flow = |child_box: &LayoutBox| !child_box.has(IS_OVERLAY | IS_ABSOLUTE);
        let (first, last) = match resized {
            Some(range) => range,
            None => {
                // What's been put in hasn't been laid out, and what's been changed, or comes after what was taken out, says so.
                let mut range: Option<(usize, usize)> = None;
                for (index, child_box) in current.children.iter().enumerate() {
                    let touched = !child_box.has(HAS_LAYOUT)
                        || (in_flow(child_box) && child_box.has(SELF_DIRTY | CHILD_DIRTY | SHIFTED));
                    if touched {
                        range = Some(range.map_or((index, index), |(first, _)| (first, index)));
                    }
                }
                range?
            }
        };
        let (style, _) = Style::read(&node.props);
        if style.row_height.is_some()
            || style.scroll_x.is_some()
            || style.scroll_y.is_some()
            || style.grid
            || style.wrap
            || style.table != TablePart::None
            || style.aspect.is_some()
            || style.fit_width
            || shows_own_text(node)
            || is_paragraph(node)
            || (!style.collapse_margins && out_of_order(node))
        {
            return None;
        }
        // Only what's placed by itself has changed size, as what's drawn over an editor does when what's selected does: what's
        // one after another in it is where it was, and so is the block, so that's all that's placed again.
        if resized.is_some()
            && current.children[first..=last].iter().all(|child_box| !in_flow(child_box) && child_box.has(HAS_LAYOUT))
            && !current.children.iter().any(|child_box| child_box.has(CONTENTS))
        {
            #[cfg(debug_assertions)]
            let unchanged = current.clone();
            let size = Size::new(current.width, current.height);
            self.place_apart(current, node, size);
            current.flags &= !(SELF_DIRTY | CHILD_DIRTY | MANY_CHILDREN_DIRTY);
            #[cfg(debug_assertions)]
            self.check_moved(unchanged, current, node, current.layout_key(), size);
            return Some(false);
        }
        if style.direction == Direction::Row
            || style.reverse
            || style.cross_reverse
            || style.fit_height
            || !matches!(style.height, Length::Auto)
            || (style.collapse_margins && on_a_line(node))
        {
            return None;
        }
        let old = Size::new(current.width, current.height);
        let layout_key = current.layout_key();
        let measure_key = current.measure_key();
        let had_measure = current.has(HAS_MEASURE);
        // Asked about its size with the same width and room to be as tall as it likes, it's as tall as what's in it.
        let measure_open = had_measure && measure_key.width == layout_key.width && !measure_key.height.exact;
        if !layout_key.width.exact || old.width != layout_key.width.value || (had_measure && !measure_open) {
            return None;
        }
        // Told how tall to be, that has to have been how tall what's in it is, which is then where the new height is told.
        if layout_key.height.exact && !(measure_open && current.measured == [old.width, old.height]) {
            return None;
        }
        // Its height has to be what's in it, rather than as low or as high as it can be.
        let free = |height: f32| {
            let room = |key: Axis| key.exact || key.value < 0.0 || height < key.value;
            (height > style.min_height || style.min_height <= 0.0)
                && height < style.max_height
                && room(layout_key.height)
                && (!measure_open || room(measure_key.height))
        };
        if !free(old.height) {
            return None;
        }
        let before = (0..first).rev().find(|&at| in_flow(&current.children[at]))?;
        let after = (last + 1..count).find(|&at| in_flow(&current.children[at]))?;
        if after - before >= MOST_MOVED || current.children[before..=after].iter().any(|child_box| !in_flow(child_box)) {
            return None;
        }
        for child in &node.children[before..=after] {
            let (child_style, _) = Style::read(&child.props);
            if child_style.moved || child_style.absolute || child_style.overlay.is_some() {
                return None;
            }
        }
        // What has no box of its own is laid out as part of this, and, told how tall to be, what's sized from that changes size
        // with it.
        if node.children.iter().any(|child| is_contents(child))
            || (layout_key.height.exact && node.children.iter().any(|child| sized_by_parent_height(child)))
        {
            return None;
        }
        #[cfg(debug_assertions)]
        let unchanged = current.clone();
        // Those, in a block of their own, with the same width, and its own padding at the sides, as far down as they go.
        let window = Element {
            key: node.key.clone(),
            role: node.role,
            name: node.name.clone(),
            props: node.props.clone(),
            actions: node.actions,
            children: node.children[before..=after].iter().cloned().collect(),
        };
        let window_style = Style {
            padding_top: 0.0,
            padding_bottom: 0.0,
            min_height: 0.0,
            max_height: f32::INFINITY,
            ..style
        };
        let mut scratch = LayoutBox {
            children: current.children[before..=after].iter_mut().map(std::mem::take).collect(),
            ..LayoutBox::default()
        };
        let was_first = (scratch.children[0].x, scratch.children[0].y);
        let was_after = scratch.children[after - before].y;
        self.solve_plainly(
            &mut scratch,
            &window,
            &window_style,
            Key { width: layout_key.width, height: Axis::UNBOUNDED },
            Mode::Layout,
        );
        let boxes = std::mem::take(&mut scratch.children);
        // The one before is where it was, and the rest of them are where they are from it.
        let offset = was_first.1 - boxes[0].y;
        let across = boxes[0].x == was_first.0;
        let moved = boxes[after - before].y + offset - was_after;
        for (slot, mut child_box) in current.children[before..=after].iter_mut().zip(boxes) {
            child_box.y += offset;
            child_box.flags &= !SHIFTED;
            *slot = child_box;
        }
        let height = old.height + moved;
        if !across || !free(height) {
            return None;
        }
        if moved != 0.0 {
            for child_box in &mut current.children[after + 1..] {
                if in_flow(child_box) {
                    child_box.y += moved;
                }
            }
        }
        let size = Size::new(old.width, height);
        self.place_apart(current, node, size);
        // Told how tall to be, it's to be told how tall it is now, which is what's kept, as what it's in asks again.
        let key = if layout_key.height.exact {
            Key { width: layout_key.width, height: Axis::exact(height) }
        } else {
            layout_key
        };
        current.flags &= !(HAS_LAYOUT | HAS_MEASURE | SELF_DIRTY | CHILD_DIRTY | MANY_CHILDREN_DIRTY);
        current.store_layout(key, size);
        if measure_open {
            current.store_measure(measure_key, size);
        }
        #[cfg(debug_assertions)]
        self.check_moved(unchanged, current, node, key, size);
        Some(size != old)
    }

    /// Places what's in this that's placed by itself, or on top of everything, once what's one after another in it is placed and
    /// its size is known, as [`solve_flex`](Self::solve_flex) does, and notes again what's in it.
    fn place_apart(&mut self, current: &mut LayoutBox, node: &Element, size: Size) {
        if current.children.iter().any(|child_box| child_box.has(IS_OVERLAY)) {
            self.solve_overlays(current, node);
        }
        if current.children.iter().any(|child_box| child_box.has(IS_ABSOLUTE)) {
            self.solve_absolutes(current, node, size);
        }
        self.place_passed_absolutes(current, node, size);
        if crate::style::ANY_Z.load(std::sync::atomic::Ordering::Relaxed) {
            note_stacking(current, node);
        }
        current.update_contains_overlay();
        current.update_contains_absolute();
        current.update_clipped_paint(node);
    }

    /// In a debug build, that what [`solve_moved`](Self::solve_moved) did is what laying it all out again does.
    #[cfg(debug_assertions)]
    fn check_moved(&mut self, mut unchanged: LayoutBox, current: &LayoutBox, node: &Element, key: Key, size: Size) {
        let lists = self.list_sizes.len();
        unchanged.clear_results();
        unchanged.flags &= !(SELF_DIRTY | CHILD_DIRTY | MANY_CHILDREN_DIRTY | SPLICED);
        let full = self.solve(&mut unchanged, node, key, Mode::Layout);
        self.list_sizes.truncate(lists);
        let near = |a: f32, b: f32| (a - b).abs() < 0.01 || (a.is_nan() && b.is_nan());
        assert!(near(full.width, size.width) && near(full.height, size.height), "moved to {size:?}, laid out to {full:?}");
        for (at, (moved, laid)) in current.children.iter().zip(&unchanged.children).enumerate() {
            assert!(
                near(moved.x, laid.x) && near(moved.y, laid.y) && near(moved.width, laid.width) && near(moved.height, laid.height),
                "child {at} of {}: moved to {:?}, laid out at {:?}",
                node.children.len(),
                (moved.x, moved.y, moved.width, moved.height),
                (laid.x, laid.y, laid.width, laid.height),
            );
        }
    }

    /// Answers `key` for this node, reusing an earlier answer if it's still good.
    fn solve(&mut self, current: &mut LayoutBox, node: &Element, key: Key, mode: Mode) -> Size {
        if let Some(size) = self.known_answer(current, key, mode) {
            return size;
        }
        let (style, _) = Style::read(&node.props);
        self.solve_anew(current, node, &style, key, mode)
    }

    /// The same as [`Self::solve`], for a parent that's already read its child's style. This is how a tree goes down
    /// levels, and with no style of its own on the stack, it takes less room to do that.
    fn solve_styled(
        &mut self,
        current: &mut LayoutBox,
        node: &Element,
        style: &Style,
        key: Key,
        mode: Mode,
    ) -> Size {
        // What's here is kept small, as each level of a tree has it on the stack: see `solve_shaped` and `shrink_to_fit`.
        if style.aspect.is_some() || style.fit_width {
            return self.solve_shaped(current, node, style, key, mode);
        }
        if let Some(size) = self.known_answer(current, key, mode) {
            return size;
        }
        self.solve_anew(current, node, style, key, mode)
    }

    /// [`solve_styled`](Self::solve_styled) for what has a shape or fits what's in it: a picture's size that isn't said is as its
    /// shape says, from the one that's known, and what fits what's in it is as wide as that, as far as there's room, and no
    /// narrower than what's in it can be, as CSS's shrink-to-fit, `min(max-content, max(min-content, room))`, has it, so an
    /// inline-block that's too wide for where it is goes past it.
    #[cold]
    #[inline(never)]
    fn solve_shaped(&mut self, current: &mut LayoutBox, node: &Element, style: &Style, key: Key, mode: Mode) -> Size {
        let key = match style.aspect {
            Some(aspect) if key.width.exact && matches!(style.height, Length::Auto) => Key {
                width: key.width,
                height: Axis::exact((key.width.value / aspect).clamp_to(style.min_height, style.max_height)),
            },
            Some(aspect) if key.height.exact && matches!(style.width, Length::Auto) => Key {
                width: Axis::exact((key.height.value * aspect).clamp_to(style.min_width, style.max_width)),
                height: key.height,
            },
            _ => key,
        };
        let size = self.solve_keyed(current, node, style, key, mode);
        if style.fit_width
            && !key.width.exact
            && key.width.value >= 0.0
            && key.width.value.is_finite()
            && size.width >= key.width.value - 0.01
        {
            let least = self
                .solve_keyed(current, node, style, Key { width: Axis::MIN_CONTENT, height: key.height }, Mode::Measure)
                .width
                .clamp_to(style.min_width, style.max_width);
            if least > key.width.value + 0.01 {
                return self.solve_keyed(current, node, style, Key { width: Axis::exact(least), height: key.height }, mode);
            }
        }
        size
    }

    /// [`solve_styled`](Self::solve_styled) for a key that's as it's to be.
    fn solve_keyed(&mut self, current: &mut LayoutBox, node: &Element, style: &Style, key: Key, mode: Mode) -> Size {
        if let Some(size) = self.known_answer(current, key, mode) {
            return size;
        }
        self.solve_anew(current, node, style, key, mode)
    }

    /// The answer to this question, if there's no need to work it out.
    #[inline(always)]
    fn known_answer(&mut self, current: &mut LayoutBox, key: Key, mode: Mode) -> Option<Size> {
        if current.has(SELF_DIRTY | CHILD_DIRTY) {
            current.clear_results();
            current.flags &= !(SELF_DIRTY | CHILD_DIRTY | MANY_CHILDREN_DIRTY);
        }
        // Nothing inside can change a size that's already decided, and the answer
        // doesn't depend on the content, so this doesn't count as a question.
        if mode == Mode::Measure && key.width.exact && key.height.exact {
            return Some(Size::new(key.width.value, key.height.value));
        }
        if current.has(HAS_LAYOUT) && current.layout_key() == key {
            return Some(Size::new(current.width, current.height));
        }
        if mode == Mode::Measure && current.has(HAS_MEASURE) && current.measure_key() == key {
            return Some(Size::new(current.measured[0], current.measured[1]));
        }
        if mode == Mode::Measure && !current.answers.is_empty()
            && let Some(size) = current.kept_answer(key)
        {
            return Some(size);
        }
        None
    }

    /// Works out the answer to a question that hasn't been answered.
    fn solve_anew(
        &mut self,
        current: &mut LayoutBox,
        node: &Element,
        style: &Style,
        key: Key,
        mode: Mode,
    ) -> Size {
        if style.max_width_fit && key.width.value.is_finite() && key.width.value >= 0.0 {
            return self.solve_max_content(current, node, style, key, mode);
        }
        // Each level of a tree is several calls deep here, so a tree deeper than the stack has room for goes on with another.
        rsui_core::stack::deep(|| {
            if style.collapse_margins && on_a_line(node) {
                return self.solve_on_a_line(current, node, style, key, mode);
            }
            self.solve_plainly(current, node, style, key, mode)
        })
    }

    /// An intrinsic maximum also caps a block or a column's stretched child, not only a row's growing flex item.
    #[cold]
    #[inline(never)]
    fn solve_max_content(&mut self, current: &mut LayoutBox, node: &Element, style: &Style, key: Key, mode: Mode) -> Size {
        let mut capped = *style;
        capped.max_width_fit = false;
        let widest = self.solve_styled(
            current,
            node,
            &capped,
            Key { width: Axis::UNBOUNDED, height: key.height },
            Mode::Measure,
        ).width;
        if !widest.is_finite() {
            return self.solve_anew(current, node, &capped, key, mode);
        }
        capped.max_width = capped.max_width.min(widest.max(capped.min_width));
        let constrained = Key {
            width: Axis { value: key.width.value.min(capped.max_width), exact: key.width.exact },
            height: key.height,
        };
        let size = self.solve_anew(current, node, &capped, constrained, mode);
        // The parent asks with its available width again; remember that question as well as the capped layout.
        match mode {
            Mode::Measure => current.store_measure(key, size),
            Mode::Layout => current.store_layout(key, size),
        }
        size
    }

    /// A block of pictures on a line of text, which leaves room below them for what goes under a line. It's apart from
    /// [`Self::solve_anew`], as the style it makes would take room on the stack at every level of a tree otherwise.
    #[cold]
    #[inline(never)]
    fn solve_on_a_line(
        &mut self,
        current: &mut LayoutBox,
        node: &Element,
        style: &Style,
        key: Key,
        mode: Mode,
    ) -> Size {
        let lined = Style {
            padding_bottom: style.padding_bottom + self.measure.line_below(node),
            ..*style
        };
        self.solve_plainly(current, node, &lined, key, mode)
    }

    fn solve_plainly(
        &mut self,
        current: &mut LayoutBox,
        node: &Element,
        style: &Style,
        key: Key,
        mode: Mode,
    ) -> Size {
        if node.children.iter().any(|child| is_contents(child)) {
            return self.solve_spliced(current, node, style, key, mode);
        }
        if (!style.collapse_margins || style.grid) && style.table == TablePart::None && out_of_order(node) {
            return self.solve_ordered(current, node, style, key, mode);
        }
        self.solved += 1;
        // The answer is kept as the answer to what it was asked, `question`, though it's worked out in the room it has.
        let question = key;
        let key = within_own_max(key, style);
        // What a margin or padding that's a percentage is a share of, for what's in this: how wide it is inside its padding.
        let base = if key.width.value >= 0.0 && key.width.value.is_finite() {
            (key.width.value - style.padding_left - style.padding_right).max(0.0)
        } else {
            f32::NAN
        };
        let outer_base = crate::style::PERCENT_BASE.with(|cell| cell.replace(base));
        // Boxes for a new part of the tree are made the first time it's laid out.
        let wanted = node.children.len();
        if current.children.len() != wanted {
            set_box_count(current, wanted);
        }

        let scrolls = style.scroll_x.is_some() || style.scroll_y.is_some();
        if scrolls || style.row_height.is_some() {
            current.flags |= SCROLLS;
        } else {
            current.flags &= !SCROLLS;
        }
        current.flags &= !PARAGRAPH;
        let size = if let Some(row_height) = style.row_height {
            self.solve_list(current, node, style, key, mode, row_height)
        } else if scrolls && !node.children.is_empty() {
            self.solve_scroll(current, node, style, key, mode)
        } else if node.children.is_empty() {
            self.solve_leaf(node, style, key)
        } else if is_paragraph(node) {
            current.flags |= PARAGRAPH;
            current.flags &= !CONTAINS_OVERLAY;
            *self.paragraphs = true;
            let size = self.solve_leaf(node, style, key);
            if mode == Mode::Layout {
                self.place_parts(current, node, style, size);
            }
            size
        } else if matches!(
            style.table,
            TablePart::Table | TablePart::Group | TablePart::Row
        ) {
            self.solve_table(current, node, style, key, mode)
        } else if style.grid {
            self.solve_grid(current, node, style, key, mode)
        } else if style.wrap && !shows_own_text(node) {
            self.solve_wrapped(current, node, style, key, mode)
        } else if shows_own_text(node) {
            // It's as big as its text, and all that's in it is on top of everything.
            let size = self.solve_leaf(node, style, key);
            for (child_box, child) in current.children.iter_mut().zip(&node.children) {
                mark_overlay(child_box, child);
            }
            if mode == Mode::Layout {
                self.solve_overlays(current, node);
                current.update_contains_overlay();
            }
            size
        } else {
            self.solve_flex(current, node, style, key, mode)
        };
        // What's in a paragraph is where its text puts it already, as only text that's laid out as a box of its own needs this.
        if mode == Mode::Layout && !node.children.is_empty() && !current.has(PARAGRAPH) {
            self.fit_inline_areas(current, node);
        }
        if mode == Mode::Layout && !node.children.is_empty() && !current.has(PARAGRAPH) {
            self.keep_inside_where_it_was(current, node);
        }
        if mode == Mode::Layout && crate::style::ANY_Z.load(std::sync::atomic::Ordering::Relaxed) {
            note_stacking(current, node);
        }
        match mode {
            Mode::Measure => {
                if current.has(HAS_MEASURE) {
                    self.remember_displaced(current);
                }
                current.store_measure(question, size);
            }
            Mode::Layout => {
                current.store_layout(question, size);
                current.update_contains_absolute();
                current.update_clipped_paint(node);
            }
        }
        crate::style::PERCENT_BASE.with(|cell| cell.set(outer_base));
        size
    }

    /// Text in this whose box is only as tall as its fonts make it, as a CSS inline box is (`inline_area`), placed where its lines
    /// are: its box is moved to where its fonts' text starts, and is as much shorter, and what's drawn in it moves up with that.
    /// Room is taken as its lines take it, as only its box changes.
    #[inline(never)]
    fn fit_inline_areas(&mut self, current: &mut LayoutBox, node: &Element) {
        for (child_box, child) in current.children.iter_mut().zip(&node.children) {
            if is_inline_area(child)
                && (!child.children.is_empty() || child.role == rsui_core::Role::Text)
                && let Some((top, cut)) = self.measure.inline_area(child)
            {
                child_box.y += top;
                child_box.height = (child_box.height - cut).max(0.0);
            }
        }
    }

    /// What's in something whose box is only as tall as its fonts make it, as a link around its text is, where it was before
    /// that box moved down to where its fonts' text starts: see [it_inline_areas](Self::fit_inline_areas). What's in a paragraph
    /// is where its text puts it, which does the same.
    #[inline(never)]
    fn keep_inside_where_it_was(&mut self, current: &mut LayoutBox, node: &Element) {
        if !is_inline_area(node) {
            return;
        }
        if let Some((top, _)) = self.measure.inline_area(node) {
            for child_box in &mut current.children {
                child_box.y -= top;
            }
        }
    }

    /// Lays this out with what's in something that has no box of its own, as CSS's `display: contents` says, in its place, as if it
    /// were in this itself: a fragment's children are a flex box's items, or a grid's, wherever the fragment came from. Their boxes are
    /// taken out of the boxes they're in for that, and put back after, where they're from the top left of this, which is where each
    /// box with nothing of its own is, as big as this is.
    #[cold]
    #[inline(never)]
    fn solve_spliced(
        &mut self,
        current: &mut LayoutBox,
        node: &Element,
        style: &Style,
        key: Key,
        mode: Mode,
    ) -> Size {
        if current.children.len() != node.children.len() {
            set_box_count(current, node.children.len());
        }
        let mut flat = Element {
            key: node.key.clone(),
            role: node.role,
            name: node.name.clone(),
            props: node.props.clone(),
            actions: node.actions,
            children: thin_vec::ThinVec::new(),
        };
        let mut boxes = thin_vec::ThinVec::new();
        take_spliced(
            &mut current.children,
            &node.children,
            &mut flat.children,
            &mut boxes,
        );
        let own = std::mem::replace(&mut current.children, boxes);
        let size = self.solve_plainly(current, &flat, style, key, mode);
        let boxes = std::mem::replace(&mut current.children, own);
        let mut boxes = boxes.into_iter();
        put_back_spliced(
            &mut current.children,
            &node.children,
            &mut boxes,
            (mode == Mode::Layout).then_some(size),
        );
        current.update_contains_overlay();
        current.update_contains_absolute();
        if mode == Mode::Layout {
            current.update_clipped_paint(node);
        }
        size
    }

    /// Lays out a flex box's or a grid's items in the order CSS's `order` puts them in (the lowest first, and those that say the same
    /// in the order they're written), as they're taken one after another, while the tree, and so what's found and tabbed through,
    /// stays in the order they're written. Their boxes are taken out in that order and put back where they're from afterwards, as
    /// [`Self::solve_spliced`] does for what has no box of its own.
    #[cold]
    #[inline(never)]
    fn solve_ordered(
        &mut self,
        current: &mut LayoutBox,
        node: &Element,
        style: &Style,
        key: Key,
        mode: Mode,
    ) -> Size {
        let count = node.children.len();
        if current.children.len() != count {
            set_box_count(current, count);
        }
        let mut sorted: Vec<usize> = (0..count).collect();
        sorted.sort_by(|&a, &b| order_of(&node.children[a]).total_cmp(&order_of(&node.children[b])));
        let flat = Element {
            key: node.key.clone(),
            role: node.role,
            name: node.name.clone(),
            props: node.props.clone(),
            actions: node.actions,
            children: sorted.iter().map(|&at| node.children[at].clone()).collect(),
        };
        let mut taken: Vec<Option<LayoutBox>> = std::mem::take(&mut current.children).into_iter().map(Some).collect();
        current.children = sorted.iter().map(|&at| taken[at].take().unwrap_or_default()).collect();
        let size = self.solve_plainly(current, &flat, style, key, mode);
        let mut back: Vec<Option<LayoutBox>> = (0..count).map(|_| None).collect();
        for (laid, &at) in std::mem::take(&mut current.children).into_iter().zip(&sorted) {
            back[at] = Some(laid);
        }
        current.children = back.into_iter().map(Option::unwrap_or_default).collect();
        current.update_contains_overlay();
        current.update_contains_absolute();
        if mode == Mode::Layout {
            current.update_clipped_paint(node);
        }
        size
    }

    /// Puts the boxes of a paragraph's parts around where its text says they are, for anything that wants
    /// to know where one is. A part on two lines has a box around both, so hitting one is left to the text.
    #[cold]
    #[inline(never)]
    fn place_parts(&mut self, current: &mut LayoutBox, node: &Element, style: &Style, size: Size) {
        let (pad_x, _) = padding_totals(style);
        let mut places = Vec::new();
        self.measure
            .place_pieces(node, (size.width - pad_x).max(0.0), &mut places);
        let origin = (style.padding_left, style.padding_top);
        let mut next = 0;
        for (child_box, child) in current.children.iter_mut().zip(&node.children) {
            place_part(child_box, child, origin, &places, &mut next);
        }
    }

    /// Where the first line of text in each of a line of a wrapping row is, from its top, into `baselines`, for each that's lined up by
    /// its text, and -1 for the others. Each is laid out to find it. Gives how far the line goes above where they line up, margins
    /// included.
    #[cold]
    #[inline(never)]
    fn line_baselines(
        &mut self,
        current: &mut LayoutBox,
        node: &Element,
        items: &[Item],
        margins: &[(f32, f32, f32, f32)],
        line: std::ops::Range<usize>,
        baselines: &mut Vec<f32>,
    ) -> f32 {
        let style = Style::read(&node.props).0;
        baselines.resize(items.len(), -1.0);
        let mut ascent = 0f32;
        // On a line of a page, something with no text in it sits on the line by the bottom of its margin, as CSS's inline-block
        // does; in a flex box, by its border's.
        let page_line = matches!(
            node.props.get_symbol(NAMES.line_strut),
            Some(rsui_core::PropValue::Bool(true))
        );
        for at in line {
            let item = &items[at];
            if item.style.align_self.unwrap_or(style.align) != Align::Baseline {
                continue;
            }
            let child_box = &mut current.children[item.index];
            let child = &node.children[item.index];
            // As tall as it says it is, as it will be, so that what's in the middle of it is where it will be: an inline-flex
            // button 32 high has its text in the middle of that, and sits on the line by where that text is.
            let (_, _, least, most) = limits(&item.style, true);
            let cross = match resolve_length(item.style.height, None) {
                Some(size) => Axis::exact(size.clamp_to(least, most)),
                // On a page, as tall as what's in it, past the line if that's taller, as the last pass has it.
                None if style.min_auto => Axis::at_most(most),
                None => Axis::at_most(item.cross),
            };
            self.solve_styled(
                child_box,
                child,
                &item.style,
                make_key(true, Axis::exact(item.main), cross),
                Mode::Layout,
            );
            // What scrolls, as a `<textarea>` does or what says `overflow: hidden`, sits on a page's line by the bottom of its
            // margin too, whatever text is in it, as CSS has it.
            let by_bottom = page_line
                && (item.style.scroll_x.is_some()
                    || item.style.scroll_y.is_some()
                    || matches!(
                        child.props.get("rows"),
                        Some(rsui_core::PropValue::Int(_) | rsui_core::PropValue::Float(_))
                    ));
            let found = if by_bottom {
                None
            } else {
                self.baseline_in(child_box, child)
            }
            .unwrap_or(item.cross + if page_line { margins[at].3 } else { 0.0 });
            baselines[at] = found;
            ascent = ascent.max(found + margins[at].2);
        }
        ascent
    }

    /// How far below the top of a box that's been laid out its first line of text is, if it has any.
    fn baseline_in(&mut self, current: &LayoutBox, node: &Element) -> Option<f32> {
        if node.children.is_empty() || crate::is_paragraph(node) {
            let found = self.measure.first_baseline(node)?;
            return Some(Style::read(&node.props).0.padding_top + found);
        }
        let mut placed = current
            .children
            .iter()
            .zip(&node.children)
            .filter(|(child_box, _)| !child_box.has(IS_OVERLAY) && !child_box.has(IS_ABSOLUTE));
        // A flex box that doesn't line what's in it up by its text sits on a line where the first thing in it does, which is by
        // its bottom edge if it has no text, as CSS has it, rather than by the first text in it, as a block does.
        let names = &*NAMES;
        let flex = node.props.get_symbol(names.direction).is_some()
            && node.props.get_symbol(names.line_strut).is_none()
            && Style::read(&node.props).0.align != Align::Baseline;
        if flex {
            // What's no box, as a component's fragment is, isn't the first thing in it: what's in that is, if anything is, as
            // Mantine's segmented control's indicator, placed by itself, is in one before its first control.
            for (child_box, child) in placed {
                if is_contents(child) {
                    if let Some(found) = self.baseline_in(child_box, child) {
                        return Some(child_box.y + found);
                    }
                    continue;
                }
                let found = self
                    .baseline_in(child_box, child)
                    .unwrap_or(child_box.height);
                return Some(child_box.y + found);
            }
            return None;
        }
        placed
            .find_map(|(child_box, child)| Some(child_box.y + self.baseline_in(child_box, child)?))
    }

    /// The flex items in a row that are no wider than what's in them (`max-width: max-content`, Tailwind's `max-w-max`) grow no
    /// further than that, as a NavigationMenu's `flex-1 max-w-max` nav doesn't. It's done with `keep_to_content`, which a growing
    /// item's `min-width: auto` brings here, rather than in `solve_flex`, whose frame each level of a tree has on the stack.
    #[cold]
    #[inline(never)]
    fn cap_at_max_content(&mut self, current: &mut LayoutBox, node: &Element, items: &mut [Item]) {
        for item in items.iter_mut().filter(|item| item.style.max_width_fit) {
            let style = item.style;
            let widest = self
                .solve_styled(
                    &mut current.children[item.index],
                    &node.children[item.index],
                    &style,
                    make_key(true, Axis::UNBOUNDED, item.cross_axis),
                    Mode::Measure,
                )
                .width;
            if widest.is_finite() {
                item.max_main = item.max_main.min(widest.max(item.min_main));
                item.main = item.main.clamp_to(item.min_main, item.max_main);
            }
        }
    }

    /// What CSS lays out doesn't get smaller than what's in it, unless it said it was smaller than that: the least each item that
    /// shrinks can be, along a flex box.
    fn keep_to_content(
        &mut self,
        current: &mut LayoutBox,
        node: &Element,
        items: &mut [Item],
        row: bool,
    ) {
        if row {
            self.cap_at_max_content(current, node, items);
        }
        for item in items.iter_mut().filter(|item| {
            // What clips what's in it, as what scrolls does, `overflow: hidden` does (a scroll container too, to CSS), and an
            // `<svg>` does by itself, can be as small as anything, as CSS's `min-width: auto` comes to 0 for it: shadcn's
            // `flex-1 overflow-hidden` CardContent is the card's height less its header and footer, not its messages'.
            let child = &node.children[item.index];
            let clips = item.style.scroll_x.is_some()
                || item.style.scroll_y.is_some()
                || (!child.props.is_empty() && crate::tree::clips(child))
                || (child.role == rsui_core::Role::Image && child.props.get("src").is_none());
            item.style.keeps_to_content(row) && !clips
        }) {
            // Across, that's as narrow as it can be, which for text is its longest word. Down, it's as tall as it is.
            let shaped = self.picture_along(&node.children[item.index], item.cross_axis, row);
            let content = match item.wanted {
                _ if let Some(along) = shaped => along,
                Some((main, _)) if !row => main,
                _ => {
                    let along = if row {
                        Axis::MIN_CONTENT
                    } else {
                        Axis::UNBOUNDED
                    };
                    let answer = self.solve_styled(
                        &mut current.children[item.index],
                        &node.children[item.index],
                        &item.style,
                        make_key(row, along, item.cross_axis),
                        Mode::Measure,
                    );
                    if row { answer.width } else { answer.height }
                }
            };
            // No more than it says it is, if it says, as CSS's specified size suggestion has it. What only starts from a
            // basis, as `flex: 1` does, is no smaller than what's in it, however little it starts from.
            let said = !matches!(lengths(&item.style, row).0, Length::Auto);
            let cap = if item.style.has_basis() && !said { f32::INFINITY } else { item.base };
            // Nor more than its most, which its base can be past (see `base_past_max`).
            item.min_main = item.min_main.max(content.min(cap).min(item.max_main));
            item.main = item.main.max(item.min_main);
        }
    }

    /// What a flex item that's as big as what's in it starts from along a flex box (CSS's flex base size), given `answer`, which
    /// is how big it said it is: that's how big what's in it is, which can be past the most it can be, as only how big it can
    /// end up is held to that. Shrinking takes from each as much more as it starts from, so a box of chips with `max-width:
    /// 300px` that's 398 wide by what's in it gives up as much as 398 does beside text, not as 300 does. Only what's at its most
    /// is asked again, without it. What has a shape, as a picture does, keeps to what it said, as its size is worked out from
    /// that shape (see `solve_shaped`).
    fn base_past_max(
        &mut self,
        child_box: &mut LayoutBox,
        child: &Element,
        item: &Item,
        question: Key,
        answer: Size,
        row: bool,
    ) -> f32 {
        let along = if row { answer.width } else { answer.height };
        if along < item.max_main - 0.01 || !item.max_main.is_finite() || item.style.aspect.is_some() || item.style.fit_width {
            return along;
        }
        self.solve_past_max(child_box, child, &item.style, question, answer, row)
    }

    /// How big what's in something is along `row` or down, when it's asked `question`, without the most it can be that way:
    /// see [`base_past_max`](Self::base_past_max). What it's kept as the answer to that question stays `answer`, which is what
    /// it is with its most.
    #[cold]
    #[inline(never)]
    fn solve_past_max(
        &mut self,
        child_box: &mut LayoutBox,
        child: &Element,
        style: &Style,
        question: Key,
        answer: Size,
        row: bool,
    ) -> f32 {
        let mut unlimited = *style;
        if row {
            unlimited.max_width = f32::INFINITY;
        } else {
            unlimited.max_height = f32::INFINITY;
        }
        let size = self.solve_anew(child_box, child, &unlimited, question, Mode::Measure);
        child_box.store_measure(question, answer);
        // That isn't kept as an answer to anything, so when what's in it changes, what it's in has to ask again.
        child_box.flags |= FORGOT_A_QUESTION;
        if row { size.width } else { size.height }
    }

    /// How long a picture from a file is along a flex box, if it's as big as `cross` says the other way: as long as that makes it,
    /// keeping its shape, as CSS has it. A shape drawn in place shrinks with the rest, as an `<svg>` does on a page.
    #[cold]
    #[inline(never)]
    fn picture_along(&mut self, child: &Element, cross: Axis, row: bool) -> Option<f32> {
        if child.role != rsui_core::Role::Image || !cross.exact || child.props.get("src").is_none()
        {
            return None;
        }
        let natural = self.measure.measure(child, f32::INFINITY);
        if natural.width <= 0.0 || natural.height <= 0.0 {
            return None;
        }
        Some(if row {
            cross.value * natural.width / natural.height
        } else {
            cross.value * natural.height / natural.width
        })
    }

    /// What's in a box of its own, as text in a `<p>` is, given room that's `room` wide that it needn't fill: as CSS's
    /// `fit-content`, `min(max-content, room)`, has it, it's as wide as it is on one line where that fits, and as wide as the room
    /// where it wraps, not as its widest line, as a centred `<p>` with `max-width: 300px` that wraps is 300 wide. Asking about it on
    /// one line first costs nothing more where it fits, which is most of the time.
    #[inline(never)]
    fn fit_content(&mut self, node: &Element, room: f32) -> Size {
        let one_line = self.measure.measure(node, f32::INFINITY);
        if one_line.width <= room + 0.01 {
            return one_line;
        }
        let wrapped = self.measure.measure(node, room);
        Size::new(wrapped.width.max(room), wrapped.height)
    }

    /// Whether what a leaf shows is as wide as [`fit_content`](Self::fit_content) says, given room that isn't its own size: text
    /// is, as a box of its own, but not text that's only part of a line (`inline_area`, as a `<span>` in a block is), which is as
    /// wide as its lines are, as an inline box is, nor a picture or a field, which are as big as they are.
    fn fits_content(node: &Element, key: Key) -> bool {
        !key.width.exact
            && key.width.value >= 0.0
            && key.width.value.is_finite()
            && !matches!(node.role, rsui_core::Role::Image | rsui_core::Role::TextInput)
            && !is_inline_area(node)
    }

    #[inline(never)]
    fn solve_leaf(&mut self, node: &Element, style: &Style, key: Key) -> Size {
        let (pad_x, pad_y) = padding_totals(style);
        let content = if key.width.exact && key.height.exact {
            Size::default()
        } else if node.role == rsui_core::Role::Image
            && key.width.exact != key.height.exact
            && (if key.width.exact { style.width } else { style.height }) != crate::style::Length::Auto
            // Asked how narrow it can be, a shape drawn in place is as narrow as it is by itself, and shrinks from there.
            && key.width.value >= 0.0
        {
            // A picture given a size one way is the size its shape makes it the other way, as CSS has it.
            let natural = self.measure.measure(node, f32::INFINITY);
            if natural.width > 0.0 && natural.height > 0.0 {
                if key.width.exact {
                    let width = (key.width.value - pad_x).max(0.0);
                    Size::new(width, width * natural.height / natural.width)
                } else {
                    let height = (key.height.value - pad_y).max(0.0);
                    Size::new(height * natural.width / natural.height, height)
                }
            } else {
                let max_width = (key.width.value - pad_x).max(0.0);
                self.measure.measure(node, max_width)
            }
        } else {
            let max_width = (key.width.value - pad_x).max(0.0);
            if Self::fits_content(node, key) {
                self.fit_content(node, max_width)
            } else {
                self.measure.measure(node, max_width)
            }
        };
        // Words that don't fit in the room they're given, as a word that can't be broken or text that doesn't wrap, are as wide as
        // they are, past it: CSS's `fit-content` is never narrower than what's in it can be (a toggle's "Medium" label under its
        // "Aa" in a 38px column is 45px wide, centred over it).
        let overflowing = !key.width.exact
            && node.role == rsui_core::Role::Text
            && key.width.value >= 0.0
            && content.width + pad_x > key.width.value;
        Size::new(
            if overflowing {
                (content.width + pad_x).clamp_to(style.min_width, style.max_width)
            } else {
                resolve_auto(
                    key.width,
                    content.width + pad_x,
                    style.min_width,
                    style.max_width,
                )
            },
            resolve_auto(
                key.height,
                content.height + pad_y,
                style.min_height,
                style.max_height,
            ),
        )
    }

    /// A virtual list: rows of one height, placed by the list's scroll position. A list with an `item_width` is a grid, whose
    /// children are items rather than rows: each row has `across` of them side by side, `item_width` wide and as tall as a row
    /// less the `gap`, which is between them both ways, and how many fit across is worked out here, as how many rows fit is.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn solve_list(
        &mut self,
        current: &mut LayoutBox,
        node: &Element,
        style: &Style,
        key: Key,
        mode: Mode,
        row_height: f32,
    ) -> Size {
        let (_, list) = Style::read(&node.props);
        let (pad_x, pad_y) = padding_totals(style);
        let count = match node.props.get("count") {
            Some(rsui_core::PropValue::Int(count)) => *count as f32,
            _ => node.children.len() as f32,
        };
        let height = resolve_auto(
            key.height,
            count * row_height + pad_y,
            style.min_height,
            style.max_height,
        );
        let width = if key.width.exact {
            key.width.value
        } else if let Some(item_width) = list.item_width {
            // A grid takes all the room it's given, to fit as many across as it can, and is one item wide where there's no limit,
            // or when it's asked how narrow it can be.
            let room = key.width.value;
            let least = item_width + pad_x;
            resolve_auto(key.width, if room.is_finite() && room >= 0.0 { room.max(least) } else { least }, style.min_width, style.max_width)
        } else {
            // A list is as wide as its widest row, among those that exist right now.
            let mut widest = 0f32;
            for (child_box, child) in current.children.iter_mut().zip(&node.children) {
                let row_key = Key {
                    width: Axis::at_most((key.width.value - pad_x).max(0.0)),
                    height: Axis::exact(row_height),
                };
                widest = widest.max(self.solve(child_box, child, row_key, Mode::Measure).width);
            }
            resolve_auto(key.width, widest + pad_x, style.min_width, style.max_width)
        };
        if mode == Mode::Layout {
            let row_width = (width - pad_x).max(0.0);
            // How many items fit across a grid's row, each with the gap after it but the last.
            let fit_across = list
                .item_width
                .map(|item_width| ((row_width + style.gap) / (item_width + style.gap)).floor().max(1.0));
            // Rows beyond the window can't be seen, so however tall the list wants to be, it
            // isn't worth building them. A list with no limit on its height would build them all.
            if let Some(told) = list.view_rows {
                if row_height > 0.0 {
                    let fit = ((height - pad_y).min(self.window_height) / row_height).max(0.0);
                    let across_wrong = fit_across.is_some_and(|across| across != list.across);
                    if (fit - told).abs() > 0.001 || across_wrong {
                        self.list_sizes
                            .push((std::ptr::from_ref(node) as usize, fit, fit_across.unwrap_or(0.0)));
                    }
                }
            }
            let top = style.padding_top + (list.first_built - list.first_visible) * row_height;
            if let Some(item_width) = list.item_width {
                // Its items, a row of them at a time, as many as it was built with across, which is what fits once it's been told.
                let across = (list.across.max(1.0)) as usize;
                let item_key = Key {
                    width: Axis::exact(item_width),
                    height: Axis::exact((row_height - style.gap).max(0.0)),
                };
                for (index, (child_box, child)) in
                    current.children.iter_mut().zip(&node.children).enumerate()
                {
                    self.solve(child_box, child, item_key, Mode::Layout);
                    child_box.x = style.padding_left + (index % across) as f32 * (item_width + style.gap);
                    child_box.y = top + (index / across) as f32 * row_height;
                }
            } else {
                let row_key = Key {
                    width: Axis::exact(row_width),
                    height: Axis::exact(row_height),
                };
                for (index, (child_box, child)) in
                    current.children.iter_mut().zip(&node.children).enumerate()
                {
                    self.solve(child_box, child, row_key, Mode::Layout);
                    child_box.x = style.padding_left;
                    child_box.y = top + index as f32 * row_height;
                }
            }
        }
        Size::new(width, height)
    }

    /// An area whose content can be bigger than it is. Its children are laid out as they would be
    /// if there were no limit that way, and then moved by how far it's scrolled, which is all that
    /// changes when it scrolls: nothing inside it is worked out again.
    #[inline(never)]
    fn solve_scroll(
        &mut self,
        current: &mut LayoutBox,
        node: &Element,
        style: &Style,
        key: Key,
        mode: Mode,
    ) -> Size {
        // As narrow as it can be, it's as narrow as what's in it can be, as CSS has it, which is how much a flex box around it can
        // shrink. Otherwise what's in it is as big as it wants to be, as it scrolls.
        let free = |scroll: Option<f32>, axis: Axis| {
            if scroll.is_some() && axis != Axis::MIN_CONTENT {
                Axis::UNBOUNDED
            } else {
                axis
            }
        };
        // With `scrollbar_room`, a scrollbar takes that much room from what's in it, as a browser's does, once there's more than shows.
        let room = match node.props.get_symbol(NAMES.scrollbar_room) {
            Some(rsui_core::PropValue::Float(room)) => *room as f32,
            Some(rsui_core::PropValue::Int(room)) => *room as f32,
            _ => 0.0,
        };
        let own = if key.width.exact && key.height.exact {
            Size::new(key.width.value, key.height.value)
        } else {
            let mut inner = *style;
            if style.scroll_x.is_some() {
                (inner.min_width, inner.max_width) = (0.0, f32::INFINITY);
            }
            if style.scroll_y.is_some() {
                (inner.min_height, inner.max_height) = (0.0, f32::INFINITY);
            }
            let definite = key.height.exact && current.has(crate::tree::DEFINITE_HEIGHT);
            let view = if definite { key.height.value } else { f32::NAN };
            let content = self.with_view_height(current, view, |pass, current| {
                pass.solve_flex(
                    current,
                    node,
                    &inner,
                    Key {
                        width: free(style.scroll_x, key.width),
                        height: free(style.scroll_y, key.height),
                    },
                    Mode::Measure,
                )
            });
            let mut own = Size::new(
                resolve_auto(key.width, content.width, style.min_width, style.max_width),
                resolve_auto(
                    key.height,
                    content.height,
                    style.min_height,
                    style.max_height,
                ),
            );
            // As tall as what's in it is, and the bar along the bottom too, when it's wider than it is, as CSS has it.
            if room > 0.0
                && !key.height.exact
                && style.scroll_x.is_some()
                && content.width > own.width + 0.5
            {
                own.height = (own.height + room).min(style.max_height);
            }
            own
        };
        if mode == Mode::Measure {
            return own;
        }
        // What the scrollbars took last time, which is mostly what they take now: see `BAR_DOWN`.
        let mut taken = if room > 0.0 {
            (
                if current.has(crate::tree::BAR_DOWN) { room } else { 0.0 },
                if current.has(crate::tree::BAR_ACROSS) { room } else { 0.0 },
            )
        } else {
            (0.0f32, 0.0f32)
        };
        let mut kept = taken != (0.0, 0.0);
        let across_fills = crate::tree::fills_across(node);
        let content = loop {
            // Content shorter than the area still fills it, so that what's flexible can use the room.
            let (view_width, view_height) = (
                (own.width - taken.0).max(0.0),
                (own.height - taken.1).max(0.0),
            );
            let mut inner = *style;
            let mut width = Axis::exact(view_width);
            let mut height = Axis::exact(view_height);
            if style.scroll_x.is_some() && !across_fills {
                (inner.min_width, inner.max_width) = (view_width, f32::INFINITY);
                width = Axis::UNBOUNDED;
            }
            if style.scroll_y.is_some() {
                (inner.min_height, inner.max_height) = (view_height, f32::INFINITY);
                height = Axis::UNBOUNDED;
            }
            // What shows is of a known height when it was asked to be that tall, as CSS has a scroller with a definite height:
            // one as tall as what's in it isn't.
            let definite = key.height.exact && current.has(crate::tree::DEFINITE_HEIGHT);
            let view = if definite { view_height } else { f32::NAN };
            let mut content = self.with_view_height(current, view, |pass, current| {
                pass.solve_flex(current, node, &inner, Key { width, height }, Mode::Layout)
            });
            if across_fills {
                content.width = content.width.max(crate::tree::furthest_right(current, node));
                // Where text goes from right to left, what's wider than it reaches past its left instead, which is what there is to
                // scroll to: it's all moved over to start at 0, as what's in something that scrolls is, and scrolled back below.
                if style.scroll_x.is_some() && right_to_left(node) {
                    let left = crate::tree::furthest_left(current, node);
                    if left < -0.5 {
                        for child in &mut current.children {
                            child.x -= left;
                        }
                        content.width -= left;
                    }
                }
            }

            if room <= 0.0 {
                break content;
            }
            // A bar down the side when it's too tall, which takes room across, and one along the bottom when it's too wide.
            let needs = (
                style.scroll_y.is_some() && content.height > view_height + 0.5,
                style.scroll_x.is_some() && content.width > view_width + 0.5,
            );
            // A bar kept from last time that what's in it doesn't need any more goes, and they're worked out from none, as the
            // first time, as only ever adding them can't take one away.
            if kept && ((taken.0 > 0.0 && !needs.0) || (taken.1 > 0.0 && !needs.1)) {
                kept = false;
                taken = (0.0, 0.0);
                continue;
            }
            let wanted = (
                if needs.0 { room } else { taken.0 },
                if needs.1 { room } else { taken.1 },
            );
            if wanted == taken {
                break content;
            }
            taken = wanted;
        };
        current.flags &= !(crate::tree::BAR_DOWN | crate::tree::BAR_ACROSS);
        if taken.0 > 0.0 {
            current.flags |= crate::tree::BAR_DOWN;
        }
        if taken.1 > 0.0 {
            current.flags |= crate::tree::BAR_ACROSS;
        }
        let moved = |scroll: Option<f32>, content: f32, own: f32| {
            let furthest = (content - own).max(0.0);
            let scroll = scroll.unwrap_or(0.0);
            // Something that isn't a number stays where it starts.
            if scroll > 0.0 {
                scroll.min(furthest)
            } else {
                0.0
            }
        };
        // What a scrollbar takes isn't there to see, so it can go that much further.
        let mut x = moved(style.scroll_x, content.width, own.width - taken.0);
        // Where text goes from right to left, it starts scrolled to the right, which is its start, as a browser's does: how far
        // it's said to have gone is from there (a page's `scrollLeft` is that, less than 0), and what reaches past its left is
        // what there is to scroll to.
        if style.scroll_x.is_some() && right_to_left(node) {
            x = (content.width - (own.width - taken.0)).max(0.0) - x;
        }

        let y = moved(style.scroll_y, content.height, own.height - taken.1);
        // A scrollbar at the left, as one is where text goes from right to left, has what's in it start after it.
        if taken.0 > 0.0 && matches!(node.props.get_symbol(NAMES.scrollbar_left), Some(rsui_core::PropValue::Bool(true))) {
            x -= taken.0;
        }
        for child in &mut current.children {
            child.x -= x;
            child.y -= y;
        }
        if style.scroll_y.is_some()
            && crate::style::ANY_STICKY.load(std::sync::atomic::Ordering::Relaxed)
        {
            stick(current, node, 0.0, own.height, own.height);
        }
        own
    }

    /// Lays a scroller's content out with what a height that's a share is of in it, as `view` says (not a number when that isn't
    /// known), and what that was before back after. What its content is laid out in took that, which isn't part of what it was
    /// asked, so its answers are worked out again.
    #[inline(never)]
    fn with_view_height(
        &mut self,
        current: &mut LayoutBox,
        view: f32,
        solve: impl FnOnce(&mut Self, &mut LayoutBox) -> Size,
    ) -> Size {
        for child in current.children.iter_mut() {
            if child.has(crate::tree::USES_VIEW_HEIGHT) {
                child.clear_results();
            }
        }
        let was = crate::style::VIEW_HEIGHT.with(|cell| cell.replace(view));
        let size = solve(self, current);
        crate::style::VIEW_HEIGHT.with(|cell| cell.set(was));
        size
    }

    fn solve_flex(
        &mut self,
        current: &mut LayoutBox,
        node: &Element,
        style: &Style,
        key: Key,
        mode: Mode,
    ) -> Size {
        let row = style.direction == Direction::Row;
        let (main_key, cross_key) = if row {
            (key.width, key.height)
        } else {
            (key.height, key.width)
        };
        let (pad_x, pad_y) = padding_totals(style);
        let (pad_main, pad_cross) = if row { (pad_x, pad_y) } else { (pad_y, pad_x) };
        let mut any_overlay = false;
        let mut any_absolute = false;
        let (min_main, max_main, min_cross, max_cross) = if row {
            (
                style.min_width,
                style.max_width,
                style.min_height,
                style.max_height,
            )
        } else {
            (
                style.min_height,
                style.max_height,
                style.min_width,
                style.max_width,
            )
        };
        let main_limit = (main_key.value - pad_main).max(0.0);
        // Asked how narrow it can be, what's in it is asked the same.
        let cross_limit = if cross_key.value < 0.0 {
            -1.0
        } else {
            (cross_key.value - pad_cross).max(0.0)
        };
        let definite_main = main_key.exact.then_some(main_limit);
        let definite_cross = cross_key.exact.then_some(cross_limit);

        // Ask each child how big it wants to be.
        let room = FlexRoom {
            row,
            align: style.align,
            definite_main,
            definite_cross,
            percent_main: view_share(current, node, style, row, pad_main, definite_main),
            cross_limit,
            least_main: main_key.value < 0.0,
            collapse: style.collapse_margins && !row,
        };
        let mut items = Vec::with_capacity(node.children.len());
        let mut used = 0.0;
        let mut any_margin = false;
        let mut all_margins = 0.0;
        // With margins that go together, how much less each item's margin before it is than it says, and the last one's margin after.
        let collapse = style.collapse_margins && !row;
        let mut taken_back: Vec<f32> = Vec::new();
        let mut placed_margins: Vec<Option<(f32, f32)>> = Vec::new();
        let mut last_after = 0.0f32;
        // What goes out of this, as it's been told by what it's in, and the margins of what was last in it.
        let own_escapes = if collapse {
            current.flags & (TOP_ESCAPES | BOTTOM_ESCAPES)
        } else {
            0
        };
        let mut last_margins: Option<(usize, f32, f32)> = None;
        for (index, (child_box, child)) in
            current.children.iter_mut().zip(&node.children).enumerate()
        {
            self.looked_at += 1;
            let start = start_item(&mut items, child_box, child, room);
            if let Some(item) = items
                .last_mut()
                .filter(|_| !matches!(start, ItemStart::Overlay | ItemStart::Absolute))
            {
                item.index = index;
            }
            match start {
                ItemStart::Overlay => {
                    any_overlay = true;
                    continue;
                }
                ItemStart::Absolute => {
                    any_absolute = true;
                    continue;
                }
                _ => {}
            }
            let item_at = items.len() - 1;
            // In a block, the margins of what's first and last in a block inside it go outside that block, as CSS has them. That block
            // is told, so that it leaves them out of itself.
            let inside = if collapse && items[item_at].style.collapse_margins {
                (escaping_margin(child, false), escaping_margin(child, true))
            } else {
                (0.0, 0.0)
            };
            let escapes = if inside.0 != 0.0 { TOP_ESCAPES } else { 0 }
                | if inside.1 != 0.0 { BOTTOM_ESCAPES } else { 0 };
            if child_box.flags & (TOP_ESCAPES | BOTTOM_ESCAPES) != escapes {
                child_box.flags = (child_box.flags & !(TOP_ESCAPES | BOTTOM_ESCAPES)) | escapes;
                child_box.clear_results();
            }
            if let ItemStart::Ask(question) = start {
                let answer = self.solve_styled(
                    child_box,
                    child,
                    &items[item_at].style,
                    question,
                    Mode::Measure,
                );
                let (main, cross) = if row {
                    (answer.width, answer.height)
                } else {
                    (answer.height, answer.width)
                };
                let base = self.base_past_max(child_box, child, &items[item_at], question, answer, row);
                let item = &mut items[item_at];
                item.base = base;
                item.main = main.clamp_to(item.min_main, item.max_main);
                item.wanted = Some((main, cross));
            }
            used += items[item_at].main;
            if items[item_at].style.has_margin || inside != (0.0, 0.0) {
                any_margin = true;
                let (own_before, own_after) = if items[item_at].style.has_margin {
                    let (before, after, _, _) = margins(&Style::read_margins(&child.props), row);
                    (before, after)
                } else {
                    (0.0, 0.0)
                };
                let mut before = together(own_before, inside.0);
                let after = together(own_after, inside.1);
                if item_at == 0 && own_escapes & TOP_ESCAPES != 0 {
                    // It's gone out to what this is in.
                    before = 0.0;
                }
                all_margins += before + after;
                if collapse {
                    let shared = before + last_after - together(before, last_after);
                    taken_back.resize(items.len(), 0.0);
                    taken_back[item_at] = shared;
                    all_margins -= shared;
                    last_after = after;
                    placed_margins.resize(items.len(), None);
                    placed_margins[item_at] = Some((before, after));
                    last_margins = Some((item_at, before, after));
                }
            } else if collapse {
                last_after = 0.0;
                last_margins = Some((item_at, 0.0, 0.0));
            }
        }
        if own_escapes & BOTTOM_ESCAPES != 0
            && let Some((at, before, after)) = last_margins
            && after != 0.0
        {
            // The margin after what's last has gone out to what this is in.
            all_margins -= after;
            placed_margins.resize(items.len(), None);
            placed_margins[at] = Some((before, 0.0));
        }

        let gaps = style.gap * items.len().saturating_sub(1) as f32;
        used += gaps + all_margins;
        // Decide this node's size along its direction, then share out the space.
        let own_main = if main_key.exact {
            main_key.value
        } else {
            (used + pad_main)
                .within(main_key.value)
                .clamp_to(min_main, max_main)
        };
        let inner_main = (own_main - pad_main).max(0.0);
        let room_left = inner_main - gaps - all_margins;
        // Too big for the room, each is no smaller than what's in it. Growing from a basis, as `flex: 1` items do from nothing,
        // each is no smaller than that either, which its share of the room may not be.
        let used_now = items.iter().map(|item| item.main).sum::<f32>();
        if items
            .iter()
            .any(|item| item.style.keeps_to_content(row))
            && (used_now > room_left
                || (used_now < room_left
                    && items.iter().any(|item| {
                        item.style.keeps_to_content(row)
                            && item.style.grow > 0.0
                            && item.style.has_basis()
                            && matches!(lengths(&item.style, row).0, Length::Auto)
                    })))
        {
            self.keep_to_content(current, node, &mut items, row);
        }
        share_space(&mut items, room_left, row);

        // Now that children know their size one way, ask about the other.
        let mut widest_cross = 0f32;
        for (item, (child_box, child)) in items.iter_mut().zip(
            current
                .children
                .iter_mut()
                .zip(&node.children)
                .filter(|(child_box, _)| !child_box.has(IS_OVERLAY | IS_ABSOLUTE)),
        ) {
            item.cross = match item.wanted {
                Some((main, cross)) if main == item.main => cross,
                _ => {
                    let answer = self.solve_styled(
                        child_box,
                        child,
                        &item.style,
                        make_key(row, Axis::exact(item.main), item.cross_axis),
                        Mode::Measure,
                    );
                    if row { answer.height } else { answer.width }
                }
            };
            let cross_margins = if item.style.has_margin {
                let (_, _, before, after) = margins(&Style::read_margins(&child.props), row);
                before + after
            } else {
                0.0
            };
            widest_cross = widest_cross.max(item.cross + cross_margins);
        }
        let own_cross = if cross_key.exact {
            cross_key.value
        } else {
            (widest_cross + pad_cross)
                .within(cross_key.value)
                .clamp_to(min_cross, max_cross)
        };
        let size = if row {
            Size::new(own_main, own_cross)
        } else {
            Size::new(own_cross, own_main)
        };
        if mode == Mode::Measure {
            return size;
        }

        // Place everything.
        let inner_cross = (own_cross - pad_cross).max(0.0);
        let total_main: f32 = items.iter().map(|item| item.main).sum::<f32>() + gaps + all_margins;
        // What's left over, which is less than none when what's in it is too long: then what's said to go at the end or in the
        // middle runs over the start, as CSS has it unless it says `safe`.
        let left_over = inner_main - total_main;
        let free = left_over.max(0.0);
        let count = items.len();
        // Margins said to be `auto` along take what's left over, shared between them, as CSS has them, before anything else does.
        let (autos, auto_share) = if free > 0.0 {
            auto_margins(node, &items, row, free)
        } else {
            (Vec::new(), 0.0)
        };
        let (start_offset, more) = if autos.is_empty() {
            spread_of(style.justify).offsets(left_over, count)
        } else {
            (0.0, 0.0)
        };
        let between = style.gap + more;
        let (pad_main_start, pad_cross_start) = if row {
            (style.padding_left, style.padding_top)
        } else {
            (style.padding_top, style.padding_left)
        };
        let mut position = pad_main_start + start_offset;
        for (item_index, (item, (child_box, child))) in items
            .iter_mut()
            .zip(
                current
                    .children
                    .iter_mut()
                    .zip(&node.children)
                    .filter(|(child_box, _)| !child_box.has(IS_OVERLAY | IS_ABSOLUTE)),
            )
            .enumerate()
        {
            let align = item.style.align_self.unwrap_or(style.align);
            let (auto_before, auto_after) =
                autos.get(item_index).copied().unwrap_or((false, false));
            if auto_before {
                position += auto_share;
            }
            let placed = placed_margins.get(item_index).copied().flatten();
            if any_margin && (item.style.has_margin || placed.is_some()) {
                if let Some(shared) = taken_back.get(item_index) {
                    position -= shared;
                }
                position = self.place_with_margin(
                    child_box,
                    child,
                    item,
                    (row, inner_cross, pad_cross_start, align, style.min_auto),
                    (position, between),
                    placed,
                );
                if auto_after {
                    position += auto_share;
                }
                continue;
            }
            let (_, cross_length) = lengths(&item.style, row);
            let (_, _, child_min_cross, child_max_cross) = limits(&item.style, row);
            let final_cross = match resolve_cross(cross_length, inner_cross, item.cross_axis) {
                Some(size) => Axis::exact(size.clamp_to(child_min_cross, child_max_cross)),
                None if align == Align::Stretch && !fits_across(&item.style, row) => {
                    Axis::exact(inner_cross.clamp_to(child_min_cross, child_max_cross))
                }
                // On a page, what isn't stretched down a row is as tall as what's in it, past the row if that's taller, as
                // text with a line height taller than its inline-flex box is.
                None if row && style.min_auto => Axis::at_most(child_max_cross),
                None => item.cross_axis,
            };
            let answer = self.solve_styled(
                child_box,
                child,
                &item.style,
                make_key(row, Axis::exact(item.main), final_cross),
                Mode::Layout,
            );
            let cross_size = if row { answer.height } else { answer.width };
            let cross_position = pad_cross_start
                + match align {
                    Align::Stretch | Align::Start | Align::Baseline => 0.0,
                    Align::Center => (inner_cross - cross_size) / 2.0,
                    Align::End => inner_cross - cross_size,
                };
            if row {
                child_box.x = position;
                child_box.y = cross_position;
            } else {
                child_box.x = cross_position;
                child_box.y = position;
            }
            if item.style.moved {
                let parent = if row {
                    (inner_main, inner_cross)
                } else {
                    (inner_cross, inner_main)
                };
                apply_translate(child_box, child, parent);
            }
            position += item.main + between;
            if auto_after {
                position += auto_share;
            }
        }
        if row {
            self.order_by_direction(current, node, std::slice::from_ref(&(0..items.len())), &items, style.gap);

        }
        if style.reverse {
            reverse_along(current, style, row, size);
        }
        // Only a column's start across is at its right: a line of text in right-to-left text is still from the top.
        if style.cross_reverse && !row {
            // `true`: across a column is along x.
            reverse_along(current, style, true, size);
        }
        if any_overlay {
            self.solve_overlays(current, node);
        }
        if any_absolute {
            self.solve_absolutes(current, node, size);
        }
        self.place_passed_absolutes(current, node, size);
        current.update_contains_overlay();
        size
    }

    /// Children along lines, each starting where there's no more room on the one before, as `flex-wrap: wrap` has them. Each line shares
    /// out its own room, and is as big across as the biggest thing on it. Lines stretch to fill what they're in, unless `align_content`
    /// says where they go.
    #[cold]
    #[inline(never)]
    fn solve_wrapped(
        &mut self,
        current: &mut LayoutBox,
        node: &Element,
        style: &Style,
        key: Key,
        mode: Mode,
    ) -> Size {
        let names = &*NAMES;
        let row = style.direction == Direction::Row;
        let (main_key, cross_key) = if row {
            (key.width, key.height)
        } else {
            (key.height, key.width)
        };
        let (pad_x, pad_y) = padding_totals(style);
        let (pad_main, pad_cross) = if row { (pad_x, pad_y) } else { (pad_y, pad_x) };
        let (min_main, max_main, min_cross, max_cross) = if row {
            (
                style.min_width,
                style.max_width,
                style.min_height,
                style.max_height,
            )
        } else {
            (
                style.min_height,
                style.max_height,
                style.min_width,
                style.max_width,
            )
        };
        let inner_limit = (main_key.value - pad_main).max(0.0);
        let cross_gap =
            match node
                .props
                .get_symbol(if row { names.row_gap } else { names.column_gap })
            {
                Some(rsui_core::PropValue::Float(gap)) => *gap as f32,
                Some(rsui_core::PropValue::Int(gap)) => *gap as f32,
                _ => style.gap,
            };
        // A share of how tall it is, for what's in it, is of all of it when that's known, as CSS has it, not of a line.
        let definite_cross = cross_key
            .exact
            .then_some((cross_key.value - pad_cross).max(0.0));
        let definite_main = main_key.exact.then_some(inner_limit);
        let room = FlexRoom {
            row,
            align: Align::Start,
            definite_main,
            definite_cross,
            percent_main: view_share(current, node, style, row, pad_main, definite_main),
            cross_limit: (cross_key.value - pad_cross).max(0.0),
            least_main: false,
            collapse: false,
        };

        // How big each wants to be along.
        let mut items = Vec::with_capacity(node.children.len());
        let mut main_margins = Vec::with_capacity(node.children.len());
        let (mut any_overlay, mut any_absolute) = (false, false);
        for (index, (child_box, child)) in
            current.children.iter_mut().zip(&node.children).enumerate()
        {
            self.looked_at += 1;
            let start = start_item(&mut items, child_box, child, room);
            match start {
                ItemStart::Overlay => {
                    any_overlay = true;
                    continue;
                }
                ItemStart::Absolute => {
                    any_absolute = true;
                    continue;
                }
                _ => {}
            }
            let at = items.len() - 1;
            items[at].index = index;
            if let ItemStart::Ask(question) = start {
                let item_style = items[at].style;
                let answer =
                    self.solve_styled(child_box, child, &item_style, question, Mode::Measure);
                let (along, across) = if row {
                    (answer.width, answer.height)
                } else {
                    (answer.height, answer.width)
                };
                let base = self.base_past_max(child_box, child, &items[at], question, answer, row);
                let item = &mut items[at];
                item.base = base;
                item.main = along.clamp_to(item.min_main, item.max_main);
                item.wanted = Some((along, across));
            }
            let all = if items[at].style.has_margin {
                margins(&Style::read_margins(&child.props), row)
            } else {
                (0.0, 0.0, 0.0, 0.0)
            };
            main_margins.push(all);
        }

        // Which line each goes on is by how big it would be by itself, which is never less than what's in it lets it be (CSS's
        // `min-width: auto`), however little it starts from, as `flex: 1` starts from nothing: buttons that each grow from
        // nothing still go on as many lines as their words need.
        if items.iter().any(|item| {
            item.style.keeps_to_content(row) && item.style.has_basis()
        }) {
            self.keep_to_content(current, node, &mut items, row);
        }

        // Which go on which line.
        let mut lines: Vec<std::ops::Range<usize>> = Vec::new();
        let (mut first, mut used) = (0, 0.0f32);
        for at in 0..items.len() {
            let size = items[at].main + main_margins[at].0 + main_margins[at].1;
            if at > first && used + style.gap + size > inner_limit + 0.01 {
                lines.push(first..at);
                first = at;
                used = size;
            } else {
                used += if at > first { style.gap + size } else { size };
            }
        }
        if first < items.len() {
            lines.push(first..items.len());
        }

        // How big each line is. Where what's lined up by its text has that, and how far above it each line goes.
        let mut widest = 0f32;
        let mut line_sizes = Vec::with_capacity(lines.len());
        let mut baselines = Vec::new();
        let mut line_ascents = Vec::new();
        // As wide as its widest line, when how wide it is depends on what's in it, which what grows on a line grows to fill. A row
        // that doesn't fit on one line in the room it has fills that room, as CSS's `fit-content`, `min(max-content, room)`, has it:
        // a box of chips that wraps in 300 is 300 wide, with its lines in it as it says, not as wide as its widest line.
        let extent = |line: &std::ops::Range<usize>| {
            items[line.clone()].iter().map(|item| item.main).sum::<f32>()
                + style.gap * line.len().saturating_sub(1) as f32
                + main_margins[line.clone()].iter().map(|all| all.0 + all.1).sum::<f32>()
        };
        let widest_line = lines.iter().map(extent).fold(0f32, f32::max);
        let fills = row
            && main_key.value >= 0.0
            && inner_limit.is_finite()
            && (lines.len() > 1 || widest_line > inner_limit + 0.01);
        let fitted = if fills { inner_limit } else { widest_line.min(inner_limit) };
        for line in &lines {
            let margins_along: f32 = main_margins[line.clone()]
                .iter()
                .map(|all| all.0 + all.1)
                .sum();
            let gaps = style.gap * line.len().saturating_sub(1) as f32;
            let available = inner_limit - gaps - margins_along;
            // What's too big for its line by itself gets smaller to fit, down to as small as what's in it lets it, even when how
            // big this is depends on what's in it, as CSS has it: text that's too long for its line wraps.
            let too_big = inner_limit.is_finite()
                && items[line.clone()]
                    .iter()
                    .map(|item| item.main)
                    .sum::<f32>()
                    > available + 0.01;
            let grows = items[line.clone()].iter().any(|item| item.style.grow > 0.0);
            if too_big
                && items[line.clone()]
                    .iter()
                    .any(|item| item.style.keeps_to_content(row))
            {
                // Each no smaller than what's in it lets it be, as an inline-block `<span>` too wide for its line is, which
                // goes past the end of the line, as CSS has it.
                self.keep_to_content(current, node, &mut items[line.clone()], true);
            }
            if main_key.exact || too_big {
                share_space(&mut items[line.clone()], available, row);
                if too_big && row {
                    self.fit_wrapped_inline(current, node, &mut items[line.clone()]);
                }
            } else if grows {
                share_space(&mut items[line.clone()], fitted - gaps - margins_along, row);
            }
            widest = widest.max(
                items[line.clone()]
                    .iter()
                    .map(|item| item.main)
                    .sum::<f32>()
                    + gaps
                    + margins_along,
            );
            let mut biggest = 0f32;
            for at in line.clone() {
                let (index, item_style, main, cross_axis) = (
                    items[at].index,
                    items[at].style,
                    items[at].main,
                    items[at].cross_axis,
                );
                let across = match items[at].wanted {
                    Some((along, across)) if along == main => across,
                    _ => {
                        let answer = self.solve_styled(
                            &mut current.children[index],
                            &node.children[index],
                            &item_style,
                            make_key(row, Axis::exact(main), cross_axis),
                            Mode::Measure,
                        );
                        if row { answer.height } else { answer.width }
                    }
                };
                items[at].cross = across;
                biggest = biggest.max(across + main_margins[at].2 + main_margins[at].3);
            }
            if row
                && items[line.clone()]
                    .iter()
                    .any(|item| item.style.align_self.unwrap_or(style.align) == Align::Baseline)
            {
                let mut ascent = self.line_baselines(
                    current,
                    node,
                    &items,
                    &main_margins,
                    line.clone(),
                    &mut baselines,
                );
                let mut below = line
                    .clone()
                    .filter(|&at| baselines[at] >= 0.0)
                    .map(|at| items[at].cross - baselines[at] + main_margins[at].3)
                    .fold(0.0f32, f32::max);
                // A line of a page is at least as tall as a line of its own text would be, as CSS has it.
                if matches!(
                    node.props.get_symbol(names.line_strut),
                    Some(rsui_core::PropValue::Bool(true))
                ) && let Some((up, down)) = self.measure.strut(node)
                {
                    ascent = ascent.max(up);
                    below = below.max(down);
                }
                biggest = biggest.max(ascent + below);
                line_ascents.resize(line_sizes.len(), 0.0);
                line_ascents.push(ascent);
            }
            line_sizes.push(biggest);
        }
        let lines_total =
            line_sizes.iter().sum::<f32>() + cross_gap * lines.len().saturating_sub(1) as f32;
        let own_main = if main_key.exact {
            main_key.value
        } else {
            (if fills { widest.max(inner_limit) } else { widest } + pad_main)
                .within(main_key.value)
                .clamp_to(min_main, max_main)
        };
        let own_cross = if cross_key.exact {
            cross_key.value
        } else {
            (lines_total + pad_cross)
                .within(cross_key.value)
                .clamp_to(min_cross, max_cross)
        };
        let size = if row {
            Size::new(own_main, own_cross)
        } else {
            Size::new(own_cross, own_main)
        };
        if mode == Mode::Measure {
            return size;
        }

        // Where the lines go, and then where everything goes on each.
        let inner_main = (own_main - pad_main).max(0.0);
        let free_across = (own_cross - pad_cross).max(0.0) - lines_total;
        let (lead, more, stretch) = match node.props.get_symbol(names.align_content) {
            Some(rsui_core::PropValue::Str(how))
                if !matches!(how.as_str(), "stretch" | "normal") =>
            {
                let (lead, more) =
                    crate::grid::Spread::read(how.as_str()).offsets(free_across, lines.len());
                (lead, more, 0.0)
            }
            _ => (0.0, 0.0, free_across.max(0.0) / lines.len().max(1) as f32),
        };
        let (pad_main_start, pad_cross_start) = if row {
            (style.padding_left, style.padding_top)
        } else {
            (style.padding_top, style.padding_left)
        };
        let mut line_at = pad_cross_start + lead;
        for (number, (line, size_across)) in lines.iter().zip(&line_sizes).enumerate() {
            let line_ascent = line_ascents.get(number).copied().unwrap_or(0.0);
            let line_size = size_across + stretch;
            let margins_along: f32 = main_margins[line.clone()]
                .iter()
                .map(|all| all.0 + all.1)
                .sum();
            let total = items[line.clone()]
                .iter()
                .map(|item| item.main)
                .sum::<f32>()
                + style.gap * line.len().saturating_sub(1) as f32
                + margins_along;
            let (start_offset, extra) =
                spread_of(style.justify).offsets(inner_main - total, line.len());
            let mut position = pad_main_start + start_offset;
            for at in line.clone() {
                let (index, item_style, main) = (items[at].index, items[at].style, items[at].main);
                let (before, after, cross_before, cross_after) = main_margins[at];
                let align = item_style.align_self.unwrap_or(style.align);
                let (_, cross_length) = lengths(&item_style, row);
                let (_, _, least, most) = limits(&item_style, row);
                let room_across = (line_size - cross_before - cross_after).max(0.0);
                let final_cross =
                    match resolve_length(cross_length, Some(definite_cross.unwrap_or(line_size))) {
                        Some(size) => Axis::exact(size.clamp_to(least, most)),
                        None if align == Align::Stretch && !fits_across(&item_style, row) => {
                            Axis::exact(room_across.clamp_to(least, most))
                        }
                        // On a page, what isn't stretched down a row is as tall as what's in it, past the row if that's
                        // taller, as text with a line height taller than its inline-flex box is.
                        None if row && style.min_auto => Axis::at_most(most),
                        None => Axis::at_most(room_across.min(most)),
                    };
                let child = &node.children[index];
                let answer = self.solve_styled(
                    &mut current.children[index],
                    child,
                    &item_style,
                    make_key(row, Axis::exact(main), final_cross),
                    Mode::Layout,
                );
                let cross_size = if row { answer.height } else { answer.width };
                let offset = match align {
                    Align::Baseline
                        if row && baselines.get(at).is_some_and(|&found| found >= 0.0) =>
                    {
                        line_ascent - baselines[at]
                    }
                    Align::Stretch | Align::Start | Align::Baseline => cross_before,
                    // In the middle of a line of a page is half the height of an `x` above its text's baseline, as CSS's
                    // `vertical-align: middle` has it, rather than the middle of the line.
                    Align::Center
                        if row
                            && line_ascent > 0.0
                            && matches!(
                                node.props.get_symbol(names.line_strut),
                                Some(rsui_core::PropValue::Bool(true))
                            )
                            && let Some(x_height) = self.measure.x_height(node) =>
                    {
                        line_ascent - x_height / 2.0 - (cross_size + cross_before + cross_after) / 2.0 + cross_before
                    }
                    Align::Center => cross_before + (room_across - cross_size) / 2.0,
                    Align::End => line_size - cross_size - cross_after,
                };
                let child_box = &mut current.children[index];
                let along = position + before;
                if row {
                    child_box.x = along;
                    child_box.y = line_at + offset;
                } else {
                    child_box.x = line_at + offset;
                    child_box.y = along;
                }
                if item_style.moved {
                    let parent = if row {
                        (inner_main, line_size)
                    } else {
                        (line_size, inner_main)
                    };
                    apply_translate(child_box, child, parent);
                }
                position = along + main + after + style.gap + extra;
            }
            line_at += line_size + cross_gap + more;
        }
        if row {
            self.order_by_direction(current, node, &lines, &items, style.gap);
        }
        // From the end, as `row-reverse` (and a row in right-to-left text) has each line: turned around as `solve_flex` does.
        if style.reverse {
            reverse_along(current, style, row, size);
        }
        if any_overlay {
            self.solve_overlays(current, node);
        }
        if any_absolute {
            self.solve_absolutes(current, node, size);
        }
        self.place_passed_absolutes(current, node, size);
        current.update_contains_overlay();
        size
    }

    /// What's only part of a line (`inline_area`, as a `<span>` of text in a block is) that was too long for its line, and so got
    /// smaller, is as wide as its lines are once it wraps in what it got, as an inline box is as wide as its pieces on each line,
    /// not as all the room: a lone `<span>` that wraps in a centred block 200 wide is as wide as its widest line, in the middle.
    #[cold]
    #[inline(never)]
    fn fit_wrapped_inline(&mut self, current: &mut LayoutBox, node: &Element, items: &mut [Item]) {
        for item in items.iter_mut() {
            let child = &node.children[item.index];
            if !is_inline_area(child) || item.wanted.is_none_or(|(along, _)| item.main >= along - 0.01) {
                continue;
            }
            let answer = self.solve_styled(
                &mut current.children[item.index],
                child,
                &item.style,
                make_key(true, Axis::at_most(item.main), item.cross_axis),
                Mode::Measure,
            );
            if answer.width < item.main {
                item.main = answer.width;
                item.wanted = Some((answer.width, answer.height));
            }
        }
    }

    /// Puts one child with a margin in its place, and says where the next one goes. It's apart from [`Self::solve_flex`] so that
    /// everything else, which is nearly everything, does nothing at all about margins.
    /// `placed` is the margins along its parent it's placed with, when that isn't what it says.
    #[cold]
    #[inline(never)]
    fn place_with_margin(
        &mut self,
        child_box: &mut LayoutBox,
        child: &Element,
        item: &mut Item,
        (row, inner_cross, pad_cross_start, align, page): (bool, f32, f32, Align, bool),
        (position, between): (f32, f32),
        placed: Option<(f32, f32)>,
    ) -> f32 {
        let (main_before, main_after, cross_before, cross_after) =
            margins(&Style::read_margins(&child.props), row);
        let (main_before, main_after) = placed.unwrap_or((main_before, main_after));
        let (_, cross_length) = lengths(&item.style, row);
        let (_, _, child_min_cross, child_max_cross) = limits(&item.style, row);
        let final_cross = match resolve_cross(cross_length, inner_cross, item.cross_axis) {
            Some(size) => Axis::exact(size.clamp_to(child_min_cross, child_max_cross)),
            None if align == Align::Stretch && !fits_across(&item.style, row) => Axis::exact(
                (inner_cross - cross_before - cross_after)
                    .max(0.0)
                    .clamp_to(child_min_cross, child_max_cross),
            ),
            None if row && page => Axis::at_most(child_max_cross),
            None => item.cross_axis,
        };
        let answer = self.solve_styled(
            child_box,
            child,
            &item.style,
            make_key(row, Axis::exact(item.main), final_cross),
            Mode::Layout,
        );
        let cross_size = if row { answer.height } else { answer.width };
        let cross_position = pad_cross_start
            + match align {
                Align::Stretch | Align::Start | Align::Baseline => cross_before,
                Align::Center => {
                    cross_before + (inner_cross - cross_size - cross_before - cross_after) / 2.0
                }
                Align::End => inner_cross - cross_size - cross_after,
            };
        let at = position + main_before;
        if row {
            child_box.x = at;
            child_box.y = cross_position;
        } else {
            child_box.x = cross_position;
            child_box.y = at;
        }
        if item.style.moved {
            let parent = if row {
                (f32::NAN, inner_cross)
            } else {
                (inner_cross, f32::NAN)
            };
            apply_translate(child_box, child, parent);
        }
        at + item.main + main_after + between
    }

    /// Places what's placed by what it says, in its parent, once its parent's size is known.
    #[inline(never)]
    fn solve_absolutes(&mut self, current: &mut LayoutBox, node: &Element, outer: Size) {
        // What's placed by itself is placed from inside the borders of what it's in, as CSS has it.
        let [border_top, border_right, border_bottom, border_left] = border_sides(node);
        let parent = Size::new(
            (outer.width - border_left - border_right).max(0.0),
            (outer.height - border_top - border_bottom).max(0.0),
        );
        for (index, child) in node.children.iter().enumerate() {
            if !current.children[index].has(IS_ABSOLUTE) {
                continue;
            }
            let (style, _) = Style::read(&child.props);
            let [margin_top, margin_right, margin_bottom, margin_left] = if style.has_margin {
                Style::read_margins(&child.props)
            } else {
                [0.0; 4]
            };
            let names = &*NAMES;
            let inset = |name, size: f32| {
                child
                    .props
                    .get_symbol(name)
                    .and_then(|value| resolve_length(read_length(value), Some(size)))
            };
            let (left, right) = (
                inset(names.left, parent.width),
                inset(names.right, parent.width),
            );
            let (top, bottom) = (
                inset(names.top, parent.height),
                inset(names.bottom, parent.height),
            );
            // Both sides saying where they are comes to a size, when it doesn't say for itself. Otherwise it's as big as what's
            // in it: across, no wider than there's room for from where it starts, and down, as tall as that is, however tall
            // what it's in is, as CSS has it.
            let axis = absolute_axis;
            let width = axis(
                style.width,
                left,
                right,
                parent.width,
                margin_left,
                margin_right,
                style.min_width,
                style.max_width,
                true,
                style.fit_width,
            );
            let height = axis(
                style.height,
                top,
                bottom,
                parent.height,
                margin_top,
                margin_bottom,
                style.min_height,
                style.max_height,
                false,
                style.fit_height,
            );
            let answer = self.solve_styled(
                &mut current.children[index],
                child,
                &shrunk_to_fit(&style, width),
                Key { width, height },
                Mode::Layout,
            );
            // Placed by both sides, with `auto` margins on both, it's in the middle of the room between them, as CSS has it.
            let auto = |side: usize| {
                matches!(
                    child.props.get_symbol(names.margin_auto[side]),
                    Some(rsui_core::PropValue::Bool(true))
                )
            };
            // Saying neither side, it's where it would have been if it weren't placed by itself, as CSS's static position is.
            let unsaid = (
                left.is_none() && right.is_none(),
                top.is_none() && bottom.is_none(),
            );
            let (static_x, static_y) = if unsaid.0 || unsaid.1 {
                let outer_size = Size::new(
                    answer.width + margin_left + margin_right,
                    answer.height + margin_top + margin_bottom,
                );
                let (x, y) = static_position(current, node, index, &style, outer_size, outer);
                (x + margin_left, y + margin_top)
            } else {
                (0.0, 0.0)
            };
            let child_box = &mut current.children[index];
            child_box.x = inset_offset(left, right, parent.width, answer.width, (margin_left, margin_right), (auto(3), auto(1)))
                .map_or(static_x, |x| border_left + x);
            child_box.y = inset_offset(top, bottom, parent.height, answer.height, (margin_top, margin_bottom), (auto(0), auto(2)))
                .map_or(static_y, |y| border_top + y);
            if style.moved {
                apply_translate(child_box, child, (outer.width, outer.height));
            }
        }
    }

    /// Places what's placed by itself inside what's in this that isn't positioned, from this, which is what CSS places it from:
    /// what's in a page's `position: static` element is placed from the nearest box around it that's positioned. It's placed from
    /// what it's in first, as everything is, and moved here once this is known.
    fn place_passed_absolutes(&mut self, current: &mut LayoutBox, node: &Element, outer: Size) {
        if !places_absolutes(node)
            || !current
                .children
                .iter()
                .zip(&node.children)
                .any(|(child_box, child)| holds_passed(child_box, child))
        {
            return;
        }
        let [border_top, border_right, border_bottom, border_left] = border_sides(node);
        let room = Size::new(
            (outer.width - border_left - border_right).max(0.0),
            (outer.height - border_top - border_bottom).max(0.0),
        );
        // What's in a scrolling area has been moved by how far it's scrolled, and what's placed by itself in it is placed from
        // where it'd be without that, and then moves with it, as laying it out places it before it's scrolled: placed from where
        // things are shown, it stayed where it was in the window as the page went by, until the page was laid out again, as
        // floating-ui's dropdowns in the Mantine kitchen sink did whenever anything in them changed while it was scrolled.
        let (scrolled_x, scrolled_y) = if current.has(SCROLLS) && node.props.get_symbol(NAMES.row_height).is_none() {
            crate::Layout::scrolled_by(current, node)
        } else {
            (0.0, 0.0)
        };
        for (child_box, child) in current.children.iter_mut().zip(&node.children) {
            if holds_passed(child_box, child) {
                let base = (border_left - child_box.x - scrolled_x, border_top - child_box.y - scrolled_y);
                self.place_passed_in(child_box, child, room, base);
            }
        }
    }

    /// What's placed by itself in `current`, which passes it on, in `room`, from `base`: where the box it's placed from is, from
    /// `current`'s top left.
    fn place_passed_in(&mut self, current: &mut LayoutBox, node: &Element, room: Size, base: (f32, f32)) {
        for (child_box, child) in current.children.iter_mut().zip(&node.children) {
            if child_box.has(IS_OVERLAY) {
                continue;
            }
            if child_box.has(IS_ABSOLUTE) {
                // Only the page's own, rather than what rsui places by itself in an element, as a list's marker.
                if matches!(child.props.get_symbol(NAMES.static_box), Some(rsui_core::PropValue::Bool(false))) {
                    self.place_passed(child_box, child, room, base);
                }
            } else if holds_passed(child_box, child) {
                let inner = (base.0 - child_box.x, base.1 - child_box.y);
                self.place_passed_in(child_box, child, room, inner);
            }
        }
    }

    /// One thing placed by itself, from the box `room` big at `base`, along each way it says where it is, as `solve_absolutes`
    /// places it from what it's in. Along a way it doesn't say, it stays where it would have been.
    fn place_passed(&mut self, child_box: &mut LayoutBox, child: &Element, room: Size, base: (f32, f32)) {
        let (style, _) = Style::read(&child.props);
        if style.moved {
            return;
        }
        let [margin_top, margin_right, margin_bottom, margin_left] = if style.has_margin {
            Style::read_margins(&child.props)
        } else {
            [0.0; 4]
        };
        let names = &*NAMES;
        let inset = |name, size: f32| {
            child
                .props
                .get_symbol(name)
                .and_then(|value| resolve_length(read_length(value), Some(size)))
        };
        let (left, right) = (inset(names.left, room.width), inset(names.right, room.width));
        let (top, bottom) = (inset(names.top, room.height), inset(names.bottom, room.height));
        let (across, down) = (left.is_some() || right.is_some(), top.is_some() || bottom.is_some());
        if !across && !down {
            return;
        }
        let width = absolute_axis(
            style.width,
            left,
            right,
            room.width,
            margin_left,
            margin_right,
            style.min_width,
            style.max_width,
            true,
            style.fit_width,
        );
        let height = absolute_axis(
            style.height,
            top,
            bottom,
            room.height,
            margin_top,
            margin_bottom,
            style.min_height,
            style.max_height,
            false,
            style.fit_height,
        );
        let answer = self.solve_styled(child_box, child, &shrunk_to_fit(&style, width), Key { width, height }, Mode::Layout);
        let auto = |side: usize| {
            matches!(
                child.props.get_symbol(names.margin_auto[side]),
                Some(rsui_core::PropValue::Bool(true))
            )
        };
        if across {
            child_box.x = base.0
                + inset_offset(left, right, room.width, answer.width, (margin_left, margin_right), (auto(3), auto(1)))
                    .unwrap_or(0.0);
        }
        if down {
            child_box.y = base.1
                + inset_offset(top, bottom, room.height, answer.height, (margin_top, margin_bottom), (auto(0), auto(2)))
                    .unwrap_or(0.0);
        }
    }

    /// Rows and columns. See [`crate::grid`].
    #[inline(never)]
    fn solve_grid(
        &mut self,
        current: &mut LayoutBox,
        node: &Element,
        style: &Style,
        key: Key,
        mode: Mode,
    ) -> Size {
        let names = &*NAMES;
        let (pad_x, pad_y) = padding_totals(style);
        let text = |name| match node.props.get_symbol(name) {
            Some(rsui_core::PropValue::Str(text)) => text.as_str(),
            _ => "",
        };
        let gap_of = |name| match node.props.get_symbol(name) {
            Some(rsui_core::PropValue::Float(gap)) => *gap as f32,
            Some(rsui_core::PropValue::Int(gap)) => *gap as f32,
            _ => style.gap,
        };
        let (column_gap, row_gap) = (gap_of(names.column_gap), gap_of(names.row_gap));
        // With `grid_flow="column"`, what doesn't say where it goes fills each column before the next, and there are as many as it takes.
        let down_first = text(names.grid_flow).starts_with("column");
        // How many columns `repeat(auto-fit, ...)` comes to depends on how much room there is.
        let column_room = (key.width.value - pad_x).max(0.0);
        let mut columns = crate::grid::tracks_in(
            text(names.columns),
            Some(column_room),
            column_gap,
            node.children.len(),
        );
        let mut rows = crate::grid::tracks(text(names.rows));
        // Areas by name, as CSS's `grid-template-areas` says, which make as many tracks as they need where nothing else does.
        let areas = match node.props.get("grid_areas") {
            Some(rsui_core::PropValue::Str(said)) => crate::grid::areas(said),
            _ => crate::grid::Areas::default(),
        };
        if areas.columns > 0 && columns.len() < areas.columns {
            let first = columns.is_empty();
            columns.resize(
                areas.columns,
                if first {
                    crate::grid::Track::Fr(1.0)
                } else {
                    crate::grid::Track::Auto
                },
            );
        }
        while rows.len() < areas.rows {
            rows.push(crate::grid::Track::Auto);
        }
        if down_first {
            if rows.is_empty() {
                rows.push(crate::grid::Track::Auto);
            }
        } else if columns.is_empty() {
            // A page's grid that says nothing of its columns has one that's `auto`, which is only as wide as what's in it when
            // `justify-content` says where its columns go, as Radix's Callout's `flex-start` does, and fills it otherwise.
            let placed = style.min_auto && node.props.get_symbol(names.justify).is_some();
            columns.push(if placed {
                crate::grid::Track::Auto
            } else {
                crate::grid::Track::Fr(1.0)
            });
        }
        let width_room = key
            .width
            .exact
            .then_some((key.width.value - pad_x).max(0.0));
        let height_room = key
            .height
            .exact
            .then_some((key.height.value - pad_y).max(0.0));

        // Where everything goes.
        let mut any_overlay = false;
        let mut any_absolute = false;
        let mut styles = Vec::with_capacity(node.children.len());
        let mut all_margins: Vec<[f32; 4]> = Vec::with_capacity(node.children.len());
        let mut wanted = Vec::with_capacity(node.children.len());
        let mut in_grid = Vec::with_capacity(node.children.len());
        for (index, (child_box, child)) in
            current.children.iter_mut().zip(&node.children).enumerate()
        {
            let (child_style, _) = Style::read(&child.props);
            if child_style.overlay.is_some() {
                mark_overlay(child_box, child);
                any_overlay = true;
                continue;
            }
            child_box.flags &= !IS_OVERLAY;
            if child_style.absolute {
                child_box.flags |= IS_ABSOLUTE;
                mark_absolute_inert(child_box, child);
                any_absolute = true;
                continue;
            }
            child_box.flags &= !IS_ABSOLUTE;
            // Something in a named area is where that area is.
            let named = match child.props.get("grid_area") {
                Some(rsui_core::PropValue::Str(name)) => areas.find(name),
                _ => None,
            };
            wanted.push(match named {
                Some((across, down)) => (
                    crate::grid::span(&across, columns.len()),
                    crate::grid::span(&down, rows.len()),
                ),
                None => (
                    grid_place(
                        child,
                        [
                            names.grid_column,
                            names.grid_column_start,
                            names.grid_column_end,
                        ],
                        columns.len(),
                    ),
                    grid_place(
                        child,
                        [names.grid_row, names.grid_row_start, names.grid_row_end],
                        rows.len(),
                    ),
                ),
            });
            all_margins.push(if child_style.has_margin {
                Style::read_margins(&child.props)
            } else {
                [0.0; 4]
            });
            styles.push(child_style);
            in_grid.push(index);
        }
        // Tracks that aren't said are as `grid_auto_rows` and `grid_auto_columns` say, over and over, or as big as what's in them.
        let implicit = |name| {
            let said = crate::grid::tracks(text(name));
            if said.is_empty() {
                vec![crate::grid::Track::Auto]
            } else {
                said
            }
        };
        let places: Vec<(usize, usize)> = if down_first {
            let turned: Vec<_> = wanted
                .iter()
                .map(|&(across, down)| (down, across))
                .collect();
            let (places, column_count) = crate::grid::place(&turned, rows.len());
            let more = implicit(names.grid_auto_columns);
            let explicit = columns.len();
            while columns.len() < column_count {
                columns.push(more[(columns.len() - explicit) % more.len()]);
            }
            places
                .into_iter()
                .map(|(row, column)| (column, row))
                .collect()
        } else {
            let (places, row_count) = crate::grid::place(&wanted, columns.len());
            let more = implicit(names.grid_auto_rows);
            let explicit = rows.len();
            while rows.len() < row_count {
                rows.push(more[(rows.len() - explicit) % more.len()]);
            }
            places
        };
        // Where the tracks go in room they don't fill. Saying so stops tracks sized by what's in them from stretching, as CSS has it.
        let spread_across = node
            .props
            .get_symbol(names.justify)
            .is_some()
            .then_some(spread_of(style.justify));
        let spread_down = match text(names.align_content) {
            "" | "stretch" | "normal" => None,
            word => Some(crate::grid::Spread::read(word)),
        };

        // How wide each column has to be for what's in it, leaving out what's across several.
        let outer = |[top, right, bottom, left]: [f32; 4]| (left + right, top + bottom);
        let mut column_wants = vec![0f32; columns.len()];
        let needs_asking = columns.iter().any(|track| track.by_content())
            || (width_room.is_none()
                && columns.iter().any(|track| {
                    matches!(
                        track,
                        crate::grid::Track::Fr(_)
                            | crate::grid::Track::MinMax(_, crate::grid::Most::Fr(_))
                    )
                }));
        if needs_asking {
            for (at, &index) in in_grid.iter().enumerate() {
                if wanted[at].0.count != 1 {
                    continue;
                }
                let child_style = &styles[at];
                let answer = self.solve_styled(
                    &mut current.children[index],
                    &node.children[index],
                    child_style,
                    Key {
                        width: asked_about(
                            child_style.width,
                            Axis::UNBOUNDED,
                            child_style.min_width,
                            child_style.max_width,
                        ),
                        height: asked_about(
                            child_style.height,
                            Axis::UNBOUNDED,
                            child_style.min_height,
                            child_style.max_height,
                        ),
                    },
                    Mode::Measure,
                );
                let column = places[at].0;
                column_wants[column] =
                    column_wants[column].max(answer.width + outer(all_margins[at]).0);
            }
        }
        let column_sizes = crate::grid::sizes(
            &columns,
            &column_wants,
            width_room,
            column_gap,
            style.min_auto && spread_across.is_none(),
        );
        let cell_width = |column: usize, across: usize| {
            let end = (column + across).min(column_sizes.len());
            column_sizes[column..end].iter().sum::<f32>()
                + column_gap * end.saturating_sub(column + 1) as f32
        };

        let justify_items = match node.props.get_symbol(names.justify_items) {
            Some(value) => read_align(value, names.justify_items),
            None => Align::Stretch,
        };
        // How tall each row has to be, now that the widths are known.
        let mut row_wants = vec![0f32; rows.len()];
        for (at, &index) in in_grid.iter().enumerate() {
            if wanted[at].1.count != 1 {
                continue;
            }
            let child_style = &styles[at];
            let (extra_x, extra_y) = outer(all_margins[at]);
            let room = (cell_width(places[at].0, wanted[at].0.count) - extra_x).max(0.0);
            // As wide as it's going to be: what's stretched across its cell is as wide as that, as text inside it that wraps
            // there, as a label's with a field of width: 100% after it does, is as tall as it is there.
            let justify = match node.children[index].props.get_symbol(names.justify_self) {
                _ if child_style.fit_width => Align::Start,
                Some(value) => read_align(value, names.justify_self),
                None => justify_items,
            };
            let width = match resolve_length(child_style.width, Some(room)) {
                Some(size) => {
                    Axis::exact(size.clamp_to(child_style.min_width, child_style.max_width))
                }
                None if justify == Align::Stretch => {
                    Axis::exact(room.clamp_to(child_style.min_width, child_style.max_width))
                }
                None => Axis::at_most(room.min(child_style.max_width)),
            };
            let answer = self.solve_styled(
                &mut current.children[index],
                &node.children[index],
                child_style,
                Key {
                    width,
                    height: asked_about(
                        child_style.height,
                        Axis::UNBOUNDED,
                        child_style.min_height,
                        child_style.max_height,
                    ),
                },
                Mode::Measure,
            );
            let row = places[at].1;
            row_wants[row] = row_wants[row].max(answer.height + extra_y);
        }
        let row_sizes = crate::grid::sizes(
            &rows,
            &row_wants,
            height_room,
            row_gap,
            style.min_auto && spread_down.is_none(),
        );

        let total = |sizes: &[f32], gap: f32| {
            sizes.iter().sum::<f32>() + gap * sizes.len().saturating_sub(1) as f32
        };
        let size = Size::new(
            resolve_auto(
                key.width,
                total(&column_sizes, column_gap) + pad_x,
                style.min_width,
                style.max_width,
            ),
            resolve_auto(
                key.height,
                total(&row_sizes, row_gap) + pad_y,
                style.min_height,
                style.max_height,
            ),
        );
        if mode == Mode::Measure {
            return size;
        }

        // Place everything.
        let starts = |sizes: &[f32], gap: f32, from: f32| {
            let mut at = from;
            let mut out = Vec::with_capacity(sizes.len());
            for size in sizes {
                out.push(at);
                at += size + gap;
            }
            out
        };
        let place_tracks =
            |sizes: &[f32], gap: f32, from: f32, room: f32, spread: Option<crate::grid::Spread>| {
                let (lead, more) = spread.map_or((0.0, 0.0), |spread| {
                    spread.offsets(room - total(sizes, gap), sizes.len())
                });
                (starts(sizes, gap + more, from + lead), gap + more)
            };
        let (column_starts, column_step) = place_tracks(
            &column_sizes,
            column_gap,
            style.padding_left,
            size.width - pad_x,
            spread_across,
        );
        let (row_starts, row_step) = place_tracks(
            &row_sizes,
            row_gap,
            style.padding_top,
            size.height - pad_y,
            spread_down,
        );
        for (at, &index) in in_grid.iter().enumerate() {
            let child_style = &styles[at];
            let child = &node.children[index];
            let [margin_top, margin_right, margin_bottom, margin_left] = all_margins[at];
            let (column, row) = places[at];
            let (across, down) = (wanted[at].0.count, wanted[at].1.count);
            // Space shared out between tracks is inside what goes across them, as CSS has it.
            let cell = |sizes: &[f32], at: usize, count: usize, step: f32| {
                let end = (at + count).min(sizes.len());
                sizes[at..end].iter().sum::<f32>() + step * end.saturating_sub(at + 1) as f32
            };
            let cell_w = cell(&column_sizes, column, across, column_step);
            let cell_h = cell(&row_sizes, row, down, row_step);
            let (extra_x, extra_y) = outer(all_margins[at]);
            let vertical = child_style.align_self.unwrap_or(style.align);
            let justify_items = match child.props.get_symbol(names.justify_self) {
                Some(value) => read_align(value, names.justify_self),
                None => justify_items,
            };
            let axis =
                |own: Length, room: f32, align: Align, min: f32, max: f32| match resolve_length(
                    own,
                    Some(room),
                ) {
                    Some(size) => Axis::exact(size.clamp_to(min, max)),
                    None if align == Align::Stretch => {
                        Axis::exact(room.max(0.0).clamp_to(min, max))
                    }
                    None => Axis::at_most(room.max(0.0).min(max)),
                };
            let width = axis(
                child_style.width,
                cell_w - extra_x,
                if child_style.fit_width {
                    Align::Start
                } else {
                    justify_items
                },
                child_style.min_width,
                child_style.max_width,
            );
            let height = axis(
                child_style.height,
                cell_h - extra_y,
                vertical,
                child_style.min_height,
                child_style.max_height,
            );
            let child_box = &mut current.children[index];
            let answer = self.solve_styled(
                child_box,
                child,
                child_style,
                Key { width, height },
                Mode::Layout,
            );
            let offset = |align: Align, room: f32, size: f32, before: f32, after: f32| match align {
                Align::Stretch | Align::Start | Align::Baseline => before,
                Align::Center => before + (room - size - before - after) / 2.0,
                Align::End => room - size - after,
            };
            child_box.x = column_starts[column]
                + offset(
                    justify_items,
                    cell_w,
                    answer.width,
                    margin_left,
                    margin_right,
                );
            child_box.y = row_starts[row]
                + offset(vertical, cell_h, answer.height, margin_top, margin_bottom);
            if child_style.moved {
                apply_translate(child_box, child, (cell_w, cell_h));
            }
        }
        // Where text goes from right to left, the first column is at the right and a cell's start is its right side (CSS Grid
        // §7.1 with Writing Modes' inline direction; shadcn's CardHeader puts its action at the left in `dir="rtl"`): laid out
        // from the left and turned around, as a block and a flex row are, with what's in it having had its margins swapped for it
        // (see `mirrored_margins` in rsui-tsx). rsui-tsx says it as `cross_reverse`; a grid's own `text_direction` says it too.
        if style.cross_reverse
            || matches!(node.props.get("text_direction"), Some(rsui_core::PropValue::Str(how)) if how.as_str() == "rtl")
        {
            reverse_along(current, style, true, size);
        }
        if any_overlay {
            self.solve_overlays(current, node);
        }
        if any_absolute {
            self.solve_absolutes(current, node, size);
        }
        self.place_passed_absolutes(current, node, size);
        current.update_contains_overlay();
        size
    }

    /// Keeps a box's answer to a question that's about to be replaced, as it may be asked again, in this layout or a later one.
    #[inline(never)]
    fn remember_displaced(&mut self, current: &mut LayoutBox) {
        let displaced = Size::new(current.measured[0], current.measured[1]);
        current.keep_answer(current.measure_key(), displaced);
    }

    /// Lays out what's on top of everything else, each by itself, as if the window were its parent.
    #[inline(never)]
    fn solve_overlays(&mut self, current: &mut LayoutBox, node: &Element) {
        let (window_width, window_height) = (self.window_width, self.window_height);
        for (child_box, child) in current.children.iter_mut().zip(&node.children) {
            if !child_box.has(IS_OVERLAY) {
                continue;
            }
            let (style, _) = Style::read(&child.props);
            if style.overlay == Some(Placement::Inset) {
                let window = Size::new(window_width, window_height);
                let (width, height) = Sides::read(child, &style, window).axes(&style, window);
                self.solve_styled(child_box, child, &shrunk_to_fit(&style, width), Key { width, height }, Mode::Layout);
                continue;
            }
            let (width, height) = if style.overlay == Some(Placement::Fill) {
                (Axis::exact(window_width), Axis::exact(window_height))
            } else {
                let width = match resolve_length(style.width, Some(window_width)) {
                    Some(size) => Axis::exact(size.clamp_to(style.min_width, style.max_width)),
                    None => Axis::at_most(window_width.min(style.max_width)),
                };
                let height = match resolve_length(style.height, Some(window_height)) {
                    Some(size) => Axis::exact(size.clamp_to(style.min_height, style.max_height)),
                    None => Axis::at_most(window_height.min(style.max_height)),
                };
                (width, height)
            };
            self.solve(child_box, child, Key { width, height }, Mode::Layout);
        }
    }
}

/// Takes the boxes of what's in `nodes` out of `boxes`, into `flat`, with what's in something with no box of its own in its place,
/// however deep that goes, and the nodes they're for into `flat_nodes`. What's left behind is empty until [`put_back_spliced`].
fn take_spliced(
    boxes: &mut thin_vec::ThinVec<LayoutBox>,
    nodes: &[Node],
    flat_nodes: &mut thin_vec::ThinVec<Node>,
    flat: &mut thin_vec::ThinVec<LayoutBox>,
) {
    for (child_box, child) in boxes.iter_mut().zip(nodes) {
        if is_contents(child) {
            if child_box.children.len() != child.children.len() {
                set_box_count(child_box, child.children.len());
            }
            take_spliced(&mut child_box.children, &child.children, flat_nodes, flat);
        } else {
            flat_nodes.push(child.clone());
            flat.push(std::mem::take(child_box));
        }
    }
}

/// Puts the boxes [`take_spliced`] took back where they were. With `size`, they've just been laid out in what that's the size of, and
/// each box with nothing of its own is put where that is, as big, so that what's in it is where it was put.
fn put_back_spliced(
    boxes: &mut thin_vec::ThinVec<LayoutBox>,
    nodes: &[Node],
    flat: &mut impl Iterator<Item = LayoutBox>,
    size: Option<Size>,
) {
    for (child_box, child) in boxes.iter_mut().zip(nodes) {
        if is_contents(child) {
            put_back_spliced(&mut child_box.children, &child.children, flat, size);
            // It's never asked anything, so that anything changing in it is a change to what it's in, which lays it out.
            child_box.clear_results();
            child_box.flags &= !(IS_ABSOLUTE | IS_OVERLAY | STACKED | SCROLLS | PARAGRAPH);
            child_box.flags |= CONTENTS;
            child_box.update_contains_overlay();
            child_box.update_contains_absolute();
            if let Some(size) = size {
                (child_box.x, child_box.y) = (0.0, 0.0);
                (child_box.width, child_box.height) = (size.width, size.height);
            }
        } else if let Some(taken) = flat.next() {
            *child_box = taken;
        }
    }
}

/// Makes a box have this many children, by removing some or by adding empty ones.
#[inline(never)]
fn set_box_count(current: &mut LayoutBox, wanted: usize) {
    if current.children.len() > wanted {
        current.children.truncate(wanted);
    } else {
        let missing = wanted - current.children.len();
        current.children.reserve_exact(missing);
        current
            .children
            .extend((0..missing).map(|_| LayoutBox::default()));
    }
}

/// Works out what can be known about a child without asking it, and adds it to the items if it takes up room.
///
/// This is apart from [`Pass::solve_flex`] so that no style is on the stack while that goes down a level. The room each
/// level takes is what limits how deep a tree can be, and a real program's main thread only has 1 MB.
#[inline(never)]
fn start_item(
    items: &mut Vec<Item>,
    child_box: &mut LayoutBox,
    child: &Element,
    room: FlexRoom,
) -> ItemStart {
    let (mut style, _) = Style::read(&child.props);
    if room.collapse && is_bare_group(child) && !on_a_line(child) {
        style.collapse_margins = true;
    }
    if style.overlay.is_some() {
        mark_overlay(child_box, child);
        return ItemStart::Overlay;
    }
    // A share of what it's in, across, is how wide it can be, now that that's known.
    if let Some(share) = style.max_width_percent {
        let across = if room.row {
            room.definite_main
        } else {
            room.definite_cross.or(Some(room.cross_limit))
        };
        if let Some(across) = across.filter(|across| across.is_finite()) {
            style.max_width = style.max_width.min(share.at(across));
        }
    }
    // And how narrow it can't be, as `min-width: 100%` says.
    if let Some(share) = style.min_width_percent {
        let across = if room.row {
            room.definite_main
        } else {
            room.definite_cross.or(Some(room.cross_limit))
        };
        if let Some(across) = across.filter(|across| across.is_finite()) {
            style.min_width = style.min_width.max(share.at(across));
        }
    }
    child_box.flags &= !IS_OVERLAY;
    if style.absolute {
        child_box.flags |= IS_ABSOLUTE;
        mark_absolute_inert(child_box, child);
        return ItemStart::Absolute;
    }
    child_box.flags &= !IS_ABSOLUTE;
    let definite = !matches!(style.height, Length::Auto)
        || if room.row {
            !room.collapse && room.definite_cross.is_some() && style.align_self.unwrap_or(room.align) == Align::Stretch
        } else {
            !room.collapse && room.definite_main.is_some()
        };
    if definite != child_box.has(crate::tree::DEFINITE_HEIGHT) {
        child_box.flags ^= crate::tree::DEFINITE_HEIGHT;
    }
    let (main_length, cross_length) = lengths(&style, room.row);
    let (min_main, max_main, min_cross, max_cross) = limits(&style, room.row);
    let align = style.align_self.unwrap_or(room.align);
    let cross_margins = if style.has_margin {
        let (_, _, before, after) = margins(&Style::read_margins(&child.props), room.row);
        before + after
    } else {
        0.0
    };
    let cross_axis = match resolve_length(cross_length, room.definite_cross) {
        Some(size) => Axis::exact(size.clamp_to(min_cross, max_cross)),
        None => match room.definite_cross {
            Some(size) if align == Align::Stretch && !fits_across(&style, room.row) => Axis::exact(
                (size - cross_margins)
                    .max(0.0)
                    .clamp_to(min_cross, max_cross),
            ),
            _ if room.cross_limit < 0.0 => Axis::MIN_CONTENT,
            _ => Axis::at_most((room.cross_limit - cross_margins).max(0.0).min(max_cross)),
        },
    };
    // A share of what it's in is that much of how long it is along, and as big as what's in it while that isn't known, as CSS
    // has it.
    let basis = match style.basis_share {
        Some(share) => room.definite_main.map(|main| share.at(main).max(0.0)),
        None => style.basis,
    };
    // What it starts from is never less than its padding and border, as nothing is, so what starts from nothing, as `flex: 1`
    // does, starts from those, as CSS has it.
    let basis = basis.map(|basis| {
        let (across, down) = padding_totals(&style);
        basis.max(if room.row { across } else { down })
    });
    let known = basis.or_else(|| resolve_length(main_length, room.percent_main));
    let base = known.unwrap_or(0.0);
    items.push(Item {
        style,
        main: base.clamp_to(min_main, max_main),
        base,
        min_main,
        max_main,
        frozen: false,
        cross: 0.0,
        cross_axis,
        wanted: None,
        index: 0,
    });
    // Asked how narrow it can be along, each is asked the same, as a flex box is as narrow as what's in it can be, as CSS has it.
    let along = if room.least_main {
        Axis::MIN_CONTENT
    } else {
        Axis::UNBOUNDED
    };
    match known {
        Some(_) => ItemStart::Decided,
        None => ItemStart::Ask(make_key(room.row, along, cross_axis)),
    }
}

/// Shares `space` between items that can grow or shrink, keeping to their limits, along a row if `row`. An
/// item that hits a limit stops there, and the rest share what that leaves.
#[inline(never)]
fn share_space(items: &mut [Item], space: f32, row: bool) {
    let total: f32 = items.iter().map(|item| item.main).sum();
    let growing = space > total;
    // Every item can only stop once, so this can't loop forever.
    for _ in 0..=items.len() {
        let mut settled = 0.0;
        let mut factors = 0.0;
        for item in items.iter() {
            if item.frozen {
                settled += item.main;
            } else {
                settled += item.base;
                factors += if growing {
                    item.style.grow
                } else {
                    item.shrink_factor(row)
                };
            }
        }
        let mut left = space - settled;
        if factors <= 0.0 {
            break;
        }
        // Growing by less than one between them, they only take that share of the room, as CSS has it, so that what's growing
        // to its share from nothing, or from a smaller one, isn't all of it at once.
        if growing && factors < 1.0 {
            left *= factors;
        }
        let mut over = 0.0;
        for item in items.iter_mut().filter(|item| !item.frozen) {
            let factor = if growing {
                item.style.grow
            } else {
                item.shrink_factor(row)
            };
            let wanted = item.base + left * factor / factors;
            let limited = wanted.clamp_to(item.min_main, item.max_main);
            over += limited - wanted;
            item.main = limited;
        }
        // Freeze the items that the limits held back in the direction that matters.
        // If nothing was held back overall, everything is settled.
        let mut any_frozen = false;
        for item in items.iter_mut().filter(|item| !item.frozen) {
            let at_max = item.main >= item.max_main;
            let at_min = item.main <= item.min_main;
            let freeze = (over == 0.0) || (over < 0.0 && at_max) || (over > 0.0 && at_min);
            if freeze {
                item.frozen = true;
                any_frozen = true;
            }
        }
        if !any_frozen || over == 0.0 {
            break;
        }
    }
}

/// `f32::clamp` panics if the limits are the wrong way around, but a mistake in a
/// style shouldn't crash a program. As with `min_` and `max_` in CSS, the minimum wins.
trait ClampTo {
    fn clamp_to(self, min: f32, max: f32) -> f32;
    /// No more than `room`, unless that's [`Axis::MIN_CONTENT`]'s, which isn't any room at all.
    fn within(self, room: f32) -> f32;
}

impl ClampTo for f32 {
    fn clamp_to(self, min: f32, max: f32) -> f32 {
        self.min(max).max(min)
    }

    fn within(self, room: f32) -> f32 {
        if room < 0.0 { self } else { self.min(room) }
    }
}

/// Which of these have margins that are `auto` along their parent, before and after, and how much of `free` each of those gets,
/// as CSS has them take what's left over. It's empty if none do.
#[cold]
#[inline(never)]
fn auto_margins(node: &Element, items: &[Item], row: bool, free: f32) -> (Vec<(bool, bool)>, f32) {
    let [top_auto, right_auto, bottom_auto, left_auto] = NAMES.margin_auto;
    let (before, after) = if row {
        (left_auto, right_auto)
    } else {
        (top_auto, bottom_auto)
    };
    let is_auto = |element: &Element, name| {
        matches!(
            element.props.get_symbol(name),
            Some(rsui_core::PropValue::Bool(true))
        )
    };
    let autos: Vec<(bool, bool)> = items
        .iter()
        .map(|item| {
            let child = &node.children[item.index];
            (is_auto(child, before), is_auto(child, after))
        })
        .collect();
    let count: usize = autos
        .iter()
        .map(|&(before, after)| usize::from(before) + usize::from(after))
        .sum();
    if count == 0 {
        return (Vec::new(), 0.0);
    }
    (autos, free / count as f32)
}

/// How children share out the room left over along their parent, which is the same for a flex box's children as a grid's tracks.
fn spread_of(justify: Justify) -> crate::grid::Spread {
    match justify {
        Justify::Start => crate::grid::Spread::Start,
        Justify::Center => crate::grid::Spread::Center,
        Justify::End => crate::grid::Spread::End,
        Justify::SpaceBetween => crate::grid::Spread::Between,
        Justify::SpaceAround => crate::grid::Spread::Around,
        Justify::SpaceEvenly => crate::grid::Spread::Evenly,
    }
}

/// Notes whether any of what's in this says how far up it goes, so that it's drawn and found in that order.
#[cold]
#[inline(never)]
/// CSS's `order` of a flex or grid item, 0 when it says none.
fn order_of(child: &Element) -> f64 {
    match child.props.get_symbol(NAMES.order) {
        Some(rsui_core::PropValue::Int(order)) => *order as f64,
        Some(rsui_core::PropValue::Float(order)) if order.is_finite() => *order,
        _ => 0.0,
    }
}

/// Whether what's in this is to be laid out in another order than it's written in, by CSS's `order`: some item says a lower one
/// than one before it.
#[inline(never)]
fn out_of_order(node: &Element) -> bool {
    let name = NAMES.order;
    if !node.children.iter().any(|child| child.props.get_symbol(name).is_some()) {
        return false;
    }
    node.children.windows(2).any(|pair| order_of(&pair[1]) < order_of(&pair[0]))
}

fn note_stacking(current: &mut LayoutBox, node: &Element) {
    let name = NAMES.z_index;
    if node
        .children
        .iter()
        .any(|child| child.props.get_symbol(name).is_some())
    {
        current.flags |= STACKED;
    } else {
        current.flags &= !STACKED;
    }
}

/// Whether this is only part of a line, whose box is only as tall as its fonts make it, as a CSS inline box is (`inline_area`).
fn is_inline_area(node: &Element) -> bool {
    matches!(node.props.get_symbol(NAMES.inline_area), Some(rsui_core::PropValue::Bool(true)))
}

/// Whether all that's in a block is pictures that sit on a line of text, as `on_line` says they do, which leaves room below them for
/// what goes under a line, as a page has it.
fn on_a_line(node: &Element) -> bool {
    let name = NAMES.on_line;
    !node.children.is_empty()
        && node.children.iter().all(|child| {
            child.role == rsui_core::Role::Image
                && matches!(
                    child.props.get_symbol(name),
                    Some(rsui_core::PropValue::Bool(true))
                )
        })
}

/// How much margin goes out of the top of a block (or the bottom, with `after`) from what's first (or last) in it, as CSS has it:
/// a block with nothing between its edge and what's in it, no padding or border, doesn't keep that margin in. Its own margin
/// goes together with that, which is for what it's in to do.
fn escaping_margin(node: &Element, after: bool) -> f32 {
    let (mut style, _) = Style::read(&node.props);
    // It's only asked in a block, where a bare group lets margins through, as `start_item` has it.
    if is_bare_group(node) {
        style.collapse_margins = true;
    }
    let edge = if after {
        style.padding_bottom
    } else {
        style.padding_top
    };
    let fits = !after || (matches!(style.height, Length::Auto) && style.min_height <= 0.0);
    if !style.collapse_margins
        || style.keep_margins
        || style.direction == Direction::Row
        || style.grid
        // A table's caption and rows keep their margins in, as its wrapper box has a formatting context of its own.
        || style.table != TablePart::None
        || style.scroll_x.is_some()
        || style.scroll_y.is_some()
        || edge != 0.0
        || !fits
    {
        return 0.0;
    }
    let mut in_flow = node.children.iter().filter(|child| {
        let (child_style, _) = Style::read(&child.props);
        child_style.overlay.is_none() && !child_style.absolute
    });
    let Some(child) = (if after {
        in_flow.next_back()
    } else {
        in_flow.next()
    }) else {
        return 0.0;
    };
    let [top, _, bottom, _] = Style::read_margins(&child.props);
    let own = if after { bottom } else { top };
    together(own, escaping_margin(child, after))
}

/// Two margins that go together, as CSS has them: the biggest of them, less the most that's less than nothing.
fn together(one: f32, other: f32) -> f32 {
    one.max(other).max(0.0) + one.min(other).min(0.0)
}

/// The margins of something, as what's before it and after it along its parent's direction, then the same across it.
#[inline]
fn margins(margin: &[f32; 4], row: bool) -> (f32, f32, f32, f32) {
    let [top, right, bottom, left] = *margin;
    if row {
        (left, right, top, bottom)
    } else {
        (top, bottom, left, right)
    }
}

/// For a scroller, or what a page's scroller lays its content out in, laid out as tall as what's in it, what a height that's a
/// share is of in it: how tall the scroller shows, inside its padding, when that's known (see `VIEW_HEIGHT` in style.rs).
/// That's what it is for what a page's scroller lays its content out in whatever that's asked, as that isn't a box on the page:
/// how tall it is is how tall its content is. Otherwise a size along that's known, `definite`, is what a share is of.
#[inline(never)]
fn view_share(current: &mut LayoutBox, node: &Element, style: &Style, row: bool, pad_main: f32, definite: Option<f32>) -> Option<f32> {
    let content = !row && crate::tree::is_scroll_content(node);
    if row || (definite.is_some() && !content) || (!content && style.scroll_y.is_none()) {
        return definite;
    }
    let view = crate::style::VIEW_HEIGHT.with(std::cell::Cell::get);
    if content {
        current.flags |= crate::tree::USES_VIEW_HEIGHT;
    }
    view.is_finite().then(|| (view - pad_main).max(0.0))
}

/// What to ask something that may have a size of its own, which it's for whatever's asking to work out. A percentage of something
/// that isn't known yet is as if it hadn't said.
fn asked_about(own: Length, otherwise: Axis, min: f32, max: f32) -> Axis {
    match resolve_length(own, None) {
        Some(size) => Axis::exact(size.clamp_to(min, max)),
        None => otherwise,
    }
}

/// Moves something from where it was placed, which changes nothing about what's around it.
#[inline(never)]
/// Moves something from where it was laid out, by what it says: `translate_x` and `translate_y` as part of its own size, and `left`, `top`,
/// `right` and `bottom` as part of what it's in, which is `parent`, unless those say where it goes.
fn apply_translate(child_box: &mut LayoutBox, child: &Element, parent: (f32, f32)) {
    let (x, y) = Style::read_translate(&child.props, child_box.width, child_box.height, parent);
    child_box.x += x;
    child_box.y += y;
}

/// The room something has, asked about `key`: no wider than it can be, as `max_width` says (unless `min_width` says more, which
/// wins), so what's in it is laid out in that, as CSS has it. Text in a flex item with `max-width: 120px` that's asked how wide it
/// is with room to spare wraps at 120, and is as tall as that makes it, not as tall as it is on one line. What's told how wide to
/// be is that wide already.
#[inline(always)]
fn within_own_max(key: Key, style: &Style) -> Key {
    let most = style.max_width.max(style.min_width);
    if key.width.exact || key.width.value <= most {
        return key;
    }
    Key {
        width: Axis::at_most(most),
        height: key.height,
    }
}

fn padding_totals(style: &Style) -> (f32, f32) {
    (
        style.padding_left + style.padding_right,
        style.padding_top + style.padding_bottom,
    )
}

fn resolve_auto(axis: Axis, wanted: f32, min: f32, max: f32) -> f32 {
    if axis.exact {
        axis.value
    } else {
        wanted.within(axis.value).clamp_to(min, max)
    }
}

fn lengths(style: &Style, row: bool) -> (Length, Length) {
    if row {
        (style.width, style.height)
    } else {
        (style.height, style.width)
    }
}

fn limits(style: &Style, row: bool) -> (f32, f32, f32, f32) {
    if row {
        (
            style.min_width,
            style.max_width,
            style.min_height,
            style.max_height,
        )
    } else {
        (
            style.min_height,
            style.max_height,
            style.min_width,
            style.max_width,
        )
    }
}

/// A flex item's size across, as it's placed: a percentage of a container whose size across wasn't known when the item was
/// started (`asked`, worked out from it then, isn't exact) is as if it said nothing, as CSS has a percentage of an indefinite
/// size, so shadcn's vertical `<Separator className="h-full">` in an `items-center` row is 0px tall rather than the row's height.
/// It's stretched then, if it's stretched.
fn resolve_cross(length: Length, inner_cross: f32, asked: Axis) -> Option<f32> {
    match length {
        Length::Percent(_) if !asked.exact => None,
        Length::Calc(percent, _) if percent != 0.0 && !asked.exact => None,
        Length::Clamp(clamp) if clamp.has_share() && !asked.exact => None,
        Length::Mix(along, ends) if Mix::of(along, ends).has_share() && !asked.exact => None,
        _ => resolve_length(length, Some(inner_cross)),
    }
}

/// A length's size in pixels, if it has one. A percentage needs a parent size to be a
/// percentage of.
fn resolve_length(length: Length, parent: Option<f32>) -> Option<f32> {
    match length {
        Length::Auto => None,
        Length::Px(value) => Some(value),
        Length::Percent(percent) => parent.map(|parent| parent * percent / 100.0),
        Length::Calc(percent, pixels) => parent.map(|parent| parent * percent / 100.0 + pixels),
        Length::Clamp(clamp) => parent.map(|parent| clamp.at(parent)),
        Length::Mix(along, ends) => parent.map(|parent| Mix::of(along, ends).at(parent)),
    }
}

/// Whether something isn't stretched across what it's in, as `fit-content` says.
fn fits_across(style: &Style, row: bool) -> bool {
    if row {
        style.fit_height
    } else {
        style.fit_width
    }
}

/// Moves what sticks to the top or bottom of what it scrolls in so that it stays in view, as far as it can without leaving what it's in.
/// `top` is where `current` is, from the top of the view, and `view` is how tall the view is.
fn stick(current: &mut LayoutBox, node: &Element, top: f32, height: f32, view: f32) {
    let names = &*crate::style::NAMES;
    let number = |child: &Element, name| match child.props.get_symbol(name) {
        Some(rsui_core::PropValue::Float(value)) => Some(*value as f32),
        Some(rsui_core::PropValue::Int(value)) => Some(*value as f32),
        _ => None,
    };
    for (child_box, child) in current.children.iter_mut().zip(&node.children) {
        let at = top + child_box.y;
        if let Some(gap) = number(child, names.sticky_bottom) {
            let lowest = view - gap - child_box.height;
            if at > lowest {
                child_box.y -= (at - lowest).min(at - top).max(0.0);
            }
        }
        if let Some(gap) = number(child, names.sticky_top) {
            let at = top + child_box.y;
            if at < gap {
                let room = top + height - (at + child_box.height);
                child_box.y += (gap - at).min(room.max(0.0));
            }
        }
        // What scrolls by itself has its own view.
        if child.props.get("scroll_y").is_none() && !child_box.children.is_empty() {
            let (inner_top, inner_height) = (top + child_box.y, child_box.height);
            stick(child_box, child, inner_top, inner_height, view);
        }
    }
}

fn make_key(row: bool, main: Axis, cross: Axis) -> Key {
    if row {
        Key {
            width: main,
            height: cross,
        }
    } else {
        Key {
            width: cross,
            height: main,
        }
    }
}

/// Gives a part of a paragraph, and what's inside it, a box around where its text is. Each box
/// ends up relative to `origin`, which is where the one it's in starts from.
fn place_part(
    current: &mut LayoutBox,
    node: &Element,
    origin: (f32, f32),
    places: &[Rect],
    next: &mut usize,
) -> Rect {
    if current.children.len() != node.children.len() {
        set_box_count(current, node.children.len());
    }
    // It isn't laid out as a box, whatever it may have been the last time, and what it was told then isn't true now.
    current.clear_results();
    current.flags &= !(SELF_DIRTY
        | CHILD_DIRTY
        | MANY_CHILDREN_DIRTY
        | IS_OVERLAY
        | CONTAINS_OVERLAY
        | OVERLAY_IS_INERT
        | SCROLLS
        | PARAGRAPH);
    let place = if node.children.is_empty() {
        let place = if node.name.is_empty() {
            None
        } else {
            *next += 1;
            places.get(*next - 1).copied()
        };
        place.unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0))
    } else {
        // What's inside it says where it is, so those start out from the same place as this does.
        let mut around: Option<Rect> = None;
        for (child_box, child) in current.children.iter_mut().zip(&node.children) {
            let place = place_part(child_box, child, (0.0, 0.0), places, next);
            if place.width > 0.0 || place.height > 0.0 {
                around = Some(around.map_or(place, |so_far| {
                    let left = so_far.x.min(place.x);
                    let top = so_far.y.min(place.y);
                    let right = (so_far.x + so_far.width).max(place.x + place.width);
                    let bottom = (so_far.y + so_far.height).max(place.y + place.height);
                    Rect::new(left, top, right - left, bottom - top)
                }));
            }
        }
        let around = around.unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
        for child_box in &mut current.children {
            child_box.x -= around.x;
            child_box.y -= around.y;
        }
        around
    };
    current.x = place.x + origin.0;
    current.y = place.y + origin.1;
    current.width = place.width;
    current.height = place.height;
    // Preserve the existing part flags (including any retained IS_ABSOLUTE),
    // summarizing the boxes that exhaustive lifted discovery actually sees.
    current.update_contains_absolute();
    place
}

impl<M: Measure> Pass<'_, M> {
    /// Puts what's on each line of text that goes from the right in the order the Unicode Bidirectional Algorithm puts it in
    /// (rule L2), as CSS has its inline boxes: Arabic words and the `<kbd>`s and icons between them from the right, a run of
    /// English among them from the left. Each child's level is what its text comes to (`Measure::line_levels`), and each keeps
    /// its own margins and the line's gap after it. `lines` are ranges of `items`, laid out from the left in the order of the text.
    #[inline(never)]
    fn order_by_direction(
        &mut self,
        current: &mut LayoutBox,
        node: &Element,
        lines: &[std::ops::Range<usize>],
        items: &[Item],
        gap: f32,
    ) {
        if !right_to_left(node) || !matches!(node.props.get_symbol(NAMES.line_strut), Some(rsui_core::PropValue::Bool(true))) {
            return;
        }
        let mut levels = Vec::new();
        self.measure.line_levels(node, &mut levels);
        if levels.len() != node.children.len() {
            return;
        }
        let mut order: Vec<usize> = Vec::new();
        for line in lines {
            let on_line = &items[line.clone()];
            if on_line.len() < 2 {
                continue;
            }
            order.clear();
            order.extend(0..on_line.len());
            let level_of = |at: usize| levels[on_line[at].index];
            let highest = (0..on_line.len()).map(level_of).max().unwrap_or(0);
            let lowest_odd = (0..on_line.len()).map(level_of).filter(|level| level % 2 == 1).min().unwrap_or(highest + 1);
            for level in (lowest_odd.max(1)..=highest).rev() {
                let mut at = 0;
                while at < order.len() {
                    if level_of(order[at]) >= level {
                        let mut end = at + 1;
                        while end < order.len() && level_of(order[end]) >= level {
                            end += 1;
                        }
                        order[at..end].reverse();
                        at = end;
                    } else {
                        at += 1;
                    }
                }
            }
            if order.iter().enumerate().all(|(at, item)| at == *item) {
                continue;
            }
            let margins_of = |item: &Item| {
                if item.style.has_margin {
                    let [_, right, _, left] = Style::read_margins(&node.children[item.index].props);
                    (left, right)
                } else {
                    (0.0, 0.0)
                }
            };
            let mut pen = on_line
                .iter()
                .map(|item| current.children[item.index].x - margins_of(item).0)
                .fold(f32::INFINITY, f32::min);
            for &at in &order {
                let item = &on_line[at];
                let (left, right) = margins_of(item);
                let child = &mut current.children[item.index];
                child.x = pen + left;
                pen = child.x + child.width + right + gap;
            }
        }
    }
}

/// Turns what's laid out from the start to the end around, as CSS's `row-reverse` and `column-reverse` have it: each child is as
/// far from the end of what's inside the padding as it was from the start.
fn reverse_along(current: &mut LayoutBox, style: &Style, row: bool, size: Size) {
    for child in current
        .children
        .iter_mut()
        .filter(|child| !child.has(IS_OVERLAY | IS_ABSOLUTE))
    {
        if row {
            child.x = size.width - child.x - child.width + style.padding_left - style.padding_right;
        } else {
            child.y =
                size.height - child.y - child.height + style.padding_top - style.padding_bottom;
        }
    }
}

/// Where something goes in a grid along one way, from what it says as `[grid_column, grid_column_start, grid_column_end]` or the same
/// for rows: where it starts and ends, either of which its own `start` or `end` goes over, as CSS's longhands go over a shorthand.
fn grid_place(
    child: &Element,
    [whole, start, end]: [rsui_core::Symbol; 3],
    count: usize,
) -> crate::grid::Span {
    let text = |name| -> Option<std::borrow::Cow<'_, str>> {
        match child.props.get_symbol(name)? {
            rsui_core::PropValue::Str(text) => Some(text.as_str().into()),
            rsui_core::PropValue::Int(line) => Some(line.to_string().into()),
            rsui_core::PropValue::Float(line) => Some((*line as i64).to_string().into()),
            rsui_core::PropValue::Bool(_) => None,
        }
    };
    let whole = text(whole);
    let (own_start, own_end) = (text(start), text(end));
    if own_start.is_none() && own_end.is_none() {
        return whole.map_or_else(crate::grid::Span::default, |whole| {
            crate::grid::span(&whole, count)
        });
    }
    let (first, second) = match whole.as_deref().map(|whole| whole.split_once('/')) {
        Some(Some((first, second))) => (first.to_owned(), second.to_owned()),
        Some(None) => (
            whole.as_deref().unwrap_or("auto").to_owned(),
            "auto".to_owned(),
        ),
        None => ("auto".to_owned(), "auto".to_owned()),
    };
    crate::grid::span_between(
        own_start.as_deref().unwrap_or(&first),
        own_end.as_deref().unwrap_or(&second),
        count,
    )
}

/// Where something placed by itself would be if it didn't say where, as CSS's static position has it: the top left of its margin box,
/// in what it's in, which is `size` with its margins. In a flex box, that's where it would be if it were all that's in it, lined up as
/// the flex box lines up what's in it, so an icon over another in a button that puts what's in it in the middle is in the middle too.
/// In a block, it's where it would have been among what's in it, under what's before it. In a grid or a table, it's where what's in
/// it starts.
fn static_position(
    current: &LayoutBox,
    node: &Element,
    index: usize,
    style: &Style,
    size: Size,
    outer: Size,
) -> (f32, f32) {
    let (parent, _) = Style::read(&node.props);
    let (x, y) = static_position_in(current, node, index, style, size, outer, &parent);
    // Where text goes from right to left, the start across a block, a column or a grid is its right side (CSS Writing Modes §2.1,
    // with CSS 2.1 §10.3.7's static position): what would have started at the left starts as far from the right.
    if parent.cross_reverse && (parent.direction != Direction::Row || parent.grid || parent.table != TablePart::None) {
        let from_left = x - parent.padding_left;
        return (outer.width - parent.padding_right - size.width - from_left, y);
    }
    (x, y)
}

fn static_position_in(
    current: &LayoutBox,
    node: &Element,
    index: usize,
    style: &Style,
    size: Size,
    outer: Size,
    parent: &Style,
) -> (f32, f32) {
    let start = (parent.padding_left, parent.padding_top);
    if parent.grid || parent.table != TablePart::None {
        return start;
    }
    let row = parent.direction == Direction::Row;
    // A line of text, which is a row of what's on it (`line_strut`), has it after what's before it on the line, as CSS's static
    // position is in a line: Radix's focus proxy after a navigation menu's trigger, rather than at the start of its `<li>`.
    let on_line = row && !parent.reverse && node.props.get_symbol(NAMES.line_strut).is_some();
    if parent.collapse_margins || on_line {
        let before = current.children[..index]
            .iter()
            .zip(&node.children[..index])
            .rev()
            .find(|(child_box, _)| !child_box.has(IS_ABSOLUTE | IS_OVERLAY));
        let Some((before, child)) = before else {
            return start;
        };
        let [_, margin_right, margin_bottom, _] = Style::read_margins(&child.props);
        return if row {
            (before.x + before.width + margin_right, start.1)
        } else {
            (start.0, before.y + before.height + margin_bottom)
        };
    }
    let room = Size::new(
        outer.width - parent.padding_left - parent.padding_right,
        outer.height - parent.padding_top - parent.padding_bottom,
    );
    let (free_main, free_cross) = if row {
        (room.width - size.width, room.height - size.height)
    } else {
        (room.height - size.height, room.width - size.width)
    };
    let main = match parent.justify {
        Justify::Start | Justify::SpaceBetween => 0.0,
        Justify::End => free_main,
        Justify::Center | Justify::SpaceAround | Justify::SpaceEvenly => free_main / 2.0,
    };
    // What goes from the end starts from there.
    let main = if parent.reverse {
        free_main - main
    } else {
        main
    };
    let cross = match style.align_self.unwrap_or(parent.align) {
        Align::Center => free_cross / 2.0,
        Align::End => free_cross,
        Align::Stretch | Align::Start | Align::Baseline => 0.0,
    };
    if row {
        (start.0 + main, start.1 + cross)
    } else {
        (start.0 + cross, start.1 + main)
    }
}

/// How wide each of an element's borders is, as `[top, right, bottom, left]`: a border's only drawn, and only takes room, with a
/// color.
/// What's placed by itself, and doesn't say how wide it is or isn't between two insets, is as wide as what's in it, as far as
/// there's room, but never narrower than what's in it can be, as CSS's shrink-to-fit has it: a slider's value label, `white-space:
/// nowrap` in a thumb 8 pixels wide, is as wide as its number, rather than too narrow to show it.
fn shrunk_to_fit(style: &Style, width: Axis) -> Style {
    let mut style = *style;
    if !width.exact {
        style.fit_width = true;
    }
    style
}

/// How big something placed by itself is along one side, as `solve_absolutes` asks: as big as it says, or from both sides that
/// say where it is to the room between them, or as big as what's in it, across no wider than there's room for from where it
/// starts, and down, as tall as that is, however tall what it's in is, as CSS has it.
#[allow(clippy::too_many_arguments)]
fn absolute_axis(
    own: Length,
    near: Option<f32>,
    far: Option<f32>,
    room: f32,
    before: f32,
    after: f32,
    min: f32,
    max: f32,
    across: bool,
    fits: bool,
) -> Axis {
    match (resolve_length(own, Some(room)), near, far) {
        (Some(size), _, _) => Axis::exact(size.clamp_to(min, max)),
        // As big as what's in it, as `fit-content` says, as far as there's room between the two.
        (None, Some(near), Some(far)) if fits => {
            Axis::at_most((room - near - far - before - after).max(0.0).min(max))
        }
        (None, Some(near), Some(far)) => Axis::exact(
            (room - near - far - before - after)
                .max(0.0)
                .clamp_to(min, max),
        ),
        _ if across => {
            let room = (room - near.or(far).unwrap_or(0.0) - before - after)
                .max(0.0)
                .min(max);
            // At least as wide as it says, even where there's less room than that, which is how wide what's in it
            // has to be laid out at, as CSS has it.
            if min > room {
                Axis::exact(min.min(max))
            } else {
                Axis::at_most(room)
            }
        }
        _ => Axis::at_most(max),
    }
}

/// Where something placed by its sides starts along one way, from the start of the room it's placed in (CSS 2.1 §10.3.7 and
/// §10.6.4): from the near side and its margin, or from the far side less its size and margin; with both sides and `auto` margins
/// on both, in the middle of what's left between them (never before the near side, as a left-to-right page has it), and with only
/// the near margin `auto`, against the far side. Saying neither side, it's `None`: where it would have been.
fn inset_offset(
    near: Option<f32>,
    far: Option<f32>,
    room: f32,
    size: f32,
    (before, after): (f32, f32),
    (auto_before, auto_after): (bool, bool),
) -> Option<f32> {
    match (near, far) {
        (Some(near), Some(far)) if auto_before && auto_after => Some(near + ((room - near - far - size) / 2.0).max(0.0)),
        (Some(_), Some(far)) if auto_before => Some(room - far - size - after),
        (Some(near), _) => Some(near + before),
        (None, Some(far)) => Some(room - far - size - after),
        (None, None) => None,
    }
}

/// The sides, margins and `auto` margins of what's placed by its sides in the window (`Placement::Inset`), with shares of the
/// window worked out, as top, right, bottom and left.
struct Sides {
    insets: [Option<f32>; 4],
    margins: [f32; 4],
    auto: [bool; 4],
}

impl Sides {
    fn read(child: &Element, style: &Style, window: Size) -> Self {
        let names = &*NAMES;
        let inset = |name, size: f32| {
            child
                .props
                .get_symbol(name)
                .and_then(|value| resolve_length(read_length(value), Some(size)))
        };
        let auto = |side: usize| {
            matches!(
                child.props.get_symbol(names.margin_auto[side]),
                Some(rsui_core::PropValue::Bool(true))
            )
        };
        Self {
            insets: [
                inset(names.top, window.height),
                inset(names.right, window.width),
                inset(names.bottom, window.height),
                inset(names.left, window.width),
            ],
            margins: if style.has_margin { Style::read_margins(&child.props) } else { [0.0; 4] },
            auto: [auto(0), auto(1), auto(2), auto(3)],
        }
    }

    /// How big it's laid out across and down.
    fn axes(&self, style: &Style, window: Size) -> (Axis, Axis) {
        let [top, right, bottom, left] = self.insets;
        let [margin_top, margin_right, margin_bottom, margin_left] = self.margins;
        let width = absolute_axis(
            style.width,
            left,
            right,
            window.width,
            margin_left,
            margin_right,
            style.min_width,
            style.max_width,
            true,
            style.fit_width,
        );
        let height = absolute_axis(
            style.height,
            top,
            bottom,
            window.height,
            margin_top,
            margin_bottom,
            style.min_height,
            style.max_height,
            false,
            style.fit_height,
        );
        (width, height)
    }

    /// Where it goes in the window at this size, along each way it says anything about.
    fn offsets(&self, window: Size, (width, height): (f32, f32)) -> (Option<f32>, Option<f32>) {
        let [top, right, bottom, left] = self.insets;
        let [margin_top, margin_right, margin_bottom, margin_left] = self.margins;
        let [auto_top, auto_right, auto_bottom, auto_left] = self.auto;
        (
            inset_offset(left, right, window.width, width, (margin_left, margin_right), (auto_left, auto_right)),
            inset_offset(top, bottom, window.height, height, (margin_top, margin_bottom), (auto_top, auto_bottom)),
        )
    }
}

/// Whether something is a page's element that's positioned, which what's placed by itself in it is placed from, as CSS has it,
/// with the page itself: `web.js` says `static_box` is false for those, and true for a page's element that isn't positioned, which
/// passes what's placed by itself in it on to what's around it, as Mantine's modal root does with its overlay, which is
/// `position: absolute; inset: 0` and covers the box with `position: relative` around the root. What says neither, as what rsui
/// puts around a page's elements, passes it on too; nothing does in what isn't a page.
pub(crate) fn places_absolutes(element: &Element) -> bool {
    matches!(
        element.props.get_symbol(NAMES.static_box),
        Some(rsui_core::PropValue::Bool(false))
    )
}

/// Whether something has what's placed by itself in it, that it passes on to what's around it.
pub(crate) fn holds_passed(child_box: &LayoutBox, child: &Element) -> bool {
    !child_box.has(IS_ABSOLUTE | IS_OVERLAY) && child_box.has(CONTAINS_ABSOLUTE) && !places_absolutes(child)
}

fn border_sides(element: &Element) -> [f32; 4] {
    if element.props.is_empty() {
        return [0.0; 4];
    }
    let number = |name: &str| match element.props.get(name) {
        Some(rsui_core::PropValue::Float(width)) => Some(*width as f32),
        Some(rsui_core::PropValue::Int(width)) => Some(*width as f32),
        _ => None,
    };
    let all = if element.props.get("border_color").is_some() {
        number("border_width").unwrap_or(0.0)
    } else {
        0.0
    };
    let side = |name: &str| number(name).unwrap_or(all).max(0.0);
    [
        side("border_top_width"),
        side("border_right_width"),
        side("border_bottom_width"),
        side("border_left_width"),
    ]
}

/// Whether text in this goes from right to left, as its `text_direction` says.
pub(crate) fn right_to_left(node: &Element) -> bool {
    static TEXT_DIRECTION: std::sync::LazyLock<rsui_core::Symbol> =
        std::sync::LazyLock::new(|| rsui_core::Symbol::new("text_direction"));
    !node.props.is_empty()
        && matches!(node.props.get_symbol(*TEXT_DIRECTION), Some(rsui_core::PropValue::Str(way)) if way.as_str() == "rtl")
}
