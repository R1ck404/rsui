//! What goes on top of everything else: menus, dialogs and tooltips, drawn offscreen.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use rsui::prelude::*;
use rsui::render::Renderer;
use rsui::text::TextEngine;
use rsui::{App, Frame, KeyCode, KeyPress};
use support::plain_font;

const SIZE: (u32, u32) = (300, 200);
const RED: Color = Color::hex(0xFF0000);
const BLUE: Color = Color::hex(0x0000FF);
const RED_PIXEL: [u8; 4] = [255, 0, 0, 255];
const BLUE_PIXEL: [u8; 4] = [0, 0, 255, 255];

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
        // Moving the keyboard around can change state, which then needs another go.
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
        self.app.key(KeyPress::new(code)).expect("it goes through");
        self.pixels();
    }

    fn click(&mut self, x: f32, y: f32) {
        self.app.pointer_down(x, y, 1);
        self.app.pointer_up(x, y).expect("the click goes through");
        self.pixels();
    }

    fn outline(&self) -> String {
        self.app.outline()
    }

    fn shows(&self, text: &str) -> bool {
        self.outline().contains(text)
    }
}

fn square(key: &str, size: f32, color: Color) -> Element {
    Element::new(Role::Group).key(key).props(
        rsui::core::Props::from(Style::new().width(size).height(size))
            .merged(&Paint::new().background(color).into()),
    )
}

/// `square` in a holder at a position in the window, with an overlay of this placement inside it.
fn placed(x: f32, y: f32, placement: Placement) -> Element {
    let overlay = Element::new(Role::Group).key("over").props(
        rsui::core::Props::from(Style::new().overlay(placement).width(40.0).height(30.0))
            .merged(&Paint::new().background(BLUE).into()),
    );
    let holder = Element::new(Role::Group)
        .key("holder")
        .props(
            rsui::core::Props::from(Style::new().width(50.0).height(20.0))
                .merged(&Paint::new().background(RED).into()),
        )
        .child(overlay);
    let spacer = Element::new(Role::Group)
        .key("spacer")
        .props(Style::new().height(y));
    let line = Element::new(Role::Group)
        .key("line")
        .props(Style::row())
        .child(
            Element::new(Role::Group)
                .key("left")
                .props(Style::new().width(x)),
        )
        .child(holder);
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start)))
        .child(spacer)
        .child(line)
}

/// Where the overlay of this placement ends up, with its holder at this position.
fn overlay_at(x: f32, y: f32, placement: Placement) -> (f32, f32) {
    let root: Node = placed(x, y, placement).into_node();
    let mut layout = rsui::layout::Layout::new();
    layout.compute(
        &root,
        rsui::layout::Size::new(300.0, 200.0),
        &mut rsui::layout::FixedMetrics::default(),
    );
    let rect = layout.rect(&[1, 1, 0]).expect("it's laid out");
    (rect.x, rect.y)
}

#[test]
fn it_goes_where_it_is_told_and_where_there_is_room() {
    // The holder is 50 by 20, and the overlay is 40 by 30.
    assert_eq!(overlay_at(100.0, 50.0, Placement::Below), (100.0, 70.0));
    assert_eq!(overlay_at(100.0, 50.0, Placement::Above), (100.0, 20.0));
    assert_eq!(overlay_at(100.0, 50.0, Placement::Right), (150.0, 50.0));
    assert_eq!(overlay_at(100.0, 50.0, Placement::Left), (60.0, 50.0));
    assert_eq!(overlay_at(100.0, 50.0, Placement::Center), (130.0, 85.0));
    assert_eq!(overlay_at(100.0, 50.0, Placement::At(7.0, 9.0)), (7.0, 9.0));
    // With no room there, it goes to the other side.
    assert_eq!(overlay_at(100.0, 170.0, Placement::Below), (100.0, 140.0));
    assert_eq!(overlay_at(100.0, 10.0, Placement::Above), (100.0, 30.0));
    assert_eq!(overlay_at(250.0, 50.0, Placement::Right), (210.0, 50.0));
    assert_eq!(overlay_at(20.0, 50.0, Placement::Left), (70.0, 50.0));
    // It stays in the window along the other direction, too.
    assert_eq!(overlay_at(280.0, 50.0, Placement::Below), (260.0, 70.0));
    assert_eq!(overlay_at(100.0, 190.0, Placement::Right), (150.0, 170.0));
    assert_eq!(
        overlay_at(100.0, 50.0, Placement::At(999.0, 999.0)),
        (260.0, 170.0)
    );
}

