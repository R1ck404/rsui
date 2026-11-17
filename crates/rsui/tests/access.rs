//! What assistive technology sees and can do, apart from how Windows asks.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use rsui::access::{AccessCommand, AccessNode, AccessTree};
use rsui::prelude::*;
use rsui::text::TextEngine;
use rsui::{App, KeyCode, KeyPress};
use support::plain_font;

const SIZE: (u32, u32) = (300, 200);

fn form(cx: &mut Cx, _: &()) -> Element {
    let count = cx.state(|| 0i64);
    let checked = cx.state(|| false);
    let value = cx.state(|| Text::from("start"));
    let open = cx.state(|| false);
    let add = cx.on(
        Element::new(Role::Button)
            .key("add")
            .name(text!("Count {}", cx.get(count)))
            .props(rsui::core::Props::from(
                Style::new().width(100.0).height(20.0),
            ))
            .prop("tooltip", "Adds one"),
        ActionKind::Click,
        move |cx, _| cx.update(count, |count| *count += 1),
    );
    let check = cx.on(
        Element::new(Role::Checkbox)
            .key("check")
            .name("Agree")
            .props(style!(Style::new().width(100.0).height(20.0)))
            .prop("checked", cx.get(checked)),
        ActionKind::Toggle,
        move |cx, _| cx.update(checked, |checked| *checked = !*checked),
    );
    let input = text_input(cx, "typed", "Something", "", value);
    let items: Vec<(Text, MenuAction)> = vec![(Text::from("Only"), Box::new(|_| {}))];
    let menu = menu_button(cx, "menu", "Menu", open, items);
    Element::new(Role::Window)
        .key("root")
        .props(style!(Style::new().align(Align::Start)))
        .child(
            // Only arranges what's in it.
            Element::new(Role::Group)
                .key("arranging")
                .child(add)
                .child(check),
        )
        .child(input)
        .child(menu)
}

fn app() -> App {
    let mut app = App::new(form, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.update();
    app.scene();
    app
}

fn node<'a>(tree: &'a AccessTree, key: &str) -> &'a AccessNode {
    tree.by_key(key)
        .unwrap_or_else(|| panic!("{key} is there: {tree:#?}"))
}

#[test]
fn it_says_what_each_thing_is() {
    let tree = app().access_tree();
    let add = node(&tree, "add");
    assert_eq!(add.role, Role::Button);
    assert_eq!(add.name, "Count 0");
    assert_eq!(add.help.as_deref(), Some("Adds one"));
    assert!(add.offers(ActionKind::Click));
    assert!(add.focusable());
    assert!(add.meaningful);
    let check = node(&tree, "check");
    assert_eq!(check.checked, Some(false));
    let input = node(&tree, "typed");
    assert_eq!(input.role, Role::TextInput);
    assert_eq!(input.value.as_deref(), Some("start"));
    assert!(input.focusable());
    // Something that only arranges things isn't worth mentioning.
    assert!(!node(&tree, "arranging").meaningful);
}

#[test]
fn it_says_where_each_thing_is_in_physical_pixels() {
    let tree = app().access_tree();
    assert_eq!(node(&tree, "add").rect, [0.0, 0.0, 100.0, 20.0]);
    assert_eq!(node(&tree, "check").rect, [0.0, 20.0, 100.0, 20.0]);
    let mut scaled = App::new(form, (), TextEngine::new(plain_font()), SIZE, 2.0);
    let tree = scaled.access_tree();
    assert_eq!(node(&tree, "check").rect, [0.0, 40.0, 200.0, 40.0]);
}

#[test]
fn parents_and_children_agree() {
    let tree = app().access_tree();
    assert_eq!(tree.nodes[0].parent, None);
    for (index, node) in tree.nodes.iter().enumerate() {
        for &child in &node.children {
            assert_eq!(tree.nodes[child as usize].parent, Some(index as u32));
        }
        // Each can be found again by what it's known by.
        assert_eq!(
            tree.find(node.id).map(|found| &found.path),
            Some(&node.path)
        );
    }
    let arranging = node(&tree, "arranging");
    assert_eq!(arranging.children.len(), 2);
    assert_eq!(
        tree.nodes[arranging.children[1] as usize].key.as_deref(),
        Some("check")
    );
}

#[test]
fn something_keeps_its_id_as_things_change_around_it() {
    let mut app = app();
    let before = app.access_tree();
    let id = node(&before, "check").id;
    app.access_command(&node(&before, "add").path, AccessCommand::Invoke)
        .unwrap();
    let after = app.access_tree();
    assert_eq!(node(&after, "check").id, id);
    assert_eq!(node(&after, "add").name, "Count 1");
    // Everything has its own.
    let mut ids: Vec<u64> = after.nodes.iter().map(|node| node.id).collect();
    let count = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), count);
}

