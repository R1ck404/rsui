//! The keyboard and pointer, from the event to the state and the pixels, drawn offscreen so no
//! window opens.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use rsui::prelude::*;
use rsui::render::Renderer;
use rsui::text::TextEngine;
use rsui::{App, Cursor, Frame, KeyCode, KeyPress};
use support::plain_font;

const SIZE: (u32, u32) = (300, 200);
const RED: Color = Color::hex(0xFF0000);
const GREEN: Color = Color::hex(0x00FF00);
const BLUE: Color = Color::hex(0x0000FF);
const WHITE_PIXEL: [u8; 4] = [255, 255, 255, 255];
const RED_PIXEL: [u8; 4] = [255, 0, 0, 255];
const GREEN_PIXEL: [u8; 4] = [0, 255, 0, 255];
const BLUE_PIXEL: [u8; 4] = [0, 0, 255, 255];

const fn key(code: KeyCode) -> KeyPress {
    KeyPress::new(code)
}

/// Fixed-size elements, so that positions in these tests are easy to work out.
fn input(cx: &mut Cx, name: &str, value: State<Text>) -> Element {
    // 10 pixels per em, so `A` is 6 wide. See the test font.
    let element = Element::new(Role::TextInput)
        .key(name)
        .name(name)
        .props(
            rsui::core::Props::from(Style::new().width(100.0).height(20.0).padding(2.0))
                .merged(&Paint::new().background(Color::WHITE).into()),
        )
        .prop("font_size", 10.0)
        .prop("value", cx.get(value));
    cx.on(element, ActionKind::SetValue, move |cx, input| {
        if let Some(PropValue::Str(text)) = input {
            cx.set(value, Text::clone(text));
        }
    })
}

/// `first` at (0, 0), `second` at (0, 20), then a button at (0, 40), all on the left.
fn form(cx: &mut Cx, _: &()) -> Element {
    let first = cx.state(Text::default);
    let second = cx.state(|| Text::from("AB"));
    let clicks = cx.state(|| 0);
    let shortcuts = cx.state(Text::default);

    let first = input(cx, "first", first);
    let second = input(cx, "second", second);
    let button = Element::new(Role::Button)
        .key("go")
        .name(text!("{}", cx.get(clicks)))
        .props(
            rsui::core::Props::from(Style::new().width(50.0).height(20.0)).merged(
                &Paint::new()
                    .background(BLUE)
                    .hover(RED)
                    .pressed(GREEN)
                    .into(),
            ),
        )
        .prop("font_size", 10.0);
    let button = cx.on(button, ActionKind::Click, move |cx, _| {
        cx.update(clicks, |clicks| *clicks += 1);
    });
    let root = Element::new(Role::Window)
        .key("root")
        .name(cx.get(shortcuts))
        .props(style!(Style::new().align(Align::Start)))
        .child(first)
        .child(second)
        .child(button);
    cx.on(root, ActionKind::KeyPress, move |cx, input| {
        if let Some(PropValue::Str(text)) = input {
            cx.set(shortcuts, Text::clone(text));
        }
    })
}

struct Screen {
    app: App,
    renderer: Renderer,
}

