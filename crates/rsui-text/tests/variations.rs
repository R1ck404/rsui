//! Fonts that vary along axes, in Windows' own Segoe UI Variable, which is skipped where it isn't installed. The widths
//! are what Edge measures, as `getBoundingClientRect`, for spans of each text at 100 pixels: Edge's widths are whole
//! 64ths of a pixel, rounded up, so these are within one of those.

use std::path::Path;

use rsui_core::Symbol;
use rsui_text::{Font, FontStyle, OutlineSink, TextEngine, TextStyle};

const FILE: &str = "C:/Windows/Fonts/SegUIVar.ttf";

fn installed() -> bool {
    Path::new(FILE).exists()
}

fn engine() -> TextEngine {
    TextEngine::new(Font::open("C:/Windows/Fonts/segoeui.ttf").expect("Windows has Segoe UI"))
}

fn style(family: &str, font: FontStyle) -> TextStyle {
    TextStyle {
        family: Some(Symbol::new(family)),
        font,
        ..TextStyle::new(100.0)
    }
}

fn width(engine: &mut TextEngine, text: &str, style: TextStyle) -> f32 {
    engine.layout(text, style, f32::INFINITY).width()
}

const TEXTS: [&str; 5] = ["1", "g", "Wg1", "abcdefghijklmnopqrstuvwxyz", "AVATAR To"];

/// Edge's widths of [`TEXTS`] in each family, at each weight.
const EDGE: &[(&str, FontStyle, [f32; 5])] = &[
    (
        "Segoe UI Variable Text",
        FontStyle::Light,
        [36.875, 56.015625, 178.46875, 1204.9375, 453.859375],
    ),
    (
        "Segoe UI Variable Text",
        FontStyle::Semilight,
        [37.40625, 57.421875, 182.375, 1243.421875, 461.53125],
    ),
    (
        "Segoe UI Variable Text",
        FontStyle::Regular,
        [37.890625, 58.890625, 186.28125, 1282.53125, 469.734375],
    ),
    (
        "Segoe UI Variable Text",
        FontStyle::Medium,
        [40.4375, 60.265625, 193.359375, 1332.140625, 489.265625],
    ),
    (
        "Segoe UI Variable Text",
        FontStyle::Semibold,
        [40.4375, 60.265625, 193.359375, 1332.140625, 489.265625],
    ),
    (
        "Segoe UI Variable Text",
        FontStyle::Bold,
        [43.515625, 61.921875, 202.015625, 1392.6875, 513.09375],
    ),
    (
        "Segoe UI Variable Text",
        FontStyle::Black,
        [43.515625, 61.921875, 202.015625, 1392.6875, 513.09375],
    ),
    (
        "Segoe UI Variable Display",
        FontStyle::Light,
        [34.328125, 54.203125, 170.078125, 1166.265625, 429.84375],
    ),
    (
        "Segoe UI Variable Display",
        FontStyle::Semilight,
        [36.046875, 55.96875, 176.90625, 1213.53125, 444.875],
    ),
    (
        "Segoe UI Variable Display",
        FontStyle::Regular,
        [37.453125, 58.703125, 183.359375, 1280.859375, 457.96875],
    ),
    (
        "Segoe UI Variable Display",
        FontStyle::Medium,
        [40.046875, 60.0625, 191.46875, 1333.453125, 479.0625],
    ),
    (
        "Segoe UI Variable Display",
        FontStyle::Semibold,
        [40.046875, 60.0625, 191.46875, 1333.453125, 479.0625],
    ),
    (
        "Segoe UI Variable Display",
        FontStyle::Bold,
        [43.171875, 61.71875, 201.28125, 1397.171875, 504.6875],
    ),
    (
        "Segoe UI Variable Small",
        FontStyle::Light,
        [38.390625, 59.140625, 185.65625, 1283.5, 469.875],
    ),
    (
        "Segoe UI Variable Small",
        FontStyle::Semilight,
        [38.921875, 60.15625, 188.734375, 1312.5625, 476.328125],
    ),
    (
        "Segoe UI Variable Small",
        FontStyle::Regular,
        [39.5625, 61.140625, 191.859375, 1340.4375, 483.40625],
    ),
    (
        "Segoe UI Variable Small",
        FontStyle::Medium,
        [41.75, 62.3125, 197.953125, 1384.1875, 500.421875],
    ),
    (
        "Segoe UI Variable Small",
        FontStyle::Semibold,
        [41.75, 62.3125, 197.953125, 1384.1875, 500.421875],
    ),
    (
        "Segoe UI Variable Small",
        FontStyle::Bold,
        [44.25, 63.671875, 204.984375, 1433.9375, 519.765625],
    ),
];