#[test]
fn it_can_click_toggle_type_and_move_the_keyboard() {
    let mut app = app();
    let tree = app.access_tree();
    let (add, check, typed) = (
        node(&tree, "add").path.clone(),
        node(&tree, "check").path.clone(),
        node(&tree, "typed").path.clone(),
    );
    app.access_command(&add, AccessCommand::Invoke).unwrap();
    app.access_command(&check, AccessCommand::Toggle).unwrap();
    app.access_command(&typed, AccessCommand::SetValue("hello".into()))
        .unwrap();
    let tree = app.access_tree();
    assert_eq!(node(&tree, "add").name, "Count 1");
    assert_eq!(node(&tree, "check").checked, Some(true));
    assert_eq!(node(&tree, "typed").value.as_deref(), Some("hello"));

    assert_eq!(tree.focused, None);
    app.access_command(&check, AccessCommand::Focus).unwrap();
    let tree = app.access_tree();
    let focused = &tree.nodes[tree.focused.expect("it has the keyboard") as usize];
    assert_eq!(focused.key.as_deref(), Some("check"));
}

#[test]
fn something_that_has_gone_is_quietly_left_alone() {
    let mut app = app();
    assert!(
        app.access_command(&[9, 9, 9], AccessCommand::Invoke)
            .is_ok(),
        "a request can be overtaken by what it was about going away"
    );
}

#[test]
fn it_follows_the_keyboard() {
    let mut app = app();
    app.key(KeyPress::new(KeyCode::Tab)).unwrap();
    let tree = app.access_tree();
    let focused = &tree.nodes[tree.focused.expect("it moved") as usize];
    assert_eq!(focused.key.as_deref(), Some("add"));
}

#[test]
fn it_finds_what_is_at_a_point() {
    let tree = app().access_tree();
    assert_eq!(
        tree.at_point(5.0, 25.0)
            .and_then(|node| node.key.as_deref()),
        Some("check")
    );
    assert_eq!(
        tree.at_point(5.0, 5.0).and_then(|node| node.key.as_deref()),
        Some("add")
    );
    // Nothing else here, so it's the window.
    assert_eq!(
        tree.at_point(290.0, 190.0).map(|node| node.parent),
        Some(None)
    );
    assert!(tree.at_point(-5.0, 5.0).is_none());
}

#[test]
fn what_is_on_top_comes_first_at_a_point_even_outside_what_it_is_in() {
    let mut app = app();
    let tree = app.access_tree();
    app.access_command(&node(&tree, "menu-button").path, AccessCommand::Invoke)
        .unwrap();
    let tree = app.access_tree();
    let menu = node(&tree, "menu-menu");
    assert!(menu.overlay);
    let holder = node(&tree, "menu");
    // It's outside its holder, and what's underneath doesn't get a look in.
    assert!(menu.rect[1] >= holder.rect[1] + holder.rect[3]);
    let inside = (menu.rect[0] + 6.0, menu.rect[1] + 6.0);
    let found = tree
        .at_point(inside.0, inside.1)
        .expect("something's there");
    assert!(
        found.path.starts_with(&menu.path),
        "{:?} isn't in the menu",
        found.key
    );
}

/// A scrolling area with a row in view and one out of view, and a list.
fn scrolling(cx: &mut Cx, _: &()) -> Element {
    let block = |key: &str| {
        Element::new(Role::Button)
            .key(key)
            .name(key)
            .props(style!(Style::new().height(150.0)))
    };
    let content = Element::new(Role::Group)
        .props(style!(Style::new().height(100.0)))
        .child(block("seen"))
        .child(block("unseen"));
    let area = cx.scroll_area("area", ScrollAxis::Vertical, content);
    let list = cx.virtual_list(
        "rows",
        ListWindow {
            len: 1000,
            visible: 4,
            overscan: 0,
            reveal: None,
        },
        |_, index| {
            Element::new(Role::ListItem)
                .name(text!("{index}"))
                .into_node()
        },
    );
    let props = rsui::core::Props::from(Style::new().row_height(25.0).height(100.0));
    let list = Element {
        props: props.merged(&list.props),
        ..list
    };
    Element::new(Role::Window).child(area).child(list)
}

