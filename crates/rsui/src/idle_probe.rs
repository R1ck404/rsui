//! Finite native callback probe. Compiled out without idle-probe; no production timer.
use super::*;
#[allow(dead_code)]
#[path = "idle_probe_fixture.rs"]
mod fixture;
use rsui_core::ActionKind;
pub(super) const TIMER: usize = 1010;
#[derive(Default)]
struct Probe {
    start: Option<Instant>,
    action: Option<Instant>,
    stage: usize,
    idle: usize,
    release: usize,
    draws: usize,
    previous_idle: usize,
    previous_draws: usize,
    fixture_ticks: usize,
    drawn_stages: u32,
}
thread_local! { static PROBE: RefCell<Probe> = RefCell::new(Probe::default()); }
pub(super) fn active() -> bool {
    std::env::var_os("RSUI_010F_NATIVE").is_some() || automatic()
}
pub(super) fn opened(window: HWND) {
    if !active() {
        return;
    }
    PROBE.with(|p| p.borrow_mut().start = Some(Instant::now()));
    // SAFETY: owned window, fixture timer only, destroyed with this window.
    unsafe {
        let mut cursor = POINT::default();
        GetCursorPos(&mut cursor).expect("cursor");
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        assert!(GetMonitorInfoW(monitor, &mut info).as_bool());
        let mut rect = RECT::default();
        GetWindowRect(window, &mut rect).expect("window rectangle");
        let work = info.rcWork;
        let x = if cursor.x < (work.left + work.right) / 2 {
            work.right - (rect.right - rect.left)
        } else {
            work.left
        };
        SetWindowPos(
            window,
            None,
            x,
            work.top,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .expect("fixture placement");
        assert_ne!(SetTimer(Some(window), TIMER, 50, None), 0);
    }
}
pub(super) fn boundary(
    state: &WindowState,
    kind: &str,
    start: Instant,
    released: usize,
    reclaim_ns: u128,
) {
    let ns = start.elapsed().as_nanos();
    PROBE.with(|p| {
        let mut p = p.borrow_mut();
        if p.start.is_none() {
            return;
        }
        assert!(released <= 2 * 1024 * 1024);
        if kind == "idle" {
            p.idle += 1;
        } else {
            p.release += 1;
        }
        println!(
            "boundary={kind} stage={} ns={ns} released={released} reclaim_ns={reclaim_ns} idle={} release={}",
            p.stage, p.idle, p.release
        );
        if let Some(gpu) = &state.renderer {
            fixture::sample(&state.app, gpu, kind, p.stage, ns);
        }
    });
}
pub(super) fn rendered(state: &WindowState, start: Instant, ok: bool) {
    let ns = start.elapsed().as_nanos();
    PROBE.with(|p| {
        let mut p = p.borrow_mut();
        if p.start.is_none() {
            return;
        }
        assert!(ok);
        p.draws += 1;
        p.drawn_stages |= 1 << p.stage;
        if automatic() {
            if p.draws == 1 { println!("adapter={}",state.renderer.as_ref().expect("renderer").adapter_for_diagnostics().expect("adapter")); }
            let (draw, present) = state.app.probe_render_times();
            let work = state.renderer.as_ref().expect("renderer").draw_work();
            println!("auto_frame stage={} ui_ns={} draw_ns={} present_ns={} path={:?} presentation={:?} uploads={} submitted={} calls={} animating={}", p.stage, ns.saturating_sub(present.as_nanos()), draw.as_nanos(), present.as_nanos(), state.app.scene_work().path, work.presentation, work.quad_upload.bytes, work.submitted_quads, work.draw_calls, state.app.animating());
        }
        let latency = p.action.map(|t| t.elapsed().as_nanos()).unwrap_or(0);
        println!(
            "render stage={} ns={ns} action_to_return_ns={latency}",
            p.stage
        );
        if let Some(gpu) = &state.renderer {
            fixture::sample(&state.app, gpu, "render", p.stage, ns);
        }
    });
}
pub(super) fn tick(window: HWND) {
    PROBE.with(|p| p.borrow_mut().fixture_ticks += 1);
    if automatic() {
        automatic_tick(window);
        return;
    }
    let action = PROBE.with(|p| {
        let mut p = p.borrow_mut();
        let elapsed = p.start.expect("opened").elapsed();
        assert!(elapsed < Duration::from_secs(45), "native probe timeout");
        // SAFETY: this is our live window; observational only.
        unsafe {
            assert_ne!(GetForegroundWindow(), window, "probe activated");
        }
        let ready = match p.stage {
            0 => p.draws > 0 && elapsed > Duration::from_millis(400),
            1 => {
                p.draws > p.previous_draws
                    && p.action.unwrap().elapsed() > Duration::from_millis(500)
            }
            2 => p.idle > p.previous_idle,
            3 => p.draws > p.previous_draws,
            4 => {
                p.draws > p.previous_draws
                    && p.action.unwrap().elapsed() > Duration::from_millis(500)
            }
            5 => p.release > 0,
            _ => false,
        };
        if !ready {
            return None;
        }
        p.previous_idle = p.idle;
        p.previous_draws = p.draws;
        p.stage += 1;
        p.action = Some(Instant::now());
        println!(
            "action={} elapsed_ns={} idle={} release={}",
            p.stage,
            elapsed.as_nanos(),
            p.idle,
            p.release
        );
        // SAFETY: observational query on the fixture window.
        unsafe {
            assert!(IsWindowVisible(window).as_bool());
            assert!(!IsIconic(window).as_bool());
        }
        println!("covered={}", u8::from(nothing_of_it_shows(window)));
        Some(p.stage)
    });
    let Some(action) = action else {
        return;
    };
    if action == 6 {
        // SAFETY: close only this fixture's window through its normal cleanup.
        unsafe {
            PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0)).expect("close");
        }
        return;
    }
    with_state(window, |state| {
        state.app.set_animations(false);
        if action != 3 {
            state
                .app
                .runtime_mut()
                .dispatch(&[], ActionKind::Toggle, None)
                .expect("toggle");
        } else {
            state
                .app
                .runtime_mut()
                .dispatch(&[], ActionKind::SetValue, None)
                .expect("palette wake");
        }
        state.update_and_draw(window);
        assert_eq!(
            state.app.runtime().root().props.get("probe_large"),
            Some(&rsui_core::PropValue::Bool(action == 1 || action == 4))
        );
        assert_eq!(
            state.app.runtime().root().props.get("background"),
            Some(&rsui_core::PropValue::Int(if action >= 3 {
                0xeeeeffff
            } else {
                0xffffffff
            }))
        );
    });
}

