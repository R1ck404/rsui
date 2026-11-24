//! Another window kept where an element is, as a browser view inside a page is: something drawn by someone else, such as
//! the system's web view, that has to be a window of its own. The element says which with an `embed` prop, a number, and
//! what's registered for that number with [`embed`] is told where the element is each time the window it's in is drawn:
//! its box, what of that shows past whatever scrolls or cuts it off around it, and the parts of that something drawn on top
//! of it covers, as a window of its own can't be drawn over. That's in physical pixels, from the top left of the window.
//! Once the element has gone, it's told to hide.
//!
//! Nothing here runs until something's been registered, so a window with none pays nothing for it.

use std::cell::RefCell;

use rsui_core::{Element, Node, PropValue, Role, symbol};
use rsui_layout::{Layout, LayoutVisitor, Rect, ScrollInfo};

/// A rectangle in physical pixels, from the top left of the window, with its right and bottom edges outside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PixelBox {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PixelBox {
    fn of(rect: Rect, scale: f32) -> Self {
        Self {
            left: (rect.x * scale).round() as i32,
            top: (rect.y * scale).round() as i32,
            right: ((rect.x + rect.width) * scale).round() as i32,
            bottom: ((rect.y + rect.height) * scale).round() as i32,
        }
    }

    const EVERYWHERE: Self = Self {
        left: i32::MIN,
        top: i32::MIN,
        right: i32::MAX,
        bottom: i32::MAX,
    };

    pub fn width(&self) -> i32 {
        (self.right - self.left).max(0)
    }

    pub fn height(&self) -> i32 {
        (self.bottom - self.top).max(0)
    }

    pub fn is_empty(&self) -> bool {
        self.width() == 0 || self.height() == 0
    }

    pub fn intersect(&self, other: &Self) -> Self {
        Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        }
    }
}

/// Where an embedded window goes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Place {
    /// The element's whole box, which is where what's in the window is laid out.
    pub rect: PixelBox,
    /// What of that can be seen, past what scrolls or cuts it off around it. Never empty: an element that can't be seen at
    /// all has no place.
    pub shown: PixelBox,
    /// The parts of `shown` that something drawn after it covers, each inside `shown`, which the window mustn't show
    /// through.
    pub covered: Vec<PixelBox>,
    /// The parts of `shown` that something see-through drawn after it is over, as a dimmed backdrop is, each inside `shown`.
    /// A window can't be drawn over, so what's kept there shows a still picture of itself, which is drawn under them, as
    /// `still` says.
    pub see_through: Vec<PixelBox>,
    /// Whether the element draws a still picture of what's kept there, as its `embed_still` prop says, so that it can be cut
    /// where it's seen through.
    pub still: bool,
    /// How round the element's corners are, in physical pixels, which the window's are cut to as well.
    pub radius: i32,
}

/// What's kept where an element is.
pub trait Embedded {
    /// Where it goes now in `window`, a window handle, or `None` to hide it, as its element has gone or can't be seen. It's
    /// only told when that's changed.
    fn place(&mut self, window: isize, place: Option<&Place>);
    /// The window it's in has moved on the screen, which anything it shows outside itself, as a dropdown, has to follow.
    fn window_moved(&mut self) {}
}

struct Entry {
    id: u64,
    what: Box<dyn Embedded>,
    /// Which window it was last put in, and where, if it's showing.
    shown: Option<(isize, Place)>,
}

thread_local! {
    static EMBEDDED: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
}

/// Keeps `what` where the element with `embed={id}` is, from the next time a window with it is drawn. Anything already kept
/// for `id` is let go of.
pub fn embed(id: u64, what: Box<dyn Embedded>) {
    let before = unembed(id);
    EMBEDDED.with(|all| {
        all.borrow_mut().push(Entry {
            id,
            what,
            shown: None,
        });
    });
    drop(before);
}

