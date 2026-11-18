//! Turning a function's instructions into x86-64, one template per instruction. See `JIT.md` for the design: what's
//! kept in which machine register, how a frame is entered and left, and how to add an instruction.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::mem::offset_of;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use foldhash::fast::FixedState;

use super::asm::*;
use super::helpers::{self, BAIL, TAKEN};
use super::memory::CodeMemory;
use crate::arena::Arena;
use crate::object::ObjectLayout;
use crate::op::{Cache, IS_CLASS, IS_CONSTRUCTOR, IS_DERIVED, KEEPS_ARGS, Op, Proto};
use crate::value::Value;
use crate::vm::{Frame, Vm};

// What each machine register holds in compiled code. All are ones that Windows' calling convention has a function
// keep, so the Rust helpers leave them be.
/// The registers of the frame that's running.
const REGS: R = RBX;
/// The frame that's running.
const FRAME: R = R12;
/// The engine.
const VM: R = R13;
/// The tag of an int, to `or` into one.
const INT: R = R14;
/// The caches of the function that's running.
const ICS: R = R15;
/// `undefined`.
const UNDEF: R = RSI;
/// The list of objects.
const OBJECTS: R = RDI;
/// The list of cells.
const CELLS: R = RBP;

const INT_TAG: u64 = 0xFFF9 << 48;
const OBJECT_TAG: u64 = 0xFFFA << 48;
const TAG_INT: i32 = 0xFFF9;
const TAG_OBJECT: i32 = 0xFFFA;
const TAG_STRING: i32 = 0xFFFB;
const UNDEFINED: u64 = Value::UNDEFINED.0;
const NULL: u64 = Value::NULL.0;
const FALSE: u64 = Value::FALSE.0;
const TRUE: u64 = Value::TRUE.0;
const EMPTY: u64 = Value::EMPTY.0;
const CANONICAL_NAN: u64 = 0x7FF8 << 48;
/// What compiled code keeps under it on the machine's stack: the 32 bytes Windows has a caller leave for the function
/// it calls, and 8 more, which keeps it aligned as a call needs.
const NATIVE_FRAME: i32 = 40;
/// Where those 8 are, which nothing it calls uses: a callee compiled in line keeps where its captured variables are
/// there (see `Gen::inline_call`).
const SPARE: i32 = 32;

/// A proto's `jit` slot from here up is the address of its code. Below it, it counts down to compiling.
pub(crate) const CODE_MIN: u64 = 1 << 20;

/// Each function's code starts with four words before its entry: where a call from compiled code that has made the
/// frame itself comes in (`entry - 32`, see `Gen::call_known`), whether it's worth going into from the interpreter or
/// Rust (`entry - 24`, which its slot says too), where its body is (`entry - 16`), and where a `new` from compiled
/// code comes in, or 0 (`entry - 8`). The entry is a one-byte `nop`, so that the slot can be one past it (see
/// `Jit::slots`).
pub(crate) const HEADER: u64 = 32;

/// How many instructions a function needs, if it has no loop and makes no call, for going into its code from the
/// interpreter or from Rust to be worth it: that costs about as much as running three instructions in the interpreter,
/// and compiled code calls it directly whatever its size.
const WORTH_ENTERING: usize = 10;

/// Where things are, in the engine's own structures, for the code to read.
pub(crate) struct Layout {
    obj: ObjectLayout,
    frames_len: i32,
    frames_committed: i32,
    stack_len: i32,
    debt: i32,
    threshold: i32,
    epoch: i32,
    global: i32,
    floor: i32,
    /// `Stats::left_line` and `Stats::missed`, which compiled code counts.
    left_line: i32,
    missed: i32,
    value: i32,
    /// `Vm::stack_base` and `Vm::stack_limit`, which the trampoline works out the floor from.
    stack_base: i32,
    stack_limit: i32,
    frame_size: i32,
    f_pc: i32,
    f_base: i32,
    f_top: i32,
    f_proto: i32,
    f_closure: i32,
    f_cache_base: i32,
    f_ret_dst: i32,
    f_flags: i32,
    f_new_target: i32,
    f_upvalues: i32,
    proto_size: i32,
    p_flags: i32,
    cache_size: i32,
    c_shape: i32,
    c_other: i32,
    c_slot: i32,
    c_epoch: i32,
    c_more: i32,
    /// `String.prototype`'s and `Number.prototype`'s handles, where a string's and a number's methods are read from.
    string_proto: i32,
    number_proto: i32,
    /// `Vm::buffer_epoch`, which a typed array's quick elements are right for while theirs is the same.
    buffer_epoch: i32,
    /// `TypedData`'s fields.
    t_element: i32,
    t_quick: i32,
    t_quick_length: i32,
    t_epoch: i32,
    /// Where a native's function is in its `NativeData`.
    n_func: i32,
    /// `Vm::top`, where a native may use the stack from.
    top: i32,
    /// What `for of` over an array checks (see `Vm::get_iterator`): the usual array shape, `Array.prototype`, where its
    /// `Symbol.iterator` was last found (its shape and slot) and the built-in `values` that has to be there.
    array_shape: i32,
    array_proto: i32,
    array_values: i32,
    iterator_shape: i32,
    iterator_slot: i32,
    /// Where the list of shapes is (`Shapes::items`' pointer), how big one is, and where its prototype is.
    shapes: i32,
    shape_size: i32,
    shape_proto: i32,
    /// `Vm::instance_of_cache`, and its entries' size and fields, and `Vm::function_has_instance`.
    instance_of: i32,
    io_size: i32,
    io_shape: i32,
    io_epoch: i32,
    io_holder: i32,
    io_has_slot: i32,
    io_prototype_slot: i32,
    has_instance: i32,
    /// `Jit::site`, where compiled code says which record a helper it calls is to fill in (see `jit::feedback`).
    #[cfg(feature = "jit-opt")]
    site: i32,
}

/// Where a `Vec`'s pointer to what it holds is, in it: found from a real one, as Rust lays a `Vec` out as it likes.
fn vec_pointer() -> Option<usize> {
    let list: Vec<crate::object::Shape> = Vec::with_capacity(1);
    if size_of::<Vec<crate::object::Shape>>() != 3 * size_of::<usize>() {
        return None;
    }
    // SAFETY: a `Vec` is three words, read here as just that.
    let words: [usize; 3] = unsafe { std::mem::transmute_copy(&list) };
    words.iter().position(|&word| word == list.as_ptr() as usize).map(|at| at * size_of::<usize>())
}

impl Layout {
    fn new() -> Option<Self> {
        let obj = crate::object::object_layout();
        // An object's address is its handle shifted by 6.
        if obj.size != 64 {
            return None;
        }
        let o = |at: usize| at as i32;
        let jit = offset_of!(Vm, jit);
        Some(Self {
            obj,
            frames_len: o(offset_of!(Vm, frames) + Arena::<Frame>::LEN_OFFSET),
            frames_committed: o(offset_of!(Vm, frames) + Arena::<Frame>::COMMITTED_OFFSET),
            stack_len: o(offset_of!(Vm, stack) + Arena::<Value>::LEN_OFFSET),
            debt: o(offset_of!(Vm, debt)),
            threshold: o(offset_of!(Vm, threshold)),
            epoch: o(offset_of!(Vm, epoch)),
            global: o(offset_of!(Vm, global)),
            floor: o(jit + offset_of!(super::Jit, floor)),
            left_line: o(jit + offset_of!(super::Jit, stats) + offset_of!(super::Stats, left_line)),
            missed: o(jit + offset_of!(super::Jit, stats) + offset_of!(super::Stats, missed)),
            value: o(jit + offset_of!(super::Jit, value)),
            stack_base: o(offset_of!(Vm, stack_base)),
            stack_limit: o(offset_of!(Vm, stack_limit)),
            frame_size: o(size_of::<Frame>()),
            f_pc: o(offset_of!(Frame, pc)),
            f_base: o(offset_of!(Frame, base)),
            f_top: o(offset_of!(Frame, top)),
            f_proto: o(offset_of!(Frame, proto)),
            f_closure: o(offset_of!(Frame, closure)),
            f_cache_base: o(offset_of!(Frame, cache_base)),
            f_ret_dst: o(offset_of!(Frame, ret_dst)),
            f_flags: o(offset_of!(Frame, flags)),
            f_new_target: o(offset_of!(Frame, new_target)),
            f_upvalues: o(offset_of!(Frame, upvalues)),
            proto_size: o(size_of::<Proto>()),
            p_flags: o(offset_of!(Proto, flags)),
            cache_size: o(size_of::<Cache>()),
            c_shape: o(offset_of!(Cache, shape)),
            c_other: o(offset_of!(Cache, other)),
            c_slot: o(offset_of!(Cache, slot)),
            c_epoch: o(offset_of!(Cache, epoch)),
            c_more: o(offset_of!(Cache, more)),
            string_proto: o(offset_of!(Vm, realm) + offset_of!(crate::vm::Realm, string_proto)),
            number_proto: o(offset_of!(Vm, realm) + offset_of!(crate::vm::Realm, number_proto)),
            buffer_epoch: o(offset_of!(Vm, buffer_epoch)),
            t_element: o(offset_of!(crate::builtins::TypedData, element)),
            t_quick: o(offset_of!(crate::builtins::TypedData, quick)),
            t_quick_length: o(offset_of!(crate::builtins::TypedData, quick_length)),
            t_epoch: o(offset_of!(crate::builtins::TypedData, epoch)),
            n_func: o(offset_of!(crate::object::NativeData, func)),
            top: o(offset_of!(Vm, top)),
            array_shape: o(offset_of!(Vm, realm) + offset_of!(crate::vm::Realm, array_shape)),
            array_proto: o(offset_of!(Vm, realm) + offset_of!(crate::vm::Realm, array_proto)),
            array_values: o(offset_of!(Vm, realm) + offset_of!(crate::vm::Realm, array_values)),
            iterator_shape: o(offset_of!(Vm, array_iterator_slot) + offset_of!((u32, u32), 0)),
            iterator_slot: o(offset_of!(Vm, array_iterator_slot) + offset_of!((u32, u32), 1)),
            shapes: o(offset_of!(Vm, shapes) + offset_of!(crate::object::Shapes, items) + vec_pointer()?),
            shape_size: o(size_of::<crate::object::Shape>()),
            shape_proto: o(offset_of!(crate::object::Shape, proto)),
            instance_of: o(offset_of!(Vm, instance_of_cache)),
            io_size: o(size_of::<crate::vm::InstanceOfCache>()),
            io_shape: o(offset_of!(crate::vm::InstanceOfCache, shape)),
            io_epoch: o(offset_of!(crate::vm::InstanceOfCache, epoch)),
            io_holder: o(offset_of!(crate::vm::InstanceOfCache, holder)),
            io_has_slot: o(offset_of!(crate::vm::InstanceOfCache, has_slot)),
            io_prototype_slot: o(offset_of!(crate::vm::InstanceOfCache, prototype_slot)),
            has_instance: o(offset_of!(Vm, function_has_instance)),
            #[cfg(feature = "jit-opt")]
            site: o(jit + offset_of!(super::Jit, site)),
        })
    }
}

/// The way into compiled code from Rust (see [`trampoline`]): the engine, the frame on top, and where in its function's
/// code to start. It gives back how the code left, with what it handed back in `vm.jit.value`.
pub(crate) type Trampoline = unsafe extern "C" fn(*mut Vm, *mut Frame, usize) -> u32;

/// One compiled function, as the engine keeps it.
pub(crate) struct Compiled {
    /// Where its code is.
    pub(crate) address: usize,
    /// Where each of its instructions starts, from `address`.
    pub(crate) offsets: Arc<[u32]>,
    /// Whether each instruction can be gone into from the interpreter or Rust: where nothing's known of the machine's
    /// registers, which is the top of a loop, where a handler starts and where a generator carries on (see
    /// `Gen::targets`). A call can also be gone into from the function's code compiled before (see `Gen::call`).
    pub(crate) entries: Box<[bool]>,
    /// Where each instruction's cold paths start, from `address`, for a profile to say which instruction an address
    /// in them belongs to (see `Jit::code_map`).
    pub(crate) cold: Box<[u32]>,
    /// How many calls it has compiled in line.
    pub(crate) inlined: u32,
    /// Whether it's worth going into from the interpreter or Rust (see `HEADER`).
    pub(crate) worth: bool,
    /// What its calls that knew nothing of what they'd call count down, to compiling it again (see `Gen::call`); the
    /// code counts it, and `Vm::jit_recompile` sets it.
    pub(crate) counter: Box<AtomicU64>,
    /// Its calls that knew nothing of what they'd call, and how many times their count has started again since.
    pub(crate) unguessed: Box<[u32]>,
    pub(crate) waits: u32,
    /// How many times it was compiled before this.
    pub(crate) generation: u8,
    pub(crate) bytes: usize,
    /// Where its feedback is and how big that is (see `jit::feedback`), or 0; and, for `RSUI_OPT_LOG`, the
    /// instructions it's laid out for.
    #[cfg(feature = "jit-opt")]
    pub(crate) feedback: usize,
    #[cfg(feature = "jit-opt")]
    pub(crate) feedback_size: u32,
    #[cfg(feature = "jit-opt")]
    pub(crate) feedback_ops: Option<Box<[Op]>>,
    /// For each call that knew what it would call, by its instruction: the callees its record's flags stand for (see
    /// `feedback::KNOWN_AT`), in order.
    #[cfg(feature = "jit-opt")]
    pub(crate) feedback_knowns: Box<[(u32, Box<[u32]>)]>,
    /// Where each instruction's record is in its block, or `feedback::NONE`, for the helpers (`feedback::site_of`).
    #[cfg(feature = "jit-opt")]
    pub(crate) feedback_offsets: Arc<[u32]>,
}

impl Compiled {
    /// What its function's slot says once it's given this code (see `HEADER`).
    pub(crate) fn slot(&self) -> u64 {
        self.address as u64 + HEADER + u64::from(self.worth)
    }
}

/// What the code's writer knows of a function whose code it has written, for code it writes later to call directly or
/// go on in (see `Gen::call_known` and `Gen::inline_call`). The writer keeps its own list of these, as it may be on
/// another thread than the engine.
pub(crate) struct Known {
    address: usize,
    offsets: Arc<[u32]>,
    /// What each instruction's code takes from a machine register rather than the frame (see `Gen::incoming`), which
    /// whatever goes into the code there has to have loaded.
    incoming: Arc<[Option<(u16, R)>]>,
}

/// How many times the calls of a function's code that had nothing to go on when it was compiled can be made before
/// it's compiled again with what its caches know by then (see `Gen::call`), and how many times a function can be.
pub(crate) const RECOMPILE_AFTER: u64 = 1000;
const MOST_COMPILES: u8 = 4;
/// The longest function that's compiled again, in instructions: a long one costs a lot to compile, and its code is
/// kept twice.
const RECOMPILE_MOST: usize = 400;
/// How many more of those calls code that's being compiled again on the worker makes before it looks for the new code.
pub(crate) const RECOMPILE_POLL: u64 = 64;

/// Where an engine keeps the lists compiled code points into, which never move, so that code can be written for it
/// from another thread.
#[derive(Clone, Copy)]
pub(crate) struct Bases {
    stack: u64,
    caches: u64,
    objects: u64,
    cells: u64,
    protos: u64,
    slots: u64,
}

impl Bases {
    pub(crate) fn of(vm: &Vm) -> Self {
        Self {
            stack: vm.stack.as_ptr() as u64,
            caches: vm.caches.as_ptr() as u64,
            objects: vm.objects.as_ptr() as u64,
            cells: vm.cells.as_ptr() as u64,
            protos: vm.protos.as_ptr() as u64,
            slots: vm.jit.slots.as_ptr() as u64,
        }
    }
}

/// What compiling one function needs, taken from the engine when it's due, so that it can be compiled on another
/// thread while the engine carries on: its instructions, what's needed of its proto and the constants they name, and
/// what the engine knows now that the code is compiled for (see "Calls and properties the code knows" in `JIT.md`):
/// what its property places have found, what its calls are likely to call and what those callees are, and what the
/// frame it's being compiled for has in its registers. All copied, with where the engine's lists are, which code is
/// compiled to point into. Nothing of the engine is read while compiling, so the engine can go while a function of
/// its is being compiled.
pub(crate) struct Job {
    pub(crate) proto_id: u32,
    proto: Proto,
    ops: Box<[Op]>,
    /// The constants its instructions and its callees' name (see [`constant_of`]), by where they are in the engine's
    /// list.
    consts: Box<[(u32, Value)]>,
    bases: Bases,
    /// The cache entries its property places and its callees' have now, by where they are in the engine's list (see
    /// `Gen::fixed_read`).
    caches: Box<[(u32, Cache)]>,
    /// The shapes those entries name that code may compare with as constants (none of them a dictionary's), in order.
    /// They're pinned as the job is taken (see `Shapes::pin`): a collection may free one before the code is in place,
    /// and its number mustn't be given to another, which the code would take for it.
    plain: Box<[u32]>,
    /// The engine's epoch and its global object, as the entries are right for.
    epoch: u32,
    global: u32,
    /// What each call is likely to call, by the call's instruction (see `Look::likely_callees`), in order.
    calls: Box<[(u32, Box<[u32]>)]>,
    /// What's known of each of those callees, by its proto, in order.
    callees: Box<[Callee]>,
    /// The registers of the frame it's being compiled for, if that's the frame on top (see `Gen::looks_double`), and
    /// the innermost loop that frame is in.
    live: Box<[Value]>,
    looping: Option<(usize, usize)>,
    /// How many times it's been compiled before (see `Vm::jit_recompile`), and whether its slot is given the code
    /// as soon as it's there, which a callee compiled ahead of its caller isn't (see `Look::inline_candidates`).
    generation: u8,
    pub(crate) give: bool,
}

/// A function a job's calls are likely to call, as the job has it.
struct Callee {
    proto_id: u32,
    proto: Proto,
    /// Its slot when the job was taken.
    slot: u64,
    /// Its instructions, if it's short and simple enough to be compiled in line (see `Look::inline_ops`).
    ops: Option<Box<[Op]>>,
}

// SAFETY: what it has is its own: the addresses in `bases` are only written into code, not followed, and the values
// are only looked at, never followed.
unsafe impl Send for Job {}

/// The constant an instruction's template reads from the engine's list, if it reads one. Every template that does has
/// to be named here, as a function compiled on another thread only has what its job copied; a template asking for one
/// that isn't there leaves the function uncompiled.
fn constant_of(op: &Op) -> Option<u32> {
    match *op {
        Op::LoadConst { index, .. } => Some(index),
        Op::AddK { k, .. } | Op::SubK { k, .. } | Op::MulK { k, .. } | Op::DivK { k, .. } | Op::ModK { k, .. } => {
            Some(u32::from(k))
        }
        _ => None,
    }
}

/// The property place an instruction's template may read what's known of (see `Gen::fixed_read`). Every template
/// that does has to be named here; one that isn't knows nothing of its place, and goes the cache's way.
fn read_place(op: &Op) -> Option<u16> {
    match *op {
        Op::GetGlobal { cache, .. }
        | Op::GetProp { cache, .. }
        | Op::GetPropPoly { cache, .. }
        | Op::SetProp { cache, .. }
        | Op::SetPropPoly { cache, .. }
        | Op::CallMethod { cache, .. }
        | Op::CallMethodPoly { cache, .. } => Some(cache),
        _ => None,
    }
}

/// The callee's register of a call whose callee can be guessed (see `Look::likely_callees`), and how it's read.
fn call_of(op: &Op) -> Option<(u16, Method)> {
    match *op {
        Op::Call { f, .. } | Op::CallPlain { f, .. } => Some((f, Method::None)),
        Op::CallMethod { f, cache, .. } => Some((f, Method::Mono(cache))),
        Op::CallMethodPoly { f, cache, .. } => Some((f, Method::Poly(cache))),
        _ => None,
    }
}

/// A copy of what compiling needs of a proto: all but its captured variables' and lines' lists.
fn copy_proto(proto: &Proto) -> Proto {
    Proto {
        start: proto.start,
        cache_base: proto.cache_base,
        registers: proto.registers,
        params: proto.params,
        length: proto.length,
        flags: proto.flags,
        says: proto.says,
        locals: proto.locals,
        name: proto.name,
        file: proto.file,
        captures: Box::default(),
        capture_names: Box::default(),
        handlers: proto.handlers.clone(),
        lines: Box::default(),
    }
}

impl Job {
    /// The job of compiling a function for the `generation`th time (0 the first), its slot given the code once it's
    /// there if `give`, or `None` if it can't be compiled. Pins the shapes and the functions' numbers it may write into
    /// the code.
    pub(crate) fn new(vm: &mut Vm, proto_id: u32, generation: u8, give: bool) -> Option<Self> {
        let mut job = Self::take(vm, proto_id, generation, give)?;
        // Only a shape in use can be pinned (a cache names no other); one that can't isn't written into the code.
        let plain: Vec<u32> = job.plain.iter().copied().filter(|&shape| vm.shapes.pin(shape)).collect();
        job.plain = plain.into();
        // Its code checks what it calls by number, and is this function's by its number, so none of them is given to
        // another function once it's gone (see `compiled.rs`).
        vm.keep_number(proto_id);
        for callee in &job.callees {
            vm.keep_number(callee.proto_id);
        }
        Some(job)
    }

    fn take(vm: &Vm, proto_id: u32, generation: u8, give: bool) -> Option<Self> {
        let look = Look::new(vm, proto_id)?;
        let proto = look.proto;
        // `RSUI_JIT_LOG=2` lists each function's instructions as it's compiled.
        if super::listing() {
            eprintln!("jit: {} ({}):", vm.atom_text(proto.name), vm.atom_text(proto.file));
            for (at, op) in look.ops.iter().enumerate() {
                eprintln!("jit:   {at:4} {op:?}");
            }
        }
        let mut calls = Vec::new();
        let mut known: Vec<u32> = Vec::new();
        for (at, op) in look.ops.iter().enumerate() {
            if let Some((f, method)) = call_of(op) {
                let found = look.likely_callees(at, f, method);
                if !found.is_empty() {
                    known.extend_from_slice(&found);
                    calls.push((at as u32, found.into_boxed_slice()));
                }
            }
        }
        known.sort_unstable();
        known.dedup();
        let callees: Box<[Callee]> = known
            .iter()
            .map(|&id| Callee {
                proto_id: id,
                proto: copy_proto(&vm.protos[id as usize]),
                slot: vm.jit.slots[id as usize],
                ops: Look::inline_ops(vm, id).map(Into::into),
            })
            .collect();
        // What the instructions name, its own and those of its callees that could be compiled in line.
        let mut places: Vec<(u32, Cache)> = Vec::new();
        let mut consts: Vec<(u32, Value)> = Vec::new();
        let named = std::iter::once((proto.cache_base, look.ops)).chain(
            callees
                .iter()
                .filter_map(|callee| Some((callee.proto.cache_base, callee.ops.as_deref()?))),
        );
        for (cache_base, ops) in named {
            for op in ops {
                if let Some(index) = constant_of(op) {
                    consts.push((index, vm.consts[index as usize]));
                }
                let Some(cache) = read_place(op) else {
                    continue;
                };
                let at = (cache_base + u32::from(cache)) as usize;
                let Some(&first) = vm.caches.get(at) else {
                    continue;
                };
                places.push((at as u32, first));
                if first.more != crate::object::NONE && first.more & crate::op::MEGAMORPHIC == 0 {
                    let others = at + first.more as usize;
                    for other in others..others + crate::op::POLY {
                        if let Some(&entry) = vm.caches.get(other) {
                            places.push((other as u32, entry));
                        }
                    }
                }
            }
        }
        consts.sort_unstable_by_key(|&(index, _)| index);
        consts.dedup_by_key(|&mut (index, _)| index);
        places.sort_unstable_by_key(|&(at, _)| at);
        places.dedup_by_key(|&mut (at, _)| at);
        let mut plain: Vec<u32> = places
            .iter()
            .map(|(_, entry)| entry.shape)
            .filter(|&shape| {
                shape != crate::object::NONE
                    && (shape as usize) < vm.shapes.items.len()
                    && !vm.shapes.is_dict(shape)
            })
            .collect();
        plain.sort_unstable();
        plain.dedup();
        // The frame it's being compiled for, if that's the one on top (at the top of a loop, or at a call that
        // compiles it again), and the innermost loop it's in.
        let frame = vm.frames.last().filter(|frame| frame.proto == proto_id);
        let live: Box<[Value]> = frame
            .and_then(|frame| {
                let base = frame.base as usize;
                vm.stack.get(base..base + usize::from(proto.registers)).map(Into::into)
            })
            .unwrap_or_default();
        let looping = frame.and_then(|frame| {
            let at = frame.pc.checked_sub(proto.start)? as usize;
            look.ops
                .iter()
                .enumerate()
                .filter_map(|(bottom, op)| {
                    let top = bottom as i64 + 1 + i64::from(super::jump_offset(op)?);
                    (top >= 0 && top as usize <= at && at <= bottom).then_some((top as usize, bottom))
                })
                .min_by_key(|(top, bottom)| bottom - top)
        });
        if super::listing() {
            let at = frame.and_then(|frame| frame.pc.checked_sub(proto.start));
            eprintln!("jit:   (taken at {at:?}, in the loop {looping:?})");
        }
        Some(Self {
            proto_id,
            proto: copy_proto(proto),
            ops: look.ops.into(),
            consts: consts.into(),
            bases: Bases::of(vm),
            caches: places.into(),
            plain: plain.into(),
            epoch: vm.epoch,
            global: vm.global,
            calls: calls.into(),
            callees,
            live,
            looping,
            generation,
            give,
        })
    }
}

/// What the engine knows now that compiling a function uses: what each of its calls is likely to call, from what its
/// caches have found and what the frame being compiled for has (see `likely_callees`), and which callees are worth
/// compiling ahead of it. Only on the engine's thread: as a job is taken (`Job::new`), and when `Vm::jit_recompile`
/// looks at whether compiling a function again would know more.
pub(crate) struct Look<'a> {
    vm: &'a Vm,
    proto_id: u32,
    proto: &'a Proto,
    ops: &'a [Op],
}

impl<'a> Look<'a> {
    pub(crate) fn new(vm: &'a Vm, proto_id: u32) -> Option<Self> {
        let proto = &vm.protos[proto_id as usize];
        let length = super::extent(vm, proto)?;
        let start = proto.start as usize;
        Some(Self {
            vm,
            proto_id,
            proto,
            ops: &vm.code[start..start + length],
        })
    }

    /// What the call at `i` (with its callee in register `f`, read as `method` says) is likely to call: for a method
    /// read at a place that has seen several shapes, each of the methods its entries found on a prototype, a few at
    /// most; else what `known_callee` finds. Whether they have code is for the code's writer to say (see
    /// `Gen::known_callees`).
    fn likely_callees(&self, i: usize, f: u16, method: Method) -> Vec<u32> {
        let Method::Poly(cache) = method else {
            let cache = match method {
                Method::Mono(cache) => Some(cache),
                _ => None,
            };
            return self.known_callee(i, f, cache).into_iter().collect();
        };
        let vm = self.vm;
        let at = (self.proto.cache_base + u32::from(cache)) as usize;
        let first = vm.caches[at];
        let mut entries = vec![first];
        if first.more != crate::object::NONE {
            if first.more & crate::op::MEGAMORPHIC != 0 {
                return Vec::new();
            }
            let others = at + first.more as usize;
            entries.extend((others..others + crate::op::POLY).filter(|&at| at < vm.caches.len()).map(|at| vm.caches[at]));
        }
        let mut found = Vec::new();
        for entry in entries {
            if entry.shape == crate::object::NONE || entry.other == crate::object::NONE || entry.epoch != vm.epoch {
                continue;
            }
            let Some(value) = vm.objects[entry.other as usize].slots().get(entry.slot as usize).copied() else {
                continue;
            };
            if let Some(proto) = self.callable(value)
                && !found.contains(&proto)
                && found.len() < 4
            {
                found.push(proto);
            }
        }
        found
    }

    /// Whether the call at `i` would know something of what it calls, if this were compiled now, given which
    /// functions have code.
    pub(crate) fn call_knows(&self, i: usize, has_code: impl Fn(u32) -> bool) -> bool {
        let Some((f, method)) = self.ops.get(i).and_then(call_of) else {
            return false;
        };
        self.likely_callees(i, f, method)
            .into_iter()
            .any(|known| known == self.proto_id || has_code(known))
    }

    /// The function the call at `i` (with its callee in register `f`, read by CallMethod's `method` cache if it's
    /// that) is likely to call, by what its caches and the frame being compiled for say now, if it's one that
    /// `Gen::call_known` can make the frame for: an everyday function (not a class, a generator or async, and not
    /// keeping its `arguments`) that's been compiled to bytecode. Only a guess: the call checks it each time.
    fn known_callee(&self, i: usize, f: u16, method: Option<u16>) -> Option<u32> {
        // A place that has seen several methods, and `Math.f(x)` when it isn't the built-in one, aren't guessed at.
        if matches!(self.ops[i], Op::CallMethodPoly { .. } | Op::CallMath { .. }) {
            return None;
        }
        let value = match method {
            Some(cache) => self.guess_read(cache, f + 1, i, 0),
            None => self.guess(f, i, 0),
        }?;
        self.callable(value)
    }

    /// The proto of `value` if it's a function `Gen::call_known` can make the frame for: an everyday function (not a
    /// class, a generator or async, and not keeping its `arguments`) that's been compiled to bytecode.
    fn callable(&self, value: Value) -> Option<u32> {
        use crate::op::{IS_ASYNC, IS_GENERATOR, NOT_COMPILED};
        let vm = self.vm;
        if !value.is_object() {
            return None;
        }
        let crate::object::Kind::Function { proto, .. } = vm.objects[value.index()].kind else {
            return None;
        };
        let callee = &vm.protos[proto as usize];
        let unusual = IS_CLASS | KEEPS_ARGS | IS_GENERATOR | IS_ASYNC;
        (callee.flags & unusual == 0 && callee.start != NOT_COMPILED).then_some(proto)
    }

    /// A register of this function's frame as it is now, if that's what it's being compiled for (where it has just
    /// gone round a loop, say): what the last time round left there.
    fn live(&self, register: u16) -> Option<Value> {
        let vm = self.vm;
        let frame = vm.frames.last().filter(|frame| frame.proto == self.proto_id)?;
        let at = frame.base as usize + usize::from(register);
        (at < vm.stack.len()).then(|| vm.stack[at])
    }

