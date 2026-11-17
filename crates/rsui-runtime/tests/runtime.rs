//! The guarantees components can rely on.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use proptest::prelude::*;
use rsui_core::{ActionKind, Element, Node, Patch, PropValue, Role, apply, snapshot_json, text};
use rsui_runtime::{Cx, DispatchError, ListWindow, Runtime};
use triomphe::Arc;

fn counter(cx: &mut Cx, _: &()) -> Element {
    let count = cx.state(|| 0i64);
    let name = text!("{}", cx.get(count));
    cx.on(
        Element::new(Role::Button).key("add").name(name),
        ActionKind::Click,
        move |cx, _| cx.update(count, |count| *count += 1),
    )
}

#[test]
fn an_action_updates_the_tree_through_patches() {
    let mut runtime = Runtime::new(counter, ());
    let old = runtime.root().clone();
    runtime.dispatch(&[], ActionKind::Click, None).unwrap();
    let patches = runtime.flush();

    assert!(matches!(patches.as_slice(), [Patch::SetName { .. }]));
    assert_eq!(runtime.root().name, "1");
    assert_eq!(apply(old.clone(), &patches).unwrap(), *runtime.root());
    assert_eq!(old.name, "0", "a held tree stays as it was");
}

#[test]
fn several_actions_share_one_render() {
    let mut runtime = Runtime::new(counter, ());
    let before = runtime.renders();
    for _ in 0..5 {
        runtime.dispatch(&[], ActionKind::Click, None).unwrap();
    }
    runtime.flush();
    assert_eq!(runtime.renders() - before, 1);
    assert_eq!(runtime.root().name, "5");
}

#[test]
fn setting_the_same_value_does_not_render() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        let value = cx.state(|| 7);
        cx.on(
            Element::new(Role::Button).key("same"),
            ActionKind::Click,
            move |cx, _| cx.set(value, 7),
        )
    }
    let mut runtime = Runtime::new(app, ());
    let before = runtime.renders();
    runtime.dispatch(&[], ActionKind::Click, None).unwrap();
    assert!(runtime.flush().is_empty());
    assert_eq!(runtime.renders(), before);
}

/// A child takes a parent's state as a prop, to change it from a handler.
fn state_child(cx: &mut Cx, count: &rsui_runtime::State<i64>) -> Element {
    ROW_RENDERS.with(|renders| renders.set(renders.get() + 1));
    let count = *count;
    cx.on(
        Element::new(Role::Button).key("child"),
        ActionKind::Click,
        move |cx, _| cx.update(count, |count| *count += 1),
    )
}

fn state_parent(cx: &mut Cx, _: &()) -> Element {
    let count = cx.state(|| 0i64);
    let child = cx.keyed("child", state_child, count);
    Element::new(Role::Group)
        .key("parent")
        .name(text!("{}", cx.get(count)))
        .child(child)
}

#[test]
fn a_child_given_a_parents_state_changes_the_parent_without_rendering_itself() {
    let mut runtime = Runtime::new(state_parent, ());
    let renders = row_renders(|| {
        runtime.dispatch(&[0], ActionKind::Click, None).unwrap();
        runtime.flush();
    });
    assert_eq!(renders, 0);
    assert_eq!(runtime.root().name, "1");
}

#[test]
fn states_are_equal_only_to_themselves() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        let first = cx.state(|| 0);
        let second = cx.state(|| 0);
        let copy = first;
        assert_eq!(first, copy);
        assert_ne!(first, second);
        Element::new(Role::Text)
    }
    Runtime::new(app, ());
}

#[test]
fn flushing_with_nothing_to_do_does_nothing() {
    let mut runtime = Runtime::new(counter, ());
    let before = runtime.renders();
    assert!(runtime.flush().is_empty());
    assert_eq!(runtime.renders(), before);
}

#[test]
fn a_rejected_action_reports_why() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        cx.on(
            Element::new(Role::TextInput).key("age"),
            ActionKind::SetValue,
            |cx, input| {
                if !matches!(input, Some(PropValue::Int(_))) {
                    cx.reject("age must be a number");
                }
            },
        )
    }
    let mut runtime = Runtime::new(app, ());
    assert_eq!(
        runtime.dispatch(&[], ActionKind::SetValue, Some("old".into())),
        Err(DispatchError::Rejected("age must be a number".into()))
    );
    assert_eq!(
        runtime.dispatch(&[], ActionKind::SetValue, Some(PropValue::Int(3))),
        Ok(())
    );
}

#[test]
fn bad_actions_are_errors() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        let button = cx.on(
            Element::new(Role::Button).key("yes"),
            ActionKind::Click,
            |_, _| {},
        );
        Element::new(Role::Group)
            .child(button)
            .child(
                Element::new(Role::Button)
                    .key("no")
                    .action(ActionKind::Click),
            )
            .child(Element::new(Role::Button).action(ActionKind::Click))
    }
    let mut runtime = Runtime::new(app, ());
    let mut act = |path: &[u32], action| runtime.dispatch(path, action, None);

    assert_eq!(act(&[0], ActionKind::Click), Ok(()));
    assert_eq!(act(&[9], ActionKind::Click), Err(DispatchError::NoSuchNode));
    assert_eq!(
        act(&[0, 0], ActionKind::Click),
        Err(DispatchError::NoSuchNode)
    );
    assert_eq!(
        act(&[0], ActionKind::Scroll),
        Err(DispatchError::ActionNotAvailable)
    );
    assert_eq!(act(&[1], ActionKind::Click), Err(DispatchError::NoHandler));
    assert_eq!(act(&[2], ActionKind::Click), Err(DispatchError::NoHandler));
}

