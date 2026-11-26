use super::ops::*;
use super::*;

fn pixel(id: u32, x: i32, y: i32) -> [u8; 4] {
    let mut out = [0u8; 4];
    read(id, x, y, 1, 1, &mut out);
    out
}

fn color(r: u8, g: u8, b: u8, a: u8) -> f64 {
    f64::from(u32::from(r) << 24 | u32::from(g) << 16 | u32::from(b) << 8 | u32::from(a))
}

fn op(code: u32) -> f64 {
    f64::from(code)
}

#[test]
fn fill_rect_paints_whole_pixels() {
    let id = create(20, 20, false);
    run(&[op(SELECT), f64::from(id), op(FILL_COLOR), color(255, 0, 0, 255), op(FILL_RECT), 2.0, 2.0, 5.0, 5.0], &[]);
    assert_eq!(pixel(id, 3, 3), [255, 0, 0, 255]);
    assert_eq!(pixel(id, 7, 3), [0, 0, 0, 0]);
    assert_eq!(pixel(id, 1, 1), [0, 0, 0, 0]);
}

#[test]
fn scaled_destination_in_matches_the_layered_compositor() {
    let image = Rc::new(Image { width: 7, height: 5, pixels: (0..140).map(|n| (n * 73 % 256) as u8).collect() });
    for smooth in [false, true] {
        for alpha in [0.0, 0.17, 0.5, 1.0] {
            for flip in [false, true] {
                let render = |layered: bool, dx: f32| {
                    let mut world = World::default();
                    let mut surface = Surface::new(61, 39, false);
                    let mut drawing = Drawing { surface: &mut surface, world: &mut world, surface_id: 1 };
                    drawing.draw(Coverage::Rect([3.0, 2.0, 59.0, 37.0]), &Shader::Solid([23.0, 91.0, 127.0, 163.0]), None);
                    let mut clip = Path::default();
                    clip.rect(&Matrix::IDENTITY, 4.0, 1.0, 47.0, 34.0);
                    drawing.clip(&clip, false);
                    let m = if flip { Matrix::new([-1.0, 0.0, 0.0, -1.0, 61.0, 39.0]) } else { Matrix::IDENTITY };
                    drawing.surface.state.transform = m;
                    drawing.surface.state.op = Op::DestinationIn;
                    drawing.surface.state.smoothing = smooth;
                    drawing.surface.state.alpha = alpha;
                    if layered {
                        let to_image = Matrix::new([6.5 / 61.0, 0.0, 0.0, 4.5 / 39.0, 0.25 - dx * 6.5 / 61.0, 0.25]).then(&m.invert().unwrap());
                        let shader = Shader::Image { image: image.clone(), to_image, smooth, repeat: Repeat::Neither, bounds: [0, 0, 7, 5], pattern: false };
                        let left = if flip { -dx } else { dx };
                        drawing.draw(Coverage::Rect([left, 0.0, left + 61.0, 39.0]), &shader, None);
                    } else {
                        drawing.draw_image(&image, [0.25, 0.25, 6.5, 4.5, dx, 0.0, 61.0, 39.0]);
                    }
                    surface.pixels
                };
                for dx in [0.0, 17.0, -9.0, 80.0] {
                    assert_eq!(render(false, dx), render(true, dx), "smooth={smooth}, alpha={alpha}, flipped={flip}, x={dx}");
                }
            }
        }
    }
}

#[test]
fn half_pixels_are_half_covered() {
    let id = create(10, 10, false);
    run(&[op(SELECT), f64::from(id), op(FILL_RECT), 0.5, 0.0, 2.0, 2.0], &[]);
    assert_eq!(pixel(id, 0, 0)[3], 128);
    assert_eq!(pixel(id, 1, 0)[3], 255);
    assert_eq!(pixel(id, 2, 0)[3], 128);
}

#[test]
fn clear_rect_takes_away() {
    let id = create(10, 10, false);
    run(&[op(SELECT), f64::from(id), op(FILL_RECT), 0.0, 0.0, 10.0, 10.0, op(CLEAR_RECT), 2.0, 2.0, 3.0, 3.0], &[]);
    assert_eq!(pixel(id, 3, 3), [0, 0, 0, 0]);
    assert_eq!(pixel(id, 6, 6), [0, 0, 0, 255]);
}

#[test]
fn a_rectangle_clip_is_each_row_worked_out() {
    // The same rectangle, the second time with a corner more along its top, which isn't a rectangle to `axis_rect()`, so every
    // row of it is worked out.
    let draw = |extra: bool| {
        let id = create(40, 30, false);
        let mut list = vec![op(SELECT), f64::from(id), op(BEGIN), op(MOVE), 3.3, 2.6];
        if extra {
            list.extend([op(LINE), 15.0, 2.6]);
        }
        list.extend([op(LINE), 33.5, 2.6, op(LINE), 33.5, 23.3, op(LINE), 3.3, 23.3, op(CLOSE), op(CLIP), 0.0]);
        list.extend([op(FILL_COLOR), color(200, 80, 40, 200), op(FILL_RECT), 0.0, 0.0, 40.0, 30.0]);
        list.extend([op(STROKE_COLOR), color(0, 0, 255, 255), op(BEGIN), op(MOVE), 0.0, 0.0, op(LINE), 40.0, 29.0, op(STROKE)]);
        run(&list, &[]);
        let mut out = vec![0u8; 40 * 30 * 4];
        read(id, 0, 0, 40, 30, &mut out);
        out
    };
    let (straight, every) = (draw(false), draw(true));
    assert_eq!(straight, every);
    assert_eq!(&straight[(10 * 40 + 10) * 4..(10 * 40 + 10) * 4 + 4], &[200, 80, 40, 200]);
}

