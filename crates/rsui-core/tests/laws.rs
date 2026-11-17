//! The guarantees the rest of rsui builds on.

use proptest::collection::{btree_map, vec};
use proptest::prelude::*;
use rsui_core::{
    ActionKind, Actions, Element, Key, Node, NodePool, Patch, Path, PropValue, Props, Role, Symbol,
    apply, diff, diff_at, shared_props, snapshot, snapshot_into, snapshot_json,
};
use triomphe::Arc;

fn key() -> impl Strategy<Value = Option<Key>> {
    // A small range, so keys collide across trees and repeat within a list.
    prop_oneof![
        2 => Just(None),
        3 => (0u64..6).prop_map(|key| Some(Key::Int(key))),
        1 => prop::sample::select(vec!["a", "b", "c"]).prop_map(|key| Some(key.into())),
        1 => prop::sample::select(vec!["x", "y"]).prop_map(|key| Some(Symbol::new(key).into())),
    ]
}

fn role() -> impl Strategy<Value = Role> {
    prop_oneof![
        4 => Just(Role::Group),
        1 => Just(Role::Button),
        1 => Just(Role::Custom("chart".into())),
    ]
}

fn prop_value() -> impl Strategy<Value = PropValue> {
    prop_oneof![
        any::<bool>().prop_map(PropValue::Bool),
        (-3i64..3).prop_map(PropValue::Int),
        prop_oneof![Just(0.5), Just(-0.0), Just(0.0), Just(f64::NAN)].prop_map(PropValue::Float),
        "[ab]{0,2}".prop_map(PropValue::from),
    ]
}

fn element() -> impl Strategy<Value = Element> {
    let leaf = (
        key(),
        role(),
        prop::option::of("[xy]"),
        btree_map("[pqr]", prop_value(), 0..3),
        vec(
            prop_oneof![Just(ActionKind::Click), Just(ActionKind::Focus)],
            0..2,
        ),
    )
        .prop_map(|(key, role, name, props, actions)| Element {
            key,
            role,
            name: name.map(Into::into).unwrap_or_default(),
            props: props.into_iter().collect(),
            actions: actions.into_iter().fold(Actions::new(), |mut set, action| {
                set.insert(action);
                set
            }),
            children: Default::default(),
        });
    leaf.prop_recursive(4, 80, 10, |inner| {
        (inner.clone(), vec(inner, 0..10)).prop_map(|(parent, children)| Element {
            children: children.into_iter().map(Arc::new).collect(),
            ..parent
        })
    })
}

/// Rebuilds a tree without sharing any allocation with the original.
fn deep_clone(node: &Node) -> Node {
    Arc::new(Element {
        children: node.children.iter().map(deep_clone).collect(),
        ..(**node).clone()
    })
}

fn longest_increasing_length(values: &[u64]) -> usize {
    let mut best = vec![1; values.len()];
    for i in 0..values.len() {
        for j in 0..i {
            if values[j] < values[i] {
                best[i] = best[i].max(best[j] + 1);
            }
        }
    }
    best.into_iter().max().unwrap_or(0)
}

