//! Tracks what laying out costs, on `rsui-alloc`, as an application would run it.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use rsui_core::{ActionKind, Element, Node, Role, text};
use rsui_layout::{Align, FixedMetrics, Layout, Size, Style, style};
use rsui_runtime::{Cx, Runtime};

#[global_allocator]
static ALLOCATOR: rsui_alloc::UiAlloc = rsui_alloc::UiAlloc;

const WINDOW: Size = Size::new(1920.0, 1080.0);

fn row(cx: &mut Cx, id: &u64) -> Element {
    let clicks = cx.state(|| 0u32);
    let name = text!("row {id}: {}", cx.get(clicks));
    let label = Element::new(Role::Text)
        .name(name)
        .props(style!(Style::new().flex(1.0)));
    cx.on(
        Element::new(Role::ListItem)
            .props(style!(
                Style::row()
                    .padding_xy(8.0, 4.0)
                    .gap(8.0)
                    .align(Align::Center)
            ))
            .child(label),
        ActionKind::Click,
        move |cx, _| cx.update(clicks, |clicks| *clicks += 1),
    )
}

fn list(cx: &mut Cx, count: &u64) -> Element {
    let rows: Vec<Node> = (0..*count).map(|id| cx.keyed(id, row, id)).collect();
    Element::new(Role::List).key("list").children(rows)
}

fn benches(c: &mut Criterion) {
    let mut metrics = FixedMetrics::default();
    let mut runtime = Runtime::new(list, 10_000);

    let mut group = c.benchmark_group("layout_10k_rows");
    group.bench_function("first_time", |b| {
        b.iter(|| {
            let mut layout = Layout::new();
            layout.compute(black_box(runtime.root()), WINDOW, &mut metrics);
            layout
        })
    });

    let mut layout = Layout::new();
    layout.compute(runtime.root(), WINDOW, &mut metrics);

    group.bench_function("nothing_changed", |b| {
        b.iter(|| layout.compute(black_box(runtime.root()), WINDOW, &mut metrics))
    });
    group.bench_function("update_one_row", |b| {
        b.iter(|| {
            runtime.dispatch(&[5_000], ActionKind::Click, None).unwrap();
            let patches = runtime.flush();
            layout.apply(&patches);
            layout.compute(runtime.root(), WINDOW, &mut metrics);
        })
    });
    let mut wide = true;
    group.bench_function("resize_window", |b| {
        b.iter(|| {
            wide = !wide;
            let width = if wide { 1920.0 } else { 1600.0 };
            layout.compute(runtime.root(), Size::new(width, 1080.0), &mut metrics);
        })
    });
    group.finish();

    let mut group = c.benchmark_group("layout_filtering");
    group.sample_size(20);
    group.bench_function("half_of_10k_rows_go_and_come_back", |b| {
        let mut runtime = Runtime::new(filtered, ());
        let mut layout = Layout::new();
        layout.compute(runtime.root(), WINDOW, &mut metrics);
        b.iter(|| {
            for _ in 0..2 {
                runtime.dispatch(&[], ActionKind::Click, None).unwrap();
                let patches = runtime.flush();
                layout.apply(&patches);
                layout.compute(runtime.root(), WINDOW, &mut metrics);
            }
        })
    });
    group.finish();
}

/// A list of 10,000 rows that a click shows half of, or all of.
fn filtered(cx: &mut Cx, _: &()) -> Element {
    let half = cx.state(|| false);
    let half_now = cx.get(half);
    let rows: Vec<Node> = (0..10_000u64)
        .filter(|id| !half_now || id % 2 == 0)
        .map(|id| cx.keyed(id, row, id))
        .collect();
    cx.on(
        Element::new(Role::List).key("list").children(rows),
        ActionKind::Click,
        move |cx, _| cx.update(half, |half| *half = !*half),
    )
}

criterion_group!(layout, benches);
criterion_main!(layout);