#[test]
fn image_data_reads_back_what_was_put_whatever_was_in_the_buffer() {
    let id = create(9, 2, false);
    // Opaque pixels in fours and not, see-through ones, and nothing at all.
    let mut data = Vec::new();
    for i in 0..18u8 {
        data.extend(match i % 9 {
            0..=4 => [i * 10, 200, 7, 255],
            5 => [10, 20, 30, 0],
            6 => [100, 50, 0, 128],
            _ => [255, 255, 255, 255],
        });
    }
    write(id, &data, 9, 2, 0, 0, [0, 0, 9, 2]);
    let mut out = vec![0xAAu8; 9 * 2 * 4];
    read(id, 0, 0, 9, 2, &mut out);
    for (i, (got, put)) in out.chunks_exact(4).zip(data.chunks_exact(4)).enumerate() {
        let expected: [u8; 4] = match put[3] {
            255 => [put[0], put[1], put[2], 255],
            0 => [0; 4],
            _ => [100, 50, 0, 128],
        };
        assert_eq!(got, expected, "pixel {i}");
    }
}

#[test]
fn save_and_restore_put_the_state_back() {
    let id = create(10, 10, false);
    run(
        &[
            op(SELECT), f64::from(id),
            op(FILL_COLOR), color(0, 0, 255, 255),
            op(SAVE), op(FILL_COLOR), color(255, 0, 0, 255), op(TRANSFORM), 1.0, 0.0, 0.0, 1.0, 5.0, 5.0, op(RESTORE),
            op(FILL_RECT), 0.0, 0.0, 2.0, 2.0,
        ],
        &[],
    );
    assert_eq!(pixel(id, 1, 1), [0, 0, 255, 255]);
    assert_eq!(pixel(id, 6, 6), [0, 0, 0, 0]);
}

#[test]
fn arcs_fill_round() {
    let id = create(40, 40, false);
    run(&[op(SELECT), f64::from(id), op(BEGIN), op(ARC), 20.0, 20.0, 10.0, 0.0, std::f64::consts::TAU, 0.0, op(FILL), 0.0], &[]);
    assert_eq!(pixel(id, 20, 20)[3], 255);
    assert_eq!(pixel(id, 2, 2)[3], 0);
    // About as much as a circle's area.
    let mut all = vec![0u8; 40 * 40 * 4];
    read(id, 0, 0, 40, 40, &mut all);
    let area: f32 = all.chunks_exact(4).map(|p| f32::from(p[3]) / 255.0).sum();
    assert!((area - std::f32::consts::PI * 100.0).abs() < 1.5, "{area}");
}

#[test]
fn strokes_are_as_wide_as_said() {
    let id = create(40, 40, false);
    run(
        &[op(SELECT), f64::from(id), op(LINE_WIDTH), 4.0, op(BEGIN), op(MOVE), 5.0, 20.0, op(LINE), 35.0, 20.0, op(STROKE)],
        &[],
    );
    assert_eq!(pixel(id, 20, 18)[3], 255);
    assert_eq!(pixel(id, 20, 21)[3], 255);
    assert_eq!(pixel(id, 20, 22)[3], 0);
    assert_eq!(pixel(id, 3, 20)[3], 0);
}

#[test]
fn clipping_keeps_drawing_inside() {
    let id = create(20, 20, false);
    run(
        &[op(SELECT), f64::from(id), op(BEGIN), op(RECT), 5.0, 5.0, 5.0, 5.0, op(CLIP), 0.0, op(FILL_RECT), 0.0, 0.0, 20.0, 20.0],
        &[],
    );
    assert_eq!(pixel(id, 7, 7)[3], 255);
    assert_eq!(pixel(id, 2, 2)[3], 0);
    assert_eq!(pixel(id, 12, 12)[3], 0);
}

#[test]
fn image_data_goes_back_as_it_came() {
    let id = create(4, 4, false);
    let data: Vec<u8> = (0..64).map(|i| if i % 4 == 3 { 255 } else { (i * 3) as u8 }).collect();
    write(id, &data, 4, 4, 0, 0, [0, 0, 4, 4]);
    let mut out = vec![0u8; 64];
    read(id, 0, 0, 4, 4, &mut out);
    assert_eq!(out, data);
}

#[test]
fn gradients_go_from_one_color_to_the_next() {
    let id = create(100, 1, false);
    run(
        &[
            op(LINEAR), 7.0, 0.0, 0.0, 100.0, 0.0,
            op(STOP), 7.0, 0.0, color(0, 0, 0, 255),
            op(STOP), 7.0, 1.0, color(255, 255, 255, 255),
            op(SELECT), f64::from(id), op(FILL_GRADIENT), 7.0, op(FILL_RECT), 0.0, 0.0, 100.0, 1.0,
        ],
        &[],
    );
    let middle = pixel(id, 50, 0)[0];
    assert!((126..=130).contains(&middle), "{middle}");
    assert!(pixel(id, 0, 0)[0] < 4);
}

