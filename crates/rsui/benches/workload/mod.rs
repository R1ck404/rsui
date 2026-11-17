//! UIs to measure with, shared by the benchmarks and the frame probe.

use rsui::prelude::*;

const INK: Color = Color::hex(0x1F2328);
const MUTED: Color = Color::hex(0x656D76);
const BLUE: Color = Color::hex(0x1F6FEB);
const LINE: Color = Color::hex(0xD8DEE4);
const CARD: Color = Color::WHITE;
const TEXT: &str = "The quick brown fox jumps over the lazy dog";

fn nav_item(cx: &mut Cx, index: &usize) -> Element {
    let active = cx.state(|| false);
    let paint = if cx.get(active) {
        Paint::new().background(BLUE).text(Color::WHITE)
    } else {
        Paint::new().background(CARD).hover(LINE).text(INK)
    };
    let item = Element::new(Role::Button)
        .name(text!("Section number {index}"))
        .props(
            rsui::core::Props::from(Style::new().padding_xy(10.0, 5.0))
                .merged(&paint.corner_radius(4.0).into()),
        );
    cx.on(item, ActionKind::Click, move |cx, _| {
        cx.update(active, |active| *active = !*active);
    })
}

fn table_row(cx: &mut Cx, index: &usize) -> Element {
    let clicks = cx.state(|| 0u32);
    let name = Element::new(Role::Text)
        .name(text!("Item {index}: {TEXT}"))
        .props(style!(Style::new().flex(1.0)));
    let detail = Element::new(Role::Text)
        .name(text!("{} clicks", cx.get(clicks)))
        .props(style!(Paint::new().text(MUTED)));
    let row = Element::new(Role::ListItem)
        .props(style!(
            Style::row()
                .padding_xy(8.0, 3.0)
                .gap(12.0)
                .align(Align::Center),
            Paint::new().background(CARD).hover(LINE),
        ))
        .child(name)
        .child(detail);
    cx.on(row, ActionKind::Click, move |cx, _| {
        cx.update(clicks, |clicks| *clicks += 1);
    })
}

/// What a typical app window looks like: navigation, a header, and a table of text. About 170
/// nodes and 2,000 glyphs.
pub fn dashboard(cx: &mut Cx, _: &()) -> Element {
    let items: Vec<Node> = (0..14usize).map(|i| cx.keyed(i, nav_item, i)).collect();
    let nav = Element::new(Role::Group)
        .props(style!(
            Style::new().padding(8.0).gap(4.0).width(200.0),
            Paint::new().background(CARD).corner_radius(8.0),
        ))
        .children(items);
    let rows: Vec<Node> = (0..24usize).map(|i| cx.keyed(i, table_row, i)).collect();
    let header = Element::new(Role::Text)
        .name("Everything in one place")
        .prop("font_size", 20.0)
        .prop("font_weight", "bold");
    let table = Element::new(Role::Group)
        .props(style!(
            Style::new().padding(8.0).gap(1.0).flex(1.0),
            Paint::new().background(CARD).corner_radius(8.0),
        ))
        .child(header)
        .children(rows);
    Element::new(Role::Window)
        .props(style!(Style::row().padding(8.0).gap(8.0)))
        .child(nav)
        .child(table)
}

/// Lots of plain text.
pub fn labels(_: &mut Cx, count: &usize) -> Element {
    Element::new(Role::Window)
        .props(style!(Style::new().padding(4.0)))
        .children((0..*count).map(|index| {
            Element::new(Role::Text)
                .name(text!("Label number {index}: {TEXT}"))
                .into_node()
        }))
}

/// Lots of rectangles, with no text.
pub fn boxes(_: &mut Cx, count: &usize) -> Element {
    let per_row = 100;
    let rows = count.div_ceil(per_row);
    Element::new(Role::Window).children((0..rows).map(|row| {
        Element::new(Role::Group)
            .props(style!(Style::row().flex(1.0).gap(1.0)))
            .children((0..per_row).map(|column| {
                let shade = ((row * 7 + column * 3) % 200) as u32;
                let color = Color::hex(0x202020 + shade * 0x010101);
                Element::new(Role::Group)
                    .props(
                        rsui::core::Props::from(Style::new().flex(1.0))
                            .merged(&Paint::new().background(color).into()),
                    )
                    .into_node()
            }))
            .into_node()
    }))
}

/// One text input holding this many characters.
pub fn big_input(cx: &mut Cx, length: &usize) -> Element {
    let value = cx.state(|| {
        let line: String = TEXT.chars().cycle().take(*length).collect();
        Text::from(line)
    });
    let input = text_input(cx, "big", "A big input", "", value);
    Element::new(Role::Window)
        .props(style!(Style::new().padding(8.0).align(Align::Start)))
        .child(input)
}

/// One paragraph holding this many characters, wrapped across the window.
pub fn paragraph(cx: &mut Cx, length: &usize) -> Element {
    let body = cx.memo(*length, || {
        let words: String = TEXT
            .chars()
            .chain(" ".chars())
            .cycle()
            .take(*length)
            .collect();
        Text::from(words)
    });
    Element::new(Role::Window).child(Element::new(Role::Text).name(body))
}

#[allow(dead_code, reason = "this file's also used by what doesn't draw these")]
const SENTENCE: &str = "The quick brown fox jumps over the lazy dog, and then it carries on through the rest of the field for as long as there is light left to see by.";

