//! Long lists in a window: knowing how many rows really fit, and moving with the keyboard.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use rsui::prelude::*;
use rsui::render::Renderer;
use rsui::text::TextEngine;
use rsui::{App, KeyCode, KeyPress};
use support::plain_font;

const ROW: f32 = 20.0;
const SIZE: (u32, u32) = (200, 100);
const RED: Color = Color::hex(0xFF0000);
const BLUE: Color = Color::hex(0x0000FF);
const RED_PIXEL: [u8; 4] = [255, 0, 0, 255];
const BLUE_PIXEL: [u8; 4] = [0, 0, 255, 255];

/// How long the list is, and how many rows it guesses will fit.
type Setup = (usize, usize);

fn row(_: &mut Cx, index: &usize) -> Element {
    // Only the last row is red, so a test can tell when it's on screen.
    let color = if *index == 999 { RED } else { BLUE };
    Element::new(Role::ListItem)
        .props(rsui::core::Props::from(Style::new()).merged(&Paint::new().background(color).into()))
}

fn list(cx: &mut Cx, setup: &Setup) -> Element {
    let list = cx.virtual_list(
        "rows",
        ListWindow {
            len: setup.0,
            visible: setup.1,
            overscan: 1,
            reveal: None,
        },
        |cx, index| cx.keyed(index, row, index),
    );
    let props = list
        .props
        .merged(&Style::new().row_height(ROW).flex(1.0).into());
    Element::new(Role::Window)
        .key("root")
        .child(Element { props, ..list })
}

fn app(setup: Setup) -> App {
    let mut app = App::new(list, setup, TextEngine::new(plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    app
}

fn the_list(app: &App) -> Node {
    app.runtime().root().children[0].clone()
}

fn prop(app: &App, name: &str) -> i64 {
    match the_list(app).props.get(name) {
        Some(PropValue::Int(value)) => *value,
        other => panic!("{name} is {other:?}"),
    }
}

fn built(app: &App) -> usize {
    the_list(app).children.len()
}

fn press(app: &mut App, code: KeyCode) {
    app.key(KeyPress::new(code)).expect("the key goes through");
    app.update();
    app.scene();
}

#[test]
fn a_list_works_out_how_many_rows_fit_whatever_it_guessed() {
    // 100 pixels of 20 pixel rows is 5, plus 1 extra row below it.
    for guess in [0, 1, 5, 50, 500] {
        let app = app((1000, guess));
        assert_eq!(built(&app), 6, "guessing {guess}");
    }
}

#[test]
fn the_end_of_a_list_can_be_reached_whatever_it_guessed() {
    for guess in [1, 5, 50, 500] {
        let mut app = app((1000, guess));
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::End);
        assert_eq!(prop(&app, "first_visible"), 995, "guessing {guess}");

        // The last row is on screen, at the bottom.
        let mut renderer = Renderer::offscreen(SIZE).unwrap();
        app.render(&mut renderer).unwrap();
        let pixels = renderer.read_pixels().unwrap();
        let at = |y: usize| {
            let at = (y * SIZE.0 as usize + 10) * 4;
            [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
        };
        assert_eq!(at(90), RED_PIXEL, "guessing {guess}");
        assert_eq!(at(70), BLUE_PIXEL, "guessing {guess}");
    }
}

#[test]
fn a_row_cut_off_by_the_edge_is_built_but_the_last_one_never_is() {
    let mut app = app((1000, 5));
    app.resize((200, 110), 1.0);
    app.scene();
    // 5.5 rows fit: 6 to show, and 1 extra.
    assert_eq!(built(&app), 7);

    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::End);
    assert_eq!(prop(&app, "first_visible"), 995, "5 whole rows at the end");
    assert!(the_list(&app).children.len() >= 6);
}

#[test]
fn resizing_the_window_changes_how_many_rows_there_are() {
    let mut app = app((1000, 5));
    app.resize((200, 400), 1.0);
    app.scene();
    assert_eq!(built(&app), 21);
    app.resize((200, 40), 1.0);
    app.scene();
    assert_eq!(built(&app), 3);
}

#[test]
fn growing_at_the_end_of_a_list_shows_more_of_it_rather_than_a_gap() {
    let mut app = app((1000, 5));
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::End);
    assert_eq!(prop(&app, "first_visible"), 995);
    app.resize((200, 400), 1.0);
    app.scene();
    assert_eq!(prop(&app, "first_visible"), 980);
}

