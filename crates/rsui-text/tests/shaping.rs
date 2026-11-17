//! Text that needs shaping, in real fonts, since those have the rules. These are skipped if the
//! fonts aren't there.

use std::path::Path;
use std::time::{Duration, Instant};

use proptest::prelude::*;
use rsui_text::{Font, TextEngine, TextLayout, TextStyle};

const STYLE: TextStyle = TextStyle::new(20.0);

fn system_font(name: &str) -> Option<Font> {
    Font::open(Path::new("C:/Windows/Fonts").join(name)).ok()
}

fn engine() -> Option<TextEngine> {
    system_font("segoeui.ttf").map(TextEngine::new)
}

fn glyph_of(engine: &TextEngine, character: char) -> u16 {
    engine
        .font(0)
        .and_then(|font| font.glyph(character))
        .expect("the font has this")
}

fn glyphs_of(engine: &mut TextEngine, text: &str) -> Vec<u16> {
    let layout = engine.layout(text, STYLE, f32::INFINITY);
    layout.glyphs.iter().map(|glyph| glyph.glyph).collect()
}

#[test]
fn text_comes_out_the_same_whichever_way_it_goes() {
    const TEXT: &str = "AVATAR To Wave Yo, the quick brown bear jumps over a lazy dog";
    for name in ["segoeui.ttf", "arial.ttf", "tahoma.ttf", "times.ttf"] {
        let Some(font) = system_font(name) else {
            continue;
        };
        let mut engine = TextEngine::new(font);
        let plain = engine.layout(TEXT, STYLE, f32::INFINITY);
        // A character that shows nothing, but which needs the long way round.
        let shaped = engine.layout(&format!("\u{2060}{TEXT}"), STYLE, f32::INFINITY);
        assert_eq!(plain.glyphs.len(), shaped.glyphs.len(), "{name}");
        for (a, b) in plain.glyphs.iter().zip(&shaped.glyphs) {
            assert_eq!(a.glyph, b.glyph, "{name}");
            assert!((a.x - b.x).abs() < 0.01, "{name}: {} and {}", a.x, b.x);
            assert_eq!(a.y, b.y, "{name}");
        }
        assert!((plain.width() - shaped.width()).abs() < 0.01, "{name}");
    }
}

#[test]
fn arabic_letters_take_the_form_for_where_they_are() {
    let Some(mut engine) = engine() else { return };
    let alone = glyphs_of(&mut engine, "\u{0628}");
    let joined = glyphs_of(&mut engine, "\u{0628}\u{0628}\u{0628}");
    assert_eq!(joined.len(), 3);
    // From the left: its form at the end of a word, in the middle, and at the start.
    assert_ne!(joined[0], joined[1]);
    assert_ne!(joined[1], joined[2]);
    assert_ne!(joined[0], joined[2]);
    assert!(!joined.contains(&alone[0]));
    // Something that doesn't join stops it.
    let apart = glyphs_of(&mut engine, "\u{0628}\u{200C}\u{0628}");
    assert_eq!(apart, [alone[0], alone[0]]);
}

/// A font that draws lam and alef as one glyph, which Segoe UI doesn't.
fn arial() -> Option<TextEngine> {
    system_font("arial.ttf").map(TextEngine::new)
}

#[test]
fn each_position_in_a_word_gets_its_own_form() {
    let Some(mut engine) = arial() else { return };
    // Ain looks different in all four places, and Arial uses the same glyphs for it as for the
    // characters Unicode has for each form.
    let form = |engine: &TextEngine, code| glyph_of(engine, char::from_u32(code).expect("a form"));
    let (isolated, last, first, middle) = (
        form(&engine, 0xFEC9),
        form(&engine, 0xFECA),
        form(&engine, 0xFECB),
        form(&engine, 0xFECC),
    );
    assert_eq!(glyphs_of(&mut engine, "\u{0639}"), [isolated]);
    assert_eq!(glyphs_of(&mut engine, "\u{0639}\u{0639}"), [last, first]);
    assert_eq!(
        glyphs_of(&mut engine, "\u{0639}\u{0639}\u{0639}"),
        [last, middle, first]
    );
    // A mark in between doesn't get in the way, and a space does.
    let marked = glyphs_of(&mut engine, "\u{0639}\u{064E}\u{0639}");
    assert!(
        marked.contains(&last) && marked.contains(&first),
        "{marked:?}"
    );
    assert_eq!(
        glyphs_of(&mut engine, "\u{0639} \u{0639}"),
        [isolated, glyph_of(&engine, ' '), isolated]
    );
}

