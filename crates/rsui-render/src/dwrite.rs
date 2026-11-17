//! Glyphs drawn by Windows itself, through DirectWrite, which is how a browser on Windows draws them. A page looks the way it does
//! in one only if its letters come out the same, to the pixel.

use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::os::windows::ffi::OsStrExt;

use rsui_text::{Font, GlyphBitmap, SystemGlyphs};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_AXIS_TAG, DWRITE_FONT_AXIS_VALUE,
    DWRITE_FONT_FACE_TYPE, DWRITE_FONT_FILE_TYPE, DWRITE_FONT_SIMULATIONS_NONE, DWRITE_GLYPH_RUN,
    DWRITE_GRID_FIT_MODE_DEFAULT, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_RENDERING_MODE1_NATURAL_SYMMETRIC, DWRITE_TEXT_ANTIALIAS_MODE_CLEARTYPE,
    DWRITE_TEXTURE_CLEARTYPE_3x1, DWriteCreateFactory, IDWriteFactory, IDWriteFactory5,
    IDWriteFactory6, IDWriteFontFace, IDWriteInMemoryFontFileLoader,
};
use windows::core::{BOOL, Interface, PCWSTR};

/// The gamma and contrast a browser puts glyphs' coverage through for their color, as it is on Windows, for
/// [`Scene::text_look`](crate::Scene::text_look). These are what make letters come out the same as Chromium's, to the pixel.
pub(crate) const BROWSER_TEXT_LOOK: [f32; 2] = [1.6, 0.0];

/// What draws glyphs the way the system does, if it can here.
pub(crate) fn system_glyphs() -> Option<Box<dyn SystemGlyphs>> {
    let lcd = clear_type_is_on() && std::env::var_os("RSUI_GRAY_TEXT").is_none();
    DirectWriteGlyphs::new(lcd).map(|glyphs| Box::new(glyphs) as Box<dyn SystemGlyphs>)
}

/// What draws glyphs the way the system does, gray, as a browser draws text on a canvas, which may be see-through.
pub(crate) fn gray_glyphs() -> Option<Box<dyn SystemGlyphs>> {
    DirectWriteGlyphs::new(false).map(|glyphs| Box::new(glyphs) as Box<dyn SystemGlyphs>)
}

