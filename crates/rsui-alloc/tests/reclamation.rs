//! System is global here: UiAlloc's API must work independently of the global allocator.
use rsui_alloc::UiAlloc;
use std::alloc::{GlobalAlloc, Layout};

struct Block {
    p: *mut u8,
    layout: Layout,
    value: u8,
}
impl Block {
    fn new(size: usize, align: usize, value: u8) -> Self {
        let layout = Layout::from_size_align(size, align).unwrap();
        // SAFETY: valid nonzero layout. Check allocation before accessing it.
        let p = unsafe { UiAlloc.alloc_zeroed(layout) };
        assert!(!p.is_null());
        assert_eq!(p as usize % align, 0);
        unsafe {
            assert!(std::slice::from_raw_parts(p, size).iter().all(|b| *b == 0));
            p.write_bytes(value, size);
        }
        Self { p, layout, value }
    }
    fn check(&self) {
        // SAFETY: this owns an initialized live block.
        unsafe {
            assert!(
                std::slice::from_raw_parts(self.p, self.layout.size())
                    .iter()
                    .all(|b| *b == self.value)
            );
        }
    }
    fn resize(&mut self, size: usize) {
        self.check();
        // SAFETY: owns the allocation; size is valid and nonzero.
        unsafe {
            let p = UiAlloc.realloc(self.p, self.layout, size);
            assert!(!p.is_null());
            assert_eq!(p as usize % self.layout.align(), 0);
            assert!(
                std::slice::from_raw_parts(p, size.min(self.layout.size()))
                    .iter()
                    .all(|b| *b == self.value)
            );
            p.write_bytes(self.value, size);
            self.p = p;
            self.layout = Layout::from_size_align(size, self.layout.align()).unwrap();
        }
    }
}
// SAFETY: Block exclusively owns its allocation. UiAlloc supports cross-thread deallocation.
unsafe impl Send for Block {}
impl Drop for Block {
    fn drop(&mut self) {
        self.check();
        // SAFETY: original layout and live allocation, freed exactly once.
        unsafe {
            UiAlloc.dealloc(self.p, self.layout);
        }
    }
}
fn drain() {
    while UiAlloc.reclaim(32) != 0 {}
}

