//! Images, from a file or built in, to pixels on screen, drawn offscreen so no window opens.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use rsui::prelude::*;
use rsui::render::Renderer;
use rsui::text::TextEngine;
use rsui::{App, Frame, register_image};
use support::plain_font;

const SIZE: (u32, u32) = (100, 100);
const LONG_ENOUGH: Duration = Duration::from_secs(20);
const BACKGROUND_PIXEL: [u8; 4] = [0xF6, 0xF8, 0xFA, 255];

/// A PNG, without any compression.
fn png(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            raw.extend(pixel(x, y));
        }
    }
    let mut packed = vec![0x78, 0x01];
    let mut pieces = raw.chunks(65_535).peekable();
    while let Some(piece) = pieces.next() {
        packed.push(u8::from(pieces.peek().is_none()));
        packed.extend((piece.len() as u16).to_le_bytes());
        packed.extend((!(piece.len() as u16)).to_le_bytes());
        packed.extend(piece);
    }
    packed.extend([0; 4]);
    let mut file = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |tag: &[u8; 4], body: &[u8]| {
        file.extend((body.len() as u32).to_be_bytes());
        file.extend(tag);
        file.extend(body);
        file.extend([0; 4]);
    };
    let mut header = Vec::new();
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &packed);
    chunk(b"IEND", &[]);
    file
}

fn built_in(name: &str, width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) {
    register_image(
        name,
        Box::leak(png(width, height, pixel).into_boxed_slice()),
    );
}

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

#[derive(Clone, PartialEq)]
struct Shown {
    source: String,
    width: Option<f32>,
    height: Option<f32>,
    fit: Option<&'static str>,
    radius: f32,
}

impl Shown {
    fn new(source: &str) -> Self {
        Self {
            source: source.to_owned(),
            width: None,
            height: None,
            fit: None,
            radius: 0.0,
        }
    }

    fn sized(mut self, width: f32, height: f32) -> Self {
        self.width = Some(width);
        self.height = Some(height);
        self
    }
}

/// One image, at the top left.
fn one(_: &mut Cx, shown: &Shown) -> Element {
    let mut style = Style::new();
    if let Some(width) = shown.width {
        style = style.width(width);
    }
    if let Some(height) = shown.height {
        style = style.height(height);
    }
    let mut element = image(
        &shown.source,
        "a picture",
        rsui::core::Props::from(style).merged(&Paint::new().corner_radius(shown.radius).into()),
    );
    if let Some(fit) = shown.fit {
        element = element.prop("fit", fit);
    }
    Element::new(Role::Window)
        .props(style!(Style::new().align(Align::Start)))
        .child(element)
}

struct Screen {
    app: App,
    renderer: Renderer,
}

impl Screen {
    fn new(shown: Shown) -> Self {
        Self {
            app: App::new(one, shown, TextEngine::new(plain_font()), SIZE, 1.0),
            renderer: Renderer::offscreen(SIZE).expect("the software GPU is always there"),
        }
    }

    /// Draws, waits for what that asks for to be read, and draws what came of it.
    fn settle(&mut self) -> Vec<u8> {
        self.app.update();
        self.app.render(&mut self.renderer).expect("drawing works");
        assert!(
            self.app.wait_for_images(LONG_ENOUGH),
            "it should have been read by now"
        );
        self.app.update();
        self.app.render(&mut self.renderer).expect("drawing works");
        self.renderer.read_pixels().expect("reading works")
    }

    fn pixel(&mut self, x: usize, y: usize) -> [u8; 4] {
        let pixels = self.settle();
        let at = (y * SIZE.0 as usize + x) * 4;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    }
}

fn close(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 2)
}

#[test]
fn an_image_fades_with_opacity() {
    fn see_through(_: &mut Cx, _: &()) -> Element {
        let picture = image(
            "faded",
            "a picture",
            rsui::core::Props::from(Style::new().width(40.0).height(40.0)),
        )
        .prop("opacity", 0.5);
        Element::new(Role::Window)
            .props(style!(Style::new().align(Align::Start)))
            .child(picture)
    }
    built_in("faded", 4, 4, |_, _| RED);
    let mut app = App::new(see_through, (), TextEngine::new(plain_font()), SIZE, 1.0);
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    app.update();
    app.render(&mut renderer).unwrap();
    assert!(app.wait_for_images(LONG_ENOUGH));
    app.update();
    app.render(&mut renderer).unwrap();
    let pixels = renderer.read_pixels().unwrap();
    let at = (20 * SIZE.0 as usize + 20) * 4;
    let half: [u8; 4] = [250, 0xF8 / 2, 0xFA / 2, 255];
    assert!(
        close(pixels[at..at + 4].try_into().unwrap(), half),
        "{:?}",
        &pixels[at..at + 4]
    );
}

