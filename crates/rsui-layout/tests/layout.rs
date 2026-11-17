//! What each rule does, checked against sizes worked out by hand, and the guarantee
//! that updating a layout gives the same result as computing it from nothing.

use proptest::collection::vec;
use proptest::prelude::*;
use rsui_core::{ActionKind, Element, Node, Patch, Props, Role, diff, text};
use rsui_layout::{Align, FixedMetrics, Justify, Layout, Rect, Size, Style, style, with_margin};
use rsui_runtime::{Cx, ListWindow, Runtime};

const WINDOW: Size = Size::new(400.0, 300.0);

fn group(style: Style) -> Element {
    Element::new(Role::Group).props(style)
}

fn window(style: Style) -> Element {
    Element::new(Role::Window).props(style)
}

fn laid_out(root: &Node) -> Layout {
    let mut layout = Layout::new();
    layout.compute(root, WINDOW, &mut FixedMetrics::default());
    layout
}

#[test]
fn max_content_caps_a_stretched_block_and_keeps_its_absolute_content_anchored() {
    let root = window(Style::column())
        .child(group(Style::row().gap(8.0))
            .prop("max_width_fit", true)
            .child(group(Style::new().width(100.0).height(20.0)))
            .child(group(Style::new().width(120.0).height(20.0)))
            .child(group(Style::new().width(200.0).height(60.0))
                .prop("position", "absolute")
                .prop("left", 0.0)
                .prop("top", "100%")))
        .into_node();
    let layout = laid_out(&root);
    assert_eq!(layout.rect(&[0]).unwrap(), Rect::new(0.0, 0.0, 228.0, 20.0));
    assert_eq!(layout.rect(&[0, 2]).unwrap(), Rect::new(0.0, 20.0, 200.0, 60.0));
    // Asking again must use the same constrained answer, including when the child was stretched by a column.
    let mut again = layout;
    again.compute(&root, WINDOW, &mut FixedMetrics::default());
    assert_eq!(again.rect(&[0]).unwrap().width, 228.0);
}

fn rects(element: Element, paths: &[&[u32]]) -> Vec<Rect> {
    let root = element.into_node();
    let layout = laid_out(&root);
    paths
        .iter()
        .map(|path| layout.rect(path).expect("the node was laid out"))
        .collect()
}

const fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect::new(x, y, width, height)
}

#[test]
fn the_root_fills_the_window() {
    assert_eq!(
        rects(window(Style::new()), &[&[]]),
        [rect(0.0, 0.0, 400.0, 300.0)]
    );
}

#[test]
fn children_stack_in_a_column_and_stretch_across_by_default() {
    let root = window(Style::new())
        .child(group(Style::new().height(50.0)))
        .child(group(Style::new().height(30.0)));
    assert_eq!(
        rects(root, &[&[0], &[1]]),
        [rect(0.0, 0.0, 400.0, 50.0), rect(0.0, 50.0, 400.0, 30.0)]
    );
}

#[test]
fn a_row_stacks_children_side_by_side() {
    let root = window(Style::row())
        .child(group(Style::new().width(50.0)))
        .child(group(Style::new().width(30.0)));
    assert_eq!(
        rects(root, &[&[0], &[1]]),
        [rect(0.0, 0.0, 50.0, 300.0), rect(50.0, 0.0, 30.0, 300.0)]
    );
}

#[test]
fn gaps_go_between_children_and_padding_goes_around_them() {
    let root = window(Style::row().gap(10.0).padding_each(1.0, 2.0, 3.0, 4.0))
        .child(group(Style::new().width(50.0)))
        .child(group(Style::new().width(30.0)));
    assert_eq!(
        rects(root, &[&[0], &[1]]),
        [rect(4.0, 1.0, 50.0, 296.0), rect(64.0, 1.0, 30.0, 296.0)]
    );
}

#[test]
fn flex_shares_out_the_space_by_ratio() {
    let root = window(Style::row().gap(10.0))
        .child(group(Style::new().width(80.0)))
        .child(group(Style::new().flex(1.0)))
        .child(group(Style::new().flex(2.0)));
    // 400 pixels, minus 80 for the first child and 20 for the gaps, leaves 300.
    assert_eq!(
        rects(root, &[&[1], &[2]]),
        [
            rect(90.0, 0.0, 100.0, 300.0),
            rect(200.0, 0.0, 200.0, 300.0)
        ]
    );
}

#[test]
fn flex_ignores_the_size_of_the_content() {
    let root = window(Style::row())
        .child(
            Element::new(Role::Text)
                .name("a very long piece of text that wants to take all the room")
                .props(Style::new().flex(1.0)),
        )
        .child(group(Style::new().flex(1.0)));
    assert_eq!(rects(root, &[&[0]]), [rect(0.0, 0.0, 200.0, 300.0)]);
}

#[test]
fn grow_keeps_an_elements_own_size_and_adds_to_it() {
    let root = window(Style::row())
        .child(group(Style::new().width(100.0).grow(1.0)))
        .child(group(Style::new().width(200.0)));
    assert_eq!(
        rects(root, &[&[0], &[1]]),
        [rect(0.0, 0.0, 200.0, 300.0), rect(200.0, 0.0, 200.0, 300.0)]
    );
}

#[test]
fn elements_overflow_unless_they_can_shrink() {
    let overflow = window(Style::row())
        .child(group(Style::new().width(300.0)))
        .child(group(Style::new().width(300.0)));
    assert_eq!(rects(overflow, &[&[1]]), [rect(300.0, 0.0, 300.0, 300.0)]);

    let shrinking = window(Style::row())
        .child(group(Style::new().width(300.0).shrink(1.0)))
        .child(group(Style::new().width(300.0)));
    assert_eq!(
        rects(shrinking, &[&[0], &[1]]),
        [rect(0.0, 0.0, 100.0, 300.0), rect(100.0, 0.0, 300.0, 300.0)]
    );
}

#[test]
fn shrinking_takes_more_from_larger_elements() {
    let root = window(Style::row())
        .child(group(Style::new().width(200.0).shrink(1.0)))
        .child(group(Style::new().width(600.0).shrink(1.0)));
    // 400 pixels too many, taken in proportion to their sizes of 200 and 600.
    assert_eq!(
        rects(root, &[&[0], &[1]]),
        [rect(0.0, 0.0, 100.0, 300.0), rect(100.0, 0.0, 300.0, 300.0)]
    );
}

#[test]
fn changing_a_childs_style_can_change_its_parents_decisions() {
    let tree = |first: Style| {
        window(Style::row())
            .child(group(first))
            .child(group(Style::new().flex(1.0)))
            .into_node()
    };
    // The first child's own width doesn't decide this: its share of the space does.
    let (old, new) = (tree(Style::new().flex(1.0)), tree(Style::new().flex(3.0)));
    let mut layout = laid_out(&old);
    assert_eq!(layout.rect(&[1]), Some(rect(200.0, 0.0, 200.0, 300.0)));

    layout.apply(&diff(&old, &new));
    layout.compute(&new, WINDOW, &mut FixedMetrics::default());

    assert_eq!(layout.rect(&[0]), Some(rect(0.0, 0.0, 300.0, 300.0)));
    assert_eq!(layout.rect(&[1]), Some(rect(300.0, 0.0, 100.0, 300.0)));
}

#[test]
fn changing_a_prop_unrelated_to_layout_measures_nothing() {
    let tree = |checked: bool| {
        window(Style::new())
            .child(label("label").prop("checked", checked))
            .into_node()
    };
    let (old, new) = (tree(false), tree(true));
    let mut counting = Counting::default();
    let mut layout = Layout::new();
    layout.compute(&old, WINDOW, &mut counting);
    counting.calls = 0;
    layout.apply(&diff(&old, &new));
    layout.compute(&new, WINDOW, &mut counting);
    assert!(counting.calls <= 2, "measured {} times", counting.calls);
}

#[test]
fn limits_win_and_the_others_share_what_is_left() {
    let root = window(Style::row())
        .child(group(Style::new().flex(1.0).max_width(50.0)))
        .child(group(Style::new().flex(1.0)));
    assert_eq!(
        rects(root, &[&[0], &[1]]),
        [rect(0.0, 0.0, 50.0, 300.0), rect(50.0, 0.0, 350.0, 300.0)]
    );

    let root = window(Style::row())
        .child(group(Style::new().flex(1.0).min_width(300.0)))
        .child(group(Style::new().flex(1.0)));
    assert_eq!(
        rects(root, &[&[0], &[1]]),
        [rect(0.0, 0.0, 300.0, 300.0), rect(300.0, 0.0, 100.0, 300.0)]
    );
}

#[test]
fn a_minimum_beats_a_maximum() {
    let root = window(Style::new()).child(group(Style::new().min_height(50.0).max_height(10.0)));
    assert_eq!(rects(root, &[&[0]]), [rect(0.0, 0.0, 400.0, 50.0)]);
}

#[test]
fn percentages_are_of_the_parents_inner_size() {
    let root = window(Style::row().padding(10.0))
        .child(group(Style::new().width_percent(50.0).height_percent(50.0)));
    assert_eq!(rects(root, &[&[0]]), [rect(10.0, 10.0, 190.0, 140.0)]);
}

#[test]
fn a_percentage_of_a_size_that_depends_on_content_is_ignored() {
    let root = window(Style::new().align(Align::Start))
        .child(group(Style::new()).child(group(Style::new().height(20.0).width_percent(50.0))));
    assert_eq!(rects(root, &[&[0, 0]]), [rect(0.0, 0.0, 0.0, 20.0)]);
}

#[test]
fn an_element_without_a_size_fits_its_content() {
    let root = window(Style::new().align(Align::Start)).child(
        group(Style::row().gap(5.0).padding(10.0))
            .child(group(Style::new().width(30.0).height(40.0)))
            .child(group(Style::new().width(20.0).height(60.0))),
    );
    assert_eq!(rects(root, &[&[0]]), [rect(0.0, 0.0, 75.0, 80.0)]);
}

#[test]
fn alignment_places_children_across_the_direction() {
    let child = || group(Style::new().width(100.0).height(50.0));
    let check = |align, expected_y: f32| {
        let root = window(Style::row().align(align)).child(child());
        assert_eq!(rects(root, &[&[0]]), [rect(0.0, expected_y, 100.0, 50.0)]);
    };
    check(Align::Start, 0.0);
    check(Align::Center, 125.0);
    check(Align::End, 250.0);
    // A child with its own size doesn't stretch.
    check(Align::Stretch, 0.0);
}

#[test]
fn a_child_can_override_its_parents_alignment() {
    let root = window(Style::row().align(Align::Start)).child(group(
        Style::new().width(10.0).height(10.0).align_self(Align::End),
    ));
    assert_eq!(rects(root, &[&[0]]), [rect(0.0, 290.0, 10.0, 10.0)]);
}

#[test]
fn justify_places_children_along_the_direction() {
    let root = |justify| {
        window(Style::row().justify(justify))
            .child(group(Style::new().width(100.0)))
            .child(group(Style::new().width(100.0)))
    };
    let x_of_second = |justify| rects(root(justify), &[&[1]])[0].x;
    assert_eq!(x_of_second(Justify::Start), 100.0);
    assert_eq!(x_of_second(Justify::Center), 200.0);
    assert_eq!(x_of_second(Justify::End), 300.0);
    assert_eq!(x_of_second(Justify::SpaceBetween), 300.0);
}

#[test]
fn space_between_with_one_child_stays_at_the_start() {
    let root =
        window(Style::row().justify(Justify::SpaceBetween)).child(group(Style::new().width(100.0)));
    assert_eq!(rects(root, &[&[0]])[0].x, 0.0);
}

fn label(name: &str) -> Element {
    Element::new(Role::Text).name(name)
}

#[test]
fn text_is_measured_and_wraps_to_fit() {
    // 8 pixels per character and 16 per line, so 30 characters is 240 pixels.
    let thirty = "012345678901234567890123456789";
    let root = window(Style::new())
        .child(label(thirty))
        .child(group(Style::new().width(100.0).align_self(Align::Start)).child(label(thirty)));
    assert_eq!(
        rects(root, &[&[0], &[1], &[1, 0]]),
        [
            rect(0.0, 0.0, 400.0, 16.0),
            // 12 characters fit on each line of 100 pixels, so this takes 3 lines.
            rect(0.0, 16.0, 100.0, 48.0),
            rect(0.0, 16.0, 100.0, 48.0),
        ]
    );
}

#[test]
fn text_in_a_row_takes_its_width_on_one_line() {
    let root = window(Style::row()).child(label("abcd")).child(label("ef"));
    assert_eq!(
        rects(root, &[&[0], &[1]]),
        [rect(0.0, 0.0, 32.0, 300.0), rect(32.0, 0.0, 16.0, 300.0)]
    );
}

#[test]
fn nesting_many_levels_stays_fast() {
    // Each level asks its child about its size more than once. Without remembering
    // the answers, this many levels would need around 2^40 measurements.
    let mut measured = 0u32;
    let mut measure = |element: &Element, _: f32| {
        measured += 1;
        Size::new(element.name.len() as f32, 10.0)
    };
    let mut node = label("x");
    for _ in 0..40 {
        node = group(Style::row().align(Align::Start)).child(node);
    }
    let root = window(Style::new()).child(node).into_node();
    Layout::new().compute(&root, WINDOW, &mut measure);
    assert!(measured < 100, "measured {measured} times");
}

