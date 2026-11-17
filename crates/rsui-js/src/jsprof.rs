//! Diagnostic boundary sampling, only in builds with the `jsprof` feature. A
//! millisecond ticker asks the next VM call boundary or JIT slow-operation boundary
//! to record the currently active script stack, before entering another function.
//! Pending ticks outside any script frame are discarded. This avoids crediting
//! idle/native work to whichever cheap getter happens to run next.
//!
//! These are boundary observations, NOT unbiased CPU samples or CPU percentages:
//! native work between boundaries can still be charged to the surrounding script,
//! and call-free compiled/interpreted work is sampled less often. Multiple script
//! threads share the ticker. Results go to `RSUI_JSPROF`, most frequent first.
//! The final partial checkpoint is flushed when a VM is dropped.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};

static TICK: AtomicBool = AtomicBool::new(false);
static START: Once = Once::new();
static COUNTS: Mutex<Option<(HashMap<String, u32>, u32)>> = Mutex::new(None);

#[cfg(test)]
thread_local! {
    // Deterministic, thread-local injection avoids timing sleeps and interference
    // from other engines running in the same parallel test process.
    static TEST_TICK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TEST_SAMPLES: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
    static TEST_FLUSHES: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn discard_idle_tick() {
    #[cfg(test)]
    if TEST_SAMPLES.with(|samples| samples.borrow().is_some()) {
        TEST_TICK.with(|tick| tick.set(false));
        return;
    }
    TICK.store(false, Ordering::Relaxed);
}

fn take_tick() -> bool {
    #[cfg(test)]
    if TEST_SAMPLES.with(|samples| samples.borrow().is_some()) {
        return TEST_TICK.with(|tick| tick.replace(false));
    }
    // Most boundaries have no sample waiting; avoid an atomic read-modify-write
    // on every generic DOM operation when the profiler is enabled.
    TICK.load(Ordering::Relaxed) && TICK.swap(false, Ordering::Relaxed)
}

pub(crate) fn maybe_sample(vm: &mut crate::vm::Vm) {
    START.call_once(|| {
        if std::env::var_os("RSUI_JSPROF").is_some() {
            std::thread::spawn(|| {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    TICK.store(true, Ordering::Relaxed);
                }
            });
        }
    });
    if vm.frames.is_empty() {
        discard_idle_tick();
        return;
    }
    if !take_tick() {
        return;
    }
    let trace = vm.stack_trace();
    let top: Vec<&str> = trace.lines().take(3).map(str::trim).collect();
    let key = top.join("  <-  ");
    #[cfg(test)]
    if TEST_SAMPLES.with(|samples| {
        if let Some(samples) = samples.borrow_mut().as_mut() {
            samples.push(key.clone());
            true
        } else {
            false
        }
    }) {
        return;
    }
    let Ok(mut kept) = COUNTS.lock() else {
        return;
    };
    let (counts, total) = kept.get_or_insert_with(|| (HashMap::new(), 0));
    *counts.entry(key).or_insert(0) += 1;
    *total += 1;
    if *total % 500 == 0 {
        if let Some(path) = std::env::var_os("RSUI_JSPROF") {
            let _ = std::fs::write(path, format_counts(counts, *total));
        }
    }
}

fn format_counts(counts: &HashMap<String, u32>, total: u32) -> String {
    let mut rows: Vec<(&String, &u32)> = counts.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let mut out = format!("samples {total}\n");
    for (place, count) in rows.iter().take(400) {
        out.push_str(&format!("{count:6} {place}\n"));
    }
    out
}