fn keyed_list(keys: &[u64]) -> Node {
    Element::new(Role::List)
        .children(
            keys.iter()
                .map(|&key| Element::new(Role::ListItem).key(key)),
        )
        .into_node()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn applying_a_diff_produces_the_new_tree(old in element(), new in element()) {
        let (old, new) = (Arc::new(old), Arc::new(new));
        let patches = diff(&old, &new);
        prop_assert_eq!(apply(old, &patches).unwrap(), new);
    }

    #[test]
    fn identical_trees_have_no_patches(tree in element()) {
        let tree = Arc::new(tree);
        prop_assert!(diff(&tree, &tree).is_empty());
        prop_assert!(diff(&tree, &deep_clone(&tree)).is_empty());
    }

    #[test]
    fn diffing_is_deterministic(old in element(), new in element()) {
        let (old, new) = (Arc::new(old), Arc::new(new));
        let first = serde_json::to_string(&diff(&old, &new)).unwrap();
        let second = serde_json::to_string(&diff(&deep_clone(&old), &deep_clone(&new))).unwrap();
        prop_assert_eq!(first, second);
    }

    #[test]
    fn equal_trees_have_identical_snapshots(tree in element()) {
        let tree = Arc::new(tree);
        let copy = deep_clone(&tree);
        prop_assert_eq!(snapshot_json(&tree), snapshot_json(&copy));
        prop_assert_eq!(snapshot(&tree), snapshot(&copy));
    }

    #[test]
    fn reordering_only_moves_and_moves_as_little_as_possible(
        order in Just((0..40u64).collect::<Vec<_>>()).prop_shuffle(),
    ) {
        let sorted: Vec<u64> = (0..40).collect();
        let (old, new) = (keyed_list(&sorted), keyed_list(&order));
        let patches = diff(&old, &new);

        let only_moves = patches.iter().all(|patch| matches!(patch, Patch::Move { .. }));
        prop_assert!(only_moves);
        prop_assert_eq!(patches.len(), order.len() - longest_increasing_length(&order));
        prop_assert_eq!(apply(old, &patches).unwrap(), new);
    }

    #[test]
    fn reordering_with_changes_still_applies(
        order in Just((0..30u64).collect::<Vec<_>>()).prop_shuffle(),
        drop_first in 0usize..10,
        inserted in vec(100u64..110, 0..5),
    ) {
        let sorted: Vec<u64> = (0..30).collect();
        let mut keys = order[drop_first..].to_vec();
        for (offset, key) in inserted.into_iter().enumerate() {
            keys.insert(offset * 3, key);
        }
        let (old, new) = (keyed_list(&sorted), keyed_list(&keys));
        let patches = diff(&old, &new);
        prop_assert_eq!(apply(old, &patches).unwrap(), new);
    }

    #[test]
    fn large_reorders_use_one_patch(
        order in Just((0..400u64).collect::<Vec<_>>()).prop_shuffle(),
        drop_first in 0usize..50,
        inserted in vec(1000u64..1100, 0..20),
    ) {
        let sorted: Vec<u64> = (0..400).collect();
        let mut keys = order[drop_first..].to_vec();
        for (offset, key) in inserted.into_iter().enumerate() {
            keys.insert(offset * 7, key);
        }
        let (old, new) = (keyed_list(&sorted), keyed_list(&keys));
        let patches = diff(&old, &new);

        let reorders = patches.iter().filter(|p| matches!(p, Patch::Reorder { .. })).count();
        let moves = patches.iter().filter(|p| matches!(p, Patch::Move { .. })).count();
        prop_assert!(moves <= 64);
        prop_assert!(reorders <= 1);
        prop_assert!(reorders == 1 || moves > 0);
        prop_assert_eq!(apply(old, &patches).unwrap(), new);
    }

    #[test]
    fn large_reorders_inside_a_list_keep_its_ends(
        order in Just((0..200u64).collect::<Vec<_>>()).prop_shuffle(),
    ) {
        let all = |middle: &[u64]| -> Vec<u64> {
            (1000..1003).chain(middle.iter().copied()).chain(2000..2003).collect()
        };
        let sorted: Vec<u64> = (0..200).collect();
        let (old, new) = (keyed_list(&all(&sorted)), keyed_list(&all(&order)));
        let patches = diff(&old, &new);
        prop_assert_eq!(apply(old, &patches).unwrap(), new);
    }
    #[test]
    fn handwritten_snapshots_match_serde(tree in element(), name in ".{0,30}", number in any::<f64>()) {
        let tree = tree.name(name.as_str()).prop(name.as_str(), number).prop("n", name.as_str());
        prop_assert_eq!(snapshot_json(&tree), serde_json::to_string(&tree).unwrap());
    }

    #[test]
    fn pooled_nodes_are_indistinguishable_from_new_ones(first in element(), second in element()) {
        fn rebuild(pool: &mut NodePool, node: &Node) -> Node {
            let children: Vec<Node> = node.children.iter().map(|c| rebuild(pool, c)).collect();
            pool.node(node.role, |element| {
                element.key = node.key.clone();
                element.name = node.name.clone();
                for (name, value) in node.props.iter() {
                    element.props.insert(name, value.clone());
                }
                for action in node.actions.iter() {
                    element.actions.insert(action);
                }
                element.children.extend(children);
            })
        }
        let (first, second) = (Arc::new(first), Arc::new(second));
        let mut pool = NodePool::new();
        pool.recycle(deep_clone(&first));
        prop_assert_eq!(rebuild(&mut pool, &second), second.clone());
        prop_assert_eq!(snapshot_json(&rebuild(&mut pool, &first)), snapshot_json(&first));
    }

    #[test]
    fn props_behave_like_a_sorted_map(
        operations in vec((any::<bool>(), "[pqrst]", -3i64..3), 0..30),
    ) {
        let mut props = Props::new();
        let mut model = std::collections::BTreeMap::new();
        for (insert, name, value) in operations {
            if insert {
                props.insert(name.as_str(), value);
                model.insert(name.clone(), value);
            } else {
                prop_assert_eq!(
                    props.remove(&name),
                    model.remove(&name).map(PropValue::Int)
                );
            }
            prop_assert_eq!(props.len(), model.len());
            prop_assert_eq!(props.is_empty(), model.is_empty());
            let expected = model.get(&name).map(|value| PropValue::Int(*value));
            prop_assert_eq!(props.get(&name), expected.as_ref());
            let listed: Vec<(&str, PropValue)> =
                props.iter().map(|(name, value)| (name.as_str(), value.clone())).collect();
            let expected: Vec<(&str, PropValue)> =
                model.iter().map(|(name, value)| (name.as_str(), PropValue::Int(*value))).collect();
            prop_assert_eq!(listed, expected);
        }
    }
}