#[test]
fn it_takes_up_no_room_where_it_is() {
    let row = |with_overlay: bool| -> Node {
        let mut holder = Element::new(Role::Group)
            .props(style!(Style::new().gap(10.0)))
            .child(square("a", 10.0, RED));
        if with_overlay {
            holder =
                holder.child(Element::new(Role::Group).props(
                    style!(Style::new().overlay(Placement::Below).width(99.0).height(99.0)),
                ));
        }
        holder = holder.child(square("b", 10.0, RED));
        Element::new(Role::Window)
            .props(style!(Style::new().align(Align::Start)))
            .child(holder)
            .into_node()
    };
    let layout = |root: &Node| {
        let mut layout = rsui::layout::Layout::new();
        layout.compute(
            root,
            rsui::layout::Size::new(300.0, 200.0),
            &mut rsui::layout::FixedMetrics::default(),
        );
        layout
    };
    let (without, with) = (row(false), row(true));
    let (without, with_layout) = (layout(&without), layout(&with));
    assert_eq!(
        with_layout.rect(&[0]),
        without.rect(&[0]),
        "the holder's size"
    );
    assert_eq!(
        with_layout.rect(&[0, 2]),
        without.rect(&[0, 1]),
        "what comes after"
    );
}

#[test]
fn an_overlay_that_fills_the_window_does_so_from_anywhere() {
    let (x, y) = overlay_at(100.0, 50.0, Placement::Fill);
    assert_eq!((x, y), (0.0, 0.0));
}

/// A scrolling area cut off at 50 high, holding a holder with something below it that's outside.
fn cut_off(cx: &mut Cx, _: &()) -> Element {
    let overlay = Element::new(Role::Group).key("over").props(
        rsui::core::Props::from(
            Style::new()
                .overlay(Placement::Below)
                .width(40.0)
                .height(30.0),
        )
        .merged(&Paint::new().background(BLUE).into()),
    );
    let holder = square("holder", 40.0, RED).child(overlay);
    let content = Element::new(Role::Group)
        .props(style!(Style::new().height(50.0).align(Align::Start)))
        .child(holder);
    let area = cx.scroll_area("area", ScrollAxis::Vertical, content);
    let after = square("after", 40.0, RED);
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start)))
        .child(area)
        .child(after)
}

#[test]
fn it_shows_over_what_comes_after_and_is_not_cut_off() {
    let mut screen = Screen::new(cut_off);
    // The overlay runs from 40 to 70, past the area's edge at 50, and over `after`, at 50 to 90.
    assert_eq!(screen.pixel(10, 45), BLUE_PIXEL);
    assert_eq!(
        screen.pixel(10, 65),
        BLUE_PIXEL,
        "past where its area ends, on top"
    );
    assert_eq!(
        screen.pixel(10, 80),
        RED_PIXEL,
        "what's under it carries on"
    );
    // And it's what a pointer finds there.
    assert_eq!(
        screen.app.layout().hit_test(10.0, 65.0),
        Some(vec![0, 0, 0])
    );
    assert_eq!(screen.app.layout().hit_test(10.0, 80.0), Some(vec![1]));
}

/// A menu with two items, a button that's not in it, and something that notes what happened.
fn with_menu(cx: &mut Cx, _: &()) -> Element {
    let open = cx.state(|| false);
    let said = cx.state(Text::default);
    let items: Vec<(Text, MenuAction)> = vec![
        (
            Text::from("First"),
            Box::new(move |cx| cx.set(said, Text::from("chose first"))),
        ),
        (
            Text::from("Second"),
            Box::new(move |cx| cx.set(said, Text::from("chose second"))),
        ),
    ];
    let menu = menu_button(cx, "file", "File", open, items);
    let other = Element::new(Role::Button)
        .key("other")
        .name("Other")
        .props(style!(Style::new().width(100.0).height(40.0)));
    let other = cx.on(other, ActionKind::Click, move |cx, _| {
        cx.set(said, Text::from("clicked other"));
    });
    let root = Element::new(Role::Window)
        .key("root")
        .name(cx.get(said))
        .props(style!(Style::new().align(Align::Start).gap(100.0)))
        .child(menu)
        .child(other);
    cx.on(root, ActionKind::KeyPress, move |cx, input| {
        if let Some(PropValue::Str(key)) = input {
            cx.set(said, rsui::core::text!("key {}", key.as_str()));
        }
    })
}

