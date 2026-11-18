//! Idle release must preserve pixels and resource recovery, without another frame.
#![cfg(feature = "render-diagnostics")]
use rsui::layout::{Layout, Size};
use rsui::render::{Color, Painter, Renderer};
use rsui::{
    App,
    core::{ActionKind, Element, Role},
    runtime::Cx,
};
use std::time::Duration;
fn gallery(cx: &mut Cx, _: &()) -> Element {
    let large = cx.state(|| true);
    let mut root = Element::new(Role::Window).key("root");
    for i in 0..if cx.get(large) { 3 } else { 1 } {
        root = root.child(Element::new(Role::Image).prop("width",40.0).prop("height",40.0).prop("src",format!("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='40' height='40'><rect width='40' height='40' fill='rgb({},50,100)'/></svg>",i*80)));
    }
    cx.on(root, ActionKind::Toggle, move |cx, _| {
        cx.update(large, |v| *v = !*v)
    })
}
fn settle(a: &mut App, g: &mut Renderer) {
    for _ in 0..3 {
        a.render(g).unwrap();
        assert!(a.wait_for_images(Duration::from_secs(10)));
        a.update();
    }
    a.render(g).unwrap();
}
#[test]
fn idle_drops_reach_gpu_without_draw_and_visible_image_recovers() {
    let mut a = App::new(
        gallery,
        (),
        rsui::default_text_engine().unwrap(),
        (100, 150),
        1.0,
    );
    a.set_animations(false);
    a.update();
    let mut g = Renderer::offscreen((100, 150)).unwrap();
    settle(&mut a, &mut g);
    assert_eq!(g.retained_storage().image_entries, 3);
    a.runtime_mut()
        .dispatch(&[], ActionKind::Toggle, None)
        .unwrap();
    a.update();
    settle(&mut a, &mut g);
    let pixels = g.read_pixels().unwrap();
    let serial = g.draw_work().serial;
    assert!(!g.compact().unwrap());
    a.forget_caches();
    a.release_unused_images(&mut g);
    assert_eq!(g.draw_work().serial, serial, "cleanup must not draw");
    assert_eq!(g.retained_storage().image_entries, 1);
    assert_eq!(g.read_pixels().unwrap(), pixels);
    a.render(&mut g).unwrap();
    assert_eq!(
        g.draw_work().outcome,
        rsui::render::diagnostics::DrawOutcome::Unchanged,
        "retired offscreen images must not force a wake redraw"
    );
    g.trim_storage();
    settle(&mut a, &mut g);
    assert_eq!(g.read_pixels().unwrap(), pixels);
    g = Renderer::offscreen((100, 150)).unwrap();
    a.renderer_replaced();
    settle(&mut a, &mut g);
    assert_eq!(
        g.read_pixels().unwrap(),
        pixels,
        "visible image reload on device replacement"
    );
    a.runtime_mut()
        .dispatch(&[], ActionKind::Toggle, None)
        .unwrap();
    a.update();
    settle(&mut a, &mut g);
    assert_eq!(
        g.retained_storage().image_entries,
        3,
        "retired images reload on revisit"
    );
}
#[test]
fn pending_glyph_uploads_survive_idle_and_warm_labels_stay_cached() {
    let mut p = Painter::new();
    let mut text = rsui::default_text_engine().unwrap();
    let root = Element::new(Role::Window)
        .child(Element::new(Role::Text).name("Save / Open / Cancel"))
        .into_node();
    let mut layout = Layout::new();
    layout.compute(&root, Size::new(300.0, 100.0), &mut text);
    let before = p.paint(&root, &layout, &mut text, 1.0, (300, 100)).clone();
    assert!(!before.uploads.is_empty());
    p.forget_caches();
    assert_eq!(p.last_scene(), &before);
    let mut a = Renderer::offscreen((300, 100)).unwrap();
    let mut b = Renderer::offscreen((300, 100)).unwrap();
    a.render(p.last_scene(), Color::WHITE).unwrap();
    b.render(&mut before.clone(), Color::WHITE).unwrap();
    assert_eq!(a.read_pixels().unwrap(), b.read_pixels().unwrap());
    p.paint(&root, &layout, &mut text, 1.0, (300, 100));
    assert_eq!(
        p.scene_work().paragraph_callbacks,
        0,
        "live label run must remain cached"
    );
}

