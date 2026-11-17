//! Runs this test binary on the allocator itself, so everything the test harness
//! and the standard library do also exercises it.

use std::alloc::{GlobalAlloc, Layout};
use std::collections::HashMap;
use std::thread;

use rsui_alloc::UiAlloc;

#[global_allocator]
static ALLOCATOR: UiAlloc = UiAlloc;

/// A small deterministic random number generator.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

struct Block {
    pointer: *mut u8,
    layout: Layout,
    fill: u8,
}

impl Block {
    fn assert_intact(&self) {
        let bytes = unsafe { std::slice::from_raw_parts(self.pointer, self.layout.size()) };
        assert!(
            bytes.iter().all(|byte| *byte == self.fill),
            "block was overwritten"
        );
    }
}

/// Allocates, frees and resizes random blocks, checking that each keeps its
/// contents and alignment, and that no two blocks overlap.
fn churn(seed: u64, rounds: usize, largest: u64) {
    let mut state = seed;
    let mut blocks: Vec<Block> = Vec::new();
    for round in 0..rounds {
        let action = next(&mut state) % 10;
        if action < 5 || blocks.is_empty() {
            let size = (next(&mut state) % largest + 1) as usize;
            let align = 1usize << (next(&mut state) % 7);
            let layout = Layout::from_size_align(size, align).unwrap();
            let zeroed = next(&mut state) % 4 == 0;
            let pointer = unsafe {
                if zeroed {
                    ALLOCATOR.alloc_zeroed(layout)
                } else {
                    ALLOCATOR.alloc(layout)
                }
            };
            assert!(!pointer.is_null());
            assert_eq!(pointer as usize % align, 0, "block is misaligned");
            if zeroed {
                let bytes = unsafe { std::slice::from_raw_parts(pointer, size) };
                assert!(bytes.iter().all(|byte| *byte == 0), "block is not zeroed");
            }
            let fill = next(&mut state) as u8;
            unsafe { pointer.write_bytes(fill, size) };
            blocks.push(Block {
                pointer,
                layout,
                fill,
            });
        } else if action < 8 {
            let index = (next(&mut state) as usize) % blocks.len();
            let block = blocks.swap_remove(index);
            block.assert_intact();
            unsafe { ALLOCATOR.dealloc(block.pointer, block.layout) };
        } else {
            let index = (next(&mut state) as usize) % blocks.len();
            let block = &mut blocks[index];
            block.assert_intact();
            let new_size = (next(&mut state) % largest + 1) as usize;
            let pointer = unsafe { ALLOCATOR.realloc(block.pointer, block.layout, new_size) };
            assert!(!pointer.is_null());
            let kept = block.layout.size().min(new_size);
            let bytes = unsafe { std::slice::from_raw_parts(pointer, kept) };
            assert!(
                bytes.iter().all(|byte| *byte == block.fill),
                "resize lost data"
            );
            block.pointer = pointer;
            block.layout = Layout::from_size_align(new_size, block.layout.align()).unwrap();
            block.fill = next(&mut state) as u8;
            unsafe { pointer.write_bytes(block.fill, new_size) };
        }
        if round % 256 == 0 {
            assert_no_overlap(&blocks);
        }
    }
    for block in blocks {
        block.assert_intact();
        unsafe { ALLOCATOR.dealloc(block.pointer, block.layout) };
    }
}

fn assert_no_overlap(blocks: &[Block]) {
    let mut spans: Vec<(usize, usize)> = blocks
        .iter()
        .map(|block| (block.pointer as usize, block.layout.size()))
        .collect();
    spans.sort_unstable();
    for pair in spans.windows(2) {
        assert!(pair[0].0 + pair[0].1 <= pair[1].0, "two blocks overlap");
    }
}

#[test]
fn small_blocks_on_one_thread() {
    churn(0x1234_5678_9ABC_DEF1, 200_000, 1100);
}

#[test]
fn blocks_of_every_size_on_one_thread() {
    churn(0x0F1E_2D3C_4B5A_6978, 50_000, 100_000);
}

#[test]
fn small_blocks_on_many_threads() {
    let threads: Vec<_> = (0..8u64)
        .map(|index| {
            thread::spawn(move || churn(0xA5A5_5A5A_1234_0001 + index * 7919, 100_000, 1100))
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}

#[test]
fn blocks_freed_on_another_thread() {
    let (sender, receiver) = std::sync::mpsc::channel::<Vec<Box<[u8; 64]>>>();
    let producer = thread::spawn(move || {
        for round in 0..2_000u32 {
            let blocks = (0..64).map(|_| Box::new([round as u8; 64])).collect();
            sender.send(blocks).unwrap();
        }
    });
    for (round, blocks) in receiver.iter().enumerate() {
        assert!(
            blocks
                .iter()
                .all(|block| block.iter().all(|byte| *byte == round as u8))
        );
    }
    producer.join().unwrap();
}

#[test]
fn ordinary_collections_work() {
    let mut map = HashMap::new();
    for index in 0..50_000u32 {
        map.insert(index, index.to_string());
    }
    for index in 0..50_000u32 {
        assert_eq!(map[&index], index.to_string());
    }
}

#[test]
fn global_allocator_remains_usable_during_reclaim() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let done = AtomicBool::new(false);
    thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                ALLOCATOR.reclaim(8);
                thread::yield_now();
            }
        });
        churn(0xABCD_1234_9999_0001, 100_000, 2048);
        done.store(true, Ordering::Relaxed);
    });
}
