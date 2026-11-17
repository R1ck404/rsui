//! Fonts for other languages only cost anything once there's text that needs them, and emoji come in color.

mod support;

use std::ops::RangeInclusive;
use std::path::Path;

use rsui_text::{ColorLayer, Font, TextEngine, TextStyle};
use support::plain_font;

const HAN: &[RangeInclusive<u32>] = &[0x4E00..=0x9FFF];
const STYLE: TextStyle = TextStyle::new(16.0);

/// Whether this computer has the font, to test with.
fn system_font_path(name: &str) -> Option<String> {
    let path = format!("C:/Windows/Fonts/{name}");
    Path::new(&path).exists().then_some(path)
}

fn font_count(engine: &TextEngine) -> u16 {
    (0..).find(|index| engine.font(*index).is_none()).unwrap()
}

#[test]
fn a_lazy_font_isnt_opened_for_text_that_doesnt_need_it() {
    let Some(path) = system_font_path("msyh.ttc") else {
        return;
    };
    let mut engine = TextEngine::new(plain_font()).with_lazy_fallback(path, HAN);
    assert_eq!(font_count(&engine), 1);
    engine.layout("ABCD", STYLE, f32::INFINITY);
    assert_eq!(font_count(&engine), 1, "plain text");
    engine.layout("caf\u{E9} \u{0410}", STYLE, f32::INFINITY);
    assert_eq!(font_count(&engine), 1, "other languages");
}

#[test]
fn a_lazy_font_is_opened_by_the_first_text_that_needs_it() {
    let Some(path) = system_font_path("msyh.ttc") else {
        return;
    };
    let mut engine = TextEngine::new(plain_font()).with_lazy_fallback(path, HAN);
    let layout = engine.layout("\u{6F22}", STYLE, f32::INFINITY);
    assert_eq!(font_count(&engine), 2);
    assert_eq!(layout.glyphs[0].font, 1);
    assert_ne!(layout.glyphs[0].glyph, 0);
}

#[test]
fn every_way_of_asking_about_text_opens_the_font() {
    let Some(path) = system_font_path("msyh.ttc") else {
        return;
    };
    let fresh = || TextEngine::new(plain_font()).with_lazy_fallback(path.clone(), HAN);
    let text = "\u{6F22}\u{5B57}";
    let mut engine = fresh();
    let measured = engine.measure(text, STYLE, f32::INFINITY);
    assert_eq!(font_count(&engine), 2, "measure");
    let mut engine = fresh();
    assert_eq!(engine.line_width(text, STYLE), measured.width, "line_width");
    let mut engine = fresh();
    assert_eq!(
        engine.caret_x(text, STYLE, text.len()),
        measured.width,
        "caret_x"
    );
    let mut engine = fresh();
    assert_eq!(
        engine.byte_at_x(text, STYLE, measured.width),
        text.len(),
        "byte_at_x"
    );
    let mut engine = fresh();
    assert_eq!(
        engine.wrapped_starts(text, STYLE, 1.0).len(),
        2,
        "wrapped_starts"
    );
}

#[test]
fn a_lazy_font_isnt_opened_for_what_the_main_font_has() {
    let (Some(segoe), Some(path)) = (
        system_font_path("segoeui.ttf"),
        system_font_path("msyh.ttc"),
    ) else {
        return;
    };
    const ACCENTS: &[RangeInclusive<u32>] = &[0xC0..=0xFF];
    let mut engine = TextEngine::new(Font::open(segoe).unwrap()).with_lazy_fallback(path, ACCENTS);
    engine.layout("caf\u{E9}", STYLE, f32::INFINITY);
    assert_eq!(font_count(&engine), 1);
}