#[test]
fn nesting_that_asks_many_different_questions_stays_fast() {
    // Alternating directions with elements that grow means each level really does ask
    // its child several different things, so remembering one answer isn't enough.
    let mut measured = 0u32;
    let mut measure = |element: &Element, _: f32| {
        measured += 1;
        Size::new(element.name.len() as f32, 10.0)
    };
    let mut node = label("x");
    for level in 0..40 {
        let style = if level % 2 == 0 {
            Style::row()
        } else {
            Style::column()
        };
        node = group(style.grow(1.0).align(Align::Center)).child(node);
    }
    let root = window(Style::row().align(Align::Center))
        .child(node)
        .into_node();
    Layout::new().compute(&root, WINDOW, &mut measure);
    assert!(measured < 1000, "measured {measured} times");
}

#[test]
fn nesting_that_asks_two_questions_at_every_level_stays_fast() {
    // A limit means each child gets something other than what it asked for, so every
    // level asks a second question, and forgetting answers would double the work.
    let mut measured = 0u32;
    let mut measure = |_: &Element, _: f32| {
        measured += 1;
        Size::new(500.0, 10.0)
    };
    let mut node = label("x").props(Style::new().max_width(100.0));
    for _ in 0..40 {
        node = group(Style::row().align(Align::Start).max_width(100.0)).child(node);
    }
    let root = window(Style::row().align(Align::Start))
        .child(node)
        .into_node();
    Layout::new().compute(&root, WINDOW, &mut measure);
    assert!(measured < 1000, "measured {measured} times");
}

#[test]
fn a_child_that_gets_what_it_asked_for_is_not_asked_again() {
    let mut measured = 0u32;
    let mut measure = |_: &Element, _: f32| {
        measured += 1;
        Size::new(50.0, 10.0)
    };
    let root = window(Style::row().align(Align::Start))
        .child(label("x"))
        .into_node();
    Layout::new().compute(&root, WINDOW, &mut measure);
    // Once for the size it wants, and once to lay out at its final size.
    assert_eq!(measured, 2);
}

/// Labels of different lengths, so each has a different height at this window's width.
fn keyed_labels(keys: &[usize]) -> Node {
    window(Style::new())
        .children(
            keys.iter()
                .map(|&key| label(&"word ".repeat(key * 12)).key(key)),
        )
        .into_node()
}

fn check_update(old: &[usize], new: &[usize], expected_patch: fn(&Patch) -> bool) {
    let (old, new) = (keyed_labels(old), keyed_labels(new));
    let patches = diff(&old, &new);
    assert!(patches.iter().any(expected_patch), "{patches:?}");
    let mut layout = laid_out(&old);
    layout.apply(&patches);
    layout.compute(&new, WINDOW, &mut FixedMetrics::default());
    assert_eq!(all_rects(&layout), all_rects(&laid_out(&new)));
}

#[test]
fn moving_a_child_moves_its_box() {
    let moved = |patch: &Patch| matches!(patch, Patch::Move { .. });
    check_update(&[1, 2, 3, 4], &[4, 1, 2, 3], moved);
    check_update(&[1, 2, 3, 4], &[2, 3, 4, 1], moved);
}

#[test]
fn removing_a_child_removes_its_box() {
    let removed = |patch: &Patch| matches!(patch, Patch::Remove { .. });
    check_update(&[1, 2, 3, 4], &[2, 3, 4], removed);
    check_update(&[1, 2, 3, 4], &[1, 2, 4], removed);
}

#[test]
fn inserting_a_child_makes_room_for_its_box() {
    let inserted = |patch: &Patch| matches!(patch, Patch::Insert { .. });
    check_update(&[2, 3, 4], &[1, 2, 3, 4], inserted);
    check_update(&[1, 2, 4], &[1, 2, 3, 4], inserted);
}

#[test]
fn a_large_reorder_moves_every_box() {
    let reordered = |patch: &Patch| matches!(patch, Patch::Reorder { .. });
    let forward: Vec<usize> = (1..=200).map(|key| key % 7 + key * 8).collect();
    let backward: Vec<usize> = forward.iter().rev().copied().collect();
    check_update(&forward, &backward, reordered);

    // The boxes keep their answers as they move, so nothing inside needs working out.
    let (old, new) = (keyed_labels(&forward), keyed_labels(&backward));
    let mut layout = laid_out(&old);
    layout.apply(&diff(&old, &new));
    layout.compute(&new, WINDOW, &mut FixedMetrics::default());
    assert!(layout.last_solved() <= 2, "solved {}", layout.last_solved());
}

/// A small random number generator, so the trees below are the same on every run.
fn next_random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn random_style(state: &mut u64) -> Style {
    let mut style = if next_random(state) % 2 == 0 {
        Style::row()
    } else {
        Style::column()
    };
    style = match next_random(state) % 6 {
        0 => style.flex(1.0),
        1 => style.grow(1.0),
        2 => style.shrink(1.0),
        3 => style.grow(1.0).shrink(1.0),
        _ => style,
    };
    style = match next_random(state) % 6 {
        0 => style.width(50.0),
        1 => style.width_percent(50.0),
        2 => style.height(50.0),
        3 => style.height_percent(50.0),
        4 => style.min_width(30.0).max_height(70.0),
        _ => style,
    };
    match next_random(state) % 4 {
        0 => style.align(Align::Start),
        1 => style.align(Align::Center),
        2 => style.align(Align::End),
        _ => style,
    }
}

fn random_deep_tree(state: &mut u64, depth: usize) -> Element {
    let own = random_style(state);
    if depth == 0 {
        return label("some words to measure").props(own);
    }
    let mut element = group(own).child(random_deep_tree(state, depth - 1));
    if next_random(state) % 3 == 0 {
        element = element.child(label("beside it").props(random_style(state)));
    }
    element
}

/// Some nesting makes a node get asked several different questions. This is a search
/// for the worst of it, across many combinations of styles. The limit is close to what
/// the search finds, so anything that makes layout repeat its work will exceed it.
#[test]
fn no_combination_of_styles_makes_layout_repeat_much_work() {
    let mut state = 0x1234_5678_9ABC_DEF1u64;
    let mut worst = 0;
    for _ in 0..20_000 {
        let root = window(Style::new())
            .child(random_deep_tree(&mut state, 24))
            .into_node();
        worst = worst.max(laid_out(&root).last_solved());
    }
    assert!(worst <= 200, "worst {worst}");
}

/// Found by the random tests. The parent asks about the text's height under two different
/// limits, and it must not forget that it answered the first one.
#[test]
fn a_node_asked_under_two_limits_still_tells_its_parent_it_grew() {
    let tree = |name: &str| {
        window(Style::new())
            .child(
                group(Style::row().align(Align::Start).width(0.0)).child(
                    label(name).props(
                        Style::new()
                            .width(0.0)
                            .max_height(55.5)
                            .padding_xy(0.0, 5.0),
                    ),
                ),
            )
            .into_node()
    };
    let mut metrics = FixedMetrics::default();
    let mut root = tree("the original name");
    let mut layout = Layout::new();
    layout.compute(&root, WINDOW, &mut metrics);
    for name in ["x", "a longer name that takes up some room on a line"] {
        let next = tree(name);
        layout.apply(&diff(&root, &next));
        layout.compute(&next, WINDOW, &mut metrics);
        assert_eq!(all_rects(&layout), all_rects(&laid_out(&next)), "{name}");
        root = next;
    }
    assert_eq!(layout.rect(&[0, 0]).unwrap().height, 55.5);
}

#[test]
fn changing_two_children_updates_both() {
    let tree = |first: &str, second: &str| {
        window(Style::new())
            .child(label(first))
            .child(label("middle"))
            .child(label(second))
            .into_node()
    };
    let (old, new) = (
        tree("a", "b"),
        tree(&"long ".repeat(30), &"long ".repeat(50)),
    );
    let mut layout = laid_out(&old);
    layout.apply(&diff(&old, &new));
    layout.compute(&new, WINDOW, &mut FixedMetrics::default());
    assert_eq!(all_rects(&layout), all_rects(&laid_out(&new)));
    assert_eq!(layout.rect(&[2]), Some(rect(0.0, 64.0, 400.0, 80.0)));

    // Only one of the two changes size, whichever was reported first or last.
    let long = "long ".repeat(50);
    for (first, second) in [("c", long.as_str()), (long.as_str(), "c")] {
        let new = tree(first, second);
        let mut layout = laid_out(&old);
        layout.apply(&diff(&old, &new));
        layout.compute(&new, WINDOW, &mut FixedMetrics::default());
        assert_eq!(all_rects(&layout), all_rects(&laid_out(&new)));
    }
}

#[test]
fn replacing_a_child_starts_its_box_over() {
    let old = window(Style::new())
        .child(group(Style::new().padding(5.0)).child(group(Style::new().height(50.0))))
        .child(label("after"))
        .into_node();
    let new = window(Style::new())
        .child(Element::new(Role::Button).name("Now a button"))
        .child(label("after"))
        .into_node();
    let patches = diff(&old, &new);
    assert!(matches!(patches.as_slice(), [Patch::Replace { .. }, ..]));
    let mut layout = laid_out(&old);
    layout.apply(&patches);
    layout.compute(&new, WINDOW, &mut FixedMetrics::default());
    assert_eq!(all_rects(&layout), all_rects(&laid_out(&new)));
}

#[test]
fn hit_testing_finds_the_deepest_node() {
    let root = window(Style::new())
        .child(
            group(Style::row().height(100.0))
                .child(group(Style::new().width(50.0)))
                .child(group(Style::new().width(50.0))),
        )
        .child(group(Style::new().height(100.0)))
        .into_node();
    let layout = laid_out(&root);
    assert_eq!(layout.hit_test(60.0, 10.0), Some(vec![0, 1]));
    assert_eq!(layout.hit_test(10.0, 10.0), Some(vec![0, 0]));
    assert_eq!(layout.hit_test(200.0, 10.0), Some(vec![0]));
    assert_eq!(layout.hit_test(10.0, 150.0), Some(vec![1]));
    assert_eq!(layout.hit_test(10.0, 250.0), Some(vec![]));
    assert_eq!(layout.hit_test(500.0, 10.0), None);
}

#[test]
fn a_hidden_child_outside_its_parent_cannot_be_hit() {
    let root = window(Style::new())
        .child(group(Style::new().height(10.0)).child(group(Style::new().height(100.0))))
        .into_node();
    let layout = laid_out(&root);
    assert_eq!(layout.hit_test(5.0, 50.0), Some(vec![]));
}

#[test]
fn walking_visits_parents_first_with_positions_from_the_window() {
    let root = window(Style::new().padding(10.0))
        .child(group(Style::new().padding(5.0)).child(group(Style::new().height(20.0))))
        .into_node();
    let layout = laid_out(&root);
    let mut seen = Vec::new();
    layout.walk(|path, rect| seen.push((path.to_vec(), rect)));
    assert_eq!(
        seen,
        [
            (vec![], rect(0.0, 0.0, 400.0, 300.0)),
            (vec![0], rect(10.0, 10.0, 380.0, 30.0)),
            (vec![0, 0], rect(15.0, 15.0, 370.0, 20.0)),
        ]
    );
}

#[test]
fn an_outline_shows_where_everything_went() {
    let root = window(Style::new())
        .child(label("Hello"))
        .child(group(Style::new().height(10.0)))
        .into_node();
    assert_eq!(
        laid_out(&root).outline(&root),
        "window 0,0 400x300\n  text \"Hello\" 0,0 400x16\n  group 0,16 400x10\n"
    );
}

#[test]
fn asking_for_a_node_that_does_not_exist_is_none() {
    let root = window(Style::new()).into_node();
    assert_eq!(Layout::new().rect(&[]), None);
    assert_eq!(laid_out(&root).rect(&[0]), None);
}

#[test]
fn styles_make_props_that_only_mention_what_differs_from_the_default() {
    assert_eq!(Props::from(Style::new()), Props::new());
    assert_eq!(Props::from(Style::row().gap(4.0)).len(), 2);
    let toolbar = || Element::new(Role::Group).props(style!(Style::row().gap(8.0)));
    assert_eq!(toolbar(), toolbar());
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "must be a number")]
fn a_layout_prop_of_the_wrong_type_is_caught_in_debug_builds() {
    let root = Element::new(Role::Window).prop("gap", "wide").into_node();
    laid_out(&root);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "isn't a value for the align prop")]
fn a_layout_prop_with_a_bad_value_is_caught_in_debug_builds() {
    let root = Element::new(Role::Window)
        .prop("align", "sideways")
        .into_node();
    laid_out(&root);
}

#[test]
fn unrelated_props_are_ignored() {
    let root = Element::new(Role::Window)
        .prop("checked", true)
        .prop("value", "text")
        .into_node();
    assert_eq!(
        laid_out(&root).rect(&[]),
        Some(rect(0.0, 0.0, 400.0, 300.0))
    );
}

