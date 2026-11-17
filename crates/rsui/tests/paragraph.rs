//! Text with parts that look different, from what's asked for to the pixels, drawn offscreen so no
//! window opens.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use std::cell::RefCell;
use std::rc::Rc;

use rsui::prelude::*;
use rsui::render::Renderer;
use rsui::text::TextEngine;
use rsui::{App, Cursor};
use support::plain_font;

const SIZE: (u32, u32) = (200, 140);
const RED: Color = Color::hex(0xFF0000);
const GREEN: Color = Color::hex(0x00FF00);
const BLUE: Color = Color::hex(0x0000FF);
const RED_PIXEL: [u8; 4] = [255, 0, 0, 255];
const GREEN_PIXEL: [u8; 4] = [0, 255, 0, 255];
const BLUE_PIXEL: [u8; 4] = [0, 0, 255, 255];
const WHITE_PIXEL: [u8; 4] = [255, 255, 255, 255];

/// A clipboard this test can see into.
struct Shared(Rc<RefCell<Option<String>>>);

impl rsui::Clipboard for Shared {
    fn read(&mut self) -> Option<String> {
        self.0.borrow().clone()
    }

    fn write(&mut self, text: &str) {
        *self.0.borrow_mut() = Some(text.to_owned());
    }
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

    fn click(&mut self, x: f32, y: f32) {
        self.app.pointer_move(x, y);
        self.app.update();
        self.app.pointer_down(x, y, 1);
        self.app.update();
        let _ = self.app.pointer_up(x, y);
        self.app.update();
    }
}

fn part(text: &str) -> Element {
    Element::new(Role::Text).name(text)
}

/// Two parts with backgrounds: the first at (0, 0) to (11, 11) and the second from x = 13.5.
fn with_backgrounds(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window)
        .props(Paint::new().background(Color::WHITE))
        .child(
            Element::new(Role::Text)
                .prop("font_size", 10.0)
                .child(part("AB ").props(Paint::new().background(RED)))
                .child(part("AB").props(Paint::new().background(GREEN))),
        )
}

#[test]
fn each_part_has_its_own_background() {
    let mut screen = Screen::new(with_backgrounds);
    assert_eq!(screen.pixel(0, 0), RED_PIXEL);
    assert_eq!(screen.pixel(14, 0), GREEN_PIXEL);
    // Nothing's there past the end, or below.
    assert_eq!(screen.pixel(30, 0), WHITE_PIXEL);
    assert_eq!(screen.pixel(0, 20), WHITE_PIXEL);
}

fn with_colored_words(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window)
        .props(Paint::new().background(Color::WHITE))
        .child(
            Element::new(Role::Text)
                .prop("font_size", 40.0)
                .prop("text_color", RED.to_prop())
                .child(part("A"))
                .child(part("A").prop("text_color", BLUE.to_prop())),
        )
}

#[test]
fn a_part_has_its_own_color_and_otherwise_that_of_what_it_is_in() {
    let mut screen = Screen::new(with_colored_words);
    let pixels = screen.pixels();
    let width = SIZE.0 as usize;
    let count = |range: std::ops::Range<usize>, wanted: [u8; 4]| {
        pixels
            .chunks_exact(4)
            .enumerate()
            .filter(|(at, pixel)| range.contains(&(at % width)) && **pixel == wanted)
            .count()
    };
    // 40 pixels per em, so each `A` is 24 wide.
    assert!(count(0..24, RED_PIXEL) > 50);
    assert_eq!(count(24..48, RED_PIXEL), 0);
    assert!(count(24..48, BLUE_PIXEL) > 50);
    assert_eq!(count(0..24, BLUE_PIXEL), 0);
}

fn with_underline(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window)
        .props(Paint::new().background(Color::WHITE))
        .child(
            Element::new(Role::Text)
                .prop("font_size", 10.0)
                .prop("text_color", RED.to_prop())
                .child(part("AB ").prop("underline", true))
                .child(part("AB")),
        )
}

#[test]
fn only_the_part_that_asks_for_it_is_underlined() {
    let mut screen = Screen::new(with_underline);
    // The baseline's at 8, and it's one below that.
    assert_eq!(screen.pixel(5, 9), RED_PIXEL);
    assert_eq!(screen.pixel(20, 9), WHITE_PIXEL);
}