impl Screen {
    fn new(render: fn(&mut Cx, &()) -> Element) -> Self {
        Self {
            app: App::new(render, (), TextEngine::new(plain_font()), SIZE, 1.0),
            renderer: Renderer::offscreen(SIZE).expect("the software GPU is always there"),
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

    fn press(&mut self, code: KeyCode) {
        self.send(key(code));
    }

    fn send(&mut self, press: KeyPress) {
        self.app.key(press).expect("the key goes through");
        self.app.update();
    }

    fn type_text(&mut self, text: &str) {
        self.app.type_text(text).expect("the text goes through");
        self.app.update();
    }

    fn outline(&self) -> String {
        self.app.outline()
    }

    /// The value of the input with this key.
    fn value(&self, name: &str) -> String {
        let outline = self.outline();
        let line = outline
            .lines()
            .find(|line| line.contains(&format!("#{name} ")))
            .unwrap_or_else(|| panic!("{name} is there: {outline}"));
        let start = line.find("value=\"").expect("a value is shown") + 7;
        let end = line[start..].find('"').expect("it ends") + start;
        line[start..end].to_owned()
    }

    fn click_at(&mut self, x: f32, y: f32) {
        self.app.pointer_down(x, y, 1);
        self.app.pointer_up(x, y).expect("the click goes through");
        self.app.update();
    }
}

const FIRST: &[u32] = &[0];
const SECOND: &[u32] = &[1];
const BUTTON: &[u32] = &[2];

#[test]
fn tab_goes_through_everything_in_order_and_comes_back_around() {
    let mut screen = Screen::new(form);
    assert_eq!(screen.app.focused(), None);
    for expected in [FIRST, SECOND, BUTTON, FIRST] {
        screen.press(KeyCode::Tab);
        assert_eq!(screen.app.focused(), Some(expected));
    }
    for expected in [BUTTON, SECOND, FIRST] {
        screen.send(key(KeyCode::Tab).shift());
        assert_eq!(screen.app.focused(), Some(expected));
    }
}

#[test]
fn typing_changes_the_value_through_the_component() {
    let mut screen = Screen::new(form);
    screen.press(KeyCode::Tab);
    screen.type_text("A");
    screen.type_text("B");
    screen.type_text("C");
    assert_eq!(screen.value("first"), "ABC");
    assert_eq!(screen.value("second"), "AB", "the other one is left alone");

    screen.press(KeyCode::Left);
    screen.press(KeyCode::Backspace);
    assert_eq!(screen.value("first"), "AC");
    screen.press(KeyCode::Delete);
    assert_eq!(screen.value("first"), "A");
}

#[test]
fn control_characters_never_become_text() {
    let mut screen = Screen::new(form);
    screen.press(KeyCode::Tab);
    screen.type_text("\u{8}");
    screen.type_text("\r");
    screen.type_text("\t");
    screen.type_text("\u{1}");
    assert_eq!(screen.value("first"), "");
}

#[test]
fn the_control_character_after_ctrl_a_leaves_what_is_selected() {
    let mut screen = Screen::new(form);
    screen.press(KeyCode::Tab);
    screen.press(KeyCode::Tab);
    // Tabbing in selects all of it, as Ctrl+A does, and Ctrl+A then types U+0001.
    screen.type_text("\u{1}");
    assert_eq!(screen.value("second"), "AB");
}

#[test]
fn tabbing_to_an_input_selects_what_it_holds_so_typing_replaces_it() {
    let mut screen = Screen::new(form);
    screen.press(KeyCode::Tab);
    screen.press(KeyCode::Tab);
    screen.type_text("D");
    assert_eq!(screen.value("second"), "D");
}

#[test]
fn copy_cut_and_paste_use_the_clipboard() {
    let mut screen = Screen::new(form);
    screen.press(KeyCode::Tab);
    screen.press(KeyCode::Tab);
    let (ctrl_c, ctrl_x, ctrl_v) = (
        key(KeyCode::Character('c')).ctrl(),
        key(KeyCode::Character('x')).ctrl(),
        key(KeyCode::Character('v')).ctrl(),
    );

    screen.send(ctrl_x);
    assert_eq!(screen.value("second"), "");
    screen.send(ctrl_v);
    screen.send(ctrl_v);
    assert_eq!(screen.value("second"), "ABAB");

    screen.send(key(KeyCode::Character('a')).ctrl());
    screen.send(ctrl_c);
    assert_eq!(screen.value("second"), "ABAB", "copying changes nothing");
    screen.press(KeyCode::End);
    screen.send(ctrl_v);
    assert_eq!(screen.value("second"), "ABABABAB");
}

#[test]
fn line_breaks_become_spaces_when_pasted() {
    #[derive(Default)]
    struct Fixed;
    impl rsui::Clipboard for Fixed {
        fn read(&mut self) -> Option<String> {
            Some("A\r\nB\nC\tD".to_owned())
        }
        fn write(&mut self, _: &str) {}
    }
    let mut screen = Screen::new(form);
    screen.app.set_clipboard(Fixed);
    screen.press(KeyCode::Tab);
    screen.send(key(KeyCode::Character('v')).ctrl());
    assert_eq!(screen.value("first"), "A B C D");
}

#[test]
fn undo_and_redo_go_through_the_component_too() {
    let mut screen = Screen::new(form);
    screen.press(KeyCode::Tab);
    screen.type_text("A");
    screen.type_text("B");
    screen.send(key(KeyCode::Character('z')).ctrl());
    assert_eq!(screen.value("first"), "");
    screen.send(key(KeyCode::Character('y')).ctrl());
    assert_eq!(screen.value("first"), "AB");
    screen.send(key(KeyCode::Character('z')).ctrl());
    screen.send(key(KeyCode::Character('z')).ctrl().shift());
    assert_eq!(screen.value("first"), "AB");
}

/// An input that only takes capitals, and refuses the letter `B`.
fn picky(cx: &mut Cx, _: &()) -> Element {
    let value = cx.state(Text::default);
    let element = Element::new(Role::TextInput)
        .key("picky")
        .props(rsui::core::Props::from(
            Style::new().width(100.0).height(20.0),
        ))
        .prop("value", cx.get(value));
    let element = cx.on(element, ActionKind::SetValue, move |cx, input| {
        if let Some(PropValue::Str(text)) = input {
            if text.contains('B') {
                cx.reject("no B");
            } else {
                cx.set(value, text.to_uppercase());
            }
        }
    });
    Element::new(Role::Window).child(element)
}

#[test]
fn the_component_has_the_last_word_on_the_value() {
    let mut screen = Screen::new(picky);
    screen.press(KeyCode::Tab);
    screen.type_text("a");
    assert_eq!(screen.value("picky"), "A", "changed by the handler");

    assert!(matches!(
        screen.app.type_text("B"),
        Err(rsui::runtime::DispatchError::Rejected(_))
    ));
    screen.app.update();
    assert_eq!(screen.value("picky"), "A", "refused by the handler");

    screen.type_text("c");
    assert_eq!(
        screen.value("picky"),
        "AC",
        "the caret stayed in the right place"
    );
}

fn read_only(cx: &mut Cx, _: &()) -> Element {
    let _ = cx;
    Element::new(Role::Window).child(
        Element::new(Role::TextInput)
            .key("fixed")
            .props(rsui::core::Props::from(
                Style::new().width(100.0).height(20.0),
            ))
            .prop("value", "AB"),
    )
}

#[test]
fn an_input_with_no_handler_can_be_read_but_not_changed() {
    let mut screen = Screen::new(read_only);
    screen.press(KeyCode::Tab);
    assert_eq!(screen.app.focused(), Some(&[0][..]));
    screen.type_text("D");
    screen.press(KeyCode::Backspace);
    assert_eq!(screen.value("fixed"), "AB");
    // It can still be selected and copied.
    screen.send(key(KeyCode::Character('a')).ctrl());
    screen.send(key(KeyCode::Character('c')).ctrl());
    screen.app.focus_on(&[]);
    assert_eq!(
        screen.app.focused(),
        None,
        "the window can't take the keyboard"
    );
}

#[test]
fn enter_and_space_click_a_focused_button_once_per_press() {
    let mut screen = Screen::new(form);
    screen.app.focus_on(BUTTON);
    screen.press(KeyCode::Enter);
    assert!(
        screen.outline().contains("button \"1\""),
        "{}",
        screen.outline()
    );
    screen.press(KeyCode::Space);
    assert!(
        screen.outline().contains("button \"2\""),
        "{}",
        screen.outline()
    );
    let mut held = key(KeyCode::Space);
    held.repeat = true;
    screen.send(held);
    assert!(
        screen.outline().contains("button \"2\""),
        "{}",
        screen.outline()
    );
}

#[test]
fn enter_in_an_input_is_not_typing() {
    let mut screen = Screen::new(form);
    screen.press(KeyCode::Tab);
    screen.type_text("A");
    screen.press(KeyCode::Enter);
    assert_eq!(screen.value("first"), "A");
    assert!(
        screen.outline().contains("window \"enter\""),
        "{}",
        screen.outline()
    );
}

#[test]
fn other_keys_reach_the_nearest_node_that_wants_them_described_in_words() {
    let mut screen = Screen::new(form);
    screen.send(key(KeyCode::Character('s')).ctrl());
    assert!(
        screen.outline().contains("window \"ctrl+s\""),
        "with nothing focused"
    );

    screen.press(KeyCode::Tab);
    screen.send(key(KeyCode::PageDown).ctrl().shift());
    assert!(
        screen.outline().contains("window \"ctrl+shift+page_down\""),
        "from inside an input: {}",
        screen.outline()
    );
    screen.send(key(KeyCode::Character('q')).ctrl());
    assert!(screen.outline().contains("window \"ctrl+q\""));
}

fn no_shortcuts(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window).child(Element::new(Role::TextInput).props(rsui::core::Props::from(
        Style::new().width(100.0).height(20.0),
    )))
}

#[test]
fn escape_lets_go_of_the_keyboard_if_nothing_wants_it() {
    let mut screen = Screen::new(no_shortcuts);
    screen.press(KeyCode::Tab);
    assert!(screen.app.focused().is_some());
    screen.press(KeyCode::Escape);
    assert_eq!(screen.app.focused(), None);
}

/// Two inputs that can be swapped or removed by keys pressed anywhere.
fn shifting(cx: &mut Cx, _: &()) -> Element {
    let swapped = cx.state(|| false);
    let hidden = cx.state(|| false);
    let value = cx.state(Text::default);
    let a = input(cx, "a", value);
    let b = Element::new(Role::Button)
        .key("b")
        .props(rsui::core::Props::from(
            Style::new().width(10.0).height(10.0),
        ))
        .action(ActionKind::Toggle);
    let mut root = Element::new(Role::Window).key("root");
    if !cx.get(hidden) {
        root = if cx.get(swapped) {
            root.child(b).child(a)
        } else {
            root.child(a).child(b)
        };
    }
    cx.on(root, ActionKind::KeyPress, move |cx, input| match input {
        Some(PropValue::Str(text)) if text.as_str() == "ctrl+1" => cx.set(swapped, true),
        Some(PropValue::Str(text)) if text.as_str() == "ctrl+2" => cx.set(hidden, true),
        _ => {}
    })
}

#[test]
fn focus_stays_on_the_same_node_when_it_moves_and_lets_go_when_it_goes() {
    let mut screen = Screen::new(shifting);
    screen.press(KeyCode::Tab);
    assert_eq!(screen.app.focused(), Some(&[0][..]));
    screen.type_text("A");

    screen.send(key(KeyCode::Character('1')).ctrl());
    assert_eq!(
        screen.app.focused(),
        Some(&[1][..]),
        "it followed the input"
    );
    screen.type_text("B");
    assert_eq!(screen.value("a"), "AB", "and it's still being edited");

    screen.send(key(KeyCode::Character('2')).ctrl());
    assert_eq!(screen.app.focused(), None);
    screen.type_text("C");
    screen.pixels();
}

fn told(cx: &mut Cx, _: &()) -> Element {
    let focused = cx.state(|| false);
    let element = Element::new(Role::Group)
        .key("group")
        .name(text!("{}", cx.get(focused)))
        .props(rsui::core::Props::from(
            Style::new().width(10.0).height(10.0),
        ));
    let element = cx.on(element, ActionKind::Focus, move |cx, input| {
        cx.set(focused, matches!(input, Some(PropValue::Bool(true))));
    });
    Element::new(Role::Window).child(element).child(
        Element::new(Role::Button)
            .key("other")
            .props(rsui::core::Props::from(
                Style::new().width(10.0).height(10.0),
            ))
            .action(ActionKind::Toggle),
    )
}

#[test]
fn a_node_can_ask_to_be_told_about_focus() {
    let mut screen = Screen::new(told);
    screen.press(KeyCode::Tab);
    assert!(
        screen.outline().contains("\"true\""),
        "{}",
        screen.outline()
    );
    screen.press(KeyCode::Tab);
    assert!(
        screen.outline().contains("\"false\""),
        "{}",
        screen.outline()
    );
}

#[test]
fn a_click_needs_the_button_to_come_up_on_the_same_node() {
    let mut screen = Screen::new(form);
    screen.click_at(10.0, 50.0);
    assert!(
        screen.outline().contains("button \"1\""),
        "{}",
        screen.outline()
    );

    screen.app.pointer_down(10.0, 50.0, 1);
    screen.app.pointer_up(200.0, 150.0).unwrap();
    screen.app.update();
    assert!(screen.outline().contains("button \"1\""), "dragged off");

    screen.app.pointer_down(200.0, 150.0, 1);
    screen.app.pointer_up(10.0, 50.0).unwrap();
    screen.app.update();
    assert!(screen.outline().contains("button \"1\""), "dragged on");
}

#[test]
fn clicking_moves_the_keyboard_or_lets_go_of_it() {
    let mut screen = Screen::new(form);
    screen.click_at(10.0, 30.0);
    assert_eq!(screen.app.focused(), Some(SECOND));
    screen.click_at(10.0, 50.0);
    assert_eq!(screen.app.focused(), Some(BUTTON));
    screen.click_at(200.0, 150.0);
    assert_eq!(screen.app.focused(), None);
}

#[test]
fn hovering_and_pressing_change_the_pixels_without_rendering_anything() {
    let mut screen = Screen::new(form);
    assert_eq!(screen.pixel(40, 50), BLUE_PIXEL);
    let renders = screen.app.runtime().renders();

    screen.app.pointer_move(40.0, 50.0);
    assert_eq!(screen.app.update(), Frame::Needed);
    assert_eq!(screen.pixel(40, 50), RED_PIXEL, "hovering");
    assert_eq!(screen.app.update(), Frame::Unchanged);

    screen.app.pointer_move(41.0, 51.0);
    assert_eq!(
        screen.app.update(),
        Frame::Unchanged,
        "moving within it needs no new frame"
    );

    screen.app.pointer_down(40.0, 50.0, 1);
    assert_eq!(screen.pixel(40, 50), GREEN_PIXEL, "pressing");

    screen.app.cancel_press();
    assert_eq!(screen.pixel(40, 50), RED_PIXEL, "no longer pressing");

    screen.app.pointer_leave();
    assert_eq!(screen.app.update(), Frame::Needed);
    assert_eq!(screen.pixel(40, 50), BLUE_PIXEL, "gone");
    assert_eq!(
        screen.app.runtime().renders(),
        renders,
        "no component rendered"
    );
}

#[test]
fn hover_follows_things_that_move_under_a_still_pointer() {
    fn moving(cx: &mut Cx, _: &()) -> Element {
        let moved = cx.state(|| false);
        let square = |key: &'static str, hover: Color| {
            Element::new(Role::Group).key(key).props(
                rsui::core::Props::from(Style::new().width(50.0).height(50.0))
                    .merged(&Paint::new().background(BLUE).hover(hover).into()),
            )
        };
        let (first, second) = (square("first", RED), square("second", GREEN));
        let root = Element::new(Role::Window)
            .key("root")
            .props(style!(Style::new().align(Align::Start)));
        let root = if cx.get(moved) {
            root.child(second).child(first)
        } else {
            root.child(first).child(second)
        };
        cx.on(root, ActionKind::KeyPress, move |cx, _| cx.set(moved, true))
    }
    let mut screen = Screen::new(moving);
    screen.app.pointer_move(10.0, 10.0);
    assert_eq!(screen.pixel(10, 10), RED_PIXEL);
    screen.press(KeyCode::Enter);
    assert_eq!(screen.pixel(10, 10), GREEN_PIXEL);
}