fn long_list(cx: &mut Cx, _: &()) -> Element {
    let list = cx.virtual_list(
        "rows",
        ListWindow {
            len: 1_000_000,
            visible: 10,
            overscan: 2,
            reveal: None,
        },
        |cx, index| {
            cx.keyed(
                index,
                |_, index: &usize| label(&format!("row {index}")),
                index,
            )
        },
    );
    let mut props = list.props.clone();
    props.insert("row_height", 20.0);
    Element { props, ..list }
}

#[test]
fn a_virtual_list_places_rows_from_its_scroll_position() {
    let mut runtime = Runtime::new(long_list, ());
    let mut layout = Layout::new();
    let mut metrics = FixedMetrics::default();
    layout.compute(runtime.root(), Size::new(200.0, 200.0), &mut metrics);
    assert_eq!(layout.rect(&[0]), Some(rect(0.0, 0.0, 200.0, 20.0)));
    assert_eq!(layout.rect(&[1]), Some(rect(0.0, 20.0, 200.0, 20.0)));

    runtime
        .dispatch(&[], ActionKind::Scroll, Some(500i64.into()))
        .unwrap();
    let patches = runtime.flush();
    layout.apply(&patches);
    layout.compute(runtime.root(), Size::new(200.0, 200.0), &mut metrics);

    // Two rows of overscan come first, so the row in view is the third, at the top.
    assert_eq!(runtime.root().children[2].key, Some(500usize.into()));
    assert_eq!(layout.rect(&[0]), Some(rect(0.0, -40.0, 200.0, 20.0)));
    assert_eq!(layout.rect(&[2]), Some(rect(0.0, 0.0, 200.0, 20.0)));
}

/// Counts text measurements, to show which nodes were visited.
#[derive(Default)]
struct Counting {
    calls: u32,
    metrics: FixedMetrics,
}

impl rsui_layout::Measure for Counting {
    fn measure(&mut self, element: &Element, max_width: f32) -> Size {
        self.calls += 1;
        self.metrics.measure(element, max_width)
    }
}

fn counter_row(cx: &mut Cx, id: &usize) -> Element {
    let clicks = cx.state(|| 0);
    let name = text!("{id}:{}", cx.get(clicks));
    cx.on(
        Element::new(Role::Text).name(name),
        ActionKind::Click,
        move |cx, _| cx.update(clicks, |clicks| *clicks += 1),
    )
}

fn counter_list(cx: &mut Cx, count: &usize) -> Element {
    let rows: Vec<Node> = (0..*count)
        .map(|id| cx.keyed(id, counter_row, id))
        .collect();
    Element::new(Role::List).children(rows)
}

#[test]
fn a_change_that_keeps_a_nodes_size_only_measures_that_node() {
    let mut runtime = Runtime::new(counter_list, 100);
    let mut layout = Layout::new();
    let mut counting = Counting::default();
    layout.compute(runtime.root(), WINDOW, &mut counting);
    let first_time = counting.calls;
    assert!(first_time >= 100);

    counting.calls = 0;
    runtime.dispatch(&[50], ActionKind::Click, None).unwrap();
    let patches = runtime.flush();
    assert!(matches!(patches.as_slice(), [Patch::SetName { .. }]));
    layout.apply(&patches);
    layout.compute(runtime.root(), WINDOW, &mut counting);
    assert!(counting.calls <= 2, "measured {} times", counting.calls);
}

#[test]
fn computing_again_with_no_changes_measures_nothing() {
    let runtime = Runtime::new(counter_list, 100);
    let mut layout = Layout::new();
    let mut counting = Counting::default();
    layout.compute(runtime.root(), WINDOW, &mut counting);
    counting.calls = 0;
    layout.compute(runtime.root(), WINDOW, &mut counting);
    assert_eq!(counting.calls, 0);
    assert_eq!(layout.last_solved(), 0);
}

#[test]
fn a_change_that_keeps_a_nodes_size_stops_at_that_node() {
    let mut runtime = Runtime::new(counter_list, 1_000);
    let mut layout = Layout::new();
    let mut metrics = FixedMetrics::default();
    layout.compute(runtime.root(), WINDOW, &mut metrics);
    assert!(layout.last_solved() >= 1_000);

    runtime.dispatch(&[500], ActionKind::Click, None).unwrap();
    layout.apply(&runtime.flush());
    layout.compute(runtime.root(), WINDOW, &mut metrics);

    // Its layout, and the question its parent asked about its size. Nothing else.
    assert_eq!(layout.last_solved(), 2);
}

#[test]
fn a_change_of_size_is_passed_up_only_as_far_as_it_matters() {
    let tree = |name: &str| {
        window(Style::new())
            .child(label("unrelated"))
            .child(
                // A fixed height, so the change inside stops here.
                group(Style::new().height(100.0))
                    .child(label(name))
                    .child(label("after")),
            )
            .into_node()
    };
    let (old, new) = (tree("short"), tree(&"long ".repeat(30)));
    let mut layout = laid_out(&old);
    layout.apply(&diff(&old, &new));
    layout.compute(&new, WINDOW, &mut FixedMetrics::default());

    assert_eq!(layout.rect(&[1, 1]), Some(rect(0.0, 64.0, 400.0, 16.0)));
    assert!(layout.last_solved() <= 4, "solved {}", layout.last_solved());
    assert_eq!(all_rects(&layout), all_rects(&laid_out(&new)));
}

#[test]
fn a_change_inside_a_box_asked_several_questions_stops_where_sizes_stay_the_same() {
    // Rows of columns too wide to fit, so each row asks each column how wide it would be, how narrow it can be, and then lays it
    // out: more questions than a box keeps in its two places for answers. Something in one column changes to the same size.
    let tree = |changed: &str| {
        let mut page = window(Style::new());
        for row in 0..12 {
            let mut columns = group(Style::row());
            for column in 0..3 {
                let mut inside = group(Style::new());
                for line in 0..8 {
                    let name = if (row, column, line) == (6, 1, 4) { changed.to_owned() } else { "some words ".repeat(5) };
                    inside = inside.child(label(&name));
                }
                columns = columns.child(inside);
            }
            page = page.child(columns);
        }
        page.into_node()
    };
    let (old, new) = (tree(&"some words ".repeat(5)), tree(&"more words ".repeat(5)));
    let mut layout = laid_out(&old);
    let everything = layout.last_solved();
    layout.apply(&diff(&old, &new));
    layout.compute(&new, WINDOW, &mut FixedMetrics::default());
    assert_eq!(all_rects(&layout), all_rects(&laid_out(&new)));
    // The label, asked again what each was asked, and what it's in, up to the row, which find that nothing's another size. Before
    // the answers were kept, what the row was asked was taken to have changed, and everything was laid out again.
    assert!(layout.last_solved() <= 12, "solved {} of {everything}", layout.last_solved());
}

#[test]
fn a_change_of_height_moves_the_nodes_after_it() {
    let tree = |name: &str| {
        window(Style::new())
            .child(group(Style::new()).child(label(name)))
            .child(label("after"))
            .into_node()
    };
    let (old, new) = (tree("short"), tree(&"long ".repeat(30)));
    let mut layout = laid_out(&old);
    assert_eq!(layout.rect(&[1]), Some(rect(0.0, 16.0, 400.0, 16.0)));

    layout.apply(&diff(&old, &new));
    layout.compute(&new, WINDOW, &mut FixedMetrics::default());

    // 150 characters at 50 per line takes 3 lines.
    assert_eq!(layout.rect(&[0]), Some(rect(0.0, 0.0, 400.0, 48.0)));
    assert_eq!(layout.rect(&[1]), Some(rect(0.0, 48.0, 400.0, 16.0)));
}

#[test]
fn resizing_the_window_lays_everything_out_again() {
    let root = window(Style::row())
        .child(group(Style::new().flex(1.0)))
        .child(group(Style::new().flex(1.0)))
        .into_node();
    let mut layout = laid_out(&root);
    layout.compute(&root, Size::new(100.0, 50.0), &mut FixedMetrics::default());
    assert_eq!(layout.rect(&[1]), Some(rect(50.0, 0.0, 50.0, 50.0)));
}

/// A random style, including settings that conflict.
fn any_style() -> impl Strategy<Value = Style> {
    let size = prop_oneof![Just(0.0f32), Just(10.0), Just(55.5), Just(200.0)];
    let share = prop_oneof![Just(0.0f32), Just(1.0), Just(2.5)];
    (
        (any::<bool>(), 0..5u8, 0..5u8, 0..3u8, size.clone()),
        (0..4u8, 0..4u8, 0..4u8, share.clone(), share),
        (size.clone(), size),
    )
        .prop_map(
            |((row, width, height, flex, gap), (align, justify, limit, grow, shrink), (a, b))| {
                let mut style = if row { Style::row() } else { Style::column() };
                style = match width {
                    0 => style.width(a),
                    1 => style.width_percent(50.0),
                    _ => style,
                };
                style = match height {
                    0 => style.height(b),
                    1 => style.height_percent(25.0),
                    _ => style,
                };
                style = match flex {
                    0 => style.flex(grow),
                    1 => style.grow(grow).shrink(shrink),
                    _ => style,
                };
                style = match limit {
                    0 => style.min_width(a).max_height(b),
                    1 => style.max_width(a).min_height(b),
                    _ => style,
                };
                let align =
                    [Align::Stretch, Align::Start, Align::Center, Align::End][align as usize];
                let justify = [
                    Justify::Start,
                    Justify::Center,
                    Justify::End,
                    Justify::SpaceBetween,
                ][justify as usize];
                style
                    .gap(gap)
                    .padding_xy(a / 10.0, b / 10.0)
                    .align(align)
                    .justify(justify)
            },
        )
}

fn any_tree() -> impl Strategy<Value = Element> {
    let leaf = (
        any_style(),
        prop_oneof![Just(None), (0u64..6).prop_map(Some)],
        prop_oneof![
            Just(""),
            Just("hi"),
            Just("some text long enough to wrap around in a narrow space of a window")
        ],
        any::<bool>(),
    )
        .prop_map(|(style, key, name, text_role)| {
            let role = if text_role { Role::Text } else { Role::Group };
            let element = Element::new(role).props(style).name(name);
            match key {
                Some(key) => element.key(key),
                None => element,
            }
        });
    leaf.prop_recursive(4, 60, 6, |inner| {
        (inner.clone(), vec(inner, 0..6)).prop_map(|(parent, children)| parent.children(children))
    })
}

fn all_rects(layout: &Layout) -> Vec<(Vec<u32>, Rect)> {
    let mut rects = Vec::new();
    layout.walk(|path, rect| rects.push((path.to_vec(), rect)));
    rects
}

/// Whether two layouts put everything in the same place, to a thousandth of a pixel: what's moved rather than laid out again
/// (`solve_moved` in the engine) has the difference added to where it was, which can round apart from adding it all up again.
fn same_rects(a: &[(Vec<u32>, Rect)], b: &[(Vec<u32>, Rect)]) -> bool {
    let near = |x: f32, y: f32| (x - y).abs() < 0.001 || (x.is_nan() && y.is_nan());
    a.len() == b.len()
        && a.iter().zip(b).all(|((at, one), (other_at, other))| {
            at == other_at && near(one.x, other.x) && near(one.y, other.y) && near(one.width, other.width) && near(one.height, other.height)
        })
}