#[test]
fn lam_and_alef_become_one_unless_told_not_to() {
    let Some(mut engine) = arial() else { return };
    assert_eq!(glyphs_of(&mut engine, "\u{0644}\u{0627}").len(), 1);
    assert_eq!(glyphs_of(&mut engine, "\u{0644}\u{200C}\u{0627}").len(), 2);
    assert_eq!(glyphs_of(&mut engine, "\u{0644}\u{200D}\u{0627}").len(), 2);
}

#[test]
fn latin_ligatures_form_where_the_font_has_them() {
    let Some(font) = system_font("segoeui.ttf") else {
        return;
    };
    let mut engine = TextEngine::new(font);
    let ligature = glyphs_of(&mut engine, "fi");
    let apart = glyphs_of(&mut engine, "f\u{200C}i");
    assert_eq!(apart.len(), 2);
    assert!(ligature.len() <= 2);
    if ligature.len() == 1 {
        assert_ne!(ligature[0], apart[0]);
    }
}

#[test]
fn right_to_left_text_runs_from_the_right() {
    let Some(mut engine) = engine() else { return };
    // Alef, bet, gimel.
    let layout = engine.layout("\u{05D0}\u{05D1}\u{05D2}", STYLE, f32::INFINITY);
    let alef = glyph_of(&engine, '\u{05D0}');
    let gimel = glyph_of(&engine, '\u{05D2}');
    assert_eq!(layout.glyphs[0].glyph, gimel);
    assert_eq!(layout.glyphs[2].glyph, alef);
    assert!(layout.glyphs[0].x < layout.glyphs[2].x);
}

#[test]
fn a_line_with_both_directions_keeps_each_in_its_own_order() {
    let Some(mut engine) = engine() else { return };
    let text = "ab \u{05D0}\u{05D1} cd";
    let glyphs = glyphs_of(&mut engine, text);
    let expected: Vec<u16> = "ab \u{05D1}\u{05D0} cd"
        .chars()
        .map(|character| glyph_of(&engine, character))
        .collect();
    assert_eq!(glyphs, expected);
}

#[test]
fn brackets_are_mirrored_in_right_to_left_text() {
    let Some(mut engine) = engine() else { return };
    let glyphs = glyphs_of(&mut engine, "(\u{05D0})");
    assert_eq!(glyphs[0], glyph_of(&engine, '('));
    assert_eq!(glyphs[2], glyph_of(&engine, ')'));
    // In left-to-right text, they stay as they are.
    let glyphs = glyphs_of(&mut engine, "(\u{0301}a)");
    assert_eq!(glyphs.first(), Some(&glyph_of(&engine, '(')));
}

#[test]
fn marks_take_up_no_room_and_sit_on_their_letter() {
    let Some(mut engine) = engine() else { return };
    let bare = engine.measure("\u{05E9}", STYLE, f32::INFINITY);
    let marked = engine.measure("\u{05E9}\u{05B8}", STYLE, f32::INFINITY);
    assert!((bare.width - marked.width).abs() < 0.01);
    // A composed form, where the font has one, so that it isn't two glyphs at all.
    assert_eq!(
        glyphs_of(&mut engine, "e\u{0301}"),
        [glyph_of(&engine, '\u{E9}')]
    );
    // One it doesn't have stays apart, and stays over the letter.
    let layout = engine.layout("q\u{0307}", STYLE, f32::INFINITY);
    assert_eq!(layout.glyphs.len(), 2);
    assert!(layout.glyphs[1].x >= layout.glyphs[0].x);
    assert!(layout.glyphs[1].x < layout.lines[0].width);
    assert_eq!(layout.glyphs[1].y.fract(), 0.0, "text sits on whole pixels");
}

