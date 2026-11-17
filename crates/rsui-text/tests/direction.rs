//! Which way a paragraph goes when CSS's `direction` says, rather than its first letter (UAX #9's HL1), and `unicode-bidi`'s
//! overrides and isolates. These use Segoe UI, and are skipped if it isn't there.

use std::path::Path;

use rsui_text::{Font, RichLayout, RichText, TextEngine, TextStyle};

const STYLE: TextStyle = TextStyle::new(20.0);

fn engine() -> Option<TextEngine> {
    Font::open(Path::new("C:/Windows/Fonts").join("segoeui.ttf"))
        .ok()
        .map(TextEngine::new)
}

fn right_to_left() -> TextStyle {
    TextStyle {
        right_to_left: true,
        direction_said: true,
        ..STYLE
    }
}

/// The characters a layout shows, from left to right, by matching each glyph with the font's glyph for one of `text`'s.
fn shown(engine: &TextEngine, text: &str, glyphs: &[rsui_text::PlacedGlyph]) -> String {
    let font = engine.font(0).expect("there's a font");
    let mut sorted = glyphs.to_vec();
    sorted.sort_by(|a, b| a.x.total_cmp(&b.x));
    sorted
        .iter()
        .map(|glyph| {
            text.chars()
                .find(|character| font.glyph(*character) == Some(glyph.glyph))
                .unwrap_or('?')
        })
        .collect()
}

#[test]
fn digit_groups_go_from_the_right_in_a_right_to_left_paragraph() {
    let Some(mut engine) = engine() else { return };
    let text = "1234 5678";
    let layout = engine.layout(text, right_to_left(), f32::INFINITY);
    assert_eq!(shown(&engine, text, &layout.glyphs), "5678 1234");
    // Left to right, or with nothing said, they're as written.
    let layout = engine.layout(text, STYLE, f32::INFINITY);
    assert_eq!(shown(&engine, text, &layout.glyphs), "1234 5678");
}

#[test]
fn punctuation_at_the_end_goes_to_the_left_from_the_right() {
    let Some(mut engine) = engine() else { return };
    let text = "Hello world!";
    let layout = engine.layout(text, right_to_left(), f32::INFINITY);
    assert_eq!(shown(&engine, text, &layout.glyphs), "!Hello world");
    // A shortcut: the key's letter is a run of its own, to the left of what's before it.
    let text = "\u{00A9}+T";
    let layout = engine.layout(text, right_to_left(), f32::INFINITY);
    assert_eq!(shown(&engine, text, &layout.glyphs), "T+\u{00A9}");
    let layout = engine.layout(text, STYLE, f32::INFINITY);
    assert_eq!(shown(&engine, text, &layout.glyphs), "\u{00A9}+T");
}

#[test]
fn words_alone_stay_plain_in_a_right_to_left_paragraph() {
    let Some(mut engine) = engine() else { return };
    let text = "Hello world";
    let plain = engine.layout(text, STYLE, f32::INFINITY);
    let layout = engine.layout(text, right_to_left(), f32::INFINITY);
    assert_eq!(shown(&engine, text, &layout.glyphs), "Hello world");
    assert_eq!(plain.glyphs, layout.glyphs);
    assert!(!engine.may_reorder(text, right_to_left()));
    assert!(engine.may_reorder("1 2", right_to_left()));
}

#[test]
fn a_paragraph_said_to_go_from_the_left_does_whatever_its_first_letter_is() {
    let Some(mut engine) = engine() else { return };
    let text = "\u{05D0}\u{05D1} 12";
    let left = TextStyle {
        direction_said: true,
        ..STYLE
    };
    // On its own it goes from the right, as its first strong letter does, and lines up there.
    let layout = engine.layout(text, STYLE, 300.0);
    assert!(layout.lines[0].x > 0.0);
    // Said to go from the left, it starts at the left, with the number after the Hebrew, to its right.
    let layout = engine.layout(text, left, 300.0);
    assert_eq!(layout.lines[0].x, 0.0);
    assert_eq!(shown(&engine, text, &layout.glyphs), "12 \u{05D1}\u{05D0}");
}

