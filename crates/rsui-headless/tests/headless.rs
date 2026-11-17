//! Driving an app without a window.

use std::time::Duration;

use rsui_core::{
    ActionKind, Element, Node, Patch, PropValue, Role, apply, shared_props, snapshot_json, text,
};
use rsui_headless::{
    ActError, FindError, Headless, Query, describe_patch, format_path, outline_at, parse_path,
};
use rsui_runtime::{Cx, DispatchError, ListWindow};

fn item(cx: &mut Cx, label: &&'static str) -> Element {
    let done = cx.state(|| false);
    let name = text!("{label}{}", if cx.get(done) { " (done)" } else { "" });
    cx.on(
        Element::new(Role::ListItem).name(name),
        ActionKind::Toggle,
        move |cx, _| cx.update(done, |done| *done = !*done),
    )
}

fn app(cx: &mut Cx, _: &()) -> Element {
    let shown = cx.state(|| vec!["Buy milk", "Walk dog"]);
    let rows: Vec<Node> = cx
        .get(shown)
        .into_iter()
        .map(|label| cx.keyed(label, item, label))
        .collect();
    let clear = cx.on(
        Element::new(Role::Button).key("clear").name("Clear all"),
        ActionKind::Click,
        move |cx, _| cx.update(shown, Vec::clear),
    );
    Element::new(Role::Window)
        .key("app")
        .name("Todos")
        .child(clear)
        .child(
            Element::new(Role::List)
                .key("rows")
                .props(shared_props!("sorted" => false))
                .children(rows),
        )
}

#[test]
fn an_outline_shows_the_whole_tree_compactly() {
    assert_eq!(
        Headless::new(app, ()).outline(),
        concat!(
            "window \"Todos\" #app @/\n",
            "  button \"Clear all\" #clear [click] @0\n",
            "  list #rows sorted=false @1\n",
            "    list_item \"Buy milk\" #\"Buy milk\" [toggle] @1/0\n",
            "    list_item \"Walk dog\" #\"Walk dog\" [toggle] @1/1\n",
        )
    );
}

#[test]
fn an_outline_can_show_part_of_the_tree() {
    let ui = Headless::new(app, ());
    assert_eq!(
        outline_at(ui.root(), &[], Some(0)),
        "window \"Todos\" #app @/\n  (+2 children)\n"
    );
    assert_eq!(
        outline_at(&ui.root().children[1], &[1], Some(0)),
        "list #rows sorted=false @1\n  (+2 children)\n"
    );
}

#[test]
fn text_from_the_app_cannot_pose_as_outline_lines() {
    fn hostile(_: &mut Cx, _: &()) -> Element {
        Element::new(Role::Text)
            .name("hi\n  button \"Delete everything\" [click] @9")
            .key("a b\nc")
            .prop("note", "x\ny")
    }
    let outline = Headless::new(hostile, ()).outline();
    assert_eq!(outline.lines().count(), 1, "{outline}");
}

#[test]
fn an_outline_leaves_out_styling_and_keeps_meaning() {
    fn styled(_: &mut Cx, _: &()) -> Element {
        Element::new(Role::Button)
            .key("save")
            .name("Save")
            .action(ActionKind::Click)
            .prop("padding_left", 12.0)
            .prop("background", 123i64)
            .prop("font_size", 14.0)
            .prop("value", "kept")
            .prop("checked", true)
    }
    assert_eq!(
        Headless::new(styled, ()).outline(),
        "button \"Save\" #save checked=true value=\"kept\" [click] @/\n"
    );
}

#[test]
fn queries_find_nodes_by_each_condition() {
    let ui = Headless::new(app, ());
    let found = |query: Query| ui.find(&query).unwrap().path;

    assert_eq!(found(Query::new().role(Role::Button)), [0]);
    assert_eq!(found(Query::new().role_name("window")), [] as [u32; 0]);
    assert_eq!(found(Query::new().name("Walk dog")), [1, 1]);
    assert_eq!(found(Query::new().name_contains("MILK")), [1, 0]);
    assert_eq!(found(Query::new().key("rows")), [1]);
    assert_eq!(found(Query::new().action(ActionKind::Click)), [0]);
    assert_eq!(
        found(Query::new().action_name("toggle").name_contains("dog")),
        [1, 1]
    );
    assert_eq!(ui.find_all(&Query::new().role(Role::ListItem)).len(), 2);
}

