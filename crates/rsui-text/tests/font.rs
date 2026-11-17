//! Reading fonts, including damaged ones.

mod support;

mod damaged {
    use super::support::{Options, font_bytes};
    use proptest::prelude::*;
    use rsui_text::{Font, GlyphAtlas, GlyphKey, Rasterizer, TextEngine, TextStyle};

    /// Uses everything a font does, which mustn't panic or hang on whatever's in the file.
    fn exercise(bytes: Vec<u8>) {
        let Ok(font) = Font::from_bytes(bytes) else {
            return;
        };
        let mut rasterizer = Rasterizer::default();
        let mut atlas = GlyphAtlas::new();
        for glyph in 0..8 {
            let key = GlyphKey::new(0, glyph, 16.0, 0.25);
            atlas.glyph(key, &font, &mut rasterizer);
        }
        let mut engine = TextEngine::new(font);
        let style = TextStyle::new(16.0);
        engine.layout("ABCD AB\nCD", style, 40.0);
        engine.caret_x("ABCD", style, 2);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(4000))]

        /// A font is a file from outside the program, and could hold anything.
        #[test]
        fn damaged_fonts_never_panic(
            changes in proptest::collection::vec((any::<proptest::sample::Index>(), any::<u8>()), 1..12),
            cut in any::<proptest::sample::Index>(),
            truncate in any::<bool>(),
            kern in any::<bool>(),
            gpos in any::<bool>(),
        ) {
            let mut bytes = font_bytes(Options { kern, gpos, ..Options::default() });
            for (at, value) in changes {
                let at = at.index(bytes.len());
                bytes[at] = value;
            }
            if truncate {
                bytes.truncate(cut.index(bytes.len()));
            }
            exercise(bytes);
        }

        /// Pure noise, which should simply be refused.
        #[test]
        fn noise_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            exercise(bytes);
        }
    }
}

use std::time::{Duration, Instant};

use proptest::prelude::*;
use rsui_text::{Font, FontError, GlyphBitmap, OutlineSink, Rasterizer};
use support::{Options, font_bytes, plain_font, system_font_bytes, test_font};

#[test]
fn metrics_come_from_the_font() {
    let metrics = plain_font().metrics();
    assert_eq!(metrics.units_per_em, 1000);
    assert_eq!(metrics.ascent, 800);
    assert_eq!(metrics.descent, -200);
    assert_eq!(metrics.line_gap, 100);
}

#[test]
fn characters_map_to_glyphs_in_either_format() {
    for format_12 in [false, true] {
        let font = test_font(Options {
            format_12,
            ..Options::default()
        });
        assert_eq!(font.glyph('A'), Some(1), "format 12: {format_12}");
        assert_eq!(font.glyph('B'), Some(2));
        assert_eq!(font.glyph('D'), Some(5 - 1 + 1));
        assert_eq!(font.glyph(' '), Some(3));
        assert_eq!(font.glyph('E'), None);
        assert_eq!(font.glyph('@'), None);
    }
    assert_eq!(plain_font().glyph('😀'), None);
    let wide = test_font(Options {
        format_12: true,
        ..Options::default()
    });
    assert_eq!(wide.glyph('😀'), Some(2));
}

#[test]
fn advances_come_from_the_font() {
    let font = plain_font();
    assert_eq!(font.advance(1), 600);
    assert_eq!(font.advance(3), 250);
    assert_eq!(font.advance(5), 700);
    // Glyphs past the end of the table share the last advance.
    assert_eq!(font.advance(200), 700);
}

#[test]
fn kerning_comes_from_the_old_table() {
    let font = test_font(Options {
        kern: true,
        ..Options::default()
    });
    assert_eq!(font.kerning(1, 2), -100);
    assert_eq!(font.kerning(2, 1), 0);
    assert_eq!(font.kerning(1, 1), 0);
}

#[test]
fn kerning_comes_from_gpos_pairs_and_classes() {
    let font = test_font(Options {
        gpos: true,
        ..Options::default()
    });
    assert_eq!(font.kerning(1, 2), -50, "from a list of pairs");
    assert_eq!(
        font.kerning(1, 4),
        -30,
        "from the next list, if not in the first"
    );
    assert_eq!(font.kerning(2, 1), -70, "from classes");
    assert_eq!(font.kerning(2, 2), 0);
    assert_eq!(font.kerning(5, 1), 0);
}

#[test]
fn gpos_wins_over_the_old_table() {
    let font = test_font(Options {
        gpos: true,
        kern: true,
        ..Options::default()
    });
    assert_eq!(font.kerning(1, 2), -50);
}

#[derive(Default)]
struct Recorder(Vec<String>);

impl OutlineSink for Recorder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push(format!("M{x},{y}"));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push(format!("L{x},{y}"));
    }

    fn quad_to(&mut self, control_x: f32, control_y: f32, x: f32, y: f32) {
        self.0.push(format!("Q{control_x},{control_y} {x},{y}"));
    }

    fn close(&mut self) {
        self.0.push("Z".into());
    }
}

