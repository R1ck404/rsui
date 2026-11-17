//! Shapes, from what's asked for to the pixels, drawn offscreen so no window opens.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use rsui::prelude::*;
use rsui::render::{LineCap, Renderer, shape_layer};
use rsui::text::TextEngine;
use rsui::{App, Frame};
use support::plain_font;

const SIZE: (u32, u32) = (200, 200);
const RED: Color = Color::hex(0xFF0000);
const BLUE: Color = Color::hex(0x0000FF);
const RED_PIXEL: [u8; 4] = [255, 0, 0, 255];
const BLUE_PIXEL: [u8; 4] = [0, 0, 255, 255];
const WHITE_PIXEL: [u8; 4] = [255, 255, 255, 255];

struct Screen {
    app: App,
    renderer: Renderer,
}

impl Screen {
    fn new(render: fn(&mut Cx, &()) -> Element, scale: f32) -> Self {
        let size = (SIZE.0, SIZE.1);
        Self {
            app: App::new(render, (), TextEngine::new(plain_font()), size, scale),
            renderer: Renderer::offscreen(size).expect("the software GPU is always there"),
        }
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

fn page(child: Element) -> Element {
    Element::new(Role::Window)
        .props(Paint::new().background(Color::WHITE))
        .child(child)
}

fn shape(view: &str, width: f32, height: f32, layers: &[String]) -> Element {
    Element::new(Role::Image)
        .props(Style::new().width(width).height(height))
        .prop("view_box", view)
        .prop("shape_layers", layers.join("\n"))
}

fn red_square(_: &mut Cx, _: &()) -> Element {
    page(shape(
        "0 0 10 10",
        100.0,
        100.0,
        &[shape_layer(
            "M2 2H8V8H2Z",
            Some(Some(RED)),
            None,
            LineCap::Butt,
        )],
    ))
}

#[test]
fn a_filled_shape_is_where_it_says_at_the_size_it_is_shown() {
    let mut screen = Screen::new(red_square, 1.0);
    assert_eq!(screen.pixel(50, 50), RED_PIXEL);
    assert_eq!(screen.pixel(25, 25), RED_PIXEL);
    assert_eq!(screen.pixel(79, 79), RED_PIXEL);
    assert_eq!(screen.pixel(15, 50), WHITE_PIXEL);
    assert_eq!(screen.pixel(85, 50), WHITE_PIXEL);
    assert_eq!(screen.pixel(150, 150), WHITE_PIXEL);
}

#[test]
fn on_a_bigger_screen_it_stays_sharp_rather_than_being_stretched() {
    let mut screen = Screen::new(red_square, 2.0);
    // The same, with everything twice as far.
    assert_eq!(screen.pixel(40, 100), RED_PIXEL);
    assert_eq!(
        screen.pixel(39, 100),
        WHITE_PIXEL,
        "an edge on a pixel stays there"
    );
    assert_eq!(screen.pixel(30, 100), WHITE_PIXEL);
}

fn layers_on_top(_: &mut Cx, _: &()) -> Element {
    page(shape(
        "0 0 100 100",
        100.0,
        100.0,
        &[
            shape_layer("M0 0H100V100H0Z", Some(Some(RED)), None, LineCap::Butt),
            shape_layer("M20 50H80", None, Some((Some(BLUE), 20.0)), LineCap::Butt),
        ],
    ))
}

#[test]
fn layers_go_over_one_another_and_a_stroke_is_as_wide_as_it_says() {
    let mut screen = Screen::new(layers_on_top, 1.0);
    assert_eq!(screen.pixel(50, 50), BLUE_PIXEL);
    assert_eq!(screen.pixel(50, 41), BLUE_PIXEL);
    assert_eq!(screen.pixel(50, 38), RED_PIXEL);
    assert_eq!(screen.pixel(50, 62), RED_PIXEL);
    assert_eq!(screen.pixel(10, 50), RED_PIXEL);
}

fn takes_the_text_color(_: &mut Cx, _: &()) -> Element {
    page(
        shape(
            "0 0 10 10",
            100.0,
            100.0,
            &[shape_layer(
                "M0 0H10V10H0Z",
                Some(None),
                None,
                LineCap::Butt,
            )],
        )
        .prop("text_color", BLUE.to_prop()),
    )
}

#[test]
fn it_can_take_its_color_from_the_element() {
    let mut screen = Screen::new(takes_the_text_color, 1.0);
    assert_eq!(screen.pixel(50, 50), BLUE_PIXEL);
}

fn moved_along(_: &mut Cx, _: &()) -> Element {
    let mut layer = shape_layer("M0 0H2V2H0Z", Some(Some(RED)), None, LineCap::Butt);
    layer.push_str("\ttranslate(4 4)");
    page(shape("0 0 10 10", 100.0, 100.0, &[layer]))
}

#[test]
fn a_layer_can_be_moved_with_a_transform_before_it_is_fitted() {
    let mut screen = Screen::new(moved_along, 1.0);
    assert_eq!(screen.pixel(50, 50), RED_PIXEL, "where it's moved to");
    assert_eq!(screen.pixel(10, 10), WHITE_PIXEL, "where it was");
}

fn natural_size(_: &mut Cx, _: &()) -> Element {
    page(
        Element::new(Role::Image)
            .props(Style::new().align_self(Align::Start))
            .prop("view_box", "0 0 40 30")
            .prop(
                "shape_layers",
                shape_layer("M0 0H40V30H0Z", Some(Some(RED)), None, LineCap::Butt),
            ),
    )
}

#[test]
fn with_no_size_of_its_own_it_is_as_big_as_its_numbers() {
    let mut screen = Screen::new(natural_size, 1.0);
    assert_eq!(screen.pixel(39, 29), RED_PIXEL);
    assert_eq!(screen.pixel(41, 15), WHITE_PIXEL);
    assert_eq!(screen.pixel(15, 31), WHITE_PIXEL);
}

fn with_a_click(cx: &mut Cx, _: &()) -> Element {
    let clicks = cx.state(|| 0);
    let icon = shape(
        "0 0 10 10",
        50.0,
        50.0,
        &[shape_layer(
            "M0 0H10V10H0Z",
            Some(Some(RED)),
            None,
            LineCap::Butt,
        )],
    )
    .key("icon");
    let icon = cx.on(icon, ActionKind::Click, move |cx, _| {
        cx.update(clicks, |clicks| *clicks += 1);
    });
    page(icon).child(
        Element::new(Role::Text)
            .name(format!("clicked {}", cx.get(clicks)))
            .prop("font_size", 10.0),
    )
}

#[test]
fn a_shape_can_be_clicked_like_anything_else() {
    let mut screen = Screen::new(with_a_click, 1.0);
    screen.pixels();
    screen.app.pointer_move(25.0, 25.0);
    screen.app.pointer_down(25.0, 25.0, 1);
    let _ = screen.app.pointer_up(25.0, 25.0);
    screen.app.update();
    assert!(
        screen.app.outline().contains("clicked 1"),
        "{}",
        screen.app.outline()
    );
}

fn many_times(_: &mut Cx, _: &()) -> Element {
    let icon = shape_layer("M0 0H10V10H0Z", Some(Some(RED)), None, LineCap::Round);
    let mut row = Element::new(Role::Group).props(Style::row());
    for key in 0..20 {
        row = row.child(
            shape("0 0 10 10", 8.0, 8.0, std::slice::from_ref(&icon))
                .key(format!("{key}").as_str()),
        );
    }
    page(row)
}

#[test]
fn the_same_shape_many_times_is_one_picture() {
    let mut screen = Screen::new(many_times, 1.0);
    screen.pixels();
    assert_eq!(screen.app.update(), Frame::Unchanged);
    // Each picture has its own bit of memory on the GPU, so this says how many there are.
    assert_eq!(screen.app.images_gpu_bytes(), 8 * 8 * 4);
    assert_eq!(screen.pixel(4, 4), RED_PIXEL);
    assert_eq!(screen.pixel(156, 4), RED_PIXEL);
}

fn stretched_or_not(cx: &mut Cx, _: &()) -> Element {
    let stretched = cx.state(|| true);
    let button = Element::new(Role::Button)
        .key("change")
        .name("change")
        .prop("font_size", 10.0);
    let button = cx.on(button, ActionKind::Click, move |cx, _| {
        cx.update(stretched, |stretched| *stretched = !*stretched);
    });
    page(
        shape(
            "0 0 10 10",
            100.0,
            50.0,
            &[shape_layer(
                "M0 0H10V10H0Z",
                Some(Some(RED)),
                None,
                LineCap::Butt,
            )],
        )
        .prop("stretch", cx.get(stretched)),
    )
    .child(button)
}

#[test]
fn stretching_or_not_is_a_different_picture_whichever_way_it_goes() {
    let mut screen = Screen::new(stretched_or_not, 1.0);
    assert_eq!(screen.pixel(5, 25), RED_PIXEL, "it fills all of it");
    screen
        .app
        .key(rsui::KeyPress::new(rsui::KeyCode::Tab))
        .unwrap();
    screen
        .app
        .key(rsui::KeyPress::new(rsui::KeyCode::Enter))
        .unwrap();
    assert_eq!(
        screen.pixel(5, 25),
        WHITE_PIXEL,
        "now it keeps its shape, in the middle"
    );
    assert_eq!(screen.pixel(50, 25), RED_PIXEL);
}
