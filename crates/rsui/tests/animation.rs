//! Things that move, drawn offscreen, on a clock these tests control.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use rsui::prelude::*;
use rsui::render::Renderer;
use rsui::text::TextEngine;
use rsui::{App, Frame, KeyCode, KeyPress};
use support::plain_font;

const SIZE: (u32, u32) = (200, 200);
const MILLIS: fn(u64) -> Duration = Duration::from_millis;
const LATER: Duration = Duration::from_secs(5);

/// A bar whose width moves between 0 and 100 when its button is clicked.
fn growing(cx: &mut Cx, _: &()) -> Element {
    let open = cx.state(|| false);
    let width = cx.animate(if cx.get(open) { 100.0 } else { 0.0 }, MILLIS(100));
    let bar = Element::new(Role::Group)
        .key("bar")
        .props(rsui::core::Props::from(
            Style::new().width(width as f32).height(20.0),
        ));
    let root = Element::new(Role::Window)
        .key("root")
        .props(style!(Style::new().align(Align::Start)))
        .child(bar);
    cx.on(root, ActionKind::Toggle, move |cx, _| {
        cx.update(open, |open| *open = !*open);
    })
}

fn app_of(render: fn(&mut Cx, &()) -> Element) -> App {
    let mut app = App::new(render, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.set_animations(true);
    app.update();
    app.scene();
    app
}

fn width(app: &mut App) -> f32 {
    app.update();
    app.scene();
    app.layout().rect(&[0]).expect("it's laid out").width
}

fn toggle(app: &mut App) {
    app.runtime_mut()
        .dispatch(&[], ActionKind::Toggle, None)
        .expect("it goes through");
    app.update();
    app.scene();
}

#[test]
fn it_starts_where_it_is_and_moves_when_told() {
    let mut app = app_of(growing);
    assert_eq!(width(&mut app), 0.0);
    assert!(!app.animating(), "nothing has moved yet");
    let start = Instant::now();
    toggle(&mut app);
    assert!(app.animating());
    let mut last = width(&mut app);
    assert!(last < 100.0, "it doesn't jump straight there: {last}");
    for step in 1..=5u64 {
        app.tick(start + MILLIS(step * 15));
        let now = width(&mut app);
        assert!(now >= last, "it only goes one way: {last} to {now}");
        assert!(now < 100.0, "not there yet at {step}");
        last = now;
    }
    app.tick(start + LATER);
    assert_eq!(width(&mut app), 100.0);
    assert!(!app.animating(), "and then it's done");
}

#[test]
fn it_slows_down_as_it_gets_there() {
    let mut app = app_of(growing);
    let start = Instant::now();
    toggle(&mut app);
    let mut at = |millis: u64| {
        app.tick(start + MILLIS(millis));
        width(&mut app)
    };
    let (quarter, half, three_quarters) = (at(25), at(50), at(75));
    assert!(quarter > 25.0, "{quarter}");
    assert!(
        half - quarter > three_quarters - half,
        "{quarter} {half} {three_quarters}"
    );
}

#[test]
fn it_turns_around_from_where_it_had_got_to() {
    let mut app = app_of(growing);
    let start = Instant::now();
    toggle(&mut app);
    app.tick(start + MILLIS(30));
    let partway = width(&mut app);
    assert!(partway > 0.0 && partway < 100.0);
    toggle(&mut app);
    let turned = width(&mut app);
    assert!(
        (turned - partway).abs() < 15.0,
        "no jump: {partway} to {turned}"
    );
    app.tick(Instant::now() + LATER);
    assert_eq!(width(&mut app), 0.0);
}

#[test]
fn a_motion_after_a_rest_takes_all_its_time() {
    let mut app = app_of(growing);
    toggle(&mut app);
    app.tick(Instant::now() + MILLIS(100));
    assert_eq!(width(&mut app), 100.0);
    // Real time goes by with nothing moving, longer than the next motion takes.
    std::thread::sleep(MILLIS(250));
    let turned = Instant::now();
    toggle(&mut app);
    // Going by the time the last motion ended, this would look as if it had finished already.
    app.tick(turned + MILLIS(1));
    let now = width(&mut app);
    assert!(now > 90.0, "it's only just started back: {now}");
}

#[test]
fn a_motion_keeps_asking_for_frames_even_if_nothing_looks_different_yet() {
    let mut app = app_of(growing);
    toggle(&mut app);
    assert_eq!(app.update(), Frame::Needed);
    app.tick(Instant::now() + LATER);
    app.update();
    app.scene();
    assert_eq!(app.update(), Frame::Unchanged);
}

/// Starts at 1, and goes to what it's given when it's told to, taking as long as it's given.
fn going_to(cx: &mut Cx, goal: &(f64, Duration)) -> Element {
    let (goal, duration) = *goal;
    let target = cx.state(|| 1.0);
    let shown = cx.animate(cx.get(target), duration);
    let root = Element::new(Role::Window).key("root").prop("shown", shown);
    cx.on(root, ActionKind::Toggle, move |cx, _| cx.set(target, goal))
}

fn after_going_to(goal: f64, duration: Duration) -> App {
    let mut app = App::new(
        going_to,
        (goal, duration),
        TextEngine::new(plain_font()),
        SIZE,
        1.0,
    );
    app.set_animations(true);
    app.update();
    app.runtime_mut()
        .dispatch(&[], ActionKind::Toggle, None)
        .unwrap();
    app.update();
    app
}

#[test]
fn numbers_that_cannot_be_reached_do_not_keep_it_moving_forever() {
    for goal in [f64::NAN, f64::INFINITY] {
        assert!(!after_going_to(goal, MILLIS(100)).animating(), "{goal}");
    }
    assert!(after_going_to(2.0, MILLIS(100)).animating());
}

#[test]
fn taking_no_time_goes_straight_there() {
    let app = after_going_to(2.0, Duration::ZERO);
    assert!(!app.animating());
    assert_eq!(
        app.runtime().root().props.get("shown"),
        Some(&PropValue::Float(2.0))
    );
}

#[test]
fn being_still_costs_nothing() {
    let mut app = app_of(growing);
    let start = Instant::now();
    toggle(&mut app);
    app.tick(start + LATER);
    app.update();
    app.scene();
    assert!(!app.animating());
    let renders = app.runtime().renders();
    // However much time goes by, and however it's asked, nothing renders again.
    for step in 0..5 {
        app.tick(start + LATER + LATER * step);
        assert_eq!(app.update(), Frame::Unchanged);
    }
    assert_eq!(app.runtime().renders(), renders);
}

#[test]
fn only_what_is_moving_renders_each_frame() {
    fn still_child(_: &mut Cx, _: &()) -> Element {
        Element::new(Role::Text).name("Still")
    }
    fn with_child(cx: &mut Cx, _: &()) -> Element {
        let child = cx.child(still_child, ());
        let inner = cx.child(growing, ());
        Element::new(Role::Window).child(child).child(inner)
    }
    let mut app = app_of(with_child);
    app.runtime_mut()
        .dispatch(&[1], ActionKind::Toggle, None)
        .unwrap();
    app.update();
    app.scene();
    let start = Instant::now();
    let before = app.runtime().renders();
    for step in 1..=4u64 {
        app.tick(start + MILLIS(step * 10));
        app.update();
    }
    assert_eq!(
        app.runtime().renders() - before,
        4,
        "one render for each frame, of the one thing moving"
    );
}

#[test]
fn with_animations_off_it_goes_straight_there() {
    let mut app = App::new(growing, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    toggle(&mut app);
    assert_eq!(width(&mut app), 100.0);
    assert!(!app.animating());
}

#[test]
fn turning_them_off_finishes_what_is_moving() {
    let mut app = app_of(growing);
    toggle(&mut app);
    assert!(app.animating());
    app.set_animations(false);
    assert_eq!(width(&mut app), 100.0);
    assert!(!app.animating());
}

#[test]
fn a_new_motion_after_a_long_rest_starts_from_the_beginning() {
    let mut app = app_of(growing);
    let start = Instant::now();
    toggle(&mut app);
    app.tick(start + LATER);
    assert_eq!(width(&mut app), 100.0);
    // The clock last said a time long ago, which mustn't make this look finished already.
    toggle(&mut app);
    let now = width(&mut app);
    assert!(now > 50.0, "it hasn't gone anywhere yet: {now}");
}

struct Screen {
    app: App,
    renderer: Renderer,
}

impl Screen {
    fn new(render: fn(&mut Cx, &()) -> Element) -> Self {
        let mut app = App::new(render, (), TextEngine::new(plain_font()), SIZE, 1.0);
        app.set_animations(true);
        let mut screen = Self {
            app,
            renderer: Renderer::offscreen(SIZE).expect("the software GPU is always there"),
        };
        screen.pixels();
        screen
    }

    fn pixels(&mut self) -> Vec<u8> {
        self.app.update();
        self.app.render(&mut self.renderer).expect("drawing works");
        self.renderer.read_pixels().expect("reading works")
    }

    fn pixel(&mut self, x: usize, y: usize) -> [u8; 4] {
        let pixels = self.pixels();
        let at = (y * SIZE.0 as usize + x) * 4;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    }
}

const WHITE: Color = Color::WHITE;
const RED: Color = Color::hex(0xFF0000);

fn hover_block(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start)))
        .child(Element::new(Role::Group).key("block").props(style!(
            Style::new().width(50.0).height(50.0),
            Paint::new().background(WHITE).hover(RED),
        )))
}

