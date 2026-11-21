//! What baseline code sees as it runs, kept for an optimizing tier to compile from: which function each
//! call called, what kinds of numbers arithmetic and comparisons met, which way each branch went, what elements were
//! read and written, what each `new` made, and how hot each function is. Only with the `jit-opt` feature; the
//! interpreter records nothing, so a function that's never compiled pays nothing.
//!
//! Each compiled function has a block of it, written by its code (see `x64::Gen::note`) and by the helpers its code
//! calls out of the way (see [`record`]), which find the record from the frame and the instruction ([`site_of`]). The block is in memory the writer sets aside within 2 GB of the code
//! (`memory::CodeMemory::feedback`), so that code can write a byte of it with one instruction that names it by its
//! distance (7 bytes). It starts with an 8-byte header, the function's count to being due (see [`due`]), and has a
//! record for each instruction [`kind_of`] names, in order, at the place [`layout`] says. What the layout is depends
//! only on the instructions, and an instruction the interpreter rewrites in place (GetProp to GetPropPoly, SetElem to
//! SetElemTyped, CallMath to Call) keeps the kind of record it had, so code compiled again uses the same block.
//!
//! Nothing reads it yet but `RSUI_OPT_LOG`, which says what all of it came to when the engine goes.
#![allow(unsafe_code)]

use crate::object::Kind as ObjectKind;
use crate::op::Op;
use crate::value::Value;
use crate::vm::Vm;

/// What a number record's byte says an instruction met. Ints with an int result set none of them. `DOUBLE` is what
/// isn't that: a double, or an int result that wasn't an int32 (it overflowed, or was a zero product, which may be
/// `-0`), as V8's "number" feedback is; it's set where the template leaves its ints' way, so what isn't a number goes
/// by there too, and has `STRING` or `OTHER` as well, from the helper.
pub(crate) const DOUBLE: u8 = 1;
pub(crate) const STRING: u8 = 8;
/// Anything else: undefined, an object, a boolean, a bigint.
pub(crate) const OTHER: u8 = 16;

/// What a branch record says, as `logical` reads it: the jump was taken, or it went on to the next instruction. In the
/// record each is a byte of its own, the code's alone (`TAKEN_AT`, `FELL_AT`), which it stores 1 in: an `or` would have
/// each run of a loop wait for the last one's store.
pub(crate) const TAKEN: u8 = 1;
pub(crate) const FELL: u8 = 2;
pub(crate) const TAKEN_AT: usize = 0;
pub(crate) const FELL_AT: usize = 1;
/// The byte of a number record the code stores 1 in where it leaves its ints' way; the helpers' bits are the next.
pub(crate) const CODE_DOUBLE: usize = 0;
/// The byte of an element record the code stores 1 in for a typed array's element.
pub(crate) const TYPED_AT: usize = 2;

/// What an element record's first byte says was read or written: a hole, an index at or past the end, a typed
/// array's element (whose kind's byte is the record's second), a key that's a name (which its last four bytes say),
/// an object that's none of an array, `arguments` or a typed array, something that isn't an object, a number key
/// that isn't an index.
pub(crate) const HOLE: u8 = 1;
pub(crate) const PAST_END: u8 = 2;
pub(crate) const TYPED: u8 = 4;
pub(crate) const NAMED: u8 = 8;
pub(crate) const NOT_ARRAY: u8 = 16;
pub(crate) const PRIMITIVE: u8 = 32;
pub(crate) const ODD_KEY: u8 = 64;

/// A record's word for what it saw: nothing yet (0), one thing (its number plus one), or more than one.
pub(crate) const MANY: u32 = u32::MAX;
/// A callee that's a native, by its object's handle, with this bit.
pub(crate) const NATIVE: u32 = 1 << 31;
/// A call record's fifth byte: it called something that's neither a function nor a native (a bound function, a
/// proxy, a class), which its first word doesn't say.
pub(crate) const OTHER_CALLEE: u32 = 1;
/// Where in a call record its last eight bytes are, a flag for each callee the call's code knew and found (see
/// `x64::Gen::note_known_callee`), in the order the writer gave them out (`Compiled::feedback_knowns`): a call that
/// finds what it expected records it with one store.
pub(crate) const KNOWN_AT: usize = 8;

