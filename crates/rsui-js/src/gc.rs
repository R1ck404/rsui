//! The garbage collector. It marks everything that can be reached and frees the rest. It only ever runs at a safe
//! point in the interpreter, and only after enough has been made since the last time, so an idle program has no
//! collector running at all.
//!
//! It has two generations, and nothing moves. An object becomes old the third time in a row a small collection keeps it
//! (the second, with [`age`] at 2), and a string or a cell the second time, and their marks stay set afterwards. Until
//! then what's young and was kept is unmarked again after each small collection, with bits of its own to say how often.
//! Most collections are small: they start from
//! the roots, stop at anything already marked, and so only go through what's young, and free only what's young and
//! wasn't reached. A young thing can also be reached from an old one, so every change to an old object or cell goes
//! through a write barrier first, which remembers it, and a small collection looks through what's remembered as well.
//! After it, whatever old still refers to a young object (one kept for the first time) is remembered again. Once enough
//! has become old since the last full collection, as it may have died since, the next one is full.
//!
//! The barrier for objects is an `OLD` bit in each object's flags, which is cleared when the object is remembered, so
//! it costs one test of a byte the change is about to touch anyway. Changes through `Vm::objects[..]` go through it by
//! themselves ([`Objects`]' `IndexMut`). The interpreter, which changes objects through a pointer, sends an old object
//! the slow way, through `objects[..]`, or calls [`Objects::write_barrier`]. A cell is old if it's marked, and
//! remembering it clears its mark; changes through `Vm::cells[..]` remember it by themselves, and the interpreter calls
//! [`Cells::write_barrier`].
//!
//! What a `WeakMap`, `WeakSet`, `WeakRef` or `FinalizationRegistry` holds weakly isn't marked through it. Once nothing
//! more can be reached, a `WeakMap` entry whose key was reached has its value marked, as the key could be looked up with
//! (an ephemeron), which can reach more keys, until nothing new is. Then whatever was held weakly and wasn't reached is
//! let go of, before sweeping: entries go, a `WeakRef` points at undefined, and a registry's callback is due. A small
//! collection does the same with what it reaches and what's remembered: everything old counts as reached then, and an
//! old one of these is remembered while it holds something young, as anything old is, so it's looked at again until
//! what it holds is old or gone. What a `WeakRef` handed out stays alive until nothing's running ([`Vm::kept`]).
//!
//! Shapes are freed too, by full collections only. One marks the shape of each object it traces, which marks the shapes
//! it comes from and its prototype: that's how a prototype is reached, as part of the objects that have it. What the
//! engine makes objects with by itself is kept as well ([`Vm::held_shapes`]). The other shapes are freed after
//! sweeping, and taken out of what they came from, so a parent's transitions are weak, and whatever remembers shapes by
//! their numbers (the places in the code, what reads remember, and the few kept in `Vm`) is cleared of them, as their
//! places are used again. A small collection treats every shape as in use, and every prototype they have as a root.
//!
//! So are atoms, the names of properties and symbols, by full collections when nothing but JavaScript is running (see
//! [`Vm::nested_runs`]). A symbol reached is in use, and so is each name of a shape marked; what the engine names
//! itself, what the code names, `Symbol.for`'s symbols and what Rust asked for by name are pinned and always stay (see
//! [`crate::text::Atoms`]). The rest are freed, with the strings only they kept, and a `WeakMap`, `WeakSet`,
//! `WeakRef` or `FinalizationRegistry` lets go of a symbol that's gone. Whatever remembers an atom by its number is
//! cleared of the freed ones before their places are used again: the strings that knew them as theirs, and what reads
//! remember.
//!
//! `RSUI_GC_STRESS=1` collects at every safe point, `RSUI_GC_VERIFY=1` checks around each small collection that nothing
//! old refers to something young without being remembered, and that nothing held weakly has been freed, and
//! `RSUI_GC_LOG=1` writes a line about each collection. `RSUI_GC_DIED_OLD=1` writes what each small collection made old
//! and what each full one found dead of what small ones had made old since the last, by what it is, which is what a full
//! collection is for: as little of that as there can be is what keeps full collections few. Each line of
//! `RSUI_GC_LOG` ends with how much was made since the collection before.
//!
//! Two experiments are off by default (see "Full collections that leave the young young" in `DESIGN.md`):
//! `RSUI_GC_KEEP_YOUNG=1` (or `2`, only when what's young outweighs the old budget) has a full collection age what's
//! young as a small one would rather than make all it keeps old, and `RSUI_GC_STRING_AGE=3` has strings age as
//! objects do.

use crate::arena::Arena;
use crate::object::*;
use crate::text::Atom;
use crate::value::Value;
use crate::vm::{Job, Vm};
use std::ops::{Deref, Index, IndexMut};

#[path = "footprint.rs"]
pub(crate) mod footprint;
#[path = "giveback.rs"]
pub(crate) mod giveback;
#[path = "exact.rs"]
pub(crate) mod exact;

const MIN_THRESHOLD: usize = 128 * 1024;
/// The least that's made between small collections. With what's young kept for a third small collection (see
/// [`age`]), 128 KB had a program with a small heap that makes big strings (the `buffer` bench in `benches/js`) collect in full
/// half again as often, as each collection made a string or two old, which is what makes a full one due when what's old
/// is that small.
const MIN_YOUNG: usize = 256 * 1024;

/// Set in an object's flags while it's old and hasn't been changed since it was last looked at. See the top of this
/// file. It's the top bit, which the other flags in `object.rs` leave free.
pub(crate) const OLD: u8 = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stress {
    Off,
    /// Collect at every safe point, mostly small collections with a full one every so often.
    On,
    /// Collect at every safe point, always fully.
    Full,
}

/// What `RSUI_GC_STRESS` asks for: `1` collects at every safe point at which anything has been made, and `full` does
/// that with only full collections. It's for finding something that's held without the collector knowing, or a change
/// to an old object that didn't go through the barrier: that's freed at once, rather than once in a while.
fn stress() -> Stress {
    use std::sync::atomic::{AtomicU8, Ordering};
    static STRESS: AtomicU8 = AtomicU8::new(0);
    match STRESS.load(Ordering::Relaxed) {
        0 => {
            let mode = match std::env::var_os("RSUI_GC_STRESS") {
                Some(value) if value == "full" => Stress::Full,
                Some(value) if value != "0" => Stress::On,
                _ => Stress::Off,
            };
            STRESS.store(mode as u8 + 1, Ordering::Relaxed);
            mode
        }
        1 => Stress::Off,
        2 => Stress::On,
        _ => Stress::Full,
    }
}

/// How many small collections keep a young object before it's old: 3, or what `RSUI_GC_AGE` says (2 or 3).
///
/// What a frame of an app or a chart makes lives about as long as the frame, which is several small collections. At 2,
/// much of it was kept by two of them, became old and died old, and only a full collection found it: a chart.js line
/// update had about three full collections, which mark everything, to sixteen small ones.
fn age() -> u8 {
    use std::sync::atomic::{AtomicU8, Ordering};
    static AGE: AtomicU8 = AtomicU8::new(0);
    match AGE.load(Ordering::Relaxed) {
        0 => {
            let age = match std::env::var_os("RSUI_GC_AGE") {
                Some(value) if value == "2" => 2,
                _ => 3,
            };
            AGE.store(age, Ordering::Relaxed);
            age
        }
        age => age,
    }
}

/// How much may be made between small collections, in eighths of what's alive: 3, or what `RSUI_GC_YOUNG` says (1-8).
fn young_eighths() -> usize {
    use std::sync::atomic::{AtomicU8, Ordering};
    static EIGHTHS: AtomicU8 = AtomicU8::new(0);
    match EIGHTHS.load(Ordering::Relaxed) {
        0 => {
            let eighths = std::env::var("RSUI_GC_YOUNG").ok().and_then(|value| value.parse::<u8>().ok()).filter(|n| (1..=8).contains(n)).unwrap_or(3);
            EIGHTHS.store(eighths, Ordering::Relaxed);
            usize::from(eighths)
        }
        eighths => usize::from(eighths),
    }
}

/// The most that may be made between small collections, whatever is alive: 12 MB, or what `RSUI_GC_YOUNG_CAP` says, in
/// megabytes, with `0` for no limit.
///
/// Three-eighths of what's alive is room the list of objects keeps between collections, and it only binds on a big heap:
/// the long list (120 MB alive) made 45 MB between small collections, and ended 10 MB lower with this (148 -> 137 MB
/// private at the end, 164 -> 148 at its peak), Mantine's page 4 MB lower, with no more than 1% more time collecting. An
/// 8 MB cap took 3 MB more off each, but made shadcn's page and the long list 2% slower, from the full collections
/// that more of what's made becoming old brings on. Below 32 MB alive, as for js-bench and the explorer, it's as before.
const YOUNG_CAP_MB: usize = 12;

fn young_cap() -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CAP: AtomicUsize = AtomicUsize::new(0);
    match CAP.load(Ordering::Relaxed) {
        0 => {
            let mb = std::env::var("RSUI_GC_YOUNG_CAP").ok().and_then(|value| value.parse::<usize>().ok()).unwrap_or(YOUNG_CAP_MB);
            let cap = if mb == 0 { usize::MAX } else { mb << 20 };
            CAP.store(cap, Ordering::Relaxed);
            cap
        }
        cap => cap,
    }
}

