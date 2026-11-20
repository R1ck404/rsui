//! Strings, and atoms: strings that are kept once each, so comparing two is comparing two numbers.

use std::cell::{Cell, UnsafeCell};
use std::rc::Rc;

use crate::arena::Arena;

/// A string's characters. Text that's all ASCII is always `Ascii`, so two equal strings are always the same kind.
#[derive(Debug)]
pub(crate) enum Text {
    /// Short ASCII text, kept right here with no allocation. Unused bytes are zero, and text this short is always this.
    Small {
        length: u8,
        bytes: [u8; SMALL_TEXT],
    },
    Ascii(Box<[u8]>),
    Wide(Box<[u16]>),
    /// Long text being built a piece at a time, which is the start of a buffer it can share with what's built next.
    GrowingAscii(Growing<u8>),
    GrowingWide(Growing<u16>),
}

/// As many bytes as fit in a `Text` without making it bigger.
const SMALL_TEXT: usize = 22;
/// Joined text this long starts a buffer to grow in. Copying anything shorter costs next to nothing.
const GROW_FROM: usize = 256;

/// The first part of a buffer shared by every step of building up some text. `out += piece` only has to add the piece
/// to the end, which is what stops building text from being quadratic. A step that isn't at the end of the buffer any
/// more, because something else was already added after it, just copies.
pub(crate) struct Growing<T> {
    buffer: Rc<Buffer<T>>,
    length: u32,
}

struct Buffer<T> {
    items: UnsafeCell<Vec<T>>,
    /// Whether it's been added to since the collector last came by.
    added_to: Cell<bool>,
    /// The last [`Tally`] that counted this buffer, so that one counts it once, however many strings share it.
    counted: Cell<u32>,
    /// Whether it's text that strings with the same text share ([`share_text`]), which nothing adds to: adding to one of
    /// them copies it, as adding to text that isn't in a buffer does, so that they don't keep what's added alive.
    same_text: bool,
}

thread_local! {
    static TALLIES: Cell<u32> = const { Cell::new(0) };
}

/// One count of what strings take up (a collection, a census, or [`crate::MemoryUse`]), which counts a buffer that
/// many strings share once, rather than once for each of them. Text built a piece at a time leaves a string for each
/// step, which can all be alive: Lexical keeps about a thousand steps of its whole document's text while it reconciles
/// one, and counting each as the whole buffer once had the collector take 789 KB of text for 6.5 GB, and not collect
/// again before the process had grown by 600 MB.
#[derive(Clone, Copy)]
pub(crate) struct Tally(u32);

impl Tally {
    pub(crate) fn new() -> Self {
        // Buffers start counted by 0, which no tally is.
        Self(TALLIES.with(|tallies| {
            let next = tallies.get().wrapping_add(1).max(1);
            tallies.set(next);
            next
        }))
    }
}

#[allow(unsafe_code)]
impl<T: Copy> Growing<T> {
    /// Text that other strings with the same text are to share, in a buffer of its own that nothing's adding to.
    fn shared(items: Vec<T>) -> Self {
        let length = items.len() as u32;
        Self {
            buffer: Rc::new(Buffer {
                items: UnsafeCell::new(items),
                added_to: Cell::new(false),
                counted: Cell::new(0),
                same_text: true,
            }),
            length,
        }
    }

    /// The same text, in the same buffer.
    fn share(&self) -> Self {
        Self {
            buffer: self.buffer.clone(),
            length: self.length,
        }
    }

    fn new(items: Vec<T>) -> Self {
        let length = items.len() as u32;
        Self {
            buffer: Rc::new(Buffer {
                items: UnsafeCell::new(items),
                added_to: Cell::new(true),
                counted: Cell::new(0),
                same_text: false,
            }),
            length,
        }
    }

    #[inline]
    fn as_slice(&self) -> &[T] {
        // SAFETY: the buffer only changes in `append`, which `Strings::concat` calls with `&mut Strings`. That can't
        // happen while this borrow lasts, as a `Growing` only lives in `Strings`, and cloning one makes a plain copy.
        // Nothing before `length` ever changes.
        unsafe { (&*self.buffer.items.get()).get_unchecked(..self.length as usize) }
    }

    fn is_at_the_end(&self) -> bool {
        // SAFETY: as in `as_slice`, this borrow is over before anything could change the buffer.
        self.length as usize == unsafe { (&*self.buffer.items.get()).len() }
    }

    /// What the buffer takes up, the first time `tally` asks for any string in it, and nothing after that.
    fn bytes_once(&self, tally: Tally) -> usize {
        if self.buffer.counted.replace(tally.0) == tally.0 {
            return 0;
        }
        // SAFETY: as in `as_slice`, this borrow is over before anything could change the buffer.
        unsafe { (&*self.buffer.items.get()).capacity() * size_of::<T>() }
    }

    fn shares_buffer_with(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.buffer, &other.buffer)
    }

    /// The text with some more added, or nothing if this isn't where the buffer ends.
    fn append(&self, added: usize, fill: impl FnOnce(&mut Vec<T>)) -> Option<Self> {
        let length = self.length as usize + added;
        if self.buffer.same_text || !self.is_at_the_end() || length > u32::MAX as usize {
            return None;
        }
        // SAFETY: only `Strings::concat` comes here, holding `&mut Strings`, so no slice of this buffer is around,
        // and it has made sure that what `fill` adds doesn't come from this buffer.
        let buffer = unsafe { &mut *self.buffer.items.get() };
        self.buffer.added_to.set(true);
        if buffer.capacity() < length {
            buffer.reserve_exact(length / 2 + added);
        }
        fill(buffer);
        Some(Self {
            buffer: self.buffer.clone(),
            length: length as u32,
        })
    }

    /// The items by themselves, if nothing else has a share of the buffer and it has a lot of room left over. Text
    /// that's still being added to keeps its room, or each collection would make the next addition copy it all.
    fn settled(&self) -> Option<Box<[T]>> {
        if self.buffer.added_to.replace(false) || Rc::strong_count(&self.buffer) != 1 {
            return None;
        }
        let length = self.length as usize;
        // SAFETY: nothing else has a share, and this is only called with `&mut Strings`, so nothing's looking at it.
        let buffer = unsafe { &*self.buffer.items.get() };
        if buffer.capacity() - length <= length / 4 {
            return None;
        }
        Some(buffer[..length].into())
    }
}

impl<T> std::fmt::Debug for Growing<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Growing({})", self.length)
    }
}

impl PartialEq for Text {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        match (self.ascii(), other.ascii()) {
            (Some(a), Some(b)) => a == b,
            (None, None) => self.wide() == other.wide(),
            _ => false,
        }
    }
}

impl Eq for Text {}

impl Clone for Text {
    /// A copy never shares a buffer: the only text that can be in one is what's in the list of strings.
    fn clone(&self) -> Self {
        match self {
            Self::Small { length, bytes } => Self::Small {
                length: *length,
                bytes: *bytes,
            },
            Self::Ascii(bytes) => Self::Ascii(bytes.clone()),
            Self::Wide(units) => Self::Wide(units.clone()),
            Self::GrowingAscii(growing) => Self::Ascii(growing.as_slice().into()),
            Self::GrowingWide(growing) => Self::Wide(growing.as_slice().into()),
        }
    }
}