#[test]
fn active_long_text_can_reuse_spares_until_idle_without_unbounded_storage() {
    let mut painter = Painter::new();
    let mut text = rsui::default_text_engine().unwrap();
    let mut last = None;
    for step in 0..8 {
        let root = Element::new(Role::Window)
            .child(
                Element::new(Role::Text)
                    .name(format!(
                        "{} {step}",
                        "Ordinary paragraph text for editing. ".repeat(100)
                    ))
                    .prop("width", 760.0)
                    .prop("font_size", 14.0),
            )
            .into_node();
        let mut layout = Layout::new();
        layout.compute(&root, Size::new(800.0, 800.0), &mut text);
        painter.paint(&root, &layout, &mut text, 1.0, (800, 800));
        let bytes = painter.retained_storage().spare_text_quads.capacity_bytes;
        assert!(bytes <= 256 * 1024);
        if step >= 2 {
            assert!(
                bytes > 4096,
                "active paragraph storage should remain reusable"
            );
        }
        last = Some((root, layout));
    }
    let before = painter.last_scene().clone();
    painter.forget_caches();
    assert_eq!(
        painter.retained_storage().spare_text_quads.capacity_bytes,
        0
    );
    assert_eq!(
        painter.last_scene(),
        &before,
        "idle must preserve unconsumed commands"
    );
    let mut gpu = Renderer::offscreen((800, 800)).unwrap();
    let mut reference = Renderer::offscreen((800, 800)).unwrap();
    gpu.render(painter.last_scene(), Color::WHITE).unwrap();
    reference.render(&mut before.clone(), Color::WHITE).unwrap();
    assert_eq!(gpu.read_pixels().unwrap(), reference.read_pixels().unwrap());
    let (root, layout) = last.unwrap();
    painter.paint(&root, &layout, &mut text, 1.0, (800, 800));
    assert_eq!(
        painter.scene_work().paragraph_callbacks,
        0,
        "live text stays cached"
    );
}

#[test]
fn oversized_quad_buffer_shrinks_and_same_id_upload_after_idle_is_visible() {
    use rsui::render::{Batch, ImageUpload, PixelRect, Quad, Scene};
    fn scene(count: usize, upload: Option<[u8; 4]>) -> Scene {
        Scene {
            quads: vec![
                Quad {
                    rect: [0.0, 0.0, 20.0, 20.0],
                    glyph: [0, 0, 1, 1],
                    color: [255; 4],
                    radius: 0.0
                };
                count
            ],
            batches: vec![Batch {
                first: 0,
                count: count as u32,
                clip: PixelRect {
                    left: 0,
                    top: 0,
                    right: 40,
                    bottom: 40,
                },
                image: 7,
                transform: 0,
                filter: 0,
                round: 0,
            }],
            image_uploads: upload
                .map(|color| ImageUpload {
                    id: 7,
                    width: 1,
                    height: 1,
                    pixels: color.to_vec(),
                    changes: None,
                })
                .into_iter()
                .collect(),
            ..Default::default()
        }
    }
    for flipping in [false, true] {
        let make = || {
            if flipping {
                Renderer::offscreen_flipping((40, 40))
            } else {
                Renderer::offscreen((40, 40))
            }
            .unwrap()
        };
        let mut gpu = make();
        gpu.render(&mut scene(20000, Some([255, 0, 0, 255])), Color::WHITE)
            .unwrap();
        let large = gpu.retained_storage().quad_buffer_capacity_bytes;
        gpu.render(&mut scene(1, None), Color::WHITE).unwrap();
        let old = gpu.read_pixels().unwrap();
        let atlas = gpu.retained_storage().atlas_capacity_bytes;
        gpu.trim_storage();
        let small = gpu.retained_storage().quad_buffer_capacity_bytes;
        assert!(small * 4 < large);
        assert_eq!(gpu.retained_storage().atlas_capacity_bytes, atlas);
        gpu.render(&mut scene(1, None), Color::WHITE).unwrap();
        assert_eq!(old, gpu.read_pixels().unwrap());
        gpu.release_images(&mut vec![7, 999]);
        gpu.render(&mut scene(1, Some([0, 0, 255, 255])), Color::WHITE)
            .unwrap();
        let changed = gpu.read_pixels().unwrap();
        assert_ne!(old, changed);
        let mut reference = make();
        reference
            .render(&mut scene(1, Some([0, 0, 255, 255])), Color::WHITE)
            .unwrap();
        assert_eq!(changed, reference.read_pixels().unwrap());
        println!("quad buffer payload before={large} after={small}");
    }
}