/// Whether `RSUI_GC_VERIFY` asks for the heap to be checked around every small collection: that each old object or cell
/// that refers to something young was remembered. It finds a missing write barrier where it's missing, rather than
/// when what it lost is used, and is slow. It's meant to be used with `RSUI_GC_STRESS`.
fn verifying() -> bool {
    use std::sync::atomic::{AtomicU8, Ordering};
    static VERIFY: AtomicU8 = AtomicU8::new(0);
    match VERIFY.load(Ordering::Relaxed) {
        0 => {
            let on = std::env::var_os("RSUI_GC_VERIFY").is_some_and(|value| value != "0");
            VERIFY.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        known => known == 2,
    }
}

/// Whether `RSUI_GC_LOG` asks for a line on stderr about each collection: what kind it was, how long it took, and how
/// much it kept.
fn logging() -> bool {
    use std::sync::atomic::{AtomicU8, Ordering};
    static LOG: AtomicU8 = AtomicU8::new(0);
    match LOG.load(Ordering::Relaxed) {
        0 => {
            let on = std::env::var_os("RSUI_GC_LOG").is_some_and(|value| value != "0");
            LOG.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        known => known == 2,
    }
}

/// What `RSUI_GC_KEEP_YOUNG` asks of a full collection that's due: `0` (the default) makes everything it keeps old,
/// `1` leaves what was young young, aged as a small collection would have aged it, and `2` does that only when the
/// last small collection kept more young than the old budget (see [`Generations::keeps_young`]). It's an
/// experiment, off by default: see "Full collections that leave the young young" in `DESIGN.md`.
fn keep_young_mode() -> u8 {
    use std::sync::atomic::{AtomicU8, Ordering};
    static MODE: AtomicU8 = AtomicU8::new(u8::MAX);
    match MODE.load(Ordering::Relaxed) {
        u8::MAX => {
            let mode = match std::env::var("RSUI_GC_KEEP_YOUNG").ok().as_deref() {
                Some("1") => 1,
                Some("2") => 2,
                _ => 0,
            };
            MODE.store(mode, Ordering::Relaxed);
            mode
        }
        mode => mode,
    }
}

/// How many small collections keep a young string before it's old: 2, or with `RSUI_GC_STRING_AGE=3`, what [`age`]
/// says, as objects. Also an experiment, off by default (see `DESIGN.md`).
fn string_age() -> u8 {
    use std::sync::atomic::{AtomicU8, Ordering};
    static AGE: AtomicU8 = AtomicU8::new(0);
    match AGE.load(Ordering::Relaxed) {
        0 => {
            let age = match std::env::var("RSUI_GC_STRING_AGE").ok().as_deref() {
                Some("3") => age(),
                _ => 2,
            };
            AGE.store(age, Ordering::Relaxed);
            age
        }
        age => age,
    }
}

/// Whether `RSUI_GC_DIED_OLD` asks for each full collection to write what small collections made old that it found
/// dead, by what it is.
fn died_old_logging() -> bool {
    use std::sync::atomic::{AtomicU8, Ordering};
    static ON: AtomicU8 = AtomicU8::new(0);
    match ON.load(Ordering::Relaxed) {
        0 => {
            let on = std::env::var_os("RSUI_GC_DIED_OLD").is_some_and(|value| value != "0");
            ON.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        known => known == 2,
    }
}

/// How much a new engine makes before it first collects.
pub(crate) fn first_threshold() -> usize {
    if stress() == Stress::Off {
        MIN_THRESHOLD
    } else {
        0
    }
}

#[inline]
fn set_mark(marks: &mut [u64], index: usize) -> bool {
    let word = &mut marks[index / 64];
    let bit = 1u64 << (index % 64);
    let was_set = *word & bit != 0;
    *word |= bit;
    was_set
}

/// Marks an atom, unless it's a whole number, which is held in the atom and has nothing to free.
#[inline]
fn mark_atom(marks: &mut [u64], atom: Atom) {
    if atom.held_index().is_none()
        && let Some(word) = marks.get_mut(atom.0 as usize / 64)
    {
        *word |= 1u64 << (atom.0 % 64);
    }
}

#[inline(always)]
fn is_marked(marks: &[u64], index: usize) -> bool {
    marks
        .get(index / 64)
        .is_some_and(|word| word >> (index % 64) & 1 != 0)
}

fn clear_marks(marks: &mut Vec<u64>, count: usize) {
    marks.clear();
    marks.resize(count / 64 + 1, 0);
}

/// Makes room for the marks of places added since the last collection, which aren't marked, as they're young.
fn extend_marks(marks: &mut Vec<u64>, count: usize) {
    if marks.len() < count / 64 + 1 {
        marks.resize(count / 64 + 1, 0);
    }
}

/// Unmarks every place from `length` on, as the list has been cut short there. Something past the end can be marked
/// by a handle to something freed long ago (see [`Vm::collect_garbage`]), and a place added there later mustn't look
/// old.
fn trim_marks(marks: &mut Vec<u64>, length: usize) {
    marks.truncate(length.div_ceil(64));
    if length % 64 != 0
        && let Some(last) = marks.last_mut()
    {
        *last &= (1u64 << (length % 64)) - 1;
    }
}

/// The list of objects, which remembers an old object when it's about to be changed. See the top of this file.
pub(crate) struct Objects {
    list: Arena<Object>,
    /// The old objects that have been changed since the last collection, which have `OLD` cleared.
    remembered: Vec<u32>,
    /// The objects a result kept for a render was worked out from, or is: see [`Watch`].
    pub(crate) watch: Watch,
}

/// The objects that what's kept for the next render was worked out from, and what it is, so that a change to any of them
/// is known (see `keeping.rs`), at no cost to anything that changes something else.
///
/// A change to an object goes through the write barrier when the object has `OLD`, which an old object has until it's
/// changed. A young one that's watched is given `OLD` too, as if it were old, and is called faked: changing it then goes
/// the slow way once, which sees it's watched. A faked object is never remembered, as it's young, and the collector never
/// sees one: they're put back as they were before each collection ([`Watch::unfake`]), and given `OLD` again after it
/// ([`Watch::refake`]). An old object that a collection remembered again, as it holds something young, has `OLD` cleared,
/// and is faked in the same way: changing it doesn't remember it again, as it's remembered already.
///
/// A change to a watched object stops it being watched and counts one more change. That's all it does: what's kept is
/// only given again while there's been no change since it was kept.
#[derive(Default)]
pub(crate) struct Watch {
    /// Whether anything is watched, so the slow way of a change only looks further when something is.
    pub(crate) any: bool,
    watched: Vec<u64>,
    faked: Vec<u64>,
    /// How many changes there have been to what was watched.
    pub(crate) changes: u32,
    /// How many objects are watched.
    pub(crate) count: usize,
    /// The last watched object to change, for saying why what was kept was forgotten.
    pub(crate) last_changed: usize,
}

impl Watch {
    pub(crate) fn is_watched(&self, handle: usize) -> bool {
        is_marked(&self.watched, handle)
    }

    /// Watches `object`, at `handle`, if it isn't already.
    pub(crate) fn watch(&mut self, object: &mut Object, handle: usize) {
        extend_marks(&mut self.watched, handle + 1);
        if set_mark(&mut self.watched, handle) {
            return;
        }
        self.any = true;
        self.count += 1;
        if object.flags & OLD == 0 {
            extend_marks(&mut self.faked, handle + 1);
            set_mark(&mut self.faked, handle);
            object.flags |= OLD;
        }
    }

    /// `object`, at `handle`, which is watched, is about to be changed: it's no longer watched, and if it was faked, it has
    /// `OLD` taken off again, and gives true, as it doesn't need remembering.
    #[cold]
    fn changing(&mut self, object: &mut Object, handle: usize) -> bool {
        self.watched[handle / 64] &= !(1u64 << (handle % 64));
        self.count -= 1;
        self.changes = self.changes.wrapping_add(1);
        self.last_changed = handle;
        if is_marked(&self.faked, handle) {
            self.faked[handle / 64] &= !(1u64 << (handle % 64));
            object.flags &= !OLD;
            return true;
        }
        false
    }

    /// Takes `OLD` off every faked object, so the collector sees each as it is, which comes before every collection. Which
    /// they were stays marked, for [`Watch::refake`] after a small collection.
    fn unfake(&mut self, objects: &mut [Object]) {
        for (word_at, word) in self.faked.iter().enumerate() {
            let mut bits = *word;
            while bits != 0 {
                let handle = word_at * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if let Some(object) = objects.get_mut(handle) {
                    object.flags &= !OLD;
                }
            }
        }
    }

    /// After a collection: what was watched and has been freed isn't any more, and what's watched without `OLD` is given it,
    /// as a young object kept, or an old one remembered again, has it cleared.
    ///
    /// After a small one, that's only what was faked before it: an old object that's watched is neither freed nor changed by
    /// one, nor is one remembered, as changing it stops it being watched. What was faked and is old now stops being faked.
    /// Going through everything watched after each small collection was 2.5 ms of each of Mantine's page's, which watches
    /// what the results its components keep were worked out from (see `keeping.rs`).
    fn refake(&mut self, objects: &mut [Object], full: bool) {
        if !self.any {
            return;
        }
        if !full {
            for word_at in 0..self.faked.len() {
                let mut bits = self.faked[word_at];
                while bits != 0 {
                    let handle = word_at * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    match objects.get_mut(handle) {
                        Some(object) if !matches!(object.kind, Kind::Free) => {
                            if object.flags & OLD == 0 {
                                object.flags |= OLD;
                            } else {
                                self.faked[word_at] &= !(1u64 << (handle % 64));
                            }
                        }
                        _ => {
                            self.faked[word_at] &= !(1u64 << (handle % 64));
                            if is_marked(&self.watched, handle) {
                                self.watched[word_at] &= !(1u64 << (handle % 64));
                                self.count -= 1;
                            }
                        }
                    }
                }
            }
            return;
        }
        self.faked.iter_mut().for_each(|word| *word = 0);
        extend_marks(&mut self.faked, self.watched.len() * 64);
        for word_at in 0..self.watched.len() {
            let mut bits = self.watched[word_at];
            while bits != 0 {
                let handle = word_at * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                match objects.get_mut(handle) {
                    Some(object) if !matches!(object.kind, Kind::Free) => {
                        if object.flags & OLD == 0 {
                            object.flags |= OLD;
                            set_mark(&mut self.faked, handle);
                        }
                    }
                    _ => {
                        self.watched[word_at] &= !(1u64 << (handle % 64));
                        self.count -= 1;
                    }
                }
            }
        }
    }

    /// Stops watching everything, with each faked object put back as it was.
    pub(crate) fn clear(&mut self, objects: &mut [Object]) {
        self.unfake(objects);
        self.watched.clear();
        self.faked.clear();
        self.count = 0;
        self.any = false;
    }
}

impl Objects {
    pub(crate) fn new(list: Arena<Object>) -> Self {
        Self {
            list,
            remembered: Vec::new(),
            watch: Watch::default(),
        }
    }

    /// Stops watching everything: see [`Watch::clear`].
    pub(crate) fn stop_watching(&mut self) {
        self.watch.clear(&mut self.list);
    }

    /// Adds `flags` to the object at `handle` without going through the write barrier: a flag that says something of the
    /// object, as `NOT_HANDED_ON` does, isn't a change to what it holds, which the collector and what's watched look at.
    pub(crate) fn add_flags(&mut self, handle: usize, flags: u8) {
        self.list[handle].flags |= flags;
    }

    /// The object at `handle`, to change only what the engine keeps in it for itself and the program can't see, as where
    /// calls of a closure start: that doesn't go through the write barrier, so nothing can be stored in it that refers to
    /// anything, and it isn't taken for a change by what's watched.
    pub(crate) fn for_the_engine(&mut self, handle: usize) -> &mut Object {
        &mut self.list[handle]
    }

    /// Watches the object at `handle`: see [`Watch`].
    pub(crate) fn watch(&mut self, handle: usize) {
        let object = &mut self.list[handle];
        self.watch.watch(object, handle);
    }

    #[inline(always)]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut Object {
        self.list.as_mut_ptr()
    }

    #[inline(always)]
    pub(crate) fn push(&mut self, object: Object) {
        self.list.push(object);
    }

    /// Has to come before anything new is stored in `object`, the one at `handle`, other than through `objects[..]`,
    /// which does this itself.
    #[inline(always)]
    pub(crate) fn write_barrier(&mut self, object: &mut Object, handle: usize) {
        if object.flags & OLD != 0 {
            changing(&mut self.remembered, &mut self.watch, object, handle);
        }
    }
}

/// An object with `OLD` that's about to be changed: it's remembered, unless it's watched and was only faked (see [`Watch`]).
#[cold]
#[inline(never)]
fn changing(remembered: &mut Vec<u32>, watch: &mut Watch, object: &mut Object, handle: usize) {
    if watch.any && watch.is_watched(handle) && watch.changing(object, handle) {
        return;
    }
    remember(remembered, object, handle);
}

#[cold]
#[inline(never)]
fn remember(remembered: &mut Vec<u32>, object: &mut Object, handle: usize) {
    object.flags &= !OLD;
    remembered.push(handle as u32);
}

impl Deref for Objects {
    type Target = [Object];

    #[inline(always)]
    fn deref(&self) -> &[Object] {
        &self.list
    }
}

impl Index<usize> for Objects {
    type Output = Object;

    #[inline(always)]
    fn index(&self, index: usize) -> &Object {
        &self.list[index]
    }
}

impl IndexMut<usize> for Objects {
    /// An object that's about to be changed, which is remembered if it's old.
    #[inline(always)]
    fn index_mut(&mut self, index: usize) -> &mut Object {
        let object = &mut self.list[index];
        if object.flags & OLD != 0 {
            changing(&mut self.remembered, &mut self.watch, object, index);
        }
        object
    }
}

impl<'a> IntoIterator for &'a Objects {
    type Item = &'a Object;
    type IntoIter = std::slice::Iter<'a, Object>;

    fn into_iter(self) -> Self::IntoIter {
        self.list.iter()
    }
}

/// The list of cells, which remembers an old cell when it's about to be changed. A cell is old if it's marked, so the
/// marks stay here between collections.
pub(crate) struct Cells {
    list: Arena<Value>,
    marks: Vec<u64>,
    /// The old cells that have been changed since the last collection, which are unmarked until it.
    remembered: Vec<u32>,
}

impl Cells {
    pub(crate) fn new(list: Arena<Value>) -> Self {
        Self {
            list,
            marks: Vec::new(),
            remembered: Vec::new(),
        }
    }

    #[inline(always)]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut Value {
        self.list.as_mut_ptr()
    }

    #[inline(always)]
    pub(crate) fn push(&mut self, value: Value) {
        self.list.push(value);
    }

    /// Has to come before `value` is stored in the cell at `index` other than through `cells[..]`, which does this
    /// itself.
    #[inline(always)]
    pub(crate) fn write_barrier(&mut self, index: usize, value: Value) {
        // Only the test of the value goes inline, as the interpreter's loop is short of registers.
        if !value.is_number() {
            self.remember_if_old(index);
        }
    }

    #[inline(never)]
    fn remember_if_old(&mut self, index: usize) {
        if is_marked(&self.marks, index) {
            self.remember(index);
        }
    }

    #[cold]
    #[inline(never)]
    fn remember(&mut self, index: usize) {
        self.marks[index / 64] &= !(1u64 << (index % 64));
        self.remembered.push(index as u32);
    }
}

impl Deref for Cells {
    type Target = [Value];

    #[inline(always)]
    fn deref(&self) -> &[Value] {
        &self.list
    }
}

impl Index<usize> for Cells {
    type Output = Value;

    #[inline(always)]
    fn index(&self, index: usize) -> &Value {
        &self.list[index]
    }
}

impl IndexMut<usize> for Cells {
    #[inline(always)]
    fn index_mut(&mut self, index: usize) -> &mut Value {
        if is_marked(&self.marks, index) {
            self.remember(index);
        }
        &mut self.list[index]
    }
}

/// What the collector keeps between collections to decide what the next one does.
#[derive(Default)]
pub(crate) struct Generations {
    /// What was alive after the last full collection.
    full_live: usize,
    /// What has become old since then, some of which may have died.
    promoted: usize,
    small_since_full: u32,
    /// Whether the last small collection kept most of what had been made since the one before, as happens while a
    /// program builds something big, or while what it's working on lives a little longer than small collections are
    /// apart. See [`Generations::full_due`].
    mostly_kept: bool,
    /// The young objects kept once, one bit each, which become old if they're kept again (or, with [`age`] at 3, are
    /// kept twice, and then old the time after that).
    surviving: Vec<u64>,
    /// The young objects kept twice, with [`age`] at 3, which become old if they're kept again.
    surviving_twice: Vec<u64>,
    spare_twice: Vec<u64>,
    /// What those take up.
    surviving_bytes: usize,
    /// What was old before the small collection that's running: everything marked when it started.
    old_before: Vec<u64>,
    /// The shapes whose prototypes were young after the last collection, which a small collection keeps (with the shapes
    /// made since, in `Shapes::touched`) until they're old.
    young_protos: Vec<u32>,
    seen_shapes: Vec<u64>,
    /// The marks of strings before a small collection, to tell which ones it kept.
    strings_before: Vec<u64>,
    /// Lists a collection uses, kept for the next so as not to make them each time.
    new_cells: Vec<u32>,
    spare_handles: Vec<u32>,
    spare_marks: Vec<u64>,
    weak: Vec<u32>,
    waiting: Waiting,
    /// The shapes a full collection found in use, and the ones it freed.
    shape_marks: Vec<u64>,
    shapes_freed: Vec<u64>,
    /// The atoms a full collection found in use, and the ones it freed.
    atom_marks: Vec<u64>,
    atoms_freed: Vec<u64>,
    /// What cells and strings aging needs, with `remembered_bytes`, which was here before, kept in a box. `Generations`
    /// is a field of `Vm`, and with these 224 bytes in it js-bench's interpreter ran 1.7% slower by geomean (prop_mono
    /// 8%, string_ops 7%, int_loop and try_catch 5%), none of which collect. With the box in the place of one 8-byte
    /// field, `Generations`, and so `Vm`, is laid out as before (bulk.rs's state is in `NumberStrings` for that too).
    more: Box<Aging>,
}

/// The part of [`Generations`] kept in a box, so that `Vm` is laid out as it was before strings and cells aged.
#[derive(Default)]
pub(crate) struct Aging {
    /// What was young and alive after it, which only a full collection that leaves the young young has.
    full_young: usize,
    /// The cells the last small collection kept for the first time, which are still young, one bit each: the next one
    /// to keep them makes them old. A cell used to be old as soon as a collection kept it. Then the variables of a
    /// function that was running when one came along were old, and once it had returned they still held what they
    /// were last given, which a small collection can't tell is garbage: an old cell that refers to something young is
    /// remembered until that's old, which takes three, so what it held became old and died old. The page's tree that
    /// `pageTree` in web.js makes on every key, half a megabyte, did that at nearly every small collection while
    /// typing in shadcn's page, and was most of what full collections found dead.
    cells_surviving: Vec<u64>,
    cells_spare: Vec<u64>,
    /// The cells that were old when the small collection that's running started.
    cells_old_before: Vec<u64>,
    /// The strings kept once by the last small collection, which are still young, as cells are.
    strings_surviving: Vec<u64>,
    strings_spare: Vec<u64>,
    /// Of those, the ones kept twice, with [`age`] at 3, which become old if they're kept again.
    strings_twice: Vec<u64>,
    strings_twice_spare: Vec<u64>,
    /// What the last collection looked through in old objects that were remembered, as the values it found there.
    remembered_bytes: usize,
    /// What was made between two frames, the most of the last few (an eighth less each time, unless what was made is more), what had
    /// been made since the last collection when that was last asked, and how many collections there had been then
    /// (see [`Vm::due_soon`]).
    between_frames: usize,
    made_between_frames: usize,
    collections_between_frames: u32,
    /// What the last full collection kept that was young before it, and what small collections have made old since.
    young_at_full: usize,
    promoted_since_full: usize,
    /// How long the last full collection took, in microseconds.
    last_full_micros: u32,
    /// With `RSUI_GC_DIED_OLD`, the objects that were old after the last full collection, and the ones old before
    /// this one, to tell what small collections made old that died before it.
    died_at_full: Vec<u64>,
    died_before: Vec<u64>,
}

impl Generations {
    /// How much more than what was alive after the last full collection can have become old since, as some of it may
    /// be garbage by now: three-eighths as much again, or `MIN_THRESHOLD`. Before there were generations this was a
    /// half, with nothing old left over as garbage; a half here kept the peak a fifth higher on programs that keep a
    /// lot for a little while, such as parsing JSON, and three-eighths keeps it about where it was.
    fn old_budget(&self) -> usize {
        // What a full collection left young was alive too, and would have counted had it been made old.
        ((self.full_live + self.more.full_young) / 8 * 3).max(MIN_THRESHOLD)
    }

    /// How much can be made between two small collections, before what's young and was kept is taken out: as much
    /// again as three-eighths of what's alive. What's young and was kept counts as alive here, as it is, and a full
    /// collection would have counted it. Leaving it out had a program whose young objects mostly lived through one
    /// small collection collect four times as often, as what could be made shrank to a quarter of the budget. It's
    /// [`MIN_YOUNG`] at least.
    fn budget(&self) -> usize {
        ((self.full_live + self.surviving_bytes) / 8 * young_eighths()).min(young_cap()).max(MIN_YOUNG)
    }

    /// The least that's made between two collections, a quarter of the budget.
    fn least(&self) -> usize {
        self.budget() / 4
    }

    /// Whether the next collection is full: once three-quarters of the old budget has become old since the last one,
    /// or a quarter of it has and the last small collection kept most of what was made. That's when it's likely to
    /// find old garbage, as what's being worked on lives a little longer than small collections are apart and goes old
    /// before it dies (rendering a tree of elements each frame, or parsing JSON over and over).
    ///
    /// A small collection keeping most of what was made used to make the next one full by itself, which re-marked the
    /// whole heap every other collection while a program built something that died young: a full collection then
    /// frees no more than a small one, as nothing became old that could be garbage. A program awaiting promises one at
    /// a time ran 9% faster without those, with a third less time collecting and a sixth less at its peak.
    fn full_due(&self) -> bool {
        match stress() {
            Stress::Full => true,
            Stress::On => self.small_since_full >= 15,
            Stress::Off => {
                let old = self.old_budget();
                self.promoted * 4 > old * 3 || (self.mostly_kept && self.promoted * 4 > old)
            }
        }
    }

    /// Whether a full collection that's due leaves what's young young (see `RSUI_GC_KEEP_YOUNG`): always, or when
    /// the last small collection kept more young than the old budget, as a full collection then lands in the middle
    /// of what's being worked on. Making that old had the `json` bench's full collections keep 3.0 MB where 0.8 MB was
    /// alive between rounds, and so allow four times as much to be made and become old.
    fn keeps_young(&self) -> bool {
        match keep_young_mode() {
            1 => true,
            2 => stress() != Stress::Off || self.surviving_bytes > self.old_budget(),
            _ => false,
        }
    }

    /// How much can be made before the next collection.
    fn threshold(&self) -> usize {
        if stress() != Stress::Off {
            return 0;
        }
        // What's young and was kept is alive, and takes up room until the next one too. An old array that's being added
        // to is looked through by each small collection, so at least as much as that is made between them, or one that
        // keeps growing would take quadratic time; it's alive, so a full collection would have allowed as much.
        //
        // What has become old since the last full collection only comes out of it once the next one is due to be full,
        // though, after a burst or once enough has. Until then it may well still be alive, as it is in a program whose
        // objects mostly stay, such as an app, and collecting more often costs that more than each small collection
        // saves, as each old object that's changed after a collection goes the slow way once and is remembered. Taking
        // it out all along had typing in the invoices example 5% slower, with collections 1.7 times as often.
        let taken = if self.full_due() {
            self.promoted + self.surviving_bytes
        } else {
            self.surviving_bytes
        };
        self.budget()
            .saturating_sub(taken)
            .max(self.least())
            .max(self.more.remembered_bytes)
    }
}

/// The places in a list that are free to use again, one bit each, so a collection can work out which they are a word
/// at a time rather than visiting each place. They're handed out lowest first, which lets the top of the list go quiet
/// and be given back.
#[derive(Default, Debug)]
pub(crate) struct FreeSlots {
    bits: Vec<u64>,
    /// No word before this one has a free place left.
    next: usize,
    /// The places on pages given back, which aren't free (see `giveback.rs`).
    holes: giveback::Holes,
}

impl FreeSlots {
    /// A free place, which isn't free any more, or nothing if the list has to grow.
    #[inline]
    pub(crate) fn pop(&mut self) -> Option<u32> {
        while let Some(word) = self.bits.get_mut(self.next) {
            if *word != 0 {
                let bit = word.trailing_zeros();
                *word &= *word - 1;
                return Some((self.next * 64) as u32 + bit);
            }
            self.next += 1;
        }
        None
    }

    /// How many pages of the list have been given back while their places were free.
    pub(crate) fn given_back_pages(&self) -> usize {
        self.holes.pages()
    }

    /// Whether this place is free, as of the last collection or since.
    pub(crate) fn is_free(&self, index: usize) -> bool {
        is_marked(&self.bits, index)
    }

    /// After marking a list of `len` places: calls `free` with each place that was in use and wasn't reached, makes
    /// every place that wasn't reached free, and says how long the list needs to be now, as the free places at its end
    /// can go. A place already free isn't looked at, so this costs a little per word plus whatever has just died.
    fn sweep(&mut self, marks: &[u64], len: usize, mut free: impl FnMut(usize)) -> usize {
        let words = len.div_ceil(64);
        self.bits.resize(words.max(self.bits.len()), 0);
        let mut needed = 0;
        for (index, (bits, marked)) in self.bits[..words].iter_mut().zip(marks).enumerate() {
            let valid = if (index + 1) * 64 <= len {
                u64::MAX
            } else {
                (1u64 << (len % 64)) - 1
            };
            let reached = marked & valid;
            let unreached = !marked & valid;
            let mut dead = unreached & !*bits;
            while dead != 0 {
                free(index * 64 + dead.trailing_zeros() as usize);
                dead &= dead - 1;
            }
            *bits = unreached;
            if reached != 0 {
                needed = index * 64 + 64 - reached.leading_zeros() as usize;
            }
        }
        // The places from `needed` on are all free, and the list is cut short there.
        self.bits.truncate(needed.div_ceil(64));
        if needed % 64 != 0 {
            if let Some(last) = self.bits.last_mut() {
                *last &= (1u64 << (needed % 64)) - 1;
            }
        }
        shrink_if_oversized(&mut self.bits);
        self.next = 0;
        needed
    }
}

/// What [`Vm::due_soon`] says to do between frames.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Soon {
    No,
    /// The collection that's next, full or small.
    Next,
    /// A full collection, which is cheap.
    Full,
}

