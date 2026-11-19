//! The pill in the corner of a window being worked on: it's drawn over the app, a click on it opens the inspector rather
//! than reaching the app, and its × puts it away.
use super::*;
use crate::replay::Input;
#[path = "../../rsui-text/tests/support/mod.rs"]
mod font;

const SIZE: (u32, u32) = (240, 180);

fn fixture(cx: &mut Cx, _: &()) -> Element {
    let clicks = cx.state(|| 0_i64);
    let button = Element::new(Role::Button)
        .key("everything")
        .name(format!("Clicked {}", cx.get(clicks)))
        .prop("width", 240)
        .prop("height", 180);
    cx.on(button, ActionKind::Click, move |cx, _| cx.update(clicks, |n| *n += 1))
}

fn clicks(app: &App) -> String {
    app.runtime.root().name.to_string()
}

#[test]
fn the_pill_opens_the_inspector_and_goes_away() {
    let mut app = App::new(fixture, (), TextEngine::new(font::plain_font()), SIZE, 1.0);
    app.set_pill(true);
    assert!(app.pill.showing(), "the inspector's allowed in a debug build, so a window has the pill");
    app.update();
    app.scene();
    assert!(!app.pill_overlay.is_empty(), "it's drawn");
    // The pill is 176 by 22 in the bottom right, 8 from the edges.
    let (left, top) = (240.0 - 176.0 - 8.0, 180.0 - 22.0 - 8.0);
    app.feed(&Input::Down { x: left + 20.0, y: top + 10.0, clicks: 1 }).unwrap();
    app.feed(&Input::Up { x: left + 20.0, y: top + 10.0 }).unwrap();
    app.update();
    assert_eq!(clicks(&app), "Clicked 0", "the app didn't hear it");
    assert!(app.inspector_showing(), "with no viewer to open, what's drawn over the app shows");
    // Somewhere else, the app hears the click as ever.
    app.feed(&Input::Down { x: 20.0, y: 20.0, clicks: 1 }).unwrap();
    app.feed(&Input::Up { x: 20.0, y: 20.0 }).unwrap();
    app.update();
    assert_eq!(clicks(&app), "Clicked 1");
    // The × puts it away.
    app.feed(&Input::Down { x: left + 176.0 - 6.0, y: top + 10.0, clicks: 1 }).unwrap();
    app.feed(&Input::Up { x: left + 176.0 - 6.0, y: top + 10.0 }).unwrap();
    app.update();
    app.scene();
    assert!(!app.pill.showing());
    assert_eq!(clicks(&app), "Clicked 1");
    app.feed(&Input::Down { x: left + 20.0, y: top + 10.0, clicks: 1 }).unwrap();
    app.feed(&Input::Up { x: left + 20.0, y: top + 10.0 }).unwrap();
    app.update();
    assert_eq!(clicks(&app), "Clicked 2", "once it's gone, the app has its corner back");
}

#[test]
fn picking_takes_the_next_click_for_the_viewer() {
    let mut app = App::new(fixture, (), TextEngine::new(font::plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    app.set_picking(true);
    app.feed(&Input::Move { x: 30.0, y: 30.0 }).unwrap();
    assert!(app.inspector.as_ref().is_some_and(|inspector| inspector.highlight.is_some()), "what it's on is filled in");
    app.feed(&Input::Down { x: 30.0, y: 30.0, clicks: 1 }).unwrap();
    app.feed(&Input::Up { x: 30.0, y: 30.0 }).unwrap();
    app.update();
    assert_eq!(clicks(&app), "Clicked 0", "the app didn't hear the click that picked");
    assert_eq!(app.take_picked(), Some(Vec::new()), "it picked the button, which is the root");
    assert!(!app.picking());
    app.feed(&Input::Down { x: 30.0, y: 30.0, clicks: 1 }).unwrap();
    app.feed(&Input::Up { x: 30.0, y: 30.0 }).unwrap();
    app.update();
    assert_eq!(clicks(&app), "Clicked 1", "and the next one is the app's again");
}

#[test]
fn without_the_pill_nothing_is_drawn_over_the_app() {
    let mut app = App::new(fixture, (), TextEngine::new(font::plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    assert!(app.pill_overlay.is_empty(), "only a window asks for the pill");
}