#[test]
fn a_hover_color_fades_in_and_out() {
    let mut screen = Screen::new(hover_block);
    assert_eq!(screen.pixel(10, 10), [255, 255, 255, 255]);
    screen.app.pointer_move(10.0, 10.0);
    let start = Instant::now();
    assert!(screen.app.animating());
    assert!(screen.pixel(10, 10)[1] > 200, "only just started");
    screen.app.tick(start + MILLIS(40));
    let middle = screen.pixel(10, 10)[1];
    assert!(middle > 0 && middle < 200, "partway: {middle}");
    screen.app.tick(start + LATER);
    assert_eq!(screen.pixel(10, 10), [255, 0, 0, 255], "there");
    assert!(!screen.app.animating());

    screen.app.pointer_move(100.0, 100.0);
    let left = Instant::now();
    assert!(screen.app.animating(), "and it fades out");
    assert!(screen.pixel(10, 10)[1] < 60, "only just started going");
    screen.app.tick(left + MILLIS(40));
    let middle = screen.pixel(10, 10)[1];
    assert!(middle > 0 && middle < 255, "partway: {middle}");
    screen.app.tick(left + LATER);
    assert_eq!(screen.pixel(10, 10), [255, 255, 255, 255]);
    assert!(!screen.app.animating());
}

