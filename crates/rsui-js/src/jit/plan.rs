//! What compiling a function needs, whatever the machine: the job taken from the engine (its instructions, what its
//! caches and frame know), what the engine knows that guides it (`Look`), where things are in the engine's own
//! structures (`Layout`), and what comes of it (`Compiled`, `Done`). The backends (`x64.rs`, `arm64.rs`) turn a job
//! into machine code; see `JIT.md`.
#![allow(unsafe_code)]

use std::mem::offset_of;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use crate::arena::Arena;
use crate::object::ObjectLayout;
use crate::op::{Cache, IS_CLASS, KEEPS_ARGS, Op, Proto};
use crate::value::Value;
use crate::vm::{Frame, Vm};

pub(crate) const INT_TAG: u64 = 0xFFF9 << 48;
pub(crate) const OBJECT_TAG: u64 = 0xFFFA << 48;
pub(crate) const TAG_INT: i32 = 0xFFF9;
pub(crate) const TAG_OBJECT: i32 = 0xFFFA;
pub(crate) const TAG_STRING: i32 = 0xFFFB;
pub(crate) const UNDEFINED: u64 = Value::UNDEFINED.0;
pub(crate) const NULL: u64 = Value::NULL.0;
pub(crate) const FALSE: u64 = Value::FALSE.0;
pub(crate) const TRUE: u64 = Value::TRUE.0;
pub(crate) const EMPTY: u64 = Value::EMPTY.0;
pub(crate) const CANONICAL_NAN: u64 = 0x7FF8 << 48;

/// A proto's `jit` slot from here up is the address of its code. Below it, it counts down to compiling.
pub(crate) const CODE_MIN: u64 = 1 << 20;

/// Each function's code starts with four words before its entry: where a call from compiled code that has made the
/// frame itself comes in (`entry - 32`, see `Gen::call_known`), whether it's worth going into from the interpreter or
/// Rust (`entry - 24`, which its slot says too), where its body is (`entry - 16`), and where a `new` from compiled
/// code comes in, or 0 (`entry - 8`). The entry is a one-byte `nop`, so that the slot can be one past it (see
/// `Jit::slots`).
pub(crate) const HEADER: u64 = 32;

/// Where things are, in the engine's own structures, for the code to read.
pub(crate) struct Layout {
    pub(crate) obj: ObjectLayout,
    pub(crate) frames_len: i32,
    pub(crate) frames_committed: i32,
    pub(crate) stack_len: i32,
    pub(crate) debt: i32,
    pub(crate) tracking: i32,
    pub(crate) threshold: i32,
    pub(crate) epoch: i32,
    pub(crate) global: i32,
    pub(crate) floor: i32,
    /// `Stats::left_line` and `Stats::missed`, which compiled code counts.
    pub(crate) left_line: i32,
    pub(crate) missed: i32,
    pub(crate) value: i32,
    /// `Vm::stack_base` and `Vm::stack_limit`, which the trampoline works out the floor from.
    pub(crate) stack_base: i32,
    pub(crate) stack_limit: i32,
    pub(crate) frame_size: i32,
    pub(crate) f_pc: i32,
    pub(crate) f_base: i32,
    pub(crate) f_top: i32,
    pub(crate) f_proto: i32,
    pub(crate) f_closure: i32,
    pub(crate) f_cache_base: i32,
    pub(crate) f_ret_dst: i32,
    pub(crate) f_flags: i32,
    pub(crate) f_new_target: i32,
    pub(crate) f_upvalues: i32,
    pub(crate) proto_size: i32,
    pub(crate) p_flags: i32,
    pub(crate) cache_size: i32,
    pub(crate) c_shape: i32,
    pub(crate) c_other: i32,
    pub(crate) c_slot: i32,
    pub(crate) c_epoch: i32,
    pub(crate) c_more: i32,
    /// `String.prototype`'s and `Number.prototype`'s handles, where a string's and a number's methods are read from.
    pub(crate) string_proto: i32,
    pub(crate) number_proto: i32,
    /// `Vm::buffer_epoch`, which a typed array's quick elements are right for while theirs is the same.
    pub(crate) buffer_epoch: i32,
    /// `TypedData`'s fields.
    pub(crate) t_element: i32,
    pub(crate) t_quick: i32,
    pub(crate) t_quick_length: i32,
    pub(crate) t_epoch: i32,
    /// Where a native's function is in its `NativeData`.
    pub(crate) n_func: i32,
    /// `Vm::top`, where a native may use the stack from.
    pub(crate) top: i32,
    /// What `for of` over an array checks (see `Vm::get_iterator`): the usual array shape, `Array.prototype`, where its
    /// `Symbol.iterator` was last found (its shape and slot) and the built-in `values` that has to be there.
    pub(crate) array_shape: i32,
    pub(crate) array_proto: i32,
    pub(crate) array_values: i32,
    pub(crate) iterator_shape: i32,
    pub(crate) iterator_slot: i32,
    /// Where the list of shapes is (`Shapes::items`' pointer), how big one is, and where its prototype is.
    pub(crate) shapes: i32,
    pub(crate) shape_size: i32,
    pub(crate) shape_proto: i32,
    /// `Vm::instance_of_cache`, and its entries' size and fields, and `Vm::function_has_instance`.
    pub(crate) instance_of: i32,
    pub(crate) io_size: i32,
    pub(crate) io_shape: i32,
    pub(crate) io_epoch: i32,
    pub(crate) io_holder: i32,
    pub(crate) io_has_slot: i32,
    pub(crate) io_prototype_slot: i32,
    pub(crate) has_instance: i32,
    /// `Jit::site`, where compiled code says which record a helper it calls is to fill in (see `jit::feedback`).
    #[cfg(feature = "jit-opt")]
    pub(crate) site: i32,
}