const ANIMATED: &[u8] = include_bytes!("../../rsui-image/tests/fixtures/apng_source.png");

fn animated(cx: &mut Cx, _: &()) -> Element {
    let shown = cx.state(|| true);
    let mut root = Element::new(Role::Window)
        .key("root")
        .props(style!(Style::new().align(Align::Start)));
    if cx.get(shown) {
        root = root.child(image(
            "playing",
            "something moving",
            rsui::core::Props::from(Style::new().width(24.0).height(18.0)),
        ));
    }
    cx.on(root, ActionKind::Toggle, move |cx, _| cx.set(shown, false))
}

struct Playing {
    app: App,
    renderer: Renderer,
}

impl Playing {
    fn new(animations: bool) -> Self {
        register_image("playing", ANIMATED);
        let mut app = App::new(animated, (), TextEngine::new(plain_font()), SIZE, 1.0);
        app.set_animations(animations);
        let mut renderer = Renderer::offscreen(SIZE).expect("the software GPU is always there");
        app.update();
        app.render(&mut renderer).unwrap();
        assert!(app.wait_for_images(LONG_ENOUGH));
        app.update();
        app.render(&mut renderer).unwrap();
        Self { app, renderer }
    }

    fn pixels(&mut self) -> Vec<u8> {
        self.app.update();
        self.app.render(&mut self.renderer).unwrap();
        self.renderer.read_pixels().unwrap()
    }
}

#[test]
fn an_animated_image_plays_its_frames_in_turn_and_starts_again() {
    let mut screen = Playing::new(true);
    let start = Instant::now();
    assert!(
        screen.app.next_wake().is_some(),
        "it wants to hear when its next frame is due"
    );
    // Frames last 40, 60, 80, 100 and 120 milliseconds. Being a little behind doesn't matter here, as
    // what's being checked is that going on by a whole play comes back to the same frame.
    let mut seen = Vec::new();
    for millis in [0u64, 20, 70, 140, 240, 340] {
        screen.app.wake_at(start + Duration::from_millis(millis));
        seen.push(screen.pixels());
    }
    let mut different = seen.clone();
    different.sort();
    different.dedup();
    assert!(
        different.len() >= 4,
        "it moved through its frames: {}",
        different.len()
    );

    screen
        .app
        .wake_at(start + Duration::from_millis(400 * 50 + 70));
    let later = screen.pixels();
    screen
        .app
        .wake_at(start + Duration::from_millis(400 * 51 + 70));
    assert_eq!(
        screen.pixels(),
        later,
        "a whole play later, it's on the same frame"
    );
}

#[test]
fn a_new_frame_asks_for_the_window_to_draw_and_the_same_one_does_not() {
    let mut screen = Playing::new(true);
    let start = Instant::now();
    screen.app.wake_at(start + Duration::from_secs(10));
    screen.pixels();
    assert_eq!(screen.app.update(), Frame::Unchanged);
    // Half of a frame later would be the same frame, unless this lands right on the edge of one, and
    // somewhere in a whole play has to be a different one.
    let moved = (1..=400u64).any(|millis| {
        screen
            .app
            .wake_at(start + Duration::from_secs(10) + Duration::from_millis(millis));
        screen.app.update() == Frame::Needed
    });
    assert!(moved);
}

#[test]
fn with_animations_off_an_image_stays_on_its_first_frame() {
    let mut screen = Playing::new(false);
    let first = screen.pixels();
    assert!(
        screen.app.next_wake().is_none(),
        "there's nothing to wait for"
    );
    let start = Instant::now();
    for millis in [50u64, 150, 300] {
        screen.app.wake_at(start + Duration::from_millis(millis));
        assert_eq!(screen.app.update(), Frame::Unchanged);
    }
    assert_eq!(screen.pixels(), first);
}

