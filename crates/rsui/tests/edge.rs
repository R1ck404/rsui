//! The things that shouldn't happen, happening: absurd sizes, values of the wrong kind, things
//! disappearing mid-action, and very large inputs. None of these may panic, hang, or use memory
//! out of proportion to what they ask for.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use rsui::prelude::*;
use rsui::render::Renderer;
use rsui::text::TextEngine;
use rsui::{App, KeyCode, KeyPress};
use support::plain_font;

const SIZE: (u32, u32) = (200, 100);

fn app<P: 'static, R: IntoNode + 'static>(render: fn(&mut Cx<'_>, &P) -> R, props: P) -> App {
    App::new(render, props, TextEngine::new(plain_font()), SIZE, 1.0)
}

/// Works out a frame, and draws it if it's small enough to be worth the software GPU's time.
fn frame(app: &mut App) {
    app.update();
    let renderer = &mut Renderer::offscreen(SIZE).expect("the software GPU is always there");
    app.render(renderer).expect("drawing works");
}

fn key(code: KeyCode) -> KeyPress {
    KeyPress::new(code)
}

/// Runs something on a thread with a stack the size of a real program's main thread, so a test
/// finds out what a real program would.
fn on_main_sized_stack(run: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(run)
        .expect("a thread starts")
        .join()
        .expect("it doesn't panic");
}

fn label(_: &mut Cx, text: &&'static str) -> Element {
    Element::new(Role::Window).child(Element::new(Role::Text).name(*text))
}

#[test]
fn any_window_size_can_be_drawn() {
    let mut app = app(label, "Some text");
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    for size in [
        (0, 0),
        (1, 1),
        (0, 500),
        (500, 0),
        (1, 4000),
        (4000, 1),
        (200, 100),
    ] {
        app.resize(size, 1.0);
        renderer.resize(size).unwrap();
        app.update();
        app.render(&mut renderer).unwrap();
    }
}

#[test]
fn any_scale_can_be_drawn() {
    // A closed window reports a scale of zero, and nothing else should ever produce the others.
    for scale in [
        0.0,
        -1.0,
        f32::NAN,
        f32::INFINITY,
        f32::MIN_POSITIVE,
        0.25,
        1.0,
        3.5,
        100.0,
    ] {
        let mut app = app(label, "Some text");
        app.resize(SIZE, scale);
        frame(&mut app);
        app.pointer_move(10.0, 10.0);
        app.click(10.0, 10.0).unwrap();
        frame(&mut app);
    }
}

fn sized_text(_: &mut Cx, size: &u64) -> Element {
    let size = f64::from_bits(*size);
    Element::new(Role::Window).child(Element::new(Role::Text).name("ABC").prop("font_size", size))
}

#[test]
fn any_font_size_can_be_drawn_without_using_all_the_memory() {
    for size in [
        0.0,
        -5.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        0.001,
        1.0,
        500.0,
        5_000.0,
        1e6,
        1e30,
    ] {
        let started = Instant::now();
        let mut app = app(sized_text, size.to_bits());
        frame(&mut app);
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "font size {size} took {:?}",
            started.elapsed()
        );
    }
}

fn wrong_kinds(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window).child(
        Element::new(Role::Text)
            .name("ABC")
            .prop("font_size", "large")
            .prop("font_weight", 700i64)
            .prop("background", "red")
            .prop("text_color", true)
            .prop("corner_radius", "round")
            .prop("border_width", "thin")
            .prop("border_color", 1.5)
            .prop("hover_background", false),
    )
}

#[test]
fn props_of_the_wrong_kind_are_ignored() {
    let mut app = app(wrong_kinds, ());
    frame(&mut app);
    app.pointer_move(5.0, 5.0);
    frame(&mut app);
}

fn wrong_kind_of_layout(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window).child(Element::new(Role::Group).prop("width", "all"))
}

/// A layout prop of the wrong kind is a mistake in the program, so debug builds say so, while a
/// finished program carries on without it.
#[test]
#[cfg_attr(debug_assertions, should_panic(expected = "must be a number"))]
fn layout_props_of_the_wrong_kind_only_stop_debug_builds() {
    let mut app = app(wrong_kind_of_layout, ());
    frame(&mut app);
}

