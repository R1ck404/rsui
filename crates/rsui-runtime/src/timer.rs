//! One thread for every timer in the process, which only exists while there are timers.
//!
//! A timer costs one entry in a heap, instead of a thread of its own with a stack. The thread
//! leaves soon after the last timer fires, so a program with nothing scheduled has no thread here.
//!
//! On Windows it waits on a high resolution timer rather than a condition variable, whose waits only end on the system's clock
//! tick: every 15.6 ms unless something's asked for it to be faster, which made a timer for 66.7 ms go off at 78.1, and an
//! animation asking for 15 frames a second get 12.8.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

type Job = Box<dyn FnOnce() + Send>;

/// How long the thread waits for more timers before leaving.
const LINGER: Duration = Duration::from_secs(2);
/// A timer thread only passes messages along, so it needs very little stack.
const STACK: usize = 64 * 1024;

struct Timer {
    due: Instant,
    /// Timers due at the same time fire in the order they were made.
    order: u64,
    job: Job,
}

impl PartialEq for Timer {
    fn eq(&self, other: &Self) -> bool {
        (self.due, self.order) == (other.due, other.order)
    }
}

impl Eq for Timer {}

impl PartialOrd for Timer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Timer {
    /// The other way around, as a [`BinaryHeap`] gives the largest first.
    fn cmp(&self, other: &Self) -> Ordering {
        (other.due, other.order).cmp(&(self.due, self.order))
    }
}

struct Timers {
    heap: BinaryHeap<Timer>,
    next_order: u64,
    running: bool,
}

static TIMERS: Mutex<Timers> = Mutex::new(Timers {
    heap: BinaryHeap::new(),
    next_order: 0,
    running: false,
});
static CHANGED: Condvar = Condvar::new();

/// Runs `job` after `delay`, on the timer thread. It must be quick, as other timers wait for it.
pub(crate) fn after(delay: Duration, job: impl FnOnce() + Send + 'static) {
    let due = Instant::now().checked_add(delay);
    // A delay beyond what a clock can count is never going to happen. It still has to be
    // dropped rather than leaked, to finish what's waiting on it.
    let Some(due) = due else {
        drop(job);
        return;
    };
    let mut timers = TIMERS
        .lock()
        .expect("timers don't panic while holding this");
    let order = timers.next_order;
    timers.next_order += 1;
    timers.heap.push(Timer {
        due,
        order,
        job: Box::new(job),
    });
    if timers.running {
        CHANGED.notify_one();
        #[cfg(windows)]
        if let Some(clock) = precise::clock() {
            clock.changed();
        }
        return;
    }
    let started = std::thread::Builder::new()
        .name("rsui timers".into())
        .stack_size(STACK)
        .spawn(run);
    if started.is_ok() {
        timers.running = true;
    }
}

fn run() {
    let mut timers = TIMERS
        .lock()
        .expect("timers don't panic while holding this");
    // Since when there's been nothing to wait for.
    let mut idle: Option<Instant> = None;
    loop {
        let wait = match timers.heap.peek() {
            Some(next) => match next.due.checked_duration_since(Instant::now()) {
                Some(wait) if !wait.is_zero() => wait,
                _ => {
                    let timer = timers.heap.pop().expect("one was just seen");
                    drop(timers);
                    // One bad timer mustn't stop all the others from ever firing.
                    let _ = catch_unwind(AssertUnwindSafe(timer.job));
                    timers = TIMERS
                        .lock()
                        .expect("timers don't panic while holding this");
                    continue;
                }
            },
            None => {
                let since = *idle.get_or_insert_with(Instant::now);
                match LINGER.checked_sub(since.elapsed()) {
                    Some(wait) if !wait.is_zero() => wait,
                    _ => {
                        timers.running = false;
                        return;
                    }
                }
            }
        };
        timers = sleep(timers, wait);
        if !timers.heap.is_empty() {
            idle = None;
        }
    }
}

/// Waits for `wait`, or until a timer's added, and has the timers again.
fn sleep(timers: MutexGuard<'static, Timers>, wait: Duration) -> MutexGuard<'static, Timers> {
    #[cfg(windows)]
    if let Some(clock) = precise::clock() {
        // A timer added meanwhile sets the clock's event, which ends this wait at once.
        drop(timers);
        clock.wait(wait);
        return TIMERS
            .lock()
            .expect("timers don't panic while holding this");
    }
    CHANGED
        .wait_timeout(timers, wait)
        .expect("timers don't panic while holding this")
        .0
}

/// A wait that ends on time, to within a millisecond or so, rather than on the system's next clock tick.
#[cfg(windows)]
mod precise {
    use std::ffi::c_void;
    use std::sync::OnceLock;
    use std::time::Duration;

    type Handle = *mut c_void;

    const CREATE_WAITABLE_TIMER_HIGH_RESOLUTION: u32 = 2;
    const TIMER_ALL_ACCESS: u32 = 0x1F_0003;
    const INFINITE: u32 = u32::MAX;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateWaitableTimerExW(attributes: *const c_void, name: *const u16, flags: u32, access: u32) -> Handle;
        fn SetWaitableTimer(timer: Handle, due: *const i64, period: i32, routine: *const c_void, argument: *const c_void, resume: i32) -> i32;
        fn CreateEventW(attributes: *const c_void, manual_reset: i32, initial_state: i32, name: *const u16) -> Handle;
        fn SetEvent(event: Handle) -> i32;
        fn WaitForMultipleObjects(count: u32, handles: *const Handle, wait_all: i32, milliseconds: u32) -> u32;
    }

    pub(super) struct Clock {
        timer: usize,
        changed: usize,
    }

    /// The clock, made the first time it's wanted and kept, or `None` before Windows 10 1803, which has no such timer.
    pub(super) fn clock() -> Option<&'static Clock> {
        static CLOCK: OnceLock<Option<Clock>> = OnceLock::new();
        CLOCK
            .get_or_init(|| {
                // SAFETY: plain object creation; what's made is kept for as long as the process runs.
                unsafe {
                    let timer = CreateWaitableTimerExW(std::ptr::null(), std::ptr::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS);
                    if timer.is_null() {
                        return None;
                    }
                    let changed = CreateEventW(std::ptr::null(), 0, 0, std::ptr::null());
                    if changed.is_null() {
                        return None;
                    }
                    Some(Clock { timer: timer as usize, changed: changed as usize })
                }
            })
            .as_ref()
    }

    impl Clock {
        /// Ends a wait that's going on, or the next one if none is.
        pub(super) fn changed(&self) {
            // SAFETY: the event is kept for as long as the process runs.
            unsafe {
                SetEvent(self.changed as Handle);
            }
        }

        /// Waits for `wait`, or until [`changed`](Self::changed). Only the timer thread waits.
        pub(super) fn wait(&self, wait: Duration) {
            let handles = [self.changed as Handle, self.timer as Handle];
            // Relative, in 100 ns units, as a negative number.
            let due = -((wait.as_nanos() / 100).clamp(1, i64::MAX as u128) as i64);
            // SAFETY: the handles are kept for as long as the process runs, and only the timer thread sets the timer.
            unsafe {
                if SetWaitableTimer(handles[1], &due, 0, std::ptr::null(), std::ptr::null(), 0) != 0 {
                    WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE);
                } else {
                    WaitForMultipleObjects(1, handles.as_ptr(), 0, wait.as_millis().clamp(1, u32::MAX as u128 - 1) as u32);
                }
            }
        }
    }
}