#[test]
fn with_animations_off_a_hover_color_is_there_at_once() {
    let mut app = App::new(hover_block, (), TextEngine::new(plain_font()), SIZE, 1.0);
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    app.update();
    app.render(&mut renderer).unwrap();
    app.pointer_move(10.0, 10.0);
    assert!(!app.animating());
    app.update();
    app.render(&mut renderer).unwrap();
    let pixels = renderer.read_pixels().unwrap();
    let at = (10 * SIZE.0 as usize + 10) * 4;
    assert_eq!(&pixels[at..at + 4], [255, 0, 0, 255]);
}

fn long_area(cx: &mut Cx, _: &()) -> Element {
    let mut content = Element::new(Role::Group).props(style!(Style::new().flex(1.0)));
    for index in 0..20 {
        content = content.child(
            Element::new(Role::Group)
                .key(index.to_string())
                .props(style!(Style::new().height(50.0))),
        );
    }
    Element::new(Role::Window).child(cx.scroll_area("area", ScrollAxis::Vertical, content))
}

fn scrolled(app: &mut App) -> f32 {
    app.update();
    app.scene();
    let root = app.runtime().root().clone();
    app.layout()
        .scroll_info(&[0], &root.children[0])
        .expect("it scrolls")
        .y
}

#[test]
fn scrolling_glides_and_notches_add_up() {
    let mut app = app_of(long_area);
    let start = Instant::now();
    app.scroll(50.0, 50.0, 3).unwrap();
    assert!(app.animating());
    assert_eq!(scrolled(&mut app), 0.0, "it hasn't gone yet");
    app.tick(start + MILLIS(40));
    let partway = scrolled(&mut app);
    assert!(partway > 0.0 && partway < 100.0, "{partway}");
    // Another notch on the way adds to where it's going, not where it's got to.
    app.scroll(50.0, 50.0, 3).unwrap();
    app.tick(Instant::now() + LATER);
    let end = scrolled(&mut app);
    assert!((end - 200.0).abs() < 1.0, "{end}");
    assert!(!app.animating());
}

