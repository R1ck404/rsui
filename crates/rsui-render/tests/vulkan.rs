//! What Vulkan draws, against what the CPU renderer draws, for the same scenes. Both are ports of `quad.hlsl`, so they agree
//! to within a little rounding where a GPU's arithmetic differs from the CPU's: at smoothed edges, and where images are
//! read between their pixels. These run on whatever Vulkan there is, a software one (Mesa's lavapipe) included, and pass,
//! saying so, where there's none. With `RSUI_VULKAN_VALIDATE=1` and the validation layers installed, they also check that
//! Vulkan was used without a single error.
#![cfg(any(target_os = "linux", target_os = "android"))]

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use rsui_core::{Element, Node, Props, Role};
use rsui_layout::{Layout, Size, Style};
use rsui_render::{Batch, Color, Filter, ImageUpload, Layer, Paint, Painter, PixelRect, Quad, Renderer, RoundClip, Scene};
use rsui_text::{TextEngine, Upload};

const SIZE: (u32, u32) = (160, 120);
const WHOLE: PixelRect = PixelRect { left: 0, top: 0, right: SIZE.0 as i32, bottom: SIZE.1 as i32 };
const INSET_MARK: u16 = 65534;

/// A Vulkan renderer drawing into memory, or nothing if this machine has no Vulkan.
fn vulkan(size: (u32, u32)) -> Option<Renderer> {
    match Renderer::offscreen_vulkan(size) {
        Ok(renderer) => Some(renderer),
        Err(error) => {
            eprintln!("no Vulkan to test ({error}); skipped");
            None
        }
    }
}

fn batch(first: u32, count: u32) -> Batch {
    Batch { first, count, clip: WHOLE, image: 0, transform: 0, filter: 0, round: 0 }
}

fn rect(rect: [f32; 4], color: [u8; 4], radius: f32) -> Quad {
    Quad { rect, glyph: [0; 4], color, radius }
}

/// Coverage that goes smoothly from nothing to everything and back, as an antialiased glyph's edges do.
fn coverage(width: u32, height: u32, seed: u32) -> Vec<u8> {
    (0..width * height)
        .map(|at| {
            let (x, y) = (at % width, at / width);
            ((x * 37 + y * 23 + seed * 11) % 256) as u8
        })
        .collect()
}

