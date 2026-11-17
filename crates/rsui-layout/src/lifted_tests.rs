//! Private tests so discovery instrumentation never becomes a public API or a
//! release-library cost. The reference deliberately does not read the new bit.
use super::*;
use crate::{FixedMetrics, Placement};
use rsui_core::{Role, diff};
use std::cell::Cell;

thread_local! {
    static GATHER_VISITS: Cell<usize> = const { Cell::new(0) };
}

pub(super) fn note_gather_visit() {
    GATHER_VISITS.with(|count| count.set(count.get() + 1));
}

fn counted<T>(f: impl FnOnce() -> T) -> (T, usize) {
    GATHER_VISITS.with(|count| count.set(0));
    let result = f();
    (result, GATHER_VISITS.with(Cell::get))
}

// Pre-011g exhaustive gather and reverse hit algorithm. Keep this independent of
// contributes_absolute/update_contains_absolute, including its overlay exclusion.
fn exhaustive_lifted(layout: &Layout, x: f32, y: f32) -> (Option<Vec<u32>>, usize) {
    fn gather(
        current: &LayoutBox,
        path: &mut Vec<u32>,
        found: &mut Vec<Vec<u32>>,
        visits: &mut usize,
    ) {
        for (index, child) in current.children.iter().enumerate() {
            *visits += 1;
            if child.has(IS_OVERLAY) {
                continue;
            }
            path.push(index as u32);
            if child.has(IS_ABSOLUTE) {
                found.push(path.clone());
            }
            gather(child, path, found, visits);
            path.pop();
        }
    }
    let mut found = Vec::new();
    let mut visits = 0;
    gather(&layout.root, &mut Vec::new(), &mut found, &mut visits);
    'each: for path in found.iter().rev() {
        let mut current = &layout.root;
        let (mut left, mut top) = (0.0, 0.0);
        for &index in path {
            let (inner_x, inner_y) = (left + current.x, top + current.y);
            if current.has(SCROLLS)
                && !Rect::new(inner_x, inner_y, current.width, current.height).contains(x, y)
            {
                continue 'each;
            }
            (left, top) = (inner_x, inner_y);
            current = &current.children[index as usize];
        }
        let mut deeper = path.clone();
        if Layout::hit_box(current, x - left, y - top, &mut deeper) {
            return (Some(deeper), visits);
        }
    }
    (None, visits)
}

// The same, with the point taken through what each box on the way undoes, as a turned tree is hit.
fn exhaustive_lifted_turned(layout: &Layout, root: &Node, x: f32, y: f32) -> Option<Vec<u32>> {
    fn gather(current: &LayoutBox, path: &mut Vec<u32>, found: &mut Vec<Vec<u32>>) {
        for (index, child) in current.children.iter().enumerate() {
            if child.has(IS_OVERLAY) {
                continue;
            }
            path.push(index as u32);
            if child.has(IS_ABSOLUTE) {
                found.push(path.clone());
            }
            gather(child, path, found);
            path.pop();
        }
    }
    let mut found = Vec::new();
    gather(&layout.root, &mut Vec::new(), &mut found);
    'each: for path in found.iter().rev() {
        let (mut current, mut node) = (&layout.root, root);
        let (mut x, mut y) = (x, y);
        for &index in path {
            let rect = Rect::new(current.x, current.y, current.width, current.height);
            if let Some([a, b, c, d, e, f]) = undo(node, rect) {
                (x, y) = (a * x + c * y + e, b * x + d * y + f);
            }
            if current.has(SCROLLS) && !rect.contains(x, y) {
                continue 'each;
            }
            (x, y) = (x - current.x, y - current.y);
            current = &current.children[index as usize];
            node = &node.children[index as usize];
        }
        let mut deeper = path.clone();
        if Layout::hit_box_turned(current, node, x, y, &undo, &mut deeper) {
            return Some(deeper);
        }
    }
    None
}