#[test]
fn a_menu_opens_and_closes_with_its_button() {
    let mut screen = Screen::new(with_menu);
    assert!(!screen.shows("First"));
    screen.click(10.0, 10.0);
    assert!(screen.shows("First"), "{}", screen.outline());
    // Pressing it again only closes it, rather than closing and opening.
    screen.click(10.0, 10.0);
    assert!(!screen.shows("First"), "{}", screen.outline());
}

#[test]
fn clicking_an_item_does_what_it_does_and_closes_the_menu() {
    let mut screen = Screen::new(with_menu);
    screen.click(10.0, 10.0);
    let path = screen
        .app
        .layout()
        .overlays()
        .pop()
        .expect("the menu is up");
    let rect = screen
        .app
        .layout()
        .rect(&[path.as_slice(), &[1]].concat())
        .expect("the second item");
    screen.click(rect.x + 5.0, rect.y + 5.0);
    assert!(screen.shows("chose second"), "{}", screen.outline());
    assert!(!screen.shows("First"), "{}", screen.outline());
}

#[test]
fn clicking_away_closes_it_and_the_click_still_counts() {
    let mut screen = Screen::new(with_menu);
    screen.click(10.0, 10.0);
    assert!(screen.shows("First"));
    let other = screen.app.layout().rect(&[1]).expect("it's there");
    screen.click(other.x + 5.0, other.y + 5.0);
    assert!(!screen.shows("First"), "{}", screen.outline());
    assert!(screen.shows("clicked other"), "{}", screen.outline());
}

#[test]
fn clicking_blank_space_in_a_menu_leaves_it_alone() {
    let mut screen = Screen::new(with_menu);
    screen.click(10.0, 10.0);
    let path = screen
        .app
        .layout()
        .overlays()
        .pop()
        .expect("the menu is up");
    let rect = screen.app.layout().rect(&path).expect("it's there");
    // Its padding, which isn't on any item.
    screen.click(rect.x + 1.0, rect.y + 1.0);
    assert!(screen.shows("First"), "{}", screen.outline());
}

#[test]
fn the_keyboard_goes_into_a_menu_and_through_it_and_back() {
    let mut screen = Screen::new(with_menu);
    // To the button, then open it.
    screen.press(KeyCode::Tab);
    let button = screen.app.focused().map(<[u32]>::to_vec);
    screen.press(KeyCode::Enter);
    assert!(screen.shows("First"));
    let menu = screen
        .app
        .layout()
        .overlays()
        .pop()
        .expect("the menu is up");
    let first = [menu.as_slice(), &[0]].concat();
    let second = [menu.as_slice(), &[1]].concat();
    assert_eq!(
        screen.app.focused(),
        Some(first.as_slice()),
        "it started on the first"
    );
    screen.press(KeyCode::Down);
    assert_eq!(screen.app.focused(), Some(second.as_slice()));
    screen.press(KeyCode::Down);
    assert_eq!(
        screen.app.focused(),
        Some(first.as_slice()),
        "around to the top"
    );
    screen.press(KeyCode::Up);
    assert_eq!(
        screen.app.focused(),
        Some(second.as_slice()),
        "around to the bottom"
    );
    screen.press(KeyCode::Home);
    assert_eq!(screen.app.focused(), Some(first.as_slice()));
    screen.press(KeyCode::End);
    assert_eq!(screen.app.focused(), Some(second.as_slice()));
    // Tab stays in it.
    screen.press(KeyCode::Tab);
    assert_eq!(screen.app.focused(), Some(first.as_slice()));
    // Choosing goes back to where the keyboard was.
    screen.press(KeyCode::Down);
    screen.press(KeyCode::Enter);
    assert!(screen.shows("chose second"), "{}", screen.outline());
    assert!(!screen.shows("First"));
    assert_eq!(screen.app.focused().map(<[u32]>::to_vec), button);
}