#[test]
fn composite_destination_out_erases() {
    let id = create(10, 10, false);
    run(
        &[op(SELECT), f64::from(id), op(FILL_RECT), 0.0, 0.0, 10.0, 10.0, op(COMPOSITE), 6.0, op(FILL_RECT), 0.0, 0.0, 5.0, 10.0],
        &[],
    );
    assert_eq!(pixel(id, 2, 2)[3], 0);
    assert_eq!(pixel(id, 7, 2)[3], 255);
}

#[test]
fn copy_clears_what_is_not_drawn() {
    let id = create(10, 10, false);
    run(
        &[op(SELECT), f64::from(id), op(FILL_RECT), 0.0, 0.0, 10.0, 10.0, op(COMPOSITE), 9.0, op(FILL_RECT), 0.0, 0.0, 5.0, 10.0],
        &[],
    );
    assert_eq!(pixel(id, 2, 2)[3], 255);
    assert_eq!(pixel(id, 7, 2)[3], 0);
}

#[test]
fn shadows_fall_where_they_are_moved() {
    let id = create(30, 30, false);
    run(
        &[
            op(SELECT), f64::from(id),
            op(SHADOW_COLOR), color(0, 0, 0, 255), op(SHADOW_X), 10.0, op(SHADOW_Y), 10.0,
            op(FILL_COLOR), color(255, 0, 0, 255), op(FILL_RECT), 2.0, 2.0, 6.0, 6.0,
        ],
        &[],
    );
    assert_eq!(pixel(id, 4, 4), [255, 0, 0, 255]);
    assert_eq!(pixel(id, 14, 14), [0, 0, 0, 255]);
}

#[test]
fn images_draw_pixel_for_pixel_and_scaled() {
    let source = create(2, 2, false);
    run(&[op(SELECT), f64::from(source), op(FILL_COLOR), color(0, 255, 0, 255), op(FILL_RECT), 0.0, 0.0, 2.0, 2.0], &[]);
    let id = create(10, 10, false);
    run(&[op(SELECT), f64::from(id), op(DRAW_IMAGE), 1.0, f64::from(source), 0.0, 0.0, 2.0, 2.0, 3.0, 3.0, 2.0, 2.0], &[]);
    assert_eq!(pixel(id, 3, 3), [0, 255, 0, 255]);
    assert_eq!(pixel(id, 5, 5), [0, 0, 0, 0]);
    run(&[op(SELECT), f64::from(id), op(DRAW_IMAGE), 1.0, f64::from(source), 0.0, 0.0, 2.0, 2.0, 0.0, 6.0, 4.0, 4.0], &[]);
    assert_eq!(pixel(id, 1, 8), [0, 255, 0, 255]);
}

#[test]
fn path2d_is_drawn_through_the_transform_when_used() {
    let id = create(20, 20, false);
    run(
        &[
            op(PATH_DEFINE), 3.0, 5.0, op(RECT), 0.0, 0.0, 2.0, 2.0,
            op(SELECT), f64::from(id), op(TRANSFORM), 1.0, 0.0, 0.0, 1.0, 10.0, 10.0, op(FILL_PATH), 3.0, 0.0,
        ],
        &[],
    );
    assert_eq!(pixel(id, 11, 11)[3], 255);
    assert_eq!(pixel(id, 1, 1)[3], 0);
}

#[test]
fn point_in_path_uses_the_canvas_space() {
    let id = create(20, 20, false);
    run(&[op(SELECT), f64::from(id), op(BEGIN), op(RECT), 5.0, 5.0, 5.0, 5.0], &[]);
    assert!(point_in(id, 0, 7.0, 7.0, false, false));
    assert!(!point_in(id, 0, 12.0, 7.0, false, false));
    assert!(point_in(id, 0, 5.0, 7.0, false, true));
}

#[test]
fn resizing_starts_again() {
    let id = create(10, 10, false);
    run(&[op(SELECT), f64::from(id), op(FILL_RECT), 0.0, 0.0, 10.0, 10.0], &[]);
    resize(id, 10, 10);
    assert_eq!(pixel(id, 2, 2), [0, 0, 0, 0]);
    assert_eq!(size(id), Some((10, 10)));
}

#[test]
fn uploads_give_what_changed() {
    let id = create(50, 50, false);
    let (_, _, version) = take_upload(id, true, 0).unwrap();
    run(&[op(SELECT), f64::from(id), op(FILL_RECT), 10.0, 10.0, 5.0, 5.0], &[]);
    let (area, pixels, _) = take_upload(id, false, version).unwrap();
    assert_eq!(area, IRectOut { x: 10, y: 10, width: 5, height: 5 });
    assert_eq!(pixels.len(), 5 * 5 * 4);
}