/// Every kind of quad the shaders draw, in batches that use every kind of constant, with a layer, a round clip and an image.
fn everything() -> Scene {
    let mut quads = vec![
        // Plain rectangles: opaque, see-through, and on fractions of pixels.
        rect([0.0, 0.0, 160.0, 120.0], [240, 240, 235, 255], 0.0),
        rect([4.0, 4.0, 30.0, 20.0], [200, 30, 30, 255], 0.0),
        rect([10.3, 12.6, 30.4, 17.2], [0, 0, 100, 128], 0.0),
        // Round corners, and an outline with round corners, two and a half pixels wide.
        rect([40.0, 4.0, 36.0, 24.0], [20, 120, 40, 255], 7.5),
        Quad { rect: [80.0, 4.0, 36.0, 24.0], glyph: [40, 0, 0, 0], color: [10, 10, 10, 255], radius: 5.0 },
        // Soft shadows, square and round: 8 pixels of blur, the rectangle reaching that far out on each side.
        Quad { rect: [4.0, 30.0, 40.0, 30.0], glyph: [0, 8 * 16, 0, 0], color: [0, 0, 0, 160], radius: 0.0 },
        Quad { rect: [48.0, 30.0, 40.0, 30.0], glyph: [0, 8 * 16, 0, 0], color: [0, 0, 60, 200], radius: 10.0 },
        // A shadow inside a box: a spread of 2, 6 of blur, moved 3 right and 2 down.
        Quad {
            rect: [92.0, 30.0, 40.0, 30.0],
            glyph: [2 * 16, 6 * 16, INSET_MARK, ((128 + 3) << 8) | (128 + 2)],
            color: [80, 0, 0, 200],
            radius: 6.0,
        },
    ];
    // Glyphs, from the atlas, in two colors.
    quads.push(Quad { rect: [4.0, 64.0, 16.0, 12.0], glyph: [0, 0, 16, 12], color: [20, 20, 20, 255], radius: 0.0 });
    quads.push(Quad { rect: [22.0, 64.0, 16.0, 12.0], glyph: [16, 0, 16, 12], color: [200, 200, 255, 255], radius: 0.0 });
    let glyphs_end = quads.len() as u32;
    // Turned 30 degrees: a round rectangle, and a glyph, read smoothly from the atlas.
    quads.push(rect([0.0, 0.0, 24.0, 14.0], [200, 100, 0, 255], 4.0));
    quads.push(Quad { rect: [0.0, 18.0, 16.0, 12.0], glyph: [0, 0, 16, 12], color: [0, 0, 0, 255], radius: 0.0 });
    let turned_end = quads.len() as u32;
    // Through a filter that greys and fades.
    quads.push(rect([100.0, 64.0, 24.0, 20.0], [200, 40, 40, 255], 3.0));
    let filtered_end = quads.len() as u32;
    // Cut off at round corners.
    quads.push(rect([126.0, 62.0, 34.0, 30.0], [30, 30, 160, 255], 0.0));
    let clipped_end = quads.len() as u32;
    // An image, made bigger, with round corners.
    quads.push(Quad { rect: [44.0, 64.0, 24.0, 24.0], glyph: [0, 0, 65535, 65535], color: [255; 4], radius: 4.0 });
    let image_end = quads.len() as u32;
    // Faded as a whole: two boxes, one over the other.
    quads.push(rect([70.0, 90.0, 40.0, 26.0], [0, 0, 0, 255], 0.0));
    quads.push(rect([80.0, 96.0, 20.0, 14.0], [255, 255, 255, 255], 6.0));
    let faded_end = quads.len() as u32;
    // Cut to a clip of its own.
    quads.push(rect([4.0, 90.0, 60.0, 26.0], [120, 0, 120, 255], 13.0));
    let end = quads.len() as u32;

    let angle = 30f32.to_radians();
    let mut batches = vec![
        batch(0, glyphs_end),
        Batch { transform: 1, ..batch(glyphs_end, turned_end - glyphs_end) },
        Batch { filter: 1, ..batch(turned_end, filtered_end - turned_end) },
        Batch { round: 1, ..batch(filtered_end, clipped_end - filtered_end) },
        Batch { image: 7, ..batch(clipped_end, image_end - clipped_end) },
        batch(image_end, faded_end - image_end),
        Batch { clip: PixelRect { left: 10, top: 95, right: 50, bottom: 110 }, ..batch(faded_end, end - faded_end) },
    ];
    batches[1].clip = PixelRect { left: 60, top: 40, right: 140, bottom: 120 };
    let image_pixels: Vec<u8> = (0..8 * 8)
        .flat_map(|at| {
            let alpha = if at % 5 == 0 { 128u8 } else { 255 };
            let premultiply = |part: u32| (part * u32::from(alpha) / 255) as u8;
            [premultiply(at * 4 % 256), premultiply(255 - at * 3 % 256), premultiply(at * 29 % 256), alpha]
        })
        .collect();
    Scene {
        quads,
        batches,
        text_look: [1.2, 0.5],
        uploads: vec![
            Upload { x: 0, y: 0, width: 16, height: 12, coverage: coverage(16, 12, 1) },
            Upload { x: 16, y: 0, width: 16, height: 12, coverage: coverage(16, 12, 2) },
        ],
        atlas_size: 64,
        image_uploads: vec![ImageUpload { id: 7, width: 8, height: 8, pixels: image_pixels, changes: None }],
        transforms: vec![[angle.cos(), angle.sin(), -angle.sin(), angle.cos(), 80.0, 40.0]],
        filters: vec![Filter { rows: [0.3, 0.6, 0.1, 0.0, 0.3, 0.6, 0.1, 0.0, 0.3, 0.6, 0.1, 0.05], opacity: 0.8 }],
        round_clips: vec![RoundClip {
            edges: [128.0, 64.0, 156.0, 90.0],
            corners: [[10.0, 10.0], [6.0, 12.0], [10.0, 10.0], [0.0, 0.0]],
            outside: false,
            transform: 0,
            outer: 0,
        }],
        layers: vec![Layer { first: 5, end: 6, opacity: 0.6, bounds: PixelRect { left: 69, top: 89, right: 111, bottom: 117 } }],
        ..Scene::default()
    }
}