fn automatic() -> bool {
    std::env::var_os("RSUI_011D").is_some()
}
fn automatic_tick(window: HWND) {
    let stage = PROBE.with(|p| {
        let mut p = p.borrow_mut();
        assert!(
            p.start.expect("opened").elapsed() < Duration::from_secs(60),
            "automatic timeout"
        );
        if p.draws == 0
            || p.action
                .is_some_and(|t| t.elapsed() < Duration::from_millis(120))
        {
            return None;
        }
        if p.stage == 24 && p.release == 0 {
            return None;
        }
        if p.stage == 23 {
            // Covered-window pacing can exceed the fixture's 120 ms interval.
            // Wait for an actual scroll update, not merely the initiating draw.
            let mut moved = false;
            with_state(window, |state| {
                moved = matches!(state.app.runtime().root().children[3].props.get("scroll_y"),
                    Some(rsui_core::PropValue::Float(v)) if *v > 0.0);
            });
            if !moved {
                return None;
            }
        }
        if (p.stage == 25 || p.stage == 26) && p.drawn_stages & (1 << p.stage) == 0 {
            return None;
        }
        if p.stage == 26
            && p.action
                .is_some_and(|t| t.elapsed() < Duration::from_millis(300))
        {
            return None;
        }
        p.stage += 1;
        p.action = Some(Instant::now());
        Some(p.stage)
    });
    let Some(stage) = stage else {
        return;
    };
    if stage == 27 {
        PROBE.with(|p| {
            let p = p.borrow();
            assert!(p.release > 0 && p.idle > 0);
            println!(
                "auto_complete draws={} idle={} release={} fixture_ticks={}",
                p.draws, p.idle, p.release, p.fixture_ticks
            );
        });
        // SAFETY: closes only the finite fixture window through production cleanup.
        unsafe {
            PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0)).expect("close");
        }
        return;
    }
    if stage == 22 {
        // SAFETY: finite fixture resizes its own window through WM_SIZE.
        unsafe {
            SetWindowPos(
                window,
                None,
                0,
                0,
                700,
                650,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .expect("resize");
        }
        return;
    }
    with_state(window, |state| {
        let scale = state.app.scale();
        let before = state.app.runtime().root().clone();
        let start = Instant::now();
        if stage == 24 {
            assert!(
                matches!(state.app.runtime().root().children[3].props.get("scroll_y"),Some(rsui_core::PropValue::Float(v)) if *v > 0.0),
                "scroll must change its offset"
            );
            state.app.pointer_leave();
            state.app.cancel_press();
            state.app.set_active(false);
        } else if stage == 23 {
            state
                .app
                .scroll(10.0 * scale, 100.0 * scale, 1)
                .expect("scroll");
        } else if stage == 19 {
            state.app.set_active(true);
            state.app.pointer_down(10.0 * scale, 75.0 * scale, 1);
            state
                .app
                .pointer_up(10.0 * scale, 75.0 * scale)
                .expect("focus input");
            state.app.type_text("abc").expect("type");
        } else if stage == 20 {
            state.app.set_active(true);
            state
                .app
                .type_text("\u{e9}\u{65e5}")
                .expect("Unicode typing");
        } else if stage == 21 {
            state.app.set_animations(true);
            state.app.focus_on(&[]);
            state.app.pointer_move(10.0 * scale, 10.0 * scale);
        } else if stage == 26 {
            // Exercise the same device-loss handler as a failed production draw.
            state.handle_render_error(window, RenderError::DeviceLost);
        } else {
            let y = if (9..=16).contains(&stage) && stage % 2 == 0 {
                45.0
            } else {
                10.0
            };
            state
                .app
                .click(10.0 * scale, y * scale)
                .expect("ordinary action");
        }
        let action_ns = start.elapsed().as_nanos();
        state.update_and_draw(window);
        let total = start.elapsed();
        if stage == 19 || stage == 20 {
            assert!(
                state.app.caret_blinking(),
                "typing activation must be matched"
            );
            let expected = if stage == 19 {
                "abc"
            } else {
                "abc\u{e9}\u{65e5}"
            };
            assert_eq!(
                state.app.runtime().root().children[2].props.get("value"),
                Some(&rsui_core::PropValue::from(expected)),
                "typing delivery"
            );
        }
        if stage <= 18 || stage == 25 {
            assert_ne!(
                &before,
                state.app.runtime().root(),
                "action must change tree at {stage}"
            );
        }
        println!(
            "auto_action stage={stage} scale={scale} action_ns={action_ns} total_ns={} present_ns={} renderer={} picture={}",
            total.as_nanos(),
            state.app.probe_render_times().1.as_nanos(),
            state.renderer.is_some(),
            state.showing_picture
        );
    });
}

pub(super) fn updated(elapsed: Duration) {
    if automatic() {
        PROBE.with(|p| {
            println!(
                "auto_update stage={} ns={}",
                p.borrow().stage,
                elapsed.as_nanos()
            )
        });
    }
}