fn reference(layout: &Layout, node: &Node, x: f32, y: f32, turned: bool) -> Option<Vec<u32>> {
    let mut path = Vec::new();
    let overlays = if turned {
        let root = &layout.root;
        let (x, y) = undone(undo(node, Rect::new(root.x, root.y, root.width, root.height)), x, y);
        Layout::hit_overlays_turned(root, node, x - root.x, y - root.y, &undo, &mut path)
    } else {
        Layout::hit_overlays(&layout.root, x - layout.root.x, y - layout.root.y, &mut path)
    };
    if layout.root.has(CONTAINS_OVERLAY) && overlays {
        return Some(path);
    }
    path.clear();
    let lifted = if turned {
        exhaustive_lifted_turned(layout, node, x, y)
    } else {
        exhaustive_lifted(layout, x, y).0
    };
    if let Some(found) = lifted {
        return Some(found);
    }
    let hit = if turned {
        Layout::hit_box_turned(&layout.root, node, x, y, &undo, &mut path)
    } else {
        Layout::hit_box(&layout.root, x, y, &mut path)
    };
    hit.then_some(path)
}

fn undo(node: &Element, _: Rect) -> Option<[f32; 6]> {
    node.props
        .get("test_turn")
        .map(|_| [0.0, -1.0, 1.0, 0.0, 0.0, 100.0])
}

// Recurse independently all the way through overlays too, checking each local
// summary, but only pass non-overlay descendants' contribution to its parent.
fn assert_summary(current: &LayoutBox) -> bool {
    let mut expected = false;
    for child in &current.children {
        let below = assert_summary(child);
        expected |= !child.has(IS_OVERLAY) && (child.has(IS_ABSOLUTE) || below);
    }
    assert_eq!(current.has(CONTAINS_ABSOLUTE), expected);
    expected
}

fn compare(layout: &Layout, root: &Node) {
    assert_summary(&layout.root);
    for y in [
        -10.0, 0.0, 5.0, 19.0, 20.0, 45.0, 75.0, 100.0, 150.0, 299.0, 310.0,
    ] {
        for x in [
            -10.0, 0.0, 5.0, 19.0, 20.0, 45.0, 75.0, 100.0, 150.0, 399.0, 410.0,
        ] {
            assert_eq!(
                layout.hit_lifted_turned_fused(root, x, y, &undo),
                layout.hit_lifted_turned_materialized(root, x, y, &undo),
                "fused lifted at {x},{y}"
            );
            assert_eq!(
                layout.hit_lifted(x, y),
                exhaustive_lifted(layout, x, y).0,
                "lifted at {x},{y}"
            );
            assert_eq!(
                layout.hit_test(x, y),
                reference(layout, root, x, y, false),
                "ordinary at {x},{y}"
            );
            assert_eq!(
                layout.hit_test_turned(root, x, y, &undo),
                reference(layout, root, x, y, true),
                "turned at {x},{y}"
            );
        }
    }
}

const WINDOW: Size = Size::new(400.0, 300.0);

fn group() -> Element {
    Element::new(Role::Group).props(Style::new().width(100.0).height(100.0))
}

fn absolute(left: i64, top: i64) -> Element {
    group()
        .props(Style::new().width(20.0).height(20.0))
        .prop("position", "absolute")
        .prop("left", left)
        .prop("top", top)
}

fn window(child: Element) -> Node {
    Element::new(Role::Window).child(child).into_node()
}

fn layout(root: &Node) -> Layout {
    let mut layout = Layout::new();
    layout.compute(root, WINDOW, &mut FixedMetrics::default());
    layout
}

