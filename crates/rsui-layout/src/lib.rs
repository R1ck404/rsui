//! Works out where everything in a rsui tree goes.
//!
//! An element arranges its children in a row or a column. These are all the rules:
//!
//! - **Direction:** children stack in a column unless the element is a [`Style::row`].
//! - **Size:** an element is sized by `width` and `height` if set, in pixels or as a
//!   percentage of its parent. Otherwise it fits its content, including its padding.
//! - **Across:** children stretch to fill the direction they don't stack in, unless
//!   they have their own size there, or an [`Align`] says otherwise.
//! - **Along:** children keep their size in the direction they stack in. Use
//!   [`Style::flex`] to share out the space, with `flex(2.0)` getting twice what
//!   `flex(1.0)` does. [`Justify`] places them if space is left over.
//! - **Limits:** `min_` and `max_` sizes always win.
//! - **Overflow:** content that doesn't fit overflows its parent. Layout never hides it
//!   or shrinks it unless asked to, through [`Style::flex`] or [`Style::shrink`].
//!
//! Text comes from a [`Measure`], so layout knows nothing about fonts.
//!
//! ```
//! use rsui_core::{Element, Role};
//! use rsui_layout::{FixedMetrics, Layout, Rect, Size, Style, style};
//!
//! let root = Element::new(Role::Window)
//!     .props(style!(Style::row().gap(10.0)))
//!     .child(Element::new(Role::Group).props(style!(Style::new().width(100.0))))
//!     .child(Element::new(Role::Group).props(style!(Style::new().flex(1.0))))
//!     .into_node();
//!
//! let mut layout = Layout::new();
//! layout.compute(&root, Size::new(400.0, 300.0), &mut FixedMetrics::default());
//!
//! assert_eq!(layout.rect(&[1]), Some(Rect::new(110.0, 0.0, 290.0, 300.0)));
//! ```
//!
//! To keep it up to date, pass it the patches from each update, then compute again:
//!
//! ```text
//! let patches = runtime.flush();
//! layout.apply(&patches);
//! layout.compute(runtime.root(), window_size, &mut text);
//! ```
//!
//! A change that doesn't alter a node's size stops there, so most updates don't touch
//! anything but the node that changed.

mod engine;
mod grid;
mod hit_region;
mod measure;
mod share;
mod style;
mod tree;

pub use measure::{FixedMetrics, Measure, TEXT_INPUT_WIDTH, input_width, shows_name};
pub use hit_region::HitRegion;
pub use share::{Clamp, Ends, Kept, Line, Mix};
pub use style::{
    Align, Direction, Justify, Length, Placement, Style, is_inert, is_paragraph, shows_own_text,
    takes_pointer, with_margin,
};
pub use tree::{Layout, LayoutVisitor, MOVES_PAINT, Rect, ScrollInfo, Size, clips, moves_paint};

/// The props for a [`Style`], built once for this line and shared by every element
/// that passes through it. Use this when the style's values never change, as it
/// saves memory and makes comparing elements cheaper. For values that change, use
/// `Props::from(style)`. Anything else that converts to props can come after the style,
/// such as how the element should be painted.
///
/// ```
/// use rsui_core::{Element, Role};
/// use rsui_layout::{Style, style};
///
/// let toolbar = || Element::new(Role::Group).props(style!(Style::row().gap(8.0)));
/// assert_eq!(toolbar(), toolbar());
/// ```
///
/// Giving it values that change would quietly keep the first ones, so while developing, it panics.
#[macro_export]
macro_rules! style {
    ($first:expr $(, $rest:expr)* $(,)?) => {{
        static PROPS: ::std::sync::OnceLock<$crate::__Props> = ::std::sync::OnceLock::new();
        let make = || {
            let props: $crate::__Props = ::std::convert::Into::into($first);
            $(let props = props.merged(&::std::convert::Into::into($rest));)*
            props
        };
        let props = PROPS.get_or_init(&make).clone();
        ::std::debug_assert!(
            props == make(),
            "style! keeps what it worked out the first time, and these values have changed. \
             Use Props::from(style) for values that change."
        );
        props
    }};
}

#[doc(hidden)]
pub use rsui_core::Props as __Props;
