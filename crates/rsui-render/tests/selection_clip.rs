//! Selection clipping must preserve pixels and avoid submitting invisible rectangles.
#[path = "../../rsui-text/tests/support/mod.rs"]
mod font;
use rsui_core::{Element, Node, Role};
use rsui_layout::{Layout, Size};
use rsui_render::{Editing, EditingLines, Interaction, Painter};
use rsui_text::TextEngine;
#[cfg(feature = "render-diagnostics")]
use rsui_text::TextStyle;

fn tree(kind: &str, step: usize, effect: &str) -> Node {
    let words = "ABCD EFGH IJKL MNOP QRST UVWX YZ ".repeat(80);
    let mut selected = if kind == "paragraph" {
        Element::new(Role::Text)
            .prop("selectable", true)
            .child(Element::new(Role::Text).name(words.clone()))
            .child(Element::new(Role::Text).name(words).prop("font_size", 14.0))
    } else {
        let input = Element::new(Role::TextInput)
            .prop("value", "ABCD EFGH\n".repeat(256))
            .prop("height", 62.0);
        if kind == "area" {
            input.prop("rows", 4)
        } else {
            input
        }
    };
    selected = selected.prop("font_size", 12.0).prop("width", 128.0);
    let mut group = Element::new(Role::Group)
        .prop("height", 72.0)
        .prop("width", 144.0)
        .prop("margin_left", 6.25)
        .prop("margin_top", 3.75)
        .prop("clip", true)
        .prop(
            "scroll_y",
            if kind == "paragraph" {
                step as f64 * 19.25
            } else {
                0.0
            },
        )
        .prop(
            "background",
            if step % 2 == 0 {
                0x204060ff_i64
            } else {
                0x503020ff_i64
            },
        );
    group = match effect {
        "rotate" => group.prop("transform", "rotate(17deg)"),
        "translate" => group.prop("transform", "translate(3.25px, -2.5px)"),
        "scale" => group.prop("transform", "scale(1.25)"),
        "opacity" => group.prop("opacity", 0.6),
        "filter" => group.prop("css_filter", "brightness(0.7)"),
        _ => group,
    };
    Element::new(Role::Window)
        .child(group.child(selected))
        .into_node()
}
fn selected(root: &Node) -> &Element {
    &root.children[0].children[0]
}
fn selection_count(scene: &rsui_render::Scene) -> usize {
    scene
        .quads
        .iter()
        .filter(|q| q.color[3] == 0x55 && q.glyph == [0; 4])
        .count()
}
#[test]
fn multiline_selection_submission_is_bounded_by_the_visible_height() {
    let root = tree("area", 0, "plain");
    let mut text = TextEngine::new(font::plain_font());
    let mut layout = Layout::new();
    layout.compute(&root, Size::new(160.0, 96.0), &mut text);
    let mut painter = Painter::new();
    let spans = vec![(0.0, 100.0); 256];
    let lines: Vec<_> = (0..256).collect();
    for scroll in [0.0, 0.25, 17.5, 1000.0] {
        let interaction = Interaction {
            editing: Some(Editing {
                element: selected(&root),
                selection: &spans,
                caret: Some(7.0),
                scroll: 0.0,
                lines: Some(EditingLines {
                    selection: &lines,
                    caret: 0,
                    scroll,
                }),
            }),
            ..Default::default()
        };
        let scene = painter.paint_with(&root, &layout, &mut text, 1.0, (160, 96), interaction);
        let count = selection_count(scene);
        assert!((1..=9).contains(&count), "visible selection count {count}");
    }
}
#[test]
fn multiline_selection_keeps_invalid_public_spans_on_hidden_lines() {
    let root = tree("area", 0, "plain");
    let mut text = TextEngine::new(font::plain_font());
    let mut layout = Layout::new();
    layout.compute(&root, Size::new(160.0, 96.0), &mut text);
    let mut painter = Painter::new();
    let spans = [
        (50.0, 10.0),
        (f32::NAN, 10.0),
        (0.0, f32::INFINITY),
        (-f32::MAX, f32::MAX),
        (0.0, 100.0),
    ];
    let interaction = Interaction {
        editing: Some(Editing {
            element: selected(&root),
            selection: &spans,
            caret: None,
            scroll: 0.0,
            lines: Some(EditingLines {
                selection: &[255; 5],
                caret: 0,
                scroll: 0.0,
            }),
        }),
        ..Default::default()
    };
    let scene = painter.paint_with(&root, &layout, &mut text, 1.0, (160, 96), interaction);
    assert_eq!(selection_count(scene), 4);
}

