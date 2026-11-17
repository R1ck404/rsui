//! Measuring text, breaking lines, and arranging glyphs.

mod support;

use proptest::prelude::*;
use rsui_core::{Element, Role};
use rsui_layout::{FixedMetrics, Layout, Measure, Size, Style};
use rsui_text::{AtlasGlyph, FontStyle, GlyphAtlas, GlyphKey, Rasterizer, TextEngine, TextStyle};
use support::{Options, plain_font, test_font};

/// 10 pixels per em, so `A` is 6 wide, `B` is 5, `C` is 8, `D` is 7, and space is 2.5.
/// Lines are 11 apart: 8 of ascent, 2 of descent and 1 of gap.
const STYLE: TextStyle = TextStyle::new(10.0);

fn engine() -> TextEngine {
    TextEngine::new(plain_font())
}

#[test]
fn revisiting_large_documents_after_cache_eviction_preserves_visible_text() {
    let documents = ["A\n", "BB\n", "CCC\n"].map(|line| line.repeat(100_000));
    let mut engine = engine();
    let mut layout = rsui_text::TextLayout::default();
    let visible = (550_000.0, 550_044.0);
    engine.layout_visible_into(&documents[0], STYLE, 100.0, visible, &mut layout);
    let expected_lines = layout.lines.clone();
    let expected_glyphs = layout.glyphs.clone();
    assert!(!expected_glyphs.is_empty());
    for text in &documents[1..] {
        engine.layout_visible_into(text, STYLE, 100.0, visible, &mut layout);
    }
    engine.layout_visible_into(&documents[0], STYLE, 100.0, visible, &mut layout);
    assert_eq!(layout.lines, expected_lines);
    assert_eq!(layout.glyphs, expected_glyphs);
}

fn lines_of(text: &str, max_width: f32) -> Vec<String> {
    let layout = engine().layout(text, STYLE, max_width);
    layout
        .lines
        .iter()
        .map(|line| text[line.start..line.end].to_owned())
        .collect()
}

#[test]
fn a_single_line_measures_the_sum_of_its_advances() {
    assert_eq!(
        engine().measure("ABC", STYLE, f32::INFINITY),
        Size::new(6.0 + 5.0 + 8.0, 11.0)
    );
}

/// 1 pixel per 10 units, with the test font's kerning.
fn kerned_engine() -> TextEngine {
    TextEngine::new(test_font(Options {
        kern: true,
        ..Options::default()
    }))
}

proptest! {
    /// A caret lines up with the glyph that follows it, and clicking there finds it again.
    #[test]
    fn carets_line_up_with_glyphs_and_clicks_find_them(text in "[ABCD é日]{0,12}") {
        let mut engine = kerned_engine();
        let layout = engine.layout(&text, STYLE, f32::INFINITY);
        for (glyph, (byte, _)) in layout.glyphs.iter().zip(text.char_indices()) {
            prop_assert_eq!(engine.caret_x(&text, STYLE, byte), glyph.x);
        }
        // A line's width leaves out the spaces at its end, but a caret can go after them.
        let end = engine.caret_x(&text, STYLE, text.trim_end().len());
        prop_assert_eq!(end, engine.measure(&text, STYLE, f32::INFINITY).width);

        let boundaries = text
            .char_indices()
            .map(|(byte, _)| byte)
            .chain(std::iter::once(text.len()));
        let mut previous = None;
        for byte in boundaries {
            let x = engine.caret_x(&text, STYLE, byte);
            // Characters of no width share a position, so the first of them is found.
            if previous != Some(x) {
                prop_assert_eq!(engine.byte_at_x(&text, STYLE, x), byte);
            }
            previous = Some(x);
        }
    }
}