fn outline(glyph: u16) -> Vec<String> {
    let mut recorder = Recorder::default();
    plain_font().outline(glyph, &mut recorder);
    recorder.0
}

#[test]
fn straight_outlines_are_read_in_order() {
    assert_eq!(outline(2), ["M0,0", "L0,500", "L500,0", "L0,0", "Z"]);
}

#[test]
fn control_points_in_a_row_imply_points_between_them() {
    assert_eq!(
        outline(4),
        [
            "M400,0",
            "Q0,0 0,400",
            "Q0,800 400,800",
            "Q800,800 800,400",
            "Q800,0 400,0",
            "Z"
        ]
    );
}

#[test]
fn composite_glyphs_combine_their_parts() {
    assert_eq!(
        outline(5),
        [
            "M100,0", "L100,500", "L600,500", "L600,0", "L100,0", "Z", "M0,0", "L0,250", "L250,0",
            "L0,0", "Z"
        ]
    );
}

#[test]
fn a_glyph_with_no_outline_sends_nothing() {
    assert!(outline(0).is_empty());
    assert!(outline(3).is_empty());
    assert!(outline(9_999).is_empty());
}

fn render(glyph: u16, size: f32, shift: f32) -> GlyphBitmap {
    Rasterizer::new().rasterize(&plain_font(), glyph, size, shift)
}

#[test]
fn a_square_on_pixel_edges_is_fully_covered() {
    // 500 units at 10 pixels per 1000 units is exactly 5 pixels.
    let bitmap = render(1, 10.0, 0.0);
    assert_eq!((bitmap.left, bitmap.top), (0, -5));
    assert_eq!((bitmap.width, bitmap.height), (5, 5));
    assert!(bitmap.coverage.iter().all(|pixel| *pixel == 255));
}

#[test]
fn a_shift_between_pixels_covers_the_edges_by_half() {
    let bitmap = render(1, 10.0, 0.5);
    assert_eq!(bitmap.width, 6);
    let row = &bitmap.coverage[..6];
    assert_eq!(row, [128, 255, 255, 255, 255, 128]);
}

#[test]
fn a_triangle_covers_half_of_its_pixels_along_the_diagonal() {
    let bitmap = render(2, 8.0, 0.0);
    assert_eq!((bitmap.width, bitmap.height), (4, 4));
    let rows: Vec<&[u8]> = bitmap.coverage.chunks(4).collect();
    assert_eq!(rows[0], [128, 0, 0, 0]);
    assert_eq!(rows[1], [255, 128, 0, 0]);
    assert_eq!(rows[2], [255, 255, 128, 0]);
    assert_eq!(rows[3], [255, 255, 255, 128]);
}

/// The total area a bitmap covers, in pixels.
fn area(bitmap: &GlyphBitmap) -> f32 {
    bitmap
        .coverage
        .iter()
        .map(|pixel| f32::from(*pixel) / 255.0)
        .sum()
}

#[test]
fn a_curve_covers_the_area_of_the_shape_it_draws() {
    // Four curves inside a square of side 40. Each curve cuts off a corner, of the area
    // between a parabola and its two tangents, which is 20 * 20 / 6.
    let bitmap = render(4, 50.0, 0.0);
    let expected = 40.0 * 40.0 - 4.0 * (20.0 * 20.0 / 6.0);
    assert!(
        (area(&bitmap) - expected).abs() < expected * 0.01,
        "{} against {expected}",
        area(&bitmap)
    );
}

#[test]
fn the_same_shape_covers_the_same_area_wherever_it_lands() {
    for shift in [0.0, 0.1, 0.25, 0.5, 0.75, 0.9] {
        let bitmap = render(4, 37.3, shift);
        let reference = area(&render(4, 37.3, 0.0));
        assert!(
            (area(&bitmap) - reference).abs() < reference * 0.005,
            "shift {shift}"
        );
    }
}

#[test]
fn a_composite_glyph_draws_all_its_parts() {
    let bitmap = render(5, 10.0, 0.0);
    // The square from 1 to 6 pixels, and the triangle from 0 to 2.5 that overlaps it.
    assert_eq!((bitmap.left, bitmap.top), (0, -5));
    assert_eq!(bitmap.width, 6);
    assert_eq!(bitmap.coverage[bitmap.width as usize - 1], 255);
}

#[test]
fn glyphs_with_no_outline_have_an_empty_bitmap() {
    assert_eq!(render(3, 14.0, 0.0), GlyphBitmap::default());
    assert_eq!(render(9_999, 14.0, 0.0), GlyphBitmap::default());
}

#[test]
fn a_bad_size_or_a_huge_glyph_has_an_empty_bitmap() {
    for size in [0.0, -1.0, f32::NAN, f32::INFINITY, 1_000_000.0] {
        assert_eq!(render(1, size, 0.0), GlyphBitmap::default(), "{size}");
    }
}

#[test]
fn a_font_with_postscript_outlines_says_so() {
    let mut bytes = font_bytes(Options::default());
    bytes[..4].copy_from_slice(b"OTTO");
    assert!(matches!(
        Font::from_bytes(bytes),
        Err(FontError::Unsupported(_))
    ));
}

