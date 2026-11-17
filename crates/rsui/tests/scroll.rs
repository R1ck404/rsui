//! Scrolling areas: from the wheel, the keyboard and the scrollbar to what's on screen.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use proptest::prelude::*;
use rsui::prelude::*;
use rsui::render::Renderer;
use rsui::text::TextEngine;
use rsui::{App, KeyCode, KeyPress};
use support::plain_font;

const SIZE: (u32, u32) = (200, 100);
const RED: Color = Color::hex(0xFF0000);
const BLUE: Color = Color::hex(0x0000FF);
const RED_PIXEL: [u8; 4] = [255, 0, 0, 255];
const BLUE_PIXEL: [u8; 4] = [0, 0, 255, 255];

fn block(key: String, height: f32, color: Color) -> Element {
    Element::new(Role::Button)
        .key(key)
        .props(
            rsui::core::Props::from(Style::new().height(height))
                .merged(&Paint::new().background(color).into()),
        )
        .action(ActionKind::Click)
}

/// A vertical area filling the window, holding ten blocks 50 tall, red then blue.
fn blocks(cx: &mut Cx, _: &()) -> Element {
    let mut content = Element::new(Role::Group).props(style!(Style::new().flex(1.0)));
    for index in 0..10 {
        let color = if index % 2 == 0 { RED } else { BLUE };
        content = content.child(block(format!("b{index}"), 50.0, color));
    }
    Element::new(Role::Window).child(cx.scroll_area("area", ScrollAxis::Vertical, content))
}

const AREA: &[u32] = &[0];

struct Screen {
    app: App,
    renderer: Renderer,
}