/// The header of a block: the count to being due, then four bytes kept for later.
pub(crate) const HEADER: u32 = 8;

/// How many calls and loop turns in baseline code make a function due for the optimizing tier, from when it has
/// baseline code; then how many more its feedback has to go unchanged for, and how many times it's waited for that
/// before it's due anyway (see [`due`]).
pub(crate) const THRESHOLD: i32 = 10_000;
pub(crate) const SETTLE: i32 = 1_000;
pub(crate) const MOST_WAITS: u8 = 3;

/// The kinds of record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    /// Two bytes: the code's `DOUBLE`, then the helpers' bits for what kinds of numbers it met.
    Numbers,
    /// Two: whether it jumped, and whether it went on.
    Branch,
    /// Four: a number record, then a branch record.
    CompareBranch,
    /// Sixteen: what it called the way that works for anything (`MANY`, a proto plus one, or a native's handle with
    /// `NATIVE`), then `OTHER_CALLEE`, three kept, then a flag for each callee it knew that it found (`KNOWN_AT`).
    Call,
    /// Eight: what it called, as a call's, then the shape of the `this` it made, plus one, or `MANY`.
    New,
    /// Eight: the helpers' bits, the typed array kind's byte, the code's `TYPED`, one kept, then the name of a key
    /// that was one, plus one, or `MANY`.
    Elem,
}

impl Kind {
    pub(crate) fn size(self) -> u32 {
        match self {
            Kind::Numbers | Kind::Branch => 2,
            Kind::CompareBranch => 4,
            Kind::Call => 16,
            Kind::New | Kind::Elem => 8,
        }
    }

    /// Whether a helper fills it in, from what it's given (see [`record`]), rather than only the code.
    pub(crate) fn from_helpers(self) -> bool {
        !matches!(self, Kind::Branch)
    }
}

