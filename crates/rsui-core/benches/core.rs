//! Tracks the cost of the core operations on a tree of about 10,000 nodes. Add
//! `--features ui-alloc` to run them on `rsui-alloc`.

#[cfg(feature = "ui-alloc")]
#[global_allocator]
static ALLOCATOR: rsui_alloc::UiAlloc = rsui_alloc::UiAlloc;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use rsui_core::{
    ActionKind, Element, Key, Node, NodePool, Role, apply, diff, shared_props, snapshot_into,
    snapshot_json, symbol, text,
};
use std::hint::black_box;
use triomphe::Arc;

const SECTIONS: usize = 100;
const ITEMS: usize = 99;
const FLAT: u64 = 10_000;

fn item(section: usize, item: usize, label: &str) -> Node {
    Element::new(Role::Button)
        .key(item)
        .name(format!("{label} {section}.{item}"))
        .prop(symbol!("enabled"), true)
        .prop(symbol!("width"), 120.0)
        .action(ActionKind::Click)
        .into_node()
}

fn section(index: usize, changed: Option<usize>) -> Node {
    Element::new(Role::Group)
        .key(index)
        .children((0..ITEMS).map(|i| {
            let label = if changed == Some(i) {
                "changed"
            } else {
                "item"
            };
            item(index, i, label)
        }))
        .into_node()
}

/// 1 root, 100 sections and 9,900 items: 10,001 nodes.
fn tree(changed: Option<(usize, usize)>) -> Node {
    Element::new(Role::Window)
        .children(
            (0..SECTIONS)
                .map(|index| section(index, changed.filter(|(s, _)| *s == index).map(|(_, i)| i))),
        )
        .into_node()
}

/// Copies only the nodes between the root and one changed item, the way a
/// memoizing runtime would.
fn tree_sharing_with(base: &Node, s: usize, i: usize) -> Node {
    let mut sect = (*base.children[s]).clone();
    sect.children[i] = item(s, i, "changed");
    let mut root = (**base).clone();
    root.children[s] = Arc::new(sect);
    Arc::new(root)
}

fn flat(keys: impl IntoIterator<Item = u64>) -> Node {
    Element::new(Role::List)
        .children(
            keys.into_iter()
                .map(|key| Element::new(Role::ListItem).key(key)),
        )
        .into_node()
}

/// A fixed permutation, so runs stay comparable: 7919 is coprime with `FLAT`.
fn shuffled() -> Vec<u64> {
    (0..FLAT).map(|i| i * 7919 % FLAT).collect()
}

