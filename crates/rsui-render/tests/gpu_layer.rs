//! Something faded as a whole (`Layer`), as CSS's `opacity` fades a disabled button's text and background together: its parts are
//! drawn by themselves, then put down at its opacity, on Windows' software GPU and on the system's, which a window uses.

use rsui_render::{Batch, Color, Layer, PixelRect, Quad, Renderer, Scene};

const SIZE: (u32, u32) = (64, 32);

/// A black box with a white one over part of it, faded to a half as one, over a white window.
fn scene() -> Scene {
    let clip = PixelRect {
        left: 0,
        top: 0,
        right: SIZE.0 as i32,
        bottom: SIZE.1 as i32,
    };
    let quad = |rect, color: Color| Quad {
        rect,
        glyph: [0; 4],
        color: [color.r, color.g, color.b, color.a],
        radius: 0.0,
    };
    let quads = vec![
        quad([8.0, 4.0, 48.0, 24.0], Color::hex(0x000000)),
        quad([24.0, 10.0, 16.0, 12.0], Color::hex(0xFFFFFF)),
    ];
    let batches = vec![Batch {
        first: 0,
        count: 2,
        clip,
        image: 0,
        transform: 0,
        filter: 0,
        round: 0,
    }];
    Scene {
        quads,
        batches,
        layers: vec![Layer {
            first: 0,
            end: 1,
            opacity: 0.5,
            bounds: PixelRect {
                left: 7,
                top: 3,
                right: 57,
                bottom: 29,
            },
        }],
        atlas_size: 64,
        ..Scene::default()
    }
}

