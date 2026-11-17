//! Where everything was put the last time each window was laid out, for code that asks while it runs, as a page's
//! `getBoundingClientRect()` and `elementFromPoint()` do. Each window says so once it's laid out, which only keeps a
//! reference to what it already has, and the tree it laid out, so that what's asked about is the same as what's shown.
//! Keeping that tree while the next one's made from it would have that copy what it changes, which costs, so it's only
//! kept then once something has said it may ask, with [`keep`], or has asked, and otherwise is let go of until it's been
//! laid out again.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use rsui_core::Node;
use rsui_layout::{Layout, Size};
use rsui_render::Images;
use rsui_text::TextEngine;

/// A window's layout, the tree it's of, if that's kept, and whether the window's been resized since, which leaves it where things
/// were in a window of another size: see [`resized`].
type Shown = (Weak<RefCell<Layout>>, Option<Node>, bool);

thread_local! {
    /// Each window's, the one laid out last at the end.
    static SHOWN: RefCell<Vec<Shown>> = const { RefCell::new(Vec::new()) };
    /// Whether anything has asked where something is.
    static ASKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// How big the window last given a size is, in logical pixels, for laying out a tree it hasn't laid out yet.
    static WINDOW: std::cell::Cell<(f32, f32)> = const { std::cell::Cell::new((0.0, 0.0)) };
    /// The tree as it is now, laid out for something that asked where something in it is before its window had: see [`ask_now`].
    static NOW: RefCell<Option<Now>> = const { RefCell::new(None) };
    /// A number that's another each time anything here may say something different about the same tree: see [`generation`].
    static GENERATION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Whether a window's been laid out since [`ask_now`] last laid a tree out.
    static SHOWN_SINCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A number that stays the same for as long as what's asked here about a tree can't change, other than by the tree changing:
/// it's another once a window's laid out, is about to change, or changes size, or a tree's been laid out to be asked about.
/// What keeps something it worked out from where things are can keep it for as long as this and the tree are the same.
pub fn generation() -> u64 {
    GENERATION.with(std::cell::Cell::get)
}

fn next_generation() {
    GENERATION.with(|generation| generation.set(generation.get().wrapping_add(1)));
}

/// A layout made for [`ask_now`], with what it measures with, which is kept for the next one.
struct Now {
    root: Option<Node>,
    size: (f32, f32),
    layout: Layout,
    text: Option<TextEngine>,
    images: Images,
}

/// Notes how big the window is, in logical pixels.
pub(crate) fn window_size(size: (f32, f32)) {
    if WINDOW.with(|window| window.replace(size)) != size {
        next_generation();
    }
}

/// Asks `what` about `root` laid out as it is now, in a window the size of the one last given a size, laying it out for this if
/// it hasn't been, as a browser lays out a page that's changed when it's asked where something in it is. That's for code that
/// asks before the window has laid out what it's just made, as an effect does; the window's own layout is what's asked once it's
/// there. It's laid out again only when the tree's different.
pub fn ask_now<R>(root: &Node, what: impl FnOnce(&Node, &Layout) -> Option<R>) -> Option<R> {
    let size = WINDOW.with(std::cell::Cell::get);
    if !(size.0 > 0.0 && size.1 > 0.0) {
        return None;
    }
    NOW.with(|now| {
        let mut now = now.borrow_mut();
        let same = now
            .as_ref()
            .is_some_and(|kept| kept.size == size && kept.root.as_ref().is_some_and(|kept| std::ptr::eq::<rsui_core::Element>(&**kept, &**root)));
        if !same {
            next_generation();
            let _owner = rsui_alloc::owners::scope(rsui_alloc::owners::GEOMETRY);
            let mut kept = now.take().unwrap_or_else(|| Now { root: None, size, layout: Layout::default(), text: None, images: Images::default() });
            if kept.text.is_none() {
                kept.text = crate::app::default_text_engine().ok();
            }
            // Only what's changed since the tree it last laid out is laid out again, as a window lays out only what its updates
            // changed, starting the first time from a copy of the window's own layout. An editor asks where things are after each
            // key, and a document of thousands of paragraphs laid out from nothing each time took longer than the rest of the key.
            // It starts from the window's own layout again once the window has laid out a tree since, other than the one kept here:
            // what's changed since then is what this frame changed, where what's changed since the tree kept here, from long before
            // when nothing asked, can be much of the page (a click in the Mantine kitchen sink laid out 1,200 boxes for
            // that, where the window's layout needed 20).
            let split = rsui_alloc::split::on().then(|| rsui_alloc::split::enter(rsui_alloc::split::named("geom:copy")));
            let kept_root = kept.root.take();
            let window_newer = SHOWN_SINCE.with(|since| since.replace(false))
                && kept_root.as_ref().is_none_or(|kept_root| !shown_is(kept_root));
            let copied = if kept_root.is_none() || window_newer { shown_layout() } else { None };
            let before = match copied {
                Some((layout, before)) => {
                    kept.layout = layout;
                    Some(before)
                }
                None => kept_root,
            };
            drop(split);
            let split = rsui_alloc::split::on().then(|| rsui_alloc::split::enter(rsui_alloc::split::named("geom:diff")));
            match before {
                Some(before) => kept.layout.apply(&rsui_core::diff(&before, root)),
                None => kept.layout = Layout::default(),
            }
            drop(split);
            let _split = rsui_alloc::split::on().then(|| rsui_alloc::split::enter(rsui_alloc::split::named("geom:layout")));
            let text = kept.text.as_mut()?;
            let mut measuring = crate::app::Measuring { text, images: &mut kept.images };
            kept.layout.compute(root, Size::new(size.0, size.1), &mut measuring);
            if rsui_alloc::split::on() {
                rsui_alloc::split::count(rsui_alloc::split::named("n:geomsolved"), kept.layout.last_solved() as u64);
            }
            kept.root = Some(root.clone());
            kept.size = size;
            *now = Some(kept);
        }
        let kept = now.as_ref()?;
        what(kept.root.as_ref()?, &kept.layout)
    })
}

/// Whether `root` is the tree of the window laid out last.
fn shown_is(root: &Node) -> bool {
    SHOWN.with(|shown| {
        shown
            .borrow()
            .last()
            .and_then(|(_, kept, _)| kept.as_ref())
            .is_some_and(|kept| std::ptr::eq::<rsui_core::Element>(&**kept, &**root))
    })
}

/// A copy of the layout of the window laid out last, and the tree it's of, if that's kept.
fn shown_layout() -> Option<(Layout, Node)> {
    SHOWN.with(|shown| {
        let shown = shown.borrow();
        let (layout, root, _) = shown.last()?;
        let layout = layout.upgrade()?;
        let layout = layout.try_borrow().ok()?;
        Some((layout.clone(), root.clone()?))
    })
}

/// Asks `what` about `root` as it's laid out, with something to measure its text with: the window's own layout if it's laid out
/// that tree, and one made for this as [`ask_now`] makes otherwise.
pub fn measure<R>(root: &Node, what: impl FnOnce(&Node, &Layout, &mut TextEngine) -> Option<R>) -> Option<R> {
    let mut what = Some(what);
    let shown = ask(|kept, layout| {
        if !std::ptr::eq::<rsui_core::Element>(&**kept, &**root) {
            return None;
        }
        let what = what.take()?;
        Some(with_text(|text| what(kept, layout, text)).flatten())
    });
    if let Some(found) = shown {
        return found;
    }
    let what = what?;
    // Laid out now, with what's kept for that measuring it.
    ask_now(root, |_, _| Some(()))?;
    NOW.with(|now| {
        let mut now = now.borrow_mut();
        let kept = now.as_mut()?;
        let root = kept.root.clone()?;
        let text = kept.text.as_mut()?;
        what(&root, &kept.layout, text)
    })
}

/// Runs `what` with the text engine that's kept for measuring what's asked about, made the first time it's wanted.
fn with_text<R>(what: impl FnOnce(&mut TextEngine) -> R) -> Option<R> {
    NOW.with(|now| {
        let mut now = now.try_borrow_mut().ok()?;
        let kept = now.get_or_insert_with(|| Now { root: None, size: (0.0, 0.0), layout: Layout::default(), text: None, images: Images::default() });
        if kept.text.is_none() {
            kept.text = crate::app::default_text_engine().ok();
        }
        Some(what(kept.text.as_mut()?))
    })
}

/// Notes that `layout` is now of `root`.
pub(crate) fn laid_out(layout: &Rc<RefCell<Layout>>, root: &Node) {
    publish(layout, root, true);
}

/// Publishes a tree whose existing layout is still current. An empty update can
/// reach this while paint is pending; it must not invalidate geometry queries
/// when neither the tree nor the boxes changed.
pub(crate) fn republish(layout: &Rc<RefCell<Layout>>, root: &Node) {
    publish(layout, root, false);
}

fn publish(layout: &Rc<RefCell<Layout>>, root: &Node, computed: bool) {
    // What was laid out for something that asked first is kept, though the window's laid that tree out now, so that the next
    // tree asked about before the window has it is laid out from it, by what's changed: see `ask_now`.
    SHOWN.with(|shown| {
        let mut shown = shown.borrow_mut();
        let own = Rc::downgrade(layout);
        let same = !computed && shown.last().is_some_and(|(one, kept, stale)| {
            one.ptr_eq(&own) && !stale && kept.as_ref().is_some_and(|kept| std::ptr::eq::<rsui_core::Element>(&**kept, &**root))
        });
        if same {
            return;
        }
        SHOWN_SINCE.with(|since| since.set(true));
        next_generation();
        shown.retain(|(one, _, _)| one.strong_count() > 0 && !one.ptr_eq(&own));
        shown.push((own, Some(root.clone()), false));
    });
}

/// Notes that the window `layout` is of has changed size, so that until it's laid out again what's asked is laid out at the new
/// size, as [`ask_now`] does, as a browser lays out a resized page again when its `resize` event asks where something is. The old
/// layout had floating-ui put Mantine's dropdowns where their buttons had been in the smaller window.
pub(crate) fn resized(layout: &Rc<RefCell<Layout>>) {
    SHOWN.with(|shown| {
        let own = Rc::downgrade(layout);
        for (one, _, stale) in shown.borrow_mut().iter_mut() {
            if one.ptr_eq(&own) {
                *stale = true;
            }
        }
    });
}

/// Says that something may ask where things are while the tree's changing, as an effect does, which keeps each window's tree
/// from then on.
pub fn keep() {
    ASKED.with(|asked| asked.set(true));
}

/// Notes that the tree `layout` is of is about to change, which lets go of it, unless something has asked where things are.
pub(crate) fn changing(layout: &Rc<RefCell<Layout>>) {
    // Merely starting a flush does not change geometry. The tree identity tells
    // queries about actual tree changes during effects; publishing changed boxes
    // or changing the window size advances the geometry generation separately.
    if ASKED.with(std::cell::Cell::get) {
        return;
    }
    SHOWN.with(|shown| {
        let own = Rc::downgrade(layout);
        for (one, root, _) in shown.borrow_mut().iter_mut() {
            if one.ptr_eq(&own) {
                *root = None;
            }
        }
    });
}

/// Asks `what` about each window's tree and where everything in it is, the one laid out last first, until it says something.
/// A window that's being laid out right now isn't asked. One that's changed size since it was laid out has its tree laid out
/// again at the new size for this, as [`ask_now`] does.
pub fn ask<R>(mut what: impl FnMut(&Node, &Layout) -> Option<R>) -> Option<R> {
    ASKED.with(|asked| asked.set(true));
    let windows: Vec<_> = SHOWN.with(|shown| shown.borrow().iter().rev().cloned().collect());
    for (layout, root, stale) in windows {
        let Some(root) = root else {
            continue;
        };
        if stale {
            if let Some(found) = ask_now(&root, &mut what) {
                return Some(found);
            }
            continue;
        }
        let Some(layout) = layout.upgrade() else {
            continue;
        };
        let Ok(layout) = layout.try_borrow() else {
            continue;
        };
        if let Some(found) = what(&root, &layout) {
            return Some(found);
        }
    }
    None
}

/// What a pointer at this place in the window (in CSS pixels) lands on, as the window has it: through transforms, and short of
/// what's `pointer-events: none`, as `document.elementFromPoint()` says.
pub fn pointer_path(root: &Node, layout: &Layout, x: f32, y: f32) -> Option<Vec<u32>> {
    crate::app::pointer_path(root, layout, x, y, true).map(|(path, _)| path)
}