#[test]
fn escape_closes_it_and_the_keyboard_goes_back() {
    let mut screen = Screen::new(with_menu);
    screen.press(KeyCode::Tab);
    let button = screen.app.focused().map(<[u32]>::to_vec);
    screen.press(KeyCode::Enter);
    screen.press(KeyCode::Escape);
    assert!(!screen.shows("First"));
    assert_eq!(screen.app.focused().map(<[u32]>::to_vec), button);
    // With nothing up, it's a key like any other.
    screen.press(KeyCode::Escape);
    assert!(screen.shows("key escape"), "{}", screen.outline());
}

#[test]
fn keys_do_not_reach_what_is_behind_a_menu() {
    let mut screen = Screen::new(with_menu);
    screen.press(KeyCode::Tab);
    screen.press(KeyCode::Enter);
    screen.press(KeyCode::Character('x'));
    assert!(!screen.shows("key x"), "{}", screen.outline());
}

fn with_select(cx: &mut Cx, _: &()) -> Element {
    let chosen = cx.state(|| 1usize);
    let select = select(cx, "fruit", &["Apple", "Pear", "Plum"], chosen);
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start)))
        .child(select)
}

#[test]
fn a_select_shows_what_is_chosen_and_changes_it() {
    let mut screen = Screen::new(with_select);
    assert!(screen.shows("\"Pear\""), "{}", screen.outline());
    assert!(!screen.shows("Plum"));
    screen.press(KeyCode::Tab);
    screen.press(KeyCode::Enter);
    assert!(screen.shows("Plum"));
    screen.press(KeyCode::End);
    screen.press(KeyCode::Enter);
    assert!(!screen.shows("Pear"), "{}", screen.outline());
    assert!(screen.shows("\"Plum\""), "{}", screen.outline());
}

/// A button that opens a dialog with a text input and a close button, over a scrolling area.
fn with_dialog(cx: &mut Cx, _: &()) -> Element {
    let open = cx.state(|| false);
    let value = cx.state(Text::default);
    let said = cx.state(Text::default);
    let opener = Element::new(Role::Button)
        .key("opener")
        .name("Open")
        .props(style!(Style::new().width(100.0).height(30.0)));
    let opener = cx.on(opener, ActionKind::Click, move |cx, _| cx.set(open, true));
    let input = text_input(cx, "typed", "Typed", "", value);
    let close = Element::new(Role::Button)
        .key("close")
        .name("Close")
        .props(style!(Style::new().padding(4.0)));
    let close = cx.on(close, ActionKind::Click, move |cx, _| cx.set(open, false));
    let content = Element::new(Role::Group).child(input).child(close);
    let shown = dialog(cx, "asking", "A question", open, content);
    let behind = Element::new(Role::Group)
        .props(style!(Style::new().flex(1.0)))
        .child(square("tall", 500.0, RED));
    let behind = cx.scroll_area("behind", ScrollAxis::Vertical, behind);
    let root = Element::new(Role::Window)
        .key("root")
        .name(cx.get(said))
        .child(opener)
        .child(behind)
        .children(shown);
    cx.on(root, ActionKind::KeyPress, move |cx, input| {
        if let Some(PropValue::Str(key)) = input {
            cx.set(said, rsui::core::text!("key {}", key.as_str()));
        }
    })
}

const BEHIND: &[u32] = &[1];

#[test]
fn a_dialog_takes_the_keyboard_and_gives_it_back() {
    let mut screen = Screen::new(with_dialog);
    screen.press(KeyCode::Tab);
    assert_eq!(screen.app.focused(), Some(&[0u32][..]));
    screen.press(KeyCode::Enter);
    assert!(screen.shows("dialog"), "{}", screen.outline());
    let dialog = screen.app.layout().overlays().pop().expect("it's up");
    let in_dialog = |path: Option<&[u32]>| path.is_some_and(|path| path.starts_with(&dialog));
    assert!(
        in_dialog(screen.app.focused()),
        "{:?}",
        screen.app.focused()
    );
    // Tab goes around inside it, however many times.
    for _ in 0..5 {
        screen.press(KeyCode::Tab);
        assert!(
            in_dialog(screen.app.focused()),
            "{:?}",
            screen.app.focused()
        );
    }
    // Typing goes to the input in it.
    let mut found_input = false;
    for _ in 0..3 {
        screen.app.type_text("Z").unwrap();
        screen.pixels();
        if screen.shows("value=\"Z") {
            found_input = true;
            break;
        }
        screen.press(KeyCode::Tab);
    }
    assert!(found_input, "{}", screen.outline());
    screen.press(KeyCode::Escape);
    assert!(!screen.shows("dialog"));
    assert_eq!(
        screen.app.focused(),
        Some(&[0u32][..]),
        "back on the button"
    );
}