/// A full collection that took no longer than this is cheap enough to do between frames before it's due (see
/// [`Vm::due_soon`]): a frame's spare time, with room to spare. The explorer's take about a millisecond, shadcn's page's 5-15.
const CHEAP_FULL_MICROS: u32 = 4_000;

/// How big a list can stay however short it's become, in bytes. Shrinking the engine's list of what natives have made since
/// the last safe point, which in `string_ops` grew to 6,144 values between collections, had it asked for again by every
/// collection's end, and the system's heap handed back and took its pages again: on some lengths of the environment that
/// was 16 page faults a collection, 3,500 in the run, and 6% of its time. 16K items of each list had zlib's peak 0.3 MB higher.
const SHRINK_FLOOR: usize = 64 * 1024;

/// Gives memory back from a list that's much bigger than it needs to be.
pub(crate) fn shrink_if_oversized<T>(list: &mut Vec<T>) {
    if list.capacity() > 1024 && list.capacity() * size_of::<T>() > SHRINK_FLOOR && list.len() * 4 < list.capacity() {
        list.shrink_to(list.len() * 2);
    }
}

/// What an object takes up, apart from what's in other objects.
fn object_bytes(object: &Object) -> usize {
    size_of::<Object>()
        + object.slots_heap_bytes()
        + match &object.kind {
            Kind::Array(items) | Kind::Arguments(items) => items.capacity() * 8,
            Kind::ArrayBuffer(data) => data.bytes.capacity(),
            _ => 0,
        }
}

/// Something that's given each value an object refers to. Marking is one, and goes through a method that's always
/// inlined, as a closure called from all of these places wasn't.
trait Visit: Sized {
    fn visit(&mut self, value: Value);

    /// Given a `WeakMap`, `WeakSet`, `WeakRef` or `FinalizationRegistry`, once its properties have been. Anything but
    /// marking is given everything in it, as what it holds weakly is still something it refers to, which a small
    /// collection has to be led to if it's young.
    #[inline(always)]
    fn weakly(&mut self, object: &Object) {
        each_weak_child(object, self);
    }

    /// Given the compiled function a function object or a generator runs. Only marking looks at these, and only when
    /// it frees code (see [`crate::compiled`]).
    #[inline(always)]
    fn proto(&mut self, _proto: u32) {}
}

impl<F: FnMut(Value)> Visit for F {
    #[inline(always)]
    fn visit(&mut self, value: Value) {
        self(value);
    }
}

/// Calls `visit` with everything an object that holds things weakly refers to, weakly or not. It's kept out of the
/// loops that go through every object, as these are rare and the loops are faster small.
#[inline(never)]
fn each_weak_child(object: &Object, visit: &mut impl Visit) {
    match &object.kind {
        Kind::WeakRef(target) => visit.visit(*target),
        Kind::WeakMap(data) | Kind::WeakSet(data) => {
            for (key, value) in &data.entries {
                visit.visit(*key);
                visit.visit(*value);
            }
        }
        Kind::FinalizationRegistry(data) => {
            visit.visit(data.cleanup);
            for cell in &data.cells {
                visit.visit(cell.target);
                visit.visit(cell.held);
                visit.visit(cell.token);
            }
            for (held, token) in &data.cleared {
                visit.visit(*held);
                visit.visit(*token);
            }
        }
        _ => {}
    }
}

/// Calls `visit` with everything an object refers to. Marking and checking the heap both go through this, so they can't
/// disagree about what an object holds.
#[inline(always)]
fn each_child(object: &Object, visit: &mut impl Visit) {
    for value in object.slots() {
        visit.visit(*value);
    }
    match &object.kind {
        Kind::Free
        | Kind::Plain
        | Kind::Error
        | Kind::Date(_)
        | Kind::ArrayBuffer(_)
        | Kind::BigInt(_) => {}
        Kind::Array(items) | Kind::Arguments(items) => {
            for value in items.iter() {
                visit.visit(*value);
            }
        }
        Kind::Function {
            proto,
            home,
            upvalues,
            ..
        } => {
            visit.proto(*proto);
            if *home != NONE {
                visit.visit(Value::object(*home));
            }
            for value in upvalues.iter() {
                visit.visit(*value);
            }
        }
        Kind::Native(native) => visit.visit(native.data),
        Kind::Bound(bound) => {
            visit.visit(bound.target);
            visit.visit(bound.this);
            for value in &bound.args {
                visit.visit(*value);
            }
        }
        Kind::Boxed(value) => visit.visit(*value),
        Kind::Map(data) | Kind::Set(data) => {
            for (key, value) in &data.entries {
                visit.visit(*key);
                visit.visit(*value);
            }
        }
        Kind::WeakRef(_) | Kind::WeakMap(_) | Kind::WeakSet(_) | Kind::FinalizationRegistry(_) => {
            visit.weakly(object)
        }
        Kind::Promise(data) => {
            visit.visit(data.value);
            for reaction in &data.reactions {
                visit.visit(reaction.on_fulfilled);
                visit.visit(reaction.on_rejected);
                visit.visit(reaction.derived);
            }
        }
        Kind::Generator(data) => {
            if data.closure != NONE {
                visit.visit(Value::object(data.closure));
            }
            if data.proto != NONE {
                visit.proto(data.proto);
            }
            for value in data.registers.iter() {
                visit.visit(*value);
            }
            if let Some(args) = &data.extra_args {
                for value in args {
                    visit.visit(*value);
                }
            }
            visit.visit(data.new_target);
            visit.visit(data.promise);
            visit.visit(data.return_value);
            for (_, value, promise) in &data.queue {
                visit.visit(*value);
                visit.visit(*promise);
            }
        }
        Kind::Iter(data) => visit.visit(data.target),
        Kind::Accessor { get, set } => {
            visit.visit(*get);
            visit.visit(*set);
        }
        Kind::RegExp(data) => {
            visit.visit(data.source);
            visit.visit(data.flags);
        }
        Kind::TypedArray(data) => visit.visit(data.buffer),
        Kind::DataView(data) => visit.visit(data.buffer),
        Kind::Proxy(data) => {
            visit.visit(data.target);
            visit.visit(data.handler);
        }
    }
}

/// The next few objects to trace, taken from the list of work early and asked for from memory then, so that by the time
/// each is traced it's likely to be at hand. Tracing goes from object to object all over the heap, and without this,
/// most of the time went on waiting for each one to arrive.
#[derive(Default)]
struct Ahead {
    handles: [u32; AHEAD],
    /// Where the next to trace is, and how many there are.
    first: usize,
    count: usize,
}