#[test]
fn a_mark_takes_no_room_even_if_its_glyph_says_it_does() {
    let marks = (0x0300..=0x0360u32)
        .chain(0x05B0..=0x05B9)
        .chain(0x064B..=0x0652);
    let mut found = 0;
    for name in [
        "segoeui.ttf",
        "arial.ttf",
        "times.ttf",
        "calibri.ttf",
        "tahoma.ttf",
        "micross.ttf",
    ] {
        let (Some(font), Some(bytes)) = (system_font(name), font_file(name)) else {
            continue;
        };
        // The font's own word for what's a mark is what counts.
        let with_room: Vec<char> = marks
            .clone()
            .filter_map(char::from_u32)
            .filter(|mark| {
                font.glyph(*mark)
                    .is_some_and(|glyph| font.advance(glyph) > 0 && glyph_is_a_mark(&bytes, glyph))
            })
            .collect();
        let mut engine = TextEngine::new(font);
        // A letter none of these compose with, so that the mark stays apart.
        let bare = engine.measure("q", STYLE, f32::INFINITY).width;
        for mark in with_room {
            let marked = engine
                .measure(&format!("q{mark}"), STYLE, f32::INFINITY)
                .width;
            assert!(
                (marked - bare).abs() < 0.5,
                "{name}, {mark:?}: {marked} and {bare}"
            );
            found += 1;
        }
    }
    if system_font("segoeui.ttf").is_some() {
        assert!(found > 0, "no font tested this");
    }
}

#[test]
fn characters_that_show_nothing_are_not_drawn() {
    let Some(mut engine) = engine() else { return };
    for invisible in [
        '\u{200B}', '\u{200D}', '\u{2060}', '\u{FEFF}', '\u{202A}', '\u{FE0F}',
    ] {
        let text = format!("a{invisible}b");
        assert_eq!(glyphs_of(&mut engine, &text).len(), 2, "{invisible:?}");
    }
}

#[test]
fn right_to_left_paragraphs_line_up_on_the_right() {
    let Some(mut engine) = engine() else { return };
    let layout = engine.layout("\u{05D0}\u{05D1}", STYLE, 300.0);
    let line = layout.lines[0];
    assert!((line.x + line.width - 300.0).abs() < 0.01);
    assert!(layout.glyphs[0].x >= line.x - 0.01);
    // Each paragraph goes by its own direction.
    let layout = engine.layout("abc\n\u{05D0}\u{05D1}", STYLE, 300.0);
    assert_eq!(layout.lines[0].x, 0.0);
    assert!(layout.lines[1].x > 0.0);
    // With no limit, it's within the widest line.
    let layout = engine.layout("abcdefgh\n\u{05D0}", STYLE, f32::INFINITY);
    let (first, second) = (layout.lines[0], layout.lines[1]);
    assert!((second.x + second.width - first.width).abs() < 0.01);
}

#[test]
fn a_long_right_to_left_paragraph_wraps_with_its_first_words_on_top() {
    let Some(mut engine) = engine() else { return };
    let word = "\u{05E9}\u{05DC}\u{05D5}\u{05DD}";
    let text = [word; 20].join(" ");
    let layout = engine.layout(&text, STYLE, 200.0);
    assert!(layout.lines.len() > 1);
    let size = engine.measure(&text, STYLE, 200.0);
    assert_eq!(size.width, layout.width());
    assert_eq!(size.height, layout.height());
    for (index, line) in layout.lines.iter().enumerate() {
        assert!(line.width <= 200.01, "line {index} is {}", line.width);
        if index > 0 {
            assert!(line.start >= layout.lines[index - 1].end);
        }
    }
    assert_eq!(layout.lines[0].start, 0);
    assert_eq!(layout.lines.last().unwrap().end, text.len());
}

