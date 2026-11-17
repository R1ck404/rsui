//! Changing state from outside the thread running the UI.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rsui_core::{ActionKind, Element, Patch, Role, apply, text};
use rsui_runtime::{Cx, Remote, Runtime};

const PATIENCE: Duration = Duration::from_secs(10);

/// Another thread sending as fast as it can must not look like effects that never settle, and a
/// flush has to finish even though there's always more on the way.
#[test]
fn a_flood_from_another_thread_never_stops_a_flush() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        let count = cx.state(|| 0u64);
        cx.effect((), move |cx| {
            cx.spawn(move |remote| {
                for _ in 0..200_000 {
                    if !remote.send(move |cx| cx.update(count, |count| *count += 1)) {
                        return;
                    }
                }
            });
        });
        Element::new(Role::Text).name(text!("{}", cx.get(count)))
    }
    let mut runtime = Runtime::new(app, ());
    let started = Instant::now();
    while runtime.root().name != "200000" {
        runtime.flush();
        assert!(started.elapsed() < Duration::from_secs(30), "it kept up");
    }
}

/// Timers fire in order of when they're due, not when they were made, and a late one doesn't hold
/// up an earlier one.
#[test]
fn timers_fire_in_the_order_they_are_due() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        let fired = cx.state(Vec::<u32>::new);
        cx.effect((), move |cx| {
            for (delay, tag) in [(60, 3), (10, 1), (30, 2), (10, 11), (0, 0)] {
                cx.after(Duration::from_millis(delay), move |cx| {
                    cx.update(fired, |fired| fired.push(tag));
                });
            }
        });
        let list = cx.with(fired, |fired| format!("{fired:?}"));
        Element::new(Role::Text).name(list)
    }
    let mut runtime = Runtime::new(app, ());
    let (_, finished) = runtime.settle(PATIENCE);
    assert!(finished);
    assert_eq!(runtime.root().name, "[0, 1, 11, 2, 3]");
}

/// A delay too long for any clock counts as never, rather than breaking anything, and doesn't
/// leave anything waiting for it forever.
#[test]
fn a_timer_that_can_never_fire_is_not_waited_for() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        cx.effect((), move |cx| {
            cx.after(Duration::MAX, |_| {});
        });
        Element::new(Role::Text)
    }
    let mut runtime = Runtime::new(app, ());
    let (_, finished) = runtime.settle(PATIENCE);
    assert!(finished);
}

/// After the last timer, the thread that runs them goes away, and a later timer brings it back.
#[test]
fn timers_still_work_after_the_thread_has_gone_away() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        let count = cx.state(|| 0);
        cx.effect((), move |cx| {
            cx.after(Duration::from_millis(1), move |cx| cx.set(count, 1));
        });
        Element::new(Role::Text).name(text!("{}", cx.get(count)))
    }
    let mut first = Runtime::new(app, ());
    assert!(first.settle(PATIENCE).1);
    assert_eq!(first.root().name, "1");
    std::thread::sleep(Duration::from_millis(2300));
    let mut second = Runtime::new(app, ());
    assert!(second.settle(PATIENCE).1);
    assert_eq!(second.root().name, "1");
}

fn loader(cx: &mut Cx, _: &()) -> Element {
    let status = cx.state(|| "loading");
    cx.effect((), move |cx| {
        cx.spawn(move |remote| {
            remote.send(move |cx| cx.set(status, "done"));
        });
    });
    Element::new(Role::Text).name(cx.get(status))
}

#[test]
fn a_task_can_report_back_and_settling_waits_for_it() {
    let mut runtime = Runtime::new(loader, ());
    let mirror = runtime.root().clone();
    assert_eq!(mirror.name, "loading");

    let (patches, settled) = runtime.settle(PATIENCE);

    assert!(settled);
    assert_eq!(runtime.root().name, "done");
    assert!(!runtime.has_background_work());
    assert_eq!(apply(mirror, &patches).unwrap(), *runtime.root());
}

/// A task that blocks until the test lets it continue.
fn gated(cx: &mut Cx, gate: &Gate) -> Element {
    let status = cx.state(|| "waiting");
    let gate = gate.0.clone();
    cx.effect((), move |cx| {
        cx.spawn(move |remote| {
            let (receiver, stopped) = &*gate;
            let _ = receiver.lock().unwrap().recv();
            let delivered = remote.send(move |cx| cx.set(status, "done"));
            stopped.store(usize::from(delivered) + 1, Ordering::SeqCst);
        });
    });
    Element::new(Role::Text).name(cx.get(status))
}

#[derive(Clone)]
#[allow(clippy::type_complexity)]
struct Gate(Arc<(Mutex<mpsc::Receiver<()>>, AtomicUsize)>);

