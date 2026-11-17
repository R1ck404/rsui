//! Paint invalidation must agree with a newly constructed App, not just a warm layout.
#![cfg(feature = "render-diagnostics")]
#[path = "../../rsui-text/tests/support/mod.rs"]
mod font;
#[allow(dead_code)]
#[path = "support/render_oracle.rs"]
mod oracle;
use oracle::{Lane, compare};
use rsui::prelude::*;
use rsui::{App, Frame};
const SIZE: (u32, u32) = (180, 140);
#[derive(Clone)]
struct Case {
    name: &'static str,
    value: PropValue,
    initial: bool,
    paragraph: bool,
    mixed: bool,
}
fn fixture(cx: &mut Cx, case: &Case) -> Element {
    let on = cx.state(|| case.initial);
    let active = cx.get(on);
    let mut target = if case.paragraph {
        Element::new(Role::Text)
            .prop("selectable", true)
            .child(Element::new(Role::Text).name("Words that wrap across lines"))
    } else {
        Element::new(Role::Group).child(Element::new(Role::Text).name("Words"))
    };
    target = target
        .key("target")
        .prop("width", if case.mixed && active { 70.0 } else { 100.0 })
        .prop("border_width", 3.0);
    if active {
        target = target.prop(case.name, case.value.clone());
    }
    let root = Element::new(Role::Window).key("root").child(target).child(
        Element::new(Role::TextInput)
            .key("input")
            .prop("value", "abc")
            .prop("width", 100.0)
            .prop("height", 24.0),
    );
    cx.on(root, ActionKind::SetValue, move |cx, value| {
        if let Some(PropValue::Bool(value)) = value {
            cx.set(on, *value);
        }
    })
}
fn set(app: &mut App, on: bool) {
    app.runtime_mut()
        .dispatch(&[], ActionKind::SetValue, Some(PropValue::Bool(on)))
        .unwrap();
}
#[test]
fn paint_set_remove_and_fallbacks_match_fresh_pixels() {
    for flipping in [false, true] {
        for paragraph in [false, true] {
            for (name, value, fast) in [
                ("background", PropValue::Int(0x2040e080), true),
                ("hover_background", PropValue::Int(0x20e040ff), true),
                ("pressed_background", PropValue::Int(0xe04020ff), true),
                ("border_color", PropValue::Int(0xe04020ff), false),
                ("text_color", PropValue::Int(0xe04020ff), true),
                ("font_size", PropValue::Float(22.0), false),
                ("no_wrap", PropValue::Bool(true), false),
            ] {
                let case = Case {
                    name,
                    value,
                    initial: false,
                    paragraph,
                    mixed: false,
                };
                let mut lane = Lane::new(fixture, case.clone(), SIZE, 1.0, flipping, false);
                lane.frame();
                lane.app.pointer_move(8.0, 8.0);
                lane.app.focus_on(&[1]);
                lane.frame();
                for on in [true, false] {
                    set(&mut lane.app, on);
                    let got = lane.frame();
                    assert_eq!(got.update, Frame::Needed);
                    let mut fresh = Lane::new(
                        fixture,
                        Case {
                            initial: on,
                            ..case.clone()
                        },
                        SIZE,
                        1.0,
                        flipping,
                        true,
                    );
                    fresh.frame();
                    fresh.app.pointer_move(8.0, 8.0);
                    fresh.app.focus_on(&[1]);
                    let expected = fresh.frame();
                    compare(name, SIZE, 1.0, flipping, &got, &expected);
                    assert_eq!(lane.app.access_tree(), fresh.app.access_tree());
                    // Profile separately: inspector overlays must not enter pixel comparisons.
                    lane.app.set_inspecting(true);
                    set(&mut lane.app, !on);
                    lane.app.update();
                    lane.app.scene();
                    assert_eq!(
                        lane.app.frames().next_back().unwrap().layout.is_zero(),
                        fast,
                        "{name}"
                    );
                    lane.app.set_inspecting(false);
                    set(&mut lane.app, on);
                    lane.frame();
                }
            }
        }
    }
}
#[test]
fn deferred_mixed_updates_and_resize_cannot_be_cancelled_by_recolor() {
    let case = Case {
        name: "background",
        value: PropValue::Int(0xff0000ff),
        initial: false,
        paragraph: true,
        mixed: true,
    };
    let mut lane = Lane::new(fixture, case.clone(), SIZE, 1.0, false, false);
    lane.frame();
    set(&mut lane.app, true);
    lane.app.update();
    // A second flush before scene must retain the first one's required layout.
    lane.app.update();
    let got = lane.frame();
    let mut fresh = Lane::new(
        fixture,
        Case {
            initial: true,
            ..case
        },
        SIZE,
        1.0,
        false,
        true,
    );
    compare("mixed-deferred", SIZE, 1.0, false, &got, &fresh.frame());
    lane.resize((220, 160), 1.25);
    set(&mut lane.app, false);
    lane.app.update();
    set(&mut lane.app, true);
    fresh.resize((220, 160), 1.25);
    compare(
        "resize",
        (220, 160),
        1.25,
        false,
        &lane.frame(),
        &fresh.frame(),
    );
}
#[test]
fn geometry_snapshot_is_published_after_skipping_layout() {
    let case = Case {
        name: "background",
        value: PropValue::Int(0xff0000ff),
        initial: false,
        paragraph: false,
        mixed: false,
    };
    let mut lane = Lane::new(fixture, case, SIZE, 1.0, false, false);
    lane.frame();
    rsui::geometry::keep();
    set(&mut lane.app, true);
    lane.app.update();
    lane.app.scene();
    assert!(
        rsui::geometry::ask(|root, layout| {
            (root.children[0].props.get("background") == Some(&PropValue::Int(0xff0000ff)))
                .then(|| layout.rect(&[0]).is_some())
        })
        .unwrap_or(false)
    );
}