fn root_of(element: Element) -> Node {
    Element::new(Role::Window).child(element).into_node()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    /// However a tree changes, updating a layout gives what computing it from nothing
    /// would. This is what makes it safe to skip the parts that didn't change.
    #[test]
    fn updating_gives_the_same_result_as_starting_over(
        trees in vec(any_tree(), 2..5),
        sizes in vec((10.0f32..600.0, 10.0f32..600.0), 2..5),
    ) {
        let mut metrics = FixedMetrics::default();
        let mut layout = Layout::new();
        let mut previous: Option<Node> = None;
        for (tree, (width, height)) in trees.into_iter().zip(sizes.into_iter().cycle()) {
            let root = root_of(tree);
            let available = Size::new(width, height);
            if let Some(previous) = &previous {
                layout.apply(&diff(previous, &root));
            }
            layout.compute(&root, available, &mut metrics);

            let mut fresh = Layout::new();
            fresh.compute(&root, available, &mut metrics);
            prop_assert_eq!(all_rects(&layout), all_rects(&fresh));
            previous = Some(root);
        }
    }

    /// Small changes to one tree, the usual case, where most of it stays as it was.
    #[test]
    fn small_changes_give_the_same_result_as_starting_over(
        tree in any_tree(),
        names in vec(prop_oneof![Just("x"), Just("a longer name that takes up some room on a line")], 1..8),
        targets in vec(any::<prop::sample::Index>(), 8),
    ) {
        fn with_name(node: &Node, target: &mut usize, name: &str) -> Node {
            let mut element = (**node).clone();
            if *target == 0 {
                element.name = name.into();
            }
            *target = target.wrapping_sub(1);
            element.children = element
                .children
                .iter()
                .map(|child| with_name(child, target, name))
                .collect();
            triomphe::Arc::new(element)
        }
        fn count(node: &Node) -> usize {
            1 + node.children.iter().map(count).sum::<usize>()
        }

        let mut metrics = FixedMetrics::default();
        let mut root = root_of(tree);
        let mut layout = Layout::new();
        layout.compute(&root, WINDOW, &mut metrics);
        for (name, target) in names.into_iter().zip(targets) {
            let mut position = target.index(count(&root));
            let next = with_name(&root, &mut position, name);
            layout.apply(&diff(&root, &next));
            layout.compute(&next, WINDOW, &mut metrics);

            let mut fresh = Layout::new();
            fresh.compute(&next, WINDOW, &mut metrics);
            let (updated, fresh) = (all_rects(&layout), all_rects(&fresh));
            prop_assert!(same_rects(&updated, &fresh), "{updated:?} against {fresh:?}");
            root = next;
        }
    }

    /// Putting children in a long column and taking them out, or making one longer, where what's after moves rather than being
    /// laid out again (`solve_moved` in the engine), gives what laying it all out from nothing does: with margins that go
    /// together or don't, a gap, padding, and the column as tall as it's told, which it's told from how tall it says it is, and
    /// something at its end placed by itself from its bottom, as what's drawn over an editor is, which changes size by itself.
    #[test]
    fn putting_in_and_taking_out_of_a_long_column_gives_the_same_result_as_starting_over(
        kinds in vec(0..4u8, 10..40),
        edits in vec((0..4u8, any::<prop::sample::Index>(), 0..4u8), 1..10),
        collapse in any::<bool>(),
        wrapped in any::<bool>(),
    ) {
        let child = |kind: u8, key: u64| {
            let margin = [0.0, 4.0, 10.0, -3.0][kind as usize];
            let text = if kind == 2 { "some text long enough to wrap around in a narrow space" } else { "hi" };
            Element::new(Role::Text)
                .name(text)
                .key(key)
                .props(with_margin(Style::new(), margin, 0.0, margin + 1.0, 0.0))
        };
        let apart = |kind: u8| {
            with(
                Element::new(Role::Text).name(["", "x", "two words", "a longer name that takes up some room"][kind as usize]).key(u64::MAX),
                &[("position", words("absolute")), ("bottom", rsui_core::PropValue::Float(0.0)), ("left", rsui_core::PropValue::Float(5.0))],
            )
        };
        let tree = |kids: &[(u8, u64)], over: u8| {
            let column = group(Style::new().padding(3.0).gap(2.0));
            let column = if collapse { with(column, &[("collapse_margins", rsui_core::PropValue::Bool(true))]) } else { column };
            let column = column.children(kids.iter().map(|&(kind, key)| child(kind, key))).child(apart(over));
            let column = if wrapped { group(Style::new().width(120.0)).child(column) } else { column };
            Element::new(Role::Window).child(column).into_node()
        };
        let mut kids: Vec<(u8, u64)> = kinds.iter().enumerate().map(|(at, &kind)| (kind, at as u64)).collect();
        let mut next_key = kids.len() as u64;
        let mut over = 1u8;
        let mut metrics = FixedMetrics::default();
        let mut root = tree(&kids, over);
        let mut layout = Layout::new();
        layout.compute(&root, WINDOW, &mut metrics);
        for (what, at, kind) in edits {
            let index = at.index(kids.len());
            match what {
                0 => {
                    kids.insert(index, (kind, next_key));
                    next_key += 1;
                }
                1 if kids.len() > 1 => {
                    kids.remove(index);
                }
                3 => over = kind,
                _ => kids[index].0 = kind,
            }
            let next = tree(&kids, over);
            layout.apply(&diff(&root, &next));
            layout.compute(&next, WINDOW, &mut metrics);
            let mut fresh = Layout::new();
            fresh.compute(&next, WINDOW, &mut metrics);
            let (updated, fresh) = (all_rects(&layout), all_rects(&fresh));
            prop_assert!(same_rects(&updated, &fresh), "{updated:?} against {fresh:?}");
            root = next;
        }
    }

    /// Whatever the styles say, sizes are real sizes.
    #[test]
    fn every_size_is_finite_and_not_negative(tree in any_tree()) {
        let root = root_of(tree);
        let layout = laid_out(&root);
        for (path, rect) in all_rects(&layout) {
            prop_assert!(rect.width.is_finite() && rect.width >= 0.0, "{path:?} {rect:?}");
            prop_assert!(rect.height.is_finite() && rect.height >= 0.0, "{path:?} {rect:?}");
            prop_assert!(rect.x.is_finite() && rect.y.is_finite(), "{path:?} {rect:?}");
        }
    }

    /// Reordering keyed children moves their boxes with them, and can't be told apart
    /// from starting over.
    #[test]
    fn reordering_gives_the_same_result_as_starting_over(
        order in Just((0..120usize).collect::<Vec<_>>()).prop_shuffle(),
    ) {
        let tree = |keys: &[usize]| {
            window(Style::new())
                .children(keys.iter().map(|&key| label(&"word ".repeat(key % 20)).key(key)))
                .into_node()
        };
        let sorted: Vec<usize> = (0..120).collect();
        let (old, new) = (tree(&sorted), tree(&order));
        let mut layout = laid_out(&old);
        layout.apply(&diff(&old, &new));
        layout.compute(&new, WINDOW, &mut FixedMetrics::default());
        prop_assert_eq!(all_rects(&layout), all_rects(&laid_out(&new)));
    }
}

mod scrolling {
    use super::*;
    use rsui_layout::LayoutVisitor;

    fn area(children: Vec<Element>, style: Style) -> Element {
        Element::new(Role::Group)
            .props(Props::from(style))
            .children(children.into_iter().map(Into::into).collect::<Vec<Node>>())
    }

    fn sized(width: f32, height: f32) -> Element {
        Element::new(Role::Group).props(Props::from(Style::new().width(width).height(height)))
    }

    fn laid_out(root: &Node, size: Size) -> Layout {
        let mut layout = Layout::new();
        layout.compute(root, size, &mut FixedMetrics::default());
        layout
    }

    /// An area that scrolls both ways with scrollbars that take room, as a page's do.
    fn page(children: Vec<Element>) -> Element {
        area(children, Style::new().scroll_x(0.0).scroll_y(0.0)).prop("scrollbar_room", 17.0)
    }

    #[test]
    fn a_scrollbar_that_leaves_too_little_room_across_brings_the_other() {
        // 90 wide fits in 100, but not in what's left beside a bar down the side, which being 110 tall needs.
        let root: Node = page(vec![sized(90.0, 110.0)]).into();
        let layout = laid_out(&root, Size::new(100.0, 100.0));
        let info = layout.scroll_info(&[], &root).expect("it scrolls");
        assert_eq!(info.view, Size::new(83.0, 83.0));
        assert_eq!(info.content, Size::new(90.0, 110.0));
    }

    #[test]
    fn what_fits_needs_no_scrollbars() {
        let root: Node = page(vec![sized(100.0, 100.0)]).into();
        let layout = laid_out(&root, Size::new(100.0, 100.0));
        let info = layout.scroll_info(&[], &root).expect("it scrolls");
        assert_eq!(info.view, Size::new(100.0, 100.0));
    }

    #[test]
    fn it_scrolls_as_far_as_what_a_scrollbar_hides() {
        let root: Node = area(
            vec![sized(10.0, 200.0)],
            Style::new().scroll_x(0.0).scroll_y(1000.0),
        )
        .prop("scrollbar_room", 17.0)
        .into();
        let layout = laid_out(&root, Size::new(100.0, 100.0));
        // Only a bar down the side, which takes nothing from how tall it is, so it goes 100 down.
        assert_eq!(layout.rect(&[0]).unwrap().y, -100.0);
        let wide: Node = area(
            vec![sized(200.0, 200.0)],
            Style::new().scroll_x(1000.0).scroll_y(1000.0),
        )
        .prop("scrollbar_room", 17.0)
        .into();
        let layout = laid_out(&wide, Size::new(100.0, 100.0));
        // Both, so 17 more each way is hidden.
        let rect = layout.rect(&[0]).unwrap();
        assert_eq!((rect.x, rect.y), (-117.0, -117.0));
    }

    #[test]
    fn children_move_by_the_scroll_and_the_area_does_not() {
        let root: Node = area(
            vec![sized(10.0, 100.0), sized(10.0, 100.0)],
            Style::new().scroll_y(30.0),
        )
        .into();
        let layout = laid_out(&root, Size::new(50.0, 80.0));
        assert_eq!(layout.rect(&[]), Some(Rect::new(0.0, 0.0, 50.0, 80.0)));
        assert_eq!(layout.rect(&[0]), Some(Rect::new(0.0, -30.0, 10.0, 100.0)));
        assert_eq!(layout.rect(&[1]), Some(Rect::new(0.0, 70.0, 10.0, 100.0)));
        let info = layout.scroll_info(&[], &root).expect("it scrolls");
        assert_eq!(info.y, 30.0);
        assert_eq!(info.content, Size::new(50.0, 200.0));
        assert_eq!(info.view, Size::new(50.0, 80.0));
        assert_eq!(info.furthest_y(), 120.0);
    }

    #[test]
    fn padding_counts_as_part_of_the_content() {
        let root: Node = area(
            vec![sized(10.0, 100.0)],
            Style::new().scroll_y(1000.0).padding(5.0),
        )
        .into();
        let layout = laid_out(&root, Size::new(50.0, 60.0));
        let info = layout.scroll_info(&[], &root).expect("it scrolls");
        assert_eq!(info.content.height, 110.0);
        assert_eq!(info.y, 50.0, "as far as it goes");
        assert_eq!(layout.rect(&[0]).unwrap().y, -45.0);
    }

    #[test]
    fn scrolling_is_held_within_what_there_is() {
        for (asked, expected) in [(-5.0, 0.0), (f32::NAN, 0.0), (1e9, 120.0), (50.0, 50.0)] {
            let root: Node = area(vec![sized(10.0, 200.0)], Style::new().scroll_y(asked)).into();
            let layout = laid_out(&root, Size::new(50.0, 80.0));
            assert_eq!(layout.rect(&[0]).unwrap().y, -expected, "{asked}");
        }
    }

    #[test]
    fn content_shorter_than_the_area_can_fill_it() {
        let root: Node = area(
            vec![
                sized(10.0, 10.0),
                Element::new(Role::Group).props(Props::from(Style::new().flex(1.0))),
            ],
            Style::new().scroll_y(0.0),
        )
        .into();
        let layout = laid_out(&root, Size::new(50.0, 80.0));
        assert_eq!(layout.rect(&[1]), Some(Rect::new(0.0, 10.0, 50.0, 70.0)));
        let info = layout.scroll_info(&[], &root).expect("it scrolls");
        assert_eq!(info.furthest_y(), 0.0);
    }

    #[test]
    fn an_area_with_no_size_of_its_own_takes_what_it_needs_up_to_its_limit() {
        let inner = |limit: f32| {
            area(
                vec![sized(10.0, 200.0)],
                Style::new().scroll_y(0.0).max_height(limit),
            )
        };
        let root: Node = area(
            vec![inner(120.0), inner(1000.0)],
            Style::new().align(Align::Start),
        )
        .into();
        let layout = laid_out(&root, Size::new(400.0, 400.0));
        assert_eq!(layout.rect(&[0]).unwrap().height, 120.0);
        assert_eq!(layout.rect(&[1]).unwrap().height, 200.0);
    }

    #[test]
    fn it_does_not_ask_more_of_its_parent_than_a_box_of_that_size_would() {
        let root: Node = area(
            vec![area(
                vec![sized(10.0, 200.0)],
                Style::new().scroll_y(0.0).height(80.0),
            )],
            Style::new(),
        )
        .into();
        let layout = laid_out(&root, Size::new(100.0, 100.0));
        assert_eq!(layout.rect(&[0]).unwrap().height, 80.0);
    }

    #[test]
    fn a_child_can_only_be_hit_inside_the_area() {
        let root: Node = area(
            vec![
                area(
                    vec![sized(10.0, 200.0)],
                    Style::new().scroll_y(10.0).height(50.0),
                ),
                sized(10.0, 50.0),
            ],
            Style::new(),
        )
        .into();
        let layout = laid_out(&root, Size::new(100.0, 100.0));
        assert_eq!(layout.hit_test(5.0, 25.0), Some(vec![0, 0]));
        assert_eq!(layout.hit_test(5.0, 75.0), Some(vec![1]));
    }

    #[test]
    fn what_is_out_of_view_is_not_visited() {
        #[derive(Default)]
        struct Count(usize);
        impl LayoutVisitor for Count {
            fn enter(&mut self, _: &Element, _: Rect) {
                self.0 += 1;
            }
            fn exit(&mut self, _: &Element) {}
        }
        let root: Node = area(
            (0..1000).map(|_| sized(10.0, 10.0)).collect(),
            Style::new().scroll_y(5000.0),
        )
        .into();
        let layout = laid_out(&root, Size::new(100.0, 40.0));
        let mut count = Count::default();
        layout.visit(&root, &mut count);
        assert!(count.0 <= 6, "visited {}", count.0);
    }