/// Whether Windows draws text with ClearType, in color for each part of a pixel, as a browser then does too. `RSUI_GRAY_TEXT`
/// in the environment has it gray anyway, as a browser started with `--disable-lcd-text` is, for comparing with one.
fn clear_type_is_on() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETFONTSMOOTHING, SPI_GETFONTSMOOTHINGTYPE, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
        SystemParametersInfoW,
    };
    let (mut smoothing, mut kind) = (0u32, 0u32);
    // SAFETY: each is asked for into a number that's big enough for it.
    unsafe {
        let on = SystemParametersInfoW(
            SPI_GETFONTSMOOTHING,
            0,
            Some((&raw mut smoothing).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        let typed = SystemParametersInfoW(
            SPI_GETFONTSMOOTHINGTYPE,
            0,
            Some((&raw mut kind).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        // 2 is ClearType, as against plain smoothing.
        on.is_ok() && typed.is_ok() && smoothing != 0 && kind == 2
    }
}

/// Draws glyphs through DirectWrite, keeping each font it's been asked about.
struct DirectWriteGlyphs {
    /// Whether glyphs come with each part of each pixel apart, as ClearType draws them.
    lcd: bool,
    factory: IDWriteFactory5,
    loader: IDWriteInMemoryFontFileLoader,
    /// Each font, by where its file's bytes are, how long they are, which font in them it is and where it's set along its axes,
    /// for one that varies. `None` is one DirectWrite can't read.
    faces: HashMap<FaceKey, Option<IDWriteFontFace>>,
}

/// What tells fonts apart for DirectWrite: their file's bytes, which font in it, and each axis's tag and value, as bits.
type FaceKey = (usize, usize, u32, Vec<(u32, u32)>);

impl DirectWriteGlyphs {
    /// `None` on a Windows too old to have what this needs.
    fn new(lcd: bool) -> Option<Self> {
        // SAFETY: these are DirectWrite's own calls, given what they ask for.
        unsafe {
            let factory: IDWriteFactory5 =
                DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED)
                    .ok()?
                    .cast()
                    .ok()?;
            let loader = factory.CreateInMemoryFontFileLoader().ok()?;
            factory.RegisterFontFileLoader(&loader).ok()?;
            Some(Self {
                lcd,
                factory,
                loader,
                faces: HashMap::new(),
            })
        }
    }

    fn face(&mut self, font: &Font) -> Option<IDWriteFontFace> {
        let bytes = font.file_bytes();
        // A font set somewhere along its axes is drawn there, as one at its defaults is drawn as its tables have it.
        let axes: Vec<DWRITE_FONT_AXIS_VALUE> = if font.is_varied() {
            font.variations()
                .map(|(tag, value)| DWRITE_FONT_AXIS_VALUE {
                    axisTag: DWRITE_FONT_AXIS_TAG(u32::from_le_bytes(tag)),
                    value,
                })
                .collect()
        } else {
            Vec::new()
        };
        let key = (
            bytes.as_ptr() as usize,
            bytes.len(),
            font.face_index(),
            axes.iter().map(|axis| (axis.axisTag.0, axis.value.to_bits())).collect(),
        );
        if let Some(found) = self.faces.get(&key) {
            return found.clone();
        }
        // SAFETY: the loader copies the bytes, as it isn't given anything that holds on to them, and the path is a string that
        // ends in a 0 until the call's done.
        let made = unsafe {
            (|| {
                // A font that's in a file is read from there, where DirectWrite can share it with everything else that uses it,
                // rather than as a copy.
                let file = match font.file_path() {
                    Some(path) => {
                        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
                        self.factory
                            .CreateFontFileReference(PCWSTR(wide.as_ptr()), None)
                            .ok()?
                    }
                    None => self
                        .loader
                        .CreateInMemoryFontFileReference(
                            &self.factory,
                            bytes.as_ptr().cast(),
                            u32::try_from(bytes.len()).ok()?,
                            None,
                        )
                        .ok()?,
                };
                let mut supported = BOOL(0);
                let mut file_type = DWRITE_FONT_FILE_TYPE::default();
                let mut face_type = DWRITE_FONT_FACE_TYPE::default();
                let mut faces = 0u32;
                file.Analyze(
                    &mut supported,
                    &mut file_type,
                    Some(&mut face_type),
                    &mut faces,
                )
                .ok()?;
                if !supported.as_bool() {
                    return None;
                }
                if !axes.is_empty() {
                    let factory: IDWriteFactory6 = self.factory.cast().ok()?;
                    let resource = factory.CreateFontResource(&file, font.face_index()).ok()?;
                    let varied = resource
                        .CreateFontFace(DWRITE_FONT_SIMULATIONS_NONE, &axes)
                        .ok()?;
                    return varied.cast().ok();
                }
                self.factory
                    .CreateFontFace(
                        face_type,
                        &[Some(file)],
                        font.face_index(),
                        DWRITE_FONT_SIMULATIONS_NONE,
                    )
                    .ok()
            })()
        };
        self.faces.insert(key, made.clone());
        made
    }
}

impl SystemGlyphs for DirectWriteGlyphs {
    fn rasterize(
        &mut self,
        font: &Font,
        glyph: u16,
        size: f32,
        shift_x: f32,
    ) -> Option<GlyphBitmap> {
        let face = self.face(font)?;
        let advance = 0.0f32;
        let run = DWRITE_GLYPH_RUN {
            fontFace: ManuallyDrop::new(Some(face)),
            fontEmSize: size,
            glyphCount: 1,
            glyphIndices: &glyph,
            glyphAdvances: &advance,
            glyphOffsets: std::ptr::null(),
            isSideways: BOOL(0),
            bidiLevel: 0,
        };
        // SAFETY: the run and what it points to are all here until DirectWrite's done with them.
        let drawn = unsafe {
            (|| {
                let analysis = self
                    .factory
                    .CreateGlyphRunAnalysis(
                        &run,
                        None,
                        DWRITE_RENDERING_MODE1_NATURAL_SYMMETRIC,
                        DWRITE_MEASURING_MODE_NATURAL,
                        DWRITE_GRID_FIT_MODE_DEFAULT,
                        DWRITE_TEXT_ANTIALIAS_MODE_CLEARTYPE,
                        shift_x,
                        0.0,
                    )
                    .ok()?;
                let bounds: RECT = analysis
                    .GetAlphaTextureBounds(DWRITE_TEXTURE_CLEARTYPE_3x1)
                    .ok()?;
                let (width, height) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
                if width <= 0 || height <= 0 {
                    return Some(GlyphBitmap::default());
                }
                let mut rgb = vec![0u8; width as usize * height as usize * 3];
                analysis
                    .CreateAlphaTexture(DWRITE_TEXTURE_CLEARTYPE_3x1, &bounds, &mut rgb)
                    .ok()?;
                // Each pixel's three parts as ClearType has them, or together, which spreads a stem's edge across a pixel on
                // each side the way a browser's gray text has it.
                let coverage = if self.lcd {
                    rgb
                } else {
                    rgb.chunks_exact(3)
                        .map(|three| {
                            ((u32::from(three[0]) + u32::from(three[1]) + u32::from(three[2])) / 3)
                                as u8
                        })
                        .collect()
                };
                Some(GlyphBitmap {
                    left: bounds.left,
                    top: bounds.top,
                    width: width as u32,
                    height: height as u32,
                    coverage,
                    subpixel: self.lcd,
                })
            })()
        };
        // The run's font is only borrowed for this.
        let _ = ManuallyDrop::into_inner(run.fontFace);
        drawn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arial() -> Font {
        Font::open("C:/Windows/Fonts/arial.ttf").expect("Windows has Arial")
    }

    #[test]
    fn a_letter_drawn_by_the_system_has_ink_above_the_baseline() {
        let font = arial();
        let glyph = font.glyph('H').expect("Arial has H");
        let mut system = DirectWriteGlyphs::new(false).expect("DirectWrite is there");
        let drawn = system
            .rasterize(&font, glyph, 16.0, 0.0)
            .expect("DirectWrite draws it");
        assert!(drawn.width > 4 && drawn.height > 8, "{drawn:?}");
        assert!(
            drawn.top < -8 && drawn.top + drawn.height as i32 <= 1,
            "{drawn:?}"
        );
        assert_eq!(drawn.coverage.len(), (drawn.width * drawn.height) as usize);
        assert!(drawn.coverage.contains(&255), "a stem covers whole pixels");
    }

    #[test]
    fn a_space_comes_out_empty() {
        let font = arial();
        let glyph = font.glyph(' ').expect("Arial has a space");
        let mut system = DirectWriteGlyphs::new(false).expect("DirectWrite is there");
        let drawn = system
            .rasterize(&font, glyph, 16.0, 0.0)
            .expect("DirectWrite draws it");
        assert_eq!(drawn.width, 0);
        assert!(drawn.coverage.is_empty());
    }

    #[test]
    fn half_a_pixel_along_moves_a_stem_into_the_next_pixel() {
        let font = arial();
        let glyph = font.glyph('l').expect("Arial has l");
        let mut system = DirectWriteGlyphs::new(false).expect("DirectWrite is there");
        let ink = |drawn: &GlyphBitmap| -> f32 {
            let row = drawn.height as usize / 2 * drawn.width as usize;
            let mut sum = 0.0;
            let mut weighted = 0.0;
            for (x, &value) in drawn.coverage[row..row + drawn.width as usize]
                .iter()
                .enumerate()
            {
                sum += f32::from(value);
                weighted += f32::from(value) * (x as f32 + drawn.left as f32);
            }
            weighted / sum
        };
        let still = system.rasterize(&font, glyph, 16.0, 0.0).expect("drawn");
        let moved = system.rasterize(&font, glyph, 16.0, 0.5).expect("drawn");
        let shift = ink(&moved) - ink(&still);
        assert!((shift - 0.5).abs() < 0.2, "{shift}");
    }

    #[test]
    fn a_font_from_memory_is_drawn_too() {
        let bytes = std::fs::read("C:/Windows/Fonts/arial.ttf").expect("Windows has Arial");
        let font = Font::from_bytes(bytes).expect("it's a font");
        assert!(font.file_path().is_none());
        let glyph = font.glyph('H').expect("Arial has H");
        let mut system = DirectWriteGlyphs::new(false).expect("DirectWrite is there");
        let from_memory = system.rasterize(&font, glyph, 16.0, 0.0).expect("drawn");
        let from_file = system.rasterize(&arial(), glyph, 16.0, 0.0).expect("drawn");
        assert_eq!(from_memory, from_file);
    }

    #[test]
    fn clear_type_gives_each_part_of_a_pixel_apart() {
        let font = arial();
        let glyph = font.glyph('l').expect("Arial has l");
        let mut gray = DirectWriteGlyphs::new(false).expect("DirectWrite is there");
        let mut parts = DirectWriteGlyphs::new(true).expect("DirectWrite is there");
        let one = gray.rasterize(&font, glyph, 16.0, 0.3).expect("drawn");
        let three = parts.rasterize(&font, glyph, 16.0, 0.3).expect("drawn");
        assert!(three.subpixel && !one.subpixel);
        assert_eq!((three.width, three.height), (one.width, one.height));
        assert_eq!(three.coverage.len(), one.coverage.len() * 3);
        // A stem's edge covers the parts of a pixel differently, which is what makes it look sharper.
        assert!(three.coverage.chunks_exact(3).any(|rgb| rgb[0] != rgb[2]));
    }

    #[test]
    fn a_font_that_varies_is_drawn_where_it_is_set() {
        let Ok(regular) = Font::open("C:/Windows/Fonts/SegUIVar.ttf") else {
            return;
        };
        let mut bold = Font::open("C:/Windows/Fonts/SegUIVar.ttf").expect("it opened once");
        bold.set_variations(&[(*b"wght", 700.0)]);
        let glyph = regular.glyph('l').expect("it has l");
        let mut system = DirectWriteGlyphs::new(false).expect("DirectWrite is there");
        let ink = |drawn: GlyphBitmap| drawn.coverage.iter().map(|&value| u32::from(value)).sum::<u32>();
        let thin = ink(system.rasterize(&regular, glyph, 40.0, 0.0).expect("drawn"));
        let thick = ink(system.rasterize(&bold, glyph, 40.0, 0.0).expect("drawn"));
        // A bold stem covers far more.
        assert!(thick as f32 > thin as f32 * 1.3, "{thin} {thick}");
        assert_eq!(system.faces.len(), 2);
    }

    #[test]
    fn each_font_is_read_once() {
        let font = arial();
        let glyph = font.glyph('a').expect("Arial has a");
        let mut system = DirectWriteGlyphs::new(false).expect("DirectWrite is there");
        system.rasterize(&font, glyph, 12.0, 0.0);
        system.rasterize(&font, glyph, 20.0, 0.25);
        assert_eq!(system.faces.len(), 1);
    }
}
