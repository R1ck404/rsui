//! Shared checked measurement fixture; only compiled for probes.
use crate::App;
use rsui_core::{ActionKind, Element, Role};
use rsui_render::{Renderer, diagnostics::CommandStorage};
use rsui_runtime::Cx;
use windows::Win32::System::{
    ProcessStatus::K32GetProcessMemoryInfo, Threading::GetCurrentProcess,
};
#[repr(C)]
#[derive(Default)]
struct Counters {
    cb: u32,
    faults: u32,
    peak_ws: usize,
    ws: usize,
    peak_paged: usize,
    paged: usize,
    peak_nonpaged: usize,
    nonpaged: usize,
    pagefile: usize,
    peak_pagefile: usize,
    private: usize,
    private_ws: usize,
    shared: u64,
}
fn counters() -> Counters {
    let mut c = Counters {
        cb: size_of::<Counters>() as u32,
        ..Default::default()
    };
    // SAFETY: EX2 layout matches Windows, buffer is writable and cb is its size.
    unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&raw mut c).cast(),
            size_of::<Counters>() as u32,
        )
        .expect("process counters");
    }
    c
}
pub fn view(cx: &mut Cx, mode: &u8) -> Element {
    let large = cx.state(|| false);
    let color = cx.state(|| false);
    let shown = cx.get(large);
    let mut root = Element::new(Role::Window)
        .key("root")
        .prop("direction", "row")
        .prop(
            "background",
            if cx.get(color) {
                0xeeeeffff_i64
            } else {
                0xffffffff_i64
            },
        )
        .prop("probe_large", shown);
    for col in 0..if shown {
        if *mode == 2 { 4 } else { 12 }
    } else {
        1
    } {
        let mut column =
            Element::new(Role::Group).prop("width", if *mode == 2 { 300.0 } else { 100.0 });
        if *mode == 2 {
            for row in 0..if shown { 3 } else { 1 } {
                column = column.child(Element::new(Role::Image).prop("width",300.0).prop("height",300.0).prop("src",format!("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='300' height='300'><rect width='300' height='300' fill='rgb({},{},90)'/></svg>",col*50,row*70)));
            }
        }
        for row in 0..if shown && *mode == 0 { 90 } else { 1 } {
            let label = if shown && *mode == 1 {
                "Long paragraph English \u{395}\u{3bb}\u{3bb}\u{3b7}\u{3bd}\u{3b9}\u{3ba}\u{3ac} \u{627}\u{644}\u{639}\u{631}\u{628}\u{64a}\u{629} \u{65e5}\u{672c}\u{8a9e} caf\u{e9}. ".repeat(400)
            } else {
                format!("Label {col} / {row}: Save")
            };
            column = column.child(
                Element::new(Role::Text)
                    .name(label)
                    .prop("font_size", if shown { 8.0 } else { 14.0 }),
            );
        }
        root = root.child(column);
    }
    let root = cx.on(root, ActionKind::SetValue, move |cx, _| {
        cx.update(color, |v| *v = !*v)
    });
    cx.on(root, ActionKind::Toggle, move |cx, _| {
        cx.update(large, |v| *v = !*v)
    })
}
fn command(c: CommandStorage) -> u64 {
    c.quads.capacity_bytes
        + c.batches.capacity_bytes
        + c.transforms.capacity_bytes
        + c.filters.capacity_bytes
}
pub fn sample(app: &App, gpu: &Renderer, phase: &str, cycle: usize, ns: u128) {
    let p = app.render_storage();
    let g = gpu.retained_storage();
    let c = counters();
    let mut created = windows::Win32::Foundation::FILETIME::default();
    let mut exited = created;
    let mut kernel = created;
    let mut user = created;
    // SAFETY: current process pseudo handle; four writable FILETIMEs.
    unsafe {
        windows::Win32::System::Threading::GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
        .expect("CPU counters");
    }
    let ticks = |t: windows::Win32::Foundation::FILETIME| {
        (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
    };
    println!(
        "process_cpu_100ns={} phase={phase} cycle={cycle}",
        ticks(kernel) + ticks(user)
    );
    println!(
        "fixed_app={} fixed_renderer={}",
        size_of::<App>(),
        size_of::<Renderer>()
    );
    let (fixed, dynamic) = app.probe_metadata();
    println!(
        "auto_storage fixed={fixed} dynamic={dynamic} history={} scene={} scene_len={} partial_quads={} partial_batches={}",
        command(p.comparison_spare),
        command(p.scene),
        p.scene.quads.len_bytes,
        g.partial_quads.capacity_bytes,
        g.partial_batches.capacity_bytes
    );
    let retained = command(p.comparison_spare)
        + command(p.scene)
        + command(p.kept_caret)
        + p.live_text_quads.capacity_bytes
        + p.spare_text_quads.capacity_bytes
        + p.scratch_glyphs.capacity_bytes
        + p.scratch_lines.capacity_bytes
        + p.glyph_descriptors.capacity_bytes
        + p.glyph_payload.capacity_bytes
        + p.image_descriptors.capacity_bytes
        + p.image_payload.capacity_bytes
        + p.dropped_images.capacity_bytes
        + p.spare_text_descriptors.capacity_bytes
        + p.clips.capacity_bytes
        + p.fades.capacity_bytes
        + p.turning.capacity_bytes;
    println!(
        "phase={phase} cycle={cycle} ns={ns} private={} private_ws={} ws={} painter_capacity={retained} spare={} text_map={} renderer_history={} quad_gpu={} atlas_gpu={} images={}",
        c.private,
        c.private_ws,
        c.ws,
        p.spare_text_quads.capacity_bytes,
        p.text_map_capacity,
        command(g.history),
        g.quad_buffer_capacity_bytes,
        g.atlas_capacity_bytes,
        g.image_entries
    );
}