/// A wheel turned steadily, a notch every 50 ms, moves at an even pace: each notch keeps the speed the glide has, rather than
/// setting off again from it, which went from three times the average speed to a third of it and back with every notch.
#[test]
fn notches_in_quick_succession_keep_an_even_pace() {
    let mut app = app_of(long_area);
    let mut seen = vec![(Instant::now(), scrolled(&mut app))];
    let mut notched = Vec::new();
    for _ in 0..6 {
        app.scroll(50.0, 50.0, 3).unwrap();
        notched.push(Instant::now());
        for _ in 0..5 {
            std::thread::sleep(MILLIS(10));
            let now = Instant::now();
            app.tick(now);
            seen.push((now, scrolled(&mut app)));
        }
    }
    app.tick(Instant::now() + LATER);
    let end = scrolled(&mut app);
    assert!((end - 600.0).abs() < 1.0, "{end}");
    assert!(seen.windows(2).all(|pair| pair[1].1 >= pair[0].1), "it only goes one way: {seen:?}");
    // How fast it went between frames, once it's under way, while the wheel's still turning.
    let speeds: Vec<f32> = seen
        .windows(2)
        .filter(|pair| pair[0].0 >= notched[1] && pair[1].0 <= notched[5])
        .map(|pair| (pair[1].1 - pair[0].1) / pair[1].0.duration_since(pair[0].0).as_secs_f32())
        .collect();
    let fastest = speeds.iter().copied().fold(0.0, f32::max);
    let slowest = speeds.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(fastest < slowest * 2.2, "from {slowest} to {fastest} px/s: {speeds:?}");
}

#[test]
fn gliding_stops_at_the_end_and_chains_from_there() {
    let mut app = app_of(long_area);
    app.scroll(50.0, 50.0, 10_000).unwrap();
    app.tick(Instant::now() + LATER);
    let end = scrolled(&mut app);
    assert_eq!(end, 800.0);
    app.scroll(50.0, 50.0, 3).unwrap();
    assert!(!app.animating(), "there's nowhere further to go");
}

#[test]
fn going_right_to_an_end_goes_straight_there_and_stops_any_glide() {
    let mut app = app_of(long_area);
    app.pointer_move(50.0, 50.0);
    app.scroll(50.0, 50.0, 3).unwrap();
    assert!(app.animating());
    app.key(KeyPress::new(KeyCode::End)).unwrap();
    assert_eq!(scrolled(&mut app), 800.0, "it goes straight there");
    assert!(!app.animating(), "and nothing carries on moving it");
    app.tick(Instant::now() + LATER);
    assert_eq!(scrolled(&mut app), 800.0);
}

/// So many rows, each 50 tall.
fn rows(count: usize) -> Element {
    (0..count).fold(
        Element::new(Role::Group).props(style!(Style::new().flex(1.0))),
        |rows, index| {
            rows.child(
                Element::new(Role::Group)
                    .key(index.to_string())
                    .props(style!(Style::new().height(50.0))),
            )
        },
    )
}

/// An area that can go 100, inside one that can go much further.
fn nested_areas(cx: &mut Cx, _: &()) -> Element {
    let content = Element::new(Role::Group)
        .props(style!(Style::new().flex(1.0)))
        .child(
            Element::new(Role::Group)
                .key("holder")
                .props(style!(Style::new().height(100.0)))
                .child(cx.scroll_area("inner", ScrollAxis::Vertical, rows(4))),
        )
        .child(rows(20).key("rows").props(style!(Style::new())));
    Element::new(Role::Window).child(cx.scroll_area("outer", ScrollAxis::Vertical, content))
}

#[test]
fn moving_on_from_an_area_at_its_end_still_gets_that_area_there() {
    let mut app = app_of(nested_areas);
    let moved = |app: &App| {
        let root = app.runtime().root().clone();
        let outer = &root.children[0];
        let inner = &outer.children[0].children[0];
        (
            app.layout().scroll_info(&[0, 0, 0], inner).unwrap().y,
            app.layout().scroll_info(&[0], outer).unwrap().y,
        )
    };
    // The first notch takes the inner one to its end.
    app.scroll(50.0, 50.0, 3).unwrap();
    app.update();
    app.scene();
    assert_eq!(moved(&app), (0.0, 0.0), "neither has gone yet");
    // The next can only go to the outer one, and the inner one mustn't be left behind by that.
    app.scroll(50.0, 50.0, 3).unwrap();
    app.update();
    app.scene();
    assert_eq!(moved(&app), (100.0, 0.0));
    app.tick(Instant::now() + LATER);
    app.update();
    app.scene();
    assert_eq!(moved(&app), (100.0, 100.0));
    assert!(!app.animating());
}