impl PartialEq for Gate {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

fn gate() -> (mpsc::Sender<()>, Gate) {
    let (sender, receiver) = mpsc::channel();
    (
        sender,
        Gate(Arc::new((Mutex::new(receiver), AtomicUsize::new(0)))),
    )
}

#[test]
fn settling_gives_up_at_the_timeout_and_can_be_resumed() {
    let (open, gate) = gate();
    let mut runtime = Runtime::new(gated, gate);

    let (_, settled) = runtime.settle(Duration::from_millis(20));
    assert!(!settled);
    assert!(runtime.has_background_work());
    assert_eq!(runtime.root().name, "waiting");

    open.send(()).unwrap();
    let (_, settled) = runtime.settle(PATIENCE);
    assert!(settled);
    assert_eq!(runtime.root().name, "done");
}

#[test]
fn waiting_returns_when_a_task_reports_or_times_out() {
    let (open, gate) = gate();
    let runtime = Runtime::new(gated, gate);

    let start = Instant::now();
    runtime.wait(Duration::from_millis(20));
    assert!(start.elapsed() >= Duration::from_millis(20));

    open.send(()).unwrap();
    runtime.wait(PATIENCE);
    assert!(start.elapsed() < PATIENCE, "the wait ended early");
}

#[test]
fn waiting_with_no_background_work_returns_at_once() {
    fn idle(_: &mut Cx, _: &()) -> Element {
        Element::new(Role::Text)
    }
    let runtime = Runtime::new(idle, ());
    let start = Instant::now();
    runtime.wait(PATIENCE);
    assert!(start.elapsed() < PATIENCE);
    assert!(!runtime.has_background_work());
}

#[test]
fn the_waker_runs_when_a_task_reports() {
    let (open, gate) = gate();
    let mut runtime = Runtime::new(gated, gate);
    let (woken, wakes) = mpsc::channel();
    runtime.set_waker(move || {
        let _ = woken.send(());
    });

    open.send(()).unwrap();
    wakes.recv_timeout(PATIENCE).expect("the waker ran");
    runtime.settle(PATIENCE);
    assert_eq!(runtime.root().name, "done");
}

fn sender(cx: &mut Cx, slot: &Slot) -> Element {
    let value = cx.state(|| 0i64);
    let slot = slot.0.clone();
    cx.effect((), move |cx| {
        *slot.lock().unwrap() = Some((cx.remote(), value));
    });
    Element::new(Role::Text).name(text!("{}", cx.get(value)))
}

type Sent = Option<(Remote, rsui_runtime::State<i64>)>;

#[derive(Clone, Default)]
struct Slot(Arc<Mutex<Sent>>);

impl PartialEq for Slot {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

#[test]
fn changes_from_another_thread_wait_for_the_flush_and_share_one_render() {
    let slot = Slot::default();
    let mut runtime = Runtime::new(sender, slot.clone());
    let (remote, value) = slot.0.lock().unwrap().take().unwrap();
    let before = runtime.renders();

    let thread = std::thread::spawn(move || {
        for number in 1..=100 {
            assert!(remote.send(move |cx| cx.set(value, number)));
        }
    });
    thread.join().unwrap();

    assert!(runtime.has_background_work());
    assert_eq!(runtime.root().name, "0");
    let patches = runtime.flush();
    assert_eq!(runtime.root().name, "100");
    assert_eq!(runtime.renders() - before, 1);
    assert!(matches!(patches.as_slice(), [Patch::SetName { .. }]));
    assert!(!runtime.has_background_work());
}

#[test]
fn sending_after_the_runtime_is_gone_says_so() {
    let slot = Slot::default();
    let runtime = Runtime::new(sender, slot.clone());
    let (remote, value) = slot.0.lock().unwrap().take().unwrap();

    assert!(remote.send(|_| {}));
    drop(runtime);
    assert!(!remote.send(move |cx| cx.set(value, 1)));
}

fn switchable(cx: &mut Cx, _: &()) -> Element {
    let shown = cx.state(|| true);
    let mut element = Element::new(Role::Group).key("switch");
    if cx.get(shown) {
        element = element.child(cx.child(delayed, ()));
    }
    cx.on(element, ActionKind::Toggle, move |cx, _| {
        cx.set(shown, false);
    })
}

fn delayed(cx: &mut Cx, _: &()) -> Element {
    let status = cx.state(|| "soon");
    cx.effect((), move |cx| {
        cx.after(Duration::from_millis(10), move |cx| cx.set(status, "later"));
    });
    Element::new(Role::Text).name(cx.get(status))
}

#[test]
fn a_timer_fires_after_its_delay() {
    let mut runtime = Runtime::new(delayed, ());
    assert_eq!(runtime.root().name, "soon");
    let start = Instant::now();
    let (_, settled) = runtime.settle(PATIENCE);
    assert!(settled);
    assert!(start.elapsed() >= Duration::from_millis(5));
    assert_eq!(runtime.root().name, "later");
}

#[test]
fn a_timer_for_an_unmounted_component_does_nothing() {
    let mut runtime = Runtime::new(switchable, ());
    runtime.dispatch(&[], ActionKind::Toggle, None).unwrap();
    runtime.flush();
    assert!(runtime.root().children.is_empty());

    let (patches, settled) = runtime.settle(PATIENCE);
    assert!(settled);
    assert!(patches.is_empty());
}

#[test]
fn a_panicking_task_still_counts_as_finished() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        cx.effect((), |cx| cx.spawn(|_| panic!("expected in this test")));
        Element::new(Role::Text)
    }
    let mut runtime = Runtime::new(app, ());
    let (_, settled) = runtime.settle(PATIENCE);
    assert!(settled);
}

#[test]
fn rejecting_from_a_job_does_not_fail_a_later_action() {
    fn app(cx: &mut Cx, _: &()) -> Element {
        cx.effect((), |cx| {
            cx.remote().send(|cx| cx.reject("nobody is listening"));
        });
        cx.on(
            Element::new(Role::Button).key("ok"),
            ActionKind::Click,
            |_, _| {},
        )
    }
    let mut runtime = Runtime::new(app, ());
    runtime.flush();
    assert_eq!(runtime.dispatch(&[], ActionKind::Click, None), Ok(()));
}