    /// What register `register` is likely to have at instruction `before`, for guessing what a call calls: worked
    /// out from the instruction that put it there, a few before, if it's one of those that says (a read whose cache
    /// knows where it's from, a global, a captured variable, a move), or if none of them wrote it and it's one of the
    /// function's own variables (which code sets once, mostly: a function, an object made earlier), what the frame
    /// has there now. Nothing if something else wrote it or a call in between could have, or it's a parameter or a
    /// temporary, which a call is given different things in (callbacks).
    fn guess(&self, register: u16, before: usize, depth: u8) -> Option<Value> {
        let vm = self.vm;
        if depth > 3 {
            return None;
        }
        for at in (before.saturating_sub(8)..before).rev() {
            let op = self.ops[at];
            match op {
                Op::GetProp { dst, object, cache } | Op::GetPropPoly { dst, object, cache } if dst == register => {
                    return self.guess_read(cache, object, at, depth + 1);
                }
                Op::GetGlobal { dst, cache } if dst == register => {
                    let entry = vm.caches[(self.proto.cache_base + u32::from(cache)) as usize];
                    let global = &vm.objects[vm.global as usize];
                    return (global.shape == entry.shape)
                        .then(|| global.slots().get(entry.slot as usize).copied())
                        .flatten();
                }
                Op::GetUp { dst, index } if dst == register => {
                    // A captured variable, as this function's frame that's running has it.
                    let frame = vm.frames.last().filter(|frame| frame.proto == self.proto_id)?;
                    if frame.upvalues.is_null() {
                        return None;
                    }
                    // SAFETY: a frame's captured variables are its closure's, which is alive while it runs, and its
                    // proto has at least this many.
                    let cell = unsafe { *frame.upvalues.add(usize::from(index)) };
                    return Some(vm.cells[cell.0 as u32 as usize]);
                }
                Op::GetUpValue { dst, index } if dst == register => {
                    let frame = vm.frames.last().filter(|frame| frame.proto == self.proto_id)?;
                    if frame.upvalues.is_null() {
                        return None;
                    }
                    // SAFETY: as for GetUp.
                    return Some(unsafe { *frame.upvalues.add(usize::from(index)) });
                }
                Op::GetCell { dst, cell } if dst == register => {
                    return self
                        .guess(cell, at, depth + 1)
                        .filter(|cell| cell.is_cell() && (cell.0 as u32 as usize) < vm.cells.len())
                        .map(|cell| vm.cells[cell.0 as u32 as usize]);
                }
                Op::Move { dst, src } if dst == register => return self.guess(src, at, depth + 1),
                Op::LoadCallee { dst } if dst == register => {
                    let frame = vm.frames.last().filter(|frame| frame.proto == self.proto_id)?;
                    return Some(Value(OBJECT_TAG | u64::from(frame.closure)));
                }
                // What a call gave is in the frame from then on (a variable set from a call, say); what's above
                // where its callee went, the callee's frame may have used.
                Op::Call { dst, f, .. }
                | Op::CallPlain { dst, f, .. }
                | Op::CallMethod { dst, f, .. }
                | Op::CallMethodPoly { dst, f, .. }
                | Op::CallMath { dst, f, .. }
                | Op::New { dst, f, .. }
                | Op::CallSpread { dst, f, .. }
                | Op::NewSpread { dst, f, .. }
                | Op::SuperCall { dst, f, .. }
                | Op::SuperCallSpread { dst, f, .. } => {
                    if dst == register {
                        return self.live(register);
                    }
                    if register >= f {
                        return None;
                    }
                }
                // Anything else that writes it says nothing.
                Op::GetIterator { iter, .. } if register == iter || register == iter + 1 => return None,
                _ if written(&op) == Some(register) => return None,
                _ => {}
            }
        }
        let variables = 1 + usize::from(self.proto.params)..self.proto.clear();
        if variables.contains(&usize::from(register)) {
            self.live(register)
        } else {
            None
        }
    }

    /// What a read at place `cache` of register `object` at instruction `at` finds, as the place's first entry says:
    /// on a prototype (a method), or on the object, as `guess` has it.
    fn guess_read(&self, cache: u16, object: u16, at: usize, depth: u8) -> Option<Value> {
        let vm = self.vm;
        let entry = vm.caches[(self.proto.cache_base + u32::from(cache)) as usize];
        // A place that has seen more than one shape could find anything.
        if entry.more != crate::object::NONE {
            return None;
        }
        let holder = if entry.other != crate::object::NONE {
            if entry.epoch != vm.epoch {
                return None;
            }
            &vm.objects[entry.other as usize]
        } else {
            let value = self.guess(object, at, depth).filter(|value| value.is_object())?;
            let found = &vm.objects[value.index()];
            if found.shape != entry.shape {
                return None;
            }
            found
        };
        holder.slots().get(entry.slot as usize).copied()
    }

    /// A callee's instructions, if a call of it could be compiled in line (see `Gen::inline_call`), but for whether
    /// it has code yet and how much has been compiled in line already: one with no handler, not a class and not
    /// keeping its `arguments`, at most `INLINE_MOST` instructions long, each of them `Gen::inlinable`, which jumps
    /// only forward.
    fn inline_ops(vm: &'a Vm, known: u32) -> Option<&'a [Op]> {
        let callee = &vm.protos[known as usize];
        if !callee.handlers.is_empty() || callee.flags & (IS_CLASS | KEEPS_ARGS) != 0 {
            return None;
        }
        let length = super::extent(vm, callee)?;
        if length > INLINE_MOST {
            return None;
        }
        let ops = &vm.code[callee.start as usize..callee.start as usize + length];
        let forward = |at: usize, op: &Op| {
            super::jump_offset(op).is_none_or(|offset| offset >= 0 && at + 1 + (offset as usize) < length)
        };
        ops.iter()
            .enumerate()
            .all(|(at, op)| Gen::inlinable(op) && forward(at, op))
            .then_some(ops)
    }

    /// The functions worth compiling just before this one: those its calls are likely to call that could be compiled
    /// in line, if they had code, and those that will soon have some anyway (more than halfway to being compiled,
    /// with `threshold` runs to it), which its calls then call directly. Their slots go on counting as before, and
    /// such a function is only given its code when it's due.
    pub(crate) fn inline_candidates(&self, threshold: u64) -> Vec<u32> {
        let mut found = Vec::new();
        for (i, op) in self.ops.iter().enumerate() {
            let Some((f, method)) = call_of(op) else {
                continue;
            };
            for known in self.likely_callees(i, f, method) {
                if known == self.proto_id || found.contains(&known) {
                    continue;
                }
                let slot = self.vm.jit.slots[known as usize];
                let warm = slot != 0 && slot < CODE_MIN && slot / 2 <= threshold / 2;
                if warm || Self::inline_ops(self.vm, known).is_some() {
                    found.push(known);
                }
            }
        }
        found
    }
}

/// What came of a job: the function's code, with what its slot is to say, or `None` if it couldn't be compiled.
pub(crate) struct Done {
    pub(crate) proto_id: u32,
    pub(crate) code: Option<(u64, Compiled)>,
    /// What the job said: how many times the function had been compiled before, and whether its slot is given the
    /// code now.
    pub(crate) generation: u8,
    pub(crate) give: bool,
    /// The calls compiled in line, by their callee and where they are, for `RSUI_JIT_LOG=2`.
    pub(crate) inlines: Vec<(u32, u32)>,
    /// How long it took, when `timed` asked.
    pub(crate) took: std::time::Duration,
    /// How many bytes of code there are now, and how much memory they have.
    pub(crate) used: usize,
    pub(crate) committed: usize,
}

/// What writes an engine's code: the memory it goes in, where things are in the engine, and what it has written. On
/// the engine's own thread when every function is compiled the first time it's called (`RSUI_JIT=eager`), else on a
/// thread of its own (see `jit::Worker`), so that compiling doesn't hold the engine up.
pub(crate) struct Writer {
    memory: CodeMemory,
    layout: Layout,
    /// The functions it has written code for, by their protos: the code they have now.
    known: HashMap<u32, Known, FixedState>,
    /// Whether to say how long each function took.
    timed: bool,
    /// Each function's feedback, how big it is, and the callees its calls' bits stand for: one block for all the code
    /// it's given (see `jit::feedback`), so that a bit means the same in each.
    #[cfg(feature = "jit-opt")]
    blocks: HashMap<u32, (usize, u32, HashMap<u32, Vec<u32>, FixedState>), FixedState>,
    /// Where its two routines for records are (see `feedback_routines`), or 0 if there's no feedback.
    #[cfg(feature = "jit-opt")]
    routines: (u64, u64),
}

impl Writer {
    /// Sets aside room for this much code, and writes the trampoline for the engine whose lists are at `bases`: the
    /// writer and the trampoline, or `None` if there's no JIT to be had here.
    pub(crate) fn new(bases: Bases, code_bytes: usize, timed: bool) -> Option<(Self, Trampoline)> {
        let layout = Layout::new()?;
        let mut memory = CodeMemory::new(code_bytes)?;
        if (memory.address(0) as u64) < CODE_MIN {
            return None;
        }
        let code = trampoline(&bases, &layout);
        let at = memory.add(&code)?;
        // SAFETY: the trampoline was written for this signature.
        let trampoline = unsafe { std::mem::transmute::<usize, Trampoline>(memory.address(at)) };
        #[cfg(feature = "jit-opt")]
        let routines = {
            let (code, note) = feedback_routines();
            let at = memory.add(&code)?;
            (memory.address(at) as u64, (memory.address(at) + note) as u64)
        };
        Some((
            Self {
                memory,
                layout,
                known: HashMap::default(),
                timed,
                #[cfg(feature = "jit-opt")]
                blocks: HashMap::default(),
                #[cfg(feature = "jit-opt")]
                routines,
            },
            trampoline,
        ))
    }

    /// Something that keeps the code's memory there while it's held.
    pub(crate) fn keep(&self) -> std::sync::Arc<super::memory::Views> {
        self.memory.keep()
    }

    /// Compiles the function a job is for.
    pub(crate) fn compile(&mut self, job: &Job) -> Done {
        let started = self.timed.then(std::time::Instant::now);
        let mut inlines = Vec::new();
        let code = self.write(job, &mut inlines);
        Done {
            proto_id: job.proto_id,
            code,
            generation: job.generation,
            give: job.give,
            inlines,
            took: started.map(|started| started.elapsed()).unwrap_or_default(),
            used: self.memory.used(),
            committed: self.memory.committed(),
        }
    }

    fn write(&mut self, job: &Job, inlines: &mut Vec<(u32, u32)>) -> Option<(u64, Compiled)> {
        let address = self.memory.address(self.memory.next_offset());
        let counter = Box::new(AtomicU64::new(RECOMPILE_AFTER));
        let mut generator = Gen::new(&self.layout, job, &self.known);
        if job.generation + 1 < MOST_COMPILES && job.ops.len() <= RECOMPILE_MOST {
            generator.counter = Some(counter.as_ptr() as u64);
        }
        // Its feedback: the block it had, if it's compiled again, or a new one, which starts with its count.
        #[cfg(feature = "jit-opt")]
        let (feedback, feedback_size) = {
            let layout = super::feedback::layout(&job.ops);
            let block = match self.blocks.get_mut(&job.proto_id) {
                Some((block, size, knowns)) if *size == layout.size => Some((*block, std::mem::take(knowns))),
                Some(_) => None,
                None => self.memory.feedback(layout.size as usize).inspect(|&block| {
                    // SAFETY: the block is new, and the header's first four bytes are the count.
                    unsafe { (block as *mut i32).write(super::feedback::THRESHOLD) };
                    self.blocks.insert(job.proto_id, (block, layout.size, HashMap::default()));
                }).map(|block| (block, HashMap::default())),
            };
            match block {
                Some((block, knowns)) => {
                    generator.fb_block = block as u64;
                    generator.fb_offsets = layout.offsets.clone();
                    generator.fb_knowns = knowns;
                    (generator.fb_due, generator.fb_note) = self.routines;
                    (block, layout.size)
                }
                None => (0, 0),
            }
        };
        let made = generator.function(address);
        *inlines = std::mem::take(&mut generator.inlines);
        // What the calls' bits stand for now, kept for the next time it's compiled, and with its code.
        #[cfg(feature = "jit-opt")]
        let feedback_knowns: Box<[(u32, Box<[u32]>)]> = {
            let knowns = std::mem::take(&mut generator.fb_knowns);
            let mut list: Vec<(u32, Box<[u32]>)> =
                knowns.iter().map(|(&at, callees)| (at, callees.clone().into_boxed_slice())).collect();
            list.sort_unstable_by_key(|&(at, _)| at);
            if let Some(kept) = self.blocks.get_mut(&job.proto_id) {
                kept.2 = knowns;
            }
            list.into()
        };
        let (code, _, offsets, cold) = made?;
        let incoming: Arc<[Option<(u16, R)>]> = std::mem::take(&mut generator.incomings).into();
        let unguessed = std::mem::take(&mut generator.unguessed).into_boxed_slice();
        let inlined = generator.inline_calls;
        let entries = generator.targets.clone().into_boxed_slice();
        if !self.memory.has_room(code.len()) {
            return None;
        }
        let at = self.memory.add(&code)?;
        debug_assert_eq!(self.memory.address(at), address);
        let offsets: Arc<[u32]> = offsets.into();
        self.known.insert(
            job.proto_id,
            Known {
                address,
                offsets: offsets.clone(),
                incoming,
            },
        );
        let compiled = Compiled {
            address,
            offsets,
            entries,
            cold,
            inlined,
            // The slot holds the entry, which is just after the words of the header (see `HEADER`).
            worth: code[8] != 0,
            counter,
            unguessed,
            waits: 0,
            generation: job.generation,
            bytes: code.len(),
            #[cfg(feature = "jit-opt")]
            feedback,
            #[cfg(feature = "jit-opt")]
            feedback_size,
            #[cfg(feature = "jit-opt")]
            feedback_ops: (feedback != 0 && super::feedback::logging()).then(|| job.ops.clone()),
            #[cfg(feature = "jit-opt")]
            feedback_knowns,
            #[cfg(feature = "jit-opt")]
            feedback_offsets: if feedback != 0 { super::feedback::layout(&job.ops).offsets.into() } else { Arc::from([]) },
        };
        Some((compiled.slot(), compiled))
    }
}

/// The way in from Rust, a [`Trampoline`] (`rcx` the engine, `rdx` the frame on top, `r8` where to start): keeps the
/// registers Rust needs kept, sets up the ones compiled code works with (the frame's registers and caches from the
/// frame, the lists from where this engine has them, which never move), and calls `r8` as if it were the body of the
/// frame's function. What that hands back goes in `vm.jit.value`, and how it left is the result.
///
/// It works out `Jit::floor` first, half of the engine's stack limit below where its stack started, which compiled
/// calls compare the stack with; if Rust's stack is already below that, it gives back [`helpers::NOT_ENTERED`] at
/// once, and the interpreter runs the frame. Everything is in registers or the frame, so going in costs about what
/// starting the interpreter's loop does.
fn trampoline(bases: &Bases, l: &Layout) -> Vec<u8> {
    let mut a = Asm::new();
    let refuse = a.label();
    let done = a.label();
    for r in [RBX, RBP, RDI, RSI, R12, R13, R14, R15] {
        a.push(r);
    }
    a.mov_rr(VM, RCX);
    a.mov_rr(FRAME, RDX);
    a.load(RAX, m(VM, l.stack_base));
    a.load(R9, m(VM, l.stack_limit));
    a.shift_ri(Shift::Shr, R9, 1);
    a.alu_rr32(Alu::Xor, R10, R10);
    a.alu_rr(Alu::Sub, RAX, R9);
    a.cmov(B, RAX, R10);
    a.store(m(VM, l.floor), RAX);
    a.alu_rr(Alu::Cmp, RSP, RAX);
    a.jcc(B, refuse);
    a.load32(RAX, m(FRAME, l.f_base));
    a.mov_imm64(REGS, bases.stack);
    a.lea(REGS, mi(REGS, RAX, 3, 0));
    a.load32(RAX, m(FRAME, l.f_cache_base));
    a.imul_rri(RAX, RAX, l.cache_size);
    a.mov_imm64(ICS, bases.caches);
    a.alu_rr(Alu::Add, ICS, RAX);
    a.mov_imm64(OBJECTS, bases.objects);
    a.mov_imm64(CELLS, bases.cells);
    a.mov_imm64(INT, INT_TAG);
    a.mov_imm64(UNDEF, UNDEFINED);
    let enter = a.label();
    let call_at = a.label();
    a.jmp(call_at);
    a.bind(enter);
    a.alu_ri(Alu::Sub, RSP, NATIVE_FRAME);
    a.jmp_r(R8);
    a.bind(call_at);
    // Eight pushes and this leave the stack aligned for the call. A `call` to `enter`, written as one to a label.
    a.alu_ri(Alu::Sub, RSP, 8);
    a.call_label(enter);
    a.alu_ri(Alu::Add, RSP, 8);
    a.store(m(VM, l.value), RAX);
    a.mov_rr32(RAX, RDX);
    a.bind(done);
    for r in [R15, R14, R13, R12, RSI, RDI, RBP, RBX] {
        a.pop(r);
    }
    a.ret();
    a.bind(refuse);
    a.mov_imm64(RAX, u64::from(helpers::NOT_ENTERED));
    a.jmp(done);
    a.finish()
}

/// The two routines compiled code calls for its records, written once with the trampoline: the code, and where the
/// second starts in it.
///
/// - The first says the function on top's count has come to 0 (`helpers::opt_due`). It's called from where code calls
///   helpers, so its return address is under the 40 bytes those have, and it keeps that 40 again for the helper.
/// - The second records `r9d` in the call record's word at `r11`: if it says nothing yet, that; else, as it's another
///   (the code has compared it already), `MANY`.
#[cfg(feature = "jit-opt")]
fn feedback_routines() -> (Vec<u8>, usize) {
    let mut a = Asm::new();
    a.alu_ri(Alu::Sub, RSP, NATIVE_FRAME);
    a.mov_rr(RCX, VM);
    a.mov_imm64(RAX, helpers::opt_due as *const () as usize as u64);
    a.call_r(RAX);
    a.alu_ri(Alu::Add, RSP, NATIVE_FRAME);
    a.ret();
    let note = a.label();
    let many = a.label();
    a.bind(note);
    a.alu_mi32(Alu::Cmp, m(R11, 0), 0);
    a.jcc(NE, many);
    a.store32(m(R11, 0), R9);
    a.ret();
    a.bind(many);
    a.store32_imm(m(R11, 0), super::feedback::MANY);
    a.ret();
    let code = a.finish();
    let at = a.offset_of(note);
    (code, at)
}

/// Where an operand comes from: a register of the frame, or a value known when compiling.
#[derive(Clone, Copy)]
enum Src {
    Reg(u16),
    Imm(Value),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Rel {
    Lt,
    Le,
    Gt,
    Ge,
}

impl Rel {
    fn int_cc(self) -> Cc {
        match self {
            Rel::Lt => L,
            Rel::Le => LE,
            Rel::Gt => G,
            Rel::Ge => GE,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arith {
    Add,
    Sub,
    Mul,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bit {
    And,
    Or,
    Xor,
    Shl,
    Shr,
    UShr,
}

struct Gen<'a> {
    a: Asm,
    /// What the function is compiled from, and what the engine knew when it was due (see `Job`).
    job: &'a Job,
    /// The constants the instructions name (see `Job`).
    consts: &'a [(u32, Value)],
    /// Whether a template asked for a constant the job doesn't have, which leaves the function uncompiled.
    missing: std::cell::Cell<bool>,
    l: &'a Layout,
    proto_id: u32,
    proto: &'a Proto,
    ops: &'a [Op],
    /// Where each instruction starts.
    labels: Vec<Label>,
    /// Hands the status in `eax` back to the caller, with `vm.jit.value`.
    exit: Label,
    /// Hands back what's in `rax` and `edx`, as a call returned them.
    propagate: Label,
    /// Where a call that has made the frame itself comes in (see `call_known`).
    pre_body: Label,
    ics: u64,
    protos: u64,
    slots: u64,
    /// Where the engine's lists are.
    bases: Bases,
    /// Whether the processor has SSE4.1's `roundsd`, for `Math.floor` and the like.
    sse41: bool,
    /// Which instructions something jumps to (a jump, a handler, or the interpreter going in at a loop's top), where
    /// nothing can be known of the machine registers.
    targets: Vec<bool>,
    /// What the instruction being compiled can know from the one before: that this register of the frame is also in
    /// this machine register, as that one's template left it on every way it ends (the value is in the frame too). The
    /// template's first loads of its operands can use it; anything else it does in that register forgets it.
    incoming: Option<(u16, R)>,
    /// What the template being compiled says it leaves, for the next.
    known: Option<(u16, R)>,
    /// What `incoming` was at each instruction of the function, as it's compiled (see `Known::incoming`).
    incomings: Vec<Option<(u16, R)>>,
    /// The functions the writer has compiled so far, whose code a call can call directly, and a call compiled in
    /// line goes on in when it can't go on in line.
    compiled: &'a HashMap<u32, Known, FixedState>,
    /// The instruction being compiled.
    at: usize,
    /// While a callee's instructions are compiled in line (`inline_call`): how far its registers are from the running
    /// frame's, in registers, and its caches from the running function's, in bytes.
    shift: i32,
    ics_delta: i32,
    /// While a callee is compiled in line: where its instructions go when they can't go on in line, and where its
    /// value goes.
    inline: Option<Inlining>,
    /// How many of its callees' instructions this function has compiled in line, which is kept to `INLINE_BUDGET`,
    /// and how many calls, by their callee and where they are.
    inlined: usize,
    inline_calls: u32,
    inlines: Vec<(u32, u32)>,
    /// Where the count its calls that know nothing of what they call count down is, if they do (see `Gen::call`),
    /// and which they are.
    counter: Option<u64>,
    unguessed: Vec<u32>,
    /// The instructions of the loop the frame being compiled for is going round, from its top to the jump back, which
    /// have just run, so that what's in their registers says what they work on (see `looks_double`).
    looping: Option<(usize, usize)>,
    /// Where the function's feedback is, or 0 for none, and where each instruction's record is in it (see
    /// `jit::feedback`).
    #[cfg(feature = "jit-opt")]
    fb_block: u64,
    #[cfg(feature = "jit-opt")]
    fb_offsets: Box<[u32]>,
    /// For a branch being compiled: where its jump goes by way of its record (see `branch_target`), and where it
    /// really goes.
    #[cfg(feature = "jit-opt")]
    taken: Option<(Label, Label)>,
    /// For each call, by its instruction, the callees its record's bits stand for (see `note_known_callee`).
    #[cfg(feature = "jit-opt")]
    fb_knowns: HashMap<u32, Vec<u32>, FixedState>,
    /// Where the writer's two routines for records are (see `feedback_routines`): one that says a function's count has
    /// come to 0 (`count`), and one that records a callee in a call record's first word (`note_word`).
    #[cfg(feature = "jit-opt")]
    fb_due: u64,
    #[cfg(feature = "jit-opt")]
    fb_note: u64,
}

/// What a call reads its callee as first (see `Gen::call`): nothing (it's in its register), or a method of the value
/// in the next, at a place that has seen one shape or several.
#[derive(Clone, Copy)]
enum Method {
    None,
    Mono(u16),
    Poly(u16),
}

/// What `Gen::function` makes: the code, where its body is, and where each instruction's code and cold paths start.
type Made = (Vec<u8>, usize, Box<[u32]>, Box<[u32]>);

/// A callee being compiled in line (see `Gen::inline_call`).
struct Inlining {
    /// For each of its instructions, where the code makes its frame and goes on in its own code from there.
    deopts: Vec<Label>,
    /// The caller's register for what it returns, and where the call goes on after.
    dst: u16,
    end: Label,
}

/// The longest callee compiled in line, in instructions, and how many instructions of callees a function may have
/// in line in all.
const INLINE_MOST: usize = 40;
const INLINE_BUDGET: usize = 600;

impl<'a> Gen<'a> {
    fn new(l: &'a Layout, job: &'a Job, compiled: &'a HashMap<u32, Known, FixedState>) -> Self {
        let (proto_id, proto, ops, bases) = (job.proto_id, &job.proto, &*job.ops, job.bases);
        let mut a = Asm::new();
        let labels = (0..ops.len()).map(|_| a.label()).collect();
        let exit = a.label();
        let propagate = a.label();
        let pre_body = a.label();
        let mut targets = vec![false; ops.len()];
        for (at, op) in ops.iter().enumerate() {
            if let Some(offset) = super::jump_offset(op) {
                let target = at as i64 + 1 + i64::from(offset);
                if let Some(place) = usize::try_from(target).ok().and_then(|t| targets.get_mut(t)) {
                    *place = true;
                }
            }
        }
        for handler in proto.handlers.iter() {
            if let Some(place) = targets.get_mut(handler.target as usize) {
                *place = true;
            }
        }
        // A generator carries on after where it stopped, from Rust.
        for (at, op) in ops.iter().enumerate() {
            if matches!(op, Op::InitialYield | Op::Yield { .. })
                && let Some(place) = targets.get_mut(at + 1)
            {
                *place = true;
            }
        }
        Self {
            a,
            job,
            consts: &job.consts,
            missing: std::cell::Cell::new(false),
            l,
            proto_id,
            proto,
            ops,
            labels,
            exit,
            propagate,
            pre_body,
            ics: bases.caches + u64::from(proto.cache_base) * size_of::<Cache>() as u64,
            protos: bases.protos,
            slots: bases.slots,
            bases,
            sse41: std::arch::is_x86_feature_detected!("sse4.1"),
            targets,
            incoming: None,
            known: None,
            incomings: Vec::with_capacity(ops.len()),
            compiled,
            at: 0,
            shift: 0,
            ics_delta: 0,
            inline: None,
            inlined: 0,
            inline_calls: 0,
            inlines: Vec::new(),
            counter: None,
            unguessed: Vec::new(),
            looping: job.looping,
            #[cfg(feature = "jit-opt")]
            fb_block: 0,
            #[cfg(feature = "jit-opt")]
            fb_offsets: Box::default(),
            #[cfg(feature = "jit-opt")]
            taken: None,
            #[cfg(feature = "jit-opt")]
            fb_knowns: HashMap::default(),
            #[cfg(feature = "jit-opt")]
            fb_due: 0,
            #[cfg(feature = "jit-opt")]
            fb_note: 0,
        }
    }

    /// The cache entry at `at` in the engine's list as the job has it, if it does (see `read_place`).
    fn cache_entry(&self, at: u32) -> Option<Cache> {
        let found = self.job.caches.binary_search_by_key(&at, |&(place, _)| place).ok()?;
        Some(self.job.caches[found].1)
    }

    /// What the job knows of a callee (see `Job::callees`).
    fn callee(&self, known: u32) -> Option<&'a Callee> {
        let callees: &'a [Callee] = &self.job.callees;
        let found = callees.binary_search_by_key(&known, |callee| callee.proto_id).ok()?;
        Some(&callees[found])
    }

    /// The engine's constant at `index`, which an instruction names (see [`constant_of`]).
    fn constant(&self, index: usize) -> Value {
        match self.consts.binary_search_by_key(&(index as u32), |&(at, _)| at) {
            Ok(found) => self.consts[found].1,
            Err(_) => {
                debug_assert!(false, "a template read a constant `constant_of` doesn't name");
                self.missing.set(true);
                Value::UNDEFINED
            }
        }
    }

    /// The instruction's place in the engine's code, which is what a frame's `pc` counts in.
    fn pc(&self, i: usize) -> u32 {
        self.proto.start + i as u32
    }

    fn reg(&self, index: u16) -> Mem {
        m(REGS, 8 * (self.shift + i32::from(index)))
    }

    /// A cache entry's field.
    fn cache(&self, cache: u16, field: i32) -> Mem {
        m(ICS, self.ics_delta + i32::from(cache) * self.l.cache_size + field)
    }

    /// Where the instruction being compiled goes when it can't go on, if it's a callee's compiled in line (see
    /// `inline_call`): instead of any helper that needs its frame.
    fn deopt(&self) -> Option<Label> {
        self.inline.as_ref().map(|inlining| inlining.deopts[self.at])
    }

    /// The running function's captured variables, in `r`: its frame's, or a callee's compiled in line, from its
    /// closure (whose list starts after the count).
    fn upvalues(&mut self, r: R) {
        if self.inline.is_some() {
            // Found once, as the call went in line (see `inline_call`).
            self.a.load(r, m(RSP, SPARE));
        } else {
            self.a.load(r, m(FRAME, self.l.f_upvalues));
        }
    }

    fn target(&mut self, i: usize, offset: i32) -> Label {
        let to = self.labels[(i as i64 + 1 + i64::from(offset)) as usize];
        #[cfg(feature = "jit-opt")]
        if offset >= 0 && i == self.at {
            return self.branch_target(to);
        }
        to
    }

    fn load(&mut self, dst: R, src: Src) {
        match src {
            Src::Reg(index) => self.a.load(dst, self.reg(index)),
            Src::Imm(value) => self.a.mov_imm64(dst, value.0),
        }
    }

    /// An operand, as `load` gives it, from the machine register the instruction before left it in if it did (see
    /// `incoming`). Only for a template's first loads, before it does anything else in those registers.
    fn operand(&mut self, dst: R, src: Src) {
        if let (Src::Reg(index), Some((known, r))) = (src, self.incoming)
            && known == index
        {
            if r != dst {
                self.a.mov_rr(dst, r);
            }
            return;
        }
        if self.incoming.is_some_and(|(_, r)| r == dst) {
            self.incoming = None;
        }
        self.load(dst, src);
    }

    /// A register of the frame as an operand (see `operand`).
    fn operand_reg(&mut self, dst: R, index: u16) {
        self.operand(dst, Src::Reg(index));
    }

    /// Says the template being compiled leaves register `dst` of the frame in `r` on every way it ends.
    fn leaves(&mut self, dst: u16, r: R) {
        self.known = Some((dst, r));
    }

    /// A cold path that does instruction `i` the generic way and goes back to `done` with register `dst` of the frame
    /// in `r` too, for a template that `leaves` it.
    fn cold_generic_leaving(&mut self, i: usize, at: Label, done: Label, dst: u16, r: R) {
        let was = self.a.is_cold();
        self.a.set_cold(true);
        self.a.bind(at);
        self.generic(i);
        self.a.load(r, self.reg(dst));
        self.a.jmp(done);
        self.a.set_cold(was);
    }

    fn call_helper(&mut self, f: usize) {
        self.a.mov_imm64(RAX, f as u64);
        self.a.call_r(RAX);
    }

    /// Calls `helpers::op` for instruction `i`, and leaves if it says to.
    fn generic(&mut self, i: usize) {
        if let Some(deopt) = self.deopt() {
            self.a.jmp(deopt);
            return;
        }
        self.a.mov_rr(RCX, VM);
        self.a.mov_imm64(RDX, u64::from(self.pc(i)));
        self.call_helper(helpers::op as *const () as usize);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NE, self.exit);
    }

    /// Calls `helpers::op` for a jump: on to `taken` if it says so, else `not_taken`.
    fn generic_jump(&mut self, i: usize, taken: Label, not_taken: Label) {
        if let Some(deopt) = self.deopt() {
            self.a.jmp(deopt);
            return;
        }
        self.a.mov_rr(RCX, VM);
        self.a.mov_imm64(RDX, u64::from(self.pc(i)));
        self.call_helper(helpers::op as *const () as usize);
        self.a.alu_ri32(Alu::Cmp, RAX, TAKEN as i32);
        self.a.jcc(E, taken);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NE, self.exit);
        self.a.jmp(not_taken);
    }

    /// Calls `helper(vm, pc)` for instruction `i`, a helper for that one instruction, and leaves if it says to.
    fn direct(&mut self, i: usize, helper: extern "C" fn(&mut Vm, u32) -> u32) {
        if let Some(deopt) = self.deopt() {
            self.a.jmp(deopt);
            return;
        }
        self.a.mov_rr(RCX, VM);
        self.a.mov_imm64(RDX, u64::from(self.pc(i)));
        self.call_helper(helper as *const () as usize);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NE, self.exit);
    }

    /// A cold path that does instruction `i` by `helper`, a helper for that one instruction, and goes back to `done`.
    fn cold_direct(&mut self, i: usize, at: Label, done: Label, helper: extern "C" fn(&mut Vm, u32) -> u32) {
        let was = self.a.is_cold();
        self.a.set_cold(true);
        self.a.bind(at);
        self.direct(i, helper);
        self.a.jmp(done);
        self.a.set_cold(was);
    }

    /// A cold path that does instruction `i` the generic way and goes back to `done`.
    fn cold_generic(&mut self, i: usize, at: Label, done: Label) {
        let was = self.a.is_cold();
        self.a.set_cold(true);
        self.a.bind(at);
        self.generic(i);
        self.a.jmp(done);
        self.a.set_cold(was);
    }

    /// Jumps to `fail` unless `v` is an int. Uses r10.
    fn guard_int(&mut self, v: R, fail: Label) {
        self.a.mov_rr(R10, v);
        self.a.shift_ri(Shift::Shr, R10, 32);
        self.a.alu_ri32(Alu::Cmp, R10, (INT_TAG >> 32) as u32 as i32);
        self.a.jcc(NE, fail);
    }