#[test]
fn turning_them_off_puts_a_glide_where_it_was_going() {
    let mut app = app_of(long_area);
    app.scroll(50.0, 50.0, 3).unwrap();
    assert_eq!(scrolled(&mut app), 0.0);
    app.set_animations(false);
    assert_eq!(scrolled(&mut app), 100.0);
    assert!(!app.animating());
}

#[test]
fn a_menu_fades_in() {
    fn with_menu(cx: &mut Cx, _: &()) -> Element {
        let open = cx.state(|| false);
        let items: Vec<(Text, MenuAction)> = vec![(Text::from("First"), Box::new(|_| {}))];
        let menu = menu_button(cx, "file", "File", open, items);
        let root = Element::new(Role::Window).key("root").child(menu);
        cx.on(root, ActionKind::Toggle, move |cx, _| cx.set(open, true))
    }
    let opacity = |app: &App| {
        let root = app.runtime().root();
        match root.children[0].children[1].props.get("opacity") {
            Some(PropValue::Float(opacity)) => Some(*opacity),
            _ => None,
        }
    };
    let mut app = app_of(with_menu);
    let start = Instant::now();
    toggle(&mut app);
    assert_eq!(opacity(&app), Some(0.0), "it's just opened");
    app.tick(start + MILLIS(30));
    app.update();
    let partly = opacity(&app).expect("it's partway");
    assert!(partly > 0.0 && partly < 1.0, "{partly}");
    app.tick(start + LATER);
    app.update();
    assert_eq!(
        opacity(&app),
        None,
        "nothing's left to look at once it's there"
    );
    assert!(!app.animating());
}

#[test]
fn opacity_leaves_what_came_before_it_alone() {
    fn with_sibling(_: &mut Cx, _: &()) -> Element {
        Element::new(Role::Window)
            .props(style!(Style::new().align(Align::Start)))
            .child(Element::new(Role::Group).props(style!(
                Style::new().width(50.0).height(50.0),
                Paint::new().background(Color::hex(0xFF0000)),
            )))
            .child(Element::new(Role::Group).props(style!(
                Style::new().width(50.0).height(50.0),
                Paint::new().background(Color::hex(0xFF0000)).opacity(0.5),
            )))
    }
    let mut screen = Screen::new(with_sibling);
    assert_eq!(screen.pixel(25, 25), [255, 0, 0, 255]);
    assert_ne!(screen.pixel(25, 75), [255, 0, 0, 255]);
}

#[test]
fn a_glide_ends_if_what_was_scrolling_goes_away() {
    fn area(cx: &mut Cx, _: &()) -> Element {
        cx.scroll_area("area", ScrollAxis::Vertical, rows(20))
    }
    fn sometimes(cx: &mut Cx, _: &()) -> Element {
        let shown = cx.state(|| true);
        let mut root = Element::new(Role::Window).key("root");
        if cx.get(shown) {
            root = root.child(cx.child(area, ()));
        }
        cx.on(root, ActionKind::Toggle, move |cx, _| cx.set(shown, false))
    }
    let mut app = app_of(sometimes);
    app.scroll(50.0, 50.0, 3).unwrap();
    assert!(app.animating());
    toggle(&mut app);
    app.tick(Instant::now() + MILLIS(10));
    app.update();
    assert!(!app.animating());
}

fn see_through(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start)))
        .child(
            Element::new(Role::Group)
                .key("outer")
                .props(style!(
                    Style::new().width(100.0).height(100.0),
                    Paint::new().background(Color::hex(0xFF0000)).opacity(0.5),
                ))
                .child(Element::new(Role::Group).key("inner").props(style!(
                    Style::new().width(50.0).height(50.0),
                    Paint::new().background(Color::hex(0x0000FF)),
                ))),
        )
}

#[test]
fn opacity_fades_something_and_everything_in_it() {
    let mut screen = Screen::new(see_through);
    let (behind, outer, inner) = (
        screen.pixel(150, 150),
        screen.pixel(75, 75),
        screen.pixel(25, 25),
    );
    assert_eq!(behind, [0xF6, 0xF8, 0xFA, 255]);
    // Half of red over what's behind it.
    assert!(
        outer[0].abs_diff(250) <= 2 && outer[1].abs_diff(124) <= 2,
        "{outer:?}"
    );
    // The two drawn as one, then faded as one, as CSS has it: where blue covers red, half of blue over what's behind, with none
    // of the red showing through it.
    assert!(
        inner[0].abs_diff(123) <= 3 && inner[1].abs_diff(124) <= 3 && inner[2].abs_diff(253) <= 3,
        "{inner:?}"
    );
}

