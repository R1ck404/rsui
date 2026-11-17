//! Reuse must preserve fresh layout and leave bounded metadata behind.
#![cfg(feature = "render-diagnostics")]
#[path = "../../rsui-text/tests/support/mod.rs"]
mod font;
use rsui_core::{Element, Node, Role};
use rsui_layout::{Layout, Size};
use rsui_render::{Color, Interaction, Painter, Renderer};
use rsui_text::{Font, TextEngine};

fn paragraph(words: &str, step: usize) -> Element {
    let mut p = Element::new(Role::Text)
        .prop("width", if step % 5 == 1 { 130 } else { 145 })
        .prop("height", 48)
        .prop("font_size", if step % 5 == 2 { 16 } else { 12 })
        .prop("letter_spacing", if step % 7 == 3 { 0.25 } else { 0.0 })
        .prop("text_align", if step % 5 == 3 { "right" } else { "left" })
        .prop("no_wrap", step % 7 == 4)
        .prop(
            "text_color",
            if step % 2 == 0 {
                0x204060ff_i64
            } else {
                0x904020ff_i64
            },
        );
    let a = Element::new(Role::Text)
        .name(words)
        .prop("padding_left", if step % 5 == 4 { 3.0 } else { 0.0 });
    let b = Element::new(Role::Text)
        .name(" ABCD End ")
        .prop("font_size", 15);
    p = if step % 7 == 6 {
        p.child(b).child(a)
    } else {
        p.child(a).child(b)
    };
    p
}
fn tree(words: &str, step: usize, count: usize) -> Node {
    let mut group = Element::new(Role::Group)
        .prop("clip", true)
        .prop("height", 116)
        .prop("scroll_y", (step % 3) as f64 * 2.25);
    if step % 6 == 2 {
        group = group.prop("transform", "rotate(4deg)");
    }
    if step % 6 == 3 {
        group = group.prop("opacity", 0.7);
    }
    for _ in 0..count {
        group = group.child(paragraph(words, step));
    }
    Element::new(Role::Window).child(group).into_node()
}
fn arrange(root: &Node, text: &mut TextEngine, scale: f32) -> Layout {
    let mut layout = Layout::new();
    layout.compute(root, Size::new(180.0 / scale, 128.0 / scale), text);
    layout
}
fn paint(painter: &mut Painter, root: &Node, layout: &Layout, text: &mut TextEngine) {
    painter.paint_with(root, layout, text, 1.0, (180, 128), Interaction::default());
}

#[test]
fn unchanged_paragraph_reuses_but_public_scratch_access_and_resets_rebuild() {
    let root = tree(&"ABCD EFGH ".repeat(32), 0, 1);
    let mut text = TextEngine::new(font::plain_font());
    let layout = arrange(&root, &mut text, 1.0);
    let mut painter = Painter::new();
    paint(&mut painter, &root, &layout, &mut text);
    assert_eq!(painter.retained_storage().paragraph_key_capacity_bytes, 0);
    for _ in 0..2 {
        paint(&mut painter, &root, &layout, &mut text);
    }
    assert_eq!(painter.scene_work().paragraph_reuses, 1);
    assert_eq!(painter.scene_work().paragraph_layouts, 0);
    let expected = painter.last_scene().quads.clone();
    let mut scratch = text.take_rich();
    scratch.layout.glyphs.clear();
    scratch.layout.pieces.clear();
    scratch.layout.lines.clear();
    text.give_back_rich(scratch);
    paint(&mut painter, &root, &layout, &mut text);
    assert_eq!(painter.scene_work().paragraph_layouts, 1);
    assert_eq!(painter.last_scene().quads, expected);
    paint(&mut painter, &root, &layout, &mut text);
    assert_eq!(painter.scene_work().paragraph_reuses, 1);
    text.forget_caches();
    paint(&mut painter, &root, &layout, &mut text);
    assert_eq!(painter.scene_work().paragraph_layouts, 1);
    text = text.with_lazy_fallback("missing-014-test-font.ttf", &[0x400..=0x4ff]);
    paint(&mut painter, &root, &layout, &mut text);
    assert_eq!(painter.scene_work().paragraph_layouts, 1);
    text = text.with_bold(font::plain_font());
    paint(&mut painter, &root, &layout, &mut text);
    assert_eq!(painter.scene_work().paragraph_layouts, 1);
    text = TextEngine::new(font::plain_font());
    paint(&mut painter, &root, &layout, &mut text);
    assert_eq!(painter.scene_work().paragraph_layouts, 1);
}