fn bad_numbers(_: &mut Cx, value: &u64) -> Element {
    let value = f64::from_bits(*value);
    let mut element = Element::new(Role::Button).name("ABC");
    for name in [
        "width",
        "height",
        "min_width",
        "max_width",
        "min_height",
        "max_height",
        "padding_left",
        "padding_top",
        "padding_right",
        "padding_bottom",
        "gap",
        "grow",
        "shrink",
        "basis",
        "corner_radius",
        "border_width",
        "width_percent",
        "height_percent",
    ] {
        element = element.prop(name, value);
    }
    Element::new(Role::Window)
        .prop("gap", value)
        .child(element)
        .child(Element::new(Role::Text).name("after"))
}

#[test]
fn numbers_of_any_size_lay_out_and_draw() {
    for value in [
        0.0,
        -1.0,
        -1e30,
        1e30,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        f64::MIN_POSITIVE,
    ] {
        let mut app = app(bad_numbers, value.to_bits());
        frame(&mut app);
        app.pointer_move(5.0, 5.0);
        app.click(5.0, 5.0).unwrap();
        frame(&mut app);
    }
}

fn nested(_: &mut Cx, depth: &usize) -> Element {
    let mut node = Element::new(Role::Text).name("deep").into_node();
    for _ in 0..*depth {
        node = Element::new(Role::Group).child(node).into_node();
    }
    Element::new(Role::Window).child(node)
}

/// How deep to test, in every build: what goes down a tree level by level does it with a list of where it's got to, or, as
/// layout and drawing are many calls a level, with `rsui_core::stack::deep`, so even a debug build's big frames fit.
const DEEP: usize = 1_000;

#[test]
fn a_tree_deeper_than_any_real_one_works_on_a_normal_stack() {
    on_main_sized_stack(|| {
        let depth = std::env::var("RSUI_TEST_DEPTH")
            .ok()
            .and_then(|depth| depth.parse().ok())
            .unwrap_or(DEEP);
        let mut app = app(nested, depth);
        app.update();
        app.scene();
        app.pointer_move(5.0, 5.0);
        app.click(5.0, 5.0).unwrap();
        app.key(key(KeyCode::Tab)).unwrap();
        app.scene();
    });
}

fn wide(_: &mut Cx, count: &usize) -> Element {
    Element::new(Role::Window)
        .props(style!(Style::new()))
        .children((0..*count).map(|index| {
            Element::new(Role::Group)
                .key(index)
                .props(style!(Style::new().height(1.0)))
                .into_node()
        }))
}

#[test]
fn a_very_wide_tree_stays_quick() {
    let started = Instant::now();
    let mut app = app(wide, 200_000usize);
    app.update();
    app.scene();
    app.pointer_move(5.0, 50.0);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
}

fn empty(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window)
}

#[test]
fn an_empty_window_handles_everything() {
    let mut app = app(empty, ());
    frame(&mut app);
    app.pointer_move(1.0, 1.0);
    app.pointer_down(1.0, 1.0, 1);
    app.pointer_up(1.0, 1.0).unwrap();
    app.click(1.0, 1.0).unwrap();
    app.scroll(1.0, 1.0, 5).unwrap();
    app.pointer_leave();
    app.cancel_press();
    for code in [
        KeyCode::Tab,
        KeyCode::Enter,
        KeyCode::Space,
        KeyCode::Escape,
        KeyCode::Left,
        KeyCode::Backspace,
    ] {
        app.key(key(code)).unwrap();
    }
    app.type_text("nothing to type into").unwrap();
    app.blink();
    app.set_active(false);
    app.set_active(true);
    app.forget_caches();
    frame(&mut app);
}