/// The record an instruction has, if any. Loops' jumps back have none: their counts are the function's.
pub(crate) fn kind_of(op: &Op) -> Option<Kind> {
    Some(match *op {
        Op::Add { .. }
        | Op::Sub { .. }
        | Op::Mul { .. }
        | Op::AddInt { .. }
        | Op::SubInt { .. }
        | Op::MulInt { .. }
        | Op::AddInt32 { .. }
        | Op::AddK { .. }
        | Op::SubK { .. }
        | Op::MulK { .. }
        | Op::Div { .. }
        | Op::DivK { .. }
        | Op::DivInt { .. }
        | Op::Mod { .. }
        | Op::ModInt { .. }
        | Op::ModK { .. }
        | Op::BitAnd { .. }
        | Op::BitOr { .. }
        | Op::BitXor { .. }
        | Op::Shl { .. }
        | Op::Shr { .. }
        | Op::UShr { .. }
        | Op::BitAndInt { .. }
        | Op::BitOrInt { .. }
        | Op::ShlInt { .. }
        | Op::ShrInt { .. }
        | Op::UShrInt { .. }
        | Op::Lt { .. }
        | Op::Le { .. }
        | Op::Gt { .. }
        | Op::Ge { .. }
        | Op::Inc { .. }
        | Op::Dec { .. }
        | Op::Neg { .. } => Kind::Numbers,
        Op::JumpLt { offset, .. }
        | Op::JumpLe { offset, .. }
        | Op::JumpGt { offset, .. }
        | Op::JumpGe { offset, .. }
        | Op::JumpNotLt { offset, .. }
        | Op::JumpNotLe { offset, .. }
        | Op::JumpNotGt { offset, .. }
        | Op::JumpNotGe { offset, .. }
        | Op::JumpLtInt { offset, .. }
        | Op::JumpLeInt { offset, .. }
        | Op::JumpGtInt { offset, .. }
        | Op::JumpGeInt { offset, .. }
        | Op::JumpNotLtInt { offset, .. }
        | Op::JumpNotLeInt { offset, .. }
        | Op::JumpNotGtInt { offset, .. }
        | Op::JumpNotGeInt { offset, .. }
            if offset >= 0 =>
        {
            Kind::CompareBranch
        }
        Op::JumpIfTrue { offset, .. }
        | Op::JumpIfFalse { offset, .. }
        | Op::JumpIfNullish { offset, .. }
        | Op::JumpIfNotNullish { offset, .. }
        | Op::JumpIfUndefined { offset, .. }
        | Op::JumpIfNotUndefined { offset, .. }
        | Op::JumpIfNull { offset, .. }
        | Op::JumpIfNotNull { offset, .. }
            if offset >= 0 =>
        {
            Kind::Branch
        }
        Op::JumpStrictEq { offset, .. }
        | Op::JumpStrictNe { offset, .. }
        | Op::JumpStrictEqInt { offset, .. }
        | Op::JumpStrictNeInt { offset, .. }
        | Op::JumpEq { offset, .. }
        | Op::JumpNe { offset, .. }
        | Op::JumpTypeIs { offset, .. }
        | Op::JumpTypeIsNot { offset, .. }
            if offset >= 0 =>
        {
            Kind::Branch
        }
        Op::Call { .. } | Op::CallPlain { .. } | Op::CallMethod { .. } | Op::CallMethodPoly { .. } | Op::CallMath { .. } => {
            Kind::Call
        }
        Op::New { .. } => Kind::New,
        Op::GetElem { .. } | Op::SetElem { .. } | Op::SetElemTyped { .. } => Kind::Elem,
        _ => return None,
    })
}

/// Where each instruction's record is in its function's block (`NONE` for none), and how big the block is.
pub(crate) struct Layout {
    pub(crate) size: u32,
    pub(crate) offsets: Box<[u32]>,
}

pub(crate) const NONE: u32 = u32::MAX;

pub(crate) fn layout(ops: &[Op]) -> Layout {
    let mut size = HEADER;
    let offsets = ops
        .iter()
        .map(|op| match kind_of(op) {
            Some(kind) => {
                let at = size.next_multiple_of(kind.size());
                size = at + kind.size();
                at
            }
            None => NONE,
        })
        .collect();
    Layout {
        size: size.next_multiple_of(8),
        offsets,
    }
}

// Recording, from the helpers compiled code calls.

/// The record compiled code said is the current instruction's before it called `helpers::typed_set`
/// (`x64::Gen::set_site`), taken, so that nothing else writes it. Null if it said none.
#[inline(always)]
fn take(vm: &mut Vm) -> *mut u8 {
    std::mem::replace(&mut vm.jit.site, 0) as *mut u8
}

/// The record of instruction `pc` of the compiled function on top, which a helper doing that instruction is to fill in:
/// null if there's none, or `pc` isn't that function's (a callee compiled in line can call a helper that needs no
/// frame, with its caller's on top). The last few functions' blocks and where their records are are kept, by their
/// protos' low bits, so that finding one is a compare and a load.
pub(crate) fn site_of(vm: &mut Vm, pc: u32) -> *mut u8 {
    let Some(frame) = vm.frames.last() else {
        return std::ptr::null_mut();
    };
    let proto = frame.proto;
    let way = proto as usize % vm.jit.fb_last.len();
    if vm.jit.fb_last[way].0 != proto {
        let found = vm.jit.backend.as_deref().and_then(|backend| backend.compiled.get(&proto)).and_then(|compiled| {
            (compiled.feedback != 0).then(|| (compiled.feedback, compiled.feedback_offsets.clone()))
        });
        let Some((block, offsets)) = found else {
            return std::ptr::null_mut();
        };
        vm.jit.fb_last[way] = (proto, vm.protos[proto as usize].start, block, offsets);
    }
    let (_, start, block, offsets) = &vm.jit.fb_last[way];
    // A function that's due has the feedback it's to be compiled from: its helpers record no more (its code does,
    // which costs less). Its count is set past where it started then (see `due`).
    // SAFETY: the block's first four bytes are its count.
    if unsafe { (*block as *const i32).read() } > THRESHOLD {
        return std::ptr::null_mut();
    }
    let Some(&at) = pc.checked_sub(*start).and_then(|at| offsets.get(at as usize)) else {
        return std::ptr::null_mut();
    };
    if at == NONE {
        return std::ptr::null_mut();
    }
    (*block + at as usize) as *mut u8
}

