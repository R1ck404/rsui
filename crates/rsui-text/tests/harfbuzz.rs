//! Text that needs shaping comes out the same as with HarfBuzz, the shaper fonts are mostly checked with.
//! What it made of this text is in files, so these need no other program, only the same font as was used
//! to make them. On a computer with a different one, they say so and pass.

use rsui_text::{Font, TextEngine, TextStyle};

const INDIC: &str = include_str!("fixtures/indic_harfbuzz.txt");
const EMOJI: &str = include_str!("fixtures/emoji_harfbuzz.txt");
const THAI: &str = include_str!("fixtures/thai_harfbuzz.txt");
const KHMER: &str = include_str!("fixtures/khmer_harfbuzz.txt");
const BURMESE: &str = include_str!("fixtures/burmese_harfbuzz.txt");
const JAVANESE: &str = include_str!("fixtures/javanese_harfbuzz.txt");
const BUGINESE: &str = include_str!("fixtures/buginese_harfbuzz.txt");
const CHAKMA: &str = include_str!("fixtures/chakma_harfbuzz.txt");
const MEETEI: &str = include_str!("fixtures/meetei_harfbuzz.txt");
const BRAHMI: &str = include_str!("fixtures/brahmi_harfbuzz.txt");

type Placed = (u16, i32, i32);

/// The font a file was made with, which its first line names along with how big it was.
fn font_of(file: &str) -> Option<Font> {
    let first = file.lines().next().expect("the file has a first line");
    let words: Vec<&str> = first.split_whitespace().collect();
    let at = words
        .iter()
        .position(|word| *word == "with")
        .expect("it names the font");
    let path = format!("C:/Windows/Fonts/{}", words[at + 1]);
    let size: u64 = words[at + 3].parse().expect("it says how big the font is");
    let found = std::fs::metadata(&path).ok()?.len();
    if found != size {
        eprintln!("skipped: this computer's {path} isn't the one the file was made with");
        return None;
    }
    Font::open(path).ok()
}

/// Pairs of text, and the glyphs and places HarfBuzz gave it.
fn cases(file: &'static str) -> Vec<(&'static str, Vec<Placed>)> {
    // Past the two lines that say what the file is.
    let lines: Vec<&str> = file.lines().skip(2).collect();
    lines
        .chunks(2)
        .map(|pair| {
            let numbers: Vec<i32> = pair[1]
                .split_whitespace()
                .map(|number| number.parse().expect("a number"))
                .collect();
            let glyphs = numbers
                .chunks(3)
                .map(|item| (item[0] as u16, item[1], item[2]))
                .collect();
            (pair[0], glyphs)
        })
        .collect()
}

fn check_all(file: &'static str) {
    let Some(font) = font_of(file) else {
        return;
    };
    let units = f32::from(font.metrics().units_per_em);
    let mut engine = TextEngine::new(font);
    for (text, wanted) in cases(file) {
        let layout = engine.layout(text, TextStyle::new(units), f32::INFINITY);
        // Glyphs that draw nothing aren't in the file, and don't matter. Spaces are the usual ones.
        let ours: Vec<Placed> = layout
            .glyphs
            .iter()
            .map(|glyph| (glyph.glyph, glyph.x.round() as i32, glyph.y.round() as i32))
            .filter(|item| wanted.iter().any(|other| other.0 == item.0))
            .collect();
        let glyphs = |list: &[Placed]| list.iter().map(|item| item.0).collect::<Vec<_>>();
        assert_eq!(glyphs(&ours), glyphs(&wanted), "the glyphs for {text:?}");
        // Ours goes down the page from the baseline, and theirs up from it.
        let baseline = ours.first().map_or(0, |item| item.2 + wanted[0].2);
        for (a, b) in ours.iter().zip(&wanted) {
            assert!(
                (a.1 - b.1).abs() <= 1 && (baseline - a.2 - b.2).abs() <= 1,
                "glyph {} of {text:?} is at {}, {} where it should be at {}, {}",
                a.0,
                a.1,
                baseline - a.2,
                b.1,
                b.2,
            );
        }
        engine.forget_caches();
    }
}

#[test]
fn indic_scripts_come_out_the_same() {
    check_all(INDIC);
}

#[test]
fn emoji_come_out_the_same() {
    check_all(EMOJI);
}

#[test]
fn thai_comes_out_the_same() {
    check_all(THAI);
}

#[test]
fn khmer_comes_out_the_same() {
    check_all(KHMER);
}