    #[test]
    fn scrolling_does_not_work_out_what_is_inside_again() {
        let child = || area((0..50).map(|_| sized(10.0, 10.0)).collect(), Style::new());
        let build = |scroll: f32| -> Node {
            area(vec![child(), child()], Style::new().scroll_y(scroll)).into()
        };
        let before = build(0.0);
        let mut layout = Layout::new();
        layout.compute(
            &before,
            Size::new(100.0, 40.0),
            &mut FixedMetrics::default(),
        );
        let after: Node = Element {
            props: Props::from(Style::new().scroll_y(25.0)),
            ..(*before).clone()
        }
        .into();
        layout.apply(&diff(&before, &after));
        layout.compute(&after, Size::new(100.0, 40.0), &mut FixedMetrics::default());
        assert!(layout.last_solved() <= 2, "solved {}", layout.last_solved());
        assert_eq!(layout.rect(&[0]).unwrap().y, -25.0);
    }
}

#[test]
fn margins_take_up_room_in_a_column() {
    let root = window(Style::new())
        .child(Element::new(Role::Group).props(with_margin(
            Style::new().height(50.0),
            10.0,
            20.0,
            5.0,
            30.0,
        )))
        .child(group(Style::new().height(30.0)));
    assert_eq!(
        rects(root, &[&[0], &[1]]),
        [rect(30.0, 10.0, 350.0, 50.0), rect(0.0, 65.0, 400.0, 30.0)]
    );
}

#[test]
fn margins_take_up_room_in_a_row_with_flex_sharing_what_is_left() {
    let root = window(Style::row().gap(10.0))
        .child(Element::new(Role::Group).props(with_margin(
            Style::new().width(50.0),
            5.0,
            5.0,
            5.0,
            5.0,
        )))
        .child(Element::new(Role::Group).props(with_margin(
            Style::new().flex(1.0),
            0.0,
            0.0,
            0.0,
            20.0,
        )));
    // 400 - 50 - 10 for the first's margins - 10 for the gap - 20 for the second's margin leaves 310.
    assert_eq!(
        rects(root, &[&[0], &[1]]),
        [rect(5.0, 5.0, 50.0, 290.0), rect(90.0, 0.0, 310.0, 300.0)]
    );
}

#[test]
fn something_as_big_as_its_contents_includes_their_margins() {
    let root = window(Style::new().align(Align::Start)).child(
        group(Style::row())
            .child(Element::new(Role::Group).props(with_margin(
                Style::new().width(40.0).height(20.0),
                10.0,
                10.0,
                10.0,
                10.0,
            )))
            .child(group(Style::new().width(40.0).height(20.0))),
    );
    assert_eq!(rects(root, &[&[0]]), [rect(0.0, 0.0, 100.0, 40.0)]);
}

#[test]
fn a_margin_can_be_less_than_nothing_to_overlap_what_is_next_to_it() {
    let root = window(Style::new())
        .child(group(Style::new().height(50.0)))
        .child(Element::new(Role::Group).props(with_margin(
            Style::new().height(50.0),
            -20.0,
            0.0,
            0.0,
            0.0,
        )));
    assert_eq!(rects(root, &[&[1]]), [rect(0.0, 30.0, 400.0, 50.0)]);
}

#[test]
fn margins_keep_something_centered_or_at_the_end() {
    let root = window(Style::new().align(Align::Center)).child(Element::new(Role::Group).props(
        with_margin(Style::new().width(100.0).height(10.0), 0.0, 0.0, 0.0, 40.0),
    ));
    assert_eq!(rects(root, &[&[0]]), [rect(170.0, 0.0, 100.0, 10.0)]);
    let root = window(Style::new().align(Align::End)).child(Element::new(Role::Group).props(
        with_margin(Style::new().width(100.0).height(10.0), 0.0, 30.0, 0.0, 0.0),
    ));
    assert_eq!(rects(root, &[&[0]]), [rect(270.0, 0.0, 100.0, 10.0)]);
}

/// Something with props that aren't in `Style`, as they're mostly written from TSX.
fn with(element: Element, props: &[(&str, rsui_core::PropValue)]) -> Element {
    let mut element = element;
    for (name, value) in props {
        element = element.prop(*name, value.clone());
    }
    element
}

fn int(value: i64) -> rsui_core::PropValue {
    rsui_core::PropValue::Int(value)
}

fn words(value: &str) -> rsui_core::PropValue {
    value.into()
}

#[test]
fn a_length_can_be_a_percentage_and_pixels_together() {
    let parent = group(Style::new().width(200.0).height(100.0))
        .child(with(
            group(Style::new().width(14.0).height(14.0)),
            &[
                ("position", words("absolute")),
                ("left", words("100% + -14px")),
            ],
        ))
        .child(with(
            group(Style::new().height(10.0)),
            &[("width", words("50% + 10px"))],
        ));
    assert_eq!(
        rects(window(Style::new()).child(parent), &[&[0, 0], &[0, 1]]),
        [rect(186.0, 0.0, 14.0, 14.0), rect(0.0, 0.0, 110.0, 10.0)]
    );
}

#[test]
fn something_absolute_takes_up_no_room_and_goes_where_it_says_in_its_parent() {
    let root = window(Style::new().padding(10.0))
        .child(group(Style::new().height(50.0)))
        .child(with(
            group(Style::new().width(30.0).height(20.0)),
            &[
                ("position", words("absolute")),
                ("top", int(5)),
                ("left", int(7)),
            ],
        ))
        .child(group(Style::new().height(50.0)));
    assert_eq!(
        rects(root, &[&[0], &[1], &[2]]),
        [
            rect(10.0, 10.0, 380.0, 50.0),
            rect(7.0, 5.0, 30.0, 20.0),
            rect(10.0, 60.0, 380.0, 50.0),
        ]
    );
}

#[test]
fn something_absolute_is_in_its_parent_rather_than_the_window() {
    let root =
        window(Style::new().padding(100.0)).child(group(Style::new().height(50.0)).child(with(
            group(Style::new().width(30.0).height(20.0)),
            &[
                ("position", words("absolute")),
                ("right", int(4)),
                ("bottom", int(6)),
            ],
        )));
    assert_eq!(rects(root, &[&[0, 0]]), [rect(266.0, 124.0, 30.0, 20.0)]);
}

#[test]
fn both_sides_of_something_absolute_say_how_big_it_is() {
    let root = window(Style::new()).child(with(
        group(Style::new()),
        &[
            ("position", words("absolute")),
            ("left", int(10)),
            ("right", int(20)),
            ("top", int(30)),
            ("bottom", int(40)),
        ],
    ));
    assert_eq!(rects(root, &[&[0]]), [rect(10.0, 30.0, 370.0, 230.0)]);
}

#[test]
fn percentages_and_translate_center_something_the_usual_way() {
    let root = window(Style::new()).child(with(
        group(Style::new().width(100.0).height(40.0)),
        &[
            ("position", words("absolute")),
            ("left", words("50%")),
            ("top", words("50%")),
            ("translate_x", words("-50%")),
            ("translate_y", words("-50%")),
        ],
    ));
    assert_eq!(rects(root, &[&[0]]), [rect(150.0, 130.0, 100.0, 40.0)]);
}

#[test]
fn translate_moves_something_without_moving_what_is_around_it() {
    let root = window(Style::new())
        .child(with(
            group(Style::new().height(50.0)),
            &[("translate_x", int(5)), ("translate_y", int(-3))],
        ))
        .child(group(Style::new().height(50.0)));
    assert_eq!(
        rects(root, &[&[0], &[1]]),
        [rect(5.0, -3.0, 400.0, 50.0), rect(0.0, 50.0, 400.0, 50.0)]
    );
}

fn grid(columns: &str) -> Element {
    with(
        window(Style::new()),
        &[("display", words("grid")), ("columns", words(columns))],
    )
}

fn cell(height: f32) -> Element {
    group(Style::new().height(height))
}

#[test]
fn a_grid_puts_children_in_rows_and_columns() {
    let root = grid("1fr 1fr")
        .child(cell(10.0))
        .child(cell(20.0))
        .child(cell(30.0));
    assert_eq!(
        rects(root, &[&[0], &[1], &[2]]),
        [
            rect(0.0, 0.0, 200.0, 10.0),
            rect(200.0, 0.0, 200.0, 20.0),
            rect(0.0, 20.0, 200.0, 30.0),
        ]
    );
}

#[test]
fn a_grid_has_gaps_and_mixes_kinds_of_column() {
    let root = with(
        grid("100px 1fr 50%").props(Style::new().gap(10.0)),
        &[
            ("display", words("grid")),
            ("columns", words("100px 1fr 50%")),
        ],
    )
    .child(cell(10.0))
    .child(cell(10.0))
    .child(cell(10.0))
    .child(cell(10.0));
    assert_eq!(
        rects(root, &[&[0], &[1], &[2], &[3]]),
        [
            rect(0.0, 0.0, 100.0, 10.0),
            rect(110.0, 0.0, 80.0, 10.0),
            rect(200.0, 0.0, 200.0, 10.0),
            rect(0.0, 20.0, 100.0, 10.0),
        ]
    );
}

#[test]
fn something_in_a_grid_can_span_and_say_where_it_goes() {
    let root = grid("1fr 1fr 1fr")
        .child(with(cell(10.0), &[("grid_column", words("span 2"))]))
        .child(cell(10.0))
        .child(with(cell(10.0), &[("grid_column", words("1 / -1"))]))
        .child(with(
            cell(10.0),
            &[("grid_column", int(3)), ("grid_row", int(4))],
        ));
    let third = 400.0 / 3.0;
    let got = rects(root, &[&[0], &[1], &[2], &[3]]);
    assert_eq!(got[0].width, third * 2.0);
    assert_eq!(got[1].x, third * 2.0);
    assert_eq!((got[2].x, got[2].y, got[2].width), (0.0, 10.0, 400.0));
    assert_eq!((got[3].x, got[3].y), (third * 2.0, 20.0));
}

#[test]
fn rows_of_a_grid_can_say_how_tall_they_are_and_share_what_is_left() {
    let root = with(grid("1fr"), &[("rows", words("40px 1fr 2fr"))])
        .child(group(Style::new()))
        .child(group(Style::new()))
        .child(group(Style::new()));
    assert_eq!(
        rects(root, &[&[0], &[1], &[2]]),
        [
            rect(0.0, 0.0, 400.0, 40.0),
            rect(0.0, 40.0, 400.0, 260.0 / 3.0),
            rect(0.0, 40.0 + 260.0 / 3.0, 400.0, 520.0 / 3.0),
        ]
    );
}

#[test]
fn a_grid_as_big_as_its_contents_works_out_its_columns_from_them() {
    let root = window(Style::new().align(Align::Start)).child(
        with(
            group(Style::new()),
            &[("display", words("grid")), ("columns", words("auto auto"))],
        )
        .child(group(Style::new().width(70.0).height(10.0)))
        .child(group(Style::new().width(30.0).height(25.0))),
    );
    assert_eq!(
        rects(root, &[&[0], &[0, 0], &[0, 1]]),
        [
            rect(0.0, 0.0, 100.0, 25.0),
            rect(0.0, 0.0, 70.0, 10.0),
            rect(70.0, 0.0, 30.0, 25.0)
        ]
    );
}

#[test]
fn what_is_in_a_grid_can_be_aligned_in_its_cell() {
    let root = with(grid("200px 200px"), &[("justify_items", words("center"))])
        .props(Style::new().align(Align::Center))
        .child(group(Style::new().width(100.0).height(10.0)))
        .child(group(Style::new().width(50.0).height(30.0)));
    let root = with(
        root,
        &[
            ("display", words("grid")),
            ("columns", words("200px 200px")),
            ("justify_items", words("center")),
        ],
    );
    assert_eq!(
        rects(root, &[&[0], &[1]]),
        [rect(50.0, 10.0, 100.0, 10.0), rect(275.0, 0.0, 50.0, 30.0)]
    );
}

#[test]
fn text_that_can_wrap_lets_what_its_in_get_narrower_than_its_one_line() {
    let item = || {
        with(
            Element::new(Role::Text)
                .name("abcdefghijklmnop")
                .props(Style::new().width(150.0).shrink(1.0)),
            &[("min_auto", rsui_core::PropValue::Bool(true))],
        )
    };
    let row = group(Style::row().width(200.0)).child(item()).child(item());
    let placed = rects(window(Style::new()).child(row), &[&[0, 0], &[0, 1]]);
    assert_eq!((placed[0].width, placed[1].width), (100.0, 100.0));
}

#[test]
fn something_that_can_shrink_stops_at_how_narrow_what_is_in_it_can_be() {
    // `ab` is 16 wide on one line and 8 at its narrowest. Shrinking with the rest would make it narrower than that.
    let text = with(
        Element::new(Role::Text)
            .name("ab")
            .props(Style::new().shrink(1.0)),
        &[("min_auto", rsui_core::PropValue::Bool(true))],
    );
    let row = group(Style::row().width(100.0))
        .child(text)
        .child(group(Style::new().width(400.0).height(10.0).shrink(1.0)));
    let placed = rects(window(Style::new()).child(row), &[&[0, 0], &[0, 1]]);
    assert_eq!((placed[0].width, placed[1].width), (8.0, 92.0));
}