proptest! {
    /// Finding lines and then placing them gives exactly what the one-pass route gives, across
    /// text that fits, text that doesn't, line breaks, and spaces at the end.
    #[test]
    fn every_way_of_laying_out_text_agrees(
        text in "[ABCD é日\\r\\n-]{0,30}",
        width in prop_oneof![Just(f32::INFINITY), 0.0f32..120.0],
    ) {
        let mut engine = kerned_engine();
        let expected = engine.layout(&text, STYLE, width);
        // A line break at the end forces the route that finds each line first, which everything
        // has to match, apart from the extra empty line that it adds.
        let slow = engine.layout(&format!("{text}\n"), STYLE, width);
        prop_assert_eq!(&slow.glyphs, &expected.glyphs);
        prop_assert_eq!(&slow.lines[..slow.lines.len() - 1], &expected.lines[..]);
        let mut reused = rsui_text::TextLayout::default();
        // Leftovers from something else mustn't show through.
        engine.layout_into("ABCD\nABCD", STYLE, 5.0, &mut reused);
        engine.layout_into(&text, STYLE, width, &mut reused);
        prop_assert_eq!(&reused, &expected);
        prop_assert_eq!(
            expected.width(),
            engine.measure(&text, STYLE, width).width
        );
        let height = engine.measure(&text, STYLE, width).height;
        prop_assert_eq!(expected.height(), height);
    }

    /// Long text placed by what's visible is the same as those lines placed from a full layout.
    #[test]
    fn showing_part_of_long_text_places_the_same_lines(
        extra in 0usize..2000,
        top in 0.0f32..2000.0,
        span in 1.0f32..400.0,
    ) {
        let text: String = "ABCD ABC\u{e9} ".chars().cycle().take(2100 + extra).collect();
        let mut engine = kerned_engine();
        let full = engine.layout(&text, STYLE, 100.0);
        let mut part = rsui_text::TextLayout::default();
        engine.layout_visible_into(&text, STYLE, 100.0, (top, top + span), &mut part);
        let first = (top / 11.0).floor() as usize;
        prop_assert!(!part.lines.is_empty() || first >= full.lines.len());
        for (offset, line) in part.lines.iter().enumerate() {
            let same = &full.lines[first + offset];
            prop_assert_eq!(
                (line.start, line.end, line.width, line.baseline),
                (same.start, same.end, same.width, same.baseline)
            );
            let glyphs = &part.glyphs[line.first_glyph as usize..][..line.glyph_count as usize];
            let expected = &full.glyphs[same.first_glyph as usize..][..same.glyph_count as usize];
            prop_assert_eq!(glyphs, expected);
        }
        // It's found again from memory, and gives the same answer.
        let mut again = rsui_text::TextLayout::default();
        engine.layout_visible_into(&text, STYLE, 100.0, (top, top + span), &mut again);
        prop_assert_eq!(&again, &part);
    }

    /// One line placed by what's across a range matches where a full layout puts those glyphs.
    #[test]
    fn showing_part_of_a_line_places_the_same_glyphs(
        text in "[ABCD é日]{1,60}",
        from in 0.0f32..200.0,
    ) {
        let mut engine = kerned_engine();
        let full = engine.layout(&text, STYLE, f32::INFINITY);
        let mut part = rsui_text::TextLayout::default();
        engine.layout_across_into(&text, STYLE, (from, from + 30.0), &mut part);
        for glyph in &part.glyphs {
            if let Some(same) = full.glyphs.iter().find(|same| same.x == glyph.x) {
                prop_assert_eq!(same, glyph);
            }
        }
        // Nothing in view is left out.
        let reach = STYLE.size * 2.0;
        for glyph in &full.glyphs {
            if glyph.x >= from - reach && glyph.x <= from + 30.0 + reach {
                prop_assert!(part.glyphs.contains(glyph), "missing {:?}", glyph);
            }
        }
    }
}

#[test]
fn caret_positions_agree_with_asking_one_at_a_time() {
    let mut engine = kerned_engine();
    let text = "ABCD é日 AB";
    let boundaries: Vec<usize> = text
        .char_indices()
        .map(|(byte, _)| byte)
        .chain(std::iter::once(text.len()))
        .collect();
    let mut positions: Vec<(usize, f32)> = boundaries.iter().map(|byte| (*byte, -1.0)).collect();
    engine.caret_positions(text, STYLE, &mut positions);
    for (byte, x) in positions {
        assert_eq!(x, engine.caret_x(text, STYLE, byte), "at {byte}");
    }
    let mut same_place = [(0, -1.0), (0, -1.0), (0, -1.0)];
    engine.caret_positions("", STYLE, &mut same_place);
    assert_eq!(same_place, [(0, 0.0), (0, 0.0), (0, 0.0)]);
}

#[test]
fn a_click_finds_the_nearest_caret() {
    let mut engine = engine();
    // `A` is 6 wide and `B` is 5.
    assert_eq!(engine.byte_at_x("AB", STYLE, -5.0), 0);
    assert_eq!(engine.byte_at_x("AB", STYLE, 2.9), 0);
    assert_eq!(engine.byte_at_x("AB", STYLE, 3.1), 1);
    assert_eq!(engine.byte_at_x("AB", STYLE, 8.4), 1);
    assert_eq!(engine.byte_at_x("AB", STYLE, 8.6), 2);
    assert_eq!(engine.byte_at_x("AB", STYLE, 500.0), 2);
    assert_eq!(engine.byte_at_x("", STYLE, 5.0), 0);
}

#[test]
fn a_text_input_is_one_line_high_and_does_not_depend_on_what_it_holds() {
    let empty = Element::new(Role::TextInput).prop("font_size", 10.0);
    let full = Element::new(Role::TextInput)
        .prop("font_size", 10.0)
        .prop("value", "A very long value");
    let mut engine = engine();
    let size = Measure::measure(&mut engine, &empty, f32::INFINITY);
    assert_eq!(size, Size::new(rsui_layout::TEXT_INPUT_WIDTH, 11.0));
    assert_eq!(Measure::measure(&mut engine, &full, f32::INFINITY), size);
    assert_eq!(Measure::measure(&mut engine, &full, 50.0).width, 50.0);
}

#[test]
fn empty_text_is_one_empty_line() {
    assert_eq!(
        engine().measure("", STYLE, f32::INFINITY),
        Size::new(0.0, 11.0)
    );
    assert_eq!(lines_of("", 100.0), [""]);
}

#[test]
fn line_breaks_start_new_lines() {
    assert_eq!(lines_of("A\nB", f32::INFINITY), ["A", "B"]);
    assert_eq!(lines_of("A\n", f32::INFINITY), ["A", ""]);
    assert_eq!(lines_of("\n\nA", f32::INFINITY), ["", "", "A"]);
    assert_eq!(lines_of("A\r\nB", f32::INFINITY), ["A", "B"]);
    assert_eq!(
        engine().measure("A\nAB", STYLE, f32::INFINITY),
        Size::new(11.0, 22.0)
    );
}