const AHEAD: usize = 8;

impl Ahead {
    #[inline(always)]
    fn next(&mut self, work: &mut Vec<u32>, objects: &[Object]) -> Option<u32> {
        while self.count < AHEAD {
            let Some(handle) = work.pop() else { break };
            if let Some(object) = objects.get(handle as usize) {
                prefetch(object);
            }
            self.handles[(self.first + self.count) % AHEAD] = handle;
            self.count += 1;
        }
        if self.count == 0 {
            return None;
        }
        let handle = self.handles[self.first];
        self.first = (self.first + 1) % AHEAD;
        self.count -= 1;
        Some(handle)
    }
}

/// Asks for the memory of an object, without waiting for it.
#[inline(always)]
#[allow(unsafe_code)]
fn prefetch(object: &Object) {
    // SAFETY: a prefetch reads nothing and changes nothing, it only says what will be read soon, and every x86-64
    // processor has the instruction.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        std::arch::x86_64::_mm_prefetch::<{ std::arch::x86_64::_MM_HINT_T0 }>(
            (object as *const Object).cast(),
        );
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = object;
}

/// The values of `WeakMap` entries whose keys haven't been reached (yet), by key. A key that's reached has its values
/// marked then. Whatever is still here once nothing more can be reached belongs to an entry that's gone.
#[derive(Default)]
pub(crate) struct Waiting {
    /// Each key's latest value in `values`, which leads to the one before through its second half.
    latest: std::collections::HashMap<u32, u32, foldhash::fast::FixedState>,
    values: Vec<(Value, u32)>,
}

impl Waiting {
    fn add(&mut self, key: u32, value: Value) {
        let at = self.values.len() as u32;
        let before = self.latest.insert(key, at).unwrap_or(NONE);
        self.values.push((value, before));
    }

    fn clear(&mut self) {
        self.latest.clear();
        self.values.clear();
        if self.latest.capacity() > 4096 {
            self.latest.shrink_to(0);
        }
        shrink_if_oversized(&mut self.values);
    }
}

/// What a small collection works out about each young object it reaches, while the object is at hand: whether it's
/// kept for the first time, or for the second and so becomes old, what it takes up, and, if it becomes old, whether it
/// refers to one that stays young, as then it's remembered. Doing this afterwards, as a pass of its own over the
/// objects reached, had a small collection that made a big tree old take three times as long as marking it.
struct Young<'a> {
    /// The young objects that become old if this collection keeps them: the ones the last small collection kept
    /// (at [`age`] 2), or the ones the last two kept (at 3).
    promoting: &'a [u64],
    /// At [`age`] 3, the young objects the last small collection kept for the first time, which are kept twice if
    /// this one keeps them. Empty at 2.
    survived_before: &'a [u64],
    /// The ones kept for the second time, at [`age`] 3.
    surviving_twice: &'a mut Vec<u64>,
    /// What was old when this collection started.
    old_before: &'a [u64],
    /// The cells and strings that were old when it started.
    old_cells: &'a [u64],
    old_strings: &'a [u64],
    /// The ones kept for the first time.
    surviving: &'a mut Vec<u64>,
    /// What the ones kept for the first time take up, and the ones kept for the second.
    surviving_bytes: usize,
    twice_bytes: usize,
    promoted: usize,
    /// The objects that became old and refer to one that's young.
    remembered: &'a mut Vec<u32>,
}

impl Young<'_> {
    #[inline(always)]
    fn reached(&mut self, object: &mut Object, handle: usize) {
        if is_marked(self.promoting, handle) {
            self.promote(object, handle);
        } else if is_marked(self.survived_before, handle) {
            set_mark(self.surviving_twice, handle);
            self.twice_bytes += object_bytes(object);
        } else {
            set_mark(self.surviving, handle);
            self.surviving_bytes += object_bytes(object);
        }
    }

    /// Makes an object old, as small collections have kept it as often as [`age`] says. It's out of line, as it goes
    /// through what the object refers to a second time, and two more copies of that in the loops that trace cost 16 KB.
    #[inline(never)]
    fn promote(&mut self, object: &mut Object, handle: usize) {
        object.flags |= OLD;
        self.promoted += object_bytes(object);
        // What it refers to was all reached, and what wasn't old and wasn't kept before stays young. A BigInt is in the list
        // of objects too, and ages the same way. A cell or a string that wasn't old may be young after this too, which
        // this has to be remembered for until it isn't.
        let (old, before) = (self.old_before, self.promoting);
        let (old_cells, old_strings) = (self.old_cells, self.old_strings);
        let mut young = false;
        each_child(object, &mut |child: Value| {
            young |= ((child.is_object() || child.is_bigint())
                && !is_marked(old, child.index())
                && !is_marked(before, child.index()))
                || (child.is_cell() && !is_marked(old_cells, child.index()))
                || (child.is_string() && !is_marked(old_strings, child.index()))
        });
        if young {
            remember(self.remembered, object, handle);
        }
    }
}

struct Marker<'a> {
    work: Vec<u32>,
    /// What the objects reached take up, worked out as each is traced, while it's at hand.
    live_bytes: usize,
    /// Of that, in a full collection, what the objects that weren't old before it take up.
    young_bytes: usize,
    object_marks: &'a mut Vec<u64>,
    string_marks: &'a mut Vec<u64>,
    cell_marks: &'a mut Vec<u64>,
    cells: &'a [Value],
    /// The cells marked, which a small collection looks at again afterwards.
    new_cells: Vec<u32>,
    /// Where the list of objects starts, to tell which one is being traced from where it is, as only one that holds
    /// something weakly needs to be told. The list doesn't move while marking.
    base: usize,
    /// The objects reached that hold something weakly, which are looked at again once everything else has been
    /// marked, in the order they were reached.
    weak: Vec<u32>,
    shapes: &'a Shapes,
    /// The shapes in use, which only a full collection marks (see [`Marker::shape`]).
    shape_marks: &'a mut Vec<u64>,
    /// The atoms in use, which only a full collection goes by: the symbols reached and the names of the shapes marked.
    atom_marks: &'a mut Vec<u64>,
    atoms: &'a crate::text::Atoms,
    /// Whether this collection frees atoms, so that a symbol that's a `WeakMap`'s key has to be reached for its value
    /// to be marked.
    freeing_atoms: bool,
    /// The compiled functions behind the function objects and generators reached, when this collection frees code.
    code_marks: Option<&'a mut Vec<u64>>,
}

impl Visit for Marker<'_> {
    #[inline(always)]
    fn visit(&mut self, value: Value) {
        self.value(value);
    }

    #[inline(always)]
    fn proto(&mut self, proto: u32) {
        if let Some(marks) = self.code_marks.as_deref_mut()
            && let Some(word) = marks.get_mut(proto as usize / 64)
        {
            *word |= 1u64 << (proto % 64);
        }
    }

    /// What a registry gives its callback, and the callback, are held strongly. Everything else is left for
    /// [`Marker::ephemerons`] and [`Vm::clear_weak`].
    #[inline(never)]
    fn weakly(&mut self, object: &Object) {
        if let Kind::FinalizationRegistry(data) = &object.kind {
            self.value(data.cleanup);
            for cell in &data.cells {
                self.value(cell.held);
            }
            for (held, _) in &data.cleared {
                self.value(*held);
            }
        }
        let handle = (object as *const Object as usize - self.base) / size_of::<Object>();
        self.weak.push(handle as u32);
    }
}

impl Marker<'_> {
    #[inline(always)]
    fn value(&mut self, value: Value) {
        // Numbers, and the values that aren't anything on the heap, are done with in one test.
        let tag = value.tag();
        if tag == Value::OBJECT_TAG {
            if !set_mark(self.object_marks, value.index()) {
                self.work.push(value.handle());
            }
        } else if tag == Value::STRING_TAG {
            set_mark(self.string_marks, value.index());
        } else if tag >= Value::SYMBOL_TAG {
            self.symbol_cell_or_bigint(value);
        }
    }

    #[inline(never)]
    fn symbol_cell_or_bigint(&mut self, value: Value) {
        if value.tag() == Value::CELL_TAG {
            if !set_mark(self.cell_marks, value.index()) {
                self.new_cells.push(value.handle());
                let inner = self.cells[value.index()];
                self.value(inner);
            }
        } else if value.is_bigint() {
            if !set_mark(self.object_marks, value.index()) {
                self.live_bytes += size_of::<Object>();
            }
        } else {
            self.atom(Atom(value.handle()));
        }
    }

    fn values(&mut self, values: &[Value]) {
        for value in values {
            self.value(*value);
        }
    }

    fn object_handle(&mut self, handle: u32) {
        if handle != NONE {
            self.value(Value::object(handle));
        }
    }

    #[inline(always)]
    fn trace(&mut self, object: &Object) {
        self.live_bytes += object_bytes(object);
        each_child(object, self);
    }

    /// Marks a shape as in use, in a full collection, as an object that's reached has it. What it comes from is in
    /// use too, as finding a property goes through it, and so is its prototype, which is how prototypes are reached:
    /// an object's prototype is kept for as long as the object, through its shape. Most objects share a few shapes,
    /// so this is nearly always one test.
    #[inline(always)]
    fn shape(&mut self, shape: u32) {
        if !is_marked(self.shape_marks, shape as usize) {
            self.new_shape(shape);
        }
    }

    /// Marks the shapes from this one back to its root, up to one that's already marked, with their prototypes and
    /// the names of their properties.
    #[inline(never)]
    fn new_shape(&mut self, mut shape: u32) {
        while shape != NONE && !set_mark(self.shape_marks, shape as usize) {
            self.object_handle(self.shapes.items[shape as usize].proto);
            let atom_marks = &mut *self.atom_marks;
            self.shapes
                .own_names(shape, |name| mark_atom(atom_marks, name));
            shape = self.shapes.parent_of(shape);
        }
    }

    /// Marks an atom as in use. Only a full collection goes by these marks, and a whole number used as a name isn't an
    /// atom that can be freed.
    #[inline]
    fn atom(&mut self, atom: Atom) {
        mark_atom(self.atom_marks, atom);
    }

    /// Goes through the `WeakMap`s reached since this was last called, from `*from` on in `weak`: the value of an entry
    /// whose key has been reached is marked, and the others wait in `waiting` for their key, or in `symbols` for a
    /// symbol's. A symbol only has to be reached when atoms are being freed, and one that's pinned never goes. Says
    /// whether that marked anything new to go through.
    fn ephemerons(
        &mut self,
        objects: &[Object],
        from: &mut usize,
        waiting: &mut Waiting,
        symbols: &mut Vec<(Value, Value)>,
    ) -> bool {
        while let Some(&handle) = self.weak.get(*from) {
            *from += 1;
            let Kind::WeakMap(data) = &objects[handle as usize].kind else {
                continue;
            };
            for (key, value) in &data.entries {
                if key.is_object() && !is_marked(self.object_marks, key.index()) {
                    waiting.add(key.handle(), *value);
                } else if key.is_symbol() && !self.symbol_reached(*key) {
                    symbols.push((*key, *value));
                } else if !key.is_empty() {
                    self.value(*value);
                }
            }
        }
        !self.work.is_empty()
    }

    fn symbol_reached(&self, symbol: Value) -> bool {
        !self.freeing_atoms
            || is_marked(self.atom_marks, symbol.index())
            || self.atoms.is_pinned(Atom(symbol.handle()))
    }

    /// Marks the values waiting for symbols that have been reached since, and says whether that marked anything new to
    /// go through. A symbol is reached as a value, with nothing to trace, so this is looked at after each round.
    fn symbol_keys_reached(&mut self, symbols: &mut Vec<(Value, Value)>) -> bool {
        let mut at = 0;
        while at < symbols.len() {
            let (key, value) = symbols[at];
            if self.symbol_reached(key) {
                self.value(value);
                symbols.swap_remove(at);
            } else {
                at += 1;
            }
        }
        !self.work.is_empty()
    }

    /// The main loop of a full collection, which makes everything it reaches old and marks its shape. It's a function of
    /// its own so that the loop of a small collection, in [`Vm::collect`], keeps all the room it had.
    #[inline(never)]
    fn mark_all(&mut self, objects: &mut [Object]) {
        let mut ahead = Ahead::default();
        while let Some(handle) = ahead.next(&mut self.work, objects) {
            let object = &mut objects[handle as usize];
            // Most of what a full collection reaches is old already, and an object that isn't written to stays clean
            // in the cache, so it needn't be written back to memory once it's pushed out by the rest of the heap.
            if object.flags & OLD == 0 {
                object.flags |= OLD;
                self.young_bytes += object_bytes(object);
            }
            self.shape(object.shape);
            self.trace(object);
        }
    }

    /// The main loop of a small collection, which marks what's young and tells `young` about each. It's a function of
    /// its own, as [`Marker::mark_all`] is.
    #[inline(never)]
    fn mark_young(&mut self, objects: &mut [Object], young: &mut Young) {
        let mut ahead = Ahead::default();
        while let Some(handle) = ahead.next(&mut self.work, objects) {
            let object = &mut objects[handle as usize];
            each_child(object, self);
            young.reached(object, handle as usize);
        }
    }

    /// Marks what's reached through the `WeakMap`s reached (see the comment where this is called), tracing as the main
    /// loop of [`Vm::collect`] does. It's a function of its own so as to keep that loop as small as it was.
    #[inline(never)]
    fn mark_through_ephemerons(
        &mut self,
        objects: &mut [Object],
        mut young: Option<&mut Young>,
        waiting: &mut Waiting,
    ) {
        let mut from = 0;
        let mut symbols = Vec::new();
        while self.ephemerons(objects, &mut from, waiting, &mut symbols)
            | self.symbol_keys_reached(&mut symbols)
        {
            while let Some(handle) = self.work.pop() {
                let object = &mut objects[handle as usize];
                // What `young` works out only goes by what was old or young before this collection, so it can come
                // first, and there's one way through to tracing, rather than a copy of it for each kind.
                if let Some(young) = young.as_deref_mut() {
                    young.reached(object, handle as usize);
                } else {
                    object.flags |= OLD;
                    self.shape(object.shape);
                }
                self.trace(object);
                if !waiting.latest.is_empty() {
                    self.key_reached(handle, waiting);
                }
            }
        }
        waiting.clear();
    }

    /// Marks the values that were waiting for this key, now that it's been reached.
    #[inline(never)]
    fn key_reached(&mut self, handle: u32, waiting: &mut Waiting) {
        let Some(mut at) = waiting.latest.remove(&handle) else {
            return;
        };
        while at != NONE {
            let (value, before) = waiting.values[at as usize];
            self.value(value);
            at = before;
        }
    }
}