#[test]
fn nothing_comes_between_a_letter_and_its_mark_when_a_word_is_split() {
    let Some(mut engine) = engine() else { return };
    let text = "\u{05E9}\u{05B8}".repeat(40);
    // The second is narrower than a letter, so each letter is alone on its line.
    for limit in [60.0, 3.0] {
        let layout = engine.layout(&text, STYLE, limit);
        assert!(layout.lines.len() > 1);
        for line in &layout.lines {
            let first = text[line.start..].chars().next().expect("a line has text");
            assert_eq!(first, '\u{05E9}', "a line starts with a letter at {limit}");
        }
    }
}

/// `abc` then the Hebrew letters that take bytes 3, 5 and 7.
const MIXED: &str = "abc\u{05D0}\u{05D1}\u{05D2}";

#[test]
fn a_caret_goes_where_the_text_it_comes_before_is() {
    let Some(mut engine) = engine() else { return };
    let x: Vec<f32> = [0, 1, 2, 3, 5, 7, 9]
        .iter()
        .map(|at| engine.caret_x(MIXED, STYLE, *at))
        .collect();
    // From the left, the Latin letters.
    assert!(x[0] < x[1] && x[1] < x[2]);
    // Before alef is its right side, the far right.
    assert!(x[3] > x[4] && x[4] > x[5]);
    // After gimel, which ends the text, is its left side, right after the Latin.
    assert!(x[6] > x[2] && x[6] < x[5]);
    assert!(x[3] > x[6]);
}

#[test]
fn the_arrow_keys_go_across_the_screen_whatever_order_the_text_is_in() {
    let Some(mut engine) = engine() else { return };
    let mut go = |at: usize, right: bool| engine.caret_across(MIXED, STYLE, at, right);
    let rightward = [0, 1, 2, 9, 7, 5, 3];
    for pair in rightward.windows(2) {
        assert_eq!(go(pair[0], true), Some(pair[1]), "from {}", pair[0]);
        assert_eq!(go(pair[1], false), Some(pair[0]), "from {}", pair[1]);
    }
    assert_eq!(go(3, true), Some(3), "it stops at the end");
    assert_eq!(go(0, false), Some(0), "and at the other");
}

#[test]
fn plain_text_has_no_special_order() {
    let Some(mut engine) = engine() else { return };
    assert_eq!(engine.caret_across("abc", STYLE, 1, true), None);
    assert!(!engine.is_right_to_left("abc", STYLE));
    assert!(engine.is_right_to_left("\u{05D0}bc", STYLE));
    assert!(!engine.is_right_to_left("a\u{05D0}", STYLE));
}

#[test]
fn a_selection_across_both_directions_is_in_two_pieces() {
    let Some(mut engine) = engine() else { return };
    // `c` and alef, which are next to each other in the text but not on the screen.
    let mut spans = Vec::new();
    engine.selection_spans(MIXED, STYLE, (2, 5), &mut spans);
    assert_eq!(spans.len(), 2, "{spans:?}");
    assert!(spans[0].1 < spans[1].0);
    let width = engine.line_width(MIXED, STYLE);
    assert!((spans[1].1 - width).abs() < 0.01);
    // The whole text is one.
    let mut spans = Vec::new();
    engine.selection_spans(MIXED, STYLE, (0, MIXED.len()), &mut spans);
    assert_eq!(spans.len(), 1, "{spans:?}");
    assert!(spans[0].0.abs() < 0.01 && (spans[0].1 - width).abs() < 0.01);
}

#[test]
fn a_caret_can_stop_inside_a_ligature() {
    let Some(mut engine) = arial() else { return };
    // Lam-alef, which is one glyph.
    let text = "\u{0644}\u{0627}";
    assert_eq!(glyphs_of(&mut engine, text).len(), 1);
    let (start, middle, end) = (
        engine.caret_x(text, STYLE, 0),
        engine.caret_x(text, STYLE, 2),
        engine.caret_x(text, STYLE, 4),
    );
    assert!(start > middle && middle > end, "{start} {middle} {end}");
    assert_eq!(engine.byte_at_x(text, STYLE, middle), 2);
}

