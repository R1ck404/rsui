//! What tracing keeps: what rendered, why, whether it changed anything, and what was carried out.

use rsui_core::{ActionKind, Element, Role, text};
use rsui_runtime::{Cause, Cx, Runtime};

fn label(cx: &mut Cx, words: &&'static str) -> Element {
    cx.trace_name("label");
    Element::new(Role::Text).name(*words)
}

fn shown(cx: &mut Cx, count: &i64) -> Element {
    cx.trace_name("shown");
    Element::new(Role::Text).name(text!("{count}"))
}

fn app(cx: &mut Cx, _: &()) -> Element {
    cx.trace_name("app");
    let count = cx.state(|| 0i64);
    let other = cx.state(|| 0i64);
    let now = cx.get(count);
    let button = cx.on(
        Element::new(Role::Button).key("add"),
        ActionKind::Click,
        move |cx, _| cx.update(count, |count| *count += 1),
    );
    let same = cx.on(
        Element::new(Role::Button).key("same"),
        ActionKind::Click,
        move |cx, _| cx.update(other, |other| *other += 1),
    );
    Element::new(Role::Group)
        .child(button)
        .child(same)
        .child(cx.child(label, "always the same"))
        .child(cx.child(shown, now))
}

#[test]
fn nothing_is_kept_unless_asked() {
    let mut runtime = Runtime::new(app, ());
    runtime.dispatch(&[0], ActionKind::Click, None).unwrap();
    runtime.flush();
    assert!(runtime.take_trace().is_empty());
}

#[test]
fn a_render_says_why_and_whether_it_changed_anything() {
    let mut runtime = Runtime::new_tracing(app, ());
    let first = runtime.take_trace();
    let names: Vec<_> = first
        .renders
        .iter()
        .map(|render| render.name.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(names, ["app", "label", "shown"]);
    assert!(
        first
            .renders
            .iter()
            .all(|render| render.cause == Cause::Mounted && render.changed)
    );
    assert_eq!(first.renders[2].path.as_deref(), Some(&[3][..]));
    assert_eq!(first.renders[2].parent, Some(first.renders[0].component));

    runtime.dispatch(&[0], ActionKind::Click, None).unwrap();
    runtime.flush();
    let clicked = runtime.take_trace();
    assert_eq!(clicked.handled.len(), 1);
    assert_eq!(clicked.handled[0].key.to_string(), "add");
    // The label's props didn't change, so it didn't render.
    let rendered: Vec<_> = clicked
        .renders
        .iter()
        .map(|render| (render.name.as_deref().unwrap_or(""), render.cause))
        .collect();
    assert_eq!(rendered, [("app", Cause::State), ("shown", Cause::Props)]);
    assert!(clicked.renders.iter().all(|render| render.changed));
    assert!(
        clicked.renders[0].took >= clicked.renders[1].took,
        "a render's time takes in what rendered inside it"
    );

    runtime.dispatch(&[1], ActionKind::Click, None).unwrap();
    runtime.flush();
    let wasted = runtime.take_trace();
    assert_eq!(wasted.renders.len(), 1);
    assert!(
        !wasted.renders[0].changed,
        "state nothing shows changed, so nothing did"
    );

    runtime.set_tracing(false);
    runtime.dispatch(&[0], ActionKind::Click, None).unwrap();
    runtime.flush();
    assert!(runtime.take_trace().is_empty());
}

#[test]
fn a_component_is_found_from_what_it_made() {
    let runtime = Runtime::new(app, ());
    let (root, _) = runtime.component_at(&[0]);
    let (shown, generation) = runtime.component_at(&[3]);
    assert_ne!(root, shown);
    assert_eq!(
        runtime.component_path(shown, generation).as_deref(),
        Some(&[3][..])
    );
    assert_eq!(
        runtime
            .component_parent(shown, generation)
            .map(|(id, _)| id),
        Some(root)
    );
    assert_eq!(
        runtime.component_path(shown, generation + 1),
        None,
        "another generation is another component"
    );
}