#[test]
fn an_override_turns_every_letter_around() {
    let Some(mut engine) = engine() else { return };
    let text = "abc";
    let style = TextStyle {
        bidi_override: true,
        ..right_to_left()
    };
    let layout = engine.layout(text, style, f32::INFINITY);
    assert_eq!(shown(&engine, text, &layout.glyphs), "cba");
}

#[test]
fn carets_in_a_right_to_left_number_line_start_at_the_right() {
    let Some(mut engine) = engine() else { return };
    let text = "12 34";
    let style = right_to_left();
    // Before "12", which shows at the right, is the right of its "1".
    let start = engine.caret_x(text, style, 0);
    let between = engine.caret_x(text, style, 3);
    let end = engine.caret_x(text, style, text.len());
    let width = engine.line_width(text, style);
    assert!(start < width && start > between, "{start} {between} {width}");
    assert!(end > between - 0.01, "after 34 is at its right: {end} {between}");
    assert!(engine.is_right_to_left(text, style));
}

fn rich_of(parts: &[(&str, TextStyle)]) -> RichText {
    let mut rich = RichText::default();
    for (text, style) in parts {
        rich.push(text, *style);
    }
    rich
}

/// The text of each piece of a paragraph's first line, from left to right.
fn pieces(rich: &RichText, layout: &RichLayout) -> Vec<String> {
    let line = layout.lines[0];
    let first = line.first_piece as usize;
    layout.pieces[first..first + line.piece_count as usize]
        .iter()
        .map(|piece| rich.text[piece.start as usize..piece.end as usize].to_string())
        // Spaces that go another way than what's around them are pieces of their own, with nothing to show.
        .filter(|piece| !piece.is_empty())
        .collect()

}

#[test]
fn parts_of_a_right_to_left_paragraph_go_from_the_right() {
    let Some(mut engine) = engine() else { return };
    let style = right_to_left();
    let rich = rich_of(&[("1 2 ", style), ("Ctrl", style)]);
    let mut layout = RichLayout::default();
    engine.layout_rich_into(&rich, 400.0, None, &mut layout);
    // Numbers that start it are at the right, the first rightmost, and the word after them at the left.
    assert_eq!(pieces(&rich, &layout), ["Ctrl", "2", "1"]);

    assert!(layout.lines[0].right_to_left);
    assert!(layout.lines[0].x > 0.0, "it lines up on the right");
}

#[test]
fn an_isolate_keeps_its_own_direction_in_a_paragraph() {
    let Some(mut engine) = engine() else { return };
    let style = right_to_left();
    let hebrew = "\u{05D0}\u{05D1}";
    let mut rich = rich_of(&[(hebrew, style), (" ", style), ("a 1", style), (" ", style), (hebrew, style)]);
    let mut layout = RichLayout::default();
    engine.layout_rich_into(&rich, 400.0, None, &mut layout);
    let without = pieces(&rich, &layout);
    // Around the middle run, a left-to-right isolate: the same here, as it's already from the left.
    rich.controls = vec![(5, '\u{2066}'), (8, '\u{2069}')];
    engine.layout_rich_into(&rich, 400.0, None, &mut layout);
    assert_eq!(pieces(&rich, &layout), without);
    // A right-to-left override turns the middle around.
    rich.controls = vec![(5, '\u{202E}'), (8, '\u{202C}')];
    engine.layout_rich_into(&rich, 400.0, None, &mut layout);
    let line = layout.lines[0];
    let first = line.first_piece as usize;
    let middle = layout.pieces[first..first + line.piece_count as usize]
        .iter()
        .find(|piece| piece.start == 5)
        .expect("the middle is there");
    assert_eq!(middle.level % 2, 1, "{layout:?}");
}