    /// Jumps to `fail` unless both are ints. Uses r10 and r11.
    fn guard_ints(&mut self, x: R, y: R, fail: Label) {
        self.a.mov_rr(R10, x);
        self.a.alu_rr(Alu::Xor, R10, INT);
        self.a.mov_rr(R11, y);
        self.a.alu_rr(Alu::Xor, R11, INT);
        self.a.alu_rr(Alu::Or, R10, R11);
        self.a.shift_ri(Shift::Shr, R10, 32);
        self.a.jcc(NE, fail);
    }

    /// Jumps to `fail` unless `v` is an object, and leaves its address in `v`. Uses r10.
    fn object_address(&mut self, v: R, fail: Label) {
        self.a.mov_rr(R10, v);
        self.a.shift_ri(Shift::Shr, R10, 48);
        self.a.alu_ri32(Alu::Cmp, R10, TAG_OBJECT);
        self.a.jcc(NE, fail);
        self.a.mov_rr32(v, v);
        self.a.shift_ri(Shift::Shl, v, 6);
        self.a.alu_rr(Alu::Add, v, OBJECTS);
    }

    /// The number `v` is, as a double in `x`, or on to `fail` if it isn't one. Uses r10.
    fn to_double(&mut self, v: R, x: X, fail: Label) {
        let is_int = self.a.label();
        let done = self.a.label();
        self.a.mov_rr(R10, v);
        self.a.shift_ri(Shift::Shr, R10, 48);
        self.a.alu_ri32(Alu::Cmp, R10, TAG_INT);
        self.a.jcc(E, is_int);
        self.a.jcc(A, fail);
        self.a.movq_xr(x, v);
        self.a.jmp(done);
        self.a.bind(is_int);
        self.a.cvtsi2sd(x, v);
        self.a.bind(done);
    }

    /// The operand as a double in `x`: loaded into `v` first unless it's known.
    fn src_double(&mut self, src: Src, v: R, x: X, fail: Label) {
        match src {
            Src::Imm(value) if value.is_int() => {
                self.a.mov_imm64(v, u64::from(value.as_int() as u32));
                self.a.cvtsi2sd(x, v);
            }
            Src::Imm(value) if value.is_double() => {
                self.a.mov_imm64(v, value.0);
                self.a.movq_xr(x, v);
            }
            _ => self.to_double(v, x, fail),
        }
    }

    /// What a bitwise operator makes of `v`, in its low 32 bits, or on to `fail`. Uses r10 and xmm0.
    fn to_int32(&mut self, v: R, fail: Label) {
        let done = self.a.label();
        self.a.mov_rr(R10, v);
        self.a.shift_ri(Shift::Shr, R10, 48);
        self.a.alu_ri32(Alu::Cmp, R10, TAG_INT);
        self.a.jcc(E, done);
        self.a.jcc(A, fail);
        self.a.movq_xr(XMM0, v);
        self.a.cvttsd2si64(v, XMM0);
        // A double too big for 64 bits (or NaN) comes out as this, and goes the slow way, as in the interpreter.
        self.a.mov_imm64(R10, 1 << 63);
        self.a.alu_rr(Alu::Cmp, v, R10);
        self.a.jcc(E, fail);
        self.a.bind(done);
    }

    /// Stores what's in `x` as `Value::number` would make it: an int if it's a whole one, else the double, with NaN
    /// as the one NaN.
    fn store_number(&mut self, dst: u16, x: X) {
        let as_double = self.a.label();
        let as_int = self.a.label();
        let nan = self.a.label();
        let done = self.a.label();
        self.a.cvttsd2si(RAX, x);
        self.a.cvtsi2sd(XMM2, RAX);
        self.a.ucomisd(x, XMM2);
        self.a.jcc(NE, as_double);
        self.a.jcc(P, as_double);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NE, as_int);
        // Zero: a negative one stays a double.
        self.a.movq_rx(RCX, x);
        self.a.test_rr(RCX, RCX);
        self.a.jcc(S, as_double);
        self.a.bind(as_int);
        self.a.alu_rr(Alu::Or, RAX, INT);
        self.a.store(self.reg(dst), RAX);
        self.a.jmp(done);
        self.a.bind(as_double);
        self.a.ucomisd(x, x);
        self.a.jcc(P, nan);
        self.a.movq_rx(RAX, x);
        self.a.store(self.reg(dst), RAX);
        self.a.jmp(done);
        self.a.bind(nan);
        self.a.mov_imm64(RAX, CANONICAL_NAN);
        self.a.store(self.reg(dst), RAX);
        self.a.bind(done);
    }

    /// Collects garbage if enough has been made, as `safe_point!` does. `pc` is what the frame says meanwhile.
    fn safe_point(&mut self, pc: u32) {
        let collect = self.a.label();
        let back = self.a.label();
        self.a.load(RAX, m(VM, self.l.debt));
        self.a.alu_rm(Alu::Cmp, RAX, m(VM, self.l.threshold));
        self.a.jcc(A, collect);
        self.a.bind(back);
        let was = self.a.is_cold();
        self.a.set_cold(true);
        self.a.bind(collect);
        self.a.mov_rr(RCX, VM);
        self.a.mov_imm64(RDX, u64::from(pc));
        self.call_helper(helpers::collect as *const () as usize);
        self.a.jmp(back);
        self.a.set_cold(was);
    }

    /// Leaves for the interpreter, which does instruction `i` from its start.
    fn bail_stub(&mut self, i: usize) -> Label {
        let was = self.a.is_cold();
        self.a.set_cold(true);
        let here = self.a.label();
        self.a.bind(here);
        self.a.store32_imm(m(FRAME, self.l.f_pc), self.pc(i));
        self.a.mov_imm64(RDX, u64::from(BAIL));
        self.a.alu_ri(Alu::Add, RSP, NATIVE_FRAME);
        self.a.ret();
        self.a.set_cold(was);
        here
    }

    /// Reads slot `slot` (a register holding its number) of the object at `object` into `out`. Uses r10.
    fn read_slot(&mut self, object: R, slot: R, out: R) {
        let o = self.l.obj;
        self.a.lea(R10, m(object, o.inline_slots as i32));
        self.a.cmp_m8(m(object, o.slot_count as i32), o.on_heap);
        self.a.cmov_rm(E, R10, m(object, o.heap_slots as i32));
        self.a.load(out, mi(R10, slot, 3, 0));
    }

    /// Reads slot number `slot` of the object at `object` into `out`. Uses r10.
    fn read_slot_at(&mut self, object: R, slot: u32, out: R) {
        let o = self.l.obj;
        self.a.lea(R10, m(object, o.inline_slots as i32));
        self.a.cmp_m8(m(object, o.slot_count as i32), o.on_heap);
        self.a.cmov_rm(E, R10, m(object, o.heap_slots as i32));
        self.a.load(out, m(R10, 8 * slot as i32));
    }

    /// Where slot number `slot` of the object at `object` is, in r10.
    fn slot_place_at(&mut self, object: R, slot: u32) {
        let o = self.l.obj;
        self.a.lea(R10, m(object, o.inline_slots as i32));
        self.a.cmp_m8(m(object, o.slot_count as i32), o.on_heap);
        self.a.cmov_rm(E, R10, m(object, o.heap_slots as i32));
        self.a.lea(R10, m(R10, 8 * slot as i32));
    }

    /// What read place `cache`'s first entry knew when the job was taken, to be written into the code as constants:
    /// the shape, the object it's on if that's not the object itself (whose prototypes mustn't have changed since,
    /// which the code checks by the epoch now), and the slot. A shape stays as it is, unless it's a dictionary's, which
    /// isn't used. Once it's freed, no object has it, and its number isn't given to another (see `Job::plain`), so the
    /// code then goes the cache's way. What it's on is alive while an object with the shape is, and the epoch is the
    /// same.
    fn fixed_read(&mut self, cache: u16) -> Option<(u32, Option<u32>, u32)> {
        let entry = self.cache_entry(self.proto.cache_base + u32::from(cache))?;
        self.fixed_entry(entry)
    }

    /// `fixed_read` for each entry of a polymorphic read place (its first and the others it keeps), as many as know
    /// something that can be written into the code.
    fn fixed_poly(&mut self, cache: u16) -> Vec<(u32, Option<u32>, u32)> {
        let at = self.proto.cache_base + u32::from(cache);
        let Some(first) = self.cache_entry(at) else {
            return Vec::new();
        };
        let mut entries = vec![first];
        if first.more != crate::object::NONE && first.more & crate::op::MEGAMORPHIC == 0 {
            let others = at + first.more;
            entries.extend((others..others + crate::op::POLY as u32).filter_map(|at| self.cache_entry(at)));
        }
        let mut found: Vec<(u32, Option<u32>, u32)> = Vec::new();
        for entry in entries {
            if let Some(fixed) = self.fixed_entry(entry)
                && !found.iter().any(|known| known.0 == fixed.0)
            {
                found.push(fixed);
            }
        }
        found
    }

    /// Whether a shape a cache entry names can be written into the code (see `Job::plain`).
    fn plain(&self, shape: u32) -> bool {
        self.job.plain.binary_search(&shape).is_ok()
    }

    fn fixed_entry(&mut self, entry: Cache) -> Option<(u32, Option<u32>, u32)> {
        if !self.plain(entry.shape) || i32::try_from(entry.slot).is_err() || entry.slot > (1 << 24) {
            return None;
        }
        let holder = if entry.other == crate::object::NONE {
            None
        } else if entry.epoch == self.job.epoch {
            Some(entry.other)
        } else {
            return None;
        };
        Some((entry.shape, holder, entry.slot))
    }

    /// What write place `cache`'s first entry knew, as `fixed_read` does: the shape and the slot of a write to a
    /// property the object has. (An add goes the cache's way, which the collector clears of a shape it frees: the code
    /// mustn't give an object a shape that's gone.)
    fn fixed_write(&mut self, cache: u16) -> Option<(u32, u32)> {
        let entry = self.cache_entry(self.proto.cache_base + u32::from(cache))?;
        if !self.plain(entry.shape) || entry.slot > (1 << 24) || entry.other != crate::object::NONE {
            return None;
        }
        Some((entry.shape, entry.slot))
    }

    /// Where slot `slot` of the object at `object` is, in r10.
    fn slot_place(&mut self, object: R, slot: R) {
        let o = self.l.obj;
        self.a.lea(R10, m(object, o.inline_slots as i32));
        self.a.cmp_m8(m(object, o.slot_count as i32), o.on_heap);
        self.a.cmov_rm(E, R10, m(object, o.heap_slots as i32));
        self.a.lea(R10, mi(R10, slot, 3, 0));
    }

    // The function.

    /// The whole function: two words saying where its body is and where a `new` from compiled code comes in (or 0),
    /// the way in for a call from compiled code, the body, and the cold paths. With where the body is and where each
    /// instruction is.
    fn function(&mut self, address: usize) -> Option<Made> {
        let worth = self.ops.len() >= WORTH_ENTERING
            || self.ops.iter().enumerate().any(|(at, op)| {
                matches!(
                    op,
                    Op::Call { .. }
                        | Op::CallPlain { .. }
                        | Op::CallMethod { .. }
                        | Op::CallMethodPoly { .. }
                        | Op::New { .. }
                        | Op::CallSpread { .. }
                        | Op::NewSpread { .. }
                        | Op::SuperCall { .. }
                        | Op::SuperCallSpread { .. }
                ) || super::jump_offset(op).is_some_and(|offset| i64::from(offset) < 0 && at > 0)
            });
        self.a.data64(0);
        self.a.data64(u64::from(worth));
        self.a.data64(0);
        self.a.data64(0);
        self.a.nop();
        let body = self.a.label();
        self.prologue(false, body);
        // Where a call that has made the frame comes in (see `call_known`).
        self.a.bind(self.pre_body);
        self.a.alu_ri(Alu::Sub, RSP, NATIVE_FRAME);
        self.a.mov_imm64(ICS, self.ics);
        self.a.bind(body);
        // A call counts towards the optimizing tier (see `jit::feedback::due`).
        #[cfg(feature = "jit-opt")]
        self.count();
        let mut cold = Vec::with_capacity(self.ops.len());
        for i in 0..self.ops.len() {
            self.a.bind(self.labels[i]);
            let cold_start = self.a.label();
            self.a.set_cold(true);
            self.a.bind(cold_start);
            self.a.set_cold(false);
            cold.push(cold_start);
            if !self.op(i) {
                return None;
            }
            #[cfg(feature = "jit-opt")]
            self.branch_went();
        }
        // Nothing falls off the end of a function, but in case: leave for the interpreter.
        let end = self.bail_stub(self.ops.len() - 1);
        self.a.jmp(end);
        self.a.set_cold(true);
        self.a.bind(self.exit);
        self.a.mov_rr32(RDX, RAX);
        self.a.load(RAX, m(VM, self.l.value));
        self.a.bind(self.propagate);
        if !self.proto.handlers.is_empty() {
            self.catch();
        }
        self.a.alu_ri(Alu::Add, RSP, NATIVE_FRAME);
        self.a.ret();
        let construct = if self.proto.flags & IS_CONSTRUCTOR != 0 {
            let entry = self.a.label();
            self.a.bind(entry);
            self.prologue(true, body);
            Some(entry)
        } else {
            None
        };
        self.a.set_cold(false);
        if self.missing.get() {
            return None;
        }
        let mut code = self.a.finish();
        self.a.place(&mut code, address as u64);
        let body_at = self.a.offset_of(body);
        Asm::patch64(&mut code, 0, (address + self.a.offset_of(self.pre_body)) as u64);
        Asm::patch64(&mut code, 16, (address + body_at) as u64);
        if let Some(entry) = construct {
            Asm::patch64(&mut code, 24, (address + self.a.offset_of(entry)) as u64);
        }
        let offsets = self.labels.iter().map(|&label| self.a.offset_of(label) as u32).collect();
        let cold = cold.iter().map(|&label| self.a.offset_of(label) as u32).collect();
        Some((code, body_at, offsets, cold))
    }

    /// On the way out with an error (status in `edx`, error in `rax`): a `catch` or `finally` of this function that's
    /// around where it happened takes it, as `Vm::unwind` would, and the code carries on there; else on out.
    fn catch(&mut self) {
        let leave = self.a.label();
        self.a.alu_ri32(Alu::Cmp, RDX, helpers::THROW as i32);
        self.a.jcc(NE, leave);
        self.a.mov_rr(RCX, VM);
        self.a.mov_rr(RDX, RAX);
        self.a.mov_rr(R8, FRAME);
        self.call_helper(helpers::catch as *const () as usize);
        let mut targets: Vec<u32> = self.proto.handlers.iter().map(|handler| handler.target).collect();
        targets.sort_unstable();
        targets.dedup();
        for target in targets {
            self.a.alu_ri32(Alu::Cmp, RAX, target as i32);
            self.a.jcc(E, self.labels[target as usize]);
        }
        // Nothing here takes it.
        self.a.mov_imm64(RDX, u64::from(helpers::THROW));
        self.a.load(RAX, m(VM, self.l.value));
        self.a.bind(leave);
    }

    /// A call from compiled code comes here with: `rbx` and `r12` at the new frame's registers and place, `eax` its
    /// base, `ecx` how many arguments, `edx` the closure, `r8` its captured variables, `r9w` where the result goes, and
    /// for a `new` (`construct`), `r10` the `new.target`. This makes the frame as the interpreter's `enter!` does, then
    /// goes on to `body`.
    fn prologue(&mut self, construct: bool, body: Label) {
        let l = self.l;
        let proto = self.proto;
        let registers = i32::from(proto.registers);
        let params = i32::from(proto.params);
        let clear = proto.clear() as i32;
        let check = self.a.label();
        let grow = self.a.label();
        let fill = self.a.label();
        let filled = self.a.label();
        let keeps = if proto.flags & KEEPS_ARGS != 0 { crate::vm::KEPT_ARGS } else { 0 };
        // What's free to work with: r10 has the `new.target` in a `new`.
        let scratch = if construct { R11 } else { R10 };
        let was = self.a.is_cold();
        self.a.alu_ri(Alu::Sub, RSP, NATIVE_FRAME);
        self.a.bind(check);
        self.a.load(scratch, m(VM, l.frames_len));
        self.a.alu_rm(Alu::Cmp, scratch, m(VM, l.frames_committed));
        self.a.jcc(AE, grow);
        self.a.lea32(scratch, m(RAX, registers + 16));
        self.a.alu_rm(Alu::Cmp, scratch, m(VM, l.stack_len));
        self.a.jcc(A, grow);
        self.a.store32_imm(m(FRAME, l.f_pc), proto.start);
        self.a.store32(m(FRAME, l.f_base), RAX);
        self.a.alu_ri32(Alu::Add, RAX, registers);
        self.a.store32(m(FRAME, l.f_top), RAX);
        self.a.store32_imm(m(FRAME, l.f_proto), self.proto_id);
        self.a.store32(m(FRAME, l.f_closure), RDX);
        self.a.store32_imm(m(FRAME, l.f_cache_base), proto.cache_base);
        self.a.store16(m(FRAME, l.f_ret_dst), R9);
        if construct {
            self.a.store8_imm(m(FRAME, l.f_flags), crate::vm::CONSTRUCTING | keeps);
            self.a.store(m(FRAME, l.f_new_target), R10);
        } else {
            self.a.store8_imm(m(FRAME, l.f_flags), keeps);
            self.a.store(m(FRAME, l.f_new_target), UNDEF);
        }
        self.a.store(m(FRAME, l.f_upvalues), R8);
        self.a.inc_m(m(VM, l.frames_len));
        if keeps != 0 {
            // Its arguments are kept before any of their registers are made undefined, as `enter_function` does.
            self.a.push(RCX);
            self.a.alu_ri(Alu::Sub, RSP, NATIVE_FRAME);
            self.a.mov_rr32(RDX, RCX);
            self.a.mov_rr(RCX, VM);
            self.call_helper(helpers::keep_args as *const () as usize);
            self.a.alu_ri(Alu::Add, RSP, NATIVE_FRAME);
            self.a.pop(RCX);
        }
        // The parameters with no argument, and the function's own variables, are undefined.
        if params > 0 {
            self.a.alu_ri32(Alu::Cmp, RCX, params);
            self.a.jcc(B, fill);
        }
        self.a.bind(filled);
        for at in 1 + params..clear {
            self.a.store(m(REGS, 8 * at), UNDEF);
        }
        self.a.mov_imm64(ICS, self.ics);
        self.a.jmp(body);
        // The cold paths.
        self.a.set_cold(true);
        if params > 0 {
            let again = self.a.label();
            self.a.bind(fill);
            self.a.lea(R10, mi(REGS, RCX, 3, 8));
            self.a.lea(R11, m(REGS, 8 * (1 + params)));
            self.a.bind(again);
            self.a.store(m(R10, 0), UNDEF);
            self.a.alu_ri(Alu::Add, R10, 8);
            self.a.alu_rr(Alu::Cmp, R10, R11);
            self.a.jcc(B, again);
            self.a.jmp(filled);
        }
        // No room for the frame: make some, or throw, with the caller's frame on top.
        let failed = self.a.label();
        self.a.bind(grow);
        let kept: &[R] = if construct {
            &[RAX, RCX, RDX, R8, R9, R10]
        } else {
            &[RAX, RCX, RDX, R8, R9]
        };
        for &r in kept {
            self.a.push(r);
        }
        // The pushes and this leave the stack aligned, with room for the helper's 32 bytes.
        let room = if kept.len() % 2 == 0 { 32 } else { 40 };
        self.a.alu_ri(Alu::Sub, RSP, room);
        self.a.lea32(RDX, m(RAX, registers + 16));
        self.a.mov_rr(RCX, VM);
        self.call_helper(helpers::grow as *const () as usize);
        self.a.alu_ri(Alu::Add, RSP, room);
        self.a.mov_rr32(R11, RAX);
        for &r in kept.iter().rev() {
            self.a.pop(r);
        }
        self.a.test_rr32(R11, R11);
        self.a.jcc(NE, failed);
        self.a.jmp(check);
        self.a.bind(failed);
        self.a.mov_rr32(RDX, R11);
        self.a.load(RAX, m(VM, l.value));
        self.a.alu_ri(Alu::Add, RSP, NATIVE_FRAME);
        self.a.ret();
        self.a.set_cold(was);
    }

