use std::cell::Cell;
use std::sync::LazyLock;

use rsui_core::{PropValue, Props, Symbol};

thread_local! {
    /// How wide the box that what's being read is in is, inside its padding, while it lays out what's in it: what a margin or
    /// padding that's a percentage is a share of, as CSS has it (of the width, even for top and bottom). Not a number where
    /// that isn't known, as when something is only asked how wide it'd like to be, where such a share is nothing.
    pub(crate) static PERCENT_BASE: Cell<f32> = const { Cell::new(f32::NAN) };
    /// How tall what shows of the scroller being laid out is, inside its padding, when that's known: what a height that's a
    /// percentage is a share of for what's in it, as CSS has a scroll container's content box (its content can be taller). Not
    /// a number while that isn't known, as when the scroller is as tall as what's in it.
    pub(crate) static VIEW_HEIGHT: Cell<f32> = const { Cell::new(f32::NAN) };
}

/// A share of what it's in and some pixels more, as a margin or padding that's a percentage says it, as `"25%"` or
/// `"50% + -5px"`, in pixels, given how wide what it's in is.
fn share_in_pixels(said: &str) -> f32 {
    let Some((share, more)) = share_and_more(said) else {
        return 0.0;
    };
    let base = PERCENT_BASE.with(Cell::get);
    if base.is_finite() { base * share / 100.0 + more } else { more }
}

/// Which way an element stacks its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Column,
    Row,
}

/// Where children go across the direction they stack in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    /// Fill the space, unless the child has its own size in that direction.
    #[default]
    Stretch,
    Start,
    Center,
    End,
    /// With the first line of text in each lined up, as what's on a line of a page is. Something with no text in it lines up its
    /// bottom edge instead. Only children in a row that wraps are lined up like this, and anywhere else it's the same as `Start`.
    Baseline,
}

/// Where children go along the direction they stack in, if space is left over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Justify {
    #[default]
    Start,
    Center,
    End,
    /// The first and last child at the ends, and equal space between them all.
    SpaceBetween,
    /// Equal space on both sides of each child, so there's half as much at the ends as between them.
    SpaceAround,
    /// Equal space between them all and at the ends.
    SpaceEvenly,
}

/// A size that may depend on the parent's size.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Length {
    /// Fit the content.
    #[default]
    Auto,
    Px(f32),
    /// A percentage of the parent's size, if the parent's size doesn't itself depend
    /// on its content. Otherwise this behaves like [`Auto`](Self::Auto).
    Percent(f32),
    /// A percentage of the parent's size and some pixels more, as CSS's `calc(100% - 14px)` is. As a prop it's written
    /// `"100% + -14px"`. Like [`Percent`](Self::Percent), it needs the parent's size.
    Calc(f32, f32),
}

/// How an element lays out. Convert it to [`Props`] for an element, usually with the
/// [`style!`](crate::style!) macro.
///
/// Only settings that differ from the default become props, so a default style costs
/// nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub(crate) direction: Direction,
    pub(crate) width: Length,
    pub(crate) height: Length,
    pub(crate) min_width: f32,
    pub(crate) max_width: f32,
    /// What `max_width` is as a share of what it's in, and some pixels more, which is worked out when that's known, as CSS's
    /// `max-width: 75%` says.
    pub(crate) max_width_percent: Option<(f32, f32)>,
    /// The same for `min_width`, as CSS's `min-width: 100%` says.
    pub(crate) min_width_percent: Option<(f32, f32)>,
    /// Whether `basis` is a share of what it's in (1.0 for all of it) rather than pixels. Where how long that is isn't known yet,
    /// it's as big as what's in it, as CSS has it.
    pub(crate) basis_share: bool,
    /// Pixels on top of a `basis` that's a share, as `flex-basis: calc(25% - 4px)` has.
    pub(crate) basis_more: f32,
    pub(crate) min_height: f32,
    pub(crate) max_height: f32,
    pub(crate) grow: f32,
    pub(crate) shrink: f32,
    /// The size to start from before sharing out space, instead of the element's own.
    pub(crate) basis: Option<f32>,
    pub(crate) gap: f32,
    pub(crate) padding_top: f32,
    pub(crate) padding_right: f32,
    pub(crate) padding_bottom: f32,
    pub(crate) padding_left: f32,
    pub(crate) align: Align,
    pub(crate) align_self: Option<Align>,
    pub(crate) justify: Justify,
    /// Marks a virtual list, whose rows all have this height.
    pub(crate) row_height: Option<f32>,
    /// Marks an area whose content can be wider than it is, and how far that's moved.
    pub(crate) scroll_x: Option<f32>,
    /// Marks an area whose content can be taller than it is, and how far that's moved.
    pub(crate) scroll_y: Option<f32>,
    /// Takes this element out of its parent's way, and shows it on top of everything.
    pub(crate) overlay: Option<Placement>,
    /// Its `Placement::At` is a page's `position: fixed`, which is where it says even past the window's edges, rather than a popup
    /// of rsui's own, which is kept in the window: said as `overlay: "fixed"`.
    pub(crate) overlay_fixed: bool,
    /// It has a margin, which is read when it's wanted, as hardly anything does, and this is built and copied for everything.
    pub(crate) has_margin: bool,
    /// Takes this element out of its parent's way, and places it by its `top`, `left`, `right` and `bottom`, in its parent.
    pub(crate) absolute: bool,
    /// Lays out children in rows and columns, as its `columns` and `rows` say.
    pub(crate) grid: bool,
    /// Moves this element from where it would be, as its `translate_x` and `translate_y` say.
    pub(crate) moved: bool,
    /// Doesn't shrink to less than what's in it, as CSS has it, unless it's smaller than that to start with.
    pub(crate) min_auto: bool,
    /// The margins of what's next to each other in this go together, as CSS's blocks have them: the bigger of the two is the gap.
    pub(crate) collapse_margins: bool,
    /// What's first and last in it keeps its margins inside it, rather than their going out of it, as a CSS box that's a
    /// formatting context of its own has it, as `display: flow-root` and `<fieldset>` are.
    pub(crate) keep_margins: bool,
    /// How wide a picture is for how tall it is, as an `<svg>`'s `viewBox` says, which its size that isn't said comes from, as a
    /// replaced element's does in CSS, even between two insets.
    pub(crate) aspect: Option<f32>,
    /// Whether what's in it goes from the end to the start, as CSS's `row-reverse` and `column-reverse` have it.
    pub(crate) reverse: bool,
    /// Whether the start of the way across is at its far side, as a column's is where text goes from right to left (CSS's
    /// `direction: rtl`): what isn't stretched across it sits at the right.
    pub(crate) cross_reverse: bool,
    /// Isn't stretched across what it's in, and is as wide or as tall as what's in it, as CSS's `fit-content` is.
    pub(crate) fit_width: bool,
    pub(crate) fit_height: bool,
    /// No wider than what's in it is, at the most (CSS's `max-width: max-content`, Tailwind's `max-w-max`), as a NavigationMenu's
    /// `flex-1` nav is, which grows no further than its list.
    pub(crate) max_width_fit: bool,
    /// Children that don't fit along go on another line, as `flex-wrap: wrap` has them.
    pub(crate) wrap: bool,
    /// What part of a table this is, as CSS's `display: table-row` and the rest say, from its `table_part`.
    pub(crate) table: TablePart,
}

