//! Bounded App replay oracle: exact full-frame RGBA on independent persistent WARP GPUs.
#![cfg(feature = "render-diagnostics")]

#[path = "../../rsui-text/tests/support/mod.rs"]
mod font;
#[path = "support/render_oracle.rs"]
mod oracle;

use oracle::{Capture, Lane, compare, difference};
use rsui::prelude::*;
use rsui::render::diagnostics::{DrawOutcome, ScenePath};
use rsui::{App, Frame, KeyCode, KeyPress};
use std::time::Duration;

const SIZE: (u32, u32) = (160, 120);

#[derive(Clone, Copy, Debug)]
enum Kind {
    Boxes,
    Nested,
    Input,
    Scroll,
    Image,
}

#[derive(Clone, Copy)]
struct Fixture {
    kind: Kind,
    flags: i64,
}

fn block(key: &str, x: f64, y: f64, color: i64) -> Element {
    Element::new(Role::Group)
        .key(key)
        .prop("position", "absolute")
        .prop("left", x)
        .prop("top", y)
        .prop("width", 38.0)
        .prop("height", 30.0)
        .prop("background", color)
}

fn fixture(cx: &mut Cx, initial: &Fixture) -> Element {
    let flags = cx.state(|| initial.flags);
    let f = cx.get(flags);
    let mut root = Element::new(Role::Window)
        .key("root")
        .props(style!(Style::new().align(Align::Start)));
    match initial.kind {
        Kind::Boxes | Kind::Nested => {
            let a = block(
                "a",
                8.0,
                10.0,
                if f & 1 == 0 { 0xE0203080 } else { 0x2060E080 },
            );
            let b = block("b", 25.0, 22.0, 0x30B04090);
            let mut group = Element::new(Role::Group)
                .key("group")
                .prop("width", 90.0)
                .prop("height", 65.0);
            if f & 8 == 0 {
                group = group.child(a).child(b);
            } else {
                group = group.child(b).child(a);
            }
            if f & 4 != 0 {
                group = group.child(block("extra", 36.0, 14.0, 0xA040C0A0));
            }
            if matches!(initial.kind, Kind::Nested) {
                group = group
                    .prop(
                        "transform",
                        if f & 16 == 0 {
                            "translate(3px, 4px) rotate(7deg)"
                        } else {
                            "translate(13px, 2px) rotate(-11deg)"
                        },
                    )
                    .prop("opacity", 0.75);
                let content = Element::new(Role::Group)
                    .prop("width", 100.0)
                    .prop("height", 100.0)
                    .prop("opacity", 0.8)
                    .prop("transform", "translate(2px, 1px)")
                    .child(group);
                root = root.child(
                    cx.scroll_area("clip", ScrollAxis::Vertical, content)
                        .prop("width", 82.0)
                        .prop("height", 55.0),
                );
            } else {
                root = root.child(group);
            }
            root = root.child(block(
                "distant",
                116.0,
                82.0,
                if f & 2 == 0 { 0x2040E0FF } else { 0xE0A020FF },
            ));
        }
        Kind::Input => {
            let value = cx.state(Text::default);
            let input = Element::new(Role::TextInput)
                .key("input")
                .prop("width", 110.0)
                .prop("height", 24.0)
                .prop("padding", 2.0)
                .prop("background", 0xFFFFFFFF_i64)
                .prop("font_size", 10.0)
                .prop("value", cx.get(value));
            root = root.child(cx.on(input, ActionKind::SetValue, move |cx, value_in| {
                if let Some(PropValue::Str(text)) = value_in {
                    cx.set(value, Text::clone(text));
                }
            }));
        }
        Kind::Scroll => {
            let mut rows = Element::new(Role::Group).prop("width", 140.0);
            for i in 0..16 {
                rows = rows.child(
                    Element::new(Role::Text)
                        .key(i as u64)
                        .name(["AAAA", "BBBB", "CCCC", "DDDD"][i % 4])
                        .prop("height", 22.0)
                        .prop("font_size", 10.0)
                        .prop(
                            "background",
                            if i % 2 == 0 {
                                0xD0E0FFFF_i64
                            } else {
                                0xFFE0C0FF
                            },
                        ),
                );
            }
            root = root.child(
                cx.scroll_area("rows", ScrollAxis::Vertical, rows)
                    .prop("width", 140.0)
                    .prop("height", 100.0),
            );
        }
        Kind::Image => {
            if f != 2 {
                root = root.child(image(
                    if f == 0 { "003c-red" } else { "003c-blue" },
                    "",
                    style!(Style::new().width(40.0).height(30.0)),
                ));
            }
        }
    }
    cx.on(root, ActionKind::SetValue, move |cx, input| {
        if let Some(PropValue::Int(next)) = input {
            cx.set(flags, *next);
        }
    })
}