#[test]
fn an_upload_after_shrinking_stays_inside_the_canvas() {
    // Drawn on at one size, then made smaller before the GPU took it: what changed at the old size mustn't be read at the new.
    let id = create(100, 100, false);
    let (_, _, version) = take_upload(id, true, 0).unwrap();
    run(&[op(SELECT), f64::from(id), op(FILL_RECT), 0.0, 0.0, 100.0, 100.0], &[]);
    resize(id, 60, 40);
    run(&[op(SELECT), f64::from(id), op(FILL_RECT), 0.0, 0.0, 10.0, 10.0], &[]);
    let (area, pixels, _) = take_upload(id, false, version).unwrap();
    assert!(area.x + area.width <= 60 && area.y + area.height <= 40, "{area:?}");
    assert_eq!(pixels.len(), (area.width * area.height * 4) as usize);
}

#[test]
fn text_has_ink() {
    let id = create(100, 40, false);
    run(
        &[op(SELECT), f64::from(id), op(FONT), 20.0, 400.0, 0.0, 0.0, op(FILL_TEXT), 1.0, 5.0, 30.0, f64::INFINITY],
        &["Arial".to_owned(), "Hello".to_owned()],
    );
    let mut all = vec![0u8; 100 * 40 * 4];
    read(id, 0, 0, 100, 40, &mut all);
    let ink = all.chunks_exact(4).filter(|p| p[3] > 0).count();
    assert!(ink > 50, "{ink}");
    let metrics = measure(&[20.0, 400.0, 0.0], "Arial", 0.0, false, 0, 0, "Hello");
    assert!(metrics[0] > 40.0 && metrics[0] < 60.0, "{}", metrics[0]);
    assert!(metrics[3] > 10.0, "{}", metrics[3]);
}

#[test]
fn filters_parse_as_css_says() {
    assert_eq!(parse_filter("none"), None);
    assert!(matches!(parse_filter("blur(2px)").unwrap()[0], Filter::Blur(v) if v == 2.0));
    assert_eq!(parse_filter("grayscale(100%) brightness(0.5)").unwrap().len(), 2);
    assert!(matches!(parse_filter("drop-shadow(2px 3px 4px red)").unwrap()[0], Filter::Shadow { x, y, .. } if x == 2.0 && y == 3.0));
    assert_eq!(parse_filter("nonsense(3)"), None);
}

/// How long drawing what the canvas benchmarks draw takes here, without JavaScript:
/// `cargo test --release -p rsui-render --lib canvas::tests::timing -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, for looking at with --nocapture"]
fn timing() {
    let mut seed = 1u32;
    let mut random = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        f64::from(seed) / 4294967296.0
    };
    let id = create(800, 600, false);
    let time = |name: &str, list: &[f64]| {
        let mut best = f64::MAX;
        for _ in 0..10 {
            let start = std::time::Instant::now();
            run(list, &[]);
            best = best.min(start.elapsed().as_secs_f64() * 1000.0);
        }
        println!("{name}: {best:.2} ms");
    };
    let mut list = vec![op(SELECT), f64::from(id), op(CLEAR_RECT), 0.0, 0.0, 800.0, 600.0, op(STROKE_COLOR), color(30, 60, 200, 128)];
    for _ in 0..5000 {
        list.extend([op(BEGIN), op(MOVE), random() * 800.0, random() * 600.0, op(LINE), random() * 800.0, random() * 600.0, op(STROKE)]);
    }
    time("5000 stroked lines", &list);
    let mut list = vec![op(SELECT), f64::from(id), op(CLEAR_RECT), 0.0, 0.0, 800.0, 600.0];
    for i in 0..5000 {
        list.extend([op(FILL_COLOR), color((i * 7 % 255) as u8, 100, 50, 255), op(BEGIN), op(ARC), random() * 800.0, random() * 600.0, 2.0 + random() * 8.0, 0.0, std::f64::consts::TAU, 0.0, op(FILL), 0.0]);
    }
    time("5000 filled circles", &list);
    let mut list = vec![op(SELECT), f64::from(id), op(CLEAR_RECT), 0.0, 0.0, 800.0, 600.0, op(FILL_COLOR), color(200, 80, 40, 153)];
    for _ in 0..10000 {
        list.extend([op(FILL_RECT), random() * 790.0, random() * 590.0, 4.0 + random() * 20.0, 4.0 + random() * 20.0]);
    }
    time("10000 fillRect", &list);
    let sprite = create(32, 32, false);
    run(&[op(SELECT), f64::from(sprite), op(FILL_COLOR), color(0, 120, 255, 200), op(FILL_RECT), 0.0, 0.0, 32.0, 32.0], &[]);
    let mut list = vec![op(SELECT), f64::from(id), op(CLEAR_RECT), 0.0, 0.0, 800.0, 600.0];
    for _ in 0..2000 {
        let size = 20.0 + random() * 40.0;
        list.extend([op(DRAW_IMAGE), 1.0, f64::from(sprite), 0.0, 0.0, 32.0, 32.0, random() * 760.0, random() * 560.0, size, size]);
    }
    time("2000 drawImage scaled", &list);
    let mut list = vec![op(SELECT), f64::from(id), op(CLEAR_RECT), 0.0, 0.0, 800.0, 600.0];
    for _ in 0..2000 {
        list.extend([op(DRAW_IMAGE), 1.0, f64::from(sprite), 0.0, 0.0, 32.0, 32.0, (random() * 760.0).floor(), (random() * 560.0).floor(), 32.0, 32.0]);
    }
    time("2000 drawImage sprites", &list);
}