impl Vm {
    /// Frees what can't be reached: only what's young, unless enough has become old since the last full collection.
    #[cold]
    #[inline(never)]
    pub(crate) fn collect_garbage(&mut self) {
        // Nothing's old before the first one, so it has to look at everything anyway.
        let full = self.gc_count == 0 || self.generations.full_due();
        let keep_young = full && self.gc_count != 0 && self.generations.keeps_young();
        self.collect(full, keep_young);
    }

    /// Whether the next collection is likely to come before this is asked again, between frames, as what's made between
    /// two of them, the most of the last few, would bring it on. It also notes what was made since
    /// it was asked last.
    pub(crate) fn due_soon(&mut self) -> Soon {
        let more = &mut self.generations.more;
        let made = if self.gc_count == more.collections_between_frames {
            self.debt.saturating_sub(more.made_between_frames)
        } else {
            self.debt
        };
        more.between_frames = made.max(more.between_frames - more.between_frames / 8);
        more.made_between_frames = self.debt;
        more.collections_between_frames = self.gc_count;
        if stress() != Stress::Off || self.gc_count == 0 {
            return Soon::No;
        }
        // The next one is due, or full and so due anyway: what's become old has had time to die, and a full one's mark of
        // everything is best done where nobody's waiting for it.
        if self.debt + more.between_frames >= self.threshold || self.generations.full_due() {
            return Soon::Next;
        }
        // A full collection that's cheap, as it is for a small heap, is done once it's a third of the way to being due: what's
        // become old since the last one may have died, and finding that out between frames costs nobody anything. The
        // explorer's last full collection was otherwise as it started, keeping what starting was working on, and it
        // ended 1 MB higher.
        let old = self.generations.old_budget();
        if self.generations.more.last_full_micros <= CHEAP_FULL_MICROS && self.generations.promoted * 4 > old {
            return Soon::Full;
        }
        Soon::No
    }

    /// Collects as [`Vm::due_soon`] said, from outside any JavaScript.
    pub(crate) fn collect_between_frames(&mut self, soon: Soon) {
        self.nursery.clear();
        self.run_mark = 0;
        self.top = 0;
        if soon != Soon::No && logging() {
            eprintln!(
                "gc {}: between frames ({}), {} made of {}",
                self.gc_count + 1,
                if soon == Soon::Full { "a cheap full one" } else { "the next" },
                self.debt,
                self.threshold
            );
        }
        match soon {
            Soon::No => return,
            Soon::Next => self.collect_garbage(),
            Soon::Full => self.collect_all_garbage(),
        }
        self.note_between_frames();
    }

    /// Notes that a collection was done between frames.
    pub(crate) fn note_between_frames(&mut self) {
        let more = &mut self.generations.more;
        more.made_between_frames = self.debt;
        more.collections_between_frames = self.gc_count;
    }

    /// Frees everything that can't be reached, young or old.
    pub(crate) fn collect_all_garbage(&mut self) {
        self.collect(true, false);
    }

    /// A small collection from outside any JavaScript, for tests of what generations do.
    #[cfg(test)]
    pub(crate) fn collect_young_garbage(&mut self) {
        self.nursery.clear();
        self.run_mark = 0;
        self.top = 0;
        self.collect(false, false);
    }