/// What part of a table something is. A table lays out what's in it as rows of cells, with each column as wide as what's
/// widest in it, wherever that is; see `solve_table` in `engine.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum TablePart {
    #[default]
    None,
    Table,
    /// What holds rows, as `<thead>`, `<tbody>` and `<tfoot>` do.
    Group,
    Row,
    Cell,
    /// What's said about the table, above it.
    Caption,
    /// What says how wide a column is, as `<col>` does, which isn't drawn.
    Column,
    /// What holds those, as `<colgroup>` does.
    Columns,
}

impl TablePart {
    fn read(text: &str) -> Self {
        match text {
            "table" => Self::Table,
            "group" => Self::Group,
            "row" => Self::Row,
            "cell" => Self::Cell,
            "caption" => Self::Caption,
            "column" => Self::Column,
            "columns" => Self::Columns,
            _ => Self::None,
        }
    }
}

impl Default for Style {
    fn default() -> Self {
        Self::new()
    }
}

impl Style {
    pub const fn new() -> Self {
        Self {
            direction: Direction::Column,
            width: Length::Auto,
            height: Length::Auto,
            min_width: 0.0,
            max_width: f32::INFINITY,
            max_width_percent: None,
            min_width_percent: None,
            basis_share: false,
            basis_more: 0.0,
            min_height: 0.0,
            max_height: f32::INFINITY,
            grow: 0.0,
            shrink: 0.0,
            basis: None,
            gap: 0.0,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            align: Align::Stretch,
            align_self: None,
            justify: Justify::Start,
            row_height: None,
            scroll_x: None,
            scroll_y: None,
            overlay: None,
            overlay_fixed: false,
            has_margin: false,
            absolute: false,
            grid: false,
            moved: false,
            min_auto: false,
            collapse_margins: false,
            keep_margins: false,
            aspect: None,
            reverse: false,
            cross_reverse: false,
            fit_width: false,
            max_width_fit: false,
            fit_height: false,
            wrap: false,
            table: TablePart::None,
        }
    }

    /// Stacks children side by side.
    pub const fn row() -> Self {
        let mut style = Self::new();
        style.direction = Direction::Row;
        style
    }

    /// Stacks children top to bottom, which is also the default.
    pub const fn column() -> Self {
        Self::new()
    }

    /// A width in pixels.
    pub const fn width(mut self, pixels: f32) -> Self {
        self.width = Length::Px(pixels);
        self
    }

    /// A width as a percentage of the parent's, from 0 to 100.
    pub const fn width_percent(mut self, percent: f32) -> Self {
        self.width = Length::Percent(percent);
        self
    }

    /// A height in pixels.
    pub const fn height(mut self, pixels: f32) -> Self {
        self.height = Length::Px(pixels);
        self
    }

    /// A height as a percentage of the parent's, from 0 to 100.
    pub const fn height_percent(mut self, percent: f32) -> Self {
        self.height = Length::Percent(percent);
        self
    }

    pub const fn min_width(mut self, pixels: f32) -> Self {
        self.min_width = pixels;
        self
    }

    pub const fn max_width(mut self, pixels: f32) -> Self {
        self.max_width = pixels;
        self
    }

    pub const fn min_height(mut self, pixels: f32) -> Self {
        self.min_height = pixels;
        self
    }

    pub const fn max_height(mut self, pixels: f32) -> Self {
        self.max_height = pixels;
        self
    }

    /// Takes a share of the parent's space, ignoring this element's own size. This is
    /// the usual way to make something fill the space: `flex(2.0)` gets twice what
    /// `flex(1.0)` does.
    pub const fn flex(mut self, share: f32) -> Self {
        self.grow = share;
        self.shrink = 1.0;
        self.basis = Some(0.0);
        self
    }

    /// Keeps this element's own size, and takes this share of any space left over.
    pub const fn grow(mut self, share: f32) -> Self {
        self.grow = share;
        self
    }

    /// Lets this element get smaller when there isn't enough space, by this share of
    /// what's missing. Elements don't get smaller unless they have this or `flex`.
    pub const fn shrink(mut self, share: f32) -> Self {
        self.shrink = share;
        self
    }

    /// Space between children.
    pub const fn gap(mut self, pixels: f32) -> Self {
        self.gap = pixels;
        self
    }

    /// The same padding on all sides.
    pub const fn padding(mut self, pixels: f32) -> Self {
        self.padding_top = pixels;
        self.padding_right = pixels;
        self.padding_bottom = pixels;
        self.padding_left = pixels;
        self
    }

    /// Padding on the left and right, and on the top and bottom.
    pub const fn padding_xy(mut self, x: f32, y: f32) -> Self {
        self.padding_top = y;
        self.padding_right = x;
        self.padding_bottom = y;
        self.padding_left = x;
        self
    }

    /// Different padding on each side.
    pub const fn padding_each(mut self, top: f32, right: f32, bottom: f32, left: f32) -> Self {
        self.padding_top = top;
        self.padding_right = right;
        self.padding_bottom = bottom;
        self.padding_left = left;
        self
    }