#[test]
fn pointers_can_be_anywhere() {
    let mut app = app(label, "Some text");
    frame(&mut app);
    for (x, y) in [
        (-1.0, -1.0),
        (-32768.0, 32767.0),
        (1e30, 1e30),
        (f32::NAN, f32::NAN),
        (f32::INFINITY, f32::NEG_INFINITY),
    ] {
        app.pointer_move(x, y);
        app.pointer_down(x, y, 2);
        app.pointer_move(x, y);
        app.pointer_up(x, y).unwrap();
        app.click(x, y).unwrap();
        app.scroll(x, y, 3).unwrap();
    }
    frame(&mut app);
}

fn strange_text(_: &mut Cx, _: &()) -> Element {
    let strange = [
        "\0\0\0",
        "\u{FFFD}\u{FEFF}\u{200B}\u{200D}",
        "مرحبا بالعالم",
        "שלום",
        "e\u{0301}\u{0302}\u{0303}\u{0304}",
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
        "\r\r\r\n\n\n\t\t\t",
        "   ",
        "日本語のテキスト",
        "\u{10FFFF}",
        "a\u{0300}\u{034F}\u{2060}b",
    ];
    Element::new(Role::Window).children(strange.iter().map(|text| {
        Element::new(Role::Text)
            .name(*text)
            .props(style!(Style::new().width(30.0)))
            .into_node()
    }))
}

#[test]
fn any_text_can_be_shown() {
    let mut app = app(strange_text, ());
    frame(&mut app);
}

fn one_input(cx: &mut Cx, start: &&'static str) -> Element {
    let value = cx.state(|| Text::from(*start));
    let input = text_input(cx, "only", "Only", "hint", value);
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start)))
        .child(input)
}

#[test]
fn an_input_can_be_edited_whatever_it_holds() {
    for start in [
        "",
        "\n\n\n",
        "line one\nline two\r\nline three",
        "\0\0",
        "e\u{0301}\u{1F468}\u{200D}\u{1F469}",
        "مرحبا",
    ] {
        let mut app = app(one_input, start);
        frame(&mut app);
        app.key(key(KeyCode::Tab)).unwrap();
        app.scene();
        for code in [
            KeyCode::End,
            KeyCode::Left,
            KeyCode::Backspace,
            KeyCode::Delete,
            KeyCode::Home,
            KeyCode::Right,
            KeyCode::Character('a'),
        ] {
            app.key(key(code)).unwrap();
            app.update();
            app.scene();
        }
        app.key(key(KeyCode::Character('a')).ctrl()).unwrap();
        app.type_text("\u{1F600}").unwrap();
        app.update();
        app.scene();
        app.pointer_down(50.0, 10.0, 2);
        app.pointer_move(1000.0, 10.0);
        app.pointer_move(-1000.0, 10.0);
        app.pointer_up(-1000.0, 10.0).unwrap();
        app.key(key(KeyCode::Character('z')).ctrl()).unwrap();
        app.update();
        app.scene();
    }
}

fn no_room(cx: &mut Cx, _: &()) -> Element {
    let value = cx.state(|| Text::from("ABC"));
    let input = text_input(cx, "tight", "Tight", "", value);
    let input = Element {
        props: input
            .props
            .merged(&Style::new().width(0.0).height(0.0).into()),
        ..input
    };
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start)))
        .child(input)
}

#[test]
fn an_input_with_no_room_still_works() {
    let mut app = app(no_room, ());
    frame(&mut app);
    app.key(key(KeyCode::Tab)).unwrap();
    app.type_text("DD").unwrap();
    app.update();
    frame(&mut app);
}