/// Where a `Vec`'s pointer to what it holds is, in it: found from a real one, as Rust lays a `Vec` out as it likes.
pub(crate) fn vec_pointer() -> Option<usize> {
    let list: Vec<crate::object::Shape> = Vec::with_capacity(1);
    if size_of::<Vec<crate::object::Shape>>() != 3 * size_of::<usize>() {
        return None;
    }
    // SAFETY: a `Vec` is three words, read here as just that.
    let words: [usize; 3] = unsafe { std::mem::transmute_copy(&list) };
    words.iter().position(|&word| word == list.as_ptr() as usize).map(|at| at * size_of::<usize>())
}

impl Layout {
    pub(crate) fn new() -> Option<Self> {
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
            tracking: o(offset_of!(Vm, tracking)),
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

/// How many times the calls of a function's code that had nothing to go on when it was compiled can be made before
/// it's compiled again with what its caches know by then (see `Gen::call`), and how many times a function can be.
pub(crate) const RECOMPILE_AFTER: u64 = 1000;
pub(crate) const MOST_COMPILES: u8 = 4;
/// The longest function that's compiled again, in instructions: a long one costs a lot to compile, and its code is
/// kept twice.
pub(crate) const RECOMPILE_MOST: usize = 400;
/// How many more of those calls code that's being compiled again on the worker makes before it looks for the new code.
pub(crate) const RECOMPILE_POLL: u64 = 64;

/// Where an engine keeps the lists compiled code points into, which never move, so that code can be written for it
/// from another thread.
#[derive(Clone, Copy)]
pub(crate) struct Bases {
    pub(crate) stack: u64,
    pub(crate) caches: u64,
    pub(crate) objects: u64,
    pub(crate) cells: u64,
    pub(crate) protos: u64,
    pub(crate) slots: u64,
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
    pub(crate) proto: Proto,
    pub(crate) ops: Box<[Op]>,
    /// The constants its instructions and its callees' name (see [`constant_of`]), by where they are in the engine's
    /// list.
    pub(crate) consts: Box<[(u32, Value)]>,
    pub(crate) bases: Bases,
    /// The cache entries its property places and its callees' have now, by where they are in the engine's list (see
    /// `Gen::fixed_read`).
    pub(crate) caches: Box<[(u32, Cache)]>,
    /// The shapes those entries name that code may compare with as constants (none of them a dictionary's), in order.
    /// They're pinned as the job is taken (see `Shapes::pin`): a collection may free one before the code is in place,
    /// and its number mustn't be given to another, which the code would take for it.
    pub(crate) plain: Box<[u32]>,
    /// The engine's epoch and its global object, as the entries are right for.
    pub(crate) epoch: u32,
    pub(crate) global: u32,
    /// What each call is likely to call, by the call's instruction (see `Look::likely_callees`), in order.
    pub(crate) calls: Box<[(u32, Box<[u32]>)]>,
    /// What's known of each of those callees, by its proto, in order.
    pub(crate) callees: Box<[Callee]>,
    /// The registers of the frame it's being compiled for, if that's the frame on top (see `Gen::looks_double`), and
    /// the innermost loop that frame is in.
    pub(crate) live: Box<[Value]>,
    pub(crate) looping: Option<(usize, usize)>,
    /// How many times it's been compiled before (see `Vm::jit_recompile`), and whether its slot is given the code
    /// as soon as it's there, which a callee compiled ahead of its caller isn't (see `Look::inline_candidates`).
    pub(crate) generation: u8,
    pub(crate) give: bool,
}

/// A function a job's calls are likely to call, as the job has it.
pub(crate) struct Callee {
    pub(crate) proto_id: u32,
    pub(crate) proto: Proto,
    /// Its slot when the job was taken.
    pub(crate) slot: u64,
    /// Its instructions, if it's short and simple enough to be compiled in line (see `Look::inline_ops`).
    pub(crate) ops: Option<Box<[Op]>>,
}

// SAFETY: what it has is its own: the addresses in `bases` are only written into code, not followed, and the values
// are only looked at, never followed.
unsafe impl Send for Job {}

/// The constant an instruction's template reads from the engine's list, if it reads one. Every template that does has
/// to be named here, as a function compiled on another thread only has what its job copied; a template asking for one
/// that isn't there leaves the function uncompiled.
pub(crate) fn constant_of(op: &Op) -> Option<u32> {
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
pub(crate) fn read_place(op: &Op) -> Option<u16> {
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
pub(crate) fn call_of(op: &Op) -> Option<(u16, Method)> {
    match *op {
        Op::Call { f, .. } | Op::CallPlain { f, .. } => Some((f, Method::None)),
        Op::CallMethod { f, cache, .. } => Some((f, Method::Mono(cache))),
        Op::CallMethodPoly { f, cache, .. } => Some((f, Method::Poly(cache))),
        _ => None,
    }
}

/// A copy of what compiling needs of a proto: all but its captured variables' and lines' lists.
pub(crate) fn copy_proto(proto: &Proto) -> Proto {
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
    pub(crate) vm: &'a Vm,
    pub(crate) proto_id: u32,
    pub(crate) proto: &'a Proto,
    pub(crate) ops: &'a [Op],
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
    /// keeping its `arguments`, at most `INLINE_MOST` instructions long, each of them `inlinable`, which jumps
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
            .all(|(at, op)| inlinable(op) && forward(at, op))
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

/// What a call reads its callee as first (see `Gen::call`): nothing (it's in its register), or a method of the value
/// in the next, at a place that has seen one shape or several.
#[derive(Clone, Copy)]
pub(crate) enum Method {
    None,
    Mono(u16),
    Poly(u16),
}

/// The longest callee compiled in line, in instructions, and how many instructions of callees a function may have
/// in line in all.
pub(crate) const INLINE_MOST: usize = 40;
pub(crate) const INLINE_BUDGET: usize = 600;

/// Whether instruction `op` of a callee can be compiled in line (see `inline_call`): one whose template needs
/// nothing of its frame but its registers (anything else goes to `deopt`), that doesn't call or loop, and that
/// doesn't leave the function but by returning.
pub(crate) fn inlinable(op: &Op) -> bool {
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
            | Op::Note { .. }
            | Op::NoteKey { .. }
            | Op::NoteName { .. }
            | Op::Return { .. }
            | Op::ReturnUndefined
    )
}

/// The register an instruction writes, if it writes one, as far as guessing what a call calls needs to know (see
/// `Gen::guess`).
pub(crate) fn written(op: &Op) -> Option<u16> {
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
