//! Text inputs with several lines, drawn offscreen. In the test font at size 10, a line is 11 high,
//! `A` is 6 wide, `C` is 8 wide, and a space is 3.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use proptest::prelude::*;
use rsui::prelude::*;
use rsui::render::Renderer;
use rsui::text::TextEngine;
use rsui::{App, KeyCode, KeyPress, MemoryClipboard};
use support::plain_font;

const SIZE: (u32, u32) = (200, 200);

fn area_of(cx: &mut Cx, key: &'static str, value: State<Text>, rows: u32) -> Element {
    let element = Element::new(Role::TextInput)
        .key(key)
        .name("Area")
        .props(
            rsui::core::Props::from(Style::new().width(100.0).padding(2.0))
                .merged(&Paint::new().background(Color::WHITE).into()),
        )
        .prop("font_size", 10.0)
        .prop("value", cx.get(value))
        .prop("rows", i64::from(rows));
    cx.on(element, ActionKind::SetValue, move |cx, input| {
        if let Some(PropValue::Str(text)) = input {
            cx.set(value, Text::clone(text));
        }
    })
}

/// A text area that starts with what's in `props`, at the top left, three lines high, and 96 wide
/// inside its padding, so 12 `C`s or 16 `A`s fit on a line.
#[allow(clippy::ptr_arg)]
fn holding(cx: &mut Cx, start: &String) -> Element {
    let start = start.clone();
    let value = cx.state(move || Text::from(start.as_str()));
    let area = area_of(cx, "area", value, 3);
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start)))
        .child(area)
}

const AREA: &[u32] = &[0];

struct Screen {
    app: App,
    renderer: Renderer,
}

