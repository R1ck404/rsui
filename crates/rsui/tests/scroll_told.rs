//! What hears that something's scrolled is told once a frame, as a browser fires `scroll`: as the frame starts, where that frame
//! shows it, before it's laid out and drawn, so that what the page moves with it is drawn in the same frame. A host that
//! draws without ticking hears it once the frame's drawn.

#[path = "../../rsui-text/tests/support/mod.rs"]
mod support;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use rsui::App;
use rsui::prelude::*;
use rsui::runtime::ScrollControl;
use rsui::text::TextEngine;
use support::plain_font;

const SIZE: (u32, u32) = (200, 100);
const AREA: &[u32] = &[0];

thread_local! {
    /// Where each area was when it was told, in the order it was told: which area, and how far down.
    static TOLD: RefCell<Vec<(&'static str, f64)>> = const { RefCell::new(Vec::new()) };
    /// How long telling takes, as a page's `scroll` listeners might.
    static SLOW: RefCell<Duration> = const { RefCell::new(Duration::ZERO) };
    /// Whether the second area's there.
    static SECOND: RefCell<bool> = const { RefCell::new(false) };
}

fn told() -> Vec<(&'static str, f64)> {
    TOLD.with(|told| told.borrow().clone())
}

fn reset(slow: Duration, second: bool) {
    TOLD.with(|told| told.borrow_mut().clear());
    SLOW.with(|said| *said.borrow_mut() = slow);
    SECOND.with(|said| *said.borrow_mut() = second);
}

fn area(cx: &mut Cx, name: &&'static str) -> Element {
    let name: &'static str = name;
    let mut content = Element::new(Role::Group).props(style!(Style::new().flex(1.0)));
    for index in 0..10 {
        content = content.child(
            Element::new(Role::Group)
                .key(format!("{name}{index}"))
                .props(rsui::core::Props::from(Style::new().height(50.0))),
        );
    }
    let control = ScrollControl {
        moved: Some(Rc::new(move |_: &mut Updates<'_>, down: f64, _: f64| {
            let slow = SLOW.with(|said| *said.borrow());
            if !slow.is_zero() {
                std::thread::sleep(slow);
            }
            TOLD.with(|told| told.borrow_mut().push((name, down)));
        })),
        ..ScrollControl::default()
    };
    cx.scroll_area_with(name, ScrollAxis::Vertical, content, control)
}

/// A window filled by an area, or with a second one beside it, which toggling the window takes away.
fn areas(cx: &mut Cx, _: &()) -> Element {
    let second = cx.state(|| SECOND.with(|said| *said.borrow()));
    let mut row = Element::new(Role::Window).key("root").props(style!(Style::row())).child(cx.keyed("first", area, "first"));
    if cx.get(second) {
        row = row.child(cx.keyed("second", area, "second"));
    }
    cx.on(row, ActionKind::Toggle, move |cx, _| cx.set(second, false))
}

fn app(animations: bool) -> App {
    let mut app = App::new(areas, (), TextEngine::new(plain_font()), SIZE, 1.0);
    app.set_animations(animations);
    app.set_frame_time(Duration::from_secs_f64(1.0 / 144.0));
    app.update();
    app.scene();
    app
}

/// Scrolls an area straight to `to`, as anything does that does what a person would.
fn scroll(app: &mut App, path: &[u32], to: f64) {
    app.runtime_mut().dispatch(path, ActionKind::Scroll, Some(PropValue::Float(to))).unwrap();
}

fn moved(app: &App, path: &[u32]) -> f64 {
    let root = app.runtime().root().clone();
    let node = path.iter().fold(&root, |node, index| &node.children[*index as usize]);
    f64::from(app.layout().scroll_info(path, node).expect("it scrolls").y)
}

/// A frame as a window has one: what's moving moves on to `now`, what's changed is applied, and it's drawn.
fn frame(app: &mut App, now: Instant) {
    app.tick(now);
    app.update();
    app.scene();
}

#[test]
fn notches_before_a_frame_are_told_once_where_they_ended_up() {
    reset(Duration::ZERO, false);
    let mut app = app(true);
    for to in [30.0, 60.0, 95.0] {
        scroll(&mut app, AREA, to);
    }
    assert_eq!(told(), [], "nothing's told while it's being scrolled");
    app.update();
    assert_eq!(told(), [], "nor while the frame's being worked out");
    app.scene();
    let down = moved(&app, AREA);
    assert_eq!(down, 95.0);
    assert_eq!(told(), [("first", down)], "once, at where it got to");
    app.update();
    app.scene();
    assert_eq!(told().len(), 1, "and not again until it moves again");
}

