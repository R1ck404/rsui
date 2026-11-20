//! Ranges of addresses reserved straight from the system, with what each is for, so a measuring step (`regions` in
//! rsui-tsx's steps) can say whose each part of the process's memory is. Noting one is two stores, when it's reserved,
//! and there are only a few dozen, so it's always on.

use std::sync::atomic::{AtomicUsize, Ordering};

const MAX: usize = 512;

#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicUsize = AtomicUsize::new(0);
static BASES: [AtomicUsize; MAX] = [ZERO; MAX];
static LABEL_PTRS: [AtomicUsize; MAX] = [ZERO; MAX];
static LABEL_LENS: [AtomicUsize; MAX] = [ZERO; MAX];

/// Says what the range reserved at `base` is for. A range that's given back is forgotten with [`forget`].
pub fn note(base: *const u8, label: &'static str) {
    let base = base as usize;
    if base == 0 {
        return;
    }
    for at in 0..MAX {
        if BASES[at].load(Ordering::Relaxed) == 0
            && BASES[at].compare_exchange(0, usize::MAX, Ordering::AcqRel, Ordering::Relaxed).is_ok()
        {
            LABEL_PTRS[at].store(label.as_ptr() as usize, Ordering::Relaxed);
            LABEL_LENS[at].store(label.len(), Ordering::Relaxed);
            BASES[at].store(base, Ordering::Release);
            return;
        }
    }
}

/// Forgets the range reserved at `base`, as it's been given back.
pub fn forget(base: *const u8) {
    let base = base as usize;
    for slot in &BASES {
        if slot.load(Ordering::Relaxed) == base {
            slot.store(0, Ordering::Release);
            return;
        }
    }
}

/// What each noted range's reservation starts at, and what it's for.
pub fn noted() -> Vec<(usize, &'static str)> {
    let mut all = Vec::new();
    for at in 0..MAX {
        let base = BASES[at].load(Ordering::Acquire);
        if base == 0 || base == usize::MAX {
            continue;
        }
        let (ptr, len) = (LABEL_PTRS[at].load(Ordering::Relaxed), LABEL_LENS[at].load(Ordering::Relaxed));
        // SAFETY: it was a `&'static str`'s pointer and length, stored before its base was.
        #[allow(unsafe_code)]
        let label = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr as *const u8, len)) };
        all.push((base, label));
    }
    all
}