#[test]
fn building_props_from_a_list_lets_the_last_value_win() {
    let props = Props::from([("b", 1i64), ("a", 2), ("b", 3)]);
    assert_eq!(props.len(), 2);
    assert_eq!(props.get("a"), Some(&PropValue::Int(2)));
    assert_eq!(props.get("b"), Some(&PropValue::Int(3)));
    assert_eq!(Props::from([] as [(&str, i64); 0]), Props::new());
}

#[test]
fn inserting_into_a_shared_set_leaves_the_other_holders_alone() {
    let shared = Props::from([("a", 1i64)]);
    let mut copy = shared.clone();
    copy.insert("a", 2i64);
    copy.insert("b", 3i64);
    assert_eq!(shared, Props::from([("a", 1i64)]));
    assert_eq!(copy, Props::from([("a", 2i64), ("b", 3)]));
}

#[test]
fn elements_can_share_a_set_of_props() {
    let button = || {
        Element::new(Role::Button)
            .props(shared_props!("enabled" => true, "width" => 120.0))
            .into_node()
    };
    assert!(diff(&button(), &button()).is_empty());
    assert_eq!(
        snapshot_json(&button()),
        r#"{"role":"button","props":{"enabled":true,"width":120.0}}"#
    );
}

#[test]
fn every_node_fits_in_a_cache_line() {
    assert_eq!(size_of::<Element>(), 56);
    assert_eq!(size_of::<Node>(), 8);
}

