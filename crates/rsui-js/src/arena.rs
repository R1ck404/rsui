//! A list that never moves. It sets aside a huge range of addresses up front, which costs no memory, and only asks
//! for real memory as it grows. So it never has to copy itself to somewhere bigger, which is what makes a growing
//! `Vec` need twice the memory at the worst moment, and the interpreter can keep a pointer to it for good.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_DECOMMIT: u32 = 0x4000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_NOACCESS: u32 = 1;
const PAGE_READWRITE: u32 = 4;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
    fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
}

/// Real memory is asked for in steps of this, or an eighth of what there is if that's more, up to [`AHEAD`].
const STEP: usize = 16 * 1024;
/// The most that's asked for ahead of what's needed. Memory that's asked for counts against the process whether it's
/// touched or not, and an eighth of the list of objects of `bench/long-list` (80 MB) was 10 MB of that, which a few more
/// objects at the end of a run decided whether it had. Asking for it a megabyte at a time is 80 calls to the system
/// over the whole of that list, and a page costs the same to zero whenever it's first written.
const AHEAD: usize = 1024 * 1024;

pub(crate) struct Arena<T> {
    base: *mut T,
    len: usize,
    /// How many items there's real memory for.
    committed: usize,
    /// How many items there are addresses for.
    reserved: usize,
    /// How far into its first page the list starts.
    offset: usize,
    owned: PhantomData<T>,
}

impl<T> Arena<T> {
    /// Where the first item is, which stays the same for as long as the list does.
    pub(crate) fn base_ptr(&self) -> *const T {
        self.base
    }

    /// Where the count of items is kept, for a sampler that reads it while this thread is stopped.
    pub(crate) fn len_ptr(&self) -> *const usize {
        &raw const self.len
    }

    /// A list with room for this many items at most. Nothing but addresses is used until something's added.
    pub(crate) fn with_limit(reserved: usize) -> Self {
        Self::with_limit_at(reserved, 0)
    }

    /// The same, starting this many bytes into its first page.
    ///
    /// A processor can take a read for one of a recent write if the two addresses have the same position in their
    /// pages, and then has to start over. A small program's instructions, registers and variables would all be near
    /// the start of their lists, so without this they can land on each other. It made a loop take 70% longer.
    pub(crate) fn with_limit_at(reserved: usize, offset: usize) -> Self {
        assert!(size_of::<T>() > 0 && align_of::<T>() <= STEP);
        assert!(offset % align_of::<T>() == 0);
        let bytes = reserved * size_of::<T>() + offset;
        // SAFETY: asking for a new range of addresses has no preconditions.
        let start =
            unsafe { VirtualAlloc(std::ptr::null_mut(), bytes, MEM_RESERVE, PAGE_NOACCESS) };
        assert!(!start.is_null(), "there are no addresses left to reserve");
        rsui_alloc::regions::note(start.cast(), std::any::type_name::<T>());
        Self {
            // SAFETY: that's inside what was just reserved.
            base: unsafe { start.cast::<u8>().add(offset).cast() },
            len: 0,
            committed: 0,
            reserved,
            offset,
            owned: PhantomData,
        }
    }

    /// How far into its first page the list starts.
    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    /// Where the memory of the list starts, on a page.
    pub(crate) fn page_start(&self) -> *mut u8 {
        self.start()
    }

    /// Where the memory of the list starts.
    fn start(&self) -> *mut u8 {
        // SAFETY: this undoes what `with_limit_at` added.
        unsafe { self.base.cast::<u8>().sub(self.offset) }
    }

    /// How much of the reserved range has real memory, from where it starts. It's never left partway through a step.
    pub(crate) fn committed_bytes(&self) -> usize {
        if self.committed == 0 {
            0
        } else {
            let end = self.committed * size_of::<T>() + self.offset;
            end.next_multiple_of(STEP)
                .min(self.reserved * size_of::<T>() + self.offset)
        }
    }