// One serial test keeps exact backing assertions independent of harness scheduling.
#[test]
fn reclamation_preserves_live_allocations_and_is_bounded() {
    assert_eq!(UiAlloc.retained_bytes(), 0);
    assert_eq!(UiAlloc.reclaim(usize::MAX), 0);
    assert!(Layout::from_size_align(usize::MAX, 16).is_err());
    let mut live = Vec::new();
    // Every accepted size and boundary, over-aligned and System fallback layouts.
    for size in 1..=1100 {
        for align in [1, 2, 4, 8, 16, 32, 64, 4096] {
            live.push(Block::new(size, align, size as u8));
        }
    }
    let backing = UiAlloc.retained_bytes();
    assert!(backing > 0);
    assert_eq!(UiAlloc.reclaim(32), 0, "all-live must be a no-op");
    for block in &mut live {
        block.resize(block.layout.size() + 1); // includes same-class and boundary realloc
        block.resize(2048);
        block.resize(17);
    }
    drain();
    for b in &live {
        b.check();
    }
    drop(live);
    drain();
    assert_eq!(UiAlloc.retained_bytes(), 0);

    // Keep scattered blocks live across many chunks; empty chunks interleave live ones.
    let mut live = Vec::new();
    for i in 0..20000 {
        live.push(Block::new(1024, 16, i as u8));
    }
    let peak = UiAlloc.retained_bytes();
    let mut keep = Vec::new();
    for (i, block) in live.into_iter().enumerate() {
        if i % 256 == 0 {
            keep.push(block);
        }
    }
    assert_eq!(UiAlloc.reclaim(0), 0);
    assert_eq!(UiAlloc.retained_bytes(), peak);
    let released = UiAlloc.reclaim(1);
    assert_eq!(released, 65536);
    assert_eq!(UiAlloc.retained_bytes(), peak - released);
    assert!(UiAlloc.reclaim(usize::MAX) <= 32 * 65536);
    drain();
    assert!(UiAlloc.retained_bytes() > 0);
    for b in &keep {
        b.check();
    }
    // Allocation while reclaimed holes exist must never alias surviving pointers.
    let fresh: Vec<_> = (0..20000)
        .map(|i| Block::new(1024, 16, (i + 1) as u8))
        .collect();
    for b in &keep {
        b.check();
    }
    drop(fresh);
    drop(keep);
    drain();
    assert_eq!(UiAlloc.retained_bytes(), 0);

    // Concurrent reclaim/alloc/realloc plus ownership transfer between threads.
    let done = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        let done = &done;
        scope.spawn(move || {
            while !done.load(std::sync::atomic::Ordering::Relaxed) {
                UiAlloc.reclaim(3);
                std::thread::yield_now();
            }
        });
        let workers: Vec<_> = (0..4)
            .map(|seed| {
                scope.spawn(move || {
                    for round in 0..100 {
                        let mut blocks: Vec<_> = (0..1000)
                            .map(|i| Block::new((i * 97 + seed * 11) % 1100 + 1, 16, round as u8))
                            .collect();
                        for b in &mut blocks {
                            b.resize((b.layout.size() * 17) % 2048 + 1);
                        }
                        UiAlloc.reclaim(2);
                        UiAlloc.reclaim_for_idle(2);
                    }
                    (0..1000)
                        .map(|i| Block::new(i + 1, 16, seed as u8))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for w in workers {
            drop(w.join().unwrap());
        }
        done.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    drain();
    assert_eq!(UiAlloc.retained_bytes(), 0);
    assert_eq!(UiAlloc.reclaim(32), 0);

    // A valid but impossible allocation must fail without destroying the old block.
    #[cfg(target_pointer_width = "64")]
    {
        let b = Block::new(32, 16, 42);
        let enormous = isize::MAX as usize - 4095;
        // SAFETY: valid new size, original allocation/layout. OS cannot satisfy it.
        let failed = unsafe { UiAlloc.realloc(b.p, b.layout, enormous) };
        assert!(failed.is_null());
        b.check();
        drop(b);
        drain();
    }

    // Short-idle release keeps a bounded existing reserve, and a small return
    // uses that storage without acquiring new backing. Full release still works.
    assert_eq!(UiAlloc.reclaim_for_idle(32), 0);
    assert_eq!(UiAlloc.retained_bytes(), 0, "must not allocate a reserve");
    let sizes = [16, 48, 192, 1024];
    let mut burst = Vec::new();
    for size in sizes {
        for _ in 0..(2 * 65536 / size + 2) {
            burst.push(Block::new(size, 16, 73));
        }
    }
    drop(burst);
    let peak = UiAlloc.retained_bytes();
    assert_eq!(UiAlloc.reclaim_for_idle(0), 0);
    assert_eq!(UiAlloc.retained_bytes(), peak);
    assert_eq!(UiAlloc.reclaim_for_idle(1), 65536);
    while UiAlloc.reclaim_for_idle(32) != 0 {}
    let reserve = sizes.len() * 65536;
    assert_eq!(UiAlloc.retained_bytes(), reserve);
    let returned: Vec<_> = sizes.into_iter().map(|s| Block::new(s, 16, 91)).collect();
    assert_eq!(UiAlloc.retained_bytes(), reserve);
    assert_eq!(UiAlloc.reclaim_for_idle(32), 0);
    for block in &returned {
        block.check();
    }
    drop(returned);
    assert_eq!(UiAlloc.reclaim_for_idle(32), 0);
    drain();
    assert_eq!(UiAlloc.retained_bytes(), 0);
}
