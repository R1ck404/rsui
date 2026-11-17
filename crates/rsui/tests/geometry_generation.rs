//! Geometry query versions follow actual tree/layout changes, not empty updates.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use rsui::core::{Element, Role};
use rsui::layout::Style;
use rsui::runtime::{Cx, ScrollAxis};
use rsui::text::TextEngine;
use rsui::App;

fn page(cx: &mut Cx, _: &()) -> Element {
    let content = Element::new(Role::Group)
        .props(Style::new().flex(1.0))
        .child(Element::new(Role::Group).key("content").props(Style::new().height(1000.0)));
    Element::new(Role::Window).child(cx.scroll_area("area", ScrollAxis::Vertical, content))
}

fn app() -> App {
    let mut app = App::new(page, (), TextEngine::new(support::plain_font()), (200, 100), 1.0);
    app.set_animations(false);
    app.update();
    app.scene();
    rsui::geometry::keep();
    app
}

#[test]
fn empty_updates_preserve_query_versions_even_with_paint_pending() {
    let mut app = app();
    let generation = rsui::geometry::generation();
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(rsui::geometry::generation(), generation);

    app.scroll(50.0, 50.0, 3).unwrap();
    app.update();
    let scrolled = rsui::geometry::generation();
    assert_ne!(scrolled, generation);
    // The scroll has not been painted yet, but its boxes and tree are current.
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(rsui::geometry::generation(), scrolled);
    assert!(app.layout().rect(&[0, 0]).unwrap().y < -90.0);
}

#[test]
fn recomputing_after_resize_invalidates_even_when_the_tree_is_the_same() {
    let mut app = app();
    let tree = app.runtime().root().clone();
    let generation = rsui::geometry::generation();
    app.resize((300, 150), 1.0);
    app.update();
    app.scene();
    assert!(std::ptr::eq::<Element>(&*tree, &**app.runtime().root()));
    assert_ne!(rsui::geometry::generation(), generation);
    let area = app.layout().rect(&[0]).unwrap();
    assert_eq!((area.width, area.height), (300.0, 150.0));
}