#[test]
fn a_right_to_left_input_starts_from_the_right_while_it_fits() {
    let Some(mut engine) = engine() else { return };
    let width = engine.line_width("\u{05D0}", STYLE);
    assert!((engine.line_indent("\u{05D0}", STYLE, 100.0) - (100.0 - width)).abs() < 0.01);
    assert_eq!(engine.line_indent("abc", STYLE, 100.0), 0.0);
    assert_eq!(engine.line_indent("\u{05D0}", STYLE, width / 2.0), 0.0);
}

#[test]
fn spaces_after_right_to_left_text_still_move_the_caret() {
    let Some(mut engine) = engine() else { return };
    let without = engine.line_width("\u{05D0}", STYLE);
    let with = engine.line_width("\u{05D0}  ", STYLE);
    assert!(with > without);
    // They go to its left, so the caret ends up there.
    let caret = engine.caret_x("\u{05D0}  ", STYLE, "\u{05D0}  ".len());
    assert!(caret.abs() < 0.01, "{caret}");
}

#[test]
fn remembered_results_are_the_same_as_new_ones() {
    let Some(mut engine) = engine() else { return };
    let texts: Vec<String> = (0..600)
        .map(|index| format!("\u{05D0}{index} \u{0628}\u{0628}{index}"))
        .collect();
    let first: Vec<TextLayout> = texts
        .iter()
        .map(|text| engine.layout(text, STYLE, 500.0))
        .collect();
    for (text, expected) in texts.iter().zip(&first).rev() {
        assert_eq!(&engine.layout(text, STYLE, 500.0), expected);
    }
    engine.forget_caches();
    for (text, expected) in texts.iter().zip(&first) {
        assert_eq!(&engine.layout(text, STYLE, 500.0), expected);
    }
}

#[test]
fn long_text_only_shapes_the_lines_it_shows() {
    let Some(mut engine) = engine() else { return };
    let sentence =
        "\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645} \u{05E9}\u{05DC}\u{05D5}\u{05DD} Hello ";
    let text = sentence.repeat(200);
    let whole = engine.layout(&text, STYLE, 400.0);
    let size = engine.measure(&text, STYLE, 400.0);
    assert!(whole.lines.len() > 10);
    assert_eq!(size.height, whole.height());
    let line_height = whole.height() / whole.lines.len() as f32;

    let mut part = TextLayout::default();
    let first = 5;
    let span = (first as f32 * line_height, (first + 3) as f32 * line_height);
    engine.layout_visible_into(&text, STYLE, 400.0, span, &mut part);
    assert_eq!(part.lines.len(), 3);
    for (index, line) in part.lines.iter().enumerate() {
        let other = whole.lines[first + index];
        assert_eq!((line.start, line.end), (other.start, other.end));
        assert_eq!(line.baseline, other.baseline);
        assert!((line.x - other.x).abs() < 1.0, "{} and {}", line.x, other.x);
        let range = |line: &rsui_text::Line| {
            line.first_glyph as usize..(line.first_glyph + line.glyph_count) as usize
        };
        let shown: Vec<u16> = part.glyphs[range(line)].iter().map(|g| g.glyph).collect();
        let expected: Vec<u16> = whole.glyphs[range(&other)]
            .iter()
            .map(|g| g.glyph)
            .collect();
        assert_eq!(shown, expected, "line {index}");
    }

    // The same lines again, now remembered, come out exactly the same.
    let mut again = TextLayout::default();
    engine.layout_visible_into(&text, STYLE, 400.0, span, &mut again);
    assert_eq!(part, again);
    // And so do they with the view moved, whichever of them are remembered by then.
    let moved = (span.0 + line_height, span.1 + line_height);
    engine.layout_visible_into(&text, STYLE, 400.0, moved, &mut again);
    engine.forget_caches();
    let mut fresh = TextLayout::default();
    engine.layout_visible_into(&text, STYLE, 400.0, moved, &mut fresh);
    assert_eq!(again, fresh);
}