#[test]
fn named_places_measure_as_edge_has_them() {
    if !installed() {
        return;
    }
    let mut engine = engine();
    let mut wrong = Vec::new();
    for (family, weight, widths) in EDGE {
        for (text, edge) in TEXTS.iter().zip(widths) {
            let ours = width(&mut engine, text, style(family, *weight));
            // Within one 64th of a pixel, which is as near as Edge says, kerning too, which changes by fractions of the
            // font's units where it's set along its axes.
            let near = ours <= *edge + 0.001 && ours > *edge - 1.0 / 64.0 - 0.001;
            if !near {
                wrong.push(format!(
                    "{family} {weight:?} {text:?}: {ours} where Edge has {edge}"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn kerning_changes_by_fractions_of_a_unit_as_harfbuzz_has_it() {
    if !installed() {
        return;
    }
    // From HarfBuzz 15 (uharfbuzz 0.57), which Edge shapes with: the pair's first glyph's advance with `kern` on, less
    // without it, in units of the font's 2048, at 1024 times that so as not to round. Kerning in a font that varies changes
    // by what GDEF's store says, which HarfBuzz doesn't round, and nor does this; rounded, each was up to half a unit off.
    #[rustfmt::skip]
    let expected: &[(f32, f32, &str, f32)] = &[
        (300.0, 8.0, "AT", -129.273_44),
        (650.0, 36.0, "Ya", -193.5),
        (650.0, 36.0, "Ty", -86.5),
        (550.0, 5.0, "AT", -100.5),
        (500.0, 36.0, "f,", 67.5),
        (450.0, 14.0, "f,", 34.485_35),
        (450.0, 12.0, "Tr", -176.514_65),
        (500.0, 14.0, "Ya", -191.519_53),
        (700.0, 14.0, "Ty", -53.470_703),
        (300.0, 20.0, "Ty", -117.470_703),
        (550.0, 20.0, "f,", 104.534_18),
        // At the font's own places, whole units, as ever.
        (700.0, 36.0, "To", -200.0),
        (400.0, 10.5, "AT", -147.0),
    ];
    let mut font = Font::open(FILE).expect("it opens");
    let mut wrong = Vec::new();
    for &(weight, size, pair, harfbuzz) in expected {
        font.set_variations(&[(*b"wght", weight), (*b"opsz", size)]);
        let mut glyphs = pair.chars().map(|character| font.glyph(character).expect("it has this"));
        let (left, right) = (glyphs.next().unwrap(), glyphs.next().unwrap());
        let ours = font.kerning(left, right);
        if (ours - harfbuzz).abs() > 0.002 {
            wrong.push(format!("{pair} at {weight} {size}: {ours}, where HarfBuzz has {harfbuzz}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn tabular_digits_are_all_as_wide() {
    if !installed() {
        return;
    }
    let mut engine = engine();
    let tabular = TextStyle {
        tabular: true,
        ..style("Segoe UI Variable Text", FontStyle::Regular)
    };
    assert!((width(&mut engine, "1", tabular) - 53.90625).abs() < 1.0 / 64.0);
}

#[test]
fn the_variable_font_by_itself_is_not_a_family_to_a_browser() {
    if !installed() {
        return;
    }
    let mut engine = engine();
    // As in Edge, the name Windows lists it under is no family of its own, so the usual font is used instead.
    let usual = width(&mut engine, "Wg1", TextStyle::new(100.0));
    assert_eq!(
        width(
            &mut engine,
            "Wg1",
            style("Segoe UI Variable", FontStyle::Regular)
        ),
        usual
    );
    // And the next family listed is.
    let listed = width(
        &mut engine,
        "Wg1",
        style(
            "Segoe UI Variable, Segoe UI Variable Display",
            FontStyle::Regular,
        ),
    );
    assert!((listed - 183.359375).abs() < 1.0 / 64.0, "{listed}");
}

#[test]
fn each_place_is_a_face_of_its_own() {
    if !installed() {
        return;
    }
    let mut engine = engine();
    let mut faces = Vec::new();
    for family in ["Segoe UI Variable Text", "Segoe UI Variable Display"] {
        for weight in [FontStyle::Regular, FontStyle::Bold] {
            let layout = engine.layout("g", style(family, weight), f32::INFINITY);
            faces.push(layout.glyphs[0].font);
        }
    }
    // Glyphs are kept by face, so no two places share any.
    let mut distinct = faces.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), faces.len(), "{faces:?}");
    let font = engine.font(faces[3]).expect("it's open");
    let at: Vec<_> = font.variations().collect();
    assert_eq!(at, [(*b"wght", 700.0), (*b"opsz", 36.0)]);
}

/// The smallest box around every point of an outline, curves' control points too.
#[derive(Default)]
struct Bounds([f32; 4], bool);

impl Bounds {
    fn add(&mut self, x: f32, y: f32) {
        if !self.1 {
            self.0 = [x, y, x, y];
            self.1 = true;
        }
        self.0 = [
            self.0[0].min(x),
            self.0[1].min(y),
            self.0[2].max(x),
            self.0[3].max(y),
        ];
    }
}

impl OutlineSink for Bounds {
    fn move_to(&mut self, x: f32, y: f32) {
        self.add(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.add(x, y);
    }
    fn quad_to(&mut self, control_x: f32, control_y: f32, x: f32, y: f32) {
        self.add(control_x, control_y);
        self.add(x, y);
    }
    fn close(&mut self) {}
}

#[test]
fn outlines_move_as_fonttools_moves_them() {
    if !installed() {
        return;
    }
    // From fontTools' `getGlyphSet(location=...)`, its points' bounds and the advance before rounding. `é` is made of
    // two others, whose places move too.
    #[rustfmt::skip]
    let expected: &[(f32, f32, char, [f32; 4], f32)] = &[
        (700.0, 36.0, 'g', [62.0, -481.0, 1143.0, 1049.0], 1264.0),
        (700.0, 36.0, 'W', [16.0, 0.0, 2038.0, 1434.0], 2054.0),
        (700.0, 36.0, '1', [16.0, 0.0, 684.0, 1466.0], 884.0),
        (700.0, 36.0, '\u{e9}', [62.0, -25.0, 1043.0, 1509.0], 1113.0),
        (700.0, 36.0, '&', [65.0, -23.0, 1670.0, 1458.0], 1693.0),
        (300.0, 8.0, 'g', [102.273, -481.455, 1036.182, 1074.909], 1210.636),
        (300.0, 8.0, 'W', [49.636, 0.0, 1833.364, 1434.0], 1884.818),
        (300.0, 8.0, '1', [54.0, 0.0, 532.364, 1466.0], 786.364),
        (300.0, 8.0, '\u{e9}', [101.818, -25.455, 991.727, 1566.0], 1088.545),
        (300.0, 8.0, '&', [166.909, -24.091, 1468.909, 1458.0], 1519.273),
        (600.0, 10.5, 'g', [85.0, -482.0, 1083.999, 1048.0], 1233.999),
        (600.0, 10.5, 'W', [20.0, 0.0, 1953.998, 1434.0], 1977.998),
        (600.0, 10.5, '1', [17.0, 0.0, 597.998, 1466.0], 827.998),
        (600.0, 10.5, '\u{e9}', [85.0, -24.0, 1017.999, 1530.0], 1087.999),
        (600.0, 10.5, '&', [150.002, -23.0, 1429.004, 1458.0], 1465.005),
    ];
    let mut font = Font::open(FILE).expect("it opens");
    for &(weight, size, character, bounds, advance) in expected {
        font.set_variations(&[(*b"wght", weight), (*b"opsz", size)]);
        let glyph = font.glyph(character).expect("it has this");
        let mut found = Bounds::default();
        font.outline(glyph, &mut found);
        for (ours, theirs) in found.0.iter().zip(bounds) {
            // fontTools works in floating point all through, where this rounds where the specification says to.
            assert!(
                (ours - theirs).abs() < 0.05,
                "{character} at {weight} {size}: {:?}, not {bounds:?}",
                found.0
            );
        }
        assert_eq!(
            f32::from(font.advance(glyph)),
            advance.round(),
            "{character} at {weight} {size}"
        );
    }
}

#[test]
fn at_its_defaults_a_font_is_as_it_was() {
    if !installed() {
        return;
    }
    let plain = Font::open(FILE).expect("it opens");
    let mut moved = Font::open(FILE).expect("it opens");
    moved.set_variations(&[(*b"wght", 700.0)]);
    assert!(moved.is_varied());
    moved.set_variations(&[(*b"wght", 400.0), (*b"opsz", 10.5)]);
    assert!(!moved.is_varied());
    let glyph = plain.glyph('g').expect("it has g");
    let (mut a, mut b) = (Bounds::default(), Bounds::default());
    plain.outline(glyph, &mut a);
    moved.outline(glyph, &mut b);
    assert_eq!(a.0, b.0);
    assert_eq!(plain.advance(glyph), moved.advance(glyph));
    assert_eq!(plain.metrics(), moved.metrics());
    let axes: Vec<_> = plain
        .variation_axes()
        .iter()
        .map(|axis| (axis.tag, axis.min, axis.default, axis.max))
        .collect();
    assert_eq!(
        axes,
        [(*b"wght", 300.0, 400.0, 700.0), (*b"opsz", 5.0, 10.5, 36.0)]
    );
    // Past the end of an axis is its end.
    moved.set_variations(&[(*b"wght", 2000.0)]);
    assert_eq!(moved.variations().next(), Some((*b"wght", 700.0)));
}

#[test]
fn a_font_that_does_not_vary_has_no_axes() {
    let Ok(mut font) = Font::open("C:/Windows/Fonts/segoeui.ttf") else {
        return;
    };
    assert!(font.variation_axes().is_empty());
    let glyph = font.glyph('g').expect("it has g");
    let advance = font.advance(glyph);
    font.set_variations(&[(*b"wght", 700.0)]);
    assert!(!font.is_varied());
    assert_eq!(font.advance(glyph), advance);
}