#[test]
fn it_says_what_is_out_of_view_and_can_bring_it_in() {
    let mut app = App::new(scrolling, (), TextEngine::new(plain_font()), SIZE, 1.0);
    let tree = app.access_tree();
    assert!(!node(&tree, "seen").offscreen);
    assert!(node(&tree, "unseen").offscreen);
    let area = node(&tree, "area");
    let info = area.scroll.expect("it scrolls");
    assert_eq!(info.y, 0.0);
    assert_eq!(info.furthest_y(), 200.0);
    let unseen = node(&tree, "unseen").path.clone();
    app.access_command(&unseen, AccessCommand::Reveal).unwrap();
    let tree = app.access_tree();
    assert!(!node(&tree, "unseen").offscreen);
    assert!(node(&tree, "area").scroll.expect("it scrolls").y > 0.0);
}

#[test]
fn it_can_scroll_areas_and_lists() {
    let mut app = App::new(scrolling, (), TextEngine::new(plain_font()), SIZE, 1.0);
    let tree = app.access_tree();
    let (area, list) = (
        node(&tree, "area").path.clone(),
        node(&tree, "rows").path.clone(),
    );
    let moved = |app: &mut App, key: &str| {
        let tree = app.access_tree();
        node(&tree, key).scroll.expect("it scrolls").y
    };
    app.access_command(&area, AccessCommand::ScrollTo(0.5))
        .unwrap();
    assert_eq!(moved(&mut app, "area"), 100.0);
    app.access_command(&area, AccessCommand::ScrollSteps(1))
        .unwrap();
    assert_eq!(moved(&mut app, "area"), 140.0);
    app.access_command(&area, AccessCommand::ScrollPages(-1))
        .unwrap();
    assert_eq!(moved(&mut app, "area"), 80.0);
    app.access_command(&list, AccessCommand::ScrollTo(1.0))
        .unwrap();
    assert!(moved(&mut app, "rows") > 20_000.0);
    app.access_command(&list, AccessCommand::ScrollSteps(-1))
        .unwrap();
    let last = moved(&mut app, "rows");
    assert!(last > 20_000.0 && last.rem_euclid(25.0) == 0.0, "{last}");
}

#[test]
fn comparing_trees_notices_what_came_and_went_but_not_new_names() {
    let mut app = app();
    let first = app.access_tree();
    assert!(first.same_structure(&app.access_tree()));
    app.access_command(&node(&first, "add").path, AccessCommand::Invoke)
        .unwrap();
    let renamed = app.access_tree();
    assert!(first.same_structure(&renamed), "only a name changed");
    app.access_command(&node(&first, "menu-button").path, AccessCommand::Invoke)
        .unwrap();
    let opened = app.access_tree();
    assert!(!first.same_structure(&opened), "something came");
}

/// Two keyed things that can swap, and two unkeyed groups, each holding an unkeyed child.
fn swapping(cx: &mut Cx, _: &()) -> Element {
    let swapped = cx.state(|| false);
    let thing = |key: &'static str| Element::new(Role::Button).key(key).name(key).into_node();
    let (first, second) = if cx.get(swapped) {
        ("b", "a")
    } else {
        ("a", "b")
    };
    let inner = || {
        Element::new(Role::Group)
            .child(Element::new(Role::Text).name("inner"))
            .into_node()
    };
    let root = Element::new(Role::Window)
        .key("root")
        .child(thing(first))
        .child(thing(second))
        .child(inner())
        .child(inner());
    cx.on(root, ActionKind::SetValue, move |cx, _| {
        cx.set(swapped, true)
    })
}

#[test]
fn a_keyed_thing_keeps_its_id_wherever_it_moves_to() {
    let mut app = App::new(swapping, (), TextEngine::new(plain_font()), SIZE, 1.0);
    let before = app.access_tree();
    let (a, b) = (node(&before, "a").id, node(&before, "b").id);
    assert_ne!(a, b);
    assert_eq!(node(&before, "a").path, [0]);
    app.runtime_mut()
        .dispatch(&[], ActionKind::SetValue, None)
        .unwrap();
    let after = app.access_tree();
    assert_eq!(node(&after, "a").path, [1], "it moved");
    assert_eq!(node(&after, "a").id, a);
    assert_eq!(node(&after, "b").id, b);
    // Being in the same places with different things in them is a different structure.
    assert!(!before.same_structure(&after));
}

#[test]
fn things_with_no_key_differ_by_where_they_are() {
    let mut app = App::new(swapping, (), TextEngine::new(plain_font()), SIZE, 1.0);
    let tree = app.access_tree();
    let inner: Vec<u64> = tree
        .nodes
        .iter()
        .filter(|node| node.name == "inner")
        .map(|node| node.id)
        .collect();
    assert_eq!(inner.len(), 2);
    assert_ne!(
        inner[0], inner[1],
        "the same position, in different parents"
    );
}
