//! What's cut off at round corners, as CSS's `overflow: hidden` with `border-radius` has it, and shadows around boxes with
//! round corners. These draw on Windows' software GPU and read the result back, so no window opens.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use rsui_core::{Element, Node, Role};
use rsui_layout::{Layout, Size, Style};
use rsui_render::{Color, Paint, Painter, Renderer, Scene};
use rsui_text::TextEngine;
use support::plain_font;

const WINDOW: (u32, u32) = (200, 120);
const RED: Color = Color::hex(0xFF0000);
const BLUE: Color = Color::hex(0x0000FF);
const WHITE: [u8; 3] = [255, 255, 255];
const RED_PIXEL: [u8; 3] = [255, 0, 0];
const BLUE_PIXEL: [u8; 3] = [0, 0, 255];
/// Where the box being tested starts, across and down.
const AT: f32 = 20.0;

/// A tree's scene, and what it draws, as red, green, blue and opacity for each pixel.
fn drawn(root: &Node) -> (Scene, Vec<u8>) {
    let mut text = TextEngine::new(plain_font());
    let mut layout = Layout::new();
    layout.compute(root, Size::new(WINDOW.0 as f32, WINDOW.1 as f32), &mut text);
    let mut painter = Painter::new();
    let scene = painter.paint(root, &layout, &mut text, 1.0, WINDOW);
    let kept = scene.clone();
    let mut renderer = Renderer::offscreen(WINDOW).unwrap();
    renderer.render(scene, Color::WHITE).unwrap();
    (kept, renderer.read_pixels().unwrap())
}