struct Pair {
    normal: Lane,
    full: Lane,
    flipping: bool,
    step: usize,
    label: String,
}

impl Pair {
    fn new(kind: Kind, flipping: bool, label: impl Into<String>) -> Self {
        let initial = Fixture { kind, flags: 0 };
        Self {
            normal: Lane::new(fixture, initial, SIZE, 1.0, flipping, false),
            full: Lane::new(fixture, initial, SIZE, 1.0, flipping, true),
            flipping,
            step: 0,
            label: label.into(),
        }
    }
    fn both(&mut self, mut action: impl FnMut(&mut App)) {
        action(&mut self.normal.app);
        action(&mut self.full.app);
    }
    fn flags(&mut self, flags: i64) {
        self.both(|app| {
            app.runtime_mut()
                .dispatch(&[], ActionKind::SetValue, Some(PropValue::Int(flags)))
                .unwrap();
        });
    }
    fn frame(&mut self, action: &str) -> (Capture, Capture) {
        let a = self.normal.frame();
        let b = self.full.frame();
        compare(
            &format!("{}-step-{}-{action}", self.label, self.step),
            self.normal.app.size(),
            self.normal.app.scale(),
            self.flipping,
            &a,
            &b,
        );
        self.step += 1;
        (a, b)
    }
    fn resize(&mut self, size: (u32, u32), scale: f32) {
        self.normal.resize(size, scale);
        self.full.resize(size, scale);
    }
    fn fresh(&mut self, kind: Kind, flags: i64, expected: &Capture) {
        let mut fresh = Lane::new(
            fixture,
            Fixture { kind, flags },
            self.normal.app.size(),
            self.normal.app.scale(),
            self.flipping,
            true,
        );
        let capture = fresh.frame();
        compare(
            &format!("{}-fresh-{}", self.label, self.step),
            self.normal.app.size(),
            self.normal.app.scale(),
            self.flipping,
            expected,
            &capture,
        );
    }
}

#[test]
fn static_recolors_insert_remove_and_translucent_keyed_order() {
    for flipping in [false, true] {
        let mut p = Pair::new(Kind::Boxes, flipping, "boxes");
        let (mut previous, _) = p.frame("initial");
        let (idle, _) = p.frame("unchanged");
        assert_eq!(idle.update, Frame::Unchanged);
        assert_eq!(idle.draw.outcome, DrawOutcome::Unchanged);
        for (flags, action) in [
            (1, "local"),
            (3, "distant"),
            (2, "other-distant"),
            (6, "insert"),
            (14, "reorder"),
            (10, "remove"),
        ] {
            p.flags(flags);
            let (current, _) = p.frame(action);
            assert_ne!(previous.pixels, current.pixels, "{action} must be visible");
            p.fresh(Kind::Boxes, flags, &current);
            previous = current;
            p.frame("unchanged-between-mutations");
        }
    }
}