thread_local! {
    static ROW_RENDERS: Cell<u32> = const { Cell::new(0) };
}

#[derive(PartialEq, Clone)]
struct RowProps {
    id: u64,
    label: &'static str,
}

fn row(cx: &mut Cx, props: &RowProps) -> Element {
    ROW_RENDERS.with(|renders| renders.set(renders.get() + 1));
    let clicks = cx.state(|| 0u32);
    let name = text!("{}#{}:{}", props.label, props.id, cx.get(clicks));
    cx.on(
        Element::new(Role::ListItem).name(name),
        ActionKind::Click,
        move |cx, _| cx.update(clicks, |clicks| *clicks += 1),
    )
}

fn row_renders(during: impl FnOnce()) -> u32 {
    let before = ROW_RENDERS.with(Cell::get);
    during();
    ROW_RENDERS.with(Cell::get) - before
}

/// A list whose order and label are state of the parent. Each row counts its clicks.
#[allow(clippy::ptr_arg)] // Props must own their value, so this has to be a Vec.
fn list(cx: &mut Cx, initial: &Vec<u64>) -> Element {
    let ids = cx.state(|| initial.clone());
    let label = cx.state(|| "row");
    let rows: Vec<Node> = cx
        .get(ids)
        .into_iter()
        .map(|id| {
            let label = cx.get(label);
            cx.keyed(id, row, RowProps { id, label })
        })
        .collect();
    let element = Element::new(Role::List).key("list").children(rows);
    let element = cx.on(element, ActionKind::Toggle, move |cx, _| {
        cx.update(ids, |ids| ids.reverse());
    });
    cx.on(element, ActionKind::Focus, move |cx, _| {
        cx.set(label, "other");
    })
}

#[test]
fn a_change_only_renders_the_component_it_belongs_to() {
    let mut runtime = Runtime::new(list, (0..100).collect());
    let renders = row_renders(|| {
        runtime.dispatch(&[50], ActionKind::Click, None).unwrap();
        let patches = runtime.flush();
        assert_eq!(patches.len(), 1);
        assert!(matches!(&patches[0], Patch::SetName { path, .. } if **path == [50]));
    });
    assert_eq!(renders, 1);
    assert_eq!(runtime.root().children[50].name, "row#50:1");
}

#[test]
fn a_parent_rendering_again_skips_children_with_the_same_props() {
    let mut runtime = Runtime::new(list, (0..100).collect());
    let old = runtime.root().clone();
    let renders = row_renders(|| {
        runtime.dispatch(&[], ActionKind::Toggle, None).unwrap();
        runtime.flush();
    });
    assert_eq!(renders, 0);
    assert!(Arc::ptr_eq(&old.children[0], &runtime.root().children[99]));
}

#[test]
fn changed_props_render_the_child_and_keep_its_state() {
    let mut runtime = Runtime::new(list, vec![1, 2]);
    runtime.dispatch(&[1], ActionKind::Click, None).unwrap();
    runtime.flush();
    let renders = row_renders(|| {
        runtime.dispatch(&[], ActionKind::Focus, None).unwrap();
        runtime.flush();
    });
    assert_eq!(renders, 2);
    assert_eq!(runtime.root().children[1].name, "other#2:1");
}

#[test]
fn reordering_keyed_children_keeps_their_state_and_only_moves_them() {
    let mut runtime = Runtime::new(list, vec![1, 2, 3]);
    runtime.dispatch(&[0], ActionKind::Click, None).unwrap();
    runtime.flush();
    runtime.dispatch(&[], ActionKind::Toggle, None).unwrap();
    let patches = runtime.flush();

    assert!(
        patches
            .iter()
            .all(|patch| matches!(patch, Patch::Move { .. }))
    );
    let names: Vec<&str> = runtime
        .root()
        .children
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names, ["row#3:0", "row#2:0", "row#1:1"]);

    // After moving, an action still reaches the right row.
    runtime.dispatch(&[2], ActionKind::Click, None).unwrap();
    let patches = runtime.flush();
    assert!(matches!(&patches[0], Patch::SetName { path, .. } if **path == [2]));
    assert_eq!(runtime.root().children[2].name, "row#1:2");
}

#[test]
fn updating_the_tree_does_not_copy_a_list_nobody_else_holds() {
    let mut runtime = Runtime::new(list, (0..100).collect());
    let list_before = Arc::as_ptr(runtime.root());
    runtime.dispatch(&[50], ActionKind::Click, None).unwrap();
    runtime.flush();
    assert_eq!(Arc::as_ptr(runtime.root()), list_before);
}

#[derive(Default)]
struct Log(RefCell<Vec<String>>);