#[test]
fn the_pointer_shows_what_clicking_would_do() {
    let mut screen = Screen::new(form);
    screen.pixels();
    screen.app.pointer_move(10.0, 10.0);
    assert_eq!(screen.app.cursor(), Cursor::Text);
    screen.app.pointer_move(10.0, 50.0);
    assert_eq!(screen.app.cursor(), Cursor::Hand);
    screen.app.pointer_move(200.0, 150.0);
    assert_eq!(screen.app.cursor(), Cursor::Arrow);
}

/// An input with `CCCC` in it, at the top left, whose text starts at x = 2. Each `C` is 8 wide.
fn four_wide(cx: &mut Cx, _: &()) -> Element {
    let value = cx.state(|| Text::from("CCCC"));
    let element = input(cx, "wide", value);
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start)))
        .child(element)
}

#[test]
fn clicking_in_an_input_puts_the_caret_at_the_nearest_character() {
    let mut screen = Screen::new(four_wide);
    screen.pixels();
    // Between the first and second, nearer the second.
    screen.click_at(2.0 + 9.0, 10.0);
    screen.type_text("A");
    assert_eq!(screen.value("wide"), "CACCC");
    // Beyond the end.
    screen.click_at(90.0, 10.0);
    screen.type_text("B");
    assert_eq!(screen.value("wide"), "CACCCB");
    // Before the start.
    screen.click_at(1.0, 10.0);
    screen.type_text("D");
    assert_eq!(screen.value("wide"), "DCACCCB");
}

#[test]
fn dragging_selects_and_a_double_click_selects_a_word() {
    let mut screen = Screen::new(four_wide);
    screen.pixels();
    screen.app.pointer_down(2.0 + 8.0, 10.0, 1);
    screen.app.pointer_move(2.0 + 24.0, 10.0);
    screen.app.pointer_up(2.0 + 24.0, 10.0).unwrap();
    screen.type_text("A");
    assert_eq!(screen.value("wide"), "CAC");

    screen.app.pointer_down(2.0 + 8.0, 10.0, 1);
    screen.app.pointer_up(2.0 + 8.0, 10.0).unwrap();
    screen.app.pointer_down(2.0 + 8.0, 10.0, 2);
    screen.app.pointer_up(2.0 + 8.0, 10.0).unwrap();
    screen.type_text("D");
    assert_eq!(screen.value("wide"), "D");
}

/// Whether every pixel in this column, down the height of the input's text, is nearly black.
/// The input's padding is 2 and its lines are 11 high.
fn caret_in_column(pixels: &[u8], x: usize) -> bool {
    (2..13).all(|y| {
        let at = (y * SIZE.0 as usize + x) * 4;
        pixels[at] < 80 && pixels[at + 1] < 80 && pixels[at + 2] < 80
    })
}

#[test]
fn the_caret_is_drawn_where_it_is_and_blinks_then_stays_on() {
    // An empty input, so the text can't be mistaken for the caret.
    let mut screen = Screen::new(form);
    screen.press(KeyCode::Tab);
    let pixels = screen.pixels();
    assert!(caret_in_column(&pixels, 2), "at the start of the text");

    assert!(screen.app.caret_blinking());
    screen.app.blink();
    assert_eq!(screen.app.update(), Frame::Needed);
    let pixels = screen.pixels();
    assert!(!caret_in_column(&pixels, 2), "blinked off");

    screen.press(KeyCode::Left);
    let pixels = screen.pixels();
    assert!(caret_in_column(&pixels, 2), "a key brings it straight back");

    let mut changes = 0;
    while screen.app.caret_blinking() {
        screen.app.blink();
        changes += 1;
        assert!(changes < 100, "it stops, so the window can go quiet");
    }
    let pixels = screen.pixels();
    assert!(caret_in_column(&pixels, 2), "it ends up on");
}

