#![cfg(all(feature = "mutation-pilot", feature = "render-diagnostics"))]
#[path = "../../rsui-text/tests/support/mod.rs"]
mod font;
#[allow(dead_code)]
#[path = "support/render_oracle.rs"]
mod oracle;
use rsui::prelude::*;
use rsui::render::Renderer;
use rsui::{App, MutationPilot};
const SIZE: (u32, u32) = (180, 140);

fn fixture(cx: &mut Cx, _: &()) -> Element {
    let state = cx.state(|| 0i64);
    let mode = cx.get(state);
    let mut target = Element::new(Role::Button)
        .key("target")
        .name(if mode == 3 { "Changed" } else { "Save" })
        .prop("width", if mode == 2 { 120 } else { 100 })
        .prop("height", 30)
        .prop(
            "background",
            if mode % 2 == 0 {
                0xe02030ff_i64
            } else {
                0x2060e0ff_i64
            },
        )
        .prop(
            "text_color",
            if mode % 2 == 0 {
                0xffffffff_i64
            } else {
                0x101010ff_i64
            },
        )
        .prop("hover_background", 0x20a060ff_i64)
        .prop("pressed_background", 0x6020a0ff_i64)
        .prop("border_color", 0x101010ff_i64)
        .prop("border_width", 2);
    if mode == 8 {
        target = target.prop("rotate", 15).prop("opacity", 0.5);
    }
    if mode == 10 {
        target = target.prop("background", 0_i64);
    }
    let input = Element::new(Role::TextInput)
        .key("input")
        .prop("value", "abc")
        .prop("height", 24);
    let mut root = Element::new(Role::Window)
        .key("root")
        .prop("background", 0xf6f8faff_i64);
    if mode == 4 {
        root = root.child(Element::new(Role::Text).key("insert").name("Inserted"));
    }
    if mode == 5 {
        root = root.child(input.clone());
    }
    if mode != 6 {
        root = root.child(target);
    }
    if mode != 5 {
        root = root.child(input);
    }
    let mut rows = Element::new(Role::Group)
        .key("rows")
        .prop("height", 40)
        .prop("overflow", "scroll");
    for i in 0..if mode == 7 { 500 } else { 10 } {
        rows = rows.child(
            Element::new(Role::Text)
                .key(i.to_string())
                .name("Row")
                .prop("height", 20)
                .prop(
                    "background",
                    if mode == 7 {
                        0x60a0e0ff_i64
                    } else {
                        0xeeeeeeff_i64
                    },
                ),
        );
    }
    root = root.child(rows);
    cx.on(root, ActionKind::SetValue, move |cx, value| {
        if let Some(PropValue::Int(v)) = value {
            cx.set(state, *v)
        }
    })
}

fn check(pilot: &mut MutationPilot, reference: &mut oracle::Lane) {
    pilot.update();
    pilot.prepare();
    pilot.draw().unwrap();
    pilot.present().unwrap();
    assert_eq!(
        pilot.renderer().read_pixels().unwrap(),
        reference.frame().pixels
    );
}

#[test]
fn owned_palette_and_direct_fallback_match_full_oracle_both_histories() {
    for flipping in [false, true] {
        let mut app = App::new(
            fixture,
            (),
            rsui::text::TextEngine::new(font::plain_font()),
            SIZE,
            1.0,
        );
        app.set_animations(false);
        let mut reference = oracle::Lane::new(fixture, (), SIZE, 1.0, flipping, true);
        let gpu = if flipping {
            Renderer::offscreen_flipping(SIZE)
        } else {
            Renderer::offscreen(SIZE)
        }
        .unwrap();
        let mut pilot = MutationPilot::new(app, &[0], gpu);
        check(&mut pilot, &mut reference);
        for mode in [1, 0, 2, 0, 3, 0, 4, 1, 5, 1, 6, 0, 7, 0, 8, 0, 10, 0, 1] {
            pilot
                .dispatch(&[], ActionKind::SetValue, Some(PropValue::Int(mode)))
                .unwrap();
            reference
                .app
                .runtime_mut()
                .dispatch(&[], ActionKind::SetValue, Some(PropValue::Int(mode)))
                .unwrap();
            check(&mut pilot, &mut reference);
            if mode == 2 || mode >= 3 {
                assert!(!pilot.work().0, "unsupported mode {mode}");
            }
        }
        assert!(
            pilot.work().0,
            "last local property commit uses owned spans"
        );
        for x in [10.0, 170.0, 10.0] {
            pilot.pointer_move(x, 10.0);
            reference.app.pointer_move(x, 10.0);
            check(&mut pilot, &mut reference);
        }
        pilot.pointer_down(10.0, 10.0);
        reference.app.pointer_down(10.0, 10.0, 1);
        check(&mut pilot, &mut reference);
        pilot.pointer_up(170.0, 10.0).unwrap();
        reference.app.pointer_up(170.0, 10.0).unwrap();
        check(&mut pilot, &mut reference);
        // Scroll, selection, caret and unsupported arbitrary mutations explicitly invalidate.
        pilot.edit(|a| a.scroll(20.0, 90.0, 2).unwrap());
        reference.app.scroll(20.0, 90.0, 2).unwrap();
        check(&mut pilot, &mut reference);
        pilot.edit(|a| a.click(20.0, 40.0).unwrap());
        reference.app.click(20.0, 40.0).unwrap();
        pilot.edit(|a| a.type_text("xy").unwrap());
        reference.app.type_text("xy").unwrap();
        check(&mut pilot, &mut reference);
        pilot.edit(App::blink);
        reference.app.blink();
        check(&mut pilot, &mut reference);
        pilot.resize((210, 160), 1.25).unwrap();
        reference.resize((210, 160), 1.25);
        check(&mut pilot, &mut reference);
        pilot.edit(App::forget_caches);
        reference.app.forget_caches();
        check(&mut pilot, &mut reference);
        // Fresh device/atlas: resource replay must precede any retained command reuse.
        pilot.edit(App::renderer_replaced);
        reference.app.renderer_replaced();
        pilot.replace_renderer(Renderer::offscreen((210, 160)).unwrap());
        reference.renderer = Renderer::offscreen((210, 160)).unwrap();
        reference.renderer.set_always_redraw(true);
        check(&mut pilot, &mut reference);
    }
}