#[test]
fn lines_break_at_spaces_when_too_wide() {
    // "AA" is 12 wide, the space 2.5, and "AA AA" is 26.5.
    assert_eq!(lines_of("AA AA", 26.5), ["AA AA"]);
    assert_eq!(lines_of("AA AA", 26.0), ["AA", "AA"]);
    assert_eq!(lines_of("AA AA", 13.0), ["AA", "AA"]);
}

#[test]
fn spaces_at_the_end_of_a_line_do_not_count() {
    assert_eq!(engine().measure("AA   ", STYLE, f32::INFINITY).width, 12.0);
    // The spaces don't push the word onto a second line, however many there are.
    assert_eq!(lines_of("AA     ", 12.0), ["AA"]);
    assert_eq!(lines_of("AA     AA", 12.0), ["AA", "AA"]);
}

#[test]
fn a_word_too_long_for_a_line_is_split() {
    assert_eq!(lines_of("AAAAA", 12.0), ["AA", "AA", "A"]);
    assert_eq!(lines_of("AA AAAAA", 12.0), ["AA", "AA", "AA", "A"]);
}

#[test]
fn a_line_too_narrow_for_anything_still_moves_forward() {
    assert_eq!(lines_of("AB", 1.0), ["A", "B"]);
    assert_eq!(lines_of("AB", 0.0), ["A", "B"]);
}

#[test]
fn lines_can_break_after_hyphens() {
    assert_eq!(lines_of("AA-AA", 20.0), ["AA-", "AA"]);
}

#[test]
fn lines_can_break_between_ideographs() {
    // None of these are in the test font, so each uses its missing glyph, 3 wide.
    assert_eq!(lines_of("日本語", 6.0), ["日本", "語"]);
    assert_eq!(lines_of("日本語", 3.0), ["日", "本", "語"]);
}

#[test]
fn a_line_can_end_before_an_ideograph_that_follows_other_text() {
    // "AA A" is 20.5 wide, and the ideograph's missing glyph 3 more. Without a break before
    // it, the line would have to end at the space, taking the second A with it.
    assert_eq!(lines_of("AA A日", 21.0), ["AA A", "日"]);
}

/// Far more pairs than the cache has room for, so many of them share a slot.
#[test]
fn kerning_is_right_for_every_pair_however_they_share_the_cache() {
    let Some(bytes) = support::system_font_bytes("arial.ttf") else {
        return;
    };
    let font = rsui_text::Font::from_bytes(bytes.clone()).unwrap();
    let mut text = TextEngine::new(rsui_text::Font::from_bytes(bytes).unwrap());
    let units = f32::from(font.metrics().units_per_em);
    let style = TextStyle::new(units);
    for _ in 0..2 {
        for first in '!'..='~' {
            for second in '!'..='~' {
                let (a, b) = (font.glyph(first).unwrap(), font.glyph(second).unwrap());
                let expected = f32::from(font.advance(a))
                    + f32::from(font.advance(b))
                    + f32::from(font.kerning(a, b));
                let pair = format!("{first}{second}");
                let width = text.measure(&pair, style, f32::INFINITY).width;
                assert_eq!(width, expected, "{pair}");
            }
        }
    }
}

#[test]
fn glyphs_are_placed_along_the_baseline() {
    let layout = engine().layout("AB\nC", STYLE, f32::INFINITY);
    let placed: Vec<(u16, f32, f32)> = layout
        .glyphs
        .iter()
        .map(|glyph| (glyph.glyph, glyph.x, glyph.y))
        .collect();
    assert_eq!(placed, [(1, 0.0, 8.0), (2, 6.0, 8.0), (4, 0.0, 19.0)]);
    assert_eq!(layout.lines[0].glyph_count, 2);
    assert_eq!(layout.lines[1].first_glyph, 2);
    assert_eq!(layout.lines[1].baseline, 19.0);
    assert_eq!((layout.width(), layout.height()), (11.0, 22.0));
}

#[test]
fn a_character_the_font_lacks_uses_its_missing_glyph() {
    let layout = engine().layout("é", STYLE, f32::INFINITY);
    assert_eq!(layout.glyphs[0].glyph, 0);
    assert_eq!(layout.width(), 3.0);
}

#[test]
fn kerning_brings_pairs_closer_and_only_those() {
    let mut kerned = TextEngine::new(test_font(Options {
        kern: true,
        ..Options::default()
    }));
    // A 6 plus B 5, minus 1 of kerning.
    assert_eq!(kerned.measure("AB", STYLE, f32::INFINITY).width, 10.0);
    assert_eq!(kerned.measure("BA", STYLE, f32::INFINITY).width, 11.0);
    let layout = kerned.layout("AB", STYLE, f32::INFINITY);
    assert_eq!(layout.glyphs[1].x, 5.0);
    // A second time through the cache.
    assert_eq!(kerned.measure("AB", STYLE, f32::INFINITY).width, 10.0);
}

#[test]
fn kerning_does_not_cross_lines() {
    let mut kerned = TextEngine::new(test_font(Options {
        kern: true,
        ..Options::default()
    }));
    assert_eq!(kerned.measure("A\nB", STYLE, f32::INFINITY).width, 6.0);
    // A split word: B starts a line of its own, with nothing to kern against.
    let layout = kerned.layout("AB", STYLE, 6.0);
    assert_eq!(layout.lines.len(), 2);
    assert_eq!(layout.lines[1].width, 5.0);
}