    /// Where children go across the direction they stack in.
    pub const fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// Where this element goes across its parent's direction, overriding the parent's
    /// [`align`](Self::align).
    pub const fn align_self(mut self, align: Align) -> Self {
        self.align_self = Some(align);
        self
    }

    /// Where children go along the direction they stack in.
    pub const fn justify(mut self, justify: Justify) -> Self {
        self.justify = justify;
        self
    }

    /// For the element from `Cx::virtual_list`: every row has this height, which lets
    /// layout place rows from the list's scroll position, and skip the rest.
    pub const fn row_height(mut self, pixels: f32) -> Self {
        self.row_height = Some(pixels);
        self
    }

    /// Lets content be taller than this element, moved up by this much, and cut off at its edges.
    /// `Cx::scroll_area` sets this, and lets people move it.
    pub const fn scroll_y(mut self, pixels: f32) -> Self {
        self.scroll_y = Some(pixels);
        self
    }

    /// Shows this element on top of everything else, wherever `placement` says, taking up no room in
    /// its parent. It stays inside the window, and isn't cut off by anything it's inside.
    pub const fn overlay(mut self, placement: Placement) -> Self {
        self.overlay = Some(placement);
        self
    }

    /// Lets content be wider than this element, moved left by this much, and cut off at its edges.
    pub const fn scroll_x(mut self, pixels: f32) -> Self {
        self.scroll_x = Some(pixels);
        self
    }
}

/// Where something on top of everything else goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placement {
    /// Under its parent, or over it if there's more room there.
    Below,
    /// Over its parent, or under it if there's more room there.
    Above,
    /// To the right of its parent, or the left if there's more room there.
    Right,
    /// To the left of its parent, or the right if there's more room there.
    Left,
    /// In the middle of the window.
    Center,
    /// Covering the whole window.
    Fill,
    /// At this position in the window, in layout pixels, as for a menu that opens at the pointer.
    At(f32, f32),
    /// Placed in the window by its own `left`, `top`, `right` and `bottom`, as CSS places what's `position: fixed` (CSS 2.1
    /// §10.3.7 and §10.6.4, with the window as what it's in): both sides of a way and no size stretch it between them, one side
    /// and no size makes it as big as what's in it, shares are of the window, `auto` margins on both sides put it in the
    /// middle of the room between them, and saying neither side leaves it where what it's in starts.
    Inset,
}

/// Props with a margin as well: room outside something, in its parent, as top, right, bottom and left. A margin can be less than
/// nothing, to overlap what's next to it. Unlike in CSS, margins next to each other don't merge.
pub fn with_margin(style: impl Into<Props>, top: f32, right: f32, bottom: f32, left: f32) -> Props {
    let names = &*NAMES;
    let props: Props = style.into();
    let mut entries: Vec<(Symbol, PropValue)> = props
        .iter()
        .map(|(name, value)| (name, value.clone()))
        .collect();
    for (name, value) in [
        (names.margin_top, top),
        (names.margin_right, right),
        (names.margin_bottom, bottom),
        (names.margin_left, left),
    ] {
        if value != 0.0 {
            entries.push((name, PropValue::Float(f64::from(value))));
        }
    }
    entries.into_iter().collect()
}

/// The prop name for each style setting, looked up once.
pub(crate) struct Names {
    pub(crate) direction: Symbol,
    width: Symbol,
    width_percent: Symbol,
    height: Symbol,
    height_percent: Symbol,
    min_width: Symbol,
    max_width: Symbol,
    min_height: Symbol,
    max_height: Symbol,
    grow: Symbol,
    shrink: Symbol,
    min_auto: Symbol,
    collapse_margins: Symbol,
    keep_margins: Symbol,
    aspect: Symbol,
    reverse: Symbol,
    cross_reverse: Symbol,
    fit_width: Symbol,
    max_width_fit: Symbol,
    fit_height: Symbol,
    pub(crate) on_line: Symbol,
    pub(crate) line_strut: Symbol,
    pub(crate) z_index: Symbol,
    /// CSS's `order` on a flex or grid item: where it's laid out among the others, apart from where it's written.
    pub(crate) order: Symbol,
    pub(crate) scrollbar_room: Symbol,
    /// Its scrollbar down the side is at its left, as a browser has it where text goes from right to left.
    pub(crate) scrollbar_left: Symbol,
    pub(crate) fill_across: Symbol,
    pub(crate) scroll_content: Symbol,
    pub(crate) margin_auto: [Symbol; 4],
    pub(crate) grid_auto_rows: Symbol,
    pub(crate) grid_auto_columns: Symbol,
    flex_wrap: Symbol,
    pub(crate) sticky_top: Symbol,
    pub(crate) sticky_bottom: Symbol,
    basis: Symbol,
    gap: Symbol,
    padding_top: Symbol,
    padding_right: Symbol,
    padding_bottom: Symbol,
    padding_left: Symbol,
    align: Symbol,
    align_self: Symbol,
    pub(crate) justify: Symbol,
    pub(crate) row_height: Symbol,
    pub(crate) scroll_x: Symbol,
    pub(crate) scroll_y: Symbol,
    overlay: Symbol,
    overlay_x: Symbol,
    overlay_y: Symbol,
    /// A page's element that isn't positioned, which passes what's placed by itself in it on to what's around it.
    pub(crate) static_box: Symbol,
    margin_top: Symbol,
    margin_right: Symbol,
    margin_bottom: Symbol,
    margin_left: Symbol,
    /// A margin or padding that's a share of what it's in, as `"25%"` or `"50% + -5px"`, as top, right, bottom and left, which
    /// goes over the plain one.
    margin_share: [Symbol; 4],
    padding_share: [Symbol; 4],
    pub(crate) position: Symbol,
    display: Symbol,
    translate_x: Symbol,
    translate_y: Symbol,
    pub(crate) top: Symbol,
    pub(crate) right: Symbol,
    pub(crate) bottom: Symbol,
    pub(crate) left: Symbol,
    pub(crate) columns: Symbol,
    pub(crate) rows: Symbol,
    pub(crate) grid_column: Symbol,
    pub(crate) grid_row: Symbol,
    pub(crate) grid_column_start: Symbol,
    pub(crate) grid_column_end: Symbol,
    pub(crate) grid_row_start: Symbol,
    pub(crate) grid_row_end: Symbol,
    pub(crate) column_gap: Symbol,
    pub(crate) row_gap: Symbol,
    pub(crate) justify_items: Symbol,
    pub(crate) justify_self: Symbol,
    pub(crate) grid_flow: Symbol,
    pub(crate) align_content: Symbol,
    inert: Symbol,
    first_built: Symbol,
    first_visible: Symbol,
    view_rows: Symbol,
    pub(crate) table_part: Symbol,
    pub(crate) col_span: Symbol,
    pub(crate) row_span: Symbol,
    pub(crate) table_fixed: Symbol,
    pub(crate) table_collapse: Symbol,
}