#[test]
fn a_scale_counts_in_layout_pixels() {
    let mut app = App::new(
        list,
        (1000, 5),
        TextEngine::new(plain_font()),
        (200, 200),
        2.0,
    );
    app.update();
    app.scene();
    // 200 physical pixels is 100 layout pixels.
    assert_eq!(built(&app), 6);
}

fn unbounded(cx: &mut Cx, _: &()) -> Element {
    let list = cx.virtual_list(
        "rows",
        ListWindow {
            len: 1_000_000,
            visible: 5,
            overscan: 1,
            reveal: None,
        },
        |cx, index| cx.keyed(index, row, index),
    );
    let props = list.props.merged(&Style::new().row_height(ROW).into());
    // Nothing limits how tall this list is, so it wants to be 20 million pixels.
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start)))
        .child(Element { props, ..list })
}

#[test]
fn a_list_with_no_limit_on_its_height_only_builds_what_the_window_can_show() {
    let started = Instant::now();
    let mut app = App::new(unbounded, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    assert!(
        the_list(&app).children.len() <= 7,
        "built {}",
        the_list(&app).children.len()
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn the_size_is_only_worked_out_when_it_changes() {
    let mut app = app((1000, 5));
    let renders = app.runtime().renders();
    press(&mut app, KeyCode::Tab);
    app.scene();
    app.scene();
    assert_eq!(
        app.runtime().renders(),
        renders,
        "nothing changed, so nothing rendered"
    );
}

#[test]
fn tab_reaches_a_list_and_keys_move_it() {
    let mut app = app((1000, 5));
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.focused(), Some(&[0][..]));

    press(&mut app, KeyCode::Down);
    assert_eq!(prop(&app, "first_visible"), 1);
    press(&mut app, KeyCode::Down);
    assert_eq!(prop(&app, "first_visible"), 2);
    press(&mut app, KeyCode::Up);
    assert_eq!(prop(&app, "first_visible"), 1);
    // 5 rows fit, so a page moves 4, keeping one in view.
    press(&mut app, KeyCode::PageDown);
    assert_eq!(prop(&app, "first_visible"), 5);
    press(&mut app, KeyCode::PageUp);
    assert_eq!(prop(&app, "first_visible"), 1);
    press(&mut app, KeyCode::End);
    assert_eq!(prop(&app, "first_visible"), 995);
    press(&mut app, KeyCode::Down);
    assert_eq!(prop(&app, "first_visible"), 995, "it stops at the end");
    press(&mut app, KeyCode::PageDown);
    assert_eq!(prop(&app, "first_visible"), 995);
    press(&mut app, KeyCode::Home);
    assert_eq!(prop(&app, "first_visible"), 0);
    press(&mut app, KeyCode::Up);
    assert_eq!(prop(&app, "first_visible"), 0, "it stops at the start");
    assert_eq!(app.focused(), Some(&[0][..]), "and it keeps the keyboard");
}

#[test]
fn clicking_a_list_gives_it_the_keyboard() {
    let mut app = app((1000, 5));
    app.pointer_down(10.0, 10.0, 1);
    app.pointer_up(10.0, 10.0).unwrap();
    assert_eq!(app.focused(), Some(&[0][..]));
    press(&mut app, KeyCode::Down);
    assert_eq!(prop(&app, "first_visible"), 1);
}

#[test]
fn with_nothing_focused_keys_move_the_list_under_the_pointer() {
    let mut app = app((1000, 5));
    press(&mut app, KeyCode::Down);
    assert_eq!(prop(&app, "first_visible"), 0, "the pointer isn't over it");
    app.pointer_move(10.0, 10.0);
    press(&mut app, KeyCode::Down);
    assert_eq!(prop(&app, "first_visible"), 1);
}

#[test]
fn keys_held_with_others_are_left_for_shortcuts() {
    let mut app = app((1000, 5));
    press(&mut app, KeyCode::Tab);
    for key in [
        KeyPress::new(KeyCode::Down).ctrl(),
        KeyPress::new(KeyCode::Down).shift(),
    ] {
        app.key(key).unwrap();
        app.update();
        assert_eq!(prop(&app, "first_visible"), 0);
    }
}

fn with_shortcut(cx: &mut Cx, _: &()) -> Element {
    let pressed = cx.state(|| 0);
    let list = cx.virtual_list(
        "rows",
        ListWindow {
            len: 100,
            visible: 5,
            overscan: 1,
            reveal: None,
        },
        |cx, index| cx.keyed(index, row, index),
    );
    let props = list
        .props
        .merged(&Style::new().row_height(ROW).flex(1.0).into());
    let holder = Element::new(Role::Group)
        .key("holder")
        .name(text!("{}", cx.get(pressed)))
        .props(style!(Style::new().flex(1.0)))
        .child(Element { props, ..list });
    let holder = cx.on(holder, ActionKind::KeyPress, move |cx, _| {
        cx.update(pressed, |pressed| *pressed += 1);
    });
    Element::new(Role::Window).child(holder)
}

#[test]
fn the_nearest_thing_wanting_a_key_gets_it() {
    let mut app = App::new(with_shortcut, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    press(&mut app, KeyCode::Tab);
    let list = || |app: &App| app.runtime().root().children[0].children[0].clone();
    assert!(list()(&app).actions.contains(ActionKind::Scroll));
    press(&mut app, KeyCode::Down);
    // The list is nearer than the group around it.
    assert_eq!(
        list()(&app).props.get("first_visible"),
        Some(&PropValue::Int(1))
    );
    assert_eq!(app.runtime().root().children[0].name, "0");
    // A key that means nothing to a list goes on past it.
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.runtime().root().children[0].name, "1");
}

fn padded(cx: &mut Cx, _: &()) -> Element {
    let list = cx.virtual_list(
        "rows",
        ListWindow {
            len: 1000,
            visible: 50,
            overscan: 1,
            reveal: None,
        },
        |cx, index| cx.keyed(index, row, index),
    );
    let props = list
        .props
        .merged(&Style::new().row_height(ROW).flex(1.0).padding(10.0).into());
    Element::new(Role::Window).child(Element { props, ..list })
}

#[test]
fn padding_does_not_count_as_room_for_rows() {
    let mut app = App::new(padded, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    // 100 pixels, less 20 of padding, is 4 rows, plus 1 extra.
    assert_eq!(built(&app), 5);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::End);
    assert_eq!(prop(&app, "first_visible"), 996);
}

fn own_keys(cx: &mut Cx, _: &()) -> Element {
    let pressed = cx.state(|| 0);
    let list = cx.virtual_list(
        "rows",
        ListWindow {
            len: 100,
            visible: 5,
            overscan: 1,
            reveal: None,
        },
        |cx, index| cx.keyed(index, row, index),
    );
    let props = list
        .props
        .merged(&Style::new().row_height(ROW).flex(1.0).into());
    let list = cx.on(
        Element { props, ..list },
        ActionKind::KeyPress,
        move |cx, _| cx.update(pressed, |pressed| *pressed += 1),
    );
    Element::new(Role::Window)
        .name(text!("{}", cx.get(pressed)))
        .child(list)
}

#[test]
fn a_list_with_its_own_key_handler_decides_for_itself() {
    let mut app = App::new(own_keys, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.runtime().root().name, "1", "its handler heard the key");
    assert_eq!(
        prop(&app, "first_visible"),
        0,
        "so it didn't move by itself"
    );
}

#[test]
fn a_list_refuses_a_size_that_makes_no_sense() {
    let mut app = app((1000, 5));
    let path = [0];
    let mut resize = |value: PropValue| {
        app.runtime_mut()
            .dispatch(&path, ActionKind::Resize, Some(value))
    };
    assert!(resize(PropValue::Float(-1.0)).is_err());
    assert!(resize(PropValue::Float(f64::NAN)).is_err());
    assert!(resize(PropValue::Float(f64::INFINITY)).is_err());
    assert!(resize(PropValue::from("many")).is_err());
    assert!(resize(PropValue::Int(-3)).is_err());
    assert!(resize(PropValue::Int(3)).is_ok());
}
