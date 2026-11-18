//! Exact independent-GPU reference for clipped descendant pruning.
#![cfg(feature = "render-diagnostics")]
#[path = "../../rsui-text/tests/support/mod.rs"]
mod font;
use rsui_core::{Element, Node, Role, diff};
use rsui_layout::{Layout, Size};
use rsui_render::{Color, Interaction, Painter, Renderer};
use rsui_text::TextEngine;

fn tree(step: usize, special: &str) -> Node {
    let mut rows = Element::new(Role::Group);
    for i in 0..24 {
        let mut child = Element::new(Role::Group)
            .prop("height", 32.0)
            .prop(
                "background",
                if step % 2 == 0 {
                    0xA0B0C0FF_i64
                } else {
                    0xE0A0B0FF_i64
                },
            )
            .child(
                Element::new(Role::Text)
                    .name(if step % 2 == 0 { "AB" } else { "CD" })
                    .prop("font_size", 10.0),
            );
        if i == 15 {
            child = match special {
                // A visible descendant of an offscreen, overflow-visible row.
                "overflow" => child.prop("margin_top", -450.0),
                "transform" => child.prop("transform", "translate(0px, -450px)"),
                "absolute" => child.prop("position", "absolute").prop("top", -450.0),
                "relative" => child.prop("position", "relative").prop("top", -450.0),
                "shadow" => child
                    .prop("shadow_blur", 10.0)
                    .prop("shadow_color", 0x000000FF_i64),
                "overlay" => child.prop("overlay", "above"),
                "filter" => child.prop("css_filter", "brightness(0.5)"),
                "opacity" => child.prop("opacity", 0.5),
                "unknown" => child.prop("future_paint_effect", true),
                _ => child,
            };
        }
        rows = rows.child(
            Element::new(Role::Group)
                .key(format!("{i}"))
                .prop("height", 32.0)
                .prop("clip", special != "overflow")
                .prop("text_color", 0x204060FF_i64)
                .child(child),
        );
    }
    Element::new(Role::Window)
        .child(
            Element::new(Role::Group)
                .prop("height", 96.0)
                .prop("scroll_y", (step % 5) as f64 * 26.25)
                .child(rows),
        )
        .into_node()
}
#[test]
fn clips_match_unchanged_traversal_across_history_and_fresh_layouts() {
    for flipping in [false, true] {
        for special in [
            "plain",
            "overflow",
            "transform",
            "absolute",
            "relative",
            "shadow",
            "overlay",
            "filter",
            "opacity",
            "unknown",
        ] {
            let mut root = tree(0, special);
            let mut layout = Layout::new();
            let mut text = TextEngine::new(font::plain_font());
            let mut reference_text = TextEngine::new(font::plain_font());
            let mut painter = Painter::new();
            let mut reference = Painter::new();
            let make = || {
                if flipping {
                    Renderer::offscreen_flipping((160, 96))
                } else {
                    Renderer::offscreen((160, 96))
                }
                .unwrap()
            };
            let mut gpu = make();
            let mut full = make();
            full.set_always_redraw(true);
            for step in 0..8 {
                // Change eligibility without changing box dimensions, then restore it.
                let next = tree(
                    step,
                    if step == 3 || step == 4 {
                        "plain"
                    } else {
                        special
                    },
                );
                let patches = diff(&root, &next);
                layout.apply(&patches);
                root = next;
                let size = (160 + (step % 2) as u32 * 8, 96);
                let available = Size::new(size.0 as f32 / 1.25, size.1 as f32 / 1.25);
                layout.compute(&root, available, &mut text);
                let mut fresh = Layout::new();
                fresh.compute(&root, available, &mut reference_text);
                gpu.resize(size).unwrap();
                full.resize(size).unwrap();
                let scene = painter.paint(&root, &layout, &mut text, 1.25, size);
                let other = reference.paint_full_for_diagnostics(
                    &root,
                    &fresh,
                    &mut reference_text,
                    1.25,
                    size,
                    Interaction::default(),
                );
                assert_eq!(scene.quads, other.quads, "quads {special} {step}");
                assert_eq!(scene.batches, other.batches, "batches {special} {step}");
                assert_eq!(scene.transforms, other.transforms);
                assert_eq!(scene.filters, other.filters);
                assert_eq!(scene.round_clips, other.round_clips);
                gpu.render(scene, Color::WHITE).unwrap();
                full.render(other, Color::WHITE).unwrap();
                assert_eq!(
                    gpu.read_pixels().unwrap(),
                    full.read_pixels().unwrap(),
                    "pixels {special} {step} {flipping}"
                );
                if special == "plain" {
                    assert!(painter.scene_work().node_enters < reference.scene_work().node_enters);
                }
            }
        }
    }
}