/// What a value is, for a number record.
fn number_bits(value: Value) -> u8 {
    if value.is_int() {
        0
    } else if value.is_double() {
        DOUBLE
    } else if value.is_string() {
        STRING
    } else {
        OTHER
    }
}

/// Records what the helper doing instruction `pc` in the frame on top meets, if it has a record: what its operands
/// are, for arithmetic and comparisons; what's called, for a call or `new`; what's read or written, for an element.
/// Called at the start of each helper that does a whole instruction (`op`, `add`, `compare`, `call`, `set_elem`),
/// before anything is done.
#[inline]
pub(crate) fn record(vm: &mut Vm, pc: u32) {
    if kind_of(&vm.code[pc as usize]).is_some_and(Kind::from_helpers) {
        record_slow(vm, pc);
    }
}

#[cold]
#[inline(never)]
fn record_slow(vm: &mut Vm, pc: u32) {
    let site = site_of(vm, pc);
    if site.is_null() {
        return;
    }
    let op = vm.code[pc as usize];
    let Some(frame) = vm.frames.last() else {
        return;
    };
    let base = frame.base as usize;
    let reg = |vm: &Vm, r: u16| vm.stack[base + r as usize];
    match kind_of(&op) {
        Some(Kind::Numbers | Kind::CompareBranch) => {
            let (a, b) = number_operands(vm, &op, base);
            let bits = number_bits(a) | b.map_or(0, number_bits);
            // SAFETY: this instruction's record, in a block that's there for as long as its code is; the helpers' byte.
            unsafe { *site.add(1) |= bits };
        }
        Some(Kind::Call | Kind::New) => {
            let f = match op {
                Op::Call { f, .. }
                | Op::CallPlain { f, .. }
                | Op::CallMethod { f, .. }
                | Op::CallMethodPoly { f, .. }
                | Op::CallMath { f, .. }
                | Op::New { f, .. } => f,
                _ => return,
            };
            let callee = reg(vm, f);
            callee_seen(vm, site, callee);
        }
        Some(Kind::Elem) => {
            let (object, key) = match op {
                Op::GetElem { object, key, .. }
                | Op::SetElem { object, key, .. }
                | Op::SetElemTyped { object, key, .. } => (object, key),
                _ => return,
            };
            let (object, key) = (reg(vm, object), reg(vm, key));
            elem_seen(vm, site, object, key);
        }
        _ => {}
    }
}

