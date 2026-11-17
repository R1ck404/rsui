//! Text drawn on the system's GPU, as a window draws it, against the same scene on Windows' software GPU, which is what
//! headless runs and the other pixel tests use. Light text on a dark background once came out on a real GPU with nothing at
//! all where its letters should be, while the software GPU drew it.

use rsui_render::{Batch, Color, PixelRect, Quad, Renderer, Scene};
use rsui_text::Upload;

const SIZE: (u32, u32) = (64, 32);

/// What a browser's text looks like on Windows: the gamma and contrast its coverage goes through.
const BROWSER_LOOK: [f32; 2] = [1.6, 0.0];

/// A dark rounded box with a light ClearType glyph on it, and a light box with a dark one, as shadcn's primary and outline
/// buttons are.
fn scene(look: [f32; 2], subpixel: bool) -> Scene {
    let clip = PixelRect {
        left: 0,
        top: 0,
        right: SIZE.0 as i32,
        bottom: SIZE.1 as i32,
    };
    // A glyph 8 pixels wide and 10 high: a solid bar with soft edges, in three bytes a pixel for ClearType, one otherwise.
    let parts = if subpixel { 3 } else { 1 };
    let (width, height) = (8 * parts, 10);
    let coverage: Vec<u8> = (0..height)
        .flat_map(|_| {
            (0..width).map(move |x| {
                let edge = x.min(width - 1 - x);
                [40, 120, 200, 255][edge.min(3) as usize]
            })
        })
        .collect();
    let box_quad = |rect, color: Color| Quad {
        rect,
        glyph: [0; 4],
        color: [color.r, color.g, color.b, color.a],
        radius: 6.0,
    };
    let glyph = |x: f32, color: Color| Quad {
        rect: [x, 11.0, 8.0, 10.0],
        glyph: [0, 0, width as u16, height as u16],
        color: [color.r, color.g, color.b, color.a],
        radius: if subpixel { -1.0 } else { 0.0 },
    };
    let quads = vec![
        box_quad([2.0, 4.0, 28.0, 24.0], Color::hex(0x171717)),
        glyph(12.0, Color::hex(0xFAFAFA)),
        box_quad([34.0, 4.0, 28.0, 24.0], Color::hex(0xFFFFFF)),
        glyph(44.0, Color::hex(0x0A0A0A)),
    ];
    let batches = vec![Batch {
        first: 0,
        count: quads.len() as u32,
        clip,
        image: 0,
        transform: 0,
        filter: 0,
    }];
    Scene {
        quads,
        batches,
        text_look: look,
        uploads: vec![Upload {
            x: 0,
            y: 0,
            width,
            height,
            coverage,
        }],
        atlas_size: 64,
        ..Scene::default()
    }
}

fn draw(renderer: &mut Renderer, mut scene: Scene) -> Vec<u8> {
    renderer
        .render(&mut scene, Color::hex(0x808080))
        .expect("render");
    renderer.read_pixels().expect("read")
}

/// How much brighter than the box's own color the middle of the glyph on it is, in red, green and blue together.
fn lift(pixels: &[u8], x: u32, y: u32, behind: u8) -> i32 {
    let at = ((y * SIZE.0 + x) * 4) as usize;
    pixels[at..at + 3]
        .iter()
        .map(|&part| i32::from(part) - i32::from(behind))
        .sum()
}

#[test]
fn light_text_on_a_dark_box_draws_on_the_gpu_as_it_does_in_software() {
    if Renderer::offscreen_on_hardware(SIZE).is_err() {
        eprintln!("no hardware GPU here; skipping");
        return;
    }
    let mut failures = Vec::new();
    for look in [BROWSER_LOOK, [0.0, 0.0]] {
        for subpixel in [true, false] {
            let mut software = Renderer::offscreen(SIZE).expect("WARP renderer");
            let expected = draw(&mut software, scene(look, subpixel));
            let mut on_hardware = Renderer::offscreen_on_hardware(SIZE).expect("hardware renderer");
            let actual = draw(&mut on_hardware, scene(look, subpixel));
            let label = format!("look {look:?}, subpixel {subpixel}");
            // The light glyph's middle is nearly white on the dark box, and the dark one's nearly black on the light one.
            let (light, dark) = (lift(&actual, 16, 16, 0x17), lift(&actual, 48, 16, 0xFF));
            if light < 3 * 200 {
                failures.push(format!("{label}: light glyph lifted its box by only {light}"));
            }
            if dark > -3 * 200 {
                failures.push(format!("{label}: dark glyph darkened its box by only {}", -dark));
            }
            // Hardware rounds differently from WARP in the last bit or two, but nothing more.
            let worst = actual
                .iter()
                .zip(&expected)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            if worst > 3 {
                failures.push(format!("{label}: hardware differs from WARP by up to {worst}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