    /// One instruction. False if it can't be compiled.
    fn op(&mut self, i: usize) -> bool {
        let op = self.ops[i];
        self.at = i;
        // What the last template left in a machine register holds here, unless something jumps here.
        self.incoming = if self.targets[i] { None } else { self.known.take() };
        self.known = None;
        if self.inline.is_none() {
            self.incomings.push(self.incoming);
        }
        match op {
            Op::LoadConst { dst, index } => {
                let value = self.constant(index as usize);
                self.a.mov_imm64(RAX, value.0);
                self.a.store(self.reg(dst), RAX);
                self.leaves(dst, RAX);
            }
            Op::LoadInt { dst, value } => {
                self.a.mov_imm64(RAX, Value::int(value).0);
                self.a.store(self.reg(dst), RAX);
                self.leaves(dst, RAX);
            }
            Op::LoadUndefined { dst } => self.a.store(self.reg(dst), UNDEF),
            Op::LoadNull { dst } => self.store_imm(dst, NULL),
            Op::LoadTrue { dst } => self.store_imm(dst, TRUE),
            Op::LoadFalse { dst } => self.store_imm(dst, FALSE),
            Op::LoadEmpty { dst } => self.store_imm(dst, EMPTY),
            Op::Move { dst, src } => {
                self.operand_reg(RAX, src);
                self.a.store(self.reg(dst), RAX);
                self.leaves(dst, RAX);
            }

            Op::Add { dst, a, b } => self.arith(i, dst, Src::Reg(a), Src::Reg(b), Arith::Add),
            Op::Sub { dst, a, b } => self.arith(i, dst, Src::Reg(a), Src::Reg(b), Arith::Sub),
            Op::Mul { dst, a, b } => self.arith(i, dst, Src::Reg(a), Src::Reg(b), Arith::Mul),
            Op::AddInt { dst, a, value } => {
                self.arith(i, dst, Src::Reg(a), Src::Imm(Value::int(value.into())), Arith::Add)
            }
            Op::SubInt { dst, a, value } => {
                self.arith(i, dst, Src::Reg(a), Src::Imm(Value::int(value.into())), Arith::Sub)
            }
            Op::MulInt { dst, a, value } => {
                self.arith(i, dst, Src::Reg(a), Src::Imm(Value::int(value.into())), Arith::Mul)
            }
            Op::AddInt32 { dst, a, b } => {
                // Two ints add round, as `| 0` makes them. Other numbers (a product that was 0 is a double) are added
                // as doubles, and the sum's low 32 bits are what `| 0` makes of it, while it fits in 64 bits.
                let doubles = self.a.label();
                let slow = self.a.label();
                let done = self.a.label();
                self.operand_reg(RAX, a);
                self.operand_reg(RCX, b);
                self.guard_ints(RAX, RCX, doubles);
                self.a.alu_rr32(Alu::Add, RAX, RCX);
                self.a.alu_rr(Alu::Or, RAX, INT);
                self.a.store(self.reg(dst), RAX);
                self.a.bind(done);
                self.a.set_cold(true);
                self.a.bind(doubles);
                self.note(DOUBLE);
                self.to_double(RAX, XMM0, slow);
                self.to_double(RCX, XMM1, slow);
                self.a.sd(Sd::Add, XMM0, XMM1);
                self.a.cvttsd2si64(RAX, XMM0);
                self.a.mov_imm64(R10, 1 << 63);
                self.a.alu_rr(Alu::Cmp, RAX, R10);
                self.a.jcc(E, slow);
                self.a.mov_rr32(RAX, RAX);
                self.a.alu_rr(Alu::Or, RAX, INT);
                self.a.store(self.reg(dst), RAX);
                self.a.jmp(done);
                self.a.set_cold(false);
                self.cold_generic_leaving(i, slow, done, dst, RAX);
                self.leaves(dst, RAX);
            }
            Op::DivInt { dst, a, value } => self.divide_int(i, dst, a, value),
            Op::AddK { dst, a, k } => self.arith_k(i, dst, a, k, Arith::Add),
            Op::SubK { dst, a, k } => self.arith_k(i, dst, a, k, Arith::Sub),
            Op::MulK { dst, a, k } => self.arith_k(i, dst, a, k, Arith::Mul),
            Op::Div { dst, a, b } => self.divide(i, dst, Src::Reg(a), Src::Reg(b)),
            Op::DivK { dst, a, k } => {
                let value = self.constant(k as usize);
                if value.is_number() {
                    self.divide(i, dst, Src::Reg(a), Src::Imm(value));
                } else {
                    self.generic(i);
                }
            }
            Op::Mod { dst, a, b } => self.modulo(i, dst, a, Src::Reg(b)),
            Op::ModInt { dst, a, value } => {
                self.modulo(i, dst, a, Src::Imm(Value::int(value.into())))
            }
            Op::ModK { dst, a, k } => {
                let value = self.constant(k as usize);
                if value.is_number() {
                    self.modulo(i, dst, a, Src::Imm(value));
                } else {
                    self.generic(i);
                }
            }
            Op::BitAnd { dst, a, b } => self.bitwise(i, dst, a, Src::Reg(b), Bit::And),
            Op::BitOr { dst, a, b } => self.bitwise(i, dst, a, Src::Reg(b), Bit::Or),
            Op::BitXor { dst, a, b } => self.bitwise(i, dst, a, Src::Reg(b), Bit::Xor),
            Op::Shl { dst, a, b } => self.bitwise(i, dst, a, Src::Reg(b), Bit::Shl),
            Op::Shr { dst, a, b } => self.bitwise(i, dst, a, Src::Reg(b), Bit::Shr),
            Op::UShr { dst, a, b } => self.bitwise(i, dst, a, Src::Reg(b), Bit::UShr),
            Op::BitAndInt { dst, a, value } => self.bitwise_int(i, dst, a, value, Bit::And),
            Op::BitOrInt { dst, a, value } => self.bitwise_int(i, dst, a, value, Bit::Or),
            Op::ShlInt { dst, a, value } => self.bitwise_int(i, dst, a, value, Bit::Shl),
            Op::ShrInt { dst, a, value } => self.bitwise_int(i, dst, a, value, Bit::Shr),
            Op::UShrInt { dst, a, value } => self.bitwise_int(i, dst, a, value, Bit::UShr),

            Op::Lt { dst, a, b } => self.compare_value(i, dst, Src::Reg(a), Src::Reg(b), Rel::Lt),
            Op::Le { dst, a, b } => self.compare_value(i, dst, Src::Reg(a), Src::Reg(b), Rel::Le),
            Op::Gt { dst, a, b } => self.compare_value(i, dst, Src::Reg(a), Src::Reg(b), Rel::Gt),
            Op::Ge { dst, a, b } => self.compare_value(i, dst, Src::Reg(a), Src::Reg(b), Rel::Ge),
            Op::StrictEq { dst, a, b } => self.strict_value(dst, a, b, false),
            Op::StrictNe { dst, a, b } => self.strict_value(dst, a, b, true),
            Op::Eq { dst, a, b } => self.loose_value(i, dst, a, b, false),
            Op::Ne { dst, a, b } => self.loose_value(i, dst, a, b, true),

            Op::Not { dst, src } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.a.load(RAX, self.reg(src));
                self.a.mov_imm64(RCX, FALSE);
                self.a.mov_rr(R10, RAX);
                self.a.alu_rr(Alu::Xor, R10, RCX);
                self.a.alu_ri(Alu::Cmp, R10, 1);
                self.a.jcc(A, slow);
                self.a.alu_ri(Alu::Xor, RAX, 1);
                self.a.store(self.reg(dst), RAX);
                self.a.bind(done);
                // Anything but a boolean by the helper that only says whether it's truthy.
                self.a.set_cold(true);
                self.a.bind(slow);
                self.a.load(RDX, self.reg(src));
                self.a.mov_rr(RCX, VM);
                self.call_helper(helpers::truthy as *const () as usize);
                self.a.mov_imm64(RCX, FALSE);
                self.a.alu_rr(Alu::Xor, RAX, RCX);
                self.a.alu_ri(Alu::Xor, RAX, 1);
                self.a.store(self.reg(dst), RAX);
                self.a.jmp(done);
                self.a.set_cold(false);
            }
            Op::Inc { dst, src } | Op::Dec { dst, src } => {
                // An int here; a double, or the int that goes past the end, as `step_slow` makes it
                // (`Value::number`); anything else the generic way.
                let not_int = self.a.label();
                let doubles = self.a.label();
                let slow = self.a.label();
                let done = self.a.label();
                let inc = matches!(op, Op::Inc { .. });
                self.operand_reg(RAX, src);
                self.guard_int(RAX, not_int);
                self.a.alu_ri32(Alu::Cmp, RAX, if inc { i32::MAX } else { i32::MIN });
                self.a.jcc(E, not_int);
                self.a.alu_ri32(Alu::Add, RAX, if inc { 1 } else { -1 });
                self.a.alu_rr(Alu::Or, RAX, INT);
                self.a.store(self.reg(dst), RAX);
                self.a.bind(done);
                self.a.set_cold(true);
                self.a.bind(not_int);
                self.note(DOUBLE);
                self.to_double(RAX, XMM0, slow);
                self.a.bind(doubles);
                self.a.mov_imm64(RCX, if inc { 1f64 } else { -1f64 }.to_bits());
                self.a.movq_xr(XMM1, RCX);
                self.a.sd(Sd::Add, XMM0, XMM1);
                self.store_number(dst, XMM0);
                self.a.jmp(done);
                self.a.set_cold(false);
                self.cold_generic_leaving(i, slow, done, dst, RAX);
                self.leaves(dst, RAX);
            }
            Op::Neg { dst, src } => {
                // An int here; a double (and 0 and the most negative int, whose negations are), as `negate_slow`
                // makes it (`Value::number`); anything else the generic way.
                let not_int = self.a.label();
                let doubles = self.a.label();
                let negate = self.a.label();
                let slow = self.a.label();
                let done = self.a.label();
                self.operand_reg(RAX, src);
                self.guard_int(RAX, not_int);
                self.a.test_rr32(RAX, RAX);
                self.a.jcc(E, doubles);
                self.a.alu_ri32(Alu::Cmp, RAX, i32::MIN);
                self.a.jcc(E, doubles);
                self.a.neg32(RAX);
                self.a.alu_rr(Alu::Or, RAX, INT);
                self.a.store(self.reg(dst), RAX);
                self.a.bind(done);
                self.a.set_cold(true);
                self.a.bind(not_int);
                self.a.alu_rr(Alu::Cmp, RAX, INT);
                self.a.jcc(AE, slow);
                self.note(DOUBLE);
                self.a.jmp(negate);
                self.a.bind(doubles);
                self.note(DOUBLE);
                self.a.cvtsi2sd(XMM0, RAX);
                self.a.movq_rx(RAX, XMM0);
                self.a.bind(negate);
                self.a.mov_imm64(R10, 1 << 63);
                self.a.alu_rr(Alu::Xor, RAX, R10);
                self.a.movq_xr(XMM0, RAX);
                self.store_number(dst, XMM0);
                self.a.jmp(done);
                self.a.set_cold(false);
                self.cold_generic_leaving(i, slow, done, dst, RAX);
                self.leaves(dst, RAX);
            }
            Op::SloppyThis => {
                // An object is its own `this`; undefined, null and the rest the generic way.
                let slow = self.a.label();
                let done = self.a.label();
                self.a.load(RAX, self.reg(0));
                self.a.shift_ri(Shift::Shr, RAX, 48);
                self.a.alu_ri32(Alu::Cmp, RAX, TAG_OBJECT);
                self.a.jcc(NE, slow);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::CheckThis { reg } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.a.load(RAX, self.reg(reg));
                self.a.mov_imm64(RCX, EMPTY);
                self.a.alu_rr(Alu::Cmp, RAX, RCX);
                self.a.jcc(E, slow);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::CheckObjectCoercible { src } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.a.load(RAX, self.reg(src));
                self.a.alu_ri(Alu::Or, RAX, 1);
                self.a.mov_imm64(RCX, NULL);
                self.a.alu_rr(Alu::Cmp, RAX, RCX);
                self.a.jcc(E, slow);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::IterClose { iter } | Op::IterCloseQuietly { iter } => {
                // Nothing to do once it's finished with, or for an array, which goes by a count.
                let slow = self.a.label();
                let done = self.a.label();
                self.a.load(RAX, self.reg(iter));
                self.a.mov_imm64(RCX, EMPTY);
                self.a.alu_rr(Alu::Cmp, RAX, RCX);
                self.a.jcc(E, done);
                self.a.load(RAX, self.reg(iter + 1));
                self.a.shift_ri(Shift::Shr, RAX, 32);
                self.a.alu_ri32(Alu::Cmp, RAX, (INT_TAG >> 32) as u32 as i32);
                self.a.jcc(NE, slow);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::ToNumber { dst, src } | Op::ToNumeric { dst, src } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.a.load(RAX, self.reg(src));
                self.a.mov_imm64(RCX, OBJECT_TAG);
                self.a.alu_rr(Alu::Cmp, RAX, RCX);
                self.a.jcc(AE, slow);
                self.a.store(self.reg(dst), RAX);
                self.a.bind(done);
                self.cold_direct(i, slow, done, helpers::to_number);
            }

            Op::Jump { offset } => {
                let to = self.target(i, offset);
                self.a.jmp(to);
            }
            Op::Loop { offset } => {
                self.count();
                self.safe_point(self.pc(i) + 1);
                let to = self.target(i, offset);
                self.a.jmp(to);
            }
            Op::JumpIfTrue { condition, offset } | Op::JumpIfFalse { condition, offset } => {
                let to = self.target(i, offset);
                let slow = self.a.label();
                let next = self.a.label();
                let on_true = matches!(op, Op::JumpIfTrue { .. });
                self.operand_reg(RAX, condition);
                self.a.mov_imm64(RCX, TRUE);
                self.a.alu_rr(Alu::Cmp, RAX, RCX);
                self.a.jcc(E, if on_true { to } else { next });
                self.a.alu_ri(Alu::Sub, RCX, 1);
                self.a.alu_rr(Alu::Cmp, RAX, RCX);
                self.a.jcc(E, if on_true { next } else { to });
                // What else is quick to tell: an int is true unless it's 0, an object is, null and undefined
                // aren't. Doubles, strings and the rest the generic way.
                let (truthy, falsy) = if on_true { (to, next) } else { (next, to) };
                let not_int = self.a.label();
                self.a.mov_rr(R10, RAX);
                self.a.shift_ri(Shift::Shr, R10, 48);
                self.a.alu_ri32(Alu::Cmp, R10, TAG_INT);
                self.a.jcc(NE, not_int);
                self.a.test_rr32(RAX, RAX);
                self.a.jcc(NE, truthy);
                self.a.jmp(falsy);
                self.a.bind(not_int);
                self.a.alu_ri32(Alu::Cmp, R10, TAG_OBJECT);
                self.a.jcc(E, truthy);
                self.a.alu_ri(Alu::Or, RAX, 1);
                self.a.mov_imm64(RCX, NULL);
                self.a.alu_rr(Alu::Cmp, RAX, RCX);
                self.a.jcc(E, falsy);
                self.a.jmp(slow);
                self.a.set_cold(true);
                self.a.bind(slow);
                self.a.load(RDX, self.reg(condition));
                self.a.mov_rr(RCX, VM);
                self.call_helper(helpers::truthy as *const () as usize);
                self.a.test_rr32(RAX, RAX);
                self.a.jcc(NE, truthy);
                self.a.jmp(falsy);
                self.a.set_cold(false);
                self.a.bind(next);
            }
            Op::JumpIfNullish { condition, offset } | Op::JumpIfNotNullish { condition, offset } => {
                let to = self.target(i, offset);
                self.a.load(RAX, self.reg(condition));
                self.a.alu_ri(Alu::Or, RAX, 1);
                self.a.mov_imm64(RCX, NULL);
                self.a.alu_rr(Alu::Cmp, RAX, RCX);
                self.a.jcc(if matches!(op, Op::JumpIfNullish { .. }) { E } else { NE }, to);
            }
            Op::JumpIfUndefined { condition, offset }
            | Op::JumpIfNotUndefined { condition, offset } => {
                let to = self.target(i, offset);
                self.a.load(RAX, self.reg(condition));
                self.a.alu_rr(Alu::Cmp, RAX, UNDEF);
                self.a.jcc(if matches!(op, Op::JumpIfUndefined { .. }) { E } else { NE }, to);
            }
            Op::JumpIfNull { condition, offset } | Op::JumpIfNotNull { condition, offset } => {
                let to = self.target(i, offset);
                self.a.load(RAX, self.reg(condition));
                self.a.mov_imm64(RCX, NULL);
                self.a.alu_rr(Alu::Cmp, RAX, RCX);
                self.a.jcc(if matches!(op, Op::JumpIfNull { .. }) { E } else { NE }, to);
            }
            Op::JumpLt { a, b, offset } => self.jump_compare(i, Src::Reg(a), Src::Reg(b), Rel::Lt, false, offset, false),
            Op::JumpLe { a, b, offset } => self.jump_compare(i, Src::Reg(a), Src::Reg(b), Rel::Le, false, offset, false),
            Op::JumpGt { a, b, offset } => self.jump_compare(i, Src::Reg(a), Src::Reg(b), Rel::Gt, false, offset, false),
            Op::JumpGe { a, b, offset } => self.jump_compare(i, Src::Reg(a), Src::Reg(b), Rel::Ge, false, offset, false),
            Op::JumpNotLt { a, b, offset } => self.jump_compare(i, Src::Reg(a), Src::Reg(b), Rel::Lt, true, offset, false),
            Op::JumpNotLe { a, b, offset } => self.jump_compare(i, Src::Reg(a), Src::Reg(b), Rel::Le, true, offset, false),
            Op::JumpNotGt { a, b, offset } => self.jump_compare(i, Src::Reg(a), Src::Reg(b), Rel::Gt, true, offset, false),
            Op::JumpNotGe { a, b, offset } => self.jump_compare(i, Src::Reg(a), Src::Reg(b), Rel::Ge, true, offset, false),
            Op::JumpLtInt { a, value, offset } => self.jump_compare(i, Src::Reg(a), int(value), Rel::Lt, false, offset, false),
            Op::JumpLeInt { a, value, offset } => self.jump_compare(i, Src::Reg(a), int(value), Rel::Le, false, offset, false),
            Op::JumpGtInt { a, value, offset } => self.jump_compare(i, Src::Reg(a), int(value), Rel::Gt, false, offset, false),
            Op::JumpGeInt { a, value, offset } => self.jump_compare(i, Src::Reg(a), int(value), Rel::Ge, false, offset, false),
            Op::JumpNotLtInt { a, value, offset } => self.jump_compare(i, Src::Reg(a), int(value), Rel::Lt, true, offset, false),
            Op::JumpNotLeInt { a, value, offset } => self.jump_compare(i, Src::Reg(a), int(value), Rel::Le, true, offset, false),
            Op::JumpNotGtInt { a, value, offset } => self.jump_compare(i, Src::Reg(a), int(value), Rel::Gt, true, offset, false),
            Op::JumpNotGeInt { a, value, offset } => self.jump_compare(i, Src::Reg(a), int(value), Rel::Ge, true, offset, false),
            Op::LoopLt { a, b, offset } => self.jump_compare(i, Src::Reg(a), Src::Reg(b), Rel::Lt, false, offset, true),
            Op::LoopLe { a, b, offset } => self.jump_compare(i, Src::Reg(a), Src::Reg(b), Rel::Le, false, offset, true),
            Op::LoopGt { a, b, offset } => self.jump_compare(i, Src::Reg(a), Src::Reg(b), Rel::Gt, false, offset, true),
            Op::LoopGe { a, b, offset } => self.jump_compare(i, Src::Reg(a), Src::Reg(b), Rel::Ge, false, offset, true),
            Op::LoopLtInt { a, value, offset } => self.jump_compare(i, Src::Reg(a), int(value), Rel::Lt, false, offset, true),
            Op::LoopLeInt { a, value, offset } => self.jump_compare(i, Src::Reg(a), int(value), Rel::Le, false, offset, true),
            Op::LoopGtInt { a, value, offset } => self.jump_compare(i, Src::Reg(a), int(value), Rel::Gt, false, offset, true),
            Op::LoopGeInt { a, value, offset } => self.jump_compare(i, Src::Reg(a), int(value), Rel::Ge, false, offset, true),
            Op::JumpStrictEq { a, b, offset } => self.jump_strict(i, a, b, false, offset),
            Op::JumpStrictNe { a, b, offset } => self.jump_strict(i, a, b, true, offset),
            Op::JumpStrictEqInt { a, value, offset } => self.jump_strict_int(i, a, value, false, offset, false),
            Op::JumpStrictNeInt { a, value, offset } => self.jump_strict_int(i, a, value, true, offset, false),
            Op::LoopStrictNe { a, b, offset } => self.loop_strict(i, a, b, offset),
            Op::LoopStrictNeInt { a, value, offset } => self.jump_strict_int(i, a, value, true, offset, true),
            Op::JumpEq { a, b, offset } => self.jump_loose(i, a, b, false, offset),
            Op::JumpNe { a, b, offset } => self.jump_loose(i, a, b, true, offset),
            Op::IncLoopLt { i: counter, limit, offset } => {
                self.inc_loop(i, counter, Src::Reg(limit), offset)
            }
            Op::IncLoopLtInt { i: counter, limit, offset } => {
                self.inc_loop(i, counter, int(limit), offset)
            }
            Op::IncLoopLtLength { i: counter, object, offset } => {
                self.inc_loop_length(i, counter, object, offset)
            }
            Op::IterNext {
                value,
                iter,
                offset,
            } => self.iter_next(i, value, iter, offset),
            Op::JumpTypeIs { a, kind, offset } => self.jump_type(i, a, kind, false, offset),
            Op::JumpTypeIsNot { a, kind, offset } => self.jump_type(i, a, kind, true, offset),
            Op::IterResult { offset, .. } => {
                let to = self.target(i, i32::from(offset));
                let next = self.a.label();
                self.generic_jump(i, to, next);
                self.a.bind(next);
            }

            Op::GetGlobal { dst, cache } => {
                let slow = self.a.label();
                let done = self.a.label();
                // The global object's shape and slot as the place knew them when this was compiled, in the code (see
                // `fixed_read`), then the cache's way.
                let data = self.a.label();
                let global = self.bases.objects + 64 * u64::from(self.job.global);
                if let Some((shape, None, slot)) = self.fixed_read(cache) {
                    self.a.mov_imm64(RAX, global);
                    self.a.alu_mi32(Alu::Cmp, m(RAX, self.l.obj.shape as i32), shape as i32);
                    self.a.jcc(NE, data);
                    self.read_slot_at(RAX, slot, RDX);
                    self.a.store(self.reg(dst), RDX);
                    self.a.jmp(done);
                }
                self.a.bind(data);
                self.a.load32(RAX, m(VM, self.l.global));
                self.a.shift_ri(Shift::Shl, RAX, 6);
                self.a.alu_rr(Alu::Add, RAX, OBJECTS);
                self.a.load32(RCX, m(RAX, self.l.obj.shape as i32));
                self.a.alu_rm32(Alu::Cmp, RCX, self.cache(cache, self.l.c_shape));
                self.a.jcc(NE, slow);
                self.a.load32(RCX, self.cache(cache, self.l.c_slot));
                self.read_slot(RAX, RCX, RDX);
                self.a.store(self.reg(dst), RDX);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::GetCell { dst, cell } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.a.load32(RAX, self.reg(cell));
                self.a.load(RAX, mi(CELLS, RAX, 3, 0));
                self.a.mov_imm64(RCX, EMPTY);
                self.a.alu_rr(Alu::Cmp, RAX, RCX);
                self.a.jcc(E, slow);
                self.a.store(self.reg(dst), RAX);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::SetCell { cell, src } | Op::InitCell { cell, src } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.a.load32(RAX, self.reg(cell));
                if matches!(op, Op::SetCell { .. }) {
                    self.a.load(RCX, mi(CELLS, RAX, 3, 0));
                    self.a.mov_imm64(RDX, EMPTY);
                    self.a.alu_rr(Alu::Cmp, RCX, RDX);
                    self.a.jcc(E, slow);
                }
                self.a.load(RCX, self.reg(src));
                self.a.store(mi(CELLS, RAX, 3, 0), RCX);
                self.cell_barrier(done);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::GetUp { dst, index } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.upvalues(RAX);
                self.a.load32(RAX, m(RAX, 8 * i32::from(index)));
                self.a.load(RAX, mi(CELLS, RAX, 3, 0));
                self.a.mov_imm64(RCX, EMPTY);
                self.a.alu_rr(Alu::Cmp, RAX, RCX);
                self.a.jcc(E, slow);
                self.a.store(self.reg(dst), RAX);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::GetUpValue { dst, index } => {
                self.upvalues(RAX);
                self.a.load(RAX, m(RAX, 8 * i32::from(index)));
                self.a.store(self.reg(dst), RAX);
            }
            Op::SetUp { index, src } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.upvalues(RAX);
                self.a.load32(RAX, m(RAX, 8 * i32::from(index)));
                self.a.load(RCX, mi(CELLS, RAX, 3, 0));
                self.a.mov_imm64(RDX, EMPTY);
                self.a.alu_rr(Alu::Cmp, RCX, RDX);
                self.a.jcc(E, slow);
                self.a.load(RCX, self.reg(src));
                self.a.store(mi(CELLS, RAX, 3, 0), RCX);
                self.cell_barrier(done);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::LoadCallee { dst } if self.inline.is_some() => {
                self.a.load(RAX, m(REGS, 8 * (self.shift - 1)));
                self.a.store(self.reg(dst), RAX);
            }
            Op::LoadCallee { dst } => {
                self.a.load32(RAX, m(FRAME, self.l.f_closure));
                self.a.mov_imm64(RCX, OBJECT_TAG);
                self.a.alu_rr(Alu::Or, RAX, RCX);
                self.a.store(self.reg(dst), RAX);
            }

            Op::GetProp { dst, object, cache } => {
                let slow = self.a.label();
                let read = self.a.label();
                let done = self.a.label();
                self.operand_reg(RAX, object);
                self.get_prop(cache, object, slow, RDX);
                self.a.bind(read);
                self.a.store(self.reg(dst), RDX);
                self.a.bind(done);
                self.cold_generic_leaving(i, slow, done, dst, RDX);
                self.leaves(dst, RDX);
            }
            Op::SetProp {
                object,
                value,
                cache,
            } => {
                let slow = self.a.label();
                let done = self.a.label();
                let others = self.a.label();
                let data = self.a.label();
                let l = self.l;
                let fixed = self.fixed_write(cache);
                self.a.load(RAX, self.reg(object));
                self.object_address(RAX, slow);
                // What the place knew when this was compiled, in the code (see `fixed_write`), with the cache's way
                // out of the way.
                let was = self.a.is_cold();
                if let Some((shape, slot)) = fixed {
                    self.a.alu_mi32(Alu::Cmp, m(RAX, l.obj.shape as i32), shape as i32);
                    self.a.jcc(NE, data);
                    self.a.test_m8(m(RAX, l.obj.flags as i32), crate::gc::OLD);
                    self.a.jcc(NE, slow);
                    self.slot_place_at(RAX, slot);
                    self.a.load(RDX, self.reg(value));
                    self.a.store(m(R10, 0), RDX);
                    self.a.jmp(done);
                    self.a.set_cold(true);
                }
                self.a.bind(data);
                self.a.load32(RCX, m(RAX, l.obj.shape as i32));
                self.a.alu_rm32(Alu::Cmp, RCX, self.cache(cache, l.c_shape));
                self.a.jcc(NE, others);
                self.a.test_m8(m(RAX, l.obj.flags as i32), crate::gc::OLD);
                self.a.jcc(NE, slow);
                let adds = self.a.label();
                self.a.alu_mi32(Alu::Cmp, self.cache(cache, l.c_other), -1);
                // The cache says what shape adding it makes, which a constructor's `this.x = x` does each time: in
                // line if that's what the place has done.
                if fixed.is_none() && self.adds(cache) {
                    let writes = self.a.label();
                    self.a.jcc(E, writes);
                    self.add_slot(cache, None, object, value, slow, done);
                    self.a.set_cold(true);
                    self.a.bind(writes);
                    self.a.load32(RCX, self.cache(cache, l.c_slot));
                    self.slot_place(RAX, RCX);
                    self.a.load(RDX, self.reg(value));
                    self.a.store(m(R10, 0), RDX);
                    self.a.jmp(done);
                } else {
                    self.a.jcc(NE, adds);
                    self.a.load32(RCX, self.cache(cache, l.c_slot));
                    self.slot_place(RAX, RCX);
                    self.a.load(RDX, self.reg(value));
                    self.a.store(m(R10, 0), RDX);
                    self.a.jmp(done);
                    self.a.set_cold(true);
                    self.a.bind(adds);
                    self.add_slot(cache, None, object, value, slow, done);
                }
                // Another shape than the first entry's: the place may have become polymorphic since this was compiled.
                self.a.bind(others);
                let adds_other = self.a.label();
                self.poly_others(cache, slow, slow);
                self.a.test_m8(m(RAX, l.obj.flags as i32), crate::gc::OLD);
                self.a.jcc(NE, slow);
                self.a.alu_mi32(Alu::Cmp, m(R9, l.c_other), -1);
                self.a.jcc(NE, adds_other);
                self.a.load32(R8, m(R9, l.c_slot));
                self.slot_place(RAX, R8);
                self.a.load(RDX, self.reg(value));
                self.a.store(m(R10, 0), RDX);
                self.a.jmp(done);
                self.a.bind(adds_other);
                self.add_slot(cache, Some(R9), object, value, slow, done);
                self.a.set_cold(was);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::GetLength { dst, object, .. } => {
                let slow = self.a.label();
                let done = self.a.label();
                let not_object = self.a.label();
                let not_array = self.a.label();
                let length = self.a.label();
                let o = self.l.obj;
                self.a.load(RAX, self.reg(object));
                self.object_address(RAX, not_object);
                self.a.cmp_m8(m(RAX, o.kind as i32), o.array_tag);
                self.a.jcc(NE, not_array);
                self.a.bind(length);
                self.a.load32(RAX, m(RAX, (o.kind + o.array_length) as i32));
                self.a.test_rr32(RAX, RAX);
                self.a.jcc(S, slow);
                self.a.alu_rr(Alu::Or, RAX, INT);
                self.a.store(self.reg(dst), RAX);
                self.a.bind(done);
                self.a.set_cold(true);
                // `arguments.length` is how many there are, as for an array.
                self.a.bind(not_array);
                self.a.cmp_m8(m(RAX, o.kind as i32), o.arguments_tag);
                self.a.jcc(E, length);
                self.a.jmp(slow);
                // A string's, by a helper that does only that. (r10 has the value's tag.)
                self.a.bind(not_object);
                self.a.alu_ri32(Alu::Cmp, R10, TAG_STRING);
                self.a.jcc(NE, slow);
                self.a.mov_rr(RCX, VM);
                self.a.mov_rr(RDX, RAX);
                self.call_helper(helpers::string_length as *const () as usize);
                self.a.store(self.reg(dst), RAX);
                self.a.jmp(done);
                // Anything else (an object's `length` property) by a helper that does only that.
                self.a.bind(slow);
                self.a.mov_rr(RCX, VM);
                self.a.mov_imm64(RDX, u64::from(self.pc(i)));
                self.a.load(R8, self.reg(object));
                self.call_helper(helpers::get_length as *const () as usize);
                self.a.test_rr32(RAX, RAX);
                self.a.jcc(NE, self.exit);
                self.a.jmp(done);
                self.a.set_cold(false);
            }
            Op::GetElem { dst, object, key } => {
                let slow = self.a.label();
                let done = self.a.label();
                let not_array = self.a.label();
                let elements = self.a.label();
                let o = self.l.obj;
                self.operand_reg(RAX, object);
                self.operand_reg(RCX, key);
                self.object_address(RAX, slow);
                self.guard_int(RCX, slow);
                self.a.mov_rr32(RCX, RCX);
                self.a.cmp_m8(m(RAX, o.kind as i32), o.array_tag);
                self.a.jcc(NE, not_array);
                self.a.bind(elements);
                self.a.alu_rm32(Alu::Cmp, RCX, m(RAX, (o.kind + o.array_length) as i32));
                self.a.jcc(AE, slow);
                self.a.load(RAX, m(RAX, (o.kind + o.array_start) as i32));
                self.a.load(RAX, mi(RAX, RCX, 3, 0));
                self.a.mov_imm64(RCX, EMPTY);
                self.a.alu_rr(Alu::Cmp, RAX, RCX);
                self.a.jcc(E, slow);
                self.a.store(self.reg(dst), RAX);
                self.a.bind(done);
                self.a.set_cold(true);
                // `arguments` keeps its elements where an array does; a typed array's are where its buffer's bytes
                // are.
                self.a.bind(not_array);
                self.a.cmp_m8(m(RAX, o.kind as i32), o.arguments_tag);
                self.a.jcc(E, elements);
                self.typed_read(dst, slow, done);
                // Anything else (an element that isn't there, a key that's a name) by a helper that does only that.
                self.a.bind(slow);
                self.a.mov_rr(RCX, VM);
                self.a.mov_imm64(RDX, u64::from(self.pc(i)) | u64::from(dst) << 32);
                self.a.load(R8, self.reg(object));
                self.a.load(R9, self.reg(key));
                self.call_helper(helpers::get_elem as *const () as usize);
                self.a.test_rr32(RAX, RAX);
                self.a.jcc(NE, self.exit);
                self.a.load(RAX, self.reg(dst));
                self.a.jmp(done);
                self.a.set_cold(false);
                self.leaves(dst, RAX);
            }
            Op::SetElem { object, key, value } | Op::SetElemTyped { object, key, value } => {
                // An array's element or a typed array's, whichever the place last wrote (SetElemTyped) first.
                let typed_first = matches!(op, Op::SetElemTyped { .. });
                let slow = self.a.label();
                let done = self.a.label();
                let array = self.a.label();
                let typed = self.a.label();
                let o = self.l.obj;
                self.a.load(RAX, self.reg(object));
                self.object_address(RAX, slow);
                self.a.load(RCX, self.reg(key));
                self.guard_int(RCX, slow);
                self.a.mov_rr32(RCX, RCX);
                if typed_first {
                    self.a.cmp_m8(m(RAX, o.kind as i32), o.typed_tag);
                    self.a.jcc(NE, array);
                    self.typed_write(value, slow, done);
                    self.a.bind(done);
                    self.a.set_cold(true);
                    self.a.bind(array);
                    self.a.cmp_m8(m(RAX, o.kind as i32), o.array_tag);
                    self.a.jcc(NE, slow);
                    self.array_write(value, slow, done);
                } else {
                    self.a.cmp_m8(m(RAX, o.kind as i32), o.array_tag);
                    self.a.jcc(NE, typed);
                    self.array_write(value, slow, done);
                    self.a.bind(done);
                    self.a.set_cold(true);
                    self.a.bind(typed);
                    self.a.cmp_m8(m(RAX, o.kind as i32), o.typed_tag);
                    self.a.jcc(NE, slow);
                    self.typed_write(value, slow, done);
                }
                // A typed array's element whose quick place isn't known, by a helper that does only that, before
                // the generic way.
                let generic = self.a.label();
                self.a.bind(slow);
                self.a.mov_rr(RCX, VM);
                self.a.load(RDX, self.reg(object));
                self.a.load(R8, self.reg(key));
                self.a.load(R9, self.reg(value));
                self.set_site();
                self.call_helper(helpers::typed_set as *const () as usize);
                self.a.test_rr32(RAX, RAX);
                self.a.jcc(NE, generic);
                self.a.jmp(done);
                self.a.set_cold(false);
                self.cold_direct(i, generic, done, helpers::set_elem);
            }

            Op::Call { dst, f, argc } => self.call(i, dst, f, argc, Method::None),
            Op::CallMath { dst, f, which } => self.call_math(i, dst, f, which),
            Op::CallPlain { dst, f, argc } => {
                self.a.store(self.reg(f + 1), UNDEF);
                self.call(i, dst, f, argc, Method::None)
            }
            Op::CallMethod { dst, f, cache } => self.call(i, dst, f, 0, Method::Mono(cache)),
            Op::CallMethodPoly { dst, f, cache } => self.call(i, dst, f, 0, Method::Poly(cache)),
            Op::GetPropPoly { dst, object, cache } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.a.load(RAX, self.reg(object));
                self.get_prop_poly(cache, slow, done, dst);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::SetPropPoly {
                object,
                value,
                cache,
            } => {
                let slow = self.a.label();
                let done = self.a.label();
                let l = self.l;
                let adds = self.a.label();
                let mega = self.a.label();
                self.a.load(RAX, self.reg(object));
                // The shapes whose slot the place knew when this was compiled, in the code, then the cache's way.
                let fixed: Vec<(u32, u32)> = self
                    .fixed_poly(cache)
                    .into_iter()
                    .filter(|&(_, holder, _)| holder.is_none())
                    .map(|(shape, _, slot)| (shape, slot))
                    .collect();
                if !fixed.is_empty() {
                    let o = l.obj;
                    self.a.mov_rr(R11, RAX);
                    self.object_address(RAX, slow);
                    self.a.load32(RCX, m(RAX, o.shape as i32));
                    for (shape, slot) in fixed {
                        let next = self.a.label();
                        self.a.alu_ri32(Alu::Cmp, RCX, shape as i32);
                        self.a.jcc(NE, next);
                        self.a.test_m8(m(RAX, o.flags as i32), crate::gc::OLD);
                        self.a.jcc(NE, slow);
                        self.slot_place_at(RAX, slot);
                        self.a.load(RDX, self.reg(value));
                        self.a.store(m(R10, 0), RDX);
                        self.a.jmp(done);
                        self.a.bind(next);
                    }
                    self.a.mov_rr(RAX, R11);
                }
                self.poly_find(cache, slow, mega);
                self.a.test_m8(m(RAX, l.obj.flags as i32), crate::gc::OLD);
                self.a.jcc(NE, slow);
                self.a.alu_mi32(Alu::Cmp, m(R9, l.c_other), -1);
                self.a.jcc(NE, adds);
                self.a.load32(R8, m(R9, l.c_slot));
                self.slot_place(RAX, R8);
                self.a.load(RDX, self.reg(value));
                self.a.store(m(R10, 0), RDX);
                self.a.bind(done);
                self.a.set_cold(true);
                self.a.bind(adds);
                self.add_slot(cache, Some(R9), object, value, slow, done);
                // Too many shapes for the place: where writes to this one went before, by a helper that does only
                // that. (rax has the object's address.)
                self.a.bind(mega);
                self.a.test_m8(m(RAX, l.obj.flags as i32), crate::gc::OLD);
                self.a.jcc(NE, slow);
                self.a.mov_rr(RCX, VM);
                self.a.load(RDX, self.reg(object));
                self.a.load(R8, self.reg(value));
                self.a.mov_imm64(R9, u64::from(self.proto.cache_base) + u64::from(cache));
                self.call_helper(helpers::mega_set as *const () as usize);
                self.a.test_rr32(RAX, RAX);
                self.a.jcc(NE, slow);
                self.a.jmp(done);
                self.a.set_cold(false);
                self.cold_generic(i, slow, done);
            }
            Op::New { dst, f, argc } => self.construct(i, dst, f, argc, false),
            // In an arrow, `super` is found through what it inherited, the long way.
            Op::SuperCall { dst, f, argc } if self.proto.flags & crate::op::IS_ARROW == 0 => {
                self.construct(i, dst, f, argc, true)
            }
            Op::InstanceOf { dst, a, b } => self.instance_of(i, dst, a, b),
            Op::CallSpread { .. }
            | Op::NewSpread { .. }
            | Op::SuperCall { .. }
            | Op::SuperCallSpread { .. } => {
                self.a.mov_rr(RCX, VM);
                self.a.mov_imm64(RDX, u64::from(self.pc(i)));
                self.call_helper(helpers::call as *const () as usize);
                self.a.test_rr32(RAX, RAX);
                self.a.jcc(NE, self.exit);
            }
            Op::Return { src } if self.inline.is_some() => {
                self.operand_reg(RAX, src);
                self.inline_return(RAX);
            }
            Op::ReturnUndefined if self.inline.is_some() => self.inline_return(UNDEF),
            Op::Return { src } => {
                self.operand_reg(RAX, src);
                self.ret(i);
            }
            Op::ReturnUndefined => {
                self.a.mov_rr(RAX, UNDEF);
                self.ret(i);
            }

            Op::NewObject { dst, capacity } => {
                self.a.mov_rr(RCX, VM);
                self.a.mov_imm64(RDX, u64::from(capacity));
                self.call_helper(helpers::new_object as *const () as usize);
                self.a.store(self.reg(dst), RAX);
            }
            Op::NewArray { dst, capacity } => {
                self.a.mov_rr(RCX, VM);
                self.a.mov_imm64(RDX, u64::from(capacity));
                self.call_helper(helpers::new_array as *const () as usize);
                self.a.store(self.reg(dst), RAX);
            }
            Op::ArrayPush { array, value } => {
                self.a.mov_rr(RCX, VM);
                self.a.load(RDX, self.reg(array));
                self.a.load(R8, self.reg(value));
                self.call_helper(helpers::array_push as *const () as usize);
            }
            Op::Closure { dst, proto } => {
                self.a.mov_rr(RCX, VM);
                self.a.mov_imm64(RDX, u64::from(proto));
                self.call_helper(helpers::closure as *const () as usize);
                self.a.store(self.reg(dst), RAX);
            }
            Op::NewCell { dst, src } => {
                self.a.mov_rr(RCX, VM);
                self.a.load(RDX, self.reg(src));
                self.call_helper(helpers::new_cell as *const () as usize);
                self.a.store(self.reg(dst), RAX);
            }
            Op::CopyCell { reg } => {
                self.a.load32(RAX, self.reg(reg));
                self.a.load(RDX, mi(CELLS, RAX, 3, 0));
                self.a.mov_rr(RCX, VM);
                self.call_helper(helpers::new_cell as *const () as usize);
                self.a.store(self.reg(reg), RAX);
            }
            Op::TypeOf { dst, src } => {
                self.a.mov_rr(RCX, VM);
                self.a.load(RDX, self.reg(src));
                self.call_helper(helpers::type_of as *const () as usize);
                self.a.store(self.reg(dst), RAX);
            }
            Op::DefineField {
                object,
                value,
                cache,
            } => {
                // An object literal's next field, on the shape transition the place knows: inside a plain object
                // while there's room, as `push_slot` puts it, else by a helper.
                let slow = self.a.label();
                let done = self.a.label();
                let inside = self.a.label();
                let call = self.a.label();
                let l = self.l;
                let o = l.obj;
                self.a.load(RAX, self.reg(object));
                self.a.mov_rr(R11, RAX);
                self.object_address(RAX, slow);
                self.a.load32(RCX, m(RAX, o.shape as i32));
                self.a.alu_rm32(Alu::Cmp, RCX, self.cache(cache, l.c_shape));
                self.a.jcc(NE, slow);
                self.a.test_m8(m(RAX, o.flags as i32), crate::gc::OLD);
                self.a.jcc(NE, slow);
                self.a.load8(RCX, m(RAX, o.slot_count as i32));
                self.a.alu_ri32(Alu::Cmp, RCX, i32::from(o.inline_count));
                self.a.jcc(AE, call);
                self.a.test_rr32(RCX, RCX);
                self.a.jcc(NE, inside);
                self.a.cmp_m8(m(RAX, o.kind as i32), o.plain_tag);
                self.a.jcc(NE, call);
                self.a.bind(inside);
                self.a.load(RDX, self.reg(value));
                self.a.store(mi(RAX, RCX, 3, o.inline_slots as i32), RDX);
                self.a.inc_m8(m(RAX, o.slot_count as i32));
                self.a.load32(R8, self.cache(cache, l.c_other));
                self.a.store32(m(RAX, o.shape as i32), R8);
                self.a.bind(done);
                self.a.set_cold(true);
                self.a.bind(call);
                self.a.mov_rr(RCX, VM);
                self.a.mov_rr32(RDX, R11);
                self.a.load32(R8, self.cache(cache, l.c_other));
                self.a.load(R9, self.reg(value));
                self.call_helper(helpers::define_slot as *const () as usize);
                self.a.jmp(done);
                self.a.set_cold(false);
                self.cold_generic(i, slow, done);
            }
            Op::GetIterator { iter, source } => {
                // `for of` over an array, while `Array.prototype[Symbol.iterator]` is the built-in one, goes by a
                // count (as `Vm::get_iterator` decides); anything else the generic way.
                let slow = self.a.label();
                let done = self.a.label();
                let l = self.l;
                self.a.load(RAX, self.reg(source));
                self.a.mov_rr(RDX, RAX);
                self.object_address(RAX, slow);
                self.a.load32(RCX, m(RAX, l.obj.shape as i32));
                self.a.alu_rm32(Alu::Cmp, RCX, m(VM, l.array_shape));
                self.a.jcc(NE, slow);
                self.a.cmp_m8(m(RAX, l.obj.kind as i32), l.obj.array_tag);
                self.a.jcc(NE, slow);
                self.a.load32(RAX, m(VM, l.array_proto));
                self.a.shift_ri(Shift::Shl, RAX, 6);
                self.a.alu_rr(Alu::Add, RAX, OBJECTS);
                self.a.load32(RCX, m(RAX, l.obj.shape as i32));
                self.a.alu_rm32(Alu::Cmp, RCX, m(VM, l.iterator_shape));
                self.a.jcc(NE, slow);
                self.a.load32(RCX, m(VM, l.iterator_slot));
                self.read_slot(RAX, RCX, R8);
                self.a.alu_rm(Alu::Cmp, R8, m(VM, l.array_values));
                self.a.jcc(NE, slow);
                self.a.store(self.reg(iter), RDX);
                self.a.store(self.reg(iter + 1), INT);
                self.a.bind(done);
                self.cold_direct(i, slow, done, helpers::get_iterator);
            }
            Op::ArraySpread { .. } => self.direct(i, helpers::array_spread),
            Op::Rest { .. } => self.direct(i, helpers::rest),
            Op::SuperGet { .. } => self.direct(i, helpers::super_get),
            Op::CreateArguments { .. } => self.direct(i, helpers::create_arguments),
            Op::SetHome { .. } => self.direct(i, helpers::set_home),
            Op::Concat { .. } => {
                self.a.mov_rr(RCX, VM);
                self.a.mov_imm64(RDX, u64::from(self.pc(i)));
                self.call_helper(helpers::concat as *const () as usize);
                self.a.test_rr32(RAX, RAX);
                self.a.jcc(NE, self.exit);
            }

            // Never in a function that's compiled (see `jit::compilable`).
            // A generator stops: the frame has gone, and what it hands back is the generator, or what it yields.
            Op::InitialYield => self.stop_here(i, helpers::initial_yield),
            Op::Yield { .. } => self.stop_here(i, helpers::yield_value),
            Op::Await { .. } => return false,
            // Everything else the generic way.
            _ => self.generic(i),
        }
        true
    }

    fn store_imm(&mut self, dst: u16, value: u64) {
        self.a.mov_imm64(RAX, value);
        self.a.store(self.reg(dst), RAX);
    }

    /// `+`, `-` or `*`, with ints here and doubles out of the way.
    fn arith(&mut self, i: usize, dst: u16, x: Src, y: Src, kind: Arith) {
        let doubles = self.a.label();
        let slow = self.a.label();
        let done = self.a.label();
        let known_double = |src: Src| matches!(src, Src::Imm(value) if !value.is_int());
        self.operand(RAX, x);
        self.operand(RCX, y);
        self.leaves(dst, RDX);
        // One's a double, so the answer is; or one was a double the last time round and the other an int (adding
        // ints up into a double, say), which the double way does too.
        let mixed = matches!((x, y), (Src::Reg(_), Src::Reg(_)))
            && ((self.looks_double(x) && self.looks_int(y)) || (self.looks_int(x) && self.looks_double(y)));
        if known_double(x) || known_double(y) || mixed {
            self.note(DOUBLE);
            self.double_arith(dst, x, y, kind, slow);
            self.a.bind(done);
            self.arith_slow(i, slow, done, x, y, kind, dst);
            return;
        }
        let int_with = |src: Src| matches!(src, Src::Imm(value) if value.is_int());
        let doubles_first = match (x, y) {
            (Src::Reg(_), Src::Reg(_)) => self.looks_double(x) && self.looks_double(y),
            (Src::Reg(_), Src::Imm(_)) => int_with(y) && self.looks_double(x),
            (Src::Imm(_), Src::Reg(_)) => int_with(x) && self.looks_double(y),
            (Src::Imm(_), Src::Imm(_)) => false,
        };
        if doubles_first {
            // Doubles the last time round (see `looks_double`): two doubles here, with one test of what they are
            // (an int known here is made one), and everything else out of the way, ints first as ever.
            let other = self.a.label();
            let mixed = self.a.label();
            match (x, y) {
                (Src::Reg(_), Src::Reg(_)) => {
                    self.a.mov_rr(R10, RAX);
                    self.a.alu_rr(Alu::Cmp, R10, RCX);
                    self.a.cmov(B, R10, RCX);
                    self.a.alu_rr(Alu::Cmp, R10, INT);
                    self.a.jcc(AE, other);
                    self.a.movq_xr(XMM0, RAX);
                    self.a.movq_xr(XMM1, RCX);
                }
                (Src::Reg(_), Src::Imm(_)) => {
                    self.a.alu_rr(Alu::Cmp, RAX, INT);
                    self.a.jcc(AE, other);
                    self.a.movq_xr(XMM0, RAX);
                    self.a.cvtsi2sd(XMM1, RCX);
                }
                _ => {
                    self.a.alu_rr(Alu::Cmp, RCX, INT);
                    self.a.jcc(AE, other);
                    self.a.cvtsi2sd(XMM0, RAX);
                    self.a.movq_xr(XMM1, RCX);
                }
            }
            self.note(DOUBLE);
            let op = match kind {
                Arith::Add => Sd::Add,
                Arith::Sub => Sd::Sub,
                Arith::Mul => Sd::Mul,
            };
            self.a.sd(op, XMM0, XMM1);
            self.a.movq_rx(RDX, XMM0);
            self.a.store(self.reg(dst), RDX);
            self.a.bind(done);
            self.a.set_cold(true);
            self.a.bind(other);
            match (x, y) {
                (Src::Reg(_), Src::Reg(_)) => self.guard_ints(RAX, RCX, mixed),
                (Src::Reg(_), Src::Imm(_)) => self.guard_int(RAX, mixed),
                _ => self.guard_int(RCX, mixed),
            }
            self.a.mov_rr32(RDX, RAX);
            match kind {
                Arith::Add => self.a.alu_rr32(Alu::Add, RDX, RCX),
                Arith::Sub => self.a.alu_rr32(Alu::Sub, RDX, RCX),
                Arith::Mul => self.a.imul_rr32(RDX, RCX),
            }
            self.a.jcc(O, mixed);
            if kind == Arith::Mul {
                self.a.test_rr32(RDX, RDX);
                self.a.jcc(E, mixed);
            }
            self.a.alu_rr(Alu::Or, RDX, INT);
            self.a.store(self.reg(dst), RDX);
            self.a.jmp(done);
            self.a.bind(mixed);
            self.note(DOUBLE);
            self.double_arith(dst, x, y, kind, slow);
            self.a.jmp(done);
            self.a.set_cold(false);
            self.arith_slow(i, slow, done, x, y, kind, dst);
            return;
        }
        match (x, y) {
            (Src::Reg(_), Src::Reg(_)) => self.guard_ints(RAX, RCX, doubles),
            (Src::Reg(_), Src::Imm(_)) => self.guard_int(RAX, doubles),
            (Src::Imm(_), Src::Reg(_)) => self.guard_int(RCX, doubles),
            (Src::Imm(_), Src::Imm(_)) => {}
        }
        self.a.mov_rr32(RDX, RAX);
        match kind {
            Arith::Add => self.a.alu_rr32(Alu::Add, RDX, RCX),
            Arith::Sub => self.a.alu_rr32(Alu::Sub, RDX, RCX),
            Arith::Mul => self.a.imul_rr32(RDX, RCX),
        }
        self.a.jcc(O, doubles);
        if kind == Arith::Mul {
            // Zero is a double here, as it might be negative.
            self.a.test_rr32(RDX, RDX);
            self.a.jcc(E, doubles);
        }
        self.a.alu_rr(Alu::Or, RDX, INT);
        self.a.store(self.reg(dst), RDX);
        let mixed = self.a.label();
        let both_regs = matches!((x, y), (Src::Reg(_), Src::Reg(_)));
        if both_regs {
            // Two doubles, which float code has most, in line after the ints, with one test of what they are
            // (`Value::both_doubles`: both below the int tag), as the interpreter tests them.
            let joined = self.a.label();
            self.a.jmp(joined);
            self.a.bind(doubles);
            self.note(DOUBLE);
            self.a.mov_rr(R10, RAX);
            self.a.alu_rr(Alu::Cmp, R10, RCX);
            self.a.cmov(B, R10, RCX);
            self.a.alu_rr(Alu::Cmp, R10, INT);
            self.a.jcc(AE, mixed);
            self.a.movq_xr(XMM0, RAX);
            self.a.movq_xr(XMM1, RCX);
            let op = match kind {
                Arith::Add => Sd::Add,
                Arith::Sub => Sd::Sub,
                Arith::Mul => Sd::Mul,
            };
            self.a.sd(op, XMM0, XMM1);
            self.a.movq_rx(RDX, XMM0);
            self.a.store(self.reg(dst), RDX);
            self.a.bind(joined);
        }
        self.a.bind(done);
        self.a.set_cold(true);
        if !both_regs {
            self.a.bind(doubles);
            self.note(DOUBLE);
        }
        self.a.bind(mixed);
        self.double_arith(dst, x, y, kind, slow);
        self.a.jmp(done);
        self.a.set_cold(false);
        self.arith_slow(i, slow, done, x, y, kind, dst);
    }

    /// Whether the instruction being compiled has just run in the frame it's compiled for: it's in the loop that
    /// frame is going round (see `looping`). Only that loop's: the registers of a loop that hasn't run yet hold what
    /// some other frame left there, and taking them for what it works on made typed_arrays' int loops 3.5 times as
    /// slow.
    fn ran(&self) -> bool {
        self.inline.is_none() && self.looping.is_some_and(|(top, bottom)| (top..=bottom).contains(&self.at))
    }

    /// Whether an operand was an int the last time round (see `looks_double`).
    fn looks_int(&self, src: Src) -> bool {
        let ran = self.ran();
        match src {
            Src::Reg(register) => ran && self.live(register).is_some_and(|value| value.is_int()),
            Src::Imm(value) => value.is_int(),
        }
    }

    /// Whether write place `cache`'s first entry adds a property (its `other` is the shape that makes), as a
    /// constructor's do: its template then does that in line.
    fn adds(&self, cache: u16) -> bool {
        self.cache_entry(self.proto.cache_base + u32::from(cache))
            .is_some_and(|entry| entry.shape != crate::object::NONE && entry.other != crate::object::NONE)
    }

    /// Whether an operand was a double the last time round, as the frame being compiled for has it (see `live`):
    /// float code's templates then try two doubles first. A guess about the code's layout only: both ways are there.
    fn looks_double(&self, src: Src) -> bool {
        let ran = self.ran();
        match src {
            Src::Reg(register) => ran && self.live(register).is_some_and(|value| value.is_double()),
            Src::Imm(value) => value.is_double(),
        }
    }

    /// What isn't two numbers: a `+` (which is mostly joining strings) by a helper that does only that, the rest the
    /// generic way. Either goes back with the answer in rdx, as `arith` leaves it.
    #[allow(clippy::too_many_arguments)]
    fn arith_slow(&mut self, i: usize, slow: Label, done: Label, x: Src, y: Src, kind: Arith, dst: u16) {
        if kind != Arith::Add {
            self.cold_generic_leaving(i, slow, done, dst, RDX);
            return;
        }
        let was = self.a.is_cold();
        self.a.set_cold(true);
        self.a.bind(slow);
        if let Some(deopt) = self.deopt() {
            self.a.jmp(deopt);
            self.a.set_cold(was);
            return;
        }
        self.load(R8, x);
        self.load(R9, y);
        self.a.mov_rr(RCX, VM);
        self.a.mov_imm64(RDX, u64::from(self.pc(i)));
        self.call_helper(helpers::add as *const () as usize);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NE, self.exit);
        self.a.load(RDX, self.reg(dst));
        self.a.jmp(done);
        self.a.set_cold(was);
    }

    /// The operands are in `rax` and `rcx`; on to `slow` if either isn't a number.
    fn double_arith(&mut self, dst: u16, x: Src, y: Src, kind: Arith, slow: Label) {
        self.src_double(x, RAX, XMM0, slow);
        self.src_double(y, RCX, XMM1, slow);
        let op = match kind {
            Arith::Add => Sd::Add,
            Arith::Sub => Sd::Sub,
            Arith::Mul => Sd::Mul,
        };
        self.a.sd(op, XMM0, XMM1);
        self.a.movq_rx(RDX, XMM0);
        self.a.store(self.reg(dst), RDX);
    }

    fn arith_k(&mut self, i: usize, dst: u16, a: u16, k: u16, kind: Arith) {
        let value = self.constant(k as usize);
        if value.is_number() {
            self.arith(i, dst, Src::Reg(a), Src::Imm(value), kind);
        } else if let Some(deopt) = self.deopt() {
            self.a.jmp(deopt);
        } else if kind == Arith::Add {
            // `x + "..."` joins strings, by the helper that does only `+`.
            self.load(R8, Src::Reg(a));
            self.load(R9, Src::Imm(value));
            self.a.mov_rr(RCX, VM);
            self.a.mov_imm64(RDX, u64::from(self.pc(i)));
            self.call_helper(helpers::add as *const () as usize);
            self.a.test_rr32(RAX, RAX);
            self.a.jcc(NE, self.exit);
        } else {
            self.generic(i);
        }
    }

    fn divide(&mut self, i: usize, dst: u16, x: Src, y: Src) {
        let slow = self.a.label();
        let done = self.a.label();
        self.operand(RAX, x);
        self.operand(RCX, y);
        self.src_double(x, RAX, XMM0, slow);
        self.src_double(y, RCX, XMM1, slow);
        self.a.sd(Sd::Div, XMM0, XMM1);
        self.store_number(dst, XMM0);
        self.a.bind(done);
        self.cold_generic_leaving(i, slow, done, dst, RAX);
        self.leaves(dst, RAX);
    }

    /// Dividing by `value`, a power of two above zero: an int it divides exactly is shifted, a number is divided as
    /// doubles, anything else the generic way.
    fn divide_int(&mut self, i: usize, dst: u16, a: u16, value: i16) {
        let doubles = self.a.label();
        let slow = self.a.label();
        let done = self.a.label();
        let by = i32::from(value);
        self.operand_reg(RAX, a);
        self.guard_int(RAX, doubles);
        self.a.test_ri32(RAX, (by - 1) as u32);
        self.a.jcc(NE, doubles);
        self.a.shift_ri32(Shift::Sar, RAX, by.trailing_zeros() as u8);
        self.a.alu_rr(Alu::Or, RAX, INT);
        self.a.store(self.reg(dst), RAX);
        self.a.bind(done);
        self.a.set_cold(true);
        self.a.bind(doubles);
        self.note(DOUBLE);
        self.to_double(RAX, XMM0, slow);
        self.a.mov_imm64(RCX, f64::from(by).to_bits());
        self.a.movq_xr(XMM1, RCX);
        self.a.sd(Sd::Div, XMM0, XMM1);
        self.store_number(dst, XMM0);
        self.a.jmp(done);
        self.a.set_cold(false);
        self.cold_generic_leaving(i, slow, done, dst, RAX);
        self.leaves(dst, RAX);
    }

    /// `%` on two ints when the left one isn't negative and the right one is above zero; other numbers as doubles, by
    /// `fmod` (as the interpreter's `remainder!`); anything else the generic way.
    fn modulo(&mut self, i: usize, dst: u16, a: u16, y: Src) {
        let doubles = self.a.label();
        let slow = self.a.label();
        let done = self.a.label();
        self.operand_reg(RAX, a);
        match y {
            Src::Reg(b) => {
                self.operand_reg(RCX, b);
                self.guard_ints(RAX, RCX, doubles);
                self.a.test_rr32(RCX, RCX);
                self.a.jcc(LE, doubles);
            }
            Src::Imm(value) if value.is_int() && value.as_int() > 0 => {
                self.guard_int(RAX, doubles);
                self.a.mov_imm64(RCX, u64::from(value.as_int() as u32));
            }
            Src::Imm(_) => self.a.jmp(doubles),
        }
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(S, doubles);
        self.a.idiv32(RCX);
        self.a.mov_rr32(RAX, RDX);
        self.a.alu_rr(Alu::Or, RAX, INT);
        self.a.store(self.reg(dst), RAX);
        self.a.bind(done);
        self.a.set_cold(true);
        self.a.bind(doubles);
        self.note(DOUBLE);
        self.load(RAX, Src::Reg(a));
        self.load(RCX, y);
        self.src_double(Src::Reg(a), RAX, XMM0, slow);
        self.src_double(y, RCX, XMM1, slow);
        self.call_helper(helpers::fmod as *const () as usize);
        self.store_number(dst, XMM0);
        self.a.jmp(done);
        self.a.set_cold(false);
        self.cold_generic_leaving(i, slow, done, dst, RAX);
        self.leaves(dst, RAX);
    }

    fn bitwise(&mut self, i: usize, dst: u16, a: u16, y: Src, kind: Bit) {
        let quick = self.a.label();
        let slow = self.a.label();
        let operate = self.a.label();
        let ushr_double = self.a.label();
        let done = self.a.label();
        self.operand_reg(RAX, a);
        match y {
            Src::Reg(b) => {
                self.operand_reg(RCX, b);
                self.guard_ints(RAX, RCX, quick);
            }
            Src::Imm(value) => {
                self.a.mov_imm64(RCX, u64::from(value.as_int() as u32));
                self.guard_int(RAX, quick);
            }
        }
        self.a.bind(operate);
        match kind {
            Bit::And => self.a.alu_rr32(Alu::And, RAX, RCX),
            Bit::Or => self.a.alu_rr32(Alu::Or, RAX, RCX),
            Bit::Xor => self.a.alu_rr32(Alu::Xor, RAX, RCX),
            Bit::Shl => self.a.shift_cl32(Shift::Shl, RAX),
            Bit::Shr => self.a.shift_cl32(Shift::Sar, RAX),
            Bit::UShr => {
                self.a.shift_cl32(Shift::Shr, RAX);
                self.a.test_rr32(RAX, RAX);
                self.a.jcc(S, ushr_double);
            }
        }
        self.a.alu_rr(Alu::Or, RAX, INT);
        self.a.store(self.reg(dst), RAX);
        self.a.bind(done);
        self.a.set_cold(true);
        // Doubles that fit in 64 bits, as `quick_int32` does.
        self.a.bind(quick);
        self.note(DOUBLE);
        self.to_int32(RAX, slow);
        if let Src::Reg(_) = y {
            self.to_int32(RCX, slow);
        }
        self.a.jmp(operate);
        if kind == Bit::UShr {
            self.a.bind(ushr_double);
            self.note(DOUBLE);
            self.a.mov_rr32(RAX, RAX);
            self.a.cvtsi2sd64(XMM0, RAX);
            self.a.movq_rx(RAX, XMM0);
            self.a.store(self.reg(dst), RAX);
            self.a.jmp(done);
        }
        self.a.set_cold(false);
        self.cold_generic_leaving(i, slow, done, dst, RAX);
        self.leaves(dst, RAX);
    }

    fn bitwise_int(&mut self, i: usize, dst: u16, a: u16, value: i16, kind: Bit) {
        self.bitwise(i, dst, a, Src::Imm(Value::int(value.into())), kind)
    }

    /// Jumps to `on_true` or `on_false` on whether `x rel y`: ints here, doubles out of the way, anything else by a
    /// helper that only compares.
    fn branch_compare(&mut self, i: usize, x: Src, y: Src, rel: Rel, on_true: Label, on_false: Label) {
        let doubles = self.a.label();
        let slow = self.a.label();
        self.operand(RAX, x);
        self.operand(RCX, y);
        // A double compared the last time round (see `looks_double`) is compared as doubles in line, which does ints
        // too.
        let doubles_first = match (x, y) {
            (Src::Reg(_), Src::Reg(_)) => self.looks_double(x) && self.looks_double(y),
            _ => self.looks_double(x) || self.looks_double(y),
        };
        if !doubles_first {
            match (x, y) {
                (Src::Reg(_), Src::Reg(_)) => self.guard_ints(RAX, RCX, doubles),
                (Src::Reg(_), Src::Imm(_)) => self.guard_int(RAX, doubles),
                (Src::Imm(_), Src::Reg(_)) => self.guard_int(RCX, doubles),
                (Src::Imm(_), Src::Imm(_)) => {}
            }
            self.a.alu_rr32(Alu::Cmp, RAX, RCX);
            self.a.jcc(rel.int_cc(), on_true);
            self.a.jmp(on_false);
            self.a.set_cold(true);
        }
        self.a.bind(doubles);
        self.note(DOUBLE);
        self.src_double(x, RAX, XMM0, slow);
        self.src_double(y, RCX, XMM1, slow);
        // Ordered and true: NaN makes every one of these false.
        match rel {
            Rel::Lt => {
                self.a.ucomisd(XMM1, XMM0);
                self.a.jcc(A, on_true);
            }
            Rel::Le => {
                self.a.ucomisd(XMM1, XMM0);
                self.a.jcc(AE, on_true);
            }
            Rel::Gt => {
                self.a.ucomisd(XMM0, XMM1);
                self.a.jcc(A, on_true);
            }
            Rel::Ge => {
                self.a.ucomisd(XMM0, XMM1);
                self.a.jcc(AE, on_true);
            }
        }
        self.a.jmp(on_false);
        self.a.set_cold(true);
        self.a.bind(slow);
        if let Some(deopt) = self.deopt() {
            self.a.jmp(deopt);
            self.a.set_cold(false);
            return;
        }
        self.load(R8, x);
        self.load(R9, y);
        self.a.mov_rr(RCX, VM);
        self.a.mov_imm64(RDX, u64::from(self.pc(i)));
        self.call_helper(helpers::compare as *const () as usize);
        self.a.alu_ri32(Alu::Cmp, RAX, TAKEN as i32);
        self.a.jcc(E, on_true);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NE, self.exit);
        self.a.jmp(on_false);
        self.a.set_cold(false);
    }