/// Changes what's on screen from under whatever's happening, when keys are pressed.
fn vanishing(cx: &mut Cx, _: &()) -> Element {
    let step = cx.state(|| 0);
    let value = cx.state(|| Text::from("a long value here"));
    let input = text_input(cx, "input", "Input", "", value);
    let button = Element::new(Role::Button)
        .key("button")
        .name("Press")
        .props(style!(
            Style::new().width(50.0).height(20.0),
            Paint::new()
                .background(Color::hex(0xFF0000))
                .pressed(Color::hex(0x00FF00)),
        ));
    let button = cx.on(button, ActionKind::Click, move |_, _| {});
    let mut root = Element::new(Role::Window)
        .key("root")
        .props(style!(Style::new().align(Align::Start)));
    root = match cx.get(step) {
        0 => root.child(input).child(button),
        // The value got much shorter, and the input moved.
        1 => root.child(button).child(input),
        _ => root,
    };
    cx.on(root, ActionKind::KeyPress, move |cx, input| match input {
        Some(PropValue::Str(text)) if text.as_str() == "ctrl+1" => cx.set(step, 1),
        Some(PropValue::Str(text)) if text.as_str() == "ctrl+2" => cx.set(step, 2),
        Some(PropValue::Str(text)) if text.as_str() == "ctrl+3" => {
            cx.set(value, Text::from("x"));
        }
        _ => {}
    })
}

#[test]
fn things_can_disappear_during_any_action() {
    let press = |app: &mut App, character| {
        app.key(key(KeyCode::Character(character)).ctrl()).unwrap();
        app.update();
    };
    // Focus, with a caret far along, then the value shrinks.
    let mut app = app(vanishing, ());
    frame(&mut app);
    app.key(key(KeyCode::Tab)).unwrap();
    app.key(key(KeyCode::End)).unwrap();
    app.scene();
    press(&mut app, '3');
    app.scene();
    app.type_text("y").unwrap();
    app.update();
    app.scene();

    // The focused input moves, then is removed, with a drag still going on over it.
    app.pointer_down(10.0, 10.0, 1);
    app.pointer_move(30.0, 10.0);
    press(&mut app, '1');
    app.pointer_move(40.0, 10.0);
    app.scene();
    press(&mut app, '2');
    assert_eq!(app.focused(), None);
    app.pointer_move(50.0, 10.0);
    app.pointer_up(50.0, 10.0).unwrap();
    app.scene();

    // Nothing can have focus now.
    app.key(key(KeyCode::Tab)).unwrap();
    app.type_text("z").unwrap();
    app.key(key(KeyCode::Backspace)).unwrap();
    app.scene();
}

#[test]
fn a_press_can_outlive_what_was_pressed() {
    let mut app = app(vanishing, ());
    frame(&mut app);
    // The button is the second thing, below the input.
    let below_input = 40.0;
    app.pointer_down(10.0, below_input, 1);
    app.key(key(KeyCode::Character('2')).ctrl()).unwrap();
    app.update();
    app.scene();
    app.pointer_up(10.0, below_input).unwrap();
    app.update();
    app.scene();
}

fn long_list(cx: &mut Cx, count: &u64) -> Element {
    let list = cx.virtual_list(
        "rows",
        ListWindow {
            len: *count as usize,
            visible: 5,
            overscan: 1,
            reveal: None,
        },
        |cx, index| {
            cx.keyed(
                index,
                |_: &mut Cx, index: &usize| Element::new(Role::ListItem).name(text!("{index}")),
                index,
            )
        },
    );
    let props = list
        .props
        .merged(&Style::new().row_height(20.0).flex(1.0).into());
    Element::new(Role::Window).child(Element { props, ..list })
}

#[test]
fn a_list_can_be_scrolled_any_distance() {
    for count in [0u64, 1, 5, 1_000_000] {
        let mut app = app(long_list, count);
        frame(&mut app);
        for rows in [
            i64::MAX,
            i64::MAX,
            i64::MIN,
            i64::MIN,
            1,
            -1,
            0,
            i64::MAX - 1,
            i64::MIN + 1,
        ] {
            app.scroll(10.0, 10.0, rows).unwrap();
            app.update();
            app.scene();
        }
    }
}

fn bad_rows(_: &mut Cx, height: &u64) -> Element {
    let height = f64::from_bits(*height);
    Element::new(Role::Window).child(
        Element::new(Role::List)
            .prop("row_height", height)
            .prop("count", 1000i64)
            .prop("first_built", 0i64)
            .child(Element::new(Role::ListItem).name("row")),
    )
}

#[test]
fn a_list_with_rows_of_any_height_lays_out() {
    for height in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e30, f64::MIN_POSITIVE] {
        let mut app = app(bad_rows, height.to_bits());
        frame(&mut app);
    }
}