#[test]
fn long_text_with_ligatures_comes_out_right_too() {
    let Some(mut engine) = engine() else { return };
    let text = "office fish flag waffle ".repeat(500);
    let mut part = TextLayout::default();
    engine.layout_visible_into(&text, STYLE, 400.0, (0.0, 200.0), &mut part);
    assert!(part.lines.len() > 3);
    for line in &part.lines {
        assert!(line.width > 0.0);
        assert!(line.glyph_count > 0);
    }
}

#[test]
fn one_enormous_line_can_be_shaped() {
    let Some(mut engine) = engine() else { return };
    let text = "\u{0628}".repeat(20_000);
    let layout = engine.layout(&text, STYLE, f32::INFINITY);
    assert_eq!(layout.lines.len(), 1);
    assert_eq!(layout.glyphs.len(), 20_000);
    let end = engine.caret_x(&text, STYLE, text.len());
    assert!(end.abs() < 1.0, "{end}");
}

fn mixed_text() -> impl Strategy<Value = String> {
    let pieces = prop_oneof![
        Just("a"),
        Just("fi"),
        Just("AV"),
        Just(" "),
        Just("  "),
        Just("1"),
        Just("23"),
        Just("("),
        Just(")"),
        Just("."),
        Just("-"),
        Just("\u{05D0}"),
        Just("\u{05E9}\u{05C1}"),
        Just("\u{05B8}"),
        Just("\u{05BC}"),
        Just("\u{0628}"),
        Just("\u{0644}\u{0627}"),
        Just("\u{064E}"),
        Just("\u{0651}"),
        Just("\u{0640}"),
        Just("\u{0661}"),
        Just("\u{200C}"),
        Just("\u{200D}"),
        Just("\u{200F}"),
        Just("\u{2067}"),
        Just("\u{2069}"),
        Just("\u{202B}"),
        Just("\u{202C}"),
        Just("e\u{0301}"),
        Just("\u{0301}"),
        Just("\u{4E2D}"),
        Just("\u{1F600}"),
        Just("\u{0915}\u{094D}"),
    ];
    proptest::collection::vec(pieces, 0..24).prop_map(|pieces| pieces.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// A caret can be put at every stop by clicking there, and walking across visits each once.
    #[test]
    fn every_caret_stop_is_reachable(text in mixed_text()) {
        let Some(mut engine) = engine() else { return Ok(()) };
        let width = engine.line_width(&text, STYLE);
        // Walk to the far left, then back across it all.
        let mut at = text.len();
        let mut steps = 0;
        while let Some(next) = engine.caret_across(&text, STYLE, at, false) {
            if next == at {
                break;
            }
            at = next;
            steps += 1;
            prop_assert!(steps <= text.len() + 1, "it goes around in circles");
        }
        let mut seen = vec![at];
        let mut previous = engine.caret_x(&text, STYLE, at);
        while let Some(next) = engine.caret_across(&text, STYLE, at, true) {
            if next == at {
                break;
            }
            let x = engine.caret_x(&text, STYLE, next);
            prop_assert!(x >= previous - 0.01, "backwards at {next}: {x} after {previous}");
            prop_assert!(text.is_char_boundary(next));
            prop_assert!(!seen.contains(&next), "{next} came up twice");
            prop_assert!(x >= -0.01 && x <= width + 0.01, "{x} is outside {width}");
            seen.push(next);
            previous = x;
            at = next;
        }
        if engine.caret_across(&text, STYLE, 0, true).is_some() {
            prop_assert!(seen.contains(&0), "the start is one of them");
            prop_assert!(seen.contains(&text.len()), "and so is the end");
        }
    }

    /// Selecting everything covers everything, and the pieces never overlap.
    #[test]
    fn selections_cover_what_they_should(text in mixed_text(), from: prop::sample::Index, to: prop::sample::Index) {
        let Some(mut engine) = engine() else { return Ok(()) };
        let boundaries: Vec<usize> = text
            .char_indices()
            .map(|(at, _)| at)
            .chain([text.len()])
            .collect();
        let (a, b) = (boundaries[from.index(boundaries.len())], boundaries[to.index(boundaries.len())]);
        let width = engine.line_width(&text, STYLE);
        let mut spans = Vec::new();
        engine.selection_spans(&text, STYLE, (a.min(b), a.max(b)), &mut spans);
        for span in &spans {
            prop_assert!(span.0 <= span.1, "{spans:?}");
            prop_assert!(span.0 >= -0.01 && span.1 <= width + 0.01, "{spans:?} in {width}");
        }
        for pair in spans.windows(2) {
            prop_assert!(pair[0].1 <= pair[1].0 + 0.01, "{spans:?}");
        }
    }

    /// What's measured is what's laid out, and nothing is lost along the way.
    #[test]
    fn measuring_and_laying_out_agree(text in mixed_text(), limit in 1.0f32..400.0) {
        let Some(mut engine) = engine() else { return Ok(()) };
        for limit in [limit, f32::INFINITY] {
            let layout = engine.layout(&text, STYLE, limit);
            let size = engine.measure(&text, STYLE, limit);
            prop_assert_eq!(size.width, layout.width());
            prop_assert_eq!(size.height, layout.height());
            for line in &layout.lines {
                prop_assert!(line.start <= line.end && line.end <= text.len());
                prop_assert!(text.is_char_boundary(line.start) && text.is_char_boundary(line.end));
                prop_assert!(line.width.is_finite() && line.x.is_finite());
            }
            prop_assert!(layout.glyphs.iter().all(|glyph| glyph.x.is_finite() && glyph.y.is_finite()));
        }
        // A width that was measured fits when it comes back as the limit.
        let size = engine.measure(&text, STYLE, f32::INFINITY);
        if !text.contains('\n') {
            let again = engine.layout(&text, STYLE, size.width);
            prop_assert_eq!(again.lines.len(), 1, "wrapped at its own width of {}", size.width);
        }
    }

    /// Damage in the tables that drive all this.
    #[test]
    fn damaged_shaping_tables_never_panic_or_hang(
        changes in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..40),
        which in 0usize..3,
    ) {
        let Some(mut bytes) = font_file("arial.ttf") else { return Ok(()) };
        let tag: &[u8; 4] = [b"GSUB", b"GPOS", b"GDEF"][which];
        let range = table_range(&bytes, tag).expect("the font has these");
        for (at, value) in changes {
            let at = range.start + at.index(range.len().min(8192));
            bytes[at] = value;
        }
        let Ok(font) = Font::from_bytes(bytes) else { return Ok(()) };
        let mut engine = TextEngine::new(font);
        let started = Instant::now();
        for text in SAMPLES {
            engine.layout(text, STYLE, 100.0);
            engine.caret_x(text, STYLE, text.len());
        }
        prop_assert!(started.elapsed() < Duration::from_secs(20));
    }
}

