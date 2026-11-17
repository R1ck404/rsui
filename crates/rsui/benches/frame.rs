//! What a frame costs the CPU before the GPU gets involved, and what input costs, using real
//! fonts on `rsui-alloc`, as an application would run them. Needs no window.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use rsui::prelude::*;
use rsui::{App, KeyCode, KeyPress, default_text_engine};

mod workload;

#[global_allocator]
static ALLOCATOR: rsui::UiAlloc = rsui::UiAlloc;

const SIZE: (u32, u32) = (1280, 800);

fn app<P: 'static, R: IntoNode + 'static>(render: fn(&mut Cx<'_>, &P) -> R, props: P) -> App {
    let text = default_text_engine().expect("the system font loads");
    let mut app = App::new(render, props, text, SIZE, 1.0);
    app.update();
    app.scene();
    app
}

fn paint(c: &mut Criterion) {
    let mut group = c.benchmark_group("paint");
    let mut dashboard = app(workload::dashboard, ());
    group.bench_function("dashboard", |b| {
        b.iter(|| black_box(dashboard.scene().quads.len()))
    });
    let mut labels = app(workload::labels, 2_000usize);
    group.bench_function("labels_2k", |b| {
        b.iter(|| black_box(labels.scene().quads.len()))
    });
    let mut boxes = app(workload::boxes, 10_000usize);
    group.bench_function("boxes_10k", |b| {
        b.iter(|| black_box(boxes.scene().quads.len()))
    });
    let mut paragraph = app(workload::paragraph, 100_000usize);
    group.bench_function("paragraph_100k", |b| {
        b.iter(|| black_box(paragraph.scene().quads.len()))
    });
    let mut plain_page = app(workload::page, false);
    group.bench_function("page_of_plain_text", |b| {
        b.iter(|| black_box(plain_page.scene().quads.len()))
    });
    let mut rich_page = app(workload::page, true);
    group.bench_function("page_with_parts", |b| {
        b.iter(|| black_box(rich_page.scene().quads.len()))
    });
    group.finish();
}

fn layout(c: &mut Criterion) {
    let mut group = c.benchmark_group("layout_real_text");
    group.sample_size(20);
    let mut labels = app(workload::labels, 2_000usize);
    let mut wide = true;
    group.bench_function("resize_labels_2k", |b| {
        b.iter(|| {
            wide = !wide;
            labels.resize(if wide { SIZE } else { (1000, 800) }, 1.0);
            black_box(labels.scene().quads.len())
        })
    });
    let mut plain_page = app(workload::page, false);
    let mut wide = true;
    group.bench_function("resize_page_of_plain_text", |b| {
        b.iter(|| {
            wide = !wide;
            plain_page.resize(if wide { SIZE } else { (1000, 800) }, 1.0);
            black_box(plain_page.scene().quads.len())
        })
    });
    let mut rich_page = app(workload::page, true);
    let mut wide = true;
    group.bench_function("resize_page_with_parts", |b| {
        b.iter(|| {
            wide = !wide;
            rich_page.resize(if wide { SIZE } else { (1000, 800) }, 1.0);
            black_box(rich_page.scene().quads.len())
        })
    });
    let mut dashboard = app(workload::dashboard, ());
    group.bench_function("resize_dashboard", |b| {
        b.iter(|| {
            wide = !wide;
            dashboard.resize(if wide { SIZE } else { (1000, 800) }, 1.0);
            black_box(dashboard.scene().quads.len())
        })
    });
    group.finish();
}

fn pointer(c: &mut Criterion) {
    let mut group = c.benchmark_group("pointer");
    let mut dashboard = app(workload::dashboard, ());
    let mut x = 300.0;
    group.bench_function("move_within_dashboard", |b| {
        b.iter(|| {
            x = if x > 900.0 { 300.0 } else { x + 1.0 };
            dashboard.pointer_move(x, 200.0);
            black_box(dashboard.update())
        })
    });
    let mut labels = app(workload::labels, 10_000usize);
    group.bench_function("move_over_10k_labels", |b| {
        b.iter(|| {
            x = if x > 900.0 { 300.0 } else { x + 1.0 };
            labels.pointer_move(x, 700.0);
            black_box(labels.update())
        })
    });
    group.bench_function("click_row_and_paint", |b| {
        b.iter(|| {
            dashboard.click(700.0, 300.0).unwrap();
            dashboard.update();
            black_box(dashboard.scene().quads.len())
        })
    });
    group.finish();
}

fn typing(c: &mut Criterion) {
    let mut group = c.benchmark_group("typing");
    group.sample_size(20);
    for length in [20usize, 10_000, 1_000_000] {
        let mut input = app(workload::big_input, length);
        input.key(KeyPress::new(KeyCode::Tab)).unwrap();
        input.key(KeyPress::new(KeyCode::End)).unwrap();
        input.update();
        input.scene();
        group.bench_function(format!("key_in_{length}"), |b| {
            b.iter_batched(
                || (),
                |()| {
                    input.type_text("x").unwrap();
                    input.key(KeyPress::new(KeyCode::Backspace)).unwrap();
                    input.update();
                    black_box(input.scene().quads.len())
                },
                BatchSize::PerIteration,
            )
        });
    }
    group.finish();
}

/// A notch of the wheel, as far as what's drawn for it, on something that holds a lot.
fn scrolling(c: &mut Criterion) {
    /// One label in a row of its own, in a component that doesn't render again as things scroll.
    fn label(_: &mut Cx, index: &usize) -> Element {
        Element::new(Role::Text).name(text!("Row number {index}"))
    }
    fn area(cx: &mut Cx, count: &usize) -> Element {
        let mut content = Element::new(Role::Group).props(style!(Style::new().flex(1.0)));
        for index in 0..*count {
            content = content.child(cx.keyed(index, label, index));
        }
        Element::new(Role::Window).child(cx.scroll_area("area", ScrollAxis::Vertical, content))
    }
    let mut group = c.benchmark_group("scrolling");
    for count in [100usize, 1_000, 10_000] {
        let mut scrolling = app(area, count);
        let mut down = true;
        group.bench_function(format!("notch_with_{count}_rows"), |b| {
            b.iter(|| {
                let rows = if down { 3 } else { -3 };
                down = !down;
                scrolling.scroll(100.0, 100.0, rows).unwrap();
                scrolling.update();
                black_box(scrolling.scene().quads.len())
            })
        });
    }
    group.finish();
}

/// Text that needs shaping, against plain text of about the same length.
fn shaping(c: &mut Criterion) {
    const ARABIC: &str = "\u{0627}\u{0644}\u{0639}\u{0631}\u{0628}\u{064A}\u{0629} \u{0644}\u{063A}\u{0629} \u{062C}\u{0645}\u{064A}\u{0644}\u{0629} \u{0644}\u{0627} \u{0625}\u{0644}\u{0647}";
    const HEBREW: &str = "\u{05D9}\u{05E8}\u{05D5}\u{05E9}\u{05DC}\u{05D9}\u{05DD} \u{05E9}\u{05DC} \u{05D6}\u{05D4}\u{05D1} \u{05E9}\u{05DC}\u{05D5}\u{05DD}";
    const LIGATURES: &str = "office fish flag waffle";
    const PLAIN: &str = "The quick brown bear ran";
    let style = rsui::text::TextStyle::new(14.0);
    let mut group = c.benchmark_group("shaping");
    let mut text = default_text_engine().expect("the system font loads");
    let mut layout = rsui::text::TextLayout::default();
    for (name, sample) in [
        ("plain", PLAIN),
        ("ligatures", LIGATURES),
        ("hebrew", HEBREW),
        ("arabic", ARABIC),
    ] {
        group.bench_function(format!("{name}_first_time"), |b| {
            b.iter(|| {
                text.forget_caches();
                text.layout_into(black_box(sample), style, 400.0, &mut layout);
                black_box(layout.glyphs.len())
            })
        });
        group.bench_function(format!("{name}_again"), |b| {
            b.iter(|| {
                text.layout_into(black_box(sample), style, 400.0, &mut layout);
                black_box(layout.glyphs.len())
            })
        });
    }
    let page = format!("{ARABIC} ").repeat(2_500);
    group.sample_size(10);
    group.bench_function("arabic_page_of_100k_first_time", |b| {
        b.iter(|| {
            text.forget_caches();
            text.layout_visible_into(&page, style, 800.0, (0.0, 800.0), &mut layout);
            black_box(layout.glyphs.len())
        })
    });
    group.sample_size(30);
    group.bench_function("arabic_page_of_100k_again", |b| {
        b.iter(|| {
            text.layout_visible_into(&page, style, 800.0, (0.0, 800.0), &mut layout);
            black_box(layout.glyphs.len())
        })
    });
    group.finish();
}

fn rendering_local_change(c: &mut Criterion) {
    use workload::measured as m;
    let mut group = c.benchmark_group("rendering_local_change");
    group
        .sample_size(100)
        .warm_up_time(std::time::Duration::from_secs(3))
        .measurement_time(std::time::Duration::from_secs(5));
    for count in [100, 1000, 10000] {
        for (name, text) in [("color", false), ("text", true)] {
            m::check_local(count, text);
            let mut app = m::local(count, text);
            group.bench_function(format!("{name}_{count}_update_scene"), |b| {
                b.iter(|| {
                    m::toggle(&mut app);
                    app.update();
                    black_box(m::guard(app.scene()))
                })
            });
            // Per-iteration setup excludes dispatch/update and does not clone App.
            let app = std::cell::RefCell::new(app);
            group.bench_function(format!("{name}_{count}_scene_only"), |b| {
                b.iter_batched(
                    || {
                        let mut a = app.borrow_mut();
                        m::toggle(&mut a);
                        a.update();
                    },
                    |()| black_box(m::guard(app.borrow_mut().scene())),
                    BatchSize::PerIteration,
                )
            });
        }
    }
    group.finish();
}
fn rendering_blink(c: &mut Criterion) {
    use workload::measured as m;
    m::check_blink();
    let mut group = c.benchmark_group("rendering_blink");
    group
        .sample_size(100)
        .warm_up_time(std::time::Duration::from_secs(3))
        .measurement_time(std::time::Duration::from_secs(5));
    for (name, reuse) in [("first_preparation", false), ("warmed_reuse", true)] {
        let app = std::cell::RefCell::new(m::input());
        group.bench_function(name, |b| {
            b.iter_batched(
                || m::prepare_blink(&mut app.borrow_mut(), reuse),
                |()| {
                    let mut a = app.borrow_mut();
                    a.blink();
                    a.update();
                    black_box(m::guard(a.scene()))
                },
                BatchSize::PerIteration,
            )
        });
    }
    group.finish();
}
criterion_group!(
    benches,
    paint,
    layout,
    pointer,
    typing,
    shaping,
    scrolling,
    rendering_local_change,
    rendering_blink
);

criterion_main!(benches);
