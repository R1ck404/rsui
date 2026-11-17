use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use crate::cx::Updates;

pub(crate) type Job = Box<dyn FnOnce(&mut Updates<'_>) + Send>;

/// Called when the runtime has work to flush, from whichever thread caused it.
pub(crate) type Waker = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Queue {
    jobs: Vec<Job>,
    /// Background tasks still running.
    tasks: usize,
    closed: bool,
}

/// How other threads reach a runtime.
#[derive(Default)]
pub(crate) struct Inbox {
    queue: Mutex<Queue>,
    changed: Condvar,
    waker: Mutex<Option<Waker>>,
}

impl Inbox {
    pub(crate) fn set_waker(&self, waker: Waker) {
        *self.waker.lock().expect("lock is not poisoned") = Some(waker.clone());
        // What arrived before there was anyone to tell, as a timer an effect started while the first frame was drawn, is
        // said now. One that arrives in between is said twice, which does no harm.
        if !self.queue.lock().expect("lock is not poisoned").jobs.is_empty() {
            waker();
        }
    }

    fn wake(&self) {
        self.changed.notify_all();
        let waker = self.waker.lock().expect("lock is not poisoned").clone();
        if let Some(waker) = waker {
            waker();
        }
    }

    pub(crate) fn take_jobs(&self) -> Vec<Job> {
        std::mem::take(&mut self.queue.lock().expect("lock is not poisoned").jobs)
    }

    /// Whether any job is waiting, or any task could still send one.
    pub(crate) fn is_busy(&self) -> bool {
        let queue = self.queue.lock().expect("lock is not poisoned");
        !queue.jobs.is_empty() || queue.tasks > 0
    }

    /// Blocks until a job is waiting, no task could send one, or `deadline` passes.
    pub(crate) fn wait(&self, deadline: Instant) {
        let mut queue = self.queue.lock().expect("lock is not poisoned");
        while queue.jobs.is_empty() && queue.tasks > 0 {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return;
            };
            queue = self
                .changed
                .wait_timeout(queue, remaining)
                .expect("lock is not poisoned")
                .0;
        }
    }

    /// Stops accepting jobs, since the runtime they're for is gone.
    pub(crate) fn close(&self) {
        let mut queue = self.queue.lock().expect("lock is not poisoned");
        queue.closed = true;
        queue.jobs = Vec::new();
    }

    pub(crate) fn begin_task(&self) {
        self.queue.lock().expect("lock is not poisoned").tasks += 1;
    }
}

/// A way to change state from another thread. Clone it and send it anywhere.
///
/// Changes wait for the next [`Runtime::flush`](crate::Runtime::flush). Changing the
/// state of a component that has since unmounted does nothing, so a slow task can't
/// disturb a screen the user has already left.
#[derive(Clone)]
pub struct Remote {
    inbox: Arc<Inbox>,
}

impl Remote {
    pub(crate) fn new(inbox: Arc<Inbox>) -> Self {
        Self { inbox }
    }

    /// Queues a change, and returns `false` if the runtime no longer exists, so a
    /// long-running task knows it can stop.
    pub fn send(&self, job: impl FnOnce(&mut Updates<'_>) + Send + 'static) -> bool {
        {
            let mut queue = self.inbox.queue.lock().expect("lock is not poisoned");
            if queue.closed {
                return false;
            }
            queue.jobs.push(Box::new(job));
        }
        self.inbox.wake();
        true
    }
}

/// Marks a background task as finished, even if it panics.
pub(crate) struct TaskGuard(pub(crate) Arc<Inbox>);

impl Drop for TaskGuard {
    fn drop(&mut self) {
        // A panicking task must not poison anything further, or hide its own panic.
        if let Ok(mut queue) = self.0.queue.lock() {
            queue.tasks -= 1;
        }
        self.0.wake();
    }
}