#[test]
fn a_blink_gives_the_scene_working_it_out_again_would() {
    // In the empty input the caret is all there is in what's cut off at its edges; in the other there's text with it.
    for keys in [&[KeyCode::Tab][..], &[KeyCode::Tab, KeyCode::Tab, KeyCode::End]] {
        let focused = |blinks: usize| {
            let mut app = App::new(form, (), TextEngine::new(plain_font()), SIZE, 1.0);
            for code in keys {
                app.key(key(*code)).unwrap();
                app.update();
            }
            for _ in 0..blinks {
                app.blink();
            }
            app
        };
        let mut blinking = focused(0);
        blinking.scene();
        assert!(blinking.caret_blinking(), "{keys:?}");
        for blinks in 1..6 {
            blinking.blink();
            assert_eq!(blinking.update(), Frame::Needed);
            let scene = blinking.scene();
            let (quads, batches) = (scene.quads.clone(), scene.batches.clone());
            // This one has never been drawn, so it has to work it all out.
            let mut fresh = focused(blinks);
            fresh.update();
            let scene = fresh.scene();
            assert_eq!(quads, scene.quads, "{keys:?}, after {blinks} blinks");
            assert_eq!(batches, scene.batches, "{keys:?}, after {blinks} blinks");
        }
    }
}

#[test]
fn a_selection_is_drawn_behind_the_text_and_hides_the_caret_blink() {
    let mut screen = Screen::new(four_wide);
    screen.press(KeyCode::Tab);
    assert!(!screen.app.caret_blinking(), "a selection doesn't blink");
    // Below the baseline, so there's no glyph in the way, but still inside the line.
    let selected = screen.pixel(2 + 16, 11);
    assert!(
        selected[2] > selected[0] && selected != WHITE_PIXEL,
        "the selection color shows under the text: {selected:?}"
    );
    screen.press(KeyCode::End);
    assert_eq!(screen.pixel(2 + 16, 11), WHITE_PIXEL, "gone");
}

#[test]
fn only_inputs_and_keyboard_focus_get_a_ring() {
    let mut screen = Screen::new(form);
    // A button sits at (0, 40), so the ring would be just below it.
    let below = (10, 61);
    let before = screen.pixel(below.0, below.1);

    screen.click_at(10.0, 50.0);
    assert_eq!(
        screen.pixel(below.0, below.1),
        before,
        "clicked, so no ring"
    );

    screen.press(KeyCode::Tab);
    screen.send(key(KeyCode::Tab).shift());
    assert_eq!(screen.app.focused(), Some(BUTTON));
    assert_ne!(screen.pixel(below.0, below.1), before, "reached with tab");

    screen.app.set_active(false);
    assert_eq!(
        screen.pixel(below.0, below.1),
        before,
        "the window lost the keyboard"
    );
    screen.app.set_active(true);
    assert_ne!(screen.pixel(below.0, below.1), before);
}

#[test]
fn long_text_slides_to_keep_the_caret_in_view_and_stays_inside_the_input() {
    let mut screen = Screen::new(four_wide);
    screen.press(KeyCode::Tab);
    screen.press(KeyCode::End);
    for _ in 0..30 {
        screen.type_text("C");
    }
    // The caret is at the input's far edge. Its text is 96 wide, inside 2 of padding.
    let pixels = screen.pixels();
    let found = (2..98).rev().find(|x| caret_in_column(&pixels, *x));
    assert!(found.is_some_and(|x| x >= 90), "at {found:?}");
    assert_eq!(
        screen.pixel(99, 8),
        WHITE_PIXEL,
        "text is cut off at the padding"
    );

    screen.press(KeyCode::Home);
    let pixels = screen.pixels();
    assert!(caret_in_column(&pixels, 2), "and back again");
}

#[test]
fn typing_only_lays_out_the_input() {
    let mut screen = Screen::new(form);
    screen.pixels();
    screen.press(KeyCode::Tab);
    screen.pixels();
    screen.type_text("A");
    screen.pixels();
    assert!(
        screen.app.layout().last_solved() <= 1,
        "solved {}",
        screen.app.layout().last_solved()
    );
}

#[test]
fn text_never_shows_outside_the_inside_of_its_input() {
    let mut screen = Screen::new(four_wide);
    screen.press(KeyCode::Tab);
    screen.press(KeyCode::End);
    for _ in 0..30 {
        screen.type_text("C");
    }
    const PAGE: [u8; 4] = [0xF6, 0xF8, 0xFA, 255];
    let check = |pixels: &[u8], columns: std::ops::Range<usize>, expected, side: &str| {
        for x in columns {
            for y in 2..13 {
                let at = (y * SIZE.0 as usize + x) * 4;
                assert_eq!(pixels[at..at + 4], expected, "{side}, at ({x}, {y})");
            }
        }
    };
    let pixels = screen.pixels();
    check(
        &pixels,
        0..2,
        WHITE_PIXEL,
        "slid along, past the left padding",
    );

    // Now the text runs far past the right. The ring takes up the two pixels after the input.
    screen.press(KeyCode::Home);
    let pixels = screen.pixels();
    check(&pixels, 98..100, WHITE_PIXEL, "the right padding");
    check(&pixels, 102..300, PAGE, "past the input");
}

/// Ignores any value with a `D` in it, without saying so.
fn quietly_picky(cx: &mut Cx, _: &()) -> Element {
    let value = cx.state(|| Text::from("AC"));
    let element = Element::new(Role::TextInput)
        .key("quiet")
        .props(rsui::core::Props::from(
            Style::new().width(100.0).height(20.0),
        ))
        .prop("value", cx.get(value));
    let element = cx.on(element, ActionKind::SetValue, move |cx, input| {
        if let Some(PropValue::Str(text)) = input {
            if !text.contains('D') {
                cx.set(value, Text::clone(text));
            }
        }
    });
    Element::new(Role::Window).child(element)
}

#[test]
fn a_change_that_was_quietly_ignored_leaves_the_caret_and_history_alone() {
    let mut screen = Screen::new(quietly_picky);
    screen.press(KeyCode::Tab);
    screen.press(KeyCode::Home);
    screen.press(KeyCode::Right);
    screen.type_text("D");
    assert_eq!(screen.value("quiet"), "AC");
    screen.type_text("B");
    assert_eq!(screen.value("quiet"), "ABC", "the caret hadn't moved");

    screen.send(key(KeyCode::Character('z')).ctrl());
    assert_eq!(screen.value("quiet"), "AC");
    screen.send(key(KeyCode::Character('z')).ctrl());
    assert_eq!(screen.value("quiet"), "AC", "there's nothing else to undo");
}

/// A gap that opens up above two squares when a key is pressed.
fn growing_gap(cx: &mut Cx, _: &()) -> Element {
    let open = cx.state(|| false);
    let height = if cx.get(open) { 100.0 } else { 0.0 };
    let square = |name: &'static str, hover: Color| {
        Element::new(Role::Group).key(name).props(
            rsui::core::Props::from(Style::new().width(50.0).height(50.0))
                .merged(&Paint::new().background(BLUE).hover(hover).into()),
        )
    };
    let root = Element::new(Role::Window)
        .key("root")
        .props(style!(Style::new().align(Align::Start)))
        .child(
            Element::new(Role::Group)
                .key("gap")
                .props(rsui::core::Props::from(Style::new().height(height))),
        )
        .child(square("first", RED))
        .child(square("second", GREEN));
    cx.on(root, ActionKind::KeyPress, move |cx, _| cx.set(open, true))
}

#[test]
fn hover_lets_go_of_what_moved_out_from_under_a_still_pointer() {
    let mut screen = Screen::new(growing_gap);
    screen.pixels();
    screen.app.pointer_move(10.0, 60.0);
    assert_eq!(screen.pixel(10, 60), GREEN_PIXEL, "over the second square");

    screen.press(KeyCode::Enter);
    assert_eq!(screen.pixel(10, 160), BLUE_PIXEL, "it moved away");
}

fn own_counter(cx: &mut Cx, _: &()) -> Element {
    let count = cx.state(|| 0);
    let button = Element::new(Role::Button)
        .key("add")
        .name(text!("count {}", cx.get(count)))
        .props(rsui::core::Props::from(
            Style::new().width(50.0).height(20.0),
        ));
    cx.on(button, ActionKind::Click, move |cx, _| {
        cx.update(count, |count| *count += 1);
    })
}

