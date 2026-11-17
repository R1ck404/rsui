use super::*;
use rsui_core::Role;
#[path = "../../rsui-text/tests/support/mod.rs"]
mod font;

#[test]
fn svg_hash_inputs_repeat_and_change_without_losing_legacy_bytes() {
    let fast = SvgHash::Fast(foldhash::fast::RandomState::default());
    let legacy = SvgHash::Legacy;
    let element = Element::new(Role::Group).prop("view_box", "0 0 24 24");
    let color = Color::rgba(1, 2, 3, 4);
    let layers = "path data";
    let base = fast.key(&element, layers, 24.0, 32.0, color);
    assert_eq!(base, fast.key(&element, layers, 24.0, 32.0, color));
    let mut old = std::collections::hash_map::DefaultHasher::new();
    layers.hash(&mut old);
    (24_u32, 32_u32).hash(&mut old);
    [1_u8, 2, 3, 4].hash(&mut old);
    if let Some(PropValue::Str(view)) = element.props.get("view_box") { view.hash(&mut old); }
    false.hash(&mut old);
    assert_eq!(legacy.key(&element, layers, 24.0, 32.0, color), old.finish());
    for mode in [&legacy, &fast] {
        let original = mode.key(&element, layers, 24.0, 32.0, color);
        for changed in [
            mode.key(&element, "changed path", 24.0, 32.0, color),
            mode.key(&element, layers, 25.0, 32.0, color),
            mode.key(&element, layers, 24.0, 33.0, color),
            mode.key(&element, layers, 24.0, 32.0, Color::rgba(9, 2, 3, 4)),
            mode.key(&element, layers, 24.0, 32.0, Color::rgba(1, 9, 3, 4)),
            mode.key(&element, layers, 24.0, 32.0, Color::rgba(1, 2, 9, 4)),
            mode.key(&element, layers, 24.0, 32.0, Color::rgba(1, 2, 3, 9)),
            mode.key(&Element::new(Role::Group).prop("view_box", "0 0 48 24"), layers, 24.0, 32.0, color),
            mode.key(&Element::new(Role::Group), layers, 24.0, 32.0, color),
            mode.key(&Element::new(Role::Group).prop("view_box", "0 0 24 24").prop("stretch", true), layers, 24.0, 32.0, color),
        ] { assert_ne!(original, changed); }
    }
}

#[test]
fn svg_hash_seed_survives_cache_trimming_and_renderer_replacement() {
    let mut painter = Painter { svg_hash: SvgHash::Fast(foldhash::fast::RandomState::default()), ..Default::default() };
    let element = Element::new(Role::Group);
    let key = painter.svg_hash.key(&element, "path", 30.0, 20.0, Color::WHITE);
    painter.forget_caches();
    assert_eq!(key, painter.svg_hash.key(&element, "path", 30.0, 20.0, Color::WHITE));
    painter.renderer_replaced();
    assert_eq!(key, painter.svg_hash.key(&element, "path", 30.0, 20.0, Color::WHITE));
}

#[test]
fn svg_hash_offscreen_pixels_match_legacy_across_shape_changes_and_cache_reuse() {
    const SIZE: (u32, u32) = (80, 64);
    let mut fast = Painter { svg_hash: SvgHash::Fast(foldhash::fast::RandomState::default()), ..Default::default() };
    let mut legacy = Painter { svg_hash: SvgHash::Legacy, ..Default::default() };
    let mut text = TextEngine::new(font::plain_font());
    let mut reference_text = TextEngine::new(font::plain_font());
    let mut gpu = crate::Renderer::offscreen(SIZE).unwrap();
    let mut reference = crate::Renderer::offscreen(SIZE).unwrap();
    reference.set_always_redraw(true);
    for step in 0..10 {
        let path = if step < 3 { "M2 2 L22 2 L22 22 L2 22 Z" } else { "M2 2 L22 22 L2 22 Z" };
        let layers = crate::shape_layer(path, Some(None), Some((Some(Color::BLACK), 1.0)), crate::LineCap::Round);
        let root = Element::new(Role::Window).child(Element::new(Role::Image)
            .prop("width", if step < 5 { 35.0 } else { 46.0 })
            .prop("height", if step < 4 { 32.0 } else { 45.0 })
            .prop("shape_layers", layers)
            .prop("view_box", if step < 6 { "0 0 24 24" } else { "0 0 30 24" })
            .prop("stretch", step >= 7)
            .prop("text_color", if step < 2 { 0x804020FF_i64 } else { 0x204080AA_i64 }))
            .into_node();
        let mut layout = Layout::new();
        layout.compute(&root, rsui_layout::Size::new(80.0,64.0), &mut text);
        let mut other_layout = Layout::new();
        other_layout.compute(&root, rsui_layout::Size::new(80.0,64.0), &mut reference_text);
        let scene = fast.paint(&root,&layout,&mut text,1.0,SIZE);
        if step == 1 { assert!(scene.image_uploads.is_empty(), "unchanged SVG must reuse its cached raster"); }
        assert!(!scene.quads.is_empty(), "fixture must draw an SVG");
        gpu.render(scene,Color::WHITE).unwrap();
        reference.render(legacy.paint(&root,&other_layout,&mut reference_text,1.0,SIZE),Color::WHITE).unwrap();
        assert_eq!(gpu.read_pixels().unwrap(),reference.read_pixels().unwrap(),"step {step}");
    }
}