#[test]
fn whats_joined_to_an_emoji_stays_with_it_though_a_font_before_it_has_it() {
    let (Some(segoe), Some(_), Some(han)) = (
        system_font_path("segoeui.ttf"),
        system_font_path("seguiemj.ttf"),
        system_font_path("msyh.ttc"),
    ) else {
        return;
    };
    // This font comes first, for anything at all, and has a sign for female by itself.
    let mut engine = TextEngine::new(Font::open(segoe).unwrap())
        .with_fallback(Font::open(han).unwrap())
        .with_system_fallbacks();
    let alone = engine.layout("\u{2640}", STYLE, f32::INFINITY).glyphs[0].font;
    assert!(
        !engine.font(alone).unwrap().has_color_layers(),
        "this needs a font before the emoji to have the sign"
    );
    // Someone running, a joiner, and that sign, with nothing after it to say it's an emoji.
    let layout = engine.layout("\u{1F3C3}\u{200D}\u{2640}", STYLE, f32::INFINITY);
    assert_eq!(layout.glyphs.len(), 1, "{:?}", layout.glyphs);
    assert!(
        engine
            .font(layout.glyphs[0].font)
            .unwrap()
            .has_color_layers()
    );
}

/// An engine with the system's fonts, and which font a bit of text ends up in, in each weight.
fn fonts_of(text: &str) -> Option<(u16, u16, TextEngine)> {
    let segoe = system_font_path("segoeui.ttf")?;
    let bold = system_font_path("segoeuib.ttf")?;
    let mut engine = TextEngine::new(Font::open(segoe).ok()?)
        .with_bold(Font::open(bold).ok()?)
        .with_system_fallbacks();
    let regular = engine.layout(text, STYLE, f32::INFINITY).glyphs[0].font;
    let heavy = engine.layout(text, STYLE.bold(), f32::INFINITY).glyphs[0].font;
    Some((regular, heavy, engine))
}

#[test]
fn bold_chinese_uses_a_bold_font() {
    if system_font_path("msyhbd.ttc").is_none() {
        return;
    }
    let Some((regular, heavy, engine)) = fonts_of("\u{6F22}") else {
        return;
    };
    assert_ne!(regular, heavy);
    assert_ne!(heavy, 1, "the main bold font doesn't have it");
    assert!(engine.font(heavy).is_some());
}

#[test]
fn bold_hindi_uses_the_bold_font_in_the_same_file_as_the_regular_one() {
    if system_font_path("Nirmala.ttc").is_none() {
        return;
    }
    let Some((regular, heavy, _)) = fonts_of("\u{0915}") else {
        return;
    };
    assert_ne!(regular, heavy);
}

#[test]
fn text_that_isnt_bold_never_opens_a_bold_font() {
    let Some(segoe) = system_font_path("segoeui.ttf") else {
        return;
    };
    let Some(bold) = system_font_path("segoeuib.ttf") else {
        return;
    };
    let mut engine = TextEngine::new(Font::open(segoe).unwrap())
        .with_bold(Font::open(bold).unwrap())
        .with_system_fallbacks();
    engine.layout("\u{6F22}", STYLE, f32::INFINITY);
    assert_eq!(
        font_count(&engine),
        3,
        "the two main ones, and one for Chinese"
    );
}

#[test]
fn bold_text_still_gets_emoji_with_no_bold_font_for_them() {
    let Some((_, heavy, engine)) = fonts_of("\u{1F600}") else {
        return;
    };
    assert!(engine.font(heavy).unwrap().has_color_layers());
}

#[test]
fn a_font_is_only_used_for_what_it_was_added_for() {
    let Some(path) = system_font_path("msyh.ttc") else {
        return;
    };
    let mut engine = TextEngine::new(plain_font()).with_lazy_fallback(path, HAN);
    assert_eq!(
        engine.layout("\u{6F22}", STYLE, f32::INFINITY).glyphs[0].font,
        1
    );
    // That font has this too, but wasn't added for it.
    let other = engine.layout("\u{E9}", STYLE, f32::INFINITY).glyphs[0];
    assert_eq!((other.font, other.glyph), (0, 0));
}