fn many_timers(cx: &mut Cx, count: &usize) -> Element {
    let fired = cx.state(|| 0usize);
    let count = *count;
    cx.effect((), move |cx| {
        for index in 0..count {
            cx.after(Duration::from_millis((index % 50) as u64), move |cx| {
                cx.update(fired, |fired| *fired += 1);
            });
        }
    });
    Element::new(Role::Window).name(text!("{}", cx.get(fired)))
}

#[test]
fn thousands_of_timers_are_fine() {
    let started = Instant::now();
    let mut app = app(many_timers, 20_000usize);
    let (_, finished) = app.runtime_mut().settle(Duration::from_secs(30));
    assert!(finished, "they all fired");
    assert_eq!(app.runtime().root().name, "20000");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn timers_for_something_gone_do_nothing() {
    let app = app(many_timers, 100usize);
    drop(app);
    std::thread::sleep(Duration::from_millis(100));
}

#[test]
fn a_huge_paste_works_and_stays_quick() {
    #[derive(Default)]
    struct Huge;
    impl rsui::Clipboard for Huge {
        fn read(&mut self) -> Option<String> {
            Some("pasted text ".repeat(1 << 17))
        }
        fn write(&mut self, _: &str) {}
    }
    let mut app = app(one_input, "");
    app.set_clipboard(Huge);
    frame(&mut app);
    app.key(key(KeyCode::Tab)).unwrap();
    let started = Instant::now();
    app.key(key(KeyCode::Character('v')).ctrl()).unwrap();
    app.update();
    app.scene();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    app.key(key(KeyCode::Character('z')).ctrl()).unwrap();
    app.update();
    app.scene();
    assert!(app.outline().contains("value=\"\""), "undone");
}

#[test]
fn rendering_survives_the_atlas_filling_up() {
    fn many_sizes(_: &mut Cx, _: &()) -> Element {
        Element::new(Role::Window).children((0..3_000).map(|index| {
            Element::new(Role::Text)
                .name("ABCD")
                .prop("font_size", 60.0 + f64::from(index) * 0.1)
                .props(style!(Style::new().height(0.001)))
                .into_node()
        }))
    }
    let mut app = app(many_sizes, ());
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    for _ in 0..3 {
        app.update();
        app.render(&mut renderer).unwrap();
    }
}

#[test]
fn working_out_a_scene_without_drawing_it_loses_nothing() {
    let mut drawn = app(label, "ABCD");
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    drawn.update();
    drawn.render(&mut renderer).unwrap();
    let expected = renderer.read_pixels().unwrap();

    let mut app = app(label, "ABCD");
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    app.update();
    // The glyphs this finds are only worked out once.
    app.scene();
    app.scene();
    app.render(&mut renderer).unwrap();
    assert_eq!(renderer.read_pixels().unwrap(), expected);
}

#[test]
fn a_lost_renderer_can_be_replaced() {
    let mut app = app(label, "ABCD");
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    app.update();
    app.render(&mut renderer).unwrap();
    let before = renderer.read_pixels().unwrap();

    drop(renderer);
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    app.renderer_replaced();
    app.update();
    app.render(&mut renderer).unwrap();
    assert_eq!(renderer.read_pixels().unwrap(), before, "the same picture");
}

#[test]
fn a_replaced_renderer_gets_every_glyph_again_even_with_no_change() {
    let mut app = app(label, "ABCD");
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    app.render(&mut renderer).unwrap();
    assert!(app.scene().uploads.is_empty(), "already uploaded");
    app.renderer_replaced();
    assert!(!app.scene().uploads.is_empty(), "uploaded again");
}

#[test]
fn losing_the_keyboard_with_nothing_focused_draws_nothing() {
    let mut app = app(label, "Some text");
    app.update();
    app.scene();
    app.set_active(false);
    assert!(
        matches!(app.update(), rsui::Frame::Unchanged),
        "nothing on screen depends on it, and waking the GPU from deep idle is expensive"
    );
}