impl PartialEq for Log {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

type SharedLog = Rc<Log>;

fn note(log: &SharedLog, entry: impl Into<String>) {
    log.0.borrow_mut().push(entry.into());
}

fn entries(log: &SharedLog) -> Vec<String> {
    std::mem::take(&mut log.0.borrow_mut())
}

fn effectful(cx: &mut Cx, log: &SharedLog) -> Element {
    let dep = cx.state(|| 0);
    let other = cx.state(|| 0);
    let value = cx.get(dep);
    let (run_log, clean_log) = (log.clone(), log.clone());
    cx.effect_with_cleanup(value, move |_| {
        note(&run_log, format!("run {value}"));
        move |_: &mut rsui_runtime::Updates<'_>| note(&clean_log, format!("clean {value}"))
    });
    let element = cx.on(
        Element::new(Role::Button).key("dep"),
        ActionKind::Click,
        move |cx, _| cx.update(dep, |dep| *dep += 1),
    );
    cx.on(element, ActionKind::Toggle, move |cx, _| {
        cx.update(other, |other| *other += 1);
    })
}

#[test]
fn effects_run_when_their_deps_change_and_clean_up_first() {
    let log = SharedLog::default();
    let mut runtime = Runtime::new(effectful, log.clone());
    assert_eq!(entries(&log), ["run 0"]);

    runtime.dispatch(&[], ActionKind::Toggle, None).unwrap();
    runtime.flush();
    assert_eq!(entries(&log), [] as [&str; 0], "the deps didn't change");

    runtime.dispatch(&[], ActionKind::Click, None).unwrap();
    runtime.flush();
    assert_eq!(entries(&log), ["clean 0", "run 1"]);

    drop(runtime);
    assert_eq!(entries(&log), ["clean 1"]);
}

fn switch(cx: &mut Cx, log: &SharedLog) -> Element {
    let shown = cx.state(|| true);
    let mut element = Element::new(Role::Group).key("switch");
    if cx.get(shown) {
        element = element.child(cx.child(effectful, log.clone()));
    }
    cx.on(element, ActionKind::Toggle, move |cx, _| {
        cx.update(shown, |shown| *shown = !*shown);
    })
}

#[test]
fn unmounting_runs_cleanups_and_remounting_starts_fresh() {
    let log = SharedLog::default();
    let mut runtime = Runtime::new(switch, log.clone());
    runtime.dispatch(&[0], ActionKind::Click, None).unwrap();
    runtime.flush();
    entries(&log);

    runtime.dispatch(&[], ActionKind::Toggle, None).unwrap();
    runtime.flush();
    assert_eq!(entries(&log), ["clean 1"]);
    assert!(runtime.root().children.is_empty());

    runtime.dispatch(&[], ActionKind::Toggle, None).unwrap();
    runtime.flush();
    assert_eq!(entries(&log), ["run 0"]);
}

#[test]
fn an_effect_can_change_state_and_the_flush_finishes_that_too() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        let ready = cx.state(|| false);
        cx.effect((), move |cx| cx.set(ready, true));
        let name = if cx.get(ready) { "ready" } else { "loading" };
        Element::new(Role::Text).name(name)
    }
    let runtime = Runtime::new(app, ());
    assert_eq!(runtime.root().name, "ready");
}

#[test]
fn an_effect_that_never_settles_is_stopped_and_the_rest_still_works() {
    fn runaway(cx: &mut Cx, _: &()) -> Element {
        let count = cx.state(|| 0u64);
        let value = cx.get(count);
        cx.effect(value, move |cx| cx.set(count, value + 1));
        Element::new(Role::Text).name(text!("{value}"))
    }
    fn app(cx: &mut Cx, _: &()) -> Element {
        let clicks = cx.state(|| 0u32);
        let label = text!("clicked {}", cx.get(clicks));
        let button = cx.on(
            Element::new(Role::Button).key("button").name(label),
            ActionKind::Click,
            move |cx, _| cx.update(clicks, |clicks| *clicks += 1),
        );
        let runaway = cx.child(runaway, ());
        Element::new(Role::Group).child(button).child(runaway)
    }
    let mut runtime = Runtime::new(app, ());
    assert_eq!(runtime.runaways(), 1);
    let stopped_at = runtime.root().children[1].name.clone();

    // Nothing carries on by itself once it's been stopped.
    assert!(runtime.flush().is_empty());
    assert_eq!(runtime.runaways(), 1);
    assert_eq!(runtime.root().children[1].name, stopped_at);

    // And what's next to it is fine.
    runtime.dispatch(&[0], ActionKind::Click, None).unwrap();
    runtime.flush();
    assert_eq!(runtime.root().children[0].name, "clicked 1");
}

#[test]
fn what_asked_is_told_when_effects_are_stopped_and_can_show_it() {
    thread_local! {
        static TOLD: Cell<u32> = const { Cell::new(0) };
    }
    fn app(cx: &mut Cx, _: &()) -> Element {
        cx.on_runaway(|_| TOLD.with(|told| told.set(told.get() + 1)));
        let count = cx.state(|| 0u64);
        let value = cx.get(count);
        cx.effect(value, move |cx| cx.set(count, value + 1));
        Element::new(Role::Text)
    }
    Runtime::new(app, ());
    assert_eq!(TOLD.with(Cell::get), 1);
}

#[test]
fn a_memo_only_recomputes_when_its_deps_change() {
    thread_local! {
        static COMPUTED: Cell<u32> = const { Cell::new(0) };
    }
    fn app(cx: &mut Cx, _: &()) -> Node {
        let dep = cx.state(|| 0);
        let other = cx.state(|| 0);
        let value = cx.get(dep);
        let label = cx.memo(value, || {
            COMPUTED.with(|count| count.set(count.get() + 1));
            Element::new(Role::Text).name(text!("{value}")).into_node()
        });
        let element = Element::new(Role::Group).key("memo").child(label);
        let element = cx.on(element, ActionKind::Click, move |cx, _| {
            cx.update(dep, |dep| *dep += 1);
        });
        cx.on(element, ActionKind::Toggle, move |cx, _| {
            cx.update(other, |other| *other += 1);
        })
        .into_node()
    }
    let computed = || COMPUTED.with(Cell::get);
    let mut runtime = Runtime::new(app, ());
    assert_eq!(computed(), 1);
    runtime.dispatch(&[], ActionKind::Toggle, None).unwrap();
    runtime.flush();
    assert_eq!(computed(), 1);
    runtime.dispatch(&[], ActionKind::Click, None).unwrap();
    runtime.flush();
    assert_eq!(computed(), 2);
    assert_eq!(runtime.root().children[0].name, "1");
}

