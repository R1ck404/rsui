//! Scrolling both ways, lists that glide, and text areas that don't have the keyboard.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use rsui::prelude::*;
use rsui::render::Renderer;
use rsui::text::TextEngine;
use rsui::{App, KeyCode, KeyPress};
use support::plain_font;

const SIZE: (u32, u32) = (200, 200);
const LATER: Duration = Duration::from_secs(5);

/// Something 1000 by 1000 in an area of 200 by 200.
fn big_picture(cx: &mut Cx, _: &()) -> Element {
    let big = Element::new(Role::Group)
        .key("big")
        .props(style!(Style::new().width(1000.0).height(1000.0)));
    let content = Element::new(Role::Group)
        .props(style!(Style::new().flex(1.0)))
        .child(big);
    Element::new(Role::Window).child(cx.scroll_area("area", ScrollAxis::Both, content))
}

fn app_of(render: fn(&mut Cx, &()) -> Element) -> App {
    let mut app = App::new(render, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    app
}

fn moved(app: &mut App) -> (f32, f32) {
    app.update();
    app.scene();
    let root = app.runtime().root().clone();
    let info = app
        .layout()
        .scroll_info(&[0], &root.children[0])
        .expect("it scrolls");
    (info.x, info.y)
}

#[test]
fn an_area_can_go_both_ways_at_once() {
    let mut app = app_of(big_picture);
    assert_eq!(moved(&mut app), (0.0, 0.0));
    app.scroll(50.0, 50.0, 3).unwrap();
    assert_eq!(moved(&mut app), (0.0, 100.0), "the wheel goes down");
    app.scroll_across(50.0, 50.0, 3).unwrap();
    assert_eq!(moved(&mut app), (100.0, 100.0), "and across, independently");
    app.scroll_across(50.0, 50.0, -3).unwrap();
    assert_eq!(moved(&mut app), (0.0, 100.0));
}

#[test]
fn precision_scrolling_preserves_small_deltas_and_independent_axes_without_a_glide() {
    let mut app = app_of(big_picture);
    app.set_animations(true);
    for _ in 0..4 {
        app.scroll_pixels(50.0, 50.0, 0.25, false, true).unwrap();
    }
    app.scroll_pixels(50.0, 50.0, 0.5, true, true).unwrap();
    assert_eq!(moved(&mut app), (0.5, 1.0));
    assert!(!app.animating());
    app.scroll_pixels(50.0, 50.0, -0.25, false, true).unwrap();
    assert_eq!(moved(&mut app), (0.5, 0.75));
}

#[test]
fn precision_input_interrupts_a_discrete_glide_at_its_current_position() {
    let mut app = app_of(big_picture);
    app.set_animations(true);
    let start = Instant::now();
    app.scroll_pixels(50.0, 50.0, 100.0, false, false).unwrap();
    app.tick(start + Duration::from_millis(30));
    let current = moved(&mut app).1;
    assert!(current > 0.0 && current < 100.0);
    app.scroll_pixels(50.0, 50.0, -0.25, false, true).unwrap();
    assert_eq!(moved(&mut app).1, current - 0.25);
    app.tick(start + LATER);
    assert_eq!(moved(&mut app).1, current - 0.25, "no old glide tail after precision input");
    assert!(!app.animating());
}

#[test]
fn pixel_wheel_delivery_is_exact_and_cancelable_before_default_scrolling() {
    fn blocked(cx: &mut Cx, _: &()) -> Element {
        let window = big_picture(cx, &()).key("wheel-window");
        cx.on(window, ActionKind::Custom(rsui::core::Symbol::new("wheel")), |cx, _| {
            let event = cx.event().expect("the wheel event is active");
            assert_eq!((event.delta_x, event.delta_y), (0.0, 0.25));
            cx.prevent_default();
        })
    }
    let mut app = app_of(blocked);
    app.set_animations(true);
    app.scroll_pixels(50.0, 50.0, 0.25, false, true).unwrap();
    assert_eq!(moved(&mut app), (0.0, 0.0));
    assert!(!app.animating());
}

#[test]
fn each_way_stops_at_its_own_end() {
    let mut app = app_of(big_picture);
    app.scroll(50.0, 50.0, 100_000).unwrap();
    app.scroll_across(50.0, 50.0, 100_000).unwrap();
    assert_eq!(moved(&mut app), (800.0, 800.0));
}

#[test]
fn arrow_keys_go_each_way() {
    let mut app = app_of(big_picture);
    app.pointer_move(50.0, 50.0);
    app.key(KeyPress::new(KeyCode::Down)).unwrap();
    assert_eq!(moved(&mut app), (0.0, 40.0));
    app.key(KeyPress::new(KeyCode::Right)).unwrap();
    assert_eq!(moved(&mut app), (40.0, 40.0));
    app.key(KeyPress::new(KeyCode::Left)).unwrap();
    app.key(KeyPress::new(KeyCode::Up)).unwrap();
    assert_eq!(moved(&mut app), (0.0, 0.0));
}

#[test]
fn going_across_leaves_alone_what_only_goes_down() {
    fn tall(cx: &mut Cx, _: &()) -> Element {
        let tall = Element::new(Role::Group)
            .key("tall")
            .props(style!(Style::new().height(1000.0)));
        let content = Element::new(Role::Group)
            .props(style!(Style::new().flex(1.0)))
            .child(tall);
        Element::new(Role::Window).child(cx.scroll_area("area", ScrollAxis::Vertical, content))
    }
    let mut app = app_of(tall);
    app.scroll_across(50.0, 50.0, 3).unwrap();
    assert_eq!(moved(&mut app), (0.0, 0.0));
}

#[test]
fn something_that_goes_across_only_still_uses_the_ordinary_wheel() {
    fn wide(cx: &mut Cx, _: &()) -> Element {
        let wide = Element::new(Role::Group)
            .key("wide")
            .props(style!(Style::new().width(1000.0).height(10.0)));
        let content = Element::new(Role::Group)
            .props(style!(Style::new().flex(1.0)))
            .child(wide);
        Element::new(Role::Window).child(cx.scroll_area("area", ScrollAxis::Horizontal, content))
    }
    let mut app = app_of(wide);
    app.scroll(50.0, 5.0, 3).unwrap();
    assert_eq!(moved(&mut app), (100.0, 0.0));
    app.scroll_across(50.0, 5.0, 3).unwrap();
    assert_eq!(moved(&mut app), (200.0, 0.0));
}

#[test]
fn scrolling_each_way_can_be_seen_in_the_tree() {
    let mut app = app_of(big_picture);
    app.runtime_mut()
        .dispatch(
            &[0],
            ActionKind::Custom(rsui::core::Symbol::new("scroll_across")),
            Some(PropValue::Float(120.0)),
        )
        .unwrap();
    assert_eq!(moved(&mut app), (120.0, 0.0));
    let root = app.runtime().root().clone();
    assert_eq!(
        root.children[0].props.get("scroll_x"),
        Some(&PropValue::Float(120.0))
    );
}

#[test]
fn a_scrollbar_for_each_way_can_be_dragged() {
    let mut app = app_of(big_picture);
    // Down the right, and along the bottom.
    app.pointer_move(197.0, 10.0);
    app.pointer_down(197.0, 10.0, 1);
    app.pointer_move(197.0, 100.0);
    app.pointer_up(197.0, 100.0).unwrap();
    let (x, y) = moved(&mut app);
    assert_eq!(x, 0.0);
    assert!(y > 300.0, "{y}");
    app.pointer_move(10.0, 197.0);
    app.pointer_down(10.0, 197.0, 1);
    app.pointer_move(100.0, 197.0);
    app.pointer_up(100.0, 197.0).unwrap();
    let (across, down) = moved(&mut app);
    assert!(across > 300.0, "{across}");
    assert_eq!(down, y, "the other way stays where it was");
}

#[test]
fn gliding_each_way_is_kept_apart() {
    let mut app = app_of(big_picture);
    app.set_animations(true);
    let start = Instant::now();
    app.scroll(50.0, 50.0, 3).unwrap();
    app.scroll_across(50.0, 50.0, 3).unwrap();
    assert_eq!(moved(&mut app).1, 100.0, "the first was finished off");
    app.tick(start + LATER);
    assert_eq!(moved(&mut app), (100.0, 100.0));
    assert!(!app.animating());
}

fn long_list(cx: &mut Cx, _: &()) -> Element {
    let list = cx.virtual_list(
        "list",
        ListWindow {
            len: 1_000,
            visible: 6,
            overscan: 2,
            reveal: None,
        },
        |_, index| {
            Element::new(Role::Text)
                .key(index.to_string())
                .name("ABC")
                .into_node()
        },
    );
    let props = list
        .props
        .merged(&Style::new().row_height(20.0).flex(1.0).into());
    let list = list.props(props);
    Element::new(Role::Window).child(list)
}

#[test]
fn a_virtual_list_tracks_precision_input_in_fractional_rows() {
    let mut app = app_of(long_list);
    app.set_animations(true);
    app.scroll_pixels(50.0, 50.0, 0.25, false, true).unwrap();
    assert_eq!(list_y(&mut app), 0.25);
    assert!(!app.animating());
}

fn list_y(app: &mut App) -> f32 {
    moved(app).1
}

#[test]
fn a_list_glides_and_stops_on_a_whole_row() {
    let mut app = app_of(long_list);
    app.set_animations(true);
    let start = Instant::now();
    app.scroll(50.0, 50.0, 3).unwrap();
    assert!(app.animating());
    assert_eq!(list_y(&mut app), 0.0, "it hasn't gone yet");
    app.tick(start + Duration::from_millis(40));
    let partway = list_y(&mut app);
    assert!(partway > 0.0 && partway < 60.0, "{partway}");
    assert_eq!(
        partway.fract(),
        0.0,
        "on whole pixels, or text would shift by parts of them"
    );
    app.tick(Instant::now() + LATER);
    assert_eq!(list_y(&mut app), 60.0);
    assert!(!app.animating());
    let root = app.runtime().root().clone();
    assert_eq!(
        root.children[0].props.get("first_visible"),
        Some(&PropValue::Int(3)),
        "a whole row once it's stopped"
    );
}

#[test]
fn notches_add_up_on_a_list_too() {
    let mut app = app_of(long_list);
    app.set_animations(true);
    app.scroll(50.0, 50.0, 3).unwrap();
    app.scroll(50.0, 50.0, 3).unwrap();
    app.tick(Instant::now() + LATER);
    assert_eq!(list_y(&mut app), 120.0);
}

#[test]
fn a_list_stops_at_its_end_and_passes_it_on() {
    let mut app = app_of(long_list);
    app.set_animations(true);
    app.scroll(50.0, 50.0, 100_000).unwrap();
    app.tick(Instant::now() + LATER);
    let root = app.runtime().root().clone();
    let _ = root;
    app.update();
    app.scene();
    let root = app.runtime().root().clone();
    assert_eq!(
        root.children[0].props.get("first_visible"),
        Some(&PropValue::Int(990)),
        "10 whole rows at the end"
    );
    app.scroll(50.0, 50.0, 3).unwrap();
    assert!(!app.animating(), "there's nowhere further to go");
}

#[test]
fn going_across_leaves_a_list_alone() {
    let mut app = app_of(long_list);
    app.scroll_across(50.0, 50.0, 3).unwrap();
    assert_eq!(list_y(&mut app), 0.0);
}

#[test]
fn a_list_stops_on_a_whole_row_whatever_size_rows_are() {
    fn odd_rows(cx: &mut Cx, _: &()) -> Element {
        let list = cx.virtual_list(
            "list",
            ListWindow {
                len: 1_000,
                visible: 6,
                overscan: 2,
                reveal: None,
            },
            |_, index| {
                Element::new(Role::Text)
                    .key(index.to_string())
                    .name("ABC")
                    .into_node()
            },
        );
        let props = list
            .props
            .merged(&Style::new().row_height(20.5).flex(1.0).into());
        let list = list.props(props);
        Element::new(Role::Window).child(list)
    }
    let mut app = app_of(odd_rows);
    app.set_animations(true);
    app.scroll(50.0, 50.0, 3).unwrap();
    app.tick(Instant::now() + LATER);
    app.update();
    app.scene();
    let root = app.runtime().root().clone();
    assert_eq!(
        root.children[0].props.get("first_visible"),
        Some(&PropValue::Int(3))
    );
}

#[test]
fn a_list_partway_shows_rows_for_all_of_the_gap() {
    let mut app = app_of(long_list);
    app.runtime_mut()
        .dispatch(&[0], ActionKind::Scroll, Some(PropValue::Float(5.5)))
        .unwrap();
    assert_eq!(list_y(&mut app), 110.0);
    let root = app.runtime().root().clone();
    let list = &root.children[0];
    let built = list.children.len();
    // 10 whole rows fit, it's between two, and there are two extra each side.
    assert_eq!(built, 15, "one more than when it's on a whole row");
}

#[test]
fn going_to_the_end_of_a_list_stops_its_glide() {
    let mut app = app_of(long_list);
    app.set_animations(true);
    app.pointer_move(50.0, 50.0);
    app.scroll(50.0, 50.0, 3).unwrap();
    app.key(KeyPress::new(KeyCode::End)).unwrap();
    app.tick(Instant::now() + LATER);
    app.update();
    app.scene();
    let root = app.runtime().root().clone();
    assert_eq!(
        root.children[0].props.get("first_visible"),
        Some(&PropValue::Int(990))
    );
    assert!(!app.animating());
}

/// An input showing this, without the keyboard.
fn input_pixels(value: String) -> Vec<u8> {
    let mut app = App::new(
        |cx: &mut Cx, value: &String| {
            let start = value.clone();
            let shown = cx.state(move || Text::from(start.as_str()));
            let input = text_input(cx, "long", "Long", "", shown);
            Element::new(Role::Window)
                .props(style!(Style::new().align(Align::Start)))
                .child(input)
        },
        value,
        TextEngine::new(plain_font()),
        SIZE,
        1.0,
    );
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    pixels(&mut app, &mut renderer)
}

#[test]
fn a_long_input_without_the_keyboard_shows_how_it_starts_if_it_reads_from_the_right() {
    let filler = "\u{05D0}ABCD ".repeat(40);
    let same_start = |end: &str| format!("\u{05D0}AAAA {filler}\u{05D0}{end}");
    let plain = input_pixels(same_start("AAAA"));
    assert_ne!(
        plain,
        input_pixels(String::new()),
        "there's something showing"
    );
    assert_eq!(
        plain,
        input_pixels(same_start("BBBB")),
        "its end doesn't show"
    );
    assert_ne!(
        plain,
        input_pixels(format!("\u{05D0}BBBB {filler}\u{05D0}AAAA")),
        "its start does"
    );
}

#[test]
fn a_long_input_without_the_keyboard_shows_how_it_starts_if_it_reads_from_the_left() {
    let filler = "ABCD ".repeat(40);
    let plain = input_pixels(format!("AAAA {filler}AAAA"));
    assert_eq!(plain, input_pixels(format!("AAAA {filler}BBBB")));
    assert_ne!(plain, input_pixels(format!("BBBB {filler}AAAA")));
}

fn two_areas(cx: &mut Cx, _: &()) -> Element {
    // Every line but the first is the same, so that what's scrolled can be told from what isn't.
    let lines = format!("ABCD\n{}", "A\n".repeat(50));
    let value = cx.state(|| Text::from(lines.as_str()));
    let first = text_area(cx, "first", "First", "", value, 3);
    let second_value = cx.state(|| Text::from(lines.as_str()));
    let second = text_area(cx, "second", "Second", "", second_value, 3);
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start).gap(4.0)))
        .child(first)
        .child(second)
}