impl Screen {
    fn new(start: &str) -> Self {
        let mut screen = Self {
            app: App::new(
                holding,
                start.to_owned(),
                TextEngine::new(plain_font()),
                SIZE,
                1.0,
            ),
            renderer: Renderer::offscreen(SIZE).expect("the software GPU is always there"),
        };
        screen.pixels();
        screen.app.focus_on(AREA);
        screen.pixels();
        // Tabbing in selects everything, which these don't want to start from.
        screen.press(KeyCode::Home);
        screen.send(KeyPress::new(KeyCode::Home).ctrl());
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

    fn send(&mut self, press: KeyPress) {
        self.app.key(press).expect("the key goes through");
        self.pixels();
    }

    fn press(&mut self, code: KeyCode) {
        self.send(KeyPress::new(code));
    }

    fn type_text(&mut self, text: &str) {
        self.app.type_text(text).expect("it goes through");
        self.pixels();
    }

    fn value(&mut self) -> String {
        let tree = self.app.access_tree();
        tree.by_key("area")
            .and_then(|node| node.value.as_ref())
            .expect("it has a value")
            .to_string()
    }

    /// Where the caret is, by typing there and seeing where that went, then taking it back.
    fn caret(&mut self) -> usize {
        let before = self.value();
        self.type_text("\u{E000}");
        let after = self.value();
        self.send(KeyPress::new(KeyCode::Backspace));
        assert_eq!(self.value(), before);
        after.find('\u{E000}').expect("it's there")
    }
}

#[test]
fn it_is_as_tall_as_its_rows_and_as_wide_as_it_says() {
    let screen = Screen::new("");
    let rect = screen.app.layout().rect(AREA).expect("it's laid out");
    assert_eq!(rect.height, 3.0 * 11.0 + 4.0);
    assert_eq!(rect.width, 100.0);
}

#[test]
fn enter_starts_a_new_line() {
    let mut screen = Screen::new("");
    screen.type_text("A");
    screen.press(KeyCode::Enter);
    screen.type_text("C");
    assert_eq!(screen.value(), "A\nC");
    screen.press(KeyCode::Backspace);
    screen.press(KeyCode::Backspace);
    assert_eq!(screen.value(), "A");
}

#[test]
fn a_single_line_input_is_left_alone_by_enter() {
    fn single(cx: &mut Cx, _: &()) -> Element {
        let value = cx.state(Text::default);
        let input = text_input(cx, "one", "One", "", value);
        Element::new(Role::Window)
            .props(style!(Style::new().align(Align::Start)))
            .child(input)
    }
    let mut app = App::new(single, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    app.focus_on(AREA);
    app.type_text("A").unwrap();
    app.key(KeyPress::new(KeyCode::Enter)).unwrap();
    app.update();
    assert!(app.outline().contains("value=\"A\""), "{}", app.outline());
}

#[test]
fn up_and_down_go_between_lines_and_keep_their_place_across() {
    // Finding where the caret is means typing, which forgets where across it was going, so each of
    // these is its own trip.
    let across = |downs: usize, ups: usize| {
        let mut screen = Screen::new("AAAA\nA\nAAAA");
        for _ in 0..3 {
            screen.press(KeyCode::Right);
        }
        for _ in 0..downs {
            screen.press(KeyCode::Down);
        }
        for _ in 0..ups {
            screen.press(KeyCode::Up);
        }
        screen.caret()
    };
    assert_eq!(across(0, 0), 3);
    assert_eq!(across(1, 0), 6, "the end of the short line");
    assert_eq!(across(2, 0), 7 + 3, "and back to where it was across");
    assert_eq!(across(2, 1), 6);
    assert_eq!(across(2, 2), 3, "all the way back");
}

#[test]
fn going_past_the_first_or_last_line_goes_to_its_end() {
    let mut screen = Screen::new("AA\nAA");
    screen.press(KeyCode::Right);
    screen.press(KeyCode::Up);
    assert_eq!(screen.caret(), 0);
    screen.press(KeyCode::Down);
    screen.press(KeyCode::Down);
    assert_eq!(screen.caret(), 5);
}

#[test]
fn home_and_end_stay_on_the_line_unless_control_is_held() {
    let mut screen = Screen::new("AAA\nCCC\nAAA");
    screen.press(KeyCode::Down);
    screen.press(KeyCode::Right);
    assert_eq!(screen.caret(), 5);
    screen.press(KeyCode::Home);
    assert_eq!(screen.caret(), 4);
    screen.press(KeyCode::End);
    assert_eq!(screen.caret(), 7);
    screen.send(KeyPress::new(KeyCode::End).ctrl());
    assert_eq!(screen.caret(), 11);
    screen.send(KeyPress::new(KeyCode::Home).ctrl());
    assert_eq!(screen.caret(), 0);
}

#[test]
fn long_lines_wrap_and_carets_follow_them() {
    // 20 `C`s, 12 of which fit on a line.
    let text = "C".repeat(20);
    let mut screen = Screen::new(&text);
    screen.press(KeyCode::Down);
    assert_eq!(screen.caret(), 12, "the start of what wrapped");
    screen.press(KeyCode::End);
    assert_eq!(screen.caret(), 20);
    screen.press(KeyCode::Up);
    // 8 along the second line is 64 across, which is 8 along the first.
    assert_eq!(screen.caret(), 8);
}

#[test]
fn clicking_goes_to_the_line_and_place_clicked() {
    let mut screen = Screen::new("AAAA\nCCCC\nAAAA");
    // The second line, at 11 down, after the padding of 2, and two `C`s across.
    screen.app.pointer_down(2.0 + 16.0, 2.0 + 15.0, 1);
    screen.app.pointer_up(2.0 + 16.0, 2.0 + 15.0).unwrap();
    assert_eq!(screen.caret(), 5 + 2);
    // Past the end of a line goes to its end.
    screen.app.pointer_down(90.0, 2.0 + 15.0, 1);
    screen.app.pointer_up(90.0, 2.0 + 15.0).unwrap();
    assert_eq!(screen.caret(), 9);
    // Below everything goes to the last line.
    screen.app.pointer_down(2.0 + 12.0, 2.0 + 30.0, 1);
    screen.app.pointer_up(2.0 + 12.0, 2.0 + 30.0).unwrap();
    assert_eq!(screen.caret(), 10 + 2);
}

#[test]
fn dragging_selects_across_lines() {
    let mut screen = Screen::new("AAAA\nCCCC\nAAAA");
    screen.app.pointer_down(2.0 + 12.0, 2.0 + 4.0, 1);
    screen.app.pointer_move(2.0 + 16.0, 2.0 + 15.0);
    screen.app.pointer_up(2.0 + 16.0, 2.0 + 15.0).unwrap();
    screen.press(KeyCode::Delete);
    assert_eq!(screen.value(), "AACC\nAAAA");
}

#[test]
fn shift_with_the_arrows_selects_between_lines() {
    let mut screen = Screen::new("AAAA\nCCCC");
    screen.press(KeyCode::Right);
    screen.send(KeyPress::new(KeyCode::Down).shift());
    screen.press(KeyCode::Delete);
    assert_eq!(screen.value(), "ACCC");
}

#[test]
fn a_selection_shows_on_every_line_it_covers_and_no_others() {
    let mut screen = Screen::new("AAAA\nCCCC\nAAAA\nCCCC");
    let selected_at = |screen: &mut Screen, x: usize, line: usize| {
        let pixel = screen.pixel(x, 2 + line * 11 + 9);
        pixel[..3] != [255, 255, 255]
    };
    // From the start of the first line to the middle of the second.
    screen.send(KeyPress::new(KeyCode::Down).shift());
    screen.send(KeyPress::new(KeyCode::Right).shift());
    screen.send(KeyPress::new(KeyCode::Right).shift());
    assert!(selected_at(&mut screen, 2 + 2, 0), "the first line");
    assert!(
        selected_at(&mut screen, 2 + 2, 1),
        "the start of the second"
    );
    assert!(!selected_at(&mut screen, 2 + 24, 1), "past where it ends");
    assert!(!selected_at(&mut screen, 2 + 2, 2), "the third line");
}

#[test]
fn what_does_not_fit_scrolls_to_where_the_caret_is_and_is_cut_off() {
    let mut screen = Screen::new("A\nA\nA\nA\nA\nA\nA\nA");
    let top = screen.pixels();
    // Below the area, which is 37 high, there's nothing, however many lines it has.
    let background = screen.pixel(10, 60);
    assert_ne!(background[..3], [255, 255, 255]);
    screen.send(KeyPress::new(KeyCode::End).ctrl());
    let bottom = screen.pixels();
    assert_ne!(top, bottom, "it moved");
    assert_eq!(screen.pixel(10, 60), background);
    screen.send(KeyPress::new(KeyCode::Home).ctrl());
    assert_eq!(screen.pixels(), top, "and back");
}

#[test]
fn the_wheel_scrolls_it_while_it_has_the_keyboard() {
    let mut screen = Screen::new("A\nA\nA\nA\nA\nA\nA\nA");
    let top = screen.pixels();
    screen.app.scroll(20.0, 20.0, 1).unwrap();
    let moved = screen.pixels();
    assert_ne!(top, moved);
    // The caret stays where it was in the text.
    assert_eq!(screen.caret(), 0);
    screen.app.scroll(20.0, 20.0, -100).unwrap();
    assert_eq!(screen.pixels(), top, "it stops at the top");
    let before = screen.pixels();
    screen.app.scroll(150.0, 150.0, 1).unwrap();
    assert_eq!(screen.pixels(), before, "not when the pointer is elsewhere");
}

#[test]
fn pasting_keeps_its_lines_here() {
    let mut screen = Screen::new("");
    screen.app.set_clipboard(MemoryClipboard::default());
    screen.type_text("A");
    screen.send(KeyPress::new(KeyCode::Character('a')).ctrl());
    screen.send(KeyPress::new(KeyCode::Character('c')).ctrl());
    screen.press(KeyCode::End);
    screen.press(KeyCode::Enter);
    screen.send(KeyPress::new(KeyCode::Character('v')).ctrl());
    assert_eq!(screen.value(), "A\nA");
    screen.send(KeyPress::new(KeyCode::Character('a')).ctrl());
    screen.send(KeyPress::new(KeyCode::Character('c')).ctrl());
    screen.press(KeyCode::Delete);
    screen.send(KeyPress::new(KeyCode::Character('v')).ctrl());
    assert_eq!(
        screen.value(),
        "A\nA",
        "lines survive being copied and pasted"
    );
}

#[test]
fn undo_goes_back_through_lines() {
    let mut screen = Screen::new("");
    screen.type_text("A");
    screen.press(KeyCode::Enter);
    screen.type_text("C");
    screen.send(KeyPress::new(KeyCode::Character('z')).ctrl());
    screen.send(KeyPress::new(KeyCode::Character('z')).ctrl());
    screen.send(KeyPress::new(KeyCode::Character('z')).ctrl());
    assert_eq!(screen.value(), "");
    screen.send(KeyPress::new(KeyCode::Character('y')).ctrl());
    assert!(!screen.value().is_empty());
}

#[test]
fn right_to_left_lines_start_from_the_right_and_carets_know() {
    let mut screen = Screen::new("\u{05D0}\u{05D1}\nAB");
    let tree_rect = screen.app.layout().rect(AREA).expect("it's laid out");
    // A click near the right of the first line is at its start.
    screen.app.pointer_down(tree_rect.width - 3.0, 5.0, 1);
    screen.app.pointer_up(tree_rect.width - 3.0, 5.0).unwrap();
    assert_eq!(screen.caret(), 0);
    // The other line starts from the left, whose start is the second line's first byte.
    screen.app.pointer_down(3.0, 2.0 + 15.0, 1);
    screen.app.pointer_up(3.0, 2.0 + 15.0).unwrap();
    assert_eq!(screen.caret(), 5);
}

fn a_key() -> impl Strategy<Value = KeyPress> {
    let code = prop_oneof![
        Just(KeyCode::Up),
        Just(KeyCode::Down),
        Just(KeyCode::Left),
        Just(KeyCode::Right),
        Just(KeyCode::Home),
        Just(KeyCode::End),
        Just(KeyCode::PageUp),
        Just(KeyCode::PageDown),
        Just(KeyCode::Enter),
        Just(KeyCode::Backspace),
        Just(KeyCode::Delete),
    ];
    (code, any::<bool>(), any::<bool>()).prop_map(|(code, shift, ctrl)| {
        let mut key = KeyPress::new(code);
        if shift {
            key = key.shift();
        }
        if ctrl {
            key = key.ctrl();
        }
        key
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Whatever's pressed, nothing goes wrong, and it's always text the caret is in.
    #[test]
    fn any_keys_on_any_text_keep_things_sane(
        start in "[AC \\n\u{05D0}\u{0628}e\u{0301}]{0,40}",
        keys in proptest::collection::vec(a_key(), 1..30),
    ) {
        let mut screen = Screen::new(&start);
        for key in keys {
            screen.send(key);
            let value = screen.value();
            prop_assert!(value.len() <= start.len() + 30);
        }
        // Going to a line's end lets go of anything selected, to find a place for the caret.
        screen.press(KeyCode::End);
        let caret = screen.caret();
        let value = screen.value();
        prop_assert!(caret <= value.len());
        prop_assert!(value.is_char_boundary(caret));
    }
}

#[test]
fn going_to_a_line_end_forgets_where_across_the_caret_was_going() {
    let mut screen = Screen::new("AAAA\nAAAA\nAAAA");
    for _ in 0..3 {
        screen.press(KeyCode::Right);
    }
    screen.press(KeyCode::Down);
    screen.press(KeyCode::Home);
    screen.press(KeyCode::Down);
    assert_eq!(
        screen.caret(),
        10,
        "the start of the last line, not 3 along it"
    );
}

#[test]
fn going_to_the_end_brings_it_into_view() {
    let mut screen = Screen::new("A\nA\nA\nA\nA\nA\nA\nA");
    screen.send(KeyPress::new(KeyCode::End).ctrl());
    // Three lines show, so the top of the area is now showing the sixth of the eight.
    screen.app.pointer_down(3.0, 3.0, 1);
    screen.app.pointer_up(3.0, 3.0).unwrap();
    let caret = screen.caret();
    assert!(caret >= 10, "the top of the area is at byte {caret}");
}

#[test]
fn nothing_is_drawn_past_the_bottom_of_the_area() {
    let mut screen = Screen::new("A\nA\nA\nA\nA\nA\nA\nA");
    let pixels = screen.pixels();
    let background = screen.pixel(150, 150);
    // It's 37 high, and the text would carry on well past that.
    for y in 40..100 {
        for x in 0..100 {
            let at = (y * SIZE.0 as usize + x) * 4;
            assert_eq!(pixels[at..at + 4], background, "at {x},{y}");
        }
    }
}

#[test]
fn the_wheel_only_moves_the_area_it_is_over() {
    fn two(cx: &mut Cx, _: &()) -> Element {
        let first = cx.state(|| Text::from("A\nA\nA\nA\nA\nA\nA\nA"));
        let second = cx.state(|| Text::from("C\nC\nC\nC\nC\nC\nC\nC"));
        let first = area_of(cx, "area", first, 3);
        let second = area_of(cx, "other", second, 3);
        Element::new(Role::Window)
            .props(style!(Style::new().align(Align::Start)))
            .child(first)
            .child(second)
    }
    let mut app = App::new(two, (), TextEngine::new(plain_font()), SIZE, 1.0);
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    let mut draw = |app: &mut App| {
        app.update();
        app.render(&mut renderer).unwrap();
        renderer.read_pixels().unwrap()
    };
    draw(&mut app);
    app.focus_on(AREA);
    app.key(KeyPress::new(KeyCode::Home).ctrl()).unwrap();
    let before = draw(&mut app);
    // Over the other one, which doesn't have the keyboard.
    app.scroll(20.0, 50.0, 1).unwrap();
    assert!(draw(&mut app) == before, "the first one moved");
    app.scroll(20.0, 20.0, 1).unwrap();
    assert!(draw(&mut app) != before, "over the first one, it does");
}

#[test]
fn a_click_along_a_right_to_left_line_goes_between_its_letters() {
    let mut screen = Screen::new("\u{05D0}\u{05D1}");
    let one =
        TextEngine::new(plain_font()).line_width("\u{05D0}", rsui::text::TextStyle::new(10.0));
    // The text area ends at 98, and the first letter is the rightmost.
    screen.app.pointer_down(98.0 - one, 5.0, 1);
    screen.app.pointer_up(98.0 - one, 5.0).unwrap();
    assert_eq!(screen.caret(), 2);
}

#[test]
fn a_caret_at_the_far_right_of_a_line_still_shows() {
    let mut screen = Screen::new("\u{05D0}\u{05D1}");
    // At the start of the text, which is at its right edge.
    screen.send(KeyPress::new(KeyCode::Home).ctrl());
    let pixel = screen.pixel(97, 6);
    assert!(pixel[0] < 128, "{pixel:?}");
}

#[test]
fn a_blink_gives_the_scene_working_it_out_again_would() {
    let focused = |blinks: usize| {
        let mut app = App::new(
            holding,
            "AAAA\nCCCC".to_owned(),
            TextEngine::new(plain_font()),
            SIZE,
            1.0,
        );
        app.update();
        app.focus_on(AREA);
        app.key(KeyPress::new(KeyCode::End)).unwrap();
        app.update();
        for _ in 0..blinks {
            app.blink();
        }
        app
    };
    let mut blinking = focused(0);
    blinking.scene();
    assert!(blinking.caret_blinking());
    for blinks in 1..6 {
        blinking.blink();
        blinking.update();
        let scene = blinking.scene();
        let (quads, batches) = (scene.quads.clone(), scene.batches.clone());
        // This one has never been drawn, so it has to work it all out.
        let mut fresh = focused(blinks);
        fresh.update();
        let scene = fresh.scene();
        assert_eq!(quads, scene.quads, "after {blinks} blinks");
        assert_eq!(batches, scene.batches, "after {blinks} blinks");
    }
}