#[test]
fn nested_clip_transform_opacity_and_exposed_background() {
    for flipping in [false, true] {
        let mut p = Pair::new(Kind::Nested, flipping, "nested");
        let (mut previous, _) = p.frame("initial");
        assert!(!previous.scene.transforms.is_empty());
        assert!(
            previous
                .scene
                .batches
                .iter()
                .any(|b| b.clip.right < SIZE.0 as i32)
        );
        for flags in [16, 20, 28, 24, 8, 0] {
            p.flags(flags);
            let (current, _) = p.frame(&format!("flags-{flags}"));
            assert_ne!(previous.pixels, current.pixels);
            p.fresh(Kind::Nested, flags, &current);
            previous = current;
        }
    }
}

fn press(app: &mut App, key: KeyPress) {
    app.key(key).unwrap();
}

#[test]
fn forced_call_bypasses_an_existing_caret_cache_without_a_sticky_mode() {
    for flipping in [false, true] {
        let mut p = Pair::new(Kind::Input, flipping, "existing-caret-cache");
        p.frame("initial");
        p.both(|app| app.focus_on(&[0]));
        p.frame("focus");
        // Test both a visible and hidden forced call with a saved ordinary scene.
        for hidden in [false, true] {
            p.both(|app| press(app, KeyPress::new(KeyCode::Left)));
            let (visible, _) = p.frame("reset-visible");
            p.both(App::blink);
            p.frame("prepare-caret-cache");
            if hidden {
                p.both(App::blink);
                p.frame("reuse-visible");
            }
            p.both(App::blink);
            let before = p.normal.app.scene_work().serial;
            let a = p.normal.frame_with_full_paint(true);
            let b = p.full.frame();
            compare(
                &format!("forced-with-existing-cache-hidden-{hidden}-step-{}", p.step),
                SIZE,
                1.0,
                flipping,
                &a,
                &b,
            );
            p.step += 1;
            assert_eq!(a.work.serial, before + 1);
            assert_eq!(a.work.path, ScenePath::FullTraversal);
            assert_eq!(a.pixels != visible.pixels, hidden);
            p.both(App::blink);
            let (a, _) = p.frame("ordinary-after-forced");
            assert_eq!(a.work.path, ScenePath::CaretReuse);
        }
    }
}

#[test]
fn focus_typing_selection_blinks_blur_and_activation() {
    for flipping in [false, true] {
        let mut p = Pair::new(Kind::Input, flipping, "input");
        p.frame("initial");
        p.both(|app| app.focus_on(&[0]));
        let (visible, _) = p.frame("focus-empty");
        assert_eq!(visible.focus, Some(vec![0]));
        p.both(App::blink);
        let (hidden, full_hidden) = p.frame("first-hidden-blink");
        assert_ne!(visible.pixels, hidden.pixels);
        assert_eq!(hidden.work.path, ScenePath::CaretReuse);
        assert_eq!(full_hidden.work.path, ScenePath::FullTraversal);
        assert_eq!(
            hidden.work.serial,
            visible.work.serial + 2,
            "first blink paints then reuses"
        );
        let mut previous_serial = hidden.work.serial;
        for i in 0..4 {
            p.both(App::blink);
            let (a, _) = p.frame("subsequent-blink");
            assert_eq!(a.work.path, ScenePath::CaretReuse);
            assert_eq!(a.work.serial, previous_serial + 1, "saved-scene shortcut");
            assert_eq!(
                a.pixels,
                if i % 2 == 0 {
                    &visible.pixels
                } else {
                    &hidden.pixels
                }
                .clone()
            );
            previous_serial = a.work.serial;
        }
        // A pending interaction must win over a blink; neither lane may reuse stale text.
        p.both(|app| {
            app.blink();
            app.type_text("ABCD").unwrap();
        });
        let (typed, _) = p.frame("blink-plus-typing");
        assert_eq!(typed.work.path, ScenePath::FullTraversal);
        assert!(typed.outline.contains("ABCD"));
        p.both(|app| press(app, KeyPress::new(KeyCode::Left).shift()));
        let (selection, _) = p.frame("extend-selection");
        assert_ne!(typed.pixels, selection.pixels);
        p.both(|app| press(app, KeyPress::new(KeyCode::Right)));
        p.frame("collapse-selection");
        for active in [false, true] {
            p.both(|app| app.set_active(active));
            p.frame("active-change");
            p.both(App::blink);
            p.frame("blink-after-active-change");
        }
        p.both(|app| app.focus_on(&[]));
        let (blurred, _) = p.frame("blur");
        assert_eq!(blurred.focus, None);
    }
}

