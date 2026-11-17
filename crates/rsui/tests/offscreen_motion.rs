//! Infinite visual-only leaf motion sleeps offscreen without pausing its clock.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};
use rsui::core::{ActionKind, Element, PropValue, Role, Symbol};
use rsui::layout::Style;
use rsui::runtime::{Cx, ScrollAxis};
use rsui::text::TextEngine;
use rsui::App;

#[derive(Clone)]
struct Case { finite: bool, transform: bool, text: bool, ancestor_shadow: bool, self_shadow: bool, finished: Rc<Cell<usize>> }

fn page(cx: &mut Cx, case: &Case) -> Element {
    let animation = format!("2 0 {} normal forwards linear\n0 opacity n 0\n1 opacity n 1", if case.finite { "1" } else { "inf" });
    let mut pulse = Element::new(if case.text { Role::Text } else { Role::Group })
        .key("pulse").props(Style::new().height(20.0)).prop("animation", animation.as_str());
    if case.text { pulse = pulse.name("text"); }
    if case.self_shadow {
        pulse = pulse.prop("shadow_blur", 0.0).prop("shadow_y", -1000.0).prop("shadow_color", 255i64);
    }
    let finished = case.finished.clone();
    let pulse = cx.on(pulse, ActionKind::Custom(Symbol::new("motion_end")), move |_, input| {
        if matches!(input, Some(PropValue::Str(what)) if what.as_str() == "animation") {
            finished.set(finished.get() + 1);
        }
    });
    let mut content = Element::new(Role::Group).props(Style::new().flex(1.0))
        .child(Element::new(Role::Group).props(Style::new().height(1000.0)))
        .child(pulse);
    if case.transform { content = content.prop("transform", "translateY(-1000px)"); }
    if case.ancestor_shadow { content = content.prop("shadows", "0 1 3 0 255"); }
    Element::new(Role::Window).child(cx.scroll_area("area", ScrollAxis::Vertical, content))
}

fn app(finite: bool, transform: bool, text: bool) -> (App, Instant, Rc<Cell<usize>>) {
    let finished = Rc::new(Cell::new(0));
    app_case(Case { finite, transform, text, ancestor_shadow: false, self_shadow: false, finished })
}

fn app_case(case: Case) -> (App, Instant, Rc<Cell<usize>>) {
    let finished = case.finished.clone();
    let mut app = App::new(page, case, TextEngine::new(support::plain_font()), (200, 100), 1.0);
    app.runtime_mut().set_own_clock(true);
    let start = Instant::now();
    app.tick(start);
    app.set_animations(true);
    app.update();
    app.scene();
    (app, start, finished)
}

#[test]
fn offscreen_infinite_visual_leaf_has_no_display_wake() {
    let (mut app, _, _) = app(false, false, false);
    assert!(!app.animating());
    assert!(app.next_wake().is_none());
    assert!(app.runtime().is_animating(), "the animation still exists with its original timeline");
}

#[test]
fn a_static_ancestor_box_shadow_does_not_depend_on_the_childs_opacity() {
    let (mut app, _, _) = app_case(Case { finite: false, transform: false, text: false,
        ancestor_shadow: true, self_shadow: false, finished: Rc::new(Cell::new(0)) });
    assert!(!app.animating());
    assert!(app.next_wake().is_none());
}

#[test]
fn a_leafs_zero_blur_offset_shadow_keeps_display_scheduling() {
    let (mut app, _, _) = app_case(Case { finite: false, transform: false, text: false,
        ancestor_shadow: false, self_shadow: true, finished: Rc::new(Cell::new(0)) });
    assert!(app.animating());
    assert!(app.next_wake().is_some());
}

#[test]
fn scrolling_it_into_view_samples_the_current_phase() {
    let (mut app, start, _) = app(false, false, false);
    app.tick(start + Duration::from_millis(700));
    app.scroll_pixels(50.0, 50.0, 950.0, false, true).unwrap();
    app.update();
    app.scene();
    assert!(app.layout().rect(&[0, 1]).unwrap().y < 100.0);
    let root = app.runtime().root();
    let opacity = match root.children[0].children[1].props.get("opacity") {
        Some(PropValue::Float(value)) => *value,
        other => panic!("missing sampled opacity: {other:?}"),
    };
    assert!((opacity - 0.35).abs() < 0.0001, "offscreen time must not reset: {opacity}");
    assert!(app.animating());
    assert!(app.next_wake().is_some());
}

#[test]
fn finite_completion_deadlines_still_run_offscreen() {
    let (mut app, start, finished) = app(true, false, false);
    assert!(app.next_wake().is_some());
    app.tick(start + Duration::from_millis(2100));
    app.update();
    app.scene();
    assert_eq!(finished.get(), 1);
    assert!(!app.runtime().is_animating());
}

#[test]
fn transform_and_text_uncertainty_keep_normal_scheduling() {
    for (transform, text) in [(true, false), (false, true)] {
        let (mut app, _, _) = app(false, transform, text);
        assert!(app.animating());
        assert!(app.next_wake().is_some());
    }
}