fn benches(c: &mut Criterion) {
    let base = tree(None);
    let rebuilt = tree(None);
    let one_change = tree(Some((50, 50)));
    let one_change_shared = tree_sharing_with(&base, 50, 50);

    c.bench_function("build_10k_tree", |b| b.iter(|| tree(black_box(None))));

    c.bench_function("build_10k_inline_names", |b| {
        b.iter(|| {
            Element::new(Role::Window)
                .children((0..SECTIONS).map(|s| {
                    Element::new(Role::Group)
                        .key(s)
                        .children((0..ITEMS).map(move |i| {
                            Element::new(Role::Button)
                                .key(i)
                                .name(text!("item {s}.{i}"))
                                .prop(symbol!("enabled"), true)
                                .prop(symbol!("width"), 120.0)
                                .action(ActionKind::Click)
                        }))
                }))
                .into_node()
        })
    });

    let all_changed = Element::new(Role::Window)
        .children((0..SECTIONS).map(|s| {
            Element::new(Role::Group)
                .key(s)
                .children((0..ITEMS).map(|i| item(s, i, "changed")))
        }))
        .into_node();

    let mut group = c.benchmark_group("diff_10k");
    group.bench_function("all_names_changed", |b| {
        b.iter(|| diff(black_box(&base), black_box(&all_changed)))
    });
    group.bench_function("unchanged_same_tree", |b| {
        b.iter(|| diff(black_box(&base), black_box(&base)))
    });
    group.bench_function("unchanged_rebuilt", |b| {
        b.iter(|| diff(black_box(&base), black_box(&rebuilt)))
    });
    group.bench_function("one_leaf_rebuilt", |b| {
        b.iter(|| diff(black_box(&base), black_box(&one_change)))
    });
    group.bench_function("one_leaf_shared", |b| {
        b.iter(|| diff(black_box(&base), black_box(&one_change_shared)))
    });
    group.finish();

    let sorted = flat(0..FLAT);
    let reversed = flat((0..FLAT).rev());
    let shuffled = flat(shuffled());
    let prepended = flat(std::iter::once(FLAT).chain(0..FLAT));
    let mut swap_keys: Vec<u64> = (0..FLAT).collect();
    swap_keys.swap(100, 9_900);
    let swapped = flat(swap_keys);

    let mut group = c.benchmark_group("diff_keyed_list_10k");
    group.bench_function("reverse", |b| {
        b.iter(|| diff(black_box(&sorted), black_box(&reversed)))
    });
    group.bench_function("shuffle", |b| {
        b.iter(|| diff(black_box(&sorted), black_box(&shuffled)))
    });
    group.bench_function("swap_two", |b| {
        b.iter(|| diff(black_box(&sorted), black_box(&swapped)))
    });
    group.bench_function("prepend_one", |b| {
        b.iter(|| diff(black_box(&sorted), black_box(&prepended)))
    });
    group.finish();

    let patches = diff(&sorted, &shuffled);
    c.bench_function("apply_shuffle_10k", |b| {
        b.iter_batched(
            || (sorted.clone(), patches.as_slice()),
            |(tree, patches)| apply(tree, black_box(patches)),
            BatchSize::SmallInput,
        )
    });

    let swap_patches = diff(&sorted, &swapped);
    c.bench_function("apply_swap_two_10k", |b| {
        b.iter_batched(
            || (sorted.clone(), swap_patches.as_slice()),
            |(tree, patches)| apply(tree, black_box(patches)),
            BatchSize::SmallInput,
        )
    });

    // Whether 64 is the right limit for individual moves: these two sit on
    // either side of it, so their costs should be close.
    for (name, swaps) in [
        ("apply_64_moves_10k", 32),
        ("apply_reorder_66_moves_10k", 33),
    ] {
        let mut keys: Vec<u64> = (0..FLAT).collect();
        for swap in 0..swaps {
            keys.swap(swap * 100, FLAT as usize - 1 - swap * 100);
        }
        let target = flat(keys);
        let patches = diff(&sorted, &target);
        c.bench_function(name, |b| {
            b.iter_batched(
                || (sorted.clone(), patches.as_slice()),
                |(tree, patches)| apply(tree, black_box(patches)),
                BatchSize::SmallInput,
            )
        });
    }

    c.bench_function("snapshot_json_10k", |b| {
        b.iter(|| snapshot_json(black_box(&base)))
    });

    c.bench_function("snapshot_into_reused_buffer_10k", |b| {
        let mut out = String::with_capacity(2_000_000);
        b.iter(|| {
            out.clear();
            snapshot_into(black_box(&base), &mut out);
        })
    });

    fn tree_with_shared_props() -> Node {
        Element::new(Role::Window)
            .children((0..SECTIONS).map(|s| {
                Element::new(Role::Group)
                    .key(s)
                    .children((0..ITEMS).map(move |i| {
                        Element::new(Role::Button)
                            .key(i)
                            .name(text!("item {s}.{i}"))
                            .props(shared_props!("enabled" => true, "width" => 120.0))
                            .action(ActionKind::Click)
                    }))
            }))
            .into_node()
    }
    c.bench_function("build_10k_shared_props", |b| b.iter(tree_with_shared_props));
    let (shared_a, shared_b) = (tree_with_shared_props(), tree_with_shared_props());
    c.bench_function("diff_10k/unchanged_rebuilt_shared_props", |b| {
        b.iter(|| diff(black_box(&shared_a), black_box(&shared_b)))
    });

    c.bench_function("rebuild_10k_with_pool", |b| {
        let mut pool = NodePool::new();
        let first = pooled_tree(&mut pool);
        pool.recycle(first);
        b.iter(|| {
            let tree = pooled_tree(&mut pool);
            pool.recycle(black_box(tree));
        })
    });
}

fn pooled_tree(pool: &mut NodePool) -> Node {
    let sections: Vec<Node> = (0..SECTIONS)
        .map(|s| {
            let items: Vec<Node> = (0..ITEMS)
                .map(|i| {
                    pool.node(Role::Button, |element| {
                        element.key = Some(Key::from(i));
                        element.name = text!("item {s}.{i}");
                        element.props = shared_props!("enabled" => true, "width" => 120.0);
                        element.actions.insert(ActionKind::Click);
                    })
                })
                .collect();
            pool.node(Role::Group, |element| {
                element.key = Some(Key::from(s));
                element.children = items.into();
            })
        })
        .collect();
    pool.node(Role::Window, |element| {
        element.children = sections.into();
    })
}
criterion_group!(core, benches);
criterion_main!(core);