/// Whether anything has ever said it sticks to the edge of what it scrolls in, which is only worth looking for once something has.
/// Whether anything has ever said how far up it goes among what's next to it, which is only worth looking for once something has.
pub(crate) static ANY_Z: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) static ANY_STICKY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(crate) static NAMES: LazyLock<Names> = LazyLock::new(|| Names {
    direction: Symbol::new("direction"),
    width: Symbol::new("width"),
    width_percent: Symbol::new("width_percent"),
    height: Symbol::new("height"),
    height_percent: Symbol::new("height_percent"),
    min_width: Symbol::new("min_width"),
    max_width: Symbol::new("max_width"),
    min_height: Symbol::new("min_height"),
    max_height: Symbol::new("max_height"),
    grow: Symbol::new("grow"),
    shrink: Symbol::new("shrink"),
    min_auto: Symbol::new("min_auto"),
    collapse_margins: Symbol::new("collapse_margins"),
    keep_margins: Symbol::new("keep_margins"),
    aspect: Symbol::new("aspect"),
    reverse: Symbol::new("reverse"),
    cross_reverse: Symbol::new("cross_reverse"),
    fit_width: Symbol::new("fit_width"),
    max_width_fit: Symbol::new("max_width_fit"),
    fit_height: Symbol::new("fit_height"),
    on_line: Symbol::new("on_line"),
    line_strut: Symbol::new("line_strut"),
    z_index: Symbol::new("z_index"),
    order: Symbol::new("order"),
    scrollbar_room: Symbol::new("scrollbar_room"),
    scrollbar_left: Symbol::new("scrollbar_left"),
    fill_across: Symbol::new("fill_across"),
    scroll_content: Symbol::new("scroll_content"),
    margin_auto: [
        Symbol::new("margin_top_auto"),
        Symbol::new("margin_right_auto"),
        Symbol::new("margin_bottom_auto"),
        Symbol::new("margin_left_auto"),
    ],
    grid_auto_rows: Symbol::new("grid_auto_rows"),
    grid_auto_columns: Symbol::new("grid_auto_columns"),
    flex_wrap: Symbol::new("flex_wrap"),
    sticky_top: Symbol::new("sticky_top"),
    sticky_bottom: Symbol::new("sticky_bottom"),
    basis: Symbol::new("basis"),
    gap: Symbol::new("gap"),
    padding_top: Symbol::new("padding_top"),
    padding_right: Symbol::new("padding_right"),
    padding_bottom: Symbol::new("padding_bottom"),
    padding_left: Symbol::new("padding_left"),
    align: Symbol::new("align"),
    align_self: Symbol::new("align_self"),
    justify: Symbol::new("justify"),
    row_height: Symbol::new("row_height"),
    scroll_x: Symbol::new("scroll_x"),
    scroll_y: Symbol::new("scroll_y"),
    overlay: Symbol::new("overlay"),
    overlay_x: Symbol::new("overlay_x"),
    overlay_y: Symbol::new("overlay_y"),
    static_box: Symbol::new("static_box"),
    margin_top: Symbol::new("margin_top"),
    margin_right: Symbol::new("margin_right"),
    margin_bottom: Symbol::new("margin_bottom"),
    margin_left: Symbol::new("margin_left"),
    margin_share: [
        Symbol::new("margin_top_share"),
        Symbol::new("margin_right_share"),
        Symbol::new("margin_bottom_share"),
        Symbol::new("margin_left_share"),
    ],
    padding_share: [
        Symbol::new("padding_top_share"),
        Symbol::new("padding_right_share"),
        Symbol::new("padding_bottom_share"),
        Symbol::new("padding_left_share"),
    ],
    position: Symbol::new("position"),
    display: Symbol::new("display"),
    translate_x: Symbol::new("translate_x"),
    translate_y: Symbol::new("translate_y"),
    top: Symbol::new("top"),
    right: Symbol::new("right"),
    bottom: Symbol::new("bottom"),
    left: Symbol::new("left"),
    columns: Symbol::new("columns"),
    rows: Symbol::new("rows"),
    grid_column: Symbol::new("grid_column"),
    grid_row: Symbol::new("grid_row"),
    grid_column_start: Symbol::new("grid_column_start"),
    grid_column_end: Symbol::new("grid_column_end"),
    grid_row_start: Symbol::new("grid_row_start"),
    grid_row_end: Symbol::new("grid_row_end"),
    column_gap: Symbol::new("column_gap"),
    row_gap: Symbol::new("row_gap"),
    justify_items: Symbol::new("justify_items"),
    justify_self: Symbol::new("justify_self"),
    grid_flow: Symbol::new("grid_flow"),
    align_content: Symbol::new("align_content"),
    inert: Symbol::new("inert"),
    first_built: Symbol::new("first_built"),
    first_visible: Symbol::new("first_visible"),
    view_rows: Symbol::new("view_rows"),
    table_part: Symbol::new("table_part"),
    col_span: Symbol::new("col_span"),
    row_span: Symbol::new("row_span"),
    table_fixed: Symbol::new("table_fixed"),
    table_collapse: Symbol::new("table_collapse"),
});