fn pixel(pixels: &[u8], x: f32, y: f32) -> [u8; 3] {
    let at = ((y as u32 * WINDOW.0 + x as u32) * 4) as usize;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

/// Whether a pixel is some of `color` and some of white, as one a curve goes through is.
fn partly(found: [u8; 3], color: [u8; 3]) -> bool {
    found != color && found != WHITE
}

fn group(style: Style, paint: Paint) -> Element {
    Element::new(Role::Group).props(rsui_core::Props::from(style).merged(&rsui_core::Props::from(paint)))
}

/// `inside` at the top left of the window, `AT` in from its edges.
fn placed(inside: Element) -> Node {
    Element::new(Role::Window)
        .props(Style::new().padding_each(AT, 0.0, 0.0, AT))
        .child(inside)
        .into_node()
}

/// A box 100 wide and 60 high that cuts off what's in it at corners this round, with a red half and a blue half in it.
fn halves(round: Paint) -> Element {
    group(Style::row().width(100.0).height(60.0), round)
        .prop("clip", true)
        .child(group(Style::new().grow(1.0), Paint::new().background(RED)))
        .child(group(Style::new().grow(1.0), Paint::new().background(BLUE)))
}

#[test]
fn what_is_cut_off_by_a_box_with_round_corners_is_cut_off_at_them() {
    let (_, pixels) = drawn(&placed(halves(Paint::new().corner_radius(12.0))));
    let (right, bottom) = (AT + 99.0, AT + 59.0);
    assert_eq!(pixel(&pixels, AT + 30.0, AT + 30.0), RED_PIXEL, "the red half");
    assert_eq!(pixel(&pixels, AT + 70.0, AT + 30.0), BLUE_PIXEL, "the blue half");
    for (x, y) in [(AT, AT), (right, AT), (right, bottom), (AT, bottom)] {
        assert_eq!(pixel(&pixels, x, y), WHITE, "the corner at {x}, {y} is cut off");
    }
    assert_eq!(pixel(&pixels, AT, AT + 30.0), RED_PIXEL, "the middle of an edge isn't");
    assert!(partly(pixel(&pixels, AT + 3.0, AT + 3.0), RED_PIXEL), "the curve is smooth");
    assert!(partly(pixel(&pixels, right - 3.0, bottom - 3.0), BLUE_PIXEL), "the curve is smooth");
}

#[test]
fn corners_that_are_each_as_round_as_they_say_cut_off_as_much() {
    // The top left and bottom right are round, one more across than down, and the others square.
    let (_, pixels) = drawn(&placed(halves(Paint::new()).prop("corner_radii", "24 12 0 0 16 16 0 0")));
    let (right, bottom) = (AT + 99.0, AT + 59.0);
    assert_eq!(pixel(&pixels, AT, AT), WHITE, "the top left is cut off");
    assert_eq!(pixel(&pixels, AT + 10.0, AT), WHITE, "further across than down");
    assert_eq!(pixel(&pixels, AT, AT + 14.0), RED_PIXEL, "and not as far down");
    assert_eq!(pixel(&pixels, right, AT), BLUE_PIXEL, "the top right is square");
    assert_eq!(pixel(&pixels, right, bottom), WHITE, "the bottom right is cut off");
    assert_eq!(pixel(&pixels, AT, bottom), RED_PIXEL, "the bottom left is square");
}

#[test]
fn what_is_in_a_box_with_a_border_is_cut_off_inside_it() {
    // A border 4 wide around corners 12 round leaves corners 8 round inside it, and what's in it doesn't cover the border.
    let border = group(Style::row().width(100.0).height(60.0), Paint::new().corner_radius(12.0).border(BLUE, 4.0))
        .prop("clip", true)
        .child(group(Style::new().grow(1.0), Paint::new().background(RED)));
    let (_, pixels) = drawn(&placed(border));
    assert_eq!(pixel(&pixels, AT + 50.0, AT + 1.0), BLUE_PIXEL, "the border shows over the middle of the top");
    assert_eq!(pixel(&pixels, AT + 50.0, AT + 30.0), RED_PIXEL, "what's in it shows inside it");
    // Inside the border's corner, but outside the curve inside it.
    assert_ne!(pixel(&pixels, AT + 5.0, AT + 5.0), RED_PIXEL, "what's in it is cut off at the curve inside the border");
}

#[test]
fn what_is_in_two_boxes_with_round_corners_is_cut_off_by_both() {
    // The inner box is square at the outer one's top left, and round at its own bottom right.
    let inner = group(Style::new().width(100.0).height(60.0), Paint::new())
        .prop("clip", true)
        .prop("corner_radii", "0 0 0 0 20 20 0 0")
        .child(group(Style::new().width(100.0).height(60.0), Paint::new().background(RED)));
    let outer = group(Style::new().width(100.0).height(60.0), Paint::new().corner_radius(20.0)).prop("clip", true).child(inner);
    let (_, pixels) = drawn(&placed(outer));
    assert_eq!(pixel(&pixels, AT + 1.0, AT + 1.0), WHITE, "the outer box's corner cuts it off");
    assert_eq!(pixel(&pixels, AT + 98.0, AT + 58.0), WHITE, "and so does the inner box's");
    assert_eq!(pixel(&pixels, AT + 98.0, AT + 1.0), WHITE, "the outer box's other corners too");
    assert_eq!(pixel(&pixels, AT + 50.0, AT + 30.0), RED_PIXEL);
}

#[test]
fn round_corners_cut_off_what_is_placed_by_itself_in_them() {
    // What's `position: absolute` in what it's placed in is drawn after everything else, and still cut off at its corners.
    let cover = group(Style::new().width(100.0).height(60.0), Paint::new().background(RED))
        .prop("position", "absolute")
        .prop("top", 0.0)
        .prop("left", 0.0);
    let round = group(Style::new().width(100.0).height(60.0), Paint::new().corner_radius(12.0))
        .prop("clip", true)
        .prop("position", "relative")
        .child(cover);
    let (_, pixels) = drawn(&placed(round));
    assert_eq!(pixel(&pixels, AT + 50.0, AT + 30.0), RED_PIXEL);
    assert_eq!(pixel(&pixels, AT, AT), WHITE, "its corner is cut off");
}

#[test]
fn round_corners_go_where_they_are_moved_to_and_cut_off_what_is_moved_in_them() {
    // Moved along by a transform, the box's corners go with it.
    let moved = halves(Paint::new().corner_radius(12.0)).prop("transform", "translate(30px, 0)");
    let (_, pixels) = drawn(&placed(moved));
    assert_eq!(pixel(&pixels, AT + 30.0, AT), WHITE, "its corner, where it's moved to");
    assert_eq!(pixel(&pixels, AT + 30.0, AT + 30.0), RED_PIXEL);
    // What's moved inside it is cut off where its corners are, on the screen.
    let inside = group(Style::new().width(100.0).height(60.0), Paint::new().corner_radius(12.0))
        .prop("clip", true)
        .child(group(Style::new().width(100.0).height(60.0), Paint::new().background(RED)).prop("transform", "translate(10px, 0)"));
    let (_, pixels) = drawn(&placed(inside));
    assert_eq!(pixel(&pixels, AT + 99.0, AT), WHITE, "the corner it's moved under");
    assert_eq!(pixel(&pixels, AT + 50.0, AT + 30.0), RED_PIXEL);
}

#[test]
fn round_corners_inside_something_that_cuts_off_more_are_cut_off_by_it_too() {
    // A box with round corners, of which the top half shows inside what's around it.
    let round = halves(Paint::new().corner_radius(12.0));
    let around = group(Style::new().width(100.0).height(30.0), Paint::new()).prop("clip", true).child(round);
    let (_, pixels) = drawn(&placed(around));
    assert_eq!(pixel(&pixels, AT, AT), WHITE, "its own corner");
    assert_eq!(pixel(&pixels, AT, AT + 29.0), RED_PIXEL, "what's left of it");
    assert_eq!(pixel(&pixels, AT, AT + 31.0), WHITE, "what's around it cuts it off");
}

/// A box 80 square with corners 26 round, a shadow around it as `shadows` says, and a background `background`, if any.
fn shadowed(shadows: &str, background: Option<Color>) -> Element {
    let mut paint = Paint::new().corner_radius(26.0);
    if let Some(background) = background {
        paint = paint.background(background);
    }
    group(Style::new().width(80.0).height(80.0), paint).prop("shadows", shadows)
}

#[test]
fn a_shadow_around_what_can_be_seen_through_goes_right_up_to_its_round_corners() {
    // Black, a pixel down and 6 bigger all round, without a blur.
    let shadow = "0 1 0 6 255";
    let (scene, pixels) = drawn(&placed(shadowed(shadow, None)));
    // Inside the box's square but outside its curve, the shadow shows, as it's outside the box.
    assert_eq!(pixel(&pixels, AT + 6.0, AT + 6.0), [0, 0, 0], "outside its corner");
    assert_eq!(pixel(&pixels, AT - 3.0, AT + 40.0), [0, 0, 0], "beside it");
    assert_eq!(pixel(&pixels, AT + 40.0, AT + 40.0), WHITE, "it isn't under the box");
    assert_eq!(pixel(&pixels, AT + 12.0, AT + 12.0), WHITE, "even inside its corner");
    assert!(!scene.draws_at(AT + 40.0, AT + 40.0), "nothing's drawn in it");
    assert!(scene.draws_at(AT - 3.0, AT + 40.0), "the shadow is drawn around it");
}

#[test]
fn soft_shadows_around_what_can_be_seen_through_have_no_square_corners() {
    // As a page's `box-shadow: 0 0 0 1px #0000000d, 0 4px 12px #00000010, 0 24px 60px #00000024` says.
    let shadows = "0 0 0 1 13\n0 4 12 0 16\n0 24 60 0 36";
    let gradient = shadowed(shadows, None).prop("background_gradient", "145|#4f8df5ff|#37599bff");
    let (_, pixels) = drawn(&placed(gradient));
    // Between the box's square and its curve, the shadow's as dark as just outside the curve, rather than white.
    let corner = pixel(&pixels, AT + 1.0, AT + 1.0);
    assert!(corner[0] < 255, "{corner:?}");
}

#[test]
fn a_shadow_inside_a_box_with_round_corners_follows_them() {
    let (_, pixels) = drawn(&placed(shadowed("0 0 4 8 255 inset", Some(Color::WHITE))));
    assert_eq!(pixel(&pixels, AT, AT), WHITE, "outside the corner");
    assert!(pixel(&pixels, AT + 9.0, AT + 9.0)[0] < 128, "inside the corner, it's dark");
    assert_eq!(pixel(&pixels, AT + 40.0, AT + 40.0), WHITE, "the middle isn't");
}