#[allow(dead_code)]
#[path = "../benches/workload/mod.rs"]
mod workload;
#[test]
fn selection_and_kept_caret_pixels_survive_idle_and_device_recreation() {
    let mut a = workload::measured::input();
    let mut g = Renderer::offscreen(workload::measured::SIZE).unwrap();
    let visible = workload::measured::pixels(&mut a, &mut g);
    a.blink();
    a.update();
    let hidden = workload::measured::pixels(&mut a, &mut g);
    assert_ne!(visible, hidden);
    a.forget_caches();
    g.trim_storage();
    a.blink();
    a.update();
    assert_eq!(visible, workload::measured::pixels(&mut a, &mut g));
    a.key(rsui::KeyPress::new(rsui::KeyCode::Character('a')).ctrl())
        .unwrap();
    a.update();
    let selected = workload::measured::pixels(&mut a, &mut g);
    assert_ne!(visible, selected);
    a.forget_caches();
    g.trim_storage();
    assert_eq!(selected, workload::measured::pixels(&mut a, &mut g));
    g = Renderer::offscreen(workload::measured::SIZE).unwrap();
    a.renderer_replaced();
    assert_eq!(selected, workload::measured::pixels(&mut a, &mut g));
}

#[test]
fn first_small_frame_can_release_the_swapped_out_large_scene() {
    fn view(cx: &mut Cx, _: &()) -> Element {
        let large = cx.state(|| true);
        let root = Element::new(Role::Window).key("root").children(
            (0..if cx.get(large) { 2048 } else { 1 }).map(|_| {
                Element::new(Role::Group)
                    .prop("height", 1.0)
                    .prop("width", 10.0)
                    .prop("background", 0xff0000ff_i64)
                    .into_node()
            }),
        );
        cx.on(root, ActionKind::Toggle, move |cx, _| {
            cx.update(large, |v| *v = !*v)
        })
    }
    let mut a = App::new(
        view,
        (),
        rsui::default_text_engine().unwrap(),
        (100, 2048),
        1.0,
    );
    a.set_animations(false);
    a.update();
    let mut g = Renderer::offscreen((100, 2048)).unwrap();
    a.render(&mut g).unwrap();
    a.runtime_mut()
        .dispatch(&[], ActionKind::Toggle, None)
        .unwrap();
    a.update();
    a.render(&mut g).unwrap();
    let pixels = g.read_pixels().unwrap();
    #[cfg(not(feature = "automatic-render"))]
    assert!(a.render_storage().scene.quads.capacity_bytes > 16384);
    // Owned submission already trims the comparison spare on the shrinking draw.
    // Its live commands must remain authoritative until the next frame.
    #[cfg(feature = "automatic-render")]
    assert!(a.render_storage().scene.quads.len_bytes > 0);
    a.forget_caches();
    let storage = a.render_storage();
    assert!(
        storage.scene.quads.capacity_bytes + storage.comparison_spare.quads.capacity_bytes <= 8192
    );
    a.render(&mut g).unwrap();
    assert_eq!(pixels, g.read_pixels().unwrap());
}