impl Screen {
    fn new(render: fn(&mut Cx, &()) -> Element) -> Self {
        let mut screen = Self {
            app: App::new(render, (), TextEngine::new(plain_font()), SIZE, 1.0),
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

    fn moved(&mut self, path: &[u32]) -> f32 {
        self.pixels();
        let root = self.app.runtime().root().clone();
        let node = path
            .iter()
            .fold(&root, |node, index| &node.children[*index as usize]);
        let info = self
            .app
            .layout()
            .scroll_info(path, node)
            .expect("it scrolls");
        info.y.max(info.x)
    }

    fn press(&mut self, code: KeyCode) {
        self.app.key(KeyPress::new(code)).expect("it goes through");
        self.app.update();
    }

    fn wheel(&mut self, rows: i64) {
        self.app.scroll(50.0, 50.0, rows).expect("it goes through");
        self.app.update();
    }
}

#[test]
fn what_goes_past_the_edge_is_cut_off_and_scrolling_brings_it_in() {
    let mut screen = Screen::new(blocks);
    assert_eq!(screen.pixel(50, 25), RED_PIXEL);
    assert_eq!(screen.pixel(50, 75), BLUE_PIXEL);
    assert_eq!(screen.moved(AREA), 0.0);
    // Three rows make a notch of 100.
    screen.wheel(3);
    assert!((screen.moved(AREA) - 100.0).abs() < 0.5);
    assert_eq!(screen.pixel(50, 25), RED_PIXEL, "the third block");
    screen.wheel(-1);
    let moved = screen.moved(AREA);
    assert!((moved - 67.0).abs() < 0.5, "{moved}");
}

#[test]
fn it_stops_at_both_ends() {
    let mut screen = Screen::new(blocks);
    screen.wheel(-5);
    assert_eq!(screen.moved(AREA), 0.0);
    screen.wheel(1000);
    assert_eq!(screen.moved(AREA), 400.0);
    assert_eq!(screen.pixel(50, 75), BLUE_PIXEL, "the last block");
    // Whatever someone sets it to, it shows only what's there.
    let input = Some(PropValue::Float(1e9));
    screen
        .app
        .runtime_mut()
        .dispatch(AREA, ActionKind::Scroll, input)
        .unwrap();
    assert_eq!(screen.moved(AREA), 400.0);
    screen.wheel(-3);
    assert_eq!(screen.moved(AREA), 300.0, "it goes back from what shows");
}

#[test]
fn several_notches_before_a_frame_all_count() {
    let mut screen = Screen::new(blocks);
    for _ in 0..3 {
        screen.app.scroll(50.0, 50.0, 3).unwrap();
    }
    assert!((screen.moved(AREA) - 300.0).abs() < 1.0);
}

#[test]
fn the_keyboard_moves_it() {
    let mut screen = Screen::new(blocks);
    screen.app.focus_on(AREA);
    screen.press(KeyCode::Down);
    assert_eq!(screen.moved(AREA), 40.0);
    screen.press(KeyCode::PageDown);
    assert_eq!(screen.moved(AREA), 100.0);
    screen.press(KeyCode::End);
    assert_eq!(screen.moved(AREA), 400.0);
    screen.press(KeyCode::PageUp);
    assert_eq!(screen.moved(AREA), 340.0);
    screen.press(KeyCode::Up);
    assert_eq!(screen.moved(AREA), 300.0);
    screen.press(KeyCode::Home);
    assert_eq!(screen.moved(AREA), 0.0);
}

#[test]
fn clicks_land_on_what_shows_there() {
    let mut screen = Screen::new(blocks);
    screen.wheel(1000);
    screen.pixels();
    // Click where the last block now is, and press the first one's old spot.
    let path = screen.app.layout().hit_test(50.0, 75.0).expect("something");
    assert_eq!(path, [0, 9]);
    let path = screen.app.layout().hit_test(50.0, 25.0).expect("something");
    assert_eq!(path, [0, 8]);
}

#[test]
fn tabbing_to_something_brings_it_into_view() {
    let mut screen = Screen::new(blocks);
    // The area first, then each block in it.
    for index in 0..6 {
        screen.press(KeyCode::Tab);
        let rect_bottom = if index == 0 {
            0.0
        } else {
            screen.pixels();
            let rect = screen
                .app
                .layout()
                .rect(&[0, index - 1])
                .expect("it's there");
            assert!(rect.y >= 0.0, "{index}: starts at {}", rect.y);
            rect.y + rect.height
        };
        assert!(rect_bottom <= 100.0, "{index}: ends at {rect_bottom}");
    }
    assert!(screen.moved(AREA) > 0.0);
    // And going back brings the first back.
    for _ in 0..5 {
        screen.app.key(KeyPress::new(KeyCode::Tab).shift()).unwrap();
        screen.app.update();
    }
    assert_eq!(screen.moved(AREA), 0.0);
}

#[test]
fn the_scrollbar_shows_where_it_is_and_can_be_dragged() {
    let mut screen = Screen::new(blocks);
    // The thumb covers the top fifth of the 100 high track, on the right.
    let thumb = |pixel: [u8; 4]| pixel != RED_PIXEL && pixel != BLUE_PIXEL;
    assert!(thumb(screen.pixel(195, 10)));
    assert!(!thumb(screen.pixel(195, 60)));
    assert!(!thumb(screen.pixel(50, 10)));

    // Grabbing it where it is doesn't move it. 24 is the shortest it gets, and it's 20 here.
    screen.app.pointer_down(195.0, 10.0, 1);
    assert_eq!(screen.moved(AREA), 0.0);
    // Dragging all the way goes all the way.
    screen.app.pointer_move(195.0, 500.0);
    assert_eq!(screen.moved(AREA), 400.0);
    assert!(thumb(screen.pixel(195, 90)));
    screen.app.pointer_up(195.0, 500.0).unwrap();
    // It's let go.
    screen.app.pointer_move(195.0, 10.0);
    assert_eq!(screen.moved(AREA), 400.0);

    // Pressing on the track jumps there.
    screen.app.pointer_down(195.0, 12.0, 1);
    screen.app.pointer_up(195.0, 12.0).unwrap();
    assert_eq!(screen.moved(AREA), 0.0);
}

#[test]
fn pressing_the_scrollbar_does_not_press_what_is_under_it() {
    let mut screen = Screen::new(blocks);
    screen.app.pointer_down(195.0, 10.0, 1);
    screen.app.pointer_up(195.0, 10.0).unwrap();
    assert_eq!(screen.app.focused(), None);
}

/// One area inside another, each taking half the window and holding too much.
fn nested(cx: &mut Cx, _: &()) -> Element {
    fn tall(key: &str, height: f32) -> Element {
        Element::new(Role::Group).child(block(key.into(), height, RED))
    }
    let inner_content = tall("inner_block", 200.0).props(style!(Style::new().height(50.0)));
    let inner = cx.scroll_area("inner", ScrollAxis::Vertical, inner_content);
    let outer_content = Element::new(Role::Group)
        .props(style!(Style::new().flex(1.0)))
        .child(inner)
        .child(block("rest".into(), 300.0, BLUE));
    Element::new(Role::Window).child(cx.scroll_area("outer", ScrollAxis::Vertical, outer_content))
}

#[test]
fn reaching_the_end_of_an_inner_area_carries_on_with_the_outer_one() {
    let mut screen = Screen::new(nested);
    const OUTER: &[u32] = &[0];
    const INNER: &[u32] = &[0, 0];
    screen.app.scroll(50.0, 20.0, 3).unwrap();
    screen.app.update();
    assert!((screen.moved(INNER) - 100.0).abs() < 0.5);
    assert_eq!(screen.moved(OUTER), 0.0);
    screen.app.scroll(50.0, 20.0, 3).unwrap();
    screen.app.update();
    assert_eq!(screen.moved(INNER), 150.0);
    assert_eq!(screen.moved(OUTER), 0.0);
    // That one's done, so this goes to the outer one.
    screen.app.scroll(50.0, 20.0, 3).unwrap();
    screen.app.update();
    assert_eq!(screen.moved(INNER), 150.0);
    assert!(screen.moved(OUTER) > 0.0);
}

#[test]
fn it_goes_sideways_too() {
    fn sideways(cx: &mut Cx, _: &()) -> Element {
        let mut content = Element::new(Role::Group).props(style!(Style::row().flex(1.0)));
        for index in 0..4 {
            let width = Style::new().width(100.0);
            let color = if index % 2 == 0 { RED } else { BLUE };
            content = content.child(Element::new(Role::Group).props(
                rsui::core::Props::from(width).merged(&Paint::new().background(color).into()),
            ));
        }
        Element::new(Role::Window)
            .props(style!(Style::row()))
            .child(cx.scroll_area("across", ScrollAxis::Horizontal, content))
    }
    let mut screen = Screen::new(sideways);
    assert_eq!(screen.pixel(150, 50), BLUE_PIXEL);
    screen.app.focus_on(AREA);
    screen.press(KeyCode::Right);
    assert_eq!(screen.moved(AREA), 40.0);
    screen.press(KeyCode::End);
    assert_eq!(screen.moved(AREA), 200.0);
    assert_eq!(screen.pixel(50, 50), RED_PIXEL, "the third");
    screen.wheel(-3);
    assert_eq!(screen.moved(AREA), 100.0);
}

#[test]
fn content_that_fits_has_nothing_to_scroll() {
    fn short(cx: &mut Cx, _: &()) -> Element {
        let content = Element::new(Role::Group)
            .props(style!(Style::new().flex(1.0)))
            .child(block("only".into(), 50.0, RED));
        Element::new(Role::Window).child(cx.scroll_area("area", ScrollAxis::Vertical, content))
    }
    let mut screen = Screen::new(short);
    screen.wheel(3);
    assert_eq!(screen.moved(AREA), 0.0);
    assert_eq!(screen.pixel(195, 10), RED_PIXEL, "no scrollbar");
    screen.app.pointer_down(195.0, 10.0, 1);
    screen.app.pointer_move(195.0, 90.0);
    assert_eq!(screen.moved(AREA), 0.0);
}

#[test]
fn a_smaller_window_keeps_the_scroll_within_what_there_is() {
    let mut screen = Screen::new(blocks);
    screen.wheel(1000);
    assert_eq!(screen.moved(AREA), 400.0);
    screen.app.resize((200, 500), 1.0);
    assert_eq!(screen.moved(AREA), 0.0, "it all fits now");
    screen.app.resize((200, 100), 1.0);
    // It remembers where it was asked to be.
    assert_eq!(screen.moved(AREA), 400.0);
}

#[test]
fn scrolled_text_input_keeps_working() {
    fn with_input(cx: &mut Cx, _: &()) -> Element {
        let value = cx.state(Text::default);
        let input = text_input(cx, "typed", "Typed", "", value);
        let content = Element::new(Role::Group)
            .props(style!(Style::new().flex(1.0)))
            .child(block("above".into(), 300.0, RED))
            .child(input);
        Element::new(Role::Window).child(cx.scroll_area("area", ScrollAxis::Vertical, content))
    }
    let mut screen = Screen::new(with_input);
    screen.app.focus_on(&[0, 1]);
    assert!(screen.moved(AREA) > 0.0, "focusing it brought it into view");
    screen.wheel(-1000);
    assert_eq!(screen.moved(AREA), 0.0);
    screen.app.type_text("A").unwrap();
    assert!(screen.moved(AREA) > 0.0, "typing brought it back");
    assert!(screen.app.outline().contains("value=\"A\""));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// However it's moved, it stays in range, and what's on screen is what a new one would draw at
    /// that spot.
    #[test]
    fn scrolling_always_lands_in_range_and_draws_the_same_as_starting_there(
        moves in proptest::collection::vec(-30i64..30, 1..12),
    ) {
        let mut screen = Screen::new(blocks);
        for rows in moves {
            screen.wheel(rows);
            let moved = screen.moved(AREA);
            prop_assert!((0.0..=400.0).contains(&moved), "{moved}");
        }
        let moved = screen.moved(AREA);
        let seen = screen.pixels();
        let mut fresh = Screen::new(blocks);
        let input = Some(PropValue::Float(f64::from(moved)));
        fresh.app.runtime_mut().dispatch(AREA, ActionKind::Scroll, input).unwrap();
        prop_assert!(seen == fresh.pixels(), "at {}", moved);
    }
}

#[test]
fn it_lands_on_whole_pixels_so_text_does_not_shimmer() {
    let mut app = App::new(blocks, (), TextEngine::new(plain_font()), SIZE, 1.25);
    app.update();
    app.scene();
    for rows in [1, 1, 1, 2, -1] {
        app.scroll(50.0, 50.0, rows).unwrap();
        app.update();
        app.scene();
        let root = app.runtime().root().clone();
        let moved = app
            .layout()
            .scroll_info(AREA, &root.children[0])
            .expect("it scrolls")
            .y;
        let physical = moved * 1.25;
        assert!((physical - physical.round()).abs() < 0.001, "{physical}");
    }
}

#[test]
fn a_thumb_never_gets_too_short_to_grab_and_stays_on_its_track() {
    use rsui::render::{scroll_for_thumb, scroll_thumb};
    assert_eq!(scroll_thumb(100.0, 100.0, 0.0), None);
    assert_eq!(scroll_thumb(100.0, 50.0, 0.0), None);
    let (start, length) = scroll_thumb(100.0, 30_000_000.0, 0.0).expect("it scrolls");
    assert_eq!((start, length), (0.0, 24.0));
    let (start, _) = scroll_thumb(100.0, 30_000_000.0, 1e12).expect("it scrolls");
    assert_eq!(start, 76.0);
    assert_eq!(scroll_for_thumb(100.0, 200.0, -5.0), 0.0);
    assert_eq!(scroll_for_thumb(100.0, 200.0, 1e6), 100.0);
    assert_eq!(scroll_for_thumb(100.0, 200.0, 25.0), 50.0);
}

#[test]
fn a_list_has_a_scrollbar_too() {
    fn long(cx: &mut Cx, _: &()) -> Element {
        let list = cx.virtual_list(
            "rows",
            ListWindow {
                len: 1000,
                visible: 4,
                overscan: 0,
                reveal: None,
            },
            |_, index| {
                Element::new(Role::ListItem)
                    .name(text!("{index}"))
                    .into_node()
            },
        );
        let props = rsui::core::Props::from(Style::new().row_height(25.0).flex(1.0));
        let list = Element {
            props: props.merged(&list.props),
            ..list
        };
        Element::new(Role::Window).child(list)
    }
    let mut screen = Screen::new(long);
    let empty = screen.pixel(195, 90);
    assert_ne!(screen.pixel(195, 10), empty, "the thumb is at the top");
    screen.app.pointer_down(195.0, 10.0, 1);
    screen.app.pointer_move(195.0, 500.0);
    screen.app.pointer_up(195.0, 500.0).unwrap();
    screen.pixels();
    assert!(
        screen.app.outline().contains("996"),
        "{}",
        screen.app.outline()
    );
    assert_ne!(screen.pixel(195, 90), empty, "and it's gone to the bottom");
    assert_eq!(screen.pixel(195, 10), empty);
}

#[test]
fn nothing_shows_past_the_edge_of_an_area_that_does_not_fill_the_window() {
    fn half(cx: &mut Cx, _: &()) -> Element {
        let mut content = Element::new(Role::Group).props(style!(Style::new().height(50.0)));
        for index in 0..4 {
            content = content.child(block(format!("b{index}"), 50.0, RED));
        }
        Element::new(Role::Window).child(cx.scroll_area("area", ScrollAxis::Vertical, content))
    }
    let mut screen = Screen::new(half);
    assert_eq!(screen.pixel(50, 25), RED_PIXEL);
    assert_ne!(
        screen.pixel(50, 75),
        RED_PIXEL,
        "the next block is past the edge"
    );
    // One that's partly in view is only drawn as far as the edge. The wheel has to be over the area.
    screen.app.scroll(50.0, 25.0, 1).unwrap();
    screen.app.update();
    assert_eq!(screen.pixel(50, 45), RED_PIXEL);
    assert_ne!(
        screen.pixel(50, 55),
        RED_PIXEL,
        "the rest of it is past the edge"
    );
}

#[test]
fn grabbing_the_thumb_away_from_its_middle_does_not_make_it_jump() {
    let mut screen = Screen::new(blocks);
    screen.wheel(3);
    let moved = screen.moved(AREA);
    // The thumb starts about a fifth of the way down, and this is near its bottom end.
    screen.app.pointer_down(195.0, 40.0, 1);
    let after = screen.moved(AREA);
    assert!((after - moved).abs() < 1.0, "{moved} to {after}");
    screen.app.pointer_up(195.0, 40.0).unwrap();
}
