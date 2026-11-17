//! Tracks what updates cost, with a list of rows that each hold state and a handler,
//! on `rsui-alloc`, as an application would run them.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use rsui_core::{ActionKind, Element, Node, PropValue, Role, shared_props, text};
use rsui_runtime::{Cx, ListWindow, Runtime};

#[global_allocator]
static ALLOCATOR: rsui_alloc::UiAlloc = rsui_alloc::UiAlloc;

fn row(cx: &mut Cx, id: &u64) -> Element {
    let clicks = cx.state(|| 0u32);
    let name = text!("row {id}: {}", cx.get(clicks));
    cx.on(
        Element::new(Role::ListItem)
            .name(name)
            .props(shared_props!("enabled" => true, "height" => 24.0)),
        ActionKind::Click,
        move |cx, _| cx.update(clicks, |clicks| *clicks += 1),
    )
}

/// `generation` changes nothing about the rows, so rendering the list again only
/// has to confirm that.
fn list(cx: &mut Cx, count: &u64) -> Element {
    let reversed = cx.state(|| false);
    let generation = cx.state(|| 0u64);
    let order: Vec<u64> = if cx.get(reversed) {
        (0..*count).rev().collect()
    } else {
        (0..*count).collect()
    };
    let rows: Vec<Node> = order.into_iter().map(|id| cx.keyed(id, row, id)).collect();
    let element = Element::new(Role::List)
        .key("list")
        .name(text!("{}", cx.get(generation)))
        .children(rows);
    let element = cx.on(element, ActionKind::Toggle, move |cx, _| {
        cx.update(reversed, |reversed| *reversed = !*reversed);
    });
    cx.on(element, ActionKind::Focus, move |cx, _| {
        cx.update(generation, |generation| *generation += 1);
    })
}

fn long_list(cx: &mut Cx, len: &usize) -> Element {
    cx.virtual_list(
        "rows",
        ListWindow {
            len: *len,
            visible: 40,
            overscan: 5,
            reveal: None,
        },
        |cx, index| cx.keyed(index, row, index as u64),
    )
}

fn benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("list_10k");
    group.bench_function("mount", |b| {
        b.iter(|| Runtime::new(list, black_box(10_000)))
    });

    let mut runtime = Runtime::new(list, 10_000);
    group.bench_function("update_one_row", |b| {
        b.iter(|| {
            runtime.dispatch(&[5_000], ActionKind::Click, None).unwrap();
            runtime.flush()
        })
    });
    group.bench_function("render_parent_rows_unchanged", |b| {
        b.iter(|| {
            runtime.dispatch(&[], ActionKind::Focus, None).unwrap();
            runtime.flush()
        })
    });
    group.bench_function("reverse", |b| {
        b.iter(|| {
            runtime.dispatch(&[], ActionKind::Toggle, None).unwrap();
            runtime.flush()
        })
    });
    group.finish();

    let mut group = c.benchmark_group("virtual_list_1m");
    group.bench_function("mount", |b| {
        b.iter(|| Runtime::new(long_list, black_box(1_000_000)))
    });
    group.bench_function("scroll_one_row", |b| {
        b.iter_batched_ref(
            || Runtime::new(long_list, 1_000_000),
            |runtime| {
                runtime
                    .dispatch(&[], ActionKind::Scroll, Some(PropValue::Int(10)))
                    .unwrap();
                runtime.flush();
                runtime
                    .dispatch(&[], ActionKind::Scroll, Some(PropValue::Int(11)))
                    .unwrap();
                runtime.flush()
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

criterion_group!(runtime, benches);
criterion_main!(runtime);