#[test]
fn no_opacity_at_all_hides_text_too() {
    fn text_at(opacity: f32) -> Element {
        Element::new(Role::Window)
            .props(style!(Style::new().align(Align::Start)))
            .child(
                Element::new(Role::Group)
                    .prop("opacity", f64::from(opacity))
                    .child(
                        Element::new(Role::Text)
                            .name("ABCDABCD")
                            .props(style!(Paint::new().text(Color::BLACK))),
                    ),
            )
    }
    fn hidden(_: &mut Cx, _: &()) -> Element {
        text_at(0.0)
    }
    fn shown(_: &mut Cx, _: &()) -> Element {
        text_at(1.0)
    }
    let is_background = |pixel: &[u8]| pixel == [0xF6, 0xF8, 0xFA, 255];
    let pixels = Screen::new(shown).pixels();
    assert!(!pixels.chunks(4).all(is_background), "there's text to hide");
    let pixels = Screen::new(hidden).pixels();
    assert!(pixels.chunks(4).all(is_background), "and it's hidden");
}

#[test]
fn opacity_only_applies_inside_what_has_it() {
    fn with_sibling(_: &mut Cx, _: &()) -> Element {
        Element::new(Role::Window)
            .props(style!(Style::new().align(Align::Start)))
            .child(Element::new(Role::Group).props(style!(
                Style::new().width(50.0).height(50.0),
                Paint::new().background(Color::hex(0xFF0000)).opacity(0.5),
            )))
            .child(Element::new(Role::Group).props(style!(
                Style::new().width(50.0).height(50.0),
                Paint::new().background(Color::hex(0xFF0000)),
            )))
    }
    let mut screen = Screen::new(with_sibling);
    assert_ne!(screen.pixel(25, 25), [255, 0, 0, 255]);
    assert_eq!(screen.pixel(25, 75), [255, 0, 0, 255]);
}

#[test]
fn a_dialog_fades_in_and_can_be_used_all_the_while() {
    fn with_dialog(cx: &mut Cx, _: &()) -> Element {
        let open = cx.state(|| false);
        let close = Element::new(Role::Button).key("close").name("Close");
        let close = cx.on(close, ActionKind::Click, move |cx, _| cx.set(open, false));
        let shown = dialog(cx, "asked", "A question", open, close);
        let root = Element::new(Role::Window).key("root").children(shown);
        cx.on(root, ActionKind::Toggle, move |cx, _| cx.set(open, true))
    }
    let mut screen = Screen::new(with_dialog);
    let plain = screen.pixel(5, 5);
    screen
        .app
        .runtime_mut()
        .dispatch(&[], ActionKind::Toggle, None)
        .unwrap();
    let start = Instant::now();
    screen.pixels();
    assert!(screen.app.animating());
    assert!(
        screen.app.outline().contains("dialog"),
        "it's there at once"
    );
    screen.app.tick(start + MILLIS(20));
    let partly = screen.pixel(5, 5);
    screen.app.tick(start + LATER);
    let dimmed = screen.pixel(5, 5);
    assert!(
        dimmed[0] < partly[0] && partly[0] < plain[0],
        "{plain:?} {partly:?} {dimmed:?}"
    );
    assert!(!screen.app.animating());
    // Closing fades it out.
    screen.app.key(KeyPress::new(KeyCode::Escape)).unwrap();
    let closing = Instant::now();
    screen.pixels();
    assert!(screen.app.animating());
    assert!(screen.app.outline().contains("dialog"), "still on its way");
    screen.app.tick(closing + MILLIS(30));
    let going = screen.pixel(5, 5);
    assert!(
        going[0] > dimmed[0] && going[0] < plain[0],
        "{plain:?} {going:?} {dimmed:?}"
    );
    screen.app.tick(closing + LATER);
    assert_eq!(screen.pixel(5, 5), plain);
    assert!(
        !screen.app.outline().contains("dialog"),
        "and then it's gone"
    );
    assert!(!screen.app.animating());
}