#[test]
fn scroll_new_text_rows_resize_and_scale() {
    for flipping in [false, true] {
        let mut p = Pair::new(Kind::Scroll, flipping, "scroll-scale");
        let (start, _) = p.frame("initial");
        p.both(|app| app.scroll(40.0, 40.0, 1).unwrap());
        let (scrolled, _) = p.frame("scroll-down");
        assert_ne!(start.pixels, scrolled.pixels);
        for (size, scale) in [(SIZE, 1.25), ((200, 150), 2.0), (SIZE, 1.0)] {
            p.resize(size, scale);
            p.frame("resize-scale");
            p.frame("unchanged");
        }
        p.both(|app| app.scroll(40.0, 40.0, -100).unwrap());
        let (top, _) = p.frame("return-top");
        p.fresh(Kind::Scroll, 0, &top);
    }
}

// Same uncompressed in-memory PNG construction as tests/images.rs; immutable bytes
// are shared as input only. Decoders, Painter resources and GPU textures are per lane.
fn png(pixel: [u8; 4]) -> Vec<u8> {
    let raw = [0, pixel[0], pixel[1], pixel[2], pixel[3]];
    let mut packed = vec![0x78, 0x01, 1];
    packed.extend((raw.len() as u16).to_le_bytes());
    packed.extend((!(raw.len() as u16)).to_le_bytes());
    packed.extend(raw);
    packed.extend([0; 4]);
    let mut file = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10];
    let mut chunk = |tag: &[u8; 4], body: &[u8]| {
        file.extend((body.len() as u32).to_be_bytes());
        file.extend(tag);
        file.extend(body);
        file.extend([0; 4]);
    };
    let mut header = Vec::new();
    header.extend(1_u32.to_be_bytes());
    header.extend(1_u32.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &packed);
    chunk(b"IEND", &[]);
    file
}

fn register_images() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        rsui::register_image(
            "003c-red",
            Box::leak(png([255, 0, 0, 255]).into_boxed_slice()),
        );
        rsui::register_image(
            "003c-blue",
            Box::leak(png([0, 0, 255, 255]).into_boxed_slice()),
        );
    });
}

fn barrier(p: &mut Pair) {
    p.both(|app| {
        assert!(
            app.wait_for_images(Duration::from_secs(20)),
            "image completion barrier"
        )
    });
}

#[test]
fn image_arrival_replacement_drop_and_revisit() {
    register_images();
    for flipping in [false, true] {
        let mut p = Pair::new(Kind::Image, flipping, "images");
        let (empty, _) = p.frame("request-red");
        barrier(&mut p);
        let (red, _) = p.frame("red-arrived");
        assert_ne!(empty.pixels, red.pixels);
        assert_eq!(red.draw.image_upload.count, 1);
        let center = ((15 * SIZE.0 + 20) * 4) as usize;
        assert_eq!(&red.pixels[center..center + 4], &[255, 0, 0, 255]);
        p.flags(1);
        p.frame("request-blue");
        barrier(&mut p);
        let (blue, _) = p.frame("blue-arrived");
        assert_ne!(red.pixels, blue.pixels);
        assert_eq!(blue.draw.image_upload.count, 1);
        assert_eq!(&blue.pixels[center..center + 4], &[0, 0, 255, 255]);
        p.flags(2);
        p.frame("remove");
        p.both(App::forget_caches);
        let (dropped, _) = p.frame("drop-unused");
        assert!(dropped.work.pending_drops > 0);
        p.flags(0);
        p.frame("revisit-request");
        barrier(&mut p);
        let (revisit, _) = p.frame("revisit-arrived");
        assert_eq!(revisit.pixels, red.pixels);
        assert_eq!(revisit.draw.image_upload.count, 1);
        let mut fresh = Lane::new(
            fixture,
            Fixture {
                kind: Kind::Image,
                flags: 0,
            },
            SIZE,
            1.0,
            flipping,
            true,
        );
        fresh.frame();
        assert!(fresh.app.wait_for_images(Duration::from_secs(20)));
        let cold = fresh.frame();
        let image_id = |capture: &Capture| {
            capture
                .scene
                .batches
                .iter()
                .find(|b| b.image != 0)
                .unwrap()
                .image
        };
        // Same pixels despite different resource histories and valid image IDs.
        assert_ne!(image_id(&revisit), image_id(&cold));
        compare("images-fresh", SIZE, 1.0, flipping, &revisit, &cold);
    }
}

