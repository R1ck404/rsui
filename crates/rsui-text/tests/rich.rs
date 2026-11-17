//! A paragraph with parts that look different.

mod support;

use rsui_core::{Element, Role};
use rsui_layout::{Layout, Measure, Size, Style};
use rsui_text::{RichLayout, RichText, TextEngine, TextStyle};
use support::plain_font;

/// 10 pixels per em, so `A` is 6 wide, `B` is 5, `C` is 8, `D` is 7, and space is 2.5.
/// Lines are 11 apart: 8 of ascent, 2 of descent and 1 of gap.
const STYLE: TextStyle = TextStyle::new(10.0);
const TWICE: TextStyle = TextStyle::new(20.0);

fn engine() -> TextEngine {
    TextEngine::new(plain_font())
}

fn rich(parts: &[(&str, TextStyle)]) -> RichText {
    let mut text = RichText::default();
    for (part, style) in parts {
        text.push(part, *style);
    }
    text
}

fn arrange(parts: &[(&str, TextStyle)], max_width: f32) -> RichLayout {
    let mut layout = RichLayout::default();
    engine().layout_rich_into(&rich(parts), max_width, None, &mut layout);
    layout
}

/// Each line, as how many glyphs each of its pieces has.
fn glyphs_on_lines(layout: &RichLayout) -> Vec<Vec<u32>> {
    layout
        .lines
        .iter()
        .map(|line| {
            let first = line.first_piece as usize;
            layout.pieces[first..first + line.piece_count as usize]
                .iter()
                .map(|piece| piece.glyph_count)
                .collect()
        })
        .collect()
}

#[test]
fn parts_with_one_look_measure_the_same_as_plain_text() {
    let parts = [("AB ", STYLE), ("CD", STYLE)];
    let layout = arrange(&parts, f32::INFINITY);
    let plain = engine().measure("AB CD", STYLE, f32::INFINITY);
    assert_eq!(Size::new(layout.width(), layout.height()), plain);
}

#[test]
fn the_space_between_two_parts_is_kept() {
    let layout = arrange(&[("AB ", STYLE), ("CD", TWICE)], f32::INFINITY);
    assert_eq!(layout.pieces.len(), 2);
    assert_eq!(layout.pieces[1].x, 6.0 + 5.0 + 2.5);
    // And a part starting with one is the same.
    let layout = arrange(&[("AB", STYLE), (" CD", TWICE)], f32::INFINITY);
    assert!(layout.pieces.last().unwrap().x > 11.0, "{layout:?}");
}

#[test]
fn lines_wrap_across_parts_as_if_they_were_one_text() {
    // A word is 11 wide, with 2.5 between them. Two take 24.5, and three take 38.
    let parts = [("AB AB ", STYLE), ("AB AB", STYLE)];
    let layout = arrange(&parts, 25.0);
    assert_eq!(layout.lines.len(), 2);
    assert_eq!(layout.lines[0].width, 24.5);
    assert_eq!(layout.lines[1].width, 24.5);
    assert_eq!(layout.lines[1].top, 11.0);
    assert_eq!(layout.height(), 22.0);
    // A part wrapping halfway through is the same.
    let parts = [("AB ", STYLE), ("AB AB", STYLE), (" AB", STYLE)];
    let layout = arrange(&parts, 25.0);
    assert_eq!(glyphs_on_lines(&layout), [vec![2, 2], vec![2, 2]]);
}

#[test]
fn a_word_in_two_looks_stays_together() {
    // "AB" and "CD" are one word, which doesn't fit after what's first.
    let parts = [("AB ", STYLE), ("AB", TWICE), ("CD", STYLE)];
    let width = 11.0 + 2.5 + 22.0 + 15.0;
    let layout = arrange(&parts, width - 1.0);
    assert_eq!(glyphs_on_lines(&layout), [vec![2], vec![2, 2]]);
    assert_eq!(arrange(&parts, width).lines.len(), 1);
}

#[test]
fn a_word_wider_than_a_line_is_split() {
    let layout = arrange(&[("ABABABAB", STYLE)], 25.0);
    assert!(layout.lines.len() > 1, "{layout:?}");
    assert!(layout.lines.iter().all(|line| line.width <= 25.0));
    assert_eq!(layout.glyphs.len(), 8);
}

#[test]
fn nothing_goes_wrong_with_no_room_at_all() {
    let layout = arrange(&[("AB CD ", STYLE), ("A", TWICE)], 0.0);
    assert_eq!(layout.glyphs.len(), 5);
    assert_eq!(layout.lines.len(), 5);
}