/// Whether a prop can change how an element or its parent lays out. Other props, like
/// whether a checkbox is checked, never move anything.
pub(crate) fn is_layout_prop(name: Symbol) -> bool {
    let names = &*NAMES;
    [
        names.direction,
        names.width,
        names.width_percent,
        names.height,
        names.height_percent,
        names.min_width,
        names.max_width,
        names.max_width_fit,
        names.min_height,
        names.max_height,
        names.grow,
        names.shrink,
        names.basis,
        names.gap,
        names.padding_top,
        names.padding_right,
        names.padding_bottom,
        names.padding_left,
        names.align,
        names.align_self,
        names.justify,
        names.row_height,
        names.overlay,
        names.overlay_x,
        names.overlay_y,
        names.static_box,
        names.inert,
        names.margin_top,
        names.margin_right,
        names.margin_bottom,
        names.margin_left,
        names.margin_share[0],
        names.margin_share[1],
        names.margin_share[2],
        names.margin_share[3],
        names.padding_share[0],
        names.padding_share[1],
        names.padding_share[2],
        names.padding_share[3],
        names.position,
        names.display,
        names.translate_x,
        names.translate_y,
        names.top,
        names.right,
        names.bottom,
        names.left,
        names.columns,
        names.rows,
        names.grid_column,
        names.grid_row,
        names.grid_column_start,
        names.grid_column_end,
        names.grid_row_start,
        names.grid_row_end,
        names.column_gap,
        names.row_gap,
        names.justify_items,
        names.justify_self,
        names.grid_flow,
        names.align_content,
        names.min_auto,
        names.sticky_top,
        names.sticky_bottom,
        names.collapse_margins,
        names.keep_margins,
        names.aspect,
        names.reverse,
        names.cross_reverse,
        names.fit_width,
        names.fit_height,
        names.max_width_fit,
        names.on_line,
        names.line_strut,
        names.z_index,
        names.order,
        names.flex_wrap,
        names.scrollbar_room,
        names.scrollbar_left,
        names.fill_across,
        names.margin_auto[0],
        names.margin_auto[1],
        names.margin_auto[2],
        names.margin_auto[3],
        names.grid_auto_rows,
        names.grid_auto_columns,
        names.table_part,
        names.col_span,
        names.row_span,
        names.table_fixed,
        names.table_collapse,
    ]
    .contains(&name)
}

/// Properties read directly from the current tree by paint, with no layout or
/// measurement dependency. This deliberately is not the inverse of `is_layout_prop`:
/// unknown properties, text metrics/paragraph styles and image sources must still invalidate.
/// Even `border_color` is excluded: its presence determines whether borders take space.
pub(crate) fn is_paint_only_prop(name: Symbol) -> bool {
    static NAMES: LazyLock<[Symbol; 4]> = LazyLock::new(|| {
        [
            Symbol::new("background"),
            Symbol::new("hover_background"),
            Symbol::new("pressed_background"),
            Symbol::new("text_color"),
        ]
    });
    NAMES.contains(&name)
}

/// Whether this is only there to be seen, on its way out: the pointer goes straight through it, and
/// it can't have the keyboard. That's what the `inert` prop says, for something on top of everything.
/// Whether it's said not to be inert, as CSS's `pointer-events: auto` says it of something in what's `pointer-events: none`.
pub fn takes_pointer(element: &rsui_core::Element) -> bool {
    matches!(
        element.props.get_symbol(NAMES.inert),
        Some(PropValue::Bool(false))
    )
}

pub fn is_inert(element: &rsui_core::Element) -> bool {
    matches!(
        element.props.get_symbol(NAMES.inert),
        Some(PropValue::Bool(true))
    )
}

/// Whether how tall something is depends on how tall what it's in is told to be: a height that's a share of that, as
/// `height_percent`, or a percentage and pixels, or a basis that's a share. Read from its props, as it's asked of everything in a
/// block, which reading its style would be slower for.
pub(crate) fn sized_by_parent_height(element: &rsui_core::Element) -> bool {
    if element.props.is_empty() {
        return false;
    }
    element.props.get_symbol(NAMES.height_percent).is_some()
        || matches!(element.props.get_symbol(NAMES.height), Some(PropValue::Str(_)))
        || matches!(element.props.get_symbol(NAMES.basis), Some(PropValue::Str(_)))
}

/// Whether this has no box of its own, as CSS's `display: contents` says, which is what a fragment is on a page: what's in it is laid
/// out as part of what it's in.
pub(crate) fn is_contents(element: &rsui_core::Element) -> bool {
    // Only a group, as text that's in something is what's drawn there, which has to have a box.
    element.role == rsui_core::Role::Group
        && !element.props.is_empty()
        && matches!(
            element.props.get_symbol(NAMES.display),
            Some(PropValue::Str(text)) if text.as_str() == "contents"
        )
}

/// Whether this is shown as text. Something on top of everything can come from it, as that takes up no
/// room in it, while anything else inside takes the place of the text.
pub fn shows_own_text(element: &rsui_core::Element) -> bool {
    crate::measure::shows_name(element)
        && !element.name.is_empty()
        && element
            .children
            .iter()
            .all(|child| child.props.get_symbol(NAMES.overlay).is_some())
}

/// Whether this is text made of parts that can each look different, which wrap together as one
/// paragraph. That's text with only other text inside it, however deep. Those parts aren't laid out
/// as boxes, or drawn on their own. The paragraph is measured and drawn as a whole.
pub fn is_paragraph(element: &rsui_core::Element) -> bool {
    fn is_part(element: &rsui_core::Element) -> bool {
        matches!(
            element.role,
            rsui_core::Role::Text | rsui_core::Role::Link | rsui_core::Role::Group
        ) && element.props.get_symbol(NAMES.overlay).is_none()
            && element.children.iter().all(|child| is_part(child))
    }
    element.role == rsui_core::Role::Text
        && !element.children.is_empty()
        && element.children.iter().all(|child| is_part(child))
}

fn align_name(align: Align) -> &'static str {
    match align {
        Align::Stretch => "stretch",
        Align::Start => "start",
        Align::Center => "center",
        Align::End => "end",
        Align::Baseline => "baseline",
    }
}