    #[allow(clippy::too_many_arguments)]
    fn jump_compare(&mut self, i: usize, x: Src, y: Src, rel: Rel, negate: bool, offset: i16, is_loop: bool) {
        let to = self.target(i, i32::from(offset));
        let next = self.a.label();
        let taken = if is_loop { self.a.label() } else { to };
        let (on_true, on_false) = if negate { (next, taken) } else { (taken, next) };
        self.branch_compare(i, x, y, rel, on_true, on_false);
        if is_loop {
            self.a.bind(taken);
            self.count();
            self.safe_point(self.pc(i) + 1);
            self.a.jmp(to);
        }
        self.a.bind(next);
    }

    fn compare_value(&mut self, i: usize, dst: u16, x: Src, y: Src, rel: Rel) {
        let on_true = self.a.label();
        let on_false = self.a.label();
        let done = self.a.label();
        self.branch_compare(i, x, y, rel, on_true, on_false);
        self.a.bind(on_true);
        self.store_imm(dst, TRUE);
        self.a.jmp(done);
        self.a.bind(on_false);
        self.store_imm(dst, FALSE);
        self.a.bind(done);
    }

    /// `===`: jumps to `equal` or `differ`. Two strings or bigints are compared by a helper.
    fn branch_strict(&mut self, a: u16, b: u16, equal: Label, differ: Label) {
        let same = self.a.label();
        let first_other = self.a.label();
        let both_other = self.a.label();
        let slow = self.a.label();
        self.operand_reg(RAX, a);
        self.operand_reg(RCX, b);
        self.a.alu_rr(Alu::Cmp, RAX, RCX);
        self.a.jcc(E, same);
        // Different bits: unequal if either is only ever equal to itself (an object, null, undefined, a boolean, a
        // symbol), so its tag less the object tag is 0, 2, 3 or 4.
        self.only_itself(RAX, differ, first_other);
        self.a.bind(first_other);
        self.only_itself(RCX, differ, both_other);
        self.a.bind(same);
        // The same bits: equal, unless it's NaN.
        self.a.alu_rr(Alu::Cmp, RAX, INT);
        self.a.jcc(AE, equal);
        self.a.movq_xr(XMM0, RAX);
        self.a.ucomisd(XMM0, XMM0);
        self.a.jcc(P, differ);
        self.a.jmp(equal);
        self.a.set_cold(true);
        // Both could be equal to something with other bits: two numbers are compared here, strings and bigints the
        // generic way. Two ints with different bits are different.
        self.a.bind(both_other);
        let not_ints = self.a.label();
        self.guard_ints(RAX, RCX, not_ints);
        self.a.jmp(differ);
        self.a.bind(not_ints);
        self.to_double(RAX, XMM0, slow);
        self.to_double(RCX, XMM1, slow);
        self.a.ucomisd(XMM0, XMM1);
        self.a.jcc(P, differ);
        self.a.jcc(E, equal);
        self.a.jmp(differ);
        self.a.bind(slow);
        self.a.mov_rr(RDX, RAX);
        self.a.mov_rr(R8, RCX);
        self.a.mov_rr(RCX, VM);
        self.call_helper(helpers::strict_equals as *const () as usize);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NE, equal);
        self.a.jmp(differ);
        self.a.set_cold(false);
    }

    /// Jumps to `itself` if `v` is only ever `===` to itself, else to `other`. Uses r10.
    fn only_itself(&mut self, v: R, itself: Label, other: Label) {
        self.a.mov_rr(R10, v);
        self.a.shift_ri(Shift::Shr, R10, 48);
        self.a.alu_ri32(Alu::Sub, R10, TAG_OBJECT);
        self.a.alu_ri32(Alu::Cmp, R10, 4);
        self.a.jcc(A, other);
        self.a.alu_ri32(Alu::Cmp, R10, 1);
        self.a.jcc(E, other);
        self.a.jmp(itself);
    }

    fn jump_strict(&mut self, i: usize, a: u16, b: u16, negate: bool, offset: i16) {
        let to = self.target(i, i32::from(offset));
        let next = self.a.label();
        let (equal, differ) = if negate { (next, to) } else { (to, next) };
        self.branch_strict(a, b, equal, differ);
        self.a.bind(next);
    }

    fn strict_value(&mut self, dst: u16, a: u16, b: u16, negate: bool) {
        let equal = self.a.label();
        let differ = self.a.label();
        let done = self.a.label();
        self.branch_strict(a, b, equal, differ);
        self.a.bind(equal);
        self.store_imm(dst, if negate { FALSE } else { TRUE });
        self.a.jmp(done);
        self.a.bind(differ);
        self.store_imm(dst, if negate { TRUE } else { FALSE });
        self.a.bind(done);
    }

    /// `while (a !== b)` at the bottom of its loop: round again, by way of a safe point, while they differ.
    fn loop_strict(&mut self, i: usize, a: u16, b: u16, offset: i16) {
        let to = self.target(i, i32::from(offset));
        let next = self.a.label();
        let taken = self.a.label();
        self.branch_strict(a, b, next, taken);
        self.a.bind(taken);
        self.count();
        self.safe_point(self.pc(i) + 1);
        self.a.jmp(to);
        self.a.bind(next);
    }

    /// `a === value` (or `!==` with `negate`) and its jump; for a loop's test at the bottom (`is_loop`), the jump back
    /// is by way of a safe point.
    fn jump_strict_int(&mut self, i: usize, a: u16, value: i16, negate: bool, offset: i16, is_loop: bool) {
        let to = self.target(i, i32::from(offset));
        let next = self.a.label();
        let taken = if is_loop { self.a.label() } else { to };
        let (equal, differ) = if negate { (next, taken) } else { (taken, next) };
        let double = self.a.label();
        self.operand_reg(RAX, a);
        self.a.mov_imm64(RCX, Value::int(value.into()).0);
        self.a.alu_rr(Alu::Cmp, RAX, RCX);
        self.a.jcc(E, equal);
        self.a.alu_rr(Alu::Cmp, RAX, INT);
        self.a.jcc(B, double);
        self.a.jmp(differ);
        self.a.set_cold(true);
        self.a.bind(double);
        self.a.movq_xr(XMM0, RAX);
        self.a.mov_imm64(RCX, u64::from(i32::from(value) as u32));
        self.a.cvtsi2sd(XMM1, RCX);
        self.a.ucomisd(XMM0, XMM1);
        self.a.jcc(P, differ);
        self.a.jcc(E, equal);
        self.a.jmp(differ);
        self.a.set_cold(false);
        if is_loop {
            self.a.bind(taken);
            self.count();
            self.safe_point(self.pc(i) + 1);
            self.a.jmp(to);
        }
        self.a.bind(next);
    }

    /// `typeof a == "..."` and its jump: told here from the tag, except whether an object is a function, which is the
    /// generic way's.
    fn jump_type(&mut self, i: usize, a: u16, kind: u16, negate: bool, offset: i16) {
        use crate::op::*;
        let to = self.target(i, i32::from(offset));
        let next = self.a.label();
        let slow = self.a.label();
        let (is, isnt) = if negate { (next, to) } else { (to, next) };
        self.a.load(RAX, self.reg(a));
        self.a.mov_rr(R10, RAX);
        self.a.shift_ri(Shift::Shr, R10, 48);
        let tag_is = |g: &mut Self, tag: i32| {
            g.a.alu_ri32(Alu::Cmp, R10, tag);
            g.a.jcc(E, is);
            g.a.jmp(isnt);
        };
        match kind {
            TYPE_NUMBER => {
                self.a.alu_ri32(Alu::Cmp, R10, TAG_OBJECT);
                self.a.jcc(B, is);
                self.a.jmp(isnt);
            }
            TYPE_STRING => tag_is(self, 0xFFFB),
            TYPE_SYMBOL => tag_is(self, 0xFFFD),
            TYPE_BIGINT => tag_is(self, 0xFFFF),
            TYPE_UNDEFINED => {
                self.a.alu_rr(Alu::Cmp, RAX, UNDEF);
                self.a.jcc(E, is);
                self.a.jmp(isnt);
            }
            TYPE_BOOLEAN => {
                self.a.alu_ri(Alu::Or, RAX, 1);
                self.a.mov_imm64(RCX, TRUE);
                self.a.alu_rr(Alu::Cmp, RAX, RCX);
                self.a.jcc(E, is);
                self.a.jmp(isnt);
            }
            TYPE_FUNCTION => {
                self.a.alu_ri32(Alu::Cmp, R10, TAG_OBJECT);
                self.a.jcc(E, slow);
                self.a.jmp(isnt);
            }
            _ => {
                // "object": null is, an object might be, numbers, strings, undefined, booleans, symbols and bigints
                // aren't, and anything else the generic way says.
                self.a.alu_ri32(Alu::Cmp, R10, TAG_OBJECT);
                self.a.jcc(E, slow);
                self.a.jcc(B, isnt);
                self.a.mov_imm64(RCX, NULL);
                self.a.alu_rr(Alu::Cmp, RAX, RCX);
                self.a.jcc(E, is);
                for tag in [0xFFFB, 0xFFFD, 0xFFFF] {
                    self.a.alu_ri32(Alu::Cmp, R10, tag);
                    self.a.jcc(E, isnt);
                }
                self.a.alu_rr(Alu::Cmp, RAX, UNDEF);
                self.a.jcc(E, isnt);
                self.a.mov_rr(RCX, RAX);
                self.a.alu_ri(Alu::Or, RCX, 1);
                self.a.mov_imm64(R11, TRUE);
                self.a.alu_rr(Alu::Cmp, RCX, R11);
                self.a.jcc(E, isnt);
                self.a.jmp(slow);
            }
        }
        // Whether an object is a function, by a helper that only says what `typeof` says. (rax has the value.)
        self.a.set_cold(true);
        self.a.bind(slow);
        self.a.mov_rr(RDX, RAX);
        self.a.mov_rr(RCX, VM);
        self.call_helper(helpers::type_kind as *const () as usize);
        self.a.alu_ri32(Alu::Cmp, RAX, i32::from(kind));
        self.a.jcc(E, is);
        self.a.jmp(isnt);
        self.a.set_cold(false);
        self.a.bind(next);
    }

    /// `==`: two ints here, anything else the generic way.
    fn jump_loose(&mut self, i: usize, a: u16, b: u16, negate: bool, offset: i16) {
        let to = self.target(i, i32::from(offset));
        let next = self.a.label();
        let slow = self.a.label();
        let (equal, differ) = if negate { (next, to) } else { (to, next) };
        self.a.load(RAX, self.reg(a));
        self.a.load(RCX, self.reg(b));
        self.guard_ints(RAX, RCX, slow);
        self.a.alu_rr(Alu::Cmp, RAX, RCX);
        self.a.jcc(E, equal);
        self.a.jmp(differ);
        self.a.set_cold(true);
        self.a.bind(slow);
        self.generic_jump(i, to, next);
        self.a.set_cold(false);
        self.a.bind(next);
    }

    fn loose_value(&mut self, i: usize, dst: u16, a: u16, b: u16, negate: bool) {
        let slow = self.a.label();
        let done = self.a.label();
        self.a.load(RAX, self.reg(a));
        self.a.load(RCX, self.reg(b));
        self.guard_ints(RAX, RCX, slow);
        self.a.mov_imm64(RDX, if negate { TRUE } else { FALSE });
        self.a.mov_imm64(R8, if negate { FALSE } else { TRUE });
        self.a.alu_rr(Alu::Cmp, RAX, RCX);
        self.a.cmov(E, RDX, R8);
        self.a.store(self.reg(dst), RDX);
        self.a.bind(done);
        self.cold_generic(i, slow, done);
    }

    /// The end of `for (...; i < n; i++)`: steps `i`, and goes round again while it's below the limit.
    fn inc_loop(&mut self, i: usize, counter: u16, limit: Src, offset: i16) {
        let to = self.target(i, i32::from(offset));
        let slow = self.a.label();
        let compare_slow = self.a.label();
        let taken = self.a.label();
        let next = self.a.label();
        self.operand_reg(RAX, counter);
        self.guard_int(RAX, slow);
        self.a.alu_ri32(Alu::Cmp, RAX, i32::MAX);
        self.a.jcc(E, slow);
        self.a.alu_ri32(Alu::Add, RAX, 1);
        self.a.alu_rr(Alu::Or, RAX, INT);
        self.a.store(self.reg(counter), RAX);
        match limit {
            Src::Reg(limit) => {
                self.a.load(RCX, self.reg(limit));
                self.guard_int(RCX, compare_slow);
                self.a.alu_rr32(Alu::Cmp, RAX, RCX);
            }
            Src::Imm(value) => self.a.alu_ri32(Alu::Cmp, RAX, value.as_int()),
        }
        self.a.jcc(GE, next);
        self.a.bind(taken);
        self.count();
        self.safe_point(self.pc(i) + 1);
        self.a.jmp(to);
        self.a.set_cold(true);
        // The step the generic way does the test too.
        self.a.bind(slow);
        self.generic_jump(i, taken, next);
        if let Src::Reg(_) = limit {
            // `i` is a stepped int; the limit isn't one.
            self.a.bind(compare_slow);
            let not_number = self.a.label();
            self.a.cvtsi2sd(XMM0, RAX);
            self.to_double(RCX, XMM1, not_number);
            self.a.ucomisd(XMM1, XMM0);
            self.a.jcc(A, taken);
            self.a.jmp(next);
            self.a.bind(not_number);
            self.a.mov_rr(RCX, VM);
            self.a.mov_imm64(RDX, u64::from(self.pc(i)));
            self.call_helper(helpers::loop_test as *const () as usize);
            self.a.alu_ri32(Alu::Cmp, RAX, TAKEN as i32);
            self.a.jcc(E, taken);
            self.a.test_rr32(RAX, RAX);
            self.a.jcc(NE, self.exit);
            self.a.jmp(next);
        }
        self.a.set_cold(false);
        self.a.bind(next);
    }

    fn inc_loop_length(&mut self, i: usize, counter: u16, object: u16, offset: i16) {
        let to = self.target(i, i32::from(offset));
        let slow = self.a.label();
        let test_slow = self.a.label();
        let taken = self.a.label();
        let next = self.a.label();
        let o = self.l.obj;
        self.a.load(RAX, self.reg(counter));
        self.guard_int(RAX, slow);
        self.a.alu_ri32(Alu::Cmp, RAX, i32::MAX);
        self.a.jcc(E, slow);
        self.a.alu_ri32(Alu::Add, RAX, 1);
        self.a.alu_rr(Alu::Or, RAX, INT);
        self.a.store(self.reg(counter), RAX);
        self.a.load(RCX, self.reg(object));
        self.object_address(RCX, test_slow);
        self.a.cmp_m8(m(RCX, o.kind as i32), o.array_tag);
        self.a.jcc(NE, test_slow);
        self.a.load32(RCX, m(RCX, (o.kind + o.array_length) as i32));
        self.a.test_rr32(RCX, RCX);
        self.a.jcc(S, test_slow);
        self.a.alu_rr32(Alu::Cmp, RAX, RCX);
        self.a.jcc(GE, next);
        self.a.bind(taken);
        self.count();
        self.safe_point(self.pc(i) + 1);
        self.a.jmp(to);
        self.a.set_cold(true);
        self.a.bind(slow);
        self.generic_jump(i, taken, next);
        // Stepped already: only the test is left, with the length of what isn't an array (a typed array's,
        // `arguments`') by a helper that only finds it if it's quick to, then the helper that does the whole test.
        let full = self.a.label();
        self.a.bind(test_slow);
        self.a.mov_rr(RCX, VM);
        self.a.load(RDX, self.reg(object));
        self.call_helper(helpers::quick_length as *const () as usize);
        self.guard_int(RAX, full);
        self.a.load(RCX, self.reg(counter));
        self.a.alu_rr32(Alu::Cmp, RCX, RAX);
        self.a.jcc(GE, next);
        self.a.jmp(taken);
        self.a.bind(full);
        self.a.mov_rr(RCX, VM);
        self.a.mov_imm64(RDX, u64::from(self.pc(i)));
        self.call_helper(helpers::loop_test as *const () as usize);
        self.a.alu_ri32(Alu::Cmp, RAX, TAKEN as i32);
        self.a.jcc(E, taken);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NE, self.exit);
        self.a.jmp(next);
        self.a.set_cold(false);
        self.a.bind(next);
    }

    /// Reads property `cache` of the value in `rax` (from register `src`) into `out` if a cache entry knows where it
    /// is, else on to `slow`. The place's first entry is tried here; out of the way, its others (it may have become
    /// polymorphic since this was compiled, which the interpreter would have rewritten the instruction for), what
    /// reads remember once it's megamorphic, and a string's methods.
    fn get_prop(&mut self, cache: u16, src: u16, slow: Label, out: R) {
        let l = self.l;
        let holder = self.a.label();
        let read = self.a.label();
        let not_object = self.a.label();
        let others = self.a.label();
        let mega = self.a.label();
        let done = self.a.label();
        self.object_address(RAX, not_object);
        // What the place found when this was compiled, in the code (see `fixed_read`), with the cache's way out of
        // the way.
        let fixed = self.fixed_read(cache);
        let was = self.a.is_cold();
        if let Some((shape, holder, slot)) = fixed {
            let data = self.a.label();
            self.a.alu_mi32(Alu::Cmp, m(RAX, l.obj.shape as i32), shape as i32);
            self.a.jcc(NE, data);
            if let Some(holder) = holder {
                self.a.alu_mi32(Alu::Cmp, m(VM, l.epoch), self.job.epoch as i32);
                self.a.jcc(NE, data);
                self.a.mov_imm64(RAX, self.bases.objects + 64 * u64::from(holder));
            }
            self.read_slot_at(RAX, slot, out);
            self.a.jmp(done);
            self.a.set_cold(true);
            self.a.bind(data);
        }
        self.a.load32(RCX, m(RAX, l.obj.shape as i32));
        self.a.alu_rm32(Alu::Cmp, RCX, self.cache(cache, l.c_shape));
        self.a.jcc(NE, others);
        self.a.load32(RCX, self.cache(cache, l.c_other));
        self.a.alu_ri32(Alu::Cmp, RCX, -1);
        self.a.jcc(NE, holder);
        self.a.bind(read);
        self.a.load32(RCX, self.cache(cache, l.c_slot));
        self.read_slot(RAX, RCX, out);
        self.a.jmp(done);
        // On a prototype, as long as no prototype has changed since.
        self.a.bind(holder);
        self.a.load32(RDX, self.cache(cache, l.c_epoch));
        self.a.alu_rm32(Alu::Cmp, RDX, m(VM, l.epoch));
        self.a.jcc(NE, slow);
        self.a.mov_rr32(RAX, RCX);
        self.a.shift_ri(Shift::Shl, RAX, 6);
        self.a.alu_rr(Alu::Add, RAX, OBJECTS);
        self.a.jmp(read);
        self.a.set_cold(true);
        self.a.bind(not_object);
        self.primitive_get(cache, slow, out, done);
        // Another shape than the first entry's (in ecx): the entry of the others that knows it.
        self.a.bind(others);
        let own = self.a.label();
        self.poly_others(cache, slow, mega);
        self.a.load32(R8, m(R9, l.c_other));
        self.a.alu_ri32(Alu::Cmp, R8, -1);
        self.a.jcc(E, own);
        self.a.load32(RDX, m(R9, l.c_epoch));
        self.a.alu_rm32(Alu::Cmp, RDX, m(VM, l.epoch));
        self.a.jcc(NE, slow);
        self.a.mov_rr32(RAX, R8);
        self.a.shift_ri(Shift::Shl, RAX, 6);
        self.a.alu_rr(Alu::Add, RAX, OBJECTS);
        self.a.bind(own);
        self.a.load32(R8, m(R9, l.c_slot));
        self.read_slot(RAX, R8, out);
        self.a.jmp(done);
        // Too many shapes for the place: what reads remember, by a helper that does only that.
        self.a.bind(mega);
        self.a.mov_rr(RCX, VM);
        self.a.load(RDX, self.reg(src));
        self.a.mov_imm64(R8, u64::from(self.proto.cache_base) + u64::from(cache));
        self.call_helper(helpers::mega_get as *const () as usize);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NE, slow);
        self.a.load(out, m(VM, l.value));
        self.a.jmp(done);
        self.a.set_cold(was);
        self.a.bind(done);
    }

    /// Writes register `value` to element `rcx` of the array at `rax`, if it's there and the array can be written in
    /// place (not frozen, not old, which the write barrier's slow way is for), then on to `done`; else `slow`.
    fn array_write(&mut self, value: u16, slow: Label, done: Label) {
        let o = self.l.obj;
        self.a.test_m8(m(RAX, o.flags as i32), crate::object::FROZEN_ELEMENTS | crate::gc::OLD);
        self.a.jcc(NE, slow);
        self.a.alu_rm32(Alu::Cmp, RCX, m(RAX, (o.kind + o.array_length) as i32));
        self.a.jcc(AE, slow);
        self.a.load(R10, m(RAX, (o.kind + o.array_start) as i32));
        self.a.lea(R10, mi(R10, RCX, 3, 0));
        self.a.mov_imm64(R11, EMPTY);
        self.a.alu_rm(Alu::Cmp, R11, m(R10, 0));
        self.a.jcc(E, slow);
        self.a.load(RDX, self.reg(value));
        self.a.store(m(R10, 0), RDX);
        self.a.jmp(done);
    }

    /// The typed array at `rax`'s quick elements (see `TypedData::quick`), when they're right (its epoch is the
    /// engine's `buffer_epoch`) and `rcx` is below how many there are: leaves where they start in r11, the element
    /// kind's byte in `kind`, and r10 at its `TypedData`. Else on to `slow`.
    fn typed_place(&mut self, kind: R, slow: Label) {
        let l = self.l;
        self.a.load(R10, m(RAX, (l.obj.kind + l.obj.typed_data) as i32));
        self.a.load(R11, m(R10, l.t_epoch));
        self.a.alu_rm(Alu::Cmp, R11, m(VM, l.buffer_epoch));
        self.a.jcc(NE, slow);
        self.a.alu_rm(Alu::Cmp, RCX, m(R10, l.t_quick_length));
        self.a.jcc(AE, slow);
        self.a.load(R11, m(R10, l.t_quick));
        self.a.load8(kind, m(R10, l.t_element));
        self.note_typed(kind);
    }

    /// Reads element `rcx` of the object at `rax`, if it's a typed array whose quick elements are right, into
    /// register `dst` as `TypedData::read_quick` makes it, then on to `done`; else `slow`.
    fn typed_read(&mut self, dst: u16, slow: Label, done: Label) {
        use crate::builtins::Element as E8;
        let o = self.l.obj;
        let el = |e: E8| i32::from(o.elements[e as usize]);
        self.a.cmp_m8(m(RAX, o.kind as i32), o.typed_tag);
        self.a.jcc(NE, slow);
        self.typed_place(RDX, slow);
        let ints: [(E8, u8, u8); 6] = [
            (E8::Uint8, 0, 0),
            (E8::Uint8Clamped, 0, 0),
            (E8::Int32, 2, 0),
            (E8::Int8, 0, 1),
            (E8::Int16, 1, 1),
            (E8::Uint16, 1, 0),
        ];
        let int_done = self.a.label();
        for (element, scale, signed) in ints {
            let next = self.a.label();
            self.a.alu_ri32(Alu::Cmp, RDX, el(element));
            self.a.jcc(NE, next);
            let at = mi(R11, RCX, scale, 0);
            match (scale, signed) {
                (0, 0) => self.a.load8(RAX, at),
                (0, _) => self.a.load8s(RAX, at),
                (1, 0) => self.a.load16(RAX, at),
                (1, _) => self.a.load16s(RAX, at),
                _ => self.a.load32(RAX, at),
            }
            self.a.jmp(int_done);
            self.a.bind(next);
        }
        let not_f64 = self.a.label();
        let not_f32 = self.a.label();
        self.a.alu_ri32(Alu::Cmp, RDX, el(E8::Float64));
        self.a.jcc(NE, not_f64);
        self.a.movsd_load(XMM0, mi(R11, RCX, 3, 0));
        self.store_number(dst, XMM0);
        self.a.jmp(done);
        self.a.bind(not_f64);
        self.a.alu_ri32(Alu::Cmp, RDX, el(E8::Float32));
        self.a.jcc(NE, not_f32);
        self.a.movss_load(XMM0, mi(R11, RCX, 2, 0));
        self.a.cvtss2sd(XMM0, XMM0);
        self.store_number(dst, XMM0);
        self.a.jmp(done);
        // An unsigned 32-bit one too big for an int is a double.
        self.a.bind(not_f32);
        self.a.alu_ri32(Alu::Cmp, RDX, el(E8::Uint32));
        self.a.jcc(NE, slow);
        self.a.load32(RAX, mi(R11, RCX, 2, 0));
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NS, int_done);
        self.a.cvtsi2sd64(XMM0, RAX);
        self.a.movq_rx(RAX, XMM0);
        self.a.store(self.reg(dst), RAX);
        self.a.jmp(done);
        self.a.bind(int_done);
        self.a.mov_rr32(RAX, RAX);
        self.a.alu_rr(Alu::Or, RAX, INT);
        self.a.store(self.reg(dst), RAX);
        self.a.jmp(done);
    }

    /// Writes register `value` to element `rcx` of the typed array at `rax`, if its quick elements are right, as
    /// `TypedData::write_quick` does: an int to any kind, a double to a Float64Array or Float32Array, then on to
    /// `done`. Else (a double for an array of ints, which has to be wrapped or clamped, or what isn't a number)
    /// `slow`.
    fn typed_write(&mut self, value: u16, slow: Label, done: Label) {
        use crate::builtins::Element as E8;
        let o = self.l.obj;
        let el = |e: E8| i32::from(o.elements[e as usize]);
        let not_int = self.a.label();
        self.typed_place(R8, slow);
        self.a.load(RDX, self.reg(value));
        self.a.mov_rr(R9, RDX);
        self.a.shift_ri(Shift::Shr, R9, 32);
        self.a.alu_ri32(Alu::Cmp, R9, (INT_TAG >> 32) as u32 as i32);
        self.a.jcc(NE, not_int);
        // An int, in edx.
        let stores: [(E8, u8); 6] = [
            (E8::Uint8, 0),
            (E8::Int32, 2),
            (E8::Int8, 0),
            (E8::Int16, 1),
            (E8::Uint16, 1),
            (E8::Uint32, 2),
        ];
        for (element, scale) in stores {
            let next = self.a.label();
            self.a.alu_ri32(Alu::Cmp, R8, el(element));
            self.a.jcc(NE, next);
            let at = mi(R11, RCX, scale, 0);
            match scale {
                0 => self.a.store8(at, RDX),
                1 => self.a.store16(at, RDX),
                _ => self.a.store32(at, RDX),
            }
            self.a.jmp(done);
            self.a.bind(next);
        }
        let not_clamped = self.a.label();
        self.a.alu_ri32(Alu::Cmp, R8, el(E8::Uint8Clamped));
        self.a.jcc(NE, not_clamped);
        self.a.mov_rr32(RDX, RDX);
        self.a.mov_imm64(R9, 255);
        self.a.alu_ri32(Alu::Cmp, RDX, 255);
        self.a.cmov(G, RDX, R9);
        self.a.alu_rr32(Alu::Xor, R9, R9);
        self.a.test_rr32(RDX, RDX);
        self.a.cmov(S, RDX, R9);
        self.a.store8(mi(R11, RCX, 0, 0), RDX);
        self.a.jmp(done);
        self.a.bind(not_clamped);
        self.a.cvtsi2sd(XMM0, RDX);
        let float = self.a.label();
        self.a.jmp(float);
        // A double.
        self.a.bind(not_int);
        self.a.alu_rr(Alu::Cmp, RDX, INT);
        self.a.jcc(AE, slow);
        self.a.movq_xr(XMM0, RDX);
        self.a.bind(float);
        let not_f64 = self.a.label();
        self.a.alu_ri32(Alu::Cmp, R8, el(E8::Float64));
        self.a.jcc(NE, not_f64);
        self.a.movsd_store(mi(R11, RCX, 3, 0), XMM0);
        self.a.jmp(done);
        self.a.bind(not_f64);
        self.a.alu_ri32(Alu::Cmp, R8, el(E8::Float32));
        self.a.jcc(NE, slow);
        self.a.cvtsd2ss(XMM0, XMM0);
        self.a.movss_store(mi(R11, RCX, 2, 0), XMM0);
        self.a.jmp(done);
    }

    /// After cell `eax` has had the value in `rcx` stored in it: anything but a number may need the cell remembered
    /// (the write barrier), which a helper does out of the way; then on to `done`.
    fn cell_barrier(&mut self, done: Label) {
        let barrier = self.a.label();
        self.a.mov_imm64(RDX, OBJECT_TAG);
        self.a.alu_rr(Alu::Cmp, RCX, RDX);
        self.a.jcc(AE, barrier);
        let was = self.a.is_cold();
        self.a.set_cold(true);
        self.a.bind(barrier);
        self.a.mov_rr(R8, RCX);
        self.a.mov_rr32(RDX, RAX);
        self.a.mov_rr(RCX, VM);
        self.call_helper(helpers::cell_barrier as *const () as usize);
        self.a.jmp(done);
        self.a.set_cold(was);
    }

    /// Adds a property to the object at `rax` (from register `object`) with the value in register `value`, as the
    /// cache entry in `entry` (or the place's first) says (the shape that makes, in its `other`), as SetProp does: as
    /// long as no prototype has changed and the object can be added to, else on to `slow`. A plain object's first
    /// few values go inside it, as `Object::push_slot` puts them, and anything else by a helper.
    fn add_slot(&mut self, cache: u16, entry: Option<R>, object: u16, value: u16, slow: Label, done: Label) {
        let l = self.l;
        let o = l.obj;
        let field = |g: &Self, at: i32| match entry {
            Some(r) => m(r, at),
            None => g.cache(cache, at),
        };
        let inside = self.a.label();
        let call = self.a.label();
        self.a.load32(RCX, field(self, l.c_epoch));
        self.a.alu_rm32(Alu::Cmp, RCX, m(VM, l.epoch));
        self.a.jcc(NE, slow);
        self.a.test_m8(
            m(RAX, o.flags as i32),
            crate::object::NOT_EXTENSIBLE | crate::object::IS_PROTO,
        );
        self.a.jcc(NE, slow);
        self.a.load8(RCX, m(RAX, o.slot_count as i32));
        self.a.alu_ri32(Alu::Cmp, RCX, i32::from(o.inline_count));
        self.a.jcc(AE, call);
        self.a.test_rr32(RCX, RCX);
        self.a.jcc(NE, inside);
        self.a.cmp_m8(m(RAX, o.kind as i32), o.plain_tag);
        self.a.jcc(NE, call);
        self.a.bind(inside);
        self.a.load(RDX, self.reg(value));
        self.a.store(mi(RAX, RCX, 3, o.inline_slots as i32), RDX);
        self.a.inc_m8(m(RAX, o.slot_count as i32));
        self.a.load32(R8, field(self, l.c_other));
        self.a.store32(m(RAX, o.shape as i32), R8);
        self.a.alu_mi(Alu::Add, m(VM, l.debt), 8);
        self.a.jmp(done);
        self.a.bind(call);
        self.a.load32(R8, field(self, l.c_other));
        self.a.mov_rr(RCX, VM);
        self.a.load32(RDX, self.reg(object));
        self.a.load(R9, self.reg(value));
        self.call_helper(helpers::add_slot as *const () as usize);
        self.a.jmp(done);
    }

    /// Reads property `cache` of a string or a number into `out`, then on to `done`, or `slow` for anything else
    /// (r10 has the value's tag). The place remembers where it's found (on `String.prototype` or `Number.prototype`,
    /// further up, or nowhere, as `Vm::absent`) for that prototype's shape, in its first entry or its others, as
    /// `get_from_primitive_proto` does, while no prototype has changed.
    fn primitive_get(&mut self, cache: u16, slow: Label, out: R, done: Label) {
        let l = self.l;
        let string = self.a.label();
        let prototype = self.a.label();
        let found = self.a.label();
        self.a.alu_ri32(Alu::Cmp, R10, TAG_STRING);
        self.a.jcc(E, string);
        self.a.alu_ri32(Alu::Cmp, R10, TAG_INT);
        self.a.jcc(A, slow);
        self.a.load32(RCX, m(VM, l.number_proto));
        self.a.jmp(prototype);
        self.a.bind(string);
        self.a.load32(RCX, m(VM, l.string_proto));
        self.a.bind(prototype);
        self.a.mov_rr32(RAX, RCX);
        self.a.shift_ri(Shift::Shl, RAX, 6);
        self.a.alu_rr(Alu::Add, RAX, OBJECTS);
        self.a.load32(RCX, m(RAX, l.obj.shape as i32));
        self.a.lea(R9, self.cache(cache, 0));
        self.a.alu_rm32(Alu::Cmp, RCX, m(R9, l.c_shape));
        self.a.jcc(E, found);
        self.poly_others(cache, slow, slow);
        self.a.bind(found);
        self.a.load32(R8, m(R9, l.c_other));
        self.a.alu_ri32(Alu::Cmp, R8, -1);
        self.a.jcc(E, slow);
        self.a.load32(RDX, m(R9, l.c_epoch));
        self.a.alu_rm32(Alu::Cmp, RDX, m(VM, l.epoch));
        self.a.jcc(NE, slow);
        self.a.mov_rr32(RAX, R8);
        self.a.shift_ri(Shift::Shl, RAX, 6);
        self.a.alu_rr(Alu::Add, RAX, OBJECTS);
        self.a.load32(R8, m(R9, l.c_slot));
        self.read_slot(RAX, R8, out);
        self.a.jmp(done);
    }

    /// With `rax` an object's address and `ecx` its shape, which isn't the first entry's of place `cache`: leaves in
    /// `r9` the entry of the place's [`crate::op::POLY`] others that knows the shape. On to `mega` if the place has
    /// seen too many shapes to keep, or `slow` if it has never seen a second one, or none knows this one.
    fn poly_others(&mut self, cache: u16, slow: Label, mega: Label) {
        let l = self.l;
        let found = self.a.label();
        self.a.lea(R9, self.cache(cache, 0));
        // Where the others are, in entries from this one, with the top bit set once it's megamorphic, or `NONE`.
        self.a.load32(RDX, m(R9, l.c_more));
        self.a.alu_ri32(Alu::Cmp, RDX, -1);
        self.a.jcc(E, slow);
        self.a.test_rr32(RDX, RDX);
        self.a.jcc(S, mega);
        self.a.imul_rri(RDX, RDX, l.cache_size);
        self.a.alu_rr(Alu::Add, R9, RDX);
        for other in 0..crate::op::POLY {
            if other > 0 {
                self.a.alu_ri(Alu::Add, R9, l.cache_size);
            }
            self.a.alu_rm32(Alu::Cmp, RCX, m(R9, l.c_shape));
            self.a.jcc(E, found);
        }
        self.a.jmp(slow);
        self.a.bind(found);
    }

    /// Finds the entry of polymorphic place `cache` that knows the shape of the value in `rax`: its first, or one of
    /// its [`crate::op::POLY`] others. Leaves the object's address in `rax` and the entry's in `r9`, or goes on to
    /// `mega` if the place has seen too many shapes to keep, or `slow` if none knows it.
    fn poly_find(&mut self, cache: u16, slow: Label, mega: Label) {
        let l = self.l;
        let found = self.a.label();
        let others = self.a.label();
        self.object_address(RAX, slow);
        self.a.load32(RCX, m(RAX, l.obj.shape as i32));
        self.a.lea(R9, self.cache(cache, 0));
        self.a.alu_rm32(Alu::Cmp, RCX, m(R9, l.c_shape));
        self.a.jcc(NE, others);
        self.a.bind(found);
        let was = self.a.is_cold();
        self.a.set_cold(true);
        self.a.bind(others);
        self.poly_others(cache, slow, mega);
        self.a.jmp(found);
        self.a.set_cold(was);
    }

    /// Reads polymorphic place `cache` of the value in `rax` into register `dst` as GetPropPoly does, going on to
    /// `done`, or to `slow` for the generic way.
    fn get_prop_poly(&mut self, cache: u16, slow: Label, done: Label, dst: u16) {
        let l = self.l;
        let mega = self.a.label();
        let holder = self.a.label();
        let read = self.a.label();
        let not_object = self.a.label();
        let primitive_read = self.a.label();
        self.a.mov_rr(R11, RAX);
        self.a.mov_rr(R10, RAX);
        self.a.shift_ri(Shift::Shr, R10, 48);
        self.a.alu_ri32(Alu::Cmp, R10, TAG_OBJECT);
        self.a.jcc(NE, not_object);
        let was = self.a.is_cold();
        self.a.set_cold(true);
        // A string's or a number's, which a place that's seen objects too keeps among its entries.
        self.a.bind(not_object);
        self.primitive_get(cache, slow, RDX, primitive_read);
        self.a.bind(primitive_read);
        self.a.store(self.reg(dst), RDX);
        self.a.jmp(done);
        self.a.set_cold(was);
        // The shapes the place knew when this was compiled, in the code (see `fixed_read`), then the cache's way for
        // anything else, with the value in rax again.
        let fixed = self.fixed_poly(cache);
        if !fixed.is_empty() {
            self.a.mov_rr32(RAX, RAX);
            self.a.shift_ri(Shift::Shl, RAX, 6);
            self.a.alu_rr(Alu::Add, RAX, OBJECTS);
            self.a.load32(RCX, m(RAX, l.obj.shape as i32));
            for (shape, holder, slot) in fixed {
                let next = self.a.label();
                self.a.alu_ri32(Alu::Cmp, RCX, shape as i32);
                self.a.jcc(NE, next);
                match holder {
                    Some(holder) => {
                        self.a.alu_mi32(Alu::Cmp, m(VM, l.epoch), self.job.epoch as i32);
                        self.a.jcc(NE, next);
                        self.a.mov_imm64(R8, self.bases.objects + 64 * u64::from(holder));
                        self.read_slot_at(R8, slot, RDX);
                    }
                    None => self.read_slot_at(RAX, slot, RDX),
                }
                self.a.store(self.reg(dst), RDX);
                self.a.jmp(done);
                self.a.bind(next);
            }
            self.a.mov_rr(RAX, R11);
        }
        self.poly_find(cache, slow, mega);
        self.a.load32(RCX, m(R9, l.c_other));
        self.a.alu_ri32(Alu::Cmp, RCX, -1);
        self.a.jcc(NE, holder);
        self.a.bind(read);
        self.a.load32(R8, m(R9, l.c_slot));
        self.read_slot(RAX, R8, RDX);
        self.a.store(self.reg(dst), RDX);
        self.a.jmp(done);
        self.a.set_cold(true);
        // On a prototype, as long as no prototype has changed since.
        self.a.bind(holder);
        self.a.load32(RDX, m(R9, l.c_epoch));
        self.a.alu_rm32(Alu::Cmp, RDX, m(VM, l.epoch));
        self.a.jcc(NE, slow);
        self.a.mov_rr32(RAX, RCX);
        self.a.shift_ri(Shift::Shl, RAX, 6);
        self.a.alu_rr(Alu::Add, RAX, OBJECTS);
        self.a.jmp(read);
        // Too many shapes for the place: what reads remember, by a helper that does only that.
        self.a.bind(mega);
        self.a.mov_rr(RCX, VM);
        self.a.mov_rr(RDX, R11);
        self.a.mov_imm64(R8, u64::from(self.proto.cache_base) + u64::from(cache));
        self.call_helper(helpers::mega_get as *const () as usize);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NE, slow);
        self.a.load(RAX, m(VM, l.value));
        self.a.store(self.reg(dst), RAX);
        self.a.jmp(done);
        self.a.set_cold(false);
    }

    /// The next of what a `for of` goes through: an array's next element here, anything else the generic way, which
    /// jumps once there's nothing more.
    fn iter_next(&mut self, i: usize, value: u16, iter: u16, offset: i16) {
        let to = self.target(i, i32::from(offset));
        let o = self.l.obj;
        let next = self.a.label();
        let slow = self.a.label();
        let ended = self.a.label();
        let over = self.a.label();
        self.a.load(RAX, self.reg(iter));
        self.a.mov_imm64(R10, EMPTY);
        self.a.alu_rr(Alu::Cmp, RAX, R10);
        self.a.jcc(E, ended);
        self.a.load(RCX, self.reg(iter + 1));
        self.guard_int(RCX, slow);
        self.object_address(RAX, slow);
        self.a.cmp_m8(m(RAX, o.kind as i32), o.array_tag);
        self.a.jcc(NE, slow);
        self.a.mov_rr32(RDX, RCX);
        self.a.alu_rm32(Alu::Cmp, RDX, m(RAX, (o.kind + o.array_length) as i32));
        self.a.jcc(AE, over);
        self.a.load(R8, m(RAX, (o.kind + o.array_start) as i32));
        self.a.load(R8, mi(R8, RDX, 3, 0));
        // A hole is undefined.
        self.a.mov_imm64(R10, EMPTY);
        self.a.alu_rr(Alu::Cmp, R8, R10);
        self.a.cmov(E, R8, UNDEF);
        self.a.alu_ri32(Alu::Add, RDX, 1);
        self.a.alu_rr(Alu::Or, RDX, INT);
        self.a.store(self.reg(iter + 1), RDX);
        self.a.store(self.reg(value), R8);
        self.a.bind(next);
        self.a.set_cold(true);
        // Past the end: it's finished with.
        self.a.bind(over);
        self.a.mov_imm64(R10, EMPTY);
        self.a.store(self.reg(iter), R10);
        self.a.bind(ended);
        self.a.store(self.reg(value), UNDEF);
        self.a.jmp(to);
        // Anything else by a helper that jumps if it's done, or for a generator has had the interpreter carry the
        // frame on to its end (`helpers::iter_next`), which is then gone.
        let returned = self.a.label();
        let thrown = self.a.label();
        self.a.bind(slow);
        self.a.mov_rr(RCX, VM);
        self.a.mov_imm64(RDX, u64::from(self.pc(i)));
        self.call_helper(helpers::iter_next as *const () as usize);
        self.a.alu_ri32(Alu::Cmp, RAX, TAKEN as i32);
        self.a.jcc(E, to);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(E, next);
        let resume = self.a.label();
        self.a.alu_ri32(Alu::Cmp, RAX, helpers::RESUME as i32);
        self.a.jcc(E, resume);
        self.a.alu_ri32(Alu::Cmp, RAX, helpers::RETURNED as i32);
        self.a.jcc(E, returned);
        self.a.alu_ri32(Alu::Cmp, RAX, helpers::THROWN as i32);
        self.a.jcc(E, thrown);
        self.a.jmp(self.exit);
        // A compiled generator, whose frame the helper has put on top: carried on in its code, which is called as a
        // compiled function is, with its frame's registers and caches. It hands back what it yields (its `yield`
        // has put that where this wants it too), or that it's ended (the helper has put the loop's end in place),
        // or anything else as a call does.
        let l = self.l;
        let entered = self.a.label();
        let yielded = self.a.label();
        self.a.bind(resume);
        self.a.load(R11, m(VM, l.value));
        self.a.load32(RAX, m(FRAME, l.frame_size + l.f_base));
        self.a.mov_imm64(REGS, self.bases.stack);
        self.a.lea(REGS, mi(REGS, RAX, 3, 0));
        self.a.load32(RAX, m(FRAME, l.frame_size + l.f_cache_base));
        self.a.imul_rri(RAX, RAX, l.cache_size);
        self.a.mov_imm64(ICS, self.bases.caches);
        self.a.alu_rr(Alu::Add, ICS, RAX);
        self.a.alu_ri(Alu::Add, FRAME, l.frame_size);
        self.a.call_label(entered);
        self.a.alu_ri(Alu::Sub, FRAME, l.frame_size);
        self.a.load32(RCX, m(FRAME, l.f_base));
        self.a.mov_imm64(REGS, self.bases.stack);
        self.a.lea(REGS, mi(REGS, RCX, 3, 0));
        self.a.mov_imm64(ICS, self.ics);
        self.a.test_rr32(RDX, RDX);
        self.a.jcc(E, yielded);
        self.a.alu_ri32(Alu::Cmp, RDX, helpers::LEFT as i32);
        self.a.jcc(E, to);
        self.a.jmp(self.propagate);
        self.a.bind(yielded);
        self.a.store(self.reg(value), RAX);
        self.a.jmp(next);
        // As a compiled function's prologue leaves the machine's stack.
        self.a.bind(entered);
        self.a.alu_ri(Alu::Sub, RSP, NATIVE_FRAME);
        self.a.jmp_r(R11);
        self.a.bind(returned);
        self.a.load(RAX, m(VM, self.l.value));
        self.a.alu_rr32(Alu::Xor, RDX, RDX);
        self.a.alu_ri(Alu::Add, RSP, NATIVE_FRAME);
        self.a.ret();
        // Its error is its caller's to deal with: a `catch` here was the interpreter's to find.
        self.a.bind(thrown);
        self.a.load(RAX, m(VM, self.l.value));
        self.a.mov_imm64(RDX, u64::from(helpers::THROW));
        self.a.alu_ri(Alu::Add, RSP, NATIVE_FRAME);
        self.a.ret();
        self.a.set_cold(false);
    }

    /// A call: to compiled code directly, when the callee is an everyday function that's been compiled, else through
    /// `helpers::call`. For CallMethod (`method` is its cache), the method is read first. When the code and its caches
    /// say what's likely to be called (`known_callees`), a call of that is made with the frame made here (`call_known`),
    /// and anything else the way that works for any function, out of the way.
    fn call(&mut self, i: usize, dst: u16, f: u16, argc: u16, method: Method) {
        let l = self.l;
        let knowns = self.known_callees(i);
        // A call that knows nothing of what it calls (its place had seen nothing, say, when this was compiled) counts
        // down the function's count, at which the function is compiled again, and this goes on in the new code from
        // the start of this instruction (where nothing is taken from a register, as no call's template does).
        if let (true, Some(counter), None) = (knowns.is_empty(), self.counter, &self.inline)
            && matches!(
                self.ops[i],
                Op::Call { .. } | Op::CallPlain { .. } | Op::CallMethod { .. } | Op::CallMethodPoly { .. }
            )
        {
            self.unguessed.push(i as u32);
            let again = self.a.label();
            let back = self.a.label();
            self.a.mov_imm64(R10, counter);
            self.a.dec_m(m(R10, 0));
            self.a.jcc(E, again);
            self.a.bind(back);
            let was = self.a.is_cold();
            self.a.set_cold(true);
            self.a.bind(again);
            self.a.mov_rr(RCX, VM);
            self.a.mov_imm64(RDX, u64::from(self.pc(i)));
            self.a.mov_imm64(R8, counter);
            self.call_helper(helpers::recompile as *const () as usize);
            self.a.test_rr(RAX, RAX);
            self.a.jcc(E, back);
            self.a.jmp_r(RAX);
            self.a.set_cold(was);
        }
        // Which of them are compiled in line, as many as there's room for. A call compiled in line uses no stack and
        // makes nothing, so if they all are, it needs neither of the checks before a call: they're made on the way
        // to calling anything else.
        let mut room = INLINE_BUDGET.saturating_sub(self.inlined);
        let inline: Vec<bool> = knowns
            .iter()
            .map(|&known| {
                let length = self.inline_plan(known).map(|(_, ops, _)| ops.len());
                match length {
                    Some(length) if length <= room && self.can_inline(known) => {
                        room -= length;
                        true
                    }
                    _ => false,
                }
            })
            .collect();
        let all_inline = !knowns.is_empty() && inline.iter().all(|&inline| inline);
        if !all_inline {
            // Too deep in Rust's stack to call natively: the interpreter makes the call, which needs none.
            let bail = self.bail_stub(i);
            self.a.alu_rm(Alu::Cmp, RSP, m(VM, l.floor));
            self.a.jcc(B, bail);
        }
        match method {
            Method::Mono(cache) => {
                let read_slow = self.a.label();
                let read = self.a.label();
                self.a.load(RAX, self.reg(f + 1));
                self.get_prop(cache, f + 1, read_slow, RDX);
                self.a.store(self.reg(f), RDX);
                self.a.bind(read);
                self.cold_generic(i, read_slow, read);
            }
            Method::Poly(cache) => {
                let read_slow = self.a.label();
                let read = self.a.label();
                self.a.load(RAX, self.reg(f + 1));
                self.get_prop_poly(cache, read_slow, read, f);
                self.a.bind(read);
                self.cold_generic(i, read_slow, read);
            }
            Method::None => {}
        }
        if !all_inline {
            self.safe_point(self.pc(i) + 1);
        }
        let done = self.a.label();
        if knowns.is_empty() {
            self.call_any(i, dst, f, argc, done, false);
            self.a.bind(done);
            return;
        }
        let any = self.a.label();
        // A function calling itself by its own closure (as a recursive one does, by its name) is that function,
        // with this frame's captured variables: one compare.
        if knowns == [self.proto_id] && self.inline.is_none() {
            let other = self.a.label();
            self.a.load32(RDX, m(FRAME, l.f_closure));
            self.a.mov_imm64(R10, OBJECT_TAG);
            self.a.alu_rr(Alu::Or, R10, RDX);
            self.a.alu_rm(Alu::Cmp, R10, self.reg(f));
            self.a.jcc(NE, other);
            self.note_known_callee(self.proto_id);
            self.call_known(i, dst, f, argc, self.proto_id, any, true);
            self.a.jmp(done);
            self.a.bind(other);
        }
        // Which one it is, by its proto (in ecx, with its object in rax and its value in rdx).
        self.a.load(RAX, self.reg(f));
        self.a.mov_rr(RDX, RAX);
        self.object_address(RAX, any);
        self.a.cmp_m8(m(RAX, l.obj.kind as i32), l.obj.function_tag);
        self.a.jcc(NE, any);
        self.a.load32(RCX, m(RAX, (l.obj.kind + l.obj.function_proto) as i32));
        for (at, (&known, &in_line)) in knowns.iter().zip(inline.iter()).enumerate() {
            let next = self.a.label();
            self.a.alu_ri32(Alu::Cmp, RCX, known as i32);
            self.a.jcc(NE, if at + 1 == knowns.len() { any } else { next });
            self.note_known_callee(known);
            if in_line {
                let made = self.inline_call(i, dst, f, argc, known, any);
                debug_assert!(made, "a call that can be compiled in line is");
            } else {
                self.call_known(i, dst, f, argc, known, any, false);
            }
            self.a.jmp(done);
            self.a.bind(next);
        }
        self.a.bind(done);
        let was = self.a.is_cold();
        self.a.set_cold(true);
        self.a.bind(any);
        self.a.inc_m(m(VM, l.missed));
        self.call_any(i, dst, f, argc, done, all_inline);
        self.a.set_cold(was);
    }

    /// What the call at `i` is likely to call, as the job has it (see `Look::likely_callees`), of what has code (or is
    /// this function): a call of one that has none yet is made the way that works for any, and counts towards
    /// compiling this again (see `call`), as its code is likely to come once it's called often enough.
    fn known_callees(&self, i: usize) -> Vec<u32> {
        if self.inline.is_some() {
            return Vec::new();
        }
        let calls = &self.job.calls;
        let Ok(found) = calls.binary_search_by_key(&(i as u32), |&(at, _)| at) else {
            return Vec::new();
        };
        calls[found]
            .1
            .iter()
            .copied()
            .filter(|&known| known == self.proto_id || self.compiled.contains_key(&known))
            .collect()
    }

    /// Whether a call of proto `known` would be compiled in line (see `inline_call`).
    fn can_inline(&self, known: u32) -> bool {
        self.inline_plan(known)
            .is_some_and(|(_, ops, _)| self.compiled.get(&known).is_some_and(|code| code.offsets.len() == ops.len()))
    }

    /// A register of the frame being compiled for, as the job has it, if the job was taken in that frame (where it
    /// had just gone round a loop, say): what the last time round left there.
    fn live(&self, register: u16) -> Option<Value> {
        if self.inline.is_some() {
            return None;
        }
        self.job.live.get(usize::from(register)).copied()
    }

    /// A call of a function of proto `known` (see `known_callees`), which `call` has checked register `f` has (its
    /// object in rax, its value in rdx): makes its frame as its prologue would, with what's known of it, and calls its
    /// code where the body starts. Its code's address is known if it's been compiled (or is this function), else
    /// read from its slot, and a function that hasn't been is called the long way (`any`) until it is.
    ///
    /// With `own`, it's this function by this frame's closure (its handle in edx), whose captured variables are this
    /// frame's.
    #[allow(clippy::too_many_arguments)]
    fn call_known(&mut self, i: usize, dst: u16, f: u16, argc: u16, known: u32, any: Label, own: bool) {
        let l = self.l;
        let o = l.obj;
        // What's known of it: this function's proto, or what the job has of it.
        let (callee, slot) = if known == self.job.proto_id {
            (&self.job.proto, 0)
        } else {
            match self.callee(known) {
                Some(callee) => (&callee.proto, callee.slot),
                None => {
                    debug_assert!(false, "a known callee is in the job");
                    self.a.jmp(any);
                    return;
                }
            }
        };
        let registers = i32::from(callee.registers);
        let params = i32::from(callee.params);
        let clear = callee.clear() as i32;
        let (start, cache_base) = (callee.start, callee.cache_base);
        // Where its code's body is, when it's been compiled only later.
        let compiled_map: &'a HashMap<u32, Known, FixedState> = self.compiled;
        let code = compiled_map.get(&known).map(|code| code.address as u64).filter(|_| known != self.proto_id);
        let compiled = known != self.proto_id && (code.is_some() || slot >= CODE_MIN);
        if known != self.proto_id && !compiled {
            self.a.mov_imm64(R11, self.slots + 8 * u64::from(known));
            self.a.load(R11, m(R11, 0));
            self.a.alu_ri(Alu::Cmp, R11, CODE_MIN as i32);
            self.a.jcc(B, any);
            self.a.alu_ri(Alu::And, R11, -2);
            self.a.load(R11, m(R11, -(HEADER as i32)));
        }
        // Room for its frame and its registers, else the long way, which makes it.
        let new = |field: i32| m(FRAME, l.frame_size + field);
        self.a.load32(R8, m(FRAME, l.f_base));
        self.a.alu_ri32(Alu::Add, R8, i32::from(f) + 1);
        self.a.load(R10, m(VM, l.frames_len));
        self.a.alu_rm(Alu::Cmp, R10, m(VM, l.frames_committed));
        self.a.jcc(AE, any);
        self.a.lea32(R10, m(R8, registers + 16));
        self.a.alu_rm(Alu::Cmp, R10, m(VM, l.stack_len));
        self.a.jcc(A, any);
        // The frame, as `prologue` makes it.
        self.a.store32_imm(new(l.f_pc), start);
        self.a.store32(new(l.f_base), R8);
        self.a.alu_ri32(Alu::Add, R8, registers);
        self.a.store32(new(l.f_top), R8);
        self.a.store32_imm(new(l.f_proto), known);
        self.a.store32(new(l.f_closure), RDX);
        self.a.store32_imm(new(l.f_cache_base), cache_base);
        self.a.mov_imm64(R10, u64::from(dst));
        self.a.store16(new(l.f_ret_dst), R10);
        self.a.store8_imm(new(l.f_flags), 0);
        self.a.store(new(l.f_new_target), UNDEF);
        // Its captured variables start after their count, if it has any.
        if own {
            self.a.load(R8, m(FRAME, l.f_upvalues));
        } else {
            self.a.load(R8, m(RAX, (o.kind + o.function_upvalues) as i32));
            self.a.lea(R9, m(R8, 8));
            self.a.test_rr(R8, R8);
            self.a.cmov(NE, R8, R9);
        }
        self.a.store(new(l.f_upvalues), R8);
        self.a.inc_m(m(VM, l.frames_len));
        // The parameters with no argument, and its own variables, are undefined: its registers start at f + 1.
        for at in (1 + i32::from(argc)).min(1 + params)..clear {
            self.a.store(m(REGS, 8 * (i32::from(f) + 1 + at)), UNDEF);
        }
        self.a.store32_imm(m(FRAME, l.f_pc), self.pc(i) + 1);
        let shift = 8 * (i32::from(f) + 1);
        self.a.alu_ri(Alu::Add, REGS, shift);
        self.a.alu_ri(Alu::Add, FRAME, l.frame_size);
        if known == self.proto_id {
            self.a.call_label(self.pre_body);
        } else if compiled {
            // SAFETY: a compiled function's code starts with the header, whose first word is where this comes in.
            let pre_body = unsafe { (code.unwrap_or_else(|| (slot & !1) - HEADER) as *const u64).read() };
            self.a.call_abs(pre_body);
        } else {
            self.a.call_r(R11);
        }
        self.a.alu_ri(Alu::Sub, FRAME, l.frame_size);
        self.a.alu_ri(Alu::Sub, REGS, shift);
        self.a.mov_imm64(ICS, self.ics);
        self.a.test_rr32(RDX, RDX);
        self.a.jcc(NE, self.propagate);
        self.a.store(self.reg(dst), RAX);
    }

    /// Whether instruction `op` of a callee can be compiled in line (see `inline_call`): one whose template needs
    /// nothing of its frame but its registers (anything else goes to `deopt`), that doesn't call or loop, and that
    /// doesn't leave the function but by returning.
    fn inlinable(op: &Op) -> bool {
        matches!(
            op,
            Op::LoadConst { .. }
                | Op::LoadInt { .. }
                | Op::LoadUndefined { .. }
                | Op::LoadNull { .. }
                | Op::LoadTrue { .. }
                | Op::LoadFalse { .. }
                | Op::Move { .. }
                | Op::Add { .. }
                | Op::Sub { .. }
                | Op::Mul { .. }
                | Op::AddInt { .. }
                | Op::SubInt { .. }
                | Op::MulInt { .. }
                | Op::AddInt32 { .. }
                | Op::DivInt { .. }
                | Op::AddK { .. }
                | Op::SubK { .. }
                | Op::MulK { .. }
                | Op::Div { .. }
                | Op::DivK { .. }
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
                | Op::StrictEq { .. }
                | Op::StrictNe { .. }
                | Op::Not { .. }
                | Op::Inc { .. }
                | Op::Dec { .. }
                | Op::Neg { .. }
                | Op::SloppyThis
                | Op::Jump { .. }
                | Op::JumpIfTrue { .. }
                | Op::JumpIfFalse { .. }
                | Op::JumpIfNullish { .. }
                | Op::JumpIfNotNullish { .. }
                | Op::JumpIfUndefined { .. }
                | Op::JumpIfNotUndefined { .. }
                | Op::JumpIfNull { .. }
                | Op::JumpIfNotNull { .. }
                | Op::JumpLt { .. }
                | Op::JumpLe { .. }
                | Op::JumpGt { .. }
                | Op::JumpGe { .. }
                | Op::JumpNotLt { .. }
                | Op::JumpNotLe { .. }
                | Op::JumpNotGt { .. }
                | Op::JumpNotGe { .. }
                | Op::JumpLtInt { .. }
                | Op::JumpLeInt { .. }
                | Op::JumpGtInt { .. }
                | Op::JumpGeInt { .. }
                | Op::JumpNotLtInt { .. }
                | Op::JumpNotLeInt { .. }
                | Op::JumpNotGtInt { .. }
                | Op::JumpNotGeInt { .. }
                | Op::JumpStrictEq { .. }
                | Op::JumpStrictNe { .. }
                | Op::JumpStrictEqInt { .. }
                | Op::JumpStrictNeInt { .. }
                | Op::GetGlobal { .. }
                | Op::GetUp { .. }
                | Op::GetUpValue { .. }
                | Op::SetUp { .. }
                | Op::LoadCallee { .. }
                | Op::GetProp { .. }
                | Op::GetPropPoly { .. }
                | Op::SetProp { .. }
                | Op::SetPropPoly { .. }
                | Op::CallMath { .. }
                | Op::Return { .. }
                | Op::ReturnUndefined
        )
    }

    /// Whether a call of proto `known` could be compiled in line here (see `inline_call`), but for whether it has
    /// code yet: its instructions, and how far its caches are from this function's, in bytes.
    fn inline_plan(&self, known: u32) -> Option<(&'a Proto, &'a [Op], i32)> {
        if self.inline.is_some() || known == self.proto_id {
            return None;
        }
        let callee = self.callee(known)?;
        let ops: &'a [Op] = callee.ops.as_deref()?;
        if self.inlined + ops.len() > INLINE_BUDGET {
            return None;
        }
        let delta =
            (i64::from(callee.proto.cache_base) - i64::from(self.proto.cache_base)) * i64::from(self.l.cache_size);
        Some((&callee.proto, ops, i32::try_from(delta).ok()?))
    }

    /// A call of a short function of proto `known` (see `known_callees`) compiled in line, if it can be: one that's
    /// been compiled itself, whose every instruction is `inlinable`, and which jumps only forward. With register `f`
    /// checked to have that function (by `call`), makes its parameters and variables undefined as its prologue would,
    /// and runs its instructions here, on its registers where its frame would have them (from `f + 1`), with no frame
    /// made. Whatever one of them can't do in line (a helper that needs the frame, the generic way) makes the frame
    /// then, as the call would have, and goes on in the callee's own code at that instruction, which does it from the
    /// start (as every template only changes anything once it's taken its usual way), then returns here as a call.
    fn inline_call(&mut self, i: usize, dst: u16, f: u16, argc: u16, known: u32, any: Label) -> bool {
        let l = self.l;
        let o = l.obj;
        let Some((callee, ops, ics_delta)) = self.inline_plan(known) else {
            return false;
        };
        let length = ops.len();
        let compiled_map: &'a HashMap<u32, Known, FixedState> = self.compiled;
        let Some(compiled) = compiled_map.get(&known).filter(|code| code.offsets.len() == length) else {
            return false;
        };
        self.inlined += length;
        self.inline_calls += 1;
        self.inlines.push((known, i as u32));
        let registers = i32::from(callee.registers);
        let params = i32::from(callee.params);
        let clear = callee.clear() as i32;
        let base = i32::from(f) + 1;
        // (`call` has checked it's what's called.) Its registers are there: a frame has room for 16 past its own (see `Vm::enter_function`).
        let reach = base + registers;
        if reach > i32::from(self.proto.registers) + 16 {
            self.a.load32(R8, m(FRAME, l.f_base));
            self.a.lea32(R10, m(R8, reach + 16));
            self.a.alu_rm(Alu::Cmp, R10, m(VM, l.stack_len));
            self.a.jcc(A, any);
        }
        for at in (1 + i32::from(argc)).min(1 + params)..clear {
            self.a.store(m(REGS, 8 * (base + at)), UNDEF);
        }
        // Where its captured variables are, if it reads or writes them, from its closure (whose object `call` left in
        // rax), after their count, as its frame would have them.
        if ops
            .iter()
            .any(|op| matches!(op, Op::GetUp { .. } | Op::GetUpValue { .. } | Op::SetUp { .. }))
        {
            self.a.load(R8, m(RAX, (o.kind + o.function_upvalues) as i32));
            self.a.alu_ri(Alu::Add, R8, 8);
            self.a.store(m(RSP, SPARE), R8);
        }
        // Its instructions, as its own.
        let end = self.a.label();
        let deopts: Vec<Label> = (0..length).map(|_| self.a.label()).collect();
        let labels: Vec<Label> = (0..length).map(|_| self.a.label()).collect();
        let mut targets = vec![false; length];
        for (at, op) in ops.iter().enumerate() {
            if let Some(offset) = super::jump_offset(op)
                && let Some(place) = targets.get_mut(at + 1 + offset as usize)
            {
                *place = true;
            }
        }
        let saved = (self.proto_id, self.proto, self.ops, self.at, self.shift, self.ics_delta);
        let saved_labels = std::mem::replace(&mut self.labels, labels);
        let saved_targets = std::mem::replace(&mut self.targets, targets);
        self.proto_id = known;
        self.proto = callee;
        self.ops = ops;
        self.shift = base;
        self.ics_delta = ics_delta;
        self.inline = Some(Inlining {
            deopts: deopts.clone(),
            dst,
            end,
        });
        self.known = None;
        let mut ok = true;
        for k in 0..length {
            self.a.bind(self.labels[k]);
            ok &= self.op(k);
        }
        self.inline = None;
        self.known = None;
        (self.proto_id, self.proto, self.ops, self.at, self.shift, self.ics_delta) = saved;
        self.labels = saved_labels;
        self.targets = saved_targets;
        debug_assert!(ok, "every instruction that can be compiled in line can be compiled");
        // Out of the way: making its frame, and going on in its code at the instruction (which is in r11, with what
        // the frame's `pc` says in r10).
        let was = self.a.is_cold();
        self.a.set_cold(true);
        let common = self.a.label();
        let ics = self.bases.caches + u64::from(callee.cache_base) * size_of::<Cache>() as u64;
        for (k, &deopt) in deopts.iter().enumerate() {
            let enter = self.a.label();
            self.a.bind(deopt);
            self.a.mov_imm64(R10, u64::from(callee.start) + k as u64);
            self.a.lea_label(R11, enter);
            self.a.jmp(common);
            // As if called: its code takes what the instruction before left in a register from there.
            self.a.bind(enter);
            self.a.alu_ri(Alu::Sub, RSP, NATIVE_FRAME);
            self.a.mov_imm64(ICS, ics);
            if let Some((register, r)) = compiled.incoming[k] {
                self.a.load(r, m(REGS, 8 * i32::from(register)));
            }
            self.a.jmp_abs((compiled.address + compiled.offsets[k] as usize) as u64);
        }
        let check = self.a.label();
        let grow = self.a.label();
        let new = |field: i32| m(FRAME, l.frame_size + field);
        self.a.bind(common);
        self.a.inc_m(m(VM, l.left_line));
        self.a.bind(check);
        self.a.load(RAX, m(VM, l.frames_len));
        self.a.alu_rm(Alu::Cmp, RAX, m(VM, l.frames_committed));
        self.a.jcc(AE, grow);
        self.a.load(RDX, m(REGS, 8 * i32::from(f)));
        self.a.mov_rr32(RAX, RDX);
        self.a.shift_ri(Shift::Shl, RAX, 6);
        self.a.alu_rr(Alu::Add, RAX, OBJECTS);
        self.a.load32(R8, m(FRAME, l.f_base));
        self.a.alu_ri32(Alu::Add, R8, base);
        self.a.store32(new(l.f_pc), R10);
        self.a.store32(new(l.f_base), R8);
        self.a.alu_ri32(Alu::Add, R8, registers);
        self.a.store32(new(l.f_top), R8);
        self.a.store32_imm(new(l.f_proto), known);
        self.a.store32(new(l.f_closure), RDX);
        self.a.store32_imm(new(l.f_cache_base), callee.cache_base);
        self.a.mov_imm64(R9, u64::from(dst));
        self.a.store16(new(l.f_ret_dst), R9);
        self.a.store8_imm(new(l.f_flags), 0);
        self.a.store(new(l.f_new_target), UNDEF);
        self.a.load(R8, m(RAX, (o.kind + o.function_upvalues) as i32));
        self.a.lea(R9, m(R8, 8));
        self.a.test_rr(R8, R8);
        self.a.cmov(NE, R8, R9);
        self.a.store(new(l.f_upvalues), R8);
        self.a.inc_m(m(VM, l.frames_len));
        self.a.store32_imm(m(FRAME, l.f_pc), self.pc(i) + 1);
        self.a.alu_ri(Alu::Add, REGS, 8 * base);
        self.a.alu_ri(Alu::Add, FRAME, l.frame_size);
        self.a.call_r(R11);
        self.a.alu_ri(Alu::Sub, FRAME, l.frame_size);
        self.a.alu_ri(Alu::Sub, REGS, 8 * base);
        self.a.mov_imm64(ICS, self.ics);
        self.a.test_rr32(RDX, RDX);
        self.a.jcc(NE, self.propagate);
        self.a.store(m(REGS, 8 * i32::from(dst)), RAX);
        self.a.jmp(end);
        // No room for a frame: made as the prologue makes it, with this frame on top.
        self.a.bind(grow);
        self.a.push(R10);
        self.a.push(R11);
        self.a.alu_ri(Alu::Sub, RSP, 32);
        self.a.mov_rr(RCX, VM);
        self.a.load32(RDX, m(FRAME, l.f_base));
        self.a.alu_ri32(Alu::Add, RDX, reach + 16);
        self.call_helper(helpers::grow as *const () as usize);
        self.a.alu_ri(Alu::Add, RSP, 32);
        self.a.pop(R11);
        self.a.pop(R10);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NE, self.exit);
        self.a.jmp(check);
        self.a.set_cold(was);
        self.a.bind(end);
        true
    }

    /// A return of a callee compiled in line: what's in `value` goes to the caller's register for it, and the call
    /// goes on after.
    fn inline_return(&mut self, value: R) {
        let (dst, end) = match &self.inline {
            Some(inlining) => (inlining.dst, inlining.end),
            None => unreachable!("only a callee in line returns this way"),
        };
        self.a.store(m(REGS, 8 * i32::from(dst)), value);
        self.a.jmp(end);
    }

    /// A call of whatever register `f` has, then on to `done`: to compiled code directly, when it's an everyday
    /// function that's been compiled, a native by a helper that only calls it, else through `helpers::call`.
    /// With `checks`, out of the way after a call compiled in line has found something else: the checks before a call
    /// first, with a call too deep in Rust's stack made by `helpers::call` (after the method's read, the interpreter
    /// mustn't do the instruction again).
    fn call_any(&mut self, i: usize, dst: u16, f: u16, argc: u16, done: Label, checks: bool) {
        let l = self.l;
        let slow = self.a.label();
        if checks {
            self.a.alu_rm(Alu::Cmp, RSP, m(VM, l.floor));
            self.a.jcc(B, slow);
            let enough = self.a.label();
            self.a.load(RAX, m(VM, l.debt));
            self.a.alu_rm(Alu::Cmp, RAX, m(VM, l.threshold));
            self.a.jcc(BE, enough);
            self.a.mov_rr(RCX, VM);
            self.a.mov_imm64(RDX, u64::from(self.pc(i) + 1));
            self.call_helper(helpers::collect as *const () as usize);
            self.a.bind(enough);
        }
        self.a.load(RAX, self.reg(f));
        self.a.mov_rr(RDX, RAX);
        self.object_address(RAX, slow);
        let not_function = self.a.label();
        self.a.cmp_m8(m(RAX, l.obj.kind as i32), l.obj.function_tag);
        self.a.jcc(NE, not_function);
        self.a.mov_rr(R10, RAX);
        self.a.load32(RAX, m(R10, (l.obj.kind + l.obj.function_proto) as i32));
        self.note_callee();
        self.a.imul_rri(R11, RAX, l.proto_size);
        self.a.mov_imm64(R9, self.protos);
        self.a.alu_rr(Alu::Add, R11, R9);
        // A function that keeps its arguments does that in its prologue.
        self.a.test_m8(m(R11, l.p_flags), IS_CLASS);
        self.a.jcc(NE, slow);
        self.a.mov_imm64(R9, self.slots);
        self.a.load(R11, mi(R9, RAX, 3, 0));
        self.a.alu_ri(Alu::Cmp, R11, CODE_MIN as i32);
        self.a.jcc(B, slow);
        // Its captured variables start after their count, if it has any.
        self.a.load(R8, m(R10, (l.obj.kind + l.obj.function_upvalues) as i32));
        self.a.lea(R9, m(R8, 8));
        self.a.test_rr(R8, R8);
        self.a.cmov(NE, R8, R9);
        self.a.mov_rr32(RDX, RDX);
        self.a.load32(RAX, m(FRAME, l.f_base));
        self.a.alu_ri32(Alu::Add, RAX, i32::from(f) + 1);
        self.a.mov_imm64(RCX, u64::from(argc));
        self.a.mov_imm64(R9, u64::from(dst));
        self.a.store32_imm(m(FRAME, l.f_pc), self.pc(i) + 1);
        let shift = 8 * (i32::from(f) + 1);
        self.a.alu_ri(Alu::Add, REGS, shift);
        self.a.alu_ri(Alu::Add, FRAME, l.frame_size);
        self.a.call_r(R11);
        self.a.alu_ri(Alu::Sub, FRAME, l.frame_size);
        self.a.alu_ri(Alu::Sub, REGS, shift);
        self.a.mov_imm64(ICS, self.ics);
        self.a.test_rr32(RDX, RDX);
        self.a.jcc(NE, self.propagate);
        self.a.store(self.reg(dst), RAX);
        // In line, `done` comes next; out of the way, the rest of the cold code does.
        let was = self.a.is_cold();
        if was {
            self.a.jmp(done);
        }
        self.a.set_cold(true);
        // A native, by a helper that only calls it.
        self.a.bind(not_function);
        self.a.cmp_m8(m(RAX, l.obj.kind as i32), l.obj.native_tag);
        self.a.jcc(NE, slow);
        // Its function, the frame at this call, and the top of the stack where the native may use it, as
        // `call_function!` has them.
        self.a.load(R10, m(RAX, (l.obj.kind + l.obj.native_data) as i32));
        self.a.load(RDX, m(R10, l.n_func));
        self.a.store32_imm(m(FRAME, l.f_pc), self.pc(i) + 1);
        self.a.load32(RAX, m(FRAME, l.f_top));
        self.a.store(m(VM, l.top), RAX);
        self.a.load32(R8, m(FRAME, l.f_base));
        self.a.mov_imm64(
            R9,
            u64::from(f) | u64::from(argc) << 16 | u64::from(dst) << 32,
        );
        self.a.mov_rr(RCX, VM);
        self.call_helper(helpers::call_native as *const () as usize);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NE, self.exit);
        self.a.jmp(done);
        self.a.bind(slow);
        self.a.mov_rr(RCX, VM);
        self.a.mov_imm64(RDX, u64::from(self.pc(i)));
        self.call_helper(helpers::call as *const () as usize);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NE, self.exit);
        self.a.jmp(done);
        self.a.set_cold(was);
    }

    /// `Math.name(x)` or `Math.name(x, y)` (CallMath): worked out here while what's called is still the built-in
    /// function and it's given numbers, as `Vm::math_quick` does, with ints first where that's quick, a few in machine
    /// code, and the rest by `math_quick_apply`. Anything else is the call it stands for.
    fn call_math(&mut self, i: usize, dst: u16, f: u16, which: u16) {
        use crate::builtins::{math_quick_apply, math_quick_argc, math_quick_name, math_quick_native};
        let l = self.l;
        let o = l.obj;
        let argc = math_quick_argc(which);
        let name = math_quick_name(which);
        let call = self.a.label();
        let done = self.a.label();
        let doubles = self.a.label();
        let result = self.a.label();
        self.a.load(RAX, self.reg(f));
        self.object_address(RAX, call);
        self.a.cmp_m8(m(RAX, o.kind as i32), o.native_tag);
        self.a.jcc(NE, call);
        self.a.load(R10, m(RAX, (o.kind + o.native_data) as i32));
        self.a.load(R10, m(R10, l.n_func));
        self.a.mov_imm64(R11, math_quick_native(which) as usize as u64);
        self.a.alu_rr(Alu::Cmp, R10, R11);
        self.a.jcc(NE, call);
        self.a.load(RAX, self.reg(f + 2));
        if argc == 2 {
            self.a.load(RCX, self.reg(f + 3));
        }
        // Ints, where the answer is quick to tell.
        match name {
            "floor" | "ceil" | "trunc" | "round" => {
                self.guard_int(RAX, doubles);
                self.a.store(self.reg(dst), RAX);
                self.a.jmp(done);
            }
            "abs" => {
                self.guard_int(RAX, doubles);
                self.a.mov_rr32(RDX, RAX);
                self.a.neg32(RDX);
                // The most negative int's is a double.
                self.a.jcc(O, doubles);
                self.a.cmov(S, RDX, RAX);
                self.a.mov_rr32(RDX, RDX);
                self.a.alu_rr(Alu::Or, RDX, INT);
                self.a.store(self.reg(dst), RDX);
                self.a.jmp(done);
            }
            "min" | "max" | "imul" => {
                self.guard_ints(RAX, RCX, doubles);
                if name == "imul" {
                    self.a.imul_rr32(RAX, RCX);
                } else {
                    self.a.alu_rr32(Alu::Cmp, RAX, RCX);
                    self.a.cmov(if name == "min" { G } else { L }, RAX, RCX);
                }
                self.a.mov_rr32(RAX, RAX);
                self.a.alu_rr(Alu::Or, RAX, INT);
                self.a.store(self.reg(dst), RAX);
                self.a.jmp(done);
            }
            _ => {}
        }
        // Doubles (and ints, as doubles).
        self.a.bind(doubles);
        self.to_double(RAX, XMM0, call);
        if argc == 2 {
            self.to_double(RCX, XMM1, call);
        }
        let round_mode = match name {
            "floor" => Some(9),
            "ceil" => Some(10),
            "trunc" => Some(11),
            _ => None,
        };
        match name {
            "sqrt" => self.a.sd(Sd::Sqrt, XMM0, XMM0),
            "abs" => {
                self.a.movq_rx(RAX, XMM0);
                self.a.mov_imm64(R10, !(1u64 << 63));
                self.a.alu_rr(Alu::And, RAX, R10);
                self.a.movq_xr(XMM0, RAX);
            }
            "fround" => {
                self.a.cvtsd2ss(XMM0, XMM0);
                self.a.cvtss2sd(XMM0, XMM0);
            }
            _ if round_mode.is_some() && self.sse41 => self.a.roundsd(XMM0, XMM0, round_mode.unwrap_or(0)),
            "min" | "max" => {
                // NaN if either is; of two zeros, -0 is the smaller; else the smaller or larger.
                let nan = self.a.label();
                let differ = self.a.label();
                self.a.ucomisd(XMM0, XMM1);
                self.a.jcc(P, nan);
                self.a.jcc(NE, differ);
                self.a.movq_rx(RAX, XMM0);
                self.a.movq_rx(RCX, XMM1);
                self.a.alu_rr(if name == "min" { Alu::Or } else { Alu::And }, RAX, RCX);
                self.a.movq_xr(XMM0, RAX);
                self.a.jmp(result);
                self.a.bind(differ);
                self.a.sd(if name == "min" { Sd::Min } else { Sd::Max }, XMM0, XMM1);
                self.a.jmp(result);
                self.a.bind(nan);
                self.a.mov_imm64(RAX, CANONICAL_NAN);
                self.a.movq_xr(XMM0, RAX);
            }
            _ => {
                // `math_quick_apply(which, x, y)`, with the numbers where the calling convention has them.
                if argc == 2 {
                    self.a.movapd(XMM2, XMM1);
                }
                self.a.movapd(XMM1, XMM0);
                self.a.mov_imm64(RCX, u64::from(which));
                self.call_helper(math_quick_apply as *const () as usize);
            }
        }
        self.a.bind(result);
        self.store_number(dst, XMM0);
        self.a.jmp(done);
        // What it calls isn't the built-in function, or not with numbers: the call, which the helper makes a Call
        // from now on for the interpreter.
        self.a.bind(call);
        match self.deopt() {
            Some(deopt) => self.a.jmp(deopt),
            None => self.call(i, dst, f, argc, Method::None),
        }
        self.a.bind(done);
    }

    /// A `new`, or with `parent` a `super(...)` (of the constructor this frame is for, whose `new.target` it hands on):
    /// to compiled code directly, when what's called is a constructor that's been compiled, with its `this` made here
    /// (a derived class's is made by its own `super(...)`), else through `helpers::call`.
    fn construct(&mut self, i: usize, dst: u16, f: u16, argc: u16, parent: bool) {
        let l = self.l;
        let slow = self.a.label();
        let done = self.a.label();
        let bail = self.bail_stub(i);
        self.safe_point(self.pc(i) + 1);
        self.a.alu_rm(Alu::Cmp, RSP, m(VM, l.floor));
        self.a.jcc(B, bail);
        if parent {
            // What `super` means: the prototype of the function this frame is for, as `super_constructor` finds it.
            self.a.load32(RAX, m(FRAME, l.f_closure));
            self.a.shift_ri(Shift::Shl, RAX, 6);
            self.a.alu_rr(Alu::Add, RAX, OBJECTS);
            self.proto_of(RAX, RCX, R11);
            self.a.alu_ri32(Alu::Cmp, RCX, -1);
            self.a.jcc(E, slow);
            self.a.mov_imm64(RDX, OBJECT_TAG);
            self.a.alu_rr(Alu::Or, RDX, RCX);
            self.a.store(self.reg(f), RDX);
        }
        // The callee's proto in r11, its object in r10; on to `slow` if it isn't what's done here.
        let callee = |g: &mut Self, slow: Option<Label>| {
            g.a.load(RAX, g.reg(f));
            match slow {
                Some(slow) => g.object_address(RAX, slow),
                None => {
                    g.a.mov_rr32(RAX, RAX);
                    g.a.shift_ri(Shift::Shl, RAX, 6);
                    g.a.alu_rr(Alu::Add, RAX, OBJECTS);
                }
            }
            if let Some(slow) = slow {
                g.a.cmp_m8(m(RAX, l.obj.kind as i32), l.obj.function_tag);
                g.a.jcc(NE, slow);
            }
            g.a.mov_rr(R10, RAX);
            g.a.load32(RAX, m(R10, (l.obj.kind + l.obj.function_proto) as i32));
            g.a.imul_rri(R11, RAX, l.proto_size);
            g.a.mov_imm64(R9, g.protos);
            g.a.alu_rr(Alu::Add, R11, R9);
        };
        callee(self, Some(slow));
        self.a.test_m8(m(R11, l.p_flags), IS_CONSTRUCTOR);
        self.a.jcc(E, slow);
        // A derived class's `this` is made by its `super(...)`.
        let derived = self.a.label();
        let made = self.a.label();
        self.a.test_m8(m(R11, l.p_flags), IS_DERIVED);
        self.a.jcc(NE, derived);
        let compiled = |g: &mut Self| {
            g.a.mov_imm64(R9, g.slots);
            g.a.load(R11, mi(R9, RAX, 3, 0));
            g.a.alu_ri(Alu::Cmp, R11, CODE_MIN as i32);
            g.a.jcc(B, slow);
            g.a.alu_ri(Alu::And, R11, -2);
            g.a.alu_mi(Alu::Cmp, m(R11, -8), 0);
            g.a.jcc(E, slow);
        };
        compiled(self);
        // It's one for here: make its `this`, then find it all again, as the helper used every register.
        self.a.mov_rr(RCX, VM);
        self.a.mov_imm64(RDX, u64::from(self.pc(i)));
        self.call_helper(helpers::this_for as *const () as usize);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NE, self.exit);
        self.a.bind(made);
        {
            let was = self.a.is_cold();
            self.a.set_cold(true);
            self.a.bind(derived);
            compiled(self);
            self.a.mov_imm64(RAX, EMPTY);
            self.a.store(self.reg(f + 1), RAX);
            self.a.jmp(made);
            self.a.set_cold(was);
        }
        callee(self, None);
        self.a.mov_imm64(R9, self.slots);
        self.a.load(R11, mi(R9, RAX, 3, 0));
        self.a.alu_ri(Alu::And, R11, -2);
        self.a.load(R11, m(R11, -8));
        self.a.load(R8, m(R10, (l.obj.kind + l.obj.function_upvalues) as i32));
        self.a.lea(R9, m(R8, 8));
        self.a.test_rr(R8, R8);
        self.a.cmov(NE, R8, R9);
        self.a.load(R10, self.reg(f));
        self.a.mov_rr32(RDX, R10);
        if parent {
            self.a.load(R10, m(FRAME, l.f_new_target));
        }
        self.a.load32(RAX, m(FRAME, l.f_base));
        self.a.alu_ri32(Alu::Add, RAX, i32::from(f) + 1);
        self.a.mov_imm64(RCX, u64::from(argc));
        self.a.mov_imm64(R9, u64::from(dst));
        self.a.store32_imm(m(FRAME, l.f_pc), self.pc(i) + 1);
        let shift = 8 * (i32::from(f) + 1);
        self.a.alu_ri(Alu::Add, REGS, shift);
        self.a.alu_ri(Alu::Add, FRAME, l.frame_size);
        self.a.call_r(R11);
        self.a.alu_ri(Alu::Sub, FRAME, l.frame_size);
        self.a.alu_ri(Alu::Sub, REGS, shift);
        self.a.mov_imm64(ICS, self.ics);
        self.a.test_rr32(RDX, RDX);
        self.a.jcc(NE, self.propagate);
        self.a.store(self.reg(dst), RAX);
        self.a.bind(done);
        self.a.set_cold(true);
        self.a.bind(slow);
        self.a.mov_rr(RCX, VM);
        self.a.mov_imm64(RDX, u64::from(self.pc(i)));
        self.call_helper(helpers::call as *const () as usize);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NE, self.exit);
        self.a.jmp(done);
        self.a.set_cold(false);
    }

    /// A generator stopping at instruction `i` by `helper`, which takes its frame away and leaves what's to be handed
    /// back in `vm.jit.value`: this returns it, as `ret` does.
    fn stop_here(&mut self, i: usize, helper: extern "C" fn(&mut Vm, u32) -> u32) {
        self.direct(i, helper);
        self.a.load(RAX, m(VM, self.l.value));
        self.a.alu_rr32(Alu::Xor, RDX, RDX);
        self.a.alu_ri(Alu::Add, RSP, NATIVE_FRAME);
        self.a.ret();
    }

    /// The prototype's handle of the object at `object` (`Vm::proto_of`: its shape's), or `NONE`, in `out`.
    fn proto_of(&mut self, object: R, out: R, scratch: R) {
        let l = self.l;
        self.a.load32(out, m(object, l.obj.shape as i32));
        self.a.imul_rri(out, out, l.shape_size);
        self.a.load(scratch, m(VM, l.shapes));
        self.a.load32(out, mi(scratch, out, 0, l.shape_proto));
    }

    /// `a instanceof b`, when `b` is a function with the usual `Symbol.hasInstance` and a `prototype` where
    /// `Vm::instance_of_prototype` remembers them: whether that's on `a`'s chain of prototypes, as `inherits_from`
    /// finds. Anything else by the helper that does only that.
    fn instance_of(&mut self, i: usize, dst: u16, a: u16, b: u16) {
        let l = self.l;
        let o = l.obj;
        let slow = self.a.label();
        let done = self.a.label();
        let yes = self.a.label();
        let no = self.a.label();
        let own = self.a.label();
        let walk = self.a.label();
        let cached = |field: i32| m(R8, l.instance_of + field);
        self.a.load(RCX, self.reg(b));
        self.object_address(RCX, slow);
        self.a.load32(RDX, m(RCX, o.shape as i32));
        self.a.mov_rr32(R8, RDX);
        self.a.alu_ri32(Alu::And, R8, 7);
        self.a.imul_rri(R8, R8, l.io_size);
        self.a.alu_rr(Alu::Add, R8, VM);
        self.a.alu_rm32(Alu::Cmp, RDX, cached(l.io_shape));
        self.a.jcc(NE, slow);
        self.a.load32(R9, m(VM, l.epoch));
        self.a.alu_rm32(Alu::Cmp, R9, cached(l.io_epoch));
        self.a.jcc(NE, slow);
        self.a.test_m8(m(RCX, o.flags as i32), crate::object::LAZY_FUNCTION);
        self.a.jcc(NE, slow);
        self.a.cmp_m8(m(RCX, o.kind as i32), o.function_tag);
        self.a.jcc(NE, slow);
        // Its `Symbol.hasInstance` is the usual one.
        self.a.mov_rr(R11, RCX);
        self.a.load32(R9, cached(l.io_holder));
        self.a.alu_ri32(Alu::Cmp, R9, -1);
        self.a.jcc(E, own);
        self.a.mov_rr32(R11, R9);
        self.a.shift_ri(Shift::Shl, R11, 6);
        self.a.alu_rr(Alu::Add, R11, OBJECTS);
        self.a.bind(own);
        self.a.load32(RDX, cached(l.io_has_slot));
        self.read_slot(R11, RDX, RAX);
        self.a.alu_rm(Alu::Cmp, RAX, m(VM, l.has_instance));
        self.a.jcc(NE, slow);
        // Its `prototype`, which has to be an object.
        self.a.load32(RDX, cached(l.io_prototype_slot));
        self.read_slot(RCX, RDX, RAX);
        self.a.mov_rr(R10, RAX);
        self.a.shift_ri(Shift::Shr, R10, 48);
        self.a.alu_ri32(Alu::Cmp, R10, TAG_OBJECT);
        self.a.jcc(NE, slow);
        self.a.mov_rr32(R9, RAX);
        // Up `a`'s prototypes, if it's an object.
        self.a.load(RAX, self.reg(a));
        self.a.mov_rr(R10, RAX);
        self.a.shift_ri(Shift::Shr, R10, 48);
        self.a.alu_ri32(Alu::Cmp, R10, TAG_OBJECT);
        self.a.jcc(NE, no);
        self.a.mov_rr32(RAX, RAX);
        self.a.shift_ri(Shift::Shl, RAX, 6);
        self.a.alu_rr(Alu::Add, RAX, OBJECTS);
        self.a.bind(walk);
        self.proto_of(RAX, RCX, R11);
        self.a.alu_ri32(Alu::Cmp, RCX, -1);
        self.a.jcc(E, no);
        self.a.alu_rr32(Alu::Cmp, RCX, R9);
        self.a.jcc(E, yes);
        self.a.mov_rr32(RAX, RCX);
        self.a.shift_ri(Shift::Shl, RAX, 6);
        self.a.alu_rr(Alu::Add, RAX, OBJECTS);
        self.a.jmp(walk);
        self.a.bind(yes);
        self.store_imm(dst, TRUE);
        self.a.jmp(done);
        self.a.bind(no);
        self.store_imm(dst, FALSE);
        self.a.bind(done);
        self.cold_direct(i, slow, done, helpers::instance_of);
    }

    /// Returns what's in `rax`, as `finish_return!` does: the frame goes, and the caller gets it.
    fn ret(&mut self, i: usize) {
        let l = self.l;
        let special = self.a.label();
        let finish = self.a.label();
        let helper = self.a.label();
        // A constructor (a class's, or a function that adds to its `this`) returns from a `new` the most: that's in
        // line for one.
        let constructs = self.proto.flags & IS_CLASS != 0
            || self.ops.iter().any(|op| matches!(*op, Op::SetProp { object: 0, cache, .. } if self.adds(cache)));
        if constructs {
            let plain = self.a.label();
            self.a.cmp_m8(m(FRAME, l.f_flags), crate::vm::CONSTRUCTING);
            self.a.jcc(NE, plain);
            self.a.mov_rr(R10, RAX);
            self.a.shift_ri(Shift::Shr, R10, 48);
            self.a.alu_ri32(Alu::Cmp, R10, TAG_OBJECT);
            self.a.jcc(E, finish);
            self.a.load(RAX, m(REGS, 0));
            self.a.mov_imm64(R10, EMPTY);
            self.a.alu_rr(Alu::Cmp, RAX, R10);
            self.a.jcc(E, helper);
            self.a.jmp(finish);
            let was = self.a.is_cold();
            self.a.set_cold(true);
            self.a.bind(plain);
            self.a.cmp_m8(m(FRAME, l.f_flags), 0);
            self.a.jcc(NE, helper);
            self.a.jmp(finish);
            self.a.set_cold(was);
        } else {
            self.a.cmp_m8(m(FRAME, l.f_flags), 0);
            self.a.jcc(NE, special);
        }
        self.a.bind(finish);
        self.a.dec_m(m(VM, l.frames_len));
        self.a.alu_rr32(Alu::Xor, RDX, RDX);
        self.a.alu_ri(Alu::Add, RSP, NATIVE_FRAME);
        self.a.ret();
        self.a.set_cold(true);
        // A `new` hands back what was returned if it's an object, else its `this`, which a derived class's has to
        // have made by now.
        self.a.bind(special);
        self.a.cmp_m8(m(FRAME, l.f_flags), crate::vm::CONSTRUCTING);
        self.a.jcc(NE, helper);
        self.a.mov_rr(R10, RAX);
        self.a.shift_ri(Shift::Shr, R10, 48);
        self.a.alu_ri32(Alu::Cmp, R10, TAG_OBJECT);
        self.a.jcc(E, finish);
        self.a.load(RAX, m(REGS, 0));
        self.a.mov_imm64(R10, EMPTY);
        self.a.alu_rr(Alu::Cmp, RAX, R10);
        self.a.jcc(NE, finish);
        self.a.bind(helper);
        // What it throws is thrown at this instruction, as far as a `try` around it is concerned.
        self.a.store32_imm(m(FRAME, l.f_pc), self.pc(i) + 1);
        self.a.mov_rr(RCX, VM);
        self.a.mov_rr(RDX, RAX);
        self.call_helper(helpers::finish_return as *const () as usize);
        self.a.test_rr32(RAX, RAX);
        self.a.jcc(NE, self.exit);
        self.a.load(RAX, m(VM, l.value));
        self.a.jmp(finish);
        self.a.set_cold(false);
    }
}