#[test]
fn every_frame_counts_towards_gpu_memory_and_all_of_it_is_given_back() {
    let mut screen = Playing::new(true);
    screen.pixels();
    let per_frame = 24 * 18 * 4;
    assert_eq!(screen.app.images_gpu_bytes(), 5 * per_frame);
    screen
        .app
        .runtime_mut()
        .dispatch(&[], ActionKind::Toggle, None)
        .unwrap();
    screen.pixels();
    screen.app.forget_caches();
    assert_eq!(screen.app.images_gpu_bytes(), 0);
    assert_eq!(
        screen.app.scene().dropped_images.len(),
        5,
        "and the GPU's told to let go of each of them"
    );
}

#[test]
fn an_animated_image_that_is_not_showing_asks_for_nothing() {
    let mut screen = Playing::new(true);
    screen.pixels();
    assert!(screen.app.next_wake().is_some());
    screen
        .app
        .runtime_mut()
        .dispatch(&[], ActionKind::Toggle, None)
        .unwrap();
    screen.pixels();
    assert!(screen.app.next_wake().is_none());
}

#[test]
fn webp_shows_up_like_anything_else() {
    const WEBP: &[u8] = include_bytes!("../../rsui-image/tests/fixtures/lossless.webp");
    register_image("a webp", WEBP);
    let mut screen = Screen::new(Shown::new("a webp").sized(40.0, 33.0));
    let plain = BACKGROUND_PIXEL;
    let inside = screen.pixel(20, 16);
    assert_ne!(inside, plain);
    assert_eq!(screen.pixel(80, 80), plain);
}

#[test]
fn an_image_shows_at_its_own_size() {
    built_in("own", 40, 20, |x, _| if x < 20 { RED } else { BLUE });
    let mut screen = Screen::new(Shown::new("own"));
    assert_eq!(screen.pixel(5, 5), RED);
    assert_eq!(screen.pixel(35, 5), BLUE);
    assert_eq!(screen.pixel(45, 5), BACKGROUND_PIXEL, "past its right edge");
    assert_eq!(
        screen.pixel(5, 25),
        BACKGROUND_PIXEL,
        "past its bottom edge"
    );
    let rect = screen.app.layout().rect(&[0]).expect("it's laid out");
    assert_eq!((rect.width, rect.height), (40.0, 20.0));
}

#[test]
fn it_shows_up_once_it_has_been_read_and_asks_to_be_drawn() {
    built_in("later", 10, 10, |_, _| GREEN);
    let mut screen = Screen::new(Shown::new("later"));
    screen.app.update();
    screen.app.render(&mut screen.renderer).unwrap();
    assert!(screen.app.wait_for_images(LONG_ENOUGH));
    use rsui::Frame;
    assert_eq!(
        screen.app.update(),
        Frame::Needed,
        "it has something new to show"
    );
    screen.app.render(&mut screen.renderer).unwrap();
    assert_eq!(screen.app.update(), Frame::Unchanged);
    assert_eq!(screen.pixel(5, 5), GREEN);
}

#[test]
fn a_waker_hears_when_it_is_ready() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    built_in("wakes", 10, 10, |_, _| GREEN);
    let mut screen = Screen::new(Shown::new("wakes"));
    let woke = Arc::new(AtomicBool::new(false));
    let flag = woke.clone();
    screen
        .app
        .set_waker(move || flag.store(true, Ordering::SeqCst));
    screen.app.update();
    screen.app.render(&mut screen.renderer).unwrap();
    assert!(screen.app.wait_for_images(LONG_ENOUGH));
    let started = std::time::Instant::now();
    while !woke.load(Ordering::SeqCst) && started.elapsed() < LONG_ENOUGH {
        std::thread::yield_now();
    }
    assert!(woke.load(Ordering::SeqCst));
}

#[test]
fn an_image_too_wide_for_its_place_keeps_its_shape() {
    built_in("wide", 400, 200, |_, _| RED);
    let mut screen = Screen::new(Shown::new("wide"));
    assert_eq!(screen.pixel(50, 25), RED);
    let rect = screen.app.layout().rect(&[0]).expect("it's laid out");
    assert_eq!((rect.width, rect.height), (100.0, 50.0));
    assert_eq!(screen.pixel(50, 60), BACKGROUND_PIXEL);
}