#[test]
#[ignore = "a measurement, for looking at with --nocapture"]
fn timing_parts() {
    let mut seed = 1u32;
    let mut random = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed as f32) / 4294967296.0
    };
    let segments: Vec<[f32; 4]> = (0..5000).map(|_| [random() * 800.0, random() * 600.0, random() * 800.0, random() * 600.0]).collect();
    let mut stroker = Stroker::default();
    let style = StrokeStyle::default();
    let mut path = Path::default();
    let mut outlines = Vec::new();
    let start = std::time::Instant::now();
    for s in &segments {
        path.clear();
        path.move_to((s[0], s[1]));
        path.line_to((s[2], s[3]));
        let mut lines = Vec::new();
        stroker.stroke(&path, &style, &Matrix::IDENTITY, &mut lines);
        outlines.push(lines);
    }
    println!("stroking: {:.2} ms", start.elapsed().as_secs_f64() * 1000.0);
    let mut raster = Rasterizer::default();
    let clip = IRect::new(0, 0, 800, 600);
    let mut cells = 0usize;
    let start = std::time::Instant::now();
    for lines in &outlines {
        raster.fill(lines, clip, false, |_, _, row| cells += row.len());
    }
    println!("rasterizing: {:.2} ms, {} cells handed out", start.elapsed().as_secs_f64() * 1000.0, cells);
    let rects: Vec<Vec<[f32; 4]>> = (0..10000)
        .map(|_| {
            let (x, y, w, h) = (random() * 790.0, random() * 590.0, 4.0 + random() * 20.0, 4.0 + random() * 20.0);
            vec![[x, y, x + w, y], [x + w, y, x + w, y + h], [x + w, y + h, x, y + h], [x, y + h, x, y]]
        })
        .collect();
    let start = std::time::Instant::now();
    for lines in &rects {
        raster.fill(lines, clip, false, |_, _, row| cells += row.len());
    }
    println!("rasterizing 10000 rects: {:.2} ms", start.elapsed().as_secs_f64() * 1000.0);
}

#[test]
#[ignore = "a measurement, for looking at with --nocapture"]
fn timing_image_parts() {
    let image = Rc::new(Image { width: 32, height: 32, pixels: vec![200; 32 * 32 * 4] });
    let shader = Shader::Image {
        image,
        to_image: Matrix::new([0.8, 0.0, 0.0, 0.8, 0.0, 0.0]),
        smooth: true,
        repeat: Repeat::Neither,
        bounds: [0, 0, 32, 32],
        pattern: false,
    };
    let mut out = vec![0u32; 40];
    let mut scratch = Vec::new();
    let start = std::time::Instant::now();
    let mut sum = 0u32;
    for i in 0..80_000 {
        shader.row32(i % 7, i % 40, 255, &mut out, &mut scratch);
        sum = sum.wrapping_add(out[3]);
    }
    println!("row32, 3.2M pixels: {:.2} ms ({sum})", start.elapsed().as_secs_f64() * 1000.0);
    let mut canvas = vec![10u8; 160];
    let cover = vec![1.0f32; 40];
    let start = std::time::Instant::now();
    for _ in 0..80_000 {
        paint::blend_row(&mut canvas, &cover, |i| out[i]);
    }
    println!("blend_row, 3.2M pixels: {:.2} ms ({})", start.elapsed().as_secs_f64() * 1000.0, canvas[5]);
    let id = create(800, 600, false);
    let sprite = create(32, 32, false);
    run(&[op(SELECT), f64::from(sprite), op(FILL_COLOR), color(0, 120, 255, 200), op(FILL_RECT), 0.0, 0.0, 32.0, 32.0], &[]);
    let list = vec![op(SELECT), f64::from(id), op(DRAW_IMAGE), 1.0, f64::from(sprite), 0.0, 0.0, 32.0, 32.0, 10.5, 10.5, 40.0, 40.0];
    let start = std::time::Instant::now();
    for _ in 0..2000 {
        run(&list, &[]);
    }
    println!("2000 single drawImage runs: {:.2} ms", start.elapsed().as_secs_f64() * 1000.0);
}

#[test]
#[ignore = "a measurement, for looking at with --nocapture"]
fn timing_hairline_cache() {
    let id = create(800, 600, false);
    let mut same = vec![op(SELECT), f64::from(id), op(STROKE_COLOR), color(30, 60, 200, 128)];
    for _ in 0..5000 {
        same.extend([op(BEGIN), op(MOVE), 10.0, 20.0, op(LINE), 380.0, 300.0, op(STROKE)]);
    }
    let start = std::time::Instant::now();
    run(&same, &[]);
    println!("5000 times the same 370-pixel line: {:.2} ms", start.elapsed().as_secs_f64() * 1000.0);
    let mut opaque = vec![op(SELECT), f64::from(id), op(STROKE_COLOR), color(30, 60, 200, 255)];
    for _ in 0..5000 {
        opaque.extend([op(BEGIN), op(MOVE), 10.0, 20.0, op(LINE), 380.0, 300.0, op(STROKE)]);
    }
    let start = std::time::Instant::now();
    run(&opaque, &[]);
    println!("the same, opaque: {:.2} ms", start.elapsed().as_secs_f64() * 1000.0);
}