#[test]
fn each_glide_frame_tells_where_it_shows_it_before_it_is_drawn() {
    reset(Duration::ZERO, false);
    let mut app = app(true);
    let start = Instant::now();
    app.scroll(50.0, 50.0, 3).unwrap();
    let mut shown = Vec::new();
    for step in 1..=8u64 {
        let before = told().len();
        app.tick(start + Duration::from_millis(step * 20));
        // Told as the frame starts, before what that changes is applied and drawn.
        let after_tick = told().len();
        app.update();
        app.scene();
        let down = moved(&app, AREA);
        assert_eq!(told().len(), after_tick, "nothing more once it's drawn");
        if before == after_tick {
            continue;
        }
        assert_eq!(after_tick, before + 1, "once a frame at most");
        assert_eq!(told().last().unwrap().1, down, "where the frame drawn after it shows it");
        shown.push(down);
    }
    assert!(shown.len() >= 4, "each frame of the glide is told: {shown:?}");
    assert!(shown.windows(2).all(|pair| pair[0] < pair[1]), "{shown:?}");
    assert!((shown.last().unwrap() - 100.0).abs() < 0.5, "{shown:?}");
}

#[test]
fn a_slow_listener_is_told_every_frame_of_the_glide_and_never_held() {
    // Longer than a 144 Hz frame, as floating-ui's popovers in Mantine's kitchen sink took: told anyway, as a browser's
    // main thread fires `scroll` every frame however long its listeners take, rather than holding them until it stops and
    // having the popovers jump.
    reset(Duration::from_millis(12), false);
    let mut app = app(true);
    let start = Instant::now();
    app.scroll(50.0, 50.0, 3).unwrap();
    for step in 1..=16u64 {
        frame(&mut app, start + Duration::from_millis(step * 7));
    }
    frame(&mut app, start + Duration::from_millis(200));
    frame(&mut app, start + Duration::from_millis(207));
    let down = moved(&app, AREA);
    assert!((down - 100.0).abs() < 0.5, "{down}");
    let heard = told();
    // Each frame it moved in, which at whole pixels is all but the last few of the 16.
    assert!(heard.len() >= 12, "told each frame it moved in: {heard:?}");
    assert!(heard.windows(2).all(|pair| pair[0].1 < pair[1].1), "once a frame, each further on: {heard:?}");
    assert_eq!(heard.last().unwrap().1, down);
    assert!(!app.animating(), "nothing's waiting any more, so nothing more needs drawing");
}

#[test]
fn areas_are_told_in_the_order_they_moved_and_one_that_has_gone_is_not() {
    reset(Duration::ZERO, true);
    let mut app = app(true);
    // The second one first, then the first, then the second again: each is told once, in the order they first moved.
    scroll(&mut app, &[1], 20.0);
    scroll(&mut app, &[0], 20.0);
    scroll(&mut app, &[1], 40.0);
    app.update();
    app.scene();
    assert_eq!(told(), [("second", 40.0), ("first", 20.0)]);
    // Scrolled, then gone before it could be told: nothing's told, and nothing goes wrong.
    TOLD.with(|told| told.borrow_mut().clear());
    let mut app_on = app_with_glide_then_gone();
    app_on.update();
    app_on.scene();
    assert!(told().iter().all(|(name, _)| *name != "second"), "{:?}", told());
}

fn app_with_glide_then_gone() -> App {
    let mut app = app(true);
    // Scrolled between frames, as a scrollbar's dragged.
    scroll(&mut app, &[1], 40.0);
    app.update();
    // Taken away before the frame that would tell it.
    app.runtime_mut().dispatch(&[], ActionKind::Toggle, None).unwrap();
    app
}

#[test]
fn a_notch_taken_in_late_glides_from_when_the_wheel_was_turned() {
    // A window that was busy gets to a notch late; it glides from when it was turned, so it stops when it would have,
    // rather than going on for as long as the window was held up.
    reset(Duration::ZERO, false);
    let mut late = app(true);
    let mut on_time = app(true);
    let now = Instant::now();
    late.set_input_time(now - Duration::from_millis(40));
    late.scroll(50.0, 50.0, 3).unwrap();
    on_time.scroll(50.0, 50.0, 3).unwrap();
    let at = now + Duration::from_millis(10);
    frame(&mut late, at);
    frame(&mut on_time, at);
    assert!(moved(&late, AREA) > moved(&on_time, AREA) + 5.0, "{} {}", moved(&late, AREA), moved(&on_time, AREA));
    // There by 120 ms after it was turned, which is 80 ms after it was taken in.
    frame(&mut late, now + Duration::from_millis(81));
    frame(&mut late, now + Duration::from_millis(82));
    assert!((moved(&late, AREA) - 100.0).abs() < 0.5, "{}", moved(&late, AREA));
    assert!(!late.animating());
    // Never from further back than half a glide, however late.
    let mut very_late = app(true);
    very_late.set_input_time(now - Duration::from_secs(5));
    very_late.scroll(50.0, 50.0, 3).unwrap();
    frame(&mut very_late, Instant::now());
    assert!(moved(&very_late, AREA) < 99.0, "{}", moved(&very_late, AREA));
}