/// Stops keeping what's kept for `id`, and gives it back, if there was something.
pub fn unembed(id: u64) -> Option<Box<dyn Embedded>> {
    let taken = EMBEDDED.with(|all| {
        let mut all = all.borrow_mut();
        let at = all.iter().position(|entry| entry.id == id)?;
        Some(all.remove(at))
    })?;
    let mut what = taken.what;
    if let Some((window, _)) = taken.shown {
        what.place(window, None);
    }
    Some(what)
}

/// Whether anything's kept anywhere.
pub fn any() -> bool {
    EMBEDDED.with(|all| !all.borrow().is_empty())
}

/// Does `run` with what's registered taken out, so that what each is told can register or let go of others.
#[cfg_attr(not(windows), allow(dead_code))]
fn with_entries(run: impl FnOnce(&mut Vec<Entry>)) {
    let mut taken = EMBEDDED.with(|all| std::mem::take(&mut *all.borrow_mut()));
    run(&mut taken);
    EMBEDDED.with(|all| {
        let mut all = all.borrow_mut();
        // What was registered meanwhile replaces what was there for the same number.
        taken.retain(|entry| !all.iter().any(|newer| newer.id == entry.id));
        let newer = std::mem::take(&mut *all);
        *all = taken;
        all.extend(newer);
    });
}

/// Tells everything kept in `window` where it is now, from `places`, and hides what's in it that isn't there any more.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn place_in(window: isize, places: Vec<(u64, Place)>) {
    with_entries(|entries| {
        for entry in entries.iter_mut() {
            let wanted = places.iter().find(|(id, _)| *id == entry.id).map(|(_, place)| place);
            let there = entry.shown.as_ref().map(|(there, _)| *there);
            match wanted {
                Some(place) if entry.shown.as_ref().is_some_and(|(there, was)| *there == window && was == place) => {}
                Some(place) => {
                    // From another window, it has to leave that one first.
                    if let Some(there) = there.filter(|there| *there != window) {
                        entry.what.place(there, None);
                    }
                    entry.what.place(window, Some(place));
                    entry.shown = Some((window, place.clone()));
                }
                None if there == Some(window) => {
                    entry.what.place(window, None);
                    entry.shown = None;
                }
                None => {}
            }
        }
    });
}

/// Tells what's showing in `window` that the window has moved.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn window_moved(window: isize) {
    with_entries(|entries| {
        for entry in entries.iter_mut().filter(|entry| entry.shown.as_ref().is_some_and(|(there, _)| *there == window)) {
            entry.what.window_moved();
        }
    });
}

/// Forgets where what was in `window` was, as it's gone, and so has everything inside it.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn window_gone(window: isize) {
    with_entries(|entries| {
        for entry in entries.iter_mut() {
            if entry.shown.as_ref().is_some_and(|(there, _)| *there == window) {
                entry.shown = None;
            }
        }
    });
}

/// Where each element with an `embed` number is in `root`, laid out by `layout`, at `scale`, in a window `size` big. What
/// can't be seen at all isn't there.
pub(crate) fn places(root: &Node, layout: &Layout, scale: f32, size: (u32, u32)) -> Vec<(u64, Place)> {
    let window = PixelBox {
        left: 0,
        top: 0,
        right: size.0 as i32,
        bottom: size.1 as i32,
    };
    let mut finder = Finder {
        scale,
        clips: vec![window],
        found: Vec::new(),
        inside: Vec::new(),
        unseen: 0,
        depth: 0,
    };
    layout.visit(root, &mut finder);
    finder
        .found
        .into_iter()
        .filter_map(|found| {
            let shown = found.rect.intersect(&found.clip);
            (!shown.is_empty()).then(|| {
                let inside = |covers: Vec<PixelBox>| -> Vec<PixelBox> {
                    covers.iter().map(|cover| cover.intersect(&shown)).filter(|cover| !cover.is_empty()).collect()
                };
                (
                    found.id,
                    Place {
                        rect: found.rect,
                        shown,
                        covered: inside(found.covered),
                        see_through: inside(found.see_through),
                        still: found.still,
                        radius: found.radius,
                    },
                )
            })
        })
        .collect()
}