/// "AB " and then a link of two words, on a line wide enough for two: the first line ends with
/// the link's first word, and it goes on at the start of the next.
fn with_link(cx: &mut Cx, _: &()) -> Element {
    let clicks = cx.state(|| 0);
    let link = Element::new(Role::Link)
        .key("the-link")
        .name("AB AB")
        .props(Paint::new().hover(BLUE));
    let link = cx.on(link, ActionKind::Click, move |cx, _| {
        cx.update(clicks, |clicks| *clicks += 1);
    });
    Element::new(Role::Window)
        .props(Paint::new().background(Color::WHITE))
        .child(
            Element::new(Role::Text)
                .key("words")
                .props(Style::new().width(25.0))
                .prop("font_size", 10.0)
                .child(part("AB "))
                .child(link),
        )
        .child(part(&format!("clicked {}", cx.get(clicks))).prop("font_size", 10.0))
}

#[test]
fn a_link_in_a_paragraph_gets_a_click_only_where_its_words_are() {
    let mut screen = Screen::new(with_link);
    screen.pixels();
    // On "AB ", which isn't the link.
    screen.click(5.0, 5.0);
    assert!(
        screen.app.outline().contains("clicked 0"),
        "{}",
        screen.app.outline()
    );
    // Its box goes around both of its lines, but the end of the second line is empty.
    screen.click(20.0, 16.0);
    assert!(
        screen.app.outline().contains("clicked 0"),
        "{}",
        screen.app.outline()
    );
    // Its first line.
    screen.click(18.0, 5.0);
    assert!(
        screen.app.outline().contains("clicked 1"),
        "{}",
        screen.app.outline()
    );
    // Its second.
    screen.click(5.0, 16.0);
    assert!(
        screen.app.outline().contains("clicked 2"),
        "{}",
        screen.app.outline()
    );
}

#[test]
fn the_pointer_shows_what_it_is_over_and_a_link_lights_up() {
    let mut screen = Screen::new(with_link);
    screen.pixels();
    screen.app.pointer_move(5.0, 5.0);
    screen.app.update();
    assert_eq!(screen.app.cursor(), Cursor::Arrow);
    screen.app.pointer_move(5.0, 16.0);
    screen.app.update();
    assert_eq!(screen.app.cursor(), Cursor::Hand);
    // A hover fades in, so it takes a moment to get all the way there.
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        screen.pixel(10, 21),
        BLUE_PIXEL,
        "it's lit up on the line it's on"
    );
}

#[test]
fn a_link_in_a_paragraph_can_have_the_keyboard() {
    let mut screen = Screen::new(with_link);
    screen.pixels();
    screen
        .app
        .key(rsui::KeyPress::new(rsui::KeyCode::Tab))
        .unwrap();
    screen.app.update();
    screen
        .app
        .key(rsui::KeyPress::new(rsui::KeyCode::Enter))
        .unwrap();
    screen.app.update();
    assert!(
        screen.app.outline().contains("clicked 1"),
        "{}",
        screen.app.outline()
    );
    // Drawing the ring around something with no box of its own doesn't go wrong.
    screen.pixels();
}

fn nested_and_scrolled(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window)
        .props(Paint::new().background(Color::WHITE))
        .child(
            Element::new(Role::Text)
                .props(Style::new().padding(10.0))
                .prop("font_size", 10.0)
                .child(
                    Element::new(Role::Text)
                        .prop("font_size", 20.0)
                        .child(part("A").props(Paint::new().background(RED))),
                ),
        )
}

#[test]
fn padding_moves_what_is_inside_and_looks_carry_down_through_several_levels() {
    let mut screen = Screen::new(nested_and_scrolled);
    assert_eq!(screen.pixel(5, 15), WHITE_PIXEL);
    assert_eq!(screen.pixel(11, 15), RED_PIXEL);
    // At 20 pixels per em, `A` is 12 wide.
    assert_eq!(screen.pixel(21, 15), RED_PIXEL);
    assert_eq!(screen.pixel(23, 15), WHITE_PIXEL);
}

fn with_selectable_words(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window)
        .props(Paint::new().background(Color::WHITE))
        .child(
            Element::new(Role::Text)
                .props(Style::new().width(100.0))
                .prop("font_size", 10.0)
                .name("AB CD AB")
                .selectable(),
        )
        .child(
            Element::new(Role::Text)
                .key("mixed")
                .prop("font_size", 10.0)
                .prop("selectable", true)
                .child(part("AB "))
                .child(part("CD").prop("font_size", 20.0)),
        )
        .child(part("AB").prop("font_size", 10.0))
}

