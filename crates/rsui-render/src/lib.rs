//! Draws rsui trees with Direct3D 11.
//!
//! This is in two parts. A [`Painter`] turns a tree and its layout into a [`Scene`]: a
//! list of rectangles and glyphs, with no GPU involved. A [`Renderer`] puts a scene on
//! screen. Set how things look with [`Paint`]:
//!
//! ```
//! use rsui_core::{Element, Role};
//! use rsui_layout::{Style, style};
//! use rsui_render::{Color, Paint};
//!
//! let button = Element::new(Role::Button).name("Save").props(style!(
//!     Style::new().padding_xy(12.0, 6.0),
//!     Paint::new().background(Color::hex(0x1F6FEB)).text(Color::WHITE).corner_radius(6.0),
//! ));
//! ```
//!
//! # Memory
//!
//! A [`Renderer`] is set up to use as little as Direct3D allows, based on measurements
//! of what a window costs. Call [`Renderer::compact`] and [`Renderer::trim`] once nothing has
//! changed for a while, and [`Renderer::hide`] while the window is hidden. Dropping it lets go of
//! the driver altogether, with [`Renderer::draw_to_memory`] keeping the picture if it's still
//! on screen.

#[cfg(feature = "render-diagnostics")]
pub mod diagnostics;

mod area;
pub mod canvas;
#[cfg(windows)]
mod dwrite;
#[cfg(not(windows))]
#[path = "own_glyphs.rs"]
mod dwrite;
#[cfg(windows)]
mod gpu;
// Elsewhere, until each system has a GPU renderer of its own, scenes are drawn with the CPU, through the same interface.
#[cfg(not(windows))]
#[path = "cpu.rs"]
mod gpu;
mod gradient;
mod images;
mod paint;
mod scene;
mod shape;
mod svg_image;
mod transform;

pub use gpu::{RenderError, Renderer, WorkingSetLimit, give_back_memory, start_gpu_early};
pub use images::{ImageUpload, Images, image_size, register_image, set_files_from};
pub use paint::{Color, Paint, has_hover, has_pressed};
pub use scene::{
    Batch, ContentBox, Editing, EditingLines, Filter, Interaction, Layer, Overlay, OverlayBox, OverlayText,
    Painter, PixelRect, Quad, RoundClip, SCROLL_BAR_REACH, Scene, byte_from_mask, byte_in_mask, content_box,
    input_indent, input_value, password_mask, scroll_for_thumb, scroll_thumb, text_line_count,
};
pub use shape::{LineCap, natural_shape_size, shape_layer};
pub use transform::{Matrix, apply_matrix, invert_matrix, parse_transform, transform_of};