    /// Gets real memory for at least this many items, and says whether there was room for them.
    #[cold]
    #[inline(never)]
    fn commit(&mut self, wanted: usize) -> bool {
        if wanted > self.reserved {
            return false;
        }
        let have = self.committed_bytes();
        let need = wanted * size_of::<T>() + self.offset;
        let bytes = need
            .max(have + (have / 8).clamp(STEP, AHEAD))
            .next_multiple_of(STEP)
            .min(self.reserved * size_of::<T>() + self.offset);
        // SAFETY: the range is inside what `with_limit_at` reserved.
        let done = unsafe {
            VirtualAlloc(
                self.start().add(have).cast(),
                bytes - have,
                MEM_COMMIT,
                PAGE_READWRITE,
            )
        };
        assert!(!done.is_null(), "out of memory");
        self.committed = (bytes - self.offset) / size_of::<T>();
        true
    }

    #[inline(always)]
    pub(crate) fn as_ptr(&self) -> *const T {
        self.base
    }

    #[inline(always)]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut T {
        self.base
    }

    /// How many items there's real memory for.
    #[inline(always)]
    pub(crate) fn capacity(&self) -> usize {
        self.committed
    }

    #[inline(always)]
    pub(crate) fn push(&mut self, value: T) {
        if self.len == self.committed {
            assert!(self.commit(self.len + 1), "this list is full");
        }
        // SAFETY: there's real memory for this place, and nothing in it.
        unsafe { self.base.add(self.len).write(value) };
        self.len += 1;
    }

    #[inline]
    pub(crate) fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        // SAFETY: this place held an item, which the shorter list no longer counts.
        Some(unsafe { self.base.add(self.len).read() })
    }

    /// Cuts the list down to this long, dropping what's cut off.
    pub(crate) fn truncate(&mut self, length: usize) {
        if length >= self.len {
            return;
        }
        let cut = self.len - length;
        self.len = length;
        // SAFETY: these places held items, which the shorter list no longer counts, so each is dropped just once.
        unsafe {
            std::ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(
                self.base.add(length),
                cut,
            ));
        }
    }

    pub(crate) fn extend_from_slice(&mut self, values: &[T])
    where
        T: Copy,
    {
        let wanted = self.len + values.len();
        if wanted > self.committed {
            assert!(self.commit(wanted), "this list is full");
        }
        // SAFETY: there's real memory for these places, and the slice can't be part of them.
        unsafe {
            std::ptr::copy_nonoverlapping(values.as_ptr(), self.base.add(self.len), values.len());
        }
        self.len = wanted;
    }

    /// Hands back the real memory of whatever's far past the end, apart from `slack` bytes past it, which are kept for
    /// what's about to be added. It reads as zeros if it's ever wanted again.
    pub(crate) fn release_unused(&mut self, slack: usize) {
        let keep = (self.len * size_of::<T>() + self.offset + slack.max(STEP))
            .next_multiple_of(STEP)
            .min(self.reserved * size_of::<T>() + self.offset);
        let have = self.committed_bytes();
        // Not worth the trouble for a little, and asking again soon would cost more than it saved.
        if have < keep + 4 * STEP || have < keep + keep / 2 {
            return;
        }
        // SAFETY: everything in this range is past the end of the list, and was committed.
        unsafe {
            VirtualFree(self.start().add(keep).cast(), have - keep, MEM_DECOMMIT);
        }
        self.committed = (keep - self.offset) / size_of::<T>();
    }

    /// Only for the interpreter, which has just written the place after the end, or finished with the last one.
    ///
    /// # Safety
    /// The places up to `length` have to hold items, with memory behind them, and any cut off need no dropping.
    #[inline(always)]
    pub(crate) unsafe fn set_len(&mut self, length: usize) {
        debug_assert!(length <= self.committed);
        self.len = length;
    }
}