#[test]
fn snapshot_into_appends_to_the_buffer() {
    let tree = Element::new(Role::Button);
    let mut out = String::from("[");
    snapshot_into(&tree, &mut out);
    assert_eq!(out, r#"[{"role":"button"}"#);
}

#[test]
fn recycling_leaves_shared_nodes_alone() {
    let shared = Element::new(Role::Text).name("shared").into_node();
    let tree = Element::new(Role::Group).child(shared.clone()).into_node();
    let mut pool = NodePool::new();
    pool.recycle(tree);

    assert_eq!(pool.len(), 1);
    assert_eq!(shared.name, "shared");
    let reused = pool.node(Role::Button, |_| {});
    assert_eq!(reused, Element::new(Role::Button).into_node());
    assert!(pool.is_empty());
}

#[test]
fn apply_rejects_a_reorder_that_is_not_a_permutation() {
    let tree = keyed_list(&[1, 2, 3]);
    for order in [vec![0, 0, 1], vec![0, 1, 7]] {
        let patch = Patch::Reorder {
            parent: Path::from(&[][..]),
            start: 0,
            order: order.into(),
        };
        assert!(apply(tree.clone(), &[patch]).is_err());
    }
    let too_long = Patch::Reorder {
        parent: Path::from(&[][..]),
        start: 2,
        order: vec![0, 1].into(),
    };
    assert!(apply(tree, &[too_long]).is_err());
}
#[test]
fn handwritten_snapshots_match_serde_for_every_role_and_action() {
    let roles = [
        Role::Window,
        Role::Group,
        Role::Text,
        Role::Button,
        Role::Link,
        Role::TextInput,
        Role::Checkbox,
        Role::List,
        Role::ListItem,
        Role::Image,
        Role::Dialog,
        Role::Menu,
        Role::MenuItem,
        Role::Custom("a\"b".into()),
    ];
    let actions = [
        ActionKind::Click,
        ActionKind::Focus,
        ActionKind::Toggle,
        ActionKind::SetValue,
        ActionKind::Scroll,
        ActionKind::KeyPress,
        ActionKind::Resize,
        ActionKind::Dismiss,
        ActionKind::Custom("a\\b".into()),
    ];
    for role in roles {
        let tree = actions
            .iter()
            .cloned()
            .fold(Element::new(role), Element::action);
        assert_eq!(snapshot_json(&tree), serde_json::to_string(&tree).unwrap());
    }
}

#[test]
fn handwritten_snapshots_match_serde_for_special_characters() {
    for text in [
        "",
        "quote\"",
        "back\\slash",
        "new\nline",
        "\u{1}\u{1f}",
        "é\u{7f}€😀",
    ] {
        let tree = Element::new(Role::Text)
            .key(text)
            .name(text)
            .prop(text, text);
        assert_eq!(snapshot_json(&tree), serde_json::to_string(&tree).unwrap());
    }
}

#[test]
fn diff_at_gives_patches_for_the_larger_tree() {
    let leaf = |name: &str| Element::new(Role::Text).name(name).into_node();
    let group = |child: Node| Element::new(Role::Group).child(child).into_node();
    let root = |child: Node| Element::new(Role::Window).child(child).into_node();
    let (old, new) = (group(leaf("old")), group(leaf("new")));

    let mut patches = Vec::new();
    diff_at(&[0], &old, &new, &mut patches);

    assert_eq!(apply(root(old), &patches).unwrap(), root(new));
}

#[test]
fn snapshot_ignores_the_order_props_were_set_in() {
    let first = Element::new(Role::Group).prop("a", 1i64).prop("b", 2i64);
    let second = Element::new(Role::Group).prop("b", 2i64).prop("a", 1i64);
    assert_eq!(snapshot_json(&first), snapshot_json(&second));
    assert_eq!(
        snapshot_json(&first),
        r#"{"role":"group","props":{"a":1,"b":2}}"#
    );
}

#[test]
fn snapshot_leaves_out_empty_fields() {
    let tree = Element::new(Role::Custom("chart".into())).key(7u64);
    assert_eq!(
        snapshot(&tree),
        serde_json::json!({ "key": 7, "role": { "custom": "chart" } })
    );
}

#[test]
fn a_kept_child_is_updated_at_its_final_position() {
    let item = |key: u64, name: &str| Element::new(Role::ListItem).key(key).name(name);
    let old = Element::new(Role::List)
        .child(item(1, "one"))
        .child(item(2, "two"))
        .into_node();
    let new = Element::new(Role::List)
        .child(item(2, "TWO"))
        .child(item(1, "one"))
        .into_node();

    let patches = diff(&old, &new);
    assert_eq!(patches.len(), 2);
    assert!(matches!(patches[0], Patch::Move { .. }));
    assert_eq!(
        patches[1],
        Patch::SetName {
            path: Path::from(&[0u32][..]),
            name: "TWO".into(),
        }
    );
    assert_eq!(apply(old, &patches).unwrap(), new);
}

#[test]
fn unkeyed_children_match_by_position() {
    let text = |name: &str| Element::new(Role::Text).name(name);
    let old = Element::new(Role::Group).child(text("a")).into_node();
    let new = Element::new(Role::Group)
        .child(text("new"))
        .child(text("a"))
        .into_node();

    let patches = diff(&old, &new);
    assert!(matches!(
        patches.as_slice(),
        [Patch::Insert { index: 1, .. }, Patch::SetName { .. }]
    ));
    assert_eq!(apply(old, &patches).unwrap(), new);
}

#[test]
fn a_changed_role_or_key_replaces_the_node() {
    let old = Element::new(Role::Group).key(1u64).into_node();
    for new in [
        Element::new(Role::Button).key(1u64).into_node(),
        Element::new(Role::Group).key(2u64).into_node(),
    ] {
        let patches = diff(&old, &new);
        assert!(matches!(patches.as_slice(), [Patch::Replace { .. }]));
        assert_eq!(apply(old.clone(), &patches).unwrap(), new);
    }
}

#[test]
fn a_shared_subtree_produces_no_patches() {
    let shared = Element::new(Role::Group).name("shared").into_node();
    let old = Element::new(Role::Group).child(shared.clone()).into_node();
    let new = Element::new(Role::Group)
        .child(shared.clone())
        .name("changed")
        .into_node();
    assert!(matches!(
        diff(&old, &new).as_slice(),
        [Patch::SetName { .. }]
    ));
}

#[test]
fn nan_props_do_not_produce_patches() {
    let tree = || Element::new(Role::Group).prop("x", f64::NAN).into_node();
    assert!(diff(&tree(), &tree()).is_empty());
}

#[test]
fn apply_rejects_a_patch_for_another_tree() {
    let tree = Element::new(Role::Group).into_node();
    let patch = Patch::Remove {
        parent: Path::from(&[][..]),
        index: 3,
    };
    let error = apply(tree, &[patch]).unwrap_err();
    assert_eq!((error.patch, error.index), (0, 3));
}

#[test]
fn apply_keeps_other_holders_of_the_old_tree_intact() {
    let old = Element::new(Role::Group).name("old").into_node();
    let new = Element::new(Role::Group).name("new").into_node();
    let patched = apply(old.clone(), &diff(&old, &new)).unwrap();
    assert_eq!(old.name, "old");
    assert_eq!(patched, new);
}