#[test]
fn bold_text_uses_the_bold_font() {
    let mut text = TextEngine::new(plain_font()).with_bold(test_font(Options {
        kern: true,
        ..Options::default()
    }));
    assert_eq!(text.measure("AB", STYLE, f32::INFINITY).width, 11.0);
    assert_eq!(text.measure("AB", STYLE.bold(), f32::INFINITY).width, 10.0);
    assert_eq!(STYLE.bold().font, FontStyle::Bold);

    // With no bold font, regular takes its place.
    assert_eq!(
        engine().measure("AB", STYLE.bold(), f32::INFINITY).width,
        11.0
    );
}

#[test]
fn a_fallback_supplies_characters_the_main_font_lacks() {
    let wide = test_font(Options {
        format_12: true,
        ..Options::default()
    });
    let mut text = engine().with_fallback(wide);
    let layout = text.layout("A😀", STYLE, f32::INFINITY);
    assert_eq!((layout.glyphs[0].font, layout.glyphs[0].glyph), (0, 1));
    assert_eq!((layout.glyphs[1].font, layout.glyphs[1].glyph), (1, 2));
    assert_eq!(layout.width(), 11.0);
    assert!(text.font(1).is_some());
    assert!(text.font(2).is_none());
}

#[test]
fn elements_are_measured_by_their_name_and_style() {
    let mut text = engine();
    let measure = |text: &mut TextEngine, element: &Element, max_width| {
        Measure::measure(text, element, max_width)
    };
    let label = Element::new(Role::Text).name("AB");
    // The default size is 14 pixels: 8.4 + 7 rounds up to 16, with lines 15 or 16 apart.
    assert_eq!(measure(&mut text, &label, f32::INFINITY).width, 16.0);

    let large = Element::new(Role::Text).name("AB").prop("font_size", 100.0);
    assert_eq!(
        measure(&mut text, &large, f32::INFINITY),
        Size::new(110.0, 110.0)
    );
    let group = Element::new(Role::Group).name("AB");
    assert_eq!(
        measure(&mut text, &group, f32::INFINITY),
        Size::new(0.0, 0.0)
    );
    let empty = Element::new(Role::Text);
    assert_eq!(
        measure(&mut text, &empty, f32::INFINITY),
        Size::new(0.0, 0.0)
    );
}

#[test]
fn layout_uses_real_text_sizes() {
    let root = Element::new(Role::Window)
        .prop("direction", "row")
        .child(Element::new(Role::Text).name("AA").prop("font_size", 10.0))
        .child(
            Element::new(Role::Text)
                .name("AAAA AAAA")
                .prop("font_size", 10.0)
                .props(rsui_core::Props::from(Style::new().flex(1.0)))
                .prop("font_size", 10.0),
        )
        .into_node();
    let mut text = engine();
    let mut layout = Layout::new();
    layout.compute(&root, Size::new(42.0, 100.0), &mut text);
    assert_eq!(layout.rect(&[0]).unwrap().width, 12.0);
    assert_eq!(layout.rect(&[1]).unwrap().width, 30.0);
}

/// All the text a line can be made of in the test font, and some it can't.
fn any_text() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            8 => prop::sample::select(vec!['A', 'B', 'C', 'D']),
            3 => Just(' '),
            1 => prop::sample::select(vec!['\n', '-', 'é', '日', '\t']),
        ],
        0..80,
    )
    .prop_map(|characters| characters.into_iter().collect())
}

fn visible(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, ' ' | '\t' | '\n' | '\r'))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    /// Remembering where lines break changes nothing about where anything goes.
    #[test]
    fn remembered_lines_place_everything_just_as_finding_them_again_does(
        text in "[AB CD\n]{260,1500}",
        width in 40.0f32..400.0,
    ) {
        let mut engine = engine();
        let whole = engine.layout(&text, STYLE, width);
        let mut remembered = rsui_text::TextLayout::default();
        engine.layout_visible_into(&text, STYLE, width, (0.0, f32::MAX), &mut remembered);
        prop_assert_eq!(&remembered, &whole);
        // And again, now that it really is remembered.
        engine.layout_visible_into(&text, STYLE, width, (0.0, f32::MAX), &mut remembered);
        prop_assert_eq!(&remembered, &whole);
    }

    /// Whatever the text and width, no visible character is lost, duplicated or moved.
    #[test]
    fn lines_keep_every_character_in_order(text in any_text(), width in 0.0f32..200.0) {
        let layout = engine().layout(&text, STYLE, width);
        let joined: String = layout.lines.iter().map(|line| &text[line.start..line.end]).collect();
        prop_assert_eq!(visible(&joined), visible(&text));
        let starts_in_order = layout.lines.windows(2).all(|pair| pair[0].end <= pair[1].start);
        prop_assert!(starts_in_order);
    }

    /// A line only passes the limit if it's a single character that couldn't fit anyway.
    #[test]
    fn lines_stay_within_the_width_if_they_can(text in any_text(), width in 0.0f32..200.0) {
        let layout = engine().layout(&text, STYLE, width);
        for line in &layout.lines {
            let content = text[line.start..line.end].trim();
            prop_assert!(
                line.width <= width + 0.02 || content.chars().count() <= 1,
                "{content:?} is {} wide, past {width}", line.width
            );
        }
    }

    /// Measuring and laying out agree, since layout decides sizes from one and draws from
    /// the other.
    #[test]
    fn measuring_gives_the_size_of_the_layout(text in any_text(), width in 0.0f32..200.0) {
        let measured = engine().measure(&text, STYLE, width);
        let layout = engine().layout(&text, STYLE, width);
        prop_assert_eq!(measured, Size::new(layout.width(), layout.height()));
    }

    /// Text given the width it asked for must not wrap, even by a rounding error. If it
    /// did, layout would make room for one line and then draw two.
    #[test]
    fn text_fits_in_the_width_it_measured(
        text in any_text(),
        size in prop::sample::select(vec![9.0f32, 10.0, 11.0, 12.5, 13.0, 14.0, 17.3, 24.0]),
    ) {
        let style = TextStyle::new(size);
        let natural = engine().measure(&text, style, f32::INFINITY);
        let fitted = engine().measure(&text, style, natural.width);
        prop_assert_eq!(fitted, natural);
    }

    /// Glyphs on a line appear in order, each after the last, and within the line.
    #[test]
    fn glyphs_are_in_order_on_their_lines(text in any_text(), width in 0.0f32..200.0) {
        let layout = engine().layout(&text, STYLE, width);
        for line in &layout.lines {
            let first = line.first_glyph as usize;
            let glyphs = &layout.glyphs[first..first + line.glyph_count as usize];
            prop_assert!(glyphs.windows(2).all(|pair| pair[0].x <= pair[1].x));
            prop_assert!(glyphs.iter().all(|glyph| glyph.y == line.baseline));
        }
    }
}