#[test]
fn a_dialog_dims_what_is_behind_it_and_blocks_it() {
    let mut screen = Screen::new(with_dialog);
    let before = screen.pixel(150, 150);
    assert_eq!(before, RED_PIXEL);
    screen.click(50.0, 15.0);
    assert!(screen.shows("dialog"), "{}", screen.outline());
    let dimmed = screen.pixel(5, 150);
    assert!(dimmed[0] < 200 && dimmed[0] > 100, "{dimmed:?}");
    // The wheel doesn't move what's behind it.
    screen.app.scroll(5.0, 150.0, 30).unwrap();
    screen.pixels();
    let root = screen.app.runtime().root().clone();
    let info = screen
        .app
        .layout()
        .scroll_info(BEHIND, &root.children[1])
        .expect("it scrolls");
    assert_eq!(info.y, 0.0);
    // Nor do keys.
    screen.press(KeyCode::Character('x'));
    assert!(!screen.shows("key x"), "{}", screen.outline());
    // Clicking the dimmed part closes it, and doesn't reach the button under it.
    screen.click(50.0, 15.0);
    assert!(!screen.shows("dialog"), "{}", screen.outline());
    let dialog_gone = screen.pixel(5, 150);
    assert_eq!(dialog_gone, RED_PIXEL);
}

#[test]
fn clicking_inside_a_dialog_does_not_close_it() {
    let mut screen = Screen::new(with_dialog);
    screen.click(50.0, 15.0);
    let path = screen.app.layout().overlays().pop().expect("it's up");
    let panel = screen
        .app
        .layout()
        .rect(&[path.as_slice(), &[0]].concat())
        .expect("its panel");
    screen.click(panel.x + 2.0, panel.y + 2.0);
    assert!(screen.shows("dialog"), "{}", screen.outline());
}

#[test]
fn a_button_inside_a_dialog_works() {
    let mut screen = Screen::new(with_dialog);
    screen.click(50.0, 15.0);
    let path = screen.app.layout().overlays().pop().expect("it's up");
    let close = screen
        .app
        .layout()
        .rect(&[path.as_slice(), &[0, 0, 1]].concat())
        .expect("its close button");
    screen.click(close.x + 2.0, close.y + 2.0);
    assert!(!screen.shows("dialog"), "{}", screen.outline());
}

/// Two things with tooltips and one without.
fn explained(_: &mut Cx, _: &()) -> Element {
    let block = |key: &str, tip: Option<&str>| {
        let mut element = square(key, 40.0, RED);
        if let Some(tip) = tip {
            element = element.prop("tooltip", tip);
        }
        element
    };
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start).gap(10.0)))
        .child(block("a", Some("The first")))
        .child(block("b", Some("The second")))
        .child(block("c", None))
}

const LATER: Duration = Duration::from_secs(5);

#[test]
fn a_tooltip_waits_then_shows_and_goes_away_with_the_pointer() {
    let mut screen = Screen::new(explained);
    let empty = screen.pixels();
    assert_eq!(screen.app.next_wake(), None);
    screen.app.pointer_move(10.0, 10.0);
    let wait = screen.app.next_wake().expect("it's waiting");
    assert!(wait <= Duration::from_millis(500));
    assert_eq!(screen.pixels(), empty, "not yet");
    // Too early does nothing.
    screen.app.wake_at(Instant::now());
    assert_eq!(screen.pixels(), empty);
    screen.app.wake_at(Instant::now() + LATER);
    assert_eq!(screen.app.update(), Frame::Needed);
    assert_ne!(screen.pixels(), empty, "there it is");
    assert_eq!(screen.app.next_wake(), None, "nothing more to wait for");
    // Moving around on the same thing leaves it.
    let shown = screen.pixels();
    screen.app.pointer_move(11.0, 11.0);
    assert_eq!(screen.pixels(), shown);
    // Moving off it takes it away.
    screen.app.pointer_move(10.0, 110.0);
    assert_eq!(screen.pixels(), empty);
    assert_eq!(screen.app.next_wake(), None, "that one has no tooltip");
}

