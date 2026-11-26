//! What gets drawn. Scene tests check the list of quads exactly, with no GPU. Pixel tests
//! draw on Windows' software GPU and read the result back, so no window opens.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use rsui_core::{Element, Node, PropValue, Props, Role};
use rsui_layout::{Layout, Size, Style, style};
use rsui_render::{Color, Paint, Painter, PixelRect, Quad, Renderer, Scene};
use rsui_text::TextEngine;
use support::plain_font;

const WINDOW: (u32, u32) = (200, 100);
const RED: Color = Color::hex(0xFF0000);
const BLUE: Color = Color::hex(0x0000FF);
const WHITE: Color = Color::WHITE;

fn painted(style: Style, paint: Paint) -> Props {
    Props::from(style).merged(&Props::from(paint))
}

fn block(style: Style, color: Color) -> Element {
    Element::new(Role::Group).props(painted(style, Paint::new().background(color)))
}

struct Fixture {
    layout: Layout,
    text: TextEngine,
    painter: Painter,
}

impl Fixture {
    fn new() -> Self {
        Self {
            layout: Layout::new(),
            text: TextEngine::new(plain_font()),
            painter: Painter::new(),
        }
    }

    fn scene(&mut self, root: &Node, scale: f32) -> &mut Scene {
        let available = Size::new(WINDOW.0 as f32 / scale, WINDOW.1 as f32 / scale);
        // These tests draw unrelated trees, with no patches between them, so each needs a
        // layout of its own.
        self.layout = Layout::new();
        self.layout.compute(root, available, &mut self.text);
        let size = (WINDOW.0, WINDOW.1);
        self.painter
            .paint(root, &self.layout, &mut self.text, scale, size)
    }
}

fn solid(rect: [f32; 4], color: Color, radius: f32) -> Quad {
    Quad {
        rect,
        glyph: [0; 4],
        color: [color.r, color.g, color.b, color.a],
        radius,
    }
}

#[test]
fn only_elements_with_a_background_draw_anything() {
    let root = Element::new(Role::Window)
        .child(Element::new(Role::Group).props(Style::new().height(10.0)))
        .child(block(Style::new().height(10.0), RED))
        .into_node();
    let mut fixture = Fixture::new();
    let scene = fixture.scene(&root, 1.0);
    assert_eq!(scene.quads, [solid([0.0, 10.0, 200.0, 10.0], RED, 0.0)]);
}

mod partial {
    use proptest::prelude::*;
    use rsui_render::{Batch, Color, PixelRect, Quad, Renderer, Scene};

    const SIZE: (u32, u32) = (64, 48);
    const CLEAR: Color = Color::hex(0xF6F8FA);

    #[derive(Debug, Clone)]
    enum Change {
        Move(usize, f32, f32),
        Recolor(usize, u8),
        Add(usize, Quad),
        Remove(usize),
        Clip(i32),
        Nothing,
    }

    fn quad() -> impl Strategy<Value = Quad> {
        (
            (-8.0f32..64.0, -8.0f32..48.0, 1.0f32..30.0, 1.0f32..30.0),
            any::<u8>(),
            any::<u8>(),
            prop_oneof![Just(0.0f32), 1.0f32..8.0],
            prop_oneof![Just(0u16), Just(16), Just(32)],
        )
            .prop_map(
                |((x, y, width, height), shade, alpha, radius, outline)| Quad {
                    // Whole pixels, as glyphs and rectangles always are.
                    rect: [x.floor(), y.floor(), width.floor(), height.floor()],
                    glyph: [outline, 0, 0, 0],
                    // Premultiplied, as the renderer expects.
                    color: [shade.min(alpha), shade.min(alpha) / 2, 0, alpha],
                    radius,
                },
            )
    }

    #[cfg(windows)]
    #[test]
    fn cropped_readback_matches_the_shown_buffer_after_resize_and_sync() {
        let mut renderer = Renderer::offscreen_flipping(SIZE).unwrap();
        let bounds = PixelRect {
            left: 1,
            top: 2,
            right: 5,
            bottom: 7,
        };
        for size in [SIZE, (21, 18), SIZE] {
            renderer.resize(size).unwrap();
            let quads = [super::solid(
                [2.0, 1.0, 2.0, 4.0],
                Color::hex(0xe16a32),
                0.0,
            )];
            renderer.render(&mut scene_of(&quads, 40), CLEAR).unwrap();
            renderer.wait_for_frame().unwrap();
            let full = renderer.read_pixels().unwrap();
            let mut expected = Vec::new();
            for y in 2..7 {
                let offset = (y * size.0 as usize + 1) * 4;
                expected.extend_from_slice(&full[offset..offset + 16]);
            }
            assert_eq!(
                renderer.read_offscreen_region(bounds, false).unwrap(),
                expected
            );
            for pixel in expected.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            assert_eq!(
                renderer.read_offscreen_region(bounds, true).unwrap(),
                expected
            );
        }
    }

    fn change() -> impl Strategy<Value = Change> {
        prop_oneof![
            3 => (any::<usize>(), -6.0f32..6.0, -6.0f32..6.0)
                .prop_map(|(at, x, y)| Change::Move(at, x.floor(), y.floor())),
            3 => (any::<usize>(), any::<u8>()).prop_map(|(at, value)| Change::Recolor(at, value)),
            2 => (any::<usize>(), quad()).prop_map(|(at, quad)| Change::Add(at, quad)),
            2 => any::<usize>().prop_map(Change::Remove),
            1 => (4i32..40).prop_map(Change::Clip),
            2 => Just(Change::Nothing),
        ]
    }

    fn apply(quads: &mut Vec<Quad>, clip_edge: &mut i32, change: &Change) {
        let count = quads.len();
        match *change {
            Change::Move(at, x, y) if count > 0 => {
                quads[at % count].rect[0] += x;
                quads[at % count].rect[1] += y;
            }
            Change::Recolor(at, value) if count > 0 => {
                let alpha = quads[at % count].color[3];
                quads[at % count].color[0] = value.min(alpha);
            }
            Change::Add(at, quad) => quads.insert(at % (count + 1), quad),
            Change::Remove(at) if count > 0 => {
                quads.remove(at % count);
            }
            Change::Clip(edge) => *clip_edge = edge,
            _ => {}
        }
    }

    /// The first half of the quads aren't clipped, and the rest are cut off at a line.
    fn scene_of(quads: &[Quad], clip_edge: i32) -> Scene {
        let window = PixelRect {
            left: 0,
            top: 0,
            right: SIZE.0 as i32,
            bottom: SIZE.1 as i32,
        };
        let half = (quads.len() / 2) as u32;
        let mut batches = Vec::new();
        if half > 0 {
            batches.push(Batch {
                first: 0,
                count: half,
                clip: window,
                image: 0,
                transform: 0,
                filter: 0,
                round: 0,
            });
        }
        if quads.len() as u32 > half {
            batches.push(Batch {
                first: half,
                count: quads.len() as u32 - half,
                clip: PixelRect {
                    bottom: clip_edge,
                    ..window
                },
                image: 0,
                transform: 0,
                filter: 0,
                round: 0,
            });
        }
        Scene {
            quads: quads.to_vec(),
            batches,
            uploads: Vec::new(),
            atlas_size: 16,
            atlas_generation: 0,
            ..Scene::default()
        }
    }