/// An instruction's operands for a number record: the first, and the second if it isn't a constant int.
fn number_operands(vm: &Vm, op: &Op, base: usize) -> (Value, Option<Value>) {
    let reg = |r: u16| vm.stack[base + r as usize];
    match *op {
        Op::Add { a, b, .. }
        | Op::Sub { a, b, .. }
        | Op::Mul { a, b, .. }
        | Op::AddInt32 { a, b, .. }
        | Op::Div { a, b, .. }
        | Op::Mod { a, b, .. }
        | Op::BitAnd { a, b, .. }
        | Op::BitOr { a, b, .. }
        | Op::BitXor { a, b, .. }
        | Op::Shl { a, b, .. }
        | Op::Shr { a, b, .. }
        | Op::UShr { a, b, .. }
        | Op::Lt { a, b, .. }
        | Op::Le { a, b, .. }
        | Op::Gt { a, b, .. }
        | Op::Ge { a, b, .. }
        | Op::JumpLt { a, b, .. }
        | Op::JumpLe { a, b, .. }
        | Op::JumpGt { a, b, .. }
        | Op::JumpGe { a, b, .. }
        | Op::JumpNotLt { a, b, .. }
        | Op::JumpNotLe { a, b, .. }
        | Op::JumpNotGt { a, b, .. }
        | Op::JumpNotGe { a, b, .. } => (reg(a), Some(reg(b))),
        Op::AddK { a, k, .. } | Op::SubK { a, k, .. } | Op::MulK { a, k, .. } | Op::DivK { a, k, .. } | Op::ModK { a, k, .. } => {
            (reg(a), Some(vm.consts[k as usize]))
        }
        Op::AddInt { a, .. }
        | Op::SubInt { a, .. }
        | Op::MulInt { a, .. }
        | Op::DivInt { a, .. }
        | Op::ModInt { a, .. }
        | Op::BitAndInt { a, .. }
        | Op::BitOrInt { a, .. }
        | Op::ShlInt { a, .. }
        | Op::ShrInt { a, .. }
        | Op::UShrInt { a, .. }
        | Op::JumpLtInt { a, .. }
        | Op::JumpLeInt { a, .. }
        | Op::JumpGtInt { a, .. }
        | Op::JumpGeInt { a, .. }
        | Op::JumpNotLtInt { a, .. }
        | Op::JumpNotLeInt { a, .. }
        | Op::JumpNotGtInt { a, .. }
        | Op::JumpNotGeInt { a, .. } => (reg(a), None),
        Op::Inc { src, .. } | Op::Dec { src, .. } | Op::Neg { src, .. } => (reg(src), None),
        _ => (Value::int(0), None),
    }
}

/// A word of a record that says what it saw: one thing, or more than one.
///
/// SAFETY: `word` is in a feedback block.
unsafe fn saw(word: *mut u32, what: u32) {
    // SAFETY: as the caller says; records are aligned to their size.
    unsafe {
        let now = word.read();
        if now == 0 {
            word.write(what);
        } else if now != what {
            word.write(MANY);
        }
    }
}

/// What a call record's first word says for a callee, or `None` for one it doesn't (see `OTHER_CALLEE`).
fn callee_word(vm: &Vm, callee: Value) -> Option<u32> {
    if !callee.is_object() {
        return None;
    }
    match &vm.objects[callee.index()].kind {
        ObjectKind::Function { proto, .. } => Some(proto + 1),
        ObjectKind::Native(_) => Some(callee.handle() | NATIVE),
        _ => None,
    }
}

fn callee_seen(vm: &Vm, site: *mut u8, callee: Value) {
    let word = site.cast::<u32>();
    // SAFETY: a call record is sixteen bytes and a `new` record eight, each aligned to eight at least.
    unsafe {
        match callee_word(vm, callee) {
            Some(what) => saw(word, what),
            None => *word.add(1) |= OTHER_CALLEE,
        }
    }
}

/// A native called from compiled code (`helpers::call_native`), whose frame says it's at the call after the one
/// making it.
#[inline]
pub(crate) fn native_called(vm: &mut Vm, callee: Value) {
    let Some(pc) = vm.frames.last().map(|frame| frame.pc.wrapping_sub(1)) else {
        return;
    };
    let site = site_of(vm, pc);
    if !site.is_null() {
        // SAFETY: a call record is sixteen bytes, aligned to eight at least; it is a native, as the code found.
        unsafe { saw(site.cast::<u32>(), callee.handle() | NATIVE) };
    }
}