#[test]
fn no_absolute_discovery_at_mounted_scales_and_no_storage_growth() {
    // 64 since the kept answers (a pointer, empty for most boxes).
    assert_eq!(size_of::<LayoutBox>(), 64);
    assert_eq!(size_of_val(&LayoutBox::default().flags), size_of::<u32>());
    for count in [100, 1_000, 10_000] {
        // Exactly count mounted boxes including root, target and scroll container.
        let root = Element::new(Role::Window)
            .child(group().props(Style::new().height(20.0)))
            .child(
                group()
                    .props(Style::new().height(200.0).scroll_y(0.0))
                    .children((0..count - 3).map(|_| group().props(Style::new().height(20.0)))),
            )
            .into_node();
        let layout = layout(&root);
        assert!(!assert_summary(&layout.root));
        assert_eq!(exhaustive_lifted(&layout, 5.0, 5.0).1, count - 1);
        for turned in [false, true] {
            let (path, visits) = counted(|| {
                if turned {
                    layout.hit_test_turned(&root, 5.0, 5.0, &undo)
                } else {
                    layout.hit_test(5.0, 5.0)
                }
            });
            assert_eq!(path, Some(vec![0]));
            assert_eq!(path, reference(&layout, &root, 5.0, 5.0, turned));
            assert_eq!(visits, 0, "mounted={count}, turned={turned}");
        }
    }
}

#[test]
fn prunes_empty_branches_and_absolute_nodes_without_absolute_descendants() {
    let root = Element::new(Role::Window)
        .child(group().children((0..100).map(|_| group())))
        .child(group().child(absolute(150, 0).children((0..100).map(|_| group()))))
        .into_node();
    let layout = layout(&root);
    compare(&layout, &root);
    for turned in [false, true] {
        let (_, visits) = counted(|| {
            if turned {
                layout.hit_test_turned(&root, -5.0, -5.0, &undo)
            } else {
                layout.hit_test(-5.0, -5.0)
            }
        });
        assert_eq!(visits, 3); // two root children, one child of the positive branch
    }
    assert_eq!(exhaustive_lifted(&layout, -5.0, -5.0).1, 203);
}

#[test]
fn outside_ancestors_reverse_nested_targets_scroll_and_overlay_exclusion() {
    let root = window(
        group()
            .props(Style::new().width(20.0).height(20.0))
            .child(absolute(100, 0).key("first"))
            .child(absolute(100, 0).key("last").child(absolute(0, 0))),
    );
    let mut laid = layout(&root);
    compare(&laid, &root);
    assert_eq!(laid.hit_test(105.0, 5.0), Some(vec![0, 1, 0]));
    assert_eq!(
        laid.hit_test_turned(&root, 105.0, 5.0, &undo),
        Some(vec![0, 1, 0])
    );
    // Explicit reordering exercises retained boxes, not replacement.
    let mut reordered = (*root).clone();
    let mut parent = (*reordered.children[0]).clone();
    parent.children.swap(0, 1);
    reordered.children[0] = parent.into_node();
    let reordered = reordered.into_node();
    laid.apply(&diff(&root, &reordered));
    laid.compute(&reordered, WINDOW, &mut FixedMetrics::default());
    compare(&laid, &reordered);
    assert_eq!(laid.hit_test(105.0, 5.0), Some(vec![0, 1]));

    let scrolled = window(
        group()
            .props(Style::new().width(20.0).height(20.0).scroll_y(5.0))
            .child(group().child(absolute(100, 0))),
    );
    let laid = layout(&scrolled);
    compare(&laid, &scrolled);
    assert_eq!(laid.hit_lifted(105.0, 5.0), None);

    for inert in [false, true] {
        let root = window(
            group()
                .props(
                    Style::new()
                        .width(20.0)
                        .height(20.0)
                        .overlay(Placement::At(50.0, 50.0)),
                )
                .prop("inert", inert)
                .child(absolute(100, 0)),
        );
        let laid = layout(&root);
        compare(&laid, &root);
        assert!(!laid.root.has(CONTAINS_ABSOLUTE));
        assert!(laid.root.children[0].has(CONTAINS_ABSOLUTE));
        assert_eq!(counted(|| laid.hit_lifted(155.0, 55.0)).1, 0);
        // What's placed by itself in what's on top of everything is hit where it stands out of it, as sonner's toasts stand
        // out of their fixed list, unless that's inert.
        assert_eq!(laid.hit_test(155.0, 55.0), Some(if inert { vec![] } else { vec![0, 0] }));
        assert_eq!(
            laid.hit_test(55.0, 55.0),
            Some(if inert { vec![] } else { vec![0] })
        );
    }
}