fn atlas_glyph(atlas: &mut GlyphAtlas, glyph: u16, size: f32, x: f32) -> AtlasGlyph {
    let font = plain_font();
    atlas.glyph(
        GlyphKey::new(0, glyph, size, x),
        &font,
        &mut Rasterizer::new(),
    )
}

#[test]
fn the_atlas_draws_each_glyph_once() {
    let mut atlas = GlyphAtlas::new();
    let first = atlas_glyph(&mut atlas, 1, 10.0, 0.0);
    assert_eq!((first.width, first.height), (5, 5));
    assert_eq!((first.left, first.top), (0, -5));
    assert_eq!(atlas.take_uploads().len(), 1);

    assert_eq!(
        atlas_glyph(&mut atlas, 1, 10.0, 7.0),
        first,
        "whole pixels don't matter"
    );
    assert!(atlas.take_uploads().is_empty());
    assert_eq!(atlas.len(), 1);
}

#[test]
fn the_atlas_keeps_different_sizes_and_positions_apart() {
    let mut atlas = GlyphAtlas::new();
    let plain = atlas_glyph(&mut atlas, 1, 10.0, 0.0);
    let larger = atlas_glyph(&mut atlas, 1, 20.0, 0.0);
    let shifted = atlas_glyph(&mut atlas, 1, 10.0, 0.5);
    assert_ne!(plain, larger);
    assert_ne!(plain, shifted);
    assert_eq!(shifted.width, 6);
    assert_eq!(atlas.len(), 3);
}

#[test]
fn glyphs_with_no_pixels_take_no_room() {
    let mut atlas = GlyphAtlas::new();
    let space = atlas_glyph(&mut atlas, 3, 10.0, 0.0);
    assert_eq!((space.width, space.height), (0, 0));
    assert!(atlas.take_uploads().is_empty());
}

/// Whether two glyphs overlap or touch. They need a pixel between them, or sampling one
/// at its edge would pick up the other.
fn overlaps(a: &AtlasGlyph, b: &AtlasGlyph) -> bool {
    a.x < b.x + b.width + 1
        && b.x < a.x + a.width + 1
        && a.y < b.y + b.height + 1
        && b.y < a.y + a.height + 1
}

#[test]
fn glyphs_never_overlap_and_stay_inside_the_atlas_as_it_grows() {
    let mut atlas = GlyphAtlas::new();
    assert_eq!(atlas.size(), 256);
    let mut placed: Vec<AtlasGlyph> = Vec::new();
    // Different sizes, until the atlas has to grow.
    for step in 0..2_000 {
        let size = 8.0 + step as f32 * 0.05;
        let glyph = atlas_glyph(&mut atlas, 1, size, 0.0);
        assert_eq!(atlas.generation(), 0, "it hasn't had to start over yet");
        let inside = u32::from(glyph.x) + u32::from(glyph.width) <= atlas.size()
            && u32::from(glyph.y) + u32::from(glyph.height) <= atlas.size();
        assert!(inside, "{glyph:?} in {}", atlas.size());
        assert!(
            !placed.iter().any(|other| overlaps(other, &glyph)),
            "{glyph:?}"
        );
        placed.push(glyph);
        if atlas.size() >= 1024 {
            break;
        }
    }
    assert!(atlas.size() > 256, "this test should make the atlas grow");
}

#[test]
fn uploads_match_their_glyphs() {
    let mut atlas = GlyphAtlas::new();
    let placed = atlas_glyph(&mut atlas, 2, 16.0, 0.25);
    let uploads = atlas.take_uploads();
    assert_eq!(uploads.len(), 1);
    let upload = &uploads[0];
    assert_eq!(
        (upload.x, upload.y),
        (u32::from(placed.x), u32::from(placed.y))
    );
    assert_eq!(
        upload.coverage.len(),
        (upload.width * upload.height) as usize
    );
    assert_eq!(upload.width, u32::from(placed.width));
}

#[test]
fn when_full_the_atlas_starts_over_and_says_so() {
    let mut atlas = GlyphAtlas::new();
    let mut size = 300.0;
    while atlas.generation() == 0 {
        atlas_glyph(&mut atlas, 1, size, 0.0);
        size += 1.0;
        assert!(size < 2000.0, "it never filled up");
    }
    assert_eq!(atlas.len(), 1, "only the glyph that didn't fit is left");
    assert!(atlas.size() < 2048, "the atlas shrank back down");
}