/// A `new` of a constructor compiled code calls itself, which `helpers::this_for` has made `this` for: what it
/// called, and the shape `this` has.
#[inline]
pub(crate) fn made(vm: &mut Vm, site: *mut u8, callee: Value, this: Value) {
    if site.is_null() {
        return;
    }
    callee_seen(vm, site, callee);
    if this.is_object() {
        let shape = vm.objects[this.index()].shape;
        // SAFETY: a `new` record is eight bytes, aligned to eight.
        unsafe { saw(site.cast::<u32>().add(1), shape.wrapping_add(1)) };
    }
}

/// What `object[key]` was, for an element record.
fn elem_seen(vm: &Vm, site: *mut u8, object: Value, key: Value) {
    let mut bits = 0u8;
    let mut kind = None;
    if !object.is_object() {
        bits |= PRIMITIVE;
    } else {
        match &vm.objects[object.index()].kind {
            ObjectKind::Array(items) | ObjectKind::Arguments(items) => {
                if key.is_int() && key.as_int() >= 0 {
                    match items.get(key.as_int() as usize) {
                        None => bits |= PAST_END,
                        Some(value) if value.is_empty() => bits |= HOLE,
                        Some(_) => {}
                    }
                }
            }
            ObjectKind::TypedArray(data) => {
                bits |= TYPED;
                kind = Some(element_byte(&data.element));
            }
            _ => bits |= NOT_ARRAY,
        }
    }
    let mut name = None;
    if key.is_symbol() {
        bits |= NAMED;
        name = Some(key.handle());
    } else if key.is_string() {
        bits |= NAMED;
        // The name it's read as, if that's been needed of this string before (which the helper about to read it
        // does): nothing is made here.
        name = Some(vm.strings.items[key.index()].atom).filter(|&atom| atom != 0);
    } else if !(key.is_int() && key.as_int() >= 0) {
        bits |= ODD_KEY;
    }
    // SAFETY: an element record is eight bytes, aligned to eight.
    unsafe {
        *site |= bits;
        if let Some(kind) = kind {
            *site.add(1) = kind;
        }
        if let Some(name) = name {
            saw(site.add(4).cast::<u32>(), name.wrapping_add(1));
        }
    }
}

/// The byte a typed array's element kind is, as compiled code reads it.
fn element_byte(element: &crate::builtins::Element) -> u8 {
    // SAFETY: `Element` is a fieldless enum of one byte; compiled code reads the same byte.
    unsafe { *(element as *const crate::builtins::Element).cast::<u8>() }
}

/// A typed array's element written by `helpers::typed_set`'s quick way, which is a SetElem's before its helper: the
/// record compiled code said, if it said one, which is left for `set_elem` if the quick way didn't do it.
#[inline]
pub(crate) fn typed_written(vm: &mut Vm, object: Value) {
    if vm.jit.site == 0 {
        return;
    }
    let site = take(vm);
    if let ObjectKind::TypedArray(data) = &vm.objects[object.index()].kind {
        let kind = element_byte(&data.element);
        // SAFETY: an element record is eight bytes, aligned to eight.
        unsafe {
            *site.add(TYPED_AT) = 1;
            *site.add(1) = kind;
        }
    }
}

/// GetElem's helper, which is given the place and the operands: what they were.
#[inline]
pub(crate) fn elem_read(vm: &mut Vm, pc: u32, object: Value, key: Value) {
    let site = site_of(vm, pc);
    if !site.is_null() {
        elem_seen(vm, site, object, key);
    }
}

// Being due.

/// What the engine keeps of a function's block while it waits for its feedback to settle.
#[derive(Default, Clone, Copy)]
pub(crate) struct Waiting {
    /// A hash of its records when it was last looked at.
    hash: u64,
    waits: u8,
}

/// Its records' hash.
///
/// SAFETY: `block` is a block of the function's, `size` bytes.
unsafe fn hash_of(block: usize, size: u32) -> u64 {
    // SAFETY: as the caller says.
    let bytes = unsafe { std::slice::from_raw_parts((block + HEADER as usize) as *const u8, (size - HEADER) as usize) };
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
    }
    hash
}