struct Found {
    id: u64,
    rect: PixelBox,
    clip: PixelBox,
    covered: Vec<PixelBox>,
    see_through: Vec<PixelBox>,
    still: bool,
    radius: i32,
}

/// Goes through what's drawn, in the order it's drawn, for where embedded windows are and what's drawn over them.
struct Finder {
    scale: f32,
    /// What's being cut off at, the window's edges first.
    clips: Vec<PixelBox>,
    found: Vec<Found>,
    /// How deep each embedded element that's being gone through is, as what's in it is its own, and isn't drawn over it.
    inside: Vec<usize>,
    /// How many of what's being gone through can't be seen, as they're entirely see-through.
    unseen: u32,
    depth: usize,
}

impl Finder {
    fn clip(&self) -> PixelBox {
        *self.clips.last().unwrap_or(&PixelBox::EVERYWHERE)
    }

    fn push_clip(&mut self, rect: Rect) {
        let clip = self.clip().intersect(&PixelBox::of(rect, self.scale));
        self.clips.push(clip);
    }
}

/// The embedded window an element says it has, if it does.
fn embed_of(element: &Element) -> Option<u64> {
    if element.props.is_empty() {
        return None;
    }
    match element.props.get_symbol(symbol!("embed")) {
        Some(PropValue::Int(id)) if *id > 0 => Some(*id as u64),
        Some(PropValue::Float(id)) if *id >= 1.0 => Some(*id as u64),
        _ => None,
    }
}

/// Whether an element is entirely see-through, along with what's in it.
fn see_through(element: &Element) -> bool {
    !element.props.is_empty()
        && match element.props.get_symbol(symbol!("opacity")) {
            Some(PropValue::Float(opacity)) => *opacity <= 0.0,
            Some(PropValue::Int(opacity)) => *opacity <= 0,
            _ => false,
        }
}

/// How something drawn on top of an embedded window covers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cover {
    /// Nothing of the window shows through it.
    Solid,
    /// The window would show through it, as through a dimmed backdrop.
    SeeThrough,
}

/// Whether an element draws something of its own that a window under it would hide, and how.
fn cover_of(element: &Element) -> Option<Cover> {
    if element.role == Role::Image {
        return Some(Cover::Solid);
    }
    if element.props.is_empty() {
        return None;
    }
    let props = &element.props;
    // A color as a number, with alpha in its lowest byte as `Color::to_prop` has it, of which none means see-through.
    let background = match props.get_symbol(symbol!("background")) {
        Some(PropValue::Int(color)) if color & 0xFF == 0 => None,
        Some(PropValue::Int(color)) if color & 0xFF < 0xFF => Some(Cover::SeeThrough),
        Some(PropValue::Str(text)) if text.is_empty() => None,
        Some(_) => Some(Cover::Solid),
        None => None,
    };
    let solid = props.get_symbol(symbol!("background_gradient")).is_some()
        || props.get_symbol(symbol!("background_image")).is_some()
        || matches!(props.get_symbol(symbol!("border_width")), Some(PropValue::Float(width)) if *width > 0.0)
        || matches!(props.get_symbol(symbol!("border_width")), Some(PropValue::Int(width)) if *width > 0);
    let cover = if solid { Some(Cover::Solid) } else { background }?;
    // What's faded is seen through, whatever it draws.
    let faded = matches!(props.get_symbol(symbol!("opacity")), Some(PropValue::Float(opacity)) if *opacity < 1.0);
    Some(if faded { Cover::SeeThrough } else { cover })
}