#[test]
fn margins_that_are_auto_take_what_is_left_along_a_row() {
    let auto = |name: &str| {
        with(
            group(Style::new().width(20.0).height(10.0)),
            &[(name, rsui_core::PropValue::Bool(true))],
        )
    };
    let row = group(Style::row().width(200.0))
        .child(group(Style::new().width(20.0).height(10.0)))
        .child(auto("margin_left_auto"));
    let placed = rects(window(Style::new()).child(row), &[&[0, 1]]);
    assert_eq!(placed[0].x, 180.0);
    let row = group(Style::row().width(200.0)).child(with(
        auto("margin_left_auto"),
        &[("margin_right_auto", rsui_core::PropValue::Bool(true))],
    ));
    let placed = rects(window(Style::new()).child(row), &[&[0, 0]]);
    assert_eq!(placed[0].x, 90.0);
}

#[test]
fn a_margin_less_than_nothing_leaves_more_for_what_grows() {
    let row = group(Style::row().width(200.0))
        .child(Element::new(Role::Group).props(with_margin(
            Style::new().width(100.0),
            0.0,
            -20.0,
            0.0,
            0.0,
        )))
        .child(group(Style::new().grow(1.0)));
    assert_eq!(
        rects(window(Style::new()).child(row), &[&[0, 1]])[0].width,
        120.0
    );
}

#[test]
fn margins_go_through_a_block_with_nothing_at_its_edge_as_css_has_them() {
    let block = || {
        with(
            group(Style::new()),
            &[("collapse_margins", rsui_core::PropValue::Bool(true))],
        )
    };
    let margined = |top: f32, bottom: f32| {
        Element::new(Role::Group).props(with_margin(
            Style::new().height(10.0),
            top,
            0.0,
            bottom,
            0.0,
        ))
    };
    // A wrapper with no margin of its own, whose first and last children have them: 16 after the first, 20 before the next, which
    // go together as 20, and the wrapper's last one's 16 goes on past it.
    let wrapper = block()
        .child(margined(20.0, 16.0))
        .child(margined(20.0, 16.0));
    let root = block()
        .child(margined(0.0, 16.0))
        .child(wrapper)
        .child(margined(20.0, 0.0));
    let placed = rects(
        window(Style::new()).child(root),
        &[&[0, 0], &[0, 1, 0], &[0, 1, 1], &[0, 2], &[0, 1]],
    );
    let tops: Vec<f32> = placed.iter().map(|rect| rect.y).collect();
    assert_eq!(tops, [0.0, 30.0, 60.0, 90.0, 30.0]);
    // The wrapper itself is only as big as what's in it, with the margins that went out of it outside it, as CSS has them.
    assert_eq!(placed[4].height, 40.0);

    // Padding keeps them in.
    let padded = with(
        group(Style::new().padding(5.0)),
        &[("collapse_margins", rsui_core::PropValue::Bool(true))],
    )
    .child(margined(20.0, 0.0));
    let root = block().child(margined(0.0, 16.0)).child(padded);
    let placed = rects(window(Style::new()).child(root), &[&[0, 1], &[0, 1, 0]]);
    assert_eq!((placed[0].y, placed[1].y), (26.0, 51.0));
}

#[test]
fn a_grid_that_flows_down_first_puts_each_thing_in_a_column_of_its_own() {
    let grid = with(
        group(
            Style::new()
                .width(300.0)
                .height(200.0)
                .justify(Justify::Center),
        ),
        &[
            ("display", words("grid")),
            ("grid_flow", words("column")),
            ("column_gap", int(10)),
            ("align_content", words("start")),
        ],
    )
    .child(group(Style::new().width(100.0).height(40.0)))
    .child(group(Style::new().width(50.0).height_percent(100.0)));
    // 160 wide altogether, in the middle of 300, and only as tall as the taller one, with the other as tall as its cell.
    assert_eq!(
        rects(window(Style::new()).child(grid), &[&[0, 0], &[0, 1]]),
        [rect(70.0, 0.0, 100.0, 40.0), rect(180.0, 0.0, 50.0, 40.0)]
    );
}

#[test]
fn rows_left_to_say_nothing_share_out_the_height_of_a_grid_from_a_page() {
    let rows = |content: Option<&str>| {
        let mut grid = with(
            group(Style::new().width(100.0).height(200.0)),
            &[
                ("display", words("grid")),
                ("min_auto", rsui_core::PropValue::Bool(true)),
            ],
        );
        if let Some(content) = content {
            grid = with(grid, &[("align_content", words(content))]);
        }
        let grid = grid
            .child(group(Style::new().height(20.0).width(10.0)))
            .child(group(Style::new().height(20.0).width(10.0)));
        rects(window(Style::new()).child(grid), &[&[0, 0], &[0, 1]])
    };
    assert_eq!(rows(None)[1].y, 100.0);
    assert_eq!(rows(Some("start"))[1].y, 20.0);
    assert_eq!(rows(Some("end"))[1].y, 180.0);
    assert_eq!(rows(Some("space-between"))[1].y, 180.0);
}

#[test]
fn something_absolute_or_on_top_in_a_grid_takes_up_no_cell() {
    let root = grid("1fr 1fr")
        .child(with(
            cell(10.0),
            &[
                ("position", words("absolute")),
                ("top", int(1)),
                ("left", int(2)),
            ],
        ))
        .child(cell(10.0))
        .child(cell(10.0));
    let got = rects(root, &[&[0], &[1], &[2]]);
    assert_eq!((got[0].x, got[0].y), (2.0, 1.0));
    assert_eq!(got[1], rect(0.0, 0.0, 200.0, 10.0));
    assert_eq!(got[2], rect(200.0, 0.0, 200.0, 10.0));
}

#[test]
fn updating_any_of_these_gives_what_working_it_out_from_nothing_does() {
    let before = window(Style::new())
        .child(Element::new(Role::Group).props(with_margin(
            Style::new().height(50.0),
            5.0,
            5.0,
            5.0,
            5.0,
        )))
        .child(with(
            group(Style::new().width(10.0).height(10.0)),
            &[("position", words("absolute")), ("top", int(1))],
        ))
        .child(
            grid("1fr 1fr")
                .props(Style::new().height(100.0))
                .child(cell(10.0)),
        )
        .into_node();
    let after = window(Style::new())
        .child(Element::new(Role::Group).props(with_margin(
            Style::new().height(50.0),
            9.0,
            9.0,
            9.0,
            9.0,
        )))
        .child(with(
            group(Style::new().width(10.0).height(10.0)),
            &[("position", words("absolute")), ("top", int(7))],
        ))
        .child(
            with(
                group(Style::new().height(100.0)),
                &[("display", words("grid")), ("columns", words("1fr 3fr"))],
            )
            .child(cell(10.0))
            .child(cell(20.0)),
        )
        .into_node();
    let mut layout = Layout::new();
    let mut metrics = FixedMetrics::default();
    layout.compute(&before, WINDOW, &mut metrics);
    layout.apply(&diff(&before, &after));
    layout.compute(&after, WINDOW, &mut metrics);
    let fresh = laid_out(&after);
    for path in [&[0u32][..], &[1], &[2], &[2, 0], &[2, 1]] {
        assert_eq!(layout.rect(path), fresh.rect(path), "at {path:?}");
    }
}

/// A row that wraps, as a line of a page is, with what's in it lined up by its text.
fn line(style: Style) -> Element {
    group(style.align(Align::Baseline)).prop("flex_wrap", true)
}

#[test]
fn text_and_a_box_on_a_line_sit_on_the_same_baseline() {
    // Text is 16 high with its baseline 12 down. The box has 10 above its text, so its baseline is 22 down.
    let root = window(Style::new()).child(
        line(Style::row())
            .child(label("ab"))
            .child(group(Style::new().padding_each(10.0, 0.0, 0.0, 0.0)).child(label("cd"))),
    );
    assert_eq!(
        rects(root, &[&[0], &[0, 0], &[0, 1]]),
        [
            rect(0.0, 0.0, 400.0, 26.0),
            rect(0.0, 10.0, 16.0, 16.0),
            rect(16.0, 0.0, 16.0, 26.0)
        ]
    );
}

#[test]
fn a_box_with_no_text_sits_on_the_line_by_its_bottom() {
    let root = window(Style::new()).child(
        line(Style::row())
            .child(label("ab"))
            .child(group(Style::new().width(10.0).height(30.0))),
    );
    // The box's bottom is where the text's baseline is, and the text's line goes 4 below that.
    assert_eq!(
        rects(root, &[&[0], &[0, 0], &[0, 1]]),
        [
            rect(0.0, 0.0, 400.0, 34.0),
            rect(0.0, 18.0, 16.0, 16.0),
            rect(16.0, 0.0, 10.0, 30.0)
        ]
    );
}

#[test]
fn something_alone_on_a_line_is_as_tall_as_it_is() {
    let input = Element::new(Role::TextInput).props(Style::new().padding(3.0));
    let root = window(Style::new()).child(line(Style::row()).child(input));
    assert_eq!(
        rects(root, &[&[0], &[0, 0]]),
        [rect(0.0, 0.0, 400.0, 22.0), rect(0.0, 0.0, 206.0, 22.0)]
    );
}

#[test]
fn lines_that_wrap_each_line_up_their_own_text() {
    let tall = || group(Style::new().padding_each(10.0, 0.0, 0.0, 0.0)).child(label("cd"));
    let root = window(Style::new()).child(
        line(Style::row().width(40.0))
            .child(label("ab"))
            .child(tall())
            .child(label("ef")),
    );
    // "ef" doesn't fit on the first line, and is alone on the second.
    assert_eq!(
        rects(root, &[&[0], &[0, 0], &[0, 1], &[0, 2]]),
        [
            rect(0.0, 0.0, 40.0, 42.0),
            rect(0.0, 10.0, 16.0, 16.0),
            rect(16.0, 0.0, 16.0, 26.0),
            rect(0.0, 26.0, 16.0, 16.0)
        ]
    );
}

#[test]
fn a_line_with_a_strut_is_at_least_as_tall_as_a_line_of_its_text() {
    // A box with no text sits on the baseline, and the strut's 4 below that goes under it.
    let root = window(Style::new()).child(
        line(Style::row())
            .prop("line_strut", true)
            .child(group(Style::new().width(10.0).height(30.0))),
    );
    assert_eq!(
        rects(root, &[&[0], &[0, 0]]),
        [rect(0.0, 0.0, 400.0, 34.0), rect(0.0, 0.0, 10.0, 30.0)]
    );
}

#[test]
fn a_short_box_on_a_line_with_a_strut_sits_on_the_text_baseline() {
    let root = window(Style::new()).child(
        line(Style::row())
            .prop("line_strut", true)
            .child(group(Style::new().width(10.0).height(4.0))),
    );
    // The strut is 12 above its baseline and 4 below, and the box's bottom is on that baseline.
    assert_eq!(
        rects(root, &[&[0], &[0, 0]]),
        [rect(0.0, 0.0, 400.0, 16.0), rect(0.0, 8.0, 10.0, 4.0)]
    );
}

/// The order things are drawn in, by their names.
fn drawing_order(root: &Node) -> Vec<String> {
    struct Names(Vec<String>);
    impl rsui_layout::LayoutVisitor for Names {
        fn enter(&mut self, element: &Element, _rect: Rect) {
            if !element.name.is_empty() {
                self.0.push(element.name.to_string());
            }
        }
        fn exit(&mut self, _element: &Element) {}
    }
    let layout = laid_out(root);
    let mut names = Names(Vec::new());
    layout.visit(root, &mut names);
    names.0
}

/// A menu placed by itself below a button, in something only as tall as the button, and text after that.
fn menu_over_text() -> Node {
    let menu = with(
        group(Style::new().width(100.0))
            .child(label("first item"))
            .child(label("second item")),
        &[
            ("position", words("absolute")),
            ("top", words("100%")),
            ("left", int(0)),
        ],
    );
    window(Style::new())
        .child(
            group(Style::new().height(20.0))
                .child(label("button"))
                .child(menu),
        )
        .child(label("text after"))
        .into_node()
}

#[test]
fn something_placed_by_itself_is_drawn_over_what_comes_after_it() {
    assert_eq!(
        drawing_order(&menu_over_text()),
        ["button", "text after", "first item", "second item"]
    );
}

#[test]
fn something_placed_by_itself_is_as_tall_as_what_is_in_it_whatever_it_is_in() {
    let layout = laid_out(&menu_over_text());
    // Two lines of 16, though what it's in is only 20 tall.
    assert_eq!(layout.rect(&[0, 1]).unwrap().height, 32.0);
}