#[test]
fn a_query_with_no_match_suggests_similar_nodes() {
    let ui = Headless::new(app, ());
    let error = ui
        .find(&Query::new().role(Role::Button).name("Save"))
        .unwrap_err();
    assert!(matches!(error, FindError::NotFound { .. }));
    assert_eq!(
        error.to_string(),
        "no node matches role button, name \"Save\"; similar nodes:\n  button \"Clear all\" #clear [click] @0"
    );

    let error = ui.find(&Query::new().name("Nothing")).unwrap_err();
    assert!(error.to_string().contains("Clear all"), "{error}");
    assert!(error.to_string().contains("Buy milk"), "{error}");
}

#[test]
fn a_query_with_many_matches_lists_them_instead_of_guessing() {
    let ui = Headless::new(app, ());
    let error = ui.find(&Query::new().role(Role::ListItem)).unwrap_err();
    assert!(matches!(error, FindError::Ambiguous { count: 2, .. }));
    assert!(error.to_string().contains("@1/1"), "{error}");
}

#[test]
fn acting_changes_the_tree_and_returns_what_changed() {
    let mut ui = Headless::new(app, ());
    let before = ui.root().clone();

    let patches = ui.toggle(&Query::new().name("Walk dog")).unwrap();

    assert!(matches!(patches.as_slice(), [Patch::SetName { .. }]));
    assert_eq!(apply(before, &patches).unwrap(), *ui.root());
    assert!(ui.find(&Query::new().name("Walk dog (done)")).is_ok());
}

#[test]
fn acting_on_a_bad_target_is_an_error() {
    let mut ui = Headless::new(app, ());
    assert!(matches!(
        ui.click(&Query::new().name("Nothing")),
        Err(ActError::Find(_))
    ));
    assert_eq!(
        ui.click(&Query::new().name("Buy milk")),
        Err(ActError::Dispatch(DispatchError::ActionNotAvailable))
    );
}

#[test]
fn paths_can_be_written_and_read_back() {
    for path in [vec![], vec![0], vec![1, 0, 12]] {
        let text = format_path(&path);
        assert_eq!(parse_path(&text), Some(path.clone()));
        assert_eq!(parse_path(&format!("@{text}")), Some(path));
    }
    assert_eq!(format_path(&[]), "/");
    assert_eq!(format_path(&[1, 0]), "1/0");
    for bad in ["a", "1/", "/1", "1//2", "-1", "99999999999"] {
        assert_eq!(parse_path(bad), None, "{bad}");
    }
}

#[test]
fn every_kind_of_patch_can_be_described() {
    let mut ui = Headless::new(app, ());
    let patches = ui.toggle(&Query::new().name("Buy milk")).unwrap();
    assert_eq!(
        describe_patch(&patches[0]),
        "set_name @1/0 \"Buy milk (done)\""
    );
    let patches = ui.click(&Query::new().key("clear")).unwrap();
    let described: Vec<String> = patches.iter().map(describe_patch).collect();
    assert_eq!(described, ["remove @1/1", "remove @1/0"]);

    let node = Element::new(Role::Group)
        .key("g")
        .child(Element::new(Role::Text))
        .into_node();
    let parent = rsui_core::Path::from(&[2u32][..]);
    let check = |patch: Patch, expected: &str| assert_eq!(describe_patch(&patch), expected);
    check(
        Patch::Insert {
            parent: parent.clone(),
            index: 1,
            node: node.clone(),
        },
        "insert group #g @2/1 (+1 children)",
    );
    check(
        Patch::Replace {
            path: parent.clone(),
            node,
        },
        "replace group #g @2 (+1 children)",
    );
    check(
        Patch::Move {
            parent: parent.clone(),
            from: 0,
            to: 3,
        },
        "move @2/0 to @2/3",
    );
    check(
        Patch::Reorder {
            parent: parent.clone(),
            start: 1,
            order: vec![1, 0].into(),
        },
        "reorder @2 2 children from 1",
    );
    check(
        Patch::SetProp {
            path: parent.clone(),
            name: "width".into(),
            value: PropValue::Float(1.5),
        },
        "set_prop @2 width=1.5",
    );
    check(
        Patch::RemoveProp {
            path: parent.clone(),
            name: "width".into(),
        },
        "remove_prop @2 width",
    );
    let mut actions = rsui_core::Actions::new();
    actions.insert(ActionKind::Click);
    actions.insert(ActionKind::Focus);
    check(
        Patch::SetActions {
            path: parent,
            actions,
        },
        "set_actions @2 [click,focus]",
    );
}

