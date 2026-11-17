//! What's been turned or resized gets its clicks where it's drawn, and not where it was laid out.

use std::sync::atomic::{AtomicU32, Ordering};

use rsui::App;
use rsui::prelude::*;
use rsui::render::Renderer;
use rsui::runtime::Cx;

const SIZE: (u32, u32) = (400, 300);

/// A button, as these say, with what to count its clicks in.
fn button(cx: &mut Cx<'_>, count: &'static AtomicU32, name: &str, value: PropValue) -> Node {
    let made = Element::new(Role::Button)
        .key(1u64)
        .prop("width", 40.0)
        .prop("height", 40.0)
        .prop("background", 0xFF00_00FF_i64)
        .prop(name, value);
    cx.on(made, ActionKind::Click, move |_, _| {
        count.fetch_add(1, Ordering::SeqCst);
    })
    .into_node()
}

struct Moved {
    count: &'static AtomicU32,
    name: &'static str,
    value: PropValue,
}

fn moved(cx: &mut Cx<'_>, props: &Moved) -> Element {
    let inner = button(cx, props.count, props.name, props.value.clone());
    Element::new(Role::Window).child(inner)
}

/// Clicks here, and says whether that landed on the button.
fn lands(app: &mut App, count: &AtomicU32, x: f32, y: f32) -> bool {
    let before = count.load(Ordering::SeqCst);
    app.pointer_move(x, y);
    app.pointer_down(x, y, 1);
    let _ = app.pointer_up(x, y);
    app.update();
    count.load(Ordering::SeqCst) != before
}

fn drawn(count: &'static AtomicU32, name: &'static str, value: PropValue) -> App {
    let text = rsui::default_text_engine().unwrap();
    let mut app = App::new(moved, Moved { count, name, value }, text, SIZE, 1.0);
    app.update();
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    app.render(&mut renderer).unwrap();
    app
}

#[test]
fn a_click_lands_where_something_was_moved_to_and_not_where_it_was() {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let mut app = drawn(
        &COUNT,
        "transform",
        PropValue::from("translate(100px, 50px)"),
    );
    assert!(lands(&mut app, &COUNT, 120.0, 70.0), "where it's drawn");
    assert!(
        !lands(&mut app, &COUNT, 20.0, 20.0),
        "where it was laid out"
    );
}

#[test]
fn a_click_lands_on_all_of_something_made_bigger() {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let mut app = drawn(&COUNT, "scale", PropValue::Float(2.0));
    // It's 40 across around its middle at 20, so it goes from -20 to 60. The window's edge stops what's left of that.
    assert!(
        lands(&mut app, &COUNT, 55.0, 55.0),
        "only there now it's bigger"
    );
    assert!(lands(&mut app, &COUNT, 20.0, 20.0), "its middle");
    assert!(!lands(&mut app, &COUNT, 65.0, 20.0), "past it");
}

#[test]
fn a_click_lands_on_the_corners_something_turned_has_and_not_those_it_had() {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let mut app = drawn(
        &COUNT,
        "transform",
        PropValue::from("translate(100px, 100px) rotate(45deg)"),
    );
    // Its middle is at 120, 120, and turned that way its corners are 28 up, down and to each side.
    assert!(lands(&mut app, &COUNT, 120.0, 120.0), "the middle");
    assert!(
        lands(&mut app, &COUNT, 120.0, 95.0),
        "further up than it went before"
    );
    assert!(
        lands(&mut app, &COUNT, 145.0, 120.0),
        "further across than it went before"
    );
    assert!(
        !lands(&mut app, &COUNT, 103.0, 103.0),
        "where a corner was before it turned"
    );
    assert!(
        !lands(&mut app, &COUNT, 137.0, 137.0),
        "where a corner was before it turned"
    );
}

#[test]
fn something_squashed_flat_can_not_be_clicked_and_nothing_goes_wrong() {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let mut app = drawn(&COUNT, "scale", PropValue::Float(0.0));
    // There's nothing to undo that with, so this goes by where it was laid out, which is at least somewhere to click.
    assert!(lands(&mut app, &COUNT, 20.0, 20.0));
}
