//! Visitor opt-in and cached proof invalidation, independent of rendering.
use rsui_core::{Element, Role, diff};
use rsui_layout::{FixedMetrics, Layout, LayoutVisitor, Rect, Size};
#[derive(Default)]
struct Visitor {
    prune: bool,
    entered: Vec<String>,
    depth: usize,
}
impl LayoutVisitor for Visitor {
    fn enter(&mut self, node: &Element, _: Rect) {
        self.entered.push(node.name.to_string());
        self.depth += 1;
    }
    fn exit(&mut self, _: &Element) {
        self.depth -= 1;
    }
    fn clipped_children_are_invisible(&self) -> bool {
        self.prune
    }
}
fn tree(effect: Option<&str>) -> rsui_core::Node {
    let mut child = Element::new(Role::Group)
        .name("child")
        .prop("height", 10.0)
        .child(Element::new(Role::Text).name("grandchild"));
    if let Some(effect) = effect {
        child = child.prop(effect, true);
    }
    Element::new(Role::Window)
        .child(
            Element::new(Role::Group)
                .name("clip")
                .prop("clip", true)
                .prop("height", 20.0)
                .child(child),
        )
        .into_node()
}
#[test]
fn ordinary_visitors_keep_callbacks_and_unknown_paint_changes_invalidate_ancestors() {
    let mut layout = Layout::new();
    let mut root = tree(None);
    for effect in [
        None,
        Some("transform"),
        None,
        Some("future_paint_effect"),
        None,
    ] {
        let next = tree(effect);
        layout.apply(&diff(&root, &next));
        root = next;
        layout.compute(&root, Size::new(100.0, 80.0), &mut FixedMetrics::default());
        let mut full = Visitor::default();
        layout.visit(&root, &mut full);
        assert_eq!(full.entered, ["", "clip", "child", "grandchild"]);
        let mut paint = Visitor {
            prune: true,
            ..Default::default()
        };
        layout.visit(&root, &mut paint);
        assert_eq!(paint.depth, 0);
        if effect.is_some() {
            assert_eq!(paint.entered, full.entered);
        } else {
            assert_eq!(paint.entered, ["", "clip"]);
        }
    }
}
#[test]
fn structural_edits_do_not_reuse_a_removed_subtrees_proof() {
    let ordinary = Element::new(Role::Group).name("ordinary");
    let unusual = Element::new(Role::Group)
        .name("unusual")
        .prop("position", "relative");
    let make = |children: Vec<Element>| {
        Element::new(Role::Window)
            .child(
                Element::new(Role::Group)
                    .prop("clip", true)
                    .children(children),
            )
            .into_node()
    };
    let trees = [
        make(vec![ordinary.clone()]),
        make(vec![ordinary.clone(), unusual.clone()]),
        make(vec![unusual, ordinary.clone()]),
        make(vec![ordinary]),
    ];
    let mut layout = Layout::new();
    let mut root = trees[0].clone();
    for (i, next) in trees.into_iter().enumerate() {
        layout.apply(&diff(&root, &next));
        root = next;
        layout.compute(&root, Size::new(100.0, 80.0), &mut FixedMetrics::default());
        let mut visitor = Visitor {
            prune: true,
            ..Default::default()
        };
        layout.visit(&root, &mut visitor);
        assert_eq!(visitor.depth, 0);
        assert_eq!(
            visitor.entered.iter().any(|name| name == "ordinary"),
            i == 1 || i == 2
        );
    }
}

#[test]
fn paint_only_batches_preserve_proof_but_effect_changes_rebuild_it() {
    let make = |color: i64, effect: bool| {
        let mut child = Element::new(Role::Text).name("label");
        if effect {
            child = child.prop("transform", "translate(1px, 0px)");
        }
        Element::new(Role::Window)
            .child(
                Element::new(Role::Group)
                    .name("clip")
                    .prop("clip", true)
                    .prop("background", color)
                    .child(child),
            )
            .into_node()
    };
    let mut root = make(0xff0000ff, false);
    let mut layout = Layout::new();
    layout.compute(&root, Size::new(100.0, 80.0), &mut FixedMetrics::default());
    for (color, effect, needs_layout) in [
        (0x0000ffff, false, false),
        (0x0000ffff, true, true),
        (0xff0000ff, true, false),
        (0xff0000ff, false, true),
    ] {
        let next = make(color, effect);
        assert_eq!(layout.apply_needs_layout(&diff(&root, &next)), needs_layout);
        root = next;
        if needs_layout {
            layout.compute(&root, Size::new(100.0, 80.0), &mut FixedMetrics::default());
        }
        let mut visitor = Visitor {
            prune: true,
            ..Default::default()
        };
        layout.visit(&root, &mut visitor);
        assert_eq!(visitor.depth, 0);
        assert_eq!(visitor.entered.iter().any(|name| name == "label"), effect);
    }
}

#[test]
fn turning_what_is_laid_out_in_a_paragraph_needs_no_layout() {
    // A spinner inside a button's text, as Mantine's `<Button loading>` has: laid out as a part of the paragraph, with no layout
    // of its own to say it was turned when it was laid out, which the tree from before the change says instead.
    let make = |turn: Option<f64>| {
        let mut spinner = Element::new(Role::Group)
            .name("spinner")
            .prop("width", 10.0)
            .prop("height", 10.0);
        if let Some(turn) = turn {
            spinner = spinner.prop("transform", format!("rotate({turn}deg)"));
        }
        Element::new(Role::Window)
            .child(
                Element::new(Role::Text)
                    .name("label")
                    .child(
                        Element::new(Role::Text)
                            .name("words")
                            .prop("text", "Loading"),
                    )
                    .child(spinner),
            )
            .into_node()
    };
    let mut root = make(Some(0.0));
    let mut layout = Layout::new();
    layout.compute(&root, Size::new(200.0, 80.0), &mut FixedMetrics::default());
    // Without it, the spinner's box doesn't say, so it's laid out again.
    let mut alone = Layout::new();
    alone.compute(&root, Size::new(200.0, 80.0), &mut FixedMetrics::default());
    assert!(alone.apply_needs_layout(&diff(&root, &make(Some(10.0)))));
    for (turn, needs_layout) in [
        (Some(10.0), false),
        (Some(250.5), false),
        (None, true),
        (Some(3.0), true),
    ] {
        let next = make(turn);
        assert_eq!(
            layout.apply_needs_layout_since(&diff(&root, &next), Some(&root)),
            needs_layout,
            "{turn:?}"
        );
        root = next;
        layout.compute(&root, Size::new(200.0, 80.0), &mut FixedMetrics::default());
    }
}