#[cfg(feature = "jit")]
impl<T> Arena<T> {
    /// Where the length is, which the JIT's code reads and changes itself, as the interpreter does with `set_len`.
    pub(crate) const LEN_OFFSET: usize = std::mem::offset_of!(Self, len);
    /// Where the count of places with real memory is ([`Arena::capacity`]).
    pub(crate) const COMMITTED_OFFSET: usize = std::mem::offset_of!(Self, committed);
}

impl crate::arena::Arena<crate::value::Value> {
    /// Makes the list at least this long, and says whether it could. New places hold zeros, which is the number 0.
    pub(crate) fn grow_to(&mut self, length: usize) -> bool {
        if length > self.committed && !self.commit(length) {
            return false;
        }
        // Everything up to `committed` is either written already or zeros from the system.
        self.len = self.len.max(self.committed);
        true
    }
}

impl<T> Deref for Arena<T> {
    type Target = [T];

    #[inline(always)]
    fn deref(&self) -> &[T] {
        // SAFETY: the first `len` places hold items, and the base is aligned and never null.
        unsafe { std::slice::from_raw_parts(self.base, self.len) }
    }
}

impl<T> DerefMut for Arena<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut [T] {
        // SAFETY: as for `deref`, and having `&mut self` means nothing else is looking at them.
        unsafe { std::slice::from_raw_parts_mut(self.base, self.len) }
    }
}

impl<'a, T> IntoIterator for &'a Arena<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self::with_limit(1 << 29)
    }
}

impl<T> std::fmt::Debug for Arena<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Arena of {}", self.len)
    }
}

impl<T> Drop for Arena<T> {
    fn drop(&mut self) {
        // SAFETY: the items are all there, and these are the addresses `with_limit` got, whole.
        unsafe {
            if std::mem::needs_drop::<T>() {
                std::ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(self.base, self.len));
            }
            VirtualFree(self.start().cast(), 0, MEM_RELEASE);
        }
        rsui_alloc::regions::forget(self.start());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_stay_where_they_are_as_the_list_grows() {
        let mut list = Arena::<u64>::with_limit(1 << 24);
        list.push(7);
        let first = list.as_ptr();
        for number in 0..200_000 {
            list.push(number);
        }
        assert_eq!(list.as_ptr(), first);
        assert_eq!(list[0], 7);
        assert_eq!(list[200_000], 199_999);
    }

    #[test]
    fn memory_handed_back_can_be_used_again_and_reads_as_zeros() {
        let mut list = Arena::<u64>::with_limit(1 << 24);
        for number in 0..500_000 {
            list.push(number + 1);
        }
        let before = list.capacity();
        list.truncate(10);
        list.release_unused(0);
        assert!(list.capacity() < before);
        assert_eq!(list[9], 10);
        let released_from = list.capacity();
        assert!(list.commit(released_from + 1));
        // SAFETY: that place has just been given memory again, and any bit pattern is a `u64`.
        assert_eq!(unsafe { *list.as_ptr().add(released_from) }, 0);
        for number in 0..500_000 {
            list.push(number);
        }
        assert_eq!(list[500_009], 499_999);
    }

    #[test]
    fn dropping_the_list_drops_what_is_in_it() {
        let counted = std::rc::Rc::new(());
        let mut list = Arena::<std::rc::Rc<()>>::with_limit(1 << 16);
        for _ in 0..100 {
            list.push(counted.clone());
        }
        assert_eq!(std::rc::Rc::strong_count(&counted), 101);
        list.truncate(50);
        assert_eq!(std::rc::Rc::strong_count(&counted), 51);
        drop(list);
        assert_eq!(std::rc::Rc::strong_count(&counted), 1);
    }

    #[test]
    fn a_full_list_says_so_rather_than_going_past_its_end() {
        let mut stack = Arena::<crate::value::Value>::with_limit(1 << 16);
        assert!(stack.grow_to(1000));
        assert!(stack.len() >= 1000);
        assert!(!stack.grow_to((1 << 16) + 1));
    }
}
