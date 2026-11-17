use rsui_core::{Element, PropValue, Role, diff};
use rsui_layout::{FixedMetrics, Layout, Size};
fn tree(value: Option<(&str, PropValue)>, width: f64) -> rsui_core::Node {
    let mut child = Element::new(Role::Group)
        .prop("width", width)
        .prop("height", 20.0);
    if let Some((name, value)) = value {
        child = child.prop(name, value);
    }
    Element::new(Role::Window).child(child).into_node()
}
#[test]
fn visual_patches_do_no_solver_work_but_unknown_and_deferred_geometry_do() {
    let size = Size::new(200.0, 100.0);
    let mut layout = Layout::new();
    let mut measure = FixedMetrics::default();
    let a = tree(None, 50.0);
    layout.compute(&a, size, &mut measure);
    let b = tree(Some(("background", PropValue::Int(0xff0000ff))), 50.0);
    // Preserve the original public method's unit-returning function signature.
    let apply: fn(&mut Layout, &[rsui_core::Patch]) = Layout::apply;
    apply(&mut layout, &diff(&a, &b));
    layout.compute(&b, size, &mut measure);
    assert_eq!(layout.last_solved(), 0);
    layout.apply(&diff(&b, &a));
    layout.compute(&a, size, &mut measure);
    assert_eq!(layout.last_solved(), 0);
    let unknown = tree(
        Some(("future_measurement_property", PropValue::Bool(true))),
        50.0,
    );
    layout.apply(&diff(&a, &unknown));
    layout.compute(&unknown, size, &mut measure);
    assert!(layout.last_solved() > 0);
    let wide = tree(None, 80.0);
    let colored = tree(Some(("background", PropValue::Int(0x0000ffff))), 80.0);
    layout.apply(&diff(&unknown, &wide));
    layout.apply(&diff(&wide, &colored));
    layout.compute(&colored, size, &mut measure);
    assert!(layout.last_solved() > 0);
    let mut fresh = Layout::new();
    fresh.compute(&colored, size, &mut measure);
    assert_eq!(layout.rect(&[0]), fresh.rect(&[0]));
}