#[test]
fn a_bigger_part_makes_its_line_taller_and_they_share_a_baseline() {
    let layout = arrange(&[("A", STYLE), ("B", TWICE), ("\nA", STYLE)], f32::INFINITY);
    assert_eq!(layout.lines.len(), 2);
    // Twice the size has 16 of ascent and 22 between lines.
    assert_eq!(layout.lines[0].height, 22.0);
    assert_eq!(layout.lines[0].baseline, 16.0);
    let on_first = &layout.glyphs[..2];
    assert!(on_first.iter().all(|glyph| glyph.y == 16.0), "{on_first:?}");
    // The smaller part's own box is where its text is, which isn't the top of the line.
    assert_eq!(layout.pieces[0].top, 8.0);
    assert_eq!(layout.pieces[0].height, 11.0);
    assert_eq!(layout.lines[1].top, 22.0);
    assert_eq!(layout.lines[1].height, 11.0);
    assert_eq!(layout.glyphs[2].y, 22.0 + 8.0);
}

#[test]
fn line_breaks_are_kept_including_at_the_ends() {
    let layout = arrange(&[("A\n", STYLE), ("\nB", STYLE)], f32::INFINITY);
    assert_eq!(layout.lines.len(), 3);
    assert_eq!(layout.lines[1].width, 0.0);
    // Plain text ending with one has an empty line after it, so this does too.
    assert_eq!(arrange(&[("A\n", STYLE)], f32::INFINITY).lines.len(), 2);
}

#[test]
fn spaces_at_the_end_of_a_line_do_not_count() {
    let layout = arrange(&[("AB ", STYLE)], f32::INFINITY);
    assert_eq!(layout.width(), 11.0);
    // They can't push a line over its limit either.
    let layout = arrange(&[("AB      ", STYLE), ("AB", STYLE)], 11.0);
    assert_eq!(layout.lines.len(), 2);
}

#[test]
fn nothing_at_all_has_no_size() {
    let layout = arrange(&[], 100.0);
    assert_eq!((layout.width(), layout.height()), (0.0, 0.0));
    assert!(layout.lines.is_empty());
}

#[test]
fn an_underline_can_carry_across_the_space_within_a_part_but_not_past_it() {
    let layout = arrange(&[("AB AB ", STYLE), ("AB", STYLE)], f32::INFINITY);
    assert_eq!(layout.pieces[0].after, 2.5);
    assert_eq!(layout.pieces[1].after, 0.0);
    assert_eq!(layout.pieces[2].after, 0.0);
}

#[test]
fn a_position_says_which_part_it_is_on() {
    let layout = arrange(&[("AB ", STYLE), ("AB", TWICE)], f32::INFINITY);
    // The second part is 22 tall, and the first sits on the same baseline.
    assert_eq!(layout.run_at(1.0, 12.0), Some(0));
    assert_eq!(layout.run_at(20.0, 1.0), Some(1));
    assert_eq!(layout.run_at(100.0, 12.0), None);
    assert_eq!(layout.run_at(1.0, 100.0), None);
}

#[test]
fn lines_go_where_they_are_told_to() {
    let mut layout = RichLayout::default();
    engine().layout_rich_into(&rich(&[("AB", STYLE)]), 111.0, Some(1.0), &mut layout);
    assert_eq!(layout.lines[0].x, 100.0);
    assert_eq!(layout.pieces[0].x, 100.0);
    assert_eq!(layout.glyphs[0].x, 100.0);
}

fn paragraph(parts: Vec<Element>) -> Element {
    let mut element = Element::new(Role::Text).prop("font_size", 10.0);
    for part in parts {
        element = element.child(part);
    }
    element
}

fn part(text: &str) -> Element {
    Element::new(Role::Text).name(text)
}

#[test]
fn text_with_text_inside_is_measured_as_one_paragraph() {
    let element = paragraph(vec![part("AB "), part("CD")]);
    let size = Measure::measure(&mut engine(), &element, f32::INFINITY);
    // Sizes are in whole pixels, and this is 28.5.
    assert_eq!(size, Size::new(29.0, 11.0));
}

#[test]
fn parts_take_what_they_do_not_set_from_what_they_are_in() {
    let element = paragraph(vec![
        part("A"),
        part("A").prop("font_size", 20.0),
        Element::new(Role::Group).child(part("A")),
    ]);
    let size = Measure::measure(&mut engine(), &element, f32::INFINITY);
    assert_eq!(size, Size::new(6.0 + 12.0 + 6.0, 22.0));
}