impl Text {
    pub(crate) fn empty() -> Self {
        Self::Small {
            length: 0,
            bytes: [0; SMALL_TEXT],
        }
    }

    /// Text from bytes that are all ASCII.
    pub(crate) fn from_ascii(ascii: &[u8]) -> Self {
        if ascii.len() <= SMALL_TEXT {
            let mut bytes = [0; SMALL_TEXT];
            bytes[..ascii.len()].copy_from_slice(ascii);
            Self::Small {
                length: ascii.len() as u8,
                bytes,
            }
        } else {
            Self::Ascii(ascii.into())
        }
    }

    /// Text from a list of bytes that are all ASCII, which doesn't need copying if it's long.
    pub(crate) fn from_ascii_vec(ascii: Vec<u8>) -> Self {
        if ascii.len() <= SMALL_TEXT {
            Self::from_ascii(&ascii)
        } else {
            Self::Ascii(ascii.into_boxed_slice())
        }
    }

    /// Text made by changing each byte of some ASCII text into another ASCII byte.
    pub(crate) fn map_ascii(ascii: &[u8], change: impl Fn(u8) -> u8) -> Self {
        if ascii.len() <= SMALL_TEXT {
            let mut bytes = [0; SMALL_TEXT];
            for (to, from) in bytes.iter_mut().zip(ascii) {
                *to = change(*from);
            }
            Self::Small {
                length: ascii.len() as u8,
                bytes,
            }
        } else {
            Self::Ascii(ascii.iter().map(|byte| change(*byte)).collect())
        }
    }

    /// The bytes of this text, if it's all ASCII.
    #[inline]
    pub(crate) fn ascii(&self) -> Option<&[u8]> {
        match self {
            Self::Small { length, bytes } => Some(&bytes[..*length as usize]),
            Self::Ascii(bytes) => Some(bytes),
            Self::GrowingAscii(growing) => Some(growing.as_slice()),
            Self::Wide(_) | Self::GrowingWide(_) => None,
        }
    }

    /// The units of this text, if it isn't all ASCII.
    #[inline]
    pub(crate) fn wide(&self) -> Option<&[u16]> {
        match self {
            Self::Wide(units) => Some(units),
            Self::GrowingWide(growing) => Some(growing.as_slice()),
            _ => None,
        }
    }

    /// How many bytes it has elsewhere in memory, as one of the strings that `tally` counts: text that shares a
    /// buffer with others counts the whole buffer once, with the first of them, rather than each of them counting as
    /// much as it's using of it. See [`Tally`].
    pub(crate) fn heap_bytes_in(&self, tally: Tally) -> usize {
        match self {
            Self::GrowingAscii(growing) => growing.bytes_once(tally),
            Self::GrowingWide(growing) => growing.bytes_once(tally),
            _ => self.heap_bytes(),
        }
    }

    /// How many bytes it has elsewhere in memory, for a string by itself: one that shares a buffer counts what it's
    /// using of it. A count of many strings uses [`Text::heap_bytes_in`].
    pub(crate) fn heap_bytes(&self) -> usize {
        match self {
            Self::Small { .. } => 0,
            Self::Ascii(bytes) => bytes.len(),
            Self::Wide(units) => units.len() * 2,
            Self::GrowingAscii(growing) => growing.length as usize,
            Self::GrowingWide(growing) => growing.length as usize * 2,
        }
    }

    /// Has `second`, which has the same text as `first`, use the memory `first`'s text is in, which `first` then keeps in a
    /// buffer that can be shared ([`Growing`]), if it didn't already. Text built a piece at a time shares a buffer the same way,
    /// so nothing that reads or adds to text is any different.
    ///
    /// Only text that's settled is shared: text in a buffer that's being added to is left as it is, as sharing it would have
    /// each addition copy all of it.
    fn share(first: &mut Text, second: &mut Text) {
        let settled = |text: &Text| match text {
            Self::Ascii(_) | Self::Wide(_) => true,
            Self::GrowingAscii(growing) => growing.buffer.same_text,
            Self::GrowingWide(growing) => growing.buffer.same_text,
            Self::Small { .. } => false,
        };
        if !matches!(second, Self::Ascii(_) | Self::Wide(_)) || !settled(first) {
            return;
        }
        match first {
            Self::Ascii(bytes) => {
                let bytes = std::mem::take(bytes);
                *first = Self::GrowingAscii(Growing::shared(bytes.into_vec()));
            }
            Self::Wide(units) => {
                let units = std::mem::take(units);
                *first = Self::GrowingWide(Growing::shared(units.into_vec()));
            }
            _ => {}
        }
        match (&*first, &*second) {
            (Self::GrowingAscii(one), Self::Ascii(_)) => *second = Self::GrowingAscii(one.share()),
            (Self::GrowingWide(one), Self::Wide(_)) => *second = Self::GrowingWide(one.share()),
            _ => {}
        }
    }

    /// Stops holding on to room for growing that it isn't using, once this is all that's left of what was being built.
    pub(crate) fn settle(&mut self) {
        let settled = match self {
            Self::GrowingAscii(growing) => growing.settled().map(Self::Ascii),
            Self::GrowingWide(growing) => growing.settled().map(Self::Wide),
            _ => None,
        };
        if let Some(settled) = settled {
            *self = settled;
        }
    }

    pub(crate) fn from_units(units: &[u16]) -> Self {
        if units.iter().all(|unit| *unit < 0x80) {
            if units.len() <= SMALL_TEXT {
                let mut bytes = [0; SMALL_TEXT];
                for (to, unit) in bytes.iter_mut().zip(units) {
                    *to = *unit as u8;
                }
                return Self::Small {
                    length: units.len() as u8,
                    bytes,
                };
            }
            Self::Ascii(units.iter().map(|unit| *unit as u8).collect())
        } else {
            Self::Wide(units.into())
        }
    }