/// A button behind a dialog, counting its clicks, and what it takes to open that dialog.
fn behind_a_dialog(cx: &mut Cx, _: &()) -> Element {
    let open = cx.state(|| false);
    let clicks = cx.state(|| 0i64);
    let close = Element::new(Role::Button).key("close").name("Close");
    let close = cx.on(close, ActionKind::Click, move |cx, _| cx.set(open, false));
    let shown = dialog(cx, "asked", "A question", open, close);
    let button = Element::new(Role::Button)
        .key("behind")
        .name("ABC")
        .props(style!(Style::new().height(40.0)));
    let button = cx.on(button, ActionKind::Click, move |cx, _| {
        cx.update(clicks, |clicks| *clicks += 1);
    });
    let count = Element::new(Role::Text)
        .key("count")
        .name(text!("{}", cx.get(clicks)));
    let root = Element::new(Role::Window)
        .key("root")
        .child(button)
        .child(count)
        .children(shown);
    cx.on(root, ActionKind::Toggle, move |cx, _| cx.set(open, true))
}

#[test]
fn what_is_behind_a_closing_dialog_can_be_used_straight_away() {
    let mut screen = Screen::new(behind_a_dialog);
    screen.pixels();
    screen
        .app
        .runtime_mut()
        .dispatch(&[], ActionKind::Toggle, None)
        .unwrap();
    screen.app.update();
    screen.app.tick(Instant::now() + LATER);
    screen.pixels();
    // While it's open, a click goes to it, which closes it, and the button doesn't hear.
    screen.app.pointer_down(5.0, 5.0, 1);
    screen.app.pointer_up(5.0, 5.0).unwrap();
    screen.pixels();
    assert!(screen.app.outline().contains("dialog"), "it's fading out");
    assert!(screen.app.animating());
    let root = screen.app.runtime().root().clone();
    assert_eq!(root.children[1].name, "0");
    // A click now goes through it.
    screen.app.pointer_down(5.0, 5.0, 1);
    screen.app.pointer_up(5.0, 5.0).unwrap();
    screen.app.update();
    let root = screen.app.runtime().root().clone();
    assert_eq!(root.children[1].name, "1");
}

#[test]
fn a_dialog_opened_again_while_it_fades_out_can_be_used() {
    let mut screen = Screen::new(behind_a_dialog);
    screen.pixels();
    let open = |screen: &mut Screen| {
        screen
            .app
            .runtime_mut()
            .dispatch(&[], ActionKind::Toggle, None)
            .unwrap();
        screen.pixels();
    };
    open(&mut screen);
    screen.app.key(KeyPress::new(KeyCode::Escape)).unwrap();
    screen.pixels();
    open(&mut screen);
    // This has to land on the dialog, which closes it, and not on the button behind it.
    screen.app.pointer_down(5.0, 5.0, 1);
    screen.app.pointer_up(5.0, 5.0).unwrap();
    screen.app.update();
    let root = screen.app.runtime().root().clone();
    assert_eq!(root.children[1].name, "0");
}

#[test]
fn a_closing_dialog_gives_the_keyboard_back_and_hides_from_assistive_technology() {
    let mut screen = Screen::new(behind_a_dialog);
    screen.pixels();
    screen.app.focus_on(&[0]);
    screen
        .app
        .runtime_mut()
        .dispatch(&[], ActionKind::Toggle, None)
        .unwrap();
    screen.pixels();
    assert_eq!(
        screen.app.focused(),
        Some(&[2, 0, 0][..]),
        "the dialog has it"
    );
    let names = |screen: &mut Screen| {
        screen
            .app
            .access_tree()
            .nodes
            .iter()
            .map(|node| node.name.to_string())
            .collect::<Vec<_>>()
    };
    assert!(names(&mut screen).contains(&"A question".to_owned()));
    screen.app.key(KeyPress::new(KeyCode::Escape)).unwrap();
    screen.pixels();
    assert!(screen.app.outline().contains("dialog"), "it's fading out");
    assert_eq!(screen.app.focused(), Some(&[0][..]), "back where it was");
    assert!(!names(&mut screen).contains(&"A question".to_owned()));
    // Tab can't get back into it.
    for _ in 0..4 {
        screen.app.key(KeyPress::new(KeyCode::Tab)).unwrap();
        assert!(
            screen
                .app
                .focused()
                .is_none_or(|path| path.first() != Some(&2))
        );
    }
}

