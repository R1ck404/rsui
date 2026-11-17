//! What can't be seen is left out of a visit only when nothing in it could draw where something shows.
use rsui_core::{Element, Role, diff};
use rsui_layout::{FixedMetrics, Layout, LayoutVisitor, Rect, Size};

/// Says boxes below `shows_until` draw nothing, as a painter does for what's past the bottom of the window.
struct Visitor {
    shows_until: f32,
    entered: Vec<String>,
    depth: usize,
}

impl Visitor {
    fn new(shows_until: f32) -> Self {
        Self { shows_until, entered: Vec::new(), depth: 0 }
    }
}

impl LayoutVisitor for Visitor {
    fn enter(&mut self, node: &Element, _: Rect) {
        self.entered.push(node.name.to_string());
        self.depth += 1;
    }
    fn exit(&mut self, _: &Element) {
        self.depth -= 1;
    }
    fn draws_nothing_at(&self, rect: Rect) -> bool {
        rect.y >= self.shows_until || rect.y + rect.height <= 0.0
    }
}

fn row(name: &str) -> Element {
    Element::new(Role::Group)
        .name(name)
        .prop("height", 20.0)
        .child(Element::new(Role::Text).name(format!("{name} text")))
}

fn page(rows: Vec<Element>) -> rsui_core::Node {
    Element::new(Role::Window)
        .child(Element::new(Role::Group).name("page").children(rows))
        .into_node()
}

fn entered(layout: &mut Layout, root: &rsui_core::Node, shows_until: f32) -> Vec<String> {
    layout.compute(root, Size::new(200.0, 100.0), &mut FixedMetrics::default());
    let mut visitor = Visitor::new(shows_until);
    layout.visit(root, &mut visitor);
    assert_eq!(visitor.depth, 0);
    visitor.entered
}

#[test]
fn rows_past_the_window_are_left_out() {
    let root = page((0..50).map(|at| row(&format!("row {at}"))).collect());
    let mut layout = Layout::new();
    let names = entered(&mut layout, &root, 100.0);
    // The window, the page and the five rows that show, with their text.
    assert_eq!(names.len(), 2 + 5 * 2, "{names:?}");
    assert_eq!(names.last().map(String::as_str), Some("row 4 text"));
    // A visitor that never says so gets everything.
    struct All(usize);
    impl LayoutVisitor for All {
        fn enter(&mut self, _: &Element, _: Rect) {
            self.0 += 1;
        }
        fn exit(&mut self, _: &Element) {}
    }
    let mut all = All(0);
    layout.visit(&root, &mut all);
    assert_eq!(all.0, 2 + 50 * 2);
}

#[test]
fn what_could_draw_outside_its_box_is_still_visited() {
    for (prop, value) in [
        ("shadows", rsui_core::PropValue::from("0 0 4 black")),
        ("outline_width", rsui_core::PropValue::Float(2.0)),
        ("transform", rsui_core::PropValue::from("translateY(-200px)")),
        ("scale", rsui_core::PropValue::Float(3.0)),
        ("opacity", rsui_core::PropValue::Float(0.5)),
        ("css_filter", rsui_core::PropValue::from("blur(2px)")),
        ("position", rsui_core::PropValue::from("relative")),
        ("z_index", rsui_core::PropValue::Int(3)),
    ] {
        let mut rows: Vec<Element> = (0..10).map(|at| row(&format!("row {at}"))).collect();
        rows[8] = rows[8].clone().prop(prop, value.clone());
        let root = page(rows);
        let names = entered(&mut Layout::new(), &root, 100.0);
        assert!(names.iter().any(|name| name == "row 8"), "{prop}: {names:?}");
        assert!(!names.iter().any(|name| name == "row 9"), "{prop}: {names:?}");
    }
}

#[test]
fn something_off_screen_with_what_reaches_into_view_is_visited() {
    // A row below the window whose child is pulled up into it, past the row's own top.
    let reaching = Element::new(Role::Group)
        .name("reaching")
        .prop("height", 20.0)
        .child(Element::new(Role::Group).name("pulled up").prop("margin_top", -150.0).prop("height", 20.0));
    let mut rows: Vec<Element> = (0..10).map(|at| row(&format!("row {at}"))).collect();
    rows[8] = reaching;
    let root = page(rows);
    let names = entered(&mut Layout::new(), &root, 100.0);
    assert!(names.iter().any(|name| name == "pulled up"), "{names:?}");
    assert!(!names.iter().any(|name| name == "row 9"), "{names:?}");
}

#[test]
fn adding_and_taking_away_what_draws_outside_follows_edits() {
    let make = |shadow: bool| {
        let mut rows: Vec<Element> = (0..10).map(|at| row(&format!("row {at}"))).collect();
        if shadow {
            rows[7] = rows[7].clone().child(Element::new(Role::Group).name("shadowed").prop("shadow_blur", 8.0));
        }
        page(rows)
    };
    let mut layout = Layout::new();
    let mut root = make(false);
    assert!(!entered(&mut layout, &root, 100.0).iter().any(|name| name == "row 7"));
    for shadow in [true, false, true] {
        let next = make(shadow);
        layout.apply(&diff(&root, &next));
        root = next;
        let names = entered(&mut layout, &root, 100.0);
        assert_eq!(names.iter().any(|name| name == "shadowed"), shadow, "{names:?}");
        assert_eq!(names.iter().any(|name| name == "row 7"), shadow, "{names:?}");
        assert!(!names.iter().any(|name| name == "row 8"), "{names:?}");
    }
}

#[test]
fn absolute_and_overlays_inside_are_not_left_out() {
    let mut rows: Vec<Element> = (0..10).map(|at| row(&format!("row {at}"))).collect();
    rows[9] = rows[9]
        .clone()
        .child(Element::new(Role::Group).name("absolute").prop("position", "absolute").prop("top", -180.0).prop("height", 10.0));
    let root = page(rows);
    let names = entered(&mut Layout::new(), &root, 100.0);
    assert!(names.iter().any(|name| name == "absolute"), "{names:?}");
}