#[test]
fn first_geometry_query_after_repeated_updates_sees_recolored_tree() {
    // Start a fresh thread: an earlier geometry query permanently enables snapshots
    // in that thread and would hide the first-observer case this test protects.
    std::thread::spawn(|| {
        let case = Case {
            name: "background",
            value: PropValue::Int(0xff0000ff),
            initial: false,
            paragraph: false,
            mixed: false,
        };
        let mut lane = Lane::new(fixture, case, SIZE, 1.0, false, false);
        lane.frame();
        set(&mut lane.app, true);
        lane.app.update();
        lane.app.update();
        lane.app.scene();
        assert_eq!(
            rsui::geometry::ask(|root, layout| {
                Some((
                    root.children[0].props.get("background").cloned(),
                    layout.rect(&[0]).is_some(),
                ))
            }),
            Some((Some(PropValue::Int(0xff0000ff)), true)),
        );
    })
    .join()
    .unwrap();
}
fn pending_fixture(cx: &mut Cx, initial: &(bool, bool)) -> Element {
    let wide = cx.state(|| initial.0);
    let blue = cx.state(|| initial.1);
    let child = Element::new(Role::Group)
        .prop("width", if cx.get(wide) { 100.0 } else { 50.0 })
        .prop("height", 30.0)
        .prop(
            "background",
            if cx.get(blue) {
                0x0000ffff_i64
            } else {
                0xff0000ff_i64
            },
        );
    let root = Element::new(Role::Window).key("root").child(child);
    let root = cx.on(root, ActionKind::SetValue, move |cx, _| cx.set(wide, true));
    cx.on(root, ActionKind::Toggle, move |cx, _| cx.set(blue, true))
}
#[test]
fn later_paint_batch_preserves_pending_geometry() {
    let mut lane = Lane::new(pending_fixture, (false, false), SIZE, 1.0, false, false);
    lane.frame();
    lane.app
        .runtime_mut()
        .dispatch(&[], ActionKind::SetValue, None)
        .unwrap();
    lane.app.update();
    lane.app
        .runtime_mut()
        .dispatch(&[], ActionKind::Toggle, None)
        .unwrap();
    lane.app.update();
    let mut fresh = Lane::new(pending_fixture, (true, true), SIZE, 1.0, false, true);
    compare(
        "pending geometry",
        SIZE,
        1.0,
        false,
        &lane.frame(),
        &fresh.frame(),
    );
}