#[test]
fn a_font_for_flags_leaves_other_emoji_alone() {
    let (Some(segoe), Some(_), Some(han)) = (
        system_font_path("segoeui.ttf"),
        system_font_path("seguiemj.ttf"),
        system_font_path("msyh.ttc"),
    ) else {
        return;
    };
    // There's no font with flags here to try this with, so this one stands in for it, for what it does have.
    const SMILES: &[RangeInclusive<u32>] = &[0x4E00..=0x9FFF];
    let mut engine = TextEngine::new(Font::open(segoe).unwrap())
        .with_lazy_fallback(han, SMILES)
        .with_system_fallbacks();
    let before = engine.layout("\u{1F600}", STYLE, f32::INFINITY).glyphs[0].font;
    engine.layout("\u{6F22}", STYLE, f32::INFINITY);
    let after = engine.layout("\u{1F600}", STYLE, f32::INFINITY).glyphs[0].font;
    assert_eq!(before, after);
}

#[test]
fn a_font_that_wont_open_is_passed_over() {
    let Some(path) = system_font_path("msyh.ttc") else {
        return;
    };
    let mut engine = TextEngine::new(plain_font())
        .with_lazy_fallback("C:/Windows/Fonts/there-is-no-such-font.ttf", HAN)
        .with_lazy_fallback(path, HAN);
    let layout = engine.layout("\u{6F22}", STYLE, f32::INFINITY);
    assert_eq!(layout.glyphs[0].font, 1);
    assert_ne!(layout.glyphs[0].glyph, 0);
}

#[test]
fn no_font_at_all_still_draws_something() {
    let mut engine = TextEngine::new(plain_font())
        .with_lazy_fallback("C:/Windows/Fonts/there-is-no-such-font.ttf", HAN);
    let layout = engine.layout("\u{6F22}", STYLE, f32::INFINITY);
    assert_eq!(layout.glyphs[0].glyph, 0, "the font's missing glyph");
}

#[test]
fn fonts_already_in_use_keep_their_numbers() {
    let (Some(han), Some(hangul)) = (system_font_path("msyh.ttc"), system_font_path("malgun.ttf"))
    else {
        return;
    };
    const HANGUL: &[RangeInclusive<u32>] = &[0xAC00..=0xD7FF];
    let mut engine = TextEngine::new(plain_font())
        .with_lazy_fallback(han, HAN)
        .with_lazy_fallback(hangul, HANGUL);
    let first = engine.layout("\u{D55C}", STYLE, f32::INFINITY).glyphs[0];
    let second = engine.layout("\u{6F22}", STYLE, f32::INFINITY).glyphs[0];
    let again = engine.layout("\u{D55C}", STYLE, f32::INFINITY).glyphs[0];
    assert_ne!(first.font, second.font);
    assert_eq!(first.font, again.font);
    assert_eq!(first.glyph, again.glyph);
}

#[test]
fn the_system_fonts_have_every_language_they_should() {
    let Some(segoe) = system_font_path("segoeui.ttf") else {
        return;
    };
    let mut engine = TextEngine::new(Font::open(segoe).unwrap()).with_system_fallbacks();
    let hello = [
        ("Chinese", "\u{4F60}\u{597D}"),
        ("Japanese", "\u{3053}\u{3093}\u{306B}\u{3061}\u{306F}"),
        ("Korean", "\u{C548}\u{B155}"),
        ("Hindi", "\u{0928}\u{092E}\u{0938}\u{094D}\u{0924}\u{0947}"),
        (
            "Tamil",
            "\u{0BB5}\u{0BA3}\u{0B95}\u{0BCD}\u{0B95}\u{0BAE}\u{0BCD}",
        ),
        ("Thai", "\u{0E2A}\u{0E27}\u{0E31}\u{0E2A}\u{0E14}\u{0E35}"),
        (
            "Lao",
            "\u{0EAA}\u{0EB0}\u{0E9A}\u{0EB2}\u{0E8D}\u{0E94}\u{0EB5}",
        ),
        ("Khmer", "\u{179F}\u{17BD}\u{179F}\u{17D2}\u{178F}\u{17B8}"),
        (
            "Burmese",
            "\u{1019}\u{1004}\u{103A}\u{1039}\u{1002}\u{101C}\u{102C}\u{1015}\u{102B}",
        ),
        ("Amharic", "\u{1230}\u{120B}\u{121D}"),
        ("Tibetan", "\u{0F56}\u{0F7C}\u{0F51}"),
        ("Emoji", "\u{1F600}"),
        ("Arabic", "\u{0645}\u{0631}\u{062D}\u{0628}\u{0627}"),
    ];
    for (language, text) in hello {
        let layout = engine.layout(text, STYLE, f32::INFINITY);
        assert!(!layout.glyphs.is_empty(), "{language}");
        for glyph in &layout.glyphs {
            assert_ne!(glyph.glyph, 0, "{language} has a missing glyph");
        }
    }
}

