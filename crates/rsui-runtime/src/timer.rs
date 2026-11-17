//! One thread for every timer in the process, which only exists while there are timers.
//!
//! A timer costs one entry in a heap, instead of a thread of its own with a stack. The thread
//! leaves soon after the last timer fires, so a program with nothing scheduled has no thread here.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Condvar, Mutex};
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
            None => LINGER,
        };
        let (next, result) = CHANGED
            .wait_timeout(timers, wait)
            .expect("timers don't panic while holding this");
        timers = next;
        if result.timed_out() && timers.heap.is_empty() {
            timers.running = false;
            return;
        }
    }
}