    pub(crate) fn from_str(text: &str) -> Self {
        if text.is_ascii() {
            Self::from_ascii(text.as_bytes())
        } else {
            Self::Wide(text.encode_utf16().collect())
        }
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Small { length, .. } => *length as usize,
            Self::Ascii(bytes) => bytes.len(),
            Self::Wide(units) => units.len(),
            Self::GrowingAscii(growing) => growing.length as usize,
            Self::GrowingWide(growing) => growing.length as usize,
        }
    }

    #[inline]
    pub(crate) fn at(&self, index: usize) -> u16 {
        match self {
            Self::Small { bytes, .. } => u16::from(bytes[index]),
            Self::Ascii(bytes) => u16::from(bytes[index]),
            Self::Wide(units) => units[index],
            Self::GrowingAscii(growing) => u16::from(growing.as_slice()[index]),
            Self::GrowingWide(growing) => growing.as_slice()[index],
        }
    }

    pub(crate) fn units(&self) -> Vec<u16> {
        match (self.ascii(), self.wide()) {
            (Some(bytes), _) => bytes.iter().map(|byte| u16::from(*byte)).collect(),
            (_, units) => units.unwrap_or(&[]).to_vec(),
        }
    }

    pub(crate) fn to_rust(&self) -> String {
        match (self.ascii(), self.wide()) {
            (Some(bytes), _) => String::from_utf8_lossy(bytes).into_owned(),
            (_, units) => String::from_utf16_lossy(units.unwrap_or(&[])),
        }
    }

    pub(crate) fn slice(&self, start: usize, end: usize) -> Self {
        match (self.ascii(), self.wide()) {
            (Some(bytes), _) => Self::from_ascii(&bytes[start..end]),
            (_, units) => Self::from_units(&units.unwrap_or(&[])[start..end]),
        }
    }

    pub(crate) fn concat(&self, other: &Self) -> Self {
        match (self.ascii(), other.ascii()) {
            (Some(a), Some(b)) => {
                let total = a.len() + b.len();
                if total <= SMALL_TEXT {
                    let mut bytes = [0; SMALL_TEXT];
                    bytes[..a.len()].copy_from_slice(a);
                    bytes[a.len()..total].copy_from_slice(b);
                    return Self::Small {
                        length: total as u8,
                        bytes,
                    };
                }
                let mut out = Vec::with_capacity(total);
                out.extend_from_slice(a);
                out.extend_from_slice(b);
                Self::Ascii(out.into_boxed_slice())
            }
            _ => {
                let mut out = Vec::with_capacity(self.len() + other.len());
                self.push_to(&mut out);
                other.push_to(&mut out);
                Self::Wide(out.into_boxed_slice())
            }
        }
    }

    pub(crate) fn push_to(&self, out: &mut Vec<u16>) {
        match (self.ascii(), self.wide()) {
            (Some(bytes), _) => out.extend(bytes.iter().map(|byte| u16::from(*byte))),
            (_, units) => out.extend_from_slice(units.unwrap_or(&[])),
        }
    }

    fn hash(&self) -> u32 {
        let mut hash = 0x811C_9DC5u32;
        match (self.ascii(), self.wide()) {
            (Some(bytes), _) => {
                for byte in bytes {
                    hash = (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193);
                }
            }
            (_, units) => {
                for unit in units.unwrap_or(&[]) {
                    hash = (hash ^ u32::from(*unit)).wrapping_mul(0x0100_0193);
                }
            }
        }
        hash ^ (hash >> 15)
    }

    /// The array index this spells, in the only way of spelling one: `"5"`, but not `"05"`.
    fn array_index(&self) -> u32 {
        let Some(bytes) = self.ascii() else {
            return NO_INDEX;
        };
        if bytes.is_empty() || bytes.len() > 10 || (bytes[0] == b'0' && bytes.len() > 1) {
            return NO_INDEX;
        }
        let mut value = 0u64;
        for byte in bytes {
            if !byte.is_ascii_digit() {
                return NO_INDEX;
            }
            value = value * 10 + u64::from(*byte - b'0');
        }
        if value >= u64::from(NO_INDEX) {
            return NO_INDEX;
        }
        value as u32
    }
}

pub(crate) const NO_INDEX: u32 = u32::MAX;

#[derive(Debug)]
pub(crate) struct StringData {
    pub(crate) text: Text,
    /// The atom that has this text, if it's been needed.
    pub(crate) atom: u32,
    /// What the text hashes to, or zero if that hasn't been needed yet.
    hash: Cell<u32>,
}

impl StringData {
    /// What a free place in the list of strings holds.
    pub(crate) fn free() -> Self {
        Self {
            text: Text::empty(),
            atom: 0,
            hash: Cell::new(0),
        }
    }

    /// What the text hashes to. It only has to be worked out once.
    #[inline]
    pub(crate) fn hash(&self) -> u32 {
        let known = self.hash.get();
        if known != 0 {
            return known;
        }
        let hash = self.text.hash().max(1);
        self.hash.set(hash);
        hash
    }
}

/// Every string in the engine, in a list that reuses places, so a string is known by a 32-bit number.
#[derive(Debug, Default)]
pub(crate) struct Strings {
    pub(crate) items: Arena<StringData>,
    pub(crate) free: crate::gc::FreeSlots,
    pub(crate) marks: Vec<u64>,
}

thread_local! {
    /// The last string kept with each text, by what the text hashes to and how long it is: see [`share_text`]. It's here
    /// rather than in [`Strings`], as a field more in `Vm` moves the ones after it, which had js-bench's interpreter 17%
    /// slower on `prop_mono` (see "Many properties at once" in `DESIGN.md`). Engines on one thread share it, which is as
    /// sound as one engine using it: a place is only used once its text has been compared, in the engine that's collecting.
    static SAME_TEXT: std::cell::RefCell<std::collections::HashMap<u64, u32, foldhash::fast::FixedState>> =
        std::cell::RefCell::default();
}