#[test]
fn deliberate_color_mismatch_is_detected_and_located() {
    for flipping in [false, true] {
        let mut p = Pair::new(Kind::Boxes, flipping, "negative-control");
        let (a, _) = p.frame("initial");
        p.full
            .app
            .runtime_mut()
            .dispatch(&[], ActionKind::SetValue, Some(PropValue::Int(2)))
            .unwrap();
        let b = p.full.frame();
        let mismatch =
            difference(&a.pixels, &b.pixels, SIZE).expect("deliberate reference recolor must fail");
        assert_eq!(mismatch.first, (116, 82));
        assert_eq!(mismatch.bounds, (116, 82, 153, 111));
        assert_eq!(mismatch.count, 38 * 30);
        assert_ne!(mismatch.candidate, mismatch.reference);
    }
}

#[test]
fn one_hundred_twenty_seeded_short_traces() {
    register_images();
    // Stable xorshift64: no wall clock or external RNG; seed and step appear in artifacts.
    for seed in 1_u64..=120 {
        let kind = [
            Kind::Boxes,
            Kind::Nested,
            Kind::Input,
            Kind::Scroll,
            Kind::Image,
        ][((seed as usize - 1) / 2) % 5];
        let mut p = Pair::new(kind, seed % 2 == 0, format!("seed-{seed}-{kind:?}"));
        p.frame("initial");
        if matches!(kind, Kind::Image) {
            barrier(&mut p);
            p.frame("initial-arrival");
        }
        let mut rng = seed;
        let mut flags = 0;
        for step in 0..(8 + seed % 25) {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let action = rng % 8;
            match kind {
                Kind::Boxes | Kind::Nested => {
                    if action < 5 {
                        flags ^= 1 << action;
                        p.flags(flags);
                    } else if action == 5 {
                        p.resize(SIZE, if step % 2 == 0 { 1.25 } else { 1.0 });
                    }
                }
                Kind::Input => p.both(|app| match action {
                    0 => app.focus_on(&[0]),
                    1 => app.blink(),
                    2 => app.type_text("AB").unwrap(),
                    3 => press(app, KeyPress::new(KeyCode::Left).shift()),
                    4 => press(app, KeyPress::new(KeyCode::Right)),
                    5 => app.set_active(step % 2 == 0),
                    6 => app.focus_on(&[]),
                    _ => (),
                }),
                Kind::Scroll => {
                    if action < 6 {
                        p.both(|app| {
                            app.scroll(40.0, 40.0, if action % 2 == 0 { 1 } else { -1 })
                                .unwrap()
                        });
                    } else {
                        p.resize(SIZE, if action == 6 { 1.25 } else { 1.0 });
                    }
                }
                Kind::Image => {
                    p.flags((action % 3) as i64);
                    if action % 3 == 2 {
                        p.both(App::forget_caches);
                    }
                }
            }
            p.frame(&format!("action-{action}"));
            if matches!(kind, Kind::Image) {
                barrier(&mut p);
                p.frame("arrival-barrier");
            }
        }
    }
}