fn drag(screen: &mut Screen, from: (f32, f32), to: (f32, f32)) {
    screen.app.pointer_move(from.0, from.1);
    screen.app.pointer_down(from.0, from.1, 1);
    screen.app.update();
    screen.app.pointer_move(to.0, to.1);
    screen.app.update();
    let _ = screen.app.pointer_up(to.0, to.1);
    screen.app.update();
}

#[test]
fn dragging_over_text_selects_it_and_the_pointer_says_it_can() {
    let mut screen = Screen::new(with_selectable_words);
    screen.pixels();
    // `A` is 6 wide, `B` 5, space 2.5, `C` 8 and `D` 7.
    drag(&mut screen, (0.0, 5.0), (11.0, 5.0));
    assert_eq!(screen.app.selected_text(), Some("AB"));
    assert_eq!(screen.app.cursor(), Cursor::Text);
    // And it's drawn behind the words, which go on to the right.
    assert_ne!(screen.pixel(10, 1), WHITE_PIXEL);
    assert_eq!(screen.pixel(30, 1), WHITE_PIXEL);
    // Backwards is the same, and spaces come along.
    drag(&mut screen, (28.5, 5.0), (0.0, 5.0));
    assert_eq!(screen.app.selected_text(), Some("AB CD"));
}

#[test]
fn clicking_selects_words_and_all_of_it_and_clicking_elsewhere_clears_it() {
    let mut screen = Screen::new(with_selectable_words);
    screen.pixels();
    let click = |screen: &mut Screen, clicks: u32, x: f32, y: f32| {
        screen.app.pointer_down(x, y, clicks);
        screen.app.update();
        let _ = screen.app.pointer_up(x, y);
        screen.app.update();
    };
    click(&mut screen, 1, 15.0, 5.0);
    assert_eq!(screen.app.selected_text(), None, "one click only places it");
    click(&mut screen, 2, 15.0, 5.0);
    assert_eq!(screen.app.selected_text(), Some("CD"));
    click(&mut screen, 3, 15.0, 5.0);
    assert_eq!(screen.app.selected_text(), Some("AB CD AB"));
    // The third piece of text can't be selected, so this leaves nothing.
    click(&mut screen, 1, 150.0, 90.0);
    assert_eq!(screen.app.selected_text(), None);
}

#[test]
fn what_is_selected_goes_across_parts_and_can_be_copied() {
    let mut screen = Screen::new(with_selectable_words);
    screen.pixels();
    let (mut x, mut y) = (0.0, 0.0);
    screen.app.layout().walk(|path, rect| {
        if path == [1] {
            (x, y) = (rect.x, rect.y);
        }
    });
    let middle = y + 11.0;
    drag(&mut screen, (x + 6.0, middle), (x + 13.5 + 16.0, middle));
    assert_eq!(screen.app.selected_text(), Some("B C"));
    let copied = Rc::new(RefCell::new(None));
    screen.app.set_clipboard(Shared(copied.clone()));
    screen
        .app
        .key(rsui::KeyPress::new(rsui::KeyCode::Character('c')).ctrl())
        .unwrap();
    assert_eq!(copied.borrow().as_deref(), Some("B C"));
    screen
        .app
        .key(rsui::KeyPress::new(rsui::KeyCode::Character('a')).ctrl())
        .unwrap();
    assert_eq!(screen.app.selected_text(), Some("AB CD"));
    screen
        .app
        .key(rsui::KeyPress::new(rsui::KeyCode::Escape))
        .unwrap();
    assert_eq!(screen.app.selected_text(), None);
}

fn with_words_that_change(cx: &mut Cx, _: &()) -> Element {
    let shown = cx.state(|| "AB CD");
    let words = Element::new(Role::Text)
        .prop("font_size", 10.0)
        .name(cx.get(shown))
        .selectable();
    let swap = Element::new(Role::Button)
        .key("swap")
        .name("swap")
        .prop("font_size", 10.0);
    let swap = cx.on(swap, ActionKind::Click, move |cx, _| cx.set(shown, "AB"));
    Element::new(Role::Window)
        .props(Paint::new().background(Color::WHITE))
        .child(words)
        .child(swap)
}

#[test]
fn a_selection_goes_when_the_words_it_was_in_change() {
    let mut screen = Screen::new(with_words_that_change);
    screen.pixels();
    drag(&mut screen, (0.0, 5.0), (28.0, 5.0));
    assert_eq!(screen.app.selected_text(), Some("AB CD"));
    // This goes through a key rather than the pointer, as a press on anything clears it anyway.
    screen
        .app
        .key(rsui::KeyPress::new(rsui::KeyCode::Tab))
        .unwrap();
    screen
        .app
        .key(rsui::KeyPress::new(rsui::KeyCode::Enter))
        .unwrap();
    screen.pixels();
    assert_eq!(screen.app.selected_text(), None);
}