#[test]
fn a_glyph_too_large_for_the_atlas_is_empty_rather_than_endless() {
    let mut atlas = GlyphAtlas::new();
    // Rendered at this size it would be larger than the largest bitmap.
    let glyph = atlas_glyph(&mut atlas, 1, 5_000.0, 0.0);
    assert_eq!((glyph.width, glyph.height), (0, 0));
    assert_eq!(atlas.generation(), 0);
}

#[test]
fn fixed_metrics_still_stand_in_for_real_text() {
    // Headless runs have no fonts, and use this instead.
    let mut metrics = FixedMetrics::default();
    let label = Element::new(Role::Text).name("AB");
    assert_eq!(
        metrics.measure(&label, f32::INFINITY),
        Size::new(16.0, 16.0)
    );
}

const LONG_LINE: &str = "AAAAAAAAAA BBBBBBBBBB AAAAAAAAAA";

#[test]
fn text_that_fits_on_one_line_is_left_as_it_is() {
    let mut engine = engine();
    let whole = engine.layout("AAA", STYLE, f32::INFINITY);
    let mut one_line = rsui_text::TextLayout::default();
    engine.layout_one_line_into("AAA", STYLE, 1000.0, true, &mut one_line);
    assert_eq!(one_line, whole);
}

#[test]
fn text_that_does_not_fit_is_cut_off_with_an_ellipsis_inside_the_limit() {
    let mut engine = engine();
    let whole = engine.layout(LONG_LINE, STYLE, f32::INFINITY);
    for limit in [10.0, 30.0, 60.0, 100.0, whole.width() - 1.0] {
        let mut one_line = rsui_text::TextLayout::default();
        engine.layout_one_line_into(LONG_LINE, STYLE, limit, true, &mut one_line);
        assert_eq!(one_line.lines.len(), 1);
        assert_eq!(one_line.lines[0].width, limit);
        assert!(one_line.glyphs.len() <= whole.glyphs.len(), "at {limit}");
        let last = one_line.glyphs.last().expect("the ellipsis at least");
        // The test font has no ellipsis, so what it draws for that is its missing glyph, 3 wide at this size's scale.
        assert_eq!(last.glyph, 0, "at {limit}");
        assert!(last.x <= limit, "at {limit}, it starts at {}", last.x);
    }
}

#[test]
fn without_an_ellipsis_it_is_just_cut_off() {
    let mut engine = engine();
    let mut one_line = rsui_text::TextLayout::default();
    engine.layout_one_line_into(LONG_LINE, STYLE, 50.0, false, &mut one_line);
    assert!(one_line.glyphs.iter().all(|glyph| glyph.glyph != 0));
    assert!(one_line.glyphs.iter().all(|glyph| glyph.x < 50.0));
    assert!(!one_line.glyphs.is_empty());
}

#[test]
fn text_kept_to_one_line_measures_as_one_line() {
    let mut engine = engine();
    let long = Element::new(Role::Text).name(LONG_LINE);
    let kept = long.clone().prop("ellipsis", true);
    let wrapped = Measure::measure(&mut engine, &long, 40.0);
    let one_line = Measure::measure(&mut engine, &kept, 40.0);
    assert!(
        wrapped.height > one_line.height,
        "{wrapped:?} and {one_line:?}"
    );
    assert_eq!(one_line.width, 40.0);
    let unlimited = Measure::measure(&mut engine, &kept, f32::INFINITY);
    assert!(unlimited.width > 40.0);
}

#[test]
fn right_to_left_text_keeps_its_start_when_it_is_cut_off() {
    let mut engine =
        TextEngine::new(rsui_text::Font::open("C:/Windows/Fonts/segoeui.ttf").unwrap());
    let text = "\u{05E9}\u{05DC}\u{05D5}\u{05DD} \u{05E2}\u{05D5}\u{05DC}\u{05DD} \u{05D0}\u{05D7}\u{05D3} \u{05E9}\u{05EA}\u{05D9}\u{05DD} \u{05E9}\u{05DC}\u{05D5}\u{05E9}";
    let whole = engine.layout(text, STYLE, f32::INFINITY);
    let limit = whole.width() / 2.0;
    let mut one_line = rsui_text::TextLayout::default();
    engine.layout_one_line_into(text, STYLE, limit, true, &mut one_line);
    assert!(one_line.glyphs.len() < whole.glyphs.len());
    assert!(
        one_line
            .glyphs
            .iter()
            .all(|glyph| glyph.x >= 0.0 && glyph.x <= limit),
        "{:?}",
        one_line.glyphs
    );
    assert_eq!(
        one_line.glyphs[0].x, 0.0,
        "the ellipsis is at its left, where it ends"
    );
}

#[test]
fn a_font_can_be_asked_for_by_name_and_what_is_not_there_is_the_usual_one() {
    let mut engine =
        TextEngine::new(rsui_text::Font::open("C:/Windows/Fonts/segoeui.ttf").unwrap());
    let usual = TextStyle::new(14.0);
    let chosen = |name: &str| {
        let mut style = TextStyle::new(14.0);
        style.family = Some(rsui_core::Symbol::new(name));
        style
    };
    let width = |engine: &mut TextEngine, style| engine.measure("iiii", style, f32::INFINITY).width;
    let plain = width(&mut engine, usual);
    assert_ne!(
        width(&mut engine, chosen("Consolas")),
        plain,
        "it's another font"
    );
    let (narrow, wide) = (
        engine
            .measure("iiii", chosen("Consolas"), f32::INFINITY)
            .width,
        engine
            .measure("WWWW", chosen("Consolas"), f32::INFINITY)
            .width,
    );
    assert_eq!(narrow, wide, "and that one has every letter the same width");
    assert_eq!(width(&mut engine, chosen("no such font")), plain);
    // The first of several that's there.
    assert_eq!(width(&mut engine, chosen("no such font, Consolas")), narrow);
}