fn pixels(app: &mut App, renderer: &mut Renderer) -> Vec<u8> {
    app.update();
    app.render(renderer).unwrap();
    renderer.read_pixels().unwrap()
}

/// Where the first area's text is, among pixels.
fn in_first_area(pixels: &[u8]) -> Vec<u8> {
    let mut part = Vec::new();
    for row in pixels.chunks(SIZE.0 as usize * 4).take(40) {
        part.extend_from_slice(row);
    }
    part
}

#[test]
fn the_wheel_scrolls_a_text_area_without_the_keyboard() {
    let mut app = app_of(two_areas);
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    let before = pixels(&mut app, &mut renderer);
    app.scroll(20.0, 10.0, 3).unwrap();
    let after = pixels(&mut app, &mut renderer);
    assert_ne!(
        in_first_area(&before),
        in_first_area(&after),
        "the wheel moved it"
    );
    assert!(app.focused().is_none(), "and didn't give it the keyboard");
}

#[test]
fn a_text_area_stays_where_it_was_scrolled_when_it_gets_and_loses_the_keyboard() {
    let mut app = app_of(two_areas);
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    app.scroll(20.0, 10.0, 3).unwrap();
    let scrolled = pixels(&mut app, &mut renderer);
    // Going to the other one, with nothing having been focused here, leaves this one alone.
    app.focus_on(&[1]);
    let other_focused = pixels(&mut app, &mut renderer);
    assert_eq!(in_first_area(&scrolled), in_first_area(&other_focused));
}