fn justify_name(justify: Justify) -> &'static str {
    match justify {
        Justify::Start => "start",
        Justify::Center => "center",
        Justify::End => "end",
        Justify::SpaceBetween => "space_between",
        Justify::SpaceAround => "space_around",
        Justify::SpaceEvenly => "space_evenly",
    }
}

impl From<Style> for Props {
    fn from(style: Style) -> Props {
        let names = &*NAMES;
        let default = Style::new();
        let mut entries: Vec<(Symbol, PropValue)> = Vec::new();
        // A size that's a percentage and pixels is written as text.
        for (name, length) in [(names.width, style.width), (names.height, style.height)] {
            if let Length::Calc(percent, pixels) = length {
                entries.push((name, PropValue::from(format!("{percent}% + {pixels}px"))));
            }
        }
        // A basis that's a share is written as the percentage it was.
        if let (Some(share), true) = (style.basis, style.basis_share) {
            entries.push((names.basis, PropValue::from(format!("{}%", share * 100.0))));
        }
        let mut number = |name: Symbol, value: f32, default: f32| {
            if value != default {
                entries.push((name, PropValue::Float(f64::from(value))));
            }
        };
        match style.width {
            Length::Auto => {}
            Length::Px(value) => number(names.width, value, f32::NAN),
            Length::Percent(value) => number(names.width_percent, value, f32::NAN),
            Length::Calc(..) => {}
        }
        match style.height {
            Length::Auto => {}
            Length::Px(value) => number(names.height, value, f32::NAN),
            Length::Percent(value) => number(names.height_percent, value, f32::NAN),
            Length::Calc(..) => {}
        }
        number(names.min_width, style.min_width, default.min_width);
        number(names.max_width, style.max_width, default.max_width);
        number(names.min_height, style.min_height, default.min_height);
        number(names.max_height, style.max_height, default.max_height);
        number(names.grow, style.grow, default.grow);
        number(names.shrink, style.shrink, default.shrink);
        if let Some(basis) = style.basis.filter(|_| !style.basis_share) {
            number(names.basis, basis, f32::NAN);
        }
        number(names.gap, style.gap, default.gap);
        number(names.padding_top, style.padding_top, 0.0);
        number(names.padding_right, style.padding_right, 0.0);
        number(names.padding_bottom, style.padding_bottom, 0.0);
        number(names.padding_left, style.padding_left, 0.0);
        if let Some(row_height) = style.row_height {
            number(names.row_height, row_height, f32::NAN);
        }
        if let Some(scroll) = style.scroll_x {
            number(names.scroll_x, scroll, f32::NAN);
        }
        if let Some(scroll) = style.scroll_y {
            number(names.scroll_y, scroll, f32::NAN);
        }
        if let Some(placement) = style.overlay {
            let name = match placement {
                Placement::Below => "below",
                Placement::Above => "above",
                Placement::Right => "right",
                Placement::Left => "left",
                Placement::Center => "center",
                Placement::Fill => "fill",
                Placement::Inset => "inset",
                Placement::At(x, y) => {
                    entries.push((names.overlay_x, PropValue::Float(f64::from(x))));
                    entries.push((names.overlay_y, PropValue::Float(f64::from(y))));
                    if style.overlay_fixed { "fixed" } else { "at" }
                }
            };
            entries.push((names.overlay, name.into()));
        }
        if style.direction != Direction::Column {
            entries.push((names.direction, "row".into()));
        }
        if style.align != Align::Stretch {
            entries.push((names.align, align_name(style.align).into()));
        }
        if let Some(align) = style.align_self {
            entries.push((names.align_self, align_name(align).into()));
        }
        if style.justify != Justify::Start {
            entries.push((names.justify, justify_name(style.justify).into()));
        }
        entries.into_iter().collect()
    }
}

/// The position of a virtual list, from its props.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ListPosition {
    pub(crate) first_built: f32,
    pub(crate) first_visible: f32,
    /// How many rows the list believes fit in it, if it wants to be told.
    pub(crate) view_rows: Option<f32>,
}

impl Style {
    /// Reads a style back from an element's props, ignoring props that aren't for
    /// layout. This runs for every node laid out, so it compares interned names.
    ///
    /// # Panics
    ///
    /// In debug builds, if a layout prop has the wrong type, since that's a mistake
    /// that would otherwise silently do nothing. Props from a [`Style`] always match.
    /// The margins of what has any, as top, right, bottom and left.
    #[cold]
    pub(crate) fn read_margins(props: &Props) -> [f32; 4] {
        let names = &*NAMES;
        let side = |name: Symbol, share: Symbol| match props.get_symbol(share) {
            Some(PropValue::Str(said)) => share_in_pixels(said.as_str()),
            _ => props
                .get_symbol(name)
                .map_or(0.0, |value| number_value(value, name)),
        };
        [
            side(names.margin_top, names.margin_share[0]),
            side(names.margin_right, names.margin_share[1]),
            side(names.margin_bottom, names.margin_share[2]),
            side(names.margin_left, names.margin_share[3]),
        ]
    }

    /// The right and bottom margins, as [`read_margins`](Self::read_margins) reads them, for what only needs how far something
    /// goes down and across, as a scrolling area does of each thing in it, every time it's drawn.
    #[inline]
    pub(crate) fn read_far_margins(props: &Props) -> [f32; 2] {
        if props.is_empty() {
            return [0.0; 2];
        }
        let names = &*NAMES;
        let side = |name: Symbol, share: Symbol| match props.get_symbol(share) {
            Some(PropValue::Str(said)) => share_in_pixels(said.as_str()),
            _ => props
                .get_symbol(name)
                .map_or(0.0, |value| number_value(value, name)),
        };
        [
            side(names.margin_right, names.margin_share[1]),
            side(names.margin_bottom, names.margin_share[2]),
        ]
    }