fn draw(renderer: &mut Renderer) -> Vec<u8> {
    renderer.render(&mut scene(), Color::WHITE).expect("render");
    renderer.read_pixels().expect("read")
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let at = ((y * SIZE.0 + x) * 4) as usize;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

fn check(pixels: &[u8], label: &str) {
    // White over black, put down at a half over white, is white: the black doesn't show through the white.
    assert!(
        pixel(pixels, 32, 16).iter().all(|&part| part >= 253),
        "{label}: {:?}",
        pixel(pixels, 32, 16)
    );
    // Black alone, at a half over white, is half grey.
    assert!(
        pixel(pixels, 12, 16)
            .iter()
            .all(|&part| part.abs_diff(128) <= 2),
        "{label}: {:?}",
        pixel(pixels, 12, 16)
    );
    // Outside it, nothing's changed.
    assert_eq!(pixel(pixels, 2, 2), [255, 255, 255], "{label}");
}

#[test]
fn a_layer_is_faded_as_one_in_software() {
    let mut software = Renderer::offscreen(SIZE).expect("WARP renderer");
    check(&draw(&mut software), "software");
    // Drawn again, with nothing changed, it's the same.
    check(&draw(&mut software), "software again");
}

#[test]
fn a_layer_is_faded_as_one_on_the_gpu() {
    let Ok(mut on_hardware) = Renderer::offscreen_on_hardware(SIZE) else {
        eprintln!("no hardware GPU here; skipping");
        return;
    };
    check(&draw(&mut on_hardware), "hardware");
}

#[cfg(feature = "render-diagnostics")]
#[test]
fn cropped_layer_targets_match_full_window_surfaces_and_release_when_idle() {
    let size = (1920, 1080);
    for hardware in [false, true] {
        let make = || {
            if hardware {
                Renderer::offscreen_on_hardware(size)
            } else {
                Renderer::offscreen_flipping(size)
            }
        };
        let Ok(mut cropped) = make() else {
            continue;
        };
        let mut full = make().unwrap();
        full.set_full_layer_targets_for_diagnostics(true);
        full.set_always_redraw(true);
        for (step, (x, y)) in [
            (700.0, 300.0),
            (701.25, 301.5),
            (-8.0, -4.0),
            (1900.0, 1070.0),
            (20.0, 40.0),
        ]
        .into_iter()
        .enumerate()
        {
            let mut next = scene();
            for quad in &mut next.quads {
                quad.rect[0] += x;
                quad.rect[1] += y;
            }
            for batch in &mut next.batches {
                batch.clip = PixelRect {
                    left: 0,
                    top: 0,
                    right: size.0 as i32,
                    bottom: size.1 as i32,
                };
            }
            next.layers[0].bounds = PixelRect {
                left: (7.0 + x).floor() as i32,
                top: (3.0 + y).floor() as i32,
                right: (57.0 + x).ceil() as i32,
                bottom: (29.0 + y).ceil() as i32,
            };
            cropped.render(&mut next.clone(), Color::WHITE).unwrap();
            full.render(&mut next.clone(), Color::WHITE).unwrap();
            let a = cropped.read_pixels().unwrap();
            let b = full.read_pixels().unwrap();
            let changed = a.iter().zip(&b).filter(|(a, b)| a != b).count();
            let worst = a.iter().zip(&b).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
            assert!(
                a == b,
                "frame {step}, hardware={hardware}, channels={changed}, max={worst}"
            );
            let bytes = cropped.retained_storage().layer_target_capacity_bytes;
            assert!(
                bytes <= 64 * 32 * 4,
                "small opacity group must not allocate a full-window texture: {bytes}"
            );
            assert_eq!(
                full.retained_storage().layer_target_capacity_bytes,
                u64::from(size.0) * u64::from(size.1) * 4
            );
            if step == 0 {
                eprintln!(
                    "1080p opacity target logical bytes: cropped={bytes}, full={}",
                    full.retained_storage().layer_target_capacity_bytes
                );
            }
            if step == 2 {
                let before = cropped.read_pixels().unwrap();
                cropped.trim_storage();
                assert_eq!(cropped.retained_storage().layer_target_capacity_bytes, 0);
                assert_eq!(
                    cropped.read_pixels().unwrap(),
                    before,
                    "temporary target release preserves displayed pixels"
                );
                cropped.render(&mut next, Color::WHITE).unwrap();
                assert_eq!(
                    cropped.read_pixels().unwrap(),
                    before,
                    "later opacity recreates its target"
                );
            }
        }
    }
}

#[cfg(feature = "render-diagnostics")]
#[test]
fn separate_opacity_groups_reuse_one_target_and_restore_image_filtering() {
    let size = (800, 600);
    let clip = PixelRect {
        left: 0,
        top: 0,
        right: 800,
        bottom: 600,
    };
    let mut next = scene();
    next.batches[0].clip = clip;
    let mut other = next.quads.clone();
    for quad in &mut other {
        quad.rect[0] += 300.0;
        quad.rect[1] += 200.0;
    }
    next.quads.extend(other);
    next.batches.push(Batch {
        first: 2,
        count: 2,
        clip,
        image: 0,
        transform: 0,
        filter: 0,
        round: 0,
    });
    next.layers.push(Layer {
        first: 1,
        end: 2,
        opacity: 0.75,
        bounds: PixelRect {
            left: 307,
            top: 203,
            right: 357,
            bottom: 229,
        },
    });
    next.quads.push(Quad {
        rect: [600.0, 300.0, 60.0, 60.0],
        glyph: [0, 0, u16::MAX, u16::MAX],
        color: [255; 4],
        radius: 0.0,
    });
    next.batches.push(Batch {
        first: 4,
        count: 1,
        clip,
        image: 17,
        transform: 0,
        filter: 0,
        round: 0,
    });
    next.image_uploads.push(rsui_render::ImageUpload {
        id: 17,
        width: 2,
        height: 2,
        changes: None,
        pixels: vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ],
    });
    let mut cropped = Renderer::offscreen(size).unwrap();
    let mut full = Renderer::offscreen(size).unwrap();
    full.set_full_layer_targets_for_diagnostics(true);
    cropped.render(&mut next.clone(), Color::WHITE).unwrap();
    full.render(&mut next, Color::WHITE).unwrap();
    let pixels = cropped.read_pixels().unwrap();
    assert_eq!(pixels, full.read_pixels().unwrap());
    let at = ((330 * size.0 + 630) * 4) as usize;
    assert!(
        pixels[at..at + 3]
            .iter()
            .all(|value| *value > 100 && *value < 160),
        "an ordinary image after opacity still uses linear filtering: {:?}",
        &pixels[at..at + 4]
    );
    assert_eq!(
        cropped.retained_storage().layer_target_capacity_bytes,
        64 * 32 * 4,
        "separate positions share one small target, rather than their union rectangle"
    );
}