#[test]
fn enabled_idle_and_active_hover_keep_full_pixels() {
    let mut app = App::new(
        fixture,
        (),
        rsui::text::TextEngine::new(font::plain_font()),
        SIZE,
        1.0,
    );
    app.set_animations(true);
    let mut pilot = MutationPilot::new(app, &[0], Renderer::offscreen(SIZE).unwrap());
    let mut reference = oracle::Lane::new(fixture, (), SIZE, 1.0, false, true);
    reference.app.set_animations(true);
    check(&mut pilot, &mut reference);
    pilot
        .dispatch(&[], ActionKind::SetValue, Some(PropValue::Int(1)))
        .unwrap();
    reference
        .app
        .runtime_mut()
        .dispatch(&[], ActionKind::SetValue, Some(PropValue::Int(1)))
        .unwrap();
    check(&mut pilot, &mut reference);
    assert!(
        pilot.work().0,
        "enabled but idle animation does not forbid local paint"
    );
    pilot.pointer_move(10.0, 10.0);
    reference.app.pointer_move(10.0, 10.0);
    // Both initial hover samples are at their start, before either clock advances.
    check(&mut pilot, &mut reference);
    assert!(!pilot.work().0, "active hover takes full path");
    let now = std::time::Instant::now() + std::time::Duration::from_secs(1);
    pilot.edit(|a| a.tick(now));
    reference.app.tick(now);
    check(&mut pilot, &mut reference);
}

fn translucent_clipped_fixture(cx: &mut Cx, _: &()) -> Element {
    let mut content = fixture(cx, &());
    content.role = Role::Group;
    let mut target = (*content.children[0]).clone();
    if let Some(PropValue::Int(color)) = target.props.get("background") {
        let color = (*color & !255) | 128;
        target = target.prop("background", color).prop("corner_radius", 5);
    }
    content.children[0] = target.into_node();
    let overlay = |color| {
        Element::new(Role::Text)
            .name("overlay")
            .prop("position", "absolute")
            .prop("left", 6)
            .prop("top", 8)
            .prop("width", 60)
            .prop("height", 20)
            .prop("background", color)
            .into_node()
    };
    content.children.insert(0, overlay(0xffa00080_i64));
    content.children.push(overlay(0x00ff0080_i64));
    Element::new(Role::Window).child(
        Element::new(Role::Group)
            .prop("width", 90)
            .prop("height", 27)
            .prop("overflow", "hidden")
            .child(
                content
                    .prop("width", 80)
                    .prop("height", 35)
                    .prop("overflow", "hidden"),
            ),
    )
}