    /// How far this is moved from where it would be, given how big it is.
    pub(crate) fn read_translate(
        props: &Props,
        width: f32,
        height: f32,
        parent: (f32, f32),
    ) -> (f32, f32) {
        let names = &*NAMES;
        let along = |name: Symbol, size: f32| match props.get_symbol(name).map(read_length) {
            Some(Length::Px(pixels)) => Some(pixels),
            Some(Length::Percent(percent)) => Some(size * percent / 100.0),
            Some(Length::Calc(percent, pixels)) => Some(size * percent / 100.0 + pixels),
            _ => None,
        };
        let mut x = along(names.translate_x, width).unwrap_or(0.0);
        let mut y = along(names.translate_y, height).unwrap_or(0.0);
        let absolute = matches!(props.get_symbol(names.position), Some(PropValue::Str(text)) if text.as_str() == "absolute")
            || matches!(props.get_symbol(names.overlay), Some(PropValue::Str(text)) if text.as_str() == "inset");
        if !absolute {
            x += along(names.left, parent.0)
                .or_else(|| along(names.right, parent.0).map(|right| -right))
                .unwrap_or(0.0);
            y += along(names.top, parent.1)
                .or_else(|| along(names.bottom, parent.1).map(|bottom| -bottom))
                .unwrap_or(0.0);
        }
        (x, y)
    }

    pub(crate) fn read(props: &Props) -> (Self, ListPosition) {
        let mut style = Style::new();
        let mut has_inset = false;
        let mut list = ListPosition::default();
        if props.is_empty() {
            return (style, list);
        }
        let names = &*NAMES;
        let mut overlay_at = (0.0, 0.0);
        let mut padding_shares: [Option<f32>; 4] = [None; 4];
        for (name, value) in props.iter() {
            if name == names.width {
                style.width = size_of(value, name);
            } else if name == names.height {
                style.height = size_of(value, name);
            } else if name == names.width_percent {
                style.width = Length::Percent(pixels(value, name));
            } else if name == names.height_percent {
                style.height = Length::Percent(pixels(value, name));
            } else if name == names.direction {
                style.direction = match text(value, name) {
                    "row" => Direction::Row,
                    "column" => Direction::Column,
                    other => bad_value(name, other, Direction::Column),
                };
            } else if name == names.gap {
                style.gap = pixels(value, name);
            } else if name == names.grow {
                style.grow = pixels(value, name);
            } else if name == names.shrink {
                style.shrink = pixels(value, name);
            } else if name == names.min_auto {
                style.min_auto = matches!(value, PropValue::Bool(true));
            } else if name == names.fit_width {
                style.fit_width = matches!(value, PropValue::Bool(true));
            } else if name == names.max_width_fit {
                style.max_width_fit = matches!(value, PropValue::Bool(true));
            } else if name == names.fit_height {
                style.fit_height = matches!(value, PropValue::Bool(true));
            } else if name == names.z_index {
                ANY_Z.store(true, std::sync::atomic::Ordering::Relaxed);
            } else if name == names.flex_wrap {
                style.wrap = matches!(value, PropValue::Bool(true));
            } else if name == names.collapse_margins {
                style.collapse_margins = matches!(value, PropValue::Bool(true));
            } else if name == names.keep_margins {
                style.keep_margins = matches!(value, PropValue::Bool(true));
            } else if name == names.aspect {
                style.aspect = match value {
                    PropValue::Float(ratio) if *ratio > 0.0 && ratio.is_finite() => Some(*ratio as f32),
                    PropValue::Int(ratio) if *ratio > 0 => Some(*ratio as f32),
                    _ => None,
                };
            } else if name == names.reverse {
                style.reverse = matches!(value, PropValue::Bool(true));
            } else if name == names.cross_reverse {
                style.cross_reverse = matches!(value, PropValue::Bool(true));
            } else if name == names.sticky_top || name == names.sticky_bottom {
                ANY_STICKY.store(true, std::sync::atomic::Ordering::Relaxed);
            } else if name == names.basis {
                match value {
                    // A share of how long what it's in is along, as `flex-basis: 100%` says, kept as that share (1.0 for all of it)
                    // until that's known: see `flex_items` in engine.rs. CSS's `flex: 1` is a share of 0%, which is nothing.
                    PropValue::Str(said) if said.as_str().contains('%') => {
                        let (share, more) = share_and_more(said.as_str()).unwrap_or((0.0, 0.0));
                        let share = share / 100.0;
                        style.basis = Some(if share.is_finite() { share.max(0.0) } else { 0.0 });
                        style.basis_share = true;
                        style.basis_more = if more.is_finite() { more } else { 0.0 };
                    }
                    PropValue::Str(_) => {}
                    _ => style.basis = Some(pixels(value, name)),
                }
            } else if name == names.padding_top {
                style.padding_top = pixels(value, name);
            } else if name == names.padding_right {
                style.padding_right = pixels(value, name);
            } else if name == names.padding_bottom {
                style.padding_bottom = pixels(value, name);
            } else if name == names.padding_left {
                style.padding_left = pixels(value, name);
            } else if name == names.min_width {
                match value {
                    PropValue::Str(said) if said.as_str().contains('%') => {
                        style.min_width_percent = share_and_more(said.as_str());
                    }
                    PropValue::Str(_) => {}
                    _ => style.min_width = pixels(value, name),
                }
            } else if name == names.max_width {
                match value {
                    PropValue::Str(said) if said.as_str().contains('%') => {
                        style.max_width_percent = share_and_more(said.as_str());
                    }
                    // Anything else that's text, as `none`, is no limit.
                    PropValue::Str(_) => {}
                    _ => style.max_width = pixels(value, name),
                }
            } else if name == names.min_height {
                style.min_height = pixels(value, name);
            } else if name == names.max_height {
                style.max_height = pixels(value, name);
            } else if name == names.align {
                style.align = read_align(value, name);
            } else if name == names.align_self {
                style.align_self = Some(read_align(value, name));
            } else if name == names.justify {
                style.justify = match text(value, name) {
                    "start" => Justify::Start,
                    "center" => Justify::Center,
                    "end" => Justify::End,
                    "space_between" => Justify::SpaceBetween,
                    "space_around" => Justify::SpaceAround,
                    "space_evenly" => Justify::SpaceEvenly,
                    other => bad_value(name, other, Justify::Start),
                };
            } else if name == names.row_height {
                style.row_height = Some(pixels(value, name));
            } else if name == names.overlay {
                style.overlay = Some(match text(value, name) {
                    "below" => Placement::Below,
                    "above" => Placement::Above,
                    "right" => Placement::Right,
                    "left" => Placement::Left,
                    "center" => Placement::Center,
                    "fill" => Placement::Fill,
                    "inset" => Placement::Inset,
                    "fixed" => {
                        style.overlay_fixed = true;
                        Placement::At(0.0, 0.0)
                    }
                    "at" => Placement::At(0.0, 0.0),
                    other => bad_value(name, other, Placement::Below),
                });
            } else if name == names.overlay_x {
                overlay_at.0 = number_value(value, name);
            } else if name == names.overlay_y {
                overlay_at.1 = number_value(value, name);
            } else if name == names.margin_top
                || name == names.margin_right
                || name == names.margin_bottom
                || name == names.margin_left
                || names.margin_share.contains(&name)
            {
                style.has_margin = true;
            } else if let Some(side) = names.padding_share.iter().position(|one| *one == name) {
                if let PropValue::Str(said) = value {
                    padding_shares[side] = Some(share_in_pixels(said.as_str()));
                }
            } else if name == names.position {
                style.absolute =
                    matches!(value, PropValue::Str(text) if text.as_str() == "absolute");
            } else if name == names.display {
                style.grid = matches!(value, PropValue::Str(text) if text.as_str() == "grid");
            } else if name == names.table_part {
                if let PropValue::Str(text) = value {
                    style.table = TablePart::read(text.as_str());
                }
            } else if name == names.translate_x || name == names.translate_y {
                style.moved = true;
            } else if name == names.left
                || name == names.top
                || name == names.right
                || name == names.bottom
            {
                // Something that isn't placed by these is moved by them, as `position: relative` is in CSS.
                has_inset = true;
            } else if name == names.scroll_y {
                style.scroll_y = Some(number_value(value, name));
            } else if name == names.scroll_x {
                style.scroll_x = Some(number_value(value, name));
            } else if name == names.first_built {
                list.first_built = number_value(value, name);
            } else if name == names.first_visible {
                list.first_visible = number_value(value, name);
            } else if name == names.view_rows {
                list.view_rows = Some(number_value(value, name));
            }
        }
        if let Some(Placement::At(_, _)) = style.overlay {
            style.overlay = Some(Placement::At(overlay_at.0, overlay_at.1));
        }
        // A padding that's a share of what it's in is on top of what the plain one says, which is its border, if any.
        if padding_shares.iter().any(Option::is_some) {
            let [top, right, bottom, left] = padding_shares.map(|share| share.unwrap_or(0.0).max(0.0));
            style.padding_top += top;
            style.padding_right += right;
            style.padding_bottom += bottom;
            style.padding_left += left;
        }
        // What's fixed is placed by them, rather than moved.
        if has_inset && !style.absolute && style.overlay != Some(Placement::Inset) {
            style.moved = true;
        }
        (style, list)
    }
}