#[test]
#[should_panic(expected = "same hooks")]
fn calling_different_hooks_panics() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        let flag = cx.state(|| false);
        if cx.get(flag) {
            cx.state(|| "a different type");
        }
        cx.on(
            Element::new(Role::Button).key("flag"),
            ActionKind::Click,
            move |cx, _| cx.set(flag, true),
        )
    }
    let mut runtime = Runtime::new(app, ());
    runtime.dispatch(&[], ActionKind::Click, None).unwrap();
    runtime.flush();
}

#[test]
#[should_panic(expected = "needs a key")]
fn a_handler_on_an_element_without_a_key_panics() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        cx.on(Element::new(Role::Button), ActionKind::Click, |_, _| {})
    }
    Runtime::new(app, ());
}

#[test]
fn a_different_component_in_the_same_place_starts_fresh() {
    fn first(cx: &mut Cx, _: &()) -> Element {
        let value = cx.state(|| 1);
        Element::new(Role::Text).name(text!("first {}", cx.get(value)))
    }
    fn second(cx: &mut Cx, _: &()) -> Element {
        let value = cx.state(|| "text");
        Element::new(Role::Text).name(text!("second {}", cx.get(value)))
    }
    fn app(cx: &mut Cx, _: &()) -> Element {
        let swap = cx.state(|| false);
        let child = if cx.get(swap) {
            cx.child(second, ())
        } else {
            cx.child(first, ())
        };
        cx.on(
            Element::new(Role::Group).key("app").child(child),
            ActionKind::Click,
            move |cx, _| cx.set(swap, true),
        )
    }
    let mut runtime = Runtime::new(app, ());
    runtime.dispatch(&[], ActionKind::Click, None).unwrap();
    runtime.flush();
    assert_eq!(runtime.root().children[0].name, "second text");
}

/// Two children given places, one of them only sometimes, and a keyed one in a place of its own.
fn placed(cx: &mut Cx, log: &SharedLog) -> Element {
    let first = cx.state(|| false);
    let mut element = Element::new(Role::Group).key("placed");
    if cx.get(first) {
        element = element.child(cx.child_at(None, 10, effectful, log.clone(), |_, _| true));
    }
    element = element
        .child(cx.child_at(None, 11, effectful, log.clone(), |_, _| true))
        .child(cx.child_at(Some(rsui_core::Key::from("k")), 12, effectful, log.clone(), |_, _| true));
    cx.on(element, ActionKind::Toggle, move |cx, _| {
        cx.update(first, |first| *first = !*first);
    })
}

#[test]
fn a_child_at_a_place_keeps_its_state_when_one_before_it_comes_and_goes() {
    let log = SharedLog::default();
    let mut runtime = Runtime::new(placed, log.clone());
    for path in [0, 1, 1] {
        runtime.dispatch(&[path], ActionKind::Click, None).unwrap();
        runtime.flush();
    }
    assert_eq!(
        entries(&log),
        ["run 0", "run 0", "clean 0", "run 1", "clean 0", "run 1", "clean 1", "run 2"]
    );

    // Only the first comes and goes, as the others are where they were, with what they were clicked to.
    runtime.dispatch(&[], ActionKind::Toggle, None).unwrap();
    runtime.flush();
    assert_eq!(entries(&log), ["run 0"]);
    assert_eq!(runtime.root().children.len(), 3);
    runtime.dispatch(&[2], ActionKind::Click, None).unwrap();
    runtime.flush();
    assert_eq!(entries(&log), ["clean 2", "run 3"]);
    runtime.dispatch(&[], ActionKind::Toggle, None).unwrap();
    runtime.flush();
    assert_eq!(entries(&log), ["clean 0"]);
    runtime.dispatch(&[0], ActionKind::Click, None).unwrap();
    runtime.flush();
    assert_eq!(entries(&log), ["clean 1", "run 2"]);
}

#[test]
fn a_component_can_be_the_whole_output_of_another() {
    fn wrapper(cx: &mut Cx, _: &()) -> Node {
        cx.keyed("inner", counter, ())
    }
    let mut runtime = Runtime::new(wrapper, ());
    runtime.dispatch(&[], ActionKind::Click, None).unwrap();
    runtime.flush();
    assert_eq!(runtime.root().name, "1");
    assert_eq!(
        snapshot_json(runtime.root()),
        r#"{"key":"add","role":"button","name":"1","actions":["click"]}"#
    );
}

#[test]
fn a_child_keeps_the_key_its_parent_gave_it_when_it_renders_alone() {
    fn unkeyed(cx: &mut Cx, _: &()) -> Element {
        let count = cx.state(|| 0);
        let name = text!("{}", cx.get(count));
        cx.on(
            Element::new(Role::Button).name(name),
            ActionKind::Click,
            move |cx, _| cx.update(count, |count| *count += 1),
        )
    }
    fn app(cx: &mut Cx, _: &()) -> Element {
        Element::new(Role::Group).child(cx.keyed(5u64, unkeyed, ()))
    }
    let mut runtime = Runtime::new(app, ());
    runtime.dispatch(&[0], ActionKind::Click, None).unwrap();
    let patches = runtime.flush();
    assert!(matches!(patches.as_slice(), [Patch::SetName { .. }]));
    assert_eq!(runtime.root().children[0].key, Some(5u64.into()));
}

