//! What the engine's own lists take from the system, as opposed to what's in them: for measuring memory. It's a
//! child of `gc.rs`, so it can read the lists' insides.

use crate::gc::FreeSlots;
use crate::object::{Kind, Object};
use crate::vm::Vm;

/// One of the engine's lists of places: how many places it has, how many bytes of it have real memory behind them, how
/// many of its places are free, and how many of those are on pages with nothing else on them.
#[derive(Clone, Copy, Debug, Default)]
pub struct ListFootprint {
    pub places: usize,
    pub place_bytes: usize,
    pub committed: usize,
    pub free: usize,
    pub free_on_empty_pages: usize,
    /// Pages of it that have been given back while the places on them were free (see `RSUI_GIVE_BACK`).
    pub given_back_pages: usize,
}

/// Values objects keep outside themselves, in `Vec`s of their own: how many `Vec`s, the values in them, the room they have,
/// and what they take and would take at their length, in the allocator's size classes.
#[derive(Clone, Copy, Debug, Default)]
pub struct VecFootprint {
    pub vecs: usize,
    pub length_bytes: usize,
    pub capacity_bytes: usize,
    pub classed_bytes: usize,
    pub classed_at_length: usize,
}

/// The engine's three lists of places, the values kept outside objects, and how full the pages of the list of objects are.
#[derive(Clone, Copy, Debug, Default)]
pub struct Footprint {
    pub objects: ListFootprint,
    pub strings: ListFootprint,
    pub cells: ListFootprint,
    /// Property values past the few inside an object.
    pub slots: VecFootprint,
    /// Arrays' elements past the few inside their object.
    pub items: VecFootprint,
    /// Pages of the list of objects by how many of their 64 places are free: none, 1-15, 16-31, 32-47, 48-63, all.
    pub pages_by_free: [usize; 6],
}

/// What a block of this many bytes takes in rsui-alloc's size classes, or with a heap's header past its biggest class.
fn classed(bytes: usize) -> usize {
    const CLASSES: [usize; 20] = [16, 32, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 640, 768, 896, 1024];
    if bytes == 0 {
        return 0;
    }
    CLASSES.iter().copied().find(|class| *class >= bytes).unwrap_or(bytes.next_multiple_of(16) + 16)
}

impl VecFootprint {
    fn add(&mut self, length: usize, capacity: usize) {
        if capacity == 0 {
            return;
        }
        self.vecs += 1;
        self.length_bytes += length * 8;
        self.capacity_bytes += capacity * 8;
        self.classed_bytes += classed(capacity * 8);
        self.classed_at_length += classed(length * 8);
    }
}

impl std::fmt::Display for Footprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (name, list) in [("objects", &self.objects), ("strings", &self.strings), ("cells", &self.cells)] {
            writeln!(
                f,
                "{name}: {} places of {} B, {} B committed, {} free, {} of them on empty pages, {} pages given back",
                list.places, list.place_bytes, list.committed, list.free, list.free_on_empty_pages, list.given_back_pages
            )?;
        }
        for (name, vecs) in [("slots", &self.slots), ("items", &self.items)] {
            writeln!(
                f,
                "{name}: {} vecs, {} B of values, {} B of room, {} B in classes, {} B in classes at their length",
                vecs.vecs, vecs.length_bytes, vecs.capacity_bytes, vecs.classed_bytes, vecs.classed_at_length
            )?;
        }
        let pages = &self.pages_by_free;
        writeln!(
            f,
            "pages of objects by free places: none {}, 1-15 {}, 16-31 {}, 32-47 {}, 48-63 {}, all {}",
            pages[0], pages[1], pages[2], pages[3], pages[4], pages[5]
        )
    }
}

fn list(len: usize, place_bytes: usize, committed: usize, free: &FreeSlots, given_back: usize) -> ListFootprint {
    let per_page = (4096 / place_bytes).max(1);
    let mut free_count = 0;
    let mut on_empty = 0;
    // A page is empty when every place that starts on it is free. Places are counted by where they start.
    let mut page_free = 0;
    let mut page_all = 0;
    let mut page = 0;
    for index in 0..len {
        let at = index * place_bytes / 4096;
        if at != page {
            if page_all > 0 && page_free == page_all {
                on_empty += page_free;
            }
            page = at;
            page_free = 0;
            page_all = 0;
        }
        page_all += 1;
        if free.is_free(index) {
            free_count += 1;
            page_free += 1;
        }
    }
    if page_all > 0 && page_free == page_all {
        on_empty += page_free;
    }
    let _ = per_page;
    ListFootprint {
        places: len,
        place_bytes,
        committed,
        free: free_count,
        free_on_empty_pages: on_empty,
        given_back_pages: given_back,
    }
}

impl Vm {
    pub(crate) fn footprint(&self) -> Footprint {
        let mut slots = VecFootprint::default();
        let mut items = VecFootprint::default();
        let mut pages_by_free = [0; 6];
        let per_page = 4096 / size_of::<Object>();
        for (page, objects) in self.objects.list.chunks(per_page).enumerate() {
            let mut free = 0;
            for (at, object) in objects.iter().enumerate() {
                if self.free_objects.holes().has(page * per_page + at) {
                    free = per_page;
                    break;
                }
                if self.free_objects.is_free(page * per_page + at) {
                    free += 1;
                    continue;
                }
                let heap = object.slots_heap_bytes() / 8;
                slots.add(if heap == 0 { 0 } else { object.slots().len() }, heap);
                if let Kind::Array(list) | Kind::Arguments(list) = &object.kind {
                    items.add(list.len(), list.capacity());
                }
            }
            pages_by_free[match free {
                0 => 0,
                1..=15 => 1,
                16..=31 => 2,
                32..=47 => 3,
                48..=63 => 4,
                _ => 5,
            }] += 1;
        }
        Footprint {
            slots,
            items,
            pages_by_free,
            objects: list(
                self.objects.list.len(),
                size_of::<Object>(),
                self.objects.list.committed_bytes(),
                &self.free_objects,
                self.free_objects.given_back_pages(),
            ),
            strings: list(
                self.strings.items.len(),
                size_of::<crate::text::StringData>(),
                self.strings.items.committed_bytes(),
                &self.strings.free,
                self.strings.free.given_back_pages(),
            ),
            cells: list(
                self.cells.list.len(),
                size_of::<crate::value::Value>(),
                self.cells.list.committed_bytes(),
                &self.free_cells,
                self.free_cells.given_back_pages(),
            ),
        }
    }
}