    /// What's reached can include a handle to something already freed: a register a function no longer reads can still
    /// hold what was in it when a smaller function it called was running, and only that function's registers were
    /// looked at then. Such a handle keeps whatever has its place now alive a little longer, which does no harm. A
    /// string's can even be past the end of the list, in the last word of marks, and is left alone.
    fn collect(&mut self, full: bool, keep_young: bool) {
        let _split = rsui_alloc::split::enter(rsui_alloc::split::GC);
        let _span = rsui_spans::span(rsui_spans::Kind::Gc).map(|mut span| {
            span.arg(u32::from(full));
            span
        });
        self.gc_count += 1;
        let lengths = (
            self.objects.list.len(),
            self.strings.items.len(),
            self.cells.list.len(),
        );
        let started = std::time::Instant::now();
        // Handles can be used again after this, and a big string's units shouldn't stay around.
        self.regexp_input = (NONE, Vec::new());
        // What the numbers' strings name may be freed now, and their places given to others.
        self.number_strings.fill((0, NONE));
        self.trap_cache = [(NONE, 0, 0); 4];
        self.piece_cache = [(NONE, 0, 0, NONE); 8];
        self.last_match = crate::regex::LastMatches::default();
        // Nothing a native is holding above this point is still being held.
        self.nursery.truncate(self.run_mark);
        // Calls don't clear the registers they leave behind, so nothing above the top may name something freed.
        self.wipe_above_top();
        // What's watched and young is seen as it is: see `Watch`.
        self.objects.watch.unfake(&mut self.objects.list);
        if full && died_old_logging() {
            self.generations.more.died_before.clear();
            self.generations
                .more
                .died_before
                .extend_from_slice(&self.object_marks);
        }
        if full && !keep_young {
            // Which strings were old, to tell what this keeps that was young.
            self.generations.strings_before.clear();
            self.generations
                .strings_before
                .extend_from_slice(&self.strings.marks);
            clear_marks(&mut self.object_marks, self.objects.len());
            clear_marks(&mut self.strings.marks, self.strings.items.len());
            clear_marks(&mut self.cells.marks, self.cells.list.len());
            // Every object that's reached is looked through anyway, and has `OLD` set again then.
            self.objects.remembered.clear();
            self.cells.remembered.clear();
        } else {
            extend_marks(&mut self.object_marks, self.objects.len());
            extend_marks(&mut self.strings.marks, self.strings.items.len());
            extend_marks(&mut self.cells.marks, self.cells.list.len());
            if verifying() {
                let when = if full { "before a full collection" } else { "before a small collection" };
                self.verify_generations(when);
                self.verify_weak(when, false);
                self.verify_shapes(when);
                self.verify_atoms(when);
            }
            self.generations.strings_before.clear();
            self.generations
                .strings_before
                .extend_from_slice(&self.strings.marks);
            self.generations.old_before.clear();
            self.generations
                .old_before
                .extend_from_slice(&self.object_marks);
            // The cells that are old: the marked ones, and the ones remembered since the last collection, which are
            // unmarked until this one.
            let cells_before = &mut self.generations.more.cells_old_before;
            cells_before.clear();
            cells_before.extend_from_slice(&self.cells.marks);
            for &cell in &self.cells.remembered {
                set_mark(cells_before, cell as usize);
            }
            if keep_young {
                // A full collection marks everything again, and what's young after it is worked out from what was old
                // before it. What's remembered isn't a root then, as it may have died: it's what's looked at afterwards
                // for referring to what's still young, as nothing else old can (see `keeps_young`).
                clear_marks(&mut self.object_marks, self.objects.len());
                clear_marks(&mut self.strings.marks, self.strings.items.len());
                clear_marks(&mut self.cells.marks, self.cells.list.len());
            }
        }

        // Places on pages that were given back look reached, so nothing traces into them (see `giveback.rs`).
        self.free_objects.holes().mark(&mut self.object_marks);
        self.strings.free.holes().mark(&mut self.strings.marks);
        self.free_cells.holes().mark(&mut self.cells.marks);
        let mut object_marks = std::mem::take(&mut self.object_marks);
        let mut string_marks = std::mem::take(&mut self.strings.marks);
        let mut cell_marks = std::mem::take(&mut self.cells.marks);
        let mut remembered = std::mem::take(&mut self.objects.remembered);
        let remembered_cells = std::mem::take(&mut self.cells.remembered);
        // A young object becomes old the second time a small collection keeps it, so what was only being worked on
        // when one came along can still die young. One kept for the first time is unmarked again after sweeping, and
        // whatever old refers to it is remembered, as the next small collection has to reach it through that. See
        // [`Young`].
        let mut survived_before = std::mem::take(&mut self.generations.surviving);
        let mut surviving = std::mem::take(&mut self.generations.spare_marks);
        let mut twice_before = std::mem::take(&mut self.generations.surviving_twice);
        let mut surviving_twice = std::mem::take(&mut self.generations.spare_twice);
        let old_before = std::mem::take(&mut self.generations.old_before);
        let mut still_remembered = std::mem::take(&mut self.generations.spare_handles);
        // A cell becomes old the second time a small collection keeps it, as an object does at [`age`] 2, and for the
        // same reason: see [`Aging::cells_surviving`].
        let cells_survived_before = std::mem::take(&mut self.generations.more.cells_surviving);
        let mut cells_surviving = std::mem::take(&mut self.generations.more.cells_spare);
        let cells_old_before = std::mem::take(&mut self.generations.more.cells_old_before);
        cells_surviving.clear();
        let strings_before = std::mem::take(&mut self.generations.strings_before);
        let three = age() == 3;
        if full && !keep_young {
            survived_before.clear();
            surviving.clear();
            twice_before.clear();
            surviving_twice.clear();
        } else {
            clear_marks(&mut surviving, self.objects.len());
            if three {
                clear_marks(&mut surviving_twice, self.objects.len());
            } else {
                surviving_twice.clear();
            }
        }
        let (promoting, survived_once): (&[u64], &[u64]) = if three {
            (&twice_before, &survived_before)
        } else {
            (&survived_before, &[])
        };
        let mut young = Young {
            promoting,
            survived_before: survived_once,
            surviving_twice: &mut surviving_twice,
            old_before: &old_before,
            old_cells: &cells_old_before,
            old_strings: &strings_before,
            surviving: &mut surviving,
            surviving_bytes: 0,
            twice_bytes: 0,
            promoted: 0,
            remembered: &mut still_remembered,
        };
        let remembered_count = remembered.len() + remembered_cells.len();
        let mut new_cells;
        let mut live_bytes;
        let full_young_bytes;
        let remembered_bytes;
        let mut weak;
        let mut shape_marks = std::mem::take(&mut self.generations.shape_marks);
        if full {
            clear_marks(&mut shape_marks, self.shapes.items.len());
        }
        // Atoms are freed by a full collection, and only when nothing but JavaScript is running, as a native that has
        // called back into it can be holding atoms that nothing else has, which the collector can't see: the names it
        // was asked for, or the keys it's going through (see [`Vm::nested_runs`]). A later full collection frees
        // them.
        let free_atoms = full && self.nested_runs <= 1;
        let mut atom_marks = std::mem::take(&mut self.generations.atom_marks);
        clear_marks(&mut atom_marks, if free_atoms { self.atoms.len() } else { 0 });
        // Code is freed at the same times, for the same reason: a native can be holding a function it's compiled and
        // is about to run. See `compiled.rs`.
        let free_code = free_atoms;
        let mut code_marks = std::mem::take(&mut self.code_room.marks);
        if free_code {
            clear_marks(&mut code_marks, self.protos.len());
        }
        {
            let mut marker = Marker {
                work: Vec::with_capacity(256),
                live_bytes: 0,
                young_bytes: 0,
                object_marks: &mut object_marks,
                string_marks: &mut string_marks,
                cell_marks: &mut cell_marks,
                cells: &self.cells.list,
                new_cells: std::mem::take(&mut self.generations.new_cells),
                base: self.objects.list.as_ptr() as usize,
                weak: std::mem::take(&mut self.generations.weak),
                shapes: &self.shapes,
                shape_marks: &mut shape_marks,
                atom_marks: &mut atom_marks,
                atoms: &self.atoms,
                freeing_atoms: free_atoms,
                code_marks: free_code.then_some(&mut code_marks),
            };
            marker.values(&self.stack[..self.top]);
            marker.values(&self.consts);
            marker.values(&self.nursery);
            marker.values(&self.roots);
            for intrinsic in &self.selector_intrinsics {
                marker.value(intrinsic.owner);
                marker.value(intrinsic.value);
            }
            self.held.each(|value| marker.value(value));
            marker.values(&self.char_strings);
            marker.value(Value::object(self.global));
            marker.value(self.function_has_instance);
            marker.value(self.throw_type_error);
            marker.value(self.realm.array_values.0);
            marker.value(self.realm.disposal.0);
            marker.value(self.realm.name_function.0);
            marker.value(self.realm.timeout_proto.0);
            marker.value(self.realm.immediate_proto.0);
            marker.value(self.helpers);
            for frame in &self.frames {
                marker.object_handle(frame.closure);
                marker.value(frame.new_target);
            }
            for args in &self.kept_args {
                marker.values(args);
            }
            for job in &self.jobs {
                match job {
                    Job::Reaction {
                        handler,
                        argument,
                        derived,
                        ..
                    } => {
                        marker.value(*handler);
                        marker.value(*argument);
                        marker.value(*derived);
                    }
                    Job::Thenable {
                        promise,
                        thenable,
                        then,
                    } => {
                        marker.value(*promise);
                        marker.value(*thenable);
                        marker.value(*then);
                    }
                    Job::Call(function) => marker.value(*function),
                }
            }
            marker.values(&self.kept);
            self.each_read_value(|value| marker.value(value));
            for registry in &self.cleanups {
                marker.value(*registry);
            }
            for timer in &self.timers {
                marker.value(timer.callback);
                marker.values(&timer.args);
                marker.value(timer.object);
            }
            marker.values(&self.unhandled_rejections);
            marker.values(&self.modules.values());
            if full {
                // A prototype is reached from the objects that have it, through their shapes, and a shape from the
                // objects that have it, or from what the engine makes objects with. A free place in the list of
                // objects has shape 0.
                for shape in self.held_shapes() {
                    if shape != NONE {
                        marker.shape(shape);
                    }
                }
            } else {
                // A small collection doesn't free shapes, so it has to keep what any of them could still be given to.
                // A shape's prototype is old once a collection has kept it, and stays marked as old, so only the shapes
                // whose prototypes were young after the last one, and the ones made or changed since, need looking at.
                // Going through every shape, which is large, was over a percent of a program that makes a lot of
                // short-lived objects.
                for &shape in self.generations.young_protos.iter().chain(&self.shapes.touched) {
                    if let Some(item) = self.shapes.items.get(shape as usize) {
                        marker.object_handle(item.proto);
                    }
                }
            }
            for handle in self.realm_handles() {
                marker.object_handle(handle);
            }
            let objects = &mut self.objects.list;
            // The old objects and cells changed since the last collection can refer to young things.
            let mut looked_at = 0;
            if !full {
                for handle in &remembered {
                    set_mark(marker.object_marks, *handle as usize);
                    let object = &mut objects[*handle as usize];
                    object.flags |= OLD;
                    each_child(object, &mut |_| looked_at += 1);
                    each_child(object, &mut marker);
                }
                for cell in &remembered_cells {
                    set_mark(marker.cell_marks, *cell as usize);
                    let inner = marker.cells[*cell as usize];
                    marker.value(inner);
                }
            }
            remembered_bytes = looked_at * size_of::<Value>();
            // After a full collection everything reached is old, and its shape is in use. After a small one, see
            // below. Each has a loop of its own, as one loop with both in it had less room for either.
            if full {
                marker.mark_all(objects);
            } else {
                // What objects take up is only worked out for the ones that become old or are kept for the first
                // time, which is all a small collection goes by.
                marker.mark_young(objects, &mut young);
            }
            // What's held weakly waited until nothing more could be reached otherwise. Now the entries of the
            // `WeakMap`s reached whose keys were reached have their values marked, which can reach more keys, of these
            // or of `WeakMap`s reached only now, and so on: a key reached from then on has its values marked as it's
            // traced.
            if !marker.weak.is_empty() {
                marker.mark_through_ephemerons(
                    objects,
                    (!full).then_some(&mut young),
                    &mut self.generations.waiting,
                );
            }
            live_bytes = marker.live_bytes;
            full_young_bytes = marker.young_bytes;
            new_cells = std::mem::take(&mut marker.new_cells);
            weak = std::mem::take(&mut marker.weak);
        }
        if keep_young {
            // What a full collection reached that wasn't old ages as a small collection would have aged it, rather than
            // all being made old: see `keeps_young`. Marking made it old for the while, which is taken off again.
            let objects = &mut self.objects.list;
            let holes = self.free_objects.holes();
            for (at, word) in object_marks.iter().enumerate() {
                let mut word = *word & !old_before.get(at).copied().unwrap_or(0) & !holes.word(at);
                while word != 0 {
                    let handle = at * 64 + word.trailing_zeros() as usize;
                    word &= word - 1;
                    let Some(object) = objects.get_mut(handle) else {
                        break;
                    };
                    // A small collection makes a BigInt old the first time it keeps it.
                    if matches!(object.kind, Kind::BigInt(_)) {
                        continue;
                    }
                    object.flags &= !OLD;
                    young.reached(object, handle);
                }
            }
            live_bytes = live_bytes.saturating_sub(young.surviving_bytes + young.twice_bytes);
        }
        self.clear_weak(&weak, &object_marks, free_atoms.then_some(&atom_marks[..]));
        if full && died_old_logging() {
            self.report_died_old(&object_marks);
        }
        if !full && died_old_logging() {
            self.report_promoted(&object_marks, &old_before);
        }

        // What a full collection made old is in what it found alive.
        let promoted = if full { 0 } else { young.promoted };
        // What was made since the last collection and kept, and all that's young and kept, which takes up room until
        // the next one.
        let kept_new_bytes = young.surviving_bytes;
        let mut surviving_bytes = young.surviving_bytes + young.twice_bytes;
        if !full {
            live_bytes = 0;
        }
        self.generations.spare_marks = survived_before;
        self.generations.spare_twice = twice_before;
        self.generations.old_before = old_before;
        let objects = &mut self.objects.list;

        // Sweeping: whatever was in use and wasn't reached is freed, and every place that wasn't reached can be used
        // again. Places at the end that are free just go, so a burst that's over doesn't keep its memory.
        let shapes = &mut self.shapes;
        let keep = self
            .free_objects
            .sweep(&object_marks, objects.len(), |index| {
                let object = &mut objects[index];
                if !matches!(object.kind, Kind::Free) {
                    if shapes.is_dict(object.shape) {
                        shapes.free_dict(object.shape);
                    }
                    object.clear();
                }
            });
        objects.truncate(keep);
        trim_marks(&mut object_marks, keep);
        // What handing on has share captured variables forgets what's gone, before its place is another's or given back.
        let free = &self.free_objects;
        self.reads.forget_freed_closures(|member| member as usize >= keep || free.is_free(member as usize));
        if !full || keep_young {
            trim_marks(&mut surviving, keep);
            for (mark, survived) in object_marks.iter_mut().zip(&surviving) {
                *mark &= !survived;
            }
            trim_marks(&mut surviving_twice, keep);
            for (mark, survived) in object_marks.iter_mut().zip(&surviving_twice) {
                *mark &= !survived;
            }
        }
        // The prototypes of the shapes looked at that are still young are looked at again next time. After a full
        // collection every shape that's left has an old prototype.
        let mut young_protos = std::mem::take(&mut self.generations.young_protos);
        if full && !keep_young {
            young_protos.clear();
        } else {
            let items = &self.shapes.items;
            let young = |shape: u32| {
                items.get(shape as usize).is_some_and(|item| {
                    item.proto != NONE && !is_marked(&object_marks, item.proto as usize)
                })
            };
            // Each shape is in the list once, which a mark for each shape says.
            let seen = &mut self.generations.seen_shapes;
            clear_marks(seen, items.len());
            young_protos.retain(|shape| young(*shape) && !set_mark(seen, *shape as usize));
            for &shape in &self.shapes.touched {
                if young(shape) && !set_mark(seen, shape as usize) {
                    young_protos.push(shape);
                }
            }
        }
        self.shapes.touched.clear();
        shrink_if_oversized(&mut self.shapes.touched);
        self.generations.young_protos = young_protos;
        // Shapes nothing has go, and whatever remembers one of them forgets it, before its place can be used again.
        if full {
            let mut freed = std::mem::take(&mut self.generations.shapes_freed);
            // The epoch isn't moved on, as what goes by it is cleared of the freed shapes here, and what the collector
            // doesn't know about goes by `Shapes::generation`. Moving it on had every place in the code that adds a
            // property find out how again after each full collection that freed any shape.
            if self.shapes.sweep(&shape_marks, &mut freed) {
                self.forget_freed_shapes(&freed);
            }
            if keep_young {
                self.generations.young_protos.retain(|&shape| !is_marked(&freed, shape as usize));
            }
            self.generations.shapes_freed = freed;
            // A small collection frees no prototype and no shape that isn't a dictionary, which none of these keep.
            self.forget_shape_caches();
        }
        self.generations.shape_marks = shape_marks;
        // So does code nothing can run any more.
        self.code_room.marks = code_marks;
        if free_code {
            self.free_unused_code();
        }
        // Atoms nothing has go too, and so does what remembers them, before their places can be used again. What
        // they're named by stays as long as they do.
        let mut atoms_freed = std::mem::take(&mut self.generations.atoms_freed);
        let any_atoms_freed = free_atoms
            && self
                .atoms
                .sweep(&atom_marks, &mut atoms_freed, &mut self.strings);
        if any_atoms_freed {
            self.forget_freed_atoms(&atoms_freed);
        }
        self.generations.atom_marks = atom_marks;
        // A string kept for the first time stays young, as a cell does, unless the last small collection kept it too:
        // what an object kept once refers to would otherwise be old when the object dies young, and die old. These
        // are unmarked after sweeping. Strings made old by small collections were as much again as the objects they
        // made old in a shadcn session.
        let strings_survived_before = std::mem::take(&mut self.generations.more.strings_surviving);
        let strings_twice_before = std::mem::take(&mut self.generations.more.strings_twice);
        let mut strings_surviving = std::mem::take(&mut self.generations.more.strings_spare);
        let mut strings_twice = std::mem::take(&mut self.generations.more.strings_twice_spare);
        strings_surviving.clear();
        strings_twice.clear();
        if !full || keep_young {
            let holes = self.strings.free.holes();
            let three = string_age() == 3;
            for (at, word) in string_marks.iter().enumerate() {
                let young = *word & !strings_before.get(at).copied().unwrap_or(0) & !holes.word(at);
                let kept = strings_survived_before.get(at).copied().unwrap_or(0);
                let twice = strings_twice_before.get(at).copied().unwrap_or(0);
                if three {
                    strings_surviving.push(young & !twice);
                    strings_twice.push(young & kept & !twice);
                } else {
                    strings_surviving.push(young & !kept);
                    strings_twice.push(0);
                }
            }
        }
        // The strings that name atoms stay as long as the atoms. After a full collection they're marked again, and
        // after a small one they're still marked, being old, apart from the ones made since the last collection,
        // which become old now.
        if full {
            for handle in self.atoms.strings() {
                set_mark(&mut string_marks, handle as usize);
                if let Some(word) = strings_surviving.get_mut(handle as usize / 64) {
                    *word &= !(1u64 << (handle % 64));
                }
                if let Some(word) = strings_twice.get_mut(handle as usize / 64) {
                    *word &= !(1u64 << (handle % 64));
                }
            }
        } else {
            for handle in self.atoms.added_strings() {
                set_mark(&mut string_marks, handle as usize);
                if let Some(word) = strings_surviving.get_mut(handle as usize / 64) {
                    *word &= !(1u64 << (handle % 64));
                }
                if let Some(word) = strings_twice.get_mut(handle as usize / 64) {
                    *word &= !(1u64 << (handle % 64));
                }
            }
        }
        self.atoms.forget_added();
        let objects = &mut self.objects.list;
        let generations = &mut self.generations;

        // Only the strings kept for the first time need looking at after a small collection, and only the ones that
        // become old count, which are the ones the last one kept too, and atoms' (see `strings_surviving`). A buffer
        // that several of them share counts once (see `Tally`).
        let before: &[u64] = if full { &[] } else { &strings_before };
        let items = &mut self.strings.items;
        let tally = crate::text::Tally::new();
        let string_holes = self.strings.free.holes();
        let mut young_string_bytes = 0usize;
        for (word_index, word) in string_marks.iter().enumerate() {
            let first = *word
                & !before.get(word_index).copied().unwrap_or(0)
                & !string_holes.word(word_index);
            let young = strings_surviving.get(word_index).copied().unwrap_or(0);
            let mut word = first;
            while word != 0 {
                let bit = word & word.wrapping_neg();
                let at = word_index * 64 + bit.trailing_zeros() as usize;
                let Some(string) = items.get_mut(at) else {
                    break;
                };
                // Only a string kept for the first time is settled (after a small collection, one that stays young).
                // One kept by the last small collection too is unmarked, as it was young, and settling it again gave
                // a string that's being added to its own exact copy between additions, so each one copied all of it:
                // with RSUI_GC_STRESS, the 2,000 steps of a text built a piece at a time came to 2 GB where they
                // share 2 MB (`text_built_a_piece_at_a_time_counts_once_however_many_steps_are_kept`).
                if full || young & bit != 0 {
                    string.text.settle();
                    // Kept for the first time, it shares its text with a string kept before that has the same: by a small
                    // collection, a string that's young now, and by a full one, one that wasn't old before it.
                    let first_time = young & bit != 0
                        || (full && strings_before.get(word_index).copied().unwrap_or(0) & bit == 0);
                    if first_time {
                        crate::text::share_text(items, at, string_holes);
                    }
                }
                let string = &items[at];
                let bytes = string.text.heap_bytes_in(tally) + 32;
                if full && strings_before.get(word_index).copied().unwrap_or(0) & bit == 0 {
                    young_string_bytes += bytes;
                }
                if young & bit == 0 {
                    live_bytes += bytes;
                } else {
                    // Kept for the first time, it takes up room until the next small collection as a young object does,
                    // which comes out of what can be made before then. Leaving it out had the `json` bench's peak a
                    // third higher, as what strings were kept used to count as made old.
                    surviving_bytes += bytes;
                }
                word &= word - 1;
            }
        }
        // A string that stays can remember an atom that's gone, which isn't its own: one with the same text.
        if any_atoms_freed {
            for (word_index, word) in string_marks.iter().enumerate() {
                let mut word = *word & !string_holes.word(word_index);
                while word != 0 {
                    let Some(string) = items.get_mut(word_index * 64 + word.trailing_zeros() as usize)
                    else {
                        break;
                    };
                    if Atom(string.atom).held_index().is_none()
                        && is_marked(&atoms_freed, string.atom as usize)
                    {
                        string.atom = 0;
                    }
                    word &= word - 1;
                }
            }
        }
        generations.atoms_freed = atoms_freed;
        let keep = self
            .strings
            .free
            .sweep(&string_marks, items.len(), |index| {
                let string = &mut items[index];
                if string.text.len() != 0 {
                    string.text = crate::text::Text::empty();
                }
                string.atom = 0;
            });
        items.truncate(keep);
        trim_marks(&mut string_marks, keep);
        trim_marks(&mut strings_surviving, keep);
        trim_marks(&mut strings_twice, keep);
        for (mark, young) in string_marks.iter_mut().zip(&strings_surviving) {
            *mark &= !young;
        }

        // A free cell is always written before it's read, so there's nothing to do to one that died.
        let keep = self
            .free_cells
            .sweep(&cell_marks, self.cells.list.len(), |_| {});
        self.cells.list.truncate(keep);
        trim_marks(&mut cell_marks, keep);
        // A cell kept for the first time stays young, unmarked, unless the last small collection kept it too. The ones
        // reached that weren't marked are `new_cells`, which has the old ones that were remembered as well, if they were
        // reached before the remembered ones were looked at.
        if !full || keep_young {
            clear_marks(&mut cells_surviving, keep);
            for &cell in &new_cells {
                let index = cell as usize;
                if index < keep
                    && !is_marked(&cells_old_before, index)
                    && !is_marked(&cells_survived_before, index)
                {
                    set_mark(&mut cells_surviving, index);
                    cell_marks[index / 64] &= !(1u64 << (index % 64));
                }
            }
        }

        // What's old now and refers to something young has to be remembered, as nothing else leads a small collection
        // there: an object, or a BigInt, which is in the list of objects, a string or a cell. The objects that have
        // just become old were looked at above.
        let mut still_remembered_cells = Vec::new();
        if !full || keep_young {
            let is_young = |value: Value| {
                ((value.is_object() || value.is_bigint()) && !is_marked(&object_marks, value.index()))
                    || (value.is_cell() && !is_marked(&cell_marks, value.index()))
                    || (value.is_string() && !is_marked(&string_marks, value.index()))
            };
            for handle in &remembered {
                // After a full collection what was remembered may have died.
                if full && (*handle as usize >= objects.len() || !is_marked(&object_marks, *handle as usize)) {
                    continue;
                }
                let object = &mut objects[*handle as usize];
                if object.flags & OLD == 0 {
                    continue;
                }
                let mut young = false;
                each_child(object, &mut |child| young |= is_young(child));
                if young {
                    remember(&mut still_remembered, object, *handle as usize);
                }
            }
            // Only the old ones: a young cell is reached through what has it, and remembering one would keep it whether
            // that's alive or not.
            for cell in remembered_cells.iter().chain(&new_cells) {
                let index = *cell as usize;
                if index < self.cells.list.len()
                    && is_marked(&cell_marks, index)
                    && is_young(self.cells.list[index])
                {
                    still_remembered_cells.push(*cell);
                }
            }
            for &cell in &still_remembered_cells {
                let index = cell as usize;
                cell_marks[index / 64] &= !(1u64 << (index % 64));
            }
        }

        shrink_if_oversized(&mut self.nursery);
        remembered.clear();
        shrink_if_oversized(&mut remembered);
        new_cells.clear();
        shrink_if_oversized(&mut new_cells);
        weak.clear();
        shrink_if_oversized(&mut weak);
        // The marks stay, as they say what's old.
        if full && died_old_logging() {
            self.generations.more.died_at_full.clear();
            self.generations.more.died_at_full.extend_from_slice(&object_marks);
        }
        self.object_marks = object_marks;
        self.strings.marks = string_marks;
        self.cells.marks = cell_marks;
        self.objects.remembered = still_remembered;
        self.cells.remembered = still_remembered_cells;
        let generations = &mut self.generations;
        generations.surviving = surviving;
        generations.surviving_twice = surviving_twice;
        generations.spare_handles = remembered;
        generations.new_cells = new_cells;
        generations.weak = weak;
        generations.more.cells_surviving = cells_surviving;
        generations.more.cells_spare = cells_survived_before;
        generations.more.cells_old_before = cells_old_before;
        generations.more.strings_surviving = strings_surviving;
        generations.more.strings_spare = strings_survived_before;
        generations.more.strings_twice = strings_twice;
        generations.more.strings_twice_spare = strings_twice_before;
        generations.strings_before = strings_before;
        if verifying() {
            self.verify_generations("after a collection");
            self.verify_weak("after a collection", full);
            self.verify_shapes("after a collection");
            self.verify_atoms("after a collection");
            self.verify_code("after a collection");
        }
        self.objects.watch.refake(&mut self.objects.list, full);

        if logging() {
            eprintln!(
                "gc {}: {}, {:.1} us, {} bytes {}{}, {} young kept, {} remembered, {} objects, {} shapes, {} atoms, {} made",
                self.gc_count,
                if full { "full" } else { "small" },
                started.elapsed().as_secs_f64() * 1e6,
                live_bytes + promoted,
                if full { "kept" } else { "made old" },
                if full { format!(" ({full_young_bytes} of objects and {young_string_bytes} of strings young before)") } else { String::new() },
                surviving_bytes,
                remembered_count,
                self.objects.len(),
                self.shapes.live(),
                self.atoms.live(),
                self.debt,
            );
        }
        let generations = &mut self.generations;
        // A small collection that kept more than half of what was made since the last one isn't worth it.
        generations.mostly_kept =
            !full && (live_bytes + promoted + kept_new_bytes) * 2 > self.debt;
        if full {
            // What this one kept that was young before it is old now, as what small collections make old is, and much
            // of it is what was being worked on just then, which dies soon after. It counts as made old since this one,
            // as much of it as died of what was made old that way the last time (what was old before this collection and
            // isn't now), and half of that, as some is what's being built. With none of it counted, the `json` bench's full
            // collections, which each keep a round's text and tree, came every third collection rather than every
            // second, and it peaked 1.4 MB higher; the explorer's last full one was as it started, and it ended 1 MB higher.
            let young_now = full_young_bytes + young_string_bytes;
            let old_before = generations.full_live + generations.more.promoted_since_full;
            let dead = old_before.saturating_sub(live_bytes.saturating_sub(young_now));
            let suspect = generations.more.young_at_full + generations.more.promoted_since_full;
            // Only when most of it died, as full collections are worth it then: counting less of it had builtin_misc's
            // full collections land elsewhere, one of them in the middle of a sort, and its peak 1 MB higher.
            let seed = if suspect == 0 || dead * 2 < suspect {
                0
            } else {
                (young_now as u128 * dead.min(suspect) as u128 / suspect as u128 / 2) as usize
            };
            if logging() {
                eprintln!(
                    "gc {}: {dead} bytes died old of {suspect} made old since the last full one; {young_now} kept that was young, {seed} counted as made old",
                    self.gc_count
                );
            }
            generations.full_live = live_bytes;
            generations.more.full_young = surviving_bytes;
            generations.more.young_at_full = young_now;
            generations.more.last_full_micros = started.elapsed().as_micros().min(u32::MAX as u128) as u32;
            generations.more.promoted_since_full = 0;
            generations.promoted = seed;
            generations.small_since_full = 0;
        } else {
            generations.promoted += live_bytes + promoted;
            generations.more.promoted_since_full += live_bytes + promoted;
            generations.small_since_full += 1;
        }
        generations.surviving_bytes = surviving_bytes;
        generations.more.remembered_bytes = remembered_bytes;
        self.debt = 0;
        self.threshold = generations.threshold();
        // What's past the end of each list goes back to the system, apart from as much as the list grew by since the
        // last collection (up to what can be made before the next one), as it's likely to grow by that much again, and
        // would otherwise ask for it again at once, a step at a time, and have each page zeroed again as it's first
        // written. Handing it all back cost a program making nothing but garbage over half of each collection's time.
        // That memory was in use a moment ago, so this doesn't raise the peak.
        let slack = |before: usize, after: usize, size: usize| {
            (before.saturating_sub(after) * size).min(self.threshold)
        };
        let slack_objects = slack(lengths.0, self.objects.list.len(), size_of::<Object>());
        let slack_strings = slack(
            lengths.1,
            self.strings.items.len(),
            size_of::<crate::text::StringData>(),
        );
        let slack_cells = slack(lengths.2, self.cells.list.len(), size_of::<Value>());
        self.objects.list.release_unused(slack_objects);
        self.strings.items.release_unused(slack_strings);
        self.cells.list.release_unused(slack_cells);
        if full && exact::enabled() {
            let saved = self.shrink_storage();
            if logging() {
                eprintln!("gc {}: shrank objects' storage by {saved} bytes", self.gc_count);
            }
        }
        if full && giveback::enabled() {
            self.give_back_pages();
        }
    }

