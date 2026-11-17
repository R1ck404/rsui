use rsui_core::{ActionKind, Element, Node, Role};
use rsui_runtime::{Cx, Runtime};
use triomphe::Arc;

fn leaf() -> Element {
    Element::new(Role::Text)
        .key("leaf")
        .name("stable")
        .prop("height", 8)
}
fn app(cx: &mut Cx<'_>, _: &()) -> Element {
    let old = cx.previous_nodes();
    let n = cx.state(|| 0i64);
    let leaf = match old.as_ref().and_then(|o| o.root()).and_then(|o| o.child(0)) {
        Some(old) => old.reuse(leaf()),
        None => leaf().into_node(),
    };
    let root = Element::new(Role::Group)
        .key("root")
        .child(leaf)
        .child(Element::new(Role::Text).name(cx.get(n).to_string()));
    cx.on(root, ActionKind::Click, move |cx, _| {
        cx.update(n, |n| *n += 1)
    })
}
#[test]
fn equal_leaves_reuse_storage_without_changing_held_trees() {
    let mut runtime = Runtime::new(app, ());
    let old = runtime.root().clone();
    for i in 1..5 {
        runtime.dispatch(&[], ActionKind::Click, None).unwrap();
        runtime.flush();
        assert!(Arc::ptr_eq(&old.children[0], &runtime.root().children[0]));
        assert_eq!(runtime.root().children[1].name, i.to_string().as_str());
    }
    assert_eq!(old.children[1].name, "0");
}
fn owned_child(_: &mut Cx<'_>, _: &()) -> Element {
    leaf()
}
fn boundary_app(cx: &mut Cx<'_>, count: &usize) -> Element {
    let prior = cx.previous_nodes();
    let n = cx.state(|| 0i64);
    if cx.get(n) > 0 {
        if *count > 8 {
            assert!(prior.is_none());
        } else {
            let old = prior.as_ref().unwrap().root().unwrap();
            for i in 0..*count {
                assert!(old.child(i).is_none());
            }
            assert!(old.child(*count).is_some());
        }
    }
    let mut root = Element::new(Role::Group).key("root");
    for i in 0..*count {
        root = root.child(cx.keyed(i as u64, owned_child, ()));
    }
    if *count > 0 {
        assert!(cx.previous_nodes().is_none());
    }
    root = root.child(leaf());
    cx.on(root, ActionKind::Click, move |cx, _| {
        cx.update(n, |n| *n += 1)
    })
}
#[test]
fn component_boundaries_and_bounded_fallback_are_enforced() {
    for count in [0, 1, 8, 9, 32] {
        let mut rt = Runtime::new(boundary_app, count);
        rt.dispatch(&[], ActionKind::Click, None).unwrap();
        rt.flush();
    }
}
fn replacement(cx: &mut Cx<'_>, _: &()) -> Element {
    let old = cx.previous_nodes();
    let n = cx.state(|| 0);
    let mut root = Element::new(Role::Group).key("root");
    if cx.get(n) == 0 {
        root = root.child(cx.keyed("leaf", owned_child, ()));
    } else {
        assert!(old.as_ref().unwrap().root().unwrap().child(0).is_none());
        root = root.child(leaf()).child(cx.keyed("leaf", owned_child, ()));
    }
    cx.on(root, ActionKind::Click, move |cx, _| cx.set(n, 1))
}
#[test]
fn host_replacement_cannot_steal_a_moved_components_identity() {
    let mut rt = Runtime::new(replacement, ());
    let child = rt.root().children[0].clone();
    rt.dispatch(&[], ActionKind::Click, None).unwrap();
    rt.flush();
    assert!(!Arc::ptr_eq(&child, &rt.root().children[0]));
    assert!(Arc::ptr_eq(&child, &rt.root().children[1]));
}
fn changed(cx: &mut Cx<'_>, case: &usize) -> Node {
    let old = cx.previous_nodes();
    let n = cx.state(|| false);
    let mut e = leaf();
    if *case == 6 {
        e = e.child(Element::new(Role::Text).name("child"));
    }
    if cx.get(n) {
        match case {
            0 => e = e.name("different"),
            1 => e = e.prop("height", 9),
            2 => e = e.key("different"),
            3 => e.role = Role::Button,
            4 => e = e.action(ActionKind::Focus),
            5 => e = e.child(Element::new(Role::Group)),
            _ => {}
        }
    }
    let e = cx.on(e, ActionKind::Click, move |cx, _| cx.set(n, true));
    match old.as_ref().and_then(|o| o.root()) {
        Some(old) => old.reuse(e),
        None => e.into_node(),
    }
}
#[test]
fn changed_fields_and_new_child_identities_allocate_new_nodes() {
    for case in 0..7 {
        let mut rt = Runtime::new(changed, case);
        let old = rt.root().clone();
        rt.dispatch(&[], ActionKind::Click, None).unwrap();
        rt.flush();
        assert!(!Arc::ptr_eq(&old, rt.root()));
    }
}