#[test]
fn something_placed_by_itself_can_be_clicked_where_it_is_over_something_else() {
    let root = menu_over_text();
    let layout = laid_out(&root);
    // The second item is over the text after, past the bottom of what the menu is in.
    let menu = layout.rect(&[0, 1]).unwrap();
    let path = layout
        .hit_test(menu.x + 5.0, menu.y + 20.0)
        .expect("something's there");
    assert_eq!(path, [0, 1, 1]);
}

#[test]
fn what_is_placed_is_drawn_in_the_order_it_is_in_after_what_is_not() {
    let knob = with(
        label("knob"),
        &[("position", words("absolute")), ("top", int(0))],
    );
    let footer = with(
        group(Style::new()).child(label("footer")),
        &[("position", words("relative"))],
    );
    let inside_footer = with(
        label("badge"),
        &[("position", words("absolute")), ("top", int(0))],
    );
    let root = window(Style::new())
        .child(group(Style::new()).child(label("row")).child(knob))
        .child(footer.child(inside_footer))
        .child(label("after"))
        .into_node();
    // What isn't placed first, then each that is in the order they're in, with what's placed in one right after it.
    assert_eq!(
        drawing_order(&root),
        ["row", "after", "knob", "footer", "badge"]
    );
}

#[test]
fn something_absolute_goes_where_it_says_from_inside_the_borders_of_its_parent() {
    let parent = with(
        group(Style::new().width(100.0).height(60.0).padding(4.0)),
        &[("border_width", int(4)), ("border_color", words("#000000"))],
    )
    .child(with(
        group(Style::new().width(30.0).height(20.0)),
        &[
            ("position", words("absolute")),
            ("top", int(-9)),
            ("right", int(5)),
        ],
    ));
    let root = window(Style::new()).child(parent);
    assert_eq!(rects(root, &[&[0, 0]]), [rect(61.0, -5.0, 30.0, 20.0)]);
}

#[test]
fn what_starts_from_nothing_starts_from_its_padding_and_shares_the_rest() {
    let item = |padding: f32| {
        with(
            group(Style::new().grow(1.0).padding(padding)),
            &[("basis", int(0))],
        )
    };
    let root = window(Style::new()).child(
        group(Style::row().width(300.0))
            .child(item(10.0))
            .child(item(20.0)),
    );
    let [first, second] = rects(root, &[&[0, 0], &[0, 1]])[..] else {
        unreachable!("two were asked for");
    };
    // 300 less 20 and 40 of padding leaves 240, which is 120 each.
    assert_eq!((first.width, second.width), (140.0, 160.0));
}

#[test]
fn a_flex_box_on_a_line_sits_on_it_where_the_first_thing_in_it_does() {
    // The first thing in the row has no text, so the row sits by that one's bottom, 30 down, rather than by the text after it.
    let root = window(Style::new()).child(
        line(Style::row()).child(label("ab")).child(
            group(Style::row())
                .child(group(Style::new().width(10.0).height(30.0)))
                .child(label("cd")),
        ),
    );
    assert_eq!(rects(root, &[&[0, 0]]), [rect(0.0, 18.0, 16.0, 16.0)]);
}

/// A part of a table, as `display: table-row` and the rest say.
fn part(name: &str, style: Style) -> Element {
    group(style).prop("table_part", name)
}

#[test]
fn a_tables_columns_are_as_wide_as_whats_widest_in_them_in_any_row() {
    let cell = |width: f32, height: f32| part("cell", Style::new().width(width).height(height));
    let root = window(Style::new().align(Align::Start)).child(
        part("table", Style::new())
            .child(
                part("group", Style::new())
                    .child(
                        part("row", Style::new())
                            .child(cell(50.0, 10.0))
                            .child(cell(10.0, 20.0)),
                    )
                    .child(
                        part("row", Style::new())
                            .child(cell(20.0, 30.0))
                            .child(cell(80.0, 5.0)),
                    ),
            )
            // Across both columns, and something that isn't a row, which is in a row of its own.
            .child(part("row", Style::new()).child(cell(5.0, 5.0).prop("col_span", 2)))
            .child(group(Style::new().height(7.0))),
    );
    assert_eq!(
        rects(
            root,
            &[
                &[0],
                &[0, 0],
                &[0, 0, 1],
                &[0, 0, 0, 1],
                &[0, 0, 1, 0],
                &[0, 1, 0],
                &[0, 2]
            ]
        ),
        [
            rect(0.0, 0.0, 130.0, 62.0),
            rect(0.0, 0.0, 130.0, 50.0),
            rect(0.0, 20.0, 130.0, 30.0),
            rect(50.0, 0.0, 80.0, 20.0),
            rect(0.0, 20.0, 50.0, 30.0),
            rect(0.0, 50.0, 130.0, 5.0),
            rect(0.0, 55.0, 50.0, 7.0),
        ]
    );
}

#[test]
fn a_tables_spare_room_goes_to_its_columns_by_how_much_they_want() {
    let cell = |width: f32| part("cell", Style::new().width(width).height(10.0));
    let root = window(Style::new().align(Align::Start)).child(
        part("table", Style::new().width(280.0).gap(10.0)).child(
            part("row", Style::new())
                .child(cell(30.0))
                .child(cell(60.0)),
        ),
    );
    // 270 between them, once there's the gap, by 30 to 60.
    assert_eq!(
        rects(root, &[&[0, 0, 0], &[0, 0, 1]]),
        [rect(0.0, 0.0, 90.0, 10.0), rect(100.0, 0.0, 180.0, 10.0)]
    );
}

fn absolute(element: Element) -> Element {
    with(element, &[("position", words("absolute"))])
}

fn contents(element: Element) -> Element {
    with(element, &[("display", words("contents"))])
}

fn shrinks(style: Style) -> Element {
    with(group(style.shrink(1.0)), &[("min_auto", true.into())])
}

#[test]
fn something_placed_by_itself_that_says_nowhere_is_where_a_flex_box_puts_what_is_in_it() {
    // As CSS's static position has it: an icon over another in a button that puts what's in it in the middle is in the middle too.
    let root = window(
        Style::row()
            .justify(Justify::Center)
            .align(Align::Center)
            .padding(10.0),
    )
    .child(group(Style::new().width(20.0).height(20.0)))
    .child(absolute(group(Style::new().width(20.0).height(20.0))))
    .child(absolute(group(
        Style::new().width(20.0).height(20.0).align_self(Align::End),
    )));
    assert_eq!(
        rects(root, &[&[0], &[1], &[2]]),
        [
            rect(190.0, 140.0, 20.0, 20.0),
            rect(190.0, 140.0, 20.0, 20.0),
            rect(190.0, 270.0, 20.0, 20.0),
        ]
    );
}

#[test]
fn something_placed_by_itself_that_says_nowhere_in_a_block_is_under_what_is_before_it() {
    let block = |direction: Style| {
        with(
            window(direction.padding(5.0)),
            &[("collapse_margins", true.into())],
        )
        .child(group(Style::new().height(50.0)))
        .child(absolute(group(Style::new().width(10.0).height(10.0))))
    };
    assert_eq!(
        rects(block(Style::new()), &[&[1]]),
        [rect(5.0, 55.0, 10.0, 10.0)]
    );
    // A side it says is still where it says, and the other is where it would have been.
    let root = with(
        window(Style::new().padding(5.0)),
        &[("collapse_margins", true.into())],
    )
    .child(group(Style::new().height(50.0)))
    .child(with(
        absolute(group(Style::new().width(10.0).height(10.0))),
        &[("right", int(0))],
    ));
    assert_eq!(rects(root, &[&[1]]), [rect(390.0, 55.0, 10.0, 10.0)]);
}

#[test]
fn a_flex_box_asked_how_narrow_it_can_be_asks_what_is_in_it_the_same() {
    // 60 characters of 8 pixels each, which can be as narrow as one of them.
    let long = "0123456789".repeat(6);
    let root = window(Style::row())
        .child(shrinks(Style::new().width(80.0)))
        .child(shrinks(Style::row().width_percent(100.0)).child(label(&long)));
    // Both shrink by how big they are, as the second one can be narrower than its text is long, as CSS has it.
    let got = rects(root, &[&[0], &[1]]);
    assert!(
        (got[0].width - (80.0 - 80.0 * 80.0 / 480.0)).abs() < 0.01,
        "{got:?}"
    );
    assert!(
        (got[1].width - (400.0 - 80.0 * 400.0 / 480.0)).abs() < 0.01,
        "{got:?}"
    );
}

#[test]
fn something_that_scrolls_across_is_as_narrow_as_what_is_in_it_can_be() {
    let long = "0123456789".repeat(6);
    let root = window(Style::row())
        .child(shrinks(Style::new().width(80.0)))
        .child(
            shrinks(Style::new().width_percent(100.0))
                .child(shrinks(Style::new().scroll_x(0.0)).child(label(&long))),
        );
    let got = rects(root, &[&[0], &[1]]);
    assert!(
        (got[0].width - (80.0 - 80.0 * 80.0 / 480.0)).abs() < 0.01,
        "{got:?}"
    );
    assert!(
        (got[1].width - (400.0 - 80.0 * 400.0 / 480.0)).abs() < 0.01,
        "{got:?}"
    );
}

#[test]
fn what_has_no_box_of_its_own_lays_out_what_is_in_it_as_part_of_what_it_is_in() {
    let root = window(Style::row().justify(Justify::Center).gap(10.0))
        .child(group(Style::new().width(50.0)))
        .child(
            contents(group(Style::new()))
                .child(group(Style::new().width(50.0)))
                .child(contents(group(Style::new())).child(group(Style::new().width(50.0)))),
        )
        .into_node();
    let layout = laid_out(&root);
    // Three items 10 apart, in the middle: 170 wide, from 115.
    assert_eq!(layout.rect(&[0]), Some(rect(115.0, 0.0, 50.0, 300.0)));
    assert_eq!(layout.rect(&[1, 0]), Some(rect(175.0, 0.0, 50.0, 300.0)));
    assert_eq!(layout.rect(&[1, 1, 0]), Some(rect(235.0, 0.0, 50.0, 300.0)));
    // It isn't anything to find at a point itself, though what's in it is.
    assert_eq!(layout.hit_test(180.0, 10.0), Some(vec![1, 0]));
    assert_eq!(layout.hit_test(10.0, 10.0), Some(vec![]));
}

#[test]
fn a_change_inside_what_has_no_box_lays_out_what_it_is_in_again() {
    let tree = |width: f32| {
        window(Style::row().justify(Justify::End))
            .child(
                contents(group(Style::new()))
                    .child(group(Style::new().width(width)))
                    .child(group(Style::new().width(50.0))),
            )
            .into_node()
    };
    let (old, new) = (tree(50.0), tree(150.0));
    let mut layout = laid_out(&old);
    assert_eq!(layout.rect(&[0, 0]), Some(rect(300.0, 0.0, 50.0, 300.0)));
    layout.apply(&diff(&old, &new));
    layout.compute(&new, WINDOW, &mut FixedMetrics::default());
    assert_eq!(layout.rect(&[0, 0]), Some(rect(200.0, 0.0, 150.0, 300.0)));
    assert_eq!(layout.rect(&[0, 1]), Some(rect(350.0, 0.0, 50.0, 300.0)));
    // The same as starting again.
    let fresh = laid_out(&new);
    for path in [&[0u32, 0][..], &[0, 1]] {
        assert_eq!(layout.rect(path), fresh.rect(path));
    }
}

#[test]
fn what_has_no_box_in_a_grid_puts_what_is_in_it_in_the_grid() {
    let root = grid("1fr 1fr").child(cell(10.0)).child(
        contents(group(Style::new()))
            .child(cell(10.0))
            .child(cell(10.0)),
    );
    assert_eq!(
        rects(root, &[&[0], &[1, 0], &[1, 1]]),
        [
            rect(0.0, 0.0, 200.0, 10.0),
            rect(200.0, 0.0, 200.0, 10.0),
            rect(0.0, 10.0, 200.0, 10.0),
        ]
    );
}

#[test]
fn something_in_a_grid_says_where_it_starts_and_ends_one_side_at_a_time() {
    let root = grid("1fr 1fr 1fr 1fr")
        // `col-span-2 col-start-2`, as Tailwind writes them: the start goes over that part of `grid-column`.
        .child(with(
            cell(10.0),
            &[
                ("grid_column", words("span 2 / span 2")),
                ("grid_column_start", words("2")),
            ],
        ))
        .child(with(
            cell(10.0),
            &[("grid_column_end", words("-1")), ("grid_row_start", int(2))],
        ))
        .child(with(cell(10.0), &[("grid_column", words("span 2 / 5"))]))
        .child(with(
            group(Style::new()),
            &[("grid_row", words("span 2")), ("grid_column_start", int(1))],
        ));
    let got = rects(root, &[&[0], &[1], &[2], &[3]]);
    assert_eq!((got[0].x, got[0].y, got[0].width), (100.0, 0.0, 200.0));
    // Ending at the last line, with nothing said about where it starts, is the last track.
    assert_eq!((got[1].x, got[1].y, got[1].width), (300.0, 10.0, 100.0));
    assert_eq!((got[2].x, got[2].width), (200.0, 200.0));
    assert_eq!((got[3].x, got[3].height), (0.0, 20.0));
}