fn states(child: Element) -> Node {
    window(group().child(group().child(child)))
}

#[test]
fn same_size_position_overlay_and_structural_transitions_survive_cache_and_resize() {
    let branch = |children: Vec<Element>| group().key("branch").children(children);
    let roots = [
        states(branch(vec![group().key("a")])),
        states(branch(vec![absolute(150, 0).key("a")])),
        states(branch(vec![
            absolute(150, 0).key("a").prop("overlay", "center"),
        ])),
        states(branch(vec![absolute(150, 0).key("a")])),
        states(branch(vec![group().key("a")])),
        states(branch(vec![
            group().key("a"),
            absolute(150, 0).key("b"),
            absolute(150, 0).key("c"),
        ])),
        states(branch(vec![
            absolute(150, 0).key("c"),
            group().key("a"),
            absolute(150, 0).key("b"),
        ])),
        states(branch(vec![group().key("a")])),
    ];
    let mut laid = Layout::new();
    for (i, root) in roots.iter().enumerate() {
        if i > 0 {
            laid.apply(&diff(&roots[i - 1], root));
        }
        laid.compute(root, WINDOW, &mut FixedMetrics::default());
        assert_eq!(laid.rect(&[0]), Some(Rect::new(0.0, 0.0, 100.0, 100.0)));
        assert_eq!(laid.root.has(CONTAINS_ABSOLUTE), matches!(i, 1 | 3 | 5 | 6));
        compare(&laid, root);
        // Compare geometry to a fresh solve too; same tree, different history.
        assert_eq!(laid.outline(root), layout(root).outline(root));
        laid.compute(root, WINDOW, &mut FixedMetrics::default());
        assert_eq!(laid.last_solved(), 0);
        compare(&laid, root);
        for size in [Size::new(450.0, 350.0), WINDOW] {
            laid.compute(root, size, &mut FixedMetrics::default());
            compare(&laid, root);
        }
    }
}

struct Pieces;
impl Measure for Pieces {
    fn measure(&mut self, _: &Element, _: f32) -> Size {
        Size::new(100.0, 20.0)
    }
    fn place_pieces(&mut self, _: &Element, _: f32, places: &mut Vec<Rect>) {
        places.extend([
            Rect::new(0.0, 0.0, 20.0, 20.0),
            Rect::new(20.0, 0.0, 20.0, 20.0),
        ]);
    }
}

#[test]
fn contents_paragraph_grid_and_table_construction_and_restoration() {
    let roots = [
        states(
            group()
                .prop("display", "contents")
                .child(group().prop("display", "contents").child(absolute(150, 0))),
        ),
        states(
            group()
                .prop("display", "grid")
                .prop("columns", "1fr 1fr")
                .child(group())
                .child(absolute(150, 0)),
        ),
        states(
            group().prop("table_part", "table").child(
                group().prop("table_part", "group").child(
                    group()
                        .prop("table_part", "row")
                        .child(group().prop("table_part", "cell").child(absolute(150, 0)))
                        .child(absolute(150, 0)),
                ),
            ),
        ),
    ];
    for root in roots {
        let mut laid = layout(&root);
        assert!(assert_summary(&laid.root));
        compare(&laid, &root);
        laid.compute(&root, Size::new(250.0, 250.0), &mut FixedMetrics::default());
        compare(&laid, &root);
    }
    // Removing an overlay makes existing text become a paragraph without
    // replacing its other parts.
    // place_part historically retains IS_ABSOLUTE; the summary must describe
    // that actual state, rather than silently repairing unrelated hit semantics.
    let parts = |overlay| {
        Element::new(Role::Text)
            .child(group().child(absolute(0, 0).name("one")))
            .child(if overlay {
                group().name("two").prop("overlay", "center")
            } else {
                group().name("two")
            })
    };
    let before = window(parts(true));
    let paragraph = window(parts(false));
    let mut laid = layout(&before);
    laid.apply(&diff(&before, &paragraph));
    laid.compute(&paragraph, WINDOW, &mut Pieces);
    assert!(laid.root.children[0].has(PARAGRAPH));
    assert!(laid.root.has(CONTAINS_ABSOLUTE));
    compare(&laid, &paragraph);
    let mut fresh = Layout::new();
    fresh.compute(&paragraph, WINDOW, &mut Pieces);
    assert!(!fresh.root.has(CONTAINS_ABSOLUTE));
    compare(&fresh, &paragraph);
}

