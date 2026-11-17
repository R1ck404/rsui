//! Independent conservative interaction replay and private work assertions.
use super::*;
#[path = "../../rsui-text/tests/support/mod.rs"]
mod font;
const SIZE: (u32, u32) = (240, 180);

fn fixture(cx: &mut Cx, _: &()) -> Element {
    let mode = cx.state(|| 0_i64);
    let f = cx.get(mode);
    let value = cx.state(|| "ABCD EFGH IJKL\nMNOP QRST UVWX\nYZ ABCD EFGH".to_owned());
    let mut target = Element::new(Role::Text)
        .key("target")
        .name("Paint")
        .prop("height", 24)
        .prop("width", if f & 32 == 0 { 120 } else { 150 })
        .prop(
            "background",
            if f & 1 == 0 {
                0x205080ff_i64
            } else {
                0xd03020ff_i64
            },
        );
    if f & 2 != 0 {
        target = target.prop("pressed_background", 0x806020ff_i64);
    }
    if f & 4 == 0 {
        target = target.prop("hover_background", 0x20a060ff_i64);
    }
    if f & 128 == 0 {
        target = cx.on(target, ActionKind::Click, move |cx, _| {
            cx.update(mode, |v| *v ^= 1)
        });
    }
    let input = Element::new(Role::TextInput)
        .key("edit")
        .prop("rows", 4)
        .prop("width", 180)
        .prop("height", 50)
        .prop("font_size", if f & 2048 == 0 { 12 } else { 15 })
        .prop("disabled", f & 64 != 0)
        .prop(
            "value",
            if f & 8 == 0 {
                cx.get(value)
            } else {
                "AB".to_owned()
            },
        );
    let input = cx.on(input, ActionKind::SetValue, move |cx, value_new| {
        if let Some(PropValue::Str(s)) = value_new {
            cx.set(value, s.to_string());
        }
    });
    let paragraph = Element::new(Role::Text)
        .key("paragraph")
        .prop("width", 200)
        .prop("height", 50)
        .prop("font_size", 12)
        .prop("selectable", f & 4096 == 0)
        .child(Element::new(Role::Text).name(if f & 16 == 0 {
            "ABCD EFGH "
        } else {
            "IJKL MNOP "
        }))
        .child(Element::new(Role::Text).name("QRST UVWX YZ"));
    let mut children = vec![target];
    if f & 512 == 0 {
        children.push(input);
    }
    children.push(paragraph);
    if f & 256 != 0 {
        children.reverse();
    }
    if f & 1024 != 0 {
        children.insert(
            0,
            Element::new(Role::Text)
                .key("inserted")
                .name("Extra")
                .prop("height", 12),
        );
    }
    let root = Element::new(Role::Window).key("root").children(children);
    cx.on(root, ActionKind::SetValue, move |cx, value| {
        if let Some(PropValue::Int(n)) = value {
            cx.set(mode, *n);
        }
    })
}
fn new(reference: bool, flip: bool) -> (App, Renderer) {
    let mut app = App::new(fixture, (), TextEngine::new(font::plain_font()), SIZE, 1.0);
    app.conservative_interactions = reference;
    let mut gpu = if flip {
        Renderer::offscreen_flipping(SIZE)
    } else {
        Renderer::offscreen(SIZE)
    }
    .unwrap();
    gpu.set_always_redraw(reference);
    app.update();
    app.render(&mut gpu).unwrap();
    app.focus_on(&[1]);
    let text = input_value(node_at(app.runtime.root(), &[1])).to_owned();
    app.focus
        .as_mut()
        .unwrap()
        .edit
        .as_mut()
        .unwrap()
        .select_all(&text);
    let text = paragraph_text(node_at(app.runtime.root(), &[2]));
    let mut edit = TextEdit::new();
    edit.select_all(&text);
    app.text_selection = Some(TextSelection {
        path: vec![2],
        text,
        edit,
    });
    app.text_selection_stale = true;
    app.pointer_move(8.0, 8.0);
    app.update();
    app.render(&mut gpu).unwrap();
    assert!(app.selected_text().is_some());
    assert!(app.focus.as_ref().unwrap().carets.is_some());
    (app, gpu)
}
fn set(app: &mut App, flags: i64) {
    // Internal dispatch exercises committed patches without the opaque Runtime API.
    app.runtime
        .dispatch(&[], ActionKind::SetValue, Some(PropValue::Int(flags)))
        .unwrap();
}
fn pixels(app: &mut App, gpu: &mut Renderer) -> Vec<u8> {
    app.update();
    if app.conservative_interactions {
        app.invalidate_automatic();
    }
    app.render(gpu).unwrap();
    gpu.read_pixels().unwrap()
}
fn compare(a: &mut App, ag: &mut Renderer, b: &mut App, bg: &mut Renderer) {
    assert_eq!(pixels(a, ag), pixels(b, bg));
    assert_eq!(a.focused(), b.focused());
    assert_eq!(a.selected_text(), b.selected_text());
    assert_eq!(a.cursor(), b.cursor());
    assert_eq!(a.hovered, b.hovered);
    assert_eq!(a.caret_place(), b.caret_place());
    let edit = |app: &App| {
        app.focus.as_ref().map(|f| {
            (
                f.scroll,
                f.scroll_y,
                f.edit.as_ref().map(|e| (e.caret(), e.selection())),
            )
        })
    };
    assert_eq!(edit(a), edit(b));
}
#[test]
fn palette_transactions_preserve_hover_caret_and_selected_paragraph_work() {
    for flip in [false, true] {
        let (mut a, mut ag) = new(false, flip);
        let (mut b, mut bg) = new(true, flip);
        compare(&mut a, &mut ag, &mut b, &mut bg);
        for flags in [1, 3, 2, 0, 1] {
            let before = a.interaction_counts;
            let reference_before = b.interaction_counts;
            set(&mut a, flags);
            set(&mut b, flags);
            compare(&mut a, &mut ag, &mut b, &mut bg);
            assert_eq!(
                a.interaction_counts, before,
                "visual-only patch must preserve all four caches"
            );
            assert!(
                b.interaction_counts
                    .iter()
                    .zip(reference_before)
                    .all(|(after, before)| *after > before)
            );
        }
        // A visible recolor must still repaint, even though interaction state stays valid.
        a.pointer_leave();
        b.pointer_leave();
        set(&mut a, 0);
        set(&mut b, 0);
        compare(&mut a, &mut ag, &mut b, &mut bg);
        let before = ag.read_pixels().unwrap();
        set(&mut a, 1);
        set(&mut b, 1);
        compare(&mut a, &mut ag, &mut b, &mut bg);
        assert_ne!(before, ag.read_pixels().unwrap());
    }
}
#[test]
fn unknown_mixed_and_structural_transactions_keep_conservative_behavior() {
    for flags in [
        4,
        8,
        16,
        32,
        64,
        128,
        256,
        512,
        1024,
        2048,
        4096,
        1 | 4 | 8,
        1 | 256 | 1024,
    ] {
        let (mut a, mut ag) = new(false, false);
        let (mut b, mut bg) = new(true, false);
        let before = a.interaction_counts;
        set(&mut a, flags);
        set(&mut b, flags);
        compare(&mut a, &mut ag, &mut b, &mut bg);
        assert!(a.interaction_counts[0] > before[0], "hover control {flags}");
        assert!(a.interaction_counts[1] > before[1], "focus control {flags}");
        assert!(
            a.interaction_counts[2] > before[2],
            "selection control {flags}"
        );
    }
}
#[test]
fn pending_invalidation_is_not_erased_by_later_palette_changes() {
    for resize in [false, true] {
        let (mut a, mut ag) = new(false, false);
        let (mut b, mut bg) = new(true, false);
        for app in [&mut a, &mut b] {
            set(app, 8);
            app.update();
            if resize {
                app.resize((260, 190), 1.25);
            }
            set(app, 9);
            app.update();
            assert!(app.hover_stale);
            assert!(app.text_selection_stale);
        }
        if resize {
            ag.resize((260, 190)).unwrap();
            bg.resize((260, 190)).unwrap();
        }
        compare(&mut a, &mut ag, &mut b, &mut bg);
    }
}
#[test]
fn opaque_runtime_changes_cannot_reuse_validated_interactions() {
    for consume in [false, true] {
        let (mut a, mut ag) = new(false, false);
        let (mut b, mut bg) = new(true, false);
        for app in [&mut a, &mut b] {
            let runtime = app.runtime_mut();
            runtime
                .dispatch(&[], ActionKind::SetValue, Some(PropValue::Int(8 | 16)))
                .unwrap();
            if consume {
                assert!(!runtime.flush().is_empty());
            }
        }
        compare(&mut a, &mut ag, &mut b, &mut bg);
        assert!(a.selected_text().is_none());
        assert!(a.focus.as_ref().unwrap().edit.as_ref().unwrap().caret() <= 2);
        // Borrowing Runtime again without a flush must not clear pending exposure.
        let _ = a.runtime_mut();
        let _ = a.runtime_mut();
        a.update();
        assert!(a.hover_stale);
        assert!(a.text_selection_stale);
    }
}
#[test]
fn interaction_replay_matches_conservative_reference_in_both_buffers() {
    for flip in [false, true] {
        let (mut a, mut ag) = new(false, flip);
        let (mut b, mut bg) = new(true, flip);
        let mut state = 0x1256_u32;
        for step in 0..80 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            for app in [&mut a, &mut b] {
                match step % 10 {
                    0..=2 => set(app, i64::from(state % 4)),
                    3 => set(app, i64::from(state % 4096)),
                    4 => app.pointer_move(if state & 1 == 0 { 8.0 } else { 220.0 }, 8.0),
                    5 => app.pointer_leave(),
                    6 => {
                        app.focus_on_key(&Key::from("edit"));
                        app.type_text("AB").unwrap();
                    }
                    7 => app.set_active(state & 1 == 0),
                    8 => {
                        app.pointer_down(8.0, 8.0, 1);
                        app.cancel_press();
                    }
                    _ => {
                        set(app, 0);
                        app.blink();
                    }
                }
            }
            compare(&mut a, &mut ag, &mut b, &mut bg);
        }
    }
}