/// A page of text, in paragraphs of about 30 words. With `parts`, each has a few words that look different, the way a
/// page written in Markdown does.
#[allow(dead_code, reason = "this file's also used by what doesn't draw these")]
pub fn page(_: &mut Cx, parts: &bool) -> Element {
    let mut window = Element::new(Role::Window);
    for _ in 0..30 {
        let paragraph = if *parts {
            Element::new(Role::Text)
                .child(Element::new(Role::Text).name("The quick brown "))
                .child(
                    Element::new(Role::Text)
                        .name("fox jumps")
                        .prop("font_weight", "bold"),
                )
                .child(Element::new(Role::Text).name(" over the lazy dog, and then it "))
                .child(
                    Element::new(Role::Text)
                        .name("carries on")
                        .prop("font_style", "italic"),
                )
                .child(Element::new(Role::Text).name(
                    " through the rest of the field for as long as there is light left to see by.",
                ))
        } else {
            Element::new(Role::Text).name(SENTENCE)
        };
        window = window.child(paragraph);
    }
    window
}

/// Bounded fixtures shared by the new Criterion groups and the checked probe.
/// The first keyed component is the visible target; all other rows stay mounted.
#[allow(
    dead_code,
    reason = "the legacy frame_probe also imports this shared workload file"
)]
pub mod measured {
    use super::*;
    use rsui::render::{Renderer, Scene};
    use rsui::{App, KeyCode, KeyPress};
    pub const SIZE: (u32, u32) = (320, 200);

    fn row(cx: &mut Cx, text_change: &bool) -> Element {
        let state = cx.state(|| false);
        let on = cx.get(state);
        let label = if *text_change && on { "DCBA" } else { "ABCD" };
        let color = if !*text_change && on { BLUE } else { INK };
        let element = Element::new(Role::Text).name(label).props(
            rsui::core::Props::from(Style::new().height(20.0).width(200.0))
                .merged(&Paint::new().text(color).into()),
        );
        cx.on(element, ActionKind::Toggle, move |cx, _| {
            cx.update(state, |state| *state = !*state);
        })
    }
    fn rows(cx: &mut Cx, props: &(usize, bool)) -> Element {
        Element::new(Role::Window).children((0..props.0).map(|i| cx.keyed(i, row, props.1)))
    }
    pub fn local(count: usize, text_change: bool) -> App {
        let mut app = App::new(
            rows,
            (count, text_change),
            rsui::default_text_engine().expect("system fonts"),
            SIZE,
            1.0,
        );
        app.set_animations(false);
        app.update();
        app.scene();
        app
    }
    pub fn toggle(app: &mut App) {
        app.runtime_mut()
            .dispatch(&[0], ActionKind::Toggle, None)
            .expect("keyed row 0 toggle");
    }
    pub fn input() -> App {
        let mut app = App::new(
            super::big_input,
            0usize,
            rsui::default_text_engine().expect("system fonts"),
            SIZE,
            1.0,
        );
        app.set_animations(false);
        app.update();
        app.key(KeyPress::new(KeyCode::Tab)).expect("focus input");
        app.type_text("ABCD").expect("type actual input");
        app.update();
        app.scene();
        assert!(app.focused().is_some());
        assert!(app.outline().contains("ABCD"));
        app
    }
    /// Invalidate the kept scene, restart the finite blink budget, and return to
    /// identical text. All this belongs to setup, never the measured operation.
    pub fn prepare_blink(app: &mut App, reuse: bool) {
        app.type_text("x").expect("type reset");
        app.key(KeyPress::new(KeyCode::Backspace))
            .expect("bounded reset");
        app.update();
        app.scene();
        if reuse {
            app.blink();
            app.update();
            app.scene();
        }
    }
    /// A small output guard; pixel equality is checked separately outside timing.
    pub fn guard(scene: &Scene) -> u64 {
        let mut hash = scene.quads.len() as u64;
        for q in scene.quads.iter().take(8) {
            hash = hash.wrapping_mul(16777619) ^ u32::from_le_bytes(q.color) as u64;
            hash = hash.wrapping_mul(16777619) ^ q.rect[0].to_bits() as u64;
            hash = hash.wrapping_mul(16777619) ^ u64::from(q.glyph[0]);
        }
        hash
    }
    pub fn pixels(app: &mut App, gpu: &mut Renderer) -> Vec<u8> {
        gpu.render(app.scene(), Color::WHITE)
            .expect("checked WARP render");
        gpu.read_pixels().expect("WARP readback")
    }
    pub fn check_blink() {
        let mut a = input();
        let mut g = Renderer::offscreen(SIZE).expect("WARP");
        let visible = pixels(&mut a, &mut g);
        a.blink();
        a.update();
        let hidden = pixels(&mut a, &mut g);
        assert_ne!(visible, hidden, "first blink is visible");
        a.blink();
        a.update();
        assert_eq!(
            visible,
            pixels(&mut a, &mut g),
            "warmed blink restores caret"
        );
    }
    pub fn check_local(count: usize, text_change: bool) {
        let mut app = local(count, text_change);
        let mut gpu = Renderer::offscreen(SIZE).expect("WARP");
        let a = pixels(&mut app, &mut gpu);
        toggle(&mut app);
        app.update();
        let b = pixels(&mut app, &mut gpu);
        assert_ne!(a, b, "target row mutation must be visible");
        toggle(&mut app);
        app.update();
        assert_eq!(a, pixels(&mut app, &mut gpu), "bounded return");
    }
}