/// A function's count has come to 0 (its code took it down at its entry and each loop's jump back): it's due for the
/// optimizing tier if its feedback hasn't changed in the last [`SETTLE`], or it's waited [`MOST_WAITS`] times for
/// that; else it waits another [`SETTLE`]. Says whether it's due now, having set its count for what's next. With
/// no tier yet, a function that's due is counted, and its count set past where it'll come to 0 again.
///
/// SAFETY: `block` is the function's block, `size` bytes, there as long as its code is.
pub(crate) unsafe fn due(waiting: &mut Waiting, block: usize, size: u32) -> bool {
    // SAFETY: as the caller says.
    let hash = unsafe { hash_of(block, size) };
    let count = block as *mut i32;
    let settled = waiting.waits > 0 && hash == waiting.hash;
    if settled || waiting.waits >= MOST_WAITS {
        // SAFETY: the header's first four bytes are the count.
        unsafe { count.write(i32::MAX) };
        return true;
    }
    waiting.hash = hash;
    waiting.waits += 1;
    // SAFETY: as above.
    unsafe { count.write(SETTLE) };
    false
}

// Saying what it came to.

/// What a block's records came to, added up, for `RSUI_OPT_LOG`.
#[derive(Default, Debug, Clone, Copy)]
pub(crate) struct Summary {
    pub(crate) functions: u32,
    pub(crate) bytes: u64,
    /// Number records: never reached, ints only, met a double (or an int result that isn't one), met a string or
    /// something else.
    pub(crate) numbers: [u32; 4],
    /// Branches: never reached, only taken, only not, both.
    pub(crate) branches: [u32; 4],
    /// Calls: never reached the record (all to a callee the code guessed and compiled in line, or none), one
    /// callee, more than one, something else too.
    pub(crate) calls: [u32; 4],
    /// `new`: never, one shape, more than one.
    pub(crate) news: [u32; 3],
    /// Elements: nothing unusual, holes or past the end, typed, by name, something else.
    pub(crate) elems: [u32; 5],
}

impl Summary {
    /// Adds a function's block to it.
    ///
    /// SAFETY: `block` is a block laid out for `ops`.
    pub(crate) unsafe fn add(&mut self, ops: &[Op], block: usize, knowns: &[(u32, Box<[u32]>)]) {
        let layout = layout(ops);
        self.functions += 1;
        self.bytes += u64::from(layout.size);
        for (index, (op, &at)) in ops.iter().zip(layout.offsets.iter()).enumerate() {
            if at == NONE {
                continue;
            }
            let Some(kind) = kind_of(op) else { continue };
            // SAFETY: as the caller says.
            let raw = unsafe { std::slice::from_raw_parts((block + at as usize) as *const u8, kind.size() as usize) };
            let found = knowns.binary_search_by_key(&(index as u32), |&(at, _)| at).ok();
            let record = logical(kind, raw, found.map_or(&[][..], |found| &knowns[found].1[..]));
            let byte = |k: usize| record[k];
            let word = |k: usize| u32::from_le_bytes(record[4 * k..4 * k + 4].try_into().expect("eight bytes"));
            match kind {
                Kind::Numbers => self.numbers[number_class(byte(0))] += 1,
                Kind::Branch => self.branches[usize::from(byte(0) & 3)] += 1,
                Kind::CompareBranch => {
                    self.numbers[number_class(byte(0))] += 1;
                    self.branches[usize::from(byte(1) & 3)] += 1;
                }
                Kind::Call => {
                    let class = match (word(0), word(1)) {
                        (_, flags) if flags & OTHER_CALLEE != 0 => 3,
                        (0, _) => 0,
                        (MANY, _) => 2,
                        _ => 1,
                    };
                    self.calls[class] += 1;
                }
                Kind::New => {
                    let class = match word(1) {
                        0 => 0,
                        MANY => 2,
                        _ => 1,
                    };
                    self.news[class] += 1;
                }
                Kind::Elem => {
                    let bits = byte(0);
                    let class = if bits & (NOT_ARRAY | PRIMITIVE | ODD_KEY) != 0 {
                        4
                    } else if bits & NAMED != 0 {
                        3
                    } else if bits & TYPED != 0 {
                        2
                    } else if bits & (HOLE | PAST_END) != 0 {
                        1
                    } else {
                        0
                    };
                    self.elems[class] += 1;
                }
            }
        }
    }
}