#[test]
fn each_way_of_fitting_does_what_it_says() {
    // Red on the left half, and blue on the right, shown in a square.
    built_in("half", 40, 20, |x, _| if x < 20 { RED } else { BLUE });
    let shown = |fit| Shown {
        fit,
        ..Shown::new("half").sized(80.0, 80.0)
    };
    // All of it shows, with room above and below.
    let mut contain = Screen::new(shown(None));
    assert_eq!(contain.pixel(40, 5), BACKGROUND_PIXEL);
    assert_eq!(contain.pixel(10, 40), RED);
    assert_eq!(contain.pixel(70, 40), BLUE);
    // It fills the space, and its sides are cut off, so each color takes half of what's left.
    let mut cover = Screen::new(shown(Some("cover")));
    assert_eq!(cover.pixel(10, 5), RED);
    assert_eq!(cover.pixel(70, 5), BLUE);
    assert!(close(cover.pixel(30, 40), RED));
    // It's stretched.
    let mut fill = Screen::new(shown(Some("fill")));
    assert_eq!(fill.pixel(10, 5), RED);
    assert_eq!(fill.pixel(70, 75), BLUE);
}

#[test]
fn cover_cuts_off_what_does_not_fit() {
    // Green on the outer quarters, which should be cut off.
    built_in("edges", 40, 20, |x, _| {
        if (10..30).contains(&x) { RED } else { GREEN }
    });
    let mut cover = Screen::new(Shown {
        fit: Some("cover"),
        ..Shown::new("edges").sized(80.0, 80.0)
    });
    assert!(close(cover.pixel(3, 40), RED), "{:?}", cover.pixel(3, 40));
    assert!(close(cover.pixel(76, 40), RED), "{:?}", cover.pixel(76, 40));
}

#[test]
fn corners_can_be_round() {
    built_in("square", 10, 10, |_, _| RED);
    let mut screen = Screen::new(Shown {
        radius: 20.0,
        ..Shown::new("square").sized(60.0, 60.0)
    });
    assert_eq!(screen.pixel(30, 30), RED);
    assert_eq!(
        screen.pixel(1, 1),
        BACKGROUND_PIXEL,
        "cut off at the corner"
    );
}

#[test]
fn see_through_parts_blend_with_what_is_behind() {
    built_in("glass", 10, 10, |_, _| [255, 0, 0, 0]);
    let mut screen = Screen::new(Shown::new("glass"));
    assert_eq!(screen.pixel(5, 5), BACKGROUND_PIXEL);
}

#[test]
fn what_cannot_be_read_shows_nothing_and_breaks_nothing() {
    register_image("noise", b"this isn't an image at all");
    for source in ["C:/there/is/no/such/file.png", "noise"] {
        let mut screen = Screen::new(Shown::new(source));
        assert_eq!(screen.pixel(5, 5), BACKGROUND_PIXEL);
        let rect = screen.app.layout().rect(&[0]).expect("it's laid out");
        assert_eq!((rect.width, rect.height), (0.0, 0.0));
    }
    // Given a size, it keeps it, so nothing moves around it.
    let mut screen = Screen::new(Shown::new("noise").sized(30.0, 30.0));
    assert_eq!(screen.pixel(5, 5), BACKGROUND_PIXEL);
    assert_eq!(
        screen.app.layout().rect(&[0]).expect("it's laid out").width,
        30.0
    );
}

#[test]
fn damage_past_its_size_only_fails_the_reading() {
    let mut file = png(10, 10, |_, _| RED);
    // Its size is fine, but the data isn't.
    let at = file.len() - 40;
    file.truncate(at);
    file.extend([0xAA; 20]);
    register_image("damaged", Box::leak(file.into_boxed_slice()));
    let mut screen = Screen::new(Shown::new("damaged"));
    screen.settle();
    screen.settle();
    assert_eq!(
        screen.app.layout().rect(&[0]).expect("it's laid out").width,
        10.0
    );
}