#[test]
fn single_line_selection_keeps_partial_edges_and_reversed_public_spans() {
    let root = tree("input", 0, "plain");
    let mut text = TextEngine::new(font::plain_font());
    let mut layout = Layout::new();
    layout.compute(&root, Size::new(160.0, 96.0), &mut text);
    let mut painter = Painter::new();
    let spans = [
        (-1000.0, -900.0),
        (-30.0, 15.0),
        (20.0, 60.0),
        (300.0, 350.0),
        (50.0, 10.0),
    ];
    let interaction = Interaction {
        editing: Some(Editing {
            element: selected(&root),
            selection: &spans,
            caret: None,
            scroll: 0.0,
            lines: None,
        }),
        ..Default::default()
    };
    let scene = painter.paint_with(&root, &layout, &mut text, 1.0, (160, 96), interaction);
    assert_eq!(selection_count(scene), 3);
}

#[cfg(feature = "render-diagnostics")]
#[test]
fn selection_clipping_matches_full_paint_across_scroll_scale_and_transforms() {
    use rsui_core::diff;
    use rsui_render::{Color, Renderer};
    for flipping in [false, true] {
        for kind in ["area", "input", "paragraph"] {
            for effect in ["plain", "rotate", "translate", "scale", "opacity", "filter"] {
                let make = || {
                    if flipping {
                        Renderer::offscreen_flipping((160, 96))
                    } else {
                        Renderer::offscreen((160, 96))
                    }
                    .unwrap()
                };
                let (mut gpu, mut full) = (make(), make());
                full.set_always_redraw(true);
                let (mut painter, mut reference) = (Painter::new(), Painter::new());
                let (mut text, mut reference_text) = (
                    TextEngine::new(font::plain_font()),
                    TextEngine::new(font::plain_font()),
                );
                let mut layout = Layout::new();
                let mut root = tree(kind, 0, effect);
                let mut saw_reduction = false;
                for step in 0..8 {
                    let next = tree(kind, step, effect);
                    layout.apply(&diff(&root, &next));
                    root = next;
                    let scale = [1.0, 1.25, 1.5][step % 3];
                    let size = (160 + (step % 2) as u32 * 8, 96);
                    let available = Size::new(size.0 as f32 / scale, size.1 as f32 / scale);
                    layout.compute(&root, available, &mut text);
                    let mut fresh = Layout::new();
                    fresh.compute(&root, available, &mut reference_text);
                    let spans: Vec<_> = if kind == "input" {
                        vec![
                            (-1000.0, -900.0),
                            (-30.0, 15.0),
                            (20.0, 60.0),
                            (300.0, 350.0),
                        ]
                    } else {
                        vec![(0.0, 100.0 * scale); 256]
                    };
                    let lines: Vec<_> = (0..256).collect();
                    let element = selected(&root);
                    let line_height =
                        text.line_height(TextStyle::from_element(element).scaled(scale));
                    let interaction = if step == 6 {
                        Interaction::default()
                    } else if kind == "paragraph" {
                        Interaction {
                            text_selection: Some((
                                element,
                                (if step % 2 == 0 { 0 } else { 9 }, 5000),
                            )),
                            ..Default::default()
                        }
                    } else {
                        Interaction {
                            editing: Some(Editing {
                                element,
                                selection: &spans,
                                caret: if step % 2 == 0 { Some(7.25) } else { None },
                                scroll: step as f32 * 0.25,
                                lines: (kind == "area").then_some(EditingLines {
                                    selection: &lines,
                                    caret: (step * 3) as u32,
                                    scroll: step as f32 * line_height + 0.25,
                                }),
                            }),
                            ..Default::default()
                        }
                    };
                    gpu.resize(size).unwrap();
                    full.resize(size).unwrap();
                    let scene =
                        painter.paint_with(&root, &layout, &mut text, scale, size, interaction);
                    let other = reference.paint_full_for_diagnostics(
                        &root,
                        &fresh,
                        &mut reference_text,
                        scale,
                        size,
                        interaction,
                    );
                    if matches!(effect, "rotate" | "translate" | "scale") {
                        assert_eq!(scene.quads, other.quads, "transformed fallback");
                    } else if step != 6 {
                        saw_reduction |= scene.quads.len() < other.quads.len();
                    }
                    gpu.render(scene, Color::WHITE).unwrap();
                    full.render(other, Color::WHITE).unwrap();
                    assert_eq!(
                        gpu.read_pixels().unwrap(),
                        full.read_pixels().unwrap(),
                        "{kind} {effect} step {step} flipping {flipping}"
                    );
                }
                if !matches!(effect, "rotate" | "translate" | "scale") {
                    assert!(saw_reduction, "{kind} {effect}");
                }
            }
        }
    }
}