#[test]
fn italic_text_comes_from_the_italic_font() {
    let mut engine =
        TextEngine::new(rsui_text::Font::open("C:/Windows/Fonts/segoeui.ttf").unwrap());
    let mut style = TextStyle::new(14.0);
    let regular = engine.layout("hello", style, f32::INFINITY);
    style.italic = true;
    let italic = engine.layout("hello", style, f32::INFINITY);
    assert_eq!(regular.glyphs[0].font, 0);
    assert_ne!(italic.glyphs[0].font, 0);
    let element = Element::new(Role::Text)
        .name("x")
        .prop("font_style", "italic")
        .prop("font_family", "Consolas");
    let read = TextStyle::from_element(&element);
    assert!(read.italic);
    assert_eq!(
        read.family.map(|family| family.as_str().to_owned()),
        Some("Consolas".to_owned())
    );
}

#[test]
fn text_twice_as_big_on_a_screen_twice_as_fine_has_lines_twice_as_far_apart() {
    let style = TextStyle {
        line_height: Some(20.0),
        ..STYLE
    };
    let engine = engine();
    assert_eq!(engine.line_height(style), 20.0);
    assert_eq!(engine.line_height(style.scaled(2.0)), 40.0);
    assert_eq!(style.scaled(2.0).size, 20.0);
}

#[test]
fn text_that_keeps_its_words_is_never_narrower_than_its_longest_one() {
    let mut engine = engine();
    let text = |keep: bool| {
        let element = Element::new(Role::Text)
            .name("AB ABCD")
            .prop("font_size", 10.0);
        if keep {
            element.prop("keep_words", true)
        } else {
            element
        }
    };
    // `ABCD` is 6 + 5 + 8 + 7 = 26 wide in the test font.
    assert_eq!(Measure::measure(&mut engine, &text(true), 0.0).width, 26.0);
    assert!(Measure::measure(&mut engine, &text(false), 0.0).width < 26.0);
}

#[test]
fn css_weights_come_to_the_fonts_there_most_often_are() {
    assert_eq!(FontStyle::of_weight("bold"), FontStyle::Bold);
    assert_eq!(FontStyle::of_weight("normal"), FontStyle::Regular);
    assert_eq!(FontStyle::of_weight("300"), FontStyle::Light);
    assert_eq!(FontStyle::of_weight("500"), FontStyle::Medium);
    assert_eq!(FontStyle::of_weight("600"), FontStyle::Semibold);
    assert_eq!(FontStyle::of_weight("700"), FontStyle::Bold);
    assert_eq!(FontStyle::of_weight("900"), FontStyle::Black);
    assert!(FontStyle::Semibold.is_heavy() && !FontStyle::Light.is_heavy());
}

#[test]
fn a_family_with_a_semibold_font_uses_it() {
    let mut text = engine();
    let style = |font| TextStyle {
        family: Some(rsui_core::Symbol::new("Segoe UI")),
        font,
        ..TextStyle::new(20.0)
    };
    let width = |text: &mut TextEngine, font| {
        text.measure("System font", style(font), f32::INFINITY)
            .width
    };
    let (regular, semibold, bold) = (
        width(&mut text, FontStyle::Regular),
        width(&mut text, FontStyle::Semibold),
        width(&mut text, FontStyle::Bold),
    );
    // Windows has Segoe UI in each of these, and each is wider than the one before.
    assert!(
        regular < semibold && semibold < bold,
        "{regular} {semibold} {bold}"
    );
}

#[test]
fn a_family_with_no_semibold_font_is_bold_instead() {
    let mut text = engine();
    let style = |font| TextStyle {
        family: Some(rsui_core::Symbol::new("Arial")),
        font,
        ..TextStyle::new(20.0)
    };
    let semibold = text
        .measure("Arial text", style(FontStyle::Semibold), f32::INFINITY)
        .width;
    let bold = text
        .measure("Arial text", style(FontStyle::Bold), f32::INFINITY)
        .width;
    assert_eq!(semibold, bold);
}

#[test]
fn a_medium_weight_is_semibold_where_a_family_has_that_and_regular_where_it_does_not() {
    let mut text = engine();
    let width = |text: &mut TextEngine, family: &str, font| {
        let style = TextStyle {
            family: Some(rsui_core::Symbol::new(family)),
            font,
            ..TextStyle::new(20.0)
        };
        text.measure("Medium text", style, f32::INFINITY).width
    };
    assert_eq!(
        width(&mut text, "Segoe UI", FontStyle::Medium),
        width(&mut text, "Segoe UI", FontStyle::Semibold)
    );
    assert_eq!(
        width(&mut text, "Arial", FontStyle::Medium),
        width(&mut text, "Arial", FontStyle::Regular)
    );
}