/// What a call record says it called, all in its first word, as a record with no known callees would have it: the one
/// callee, or `MANY`, and its fifth byte, given the callees its flags stand for.
pub(crate) fn plain_call(record: &[u8], knowns: &[u32]) -> [u8; 8] {
    let mut seen: Vec<u32> = Vec::new();
    let word = u32::from_le_bytes(record[0..4].try_into().expect("a call record is sixteen bytes"));
    let mut many = word == MANY;
    if word != 0 && word != MANY {
        seen.push(word);
    }
    for (at, &known) in knowns.iter().enumerate().take(8) {
        if record[KNOWN_AT + at] != 0 && !seen.contains(&(known + 1)) {
            seen.push(known + 1);
        }
    }
    many |= seen.len() > 1;
    let what = if many { MANY } else { seen.first().copied().unwrap_or(0) };
    let mut out = [0u8; 8];
    out[0..4].copy_from_slice(&what.to_le_bytes());
    out[4] = record[4];
    out
}

/// What a record says, in the form it'd have if every bit were in one place: a number record's one byte of bits, a
/// branch record's one byte of ways, a compare-and-branch's two, a call's eight (`plain_call`), an element's with
/// `TYPED` among its bits.
pub(crate) fn logical(kind: Kind, record: &[u8], knowns: &[u32]) -> Vec<u8> {
    let numbers = |at: usize| (if record[at + CODE_DOUBLE] != 0 { DOUBLE } else { 0 }) | record[at + 1];
    let ways = |at: usize| (if record[at + TAKEN_AT] != 0 { TAKEN } else { 0 }) | (if record[at + FELL_AT] != 0 { FELL } else { 0 });
    match kind {
        Kind::Numbers => vec![numbers(0)],
        Kind::Branch => vec![ways(0)],
        Kind::CompareBranch => vec![numbers(0), ways(2)],
        Kind::Call => plain_call(record, knowns).to_vec(),
        Kind::New => record.to_vec(),
        Kind::Elem => {
            let mut out = record.to_vec();
            out[0] |= if record[TYPED_AT] != 0 { TYPED } else { 0 };
            out[TYPED_AT] = 0;
            out
        }
    }
}

fn number_class(bits: u8) -> usize {
    if bits & (STRING | OTHER) != 0 {
        3
    } else if bits & DOUBLE != 0 {
        2
    } else {
        1
    }
}

/// What `RSUI_OPT` asks for, read once: whether compiled code records feedback (`0` or `off` turns it off, with the
/// feature built in).
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
pub(crate) fn on() -> bool {
    use std::sync::atomic::{AtomicU8, Ordering};
    static ON: AtomicU8 = AtomicU8::new(0);
    match ON.load(Ordering::Relaxed) {
        0 => {
            let on = !matches!(std::env::var("RSUI_OPT").as_deref(), Ok("0" | "off"));
            ON.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        known => known == 2,
    }
}

/// What `RSUI_OPT_LOG` asks for, read once: a line for each function that's due, and what the feedback came to when
/// the engine goes.
pub(crate) fn logging() -> bool {
    use std::sync::atomic::{AtomicU8, Ordering};
    static LOG: AtomicU8 = AtomicU8::new(0);
    match LOG.load(Ordering::Relaxed) {
        0 => {
            let on = std::env::var_os("RSUI_OPT_LOG").is_some_and(|value| value != "0");
            LOG.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        known => known == 2,
    }
}