#[cfg(feature = "render-diagnostics")]
#[test]
fn selection_submission_and_capacity_census() {
    use rsui_render::{Color, Renderer};
    for kind in ["area", "paragraph"] {
        let root = tree(kind, 0, "plain");
        let spans = vec![(0.0, 100.0); 256];
        let lines: Vec<_> = (0..256).collect();
        let element = selected(&root);
        let interaction = if kind == "paragraph" {
            Interaction {
                text_selection: Some((element, (0, usize::MAX))),
                ..Default::default()
            }
        } else {
            Interaction {
                editing: Some(Editing {
                    element,
                    selection: &spans,
                    caret: None,
                    scroll: 0.0,
                    lines: Some(EditingLines {
                        selection: &lines,
                        caret: 0,
                        scroll: 0.0,
                    }),
                }),
                ..Default::default()
            }
        };
        let mut observations = Vec::new();
        let mut pixels = Vec::new();
        for unpruned in [true, false] {
            let mut text = TextEngine::new(font::plain_font());
            let mut layout = Layout::new();
            layout.compute(&root, Size::new(160.0, 96.0), &mut text);
            let mut painter = Painter::new();
            let mut gpu = Renderer::offscreen((160, 96)).unwrap();
            gpu.set_always_redraw(true);
            let scene = if unpruned {
                painter.paint_full_for_diagnostics(
                    &root,
                    &layout,
                    &mut text,
                    1.0,
                    (160, 96),
                    interaction,
                )
            } else {
                painter.paint_with(&root, &layout, &mut text, 1.0, (160, 96), interaction)
            };
            let commands = (
                scene.quads.len(),
                scene.quads.capacity() * size_of::<rsui_render::Quad>(),
            );
            gpu.render(scene, Color::WHITE).unwrap();
            pixels.push(gpu.read_pixels().unwrap());
            let work = gpu.draw_work();
            let storage = gpu.retained_storage();
            observations.push((
                commands.0,
                commands.1,
                work.submitted_quads,
                work.quad_upload.bytes,
                storage.quad_buffer_capacity_bytes,
            ));
        }
        assert_eq!(pixels[0], pixels[1]);
        assert!(
            pixels[0].chunks_exact(4).any(|pixel| pixel != [255; 4]),
            "nonempty expected rendering"
        );
        let (before, after) = (observations[0], observations[1]);
        assert!(after.0 < before.0 && after.2 < before.2 && after.3 < before.3);
        assert!(after.1 <= before.1 && after.4 <= before.4);
        println!("013_CENSUS {kind} before={before:?} after={after:?}");
    }
}

#[cfg(feature = "render-diagnostics")]
#[test]
fn mixed_direction_selection_matches_full_paint_with_real_font() {
    use rsui_render::{Color, Renderer};
    use rsui_text::Font;
    let words="ABC \u{05e9}\u{05dc}\u{05d5}\u{05dd} 123 \u{0645}\u{0631}\u{062d}\u{0628}\u{0627} e\u{301} ".repeat(32);
    for scale in [1.0, 1.25, 1.5] {
        let make_text = || TextEngine::new(Font::open("C:/Windows/Fonts/segoeui.ttf").unwrap());
        let (mut text, mut reference_text) = (make_text(), make_text());
        let (mut painter, mut reference) = (Painter::new(), Painter::new());
        let (mut gpu, mut full) = (
            Renderer::offscreen_flipping((180, 96)).unwrap(),
            Renderer::offscreen_flipping((180, 96)).unwrap(),
        );
        full.set_always_redraw(true);
        for step in 0..8 {
            let paragraph = Element::new(Role::Text)
                .prop("width", 128.0)
                .prop("font_size", 12.0)
                .child(Element::new(Role::Text).name(words.clone()))
                .child(
                    Element::new(Role::Text)
                        .name(words.clone())
                        .prop("font_size", 18.0)
                        .prop("text_color", 0x204080ff_i64),
                );
            let root = Element::new(Role::Window)
                .child(
                    Element::new(Role::Group)
                        .prop("height", 72.0)
                        .prop("clip", true)
                        .prop("scroll_y", step as f64 * 15.75)
                        .child(paragraph),
                )
                .into_node();
            let mut layout = Layout::new();
            layout.compute(&root, Size::new(180.0 / scale, 96.0 / scale), &mut text);
            let mut fresh = Layout::new();
            fresh.compute(
                &root,
                Size::new(180.0 / scale, 96.0 / scale),
                &mut reference_text,
            );
            let interaction = Interaction {
                text_selection: Some((selected(&root), (0, words.len() * 2))),
                ..Default::default()
            };
            let scene =
                painter.paint_with(&root, &layout, &mut text, scale, (180, 96), interaction);
            let other = reference.paint_full_for_diagnostics(
                &root,
                &fresh,
                &mut reference_text,
                scale,
                (180, 96),
                interaction,
            );
            assert!(scene.quads.len() < other.quads.len());
            gpu.render(scene, Color::WHITE).unwrap();
            full.render(other, Color::WHITE).unwrap();
            assert_eq!(
                gpu.read_pixels().unwrap(),
                full.read_pixels().unwrap(),
                "scale {scale} step {step}"
            );
        }
    }
}