const SAMPLES: [&str; 5] = [
    "office fish",
    "\u{0628}\u{0650}\u{0633}\u{0652}\u{0645}\u{0650} \u{0644}\u{0627} \u{0627}\u{0644}\u{0644}\u{0651}\u{064E}\u{0647}\u{0650}",
    "\u{05E9}\u{05B8}\u{05C1}\u{05DC}\u{05D5}\u{05B9}\u{05DD}",
    "e\u{0301} q\u{0323}\u{0307}",
    "abc \u{05D0} 123 (\u{0628})",
];

fn font_file(name: &str) -> Option<Vec<u8>> {
    std::fs::read(Path::new("C:/Windows/Fonts").join(name)).ok()
}

/// Whether a font's GDEF table classes this glyph as a mark.
fn glyph_is_a_mark(bytes: &[u8], glyph: u16) -> bool {
    let Some(range) = table_range(bytes, b"GDEF") else {
        return false;
    };
    let table = &bytes[range];
    let read = |at: usize| u16::from_be_bytes([table[at], table[at + 1]]);
    let classes = read(4) as usize;
    if classes == 0 {
        return false;
    }
    if read(classes) == 1 {
        let (first, count) = (read(classes + 2), read(classes + 4));
        return glyph >= first
            && glyph - first < count
            && read(classes + 6 + (glyph - first) as usize * 2) == 3;
    }
    (0..read(classes + 2) as usize).any(|index| {
        let record = classes + 4 + index * 6;
        read(record) <= glyph && glyph <= read(record + 2) && read(record + 4) == 3
    })
}

