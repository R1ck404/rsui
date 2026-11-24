//! Sampling where JavaScript is, for the inspector, without the engine doing anything while it's not wanted.
//!
//! Another thread stops the engine's thread about every millisecond, copies the top of its list of running functions out
//! of memory (each one's function and where in it it is), and lets it go again. Nothing in the engine checks for this, so
//! it costs nothing at all until it's started, and little while it is: the copy is a few kilobytes, and while the
//! thread's stopped nothing's allocated or locked, so it can't be holding up anything the copy needs.
//!
//! The copy is read with `ReadProcessMemory`, which says it couldn't rather than failing if the memory isn't there, so a
//! sample taken at a bad moment is wrong rather than harmful, and the numbers in it are checked before they're used.
//! Turning them into names is done on the engine's own thread, in [`Vm::take_samples`], which keeps each sample as a span
//! of kind [`rsui_spans::Kind::Sample`] with its stack as the label, outermost first.
//!
//! Only Windows has the stopping and copying written yet; elsewhere sampling says it can't start.
#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::vm::{Frame, Vm};

/// The most functions a sample keeps, the innermost.
const MOST_FRAMES: usize = 48;

/// A sample as it's copied: when, and each running function's number and where in it it was, innermost last.
struct Raw {
    at: u64,
    frames: Vec<(u32, u32)>,
    /// Whether there were more functions running than were kept.
    cut: bool,
}

/// Where the engine's list of running functions is, which the sampling thread reads.
#[derive(Clone, Copy)]
struct Place {
    base: usize,
    len: usize,
}

struct Sampling {
    stop: Arc<AtomicBool>,
    place: Arc<Mutex<Place>>,
    taken: Arc<Mutex<Vec<Raw>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

thread_local! {
    static SAMPLING: std::cell::RefCell<Option<Sampling>> = const { std::cell::RefCell::new(None) };
    /// What each function and line is called, as a label, once it's been worked out.
    static NAMES: std::cell::RefCell<HashMap<(u32, u32), u32>> = std::cell::RefCell::new(HashMap::new());
}

impl Vm {
    fn place(&self) -> Place {
        Place {
            base: self.frames.base_ptr() as usize,
            len: self.frames.len_ptr() as usize,
        }
    }

    /// Starts sampling where JavaScript is on this thread, every `every`. It's only there on Windows.
    pub(crate) fn start_sampling(&mut self, every: Duration) {
        let place = self.place();
        SAMPLING.with(|sampling| {
            let mut sampling = sampling.borrow_mut();
            if let Some(running) = sampling.as_ref() {
                *running.place.lock().expect("lock is not poisoned") = place;
                return;
            }
            *sampling = start(place, every);
        });
    }

    /// Stops sampling, keeping what was sampled.
    pub(crate) fn stop_sampling(&mut self) {
        let running = SAMPLING.with(|sampling| sampling.borrow_mut().take());
        if let Some(mut running) = running {
            running.stop.store(true, Ordering::Relaxed);
            if let Some(thread) = running.thread.take() {
                let _ = thread.join();
            }
            let mut raw = std::mem::take(&mut *running.taken.lock().expect("lock is not poisoned"));
            self.take_samples_from(&mut raw);
        }
    }

    pub(crate) fn sampling(&self) -> bool {
        SAMPLING.with(|sampling| sampling.borrow().is_some())
    }

    /// Keeps what's been sampled since this was last asked as spans, with names for where it was.
    pub(crate) fn take_samples(&mut self) {
        let place = self.place();
        let mut raw = SAMPLING.with(|sampling| {
            let sampling = sampling.borrow();
            let Some(running) = sampling.as_ref() else {
                return Vec::new();
            };
            // The engine could have moved since sampling started, which this puts right.
            *running.place.lock().expect("lock is not poisoned") = place;
            std::mem::take(&mut *running.taken.lock().expect("lock is not poisoned"))
        });
        self.take_samples_from(&mut raw);
    }