#[test]
fn a_menu_fades_out_too_and_with_animations_off_goes_at_once() {
    fn with_menu(cx: &mut Cx, _: &()) -> Element {
        let open = cx.state(|| true);
        let items: Vec<(Text, MenuAction)> = vec![(Text::from("First"), Box::new(|_| {}))];
        let menu = menu_button(cx, "file", "File", open, items);
        let root = Element::new(Role::Window).key("root").child(menu);
        cx.on(root, ActionKind::Toggle, move |cx, _| cx.set(open, false))
    }
    let has_menu = |app: &App| app.runtime().root().children[0].children.len() == 2;
    let mut app = app_of(with_menu);
    app.tick(Instant::now() + LATER);
    app.update();
    assert!(has_menu(&app));
    let closing = Instant::now();
    app.runtime_mut()
        .dispatch(&[], ActionKind::Toggle, None)
        .unwrap();
    app.update();
    assert!(has_menu(&app), "it's fading out");
    assert_eq!(
        app.runtime().root().children[0].children[1]
            .props
            .get("inert"),
        Some(&PropValue::Bool(true)),
        "and nothing can use it"
    );
    app.tick(closing + LATER);
    app.update();
    assert!(!has_menu(&app), "and it's gone");

    let mut app = App::new(with_menu, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.update();
    assert!(has_menu(&app));
    app.runtime_mut()
        .dispatch(&[], ActionKind::Toggle, None)
        .unwrap();
    app.update();
    assert!(
        !has_menu(&app),
        "with nothing watching, there's nothing to wait for"
    );
}

#[test]
fn anything_else_inside_something_with_text_takes_the_place_of_the_text() {
    fn button_with_a_child(_: &mut Cx, _: &()) -> Element {
        let child = Element::new(Role::Group)
            .key("child")
            .props(style!(Style::new().width(30.0).height(5.0)));
        let button = Element::new(Role::Button)
            .key("button")
            .name("ABCD")
            .props(style!(Paint::new().text(Color::BLACK)))
            .child(child);
        Element::new(Role::Window)
            .props(style!(Style::new().align(Align::Start)))
            .child(button)
    }
    let mut screen = Screen::new(button_with_a_child);
    let rect = screen.app.layout().rect(&[0]).unwrap();
    assert_eq!((rect.width, rect.height), (30.0, 5.0));
    assert!(
        screen
            .pixels()
            .chunks(4)
            .all(|pixel| pixel == [0xF6, 0xF8, 0xFA, 255]),
        "no text"
    );
}

#[test]
fn a_popup_can_come_from_something_with_text() {
    fn button_with_a_popup(_: &mut Cx, _: &()) -> Element {
        let popup = Element::new(Role::Group).key("popup").props(style!(
            Style::new()
                .overlay(Placement::Below)
                .width(30.0)
                .height(30.0),
            Paint::new().background(Color::hex(0xFF0000)),
        ));
        let with_popup = Element::new(Role::Button)
            .key("button")
            .name("ABCD")
            .props(style!(Paint::new().text(Color::BLACK)))
            .child(popup);
        Element::new(Role::Window)
            .props(style!(Style::new().align(Align::Start)))
            .child(with_popup)
    }
    fn button_alone(_: &mut Cx, _: &()) -> Element {
        Element::new(Role::Window)
            .props(style!(Style::new().align(Align::Start)))
            .child(
                Element::new(Role::Button)
                    .key("button")
                    .name("ABCD")
                    .props(style!(Paint::new().text(Color::BLACK))),
            )
    }
    let mut with = Screen::new(button_with_a_popup);
    let mut alone = Screen::new(button_alone);
    let (with_rect, alone_rect) = (
        with.app.layout().rect(&[0]).unwrap(),
        alone.app.layout().rect(&[0]).unwrap(),
    );
    assert_eq!(with_rect, alone_rect, "a popup doesn't change its size");
    assert!(with_rect.width > 0.0 && with_rect.height > 0.0);
    let popup = with.app.layout().rect(&[0, 0]).unwrap();
    assert_eq!(
        popup.y,
        with_rect.y + with_rect.height,
        "it comes out below"
    );
    // Its text is drawn as it would be alone, and the popup's there too.
    let (first, second) = (with.pixels(), alone.pixels());
    let row = SIZE.0 as usize * 4;
    let text_rows = with_rect.height as usize;
    assert_eq!(first[..text_rows * row], second[..text_rows * row]);
    assert_eq!(with.pixel(5, text_rows + 5), [255, 0, 0, 255]);
}