/// Where a table is in a font file.
fn table_range(bytes: &[u8], tag: &[u8; 4]) -> Option<std::ops::Range<usize>> {
    let count = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    (0..count).find_map(|index| {
        let record = 12 + index * 16;
        if &bytes[record..record + 4] != tag {
            return None;
        }
        let read = |at: usize| {
            u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize
        };
        let (start, length) = (read(record + 8), read(record + 12));
        Some(start..start + length)
    })
}

/// A GSUB table written to do as much damage as it can. Its one feature has a rule that calls itself
/// many times at every level, and a rule that turns one glyph into many, in a way that feeds both.
/// Without a budget, this would take longer than anyone would wait.
fn hostile_gsub(glyph: u16) -> Vec<u8> {
    fn words(out: &mut Vec<u8>, values: &[u16]) {
        for value in values {
            out.extend_from_slice(&value.to_be_bytes());
        }
    }
    let mut out = Vec::new();
    // The header. The script list starts at 10, the feature list at 30, and the lookups at 46.
    out.extend_from_slice(&[0, 1, 0, 0]);
    words(&mut out, &[10, 30, 46]);
    // Scripts: DFLT, whose default language is 4 further on.
    words(&mut out, &[1]);
    out.extend_from_slice(b"DFLT");
    words(&mut out, &[8, 4, 0, 0, 0xFFFF, 1, 0]);
    // Features: liga, with both lookups.
    words(&mut out, &[1]);
    out.extend_from_slice(b"liga");
    words(&mut out, &[8, 0, 2, 0, 1]);
    assert_eq!(out.len(), 46);
    const CALLS: u16 = 16;
    let lookup_size = 8 + 8 + CALLS * 4 + 10;
    // Lookups: at 6 and just after the first.
    words(&mut out, &[2, 6, 6 + lookup_size]);
    // Lookup 0: a context rule on every glyph that runs lookup 1, then lookup 0 over and over.
    words(&mut out, &[5, 0, 1, 8]);
    words(&mut out, &[3, 1, CALLS, 8 + CALLS * 4]);
    words(&mut out, &[0, 1]);
    for _ in 1..CALLS {
        words(&mut out, &[0, 0]);
    }
    // Its coverage: everything.
    words(&mut out, &[2, 1, 0, 0xFFFF, 0]);
    assert_eq!(out.len(), 46 + 6 + lookup_size as usize);
    // Lookup 1: this glyph becomes fifty of itself.
    words(&mut out, &[2, 0, 1, 8]);
    words(&mut out, &[1, 8, 1, 14]);
    words(&mut out, &[1, 1, glyph]);
    words(&mut out, &[50]);
    words(&mut out, &[glyph; 50]);
    out
}

#[test]
fn a_font_written_to_cause_harm_only_slows_down_a_little() {
    let Some(mut bytes) = font_file("arial.ttf") else {
        return;
    };
    let glyph = Font::from_bytes(bytes.clone())
        .ok()
        .and_then(|font| font.glyph('a'))
        .expect("the font has it");
    let table = hostile_gsub(glyph);
    let range = table_range(&bytes, b"GSUB").expect("the font has one");
    bytes[range.start..range.start + table.len()].copy_from_slice(&table);
    let font = Font::from_bytes(bytes).expect("it's still a font");
    let mut engine = TextEngine::new(font);
    let started = Instant::now();
    let layout = engine.layout(&"a".repeat(200), STYLE, f32::INFINITY);
    let elapsed = started.elapsed();
    assert!(!layout.glyphs.is_empty());
    assert!(
        layout.glyphs.len() <= 208 * 17,
        "{} glyphs",
        layout.glyphs.len()
    );
    assert!(elapsed < Duration::from_secs(20), "took {elapsed:?}");
}
