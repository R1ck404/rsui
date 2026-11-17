//! The whole stack together: actions go through hit testing, components, patches and
//! layout, and come out as pixels, drawn offscreen so no window opens.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use rsui::prelude::*;
use rsui::render::Renderer;
use rsui::text::TextEngine;
use rsui::{App, Frame};
use support::plain_font;

const SIZE: (u32, u32) = (200, 100);
const RED: Color = Color::hex(0xFF0000);
const BLUE: Color = Color::hex(0x0000FF);
const RED_PIXEL: [u8; 4] = [255, 0, 0, 255];
const BLUE_PIXEL: [u8; 4] = [0, 0, 255, 255];

fn engine() -> TextEngine {
    TextEngine::new(plain_font())
}

fn swatch(color: Color) -> rsui::core::Props {
    rsui::core::Props::from(Style::new().width(50.0).height(50.0))
        .merged(&Paint::new().background(color).into())
}

/// A square at the top left that changes color when clicked.
fn toggle(cx: &mut Cx, _: &()) -> Element {
    let on = cx.state(|| false);
    let color = if cx.get(on) { RED } else { BLUE };
    let square = Element::new(Role::Button)
        .key("square")
        .props(swatch(color));
    let square = cx.on(square, ActionKind::Click, move |cx, _| {
        cx.update(on, |on| *on = !*on);
    });
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start)))
        .child(square)
}

struct Screen {
    app: App,
    renderer: Renderer,
}

impl Screen {
    fn new(render: fn(&mut Cx, &()) -> Element, scale: f32) -> Self {
        Self {
            app: App::new(render, (), engine(), SIZE, scale),
            renderer: Renderer::offscreen(SIZE).expect("the software GPU is always there"),
        }
    }

    fn pixel(&mut self, x: usize, y: usize) -> [u8; 4] {
        self.app.render(&mut self.renderer).expect("drawing works");
        let pixels = self.renderer.read_pixels().expect("reading works");
        let at = (y * SIZE.0 as usize + x) * 4;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    }
}

fn rounded_window(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window).props(Paint::new().background(BLUE).corner_radius(20.0))
}

fn see_through_window(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window).props(
        Paint::new()
            .background(Color::rgba(0, 0, 255, 128))
            .corner_radius(20.0),
    )
}

#[test]
fn the_corners_of_a_window_with_a_radius_are_its_own_color_and_not_a_light_one() {
    let mut screen = Screen::new(rounded_window, 1.0);
    assert_eq!(screen.pixel(0, 0), BLUE_PIXEL);
    assert_eq!(screen.pixel(199, 99), BLUE_PIXEL);
    assert_eq!(screen.pixel(100, 50), BLUE_PIXEL);
}

#[test]
fn a_window_that_can_be_seen_through_still_has_the_usual_color_behind_it() {
    // There's nothing else to show through one, and the color it says isn't what's meant for its corners.
    let mut screen = Screen::new(see_through_window, 1.0);
    assert_eq!(screen.pixel(0, 0), [0xF6, 0xF8, 0xFA, 255]);
}

#[test]
fn a_click_goes_all_the_way_through_to_the_pixels() {
    let mut screen = Screen::new(toggle, 1.0);
    assert_eq!(screen.pixel(25, 25), BLUE_PIXEL);

    screen.app.click(25.0, 25.0).unwrap();
    assert_eq!(screen.app.update(), Frame::Needed);
    assert_eq!(screen.pixel(25, 25), RED_PIXEL);

    screen.app.click(25.0, 25.0).unwrap();
    assert_eq!(screen.app.update(), Frame::Needed);
    assert_eq!(screen.pixel(25, 25), BLUE_PIXEL);
}

#[test]
fn a_click_on_nothing_changes_nothing() {
    let mut screen = Screen::new(toggle, 1.0);
    screen.pixel(0, 0);
    screen.app.click(150.0, 75.0).unwrap();
    assert_eq!(screen.app.update(), Frame::Unchanged);
    assert_eq!(screen.pixel(25, 25), BLUE_PIXEL);
}

#[test]
fn clicks_are_in_physical_pixels_on_a_scaled_screen() {
    let mut screen = Screen::new(toggle, 2.0);
    // The square is 50 layout pixels, so 100 physical pixels wide. Layout pixel 25 wouldn't
    // reach here if the scale were ignored.
    assert_eq!(screen.pixel(90, 90), BLUE_PIXEL);
    screen.app.click(90.0, 90.0).unwrap();
    screen.app.update();
    assert_eq!(screen.pixel(90, 90), RED_PIXEL);
}