#[test]
fn a_child_keeps_the_key_of_its_keyed_parent_when_it_renders_alone() {
    fn inner(cx: &mut Cx, _: &()) -> Element {
        let count = cx.state(|| 0);
        cx.effect((), move |cx| cx.set(count, 1));
        Element::new(Role::Text).name(text!("{}", cx.get(count)))
    }
    fn wrapper(cx: &mut Cx, _: &()) -> Node {
        cx.child(inner, ())
    }
    fn app(cx: &mut Cx, _: &()) -> Element {
        Element::new(Role::Group).child(cx.keyed(7u64, wrapper, ()))
    }
    // The effect renders `inner` by itself, whose output has no key of its own.
    let runtime = Runtime::new(app, ());
    assert_eq!(runtime.root().children[0].name, "1");
    assert_eq!(runtime.root().children[0].key, Some(7u64.into()));
}

#[test]
fn a_parent_and_child_changing_together_render_the_child_once() {
    let mut runtime = Runtime::new(list, vec![1, 2]);
    let renders = row_renders(|| {
        runtime.dispatch(&[0], ActionKind::Click, None).unwrap();
        runtime.dispatch(&[], ActionKind::Focus, None).unwrap();
        runtime.flush();
    });
    assert_eq!(renders, 2, "each of the two rows renders once");
    assert_eq!(runtime.root().children[0].name, "other#1:1");
}

#[test]
fn a_child_changing_as_its_parent_removes_it_does_not_render() {
    fn parent(cx: &mut Cx, _: &()) -> Element {
        let ids = cx.state(|| vec![1u64]);
        let rows: Vec<Node> = cx
            .get(ids)
            .into_iter()
            .map(|id| cx.keyed(id, row, RowProps { id, label: "row" }))
            .collect();
        cx.on(
            Element::new(Role::List).key("list").children(rows),
            ActionKind::Toggle,
            move |cx, _| cx.update(ids, Vec::clear),
        )
    }
    let mut runtime = Runtime::new(parent, ());
    let renders = row_renders(|| {
        runtime.dispatch(&[0], ActionKind::Click, None).unwrap();
        runtime.dispatch(&[], ActionKind::Toggle, None).unwrap();
        runtime.flush();
    });
    assert_eq!(renders, 0);
    assert!(runtime.root().children.is_empty());
}

#[test]
fn children_placed_out_of_render_order_still_work() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        let first = cx.keyed(
            1u64,
            row,
            RowProps {
                id: 1,
                label: "row",
            },
        );
        let second = cx.keyed(
            2u64,
            row,
            RowProps {
                id: 2,
                label: "row",
            },
        );
        Element::new(Role::List).child(second).child(first)
    }
    let mut runtime = Runtime::new(app, ());
    runtime.dispatch(&[1], ActionKind::Click, None).unwrap();
    runtime.dispatch(&[0], ActionKind::Click, None).unwrap();
    runtime.dispatch(&[0], ActionKind::Click, None).unwrap();
    let patches = runtime.flush();

    assert_eq!(patches.len(), 2);
    assert_eq!(runtime.root().children[0].name, "row#2:2");
    assert_eq!(runtime.root().children[1].name, "row#1:1");
}

#[test]
fn an_action_on_a_node_between_children_finds_its_handler() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        let before = cx.keyed(
            1u64,
            row,
            RowProps {
                id: 1,
                label: "row",
            },
        );
        let after = cx.keyed(
            2u64,
            row,
            RowProps {
                id: 2,
                label: "row",
            },
        );
        let hits = cx.state(|| 0);
        let name = text!("{}", cx.get(hits));
        let middle = cx.on(
            Element::new(Role::Button).key("middle").name(name),
            ActionKind::Click,
            move |cx, _| cx.update(hits, |hits| *hits += 1),
        );
        Element::new(Role::List)
            .child(before)
            .child(middle)
            .child(after)
    }
    let mut runtime = Runtime::new(app, ());
    runtime.dispatch(&[1], ActionKind::Click, None).unwrap();
    runtime.dispatch(&[2], ActionKind::Click, None).unwrap();
    runtime.flush();
    assert_eq!(runtime.root().children[1].name, "1");
    assert_eq!(runtime.root().children[2].name, "row#2:1");
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "only appear once")]
fn a_childs_output_appearing_twice_panics() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        let child = cx.child(counter, ());
        Element::new(Role::Group).child(child.clone()).child(child)
    }
    Runtime::new(app, ());
}

#[test]
fn a_child_left_out_of_the_output_is_unmounted() {
    let log = SharedLog::default();
    fn app(cx: &mut Cx, log: &SharedLog) -> Element {
        let _dropped = cx.child(effectful, log.clone());
        Element::new(Role::Group)
    }
    drop(Runtime::new(app, log.clone()));
    assert_eq!(entries(&log), [] as [&str; 0], "its effect never ran");
}

fn long_list(cx: &mut Cx, len: &usize) -> Element {
    cx.virtual_list(
        "rows",
        ListWindow {
            len: *len,
            visible: 10,
            overscan: 2,
            reveal: None,
        },
        |cx, index| {
            cx.keyed(
                index,
                row,
                RowProps {
                    id: index as u64,
                    label: "row",
                },
            )
        },
    )
}

#[test]
fn a_virtual_list_only_builds_what_is_in_view() {
    let renders = row_renders(|| {
        let runtime = Runtime::new(long_list, 1_000_000);
        assert_eq!(runtime.root().children.len(), 12);
        assert_eq!(
            runtime.root().props.get("count"),
            Some(&PropValue::Int(1_000_000))
        );
    });
    assert_eq!(renders, 12);
}