#[test]
fn a_text_area_stays_scrolled_after_losing_the_keyboard() {
    let mut top = app_of(two_areas);
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    let at_the_top = pixels(&mut top, &mut renderer);

    let mut app = app_of(two_areas);
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    app.focus_on(&[0]);
    pixels(&mut app, &mut renderer);
    app.scroll(20.0, 10.0, 3).unwrap();
    pixels(&mut app, &mut renderer);
    app.focus_on(&[1]);
    let after = pixels(&mut app, &mut renderer);
    assert_ne!(in_first_area(&at_the_top), in_first_area(&after));
}

#[test]
fn a_text_area_with_nothing_to_scroll_passes_the_wheel_on() {
    fn in_an_area(cx: &mut Cx, _: &()) -> Element {
        let value = cx.state(|| Text::from("A"));
        let area = text_area(cx, "short", "Short", "", value, 3);
        let tall = Element::new(Role::Group)
            .key("tall")
            .props(style!(Style::new().height(1000.0)));
        let content = Element::new(Role::Group)
            .props(style!(Style::new().flex(1.0).align(Align::Start)))
            .child(area)
            .child(tall);
        Element::new(Role::Window).child(cx.scroll_area("area", ScrollAxis::Vertical, content))
    }
    let mut app = app_of(in_an_area);
    app.scroll(20.0, 10.0, 3).unwrap();
    assert_eq!(moved(&mut app).1, 100.0);
}