/// A button holding a label: the click lands on the label, and has to find the button.
fn labelled(cx: &mut Cx, _: &()) -> Element {
    let count = cx.state(|| 0);
    let label = Element::new(Role::Text)
        .name(text!("{}", cx.get(count)))
        .prop("font_size", 10.0);
    let button = Element::new(Role::Group)
        .key("button")
        .props(style!(Style::new().padding(10.0)))
        .child(label);
    cx.on(button, ActionKind::Click, move |cx, _| {
        cx.update(count, |count| *count += 1);
    })
}

#[test]
fn a_click_on_something_inside_reaches_what_offers_the_action() {
    let mut screen = Screen::new(labelled, 1.0);
    screen.pixel(0, 0);
    screen.app.click(12.0, 12.0).unwrap();
    assert_eq!(screen.app.update(), Frame::Needed);
    assert!(
        screen.app.outline().contains("\"1\""),
        "{}",
        screen.app.outline()
    );
}

fn row(_: &mut Cx, index: &usize) -> Element {
    let color = if *index == 0 { RED } else { BLUE };
    Element::new(Role::ListItem)
        .props(style!(Style::new()))
        .props(rsui::core::Props::from(Paint::new().background(color)))
}

fn long_list(cx: &mut Cx, _: &()) -> Element {
    let list = cx.virtual_list(
        "rows",
        ListWindow {
            len: 1000,
            visible: 5,
            overscan: 1,
            reveal: None,
        },
        |cx, index| cx.keyed(index, row, index),
    );
    let props = list
        .props
        .merged(&Style::new().row_height(20.0).flex(1.0).into());
    Element::new(Role::Window).child(Element { props, ..list })
}

#[test]
fn the_wheel_scrolls_a_list_and_stops_at_the_top() {
    let mut screen = Screen::new(long_list, 1.0);
    assert_eq!(screen.pixel(10, 10), RED_PIXEL, "row 0 is at the top");

    screen.app.scroll(10.0, 10.0, 3).unwrap();
    assert_eq!(screen.app.update(), Frame::Needed);
    assert_eq!(screen.pixel(10, 10), BLUE_PIXEL, "row 0 has gone");
    assert!(
        screen.app.outline().contains("first_visible=3"),
        "{}",
        screen.app.outline()
    );

    screen.app.scroll(10.0, 10.0, -100).unwrap();
    screen.app.update();
    assert_eq!(
        screen.pixel(10, 10),
        RED_PIXEL,
        "it came back, and went no further"
    );
}

#[test]
fn resizing_lays_everything_out_again() {
    fn half(_: &mut Cx, _: &()) -> Element {
        Element::new(Role::Window).child(Element::new(Role::Group).props(style!(
            Style::new().width_percent(50.0).height(50.0),
            Paint::new().background(RED),
        )))
    }
    let mut screen = Screen::new(half, 1.0);
    assert_eq!(screen.pixel(99, 10), RED_PIXEL);
    assert_ne!(screen.pixel(101, 10), RED_PIXEL);

    screen.app.resize((100, 100), 1.0);
    screen.renderer.resize((100, 100)).unwrap();
    screen.app.render(&mut screen.renderer).unwrap();
    let pixels = screen.renderer.read_pixels().unwrap();
    let at = |x: usize, y: usize| {
        let at = (y * 100 + x) * 4;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    };
    assert_eq!(at(49, 10), RED_PIXEL);
    assert_ne!(at(51, 10), RED_PIXEL);
}

fn delayed(cx: &mut Cx, _: &()) -> Element {
    let ready = cx.state(|| false);
    cx.effect((), move |cx| {
        cx.after(Duration::from_millis(5), move |cx| cx.set(ready, true));
    });
    let color = if cx.get(ready) { RED } else { BLUE };
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start)))
        .child(Element::new(Role::Group).props(swatch(color)))
}

#[test]
fn background_work_wakes_the_app_and_reaches_the_pixels() {
    let (sender, woken) = std::sync::mpsc::channel();
    let mut screen = Screen::new(delayed, 1.0);
    screen.app.set_waker(move || {
        let _ = sender.send(());
    });
    assert_eq!(screen.pixel(10, 10), BLUE_PIXEL);

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        woken
            .recv_timeout(Duration::from_secs(10))
            .expect("the task wakes the app");
        if screen.app.update() == Frame::Needed {
            break;
        }
        assert!(Instant::now() < deadline, "it never changed");
    }
    assert_eq!(screen.pixel(10, 10), RED_PIXEL);
}

#[test]
fn an_update_with_nothing_to_do_needs_no_frame() {
    let mut screen = Screen::new(toggle, 1.0);
    assert_eq!(
        screen.app.update(),
        Frame::Needed,
        "nothing has been laid out yet"
    );
    screen.pixel(0, 0);
    assert_eq!(screen.app.update(), Frame::Unchanged);
}

#[test]
fn the_outline_says_what_is_there_without_the_styling() {
    let screen = Screen::new(toggle, 1.0);
    assert_eq!(
        screen.app.outline(),
        "window @/\n  button #square [click] @0\n"
    );
}