    fn check(flipping: bool, start: &[Quad], changes: &[Change]) -> Result<(), TestCaseError> {
        let make = || {
            if flipping {
                Renderer::offscreen_flipping(SIZE)
            } else {
                Renderer::offscreen(SIZE)
            }
            .expect("the software GPU is always there")
        };
        let mut partial = make();
        let mut quads = start.to_vec();
        let mut clip_edge = 40;
        partial
            .render(&mut scene_of(&quads, clip_edge), CLEAR)
            .unwrap();
        for (frame, change) in changes.iter().enumerate() {
            apply(&mut quads, &mut clip_edge, change);
            partial
                .render(&mut scene_of(&quads, clip_edge), CLEAR)
                .unwrap();
            // A brand new renderer can't be relying on anything from before.
            let mut full = Renderer::offscreen(SIZE).unwrap();
            full.render(&mut scene_of(&quads, clip_edge), CLEAR)
                .unwrap();
            prop_assert!(
                partial.read_pixels().unwrap() == full.read_pixels().unwrap(),
                "frame {} differs after {:?}",
                frame,
                change
            );
        }
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(120))]

        /// However a scene changes, drawing only what changed gives exactly the picture that
        /// drawing everything does, with one buffer or two.
        #[test]
        fn drawing_only_what_changed_matches_drawing_everything(
            start in proptest::collection::vec(quad(), 0..12),
            changes in proptest::collection::vec(change(), 1..10),
            flipping in any::<bool>(),
        ) {
            check(flipping, &start, &changes)?;
        }
    }

    #[test]
    fn a_frame_that_changed_nothing_draws_nothing() {
        // An upload-free scene that differs in one quad, on a renderer that's always been given
        // the same one: the picture stays right, and so does the next change.
        let mut renderer = Renderer::offscreen_flipping(SIZE).unwrap();
        let quads = [Quad {
            rect: [2.0, 2.0, 10.0, 10.0],
            glyph: [0; 4],
            color: [255, 0, 0, 255],
            radius: 0.0,
        }];
        for _ in 0..5 {
            renderer.render(&mut scene_of(&quads, 40), CLEAR).unwrap();
        }
        let pixels = renderer.read_pixels().unwrap();
        let at = (5 * SIZE.0 as usize + 5) * 4;
        assert_eq!(pixels[at..at + 4], [255, 0, 0, 255]);
    }

    #[test]
    fn trimming_never_loses_the_picture() {
        let mut renderer = Renderer::offscreen_flipping(SIZE).unwrap();
        let mut quads = vec![Quad {
            rect: [2.0, 2.0, 10.0, 10.0],
            glyph: [0; 4],
            color: [255, 0, 0, 255],
            radius: 0.0,
        }];
        for round in 0..6 {
            quads[0].rect[0] = round as f32 * 3.0;
            renderer.render(&mut scene_of(&quads, 40), CLEAR).unwrap();
            if round % 2 == 1 {
                renderer.trim();
            }
            let mut full = Renderer::offscreen(SIZE).unwrap();
            full.render(&mut scene_of(&quads, 40), CLEAR).unwrap();
            assert!(
                renderer.read_pixels().unwrap() == full.read_pixels().unwrap(),
                "round {round}"
            );
        }
    }
}

#[test]
fn a_border_is_an_outline_over_the_background() {
    let root = Element::new(Role::Window)
        .child(
            Element::new(Role::Group).props(painted(
                Style::new().height(10.0),
                Paint::new()
                    .background(RED)
                    .border(BLUE, 2.0)
                    .corner_radius(3.0),
            )),
        )
        .into_node();
    let mut fixture = Fixture::new();
    let scene = fixture.scene(&root, 1.5);
    let area = [0.0, 0.0, 200.0, 15.0];
    assert_eq!(scene.quads[0], solid(area, RED, 4.5));
    assert_eq!(
        scene.quads[1],
        Quad {
            rect: area,
            // 2 layout pixels is 3 physical pixels, counted in sixteenths.
            glyph: [48, 0, 0, 0],
            color: [BLUE.r, BLUE.g, BLUE.b, BLUE.a],
            radius: 4.5,
        }
    );
}

#[test]
fn a_border_only_covers_the_edge_of_what_it_surrounds() {
    let root = Element::new(Role::Window)
        .child(Element::new(Role::Group).props(painted(
            Style::new().height(20.0),
            Paint::new().background(RED).border(BLUE, 2.0),
        )))
        .into_node();
    let mut fixture = Fixture::new();
    let scene = fixture.scene(&root, 1.0);
    let mut renderer = Renderer::offscreen(WINDOW).unwrap();
    renderer.render(scene, WHITE).unwrap();
    let pixels = renderer.read_pixels().unwrap();
    let at = |x: usize, y: usize| {
        let at = (y * WINDOW.0 as usize + x) * 4;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    };
    let (blue, red) = ([0, 0, 255, 255], [255, 0, 0, 255]);
    assert_eq!(at(0, 0), blue, "the corner");
    assert_eq!(at(1, 1), blue, "the inside of the border");
    assert_eq!(at(2, 2), red, "just inside it");
    assert_eq!(at(100, 10), red, "the middle");
    assert_eq!(at(199, 19), blue, "the far corner");
    assert_eq!(at(100, 20), [255, 255, 255, 255], "nothing beyond it");
}

#[test]
fn quads_come_in_drawing_order_with_parents_first() {
    let root = block(Style::new().padding(10.0), RED)
        .child(block(Style::new().height(20.0), BLUE))
        .into_node();
    let mut fixture = Fixture::new();
    let scene = fixture.scene(&root, 1.0);
    assert_eq!(
        scene.quads,
        [
            solid([0.0, 0.0, 200.0, 100.0], RED, 0.0),
            solid([10.0, 10.0, 180.0, 20.0], BLUE, 0.0),
        ]
    );
    assert_eq!(scene.batches.len(), 1);
    assert_eq!((scene.batches[0].first, scene.batches[0].count), (0, 2));
}

#[test]
fn scale_multiplies_everything_and_edges_land_on_whole_pixels() {
    let root = Element::new(Role::Window)
        .child(block(Style::new().height(10.3), RED))
        .into_node();
    let mut fixture = Fixture::new();
    let scene = fixture.scene(&root, 1.5);
    // 10.3 layout pixels is 15.45 physical pixels, which rounds to 15.
    assert_eq!(scene.quads, [solid([0.0, 0.0, 200.0, 15.0], RED, 0.0)]);
}

#[test]
fn colors_are_premultiplied_and_invisible_ones_are_skipped() {
    let half = Color::rgba(255, 0, 0, 128);
    let root = Element::new(Role::Window)
        .child(block(Style::new().height(10.0), half))
        .child(block(Style::new().height(10.0), Color::TRANSPARENT))
        .into_node();
    let mut fixture = Fixture::new();
    let scene = fixture.scene(&root, 1.0);
    assert_eq!(scene.quads.len(), 1);
    assert_eq!(scene.quads[0].color, [128, 0, 0, 128]);
}

#[test]
fn corner_radius_is_scaled_with_everything_else() {
    let root = Element::new(Role::Window)
        .child(Element::new(Role::Group).props(painted(
            Style::new().height(10.0),
            Paint::new().background(RED).corner_radius(3.0),
        )))
        .into_node();
    let mut fixture = Fixture::new();
    let scene = fixture.scene(&root, 2.0);
    assert_eq!(scene.quads[0].radius, 6.0);
}

#[test]
fn things_outside_the_window_are_not_drawn() {
    let root = Element::new(Role::Window)
        .child(block(Style::new().height(90.0), RED))
        .child(block(Style::new().height(50.0), BLUE))
        .child(block(Style::new().height(50.0), RED))
        .into_node();
    let mut fixture = Fixture::new();
    let scene = fixture.scene(&root, 1.0);
    assert_eq!(scene.quads.len(), 2, "the third starts past the bottom");
}