#[test]
fn it_reads_files_too() {
    let path = std::env::temp_dir().join(format!("rsui-image-test-{}.png", std::process::id()));
    std::fs::write(&path, png(20, 10, |_, _| BLUE))
        .expect("the temporary folder can be written to");
    let source = path.to_str().expect("it's text");
    let mut screen = Screen::new(Shown::new(source));
    assert_eq!(screen.pixel(5, 5), BLUE);
    assert_eq!(
        screen.app.layout().rect(&[0]).expect("it's laid out").width,
        20.0
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn only_what_is_shown_goes_to_the_gpu() {
    built_in("big", 400, 400, |_, _| RED);
    let mut screen = Screen::new(Shown::new("big").sized(20.0, 20.0));
    assert_eq!(screen.pixel(5, 5), RED);
    assert_eq!(screen.app.images_gpu_bytes(), 20 * 20 * 4);
}

#[test]
fn it_is_read_again_when_it_needs_to_be_much_sharper_or_can_be_much_smaller() {
    fn resizable(cx: &mut Cx, _: &()) -> Element {
        let width = cx.state(|| 20f64);
        let side = cx.get(width) as f32;
        let image = image("resized", "", Style::new().width(side).height(side));
        let root = Element::new(Role::Window)
            .key("root")
            .props(style!(Style::new().align(Align::Start)))
            .child(image);
        cx.on(root, ActionKind::SetValue, move |cx, input| {
            if let Some(PropValue::Float(side)) = input {
                cx.set(width, *side);
            }
        })
    }
    fn set(app: &mut App, side: f64) {
        app.runtime_mut()
            .dispatch(&[], ActionKind::SetValue, Some(PropValue::Float(side)))
            .unwrap();
    }
    built_in("resized", 400, 400, |_, _| RED);
    let mut app = App::new(resizable, (), TextEngine::new(plain_font()), SIZE, 1.0);
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    let mut settle = |app: &mut App| {
        app.update();
        app.render(&mut renderer).unwrap();
        assert!(app.wait_for_images(LONG_ENOUGH));
        app.update();
        app.render(&mut renderer).unwrap();
    };
    settle(&mut app);
    assert_eq!(app.images_gpu_bytes(), 20 * 20 * 4);
    // Being a bit smaller than it was read for doesn't read it again.
    set(&mut app, 15.0);
    settle(&mut app);
    assert_eq!(app.images_gpu_bytes(), 20 * 20 * 4);
    // Being bigger does, as it'd look blurred.
    set(&mut app, 60.0);
    settle(&mut app);
    assert_eq!(app.images_gpu_bytes(), 60 * 60 * 4);
    // Being much smaller does, as that saves memory.
    set(&mut app, 20.0);
    settle(&mut app);
    assert_eq!(app.images_gpu_bytes(), 20 * 20 * 4);
}

#[test]
fn an_image_is_read_at_the_size_it_takes_on_the_screen() {
    built_in("scaled", 400, 400, |_, _| RED);
    let mut app = App::new(
        one,
        Shown::new("scaled").sized(20.0, 20.0),
        TextEngine::new(plain_font()),
        (200, 200),
        2.0,
    );
    let mut renderer = Renderer::offscreen((200, 200)).unwrap();
    app.update();
    app.render(&mut renderer).unwrap();
    assert!(app.wait_for_images(LONG_ENOUGH));
    app.update();
    app.render(&mut renderer).unwrap();
    assert_eq!(app.images_gpu_bytes(), 40 * 40 * 4);
}

#[test]
fn a_new_gpu_gets_the_picture_back_at_once() {
    built_in("again", 10, 10, |_, _| GREEN);
    let mut screen = Screen::new(Shown::new("again"));
    assert_eq!(screen.pixel(5, 5), GREEN);
    screen.renderer = Renderer::offscreen(SIZE).unwrap();
    screen.app.renderer_replaced();
    // Multiple conservative invalidations before paint must not lose immediate replay.
    screen.app.renderer_replaced();
    // No waiting: the first thing drawn has it.
    screen.app.render(&mut screen.renderer).unwrap();
    let pixels = screen.renderer.read_pixels().unwrap();
    let at = (5 * SIZE.0 as usize + 5) * 4;
    assert_eq!(&pixels[at..at + 4], GREEN);
}

#[test]
fn going_quiet_lets_go_of_what_is_off_screen_only() {
    fn two(cx: &mut Cx, _: &()) -> Element {
        built_in("shows", 10, 10, |_, _| RED);
        built_in("hidden", 10, 10, |_, _| BLUE);
        let content = Element::new(Role::Group)
            .props(style!(Style::new().flex(1.0)))
            .child(image("shows", "", Style::new().height(100.0).width(100.0)))
            .child(image("hidden", "", Style::new().height(100.0).width(100.0)));
        Element::new(Role::Window).child(cx.scroll_area("area", ScrollAxis::Vertical, content))
    }
    let mut app = App::new(two, (), TextEngine::new(plain_font()), SIZE, 1.0);
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    let mut settle = |app: &mut App| {
        app.update();
        app.render(&mut renderer).unwrap();
        assert!(app.wait_for_images(LONG_ENOUGH));
        app.update();
        app.render(&mut renderer).unwrap();
    };
    settle(&mut app);
    // The one out of view was never even read.
    assert_eq!(app.images_gpu_bytes(), 10 * 10 * 4);
    // Scroll it into view, then back, and it stays until things go quiet.
    app.scroll(50.0, 50.0, 100).unwrap();
    settle(&mut app);
    assert_eq!(app.images_gpu_bytes(), 2 * 10 * 10 * 4);
    app.forget_caches();
    settle(&mut app);
    assert_eq!(app.images_gpu_bytes(), 10 * 10 * 4);
}

#[test]
fn two_images_can_swap_places_and_draw_right_on_flipping_buffers() {
    fn swapped(cx: &mut Cx, _: &()) -> Element {
        built_in("left", 10, 10, |_, _| RED);
        built_in("right", 10, 10, |_, _| BLUE);
        let order = cx.state(|| true);
        let (first, second) = if cx.get(order) {
            ("left", "right")
        } else {
            ("right", "left")
        };
        let root = Element::new(Role::Window)
            .key("root")
            .props(style!(Style::row()))
            .child(image(
                first,
                "",
                style!(Style::new().width(50.0).height(100.0)),
            ))
            .child(image(
                second,
                "",
                style!(Style::new().width(50.0).height(100.0)),
            ));
        cx.on(root, ActionKind::SetValue, move |cx, input| {
            if let Some(PropValue::Bool(value)) = input {
                cx.set(order, *value);
            }
        })
    }
    fn set(app: &mut App, order: bool) {
        app.runtime_mut()
            .dispatch(&[], ActionKind::SetValue, Some(PropValue::Bool(order)))
            .unwrap();
    }
    let mut app = App::new(swapped, (), TextEngine::new(plain_font()), SIZE, 1.0);
    let mut renderer = Renderer::offscreen_flipping(SIZE).unwrap();
    let mut draw = |app: &mut App| {
        app.update();
        app.render(&mut renderer).unwrap();
        assert!(app.wait_for_images(LONG_ENOUGH));
        app.update();
        app.render(&mut renderer).unwrap();
        // A second time, so that both buffers have had a turn.
        app.render(&mut renderer).unwrap();
        let pixels = renderer.read_pixels().unwrap();
        let at = |x: usize| {
            let at = (50 * SIZE.0 as usize + x) * 4;
            [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
        };
        (at(25), at(75))
    };
    assert_eq!(draw(&mut app), (RED, BLUE));
    set(&mut app, false);
    assert_eq!(draw(&mut app), (BLUE, RED));
    set(&mut app, true);
    assert_eq!(draw(&mut app), (RED, BLUE));
}

#[test]
fn text_after_an_image_still_draws() {
    fn mixed(_: &mut Cx, _: &()) -> Element {
        built_in("before", 10, 10, |_, _| RED);
        Element::new(Role::Window)
            .props(style!(Style::new().align(Align::Start)))
            .child(image(
                "before",
                "",
                style!(Style::new().width(10.0).height(10.0)),
            ))
            .child(
                Element::new(Role::Button)
                    .name("A")
                    .props(style!(Paint::new().background(Color::hex(0x00FF00))))
                    .prop("font_size", 40.0),
            )
    }
    let mut app = App::new(mixed, (), TextEngine::new(plain_font()), SIZE, 1.0);
    let mut renderer = Renderer::offscreen(SIZE).unwrap();
    app.update();
    app.render(&mut renderer).unwrap();
    assert!(app.wait_for_images(LONG_ENOUGH));
    app.update();
    app.render(&mut renderer).unwrap();
    let pixels = renderer.read_pixels().unwrap();
    let row = |y: usize| &pixels[y * 100 * 4..(y + 1) * 100 * 4];
    assert!(row(5).chunks(4).any(|pixel| pixel == RED));
    assert!(
        row(20).chunks(4).any(|pixel| pixel == GREEN),
        "the button's background"
    );
    // The glyph inside it, drawn after the image, in the glyph shader.
    let dark = row(25).chunks(4).any(|pixel| pixel[1] < 200);
    assert!(dark, "its text");
}