    /// Writes what small collections made old since the last full collection and this one found dead, by what it is,
    /// the most bytes first. See `RSUI_GC_DIED_OLD`.
    #[cold]
    #[inline(never)]
    fn report_died_old(&mut self, reached: &[u64]) {
        let generations = &self.generations;
        let mut groups: std::collections::HashMap<String, (usize, usize)> = Default::default();
        let (mut count, mut bytes) = (0, 0);
        for index in 0..self.objects.list.len() {
            if self.free_objects.holes().has(index)
                || !is_marked(&generations.more.died_before, index)
                || is_marked(&generations.more.died_at_full, index)
                || is_marked(reached, index)
            {
                continue;
            }
            let object = &self.objects.list[index];
            if matches!(object.kind, Kind::Free) {
                continue;
            }
            let size = object_bytes(object);
            let label = crate::census::label_by_keys(self, index as u32, crate::text::CONSTRUCTOR);
            let group = groups.entry(label).or_default();
            group.0 += 1;
            group.1 += size;
            count += 1;
            bytes += size;
        }
        let mut groups: Vec<_> = groups.into_iter().collect();
        groups.sort_by(|a, b| b.1.1.cmp(&a.1.1));
        eprintln!("gc {}: died old {count} objects, {bytes} bytes", self.gc_count);
        for (label, (count, bytes)) in groups.iter().take(25) {
            eprintln!("  died old {count:>7} {bytes:>9} B  {label}");
        }
    }

    /// Writes what this small collection made old, by what it is, the most bytes first. See `RSUI_GC_DIED_OLD`.
    #[cold]
    #[inline(never)]
    fn report_promoted(&self, reached: &[u64], old_before: &[u64]) {
        let constructor = crate::text::CONSTRUCTOR;
        let mut groups: std::collections::HashMap<String, (usize, usize)> = Default::default();
        let (mut count, mut bytes) = (0, 0);
        for index in 0..self.objects.list.len() {
            if self.free_objects.holes().has(index) || is_marked(old_before, index) || !is_marked(reached, index) {
                continue;
            }
            let object = &self.objects.list[index];
            if object.flags & OLD == 0 || matches!(object.kind, Kind::Free) {
                continue;
            }
            let size = object_bytes(object);
            let group = groups.entry(crate::census::label_by_keys(self, index as u32, constructor)).or_default();
            group.0 += 1;
            group.1 += size;
            count += 1;
            bytes += size;
        }
        let mut groups: Vec<_> = groups.into_iter().collect();
        groups.sort_by(|a, b| b.1.1.cmp(&a.1.1));
        eprintln!("gc {}: promoted {count} objects, {bytes} bytes", self.gc_count);
        for (label, (count, bytes)) in groups.iter().take(8) {
            eprintln!("  promoted {count:>7} {bytes:>9} B  {label}");
        }
    }

    /// Checks that nothing old refers to something young unless it's remembered, and that an object is only unmarked as
    /// old if it's remembered, and panics if not. See `RSUI_GC_VERIFY`.
    #[cold]
    #[inline(never)]
    fn verify_generations(&self, when: &str) {
        let remembered: std::collections::HashSet<u32> =
            self.objects.remembered.iter().copied().collect();
        let remembered_cells: std::collections::HashSet<u32> =
            self.cells.remembered.iter().copied().collect();
        let is_old = |value: Value| -> bool {
            match value.tag() {
                Value::OBJECT_TAG => is_marked(&self.object_marks, value.index()),
                Value::STRING_TAG => is_marked(&self.strings.marks, value.index()),
                tag if tag >= Value::CELL_TAG => {
                    if value.is_bigint() {
                        is_marked(&self.object_marks, value.index())
                    } else {
                        is_marked(&self.cells.marks, value.index())
                            || remembered_cells.contains(&value.handle())
                    }
                }
                _ => true,
            }
        };
        for index in 0..self.objects.list.len() {
            if self.free_objects.holes().has(index) {
                continue;
            }
            let object = &self.objects.list[index];
            if matches!(object.kind, Kind::Free | Kind::BigInt(_)) {
                continue;
            }
            if !is_marked(&self.object_marks, index) {
                // A young object flagged as old would be remembered when it's changed, and then not marked.
                assert!(
                    object.flags & OLD == 0,
                    "{when}: young object {index} is flagged as old"
                );
                continue;
            }
            if object.flags & OLD == 0 {
                assert!(
                    remembered.contains(&(index as u32)),
                    "{when}: object {index} is old but isn't flagged as old or remembered"
                );
                continue;
            }
            each_child(object, &mut |child| {
                assert!(
                    is_old(child),
                    "{when}: old object {index} ({}) refers to young {}, and wasn't remembered",
                    kind_name(&object.kind),
                    describe(child)
                );
            });
        }
        for index in 0..self.cells.list.len() {
            let value = &self.cells.list[index];
            if is_marked(&self.cells.marks, index) && !self.free_cells.holes().has(index) {
                assert!(
                    is_old(*value),
                    "{when}: old cell {index} holds young {}, and wasn't remembered",
                    describe(*value)
                );
            }
        }
    }

    /// Lets go of what the `WeakMap`s, `WeakSet`s, `WeakRef`s and `FinalizationRegistry`s that were reached held weakly
    /// and that wasn't reached itself, before its place can be used again: its entries go, a `WeakRef` points at
    /// undefined, and a registry's callback is due for what was registered with it. Only these need looking at. One that
    /// wasn't reached in a small collection is old and hasn't been changed since it was last looked at, so what it holds
    /// is old too (or it would have been remembered), and everything old counts as reached in a small collection.
    /// A symbol is held weakly too, when the collection frees atoms (it has `atom_marks` then): one that wasn't reached
    /// goes, unless it's pinned.
    fn clear_weak(&mut self, weak: &[u32], marks: &[u64], atom_marks: Option<&[u64]>) {
        let atoms = &self.atoms;
        let gone = |value: Value| {
            if value.is_object() {
                !is_marked(marks, value.index())
            } else if value.is_symbol()
                && let Some(atom_marks) = atom_marks
            {
                !is_marked(atom_marks, value.index()) && !atoms.is_pinned(Atom(value.handle()))
            } else {
                false
            }
        };
        for &handle in weak {
            match &mut self.objects.list[handle as usize].kind {
                Kind::WeakRef(target) => {
                    if gone(*target) {
                        *target = Value::UNDEFINED;
                    }
                }
                Kind::WeakMap(data) | Kind::WeakSet(data) => {
                    let mut lost = 0;
                    for entry in &mut data.entries {
                        if gone(entry.0) {
                            *entry = (Value::EMPTY, Value::UNDEFINED);
                            lost += 1;
                        }
                    }
                    if lost > 0 {
                        data.live -= lost;
                        data.tidy_after_losing_keys();
                    }
                }
                Kind::FinalizationRegistry(data) => {
                    let RegistryData {
                        cells,
                        cleared,
                        queued,
                        ..
                    } = &mut **data;
                    for (_, token) in cleared.iter_mut() {
                        if gone(*token) {
                            *token = Value::EMPTY;
                        }
                    }
                    cells.retain_mut(|cell| {
                        if gone(cell.token) {
                            cell.token = Value::EMPTY;
                        }
                        if gone(cell.target) {
                            cleared.push_back((cell.held, cell.token));
                            return false;
                        }
                        true
                    });
                    shrink_if_oversized(cells);
                    if !cleared.is_empty() && !*queued {
                        *queued = true;
                        self.cleanups.push_back(Value::object(handle));
                    }
                }
                _ => {}
            }
        }
    }