/// How two pictures differ: the most any channel of any pixel does, and how many pixels differ by more than `close`.
fn difference(cpu: &[u8], gpu: &[u8], close: u8) -> (u8, usize, Option<(usize, [u8; 4], [u8; 4])>) {
    assert_eq!(cpu.len(), gpu.len(), "the pictures are the same size");
    let (mut most, mut far, mut worst) = (0u8, 0usize, None);
    for (at, (a, b)) in cpu.chunks_exact(4).zip(gpu.chunks_exact(4)).enumerate() {
        let apart = a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        if apart > close {
            far += 1;
        }
        if apart > most {
            most = apart;
            worst = Some((at, [a[0], a[1], a[2], a[3]], [b[0], b[1], b[2], b[3]]));
        }
    }
    (most, far, worst)
}

/// Draws the scenes `make` makes, one frame after another, with both renderers, and checks the last frames agree: no
/// channel more than `most` apart, and no more than `far` pixels more than 2 apart.
fn compare(label: &str, size: (u32, u32), frames: usize, make: impl Fn(usize) -> (Scene, Color), most: u8, far: usize) {
    let Some(mut gpu) = vulkan(size) else { return };
    let mut cpu = Renderer::offscreen(size).expect("memory");
    for frame in 0..frames {
        let (mut scene, clear) = make(frame);
        cpu.render(&mut scene, clear).expect("the CPU draws");
        let (mut scene, clear) = make(frame);
        gpu.render(&mut scene, clear).expect("Vulkan draws");
    }
    let (cpu, gpu) = (cpu.read_pixels().expect("read"), gpu.read_pixels().expect("read back"));
    let (apart, how_many, worst) = difference(&cpu, &gpu, 2);
    let worst =
        worst.map(|(at, a, b)| format!("worst at ({}, {}): cpu {a:?}, vulkan {b:?}", at as u32 % size.0, at as u32 / size.0));
    eprintln!("{label}: at most {apart} apart, {how_many} pixels more than 2 apart; {}", worst.as_deref().unwrap_or("none"));
    assert!(apart <= most && how_many <= far, "{label}: {apart} apart at most, {how_many} pixels; {worst:?}");
    assert_eq!(Renderer::vulkan_validation_errors(), 0, "{label}: Vulkan's validation layers reported errors");
}

#[test]
fn every_kind_of_quad_draws_as_the_cpu_draws_it() {
    compare("everything", SIZE, 1, |_| (everything(), Color::WHITE), 6, 20);
}

#[test]
fn a_see_through_clear_and_nothing_else_is_the_same() {
    compare("clear", (33, 17), 1, |_| (Scene::default(), Color::rgba(10, 200, 30, 100)), 1, 0);
}

#[test]
fn the_atlas_keeps_its_glyphs_when_it_grows_and_more_quads_than_fit_at_first_draw() {
    compare(
        "growing",
        SIZE,
        2,
        |frame| {
            // Many small quads, more than the instance buffer starts with, under two glyphs: one put in the atlas when it
            // was small, the other once it's grown.
            let mut quads: Vec<Quad> = (0..3000)
                .map(|at| rect([(at % 80) as f32 * 2.0, (at / 80) as f32 * 3.0, 2.0, 3.0], [(at % 251) as u8, 90, 160, 255], 0.0))
                .collect();
            quads.push(Quad { rect: [10.0, 10.0, 16.0, 16.0], glyph: [0, 0, 16, 16], color: [0, 0, 0, 255], radius: 0.0 });
            quads.push(Quad {
                rect: [40.0, 10.0, 16.0, 16.0],
                glyph: [40, 40, 16, 16],
                color: [255, 255, 255, 255],
                radius: 0.0,
            });
            let count = quads.len() as u32;
            let uploads = if frame == 0 {
                vec![Upload { x: 0, y: 0, width: 16, height: 16, coverage: coverage(16, 16, 3) }]
            } else {
                vec![Upload { x: 40, y: 40, width: 16, height: 16, coverage: coverage(16, 16, 4) }]
            };
            let scene = Scene {
                quads,
                batches: vec![batch(0, count)],
                uploads,
                atlas_size: if frame == 0 { 32 } else { 64 },
                ..Scene::default()
            };
            (scene, Color::WHITE)
        },
        0,
        0,
    );
}