fn row(_: &mut Cx, id: &usize) -> Element {
    Element::new(Role::ListItem)
        .name(text!("row {id}"))
        .action(ActionKind::Click)
}

fn long_list(cx: &mut Cx, len: &usize) -> Element {
    cx.virtual_list(
        "rows",
        ListWindow {
            len: *len,
            visible: 10,
            overscan: 1,
            reveal: None,
        },
        |cx, index| cx.keyed(index, row, index),
    )
}

#[test]
fn scrolling_finds_rows_that_are_not_built_yet() {
    let mut ui = Headless::new(long_list, 10_000);
    let list = Query::new().role(Role::List);
    let target = Query::new().name("row 7777");
    assert!(ui.find(&target).is_err(), "it isn't built yet");

    let found = ui.scroll_to_find(&list, &target).unwrap();

    assert_eq!(found.node.name, "row 7777");
    assert_eq!(
        ui.find(&target).unwrap().path,
        found.path,
        "the list stays where the row was found"
    );
    assert_eq!(found.node.key, Some(7777usize.into()));
}

#[test]
fn scrolling_finds_the_first_and_last_rows() {
    let mut ui = Headless::new(long_list, 1_000);
    let list = Query::new().role(Role::List);
    for name in ["row 999", "row 0"] {
        let found = ui.scroll_to_find(&list, &Query::new().name(name)).unwrap();
        assert_eq!(found.node.name, name);
    }
}

#[test]
fn scrolling_for_a_row_that_does_not_exist_ends_with_an_error() {
    let mut ui = Headless::new(long_list, 500);
    let result = ui.scroll_to_find(
        &Query::new().role(Role::List),
        &Query::new().name("row 500"),
    );
    assert!(matches!(
        result,
        Err(ActError::Find(FindError::NotFound { .. }))
    ));
}

#[test]
fn settling_waits_for_background_work() {
    fn loader(cx: &mut Cx, _: &()) -> Element {
        let status = cx.state(|| "loading");
        cx.effect((), move |cx| {
            cx.after(Duration::from_millis(5), move |cx| cx.set(status, "ready"));
        });
        Element::new(Role::Text).name(cx.get(status))
    }
    let mut ui = Headless::new(loader, ());
    assert!(ui.find(&Query::new().name("loading")).is_ok());
    let (_, settled) = ui.settle(Duration::from_secs(10));
    assert!(settled);
    assert!(ui.find(&Query::new().name("ready")).is_ok());
}

#[test]
fn an_outline_is_much_smaller_than_a_json_snapshot() {
    let ui = Headless::new(app, ());
    let (outline, json) = (ui.outline(), snapshot_json(ui.root()));
    assert!(
        outline.len() * 10 < json.len() * 9,
        "outline {} bytes, JSON {} bytes",
        outline.len(),
        json.len()
    );
}

#[test]
fn what_says_nothing_is_left_out_of_an_outline() {
    fn app(_: &mut Cx, _: &()) -> Element {
        Element::new(Role::Window)
            .child(
                Element::new(Role::Group)
                    .key(rsui_core::GENERATED_KEYS_FROM + 1)
                    .name("Made up"),
            )
            .child(
                Element::new(Role::Group)
                    .prop("overlay", "fill")
                    .prop("inert", true),
            )
            .child(Element::new(Role::Button).key("mine").name("Mine"))
            .child(Element::new(Role::Button).key(7u64).name("Seven"))
    }
    let text = Headless::new(app, ()).outline();
    assert_eq!(
        text,
        "window @/\n  group \"Made up\" @0\n  button \"Mine\" #mine @2\n  button \"Seven\" #7 @3\n"
    );
}
