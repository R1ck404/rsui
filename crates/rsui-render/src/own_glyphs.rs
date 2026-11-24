//! Outside Windows, glyphs are always drawn by rsui itself, so they look the same on every system: there's no system
//! renderer to ask for them, as DirectWrite is on Windows (`dwrite.rs`).

use rsui_text::SystemGlyphs;

/// The gamma and contrast a browser puts glyphs' coverage through on Windows. Only drawing as the system does uses them,
/// which never happens here, but `canvas` reads them the same way everywhere.
pub(crate) const BROWSER_TEXT_LOOK: [f32; 2] = [1.6, 0.0];

/// There's no system renderer of glyphs here.
pub(crate) fn system_glyphs() -> Option<Box<dyn SystemGlyphs>> {
    None
}

/// Nor a gray one.
pub(crate) fn gray_glyphs() -> Option<Box<dyn SystemGlyphs>> {
    None
}