#[test]
fn memory_is_bounded_and_released_for_multiple_paragraphs_oversize_and_idle() {
    let mut text = TextEngine::new(font::plain_font());
    let mut painter = Painter::new();
    for (words, count, cached) in [
        ("ABCD ".repeat(1600), 1, true),
        ("ABCD ".repeat(32), 2, false),
        ("ABCD ".repeat(32), 1, true),
        ("ABCD ".repeat(7000), 1, false),
        ("ABCD ".repeat(32), 1, true),
    ] {
        let root = tree(&words, 0, count);
        let layout = arrange(&root, &mut text, 1.0);
        for _ in 0..3 {
            paint(&mut painter, &root, &layout, &mut text);
        }
        let bytes = painter.retained_storage().paragraph_key_capacity_bytes;
        println!(
            "KEY_BYTES text={} paragraphs={count} retained={bytes}",
            words.len()
        );
        assert!(bytes <= 32768);
        assert_eq!(bytes > 0, cached);
        assert_eq!(painter.scene_work().paragraph_reuses > 0, cached);
    }
    painter.forget_caches();
    assert_eq!(painter.retained_storage().paragraph_key_capacity_bytes, 0);
    let root = tree(&"ABCD ".repeat(32), 0, 1);
    let layout = arrange(&root, &mut text, 1.0);
    paint(&mut painter, &root, &layout, &mut text);
    assert_eq!(painter.retained_storage().paragraph_key_capacity_bytes, 0);
    let root = tree("no paragraph", 0, 0);
    let layout = arrange(&root, &mut text, 1.0);
    paint(&mut painter, &root, &layout, &mut text);
    assert_eq!(painter.retained_storage().paragraph_key_capacity_bytes, 0);
}

#[test]
fn cached_layout_matches_full_paint_across_text_style_geometry_and_language_changes() {
    for system_font in [false, true] {
        for flipping in [false, true] {
            let engine = || {
                TextEngine::new(if system_font {
                    Font::open("C:/Windows/Fonts/segoeui.ttf").unwrap()
                } else {
                    font::plain_font()
                })
            };
            let (mut text, mut reference_text) = (engine(), engine());
            let (mut painter, mut reference) = (Painter::new(), Painter::new());
            let renderer = || {
                if flipping {
                    Renderer::offscreen_flipping((180, 128))
                } else {
                    Renderer::offscreen((180, 128))
                }
                .unwrap()
            };
            let (mut gpu, mut full) = (renderer(), renderer());
            full.set_always_redraw(true);
            for step in 0..20 {
                let words = if system_font {
                    "ABC \u{5e9}\u{5dc}\u{5d5}\u{5dd} 123 \u{645}\u{631}\u{62d}\u{628}\u{627} e\u{301} ".repeat(24 + step % 3)
                } else {
                    "ABCD EFGH ".repeat(24 + step % 3)
                };
                let root = tree(&words, step, 1);
                let scale = [1.0, 1.25, 1.5][step % 3];
                let layout = arrange(&root, &mut text, scale);
                let fresh = arrange(&root, &mut reference_text, scale);
                for pass in 0..3 {
                    let interaction = if pass == 1 {
                        Interaction {
                            text_selection: Some((
                                &root.children[0].children[0],
                                (2, words.len() - 1),
                            )),
                            ..Default::default()
                        }
                    } else {
                        Interaction::default()
                    };
                    let scene = painter.paint_with(
                        &root,
                        &layout,
                        &mut text,
                        scale,
                        (180, 128),
                        interaction,
                    );
                    let other = reference.paint_full_for_diagnostics(
                        &root,
                        &fresh,
                        &mut reference_text,
                        scale,
                        (180, 128),
                        interaction,
                    );
                    gpu.render(scene, Color::WHITE).unwrap();
                    full.render(other, Color::WHITE).unwrap();
                    assert_eq!(
                        gpu.read_pixels().unwrap(),
                        full.read_pixels().unwrap(),
                        "system={system_font} flipping={flipping} step={step} pass={pass}"
                    );
                    assert_eq!(reference.scene_work().paragraph_reuses, 0);
                }
                assert_eq!(painter.scene_work().paragraph_reuses, 1, "warm step {step}");
                if step % 4 == 3 {
                    painter.forget_caches();
                    text.forget_caches();
                }
            }
        }
    }
}