/// Whether strings with the same text are to share it ([`share_text`]), as [`crate::share_same_text`] says, unless
/// `RSUI_SHARE_TEXT` (`0` or `1`) says otherwise.
static SHARE_SAME_TEXT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn set_sharing_same_text(on: bool) {
    SHARE_SAME_TEXT.store(on, std::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn sharing_same_text() -> bool {
    static SAID: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    let said = *SAID.get_or_init(|| std::env::var_os("RSUI_SHARE_TEXT").map(|value| value != "0"));
    said.unwrap_or_else(|| SHARE_SAME_TEXT.load(std::sync::atomic::Ordering::Relaxed))
}

/// Text at least this long, and at most [`SHARED_UP_TO`], is shared with the strings that have the same text. Shorter isn't
/// worth looking for, and longer is text a program is working on (a document, JSON, what it's building a piece at a time),
/// which hashing and sharing had js-bench's `json` and `string_build` a quarter slower; what an app makes again and again
/// (class names, styles, labels) is in between.
const SHARED_FROM: usize = 32;
const SHARED_UP_TO: usize = 1024;
/// The most places [`SAME_TEXT`] notes before it starts again, about a megabyte.
const MOST_NOTED: usize = 64 * 1024;


/// Has the string at `at`, which a collection has kept for the first time, share the memory its text is in with the last
/// string kept that has the same text, if there is one. Apps make the same text again and again (the long list's rows each
/// join the same classes for their `className`, which was 1.6 MB of its 1.8 MB of text), and what's kept of it is then one
/// buffer. A place that's been used again since it was noted is told apart by its text, which is compared. What's noted is
/// let go of past [`MOST_NOTED`] places. (Letting go of it at each full collection too had shadcn's page 2.6 MB higher, as
/// it makes the same text again across them.)
pub(crate) fn share_text(items: &mut [StringData], at: usize) {
    if !sharing_same_text() {
        return;
    }
    let Some(data) = items.get(at) else {
        return;
    };
    let length = data.text.len();
    if !(SHARED_FROM..=SHARED_UP_TO).contains(&data.text.heap_bytes()) {
        return;
    }
    let key = (u64::from(data.hash()) << 32) | length as u64;
    let noted = SAME_TEXT.try_with(|same| {
        let Ok(mut same) = same.try_borrow_mut() else {
            return None;
        };
        let other = same.insert(key, at as u32);
        if other.is_none() && same.len() > MOST_NOTED {
            same.clear();
        }
        other
    });
    let Ok(Some(other)) = noted else {
        return;
    };
    let other = other as usize;
    if other == at || other >= items.len() || items[other].text != items[at].text {
        return;
    }
    let (first, second) = if other < at {
        let (low, high) = items.split_at_mut(at);
        (&mut low[other], &mut high[0])
    } else {
        let (low, high) = items.split_at_mut(other);
        (&mut high[0], &mut low[at])
    };
    Text::share(&mut first.text, &mut second.text);
}

impl Strings {
    #[allow(unsafe_code)]
    #[inline]
    pub(crate) fn add(&mut self, text: Text) -> u32 {
        let data = StringData {
            text,
            atom: 0,
            hash: Cell::new(0),
        };
        if let Some(handle) = self.free.pop() {
            debug_assert_eq!(self.items[handle as usize].text.len(), 0);
            // SAFETY: what's on the free list is in the list of strings, and has only empty text in it to forget.
            unsafe { self.items.as_mut_ptr().add(handle as usize).write(data) };
            return handle;
        }
        // Pages given back after a full collection are used again before the list grows (see `giveback.rs`).
        let data = if self.free.holes().pages() != 0 {
            match self.place_in_hole(data) {
                Ok(handle) => return handle,
                Err(data) => data,
            }
        } else {
            data
        };
        self.items.push(data);
        (self.items.len() - 1) as u32
    }

    /// Two strings joined, neither of them empty, and how much new memory that took. When it can, this adds to the end
    /// of a buffer rather than copying.
    pub(crate) fn concat(&mut self, a: u32, b: u32) -> (Text, usize) {
        let left = &self.items[a as usize].text;
        let right = &self.items[b as usize].text;
        let total = left.len() + right.len();
        if total < GROW_FROM {
            let joined = left.concat(right);
            let bytes = joined.heap_bytes();
            return (joined, bytes);
        }
        match (left, right.ascii()) {
            (Text::GrowingAscii(growing), Some(more)) => {
                // Text added to itself is the one time what's being added could be in the same buffer.
                let same =
                    matches!(right, Text::GrowingAscii(other) if other.shares_buffer_with(growing));
                if !same {
                    let next = growing.append(more.len(), |buffer| buffer.extend_from_slice(more));
                    if let Some(next) = next {
                        return (Text::GrowingAscii(next), more.len());
                    }
                }
            }
            (Text::GrowingWide(growing), _) => {
                let same =
                    matches!(right, Text::GrowingWide(other) if other.shares_buffer_with(growing));
                if !same {
                    let next = growing.append(right.len(), |buffer| right.push_to(buffer));
                    if let Some(next) = next {
                        return (Text::GrowingWide(next), right.len() * 2);
                    }
                }
            }
            _ => {}
        }
        let room = total + total / 2;
        match (left.ascii(), right.ascii()) {
            (Some(first), Some(second)) => {
                let mut bytes = Vec::with_capacity(room);
                bytes.extend_from_slice(first);
                bytes.extend_from_slice(second);
                (Text::GrowingAscii(Growing::new(bytes)), room)
            }
            _ => {
                let mut units = Vec::with_capacity(room);
                left.push_to(&mut units);
                right.push_to(&mut units);
                (Text::GrowingWide(Growing::new(units)), room * 2)
            }
        }
    }
}

/// A property name or a symbol.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct Atom(pub(crate) u32);

/// An atom with this bit set is a whole number, held right in the atom. Reading `object[i]` on a plain object can't
/// fill the table with numbers that way, and there's no string for one until something wants to see it.
const INDEX_ATOM: u32 = 1 << 31;

impl Atom {
    /// A number for this atom, for keeping something about it in a table. Numbers that aren't small are whole
    /// numbers used as names.
    #[inline]
    pub fn number(self) -> u32 {
        self.0
    }

    /// The atom for a whole number, if it's small enough to be one of these.
    #[inline]
    pub(crate) fn from_index(index: usize) -> Option<Self> {
        (index < INDEX_ATOM as usize).then_some(Self(INDEX_ATOM | index as u32))
    }

    /// The whole number this atom holds.
    #[inline]
    pub(crate) fn held_index(self) -> Option<u32> {
        (self.0 & INDEX_ATOM != 0).then_some(self.0 & !INDEX_ATOM)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AtomKind {
    Name,
    Symbol,
    /// A `#name` in a class, or something only the engine knows of. Nothing can list it.
    Hidden,
}

struct AtomEntry {
    /// The string with this name, or a symbol's description. It stays for as long as the atom does.
    string: u32,
    index: u32,
    kind: AtomKind,
    /// A symbol's description may be undefined.
    described: bool,
    /// Kept whether anything has it or not: the engine's own names, what the code names, `Symbol.for`'s symbols and
    /// what Rust asked for by name (see [`Atoms::intern`]).
    pinned: bool,
    /// Whether this place is free to be used for another atom.
    free: bool,
}

impl AtomEntry {
    const FREE: Self = Self {
        string: 0,
        index: NO_INDEX,
        kind: AtomKind::Hidden,
        described: false,
        pinned: false,
        free: true,
    };
}

/// Each atom knows its text through the strings list, so a name is stored once.
///
/// An atom is kept by a full collection for as long as something has it: a shape with it as a property's name, a symbol
/// value, or the list of what natives are holding, or if it's pinned. The rest are freed (see `Vm::collect` in gc.rs)
/// and their places are used again, so whatever remembers an atom by its number without the collector knowing has to
/// forget the freed ones then: a string's `atom`, what reads remember, and whatever goes by [`Atoms::generation`].
pub(crate) struct Atoms {
    entries: Vec<AtomEntry>,
    /// A hash table of atoms, by their text. It's probed in a line, and nothing in it is ever just marked as gone: an
    /// atom that's freed is taken out, and what comes after it moves back to where it would have been.
    table: Vec<u32>,
    names: u32,
    /// The free places in `entries`, lowest last, so that the lowest is used first.
    free: Vec<u32>,
    /// Goes up each time a collection frees atoms, for what remembers atoms by their numbers, such as a host's table of
    /// what each name means.
    pub(crate) generation: u32,
    /// The atoms made since the collector last looked, whose strings a small collection has to keep (see `Vm::collect`
    /// in gc.rs). Every other atom's string is old by then.
    added: Vec<u32>,
}

/// What a text hashes to, which is where its atom goes in the table. It's what [`StringData::hash`] keeps.
#[inline]
fn hash_of(text: &Text) -> u32 {
    text.hash().max(1)
}

macro_rules! well_known {
    ($($name:ident = $text:expr,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms, dead_code)]
        #[repr(u32)]
        enum WellKnown { Reserved, $($name,)* }
        $(pub(crate) const $name: Atom = Atom(WellKnown::$name as u32);)*
        const WELL_KNOWN: &[&str] = &[$($text,)*];
    };
}

well_known! {
    LENGTH = "length", PROTOTYPE = "prototype", CONSTRUCTOR = "constructor", NAME = "name",
    MESSAGE = "message", STACK = "stack", CAUSE = "cause", ERRORS = "errors", VALUE = "value", DONE = "done", NEXT = "next",
    RETURN = "return", GET = "get", SET = "set", TO_STRING = "toString",
    VALUE_OF = "valueOf", TO_JSON = "toJSON", THEN = "then", CALLEE = "callee", RAW = "raw", DEFAULT = "default",
    WRITABLE = "writable", ENUMERABLE = "enumerable", CONFIGURABLE = "configurable", INDEX = "index", INPUT = "input",
    LAST_INDEX = "lastIndex", GROUPS = "groups", EMPTY_NAME = "",
    SYMBOL_ITERATOR = "@@iterator", SYMBOL_HAS_INSTANCE = "@@hasInstance", SYMBOL_TO_PRIMITIVE = "@@toPrimitive",
    SYMBOL_TO_STRING_TAG = "@@toStringTag", SYMBOL_ASYNC_ITERATOR = "@@asyncIterator",
    SYMBOL_MATCH = "@@match", SYMBOL_MATCH_ALL = "@@matchAll", SYMBOL_REPLACE = "@@replace",
    SYMBOL_SEARCH = "@@search", SYMBOL_SPLIT = "@@split", SYMBOL_SPECIES = "@@species",
    SYMBOL_UNSCOPABLES = "@@unscopables", SYMBOL_IS_CONCAT_SPREADABLE = "@@isConcatSpreadable",
    FIELDS = "%fields", PRIVATE_METHODS = "%private_methods",
    TYPE_NUMBER = "number", TYPE_STRING = "string", TYPE_BOOLEAN = "boolean", TYPE_UNDEFINED = "undefined",
    TYPE_SYMBOL = "symbol", TYPE_BIGINT = "bigint", TYPE_FUNCTION = "function", TYPE_OBJECT = "object",
    WORD_NULL = "null", WORD_TRUE = "true", WORD_FALSE = "false",
    SYMBOL_DISPOSE = "@@dispose", SYMBOL_ASYNC_DISPOSE = "@@asyncDispose",
    // The names of a proxy's traps, which are looked up on its handler each time one is used.
    TRAP_HAS = "has", TRAP_DELETE_PROPERTY = "deleteProperty", TRAP_OWN_KEYS = "ownKeys",
    TRAP_GET_OWN_PROPERTY_DESCRIPTOR = "getOwnPropertyDescriptor", TRAP_DEFINE_PROPERTY = "defineProperty",
    TRAP_APPLY = "apply", TRAP_CONSTRUCT = "construct", TRAP_GET_PROTOTYPE_OF = "getPrototypeOf",
    // What `Object.prototype.toString` gives for each built-in kind of thing, when nothing says otherwise.
    TAG_OBJECT = "[object Object]", TAG_ARRAY = "[object Array]", TAG_FUNCTION = "[object Function]",
    TAG_ERROR = "[object Error]", TAG_STRING = "[object String]", TAG_NUMBER = "[object Number]",
    TAG_BOOLEAN = "[object Boolean]", TAG_DATE = "[object Date]", TAG_REGEXP = "[object RegExp]",
    TAG_ARGUMENTS = "[object Arguments]", TAG_UNDEFINED = "[object Undefined]", TAG_NULL = "[object Null]",
    FLAGS = "flags", SOURCE = "source",
}

/// The atoms every engine starts with, numbered the same in each: an atom below this is the same one in any engine.
pub(crate) const FIXED_ATOMS: u32 = 1 + WELL_KNOWN.len() as u32;

impl Atoms {
    pub(crate) fn new(strings: &mut Strings) -> Self {
        let mut atoms = Self {
            entries: vec![AtomEntry {
                pinned: true,
                free: false,
                ..AtomEntry::FREE
            }],
            table: vec![0; 1024],
            names: 0,
            free: Vec::new(),
            generation: 0,
            added: Vec::new(),
        };
        for name in WELL_KNOWN {
            if let Some(symbol) = name.strip_prefix("@@") {
                let description = strings.add(Text::from_str(&format!("Symbol.{symbol}")));
                let atom = atoms.new_symbol(Some(description), AtomKind::Symbol);
                atoms.pin(atom);
            } else if name.starts_with('%') {
                let description = strings.add(Text::from_str(name));
                let atom = atoms.new_symbol(Some(description), AtomKind::Hidden);
                atoms.pin(atom);
            } else {
                atoms.intern(strings, Text::from_str(name));
            }
        }
        atoms
    }

    /// The atom with this text, which is pinned: this is for the names the engine itself uses, the ones in the code,
    /// and what Rust asks for by name, of which there are only so many. A name made up while running is
    /// [`intern_key`](Self::intern_key)'s.
    pub(crate) fn intern(&mut self, strings: &mut Strings, text: Text) -> Atom {
        let atom = self.intern_key(strings, text);
        self.pin(atom);
        atom
    }

    /// The atom with this text, which goes once nothing has it.
    pub(crate) fn intern_key(&mut self, strings: &mut Strings, text: Text) -> Atom {
        if let Some(atom) = Atom::from_index(text.array_index() as usize) {
            return atom;
        }
        let mask = self.table.len() - 1;
        let mut at = hash_of(&text) as usize & mask;
        loop {
            let found = self.table[at];
            if found == 0 {
                break;
            }
            let entry = &self.entries[found as usize];
            if strings.items[entry.string as usize].text == text {
                return Atom(found);
            }
            at = (at + 1) & mask;
        }
        let index = text.array_index();
        let string = strings.add(text);
        self.add_name(strings, string, index, at)
    }

    /// The atom for a string that's already in the list, which means that string stays for as long as the atom does.
    /// The string remembers its atom, until the atom goes.
    pub(crate) fn intern_string(&mut self, strings: &mut Strings, handle: u32) -> Atom {
        let known = strings.items[handle as usize].atom;
        if known != 0 {
            return Atom(known);
        }
        if let Some(atom) =
            Atom::from_index(strings.items[handle as usize].text.array_index() as usize)
        {
            strings.items[handle as usize].atom = atom.0;
            return atom;
        }
        let mask = self.table.len() - 1;
        let hash = strings.items[handle as usize].hash();
        let mut at = hash as usize & mask;
        loop {
            let found = self.table[at];
            if found == 0 {
                break;
            }
            let entry = &self.entries[found as usize];
            if entry.string == handle
                || strings.items[entry.string as usize].text == strings.items[handle as usize].text
            {
                strings.items[handle as usize].atom = found;
                return Atom(found);
            }
            at = (at + 1) & mask;
        }
        let index = strings.items[handle as usize].text.array_index();
        self.add_name(strings, handle, index, at)
    }

    /// Keeps an atom for as long as the engine lasts, whether anything has it or not.
    #[inline]
    pub(crate) fn pin(&mut self, atom: Atom) {
        if atom.held_index().is_none() {
            self.entries[atom.0 as usize].pinned = true;
        }
    }

    /// Puts an atom somewhere, in the lowest free place if there is one.
    fn place(&mut self, entry: AtomEntry) -> u32 {
        let atom = if let Some(atom) = self.free.pop() {
            self.entries[atom as usize] = entry;
            atom
        } else {
            self.entries.push(entry);
            (self.entries.len() - 1) as u32
        };
        self.added.push(atom);
        atom
    }

    fn add_name(&mut self, strings: &mut Strings, string: u32, index: u32, at: usize) -> Atom {
        let atom = self.place(AtomEntry {
            string,
            index,
            kind: AtomKind::Name,
            described: true,
            pinned: false,
            free: false,
        });
        strings.items[string as usize].atom = atom;
        self.table[at] = atom;
        self.names += 1;
        if self.names as usize * 2 > self.table.len() {
            self.grow(strings, self.table.len() * 2);
        }
        Atom(atom)
    }

    /// Makes the table this big, and puts every name back in it.
    fn grow(&mut self, strings: &Strings, size: usize) {
        let mut table = vec![0u32; size];
        let mask = size - 1;
        for (atom, entry) in self.entries.iter().enumerate().skip(1) {
            if entry.kind != AtomKind::Name || entry.free {
                continue;
            }
            let mut at = strings.items[entry.string as usize].hash() as usize & mask;
            while table[at] != 0 {
                at = (at + 1) & mask;
            }
            table[at] = atom as u32;
        }
        self.table = table;
    }

    /// Takes a name out of the table. What comes after it in the same run moves back into the gap if that's no further
    /// from where it would first be looked for, so a search can still stop at the first empty place.
    fn remove_name(&mut self, strings: &Strings, atom: u32) {
        let mask = self.table.len() - 1;
        let home = |entry: &AtomEntry| strings.items[entry.string as usize].hash() as usize & mask;
        let mut gap = home(&self.entries[atom as usize]);
        while self.table[gap] != atom {
            debug_assert_ne!(self.table[gap], 0, "atom {atom} isn't in the table");
            gap = (gap + 1) & mask;
        }
        let mut at = gap;
        loop {
            at = (at + 1) & mask;
            let found = self.table[at];
            if found == 0 {
                break;
            }
            let wanted = home(&self.entries[found as usize]);
            // It stays where it is if where it's first looked for is after the gap, up to where it is, going round.
            let stays = if gap <= at {
                gap < wanted && wanted <= at
            } else {
                gap < wanted || wanted <= at
            };
            if !stays {
                self.table[gap] = found;
                gap = at;
            }
        }
        self.table[gap] = 0;
        self.names -= 1;
    }

    /// A new symbol, which goes once nothing has it unless it's [`pin`](Self::pin)ned.
    pub(crate) fn new_symbol(&mut self, description: Option<u32>, kind: AtomKind) -> Atom {
        Atom(self.place(AtomEntry {
            string: description.unwrap_or(0),
            index: NO_INDEX,
            kind,
            described: description.is_some(),
            pinned: false,
            free: false,
        }))
    }

    /// The string that names this atom, or a symbol's description if it has one.
    #[inline]
    pub(crate) fn string_of(&self, atom: Atom) -> Option<u32> {
        if atom.held_index().is_some() {
            return None;
        }
        let entry = &self.entries[atom.0 as usize];
        entry.described.then_some(entry.string)
    }

    #[inline]
    pub(crate) fn index_of(&self, atom: Atom) -> Option<u32> {
        if let Some(index) = atom.held_index() {
            return Some(index);
        }
        let index = self.entries[atom.0 as usize].index;
        (index != NO_INDEX).then_some(index)
    }

    #[inline]
    pub(crate) fn kind_of(&self, atom: Atom) -> AtomKind {
        if atom.held_index().is_some() {
            return AtomKind::Name;
        }
        self.entries[atom.0 as usize].kind
    }

    /// The strings that have to stay, as atoms have them.
    pub(crate) fn strings(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries
            .iter()
            .filter(|entry| entry.described)
            .map(|entry| entry.string)
    }

    /// The strings that name the atoms made since [`Atoms::forget_added`] was last called, or describe them.
    pub(crate) fn added_strings(&self) -> impl Iterator<Item = u32> + '_ {
        self.added.iter().filter_map(|atom| {
            let entry = &self.entries[*atom as usize];
            (entry.described && !entry.free).then_some(entry.string)
        })
    }

    /// Checks that each atom's string is marked as old, as a small collection only keeps the strings of the atoms
    /// made since the last one. See `RSUI_GC_VERIFY`.
    pub(crate) fn verify_strings_kept(&self, when: &str, marks: &[u64]) {
        for (atom, entry) in self.entries.iter().enumerate() {
            if !entry.described || entry.free {
                continue;
            }
            let index = entry.string as usize;
            assert!(
                marks.get(index / 64).is_some_and(|word| word >> (index % 64) & 1 != 0)
                    || self.added.contains(&(atom as u32)),
                "{when}: atom {atom}'s string {index} is young, and a small collection wouldn't keep it"
            );
        }
    }

    /// Starts again on what [`Atoms::added_strings`] gives, once a collection has kept those strings.
    pub(crate) fn forget_added(&mut self) {
        self.added.clear();
        crate::gc::shrink_if_oversized(&mut self.added);
    }

    /// How many places there are for atoms, free or not, which is how many marks a collection needs.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// How many atoms are in use.
    pub(crate) fn live(&self) -> usize {
        self.entries.len() - self.free.len()
    }

    /// Whether this is an atom that's been freed, or never was one. One that's a whole number always is one.
    pub(crate) fn is_free(&self, atom: Atom) -> bool {
        atom.held_index().is_none()
            && self
                .entries
                .get(atom.0 as usize)
                .is_none_or(|entry| entry.free)
    }

    /// Whether this atom stays whether anything has it or not. A whole number does.
    #[inline]
    pub(crate) fn is_pinned(&self, atom: Atom) -> bool {
        atom.held_index().is_some() || self.entries[atom.0 as usize].pinned
    }

    /// After a full collection has marked the atoms in use, frees the others that aren't pinned: a name is taken out of
    /// the table, each is taken out of its string, and its place is set aside to be used again. Sets a bit in `freed`
    /// for each one freed, and says whether any were. The free places at the end go.
    pub(crate) fn sweep(
        &mut self,
        marks: &[u64],
        freed: &mut Vec<u64>,
        strings: &mut Strings,
    ) -> bool {
        let is_marked = |atom: usize| {
            marks
                .get(atom / 64)
                .is_some_and(|word| word >> (atom % 64) & 1 != 0)
        };
        freed.clear();
        freed.resize(self.entries.len() / 64 + 1, 0);
        let mut any = false;
        for atom in 1..self.entries.len() {
            let entry = &self.entries[atom];
            if entry.free || entry.pinned || is_marked(atom) {
                continue;
            }
            let (kind, string) = (entry.kind, entry.string as usize);
            if kind == AtomKind::Name {
                self.remove_name(strings, atom as u32);
            }
            if let Some(item) = strings.items.get_mut(string)
                && item.atom == atom as u32
            {
                item.atom = 0;
            }
            self.entries[atom] = AtomEntry::FREE;
            self.free.push(atom as u32);
            freed[atom / 64] |= 1 << (atom % 64);
            any = true;
        }
        if any {
            self.generation = self.generation.wrapping_add(1);
            while self.entries.last().is_some_and(|entry| entry.free) {
                self.entries.pop();
            }
            let len = self.entries.len() as u32;
            self.free.retain(|atom| *atom < len);
            // Lowest last, as that's the one used first, so the end of the list stays quiet and can go next time.
            self.free.sort_unstable_by(|a, b| b.cmp(a));
            if self.entries.capacity() > 1024 && self.entries.len() * 4 < self.entries.capacity() {
                self.entries.shrink_to(self.entries.len() * 2);
            }
            // The table gets smaller once it's less than an eighth full, to a size that's a quarter full or less.
            let mut size = self.table.len();
            while size > 1024 && self.names as usize * 8 < size {
                size /= 2;
            }
            if size < self.table.len() {
                self.grow(strings, size);
            }
        }
        any
    }

    /// Checks that the table has every name there is, where a search finds it, and nothing else, that each atom's
    /// string is there, and that the free list is right. Panics if not. See `RSUI_GC_VERIFY`.
    pub(crate) fn verify(&self, when: &str, strings: &Strings) {
        let mut names = 0;
        for (at, &atom) in self.table.iter().enumerate() {
            if atom == 0 {
                continue;
            }
            let entry = &self.entries[atom as usize];
            assert!(
                !entry.free && entry.kind == AtomKind::Name,
                "{when}: place {at} in the table of names has atom {atom}, which is free or isn't a name"
            );
            names += 1;
        }
        assert_eq!(
            names, self.names,
            "{when}: the table of names has {names} of the {} names",
            self.names
        );
        let free: std::collections::HashSet<u32> = self.free.iter().copied().collect();
        for (atom, entry) in self.entries.iter().enumerate().skip(1) {
            if entry.free {
                assert!(
                    free.contains(&(atom as u32)),
                    "{when}: atom {atom} is free but can't be used again"
                );
                continue;
            }
            assert!(
                !free.contains(&(atom as u32)),
                "{when}: atom {atom} is on the free list but in use"
            );
            if entry.described {
                assert!(
                    (entry.string as usize) < strings.items.len()
                        && !strings.free.is_free(entry.string as usize),
                    "{when}: atom {atom} has string {}, which is free",
                    entry.string
                );
            }
            if entry.kind == AtomKind::Name {
                let text = &strings.items[entry.string as usize].text;
                let mask = self.table.len() - 1;
                let mut at = hash_of(text) as usize & mask;
                while self.table[at] != atom as u32 {
                    assert_ne!(
                        self.table[at],
                        0,
                        "{when}: name atom {atom} ({}) can't be found in the table",
                        text.to_rust()
                    );
                    at = (at + 1) & mask;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Strings, Atoms) {
        let mut strings = Strings::default();
        let atoms = Atoms::new(&mut strings);
        (strings, atoms)
    }

    #[test]
    fn the_same_text_is_the_same_atom() {
        let (mut strings, mut atoms) = setup();
        let a = atoms.intern(&mut strings, Text::from_str("hello"));
        let b = atoms.intern(&mut strings, Text::from_str("hello"));
        let c = atoms.intern(&mut strings, Text::from_str("hellp"));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn well_known_names_are_found_by_their_text() {
        let (mut strings, mut atoms) = setup();
        assert_eq!(atoms.intern(&mut strings, Text::from_str("length")), LENGTH);
        assert_eq!(atoms.intern(&mut strings, Text::from_str("then")), THEN);
        assert_eq!(atoms.kind_of(SYMBOL_ITERATOR), AtomKind::Symbol);
        assert_eq!(atoms.kind_of(FIELDS), AtomKind::Hidden);
    }

    #[test]
    fn a_string_and_its_atom_agree() {
        let (mut strings, mut atoms) = setup();
        let handle = strings.add(Text::from_str("name"));
        assert_eq!(atoms.intern_string(&mut strings, handle), NAME);
        assert_eq!(strings.items[handle as usize].atom, NAME.0);
    }

    #[test]
    fn wide_and_narrow_text_of_the_same_characters_are_equal() {
        let units: Vec<u16> = "abc".encode_utf16().collect();
        assert_eq!(Text::from_units(&units), Text::from_str("abc"));
        assert!(matches!(Text::from_str("h\u{e9}"), Text::Wide(_)));
    }

    #[test]
    fn short_text_needs_no_allocation_and_compares_like_any_other() {
        assert_eq!(size_of::<Text>(), 24);
        let short = "a".repeat(22);
        let long = "a".repeat(23);
        assert!(matches!(Text::from_str(&short), Text::Small { .. }));
        assert!(matches!(Text::from_str(&long), Text::Ascii(_)));
        let joined = Text::from_str("aaaaaaaaaaa").concat(&Text::from_str("aaaaaaaaaaa"));
        assert_eq!(joined, Text::from_str(&short));
        let joined = joined.concat(&Text::from_str("a"));
        assert_eq!(joined, Text::from_str(&long));
        assert_eq!(joined.slice(0, 22), Text::from_str(&short));
        assert_eq!(Text::from_str(&long).to_rust(), long);
        assert_eq!(Text::from_str("abc").at(1), u16::from(b'b'));
    }

    fn built_up(strings: &mut Strings, pieces: &[&str]) -> u32 {
        let mut at = strings.add(Text::from_str(pieces[0]));
        for piece in &pieces[1..] {
            let piece = strings.add(Text::from_str(piece));
            let (joined, _) = strings.concat(at, piece);
            at = strings.add(joined);
        }
        at
    }

    #[test]
    fn text_built_up_in_steps_shares_one_buffer_and_every_step_stays_as_it_was() {
        let mut strings = Strings::default();
        let long = "x".repeat(300);
        let first = built_up(&mut strings, &[&long, "a"]);
        let second = built_up(&mut strings, &[&long, "a"]);
        let piece = strings.add(Text::from_str("bc"));
        let (joined, _) = strings.concat(first, piece);
        let third = strings.add(joined);
        assert!(matches!(
            strings.items[third as usize].text,
            Text::GrowingAscii(_)
        ));
        let first_text = &strings.items[first as usize].text;
        let third_text = &strings.items[third as usize].text;
        let (Text::GrowingAscii(a), Text::GrowingAscii(b)) = (first_text, third_text) else {
            panic!("both should be growing");
        };
        assert!(a.shares_buffer_with(b));
        assert_eq!(strings.items[first as usize].text.len(), 301);
        assert_eq!(
            strings.items[third as usize].text.to_rust(),
            format!("{long}abc")
        );
        assert_eq!(
            strings.items[first as usize].text,
            strings.items[second as usize].text
        );
    }

    #[test]
    fn adding_to_a_step_that_was_already_added_to_leaves_the_other_alone() {
        let mut strings = Strings::default();
        let long = "x".repeat(300);
        let base = built_up(&mut strings, &[&long, "a"]);
        let b = strings.add(Text::from_str("b"));
        let c = strings.add(Text::from_str("c"));
        let (joined, _) = strings.concat(base, b);
        let with_b = strings.add(joined);
        let (joined, _) = strings.concat(base, c);
        let with_c = strings.add(joined);
        assert_eq!(
            strings.items[with_b as usize].text.to_rust(),
            format!("{long}ab")
        );
        assert_eq!(
            strings.items[with_c as usize].text.to_rust(),
            format!("{long}ac")
        );
        assert_eq!(
            strings.items[base as usize].text.to_rust(),
            format!("{long}a")
        );
    }

    #[test]
    fn text_added_to_itself_comes_out_right() {
        let mut strings = Strings::default();
        let long = "x".repeat(300);
        let base = built_up(&mut strings, &[&long, "a"]);
        let (joined, _) = strings.concat(base, base);
        let doubled = strings.add(joined);
        assert_eq!(
            strings.items[doubled as usize].text.to_rust(),
            format!("{long}a{long}a")
        );
    }

    #[test]
    fn building_can_go_from_ascii_to_wide_and_stay_wide() {
        let mut strings = Strings::default();
        let long = "x".repeat(300);
        let at = built_up(&mut strings, &[&long, "a", "\u{e9}", "b", "\u{e8}"]);
        let text = &strings.items[at as usize].text;
        assert!(text.ascii().is_none());
        assert_eq!(text.to_rust(), format!("{long}a\u{e9}b\u{e8}"));
        assert_eq!(*text, Text::from_str(&format!("{long}a\u{e9}b\u{e8}")));
        assert_eq!(text.clone().len(), 304);
    }

    #[test]
    fn a_growing_string_is_equal_to_the_same_text_held_any_other_way() {
        let mut strings = Strings::default();
        let long = "x".repeat(300);
        let at = built_up(&mut strings, &[&long, "a"]);
        let plain = Text::from_str(&format!("{long}a"));
        assert_eq!(strings.items[at as usize].text, plain);
        assert_eq!(strings.items[at as usize].text.hash(), plain.hash());
        assert_eq!(
            strings.items[at as usize].text.slice(299, 301),
            Text::from_str("xa")
        );
    }

    #[test]
    fn a_settled_string_keeps_its_text_and_lets_go_of_the_buffer() {
        let mut strings = Strings::default();
        let long = "x".repeat(300);
        let first = strings.add(Text::from_str(&long));
        let piece = strings.add(Text::from_str("a"));
        let (joined, _) = strings.concat(first, piece);
        let at = strings.add(joined);
        strings.items[at as usize].text.settle();
        assert!(
            matches!(strings.items[at as usize].text, Text::GrowingAscii(_)),
            "it was only just being added to"
        );
        strings.items[at as usize].text.settle();
        assert!(matches!(strings.items[at as usize].text, Text::Ascii(_)));
        assert_eq!(
            strings.items[at as usize].text.to_rust(),
            format!("{long}a")
        );
    }

    #[test]
    fn numbers_are_atoms_with_no_entry_behind_them() {
        let (mut strings, mut atoms) = setup();
        let before = strings.items.len();
        let atom = atoms.intern(&mut strings, Text::from_str("1234"));
        assert_eq!(atom, Atom::from_index(1234).unwrap());
        assert_eq!(atoms.index_of(atom), Some(1234));
        assert_eq!(atoms.kind_of(atom), AtomKind::Name);
        assert_eq!(strings.items.len(), before);
        let handle = strings.add(Text::from_str("1234"));
        assert_eq!(atoms.intern_string(&mut strings, handle), atom);
    }

    #[test]
    fn only_the_true_spelling_of_a_number_is_an_index() {
        let (mut strings, mut atoms) = setup();
        let mut index = |text: &str| {
            let atom = atoms.intern(&mut strings, Text::from_str(text));
            atoms.index_of(atom)
        };
        assert_eq!(index("0"), Some(0));
        assert_eq!(index("42"), Some(42));
        assert_eq!(index("4294967294"), Some(4_294_967_294));
        assert_eq!(index("4294967295"), None);
        assert_eq!(index("05"), None);
        assert_eq!(index("1a"), None);
        assert_eq!(index(""), None);
    }

    #[test]
    fn the_table_keeps_everything_as_it_grows() {
        let (mut strings, mut atoms) = setup();
        let first: Vec<Atom> = (0..5000)
            .map(|index| atoms.intern(&mut strings, Text::from_str(&format!("name{index}"))))
            .collect();
        for (index, atom) in first.iter().enumerate() {
            assert_eq!(
                atoms.intern(&mut strings, Text::from_str(&format!("name{index}"))),
                *atom
            );
        }
    }

    #[test]
    fn names_that_are_freed_leave_the_table_and_the_rest_are_still_found() {
        let (mut strings, mut atoms) = setup();
        let made: Vec<Atom> = (0..5000)
            .map(|index| atoms.intern_key(&mut strings, Text::from_str(&format!("name{index}"))))
            .collect();
        let pinned = atoms.intern(&mut strings, Text::from_str("pinned"));
        let symbol = atoms.new_symbol(None, AtomKind::Symbol);
        let live = atoms.live();
        // A third of them are in use, as are the engine's own, which are pinned.
        let mut marks = vec![0u64; atoms.len() / 64 + 1];
        for atom in made.iter().step_by(3) {
            marks[atom.0 as usize / 64] |= 1 << (atom.0 % 64);
        }
        let mut freed = Vec::new();
        assert!(atoms.sweep(&marks, &mut freed, &mut strings));
        atoms.verify("after sweeping", &strings);
        assert_eq!(atoms.live(), live - 3333 - 1);
        assert!(atoms.is_free(symbol));
        assert!(!atoms.is_free(pinned) && !atoms.is_free(LENGTH) && !atoms.is_free(SYMBOL_ITERATOR));
        for (index, atom) in made.iter().enumerate() {
            assert_eq!(atoms.is_free(*atom), index % 3 != 0, "name{index}");
            assert_eq!(
                strings.items[atoms.entries[atom.0 as usize].string as usize].atom == atom.0,
                index % 3 == 0
            );
        }
        // What stayed is found as the same atom, and what went comes back in the places that were freed.
        for (index, atom) in made.iter().enumerate() {
            let again = atoms.intern_key(&mut strings, Text::from_str(&format!("name{index}")));
            if index % 3 == 0 {
                assert_eq!(again, *atom, "name{index}");
            } else {
                assert!(again.0 < atoms.len() as u32);
            }
        }
        atoms.verify("after interning again", &strings);
        // The symbol was last, and its place went with it.
        assert_eq!(atoms.live(), live - 1);
        assert_eq!(atoms.len(), live - 1);
        assert_eq!(atoms.intern(&mut strings, Text::from_str("length")), LENGTH);
    }
}