#[test]
fn a_forwarded_component_root_is_not_owned_by_its_parent() {
    fn parent(cx: &mut Cx<'_>, _: &()) -> Node {
        let n = cx.state(|| 0);
        if cx.get(n) == 1 {
            assert!(
                cx.previous_nodes()
                    .expect("previous output")
                    .root()
                    .is_none()
            );
        }
        cx.effect((), move |updates| updates.set(n, 1));
        cx.keyed("child", owned_child, ())
    }
    let mut rt = Runtime::new(parent, ());
    rt.flush();
    assert!(rt.renders() >= 3);
}

#[test]
fn reused_children_allow_an_unchanged_host_container_to_be_shared() {
    fn parent(cx: &mut Cx<'_>, _: &()) -> Element {
        let old = cx.previous_nodes();
        let group = old.as_ref().and_then(|o| o.root()).and_then(|o| o.child(0));
        let n = cx.state(|| 0i64);
        let child = match group.and_then(|g| g.child(0)) {
            Some(old) => old.reuse(leaf()),
            None => leaf().into_node(),
        };
        let element = Element::new(Role::Group).child(child);
        let group = match group {
            Some(old) => old.reuse(element),
            None => element.into_node(),
        };
        let root = Element::new(Role::Group)
            .key("root")
            .child(group)
            .name(cx.get(n).to_string());
        cx.on(root, ActionKind::Click, move |cx, _| {
            cx.update(n, |n| *n += 1)
        })
    }
    let mut rt = Runtime::new(parent, ());
    let before = rt.root().children[0].clone();
    rt.dispatch(&[], ActionKind::Click, None).unwrap();
    rt.flush();
    assert!(Arc::ptr_eq(&before, &rt.root().children[0]));
}

#[test]
fn retargeting_motion_to_its_displayed_value_keeps_the_new_intent() {
    use rsui_core::PropValue;
    use std::time::{Duration, Instant};
    fn bar(cx: &mut Cx<'_>, mode: &(bool, bool)) -> Node {
        let old = cx.previous_nodes();
        let width = cx.state(|| 0.0);
        let mut element = Element::new(Role::Group)
            .key("bar")
            .prop("width", cx.get(width));
        element = if mode.1 {
            element.prop(
                "animation",
                "0.1 0 1 normal none linear\n0 width n 0\n1 width n 100",
            )
        } else {
            element.prop("transitions", "width n 0.1 0 linear")
        };
        let element = cx.on(element, ActionKind::SetValue, move |cx, value| {
            if let Some(PropValue::Float(v)) = value {
                cx.set(width, *v);
            }
        });
        if mode.0 {
            if let Some(old) = old.as_ref().and_then(|old| old.root()) {
                return old.reuse(element);
            }
        }
        element.into_node()
    }
    for animation in [false, true] {
        for reuse in [false, true] {
            let mut rt = Runtime::new(bar, (reuse, animation));
            let start = Instant::now() + Duration::from_secs(5);
            rt.tick(start);
            rt.set_animations(true);
            rt.flush();
            if !animation {
                rt.tick(start);
                rt.dispatch(&[], ActionKind::SetValue, Some(PropValue::Float(100.0)))
                    .unwrap();
                rt.flush();
            }
            rt.tick(start + Duration::from_millis(50));
            rt.flush();
            let Some(PropValue::Float(shown)) = rt.root().props.get("width") else {
                panic!("width")
            };
            let shown = *shown;
            assert!(
                shown > 0.0 && shown < 100.0,
                "shown={shown} reuse={reuse} animation={animation}"
            );
            rt.dispatch(&[], ActionKind::SetValue, Some(PropValue::Float(shown)))
                .unwrap();
            rt.flush();
            rt.tick(start + Duration::from_millis(250));
            rt.flush();
            assert_eq!(
                rt.root().props.get("width"),
                Some(&PropValue::Float(shown)),
                "reuse={reuse} animation={animation}"
            );
        }
    }
}