#[test]
fn what_takes_the_pointer_in_something_inert_is_hit() {
    // As input-otp's `<input>` (`pointer-events: all`) in its `position: absolute; pointer-events: none` box: the box lets the
    // pointer through, and the input takes it. Without saying so, what's in the box lets it through too.
    let made = |takes: bool| {
        let input = absolute(0, 0).key("input");
        let input = if takes { input.prop("inert", false) } else { input };
        window(group().child(absolute(0, 0).prop("inert", true).child(input)))
    };
    let root = made(true);
    let laid = layout(&root);
    assert_eq!(laid.hit_test(5.0, 5.0), Some(vec![0, 0, 0]));
    assert_eq!(laid.hit_test_turned(&root, 5.0, 5.0, &undo), Some(vec![0, 0, 0]));
    let root = made(false);
    let laid = layout(&root);
    assert_eq!(laid.hit_test(5.0, 5.0), Some(vec![0]));
}

#[test]
fn what_is_placed_by_itself_is_hit_where_its_transform_shows_it() {
    // As a Radix menu is: `position: fixed` at 0, 0 and moved by its transform to where it's shown. A click where it's shown
    // finds it, and one where it would be unmoved doesn't, once things can be turned.
    let root = window(group().child(absolute(0, 0).key("menu").prop("test_turn", true)));
    let laid = layout(&root);
    compare(&laid, &root);
    assert_eq!(laid.hit_test_turned(&root, 90.0, 5.0, &undo), Some(vec![0, 0]));
    assert_eq!(laid.hit_test_turned(&root, 5.0, 5.0, &undo), Some(vec![0]));
    assert_eq!(laid.hit_test(5.0, 5.0), Some(vec![0, 0]));
    // And so is an overlay, which is what `position: fixed` is here.
    let root = window(
        group().child(
            group()
                .props(Style::new().width(20.0).height(20.0).overlay(Placement::At(0.0, 0.0)))
                .prop("test_turn", true),
        ),
    );
    let laid = layout(&root);
    compare(&laid, &root);
    assert_eq!(laid.hit_test_turned(&root, 90.0, 5.0, &undo), Some(vec![0, 0]));
    assert_eq!(laid.hit_test(5.0, 5.0), Some(vec![0, 0]));
}

#[test]
fn exhaustive_reference_over_combinations_of_flags_transforms_and_stacking() {
    // All combinations of absolute, scroll, overlay, inert, contents, paragraph
    // and stacked flags on nested boxes. Geometry includes overflowing targets.
    fn make(depth: usize, seed: usize) -> (LayoutBox, Node) {
        let options = [
            IS_ABSOLUTE,
            SCROLLS,
            IS_OVERLAY,
            OVERLAY_IS_INERT,
            CONTENTS,
            PARAGRAPH,
            STACKED,
        ];
        let mut current = LayoutBox {
            x: if depth == 1 { 40.0 } else { 0.0 },
            y: 0.0,
            width: 30.0,
            height: 30.0,
            flags: options
                .iter()
                .enumerate()
                .filter(|(i, _)| seed & (1 << i) != 0)
                .fold(0, |bits, (_, bit)| bits | bit),
            ..LayoutBox::default()
        };
        let mut node = group()
            .prop("test_turn", true)
            .prop("z_index", (seed % 3) as i64);
        if depth > 0 {
            for offset in [0, 37] {
                let (child_box, child) = make(depth - 1, (seed + offset) % 128);
                current.children.push(child_box);
                node.children.push(child);
            }
        }
        current.update_contains_overlay();
        current.update_contains_absolute();
        (current, node.into_node())
    }
    for seed in 0..128 {
        let (root, node) = make(2, seed);
        let layout = Layout {
            root,
            ..Layout::default()
        };
        compare(&layout, &node);
    }
}