#[test]
fn a_fonts_lines_are_as_tall_as_windows_has_them() {
    // Segoe UI Emoji's own table says lines are shorter than Windows draws them, which is what a browser goes by.
    let emoji = rsui_text::Font::open("C:/Windows/Fonts/seguiemj.ttf").expect("Windows has it");
    let metrics = emoji.metrics();
    assert_eq!(
        (metrics.ascent, metrics.descent, metrics.line_gap),
        (2210, -514, 0)
    );
    // Arial's are the same either way, with its gap as it was.
    let arial = rsui_text::Font::open("C:/Windows/Fonts/arial.ttf").expect("Windows has it");
    let metrics = arial.metrics();
    assert_eq!(
        (metrics.ascent, metrics.descent, metrics.line_gap),
        (1854, -434, 67)
    );
}

#[test]
fn an_indented_first_line_starts_further_along_and_has_less_room() {
    let mut text = engine();
    let words = "one two three four five six seven eight nine ten";
    let plain = text.layout(words, STYLE, 120.0);
    let indented = text.layout(
        words,
        TextStyle {
            indent: 30.0,
            ..STYLE
        },
        120.0,
    );
    let first = indented.glyphs[indented.lines[0].first_glyph as usize].x;
    assert_eq!(first, plain.glyphs[0].x + 30.0);
    assert!(indented.lines[0].end <= plain.lines[0].end);
    assert!(indented.lines[0].width <= 120.0);
    // Only the first line is indented.
    let second = &indented.lines[1];
    assert!(indented.glyphs[second.first_glyph as usize].x < 1.0);
    let measured = text.measure(
        words,
        TextStyle {
            indent: 30.0,
            ..STYLE
        },
        120.0,
    );
    assert_eq!(
        measured.height,
        indented.lines.len() as f32 * text.line_height(STYLE)
    );
}

#[test]
fn lines_beside_what_floats_are_shorter_until_they_are_below_it() {
    let mut text = engine();
    let words = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen";
    let style = TextStyle {
        beside_left: (40.0, 1.0),
        beside_right: (20.0, 1.0),
        ..STYLE
    };
    let layout = text.layout(words, style, 150.0);
    let first = &layout.lines[0];
    assert!(layout.glyphs[first.first_glyph as usize].x >= 40.0);
    assert!(first.width <= 150.0 - 20.0);
    // What floats ends before the second line starts, so that's as long as the text is wide.
    let second = &layout.lines[1];
    assert!(layout.glyphs[second.first_glyph as usize].x < 1.0);
    let plain = text.layout(words, STYLE, 150.0);
    assert!(layout.lines.len() >= plain.lines.len());
    let measured = text.measure(words, style, 150.0);
    assert_eq!(
        measured.height,
        layout.lines.len() as f32 * text.line_height(STYLE)
    );
}

/// Letters spaced out, or drawn closer together, as CSS's `letter-spacing` has them, break lines where they're drawn: text
/// measured as one line fits on one line at that width, as a title as wide as its text is does.
#[test]
fn spaced_letters_break_lines_where_they_are_drawn() {
    for spacing in [-0.5f32, -0.25, 0.5, 1.0] {
        let mut text = engine();
        let mut style = STYLE;
        style.letter_spacing = spacing;
        // A is 6, B 5, C 8 and D 7, with spacing after each.
        let size = text.measure_spaced("ABCD", style, f32::INFINITY).unwrap();
        assert!((size.width - (26.0 + 4.0 * spacing)).abs() < 0.01, "{spacing}: {size:?}");
        let mut layout = rsui_text::TextLayout::default();
        text.layout_into("ABCD", style, size.width, &mut layout);
        assert_eq!(layout.lines.len(), 1, "{spacing}");
        // Any narrower and the last letter goes on the next line, in both.
        let narrower = size.width - 1.0;
        text.layout_into("ABCD", style, narrower, &mut layout);
        assert_eq!(layout.lines.len(), 2, "{spacing}");
        assert_eq!(text.measure_spaced("ABCD", style, narrower).unwrap().height, 2.0 * size.height, "{spacing}");
    }
}

/// `text-wrap: balance` breaks lines in the least room that keeps them as many, so they're as near the same length as they can
/// be, as Blink balances them, and measuring says the same as laying out.
#[test]
fn balanced_text_has_lines_of_about_the_same_length() {
    let mut engine = engine();
    let text = "Taxing Laughter: The Joke Tax Chronicles";
    let one_line = engine.layout(text, STYLE, f32::INFINITY).lines[0].width;
    let room = one_line * 0.8;
    let plain = engine.layout(text, STYLE, room);
    let balanced_style = TextStyle { balance: true, ..STYLE };
    let balanced = engine.layout(text, balanced_style, room);
    assert_eq!(plain.lines.len(), 2);
    assert_eq!(balanced.lines.len(), 2, "as many lines");
    let gap = |layout: &rsui_text::TextLayout| (layout.lines[0].width - layout.lines[1].width).abs();
    assert!(gap(&balanced) < gap(&plain), "{} vs {}", gap(&balanced), gap(&plain));
    assert!(gap(&balanced) < one_line * 0.25, "{}", gap(&balanced));
    let measured = engine.measure(text, balanced_style, room);
    let widest = balanced.lines.iter().map(|line| line.width).fold(0.0, f32::max);
    assert!((measured.width - widest).abs() < 0.5, "{} vs {widest}", measured.width);
    // Text on one line, and text on more lines than are balanced, are as they'd be.
    assert_eq!(engine.layout(text, balanced_style, f32::INFINITY).lines.len(), 1);
}