#[test]
fn scrolling_a_virtual_list_only_renders_rows_that_came_into_view() {
    let mut runtime = Runtime::new(long_list, 1000);
    let mirror = runtime.root().clone();
    let renders = row_renders(|| {
        runtime
            .dispatch(&[], ActionKind::Scroll, Some(PropValue::Int(5)))
            .unwrap();
    });
    assert_eq!(renders, 0, "nothing renders until the flush");
    let mut patches = Vec::new();
    let renders = row_renders(|| patches = runtime.flush());
    assert_eq!(renders, 5);
    assert_eq!(apply(mirror, &patches).unwrap(), *runtime.root());
    let root = runtime.root();
    assert_eq!(root.props.get("first_visible"), Some(&PropValue::Int(5)));
    assert_eq!(root.props.get("first_built"), Some(&PropValue::Int(3)));
    assert_eq!(root.children[0].key, Some(3usize.into()));
    assert_eq!(root.children.len(), 14);
}

#[test]
fn a_virtual_list_stops_at_its_ends_and_refuses_bad_scrolls() {
    let mut runtime = Runtime::new(long_list, 100);
    runtime
        .dispatch(&[], ActionKind::Scroll, Some(PropValue::Int(1_000)))
        .unwrap();
    runtime.flush();
    let root = runtime.root();
    assert_eq!(root.props.get("first_visible"), Some(&PropValue::Int(90)));
    assert_eq!(root.children.last().unwrap().key, Some(99usize.into()));

    for input in [Some(PropValue::Int(-1)), Some("top".into()), None] {
        assert!(matches!(
            runtime.dispatch(&[], ActionKind::Scroll, input),
            Err(DispatchError::Rejected(_))
        ));
    }
}

#[test]
fn a_short_virtual_list_shows_everything() {
    let runtime = Runtime::new(long_list, 3);
    assert_eq!(runtime.root().children.len(), 3);
    let empty = Runtime::new(long_list, 0);
    assert!(empty.root().children.is_empty());
}

/// An app with structure that changes, for checking the runtime against a model.
/// It holds rows in order, each with a nested badge, and has a changeable title.
mod model {
    use super::*;

    #[derive(Debug, Clone)]
    pub enum Action {
        Add(usize),
        Remove(usize),
        Rotate(usize),
        Reverse,
        Title,
        Click(usize),
        BadgeClick(usize),
    }

    #[derive(PartialEq)]
    struct RowProps {
        id: u64,
    }

    fn badge(cx: &mut Cx, _: &()) -> Element {
        let clicks = cx.state(|| 0u32);
        let name = text!("{}", cx.get(clicks));
        cx.on(
            Element::new(Role::Text).key("badge").name(name),
            ActionKind::Click,
            move |cx, _| cx.update(clicks, |clicks| *clicks += 1),
        )
    }

    fn row(cx: &mut Cx, props: &RowProps) -> Element {
        let clicks = cx.state(|| 0u32);
        let name = text!("{}:{}", props.id, cx.get(clicks));
        let badge = cx.child(badge, ());
        cx.on(
            Element::new(Role::ListItem).name(name).child(badge),
            ActionKind::Click,
            move |cx, _| cx.update(clicks, |clicks| *clicks += 1),
        )
    }

    pub fn app(cx: &mut Cx, _: &()) -> Element {
        let ids = cx.state(Vec::<u64>::new);
        let next = cx.state(|| 0u64);
        let title = cx.state(|| 0u32);
        let rows: Vec<Node> = cx
            .get(ids)
            .into_iter()
            .map(|id| cx.keyed(id, row, RowProps { id }))
            .collect();
        let add = cx.on(
            Element::new(Role::Button).key("add"),
            ActionKind::SetValue,
            move |cx, input| {
                let Some(PropValue::Int(position)) = input else {
                    return;
                };
                let id = cx.get(next);
                cx.set(next, id + 1);
                cx.update(ids, |ids| ids.insert(*position as usize, id));
            },
        );
        let edit = cx.on(
            Element::new(Role::Button).key("edit"),
            ActionKind::SetValue,
            move |cx, input| {
                let Some(PropValue::Int(position)) = input else {
                    return;
                };
                cx.update(ids, |ids| {
                    ids.remove(*position as usize);
                });
            },
        );
        let edit = cx.on(edit, ActionKind::Scroll, move |cx, input| {
            let Some(PropValue::Int(by)) = input else {
                return;
            };
            cx.update(ids, |ids| ids.rotate_left(*by as usize));
        });
        let edit = cx.on(edit, ActionKind::Toggle, move |cx, _| {
            cx.update(ids, |ids| ids.reverse());
        });
        let edit = cx.on(edit, ActionKind::Focus, move |cx, _| {
            cx.update(title, |title| *title += 1);
        });
        let list = Element::new(Role::List).key("rows").children(rows);
        Element::new(Role::Window)
            .key("app")
            .name(text!("title {}", cx.get(title)))
            .child(add)
            .child(edit)
            .child(list)
    }

    /// What the tree must look like, computed separately from the components.
    pub fn expected(rows: &[(u64, u32, u32)], title: u32) -> Node {
        let button = |key: &str, actions: &[ActionKind]| {
            actions
                .iter()
                .fold(Element::new(Role::Button).key(key), |element, action| {
                    element.action(*action)
                })
        };
        Element::new(Role::Window)
            .key("app")
            .name(text!("title {title}"))
            .child(button("add", &[ActionKind::SetValue]))
            .child(button(
                "edit",
                &[
                    ActionKind::SetValue,
                    ActionKind::Scroll,
                    ActionKind::Toggle,
                    ActionKind::Focus,
                ],
            ))
            .child(
                Element::new(Role::List)
                    .key("rows")
                    .children(rows.iter().map(|&(id, clicks, badge)| {
                        Element::new(Role::ListItem)
                            .key(id)
                            .name(text!("{id}:{clicks}"))
                            .action(ActionKind::Click)
                            .child(
                                Element::new(Role::Text)
                                    .key("badge")
                                    .name(text!("{badge}"))
                                    .action(ActionKind::Click),
                            )
                    })),
            )
            .into_node()
    }
}

