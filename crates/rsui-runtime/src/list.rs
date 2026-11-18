use rsui_core::{ActionKind, Element, Key, Node, PropValue, Props, Role, symbol};

use crate::cx::{Cx, Updates};

/// Which part of a long list to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListWindow {
    /// How many rows the whole list has.
    pub len: usize,
    /// How many rows to expect to fit, until something measures it. A window measures this as
    /// soon as it lays the list out, so this only has to be a reasonable guess there.
    pub visible: usize,
    /// How many extra rows to build on each side, so scrolling can show them at once.
    pub overscan: usize,
    /// A row to bring into view whenever this changes, as when the keyboard moves through a list. Scrolling
    /// stays free otherwise, as this only does something when it's a different row from the last time.
    pub reveal: Option<usize>,
}

/// What [`Cx::virtual_list_with`] is told beyond which part of the list to build.
#[derive(Clone, Default)]
pub struct ListOptions {
    /// Makes it a grid, whose children are items rather than rows, each this wide: each row holds as many as fit across it, which
    /// the window measures as it measures how many rows fit. The list's `gap` is between them, across and down. The window's
    /// `len` is then how many items there are, its `reveal` is an item, and what `row` is given is an item's index.
    pub item_width: Option<f64>,
    /// Told how many rows fit, including part of one, and how many items fit across (1 for a list that isn't a grid), each time
    /// the window measures something different, starting with the first time it's laid out.
    pub fits: Option<std::rc::Rc<dyn Fn(&mut Updates<'_>, f64, usize)>>,
}

impl std::fmt::Debug for ListOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ListOptions")
            .field("item_width", &self.item_width)
            .field("fits", &self.fits.is_some())
            .finish()
    }
}

/// What [`Cx::scroll_area_with`] is told beyond what a scrolling area does by itself: where code wants it to go, and what hears
/// that it's moved.
#[derive(Clone, Default)]
pub struct ScrollControl {
    /// Where to go, in pixels down and across, each only if it's said, whenever `request` is one it hasn't seen, as when a
    /// page's code sets `scrollTop`. Scrolling stays free otherwise.
    pub request: u64,
    pub y: Option<f64>,
    pub x: Option<f64>,
    /// Told where it is, down and across, each time it's scrolled, by a person or by anything that does what they'd do.
    pub moved: Option<std::rc::Rc<dyn Fn(&mut Updates<'_>, f64, f64)>>,
}

impl std::fmt::Debug for ScrollControl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScrollControl")
            .field("request", &self.request)
            .field("y", &self.y)
            .field("x", &self.x)
            .field("moved", &self.moved.is_some())
            .finish()
    }
}

/// Which way a scrolling area goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollAxis {
    #[default]
    Vertical,
    Horizontal,
    /// Both ways, for something bigger than its area in both directions, like a large picture.
    Both,
}