#[test]
fn an_image_that_changes_changes_where_it_changed() {
    compare(
        "canvas",
        (40, 40),
        2,
        |frame| {
            let pixels = if frame == 0 { [255u8, 0, 0, 255].repeat(16 * 16) } else { [0u8, 0, 255, 255].repeat(4 * 4) };
            let upload = ImageUpload {
                id: 3,
                width: if frame == 0 { 16 } else { 4 },
                height: if frame == 0 { 16 } else { 4 },
                pixels,
                changes: Some(if frame == 0 { (0, 0) } else { (6, 6) }),
            };
            // Drawn one image pixel to two window pixels, and read between them.
            let scene = Scene {
                quads: vec![Quad { rect: [4.0, 4.0, 32.0, 32.0], glyph: [0, 0, 65535, 65535], color: [255; 4], radius: 0.0 }],
                batches: vec![Batch { image: 3, ..batch(0, 1) }],
                image_uploads: vec![upload],
                atlas_size: 1,
                ..Scene::default()
            };
            (scene, Color::WHITE)
        },
        4,
        10,
    );
}

/// A painter's own scene for a tree, as an app draws it: backgrounds, borders, round corners, shadows, text and what's faded.
fn painted(root: &Node) -> Scene {
    let mut text = TextEngine::new(support::plain_font());
    let mut layout = Layout::new();
    layout.compute(root, Size::new(SIZE.0 as f32, SIZE.1 as f32), &mut text);
    let mut painter = Painter::new();
    std::mem::take(painter.paint(root, &layout, &mut text, 1.0, SIZE))
}

#[test]
fn a_painted_tree_draws_as_the_cpu_draws_it() {
    let props = |style: Style, paint: Paint| Props::from(style).merged(&Props::from(paint));
    let root = Element::new(Role::Window)
        .props(props(Style::new().padding_xy(8.0, 8.0), Paint::new().background(Color::hex(0xF4F4F0))))
        .child(
            Element::new(Role::Group)
                .props(props(
                    Style::new().padding_xy(10.0, 6.0),
                    Paint::new().background(Color::hex(0x1F6FEB)).border(Color::hex(0x0B3D91), 2.0).corner_radius(8.0).shadow(
                        Color::rgba(0, 0, 0, 120),
                        10.0,
                        3.0,
                    ),
                ))
                .child(
                    Element::new(Role::Text)
                        .name("AB CD")
                        .props(props(Style::new(), Paint::new().text(Color::WHITE)))
                        .prop("font_size", 14.0),
                ),
        )
        .child(
            Element::new(Role::Group)
                .props(props(
                    Style::new().padding_xy(6.0, 6.0),
                    Paint::new().background(Color::hex(0xFFFFFF)).corner_radius(4.0).opacity(0.5),
                ))
                .child(
                    Element::new(Role::Text)
                        .name("ABBA")
                        .props(props(Style::new(), Paint::new().text(Color::hex(0x202020))))
                        .prop("font_size", 12.0),
                ),
        )
        .into_node();
    compare("painted", SIZE, 1, |_| (painted(&root), Color::WHITE), 4, 10);
}

#[test]
fn the_same_scene_twice_draws_nothing_the_second_time() {
    let Some(mut gpu) = vulkan(SIZE) else { return };
    gpu.render(&mut everything(), Color::WHITE).expect("Vulkan draws");
    let first = gpu.read_pixels().expect("read back");
    assert!(!gpu.showed_nothing());
    let mut again = everything();
    // What was uploaded is there already.
    again.uploads.clear();
    again.image_uploads.clear();
    gpu.render(&mut again, Color::WHITE).expect("Vulkan draws");
    assert!(gpu.showed_nothing());
    assert_eq!(first, gpu.read_pixels().expect("read back"));
    // A different size draws again.
    gpu.resize((80, 60)).expect("resize");
    let mut small = everything();
    small.uploads.clear();
    small.image_uploads.clear();
    gpu.render(&mut small, Color::WHITE).expect("Vulkan draws");
    assert!(!gpu.showed_nothing());
    assert_eq!(gpu.read_pixels().expect("read back").len(), 80 * 60 * 4);
    gpu.trim();
    let mut after = everything();
    after.batches[0].count -= 1;
    gpu.render(&mut after, Color::WHITE).expect("Vulkan draws after trimming");
}