/// A distance that can be in pixels, or as text like `50%` or `10px`.
/// A width or height, which is pixels unless it's written as a percentage or `"50% + 10px"`.
fn size_of(value: &PropValue, name: Symbol) -> Length {
    match value {
        PropValue::Str(_) => match read_length(value) {
            Length::Px(pixels) => Length::Px(pixels.max(0.0)),
            // Text that isn't a size is a mistake, which debug builds say.
            Length::Auto => Length::Px(pixels(value, name)),
            other => other,
        },
        _ => Length::Px(pixels(value, name)),
    }
}

pub(crate) fn read_length(value: &PropValue) -> Length {
    match value {
        PropValue::Float(number) => Length::Px(*number as f32),
        PropValue::Int(number) => Length::Px(*number as f32),
        PropValue::Str(text) => {
            let text = text.trim();
            if let Some((percent, pixels)) = text.split_once("% +") {
                let pixels = pixels.trim();
                return match (
                    percent.trim().parse(),
                    pixels.strip_suffix("px").unwrap_or(pixels).trim().parse(),
                ) {
                    (Ok(percent), Ok(pixels)) => Length::Calc(percent, pixels),
                    _ => Length::Auto,
                };
            }
            if let Some(number) = text.strip_suffix('%') {
                number.trim().parse().map_or(Length::Auto, Length::Percent)
            } else {
                text.strip_suffix("px")
                    .unwrap_or(text)
                    .trim()
                    .parse()
                    .map_or(Length::Auto, Length::Px)
            }
        }
        PropValue::Bool(_) => Length::Auto,
    }
}

pub(crate) fn read_align(value: &PropValue, name: Symbol) -> Align {
    match text(value, name) {
        "stretch" => Align::Stretch,
        "start" => Align::Start,
        "center" => Align::Center,
        "end" => Align::End,
        "baseline" => Align::Baseline,
        other => bad_value(name, other, Align::Stretch),
    }
}

fn number_value(value: &PropValue, name: Symbol) -> f32 {
    match value {
        PropValue::Float(value) => *value as f32,
        PropValue::Int(value) => *value as f32,
        _ => {
            debug_assert!(false, "the {} prop must be a number", name.as_str());
            0.0
        }
    }
}

/// A size can't be negative, and `NaN` isn't a size, so both become zero.
fn pixels(value: &PropValue, name: Symbol) -> f32 {
    number_value(value, name).max(0.0)
}

fn text(value: &PropValue, name: Symbol) -> &str {
    match value {
        PropValue::Str(value) => value.as_str(),
        _ => {
            debug_assert!(false, "the {} prop must be text", name.as_str());
            ""
        }
    }
}

fn bad_value<T>(name: Symbol, value: &str, fallback: T) -> T {
    debug_assert!(
        false,
        "{value:?} isn't a value for the {} prop",
        name.as_str()
    );
    fallback
}

/// A share of what something's in, as `75%`, or `75% + 16px` for what its padding and border add to that: the percentage, and
/// the pixels more.
fn share_and_more(said: &str) -> Option<(f32, f32)> {
    let (share, more) = said.split_once('%').unwrap_or((said, ""));
    let more = more
        .trim()
        .trim_start_matches('+')
        .trim()
        .trim_end_matches("px")
        .trim();
    share
        .trim()
        .parse::<f32>()
        .ok()
        .map(|share| (share, more.parse::<f32>().unwrap_or(0.0)))
}