#[test]
fn text_becomes_glyphs_on_whole_pixels_at_its_padding() {
    let root = Element::new(Role::Window)
        .child(
            Element::new(Role::Text)
                .name("A")
                .props(painted(
                    Style::new().padding_each(10.0, 0.0, 0.0, 20.0),
                    Paint::new().text(RED),
                ))
                .prop("font_size", 10.0),
        )
        .into_node();
    let mut fixture = Fixture::new();
    let scene = fixture.scene(&root, 1.0);
    assert_eq!(scene.quads.len(), 1);
    let glyph = scene.quads[0];
    // The test font's A is a square 5 pixels wide at this size, standing on the baseline.
    // The baseline is 8 below the top of the line, which is 10 down, and the square is 5 tall.
    assert_eq!(glyph.rect, [20.0, 13.0, 5.0, 5.0]);
    assert_eq!((glyph.glyph[2], glyph.glyph[3]), (5, 5));
    assert_eq!(glyph.color, [255, 0, 0, 255]);
    assert_eq!(scene.uploads.len(), 1);
}

#[test]
fn text_drawn_again_comes_out_the_same_and_changed_text_comes_out_changed() {
    let text = |name: &str, color: Color, top: f32| {
        Element::new(Role::Window)
            .child(
                Element::new(Role::Text)
                    .name(name)
                    .props(painted(
                        Style::new().padding_each(top, 0.0, 0.0, 20.0),
                        Paint::new().text(color),
                    ))
                    .prop("font_size", 10.0),
            )
            .into_node()
    };
    // A fresh fixture works everything out. Where glyphs are in the atlas depends on what was drawn before, so only where
    // they go and their color are compared.
    let placed = |quads: &[Quad]| -> Vec<([f32; 4], [u8; 4])> {
        quads.iter().map(|quad| (quad.rect, quad.color)).collect()
    };
    let mut fixture = Fixture::new();
    for (root, scale) in [
        (text("AB", RED, 10.0), 1.0),
        (text("AB", RED, 10.0), 1.0),
        (text("AB", BLUE, 10.0), 1.0),
        (text("AB", BLUE, 12.0), 1.0),
        (text("BA", BLUE, 12.0), 1.0),
        (text("AB", RED, 10.0), 1.0),
        (text("AB", RED, 10.0), 2.0),
    ] {
        let again = placed(&fixture.scene(&root, scale).quads);
        assert_eq!(again, placed(&Fixture::new().scene(&root, scale).quads));
        assert_eq!(again.len(), 2);
    }
}

#[test]
fn glyphs_already_uploaded_are_not_uploaded_again() {
    let root = Element::new(Role::Text)
        .name("AAA")
        .prop("font_size", 10.0)
        .into_node();
    let mut fixture = Fixture::new();
    let scene = fixture.scene(&root, 1.0);
    assert_eq!(scene.uploads.len(), 1);
    // Drawing a scene is what takes these.
    scene.uploads.clear();
    assert!(fixture.scene(&root, 1.0).uploads.is_empty());
}

#[test]
fn glyphs_wait_for_a_scene_to_be_drawn() {
    let root = Element::new(Role::Text)
        .name("AAA")
        .prop("font_size", 10.0)
        .into_node();
    let mut fixture = Fixture::new();
    fixture.scene(&root, 1.0);
    assert_eq!(fixture.scene(&root, 1.0).uploads.len(), 1);
}

#[test]
fn a_parent_with_children_does_not_draw_its_name() {
    let root = Element::new(Role::Button)
        .name("Name for agents")
        .child(Element::new(Role::Group))
        .into_node();
    let mut fixture = Fixture::new();
    assert!(fixture.scene(&root, 1.0).quads.is_empty());
}

fn virtual_list_root() -> Node {
    Element::new(Role::List)
        .props(style!(
            Style::new().row_height(20.0).flex(1.0),
            Paint::new().background(WHITE)
        ))
        .prop("count", 1000i64)
        .prop("first_built", 2i64)
        .prop("first_visible", 4i64)
        .children((0..6).map(|_| block(Style::new(), BLUE)))
        .into_node()
}

#[test]
fn a_virtual_list_clips_its_rows() {
    let root = Element::new(Role::Window)
        .prop("padding_top", 30.0)
        .prop("padding_bottom", 25.0)
        .child(virtual_list_root())
        .into_node();
    let mut fixture = Fixture::new();
    let scene = fixture.scene(&root, 1.0);
    let window = PixelRect {
        left: 0,
        top: 0,
        right: 200,
        bottom: 100,
    };
    assert_eq!(scene.batches.len(), 2);
    assert_eq!(scene.batches[0].clip, window);
    assert_eq!(
        scene.batches[1].clip,
        PixelRect {
            left: 0,
            top: 30,
            right: 200,
            bottom: 75,
        }
    );
    // Rows entirely above the list aren't drawn at all. The first is 40 above its top.
    assert!(scene.quads.iter().all(|quad| quad.rect[1] >= 30.0));
    // The last row goes past the bottom, so it's left to the clip to cut it off.
    assert!(
        scene
            .quads
            .iter()
            .any(|quad| quad.rect[1] + quad.rect[3] > 75.0)
    );
}

#[test]
fn styles_and_paint_combine_in_one_shared_set() {
    let props = || {
        style!(
            Style::row().gap(4.0),
            Paint::new().background(RED).corner_radius(2.0)
        )
    };
    assert_eq!(props(), props());
    assert_eq!(props().get("corner_radius"), Some(&PropValue::Float(2.0)));
    assert_eq!(props().get("gap"), Some(&PropValue::Float(4.0)));
}

/// Draws a tree on the software GPU, and returns the pixels.
struct Canvas {
    renderer: Renderer,
    fixture: Fixture,
}

impl Canvas {
    fn new() -> Self {
        Self {
            renderer: Renderer::offscreen(WINDOW).expect("the software GPU is always there"),
            fixture: Fixture::new(),
        }
    }

    fn draw(&mut self, root: &Node) -> Vec<u8> {
        let scene = self.fixture.scene(root, 1.0);
        self.renderer.render(scene, WHITE).expect("drawing works");
        self.renderer.read_pixels().expect("reading works")
    }
}

fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 4] {
    let at = (y * WINDOW.0 as usize + x) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

const RED_PIXEL: [u8; 4] = [255, 0, 0, 255];
const BLUE_PIXEL: [u8; 4] = [0, 0, 255, 255];
const WHITE_PIXEL: [u8; 4] = [255, 255, 255, 255];

#[test]
fn rectangles_land_on_exactly_the_right_pixels() {
    let root = Element::new(Role::Window)
        .props(Style::new().padding_each(10.0, 0.0, 0.0, 20.0))
        .child(block(Style::new().width(30.0).height(5.0), RED))
        .into_node();
    let pixels = Canvas::new().draw(&root);
    assert_eq!(pixel(&pixels, 20, 10), RED_PIXEL);
    assert_eq!(pixel(&pixels, 49, 14), RED_PIXEL);
    assert_eq!(pixel(&pixels, 19, 10), WHITE_PIXEL, "just left of it");
    assert_eq!(pixel(&pixels, 50, 10), WHITE_PIXEL, "just right of it");
    assert_eq!(pixel(&pixels, 20, 9), WHITE_PIXEL, "just above it");
    assert_eq!(pixel(&pixels, 20, 15), WHITE_PIXEL, "just below it");
}

#[test]
fn later_things_draw_over_earlier_ones() {
    let root = block(Style::new().padding(10.0), RED)
        .child(block(Style::new().height(20.0), BLUE))
        .into_node();
    let pixels = Canvas::new().draw(&root);
    assert_eq!(pixel(&pixels, 5, 5), RED_PIXEL);
    assert_eq!(pixel(&pixels, 15, 15), BLUE_PIXEL);
}

#[test]
fn transparent_colors_blend_with_what_is_behind_them() {
    let root = block(Style::new(), RED)
        .child(block(
            Style::new().height(20.0),
            Color::rgba(0, 0, 255, 128),
        ))
        .into_node();
    let pixels = Canvas::new().draw(&root);
    let [r, g, b, a] = pixel(&pixels, 10, 10);
    assert!((126..=128).contains(&r), "{r}");
    assert_eq!(g, 0);
    assert!((127..=129).contains(&b), "{b}");
    assert_eq!(a, 255);
}

#[test]
fn rounded_corners_are_cut_away_and_smooth() {
    let root = Element::new(Role::Window)
        .child(Element::new(Role::Group).props(painted(
            Style::new().width(40.0).height(40.0),
            Paint::new().background(RED).corner_radius(10.0),
        )))
        .into_node();
    let pixels = Canvas::new().draw(&root);
    assert_eq!(pixel(&pixels, 20, 20), RED_PIXEL, "the middle");
    assert_eq!(pixel(&pixels, 0, 20), RED_PIXEL, "the middle of an edge");
    assert_eq!(pixel(&pixels, 0, 0), WHITE_PIXEL, "the corner itself");
    // This pixel's center is 9.92 from the center of the corner's circle of radius 10, so
    // the curve passes through it, and it should be partly covered.
    let edge = pixel(&pixels, 3, 2);
    assert!(edge[1] > 0 && edge[1] < 255, "{edge:?}");
}

#[test]
fn individually_rounded_borders_keep_their_curves_when_one_side_is_missing() {
    let slot = |corners: &str, left: f64| {
        Element::new(Role::Group)
            .props(painted(Style::new().width(36.0).height(36.0), Paint::new().border(BLUE, 0.0)))
            .prop("corner_radii", corners)
            .prop("border_top_width", 1.0)
            .prop("border_right_width", 1.0)
            .prop("border_bottom_width", 1.0)
            .prop("border_left_width", left)
    };
    let root = Element::new(Role::Window).props(Style::row().gap(10.0))
        .child(slot("6 6 0 0 0 0 6 6", 1.0))
        .child(slot("0 0 6 6 6 6 0 0", 0.0))
        .into_node();
    let pixels = Canvas::new().draw(&root);
    assert_eq!(pixel(&pixels, 0, 0), WHITE_PIXEL, "left outside corner");
    assert_eq!(pixel(&pixels, 81, 0), WHITE_PIXEL, "right outside corner");
    assert_eq!(pixel(&pixels, 0, 18), BLUE_PIXEL, "left border remains");
    assert_eq!(pixel(&pixels, 81, 18), BLUE_PIXEL, "right border remains");
    assert_eq!(pixel(&pixels, 46, 18), WHITE_PIXEL, "missing left border stays absent");
    let arc = pixel(&pixels, 78, 1);
    assert!(arc[2] > 200 && arc[0] < 255, "the rounded arc is drawn: {arc:?}");
}

#[test]
fn glyphs_draw_their_pixels_in_their_color() {
    let root = Element::new(Role::Window)
        .child(
            Element::new(Role::Text)
                .name("A")
                .props(painted(
                    Style::new().padding_each(10.0, 0.0, 0.0, 20.0),
                    Paint::new().text(RED),
                ))
                .prop("font_size", 10.0),
        )
        .into_node();
    let pixels = Canvas::new().draw(&root);
    // The square of the test font's A, from the scene test above.
    assert_eq!(pixel(&pixels, 20, 13), RED_PIXEL);
    assert_eq!(pixel(&pixels, 24, 17), RED_PIXEL);
    assert_eq!(pixel(&pixels, 19, 13), WHITE_PIXEL);
    assert_eq!(pixel(&pixels, 25, 17), WHITE_PIXEL);
    assert_eq!(pixel(&pixels, 20, 12), WHITE_PIXEL);
    assert_eq!(pixel(&pixels, 20, 18), WHITE_PIXEL);
}

#[test]
fn a_clip_hides_what_is_outside_it() {
    let root = Element::new(Role::Window)
        .prop("padding_top", 30.0)
        .prop("padding_bottom", 25.0)
        .child(virtual_list_root())
        .into_node();
    let pixels = Canvas::new().draw(&root);
    assert_eq!(pixel(&pixels, 10, 29), WHITE_PIXEL);
    assert_eq!(pixel(&pixels, 10, 30), BLUE_PIXEL);
    // A row runs from 70 to 90, past the list's bottom at 75, and must be cut off there.
    assert_eq!(pixel(&pixels, 10, 74), BLUE_PIXEL);
    assert_eq!(pixel(&pixels, 10, 75), WHITE_PIXEL);
}

#[test]
fn new_frames_replace_old_ones() {
    let first = block(Style::new(), RED).into_node();
    let second = Element::new(Role::Window)
        .child(block(Style::new().height(10.0), BLUE))
        .into_node();
    let mut canvas = Canvas::new();
    assert_eq!(pixel(&canvas.draw(&first), 50, 50), RED_PIXEL);
    let pixels = canvas.draw(&second);
    assert_eq!(pixel(&pixels, 50, 5), BLUE_PIXEL);
    assert_eq!(pixel(&pixels, 50, 50), WHITE_PIXEL);
}

#[test]
fn glyphs_survive_the_atlas_growing() {
    let mut canvas = Canvas::new();
    let small = |name: &str| {
        Element::new(Role::Text)
            .name(name)
            .props(painted(Style::new(), Paint::new().text(RED)))
            .prop("font_size", 10.0)
            .into_node()
    };
    canvas.draw(&small("A"));
    // Enough large glyphs to outgrow the first atlas, in a window too small to show them.
    for size in 0..40 {
        let large = Element::new(Role::Text)
            .name("A")
            .prop("font_size", 100.0 + f64::from(size))
            .into_node();
        canvas.draw(&large);
    }
    let pixels = canvas.draw(&small("A"));
    assert_eq!(
        pixel(&pixels, 2, 5),
        RED_PIXEL,
        "the first glyph is still in the atlas"
    );
}

#[test]
fn more_quads_than_the_buffer_starts_with_all_draw() {
    let root = Element::new(Role::Window)
        .children((0..3_000).map(|_| {
            Element::new(Role::Group).props(painted(
                Style::new().height(0.05),
                Paint::new().background(RED),
            ))
        }))
        .into_node();
    let pixels = Canvas::new().draw(&root);
    // Their heights round to nothing or one pixel, and they cover 150 pixels' worth, so
    // only the ones in the window draw. The last row of the window should still be reached.
    assert_eq!(pixel(&pixels, 100, 99), RED_PIXEL);
}

#[test]
fn resizing_changes_what_is_drawn_to() {
    let root = block(Style::new(), RED).into_node();
    let mut canvas = Canvas::new();
    canvas.draw(&root);
    canvas.renderer.resize((10, 20)).unwrap();
    assert_eq!(canvas.renderer.size(), (10, 20));
    let scene = canvas.fixture.scene(&root, 1.0);
    canvas.renderer.render(scene, WHITE).unwrap();
    let pixels = canvas.renderer.read_pixels().unwrap();
    assert_eq!(pixels.len(), 10 * 20 * 4);
    assert_eq!(&pixels[..4], RED_PIXEL);
}

#[test]
fn trimming_does_not_stop_drawing() {
    let root = block(Style::new(), RED).into_node();
    let mut canvas = Canvas::new();
    canvas.draw(&root);
    canvas.renderer.trim();
    assert_eq!(pixel(&canvas.draw(&root), 50, 50), RED_PIXEL);
}

/// The program ships compiled shaders, which must match their source.
#[test]
#[cfg(windows)]
fn the_compiled_shaders_match_their_source() {
    use windows::Win32::Graphics::Direct3D::Fxc::{D3DCOMPILE_OPTIMIZATION_LEVEL3, D3DCompile};
    use windows::Win32::Graphics::Direct3D::ID3DBlob;
    use windows::core::s;
    let source = include_str!("../src/quad.hlsl");
    for (entry, target, shipped) in [
        (
            s!("vs"),
            s!("vs_4_0"),
            include_bytes!("../src/quad_vs.dxbc").as_slice(),
        ),
        (
            s!("ps"),
            s!("ps_4_0"),
            include_bytes!("../src/quad_ps.dxbc").as_slice(),
        ),
    ] {
        let mut code: Option<ID3DBlob> = None;
        // SAFETY: as in the `compile_shaders` example.
        let compiled = unsafe {
            D3DCompile(
                source.as_ptr().cast(),
                source.len(),
                s!("quad.hlsl"),
                None,
                None,
                entry,
                target,
                D3DCOMPILE_OPTIMIZATION_LEVEL3,
                0,
                &mut code,
                None,
            )
            .expect("the shader compiles");
            let code = code.expect("a successful compile gives code");
            std::slice::from_raw_parts(code.GetBufferPointer().cast::<u8>(), code.GetBufferSize())
                .to_vec()
        };
        assert!(
            compiled == shipped,
            "run `cargo run -p rsui-render --example compile_shaders`"
        );
    }
}

#[test]
fn a_shadow_fades_away_from_what_it_is_behind() {
    let root = Element::new(Role::Window)
        .props(Style::new().padding_each(40.0, 0.0, 0.0, 40.0))
        .child(
            Element::new(Role::Group).props(painted(
                Style::new().width(40.0).height(40.0),
                Paint::new()
                    .background(RED)
                    .shadow(Color::rgba(0, 0, 0, 255), 16.0, 0.0),
            )),
        )
        .into_node();
    let pixels = Canvas::new().draw(&root);
    assert_eq!(pixel(&pixels, 60, 60), RED_PIXEL, "it's behind the box");
    let shade = |x, y| i32::from(pixel(&pixels, x, y)[0]);
    let (close, further, far) = (shade(82, 60), shade(90, 60), shade(120, 60));
    assert!(
        close < further && further < far,
        "{close}, {further}, {far}"
    );
    assert_eq!(far, 255, "it stops");
}

#[test]
fn a_faded_box_does_not_show_its_shadow_through_itself() {
    // Mantine's Menu and Popover dropdowns fade in (`opacity`) with a shadow under an opaque white background. Under what
    // can't be seen through the whole shadow was drawn, as it's covered, but faded, the box showed it through, grey.
    let root = Element::new(Role::Window)
        .props(painted(Style::new().padding_each(30.0, 0.0, 0.0, 60.0), Paint::new().background(WHITE)))
        .child(
            Element::new(Role::Group)
                .props(painted(
                    Style::new().width(40.0).height(40.0),
                    Paint::new()
                        .background(WHITE)
                        .shadow(Color::rgba(0, 0, 0, 255), 8.0, 0.0),
                ))
                .prop("opacity", 0.5),
        )
        .into_node();
    let pixels = Canvas::new().draw(&root);
    assert_eq!(pixel(&pixels, 80, 50), WHITE_PIXEL, "the middle of the box is white");
    assert!(pixel(&pixels, 104, 50)[0] < 255, "and its shadow is still around it");
}

#[test]
fn a_shadow_moves_down_when_it_is_told_to() {
    let root = Element::new(Role::Window)
        .props(Style::new().padding_each(40.0, 0.0, 0.0, 40.0))
        .child(
            Element::new(Role::Group).props(painted(
                Style::new().width(40.0).height(40.0),
                Paint::new()
                    .background(RED)
                    .shadow(Color::rgba(0, 0, 0, 255), 8.0, 8.0),
            )),
        )
        .into_node();
    let pixels = Canvas::new().draw(&root);
    let shade = |x, y| i32::from(pixel(&pixels, x, y)[0]);
    assert!(
        shade(60, 86) < shade(60, 36),
        "there's more below than above"
    );
}

#[test]
fn lines_of_text_go_where_they_are_told_along_the_width() {
    let line = |align: &str| {
        Element::new(Role::Window)
            .child(
                Element::new(Role::Text)
                    .name("A")
                    .prop("font_size", 10.0)
                    .prop("text_align", align),
            )
            .into_node()
    };
    let mut fixture = Fixture::new();
    let left = fixture.scene(&line("left"), 1.0).quads[0].rect[0];
    let center = fixture.scene(&line("center"), 1.0).quads[0].rect[0];
    let right = fixture.scene(&line("right"), 1.0).quads[0].rect[0];
    assert!(left < center && center < right, "{left}, {center}, {right}");
    let window = WINDOW.0 as f32;
    assert!((center - window / 2.0).abs() < 8.0, "{center}");
    assert!(right > window - 10.0 && right < window, "{right}");
}

#[test]
fn text_kept_to_two_lines_is_kept_to_them_wherever_it_goes_along_them() {
    // As a grid's names are: centered, and no more than two lines, the second ending in "…". The test font only draws `A`, and this is several lines of it in the window.
    let words = |align: &str, most: Option<f64>| {
        let mut text = Element::new(Role::Text)
            .name(["AAAA"; 24].join(" "))
            .prop("font_size", 14.0)
            .prop("text_align", align);
        if let Some(most) = most {
            text = text.prop("max_lines", most);
        }
        Element::new(Role::Window).child(text).into_node()
    };
    let lowest = |scene: &Scene| scene.quads.iter().map(|quad| quad.rect[1]).fold(f32::MIN, f32::max);
    let mut fixture = Fixture::new();
    let all = lowest(fixture.scene(&words("center", None), 1.0));
    let left = lowest(fixture.scene(&words("left", Some(2.0)), 1.0));
    let center = lowest(fixture.scene(&words("center", Some(2.0)), 1.0));
    assert!(left < all, "kept to two lines, it ends higher than all of it: {left} against {all}");
    assert_eq!(center, left, "centered, it's kept to the same two lines");
    // And none, which is no lines kept rather than a crash.
    fixture.scene(&words("center", Some(0.0)), 1.0);
}

#[test]
fn underlined_text_has_a_line_under_it() {
    let line = |underline: bool| {
        Element::new(Role::Window)
            .child(
                Element::new(Role::Text)
                    .name("AAA")
                    .prop("font_size", 10.0)
                    .prop("underline", underline),
            )
            .into_node()
    };
    let mut fixture = Fixture::new();
    let plain = fixture.scene(&line(false), 1.0).quads.len();
    let scene = fixture.scene(&line(true), 1.0);
    assert_eq!(scene.quads.len(), plain + 1);
    let under = scene.quads[scene.quads.len() - 1];
    assert_eq!(under.glyph, [0; 4], "it's a rectangle");
    assert!(under.rect[2] > 10.0, "as wide as the text");
}

/// A pixel of what was drawn, as red, green and blue.
fn pixel_at(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let at = ((y * WINDOW.0 + x) * 4) as usize;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

fn drawn_with(root: &Node) -> Vec<u8> {
    let mut fixture = Fixture::new();
    let scene = fixture.scene(root, 1.0);
    let mut renderer = Renderer::offscreen(WINDOW).unwrap();
    renderer.render(scene, WHITE).unwrap();
    renderer.read_pixels().unwrap()
}

#[test]
fn backdrop_blur_softens_earlier_pixels_inside_the_card() {
    let place = |element: Element, x: i64, y: i64| {
        element.prop("position", "absolute").prop("left", x).prop("top", y)
    };
    let background = |left, color| place(block(Style::new().width(100.0).height(100.0), color), left, 0);
    let make_tree = |right| {
        let glass = Element::new(Role::Group).props(painted(
            Style::new().width(24.0).height(48.0),
            Paint::new().backdrop_blur(6.0).corner_radius(10.0),
        ));
        let mark = place(block(Style::new().width(4.0).height(6.0), WHITE), 98, 43);
        Element::new(Role::Window)
            .child(background(0, RED))
            .child(background(100, right))
            .child(place(glass, 88, 26))
            .child(mark)
            .into_node()
    };
    let mut fixture = Fixture::new();
    let mut renderer = Renderer::offscreen(WINDOW).unwrap();
    let mut check = |renderer: &mut Renderer| {
        renderer.render(fixture.scene(&make_tree(BLUE), 1.0), WHITE).unwrap();
        let pixels = renderer.read_pixels().unwrap();
        let red = pixel_at(&pixels, 99, 20);
        let blue = pixel_at(&pixels, 100, 20);
        let mixed = pixel_at(&pixels, 99, 55);
        assert_eq!(pixel_at(&pixels, 99, 45), [255; 3], "a later foreground stays sharp");
        assert_eq!(pixel_at(&pixels, 88, 27), red, "the rounded corner stays outside the blur");
        assert_ne!(red, blue);
        assert!(mixed[0] > 30 && mixed[2] > 30, "both sides contribute to {mixed:?}");

        renderer.render(fixture.scene(&make_tree(WHITE), 1.0), WHITE).unwrap();
        let changed = renderer.read_pixels().unwrap();
        assert_ne!(pixel_at(&changed, 99, 55), mixed, "the snapshot refreshes as the backdrop changes");
    };
    check(&mut renderer);
    #[cfg(any(target_os = "linux", target_os = "android"))]
    if let Ok(mut vulkan) = Renderer::offscreen_vulkan(WINDOW) {
        check(&mut vulkan);
        assert_eq!(Renderer::vulkan_validation_errors(), 0);
    }
}

/// Something square in the middle of the window, as it's described by these.
fn square_with(name: &str, value: impl Into<PropValue>) -> Node {
    let root = Element::new(Role::Window)
        .props(painted(
            Style::new()
                .align(rsui_layout::Align::Center)
                .justify(rsui_layout::Justify::Center),
            Paint::new().background(WHITE),
        ))
        .child(block(Style::new().width(40.0).height(40.0), RED).prop(name, value));
    root.into_node()
}

const TURNED_RED: [u8; 3] = [255, 0, 0];
const TURNED_WHITE: [u8; 3] = [255, 255, 255];

#[test]
fn something_turned_a_quarter_of_a_turn_of_a_circle_is_a_diamond() {
    let pixels = drawn_with(&square_with("rotate", 45.0));
    // The middle's where it was.
    assert_eq!(pixel_at(&pixels, 100, 50), TURNED_RED);
    // The corners of what it was have turned away, and its own corners now point up, down and to each side, 28 away.
    assert_eq!(
        pixel_at(&pixels, 83, 33),
        TURNED_WHITE,
        "where a corner was"
    );
    assert_eq!(
        pixel_at(&pixels, 117, 67),
        TURNED_WHITE,
        "where a corner was"
    );
    assert_eq!(
        pixel_at(&pixels, 100, 25),
        TURNED_RED,
        "further up than the square went"
    );
    assert_eq!(
        pixel_at(&pixels, 125, 50),
        TURNED_RED,
        "further across than the square went"
    );
}

#[test]
fn something_resized_grows_around_its_middle_and_nothing_else_moves() {
    let pixels = drawn_with(&square_with("scale", 2.0));
    assert_eq!(pixel_at(&pixels, 100, 50), TURNED_RED);
    assert_eq!(
        pixel_at(&pixels, 62, 12),
        TURNED_RED,
        "it reaches 40 away now"
    );
    assert_eq!(
        pixel_at(&pixels, 138, 88),
        TURNED_RED,
        "it reaches 40 away now"
    );
    assert_eq!(pixel_at(&pixels, 58, 50), TURNED_WHITE, "and no further");
    let smaller = drawn_with(&square_with("scale", 0.5));
    assert_eq!(pixel_at(&smaller, 100, 50), TURNED_RED);
    assert_eq!(
        pixel_at(&smaller, 85, 50),
        TURNED_WHITE,
        "it only reaches 10 away"
    );
}

#[test]
fn a_transform_written_as_css_does_what_it_says() {
    let pixels = drawn_with(&square_with("transform", "translateX(50px) scale(0.5)"));
    assert_eq!(
        pixel_at(&pixels, 100, 50),
        TURNED_WHITE,
        "it's gone from there"
    );
    assert_eq!(pixel_at(&pixels, 150, 50), TURNED_RED);
    assert_eq!(
        pixel_at(&pixels, 135, 50),
        TURNED_WHITE,
        "it's half as wide"
    );
}

#[test]
fn what_is_inside_something_turned_goes_with_it_and_what_is_after_it_does_not() {
    let root: Node = Element::new(Role::Window)
        .props(painted(Style::row(), Paint::new().background(WHITE)))
        .child(
            block(Style::new().width(40.0).height(40.0), WHITE)
                .prop("scale", 2.0)
                .prop("transform_origin", "top left")
                .child(block(Style::new().width(10.0).height(10.0), RED)),
        )
        .child(block(Style::new().width(10.0).height(10.0), BLUE))
        .into_node();
    let mut fixture = Fixture::new();
    let scene = fixture.scene(&root, 1.0);
    assert_eq!(scene.transforms.len(), 1);
    assert_eq!(
        scene.batches.last().unwrap().transform,
        0,
        "what comes after isn't transformed"
    );
    let mut renderer = Renderer::offscreen(WINDOW).unwrap();
    renderer.render(scene, WHITE).unwrap();
    let pixels = renderer.read_pixels().unwrap();
    assert_eq!(
        pixel_at(&pixels, 15, 15),
        TURNED_RED,
        "10 by 10 is 20 by 20 now"
    );
    assert_eq!(pixel_at(&pixels, 25, 25), TURNED_WHITE);
    assert_eq!(
        pixel_at(&pixels, 45, 5),
        [0, 0, 255],
        "what's next to it is where it was laid out"
    );
}

#[test]
fn transforms_inside_each_other_add_up() {
    let root: Node = Element::new(Role::Window)
        .props(painted(Style::new(), Paint::new().background(WHITE)))
        .child(
            block(Style::new().width(10.0).height(10.0), RED)
                .prop("transform", "translate(20px, 0)")
                .child(
                    block(Style::new().width(10.0).height(10.0), BLUE)
                        .prop("transform", "translate(30px, 0)"),
                ),
        )
        .into_node();
    let pixels = drawn_with(&root);
    assert_eq!(pixel_at(&pixels, 25, 5), TURNED_RED);
    assert_eq!(pixel_at(&pixels, 55, 5), [0, 0, 255], "20 and 30 is 50");
    assert_eq!(pixel_at(&pixels, 5, 5), TURNED_WHITE);
}

#[test]
fn a_frame_with_a_transform_is_drawn_again_when_only_that_changes() {
    let mut fixture = Fixture::new();
    let mut renderer = Renderer::offscreen(WINDOW).unwrap();
    let first = square_with("transform", "translateX(-50px)");
    renderer.render(fixture.scene(&first, 1.0), WHITE).unwrap();
    assert_eq!(
        pixel_at(&renderer.read_pixels().unwrap(), 50, 50),
        TURNED_RED
    );
    let second = square_with("transform", "translateX(50px)");
    renderer.render(fixture.scene(&second, 1.0), WHITE).unwrap();
    let pixels = renderer.read_pixels().unwrap();
    assert_eq!(pixel_at(&pixels, 150, 50), TURNED_RED);
    assert_eq!(
        pixel_at(&pixels, 50, 50),
        TURNED_WHITE,
        "where it was has been cleared"
    );
    // Back to nothing being transformed, which has to clear it too.
    let plain: Node = square_with("name_of_nothing", 1.0);
    renderer.render(fixture.scene(&plain, 1.0), WHITE).unwrap();
    let pixels = renderer.read_pixels().unwrap();
    assert_eq!(pixel_at(&pixels, 150, 50), TURNED_WHITE);
    assert_eq!(pixel_at(&pixels, 100, 50), TURNED_RED);
}

#[test]
fn what_scrolls_inside_something_resized_is_still_cut_off_at_its_edges() {
    let root: Node = Element::new(Role::Window)
        .props(painted(Style::new(), Paint::new().background(WHITE)))
        .child(
            Element::new(Role::Group)
                .props(painted(
                    Style::new().width(40.0).height(20.0).scroll_y(0.0),
                    Paint::new(),
                ))
                .prop("scale", 2.0)
                .prop("transform_origin", "top left")
                .child(block(Style::new().width(40.0).height(60.0), RED)),
        )
        .into_node();
    let pixels = drawn_with(&root);
    assert_eq!(
        pixel_at(&pixels, 40, 30),
        TURNED_RED,
        "20 down is 40 down now"
    );
    assert_eq!(
        pixel_at(&pixels, 40, 45),
        TURNED_WHITE,
        "past where it's cut off"
    );
}

#[test]
fn a_scrollbar_thumb_with_a_size_can_be_a_pill_with_room_round_it() {
    // What CSS's `::-webkit-scrollbar-thumb` says with `border-radius` and a transparent `border`: a 12 wide bar, its thumb 3 in
    // from every side of it, and as round as there's room for.
    let root: Node = Element::new(Role::Window)
        .props(painted(Style::new(), Paint::new().background(WHITE)))
        .child(
            Element::new(Role::Group)
                .props(painted(
                    Style::new().width(40.0).height(60.0).scroll_y(0.0),
                    Paint::new(),
                ))
                .prop("scrollbar_size", 12.0)
                .prop("scrollbar_inset", 3.0)
                .prop("scrollbar_radius", 999.0)
                .prop("scrollbar_thumb", 0x0000_FFFF_i64)
                .child(block(Style::new().width(40.0).height(300.0), WHITE)),
        )
        .into_node();
    let pixels = drawn_with(&root);
    let blue = [0, 0, 255];
    // The thumb is 24 long, from 3 down to 27, and 6 across, from 31 to 37.
    assert_eq!(pixel_at(&pixels, 34, 15), blue, "its middle");
    assert_eq!(pixel_at(&pixels, 29, 15), TURNED_WHITE, "room between it and the bar's edge");
    assert_eq!(pixel_at(&pixels, 38, 15), TURNED_WHITE, "room between it and the side");
    assert_eq!(pixel_at(&pixels, 34, 1), TURNED_WHITE, "room above it");
    assert_eq!(pixel_at(&pixels, 31, 3), TURNED_WHITE, "its corner is cut away");
}

/// Resource mutations must be compared using two independent, persistent GPU histories.
mod resource_damage {
    use rsui_render::{Batch, Color, ImageUpload, PixelRect, Quad, Renderer, Scene};
    use rsui_text::Upload;

    const SIZE: (u32, u32) = (64, 48);
    const CLEAR: Color = Color::hex(0xE4E8EC);
    const IMAGE: u32 = 7;

    struct Pair {
        candidate: Renderer,
        reference: Renderer,
        previous: Option<Vec<u8>>,
        failures: Vec<String>,
        flipping: bool,
    }

    impl Pair {
        fn new(flipping: bool) -> Self {
            let make = || {
                if flipping {
                    Renderer::offscreen_flipping(SIZE)
                } else {
                    Renderer::offscreen(SIZE)
                }
                .unwrap()
            };
            let mut reference = make();
            reference.set_always_redraw(true);
            Self {
                candidate: make(),
                reference,
                previous: None,
                failures: Vec::new(),
                flipping,
            }
        }

        // Build each scene separately: draw consumes uploads and swaps command lists.
        fn frame(&mut self, label: &str, changed: bool, make: impl Fn() -> Scene) {
            self.reference.render(&mut make(), CLEAR).unwrap();
            let expected = self.reference.read_pixels().unwrap();
            if let Some(previous) = &self.previous {
                assert_eq!(
                    expected != *previous,
                    changed,
                    "reference fixture: {label}, flipping={}",
                    self.flipping
                );
            }
            self.candidate.render(&mut make(), CLEAR).unwrap();
            let actual = self.candidate.read_pixels().unwrap();
            assert_eq!(actual.len(), expected.len(), "complete RGBA buffers");
            let different = actual.iter().zip(&expected).filter(|(a, b)| a != b).count();
            if different != 0 {
                self.failures.push(format!(
                    "{label}, flipping={}: {different} differing RGBA bytes",
                    self.flipping
                ));
            }
            if self.previous.is_some() && self.candidate.showed_nothing() == changed {
                self.failures.push(format!(
                    "{label}, flipping={}: unexpected showed_nothing={}",
                    self.flipping,
                    self.candidate.showed_nothing()
                ));
            }
            self.previous = Some(expected);
        }
    }

    fn scene(image: bool, transformed: bool, moved: bool) -> Scene {
        let clip = PixelRect {
            left: 0,
            top: 0,
            right: 64,
            bottom: 48,
        };
        let quad = |rect, color| Quad {
            rect,
            color,
            glyph: [0; 4],
            radius: 0.0,
        };
        let mut scene = Scene {
            quads: vec![
                quad([3.0, 3.0, 53.0, 38.0], [30, 55, 90, 180]),
                Quad {
                    glyph: if image {
                        [0, 0, 65535, 65535]
                    } else {
                        [0, 0, 12, 10]
                    },
                    ..quad(
                        [8.0, 8.0, 12.0, 10.0],
                        if image { [255; 4] } else { [0, 0, 0, 255] },
                    )
                },
                quad([14.0, 12.0, 33.0, 18.0], [80, 20, 10, 110]),
                quad(
                    [if moved { 54.0 } else { 50.0 }, 38.0, 4.0, 4.0],
                    [0, 0, 0, 255],
                ),
            ],
            atlas_size: 16,
            ..Scene::default()
        };
        for first in 0..4 {
            scene.batches.push(Batch {
                first,
                count: 1,
                clip: if first == 1 {
                    PixelRect {
                        left: 10,
                        right: 43,
                        bottom: 29,
                        ..clip
                    }
                } else {
                    clip
                },
                image: if first == 1 && image { IMAGE } else { 0 },
                transform: u16::from(first == 1 && transformed),
                filter: 0,
                round: 0,
            });
        }
        if transformed {
            // Most of this image lies beyond its untransformed quad bounds.
            scene.transforms.push([1.5, 0.25, -0.25, 1.5, 14.0, 1.0]);
        }
        scene
    }

    fn upload(id: u32, blue: bool) -> ImageUpload {
        ImageUpload {
            id,
            width: 2,
            height: 2,
            pixels: if blue {
                [10, 30, 150, 180]
            } else {
                [150, 25, 10, 180]
            }
            .repeat(4),
            changes: None,
        }
    }

    /// A spinner: one quad turned a little more, and faded a little differently, at each frame, next to a quad that stays put,
    /// with a third turned once and left alone. Only what moved is drawn again, which has to come out as drawing it all does.
    #[test]
    fn what_turns_or_fades_at_each_frame_is_drawn_again_where_it_was_and_is() {
        let spinner = |frame: u32, fade: Option<f32>| {
            let whole = PixelRect {
                left: 0,
                top: 0,
                right: SIZE.0 as i32,
                bottom: SIZE.1 as i32,
            };
            let quad = |rect, color| Quad {
                rect,
                color,
                glyph: [0; 4],
                radius: 2.0,
            };
            let angle = frame as f32 * 0.4;
            let (sin, cos) = angle.sin_cos();
            // Turned about (20, 20).
            let turned = [cos, sin, -sin, cos, 20.0 - 20.0 * cos + 20.0 * sin, 20.0 - 20.0 * sin - 20.0 * cos];
            let mut scene = Scene {
                quads: vec![
                    quad([40.0, 30.0, 18.0, 12.0], [20, 120, 60, 255]),
                    quad([10.0, 16.0, 20.0, 8.0], [200, 30, 30, 230]),
                    quad([44.0, 4.0, 10.0, 10.0], [30, 30, 200, 255]),
                ],
                transforms: vec![turned, [0.8, 0.6, -0.6, 0.8, 30.0, -20.0]],
                atlas_size: 16,
                ..Scene::default()
            };
            let mut filter = 0;
            if let Some(opacity) = fade {
                scene.filters.push(rsui_render::Filter {
                    rows: [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
                    opacity,
                });
                filter = 1;
            }
            for (first, transform, filter) in [(0, 0, 0), (1, 1, filter), (2, 2, 0)] {
                scene.batches.push(Batch {
                    first,
                    count: 1,
                    clip: whole,
                    image: 0,
                    transform,
                    filter,
                    round: 0,
                });
            }
            scene
        };
        let mut failures = Vec::new();
        for flipping in [false, true] {
            let mut pair = Pair::new(flipping);
            pair.frame("first", true, || spinner(0, None));
            for frame in 1..8 {
                pair.frame(&format!("turned {frame}"), true, || spinner(frame, None));
            }
            pair.frame("the same again", false, || spinner(7, None));
            for frame in 8..12 {
                let fade = 0.3 + frame as f32 * 0.05;
                pair.frame(&format!("turned and faded {frame}"), true, || spinner(frame, Some(fade)));
            }
            pair.frame("faded the same", false, || spinner(11, Some(0.85)));
            pair.frame("only faded", true, || spinner(11, Some(0.5)));
            pair.frame("no longer faded", true, || spinner(11, None));
            failures.extend(pair.failures);
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn text_settings_change_fractional_glyph_pixels() {
        let mut failures = Vec::new();
        for flipping in [false, true] {
            let mut pair = Pair::new(flipping);
            let make = |look, initial, moved| {
                let mut scene = scene(false, false, moved);
                scene.text_look = look;
                if initial {
                    scene.uploads.push(Upload {
                        x: 0,
                        y: 0,
                        width: 12,
                        height: 10,
                        coverage: (0..120).map(|i| [48, 96, 144, 192][i % 4]).collect(),
                    });
                }
                scene
            };
            pair.frame("initial glyph", true, || make([1.0, 0.0], true, false));
            pair.frame("unchanged glyph", false, || make([1.0, 0.0], false, false));
            pair.frame("gamma only", true, || make([2.2, 0.0], false, false));
            pair.frame("unchanged gamma", false, || make([2.2, 0.0], false, false));
            pair.frame("small geometry after gamma", true, || {
                make([2.2, 0.0], false, true)
            });
            pair.frame("contrast only", true, || make([2.2, 0.8], false, true));
            pair.frame("unchanged contrast", false, || {
                make([2.2, 0.8], false, true)
            });
            pair.frame("reset look", true, || make([1.0, 0.0], false, true));
            pair.frame("unchanged reset", false, || make([1.0, 0.0], false, true));
            failures.extend(pair.failures);
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[derive(Clone, Copy)]
    enum Mutation {
        Replace,
        Arrive,
        Drop,
        DropAndUpload,
    }

    fn images(mutation: Mutation) {
        let mut failures = Vec::new();
        for flipping in [false, true] {
            for transformed in [false, true] {
                let mut pair = Pair::new(flipping);
                let make = |initial, mutate, moved| {
                    let mut scene = scene(true, transformed, moved);
                    if initial && !matches!(mutation, Mutation::Arrive) {
                        scene.image_uploads.push(upload(IMAGE, false));
                    }
                    if mutate {
                        if matches!(mutation, Mutation::Drop | Mutation::DropAndUpload) {
                            scene.dropped_images.push(IMAGE);
                        }
                        if !matches!(mutation, Mutation::Drop) {
                            scene.image_uploads.push(upload(IMAGE, true));
                        }
                    }
                    scene
                };
                pair.frame("initial image", true, || make(true, false, false));
                pair.frame("unchanged image", false, || make(false, false, false));
                pair.frame("resource mutation", true, || make(false, true, false));
                pair.frame("unchanged mutation", false, || make(false, false, false));
                pair.frame("small geometry after mutation", true, || {
                    make(false, false, true)
                });
                pair.frame("unchanged geometry", false, || make(false, false, true));
                pair.frame("subsequent upload", true, || {
                    let mut scene = make(false, false, true);
                    scene.image_uploads.push(upload(IMAGE, false));
                    scene
                });
                pair.frame("unchanged subsequent upload", false, || {
                    make(false, false, true)
                });
                failures.extend(
                    pair.failures
                        .into_iter()
                        .map(|s| format!("transformed={transformed}: {s}")),
                );
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn same_id_image_replacement() {
        images(Mutation::Replace);
    }
    #[test]
    fn missing_image_arrives() {
        images(Mutation::Arrive);
    }
    #[test]
    fn referenced_image_is_dropped() {
        images(Mutation::Drop);
    }
    #[test]
    fn referenced_image_is_dropped_and_uploaded() {
        images(Mutation::DropAndUpload);
    }

    #[test]
    fn unrelated_uploads_and_missing_drops_keep_the_no_draw_path() {
        let mut failures = Vec::new();
        for flipping in [false, true] {
            let mut pair = Pair::new(flipping);
            pair.frame("initial", true, || {
                let mut scene = scene(true, false, false);
                scene.image_uploads.push(upload(IMAGE, false));
                scene
            });
            for round in 0..6 {
                pair.frame("unreferenced upload/drop", false, || {
                    let mut scene = scene(true, false, false);
                    scene.image_uploads.push(upload(99, round % 2 == 0));
                    scene.dropped_images.extend([99, 100]);
                    scene
                });
                pair.frame("unchanged", false, || scene(true, false, false));
            }
            pair.frame("later geometry", true, || scene(true, false, true));
            pair.frame("unchanged later geometry", false, || {
                scene(true, false, true)
            });
            failures.extend(pair.failures);
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn rejected_upload_preserves_the_old_image_unless_dropped() {
        let mut failures = Vec::new();
        for flipping in [false, true] {
            let mut pair = Pair::new(flipping);
            pair.frame("initial", true, || {
                let mut scene = scene(true, false, false);
                scene.image_uploads.push(upload(IMAGE, false));
                scene
            });
            let rejected = |drop| {
                let mut scene = scene(true, false, false);
                if drop {
                    scene.dropped_images.push(IMAGE);
                }
                // A zero-width texture is rejected; its payload is correctly empty.
                scene.image_uploads.push(ImageUpload {
                    id: IMAGE,
                    width: 0,
                    height: 1,
                    pixels: Vec::new(),
                    changes: None,
                });
                scene
            };
            pair.frame("rejected replacement", false, || rejected(false));
            pair.frame("unchanged after rejection", false, || {
                scene(true, false, false)
            });
            pair.frame("drop then rejected upload", true, || rejected(true));
            pair.frame("drop already missing image", false, || rejected(true));
            pair.frame("successful retry", true, || {
                let mut scene = scene(true, false, false);
                scene.image_uploads.push(upload(IMAGE, true));
                scene
            });
            pair.frame("unchanged retry", false, || scene(true, false, false));
            pair.frame("later geometry", true, || scene(true, false, true));
            pair.frame("unchanged later geometry", false, || {
                scene(true, false, true)
            });
            failures.extend(pair.failures);
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