#[test]
fn words_changing_while_they_are_being_selected_goes_no_further_than_losing_the_selection() {
    let mut screen = Screen::new(with_words_that_change);
    screen.pixels();
    screen.app.pointer_move(0.0, 5.0);
    screen.app.pointer_down(0.0, 5.0, 1);
    screen.app.update();
    screen.app.pointer_move(28.0, 5.0);
    assert_eq!(screen.app.selected_text(), Some("AB CD"));
    // With the button still down, nothing's been drawn since it changed.
    screen
        .app
        .key(rsui::KeyPress::new(rsui::KeyCode::Tab))
        .unwrap();
    screen
        .app
        .key(rsui::KeyPress::new(rsui::KeyCode::Enter))
        .unwrap();
    screen.app.update();
    screen.app.pointer_move(5.0, 5.0);
    assert_eq!(screen.app.selected_text(), None);
    screen
        .app
        .key(rsui::KeyPress::new(rsui::KeyCode::Character('c')).ctrl())
        .unwrap();
    screen.pixels();
}

#[test]
fn a_link_in_selectable_text_is_still_a_link() {
    fn view(cx: &mut Cx, _: &()) -> Element {
        let clicks = cx.state(|| 0);
        let link = Element::new(Role::Link).key("the-link").name("AB");
        let link = cx.on(link, ActionKind::Click, move |cx, _| {
            cx.update(clicks, |clicks| *clicks += 1);
        });
        Element::new(Role::Window)
            .child(
                Element::new(Role::Text)
                    .prop("font_size", 10.0)
                    .prop("selectable", true)
                    .child(part("AB "))
                    .child(link),
            )
            .child(part(&format!("clicked {}", cx.get(clicks))))
    }
    let mut screen = Screen::new(view);
    screen.pixels();
    screen.click(18.0, 5.0);
    assert!(
        screen.app.outline().contains("clicked 1"),
        "{}",
        screen.app.outline()
    );
    assert_eq!(screen.app.selected_text(), None);
    screen.click(1.0, 5.0);
    assert!(
        screen.app.outline().contains("clicked 1"),
        "{}",
        screen.app.outline()
    );
}

fn colored_from_around(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window)
        .props(Paint::new().background(Color::WHITE).text(RED))
        .child(part("AB").prop("font_size", 40.0))
        .child(
            Element::new(Role::Group)
                .props(Paint::new().text(BLUE))
                .child(part("AB").prop("font_size", 40.0)),
        )
        .child(part("AB").prop("font_size", 40.0))
}

fn count_pixels(pixels: &[u8], rows: std::ops::Range<usize>, wanted: [u8; 4]) -> usize {
    pixels
        .chunks_exact(4)
        .enumerate()
        .filter(|(at, pixel)| rows.contains(&(at / SIZE.0 as usize)) && **pixel == wanted)
        .count()
}

#[test]
fn text_with_no_color_takes_it_from_what_it_is_in() {
    let mut screen = Screen::new(colored_from_around);
    let pixels = screen.pixels();
    // Lines of this size are 44 apart.
    assert!(count_pixels(&pixels, 0..44, RED_PIXEL) > 50, "the window's");
    assert_eq!(count_pixels(&pixels, 0..44, BLUE_PIXEL), 0);
    assert!(
        count_pixels(&pixels, 44..88, BLUE_PIXEL) > 50,
        "the group's"
    );
    assert_eq!(count_pixels(&pixels, 44..88, RED_PIXEL), 0);
    assert!(
        count_pixels(&pixels, 88..132, RED_PIXEL) > 50,
        "and back again"
    );
}

fn colored_above_an_overlay(_: &mut Cx, _: &()) -> Element {
    Element::new(Role::Window)
        .props(Paint::new().background(Color::WHITE).text(BLUE))
        .child(
            Element::new(Role::Group).child(
                Element::new(Role::Group)
                    .prop("overlay", "fill")
                    .child(part("AB").prop("font_size", 40.0)),
            ),
        )
}

#[test]
fn what_is_on_top_of_everything_takes_its_color_from_what_it_is_in_too() {
    let mut screen = Screen::new(colored_above_an_overlay);
    let pixels = screen.pixels();
    assert!(count_pixels(&pixels, 0..140, BLUE_PIXEL) > 50);
}