impl Cx<'_> {
    /// Makes `content` scroll: whatever doesn't fit in it can be reached with the wheel, the keyboard
    /// or its scrollbar, and is cut off at its edges. Give it a size to be, usually with `flex`,
    /// or it grows to fit what's inside and has nothing to scroll.
    ///
    /// Where it's scrolled to is in the `scroll_y` prop, or `scroll_x`, so anyone reading the tree
    /// can see it, and its `scroll` action takes where to go in pixels. With [`ScrollAxis::Both`],
    /// that's for going down, and its `scroll_across` action is for going across. Scrolling renders
    /// this component again, so keep what's expensive inside child components, which only render
    /// again if their props change.
    pub fn scroll_area(
        &mut self,
        key: impl Into<Key>,
        axis: ScrollAxis,
        content: Element,
    ) -> Element {
        self.scroll_area_with(key, axis, content, ScrollControl::default())
    }

    /// [`scroll_area`](Self::scroll_area), which goes where `control` says when it says something new, and tells it when it
    /// moves.
    pub fn scroll_area_with(
        &mut self,
        key: impl Into<Key>,
        axis: ScrollAxis,
        content: Element,
        control: ScrollControl,
    ) -> Element {
        // Both are always there, as a component has to use the same hooks on every render.
        let offset = self.state(|| 0f64);
        let across = self.state(|| 0f64);
        let seen = self.state(|| 0u64);
        if control.request != self.get(seen) {
            self.set(seen, control.request);
            let (down, along) = match axis {
                ScrollAxis::Horizontal => (control.x, None),
                ScrollAxis::Vertical => (control.y, None),
                ScrollAxis::Both => (control.y, control.x),
            };
            if let Some(down) = down.filter(|pixels| pixels.is_finite()) {
                self.set(offset, down.max(0.0));
            }
            if let Some(along) = along.filter(|pixels| pixels.is_finite()) {
                self.set(across, along.max(0.0));
            }
        }
        let moved = control.moved;
        let both = axis == ScrollAxis::Both;
        let horizontal = axis == ScrollAxis::Horizontal;
        let name = match axis {
            ScrollAxis::Vertical | ScrollAxis::Both => symbol!("scroll_y"),
            ScrollAxis::Horizontal => symbol!("scroll_x"),
        };
        let mut element = content
            .key(key)
            .prop(name, PropValue::Float(self.get(offset)));
        // What's told it moved hears where it is by then, down and across, once the frame that shows it has been drawn
        // (`Runtime::deliver_scrolled`), and once however often it moved before that, as a browser fires `scroll`.
        let tell = move |cx: &mut Updates<'_>, moved: &Option<std::rc::Rc<dyn Fn(&mut Updates<'_>, f64, f64)>>| {
            if let Some(moved) = moved.clone() {
                cx.tell_scrolled_later(
                    offset,
                    std::rc::Rc::new(move |cx: &mut Updates<'_>| {
                        // What's gone by then has nothing to say.
                        let (Some(down), Some(along)) = (cx.get_if_mounted(offset), cx.get_if_mounted(across)) else {
                            return;
                        };
                        if horizontal {
                            moved(cx, 0.0, down);
                        } else {
                            moved(cx, down, if both { along } else { 0.0 });
                        }
                    }),
                );
            }
        };
        let read = |input: Option<&PropValue>| match input {
            Some(PropValue::Float(pixels)) if pixels.is_finite() => Some(pixels.max(0.0)),
            Some(PropValue::Int(pixels)) => Some((*pixels as f64).max(0.0)),
            _ => None,
        };
        if both {
            element = element.prop(symbol!("scroll_x"), PropValue::Float(self.get(across)));
            let moved = moved.clone();
            element = self.on(
                element,
                ActionKind::Custom(symbol!("scroll_across")),
                move |cx, input| match read(input) {
                    Some(pixels) => {
                        let was = cx.get(across);
                        cx.set(across, pixels);
                        if was != pixels {
                            tell(cx, &moved);
                        }
                    }
                    None => cx.reject("scroll_across needs how far to go, in pixels"),
                },
            );
        }
        self.on(element, ActionKind::Scroll, move |cx, input| match read(input) {
            Some(pixels) => {
                let was = cx.get(offset);
                cx.set(offset, pixels);
                if was != pixels {
                    tell(cx, &moved);
                }
            }
            None => cx.reject("scroll needs how far to go, in pixels"),
        })
    }

    /// Builds a list element holding only the rows in view, so a long list costs
    /// what a short one does. Rows outside the window aren't mounted, and lose their
    /// state until they come back.
    ///
    /// Since off-screen rows aren't in the tree, the element tells anyone reading it
    /// what's missing, through its props: `count` is the length of the whole list,
    /// `first_visible` is the row at the top of the view, and `first_built` is the
    /// row its first child represents. Its `scroll` action takes a row index to
    /// bring to the top of the view, and its `resize` action takes how many rows fit,
    /// which is in `view_rows`. While it's gliding, `first_visible` and what `scroll` takes
    /// can have part of a row in them.
    ///
    /// Give `row` a [`keyed`](Cx::keyed) child, so that rows staying in view keep
    /// their state and don't render again as the list scrolls.
    pub fn virtual_list(
        &mut self,
        key: impl Into<Key>,
        window: ListWindow,
        row: impl FnMut(&mut Cx<'_>, usize) -> Node,
    ) -> Element {
        self.virtual_list_with(key, window, ListOptions::default(), row)
    }

    /// [`virtual_list`](Self::virtual_list), which can be a grid, and can tell what wants to know how much fits in it, as
    /// `options` says. A grid's `count` is how many rows it has, and its `across` how many items each holds, which its
    /// `resize` action takes after how many rows fit, as `"12.5 4"`.
    pub fn virtual_list_with(
        &mut self,
        key: impl Into<Key>,
        window: ListWindow,
        options: ListOptions,
        mut row: impl FnMut(&mut Cx<'_>, usize) -> Node,
    ) -> Element {
        let offset = self.state(|| 0f64);
        let measured = self.state(|| None::<f64>);
        let revealed = self.state(|| None::<usize>);
        let measured_across = self.state(|| None::<usize>);
        let grid = options.item_width.filter(|width| width.is_finite() && *width > 0.0);
        // Until it's measured, a grid is one item across, and the window always says how many fit then.
        let across = if grid.is_some() { self.get(measured_across).unwrap_or(1).max(1) } else { 1 };
        let items = window.len;
        let rows_len = items.div_ceil(across);
        // What's revealed is an item, which is in the row that holds it.
        let reveal_item = window.reveal;
        let window = ListWindow {
            len: rows_len,
            reveal: window.reveal.map(|item| item / across),
            ..window
        };
        let view_rows = self.get(measured).unwrap_or(window.visible as f64);
        // The end of the list only needs whole rows to show, so its last row is never cut off,
        // while every row that shows at all has to be built, even if only part of it fits.
        let whole_rows = (view_rows.floor() as usize).max(1);
        let any_rows = view_rows.ceil() as usize;
        let last_top = window.len.saturating_sub(whole_rows);
        if reveal_item != self.get(revealed) {
            self.set(revealed, reveal_item);
            if let Some(row) = window.reveal {
                let top = self.get(offset);
                if (row as f64) < top {
                    self.set(offset, row as f64);
                } else if (row + 1) as f64 > top + view_rows.floor().max(1.0) {
                    self.set(offset, (row + 1 - whole_rows) as f64);
                }
            }
        }
        let position = self.get(offset).min(last_top as f64);
        let first_visible = position as usize;
        let partway = position.fract() != 0.0;
        let first_built = first_visible.saturating_sub(window.overscan);
        let end = first_visible
            .saturating_add(any_rows)
            .saturating_add(usize::from(partway))
            .saturating_add(window.overscan)
            .min(window.len);

        let rows: Vec<Node> = if grid.is_some() {
            // A grid's rows are its items, a row of them at a time.
            (first_built * across..(end * across).min(items)).map(|index| row(self, index)).collect()
        } else {
            (first_built..end).map(|index| row(self, index)).collect()
        };
        // What wants to be told how much fits is told the first time too, so until then the window's told it's not been measured.
        let told_rows = match self.get(measured) {
            Some(rows) => rows,
            None if options.fits.is_some() => -1.0,
            None => view_rows,
        };
        let mut props = Props::from([
            (symbol!("count"), PropValue::Int(window.len as i64)),
            (
                symbol!("first_visible"),
                if partway {
                    PropValue::Float(position)
                } else {
                    PropValue::Int(first_visible as i64)
                },
            ),
            (symbol!("first_built"), PropValue::Int(first_built as i64)),
            (symbol!("view_rows"), PropValue::Float(told_rows)),
        ]);
        if let Some(width) = grid {
            props.insert(symbol!("item_width"), PropValue::Float(width));
            props.insert(symbol!("across"), PropValue::Int(self.get(measured_across).map_or(0, |across| across as i64)));
        }
        let element = Element::new(Role::List).key(key).props(props).children(rows);
        let fits = options.fits;
        let element = self.on(element, ActionKind::Resize, move |cx, input| {
            let read = |text: &str| -> Option<f64> { text.parse::<f64>().ok().filter(|rows| rows.is_finite() && *rows >= 0.0) };
            let (rows, across) = match input {
                Some(PropValue::Float(rows)) if rows.is_finite() && *rows >= 0.0 => (*rows, None),
                Some(PropValue::Int(rows)) if *rows >= 0 => (*rows as f64, None),
                // A grid's says how many fit across too.
                Some(PropValue::Str(text)) => {
                    let mut words = text.split(' ');
                    match (words.next().and_then(read), words.next().and_then(read), words.next()) {
                        (Some(rows), Some(across), None) if across >= 1.0 => (rows, Some(across as usize)),
                        _ => return cx.reject("resize needs how many rows fit, and for a grid how many fit across"),
                    }
                }
                _ => return cx.reject("resize needs how many rows fit, which isn't negative"),
            };
            let was = (cx.get(measured), cx.get(measured_across));
            cx.set(measured, Some(rows));
            if let Some(now) = across {
                cx.set(measured_across, Some(now));
                // A grid that wraps again keeps the item that was at its top in view: it's in a different row now.
                let before = was.1.unwrap_or(1).max(1);
                if now != before {
                    let top = cx.get(offset);
                    cx.set(offset, (top * before as f64 / now as f64).floor());
                }
            }
            let now = (Some(rows), across.or(was.1));
            if let Some(fits) = &fits
                && now != was
            {
                fits(cx, rows, if grid.is_some() { now.1.unwrap_or(1) } else { 1 });
            }
        });
        self.on(element, ActionKind::Scroll, move |cx, input| match input {
            Some(PropValue::Int(row)) if *row >= 0 => {
                cx.set(offset, (*row as f64).min(last_top as f64));
            }
            Some(PropValue::Float(row)) if row.is_finite() && *row >= 0.0 => {
                cx.set(offset, row.min(last_top as f64));
            }
            _ => cx.reject("scroll needs a row index that isn't negative"),
        })
    }
}