#[test]
fn moving_to_another_tooltip_starts_the_wait_over() {
    let mut screen = Screen::new(explained);
    screen.app.pointer_move(10.0, 10.0);
    screen.app.wake_at(Instant::now() + LATER);
    let first = screen.pixels();
    screen.app.pointer_move(10.0, 60.0);
    assert!(screen.app.next_wake().is_some());
    assert_ne!(screen.pixels(), first, "the first went away");
}

#[test]
fn doing_something_hides_a_tooltip_until_the_pointer_moves_on() {
    let mut screen = Screen::new(explained);
    let empty = screen.pixels();
    screen.app.pointer_move(10.0, 10.0);
    screen.app.wake_at(Instant::now() + LATER);
    assert_ne!(screen.pixels(), empty);
    screen.app.pointer_down(10.0, 10.0, 1);
    screen.app.pointer_up(10.0, 10.0).unwrap();
    assert_eq!(screen.pixels(), empty);
    assert_eq!(screen.app.next_wake(), None);
    screen.app.wake_at(Instant::now() + LATER);
    assert_eq!(screen.pixels(), empty, "it stays gone");
    // Going somewhere else and back brings it back.
    screen.app.pointer_move(10.0, 110.0);
    screen.app.pointer_move(10.0, 10.0);
    assert!(screen.app.next_wake().is_some());
}

#[test]
fn a_tooltip_stays_in_the_window() {
    let mut screen = Screen::new(explained);
    let empty = screen.pixels();
    screen.app.pointer_move(39.0, 39.0);
    screen.app.wake_at(Instant::now() + LATER);
    let near_corner = screen.pixels();
    assert_ne!(near_corner, empty);
    // In a window this small, it still can't start off the edge.
    screen.app.resize((60, 60), 1.0);
    screen.renderer = Renderer::offscreen((60, 60)).unwrap();
    screen.app.pointer_move(39.0, 39.0);
    screen.app.wake_at(Instant::now() + LATER);
    screen.app.update();
    screen.app.render(&mut screen.renderer).unwrap();
    let scene_quads = screen.app.scene().quads.clone();
    for quad in scene_quads {
        assert!(quad.rect[0] >= -0.5, "{quad:?}");
    }
}

#[test]
fn the_tree_changing_under_a_tooltip_does_not_break_anything() {
    fn changing(cx: &mut Cx, _: &()) -> Element {
        let gone = cx.state(|| false);
        let root = Element::new(Role::Window)
            .key("root")
            .props(style!(Style::new().align(Align::Start)));
        let root = if cx.get(gone) {
            root
        } else {
            root.child(square("a", 40.0, RED).prop("tooltip", "Going"))
        };
        cx.on(root, ActionKind::SetValue, move |cx, _| cx.set(gone, true))
    }
    let mut screen = Screen::new(changing);
    screen.app.pointer_move(10.0, 10.0);
    screen.app.wake_at(Instant::now() + LATER);
    screen.pixels();
    screen
        .app
        .runtime_mut()
        .dispatch(&[], ActionKind::SetValue, None)
        .unwrap();
    screen.pixels();
    assert_eq!(screen.app.next_wake(), None);
}

#[test]
fn popups_coming_and_going_inside_something_do_not_leave_anything_behind() {
    // Opened and closed many times, the layout's own record of them has to stay right.
    let mut screen = Screen::new(with_menu);
    let empty = screen.pixels();
    for _ in 0..5 {
        screen.click(10.0, 10.0);
        assert!(screen.app.layout().has_overlays());
        screen.click(10.0, 10.0);
        assert!(!screen.app.layout().has_overlays());
    }
    // Without the pointer on the button, which would change its color.
    screen.app.pointer_leave();
    assert_eq!(screen.pixels(), empty);
}

#[test]
fn something_on_top_can_have_something_on_top_of_it() {
    let inner = Element::new(Role::Group).key("inner").props(
        rsui::core::Props::from(
            Style::new()
                .overlay(Placement::At(5.0, 5.0))
                .width(20.0)
                .height(20.0),
        )
        .merged(&Paint::new().background(RED).into()),
    );
    let outer = Element::new(Role::Group)
        .key("outer")
        .props(
            rsui::core::Props::from(
                Style::new()
                    .overlay(Placement::At(0.0, 0.0))
                    .width(50.0)
                    .height(50.0),
            )
            .merged(&Paint::new().background(BLUE).into()),
        )
        .child(inner);
    let root: Node = Element::new(Role::Window).child(outer).into_node();
    let mut layout = rsui::layout::Layout::new();
    layout.compute(
        &root,
        rsui::layout::Size::new(300.0, 200.0),
        &mut rsui::layout::FixedMetrics::default(),
    );
    assert_eq!(layout.overlays(), vec![vec![0], vec![0, 0]]);
    assert_eq!(layout.hit_test(10.0, 10.0), Some(vec![0, 0]));
    assert_eq!(layout.hit_test(40.0, 40.0), Some(vec![0]));
    assert_eq!(layout.rect(&[0, 0]).unwrap().x, 5.0);
}