fn two_counters(cx: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start)))
        .child(cx.child(own_counter, ()))
        .child(cx.child(own_counter, ()))
}

#[test]
fn a_click_can_tell_apart_two_buttons_with_the_same_key() {
    let mut screen = Screen::new(two_counters);
    screen.pixels();
    screen.app.pointer_down(10.0, 10.0, 1);
    screen.app.pointer_up(10.0, 30.0).unwrap();
    screen.app.update();
    assert!(
        !screen.outline().contains("count 1"),
        "it came up on a different button: {}",
        screen.outline()
    );

    screen.click_at(10.0, 30.0);
    assert!(screen.outline().contains("count 1"), "{}", screen.outline());
}

#[test]
fn the_outline_says_what_an_input_holds_without_its_styling() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        let value = cx.state(|| Text::from("Hi"));
        text_input(cx, "name", "Your name", "Type here", value)
    }
    let screen = Screen::new(app);
    assert_eq!(
        screen.outline(),
        "text_input \"Your name\" #name placeholder=\"Type here\" value=\"Hi\" [set_value] @/\n"
    );
}

/// An input holding `value`, at the top left. In the test font, each letter it lacks is as wide as
/// its missing glyph.
fn holding(value: &'static str, cx: &mut Cx) -> Element {
    let value = cx.state(|| Text::from(value));
    let element = input(cx, "held", value);
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start)))
        .child(element)
}

#[test]
fn the_arrow_keys_go_across_the_screen_in_right_to_left_text() {
    // `ab` and then Hebrew alef and bet: on screen, `a b bet alef`.
    fn render(cx: &mut Cx, _: &()) -> Element {
        holding("AB\u{05D0}\u{05D1}", cx)
    }
    let mut screen = Screen::new(render);
    screen.press(KeyCode::Tab);
    screen.press(KeyCode::Home);
    // Past `A` and `B`, and to the left of bet, which ends the text.
    for _ in 0..2 {
        screen.press(KeyCode::Right);
    }
    screen.type_text("X");
    assert_eq!(screen.value("held"), "AB\u{05D0}\u{05D1}X");

    // The other way goes back to the left of the Latin letters.
    let mut screen = Screen::new(render);
    screen.press(KeyCode::Tab);
    screen.press(KeyCode::Home);
    screen.press(KeyCode::Right);
    screen.press(KeyCode::Left);
    screen.type_text("Y");
    assert_eq!(screen.value("held"), "YAB\u{05D0}\u{05D1}");
}

#[test]
fn selecting_with_the_keyboard_works_in_right_to_left_text() {
    fn render(cx: &mut Cx, _: &()) -> Element {
        holding("\u{05D0}\u{05D1}\u{05D2}", cx)
    }
    let mut screen = Screen::new(render);
    screen.press(KeyCode::Tab);
    screen.press(KeyCode::Home);
    // The start of right-to-left text is on the right, so going left goes through it.
    screen.send(key(KeyCode::Left).shift());
    screen.press(KeyCode::Delete);
    assert_eq!(screen.value("held"), "\u{05D1}\u{05D2}");
    // Going left from the start, with nothing selected.
    screen.press(KeyCode::Left);
    screen.press(KeyCode::Backspace);
    assert_eq!(screen.value("held"), "\u{05D2}");
}

#[test]
fn a_selection_collapses_to_the_side_it_was_moved_towards() {
    fn render(cx: &mut Cx, _: &()) -> Element {
        holding("\u{05D0}\u{05D1}\u{05D2}", cx)
    }
    let mut screen = Screen::new(render);
    screen.press(KeyCode::Tab);
    screen.send(key(KeyCode::Character('a')).ctrl());
    // Everything's selected. Going right ends up at the right, which is where the text starts.
    screen.press(KeyCode::Right);
    screen.type_text("X");
    assert_eq!(screen.value("held"), "X\u{05D0}\u{05D1}\u{05D2}");
}

#[test]
fn a_right_to_left_value_lines_up_on_the_right_and_clicks_find_it() {
    fn render(cx: &mut Cx, _: &()) -> Element {
        holding("\u{05D0}\u{05D1}", cx)
    }
    let mut screen = Screen::new(render);
    screen.pixels();
    // The input's text area runs from 2 to 98. Clicking at its far right is before alef.
    screen.click_at(97.0, 10.0);
    screen.type_text("\u{05D2}");
    assert_eq!(screen.value("held"), "\u{05D2}\u{05D0}\u{05D1}");
    // And its far left is after the last letter.
    screen.click_at(3.0, 10.0);
    screen.type_text("\u{05D3}");
    assert_eq!(screen.value("held"), "\u{05D2}\u{05D0}\u{05D1}\u{05D3}");
}

#[test]
fn a_click_in_the_middle_of_a_right_to_left_value_lands_between_its_letters() {
    fn render(cx: &mut Cx, _: &()) -> Element {
        holding("\u{05D0}\u{05D1}", cx)
    }
    let width =
        TextEngine::new(plain_font()).line_width("\u{05D0}", rsui::text::TextStyle::new(10.0));
    let mut screen = Screen::new(render);
    screen.pixels();
    // The text area ends at 98, so one letter in from there.
    screen.click_at(98.0 - width, 10.0);
    screen.type_text("X");
    assert_eq!(screen.value("held"), "\u{05D0}X\u{05D1}");
}

#[test]
fn a_right_to_left_value_is_drawn_on_the_right() {
    fn render(cx: &mut Cx, _: &()) -> Element {
        holding("\u{05D0}", cx)
    }
    let mut screen = Screen::new(render);
    screen.press(KeyCode::Tab);
    screen.send(key(KeyCode::Character('a')).ctrl());
    // What's selected shows where the text is. The text area runs from 2 to 98.
    assert_ne!(screen.pixel(96, 8), WHITE_PIXEL, "the right is selected");
    assert_eq!(screen.pixel(5, 8), WHITE_PIXEL, "the left has nothing");
}