/// Recording feedback (see `jit::feedback`): what each template writes into its instruction's record, with what it
/// does anyway. A callee compiled in line records nothing; where it can't go on in line, its own code does.
#[cfg(feature = "jit-opt")]
impl Gen<'_> {
    /// Where the instruction being compiled has its record, if it has one.
    fn site(&self) -> Option<u64> {
        if self.inline.is_some() || self.fb_block == 0 {
            return None;
        }
        let at = *self.fb_offsets.get(self.at)?;
        (at != super::feedback::NONE).then(|| self.fb_block + u64::from(at))
    }

    /// Emits `body` out of the way, in the cold code: with a jump past it if that's where the template is now, as it
    /// would otherwise run on into it.
    fn out_of_line(&mut self, body: impl FnOnce(&mut Self)) {
        let was = self.a.is_cold();
        let over = self.a.label();
        if was {
            self.a.jmp(over);
        }
        self.a.set_cold(true);
        body(self);
        if was {
            self.a.bind(over);
        }
        self.a.set_cold(was);
    }

    /// The instruction's record's kind, if it has one here.
    fn site_kind(&self) -> Option<super::feedback::Kind> {
        self.site()?;
        super::feedback::kind_of(&self.ops[self.at])
    }

    /// Says a number record's instruction met what its bits say, here, if it has one: the only bit is `DOUBLE`, whose
    /// byte of the record is the code's alone, so that it's a plain store, which no later run waits for, where an `or`
    /// would have each run of a loop wait for the last one's through memory.
    fn note(&mut self, bits: u8) {
        use super::feedback::Kind;
        debug_assert_eq!(bits, DOUBLE);
        if matches!(self.site_kind(), Some(Kind::Numbers | Kind::CompareBranch)) {
            let site = self.site().expect("there's a record");
            self.a.rip_store8_imm(site + super::feedback::CODE_DOUBLE as u64, 1);
        }
    }

    /// Before calling `helpers::typed_set`, which isn't told which instruction it's for: says where the record is, in
    /// `Jit::site` (the other helpers find it from the frame, `feedback::site_of`). Uses rax.
    fn set_site(&mut self) {
        if self.site_kind() != Some(super::feedback::Kind::Elem) {
            return;
        }
        let site = self.site().expect("there's a record");
        self.a.rip_lea(RAX, site);
        self.a.store(m(VM, self.l.site), RAX);
    }

    /// Takes one off the function's count to being due, at its entry or a loop's jump back, and says it's come to 0
    /// out of the way (`helpers::opt_due`).
    fn count(&mut self) {
        if self.fb_block == 0 || self.inline.is_some() {
            return;
        }
        let due = self.a.label();
        let back = self.a.label();
        self.a.rip_sub32_imm8(self.fb_block, 1);
        self.a.jcc(E, due);
        self.a.bind(back);
        let routine = self.fb_due;
        self.out_of_line(|g| {
            g.a.bind(due);
            g.a.call_abs(routine);
            g.a.jmp(back);
        });
    }

    /// Records `r9d` in the call record at `site`, its first word: here if it says that already, else by the writer's
    /// routine, out of the way, which is given the word in r11 (loaded here, which is smaller than loading it out of
    /// the way: there's one of these at most calls). Uses r11.
    fn note_word(&mut self, site: u64) {
        let other = self.a.label();
        let back = self.a.label();
        self.a.rip_lea(R11, site);
        self.a.alu_rm32(Alu::Cmp, R9, m(R11, 0));
        self.a.jcc(NE, other);
        self.a.bind(back);
        let routine = self.fb_note;
        self.out_of_line(|g| {
            g.a.bind(other);
            g.a.call_abs(routine);
            g.a.jmp(back);
        });
    }

    /// Where a branch's jump to `to` goes: by way of a stub that records it was taken, if it has a record (see
    /// `branch_went`).
    fn branch_target(&mut self, to: Label) -> Label {
        use super::feedback::Kind;
        if !matches!(self.site_kind(), Some(Kind::Branch | Kind::CompareBranch)) {
            return to;
        }
        if let Some((stub, real)) = self.taken
            && real == to
        {
            return stub;
        }
        let stub = self.a.label();
        self.taken = Some((stub, to));
        stub
    }

    /// After a branch's template: records that it went on to the next instruction, which is where its template
    /// ends up when it doesn't jump, and out of the way, that it jumped.
    fn branch_went(&mut self) {
        let Some((stub, to)) = self.taken.take() else {
            return;
        };
        let Some(site) = self.site() else {
            return;
        };
        // Its two flags (a store each, as `note` says).
        let ways = site + if self.site_kind() == Some(super::feedback::Kind::CompareBranch) { 2 } else { 0 };
        self.a.set_cold(false);
        self.a.rip_store8_imm(ways + super::feedback::FELL_AT as u64, 1);
        self.a.set_cold(true);
        self.a.bind(stub);
        self.a.rip_store8_imm(ways + super::feedback::TAKEN_AT as u64, 1);
        self.a.jmp(to);
        self.a.set_cold(false);
    }

    /// A call about to call `known`, which it's found it has: sets that callee's flag in its record (see
    /// `feedback::KNOWN_AT`), the one the writer gave it at this call, or a new one; past eight, it's recorded as
    /// `call_any` records one. Uses r9 and r11.
    fn note_known_callee(&mut self, known: u32) {
        let Some(site) = self.site() else {
            return;
        };
        let callees = self.fb_knowns.entry(self.at as u32).or_default();
        let bit = match callees.iter().position(|&callee| callee == known) {
            Some(bit) => Some(bit),
            None if callees.len() < 8 => {
                callees.push(known);
                Some(callees.len() - 1)
            }
            None => None,
        };
        match bit {
            Some(bit) => self.a.rip_store8_imm(site + (super::feedback::KNOWN_AT + bit) as u64, 1),
            None => {
                self.a.mov_imm64(R9, u64::from(known + 1));
                self.note_word(site);
            }
        }
    }

    /// A call about to call a function of proto `eax` (in `call_any`): records it in its record's first word, which
    /// says one callee or more. Uses r9 and r11.
    fn note_callee(&mut self) {
        let Some(site) = self.site() else {
            return;
        };
        self.a.lea32(R9, m(RAX, 1));
        self.note_word(site);
    }

    /// A typed array's element, of the kind whose byte is in `kind`: records that, in an element's record.
    fn note_typed(&mut self, kind: R) {
        if self.site_kind() != Some(super::feedback::Kind::Elem) {
            return;
        }
        let site = self.site().expect("there's a record");
        self.a.rip_store8_imm(site + super::feedback::TYPED_AT as u64, 1);
        self.a.rip_store8(site + 1, kind);
    }
}

