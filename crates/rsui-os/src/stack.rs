//! How much stack is left, and running something on a new stack of its own, on the same thread, for work that goes down
//! a tree by calling itself (see `rsui_core::stack::deep`).
//!
//! Windows has both built in, as fibers, and `rsui_core` uses those there. Elsewhere the thread's stack is asked for its
//! bounds once and remembered, and a new stack is a mapping of its own with an unmapped page at its far end, which is
//! switched to and back with a few instructions.

/// How much of this thread's stack is left below here, if it can be told.
#[inline]
pub fn room_left() -> Option<usize> {
    imp::room_left()
}

/// Runs `run` on a new stack of `size` bytes, on this thread, and comes back with what it gave. Only what's used of the
/// stack is taken from memory. A panic in `run` comes back up here.
#[inline(never)]
pub fn on_new_stack<R>(size: usize, run: impl FnOnce() -> R) -> R {
    imp::on_new_stack(size, run)
}

#[cfg(windows)]
mod imp {
    pub fn room_left() -> Option<usize> {
        None
    }

    pub fn on_new_stack<R>(_size: usize, run: impl FnOnce() -> R) -> R {
        run()
    }
}

#[cfg(unix)]
mod imp {
    use std::cell::Cell;
    use std::ffi::c_void;
    use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

    thread_local! {
        /// The lowest address of the stack this thread is on now: its own, or the newest one it's switched to. 0 until
        /// it's been asked for, and 1 if it can't be told.
        static LOW: Cell<usize> = const { Cell::new(0) };
    }

    pub fn room_left() -> Option<usize> {
        // A local of this frame's, not `&0`, which would be a constant somewhere in the program rather than on the stack.
        let marker = 0u8;
        let here = std::hint::black_box(&raw const marker) as usize;
        let low = LOW.with(|low| {
            if low.get() == 0 {
                low.set(thread_stack_low().unwrap_or(1));
            }
            low.get()
        });
        (low > 1 && here > low).then(|| here - low)
    }

    /// The lowest address of this thread's own stack.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn thread_stack_low() -> Option<usize> {
        // SAFETY: the attributes are made, read and destroyed here; `pthread_getattr_np` fills them in for this thread.
        unsafe {
            let mut attr: libc::pthread_attr_t = std::mem::zeroed();
            if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
                return None;
            }
            let mut addr: *mut c_void = std::ptr::null_mut();
            let mut size = 0usize;
            let got = libc::pthread_attr_getstack(&attr, &mut addr, &mut size);
            libc::pthread_attr_destroy(&mut attr);
            // The guard page at the bottom is part of what's given, and a little more is left for the system's own
            // handling of an overflow.
            (got == 0 && !addr.is_null()).then(|| addr as usize + 64 * 1024)
        }
    }

    #[cfg(target_vendor = "apple")]
    fn thread_stack_low() -> Option<usize> {
        // SAFETY: both only read this thread's own description. The address given is the stack's top.
        unsafe {
            let this = libc::pthread_self();
            let top = libc::pthread_get_stackaddr_np(this) as usize;
            let size = libc::pthread_get_stacksize_np(this);
            (top != 0 && size != 0).then(|| top - size + 64 * 1024)
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
    fn thread_stack_low() -> Option<usize> {
        None
    }

    struct Job<F, R> {
        run: Option<F>,
        result: Option<std::thread::Result<R>>,
    }

    /// What runs first on the new stack. A panic can't go past the start of a stack, so it's caught here.
    extern "C" fn start<F: FnOnce() -> R, R>(job: *mut c_void) {
        // SAFETY: this is the `Job` below, which lasts until this returns, and nothing else touches it meanwhile.
        let job = unsafe { &mut *job.cast::<Job<F, R>>() };
        let run = job.run.take().expect("a stack is only started once");
        job.result = Some(catch_unwind(AssertUnwindSafe(run)));
    }

    pub fn on_new_stack<R, F: FnOnce() -> R>(size: usize, run: F) -> R {
        let page = crate::vm::page_size();
        let size = size.next_multiple_of(page) + page;
        let Some(base) = crate::vm::reserve(size) else {
            return run();
        };
        // Everything but the lowest page, which is left unmapped so running off the end faults rather than writing
        // over whatever's below.
        // SAFETY: inside what was just reserved.
        if !unsafe { crate::vm::commit(base.add(page), size - page) } {
            // SAFETY: what was just reserved.
            unsafe { crate::vm::release(base, size) };
            return run();
        }
        let mut job = Job { run: Some(run), result: None };
        let top = (base as usize + size) & !15;
        let low = base as usize + page + 16 * 1024;
        let was = LOW.with(|it| it.replace(low));
        // SAFETY: `top` is the 16-byte-aligned end of a fresh stack that nothing else uses, and `start` is an
        // `extern "C"` function that doesn't unwind, given the job, which lives until it returns.
        unsafe { switch(top, start::<F, R>, (&raw mut job).cast()) };
        LOW.with(|it| it.set(was));
        // SAFETY: the stack is finished with.
        unsafe { crate::vm::release(base, size) };
        match job.result.take().expect("the new stack ran") {
            Ok(result) => result,
            Err(panic) => resume_unwind(panic),
        }
    }

    /// Calls `f(arg)` with the stack pointer at `top`, and puts it back after.
    #[cfg(target_arch = "x86_64")]
    unsafe fn switch(top: usize, f: extern "C" fn(*mut c_void), arg: *mut c_void) {
        // SAFETY: the old stack pointer is kept in r12, which `f` keeps as the C convention says, and put back after.
        // `top` is 16-byte aligned, so `f` starts with the alignment a call gives.
        unsafe {
            std::arch::asm!(
                "mov r12, rsp",
                "mov rsp, {top}",
                "call {f}",
                "mov rsp, r12",
                top = in(reg) top,
                f = in(reg) f,
                in("rdi") arg,
                out("r12") _,
                clobber_abi("C"),
            );
        }
    }

    #[cfg(target_arch = "aarch64")]
    unsafe fn switch(top: usize, f: extern "C" fn(*mut c_void), arg: *mut c_void) {
        // SAFETY: as for x86-64, with the old stack pointer in x20, which `f` keeps.
        unsafe {
            std::arch::asm!(
                "mov x20, sp",
                "mov sp, {top}",
                "blr {f}",
                "mov sp, x20",
                top = in(reg) top,
                f = in(reg) f,
                in("x0") arg,
                out("x20") _,
                clobber_abi("C"),
            );
        }
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    unsafe fn switch(_top: usize, f: extern "C" fn(*mut c_void), arg: *mut c_void) {
        f(arg)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_thread_knows_how_much_stack_it_has_left() {
        let left = std::thread::Builder::new().stack_size(512 * 1024).spawn(room_left).unwrap().join().unwrap();
        let left = left.expect("this system can tell");
        // glibc hands a new thread a stack it kept from one that ended, which can be bigger than was asked for.
        assert!(left > 100 * 1024 && left < 64 * 1024 * 1024, "{left}");
    }

    #[test]
    fn a_new_stack_has_room_and_gives_back_what_ran_on_it() {
        let (before, on_it) = on_new_stack(1 << 20, || (room_left(), 41 + 1));
        assert_eq!(on_it, 42);
        assert!(before.unwrap() > 900 * 1024);
    }

    #[test]
    fn a_panic_on_a_new_stack_comes_back_up() {
        let result = std::panic::catch_unwind(|| on_new_stack(1 << 20, || panic!("down there")));
        assert!(result.is_err());
    }
}