#[test]
#[ignore = "a measurement, for looking at with --nocapture"]
fn timing_hair_walk() {
    let mut steps = 0u64;
    let start = std::time::Instant::now();
    for _ in 0..5000 {
        hair_segment((10.0, 20.0), (380.0, 300.0), &mut |_, u: (i32, i32), _, _, _| steps += (u.1 - u.0) as u64);
    }
    println!("walking alone: {:.2} ms ({steps})", start.elapsed().as_secs_f64() * 1000.0);
    let mut pixels = vec![0u8; 800 * 600 * 4];
    let start = std::time::Instant::now();
    for _ in 0..5000 {
        for p in 10..380usize {
            let index = (100 * 800 + p) * 4;
            paint::blend_pixel(&mut pixels[index..index + 4], 0x80c83c1e, 128);
            paint::blend_pixel(&mut pixels[index + 3200..index + 3204], 0x80c83c1e, 128);
        }
    }
    println!("blending alone, 3.7M pixels: {:.2} ms ({})", start.elapsed().as_secs_f64() * 1000.0, pixels[400 * 1000 + 1]);
}

#[test]
fn the_part_of_a_clip_that_lets_all_through() {
    // 6 by 5, with a soft edge round a 255 middle.
    let mut mask = vec![100u8; 30];
    for y in 1..4 {
        for x in 1..5 {
            mask[y * 6 + x] = 255;
        }
    }
    assert_eq!(full_part(&mask, 6, 5, false), Some([1, 1, 5, 4]));
    assert_eq!(full_part(&mask, 6, 5, true), Some([1, 1, 5, 4]));
    assert_eq!(full_part(&[0u8; 12], 4, 3, false), None);
    assert_eq!(full_part(&[255u8; 12], 4, 3, false), Some([0, 0, 4, 3]));
}

#[test]
fn drawing_through_a_soft_rectangle_clip_stays_inside_it() {
    // As charts clip: a rectangle on fractions of pixels, then bars and a hairline through it.
    let id = create(40, 30, false);
    run(
        &[
            op(SELECT), f64::from(id), op(BEGIN), op(RECT), 3.4, 2.5, 30.2, 20.0, op(CLIP), 0.0,
            op(FILL_COLOR), color(255, 0, 0, 128), op(BEGIN), op(RECT), 0.0, 0.0, 3.0, 30.0, op(FILL), 0.0,
            op(BEGIN), op(RECT), 10.3, 5.0, 2.6, 30.0, op(FILL), 0.0,
            op(BEGIN), op(MOVE), 0.0, 10.5, op(LINE), 40.0, 10.5, op(STROKE),
        ],
        &[],
    );
    // Nothing outside the clip.
    assert_eq!(pixel(id, 1, 10), [0, 0, 0, 0]);
    assert_eq!(pixel(id, 11, 25), [0, 0, 0, 0]);
    assert_eq!(pixel(id, 35, 10), [0, 0, 0, 0]);
    // The bar inside it, half covered across where it starts.
    assert_eq!(pixel(id, 11, 8)[3], 128);
    assert!(pixel(id, 10, 8)[3] > 0 && pixel(id, 10, 8)[3] < 128);
    // The hairline, all of a pixel inside, and partly at the clip's soft left edge.
    assert_eq!(pixel(id, 20, 10), [0, 0, 0, 255]);
    let edge = pixel(id, 3, 10)[3];
    assert!(edge > 0 && edge < 255, "{edge}");
}

/// A fill of many small shapes, big enough to be split into bands on threads, is the same as it filled without bands, by both
/// rules and laid on as usual, taken out and added.
#[test]
fn a_fill_split_into_bands_is_the_same_as_one_not() {
    let (w, h) = (400usize, 300usize);
    // Dots on a grid that don't touch, at fractions of a pixel, so their edges are partly covered.
    let mut dots = Vec::new();
    for y in 0..28 {
        for x in 0..38 {
            dots.push((5.3 + x as f64 * 10.4, 6.1 + y as f64 * 10.2, 3.6));
        }
    }
    let path = |dots: &[(f64, f64, f64)]| {
        let mut list = vec![op(BEGIN)];
        for &(x, y, r) in dots {
            // Not quite all the way round, so they're filled from their lines, not as circles.
            list.extend([op(MOVE), x + r, y, op(ARC), x, y, r, 0.0, 6.1, 0.0]);
        }
        list
    };
    for (composite, even_odd) in [(0.0, 0.0), (0.0, 1.0), (6.0, 0.0), (8.0, 0.0)] {
        let draw = |split: bool| {
            let id = create(w as u32, h as u32, false);
            let mut list = vec![op(SELECT), f64::from(id), op(FILL_COLOR), color(40, 90, 200, 255), op(FILL_RECT), 0.0, 0.0, w as f64, h as f64];
            list.extend([op(COMPOSITE), composite, op(FILL_COLOR), color(250, 200, 60, 180)]);
            list.extend(path(&dots));
            list.extend([op(FILL), even_odd]);
            super::pool::SERIAL.with(|serial| serial.set(!split));
            run(&list, &[]);
            super::pool::SERIAL.with(|serial| serial.set(false));
            let mut out = vec![0u8; w * h * 4];
            read(id, 0, 0, w as u32, h as u32, &mut out);
            out
        };
        assert!(dots.len() * 13 >= 2048, "enough lines to be split");
        assert!(draw(true) == draw(false), "composite {composite} even-odd {even_odd}");
    }
}