#[test]
fn nested_clips_translucent_order_and_full_transitions_match_independent_app() {
    for flipping in [false, true] {
        let mut app = App::new(
            translucent_clipped_fixture,
            (),
            rsui::text::TextEngine::new(font::plain_font()),
            SIZE,
            1.0,
        );
        app.set_animations(false);
        let mut reference =
            oracle::Lane::new(translucent_clipped_fixture, (), SIZE, 1.0, flipping, true);
        let gpu = if flipping {
            Renderer::offscreen_flipping(SIZE)
        } else {
            Renderer::offscreen(SIZE)
        }
        .unwrap();
        let mut pilot = MutationPilot::new(app, &[0, 0, 1], gpu);
        check(&mut pilot, &mut reference);
        for mode in [1, 0, 3, 0, 1, 0] {
            pilot
                .dispatch(&[0, 0], ActionKind::SetValue, Some(PropValue::Int(mode)))
                .unwrap();
            reference
                .app
                .runtime_mut()
                .dispatch(&[0, 0], ActionKind::SetValue, Some(PropValue::Int(mode)))
                .unwrap();
            check(&mut pilot, &mut reference);
        }
        assert!(pilot.work().0);
    }
}

// Bit 0 is local color; bit 1 changes text outside that owner's damage;
// bit 2 moves that unrelated text. This detects undersized two-buffer plans.
fn transition_fixture(cx: &mut Cx, _: &()) -> Element {
    let state = cx.state(|| 0i64);
    let mode = cx.get(state);
    let root = Element::new(Role::Window)
        .key("root")
        .prop("background", 0xffffffff_i64)
        .child(
            Element::new(Role::Button)
                .name("Save")
                .prop("width", 70)
                .prop("height", 25)
                .prop(
                    "background",
                    if mode & 1 == 0 {
                        0xff000080_i64
                    } else {
                        0x0000ff80_i64
                    },
                ),
        )
        .child(
            Element::new(Role::Text)
                .name(if mode & 2 == 0 { "ABCD" } else { "DCBA ABCD" })
                .prop("position", "absolute")
                .prop("top", 80)
                .prop("left", if mode & 4 == 0 { 10 } else { 40 }),
        );
    cx.on(root, ActionKind::SetValue, move |cx, v| {
        if let Some(PropValue::Int(v)) = v {
            cx.set(state, *v);
        }
    })
}

#[test]
fn composed_pending_prepares_and_external_damage_are_partial() {
    use rsui::render::diagnostics::DrawOutcome;
    for flipping in [false, true] {
        let mut app = App::new(
            transition_fixture,
            (),
            rsui::text::TextEngine::new(font::plain_font()),
            SIZE,
            1.0,
        );
        app.set_animations(false);
        let gpu = if flipping {
            Renderer::offscreen_flipping(SIZE)
        } else {
            Renderer::offscreen(SIZE)
        }
        .unwrap();
        let mut pilot = MutationPilot::new(app, &[0], gpu);
        let mut reference = oracle::Lane::new(transition_fixture, (), SIZE, 1.0, flipping, true);
        check(&mut pilot, &mut reference);
        let set = |pilot: &mut MutationPilot, reference: &mut oracle::Lane, mode| {
            pilot
                .dispatch(&[], ActionKind::SetValue, Some(PropValue::Int(mode)))
                .unwrap();
            reference
                .app
                .runtime_mut()
                .dispatch(&[], ActionKind::SetValue, Some(PropValue::Int(mode)))
                .unwrap();
        };
        for mode in [1, 0] {
            set(&mut pilot, &mut reference, mode);
            check(&mut pilot, &mut reference);
        }
        assert_eq!(
            pilot.storage().comparison_spare.quads.capacity_bytes,
            0,
            "local-only has no full spare"
        );
        for mode in [2, 3, 7, 6, 2, 0, 2, 0] {
            set(&mut pilot, &mut reference, mode);
            check(&mut pilot, &mut reference);
            assert_eq!(
                pilot.renderer().draw_work().outcome,
                DrawOutcome::Partial,
                "mode {mode}, flip {flipping}"
            );
            assert!(!pilot.renderer().draw_work().full_reasons.unknown_history);
            assert_eq!(pilot.storage().comparison_spare.quads.len_bytes, 0);
        }
        // Unsent local color, then two full prepares. The first snapshot already
        // contains blue; comparing it alone would miss the actual red GPU pixels.
        for mode in [1, 3, 7] {
            set(&mut pilot, &mut reference, mode);
            pilot.update();
            pilot.prepare();
        }
        pilot.draw().unwrap();
        pilot.present().unwrap();
        assert_eq!(
            pilot.renderer().read_pixels().unwrap(),
            reference.frame().pixels
        );
        assert_eq!(pilot.renderer().draw_work().outcome, DrawOutcome::Partial);
        pilot.draw().unwrap();
        assert!(pilot.renderer().showed_nothing());
        pilot.present().unwrap();
        // Skipped Present must recover the target, not acknowledge buffer advance.
        set(&mut pilot, &mut reference, 6);
        pilot.update();
        pilot.prepare();
        pilot.draw().unwrap();
        set(&mut pilot, &mut reference, 2);
        check(&mut pilot, &mut reference);
    }
}
