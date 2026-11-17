#![cfg(all(feature = "automatic-render", feature = "render-diagnostics"))]
#[path = "../../rsui-text/tests/support/mod.rs"]
mod font;
#[allow(dead_code)]
#[path = "support/render_oracle.rs"]
mod oracle;
use rsui::{
    App,
    prelude::*,
    render::{Renderer, diagnostics::ScenePath},
};
const SIZE: (u32, u32) = (180, 140);
fn input(cx: &mut Cx, _: &()) -> Element {
    let value = cx.state(String::new);
    let e = Element::new(Role::TextInput)
        .key("input")
        .prop("height", 30)
        .prop("value", cx.get(value));
    cx.on(e, ActionKind::SetValue, move |cx, v| {
        if let Some(PropValue::Str(s)) = v {
            cx.set(value, s.to_string());
        }
    })
}

fn view(cx: &mut Cx, depth: &usize) -> Element {
    let step = cx.state(|| 0usize);
    let n = cx.get(step);
    let mut group = Element::new(Role::Group);
    for i in 0..2 {
        let color = if (if i == 0 {
            n.min(3)
        } else {
            n.saturating_sub(3)
        }) % 2
            == 0
        {
            0x205080ff_i64
        } else {
            0xd03020ff_i64
        };
        group = group.child(
            Element::new(Role::Text)
                .key(format!("target{i}"))
                .name(if n == 7 {
                    "Unicode caf\u{e9} \u{65e5}\u{672c}"
                } else {
                    "Save"
                })
                .prop("width", if n == 8 { 120 } else { 100 })
                .prop("height", 30)
                .prop("opacity", if n == 8 { 0.5 } else { 1.0 })
                .prop("background", color)
                .prop("hover_background", 0x20a060ff_i64),
        );
    }
    if n == 9 {
        group = group.child(Element::new(Role::Text).name("inserted"));
    }
    for _ in 0..*depth {
        group = Element::new(Role::Group).child(group);
    }
    let root = Element::new(Role::Window)
        .key("root")
        .prop("background", 0xffffffff_i64)
        .child(group)
        .child(cx.child(input, ()));
    cx.on(root, ActionKind::Click, move |cx, _| {
        cx.update(step, |v| *v += 1)
    })
}
fn check(app: &mut App, gpu: &mut Renderer, oracle: &mut oracle::Lane) -> Vec<u8> {
    app.update();
    app.render(gpu).unwrap();
    let pixels = gpu.read_pixels().unwrap();
    assert_eq!(pixels, oracle.frame().pixels);
    pixels
}
#[test]
fn ordinary_render_selects_retargets_and_falls_back() {
    for flip in [false, true] {
        for depth in [0, 12, 40] {
            let mut app = App::new(
                view,
                depth,
                rsui::text::TextEngine::new(font::plain_font()),
                SIZE,
                1.0,
            );
            app.set_animations(false);
            let mut reference = oracle::Lane::new(view, depth, SIZE, 1.0, flip, true);
            let mut gpu = if flip {
                Renderer::offscreen_flipping(SIZE)
            } else {
                Renderer::offscreen(SIZE)
            }
            .unwrap();
            let mut before = check(&mut app, &mut gpu, &mut reference);
            for step in 1..=10 {
                app.click(170.0, 130.0).unwrap();
                reference.app.click(170.0, 130.0).unwrap();
                let after = check(&mut app, &mut gpu, &mut reference);
                assert_ne!(before, after, "non-inert step {step}");
                before = after;
                if depth < 32 && [2, 3, 5, 6].contains(&step) {
                    assert_eq!(app.scene_work().path, ScenePath::OwnedPalette);
                }
            }
            for i in 0..8 {
                for a in [&mut app, &mut reference.app] {
                    match i {
                        0 => a.pointer_move(10.0, 10.0),
                        1 => a.pointer_leave(),
                        2 => a.pointer_down(10.0, 10.0, 1),
                        3 => a.cancel_press(),
                        4 => {
                            a.focus_on(&[1]);
                            assert!(a.caret_blinking());
                        }
                        5 => a.type_text("abc\u{e9}").unwrap(),
                        _ => a.blink(),
                    }
                }
                check(&mut app, &mut gpu, &mut reference);
                if i == 7 {
                    assert_eq!(app.scene_work().path, ScenePath::CaretReuse);
                    assert!(
                        !gpu.draw_work().full_reasons.unknown_history,
                        "caret reuse must retain ordinary comparison history"
                    );
                }
            }
            app.forget_caches();
            app.release_unused_images(&mut gpu);
            gpu.compact().unwrap();
            gpu.trim_storage();
            check(&mut app, &mut gpu, &mut reference);
            let _ = app.runtime();
            let _ = app.layout();
            let _ = app.render_storage();
            check(&mut app, &mut gpu, &mut reference);
            assert_eq!(
                gpu.draw_work().outcome,
                rsui::render::diagnostics::DrawOutcome::Unchanged
            );
            app.scene().quads.clear();
            check(&mut app, &mut gpu, &mut reference);
            app.runtime_mut()
                .dispatch(&[], ActionKind::Click, None)
                .unwrap();
            reference
                .app
                .runtime_mut()
                .dispatch(&[], ActionKind::Click, None)
                .unwrap();
            check(&mut app, &mut gpu, &mut reference);
            let mut other = Renderer::offscreen(SIZE).unwrap();
            check(&mut app, &mut other, &mut reference);
            check(&mut app, &mut gpu, &mut reference);
            app.click(170.0, 130.0).unwrap();
            reference.app.click(170.0, 130.0).unwrap();
            app.update();
            gpu.fail_next_present_for_diagnostics();
            assert!(app.render(&mut gpu).is_err());
            check(&mut app, &mut gpu, &mut reference);
            let mut second = App::new(
                view,
                0,
                rsui::text::TextEngine::new(font::plain_font()),
                SIZE,
                1.0,
            );
            second.render(&mut gpu).unwrap();
            check(&mut app, &mut gpu, &mut reference);
        }
    }
}

#[test]
fn externally_consumed_patches_repair_layout_before_ordinary_render() {
    let mut app = App::new(
        view,
        0,
        rsui::text::TextEngine::new(font::plain_font()),
        SIZE,
        1.0,
    );
    let mut reference = oracle::Lane::new(view, 0, SIZE, 1.0, true, true);
    let mut gpu = Renderer::offscreen_flipping(SIZE).unwrap();
    check(&mut app, &mut gpu, &mut reference);
    for _ in 0..10 {
        let runtime = app.runtime_mut();
        runtime.dispatch(&[], ActionKind::Click, None).unwrap();
        assert!(!runtime.flush().is_empty());
        reference
            .app
            .runtime_mut()
            .dispatch(&[], ActionKind::Click, None)
            .unwrap();
        // No App::update: repair must also guard the render boundary itself.
        app.render(&mut gpu).unwrap();
        assert_eq!(gpu.read_pixels().unwrap(), reference.frame().pixels);
    }
}