/// Without feedback, the templates' hooks for it do nothing.
#[cfg(not(feature = "jit-opt"))]
impl Gen<'_> {
    #[inline(always)]
    fn note(&mut self, _bits: u8) {}
    #[inline(always)]
    fn set_site(&mut self) {}
    #[inline(always)]
    fn count(&mut self) {}
    #[inline(always)]
    fn note_known_callee(&mut self, _known: u32) {}
    #[inline(always)]
    fn note_callee(&mut self) {}
    #[inline(always)]
    fn note_typed(&mut self, _kind: R) {}
}

/// The bit of a number record the templates set (0 without feedback, as nothing uses it).
#[cfg(feature = "jit-opt")]
use super::feedback::DOUBLE;
#[cfg(not(feature = "jit-opt"))]
const DOUBLE: u8 = 0;

/// The register an instruction writes, if it writes one, as far as guessing what a call calls needs to know (see
/// `Gen::guess`).
fn written(op: &Op) -> Option<u16> {
    match *op {
        Op::LoadConst { dst, .. }
        | Op::LoadInt { dst, .. }
        | Op::LoadUndefined { dst, .. }
        | Op::LoadNull { dst, .. }
        | Op::LoadTrue { dst, .. }
        | Op::LoadFalse { dst, .. }
        | Op::LoadEmpty { dst, .. }
        | Op::Move { dst, .. }
        | Op::Add { dst, .. }
        | Op::AddInt { dst, .. }
        | Op::AddInt32 { dst, .. }
        | Op::SubInt { dst, .. }
        | Op::MulInt { dst, .. }
        | Op::ModInt { dst, .. }
        | Op::DivInt { dst, .. }
        | Op::BitAndInt { dst, .. }
        | Op::BitOrInt { dst, .. }
        | Op::ShlInt { dst, .. }
        | Op::ShrInt { dst, .. }
        | Op::UShrInt { dst, .. }
        | Op::AddK { dst, .. }
        | Op::SubK { dst, .. }
        | Op::MulK { dst, .. }
        | Op::DivK { dst, .. }
        | Op::ModK { dst, .. }
        | Op::Sub { dst, .. }
        | Op::Mul { dst, .. }
        | Op::Div { dst, .. }
        | Op::Mod { dst, .. }
        | Op::Pow { dst, .. }
        | Op::BitAnd { dst, .. }
        | Op::BitOr { dst, .. }
        | Op::BitXor { dst, .. }
        | Op::Shl { dst, .. }
        | Op::Shr { dst, .. }
        | Op::UShr { dst, .. }
        | Op::Lt { dst, .. }
        | Op::Le { dst, .. }
        | Op::Gt { dst, .. }
        | Op::Ge { dst, .. }
        | Op::Eq { dst, .. }
        | Op::Ne { dst, .. }
        | Op::StrictEq { dst, .. }
        | Op::StrictNe { dst, .. }
        | Op::In { dst, .. }
        | Op::InstanceOf { dst, .. }
        | Op::Neg { dst, .. }
        | Op::ToNumber { dst, .. }
        | Op::ToNumeric { dst, .. }
        | Op::Not { dst, .. }
        | Op::BitNot { dst, .. }
        | Op::TypeOf { dst, .. }
        | Op::Inc { dst, .. }
        | Op::Dec { dst, .. }
        | Op::ToKey { dst, .. }
        | Op::Concat { dst, .. }
        | Op::GetGlobal { dst, .. }
        | Op::TypeOfGlobal { dst, .. }
        | Op::NewCell { dst, .. }
        | Op::GetCell { dst, .. }
        | Op::GetUp { dst, .. }
        | Op::GetUpValue { dst, .. }
        | Op::Closure { dst, .. }
        | Op::LoadCallee { dst, .. }
        | Op::NewObject { dst, .. }
        | Op::NewArray { dst, .. }
        | Op::GetProp { dst, .. }
        | Op::GetPropPoly { dst, .. }
        | Op::GetLength { dst, .. }
        | Op::GetElem { dst, .. }
        | Op::Delete { dst, .. }
        | Op::ObjectRest { dst, .. }
        | Op::Call { dst, .. }
        | Op::CallMethod { dst, .. }
        | Op::CallMethodPoly { dst, .. }
        | Op::CallMath { dst, .. }
        | Op::CallPlain { dst, .. }
        | Op::New { dst, .. }
        | Op::CallSpread { dst, .. }
        | Op::NewSpread { dst, .. }
        | Op::SuperCall { dst, .. }
        | Op::SuperCallSpread { dst, .. }
        | Op::AsyncIterCall { dst, .. }
        | Op::DelegateNext { dst, .. }
        | Op::ForInKeys { dst, .. }
        | Op::CreateArguments { dst, .. }
        | Op::Rest { dst, .. }
        | Op::NewTarget { dst, .. }
        | Op::NewClass { dst, .. }
        | Op::SuperGet { dst, .. }
        | Op::ToObject { dst, .. }
        | Op::Yield { dst, .. }
        | Op::Await { dst, .. } => Some(dst),
        Op::CopyCell { reg } => Some(reg),
        Op::IterNext { value, .. } => Some(value),
        _ => None,
    }
}


fn int(value: i16) -> Src {
    Src::Imm(Value::int(value.into()))
}