#[test]
fn an_emoji_is_made_of_colored_shapes() {
    let Some(path) = system_font_path("seguiemj.ttf") else {
        return;
    };
    let font = Font::open(path).unwrap();
    assert!(font.has_color_layers());
    let glyph = font.glyph('\u{1F600}').unwrap();
    let layers: Vec<ColorLayer> = font.color_layers(glyph).collect();
    assert!(layers.len() >= 2, "{layers:?}");
    assert!(layers.iter().all(|layer| layer.glyph != 0));
    let colors: Vec<_> = layers.iter().filter_map(|layer| layer.color).collect();
    assert!(colors.len() >= 2, "{layers:?}");
    assert!(
        colors.windows(2).any(|pair| pair[0] != pair[1]),
        "more than one color"
    );
}

/// Windows's emoji font, and a picture of a smiling face in it, at this size.
fn smiling_face(size: f32) -> Option<rsui_text::ColorBitmap> {
    let font = Font::open(system_font_path("seguiemj.ttf")?).ok()?;
    let glyph = font.glyph('\u{1F600}')?;
    rsui_text::Rasterizer::new().rasterize_color(&font, glyph, size, [0, 0, 0, 255])
}

#[test]
fn an_emoji_is_drawn_with_gradients() {
    let Some(picture) = smiling_face(64.0) else {
        return;
    };
    assert_eq!(
        picture.pixels.len(),
        picture.width as usize * picture.height as usize * 4
    );
    let mut colors: Vec<&[u8]> = picture.pixels.chunks_exact(4).collect();
    colors.sort_unstable();
    colors.dedup();
    assert!(colors.len() > 300, "only {} colors", colors.len());
    // The corners are empty, as it's round, and its middle isn't.
    assert_eq!(&picture.pixels[..4], &[0, 0, 0, 0]);
    let middle = (picture.height / 2 * picture.width + picture.width / 2) as usize * 4;
    assert_eq!(picture.pixels[middle + 3], 255);
}

/// Pictures of a face, a rocket and a rainbow from Windows's emoji font. They were drawn by this code when
/// it had been compared with what Edge draws from the same font, and they keep it that way.
const EMOJI_PICTURES: &[u8] = include_bytes!("fixtures/emoji_pictures.bin");
const EMOJI_FONT_SIZE: u64 = 12_934_920;

#[test]
fn emoji_come_out_the_way_they_were_checked() {
    let Some(path) = system_font_path("seguiemj.ttf") else {
        return;
    };
    if std::fs::metadata(&path).unwrap().len() != EMOJI_FONT_SIZE {
        eprintln!("skipped: this computer has a different emoji font");
        return;
    }
    let font = Font::open(path).unwrap();
    let mut raster = rsui_text::Rasterizer::new();
    let mut rest = EMOJI_PICTURES;
    let number = |rest: &mut &[u8]| {
        let (first, others) = rest.split_at(4);
        *rest = others;
        i32::from_le_bytes(first.try_into().unwrap())
    };
    for (character, size) in [
        ('\u{1F600}', 32.0),
        ('\u{1F680}', 32.0),
        ('\u{1F308}', 24.0),
    ] {
        let glyph = font.glyph(character).unwrap();
        let picture = raster
            .rasterize_color(&font, glyph, size, [0, 0, 0, 255])
            .unwrap();
        let (width, height) = (number(&mut rest), number(&mut rest));
        let (left, top) = (number(&mut rest), number(&mut rest));
        assert_eq!(picture.width as i32, width, "{character}");
        assert_eq!(picture.height as i32, height, "{character}");
        assert_eq!((picture.left, picture.top), (left, top), "{character}");
        let (wanted, others) = rest.split_at((width * height * 4) as usize);
        rest = others;
        let worst = picture
            .pixels
            .iter()
            .zip(wanted)
            .map(|(a, b)| a.abs_diff(*b))
            .max();
        assert!(worst <= Some(1), "{character} is off by {worst:?}");
    }
}