#[test]
fn a_min_width_that_is_a_share_of_what_it_is_in_is_at_least_that() {
    let root = window(Style::new()).child(
        group(Style::new().width(300.0))
            .child(with(group(Style::new().width(100.0).height(10.0)), &[("min_width", words("100%"))]))
            .child(with(group(Style::new().width(400.0).height(10.0)), &[("max_width", words("50%"))])),
    );
    assert_eq!(rects(root, &[&[0, 0], &[0, 1]]), [rect(0.0, 0.0, 300.0, 10.0), rect(0.0, 10.0, 150.0, 10.0)]);
}
mod scrolled_without_layout {
    //! Scrolling an area again only moves what's in it, without laying any of it out: it has to come to what laying it
    //! out from nothing does, with what's placed by itself in it and what's on top of everything in it.
    use super::*;
    use rsui_layout::Placement;

    const SIZE: Size = Size::new(200.0, 100.0);

    /// A page that scrolls: blocks one under another, a block that isn't positioned with one of the page's own placed by itself
    /// in it, which the area places, and something on top of everything below one of the blocks.
    fn page(scroll: f64, inner_width: f32) -> Node {
        let placed = with(
            absolute(group(Style::new().width(30.0).height(10.0))),
            &[("top", int(15)), ("left", int(20)), ("static_box", false.into())],
        );
        // What changes inside it doesn't change its size, so only it is laid out again, and what it holds is placed again.
        let holder = with(group(Style::new().height(40.0).width(100.0)), &[("static_box", true.into())])
            .child(placed)
            .child(group(Style::new().height(10.0).width(inner_width)));
        let popup = group(Style::new().width(50.0).height(20.0).overlay(Placement::Below));
        let area = with(
            group(Style::new().height(100.0).scroll_y(scroll as f32)),
            &[("static_box", false.into())],
        )
        .child(group(Style::new().height(60.0)))
        .child(holder)
        .child(group(Style::new().height(60.0)).child(popup))
        .child(group(Style::new().height(200.0)));
        Element::new(Role::Window).child(area).into_node()
    }

    fn fresh(root: &Node) -> Vec<(Vec<u32>, Rect)> {
        let mut layout = Layout::new();
        layout.compute(root, SIZE, &mut FixedMetrics::default());
        all_rects(&layout)
    }

    #[test]
    fn scrolling_again_moves_what_is_in_it_as_laying_it_out_from_nothing_does() {
        let mut metrics = FixedMetrics::default();
        let mut layout = Layout::new();
        let mut previous = page(0.0, 100.0);
        layout.compute(&previous, SIZE, &mut metrics);
        for scroll in [10.0, 35.5, 120.0, 0.0, 250.0, 5.0, -20.0] {
            let root = page(scroll, 100.0);
            let needs = layout.apply_needs_layout_since(&diff(&previous, &root), Some(&previous));
            layout.compute(&root, SIZE, &mut metrics);
            assert!(!needs, "{scroll}: only moved");
            assert_eq!(layout.last_solved(), 0, "{scroll}: nothing laid out again");
            assert!(same_rects(&all_rects(&layout), &fresh(&root)), "{scroll}: {:?}\n{:?}", all_rects(&layout), fresh(&root));
            previous = root;
        }
        // Near the far end, where how far it can go is the layout's to say, it's laid out as before, and comes to the same.
        let root = page(1e9, 100.0);
        layout.apply_needs_layout_since(&diff(&previous, &root), Some(&previous));
        layout.compute(&root, SIZE, &mut metrics);
        assert!(same_rects(&all_rects(&layout), &fresh(&root)));
    }

    #[test]
    fn a_change_inside_a_scrolled_area_keeps_what_it_places_by_itself_where_laying_out_puts_it() {
        // Changed while it's scrolled, what holds the one placed by itself is laid out again, and that's placed again from where the
        // area's content starts, 15 below its top, not from where it's shown: from there it stayed put in the window as the page
        // went by. Fails without `place_passed_absolutes` taking the scroll back out (at 15 rather than -15).
        for (scroll, moved_first) in [(30.0, true), (30.0, false)] {
            let mut metrics = FixedMetrics::default();
            let mut layout = Layout::new();
            let start = page(if moved_first { 0.0 } else { scroll }, 100.0);
            layout.compute(&start, SIZE, &mut metrics);
            let scrolled = page(scroll, 100.0);
            layout.apply_needs_layout_since(&diff(&start, &scrolled), Some(&start));
            layout.compute(&scrolled, SIZE, &mut metrics);
            let changed = page(scroll, 60.0);
            layout.apply_needs_layout_since(&diff(&scrolled, &changed), Some(&scrolled));
            layout.compute(&changed, SIZE, &mut metrics);
            assert!(same_rects(&all_rects(&layout), &fresh(&changed)), "{:?}\n{:?}", all_rects(&layout), fresh(&changed));
            assert_eq!(layout.rect(&[0, 1, 0]).map(|rect| rect.y), Some(15.0 - scroll as f32));
        }
    }
}

/// A page's scroller lays what's in it out in a group that's no box of its own (`scroll_content`): a height that's a share, in
/// that, is of how tall the scroller shows, and stays so as the scroller, or what's in it, changes.
#[test]
fn a_share_of_a_scrollers_height_follows_it() {
    let tree = |height: f32, words: &str| -> Node {
        root_of(
            group(Style::new().height(height).scroll_y(0.0)).child(
                group(Style::new())
                    .prop("scroll_content", true)
                    .child(group(Style::new().height_percent(50.0)))
                    .child(Element::new(Role::Text).name(words)),
            ),
        )
    };
    let mut metrics = FixedMetrics::default();
    let mut layout = Layout::new();
    let mut previous = tree(200.0, "x");
    layout.compute(&previous, WINDOW, &mut metrics);
    assert_eq!(layout.rect(&[0, 0, 0]).map(|rect| rect.height), Some(100.0));
    for (height, words) in [(250.0, "x"), (250.0, "a longer name that takes up some room on a line"), (120.0, "y"), (120.0, "y")] {
        let next = tree(height, words);
        layout.apply(&diff(&previous, &next));
        layout.compute(&next, WINDOW, &mut metrics);
        let mut fresh = Layout::new();
        fresh.compute(&next, WINDOW, &mut metrics);
        assert_eq!(all_rects(&layout), all_rects(&fresh), "{height} {words}");
        assert_eq!(layout.rect(&[0, 0, 0]).map(|rect| rect.height), Some(height / 2.0));
        previous = next;
    }
}

/// Right-to-left layout: rsui-tsx says `cross_reverse` on a block, a column or a grid where text goes from right to
/// left, and lays its row out from the left and turns it around.
mod right_to_left {
    use super::*;

    fn yes() -> rsui_core::PropValue {
        rsui_core::PropValue::Bool(true)
    }

    #[test]
    fn a_grid_turned_around_has_its_first_column_at_the_right_and_starts_cells_there() {
        let root = with(grid("100px 100px 200px"), &[("cross_reverse", yes())])
            .child(cell(10.0))
            .child(group(Style::new().width(40.0).height(10.0)))
            .child(cell(10.0));
        assert_eq!(
            rects(root, &[&[0], &[1], &[2]]),
            [rect(300.0, 0.0, 100.0, 10.0), rect(260.0, 0.0, 40.0, 10.0), rect(0.0, 0.0, 200.0, 10.0)]
        );
    }

    #[test]
    fn what_is_placed_by_itself_without_sides_starts_at_the_right_of_a_block_turned_around() {
        let column = with(group(Style::new().padding(10.0).height(50.0)), &[("cross_reverse", yes())]).child(with(
            group(Style::new().width(30.0).height(5.0)),
            &[("position", words("absolute"))],
        ));
        let root = window(Style::new()).child(column);
        assert_eq!(rects(root, &[&[0, 0]]), [rect(360.0, 10.0, 30.0, 5.0)]);
    }

    #[test]
    fn a_row_turned_around_across_is_still_from_the_top() {
        // Lines of text where text goes from right to left are said to be turned around across, which is only a column's.
        let row = with(group(Style::row().align(Align::Start).height(50.0)), &[("cross_reverse", yes())])
            .child(group(Style::new().width(30.0).height(5.0)))
            .child(group(Style::new().width(30.0).height(20.0)));
        let root = window(Style::new()).child(row);
        assert_eq!(rects(root, &[&[0, 0], &[0, 1]]), [rect(0.0, 0.0, 30.0, 5.0), rect(30.0, 0.0, 30.0, 20.0)]);
    }

    #[test]
    fn a_scrollbar_at_the_left_has_what_scrolls_start_after_it() {
        let area = Element::new(Role::Group)
            .props(Props::from(Style::new().scroll_y(0.0).height(100.0)))
            .prop("scrollbar_room", 17.0)
            .prop("scrollbar_left", true)
            .child(group(Style::new().height(200.0)));
        let root: Node = window(Style::new()).child(area).into_node();
        let mut layout = Layout::new();
        layout.compute(&root, Size::new(100.0, 100.0), &mut FixedMetrics::default());
        assert_eq!(layout.rect(&[0, 0]).unwrap(), rect(17.0, 0.0, 83.0, 200.0));
        let info = layout.scroll_info(&[0], &root.children[0]).expect("it scrolls");
        assert_eq!((info.x, info.view.width, info.content.width), (0.0, 83.0, 83.0));
    }

    /// Says each child of a line is at the level given for it, as rsui-text's bidirectional algorithm would.
    struct Levels(Vec<u8>);

    impl rsui_layout::Measure for Levels {
        fn measure(&mut self, element: &Element, max_width: f32) -> Size {
            FixedMetrics::default().measure(element, max_width)
        }

        fn line_levels(&mut self, _line: &Element, levels: &mut Vec<u8>) {
            levels.clone_from(&self.0);
        }
    }

    fn line_with_levels(levels: &[u8], wrap: bool) -> Vec<Rect> {
        let mut line = with(
            group(Style::row().justify(Justify::End)),
            &[("line_strut", yes()), ("text_direction", words("rtl")), ("flex_wrap", rsui_core::PropValue::Bool(wrap))],
        );
        for width in [10.0, 20.0, 30.0] {
            line = line.child(group(Style::new().width(width).height(10.0)));
        }
        let root = window(Style::new()).child(line).into_node();
        let mut layout = Layout::new();
        layout.compute(&root, WINDOW, &mut Levels(levels.to_vec()));
        (0..3).map(|at| layout.rect(&[0, at]).expect("laid out")).collect()
    }

    #[test]
    fn what_is_on_a_line_from_the_right_goes_in_the_order_its_levels_say() {
        for wrap in [false, true] {
            // All from the right: the first at the right end of the line.
            assert_eq!(
                line_with_levels(&[1, 1, 1], wrap),
                [rect(390.0, 0.0, 10.0, 10.0), rect(370.0, 0.0, 20.0, 10.0), rect(340.0, 0.0, 30.0, 10.0)],
                "wrap {wrap}"
            );
            // The second and third are a run from the left, in order, to the left of the first.
            assert_eq!(
                line_with_levels(&[1, 2, 2], wrap),
                [rect(390.0, 0.0, 10.0, 10.0), rect(340.0, 0.0, 20.0, 10.0), rect(360.0, 0.0, 30.0, 10.0)],
                "wrap {wrap}"
            );
            // All from the left: as they come.
            assert_eq!(
                line_with_levels(&[2, 2, 2], wrap),
                [rect(340.0, 0.0, 10.0, 10.0), rect(350.0, 0.0, 20.0, 10.0), rect(370.0, 0.0, 30.0, 10.0)],
                "wrap {wrap}"
            );
        }
    }

    #[test]
    fn something_scrolling_across_from_the_right_starts_at_its_right() {
        // As a flex box's content, and as a block's, which fills it across, with what's wider reaching past its left.
        for fills in [false, true] {
            let area = |scrolled: f32| {
                let area = with(
                    Element::new(Role::Group).props(Props::from(Style::new().scroll_x(scrolled).width(100.0).height(50.0))),
                    &[("text_direction", words("rtl")), ("fill_across", rsui_core::PropValue::Bool(fills))],
                );
                let inner = if fills {
                    with(group(Style::new()), &[("cross_reverse", yes())]).child(group(Style::new().width(300.0).height(10.0)))
                } else {
                    group(Style::new().width(300.0).height(10.0))
                };
                window(Style::new().align(Align::Start)).child(area.child(inner)).into_node()
            };
            for (scrolled, x) in [(0.0, -200.0), (50.0, -150.0), (200.0, 0.0), (500.0, 0.0)] {
                let root = area(scrolled);
                let layout = laid_out(&root);
                let path: &[u32] = if fills { &[0, 0, 0] } else { &[0, 0] };
                assert_eq!(layout.rect(path).unwrap().x, x, "scrolled {scrolled} from the right, filling {fills}");
                // How far it's moved is from its left, as everything that scrolls it goes by.
                let info = layout.scroll_info(&[0], &root.children[0]).expect("it scrolls");
                assert_eq!(info.x, -x, "filling {fills}");
                assert_eq!(info.furthest_x(), 200.0, "filling {fills}");
            }
        }
    }

}