#[test]
fn multiple_dirty_branches_and_batched_reorder_keep_the_other_positive_branch() {
    let make = |a, b, reverse| {
        let mut children: Vec<_> = (0..80)
            .map(|i| {
                let child = if (i == 0 && a) || (i == 79 && b) {
                    absolute(150, 0)
                } else {
                    group()
                };
                group()
                    .key(format!("branch-{i}"))
                    .child(group().child(child))
            })
            .collect();
        if reverse {
            children.reverse();
        }
        Element::new(Role::Window).children(children).into_node()
    };
    let roots = [
        make(false, false, false),
        make(true, true, false),
        make(false, true, false),
        make(false, true, true),
        make(false, false, true),
    ];
    let mut laid = Layout::new();
    for (i, root) in roots.iter().enumerate() {
        if i > 0 {
            let patches = diff(&roots[i - 1], root);
            if i == 3 {
                assert!(patches.iter().any(|p| matches!(p, Patch::Reorder { .. })));
            }
            laid.apply(&patches);
        }
        laid.compute(root, WINDOW, &mut FixedMetrics::default());
        assert_eq!(laid.root.has(CONTAINS_ABSOLUTE), matches!(i, 1..=3));
        compare(&laid, root);
    }
}

#[test]
fn contents_and_parent_owned_grid_table_flags_follow_same_size_changes() {
    for kind in ["contents", "grid", "table"] {
        let make = |present| {
            let leaf = if present { absolute(150, 0) } else { group() };
            let inside = match kind {
                "contents" => group()
                    .prop("display", "contents")
                    .child(group().prop("display", "contents").child(leaf)),
                "grid" => group()
                    .prop("display", "grid")
                    .prop("columns", "1fr")
                    .child(leaf),
                _ => group().prop("table_part", "table").child(
                    group().prop("table_part", "group").child(
                        group()
                            .prop("table_part", "row")
                            .child(group().prop("table_part", "cell"))
                            .child(leaf),
                    ),
                ),
            };
            states(inside)
        };
        let roots = [make(false), make(true), make(false)];
        let mut laid = Layout::new();
        for (i, root) in roots.iter().enumerate() {
            if i > 0 {
                laid.apply(&diff(&roots[i - 1], root));
            }
            laid.compute(root, WINDOW, &mut FixedMetrics::default());
            assert_eq!(laid.root.has(CONTAINS_ABSOLUTE), i == 1, "{kind}");
            compare(&laid, root);
            laid.compute(root, WINDOW, &mut FixedMetrics::default());
            compare(&laid, root);
        }
    }
}

/// Records, for each element with a name, the clips in force when it's entered.
#[derive(Default)]
struct Clips {
    stack: Vec<Rect>,
    seen: Vec<(String, Vec<Rect>)>,
}

impl LayoutVisitor for Clips {
    fn enter(&mut self, element: &Element, _rect: Rect) {
        if !element.name.is_empty() {
            self.seen.push((element.name.to_string(), self.stack.clone()));
        }
    }
    fn exit(&mut self, _element: &Element) {}
    fn clip_to(&mut self, rect: Rect) {
        self.stack.push(rect);
    }
    fn unclip(&mut self) {
        self.stack.pop();
    }
}