#[test]
fn a_damaged_color_table_does_no_harm() {
    let Some(path) = system_font_path("seguiemj.ttf") else {
        return;
    };
    let whole = std::fs::read(path).unwrap();
    let count = u16::from_be_bytes([whole[4], whole[5]]) as usize;
    let table = (0..count).find_map(|index| {
        let record = 12 + index * 16;
        (&whole[record..record + 4] == b"COLR").then(|| {
            let at = |offset: usize| {
                u32::from_be_bytes(
                    whole[record + offset..record + offset + 4]
                        .try_into()
                        .unwrap(),
                )
            };
            at(8) as usize
        })
    });
    let Some(table) = table else {
        return;
    };
    let glyph = Font::from_bytes(whole.clone())
        .unwrap()
        .glyph('\u{1F600}')
        .unwrap();
    let mut bytes = whole;
    let mut seed = 12345u32;
    let mut random = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as usize
    };
    // Each try changes a few bytes near where the table starts, and in its drawing steps.
    for _ in 0..40 {
        let changed: Vec<(usize, u8)> = (0..32)
            .map(|_| {
                let at = table + random() % 300_000;
                let old = bytes[at];
                bytes[at] = random() as u8;
                (at, old)
            })
            .collect();
        if let Ok(font) = Font::from_bytes(bytes.clone()) {
            let mut raster = rsui_text::Rasterizer::new();
            let _ = raster.rasterize_color(&font, glyph, 16.0, [0, 0, 0, 255]);
        }
        for (at, old) in changed.into_iter().rev() {
            bytes[at] = old;
        }
    }
}

#[test]
fn colors_never_go_past_how_opaque_they_are() {
    let Some(picture) = smiling_face(24.0) else {
        return;
    };
    for pixel in picture.pixels.chunks_exact(4) {
        assert!(
            pixel[0] <= pixel[3] && pixel[1] <= pixel[3] && pixel[2] <= pixel[3],
            "{pixel:?}"
        );
    }
}

#[test]
fn an_emoji_at_twice_the_size_is_twice_as_big() {
    let (Some(small), Some(big)) = (smiling_face(32.0), smiling_face(64.0)) else {
        return;
    };
    assert!(big.width.abs_diff(small.width * 2) <= 4);
    assert!(big.height.abs_diff(small.height * 2) <= 4);
}

#[test]
fn only_glyphs_with_gradients_are_drawn_that_way() {
    let Some(path) = system_font_path("seguiemj.ttf") else {
        return;
    };
    let font = Font::open(path).unwrap();
    let mut raster = rsui_text::Rasterizer::new();
    assert!(
        raster
            .rasterize_color(&font, 0, 16.0, [0, 0, 0, 255])
            .is_none()
    );
    assert!(
        raster
            .rasterize_color(&plain_font(), 1, 16.0, [0, 0, 0, 255])
            .is_none()
    );
    let glyph = font.glyph('\u{1F600}').unwrap();
    assert!(
        raster
            .rasterize_color(&font, glyph, 0.0, [0, 0, 0, 255])
            .is_none()
    );
    assert!(
        raster
            .rasterize_color(&font, glyph, 100_000.0, [0, 0, 0, 255])
            .is_none()
    );
}