/// A rectangle of a gradient big enough to be split into bands on threads is the same as it drawn without bands, linear and
/// radial, its edges on fractions of a pixel.
#[test]
fn a_gradient_split_into_bands_is_the_same_as_one_not() {
    let (w, h) = (640usize, 420usize);
    for kind in [LINEAR, RADIAL] {
        let draw = |split: bool| {
            let id = create(w as u32, h as u32, false);
            let mut list = vec![op(SELECT), f64::from(id), op(FILL_COLOR), color(10, 20, 30, 255), op(FILL_RECT), 0.0, 0.0, w as f64, h as f64];
            if kind == LINEAR {
                list.extend([op(LINEAR), 9.0, 20.0, 10.0, 600.0, 380.0]);
            } else {
                list.extend([op(RADIAL), 9.0, 300.0, 200.0, 0.0, 300.0, 200.0, 350.0]);
            }
            list.extend([op(STOP), 9.0, 0.0, color(250, 120, 40, 200), op(STOP), 9.0, 1.0, color(40, 80, 250, 0)]);
            list.extend([op(FILL_GRADIENT), 9.0, op(FILL_RECT), 0.5, 1.25, 630.0, 410.6]);
            super::pool::SERIAL.with(|serial| serial.set(!split));
            run(&list, &[]);
            super::pool::SERIAL.with(|serial| serial.set(false));
            let mut out = vec![0u8; w * h * 4];
            read(id, 0, 0, w as u32, h as u32, &mut out);
            out
        };
        assert!(draw(true) == draw(false), "{kind}");
    }
}

/// Dots filled as circles, from each pixel's distance to the center, cover what the same dots filled from their lines do: the
/// same area, and no pixel more than a little different, at the edge.
#[test]
fn circles_cover_what_their_lines_do() {
    let (w, h) = (200usize, 120usize);
    let mut dots = Vec::new();
    for y in 0..5 {
        for x in 0..9 {
            dots.push((10.3 + x as f64 * 21.7, 11.6 + y as f64 * 22.1, 1.5 + (x + y) as f64 * 0.9));
        }
    }
    let draw = |whole: bool| {
        let id = create(w as u32, h as u32, false);
        let mut list = vec![op(SELECT), f64::from(id), op(FILL_COLOR), color(255, 255, 255, 255), op(BEGIN)];
        // All the way round is a circle; a hair short of it is filled from its lines, with the gap closed by a line.
        let sweep = if whole { std::f64::consts::TAU } else { std::f64::consts::TAU - 0.0002 };
        for &(x, y, r) in &dots {
            list.extend([op(MOVE), x + r, y, op(ARC), x, y, r, 0.0, sweep, 0.0]);
        }
        list.extend([op(FILL), 0.0]);
        run(&list, &[]);
        let mut out = vec![0u8; w * h * 4];
        read(id, 0, 0, w as u32, h as u32, &mut out);
        out
    };
    let (circles, lines) = (draw(true), draw(false));
    let area = |px: &[u8]| px.chunks_exact(4).map(|p| f64::from(p[3]) / 255.0).sum::<f64>();
    let true_area: f64 = dots.iter().map(|d| std::f64::consts::PI * d.2 * d.2).sum();
    assert!((area(&circles) - true_area).abs() / true_area < 0.02, "circles {} against {true_area}", area(&circles));
    assert!((area(&lines) - true_area).abs() / true_area < 0.02, "lines {} against {true_area}", area(&lines));
    let most = circles.chunks_exact(4).zip(lines.chunks_exact(4)).map(|(a, b)| a[3].abs_diff(b[3])).max().unwrap();
    assert!(most <= 64, "{most}");
}

/// A grid of small rectangles filled one by one (see `Path::rects`) covers what filling them from their lines does, to the pixel.
#[test]
fn rects_cover_what_their_lines_do() {
    let (w, h) = (160usize, 90usize);
    let draw = |separately: bool, alpha: u8| {
        let id = create(w as u32, h as u32, false);
        let mut list = vec![op(SELECT), f64::from(id), op(FILL_COLOR), color(40, 120, 255, alpha), op(BEGIN)];
        for row in 0..20 {
            let shift = if row % 2 == 0 { 0.0 } else { 2.0 };
            for col in 0..38 {
                // Some on whole pixels, some not, none sharing a pixel.
                let (x, y) = (shift + col as f64 * 4.0 + 0.3 * f64::from(col % 3 == 1), row as f64 * 4.0 + 0.25 * f64::from(row % 2));
                list.extend([op(RECT), x, y, 2.0, 2.0]);
            }
        }
        if !separately {
            // A part with no inside, which the rectangles can't be filled one by one with, so that their lines are walked.
            list.extend([op(MOVE), 1.0, 85.0, op(LINE), 5.0, 85.0, op(LINE), 9.0, 85.0]);
        }
        list.extend([op(FILL), 0.0]);
        run(&list, &[]);
        let mut out = vec![0u8; w * h * 4];
        read(id, 0, 0, w as u32, h as u32, &mut out);
        out
    };
    for alpha in [255, 90] {
        let (each, walked) = (draw(true, alpha), draw(false, alpha));
        let most = each.iter().zip(walked.iter()).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(most <= 2, "alpha {alpha}: {most}");
        assert!(each.iter().any(|v| *v != 0));
    }
}