    /// The shapes the engine makes objects with by itself, which stay whether anything has them or not. A free place in
    /// the list of objects has shape 0.
    fn held_shapes(&self) -> [u32; 10] {
        let realm = &self.realm;
        [
            0,
            realm.object_shape,
            realm.array_shape,
            realm.function_shape,
            realm.generator_function_shape,
            realm.async_function_shape,
            realm.async_generator_function_shape,
            self.match_shape,
            self.iter_result_shape,
            self.arguments_shape,
        ]
    }

    /// Clears what the places in the code and what reads and writes remember (`Vm::lookups`) of the shapes a full
    /// collection has just freed, as a free place can be given to another shape. An entry's `other` is a shape for
    /// a write that adds a property, and an object for a read, which can't be told apart here. One that's an object
    /// with the number of a freed shape is cleared as well, which only means that place finds it the long way once.
    /// An object a read found on a prototype is alive while its entry's shape is, unless a prototype has changed since,
    /// which the entry's epoch says (every prototype on the way is, through the shapes in between).
    fn forget_freed_shapes(&mut self, freed: &[u64]) {
        let gone = |shape: u32| shape != NONE && is_marked(freed, shape as usize);
        for range in self.caches_in_use() {
            for entry in &mut self.caches[range] {
                if gone(entry.shape) || gone(entry.other) {
                    entry.shape = NONE;
                    entry.other = NONE;
                }
            }
        }
        for found in &mut self.lookups {
            if gone(found.shape) || gone(found.holder) {
                found.shape = NONE;
                found.holder = NONE;
            }
        }
    }

    /// Clears what reads and writes remember (`Vm::lookups`) of the atoms a full collection has just freed, as a free
    /// place can be given to another atom. The places in the code have only pinned atoms.
    fn forget_freed_atoms(&mut self, freed: &[u64]) {
        for found in &mut self.lookups {
            if found.shape != NONE
                && Atom(found.atom).held_index().is_none()
                && is_marked(freed, found.atom as usize)
            {
                found.shape = NONE;
                found.holder = NONE;
            }
        }
    }

    /// Checks that nothing names an atom that's free: no value that's a symbol (in an object, or anything else the
    /// collector starts from), no shape, string, native, compiled function, place in the code or remembered read, and
    /// that the atoms' own table is right. Panics if not. See `RSUI_GC_VERIFY`.
    #[cold]
    #[inline(never)]
    pub(crate) fn verify_atoms(&self, when: &str) {
        let atoms = &self.atoms;
        atoms.verify(when, &self.strings);
        atoms.verify_strings_kept(when, &self.strings.marks);
        let check = |atom: Atom, what: &dyn Fn() -> String| {
            assert!(
                !atoms.is_free(atom),
                "{when}: {} names atom {}, which is free",
                what(),
                atom.0
            );
        };
        let check_value = |value: Value, what: &dyn Fn() -> String| {
            if value.is_symbol() {
                check(Atom(value.handle()), what);
            }
        };
        for index in 0..self.objects.list.len() {
            if self.free_objects.holes().has(index) {
                continue;
            }
            let object = &self.objects.list[index];
            if matches!(object.kind, Kind::Free) {
                continue;
            }
            let what = || format!("object {index} ({})", kind_name(&object.kind));
            each_child(object, &mut |child: Value| check_value(child, &what));
            if let Kind::Native(native) = &object.kind {
                check(native.name, &what);
            }
        }
        self.shapes.each_name(|shape, name| {
            check(name, &|| format!("shape {shape}"));
        });
        for index in 0..self.strings.items.len() {
            if self.strings.free.holes().has(index) {
                continue;
            }
            let string = &self.strings.items[index];
            if string.atom == 0 || self.strings.free.is_free(index) {
                continue;
            }
            let atom = Atom(string.atom);
            if atom.held_index().is_some() {
                continue;
            }
            check(atom, &|| format!("string {index}"));
            let own = atoms.string_of(atom).map(|own| &self.strings.items[own as usize].text);
            assert!(
                own == Some(&string.text),
                "{when}: string {index} ({}) says its atom is {}, which is called something else",
                string.text.to_rust(),
                atom.0
            );
        }
        for (at, found) in self.lookups.iter().enumerate() {
            if found.shape != NONE {
                check(Atom(found.atom), &|| format!("remembered lookup {at}"));
            }
        }
        for at in self.caches_in_use().into_iter().flatten() {
            check(self.caches[at].atom, &|| format!("place {at} in the code"));
        }
        for (index, proto) in self.protos.iter().enumerate() {
            let what = || format!("compiled function {index}");
            check(proto.name, &what);
            check(proto.file, &what);
            for name in &proto.capture_names {
                check(*name, &what);
            }
        }
        let roots = self.stack[..self.top]
            .iter()
            .chain(self.consts.iter())
            .chain(&self.nursery)
            .chain(&self.roots)
            .chain(&self.kept);
        for value in roots {
            check_value(*value, &|| "a root".to_owned());
        }
        self.held.each(|value| check_value(value, &|| "a Root".to_owned()));
        for atom in self.symbol_registry.values().chain(self.private_names.values()) {
            check(*atom, &|| "the engine".to_owned());
        }
    }

    /// Checks that no object, shape, place in the code or remembered read names a shape that's free, and that a
    /// shape's prototype hasn't been freed. Panics if not. See `RSUI_GC_VERIFY`.
    #[cold]
    #[inline(never)]
    fn verify_shapes(&self, when: &str) {
        let shapes = &self.shapes;
        let usable = |shape: u32| !shapes.is_free(shape);
        for index in 0..self.objects.list.len() {
            if self.free_objects.holes().has(index) {
                continue;
            }
            let object = &self.objects.list[index];
            if !matches!(object.kind, Kind::Free) {
                assert!(
                    usable(object.shape),
                    "{when}: object {index} ({}) has shape {}, which is free",
                    kind_name(&object.kind),
                    object.shape
                );
            }
        }
        for (index, shape) in shapes.items.iter().enumerate() {
            if shapes.is_free(index as u32) {
                continue;
            }
            let parent = shapes.parent_of(index as u32);
            assert!(
                parent == NONE || usable(parent),
                "{when}: shape {index} comes from {parent}, which is free"
            );
            if shape.proto != NONE {
                let proto = self.objects.list.get(shape.proto as usize);
                assert!(
                    proto.is_some_and(|proto| !matches!(proto.kind, Kind::Free)),
                    "{when}: shape {index} has prototype {}, which is free",
                    shape.proto
                );
                // A small collection only keeps the prototypes of these, as the rest are old.
                assert!(
                    is_marked(&self.object_marks, shape.proto as usize)
                        || self.generations.young_protos.contains(&(index as u32))
                        || self.shapes.touched.contains(&(index as u32)),
                    "{when}: shape {index} has a young prototype {} that a small collection wouldn't keep",
                    shape.proto
                );
            }
        }
        for shape in shapes.transition_targets() {
            assert!(
                usable(shape),
                "{when}: a transition goes to shape {shape}, which is free"
            );
        }
        for shape in self.held_shapes() {
            assert!(
                shape == NONE || usable(shape),
                "{when}: the engine holds free shape {shape}"
            );
        }
        for at in self.caches_in_use().into_iter().flatten() {
            let entry = &self.caches[at];
            assert!(
                entry.shape == NONE || usable(entry.shape),
                "{when}: place {at} in the code remembers shape {}, which is free",
                entry.shape
            );
        }
        for (at, found) in self.lookups.iter().enumerate() {
            assert!(
                found.shape == NONE || usable(found.shape),
                "{when}: remembered lookup {at} is for shape {}, which is free",
                found.shape
            );
        }
    }

    /// Forgets what's in the registers above the top as [`Vm::wipe_above_top`] does, as far as they've been used since it
    /// last did: the stack never gets smaller, and this is for the end of each turn. It stops at a long run of what it
    /// leaves, which is nearly always where use stopped. What's left past that only stays alive a little longer, as
    /// each collection wipes everything above the top anyway.
    pub(crate) fn wipe_used_above_top(&mut self) {
        let top = self.top.min(self.stack.len());
        let mut untouched = 0;
        for value in &mut self.stack[top..] {
            if value.0 == 0 {
                untouched += 1;
                if untouched == 256 {
                    break;
                }
            } else {
                untouched = 0;
                *value = Value(0);
            }
        }
    }

    /// Frees everything that can't be reached, as Node's `gc()` does when it's run with `--expose-gc`, from a native
    /// function that JavaScript has called. What natives further up are holding stays.
    #[doc(hidden)]
    pub fn collect_garbage_now(&mut self) {
        let saved = self.run_mark;
        self.run_mark = self.nursery.len();
        self.collect_all_garbage();
        self.run_mark = saved;
    }

    /// Checks that nothing a `WeakMap`, `WeakSet`, `WeakRef` or `FinalizationRegistry` holds names a place that's free,
    /// and after a full collection, that each entry of a `WeakMap` or `WeakSet` has a key that was reached, and a value
    /// that was, as one whose key wasn't should have gone. Panics if not. See `RSUI_GC_VERIFY`.
    #[cold]
    #[inline(never)]
    fn verify_weak(&self, when: &str, after_full: bool) {
        let objects = &self.objects.list;
        // A place on a page that's been given back is free, and has nothing to read (see `giveback.rs`).
        let holes = self.free_objects.holes();
        let is_free = |value: Value| match value.tag() {
            Value::OBJECT_TAG => {
                value.index() >= objects.len()
                    || holes.has(value.index())
                    || matches!(objects[value.index()].kind, Kind::Free)
            }
            Value::STRING_TAG => {
                value.index() >= self.strings.items.len()
                    || self.strings.free.is_free(value.index())
            }
            _ if value.is_bigint() => {
                value.index() >= objects.len()
                    || holes.has(value.index())
                    || matches!(objects[value.index()].kind, Kind::Free)
            }
            _ => false,
        };
        // After sweeping, whatever wasn't reached has been freed. Its mark doesn't say, as a full collection leaves
        // what's young unmarked (see `keeps_young`).
        let is_reached = |value: Value| !is_free(value);
        for index in 0..objects.len() {
            if holes.has(index) {
                continue;
            }
            let object = &objects[index];
            if !matches!(
                object.kind,
                Kind::WeakRef(_)
                    | Kind::WeakMap(_)
                    | Kind::WeakSet(_)
                    | Kind::FinalizationRegistry(_)
            ) {
                continue;
            }
            let kind = kind_name(&object.kind);
            each_weak_child(object, &mut |child: Value| {
                assert!(
                    !is_free(child),
                    "{when}: {kind} {index} holds {}, which is free",
                    describe(child)
                );
            });
            if !after_full {
                continue;
            }
            if let Kind::WeakMap(data) | Kind::WeakSet(data) = &object.kind {
                for (key, value) in &data.entries {
                    if key.is_empty() {
                        continue;
                    }
                    assert!(
                        is_reached(*key) && is_reached(*value),
                        "{when}: {kind} {index} still has an entry for {} -> {} that wasn't reached",
                        describe(*key),
                        describe(*value)
                    );
                }
            }
        }
    }

    fn realm_handles(&self) -> [u32; 38] {
        let realm = &self.realm;
        [
            realm.object_proto,
            realm.function_proto,
            realm.array_proto,
            realm.string_proto,
            realm.number_proto,
            realm.boolean_proto,
            realm.symbol_proto,
            realm.error_proto,
            realm.type_error_proto,
            realm.range_error_proto,
            realm.reference_error_proto,
            realm.syntax_error_proto,
            realm.iterator_proto,
            realm.native_iterator_proto,
            realm.generator_proto,
            realm.promise_proto,
            realm.promise_constructor,
            realm.array_constructor,
            realm.map_proto,
            realm.set_proto,
            realm.weak_map_proto,
            realm.weak_set_proto,
            realm.date_proto,
            realm.regexp_proto,
            realm.array_buffer_proto,
            realm.data_view_proto,
            realm.text_encoder_proto,
            realm.text_decoder_proto,
            realm.async_generator_proto,
            realm.bigint_proto,
            realm.generator_function_proto,
            realm.async_function_proto,
            realm.async_generator_function_proto,
            realm.weak_ref_proto,
            realm.finalization_registry_proto,
            realm.suppressed_error_proto,
            realm.disposable_stack_proto,
            realm.async_disposable_stack_proto,
        ]
    }
}

/// What a value is, for the heap checker's messages.
fn describe(value: Value) -> String {
    let kind = match value.tag() {
        Value::OBJECT_TAG => "object",
        Value::STRING_TAG => "string",
        _ if value.is_bigint() => "bigint",
        _ => "cell",
    };
    format!("{kind} {}", value.handle())
}

/// What kind of object this is, for the heap checker's messages.
fn kind_name(kind: &Kind) -> &'static str {
    match kind {
        Kind::Free => "free",
        Kind::Plain => "plain",
        Kind::Array(_) => "array",
        Kind::Arguments(_) => "arguments",
        Kind::Function { .. } => "function",
        Kind::Native(_) => "native function",
        Kind::Bound(_) => "bound function",
        Kind::Error => "error",
        Kind::Boxed(_) => "boxed",
        Kind::Date(_) => "date",
        Kind::Map(_) => "map",
        Kind::Set(_) => "set",
        Kind::WeakMap(_) => "weak map",
        Kind::WeakSet(_) => "weak set",
        Kind::Promise(_) => "promise",
        Kind::Generator(_) => "generator",
        Kind::Iter(_) => "iterator",
        Kind::Accessor { .. } => "accessor",
        Kind::RegExp(_) => "regexp",
        Kind::ArrayBuffer(_) => "array buffer",
        Kind::WeakRef(_) => "weak ref",
        Kind::FinalizationRegistry(_) => "finalization registry",
        Kind::TypedArray(_) => "typed array",
        Kind::DataView(_) => "data view",
        Kind::Proxy(_) => "proxy",
        Kind::BigInt(_) => "bigint",
    }
}
