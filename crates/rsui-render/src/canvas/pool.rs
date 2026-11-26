//! A few threads kept for the canvas's big drawing, so a fill or a picture over most of a canvas can be split into bands of
//! rows, each worked out on a thread of its own, as no row reads another. They're made the first time something's big enough
//! to want them, and wait for work after; starting threads for each fill would cost more than many fills take.
//!
//! [`each`] runs a job for every band, one on the calling thread and the rest on the kept ones, and comes back when all are
//! done, so a job can borrow what the caller has, as `std::thread::scope` lets it.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

/// A job for one band, as the kept threads are given it.
type Job = Box<dyn FnOnce() + Send + 'static>;

struct Pool {
    senders: Vec<Mutex<Sender<Job>>>,
}

static POOL: OnceLock<Pool> = OnceLock::new();

#[cfg(test)]
thread_local! {
    /// For tests to draw the same thing without bands, to check they make no difference.
    pub static SERIAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// How many bands big drawing is split into: at most 4, and fewer on a smaller processor, leaving it room for the rest. 1 is
/// none: the caller does it all.
pub fn bands() -> usize {
    #[cfg(test)]
    if SERIAL.with(|serial| serial.get()) {
        return 1;
    }
    static BANDS: OnceLock<usize> = OnceLock::new();
    *BANDS.get_or_init(|| std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).clamp(1, 4)))
}

fn pool() -> &'static Pool {
    POOL.get_or_init(|| {
        let senders = (1..bands())
            .map(|k| {
                let (send, receive) = channel::<Job>();
                std::thread::Builder::new()
                    .name(format!("rsui-canvas-{k}"))
                    .spawn(move || {
                        for job in receive {
                            job();
                        }
                    })
                    .expect("a thread for the canvas");
                Mutex::new(send)
            })
            .collect();
        Pool { senders }
    })
}

/// Runs `job(band)` for each band from 0 to `count`, the first on this thread and the rest on the kept ones, and returns once
/// every one has. A band that panics makes this panic, once the rest are done.
pub fn each(count: usize, job: &(dyn Fn(usize) + Sync)) {
    if count <= 1 || bands() <= 1 {
        for band in 0..count {
            job(band);
        }
        return;
    }
    let pool = pool();
    // (bands still running, whether one panicked)
    let state = Arc::new((Mutex::new((count - 1, false)), Condvar::new()));
    // SAFETY: the job is only called before this function returns: it waits below until every band has finished with it, and
    // a band that panics is caught and counted too, so nothing it borrows can be gone while it's used.
    let job: &'static (dyn Fn(usize) + Sync) = unsafe { std::mem::transmute::<&(dyn Fn(usize) + Sync), &'static (dyn Fn(usize) + Sync)>(job) };
    for band in 1..count {
        let state = state.clone();
        let sender = &pool.senders[(band - 1) % pool.senders.len()];
        let work: Job = Box::new(move || {
            let panicked = catch_unwind(AssertUnwindSafe(|| job(band))).is_err();
            let (lock, done) = &*state;
            let mut left = lock.lock().unwrap_or_else(|e| e.into_inner());
            left.0 -= 1;
            left.1 |= panicked;
            done.notify_all();
        });
        if let Err(back) = sender.lock().unwrap_or_else(|e| e.into_inner()).send(work) {
            // The thread's gone: the band is done here.
            (back.0)();
        }
    }
    let first = catch_unwind(AssertUnwindSafe(|| job(0)));
    let (lock, done) = &*state;
    let mut left = lock.lock().unwrap_or_else(|e| e.into_inner());
    while left.0 > 0 {
        left = done.wait(left).unwrap_or_else(|e| e.into_inner());
    }
    let panicked = left.1;
    drop(left);
    if let Err(payload) = first {
        std::panic::resume_unwind(payload);
    }
    assert!(!panicked, "a band of canvas drawing panicked");
}

/// A canvas's rows, shared by bands that each only touch rows of their own.
pub struct Rows {
    start: *mut u8,
    len: usize,
}

// SAFETY: each band only makes a slice of its own rows from it (see `band`), which no other band's slice overlaps.
unsafe impl Sync for Rows {}

impl Rows {
    pub fn new(pixels: &mut [u8]) -> Self {
        Self { start: pixels.as_mut_ptr(), len: pixels.len() }
    }

    /// The bytes from `from` up to `to`.
    ///
    /// # Safety
    /// No two slices made while they're both in use may overlap, and the pixels must outlive them.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn band(&self, from: usize, to: usize) -> &mut [u8] {
        assert!(from <= to && to <= self.len);
        // SAFETY: inside the pixels, and not overlapping another band's, as the caller promises.
        unsafe { std::slice::from_raw_parts_mut(self.start.add(from), to - from) }
    }
}