#[test]
fn things_that_are_not_fonts_are_errors() {
    for bytes in [
        vec![],
        vec![1, 2, 3],
        vec![0; 100],
        b"<html>not a font</html>".to_vec(),
    ] {
        assert!(matches!(
            Font::from_bytes(bytes),
            Err(FontError::Invalid(_))
        ));
    }
}

#[test]
fn a_file_that_does_not_exist_is_an_error() {
    assert!(matches!(
        Font::open("C:/no/such/font.ttf"),
        Err(FontError::Io(_))
    ));
}

#[test]
fn a_font_can_be_opened_from_a_file_without_reading_it_into_memory() {
    let path = std::env::temp_dir().join(format!("rsui-test-font-{}.ttf", std::process::id()));
    std::fs::write(&path, font_bytes(Options::default())).unwrap();
    {
        let font = Font::open(&path).unwrap();
        assert_eq!(font.glyph('A'), Some(1));
        assert_eq!(Rasterizer::new().rasterize(&font, 1, 10.0, 0.0).width, 5);
    }
    // The file can be removed once the font is gone, so nothing is left holding it.
    std::fs::remove_file(&path).unwrap();
}

/// Does everything a font can do, as a damaged font might make any of it fail.
fn exercise(font: &Font, characters: &[char]) {
    let mut rasterizer = Rasterizer::new();
    let mut last = 0;
    for &character in characters {
        let glyph = font.glyph(character).unwrap_or(0);
        let _ = font.advance(glyph);
        let _ = font.kerning(last, glyph);
        let _ = rasterizer.rasterize(font, glyph, 14.0, 0.25);
        last = glyph;
    }
    for glyph in [0, 1, 2, 3, 4, 5, 6, 100, 65_535] {
        let _ = rasterizer.rasterize(font, glyph, 200.0, 0.0);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    /// A damaged font can do anything but crash or hang.
    #[test]
    fn damaging_a_font_never_panics(
        changes in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..40),
        cut in any::<prop::sample::Index>(),
        truncate in any::<bool>(),
    ) {
        let mut bytes = font_bytes(Options { kern: true, gpos: true, ..Options::default() });
        for (position, value) in changes {
            let position = position.index(bytes.len());
            bytes[position] = value;
        }
        if truncate {
            bytes.truncate(cut.index(bytes.len()));
        }
        if let Ok(font) = Font::from_bytes(bytes) {
            exercise(&font, &['A', 'B', 'C', 'D', ' ', 'é', '😀']);
        }
    }

    /// Damage aimed at where a glyph's numbers are, which are the most trusting reads.
    #[test]
    fn damaging_only_the_tables_never_panics(
        changes in prop::collection::vec((12usize..600, any::<u8>()), 1..30),
    ) {
        let mut bytes = font_bytes(Options { kern: true, gpos: true, format_12: true, ..Options::default() });
        for (position, value) in changes {
            if position < bytes.len() {
                bytes[position] = value;
            }
        }
        if let Ok(font) = Font::from_bytes(bytes) {
            exercise(&font, &['A', 'B', 'C', 'D', ' ', '😀']);
        }
    }
}

#[test]
fn composite_glyphs_that_refer_to_themselves_finish_quickly() {
    // A glyph of 100 references to itself would never end if they were all followed.
    let font = test_font(Options {
        endless_composite: true,
        ..Options::default()
    });
    let start = Instant::now();
    let bitmap = Rasterizer::new().rasterize(&font, 5, 14.0, 0.0);
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(
        bitmap,
        GlyphBitmap::default(),
        "it never reaches an outline"
    );
}

/// Real fonts have far more going on than the small test font.
#[test]
fn damaging_a_real_font_never_panics() {
    let Some(original) = system_font_bytes("arial.ttf") else {
        return;
    };
    let characters: Vec<char> = ('!'..='~').chain("éßñ€".chars()).collect();
    let mut state = 0x1234_5678_9ABC_DEF1u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state as usize
    };
    for _ in 0..30 {
        let mut bytes = original.clone();
        for _ in 0..2_000 {
            let position = next() % bytes.len();
            bytes[position] = next() as u8;
        }
        if let Ok(font) = Font::from_bytes(bytes) {
            exercise(&font, &characters);
        }
    }
}

#[test]
fn real_fonts_load_and_draw() {
    for name in ["segoeui.ttf", "arial.ttf", "consola.ttf"] {
        let Some(bytes) = system_font_bytes(name) else {
            continue;
        };
        let font = Font::from_bytes(bytes).unwrap();
        let glyph = font.glyph('g').expect("every font has g");
        assert!(font.advance(glyph) > 0);
        let bitmap = Rasterizer::new().rasterize(&font, glyph, 14.0, 0.0);
        assert!(bitmap.width > 3 && bitmap.height > 5, "{name}: {bitmap:?}");
        assert!(bitmap.coverage.iter().any(|pixel| *pixel > 200), "{name}");
    }
}