#[cfg(feature = "render-diagnostics")]
#[test]
fn bounded_opacity_damage_matches_full_redraw_across_buffer_history() {
    let size = (800, 600);
    let clip = PixelRect {
        left: 0,
        top: 0,
        right: 800,
        bottom: 600,
    };
    for flipping in [false, true] {
        let mut candidate = if flipping {
            Renderer::offscreen_flipping(size)
        } else {
            Renderer::offscreen(size)
        }
        .unwrap();
        let mut reference = Renderer::offscreen(size).unwrap();
        reference.set_full_layer_targets_for_diagnostics(true);
        reference.set_always_redraw(true);
        let mut partial = 0;
        let mut largest_partial_area = 0;
        for step in 0..11 {
            let mut next = scene();
            let dx = 100.0 + (step % 3) as f32 * 12.25;
            let dy = 50.0 + (step % 2) as f32 * 6.5;
            for quad in &mut next.quads {
                quad.rect[0] += dx;
                quad.rect[1] += dy;
            }
            next.batches[0].first = 1;
            next.batches[0].clip = clip;
            next.quads.insert(
                0,
                Quad {
                    rect: [0.0, 0.0, 800.0, 600.0],
                    glyph: [0; 4],
                    color: [40, 80, 120, 255],
                    radius: 0.0,
                },
            );
            next.batches.insert(
                0,
                Batch {
                    first: 0,
                    count: 1,
                    clip,
                    image: 0,
                    transform: 0,
                    filter: 0,
                    round: 0,
                },
            );
            next.quads.push(Quad {
                rect: [125.0, 65.0, 20.0, 30.0],
                glyph: [0; 4],
                color: [80, 160, 40, 255],
                radius: 0.0,
            });
            next.batches.push(Batch {
                first: 3,
                count: 1,
                clip,
                image: 0,
                transform: 0,
                filter: 0,
                round: 0,
            });
            next.layers[0] = Layer {
                first: 1,
                end: 2,
                opacity: (step % 6) as f32 * 0.15,
                bounds: PixelRect {
                    left: (7.0 + dx).floor() as i32,
                    top: (3.0 + dy).floor() as i32,
                    right: (57.0 + dx).ceil() as i32,
                    bottom: (29.0 + dy).ceil() as i32,
                },
            };
            if step == 8 {
                next.layers.clear();
            }
            if step == 10 {
                next.transforms.push([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
                next.batches[1].transform = 1;
            }
            candidate.render(&mut next.clone(), Color::WHITE).unwrap();
            reference.render(&mut next, Color::WHITE).unwrap();
            assert!(
                candidate.read_pixels().unwrap() == reference.read_pixels().unwrap(),
                "step {step}, flipping={flipping}"
            );
            if candidate.draw_work().outcome == rsui_render::diagnostics::DrawOutcome::Partial {
                partial += 1;
                let region = candidate.draw_work().draw_region;
                largest_partial_area = largest_partial_area.max(rsui_render::diagnostics::area(region));
                assert!(
                    (region.right - region.left) * (region.bottom - region.top) < 4000,
                    "small layer updates must not redraw the full window: {region:?}"
                );
            }
            if step == 8 || step == 9 || step == 10 {
                assert_eq!(
                    candidate.draw_work().outcome,
                    rsui_render::diagnostics::DrawOutcome::Full,
                    "topology changes keep conservative fallback"
                );
            }
        }
        assert!(
            partial >= 5,
            "opacity and bounds changes use bounded damage"
        );
        eprintln!("opacity partial frames={partial}, largest actual draw area={largest_partial_area}, full area=480000, flipping={flipping}");
    }
}