#[test]
fn layout_gives_a_paragraph_a_box_and_each_part_a_box_where_it_is() {
    let tree = Element::new(Role::Window)
        .child(
            Element::new(Role::Text)
                .props(Style::new().padding(5.0))
                .prop("font_size", 10.0)
                .child(part("AB "))
                .child(Element::new(Role::Group).child(part("AB").key("inner"))),
        )
        .into_node();
    let mut layout = Layout::new();
    let mut text = engine();
    layout.compute(&tree, Size::new(200.0, 100.0), &mut text);
    let whole = layout.rect(&[0]).unwrap();
    assert_eq!((whole.height), 21.0);
    let first = layout.rect(&[0, 0]).unwrap();
    assert_eq!(
        (first.x, first.y, first.width, first.height),
        (5.0, 5.0, 11.0, 11.0)
    );
    let group = layout.rect(&[0, 1]).unwrap();
    assert_eq!((group.x, group.width), (5.0 + 13.5, 11.0));
    let inner = layout.rect(&[0, 1, 0]).unwrap();
    assert_eq!((inner.x, inner.y, inner.width), (5.0 + 13.5, 5.0, 11.0));
    // Parts aren't what a pointer lands on, as boxes can't say what's on which line.
    assert_eq!(layout.hit_test(6.0, 6.0), Some(vec![0]));
}

#[test]
fn changing_a_part_changes_the_size_of_the_paragraph() {
    let mut layout = Layout::new();
    let mut text = engine();
    let tree = |words: &str| {
        Element::new(Role::Window)
            .child(paragraph(vec![part("AB "), part(words)]))
            .into_node()
    };
    let first = tree("AB");
    layout.compute(&first, Size::new(25.0, 100.0), &mut text);
    assert_eq!(layout.rect(&[0]).unwrap().height, 11.0);
    let second = tree("AB AB");
    layout.apply(&rsui_core::diff(&first, &second));
    layout.compute(&second, Size::new(25.0, 100.0), &mut text);
    assert_eq!(layout.rect(&[0]).unwrap().height, 22.0);
    assert_eq!(
        layout.rect(&[0, 1]).unwrap().height,
        22.0,
        "its box is around both of its lines"
    );
    // And over and over, as what's been told once mustn't stop it being told again.
    let third = tree("AB");
    layout.apply(&rsui_core::diff(&second, &third));
    layout.compute(&third, Size::new(25.0, 100.0), &mut text);
    assert_eq!(layout.rect(&[0]).unwrap().height, 11.0);
}

#[test]
fn text_with_anything_else_inside_is_not_a_paragraph() {
    let inside = Element::new(Role::Text)
        .child(part("A"))
        .child(Element::new(Role::Button));
    assert!(!rsui_layout::is_paragraph(&inside));
    let plain = part("A");
    assert!(!rsui_layout::is_paragraph(&plain));
    assert!(rsui_layout::is_paragraph(&paragraph(vec![part("A")])));
}

const HELLO: &str = "\u{0645}\u{0631}\u{062D}\u{0628}\u{0627}";
const WORLD: &str = "\u{0627}\u{0644}\u{0639}\u{0627}\u{0644}\u{0645}";

/// Takes Arabic, which the font these tests use doesn't have.
fn segoe() -> Option<TextEngine> {
    let path = "C:/Windows/Fonts/segoeui.ttf";
    std::path::Path::new(path)
        .exists()
        .then(|| TextEngine::new(rsui_text::Font::open(path).unwrap()))
}

fn x_of_run(layout: &RichLayout, run: u32) -> f32 {
    layout
        .pieces
        .iter()
        .find(|piece| piece.run == run)
        .unwrap_or_else(|| panic!("no run {run} in {layout:?}"))
        .x
}

#[test]
fn a_paragraph_in_a_right_to_left_language_starts_on_the_right() {
    let Some(mut engine) = segoe() else { return };
    let first = format!("{HELLO} ");
    let mut layout = RichLayout::default();
    let text = rich(&[(first.as_str(), STYLE), (WORLD, STYLE)]);
    engine.layout_rich_into(&text, 300.0, None, &mut layout);
    assert_eq!(layout.lines.len(), 1);
    assert!(x_of_run(&layout, 0) > x_of_run(&layout, 1), "{layout:?}");
    let right = layout.pieces.iter().map(|piece| piece.x + piece.width);
    let right = right.fold(0.0f32, f32::max);
    assert!((right - 300.0).abs() < 0.01, "it ends at {right}");
    // On the last piece on the left, spaces aren't what decide where it starts.
    assert!(layout.lines[0].x > 200.0, "{:?}", layout.lines[0]);
}