impl LayoutVisitor for Finder {
    fn enter(&mut self, element: &Element, rect: Rect) {
        self.depth += 1;
        if see_through(element) || self.unseen > 0 {
            self.unseen += 1;
            return;
        }
        if let Some(id) = embed_of(element) {
            let radius = match element.props.get_symbol(symbol!("corner_radius")) {
                Some(PropValue::Float(radius)) => *radius as f32,
                Some(PropValue::Int(radius)) => *radius as f32,
                _ => 0.0,
            };
            self.found.push(Found {
                id,
                rect: PixelBox::of(rect, self.scale),
                clip: self.clip(),
                covered: Vec::new(),
                see_through: Vec::new(),
                still: matches!(element.props.get_symbol(symbol!("embed_still")), Some(PropValue::Bool(true))),
                radius: (radius.max(0.0) * self.scale).round() as i32,
            });
            self.inside.push(self.depth);
            return;
        }
        if self.found.is_empty() {
            return;
        }
        let Some(kind) = cover_of(element) else {
            return;
        };
        // Drawn after an embedded window that it isn't part of, so it's on top of it.
        let cover = PixelBox::of(rect, self.scale).intersect(&self.clip());
        if cover.is_empty() {
            return;
        }
        let depth = self.depth;
        let inside = &self.inside;
        for (at, found) in self.found.iter_mut().enumerate() {
            let own = inside.get(at).is_some_and(|&from| from > 0 && from < depth);
            if !own {
                match kind {
                    Cover::Solid => found.covered.push(cover),
                    Cover::SeeThrough => found.see_through.push(cover),
                }
            }
        }
    }

    fn enter_scrolled(&mut self, rect: Rect, _element: &Element) {
        self.push_clip(rect);
    }

    fn exit_scrolled(&mut self, _rect: Rect, _info: ScrollInfo, _element: &Element) {
        self.clips.pop();
    }

    fn clip_to(&mut self, rect: Rect, _element: &Element) {
        self.push_clip(rect);
    }

    fn unclip(&mut self) {
        self.clips.pop();
    }

    fn exit(&mut self, _element: &Element) {
        if self.unseen > 0 {
            self.unseen -= 1;
        }
        // Leaving an embedded element: what comes after it draws over it. Its place in `inside` stays, as 0, so that each
        // found one keeps its own.
        if let Some(at) = self.inside.iter().rposition(|&from| from == self.depth) {
            self.inside[at] = 0;
        }
        self.depth -= 1;
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;

    /// Says what it's told.
    struct Told(Rc<RefCell<Vec<(isize, Option<Place>)>>>);

    impl Embedded for Told {
        fn place(&mut self, window: isize, place: Option<&Place>) {
            self.0.borrow_mut().push((window, place.cloned()));
        }
    }

    fn at(left: i32) -> Place {
        let rect = PixelBox { left, top: 0, right: left + 10, bottom: 10 };
        Place { rect, shown: rect, covered: Vec::new(), see_through: Vec::new(), still: false, radius: 0 }
    }

    #[test]
    fn it_is_only_told_what_has_changed_and_hidden_once_its_element_has_gone() {
        let told = Rc::new(RefCell::new(Vec::new()));
        embed(3, Box::new(Told(told.clone())));
        place_in(1, vec![(3, at(0))]);
        place_in(1, vec![(3, at(0))]);
        place_in(1, vec![(3, at(5))]);
        // Another window with nothing of it leaves it alone.
        place_in(2, Vec::new());
        place_in(1, Vec::new());
        place_in(1, Vec::new());
        assert_eq!(*told.borrow(), vec![(1, Some(at(0))), (1, Some(at(5))), (1, None)]);
        told.borrow_mut().clear();
        // Going to another window leaves the first.
        place_in(1, vec![(3, at(0))]);
        place_in(2, vec![(3, at(0))]);
        assert_eq!(*told.borrow(), vec![(1, Some(at(0))), (1, None), (2, Some(at(0)))]);
        told.borrow_mut().clear();
        // Letting go of it hides it where it was.
        assert!(unembed(3).is_some());
        assert_eq!(*told.borrow(), vec![(2, None)]);
        assert!(!any());
    }
}
