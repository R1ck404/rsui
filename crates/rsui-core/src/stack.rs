//! More stack for work that goes down a tree by calling itself, when the tree is deeper than the stack it's on has room for.
//!
//! Most walks of a tree go through it with a list of where they've got to (see `Motions::visit` in rsui-runtime), so they need
//! no more stack for a deep tree than a shallow one. A few, as layout's, are many functions calling one another for each level,
//! where turning them into a loop would turn them inside out. Those call [`deep`] once a level: while there's room it just runs
//! what it's given, and when the stack is nearly used up it runs it on a new stack and comes back, as `stacker` does, so a tree
//! of any depth works on a thread with a normal stack, in a debug build as well. A new stack is only ever made that deep in.

/// Below this much stack left, [`deep`] goes on with another stack. One level of layout in a debug build takes about 5 KB,
/// and the stack Windows keeps for handling an overflow has to be left alone.
const LOW: usize = 192 * 1024;

/// How big each new stack is. Only what's used of it is ever taken from memory.
const SIZE: usize = 8 * 1024 * 1024;

/// Runs `run`, on another stack if there isn't much of this one left.
#[inline]
pub fn deep<R>(run: impl FnOnce() -> R) -> R {
    if room_left().is_none_or(|left| left >= LOW) {
        run()
    } else {
        on_new_stack(run)
    }
}

/// How much of the stack is left below here, if it can be told.
#[cfg(windows)]
fn room_left() -> Option<usize> {
    let mut low = 0usize;
    let mut high = 0usize;
    // SAFETY: both point to somewhere to write a number.
    unsafe { windows::GetCurrentThreadStackLimits(&mut low, &mut high) };
    let here = std::hint::black_box(&low as *const usize as usize);
    (low != 0 && here > low).then(|| here - low)
}

#[cfg(not(windows))]
fn room_left() -> Option<usize> {
    None
}

#[cfg(not(windows))]
fn on_new_stack<R>(run: impl FnOnce() -> R) -> R {
    run()
}

/// Runs `run` on a fiber of its own, which is a stack of its own on this same thread, and comes back to this one when it's
/// done. Being on the same thread, what it uses of this thread's (its thread-locals, and what isn't `Send`) is all still there.
#[cfg(windows)]
#[inline(never)]
fn on_new_stack<R, F: FnOnce() -> R>(run: F) -> R {
    use std::ffi::c_void;
    use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

    struct Job<F, R> {
        run: Option<F>,
        result: Option<std::thread::Result<R>>,
        back: *mut c_void,
    }

    unsafe extern "system" fn start<F: FnOnce() -> R, R>(job: *mut c_void) {
        // SAFETY: this is the `Job` below, which lasts until this switches back, and nothing else touches it meanwhile.
        let job = unsafe { &mut *job.cast::<Job<F, R>>() };
        let run = job.run.take().expect("a fiber is only started once");
        // A panic can't go past the start of a stack, so it's caught here and carried on with once back.
        job.result = Some(catch_unwind(AssertUnwindSafe(run)));
        // SAFETY: `back` is the fiber that switched here, which is still there, waiting for this.
        unsafe { windows::SwitchToFiber(job.back) };
        unreachable!("a finished fiber is never switched back to");
    }

    // SAFETY: these are the fiber calls as Windows documents them. This thread is made a fiber only if it isn't one, and made
    // a thread again in that case once done; the new fiber is deleted only after it's switched back from, when it's finished.
    unsafe {
        let was_fiber = windows::IsThreadAFiber() != 0;
        let back = if was_fiber {
            windows::current_fiber()
        } else {
            windows::ConvertThreadToFiber(std::ptr::null_mut())
        };
        if back.is_null() {
            return run();
        }
        let mut job = Job::<F, R> {
            run: Some(run),
            result: None,
            back,
        };
        let fiber = windows::CreateFiberEx(
            0,
            SIZE,
            windows::FIBER_FLAG_FLOAT_SWITCH,
            start::<F, R>,
            (&raw mut job).cast(),
        );
        if fiber.is_null() {
            if !was_fiber {
                windows::ConvertFiberToThread();
            }
            let run = job.run.take().expect("not run yet");
            return run();
        }
        windows::SwitchToFiber(fiber);
        windows::DeleteFiber(fiber);
        if !was_fiber {
            windows::ConvertFiberToThread();
        }
        match job.result.take().expect("the fiber finished") {
            Ok(result) => result,
            Err(panic) => resume_unwind(panic),
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;

    pub const FIBER_FLAG_FLOAT_SWITCH: u32 = 1;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetCurrentThreadStackLimits(low: *mut usize, high: *mut usize);
        pub fn IsThreadAFiber() -> i32;
        pub fn ConvertThreadToFiber(parameter: *mut c_void) -> *mut c_void;
        pub fn ConvertFiberToThread() -> i32;
        pub fn CreateFiberEx(
            commit: usize,
            reserve: usize,
            flags: u32,
            start: unsafe extern "system" fn(*mut c_void),
            parameter: *mut c_void,
        ) -> *mut c_void;
        pub fn SwitchToFiber(fiber: *mut c_void);
        pub fn DeleteFiber(fiber: *mut c_void);
    }

    /// `current_fiber` is a macro in the headers, reading the fiber from the thread's information block.
    pub unsafe fn current_fiber() -> *mut c_void {
        let fiber: *mut c_void;
        // SAFETY: on 64-bit Windows `gs` points at the thread's information block, and the fiber is at 0x20 in it.
        unsafe { std::arch::asm!("mov {}, gs:[0x20]", out(reg) fiber, options(nostack, readonly, preserves_flags)) };
        fiber
    }
}

#[cfg(test)]
mod tests {
    use super::deep;

    fn down(depth: usize) -> usize {
        // A big frame each time, as a debug build's layout has.
        let mut room = [0u8; 4096];
        room[depth % 4096] = 1;
        std::hint::black_box(&mut room);
        if depth == 0 { 0 } else { deep(|| down(depth - 1)) + usize::from(room[depth % 4096]) }
    }

    #[test]
    fn going_far_deeper_than_the_stack_has_room_for_works() {
        let deepest = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| down(5_000))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(deepest, 5_000);
    }

    #[test]
    fn a_panic_down_there_comes_back_up() {
        let result = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                std::panic::catch_unwind(|| {
                    fn fail(depth: usize) -> usize {
                        let room = std::hint::black_box([0u8; 4096]);
                        if depth == 0 { panic!("at the bottom") } else { deep(|| fail(depth - 1)) + usize::from(room[0]) }
                    }
                    fail(1_000)
                })
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(result.is_err());
    }
}