    fn take_samples_from(&mut self, raw: &mut Vec<Raw>) {
        let protos = self.protos.len() as u32;
        let mut stack = Vec::new();
        for sample in raw.drain(..) {
            stack.clear();
            if sample.cut {
                stack.push(rsui_spans::intern("…"));
            }
            for &(proto, pc) in &sample.frames {
                if proto >= protos {
                    continue;
                }
                let label = NAMES.with(|names| {
                    *names.borrow_mut().entry((proto, pc)).or_insert_with(|| {
                        let origin = self.proto_origin(proto, Some(pc));
                        let function = if origin.function.is_empty() {
                            "(anonymous)"
                        } else {
                            &origin.function
                        };
                        rsui_spans::intern(&format!("{function}\u{1}{}:{}", origin.file, origin.line))
                    })
                });
                stack.push(label);
            }
            if stack.is_empty() {
                continue;
            }
            let id = rsui_spans::intern_stack(&stack);
            rsui_spans::record(rsui_spans::Kind::Sample, sample.at, sample.at, id, 0);
        }
    }
}

#[cfg(not(windows))]
fn start(_place: Place, _every: Duration) -> Option<Sampling> {
    None
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn start(place: Place, every: Duration) -> Option<Sampling> {
    use std::ffi::c_void;

    // A handle is pointer-sized, so `isize` here and `*mut c_void` in rsui-alloc's declaration of `GetCurrentProcess` are the same
    // to the calling convention.
    #[allow(clashing_extern_declarations)]
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThreadId() -> u32;
        fn GetCurrentProcess() -> isize;
        fn OpenThread(access: u32, inherit: i32, id: u32) -> isize;
        fn CloseHandle(handle: isize) -> i32;
        fn SuspendThread(thread: isize) -> u32;
        fn ResumeThread(thread: isize) -> u32;
        fn ReadProcessMemory(
            process: isize,
            address: *const c_void,
            buffer: *mut c_void,
            size: usize,
            read: *mut usize,
        ) -> i32;
    }
    const THREAD_SUSPEND_RESUME: u32 = 0x0002;
    const THREAD_QUERY_INFORMATION: u32 = 0x0040;

    // SAFETY: a plain call on this thread's own id; the handle's closed when sampling stops.
    let target = unsafe {
        OpenThread(
            THREAD_SUSPEND_RESUME | THREAD_QUERY_INFORMATION,
            0,
            GetCurrentThreadId(),
        )
    };
    if target == 0 {
        return None;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let place = Arc::new(Mutex::new(place));
    let taken = Arc::new(Mutex::new(Vec::new()));
    let (stopping, at, into) = (stop.clone(), place.clone(), taken.clone());
    let frame_size = size_of::<Frame>();
    let (proto_at, pc_at) = (std::mem::offset_of!(Frame, proto), std::mem::offset_of!(Frame, pc));
    let thread = std::thread::Builder::new()
        .name("rsui-js sampler".to_owned())
        .stack_size(64 * 1024)
        .spawn(move || {
            // What's copied while the thread's stopped goes here, which is made before, as nothing can be allocated
            // then: the thread could be holding the allocator.
            let mut copy = vec![0u8; MOST_FRAMES * frame_size];
            // SAFETY: the pseudo handle for this process needs nothing closed.
            let process = unsafe { GetCurrentProcess() };
            while !stopping.load(Ordering::Relaxed) {
                std::thread::sleep(every);
                let Place { base, len } = *at.lock().expect("lock is not poisoned");
                let mut count = 0usize;
                let mut copied = 0usize;
                let now;
                // SAFETY: the thread's stopped while its memory's copied, and `ReadProcessMemory` copies only what
                // can be read, into memory that's this thread's own.
                unsafe {
                    if SuspendThread(target) == u32::MAX {
                        break;
                    }
                    now = rsui_spans::now();
                    let mut read = 0;
                    let got_len = ReadProcessMemory(
                        process,
                        len as *const c_void,
                        (&raw mut count).cast(),
                        size_of::<usize>(),
                        &mut read,
                    ) != 0;
                    if got_len && count > 0 && count <= crate::vm::MAX_FRAMES {
                        let kept = count.min(MOST_FRAMES);
                        let from = base + (count - kept) * frame_size;
                        if ReadProcessMemory(
                            process,
                            from as *const c_void,
                            copy.as_mut_ptr().cast(),
                            kept * frame_size,
                            &mut read,
                        ) != 0
                        {
                            copied = kept;
                        }
                    }
                    ResumeThread(target);
                }
                if copied == 0 {
                    continue;
                }
                let read_u32 = |at: usize| u32::from_le_bytes(copy[at..at + 4].try_into().unwrap_or_default());
                let frames = (0..copied)
                    .map(|index| {
                        let at = index * frame_size;
                        (read_u32(at + proto_at), read_u32(at + pc_at))
                    })
                    .collect();
                let mut taken = into.lock().expect("lock is not poisoned");
                // A viewer that's stopped asking doesn't have them pile up: a minute's worth is plenty.
                if taken.len() < 60_000 {
                    taken.push(Raw {
                        at: now,
                        frames,
                        cut: count > copied,
                    });
                }
            }
            // SAFETY: it was opened above, and nothing else has it.
            unsafe {
                CloseHandle(target);
            }
        })
        .ok()?;
    Some(Sampling {
        stop,
        place,
        taken,
        thread: Some(thread),
    })
}