#[test]
fn words_from_right_to_left_within_a_left_to_right_paragraph_swap_places() {
    let Some(mut engine) = segoe() else { return };
    let (before, first, second, after) = ("One ", format!("{HELLO} "), format!("{WORLD} "), "two");
    let parts = [
        (before, STYLE),
        (first.as_str(), STYLE),
        (second.as_str(), STYLE),
        (after, STYLE),
    ];
    let mut layout = RichLayout::default();
    engine.layout_rich_into(&rich(&parts), f32::INFINITY, None, &mut layout);
    let at: Vec<f32> = (0..4).map(|run| x_of_run(&layout, run)).collect();
    assert!(at[0] < at[2] && at[2] < at[1] && at[1] < at[3], "{at:?}");
}

#[test]
fn text_with_no_spaces_can_still_wrap_between_characters() {
    let Some(mut engine) = segoe() else { return };
    let text = rich(&[("\u{4F60}\u{597D}", STYLE), ("\u{4E16}\u{754C}", STYLE)]);
    let mut layout = RichLayout::default();
    engine.layout_rich_into(&text, f32::INFINITY, None, &mut layout);
    let width = layout.width();
    engine.layout_rich_into(&text, width - 1.0, None, &mut layout);
    assert_eq!(layout.lines.len(), 2, "{layout:?}");
}

#[test]
fn each_line_after_a_break_goes_its_own_way() {
    let Some(mut engine) = segoe() else { return };
    let mixed = format!("{HELLO}\nAB\n{WORLD}");
    let mut layout = RichLayout::default();
    engine.layout_rich_into(&rich(&[(mixed.as_str(), STYLE)]), 300.0, None, &mut layout);
    assert_eq!(layout.lines.len(), 3);
    assert!(
        layout.lines[0].right_to_left && layout.lines[2].right_to_left,
        "{layout:?}"
    );
    assert!(!layout.lines[1].right_to_left, "{layout:?}");
    assert!(layout.lines[0].x > 200.0, "{:?}", layout.lines[0]);
    assert_eq!(layout.lines[1].x, 0.0, "{:?}", layout.lines[1]);
    assert!(layout.lines[2].x > 200.0, "{:?}", layout.lines[2]);
}

#[test]
fn a_box_that_goes_back_and_forth_from_being_part_of_a_paragraph_stays_right() {
    let mut layout = Layout::new();
    let mut text = engine();
    let size = Size::new(200.0, 100.0);
    // The label, on its own as a box with padding, while something else is in with it that isn't text.
    let labelled = |other: bool| {
        let label = part("AB")
            .prop("font_size", 10.0)
            .props(Style::new().padding(4.0));
        let mut outer = Element::new(Role::Text).child(label);
        if other {
            outer = outer.child(Element::new(Role::Button).name("x"));
        }
        Element::new(Role::Window).child(outer).into_node()
    };
    let first = labelled(false);
    layout.compute(&first, size, &mut text);
    let second = labelled(true);
    layout.apply(&rsui_core::diff(&first, &second));
    layout.compute(&second, size, &mut text);
    let as_a_box = layout.rect(&[0, 0]).unwrap();
    let third = labelled(false);
    layout.apply(&rsui_core::diff(&second, &third));
    layout.compute(&third, size, &mut text);
    let fourth = labelled(true);
    layout.apply(&rsui_core::diff(&third, &fourth));
    layout.compute(&fourth, size, &mut text);
    let again = layout.rect(&[0, 0]).unwrap();
    assert_eq!(
        (as_a_box.width, as_a_box.height),
        (again.width, again.height)
    );
}

#[test]
fn something_that_stops_being_on_top_of_everything_is_only_drawn_once() {
    let mut layout = Layout::new();
    let mut text = engine();
    let size = Size::new(200.0, 100.0);
    let tree = |on_top: bool| {
        let mut inner = part("AB");
        if on_top {
            inner = inner.prop("overlay", "below");
        }
        Element::new(Role::Window)
            .child(Element::new(Role::Text).child(inner))
            .into_node()
    };
    let first = tree(true);
    layout.compute(&first, size, &mut text);
    assert!(layout.has_overlays());
    let second = tree(false);
    layout.apply(&rsui_core::diff(&first, &second));
    layout.compute(&second, size, &mut text);
    assert!(
        !layout.has_overlays(),
        "it's only a part of a paragraph now"
    );
    assert!(layout.overlays().is_empty());
}