#[test]
fn a_handler_can_pass_a_key_on_to_what_is_around_it() {
    thread_local! {
        static HEARD: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    fn heard(who: &str, input: Option<&PropValue>) -> bool {
        let Some(PropValue::Str(key)) = input else {
            return false;
        };
        HEARD.with(|heard| heard.borrow_mut().push(format!("{who} {}", key.as_str())));
        key.as_str() == "x"
    }
    fn app(cx: &mut Cx, _: &()) -> Element {
        let inner = cx.on(
            Element::new(Role::Button).key("inner").name("inner"),
            ActionKind::Click,
            |_, _| {},
        );
        let inner = cx.on(inner, ActionKind::KeyPress, |cx, input| {
            if heard("inner", input) {
                cx.pass_on();
            }
        });
        let outer = Element::new(Role::Window).key("outer").child(inner);
        cx.on(outer, ActionKind::KeyPress, |_, input| {
            heard("outer", input);
        })
    }
    let mut under_test = App::new(app, (), TextEngine::new(plain_font()), (100, 100), 1.0);
    under_test.update();
    under_test.scene();
    under_test.key(key(KeyCode::Tab)).unwrap();
    under_test.key(key(KeyCode::Character('x'))).unwrap();
    under_test.key(key(KeyCode::Character('y'))).unwrap();
    assert_eq!(
        HEARD.with(|heard| heard.borrow().clone()),
        ["inner x", "outer x", "inner y"],
        "x is passed on, and y stops at the nearest"
    );
}

#[test]
fn the_caret_says_where_it_is_for_what_composes_text() {
    let mut screen = Screen::new(form);
    screen.pixels();
    assert_eq!(screen.app.caret_place(), None, "nothing's being edited");
    // `second` has "AB" in it, and starts 20 down, with a padding of 2.
    screen.press(KeyCode::Tab);
    screen.press(KeyCode::Tab);
    let (x, y, height) = screen.app.caret_place().expect("that's being edited");
    assert_eq!(y, 22.0);
    assert_eq!(height, 11.0);
    let at_first = x;
    screen.press(KeyCode::End);
    let (x, _, _) = screen.app.caret_place().unwrap();
    assert!(x >= at_first, "{x} {at_first}");
    assert_eq!(
        x,
        2.0 + 6.0 + 5.0,
        "it's after both letters, wherever Tab left it"
    );
}

fn with_a_disabled_button(cx: &mut Cx, _: &()) -> Element {
    let clicks = cx.state(|| 0);
    let off = Element::new(Role::Button)
        .key("off")
        .name("off")
        .props(
            rsui::core::Props::from(Style::new().width(50.0).height(20.0))
                .merged(&Paint::new().background(BLUE).hover(RED).into()),
        )
        .prop("disabled", true);
    let off = cx.on(off, ActionKind::Click, move |cx, _| {
        cx.update(clicks, |clicks| *clicks += 1);
    });
    let on = Element::new(Role::Button)
        .key("on")
        .name("on")
        .props(Style::new().width(50.0).height(20.0));
    let on = cx.on(on, ActionKind::Click, move |cx, _| {
        cx.update(clicks, |clicks| *clicks += 1);
    });
    Element::new(Role::Window)
        .props(Paint::new().background(Color::WHITE))
        .child(off)
        .child(on)
        .child(
            Element::new(Role::Text)
                .name(text!("clicked {}", cx.get(clicks)))
                .prop("font_size", 10.0),
        )
}

#[test]
fn something_disabled_can_not_be_used_and_looks_it() {
    let mut screen = Screen::new(with_a_disabled_button);
    // It's faded towards what's behind it.
    let shown = screen.pixel(25, 10);
    assert_ne!(shown, BLUE_PIXEL);
    assert!(
        shown[2] == 255 && shown[0] > 100 && shown[0] < 160,
        "{shown:?}"
    );
    // It doesn't light up, or say that it can be clicked.
    screen.app.pointer_move(25.0, 10.0);
    screen.app.update();
    assert_eq!(screen.app.cursor(), Cursor::Arrow);
    screen.app.pointer_down(25.0, 10.0, 1);
    let _ = screen.app.pointer_up(25.0, 10.0);
    screen.app.update();
    assert!(
        screen.outline().contains("clicked 0"),
        "{}",
        screen.outline()
    );
    // The keyboard goes past it.
    screen.press(KeyCode::Tab);
    screen.press(KeyCode::Enter);
    assert!(
        screen.outline().contains("clicked 1"),
        "{}",
        screen.outline()
    );
}

fn with_a_special_input(cx: &mut Cx, kind: &&'static str) -> Element {
    let value = cx.state(|| Text::from("AB"));
    let element = Element::new(Role::TextInput)
        .key("field")
        .props(
            rsui::core::Props::from(Style::new().width(100.0).height(20.0).padding(2.0))
                .merged(&Paint::new().background(Color::WHITE).into()),
        )
        .prop("font_size", 10.0)
        .prop("value", cx.get(value));
    let element = match *kind {
        "password" => element.prop("password", true),
        "read_only" => element.prop("read_only", true),
        "disabled" => element.prop("disabled", true),
        _ => element.prop("max_length", 4i64),
    };
    let element = cx.on(element, ActionKind::SetValue, move |cx, input| {
        if let Some(PropValue::Str(text)) = input {
            cx.set(value, Text::clone(text));
        }
    });
    Element::new(Role::Window).child(element)
}

fn special_input(kind: &'static str) -> (App, Renderer) {
    let mut app = App::new(
        with_a_special_input,
        kind,
        TextEngine::new(plain_font()),
        SIZE,
        1.0,
    );
    app.update();
    let renderer = Renderer::offscreen(SIZE).expect("the software GPU is always there");
    app.key(key(KeyCode::Tab)).unwrap();
    // Coming in with the keyboard selects all of it, and these go on from the end.
    app.key(key(KeyCode::End)).unwrap();
    app.update();
    (app, renderer)
}

#[test]
fn a_read_only_input_can_be_selected_and_copied_but_not_changed() {
    let (mut app, _renderer) = special_input("read_only");
    app.type_text("x").unwrap();
    app.key(key(KeyCode::Backspace)).unwrap();
    app.update();
    assert!(app.outline().contains(r#"value="AB""#), "{}", app.outline());
}

#[test]
fn nothing_pasted_cut_composed_or_undone_changes_a_read_only_input_and_it_still_copies() {
    let (mut app, _renderer) = special_input("read_only");
    let clipboard = std::rc::Rc::new(std::cell::RefCell::new(Some("pasted".to_owned())));
    #[derive(Clone)]
    struct Shared(std::rc::Rc<std::cell::RefCell<Option<String>>>);
    impl rsui::Clipboard for Shared {
        fn read(&mut self) -> Option<String> {
            self.0.borrow().clone()
        }
        fn write(&mut self, text: &str) {
            *self.0.borrow_mut() = Some(text.to_owned());
        }
    }
    app.set_clipboard(Shared(clipboard.clone()));
    let ctrl = |letter| KeyPress::new(KeyCode::Character(letter)).ctrl();
    app.key(ctrl('v')).unwrap();
    app.key(key(KeyCode::Delete)).unwrap();
    app.key(key(KeyCode::Home)).unwrap();
    app.key(key(KeyCode::Delete)).unwrap();
    app.composition("start", "").unwrap();
    app.composition("update", "ni").unwrap();
    app.composition("end", "\u{4F60}").unwrap();
    app.type_text("\u{4F60}").unwrap();
    app.key(ctrl('z')).unwrap();
    app.key(ctrl('y')).unwrap();
    // Cutting does nothing at all, as a browser can't cut from it: what's selected stays, and the clipboard is as it was.
    app.key(ctrl('a')).unwrap();
    app.key(ctrl('x')).unwrap();
    app.update();
    assert!(app.outline().contains(r#"value="AB""#), "{}", app.outline());
    assert_eq!(clipboard.borrow().as_deref(), Some("pasted"));
    assert!(app.focused().is_some(), "it keeps the keyboard");
    // But what's selected copies.
    app.key(ctrl('c')).unwrap();
    assert_eq!(clipboard.borrow().as_deref(), Some("AB"));
}

#[test]
fn an_input_only_takes_as_much_as_there_is_room_for() {
    let (mut app, _renderer) = special_input("limited");
    app.type_text("CDEFG").unwrap();
    app.update();
    assert!(
        app.outline().contains(r#"value="ABCD""#),
        "{}",
        app.outline()
    );
    // More can go in place of what's selected, and it goes by characters rather than bytes.
    app.key(KeyPress::new(KeyCode::Character('a')).ctrl())
        .unwrap();
    app.type_text("\u{4F60}\u{597D}\u{4E16}\u{754C}\u{5427}")
        .unwrap();
    app.update();
    assert!(
        app.outline()
            .contains("value=\"\u{4F60}\u{597D}\u{4E16}\u{754C}\""),
        "{}",
        app.outline()
    );
}

#[test]
fn a_disabled_input_can_not_have_the_keyboard() {
    let (mut app, _renderer) = special_input("disabled");
    assert!(app.caret_place().is_none());
    app.type_text("x").unwrap();
    app.update();
    assert!(app.outline().contains(r#"value="AB""#), "{}", app.outline());
}

#[test]
fn a_password_shows_dots_with_the_caret_where_it_is_and_can_not_be_copied() {
    let (mut app, mut renderer) = special_input("password");
    app.type_text("\u{4F60}").unwrap();
    app.update();
    // Three letters' worth of dots, however many bytes each of them is, with the caret after the last of them.
    let (x, _, _) = app.caret_place().unwrap();
    let (mut other, _) = special_input("password");
    other.update();
    let (two_dots, _, _) = other.caret_place().unwrap();
    assert!(x > two_dots, "{x} {two_dots}");
    assert_eq!(
        x - 2.0,
        (two_dots - 2.0) / 2.0 * 3.0,
        "each is as wide as the rest"
    );
    // Clicking between them finds the right place in what's really there.
    app.render(&mut renderer).expect("drawing works");
    let middle = 2.0 + (x - 2.0) / 3.0 * 2.0;
    app.pointer_down(middle, 10.0, 1);
    let _ = app.pointer_up(middle, 10.0);
    app.type_text("Z").unwrap();
    app.update();
    assert!(
        app.outline().contains("value=\"ABZ\u{4F60}\""),
        "{}",
        app.outline()
    );
    // Nothing of it reaches the clipboard.
    let copied = std::rc::Rc::new(std::cell::RefCell::new(None));
    #[derive(Clone)]
    struct Watching(std::rc::Rc<std::cell::RefCell<Option<String>>>);
    impl rsui::Clipboard for Watching {
        fn read(&mut self) -> Option<String> {
            None
        }
        fn write(&mut self, text: &str) {
            *self.0.borrow_mut() = Some(text.to_owned());
        }
    }
    app.set_clipboard(Watching(copied.clone()));
    app.key(KeyPress::new(KeyCode::Character('a')).ctrl())
        .unwrap();
    app.key(KeyPress::new(KeyCode::Character('c')).ctrl())
        .unwrap();
    assert_eq!(*copied.borrow(), None);
}

#[test]
fn a_click_says_how_far_inside_it_landed_and_can_be_passed_on() {
    thread_local! {
        static HEARD: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    fn heard(who: &str, input: Option<&PropValue>) {
        let within = match input {
            Some(PropValue::Int(within)) => *within,
            _ => 0,
        };
        HEARD.with(|heard| heard.borrow_mut().push(format!("{who} {within}")));
    }
    fn app(cx: &mut Cx, _: &()) -> Element {
        let passes = cx.on(
            Element::new(Role::Button).key("passes").prop("height", 20.0),
            ActionKind::Click,
            |cx, input| {
                heard("passes", input);
                cx.pass_on();
            },
        );
        let stops = cx.on(
            Element::new(Role::Button).key("stops").prop("height", 20.0),
            ActionKind::Click,
            |_, input| heard("stops", input),
        );
        // A box with nothing to do with clicks, with a box inside it, so a press there is two boxes inside what hears it.
        let plain = Element::new(Role::Group)
            .key("plain")
            .prop("height", 20.0)
            .child(Element::new(Role::Group).key("deeper").prop("height", 20.0));
        let outer = Element::new(Role::Group)
            .key("outer")
            .prop("padding_top", 10.0)
            .child(passes)
            .child(stops)
            .child(plain);
        cx.on(outer, ActionKind::Click, |_, input| heard("outer", input))
    }
    let mut under_test = App::new(app, (), TextEngine::new(plain_font()), (100, 100), 1.0);
    under_test.update();
    under_test.scene();
    // In the middle of each, from the top: the padding, then 20 for each.
    for y in [20.0, 40.0, 60.0, 5.0] {
        under_test.click(50.0, y).unwrap();
    }
    assert_eq!(
        HEARD.with(|heard| heard.borrow().clone()),
        ["passes 0", "outer 1", "stops 0", "outer 2", "outer 0"],
        "what's passed on goes to the nearest around it, which is told how far inside the press was"
    );
}

// Events as code written for the web has them, which go from the root down to what they happened to and back up.

thread_local! {
    static LOG: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn note(said: String) {
    LOG.with(|log| log.borrow_mut().push(said));
}

fn take_log() -> Vec<String> {
    LOG.with(|log| std::mem::take(&mut *log.borrow_mut()))
}

fn custom(name: &str) -> ActionKind {
    ActionKind::Custom(rsui::core::Symbol::new(name))
}

fn sized(role: Role, key: &str, width: f32, height: f32) -> Element {
    Element::new(role)
        .key(key)
        .props(rsui::core::Props::from(Style::new().width(width).height(height)))
}

/// A track 200 wide, from (0, 0), with a thumb 10 wide at its start, which hears the pointer, and a box under it that doesn't.
fn track(cx: &mut Cx, _: &()) -> Element {
    let thumb = sized(Role::Group, "thumb", 10.0, 20.0);
    let mut track = sized(Role::Group, "track", 200.0, 20.0).child(thumb);
    for phase in ["pointer_down", "pointer_move", "pointer_up"] {
        track = cx.on(track, custom(phase), move |cx, _| {
            let event = cx.event().expect("it says where");
            let along = event.x - event.boxes[event.current.len()][0];
            note(format!("{phase} {along} {} {}", event.buttons, event.target.len()));
        });
    }
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start)))
        .child(track)
        .child(sized(Role::Group, "under", 200.0, 50.0))
}

#[test]
fn moves_go_to_what_was_pressed_until_its_let_go_of_and_only_to_what_hears_them() {
    let mut app = App::new(track, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    app.pointer_move(5.0, 10.0);
    app.pointer_down(5.0, 10.0, 1);
    app.pointer_move(50.0, 10.0);
    // Off it, and still its, while it's held.
    app.pointer_move(250.0, 40.0);
    app.pointer_up(250.0, 40.0).unwrap();
    // Not held, so only while it's over it.
    app.pointer_move(100.0, 40.0);
    app.pointer_move(120.0, 10.0);
    assert_eq!(
        take_log(),
        [
            "pointer_move 5 0 2",
            "pointer_down 5 1 2",
            "pointer_move 50 1 2",
            "pointer_move 250 1 2",
            "pointer_up 250 0 2",
            "pointer_move 120 0 1",
        ],
        "the thumb is two inside the root, and the track one"
    );
}

#[test]
fn moves_are_told_once_a_frame_at_most_and_the_last_one_isnt_lost() {
    let mut app = App::new(track, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    app.set_move_pace(std::time::Duration::from_millis(40));
    for x in [20.0, 30.0, 40.0] {
        app.pointer_move(x, 10.0);
    }
    assert_eq!(take_log(), ["pointer_move 20 0 1"]);
    let wait = app.next_wake().expect("the last one is waiting");
    assert!(wait <= std::time::Duration::from_millis(40), "{wait:?}");
    std::thread::sleep(wait + std::time::Duration::from_millis(5));
    app.wake();
    assert_eq!(take_log(), ["pointer_move 40 0 1"]);
    assert_eq!(app.next_wake(), None, "and then nothing's waiting");
}

#[test]
fn clicks_say_what_was_held_and_other_buttons_click_as_aux_clicks() {
    fn picks(cx: &mut Cx, _: &()) -> Element {
        let item = sized(Role::Button, "item", 100.0, 20.0);
        let item = cx.on(item, ActionKind::Click, |cx, _| {
            let event = cx.event().expect("a press says where it was");
            let held = event.modifiers;
            note(format!("click {} {} {} {} {}", event.button, held.ctrl, held.shift, event.x, event.detail));
        });
        let item = cx.on(item, custom("aux_click"), |cx, _| {
            note(format!("aux {}", cx.event().expect("it says").button));
        });
        let item = cx.on(item, custom("pointer_down"), |cx, _| {
            note(format!("down {}", cx.event().expect("it says").button));
        });
        Element::new(Role::Window).child(item)
    }
    let mut app = App::new(picks, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    app.set_modifiers(rsui::Modifiers { ctrl: true, ..Default::default() });
    app.pointer_down(30.0, 10.0, 1);
    app.pointer_up(30.0, 10.0).unwrap();
    app.set_modifiers(rsui::Modifiers { shift: true, ..Default::default() });
    app.click(40.0, 10.0).unwrap();
    app.set_modifiers(rsui::Modifiers::default());
    app.button_down(30.0, 10.0, rsui::PointerButton::Middle);
    app.button_up(30.0, 10.0, rsui::PointerButton::Middle).unwrap();
    assert_eq!(
        take_log(),
        ["down 0", "click 0 true false 30 1", "click 0 false true 40 1", "down 1", "aux 1"]
    );
}

#[test]
fn the_app_as_a_whole_hears_what_it_listens_for_on_the_way_down_and_back_up() {
    fn page(cx: &mut Cx, _: &()) -> Element {
        cx.on_document_event(|updates, kind| {
            let event = updates.event().expect("it says what it was");
            note(format!("{kind} {} {}", event.phase, event.target.len()));
            if kind == "keydown" && event.phase == 1 && event.key.as_ref().is_some_and(|key| key.key == "q") {
                updates.stop_propagation();
            }
        });
        let button = sized(Role::Button, "button", 100.0, 20.0);
        let button = cx.on(button, ActionKind::Click, |_, _| note("button".to_owned()));
        let button = cx.on(button, custom("key_down"), |_, _| note("button key".to_owned()));
        Element::new(Role::Window)
            .props(style!(Style::new().align(Align::Start)))
            .child(button)
            .child(sized(Role::Group, "nothing", 100.0, 50.0))
    }
    let mut app = App::new(page, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    rsui::runtime::set_document_listens(rsui::runtime::listens::CLICK | rsui::runtime::listens::KEY_DOWN);
    // Something that doesn't take clicks is still clicked, as far as the app as a whole goes.
    app.click(50.0, 40.0).unwrap();
    app.click(50.0, 10.0).unwrap();
    app.focus_on(&[0]);
    app.key(KeyPress::new(KeyCode::Character('w'))).unwrap();
    app.key(KeyPress::new(KeyCode::Character('q'))).unwrap();
    rsui::runtime::set_document_listens(0);
    app.click(50.0, 40.0).unwrap();
    assert_eq!(
        take_log(),
        [
            // Phase 2 is between what's on the way down and what's on the way up, where a page's listeners on elements hear it.
            "click 1 1",
            "click 2 1",
            "click 3 1",
            "click 1 1",
            "click 2 1",
            "button",
            "click 3 1",
            "keydown 1 1",
            "keydown 2 1",
            "button key",
            "keydown 3 1",
            // Stopped on the way down, so nothing else hears it, apart from the app as a whole being told once more, after
            // everything, for what would happen by itself (a page's browser behaviour, which stopping doesn't stop).
            "keydown 1 1",
            "keydown 4 1",
        ]
    );
}

#[test]
fn a_key_stopped_on_the_way_down_does_nothing_in_a_field_and_isnt_typed() {
    fn guarded(cx: &mut Cx, _: &()) -> Element {
        let value = cx.state(Text::default);
        let field = input(cx, "field", value);
        let root = Element::new(Role::Window).key("root").child(field);
        cx.on(root, custom("key_down_capture"), |cx, input| {
            if matches!(input, Some(PropValue::Str(key)) if key.as_str() == "x" || key.as_str() == "ctrl+a") {
                cx.prevent_default();
            }
        })
    }
    let mut screen = Screen::new(guarded);
    screen.pixels();
    screen.app.focus_on(&[0]);
    for (key, typed) in [('a', "a"), ('x', "x"), ('b', "b")] {
        screen.send(KeyPress::new(KeyCode::Character(key)));
        screen.type_text(typed);
    }
    assert_eq!(screen.value("field"), "ab");
    // Selecting everything is stopped too, so what's typed goes at the end rather than over it all.
    screen.send(KeyPress::new(KeyCode::Character('a')).ctrl());
    screen.type_text("c");
    assert_eq!(screen.value("field"), "abc");
}

#[test]
fn tab_index_says_what_tab_goes_to_first_and_what_it_never_does() {
    fn boxes(_: &mut Cx, _: &()) -> Element {
        let mut root = Element::new(Role::Window);
        for (name, index) in [("zero", Some(0)), ("never", Some(-1)), ("second", Some(2)), ("first", Some(1)), ("plain", None)] {
            let mut one = sized(Role::Group, name, 50.0, 10.0);
            if let Some(index) = index {
                one = one.prop("tab_index", index as i64);
            }
            root = root.child(one);
        }
        root
    }
    let mut app = App::new(boxes, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    let mut order = Vec::new();
    for _ in 0..4 {
        app.key(KeyPress::new(KeyCode::Tab)).unwrap();
        let at = app.focused().expect("tab went somewhere")[0] as usize;
        order.push(["zero", "never", "second", "first", "plain"][at]);
    }
    assert_eq!(order, ["first", "second", "zero", "first"]);
    // It can still have the keyboard by being clicked.
    app.pointer_down(10.0, 15.0, 1);
    app.pointer_up(10.0, 15.0).unwrap();
    assert_eq!(app.focused(), Some(&[1][..]));
}

#[test]
fn a_paste_says_what_it_is_and_can_be_stopped() {
    fn pasting(cx: &mut Cx, _: &()) -> Element {
        let value = cx.state(Text::default);
        let field = input(cx, "field", value);
        let field = cx.on(field, custom("clipboard"), |cx, what| {
            let data = cx.event().and_then(|event| event.data.clone()).unwrap_or_default();
            note(format!("{what:?} {data}"));
            if data.contains("secret") {
                cx.prevent_default();
            }
        });
        Element::new(Role::Window).child(field)
    }
    let mut screen = Screen::new(pasting);
    screen.pixels();
    screen.app.set_clipboard(rsui::MemoryClipboard::default());
    screen.app.focus_on(&[0]);
    screen.type_text("secret");
    screen.send(KeyPress::new(KeyCode::Character('a')).ctrl());
    screen.send(KeyPress::new(KeyCode::Character('c')).ctrl());
    screen.send(KeyPress::new(KeyCode::Character('v')).ctrl());
    assert_eq!(screen.value("field"), "secret", "the paste was stopped, so it's still selected");
    let log = take_log();
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(log[0].contains("copy"), "{log:?}");
    assert!(log[1].contains("paste") && log[1].ends_with(" secret"), "{log:?}");
}

#[test]
fn something_draggable_is_dragged_once_it_moves_and_files_from_outside_still_go_where_they_always_have() {
    fn board(cx: &mut Cx, _: &()) -> Element {
        let card = sized(Role::Group, "card", 50.0, 20.0).prop("draggable", true);
        let card = cx.on(card, custom("web_drag"), |_, kind| note(format!("card {kind:?}")));
        let zone = sized(Role::Group, "zone", 100.0, 40.0);
        let zone = cx.on(zone, custom("web_drag"), |cx, kind| {
            note(format!("zone {kind:?}"));
            if matches!(kind, Some(PropValue::Str(said)) if said.as_str() == "dragover") {
                cx.prevent_default();
            }
        });
        let files = sized(Role::Group, "files", 100.0, 40.0);
        let files = cx.on(files, custom("drop_files"), |_, paths| note(format!("files {paths:?}")));
        Element::new(Role::Window)
            .props(style!(Style::new().align(Align::Start)))
            .child(card)
            .child(zone)
            .child(files)
    }
    let mut app = App::new(board, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    app.pointer_down(10.0, 10.0, 1);
    // Not far enough yet.
    app.pointer_move(12.0, 11.0);
    assert!(!app.is_dragging());
    app.pointer_move(20.0, 40.0);
    app.pointer_up(20.0, 40.0).unwrap();
    let said: Vec<String> = take_log()
        .into_iter()
        .map(|line| line.replace("Some(Str(\"", "").replace("\"))", ""))
        .filter(|line| !line.ends_with(" drag"))
        .collect();
    assert_eq!(said, ["card dragstart", "zone dragenter", "zone dragover", "zone drop", "card dragend"]);
    let paths = ["C:/one.txt".to_owned()];
    assert!(app.drag_from_outside(rsui::DropPhase::Enter, 50.0, 80.0, &paths, None));
    assert!(app.drag_from_outside(rsui::DropPhase::Drop, 50.0, 80.0, &paths, None));
    assert_eq!(take_log(), ["files Some(Str(\"C:/one.txt\"))"]);
}