#[test]
fn a_click_in_a_popup_does_not_reach_what_it_is_inside() {
    fn inside_a_card(cx: &mut Cx, _: &()) -> Element {
        let said = cx.state(Text::default);
        let popup = square("popup", 60.0, BLUE).prop("overlay", "right");
        let card = square("card", 40.0, RED).child(popup);
        let card = cx.on(card, ActionKind::Click, move |cx, _| {
            cx.set(said, Text::from("clicked the card"));
        });
        Element::new(Role::Window)
            .name(cx.get(said))
            .props(style!(Style::new().align(Align::Start)))
            .child(card)
    }
    let mut screen = Screen::new(inside_a_card);
    screen.click(50.0, 10.0);
    assert!(!screen.shows("clicked the card"), "{}", screen.outline());
    // What it's inside still works in its own right.
    screen.click(10.0, 10.0);
    assert!(screen.shows("clicked the card"), "{}", screen.outline());
}

#[test]
fn reaching_the_end_of_a_scrolling_area_in_a_popup_stops_there() {
    fn popup_in_an_area(cx: &mut Cx, _: &()) -> Element {
        let inner_content = Element::new(Role::Group)
            .props(style!(
                Style::new()
                    .overlay(Placement::At(0.0, 0.0))
                    .width(100.0)
                    .height(50.0)
            ))
            .child(square("long", 100.0, BLUE));
        let inner = cx.scroll_area("inner", ScrollAxis::Vertical, inner_content);
        let outer_content = Element::new(Role::Group)
            .props(style!(Style::new().flex(1.0)))
            .child(inner)
            .child(square("tall", 1000.0, RED));
        Element::new(Role::Window).child(cx.scroll_area(
            "outer",
            ScrollAxis::Vertical,
            outer_content,
        ))
    }
    let mut screen = Screen::new(popup_in_an_area);
    screen.app.scroll(10.0, 10.0, 1000).unwrap();
    screen.pixels();
    screen.app.scroll(10.0, 10.0, 1000).unwrap();
    screen.pixels();
    let root = screen.app.runtime().root().clone();
    let outer = screen
        .app
        .layout()
        .scroll_info(&[0], &root.children[0])
        .expect("it scrolls");
    assert_eq!(outer.y, 0.0, "the area around the popup hasn't moved");
    let inner = screen
        .app
        .layout()
        .scroll_info(&[0, 0], &root.children[0].children[0])
        .expect("it scrolls");
    assert_eq!(
        inner.y, 50.0,
        "the one in the popup went as far as it could"
    );
}

#[test]
fn clicking_a_dialog_backdrop_only_closes_the_dialog() {
    fn stacked(cx: &mut Cx, _: &()) -> Element {
        let said = cx.state(Text::default);
        let menu = Element::new(Role::Menu).key("under").props(style!(
            Style::new()
                .overlay(Placement::At(0.0, 0.0))
                .width(40.0)
                .height(40.0)
        ));
        let menu = cx.on(menu, ActionKind::Dismiss, move |cx, _| {
            cx.update(said, |said| *said = rsui::core::text!("{said} menu"));
        });
        let top = Element::new(Role::Dialog)
            .key("over")
            .props(style!(Style::new().overlay(Placement::Fill)));
        let top = cx.on(top, ActionKind::Dismiss, move |cx, _| {
            cx.update(said, |said| *said = rsui::core::text!("{said} dialog"));
        });
        Element::new(Role::Window)
            .name(cx.get(said))
            .child(menu)
            .child(top)
    }
    let mut screen = Screen::new(stacked);
    screen.click(150.0, 150.0);
    assert!(screen.shows("dialog"), "{}", screen.outline());
    assert!(!screen.shows("\" menu"), "{}", screen.outline());
    assert!(!screen.shows("dialog menu"), "{}", screen.outline());
}