#[test]
fn burmese_comes_out_the_same() {
    check_all(BURMESE);
}

#[test]
fn the_rarer_scripts_come_out_the_same() {
    for file in [JAVANESE, BUGINESE, CHAKMA, MEETEI, BRAHMI] {
        check_all(file);
    }
}

#[test]
fn the_indic_file_covers_all_ten_scripts() {
    let ranges = [
        0x0900..=0x097F,
        0x0980..=0x09FF,
        0x0A00..=0x0A7F,
        0x0A80..=0x0AFF,
        0x0B00..=0x0B7F,
        0x0B80..=0x0BFF,
        0x0C00..=0x0C7F,
        0x0C80..=0x0CFF,
        0x0D00..=0x0D7F,
        0x0D80..=0x0DFF,
    ];
    let cases = cases(INDIC);
    for range in ranges {
        let found = cases
            .iter()
            .any(|case| case.0.chars().any(|c| range.contains(&(c as u32))));
        assert!(found, "nothing from {range:X?}");
    }
}

/// An engine with Nirmala and its size in its own units.
fn indic_engine() -> Option<(TextEngine, f32)> {
    let font = font_of(INDIC)?;
    let units = f32::from(font.metrics().units_per_em);
    Some((TextEngine::new(font), units))
}

#[test]
fn a_vowel_that_goes_first_is_drawn_before_its_consonant() {
    let Some((mut engine, units)) = indic_engine() else {
        return;
    };
    // Ka with the vowel I, whose sign goes before it, though it's typed after.
    let layout = engine.layout("\u{0915}\u{093F}", TextStyle::new(units), f32::INFINITY);
    let ka = engine.layout("\u{0915}", TextStyle::new(units), f32::INFINITY);
    assert_eq!(layout.glyphs.len(), 2);
    assert_eq!(layout.glyphs[1].glyph, ka.glyphs[0].glyph);
    assert!(layout.glyphs[0].x < layout.glyphs[1].x);
}

#[test]
fn a_caret_treats_a_whole_syllable_as_one_thing() {
    let Some((mut engine, units)) = indic_engine() else {
        return;
    };
    // Four characters of three bytes each make the first syllable, and then there's another letter.
    let text = "\u{0915}\u{094D}\u{0937}\u{093F}\u{0915}";
    let style = TextStyle::new(units);
    let width = engine.line_width(text, style);
    let steps = 200;
    for step in 0..=steps {
        let x = width * step as f32 / steps as f32;
        let byte = engine.byte_at_x(text, style, x);
        assert!(
            matches!(byte, 0 | 12 | 15),
            "at {x}, a caret is at byte {byte}"
        );
    }
}

#[test]
fn a_caret_treats_a_joined_emoji_as_one_thing() {
    let Some(font) = font_of(EMOJI) else {
        return;
    };
    let units = f32::from(font.metrics().units_per_em);
    let mut engine = TextEngine::new(font);
    let woman_with_a_computer = "\u{1F469}\u{200D}\u{1F4BB}";
    let text = format!("{woman_with_a_computer}x");
    let style = TextStyle::new(units);
    let width = engine.line_width(&text, style);
    for step in 0..=100 {
        let byte = engine.byte_at_x(&text, style, width * step as f32 / 100.0);
        let end = woman_with_a_computer.len();
        assert!(
            byte == 0 || byte == end || byte == end + 1,
            "a caret is at byte {byte}"
        );
    }
}

#[test]
fn broken_text_never_panics() {
    let Some((mut engine, _)) = indic_engine() else {
        return;
    };
    // Marks with nothing to go on, joiners everywhere, and halants in a row.
    let tries = [
        "\u{093F}",
        "\u{094D}\u{094D}\u{094D}",
        "\u{200D}\u{200C}\u{200D}",
        "\u{0930}\u{094D}",
        "\u{0930}\u{094D}\u{200D}",
        "\u{0CB0}\u{0CCD}\u{200D}",
        "\u{0D4A}\u{0D4A}\u{0D4A}\u{0D4A}\u{0D4A}\u{0D4A}",
        "\u{0915}\u{094D}\u{0915}\u{094D}\u{0915}\u{094D}\u{0915}\u{094D}\u{0915}\u{094D}\u{0915}\u{094D}\u{0915}",
        "\u{25CC}\u{093F}",
    ];
    for text in tries {
        let layout = engine.layout(text, TextStyle::new(16.0), f32::INFINITY);
        assert!(layout.width().is_finite(), "{text:?}");
    }
}
