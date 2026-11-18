//! Isolated feature-test allocator; WARP comparisons use independently owned resources.
#![cfg(feature = "render-diagnostics")]

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;
use rsui_core::{Element, Node, Role};
use rsui_layout::{Layout, Size, Style};
use rsui_render::diagnostics::*;
use rsui_render::{Batch, Color, ImageUpload, Paint, Painter, PixelRect, Quad, Renderer, Scene};
use rsui_text::{TextEngine, Upload};
use std::alloc::{GlobalAlloc, Layout as AllocationLayout, System};
use std::cell::Cell;

thread_local! { static ALLOCATIONS: Cell<u64> = const { Cell::new(0) }; }
struct Allocator;
// SAFETY: all allocator inputs/results are forwarded unchanged to System.
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: AllocationLayout) -> *mut u8 {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
        // SAFETY: caller supplies a valid allocation layout.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: AllocationLayout) {
        // SAFETY: caller supplies the live pointer and original layout.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: AllocationLayout, size: usize) -> *mut u8 {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
        // SAFETY: caller supplies a live allocation and valid new size.
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn alloc_zeroed(&self, layout: AllocationLayout) -> *mut u8 {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
        // SAFETY: caller supplies a valid allocation layout.
        unsafe { System.alloc_zeroed(layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
const WHOLE: PixelRect = PixelRect {
    left: 0,
    top: 0,
    right: 64,
    bottom: 48,
};
fn renderer(flipping: bool) -> Renderer {
    if flipping {
        Renderer::offscreen_flipping((64, 48))
    } else {
        Renderer::offscreen((64, 48))
    }
    .unwrap()
}
fn scene(shade: u8) -> Scene {
    Scene {
        quads: vec![
            Quad {
                rect: [2.0, 2.0, 8.0, 8.0],
                glyph: [0; 4],
                color: [shade, 0, 0, 255],
                radius: 0.0,
            },
            Quad {
                rect: [50.0, 32.0, 8.0, 8.0],
                glyph: [0; 4],
                color: [0, 0, 255, 255],
                radius: 0.0,
            },
        ],
        batches: vec![Batch {
            first: 0,
            count: 2,
            clip: WHOLE,
            image: 0,
            transform: 0,
            filter: 0,
            round: 0,
        }],
        ..Default::default()
    }
}
fn paint(painter: &mut Painter, text: &mut TextEngine, root: &Node) {
    let mut layout = Layout::new();
    layout.compute(root, Size::new(64.0, 48.0), text);
    painter.paint(root, &layout, text, 1.0, (64, 48));
}
fn label(value: &str) -> Node {
    Element::new(Role::Window)
        .child(Element::new(Role::Text).name(value))
        .into_node()
}

#[test]
fn exports_pending_and_storage_do_not_consume_resources() {
    let mut painter = Painter::new();
    let mut text = TextEngine::new(support::plain_font());
    paint(&mut painter, &mut text, &label("AB"));
    let first = painter.scene_work();
    assert_eq!(
        (
            first.paint_passes,
            first.node_enters,
            first.ancestor_passes,
            first.paragraph_callbacks
        ),
        (1, 2, 0, 0)
    );
    assert_eq!(
        (
            first.output.quads,
            first.output.batches,
            first.quads_built,
            first.batches_built
        ),
        (2, 1, 2, 1)
    );
    assert_eq!(first.glyph_exports.count, 2);
    assert_eq!(first.pending_glyphs, first.glyph_exports);
    paint(&mut painter, &mut text, &label("ABC"));
    let second = painter.scene_work();
    assert_eq!(second.glyph_exports.count, 1);
    assert_eq!(second.pending_glyphs.count, 3);
    assert_eq!(
        second.pending_glyphs.bytes,
        first.glyph_exports.bytes + second.glyph_exports.bytes
    );
    let before = painter.last_scene().clone();
    let census = painter.retained_storage();
    assert_eq!(census.glyph_payload.len_bytes, second.pending_glyphs.bytes);
    assert_eq!(census.scene.quads.len_bytes, 3 * size_of::<Quad>() as u64);
    assert_eq!(*painter.last_scene(), before);
    let mut gpu = renderer(false);
    gpu.render(painter.last_scene(), Color::WHITE).unwrap();
    assert_eq!(gpu.draw_work().glyph_upload, second.pending_glyphs);
    assert_eq!(painter.scene_work(), second); // saved output survives vector swapping
    assert_eq!(
        gpu.retained_storage().history.quads.len_bytes,
        census.scene.quads.len_bytes
    );
    assert_eq!(painter.retained_storage().glyph_payload.len_bytes, 0);
}

#[test]
fn three_repetitions_of_three_traces_have_identical_records_and_pixels() {
    for flipping in [false, true] {
        for trace in 0..3 {
            let mut expected = None;
            for _ in 0..3 {
                let mut candidate = renderer(flipping);
                let mut reference = renderer(flipping);
                reference.set_always_redraw(true);
                let mut records = Vec::new();
                for step in 0..100 {
                    let mut input = scene(if step % 4 < 2 { 120 } else { 240 });
                    if trace == 1 {
                        input.quads[1].color[2] = if step % 6 < 3 { 120 } else { 240 };
                    }
                    if trace == 2 && step % 8 >= 4 {
                        input.batches[0].clip.right = 60;
                    }
                    let mut full = input.clone();
                    candidate.render(&mut input, Color::WHITE).unwrap();
                    reference.render(&mut full, Color::WHITE).unwrap();
                    assert_eq!(
                        candidate.read_pixels().unwrap(),
                        reference.read_pixels().unwrap(),
                        "trace {trace}, step {step}, flipping {flipping}"
                    );
                    let mut work = candidate.draw_work();
                    assert_eq!(work.serial, step + 1);
                    assert_eq!((work.input.quads, work.input.batches), (2, 1));
                    if step == 0 {
                        assert_eq!(work.outcome, DrawOutcome::Full);
                        assert_eq!((work.draw_calls, work.submitted_quads), (1, 2));
                        assert_eq!(
                            work.quad_upload,
                            Payload {
                                count: 1,
                                bytes: 2 * size_of::<Quad>() as u64
                            }
                        );
                        assert_eq!(work.presentation, Presentation::Full);
                    }
                    if work.outcome == DrawOutcome::Unchanged {
                        assert_eq!(work.quad_upload, Payload::default());
                        assert_eq!(work.constant_upload, Payload::default());
                        assert_eq!(
                            (
                                work.draw_calls,
                                work.submitted_quads,
                                work.partial_quads_tested
                            ),
                            (0, 0, 0)
                        );
                        assert_eq!(work.presentation, Presentation::None);
                    } else if work.outcome == DrawOutcome::Partial {
                        assert_eq!(work.partial_quads_tested, 2);
                        assert_eq!(work.draw_calls, 1);
                        assert_eq!(
                            work.quad_upload.bytes,
                            work.submitted_quads * size_of::<Quad>() as u64
                        );
                        assert!(area(work.draw_region) >= area(work.change));
                        if !flipping && trace == 0 {
                            assert_eq!(work.submitted_quads, 1);
                        }
                    }
                    work.serial = 0;
                    records.push(work);
                }
                if let Some(previous) = &expected {
                    assert_eq!(&records, previous);
                } else {
                    expected = Some(records);
                }
            }
        }
    }
}

fn image(id: u32, value: u8) -> ImageUpload {
    ImageUpload {
        id,
        width: 1,
        height: 1,
        pixels: vec![value, 0, 0, 255],
        changes: None,
    }
}
#[test]
fn resource_mutations_rejections_and_missing_images_match_full_reference() {
    for flipping in [false, true] {
        let mut candidate = renderer(flipping);
        let mut reference = renderer(flipping);
        reference.set_always_redraw(true);
        for step in 0..8 {
            let mut input = scene(0);
            input.quads.truncate(1);
            input.quads[0].glyph = [0, 0, 65535, 65535];
            input.batches[0].count = 1;
            input.batches[0].image = if step >= 6 { 8 } else { 7 };
            match step {
                0 => input.image_uploads.push(image(7, 120)),
                1 => input.image_uploads.push(image(7, 240)),
                2 => input.image_uploads.push(image(99, 120)),
                3 => input.image_uploads.push(ImageUpload {
                    width: 0,
                    pixels: vec![],
                    ..image(7, 0)
                }),
                4 => input.dropped_images.push(7),
                5 => input.image_uploads.push(image(7, 120)),
                6 => input.image_uploads.push(image(8, 240)),
                _ => {}
            }
            let mut full = input.clone();
            candidate.render(&mut input, Color::WHITE).unwrap();
            reference.render(&mut full, Color::WHITE).unwrap();
            assert_eq!(
                candidate.read_pixels().unwrap(),
                reference.read_pixels().unwrap()
            );
            let work = candidate.draw_work();
            assert_eq!(
                work.image_upload.count,
                u64::from(matches!(step, 0 | 1 | 2 | 5 | 6))
            );
            assert_eq!(work.image_upload.bytes, work.image_upload.count * 4);
            assert_eq!(
                work.texture_creations,
                work.image_upload.count + u64::from(step == 0)
            );
            assert_eq!(
                work.full_reasons.referenced_image_changed,
                matches!(step, 1 | 4 | 5)
            );
            if matches!(step, 2 | 3 | 7) {
                assert_eq!(work.outcome, DrawOutcome::Unchanged);
            }
            if step == 4 {
                assert_eq!((work.draw_calls, work.submitted_quads), (0, 0));
            }
            if step == 6 {
                assert_eq!(
                    work.outcome,
                    if flipping {
                        DrawOutcome::Full
                    } else {
                        DrawOutcome::Partial
                    }
                );
                assert!(area(work.change) < area(WHOLE));
            }
        }
    }
}

#[test]
fn glyph_growth_text_settings_and_constant_payloads() {
    for flipping in [false, true] {
        let mut candidate = renderer(flipping);
        let mut reference = renderer(flipping);
        reference.set_always_redraw(true);
        for step in 0..4 {
            let mut input = scene(120);
            input.quads.truncate(1);
            input.quads[0].glyph = [1, 1, 1, 1];
            input.batches[0].count = 1;
            input.atlas_size = if step == 0 { 8 } else { 16 };
            if step == 0 {
                input.uploads.push(Upload {
                    x: 1,
                    y: 1,
                    width: 1,
                    height: 1,
                    coverage: vec![128],
                });
            }
            if step >= 2 {
                input.text_look = [1.8, 0.5];
            }
            let mut full = input.clone();
            candidate.render(&mut input, Color::WHITE).unwrap();
            reference.render(&mut full, Color::WHITE).unwrap();
            assert_eq!(
                candidate.read_pixels().unwrap(),
                reference.read_pixels().unwrap()
            );
            let work = candidate.draw_work();
            assert_eq!(work.glyph_upload.count, u64::from(step == 0));
            assert_eq!(work.glyph_upload.bytes, u64::from(step == 0));
            assert_eq!(
                work.atlas_copy,
                if step == 1 {
                    Payload {
                        count: 1,
                        bytes: 64,
                    }
                } else {
                    Payload::default()
                }
            );
            assert_eq!(work.texture_creations, u64::from(step < 2));
            assert_eq!(work.full_reasons.text_changed, step == 2);
            if step == 1 {
                assert_eq!(work.outcome, DrawOutcome::Unchanged);
            }
            if step == 2 {
                assert_eq!(
                    work.constant_upload,
                    Payload {
                        count: 1,
                        bytes: 256
                    }
                );
            }
        }
    }
}

#[test]
fn getters_and_census_allocate_nothing_and_preserve_history() {
    let mut painter = Painter::new();
    let mut text = TextEngine::new(support::plain_font());
    paint(&mut painter, &mut text, &label("ABC"));
    let mut gpu = renderer(false);
    gpu.render(painter.last_scene(), Color::WHITE).unwrap();
    let before = (
        painter.scene_work(),
        painter.retained_storage(),
        gpu.draw_work(),
        gpu.retained_storage(),
    );
    let allocations = ALLOCATIONS.with(Cell::get);
    for _ in 0..100 {
        std::hint::black_box((
            painter.scene_work(),
            painter.retained_storage(),
            gpu.draw_work(),
            gpu.retained_storage(),
        ));
    }
    assert_eq!(ALLOCATIONS.with(Cell::get) - allocations, 0);
    assert_eq!(
        before,
        (
            painter.scene_work(),
            painter.retained_storage(),
            gpu.draw_work(),
            gpu.retained_storage()
        )
    );
    paint(&mut painter, &mut text, &label("ABC"));
    gpu.render(painter.last_scene(), Color::WHITE).unwrap();
    assert_eq!(gpu.draw_work().outcome, DrawOutcome::Unchanged);
}

#[test]
fn warm_resource_free_draw_updates_allocate_nothing() {
    let mut gpu = renderer(false);
    for _ in 0..3 {
        let mut input = scene(120);
        gpu.render(&mut input, Color::WHITE).unwrap();
    }
    // Restore same commands outside measured region. Swaps then retain equal lists.
    let mut input = scene(120);
    let allocations = ALLOCATIONS.with(Cell::get);
    for _ in 0..100 {
        gpu.draw(&mut input, Color::WHITE).unwrap();
    }
    assert_eq!(ALLOCATIONS.with(Cell::get) - allocations, 0);
    assert_eq!(gpu.draw_work().outcome, DrawOutcome::Unchanged);
}

#[test]
fn fixed_record_sizes_and_send_invariant() {
    fn send<T: Send>() {}
    send::<Renderer>();
    println!(
        "SceneWork={} DrawWork={} PainterStorage={} RendererStorage={} Painter={} Renderer={}",
        size_of::<SceneWork>(),
        size_of::<DrawWork>(),
        size_of::<PainterStorage>(),
        size_of::<RendererStorage>(),
        size_of::<Painter>(),
        size_of::<Renderer>()
    );
    let root = Element::new(Role::Window)
        .child(Element::new(Role::Group).props(rsui_layout::style!(
            Style::new().height(10.0),
            Paint::new().background(Color::WHITE)
        )))
        .into_node();
    let mut painter = Painter::new();
    let mut text = TextEngine::new(support::plain_font());
    paint(&mut painter, &mut text, &root);
    assert_eq!(painter.scene_work().output.quads, 1);
}

#[test]
fn paragraph_and_lifted_ancestor_callbacks_are_separate() {
    let root = Element::new(Role::Window)
        .child(Element::new(Role::Text).child(Element::new(Role::Text).name("AB")))
        .child(
            Element::new(Role::Group)
                .prop("position", "absolute")
                .name("C"),
        )
        .into_node();
    let mut painter = Painter::new();
    let mut text = TextEngine::new(support::plain_font());
    paint(&mut painter, &mut text, &root);
    let work = painter.scene_work();
    assert_eq!(
        (
            work.node_enters,
            work.paragraph_callbacks,
            work.ancestor_passes
        ),
        (3, 1, 1)
    );
}

#[test]
fn actual_caret_reuse_matches_independent_full_painter() {
    use rsui_render::{Editing, Interaction};
    for flipping in [false, true] {
        let root = Element::new(Role::Window)
            .child(Element::new(Role::TextInput).prop("value", "AB"))
            .into_node();
        let mut painter = Painter::new();
        let mut full_painter = Painter::new();
        let mut text = TextEngine::new(support::plain_font());
        let mut full_text = TextEngine::new(support::plain_font());
        let mut layout = Layout::new();
        let mut full_layout = Layout::new();
        layout.compute(&root, Size::new(64.0, 48.0), &mut text);
        full_layout.compute(&root, Size::new(64.0, 48.0), &mut full_text);
        let mut candidate = renderer(flipping);
        let mut reference = renderer(flipping);
        reference.set_always_redraw(true);
        for (step, show) in [true, false, true, false].into_iter().enumerate() {
            let interaction = Interaction {
                editing: Some(Editing {
                    element: &root.children[0],
                    selection: &[],
                    caret: show.then_some(8.0),
                    scroll: 0.0,
                    lines: None,
                }),
                ..Default::default()
            };
            if step == 0 {
                painter.keep_next_for_blinking();
                painter.paint_with(&root, &layout, &mut text, 1.0, (64, 48), interaction);
            } else {
                painter
                    .show_caret(show)
                    .expect("actual caret scene was kept");
                assert_eq!(painter.scene_work().node_enters, 0);
                assert!(painter.scene_work().caret_copy_bytes > 0);
            }
            full_painter.paint_with(
                &root,
                &full_layout,
                &mut full_text,
                1.0,
                (64, 48),
                interaction,
            );
            candidate
                .render(painter.last_scene(), Color::WHITE)
                .unwrap();
            reference
                .render(full_painter.last_scene(), Color::WHITE)
                .unwrap();
            assert_eq!(
                candidate.read_pixels().unwrap(),
                reference.read_pixels().unwrap()
            );
        }
    }
}

#[test]
fn grown_quad_buffer_and_per_batch_constants_count_actual_work() {
    let mut gpu = renderer(false);
    let mut input = scene(120);
    input.quads.resize(2050, input.quads[0]);
    input.batches[0].count = 2050;
    input.batches[0].transform = 1;
    input.transforms.push([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let mut reference = renderer(false);
    reference.set_always_redraw(true);
    reference.render(&mut input.clone(), Color::WHITE).unwrap();
    gpu.render(&mut input, Color::WHITE).unwrap();
    assert_eq!(gpu.read_pixels().unwrap(), reference.read_pixels().unwrap());
    let work = gpu.draw_work();
    assert_eq!(work.buffer_creations, 1);
    assert_eq!(
        work.constant_upload,
        Payload {
            count: 2,
            bytes: 512
        }
    );
    assert_eq!(
        work.quad_upload,
        Payload {
            count: 1,
            bytes: 2050 * size_of::<Quad>() as u64
        }
    );
    assert_eq!((work.submitted_quads, work.draw_calls), (2050, 1));
    assert_eq!(
        gpu.retained_storage().quad_buffer_capacity_bytes,
        4096 * size_of::<Quad>() as u64
    );
    assert!(work.full_reasons.transformed);
}