#[test]
fn most_glyphs_and_most_fonts_have_no_colored_shapes() {
    let font = plain_font();
    assert!(!font.has_color_layers());
    assert_eq!(font.color_layers(1).count(), 0);
    let Some(path) = system_font_path("seguiemj.ttf") else {
        return;
    };
    let emoji = Font::open(path).unwrap();
    assert_eq!(emoji.color_layers(0).count(), 0, "the missing glyph");
    assert_eq!(emoji.color_layers(u16::MAX).count(), 0);
}

/// Whether any line of this starts with something that can't start one: a sign on what's before it.
fn lines_of(engine: &mut TextEngine, text: &str, width: f32) -> Vec<usize> {
    engine.wrapped_starts(text, STYLE, width)
}

#[test]
fn thai_breaks_between_words_with_no_spaces_to_go_by() {
    let Some(segoe) = system_font_path("segoeui.ttf") else {
        return;
    };
    if system_font_path("LeelawUI.ttf").is_none()
        || !Path::new("C:/Windows/System32/icu.dll").exists()
    {
        return;
    }
    let mut engine = TextEngine::new(Font::open(segoe).unwrap()).with_system_fallbacks();
    let word = "\u{0E2A}\u{0E27}\u{0E31}\u{0E2A}\u{0E14}\u{0E35}";
    let text = word.repeat(12);
    let one = engine.line_width(word, STYLE);
    // Each of these fits some whole words, and never half of one.
    for words in [1.5f32, 2.5, 3.5] {
        let starts = lines_of(&mut engine, &text, one * words);
        assert!(starts.len() > 1, "it wraps");
        for start in starts {
            assert_eq!(
                start % word.len(),
                0,
                "a line starts at byte {start} at {words} words"
            );
        }
    }
}

#[test]
fn lines_never_start_with_a_sign_for_what_was_before_them() {
    let Some(segoe) = system_font_path("segoeui.ttf") else {
        return;
    };
    let mut engine = TextEngine::new(Font::open(segoe).unwrap()).with_system_fallbacks();
    let texts = [
        "\u{0E19}\u{0E49}\u{0E33}\u{0E15}\u{0E01}\u{0E2A}\u{0E27}\u{0E31}\u{0E2A}\u{0E14}\u{0E35}\u{0E04}\u{0E23}\u{0E31}\u{0E1A}\u{0E20}\u{0E32}\u{0E29}\u{0E32}\u{0E44}\u{0E17}\u{0E22}",
        "\u{0EAA}\u{0EB0}\u{0E9A}\u{0EB2}\u{0E8D}\u{0E94}\u{0EB5}\u{0E9E}\u{0EB2}\u{0EAA}\u{0EB2}\u{0EA5}\u{0EB2}\u{0EA7}",
        "\u{1797}\u{17B6}\u{179F}\u{17B6}\u{1781}\u{17D2}\u{1798}\u{17C2}\u{179A}\u{179F}\u{17BD}\u{179F}\u{17D2}\u{178F}\u{17B8}",
        "\u{1019}\u{103C}\u{1014}\u{103A}\u{1019}\u{102C}\u{1018}\u{102C}\u{101E}\u{102C}\u{1005}\u{1000}\u{102C}\u{1038}",
    ];
    for text in texts {
        let whole = engine.line_width(text, STYLE);
        for parts in 2..6 {
            for start in lines_of(&mut engine, text, whole / parts as f32) {
                let first = text[start..].chars().next().unwrap();
                assert!(
                    !rsui_text::is_mark(first),
                    "{text:?} has a line starting {first:?}"
                );
            }
        }
    }
}

#[test]
fn text_that_doesnt_wrap_has_nothing_to_break() {
    let Some(segoe) = system_font_path("segoeui.ttf") else {
        return;
    };
    let mut engine = TextEngine::new(Font::open(segoe).unwrap()).with_system_fallbacks();
    let text = "\u{0E2A}\u{0E27}\u{0E31}\u{0E2A}\u{0E14}\u{0E35}";
    assert_eq!(engine.wrapped_starts(text, STYLE, f32::INFINITY), vec![0]);
}