/// Preserve the last partial checkpoint; a short action can have fewer than 500
/// samples. This is called only from the jsprof-enabled VM destructor.
pub(crate) fn flush() {
    #[cfg(test)]
    if let Some(rows) = TEST_SAMPLES.with(|samples| samples.borrow().clone()) {
        let mut counts = HashMap::new();
        for row in &rows {
            *counts.entry(row.clone()).or_insert(0) += 1;
        }
        TEST_FLUSHES.with(|outputs| {
            if let Some(outputs) = outputs.borrow_mut().as_mut() {
                outputs.push(format_counts(&counts, rows.len() as u32));
            }
        });
        return;
    }
    let Some(path) = std::env::var_os("RSUI_JSPROF") else {
        return;
    };
    let out = {
        let Ok(kept) = COUNTS.lock() else {
            return;
        };
        match kept.as_ref() {
            Some((counts, total)) => format_counts(counts, *total),
            None => "samples 0\n".to_owned(),
        }
    };
    let _ = std::fs::write(path, out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Engine, Value, Vm};

    struct Capture;

    impl Capture {
        fn new() -> Self {
            TEST_TICK.with(|tick| tick.set(false));
            TEST_SAMPLES.with(|samples| {
                assert!(samples.borrow().is_none());
                *samples.borrow_mut() = Some(Vec::new());
            });
            TEST_FLUSHES.with(|outputs| *outputs.borrow_mut() = Some(Vec::new()));
            Self
        }

        fn rows(&self) -> Vec<String> {
            TEST_SAMPLES.with(|samples| samples.borrow().as_ref().unwrap().clone())
        }

        fn flushes(&self) -> Vec<String> {
            TEST_FLUSHES.with(|outputs| outputs.borrow().as_ref().unwrap().clone())
        }
    }

    impl Drop for Capture {
        fn drop(&mut self) {
            TEST_SAMPLES.with(|samples| *samples.borrow_mut() = None);
            TEST_FLUSHES.with(|outputs| *outputs.borrow_mut() = None);
            TEST_TICK.with(|tick| tick.set(false));
        }
    }

    fn arm_tick(_: &mut Vm, _: crate::vm::Args) -> Result<Value, Value> {
        TEST_TICK.with(|tick| tick.set(true));
        Ok(Value::UNDEFINED)
    }

    fn engine() -> Engine {
        let mut engine = Engine::new();
        let callback = engine.vm.host_function("armSample", 0, arm_tick);
        engine.vm.set_global("armSample", callback);
        engine
    }

    #[test]
    fn a_native_armed_tick_before_a_getter_is_attributed_to_the_active_caller() {
        let capture = Capture::new();
        let mut engine = engine();
        engine.eval("const o={get cheapGetter(){return 7}};function measuredCaller(){armSample();return o.cheapGetter}measuredCaller();", "sample-caller.js").unwrap();
        let rows = capture.rows();
        assert_eq!(rows.len(), 1, "one injected tick must produce one sample: {rows:?}");
        assert!(rows[0].starts_with("at measuredCaller ("), "the caller must remain on top: {rows:?}");
        assert!(!rows[0].contains("cheapGetter"), "an unentered getter must not be credited");
    }

    #[test]
    fn a_pending_idle_tick_is_not_credited_to_the_next_script_or_getter() {
        let capture = Capture::new();
        let mut engine = engine();
        TEST_TICK.with(|tick| tick.set(true));
        engine.eval("const o={get cheapGetter(){return 7}};function started(){return o.cheapGetter}started();", "after-idle.js").unwrap();
        assert!(capture.rows().is_empty(), "idle time must not become script work");
    }

    #[test]
    fn dropping_a_vm_flushes_all_samples_below_the_periodic_checkpoint() {
        let capture = Capture::new();
        {
            let mut engine = engine();
            engine.eval("const o={get cheapGetter(){return 7}};function shortAction(){armSample();return o.cheapGetter}shortAction();shortAction();shortAction();", "short-action.js").unwrap();
            assert_eq!(capture.rows().len(), 3);
            assert!(capture.flushes().is_empty(), "three samples must not reach the 500-sample checkpoint");
        }
        let outputs = capture.flushes();
        assert_eq!(outputs.len(), 1, "the actual VM destructor must flush once");
        assert_eq!(outputs[0].lines().next(), Some("samples 3"));
        let recorded: u32 = outputs[0].lines().skip(1).map(|line| line.split_whitespace().next().unwrap().parse::<u32>().unwrap()).sum();
        assert_eq!(recorded, 3, "the final partial checkpoint must not lose samples");
    }

    #[cfg(all(feature = "jit", windows, target_arch = "x86_64"))]
    #[test]
    fn a_compiled_slow_operation_samples_without_another_function_call() {
        let capture = Capture::new();
        let mut engine = engine();
        engine.vm.jit.mode = crate::jit::Mode::On;
        engine.vm.jit.background = false;
        engine.vm.jit.threshold = 2;
        engine.eval("let arm=false;const o={selected:7};function measuredCompiled(){if(arm)armSample();return 'selected' in o}for(let i=0;i<32;i++)measuredCompiled();globalThis.enableSample=()=>arm=true;", "warm-sampler.js").unwrap();
        let name = engine.vm.intern_str("measuredCompiled");
        let proto = engine.vm.protos.iter().position(|proto| proto.name == name).unwrap() as u32;
        assert!(engine.vm.jit.backend.as_ref().is_some_and(|backend| backend.compiled.contains_key(&proto)), "fixture must actually execute compiled code");
        engine.eval("enableSample();measuredCompiled();", "sample-compiled.js").unwrap();
        let rows = capture.rows();
        assert_eq!(rows.len(), 1, "one injected tick must produce one sample: {rows:?}");
        assert!(rows[0].starts_with("at measuredCompiled ("), "the synchronized compiled caller must remain on top: {rows:?}");
    }
}