fn action() -> impl Strategy<Value = model::Action> {
    use model::Action::*;
    prop_oneof![
        3 => (0usize..50).prop_map(Add),
        1 => (0usize..50).prop_map(Remove),
        1 => (0usize..50).prop_map(Rotate),
        1 => Just(Reverse),
        1 => Just(Title),
        3 => (0usize..50).prop_map(Click),
        2 => (0usize..50).prop_map(BadgeClick),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// After any sequence of actions, in batches of any size, the tree matches a
    /// separate model of the app, and the patches take a mirror to the same tree.
    #[test]
    fn the_tree_and_the_patches_always_match_a_model(
        batches in prop::collection::vec(prop::collection::vec(action(), 1..5), 1..30),
    ) {
        let mut runtime = Runtime::new(model::app, ());
        let mut mirror = runtime.root().clone();
        let (mut rows, mut title, mut next) = (Vec::<(u64, u32, u32)>::new(), 0, 0);

        for batch in batches {
            // State changes wait for the flush, so these all act on the same tree.
            let shown = rows.clone();
            for action in batch {
                let int = |value: usize| Some(PropValue::Int(value as i64));
                match action {
                    model::Action::Add(at) => {
                        let at = at % (rows.len() + 1);
                        runtime.dispatch(&[0], ActionKind::SetValue, int(at)).unwrap();
                        rows.insert(at, (next, 0, 0));
                        next += 1;
                    }
                    model::Action::Remove(at) if !rows.is_empty() => {
                        let at = at % rows.len();
                        runtime.dispatch(&[1], ActionKind::SetValue, int(at)).unwrap();
                        rows.remove(at);
                    }
                    model::Action::Rotate(by) if !rows.is_empty() => {
                        let by = by % rows.len();
                        runtime.dispatch(&[1], ActionKind::Scroll, int(by)).unwrap();
                        rows.rotate_left(by);
                    }
                    model::Action::Reverse => {
                        runtime.dispatch(&[1], ActionKind::Toggle, None).unwrap();
                        rows.reverse();
                    }
                    model::Action::Title => {
                        runtime.dispatch(&[1], ActionKind::Focus, None).unwrap();
                        title += 1;
                    }
                    model::Action::Click(at) if !shown.is_empty() => {
                        let at = at % shown.len();
                        runtime.dispatch(&[2, at as u32], ActionKind::Click, None).unwrap();
                        let id = shown[at].0;
                        if let Some(row) = rows.iter_mut().find(|row| row.0 == id) {
                            row.1 += 1;
                        }
                    }
                    model::Action::BadgeClick(at) if !shown.is_empty() => {
                        let at = at % shown.len();
                        runtime.dispatch(&[2, at as u32, 0], ActionKind::Click, None).unwrap();
                        let id = shown[at].0;
                        if let Some(row) = rows.iter_mut().find(|row| row.0 == id) {
                            row.2 += 1;
                        }
                    }
                    _ => {}
                }
            }
            let patches = runtime.flush();
            mirror = apply(mirror, &patches).unwrap();
            prop_assert_eq!(&mirror, runtime.root());
            prop_assert_eq!(runtime.root().clone(), model::expected(&rows, title));
        }
    }
}

fn keyed_counter(cx: &mut Cx, _: &()) -> Element {
    let count = cx.state(|| 0i64);
    let name = text!("{}", cx.get(count));
    cx.on(
        Element::new(Role::Button).name(name),
        ActionKind::Click,
        move |cx, _| cx.update(count, |count| *count += 1),
    )
}

/// A row for each of these keys, in this order, each counting its own clicks.
fn keyed_rows(cx: &mut Cx, keys: &Rc<RefCell<Vec<u64>>>) -> Element {
    let version = cx.state(|| 0u32);
    cx.get(version);
    let keys = keys.borrow().clone();
    let rows: Vec<_> = keys
        .into_iter()
        .map(|key| cx.keyed(key, keyed_counter, ()))
        .collect();
    let group = rows
        .into_iter()
        .fold(Element::new(Role::Group).key("rows"), Element::child);
    cx.on(group, ActionKind::Toggle, move |cx, _| {
        cx.update(version, |version| *version += 1)
    })
}

fn counts(runtime: &Runtime) -> Vec<String> {
    runtime
        .root()
        .children
        .iter()
        .map(|row| row.name.to_string())
        .collect()
}

/// Clicks each row as many times as its place in the list, and then shows that list in another order.
fn state_after(old: &[u64], new: &[u64]) -> Vec<String> {
    let keys = Rc::new(RefCell::new(old.to_vec()));
    let mut runtime = Runtime::new(keyed_rows, keys.clone());
    for (row, _) in old.iter().enumerate() {
        for _ in 0..row {
            runtime
                .dispatch(&[row as u32], ActionKind::Click, None)
                .unwrap();
        }
    }
    runtime.flush();
    *keys.borrow_mut() = new.to_vec();
    runtime.dispatch(&[], ActionKind::Toggle, None).unwrap();
    runtime.flush();
    counts(&runtime)
}

#[test]
fn rows_keep_their_state_however_the_list_changes() {
    // Each row's count starts as its place in the old list, so the counts show who's who.
    assert_eq!(
        state_after(&[10, 11, 12, 13], &[11, 12, 13]),
        ["1", "2", "3"],
        "first removed"
    );
    assert_eq!(
        state_after(&[10, 11, 12, 13], &[10, 12, 13]),
        ["0", "2", "3"],
        "middle removed"
    );
    assert_eq!(
        state_after(&[10, 11, 12, 13], &[11, 12, 13, 10]),
        ["1", "2", "3", "0"],
        "first moved to the end"
    );
    assert_eq!(
        state_after(&[10, 11, 12, 13], &[12, 13, 10, 11]),
        ["2", "3", "0", "1"],
        "two moved"
    );
    assert_eq!(
        state_after(&[10, 11, 12, 13], &[13, 12, 11, 10]),
        ["3", "2", "1", "0"],
        "reversed"
    );
    assert_eq!(
        state_after(&[10, 11, 12], &[99, 10, 11, 12]),
        ["0", "0", "1", "2"],
        "new one first"
    );
    assert_eq!(
        state_after(&[10, 11, 12], &[10, 99, 12, 11]),
        ["0", "0", "2", "1"],
        "new one between"
    );
}

#[test]
fn a_repeated_key_gives_a_new_row_rather_than_a_crash() {
    assert_eq!(
        state_after(&[10, 11, 12], &[11, 10, 10, 12]),
        ["1", "0", "0", "2"]
    );
    assert_eq!(state_after(&[10, 11, 12], &[12, 12, 12]), ["2", "0", "0"]);
}

fn fading(cx: &mut Cx, _: &()) -> Element {
    let shown = cx.state(|| true);
    let opacity = if cx.get(shown) { 1.0 } else { 0.0 };
    cx.on(
        Element::new(Role::Button)
            .key("fade")
            .prop("opacity", opacity)
            .prop("transitions", "opacity n 1 0 linear"),
        ActionKind::Click,
        move |cx, _| cx.update(shown, |shown| *shown = !*shown),
    )
}

fn opacity_of(runtime: &Runtime) -> f64 {
    match runtime.root().props.get("opacity") {
        Some(PropValue::Float(value)) => *value,
        Some(PropValue::Int(value)) => *value as f64,
        other => panic!("no opacity: {other:?}"),
    }
}

#[test]
fn a_prop_that_says_it_moves_goes_to_its_new_value_over_time() {
    let mut runtime = Runtime::new(fading, ());
    runtime.set_animations(true);
    runtime.flush();
    let start = std::time::Instant::now();
    runtime.dispatch(&[], ActionKind::Click, None).unwrap();
    let old = runtime.root().clone();
    let patches = runtime.flush();
    // It starts from where it was, and the patches say so.
    assert_eq!(opacity_of(&runtime), 1.0);
    assert_eq!(apply(old, &patches).unwrap(), *runtime.root());
    assert!(runtime.is_animating());
    runtime.tick(start + std::time::Duration::from_millis(500));
    let old = runtime.root().clone();
    let patches = runtime.flush();
    assert!(
        (opacity_of(&runtime) - 0.5).abs() < 0.05,
        "{}",
        opacity_of(&runtime)
    );
    assert_eq!(apply(old, &patches).unwrap(), *runtime.root());
    // Turned back halfway, it goes back from where it's got to.
    runtime.dispatch(&[], ActionKind::Click, None).unwrap();
    runtime.flush();
    assert!((opacity_of(&runtime) - 0.5).abs() < 0.05);
    runtime.tick(start + std::time::Duration::from_millis(3000));
    runtime.flush();
    assert_eq!(opacity_of(&runtime), 1.0);
    assert!(!runtime.is_animating(), "it's still once it's there");
}

#[test]
fn with_things_still_a_prop_goes_straight_to_its_new_value() {
    let mut runtime = Runtime::new(fading, ());
    runtime.dispatch(&[], ActionKind::Click, None).unwrap();
    runtime.flush();
    assert_eq!(opacity_of(&runtime), 0.0);
    assert!(!runtime.is_animating());
}

fn entering(_: &mut Cx, fill: &&'static str) -> Element {
    Element::new(Role::Group).prop("opacity", 1.0).prop(
        "animation",
        format!("1 0 1 normal {fill} linear\n0 opacity n 0\n1 opacity n 0.5").as_str(),
    )
}

#[test]
fn an_animation_plays_when_something_first_shows_and_leaves_it_as_it_was() {
    let mut runtime = Runtime::new(entering, "none");
    runtime.set_animations(true);
    let start = std::time::Instant::now();
    runtime.flush();
    assert!(
        opacity_of(&runtime) < 0.01,
        "it starts at the first keyframe"
    );
    runtime.tick(start + std::time::Duration::from_millis(500));
    runtime.flush();
    assert!(
        (opacity_of(&runtime) - 0.25).abs() < 0.05,
        "{}",
        opacity_of(&runtime)
    );
    runtime.tick(start + std::time::Duration::from_millis(1500));
    runtime.flush();
    assert_eq!(
        opacity_of(&runtime),
        1.0,
        "once it's done, it's what the element says"
    );
    assert!(!runtime.is_animating());
}

#[test]
fn with_things_still_an_animation_that_stays_where_it_ends_is_there_at_once() {
    let mut runtime = Runtime::new(entering, "forwards");
    runtime.flush();
    assert_eq!(opacity_of(&runtime), 0.5);
    let mut plain = Runtime::new(entering, "none");
    plain.flush();
    assert_eq!(opacity_of(&plain), 1.0);
}
