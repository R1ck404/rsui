//! Tracks what text costs, with a real font, on `rsui-alloc`.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use rsui_core::{Element, Node, Role, text};
use rsui_layout::{Layout, Size};
use rsui_text::{Font, GlyphAtlas, GlyphKey, Rasterizer, TextEngine, TextLayout, TextStyle};

#[global_allocator]
static ALLOCATOR: rsui_alloc::UiAlloc = rsui_alloc::UiAlloc;

const FONT: &str = "C:/Windows/Fonts/segoeui.ttf";
const LABEL: &str = "Save changes";
const PARAGRAPH: &str = "The quick brown fox jumps over the lazy dog, and then keeps on going for \
quite some time, past the farm and over the river, until it comes to a very long sentence that \
has to wrap onto several lines of text in the window.";

const ENGLISH: &str =
    "It's the settings you're looking for, and they're all here for you to change";
const CURLY: &str = "It\u{2019}s the settings you\u{2019}re looking for \u{2014} and they\u{2019}re all here\u{2026}";
const CHINESE: &str = "\u{8FD9}\u{662F}\u{4F60}\u{6B63}\u{5728}\u{5BFB}\u{627E}\u{7684}\u{8BBE}\u{7F6E}\u{FF0C}\u{5B83}\u{4EEC}\u{90FD}\u{5728}\u{8FD9}\u{91CC}\u{7B49}\u{4F60}\u{6765}\u{66F4}\u{6539}";
const JAPANESE: &str = "\u{3053}\u{308C}\u{306F}\u{3042}\u{306A}\u{305F}\u{304C}\u{63A2}\u{3057}\u{3066}\u{3044}\u{308B}\u{8A2D}\u{5B9A}\u{3067}\u{3059}\u{3001}\u{3059}\u{3079}\u{3066}\u{3053}\u{3053}\u{306B}\u{3042}\u{308A}\u{307E}\u{3059}";
const ARABIC: &str = "\u{0647}\u{0630}\u{0647} \u{0647}\u{064A} \u{0627}\u{0644}\u{0625}\u{0639}\u{062F}\u{0627}\u{062F}\u{0627}\u{062A} \u{0627}\u{0644}\u{062A}\u{064A} \u{062A}\u{0628}\u{062D}\u{062B} \u{0639}\u{0646}\u{0647}\u{0627}";

fn benches(c: &mut Criterion) {
    if !std::path::Path::new(FONT).exists() {
        return;
    }
    c.bench_function("open_font", |b| b.iter(|| Font::open(FONT).unwrap()));

    let style = TextStyle::new(14.0);
    let mut engine = TextEngine::new(Font::open(FONT).unwrap());
    let mut group = c.benchmark_group("text");
    group.bench_function("measure_label", |b| {
        b.iter(|| engine.measure(black_box(LABEL), style, f32::INFINITY))
    });
    group.bench_function("measure_paragraph_wrapped", |b| {
        b.iter(|| engine.measure(black_box(PARAGRAPH), style, 300.0))
    });
    group.bench_function("layout_paragraph_wrapped", |b| {
        b.iter(|| engine.layout(black_box(PARAGRAPH), style, 300.0))
    });
    group.finish();

    // Text that needs other fonts, on what would be every frame once they've been opened.
    let mut engine = TextEngine::new(Font::open(FONT).unwrap()).with_system_fallbacks();
    let mut group = c.benchmark_group("text_beyond_latin");
    let mut layout = TextLayout::default();
    for (name, text) in [
        ("english_control", ENGLISH),
        ("curly_quotes", CURLY),
        ("chinese", CHINESE),
        ("japanese", JAPANESE),
        ("arabic", ARABIC),
    ] {
        group.bench_function(name, |b| {
            b.iter(|| engine.layout_into(black_box(text), style, 600.0, &mut layout))
        });
    }
    group.finish();

    // Text as a frame sees it, with each of these on screen at once.
    let mut engine = TextEngine::new(Font::open(FONT).unwrap());
    let mut group = c.benchmark_group("text_on_screen");
    let view = (0.0, 600.0);
    let sized = |bytes: usize, seed: usize| -> String {
        let mut text = format!("{seed} ");
        while text.len() < bytes {
            text.push_str(PARAGRAPH);
            text.push(' ');
        }
        text
    };
    for (name, bytes, count) in [
        ("one_wrapped_300_bytes", 300, 1),
        ("one_wrapped_1500_bytes", 1500, 1),
        ("one_of_5000_bytes", 5000, 1),
        ("three_of_5000_bytes", 5000, 3),
        ("six_of_5000_bytes", 5000, 6),
        ("thirty_of_300_bytes", 300, 30),
    ] {
        let texts: Vec<String> = (0..count).map(|seed| sized(bytes, seed)).collect();
        group.bench_function(name, |b| {
            b.iter(|| {
                for text in &texts {
                    engine.layout_visible_into(black_box(text), style, 400.0, view, &mut layout);
                }
            })
        });
    }
    group.finish();

    let font = Font::open(FONT).unwrap();
    let glyph = font.glyph('g').unwrap();
    let mut rasterizer = Rasterizer::new();
    let mut group = c.benchmark_group("glyph");
    group.bench_function("rasterize_14px", |b| {
        b.iter(|| rasterizer.rasterize(&font, black_box(glyph), 14.0, 0.25))
    });
    group.bench_function("rasterize_48px", |b| {
        b.iter(|| rasterizer.rasterize(&font, black_box(glyph), 48.0, 0.25))
    });
    group.bench_function("atlas_hit", |b| {
        let mut atlas = GlyphAtlas::new();
        let key = GlyphKey::new(0, glyph, 14.0, 0.25);
        atlas.glyph(key, &font, &mut rasterizer);
        b.iter(|| atlas.glyph(black_box(key), &font, &mut rasterizer))
    });
    group.bench_function("atlas_fill_ascii", |b| {
        b.iter(|| {
            let mut atlas = GlyphAtlas::new();
            for character in '!'..='~' {
                let id = font.glyph(character).unwrap();
                atlas.glyph(GlyphKey::new(0, id, 14.0, 0.0), &font, &mut rasterizer);
            }
            atlas
        })
    });
    group.finish();

    let root: Node = Element::new(Role::List)
        .children((0..10_000).map(|row| Element::new(Role::Text).name(text!("row number {row}"))))
        .into_node();
    let mut group = c.benchmark_group("layout_10k_labels");
    group.bench_function("first_time_real_text", |b| {
        b.iter(|| {
            let mut layout = Layout::new();
            layout.compute(black_box(&root), Size::new(800.0, 600.0), &mut engine);
            layout
        })
    });
    group.finish();
}

criterion_group!(text, benches);
criterion_main!(text);
