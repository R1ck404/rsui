//! Strokes no wider than a pixel, which a browser draws as Skia does: a pixel across, as faint as they're thin, so that a slanted
//! or curved one covers less than its width would. Wider ones cover what their width does. These draw on Windows' software GPU and
//! read the result back, so no window opens.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use rsui_core::{Element, Role};
use rsui_layout::{Layout, Size};
use rsui_render::{Color, LineCap, Painter, Renderer, shape_layer};
use rsui_text::TextEngine;
use support::plain_font;

const WINDOW: (u32, u32) = (64, 48);

/// What a black stroke this wide along `d` draws on white, in a shape as big as the window, as how dark each pixel is, from 0 for
/// white to 1 for black, row by row.
fn darkness(d: &str, width: f32, cap: LineCap) -> Vec<f32> {
    let layers = shape_layer(d, None, Some((Some(Color::BLACK), width)), cap);
    let (across, down) = (WINDOW.0 as f32, WINDOW.1 as f32);
    let root = Element::new(Role::Window)
        .child(
            Element::new(Role::Image)
                .prop("width", f64::from(across))
                .prop("height", f64::from(down))
                .prop("shape_layers", layers)
                .prop("view_box", format!("0 0 {across} {down}")),
        )
        .into_node();
    let mut text = TextEngine::new(plain_font());
    let mut layout = Layout::new();
    layout.compute(&root, Size::new(across, down), &mut text);
    let mut painter = Painter::new();
    let scene = painter.paint(&root, &layout, &mut text, 1.0, WINDOW);
    let mut renderer = Renderer::offscreen(WINDOW).unwrap();
    renderer.render(scene, Color::WHITE).unwrap();
    let pixels = renderer.read_pixels().unwrap();
    pixels
        .chunks_exact(4)
        .map(|pixel| 1.0 - f32::from(pixel[1]) / 255.0)
        .collect()
}

fn total(darkness: &[f32]) -> f32 {
    darkness.iter().sum()
}

fn column(darkness: &[f32], x: u32) -> f32 {
    (0..WINDOW.1)
        .map(|y| darkness[(y * WINDOW.0 + x) as usize])
        .sum()
}

fn near(found: f32, expected: f32, within: f32) -> bool {
    (found - expected).abs() <= within * expected
}

#[test]
fn a_slanted_stroke_no_wider_than_a_pixel_is_a_pixel_high_and_as_faint_as_it_is_thin() {
    // Twice as far across as down, half a pixel wide: each column it crosses is half covered, so 40 columns are 20 pixels' worth,
    // where its width along its length would be 22.4.
    let drawn = darkness("M10 10 L50 30", 0.5, LineCap::Butt);
    assert!(near(total(&drawn), 20.0, 0.02), "{}", total(&drawn));
    for x in [12, 20, 31, 47] {
        assert!(
            near(column(&drawn, x), 0.5, 0.03),
            "column {x}: {}",
            column(&drawn, x)
        );
    }
    // Going more down than across, it's a pixel across instead.
    let drawn = darkness("M10 5 L20 45", 0.25, LineCap::Butt);
    assert!(near(total(&drawn), 10.0, 0.02), "{}", total(&drawn));
}

#[test]
fn a_stroke_a_pixel_wide_is_drawn_that_way_too_and_a_wider_one_covers_its_width() {
    // A ring 14 across, a pixel wide, covers its length once its slanted parts are a pixel high or across, rather than a pixel at
    // right angles to them: on average, nine tenths of it.
    let ring = "M46 24 A14 14 0 1 1 18 24 A14 14 0 1 1 46 24 Z";
    let length = std::f32::consts::TAU * 14.0;
    let drawn = darkness(ring, 1.0, LineCap::Butt);
    assert!(
        near(total(&drawn), length * 0.9, 0.03),
        "{} of {length}",
        total(&drawn)
    );
    // A pixel and a half wide, it's all of that.
    let drawn = darkness(ring, 1.5, LineCap::Butt);
    assert!(
        near(total(&drawn), length * 1.5, 0.02),
        "{} of {}",
        total(&drawn),
        length * 1.5
    );
    let drawn = darkness("M10 10 L50 30", 1.5, LineCap::Butt);
    let length = 40f32.hypot(20.0);
    assert!(
        near(total(&drawn), length * 1.5, 0.02),
        "{} of {}",
        total(&drawn),
        length * 1.5
    );
}

#[test]
fn a_thin_stroke_level_with_the_pixels_is_as_it_was() {
    // Half a pixel wide, through the middle of a row, and along the line between two.
    let drawn = darkness("M10 10.5 H50", 0.5, LineCap::Butt);
    let at = |x: u32, y: u32| drawn[(y * WINDOW.0 + x) as usize];
    assert!(near(at(30, 10), 0.5, 0.03), "{}", at(30, 10));
    assert_eq!((at(30, 9), at(30, 11)), (0.0, 0.0));
    let drawn = darkness("M10 20 H50", 0.5, LineCap::Butt);
    let at = |x: u32, y: u32| drawn[(y * WINDOW.0 + x) as usize];
    assert!(
        near(at(30, 19), 0.25, 0.05) && near(at(30, 20), 0.25, 0.05),
        "{} {}",
        at(30, 19),
        at(30, 20)
    );
}

#[test]
fn a_thin_stroke_s_caps_carry_it_on_by_what_they_cover() {
    // A square cap is half a pixel past each end, and a round one π / 8 of one.
    let line = "M10.5 10.5 H50.5";
    let butt = total(&darkness(line, 1.0, LineCap::Butt));
    let square = total(&darkness(line, 1.0, LineCap::Square));
    let round = total(&darkness(line, 1.0, LineCap::Round));
    assert!(near(butt, 40.0, 0.01), "{butt}");
    assert!(near(square, 41.0, 0.01), "{square}");
    assert!(
        near(round, 40.0 + std::f32::consts::PI / 4.0, 0.01),
        "{round}"
    );
    // A dot is as long as its caps make it.
    let dot = total(&darkness("M20.5 20.5 L20.5 20.5", 1.0, LineCap::Square));
    assert!(near(dot, 1.0, 0.02), "{dot}");
    assert_eq!(
        total(&darkness("M20.5 20.5 L20.5 20.5", 1.0, LineCap::Butt)),
        0.0
    );
}

#[test]
fn a_thin_stroke_that_goes_back_over_itself_covers_it_once() {
    let there = total(&darkness("M10 10 L50 30", 0.5, LineCap::Butt));
    let back = total(&darkness("M10 10 L50 30 L10 10", 0.5, LineCap::Butt));
    assert!(near(back, there, 0.01), "{back} {there}");
}