/// What's `position: relative` inside something that's `overflow: hidden` (`clip`) is drawn later, with what's placed by
/// itself, and still cut off at that box's edges, as CSS has it: the shadcn docs' preview frame clips its centered example,
/// which is in a `relative` box. Something `position: absolute` whose containing block is outside the clipping box isn't.
#[test]
fn what_is_placed_by_itself_is_cut_off_by_a_clipping_box_around_it() {
    let clipping = |child: Element| Element::new(Role::Group).props(Style::new().width(100.0).height(50.0)).prop("clip", true).child(child);
    let tall = |name: &str| Element::new(Role::Group).name(name).props(Style::new().width(20.0).height(200.0));
    // A relative box in the clip, with a tall child.
    let relative = clipping(group().prop("position", "relative").child(tall("in-relative")));
    // An absolute box whose containing block is the window, outside the clip: not cut off.
    let escaping = clipping(tall("escaping").prop("position", "absolute").prop("left", 0).prop("top", 0));
    // The same in a positioned clip: cut off.
    let contained = clipping(tall("contained").prop("position", "absolute").prop("left", 0).prop("top", 0)).prop("position", "relative");
    for (root, name, cut) in [(window(relative), "in-relative", true), (window(escaping), "escaping", false), (window(contained), "contained", true)] {
        let layout = layout(&root);
        let mut clips = Clips::default();
        layout.visit(&root, &mut clips);
        let (_, stack) = clips.seen.iter().find(|(seen, _)| seen == name).unwrap_or_else(|| panic!("{name} wasn't visited"));
        assert_eq!(!stack.is_empty(), cut, "{name}: clips {stack:?}, all {:?}", clips.seen);
    }
}


#[test]
fn fused_lifted_preserves_inert_reentry_transforms_clipping_and_missing_nodes() {
    fn make(depth: usize, seed: usize) -> (LayoutBox, Node) {
        let options = [IS_ABSOLUTE, SCROLLS, IS_OVERLAY, ABSOLUTE_IS_INERT,
            ABSOLUTE_TAKES_POINTER, CONTENTS, STACKED];
        let mut current = LayoutBox {
            x: if depth == 1 { 35.0 } else { -5.0 }, y: 3.0,
            width: 30.0, height: 25.0,
            flags: options.iter().enumerate().filter(|(i, _)| seed & (1 << i) != 0)
                .fold(0, |bits, (_, bit)| bits | bit),
            ..LayoutBox::default()
        };
        let mut node = group().prop("test_turn", true).prop("z_index", (seed % 3) as i64);
        if depth > 0 {
            for offset in [0, 37, 71] {
                let (child_box, child) = make(depth - 1, (seed + offset) % 128);
                current.children.push(child_box); node.children.push(child);
            }
        }
        current.update_contains_absolute();
        (current, node.into_node())
    }
    for seed in 0..128 {
        let (root, node) = make(3, seed);
        let layout = Layout { root, ..Layout::default() };
        for y in [-20.0, 0.0, 5.0, 20.0, 35.0, 80.0, 100.0] {
            for x in [-20.0, 0.0, 5.0, 20.0, 35.0, 80.0, 100.0] {
                assert_eq!(layout.hit_lifted_turned_fused(&node, x, y, &undo),
                    layout.hit_lifted_turned_materialized(&node, x, y, &undo),
                    "seed {seed} at {x},{y}");
            }
        }
    }
    let node = window(group().child(absolute(0, 0)));
    let layout = layout(&node);
    let missing = Element::new(Role::Window).into_node();
    for x in [0.0, 5.0, 100.0] {
        assert_eq!(layout.hit_lifted_turned_fused(&missing, x, 5.0, &undo),
            layout.hit_lifted_turned_materialized(&missing, x, 5.0, &undo));
    }
}

#[test]
fn fused_lifted_applies_shared_ancestor_transforms_once() {
    let root = window(group().child(group().children((0..64).map(|_| absolute(200, 200)))));
    let layout = layout(&root);
    let calls = Cell::new(0);
    let count = |_: &Element, _: Rect| { calls.set(calls.get()+1); None };
    let reference = layout.hit_lifted_turned_materialized(&root, 5.0, 5.0, &count);
    let materialized_calls = calls.replace(0);
    assert_eq!(layout.hit_lifted_turned_fused(&root, 5.0, 5.0, &count), reference);
    assert_eq!(calls.get(), 67); // three shared ancestors, 64 candidate boxes
    assert_eq!(materialized_calls, 256);
}