#[test]
#[ignore = "a measurement, for looking at with --nocapture"]
fn timing_dots() {
    let (w, h) = (3840usize, 2160usize);
    let id = create(w as u32, h as u32, false);
    for (name, rects) in [("rects", true), ("walked", false), ("build only", true)] {
        let mut list = vec![op(SELECT), f64::from(id), op(FILL_COLOR), color(120, 140, 255, 40), op(BEGIN)];
        let mut count = 0;
        for row in 0..540 {
            let shift = if row % 2 == 0 { 0.0 } else { 2.0 };
            for col in 0..400 {
                if (row * 7 + col * 3) % 10 < 4 {
                    list.extend([op(RECT), shift + col as f64 * 4.0, row as f64 * 4.0, 2.0, 2.0]);
                    count += 1;
                }
            }
        }
        if !rects {
            list.extend([op(MOVE), 1.0, 2000.0, op(LINE), 5.0, 2000.0, op(LINE), 9.0, 2000.0]);
        }
        if name != "build only" {
            list.extend([op(FILL), 0.0]);
        }
        let mut best = f64::MAX;
        for _ in 0..7 {
            let start = std::time::Instant::now();
            run(&list, &[]);
            best = best.min(start.elapsed().as_secs_f64() * 1000.0);
        }
        println!("{name}: {count} dots, best of 7: {best:.2} ms");
    }
}

/// `fillDither()` draws the dots that a `fillRect()` for each of them draws, where the field is above their threshold, and what
/// follows it in the list is still drawn (the field is skipped over right).
#[test]
fn dither_draws_the_dots_a_fill_rect_each_would() {
    let (w, h) = (120usize, 80usize);
    let (cols, rows, cell) = (9usize, 6usize, 14.0f64);
    let field: Vec<f64> = (0..rows * cols).map(|k| 1.7 - ((k % cols) as f64 * 0.17 + (k / cols) as f64 * 0.21)).collect();
    let bayer = [0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0];
    let at = |x: f64, y: f64| {
        let (i, j) = (((x / cell) as usize).min(cols - 2), ((y / cell) as usize).min(rows - 2));
        let (fx, fy) = (x / cell - i as f64, y / cell - j as f64);
        let top = field[j * cols + i] * (1.0 - fx) + field[j * cols + i + 1] * fx;
        let bottom = field[(j + 1) * cols + i] * (1.0 - fx) + field[(j + 1) * cols + i + 1] * fx;
        top + (bottom - top) * fy
    };
    let draw = |native: bool| {
        let id = create(w as u32, h as u32, false);
        let mut list = vec![op(SELECT), f64::from(id), op(FILL_COLOR), color(255, 255, 255, 120)];
        // Moved and scaled, as it may be.
        list.extend([op(TRANSFORM), 1.0, 0.0, 0.0, 1.0, 3.0, 2.0]);
        if native {
            list.extend([op(FILL_DITHER), cols as f64, rows as f64, cell, 4.0, 2.0]);
            list.extend(field.iter().copied());
        } else {
            let (width, height) = ((cols - 1) as f64 * cell, (rows - 1) as f64 * cell);
            let mut row = 0usize;
            while row as f64 * 4.0 <= height {
                let y = row as f64 * 4.0;
                let mut col = 0usize;
                loop {
                    let x = if row % 2 == 0 { 0.0 } else { 2.0 } + col as f64 * 4.0;
                    if x > width {
                        break;
                    }
                    if at(x, y) > (bayer[(row % 4) * 4 + col % 4] + 0.5) / 16.0 {
                        list.extend([op(FILL_RECT), x, y, 2.0, 2.0]);
                    }
                    col += 1;
                }
                row += 1;
            }
        }
        // Something after it, which a field read wrongly would lose.
        list.extend([op(FILL_COLOR), color(255, 0, 0, 255), op(FILL_RECT), 100.0, 60.0, 4.0, 4.0]);
        run(&list, &[]);
        let mut out = vec![0u8; w * h * 4];
        read(id, 0, 0, w as u32, h as u32, &mut out);
        out
    };
    let (native, each) = (draw(true), draw(false));
    assert!(native.iter().any(|v| *v != 0));
    let most = native.iter().zip(each.iter()).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    assert!(most <= 1, "{most}");
    // The red square after it is there.
    let red = ((62 * w) + 103) * 4;
    assert_eq!(&native[red..red + 4], &[255, 0, 0, 255]);
}
