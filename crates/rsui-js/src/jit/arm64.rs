//! Turning a function's instructions into AArch64, one template per instruction, as x64.rs does for x86-64: the same
//! frames, values, helpers and ways in and out (see `JIT.md`), with the machine's own registers and calling
//! convention (AAPCS64, which the helpers are declared with on Arm: see `helpers::jit_abi`).
//!
//! What's kept where: the engine's fixed things are in registers the convention has a function keep (x19-x27), so
//! the Rust helpers leave them be; templates work in x0-x15, which every call may change, and x16/x17 are the
//! assembler's. x18 is never touched, as Apple and Windows keep it for themselves.
//!
//! A compiled function is called with `bl`/`blr` and keeps its return address on the machine's stack (`stp x29, x30`)
//! for the calls it makes; it hands back the value in x0 and how it went in w1 (see `helpers`), as x64.rs's code does
//! in rax and edx.
//!
//! Compared with x64.rs this does each instruction's usual case in machine code (ints, doubles, a property its cache
//! knows, an array's element, a call of compiled code or a native) and leaves the rest to the same helpers, but
//! doesn't yet compile calls in line, call a callee it knows without looking it up, compile a function again with
//! what its calls have learned, keep a value in a machine register from one instruction to the next, write what a
//! place knew into the code for objects (only the global object's), write to places that have seen several shapes,
//! or record feedback: all of those go the longer ways, which do the same.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use foldhash::fast::FixedState;

use super::asm_arm64::*;
use super::helpers::{self, BAIL, TAKEN};
use super::memory::CodeMemory;
use super::plan::*;
use crate::op::{Cache, IS_CLASS, KEEPS_ARGS, Op, Proto};
use crate::value::Value;
use crate::vm::{Frame, Vm};

/// The registers of the frame that's running.
const REGS: R = X19;
/// The frame that's running.
const FRAME: R = X20;
/// The engine.
const VM: R = X21;
/// The tag of an int, to `orr` into one.
const INT: R = X22;
/// The caches of the function that's running.
const ICS: R = X23;
/// `undefined`.
const UNDEF: R = X24;
/// The list of objects.
const OBJECTS: R = X25;
/// The list of cells.
const CELLS: R = X26;
/// `Value::EMPTY`, which holes and variables before they start are.
const EMPTY_R: R = X27;

/// How many instructions a function needs, if it has no loop and makes no call, for going into its code from the
/// interpreter or from Rust to be worth it (as in x64.rs).
const WORTH_ENTERING: usize = 10;

/// The way into compiled code from Rust (see [`trampoline`]): the engine, the frame on top, and where in its function's
/// code to start. It gives back how the code left, with what it handed back in `vm.jit.value`.
pub(crate) type Trampoline = unsafe extern "C" fn(*mut Vm, *mut Frame, usize) -> u32;

/// A helper for one instruction, called with the engine and the instruction's place.
type Direct = extern "C" fn(&mut Vm, u32) -> u32;

/// What writes an engine's code (see x64.rs's `Writer`).
pub(crate) struct Writer {
    memory: CodeMemory,
    layout: Layout,
    /// The functions it has written code for, by their protos, and where their code is.
    known: HashMap<u32, usize, FixedState>,
    /// Whether to say how long each function took.
    timed: bool,
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
        let code = trampoline(&bases, &layout)?;
        let at = memory.add(&code)?;
        // SAFETY: the trampoline was written for this signature.
        let trampoline = unsafe { std::mem::transmute::<usize, Trampoline>(memory.address(at)) };
        Some((Self { memory, layout, known: HashMap::default(), timed }, trampoline))
    }

    /// Something that keeps the code's memory there while it's held.
    pub(crate) fn keep(&self) -> Arc<super::memory::Views> {
        self.memory.keep()
    }

    /// Compiles the function a job is for.
    pub(crate) fn compile(&mut self, job: &Job) -> Done {
        let started = self.timed.then(std::time::Instant::now);
        let code = self.write(job);
        Done {
            proto_id: job.proto_id,
            code,
            generation: job.generation,
            give: job.give,
            inlines: Vec::new(),
            took: started.map(|started| started.elapsed()).unwrap_or_default(),
            used: self.memory.used(),
            committed: self.memory.committed(),
        }
    }

    fn write(&mut self, job: &Job) -> Option<(u64, Compiled)> {
        let address = self.memory.address(self.memory.next_offset());
        // A function whose cold code is out of reach of its branches is assembled again with long ones.
        let mut made = None;
        let mut entries = Vec::new();
        for long in [false, true] {
            let mut generator = Gen::new(&self.layout, job, &self.known, long);
            if let Some(code) = generator.function(address) {
                entries = generator.targets.clone();
                made = Some(code);
                break;
            }
            if generator.missing.get() || !generator.out_of_reach {
                return None;
            }
        }
        let (code, _, offsets, cold) = made?;
        if !self.memory.has_room(code.len()) {
            return None;
        }
        let at = self.memory.add(&code)?;
        debug_assert_eq!(self.memory.address(at), address);
        self.known.insert(job.proto_id, address);
        let compiled = Compiled {
            address,
            offsets: offsets.into(),
            entries: entries.into_boxed_slice(),
            cold,
            inlined: 0,
            // The slot holds the entry, which is just after the words of the header (see `HEADER`).
            worth: code[8] != 0,
            counter: Box::new(AtomicU64::new(u64::MAX)),
            unguessed: Box::default(),
            waits: 0,
            generation: job.generation,
            bytes: code.len(),
            #[cfg(feature = "jit-opt")]
            feedback: 0,
            #[cfg(feature = "jit-opt")]
            feedback_size: 0,
            #[cfg(feature = "jit-opt")]
            feedback_ops: None,
            #[cfg(feature = "jit-opt")]
            feedback_knowns: Box::default(),
            #[cfg(feature = "jit-opt")]
            feedback_offsets: Arc::from([]),
        };
        Some((compiled.slot(), compiled))
    }
}

/// The way in from Rust, a [`Trampoline`] (x0 the engine, x1 the frame on top, x2 where to start): keeps the registers
/// Rust needs kept, sets up the ones compiled code works with, and calls x2 as if it were the body of the frame's
/// function, which has kept its return address as a function's entry does. What that hands back goes in
/// `vm.jit.value`, and how it left is the result.
///
/// It works out `Jit::floor` first, as x64.rs's does, and gives back [`helpers::NOT_ENTERED`] at once if Rust's stack
/// is already below it.
fn trampoline(bases: &Bases, l: &Layout) -> Option<Vec<u8>> {
    let mut a = Asm::new(false);
    let refuse = a.label();
    let done = a.label();
    let enter = a.label();
    let saved = [(X19, X20), (X21, X22), (X23, X24), (X25, X26), (X27, X28), (FP, LR)];
    for &(x, y) in &saved {
        a.push_pair(x, y);
    }
    a.mov(VM, X0);
    a.mov(FRAME, X1);
    a.load(X9, VM, l.stack_base);
    a.load(X10, VM, l.stack_limit);
    a.lsr_imm(true, X10, X10, 1);
    a.subs(true, X9, X9, X10);
    a.csel(true, X9, X9, XZR, HS);
    a.store(X9, VM, l.floor);
    a.mov_from_sp(X16);
    a.cmp(true, X16, X9);
    a.bcond(LO, refuse);
    a.load32(X9, FRAME, l.f_base);
    a.mov_imm64(REGS, bases.stack);
    a.add_lsl(REGS, REGS, X9, 3);
    a.load32(X9, FRAME, l.f_cache_base);
    a.mov_imm64(X10, l.cache_size as u64);
    a.mul(true, X9, X9, X10);
    a.mov_imm64(ICS, bases.caches);
    a.add(true, ICS, ICS, X9);
    a.mov_imm64(OBJECTS, bases.objects);
    a.mov_imm64(CELLS, bases.cells);
    a.mov_imm64(INT, INT_TAG);
    a.mov_imm64(UNDEF, UNDEFINED);
    a.mov_imm64(EMPTY_R, EMPTY);
    a.bl(enter);
    a.store(X0, VM, l.value);
    a.mov32(X0, X1);
    a.bind(done);
    for &(x, y) in saved.iter().rev() {
        a.pop_pair(x, y);
    }
    a.ret();
    a.bind(refuse);
    a.mov_imm64(X0, u64::from(helpers::NOT_ENTERED));
    a.b(done);
    // As a function's entry keeps its return address.
    a.bind(enter);
    a.push_pair(FP, LR);
    a.br(X2);
    a.finish()
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
    fn int_cond(self) -> Cond {
        match self {
            Rel::Lt => LT,
            Rel::Le => LE,
            Rel::Gt => GT,
            Rel::Ge => GE,
        }
    }

    /// After `fcmp`: true only when ordered.
    fn double_cond(self) -> Cond {
        match self {
            Rel::Lt => MI,
            Rel::Le => LS,
            Rel::Gt => GT,
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

/// What `Gen::function` makes: the code, where its body is, and where each instruction's code and cold paths start.
type Made = (Vec<u8>, usize, Box<[u32]>, Box<[u32]>);

struct Gen<'a> {
    a: Asm,
    job: &'a Job,
    /// Whether a template asked for a constant the job doesn't have, which leaves the function uncompiled.
    missing: std::cell::Cell<bool>,
    /// Whether a branch didn't reach its label, which assembling it again with long branches mends.
    out_of_reach: bool,
    l: &'a Layout,
    proto_id: u32,
    proto: &'a Proto,
    ops: &'a [Op],
    labels: Vec<Label>,
    /// Hands the status in w0 back to the caller, with `vm.jit.value`.
    exit: Label,
    /// Hands back what's in x0 and w1, as a call returned them.
    propagate: Label,
    /// Where a call that has made the frame itself comes in.
    pre_body: Label,
    ics: u64,
    bases: Bases,
    /// Which instructions something jumps to, where the interpreter may go in (see `Compiled::entries`).
    targets: Vec<bool>,
    /// The functions the writer has compiled so far.
    #[allow(dead_code)]
    compiled: &'a HashMap<u32, usize, FixedState>,
}

impl<'a> Gen<'a> {
    fn new(l: &'a Layout, job: &'a Job, compiled: &'a HashMap<u32, usize, FixedState>, long: bool) -> Self {
        let (proto_id, proto, ops, bases) = (job.proto_id, &job.proto, &*job.ops, job.bases);
        let mut a = Asm::new(long);
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
            missing: std::cell::Cell::new(false),
            out_of_reach: false,
            l,
            proto_id,
            proto,
            ops,
            labels,
            exit,
            propagate,
            pre_body,
            ics: bases.caches + u64::from(proto.cache_base) * size_of::<Cache>() as u64,
            bases,
            targets,
            compiled,
        }
    }

    /// The cache entry at `at` in the engine's list as the job has it, if it does (see `read_place`).
    fn cache_entry(&self, at: u32) -> Option<Cache> {
        let found = self.job.caches.binary_search_by_key(&at, |&(place, _)| place).ok()?;
        Some(self.job.caches[found].1)
    }

    /// The engine's constant at `index`, which an instruction names (see `constant_of`).
    fn constant(&self, index: usize) -> Value {
        match self.job.consts.binary_search_by_key(&(index as u32), |&(at, _)| at) {
            Ok(found) => self.job.consts[found].1,
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

    /// Where register `index` of the frame is, from `REGS`.
    fn reg(&self, index: u16) -> i32 {
        8 * i32::from(index)
    }

    fn load_reg(&mut self, dst: R, index: u16) {
        self.a.load(dst, REGS, self.reg(index));
    }

    fn store_reg(&mut self, index: u16, src: R) {
        self.a.store(src, REGS, self.reg(index));
    }

    /// Where a cache entry's field is, from `ICS`.
    fn cache(&self, cache: u16, field: i32) -> i32 {
        i32::from(cache) * self.l.cache_size + field
    }

    fn target(&self, i: usize, offset: i32) -> Label {
        self.labels[(i as i64 + 1 + i64::from(offset)) as usize]
    }

    fn load(&mut self, dst: R, src: Src) {
        match src {
            Src::Reg(index) => self.load_reg(dst, index),
            Src::Imm(value) => self.a.mov_imm64(dst, value.0),
        }
    }

    fn store_imm(&mut self, dst: u16, value: u64) {
        self.a.mov_imm64(X0, value);
        self.store_reg(dst, X0);
    }

    fn call_helper(&mut self, f: usize) {
        self.a.call_abs(f as u64);
    }

    /// `w0` is a helper's status: on out unless it's OK.
    fn leave_unless_ok(&mut self) {
        let exit = self.exit;
        self.a.cbz(false, true, X0, exit);
    }

    /// Calls `helpers::op` for instruction `i`, and leaves if it says to.
    fn generic(&mut self, i: usize) {
        self.direct(i, helpers::op);
    }

    /// Calls `helpers::op` for a jump: on to `taken` if it says so, else `not_taken`.
    fn generic_jump(&mut self, i: usize, taken: Label, not_taken: Label) {
        self.a.mov(X0, VM);
        self.a.mov_imm64(X1, u64::from(self.pc(i)));
        self.call_helper(helpers::op as *const () as usize);
        self.taken_or(taken, not_taken);
    }

    /// After a helper that says whether to jump: on to `taken` for `TAKEN`, `not_taken` for OK, else out.
    fn taken_or(&mut self, taken: Label, not_taken: Label) {
        self.a.cmp_imm(false, X0, i64::from(TAKEN));
        self.a.bcond(EQ, taken);
        self.leave_unless_ok();
        self.a.b(not_taken);
    }

    /// Calls `helper(vm, pc)` for instruction `i`, a helper for that one instruction, and leaves if it says to.
    fn direct(&mut self, i: usize, helper: Direct) {
        self.a.mov(X0, VM);
        self.a.mov_imm64(X1, u64::from(self.pc(i)));
        self.call_helper(helper as *const () as usize);
        self.leave_unless_ok();
    }

    /// A cold path at `at` that does instruction `i` by `helper` and goes back to `done`.
    fn cold_direct(&mut self, i: usize, at: Label, done: Label, helper: Direct) {
        let was = self.a.is_cold();
        self.a.set_cold(true);
        self.a.bind(at);
        self.direct(i, helper);
        self.a.b(done);
        self.a.set_cold(was);
    }

    /// A cold path at `at` that does instruction `i` the generic way and goes back to `done`.
    fn cold_generic(&mut self, i: usize, at: Label, done: Label) {
        self.cold_direct(i, at, done, helpers::op);
    }

    /// The top 16 bits of `v` sign-extended into x9, which compares (unsigned) with a tag `t` as `tag(t)`, in one
    /// instruction: `cmp x9, #(t - 0x10000)` is `cmn x9, #(0x10000 - t)`.
    fn tag_of(&mut self, v: R) {
        self.a.asr_imm(true, X9, v, 48);
    }

    fn cmp_tag(&mut self, tag: i32) {
        self.a.cmp_imm(true, X9, i64::from(tag) - 0x10000);
    }

    /// Jumps to `fail` unless `v` is an int. Uses x9.
    fn guard_int(&mut self, v: R, fail: Label) {
        self.a.eor(true, X9, v, INT);
        self.a.lsr_imm(true, X9, X9, 32);
        self.a.cbz(true, true, X9, fail);
    }

    /// Jumps to `fail` unless both are ints. Uses x9 and x10.
    fn guard_ints(&mut self, x: R, y: R, fail: Label) {
        self.a.eor(true, X9, x, INT);
        self.a.eor(true, X10, y, INT);
        self.a.orr(true, X9, X9, X10);
        self.a.lsr_imm(true, X9, X9, 32);
        self.a.cbz(true, true, X9, fail);
    }

    /// Jumps to `fail` unless `v` is an object, and leaves its address in `v`. Uses x9 (which has its tag, as
    /// `tag_of` leaves it, if it fails).
    fn object_address(&mut self, v: R, fail: Label) {
        self.tag_of(v);
        self.cmp_tag(TAG_OBJECT);
        self.a.bcond(NE, fail);
        self.a.mov32(v, v);
        self.a.add_lsl(v, OBJECTS, v, 6);
    }

    /// The object whose handle is in the low 32 bits of `v` (the top ones clear), as an address in `v`.
    fn handle_address(&mut self, v: R) {
        self.a.add_lsl(v, OBJECTS, v, 6);
    }

    /// The number `v` is, as a double in `d`, or on to `fail` if it isn't one. Uses x9.
    fn to_double(&mut self, v: R, d: D, fail: Label) {
        let not_double = self.a.label();
        let done = self.a.label();
        self.a.cmp(true, v, INT);
        self.a.bcond(HS, not_double);
        self.a.fmov_to_d(d, v);
        self.a.b(done);
        self.a.bind(not_double);
        self.a.sub(true, X9, v, INT);
        self.a.lsr_imm(true, X9, X9, 32);
        self.a.cbz(true, true, X9, fail);
        self.a.scvtf(false, d, v);
        self.a.bind(done);
    }

    /// The operand as a double in `d`: loaded into `v` first unless it's known.
    fn src_double(&mut self, src: Src, v: R, d: D, fail: Label) {
        match src {
            Src::Imm(value) if value.is_int() => {
                self.a.mov_imm64(v, u64::from(value.as_int() as u32));
                self.a.scvtf(false, d, v);
            }
            Src::Imm(value) if value.is_double() => {
                self.a.mov_imm64(v, value.0);
                self.a.fmov_to_d(d, v);
            }
            _ => self.to_double(v, d, fail),
        }
    }

    /// What a bitwise operator makes of `v`, in its low 32 bits, or on to `fail` (a double too big for 64 bits, or
    /// NaN, as `quick_int32` has it). Uses x9 and d0-d2.
    fn to_int32(&mut self, v: R, fail: Label) {
        let done = self.a.label();
        self.a.eor(true, X9, v, INT);
        self.a.lsr_imm(true, X9, X9, 32);
        self.a.cbz(true, false, X9, done);
        self.a.cmp(true, v, INT);
        self.a.bcond(HS, fail);
        self.a.fmov_to_d(D0, v);
        self.a.fop1(Fop1::Abs, D1, D0);
        self.a.mov_imm64(X9, 9_223_372_036_854_775_808f64.to_bits());
        self.a.fmov_to_d(D2, X9);
        self.a.fcmp(D1, D2);
        self.a.bcond(PL, fail);
        self.a.fcvtzs(true, v, D0);
        self.a.bind(done);
    }

    /// Stores what's in `d` as `Value::number` would make it: an int if it's a whole one, else the double, with NaN as
    /// the one NaN. Uses x9, x10 and d2.
    fn store_number(&mut self, dst: u16, d: D) {
        let as_double = self.a.label();
        let as_int = self.a.label();
        let nan = self.a.label();
        let done = self.a.label();
        self.a.fcvtzs(false, X9, d);
        self.a.scvtf(false, D2, X9);
        self.a.fcmp(d, D2);
        self.a.bcond(NE, as_double);
        self.a.cbz(false, true, X9, as_int);
        // Zero: a negative one stays a double.
        self.a.fmov_from_d(X10, d);
        self.a.tbz(true, X10, 63, as_double);
        self.a.bind(as_int);
        self.a.orr(true, X9, X9, INT);
        self.store_reg(dst, X9);
        self.a.b(done);
        self.a.bind(as_double);
        self.a.fcmp(d, d);
        self.a.bcond(VS, nan);
        self.a.fmov_from_d(X9, d);
        self.store_reg(dst, X9);
        self.a.b(done);
        self.a.bind(nan);
        self.a.mov_imm64(X9, CANONICAL_NAN);
        self.store_reg(dst, X9);
        self.a.bind(done);
    }

    /// Collects garbage if enough has been made, as `safe_point!` does. `pc` is what the frame says meanwhile.
    fn safe_point(&mut self, pc: u32) {
        let collect = self.a.label();
        let back = self.a.label();
        self.a.load(X9, VM, self.l.debt);
        self.a.load(X10, VM, self.l.threshold);
        self.a.cmp(true, X9, X10);
        self.a.bcond(HI, collect);
        self.a.bind(back);
        let was = self.a.is_cold();
        self.a.set_cold(true);
        self.a.bind(collect);
        self.a.mov(X0, VM);
        self.a.mov_imm64(X1, u64::from(pc));
        self.call_helper(helpers::collect as *const () as usize);
        self.a.b(back);
        self.a.set_cold(was);
    }

    /// Hands back status `status` with x0 as it is, as a function's code returns.
    fn leave(&mut self, status: u32) {
        self.a.mov_imm64(X1, u64::from(status));
        self.a.pop_pair(FP, LR);
        self.a.ret();
    }

    /// Leaves for the interpreter, which does instruction `i` from its start.
    fn bail_stub(&mut self, i: usize) -> Label {
        let was = self.a.is_cold();
        self.a.set_cold(true);
        let here = self.a.label();
        self.a.bind(here);
        self.a.mov_imm64(X9, u64::from(self.pc(i)));
        self.a.store32(X9, FRAME, self.l.f_pc);
        self.leave(BAIL);
        self.a.set_cold(was);
        here
    }

    /// Where the values of the object at `object` are, in x11. Uses x12 and x13.
    fn slots_of(&mut self, object: R) {
        let o = self.l.obj;
        self.a.add_imm(X11, object, o.inline_slots as i64);
        self.a.load8(X12, object, o.slot_count as i32);
        self.a.load(X13, object, o.heap_slots as i32);
        self.a.cmp_imm(false, X12, i64::from(o.on_heap));
        self.a.csel(true, X11, X13, X11, EQ);
    }

    /// Reads slot `slot` (a register holding its number) of the object at `object` into `out`. Uses x11-x13.
    fn read_slot(&mut self, object: R, slot: R, out: R) {
        self.slots_of(object);
        self.a.load_index(Size::B64, out, X11, slot, true);
    }

    /// Reads slot number `slot` of the object at `object` into `out`. Uses x11-x13.
    fn read_slot_at(&mut self, object: R, slot: u32, out: R) {
        self.slots_of(object);
        self.a.load(out, X11, 8 * slot as i32);
    }

    /// Where slot `slot` (a register) of the object at `object` is, in x11. Uses x12 and x13.
    fn slot_place(&mut self, object: R, slot: R) {
        self.slots_of(object);
        self.a.add_lsl(X11, X11, slot, 3);
    }

    /// What read place `cache`'s first entry knew when the job was taken, to be written into the code (see x64.rs's
    /// `fixed_read`).
    fn fixed_read(&self, cache: u16) -> Option<(u32, Option<u32>, u32)> {
        let entry = self.cache_entry(self.proto.cache_base + u32::from(cache))?;
        if self.job.plain.binary_search(&entry.shape).is_err() || entry.slot > (1 << 24) {
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

    // The function.

    /// The whole function: the header (see `HEADER`), the way in for a call from compiled code, the body, and the
    /// cold paths. With where the body is and where each instruction is.
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
        let body = self.a.label();
        self.prologue(body);
        // Where a call that has made the frame comes in.
        self.a.bind(self.pre_body);
        self.a.push_pair(FP, LR);
        self.a.mov_imm64(ICS, self.ics);
        self.a.bind(body);
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
        }
        // Nothing falls off the end of a function, but in case: leave for the interpreter.
        let end = self.bail_stub(self.ops.len() - 1);
        self.a.b(end);
        self.a.set_cold(true);
        self.a.bind(self.exit);
        self.a.mov32(X1, X0);
        self.a.load(X0, VM, self.l.value);
        self.a.bind(self.propagate);
        if !self.proto.handlers.is_empty() {
            self.catch();
        }
        self.a.pop_pair(FP, LR);
        self.a.ret();
        self.a.set_cold(false);
        if self.missing.get() {
            return None;
        }
        let Some(mut code) = self.a.finish() else {
            self.out_of_reach = true;
            return None;
        };
        let body_at = self.a.offset_of(body);
        Asm::patch64(&mut code, 0, (address + self.a.offset_of(self.pre_body)) as u64);
        Asm::patch64(&mut code, 16, (address + body_at) as u64);
        let offsets = self.labels.iter().map(|&label| self.a.offset_of(label) as u32).collect();
        let cold = cold.iter().map(|&label| self.a.offset_of(label) as u32).collect();
        Some((code, body_at, offsets, cold))
    }

    /// On the way out with an error (status in w1, error in x0): a `catch` or `finally` of this function that's around
    /// where it happened takes it, as `Vm::unwind` would, and the code carries on there; else on out.
    fn catch(&mut self) {
        let leave = self.a.label();
        self.a.cmp_imm(false, X1, i64::from(helpers::THROW));
        self.a.bcond(NE, leave);
        self.a.mov(X1, X0);
        self.a.mov(X0, VM);
        self.a.mov(X2, FRAME);
        self.call_helper(helpers::catch as *const () as usize);
        let mut targets: Vec<u32> = self.proto.handlers.iter().map(|handler| handler.target).collect();
        targets.sort_unstable();
        targets.dedup();
        for target in targets {
            self.a.cmp_imm(false, X0, i64::from(target));
            self.a.bcond(EQ, self.labels[target as usize]);
        }
        // Nothing here takes it.
        self.a.mov_imm64(X1, u64::from(helpers::THROW));
        self.a.load(X0, VM, self.l.value);
        self.a.bind(leave);
    }

    /// A call from compiled code comes here with `REGS` and `FRAME` at the new frame's registers and place, w0 its base,
    /// w1 how many arguments, w2 the closure, x3 its captured variables and w4 where the result goes. This makes the
    /// frame as the interpreter's `enter!` does, then goes on to `body`.
    fn prologue(&mut self, body: Label) {
        let l = self.l;
        let proto = self.proto;
        let registers = i64::from(proto.registers);
        let params = i64::from(proto.params);
        let clear = proto.clear() as i32;
        let check = self.a.label();
        let grow = self.a.label();
        let fill = self.a.label();
        let filled = self.a.label();
        let keeps = if proto.flags & KEEPS_ARGS != 0 { crate::vm::KEPT_ARGS } else { 0 };
        let was = self.a.is_cold();
        self.a.push_pair(FP, LR);
        self.a.bind(check);
        self.a.load(X9, VM, l.frames_len);
        self.a.load(X10, VM, l.frames_committed);
        self.a.cmp(true, X9, X10);
        self.a.bcond(HS, grow);
        self.a.add_imm32(X9, X0, registers + 16);
        self.a.load(X10, VM, l.stack_len);
        self.a.cmp(true, X9, X10);
        self.a.bcond(HI, grow);
        self.a.mov_imm64(X9, u64::from(proto.start));
        self.a.store32(X9, FRAME, l.f_pc);
        self.a.store32(X0, FRAME, l.f_base);
        self.a.add_imm32(X9, X0, registers);
        self.a.store32(X9, FRAME, l.f_top);
        self.a.mov_imm64(X9, u64::from(self.proto_id));
        self.a.store32(X9, FRAME, l.f_proto);
        self.a.store32(X2, FRAME, l.f_closure);
        self.a.mov_imm64(X9, u64::from(proto.cache_base));
        self.a.store32(X9, FRAME, l.f_cache_base);
        self.a.store16(X4, FRAME, l.f_ret_dst);
        self.a.mov_imm64(X9, u64::from(keeps));
        self.a.store8(X9, FRAME, l.f_flags);
        self.a.store(UNDEF, FRAME, l.f_new_target);
        self.a.store(X3, FRAME, l.f_upvalues);
        self.a.load(X9, VM, l.frames_len);
        self.a.add_imm(X9, X9, 1);
        self.a.store(X9, VM, l.frames_len);
        if keeps != 0 {
            // Its arguments are kept before any of their registers are made undefined, as `enter_function` does.
            self.a.push_pair(X1, X1);
            self.a.mov32(X1, X1);
            self.a.mov(X0, VM);
            self.call_helper(helpers::keep_args as *const () as usize);
            self.a.pop_pair(X1, X9);
        }
        // The parameters with no argument, and the function's own variables, are undefined.
        if params > 0 {
            self.a.cmp_imm(false, X1, params);
            self.a.bcond(LO, fill);
        }
        self.a.bind(filled);
        for at in 1 + params as i32..clear {
            self.a.store(UNDEF, REGS, 8 * at);
        }
        self.a.mov_imm64(ICS, self.ics);
        self.a.b(body);
        // The cold paths.
        self.a.set_cold(true);
        if params > 0 {
            let again = self.a.label();
            self.a.bind(fill);
            self.a.mov32(X10, X1);
            self.a.add_lsl(X10, REGS, X10, 3);
            self.a.add_imm(X10, X10, 8);
            self.a.add_imm(X11, REGS, 8 * (1 + params));
            self.a.bind(again);
            self.a.store(UNDEF, X10, 0);
            self.a.add_imm(X10, X10, 8);
            self.a.cmp(true, X10, X11);
            self.a.bcond(LO, again);
            self.a.b(filled);
        }
        // No room for the frame: make some, or throw, with the caller's frame on top.
        let failed = self.a.label();
        self.a.bind(grow);
        self.a.push_pair(X0, X1);
        self.a.push_pair(X2, X3);
        self.a.push_pair(X4, X5);
        self.a.add_imm32(X1, X0, registers + 16);
        self.a.mov(X0, VM);
        self.call_helper(helpers::grow as *const () as usize);
        self.a.mov32(X9, X0);
        self.a.pop_pair(X4, X5);
        self.a.pop_pair(X2, X3);
        self.a.pop_pair(X0, X1);
        self.a.cbz(false, true, X9, failed);
        self.a.b(check);
        self.a.bind(failed);
        self.a.load(X0, VM, l.value);
        self.a.mov32(X1, X9);
        self.a.pop_pair(FP, LR);
        self.a.ret();
        self.a.set_cold(was);
    }

    /// One instruction. False if it can't be compiled.
    fn op(&mut self, i: usize) -> bool {
        let op = self.ops[i];
        match op {
            Op::LoadConst { dst, index } => {
                let value = self.constant(index as usize);
                self.store_imm(dst, value.0);
            }
            Op::LoadInt { dst, value } => self.store_imm(dst, Value::int(value).0),
            Op::LoadUndefined { dst } => self.store_reg(dst, UNDEF),
            Op::LoadNull { dst } => self.store_imm(dst, NULL),
            Op::LoadTrue { dst } => self.store_imm(dst, TRUE),
            Op::LoadFalse { dst } => self.store_imm(dst, FALSE),
            Op::LoadEmpty { dst } => self.store_reg(dst, EMPTY_R),
            Op::Move { dst, src } => {
                self.load_reg(X0, src);
                self.store_reg(dst, X0);
            }

            Op::Add { dst, a, b } => self.arith(i, dst, Src::Reg(a), Src::Reg(b), Arith::Add),
            Op::Sub { dst, a, b } => self.arith(i, dst, Src::Reg(a), Src::Reg(b), Arith::Sub),
            Op::Mul { dst, a, b } => self.arith(i, dst, Src::Reg(a), Src::Reg(b), Arith::Mul),
            Op::AddInt { dst, a, value } => self.arith(i, dst, Src::Reg(a), int(value), Arith::Add),
            Op::SubInt { dst, a, value } => self.arith(i, dst, Src::Reg(a), int(value), Arith::Sub),
            Op::MulInt { dst, a, value } => self.arith(i, dst, Src::Reg(a), int(value), Arith::Mul),
            Op::AddInt32 { dst, a, b } => {
                // Two ints add round, as `| 0` makes them. Other numbers (a product that was 0 is a double) are added
                // as doubles, and the sum's low 32 bits are what `| 0` makes of it, while it fits in 64 bits.
                let doubles = self.a.label();
                let slow = self.a.label();
                let done = self.a.label();
                self.load_reg(X0, a);
                self.load_reg(X1, b);
                self.guard_ints(X0, X1, doubles);
                self.a.add(false, X0, X0, X1);
                self.a.orr(true, X0, X0, INT);
                self.store_reg(dst, X0);
                self.a.bind(done);
                self.a.set_cold(true);
                self.a.bind(doubles);
                self.to_double(X0, D0, slow);
                self.to_double(X1, D1, slow);
                self.a.fop(Fop::Add, D0, D0, D1);
                self.a.fop1(Fop1::Abs, D1, D0);
                self.a.mov_imm64(X9, 9_223_372_036_854_775_808f64.to_bits());
                self.a.fmov_to_d(D2, X9);
                self.a.fcmp(D1, D2);
                self.a.bcond(PL, slow);
                self.a.fcvtzs(true, X0, D0);
                self.a.mov32(X0, X0);
                self.a.orr(true, X0, X0, INT);
                self.store_reg(dst, X0);
                self.a.b(done);
                self.a.set_cold(false);
                self.cold_generic(i, slow, done);
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
            Op::ModInt { dst, a, value } => self.modulo(i, dst, a, int(value)),
            Op::ModK { dst, a, k } => {
                let value = self.constant(k as usize);
                self.modulo(i, dst, a, Src::Imm(value));
            }
            Op::BitAnd { dst, a, b } => self.bitwise(i, dst, a, Src::Reg(b), Bit::And),
            Op::BitOr { dst, a, b } => self.bitwise(i, dst, a, Src::Reg(b), Bit::Or),
            Op::BitXor { dst, a, b } => self.bitwise(i, dst, a, Src::Reg(b), Bit::Xor),
            Op::Shl { dst, a, b } => self.bitwise(i, dst, a, Src::Reg(b), Bit::Shl),
            Op::Shr { dst, a, b } => self.bitwise(i, dst, a, Src::Reg(b), Bit::Shr),
            Op::UShr { dst, a, b } => self.bitwise(i, dst, a, Src::Reg(b), Bit::UShr),
            Op::BitAndInt { dst, a, value } => self.bitwise(i, dst, a, int(value), Bit::And),
            Op::BitOrInt { dst, a, value } => self.bitwise(i, dst, a, int(value), Bit::Or),
            Op::ShlInt { dst, a, value } => self.bitwise(i, dst, a, int(value), Bit::Shl),
            Op::ShrInt { dst, a, value } => self.bitwise(i, dst, a, int(value), Bit::Shr),
            Op::UShrInt { dst, a, value } => self.bitwise(i, dst, a, int(value), Bit::UShr),

            Op::Lt { dst, a, b } => self.compare_value(i, dst, Src::Reg(a), Src::Reg(b), Rel::Lt),
            Op::Le { dst, a, b } => self.compare_value(i, dst, Src::Reg(a), Src::Reg(b), Rel::Le),
            Op::Gt { dst, a, b } => self.compare_value(i, dst, Src::Reg(a), Src::Reg(b), Rel::Gt),
            Op::Ge { dst, a, b } => self.compare_value(i, dst, Src::Reg(a), Src::Reg(b), Rel::Ge),
            Op::StrictEq { dst, a, b } => self.strict_value(dst, a, b, false),
            Op::StrictNe { dst, a, b } => self.strict_value(dst, a, b, true),
            Op::Eq { dst, a, b } => self.loose_value(i, dst, a, b, false),
            Op::Ne { dst, a, b } => self.loose_value(i, dst, a, b, true),

            Op::Not { dst, src } => {
                // A boolean here: TRUE and FALSE differ in their lowest bit. Anything else by the helper that only says
                // whether it's truthy.
                let slow = self.a.label();
                let done = self.a.label();
                self.load_reg(X0, src);
                self.a.mov_imm64(X1, FALSE);
                self.a.eor(true, X2, X0, X1);
                self.a.cmp_imm(true, X2, 1);
                self.a.bcond(HI, slow);
                self.a.eor_imm(true, X0, X0, 1);
                self.store_reg(dst, X0);
                self.a.bind(done);
                self.a.set_cold(true);
                self.a.bind(slow);
                self.a.mov(X1, X0);
                self.a.mov(X0, VM);
                self.call_helper(helpers::truthy as *const () as usize);
                // 1 if truthy: FALSE then, else TRUE. (Only w0 is the helper's.)
                self.a.mov32(X0, X0);
                self.a.mov_imm64(X1, TRUE);
                self.a.sub(true, X0, X1, X0);
                self.store_reg(dst, X0);
                self.a.b(done);
                self.a.set_cold(false);
            }
            Op::Inc { dst, src } | Op::Dec { dst, src } => {
                // An int here; a double, or the int that goes past the end, as `Value::number` makes it; anything else
                // the generic way.
                let not_int = self.a.label();
                let slow = self.a.label();
                let done = self.a.label();
                let inc = matches!(op, Op::Inc { .. });
                self.load_reg(X0, src);
                self.guard_int(X0, not_int);
                self.a.adds_imm(false, X1, X0, if inc { 1 } else { -1 });
                self.a.bcond(VS, not_int);
                self.a.orr(true, X1, X1, INT);
                self.store_reg(dst, X1);
                self.a.bind(done);
                self.a.set_cold(true);
                self.a.bind(not_int);
                self.to_double(X0, D0, slow);
                self.a.mov_imm64(X1, if inc { 1f64 } else { -1f64 }.to_bits());
                self.a.fmov_to_d(D1, X1);
                self.a.fop(Fop::Add, D0, D0, D1);
                self.store_number(dst, D0);
                self.a.b(done);
                self.a.set_cold(false);
                self.cold_generic(i, slow, done);
            }
            Op::Neg { dst, src } => {
                // An int here but 0 and the most negative one, whose negations are doubles; a double; anything else the
                // generic way.
                let not_int = self.a.label();
                let slow = self.a.label();
                let done = self.a.label();
                self.load_reg(X0, src);
                self.guard_int(X0, not_int);
                self.a.cbz(false, false, X0, not_int);
                self.a.negs(false, X1, X0);
                self.a.bcond(VS, not_int);
                self.a.orr(true, X1, X1, INT);
                self.store_reg(dst, X1);
                self.a.bind(done);
                self.a.set_cold(true);
                self.a.bind(not_int);
                self.to_double(X0, D0, slow);
                self.a.fop1(Fop1::Neg, D0, D0);
                self.store_number(dst, D0);
                self.a.b(done);
                self.a.set_cold(false);
                self.cold_generic(i, slow, done);
            }
            Op::SloppyThis => {
                // An object is its own `this`; undefined, null and the rest the generic way.
                let slow = self.a.label();
                let done = self.a.label();
                self.load_reg(X0, 0);
                self.tag_of(X0);
                self.cmp_tag(TAG_OBJECT);
                self.a.bcond(NE, slow);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::CheckThis { reg } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.load_reg(X0, reg);
                self.a.cmp(true, X0, EMPTY_R);
                self.a.bcond(EQ, slow);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::CheckObjectCoercible { src } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.load_reg(X0, src);
                self.a.orr_imm(true, X0, X0, 1);
                self.a.mov_imm64(X1, NULL);
                self.a.cmp(true, X0, X1);
                self.a.bcond(EQ, slow);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::IterClose { iter } | Op::IterCloseQuietly { iter } => {
                // Nothing to do once it's finished with, or for an array, which goes by a count.
                let slow = self.a.label();
                let done = self.a.label();
                self.load_reg(X0, iter);
                self.a.cmp(true, X0, EMPTY_R);
                self.a.bcond(EQ, done);
                self.load_reg(X0, iter + 1);
                self.guard_int(X0, slow);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::ToNumber { dst, src } | Op::ToNumeric { dst, src } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.load_reg(X0, src);
                self.a.mov_imm64(X1, OBJECT_TAG);
                self.a.cmp(true, X0, X1);
                self.a.bcond(HS, slow);
                self.store_reg(dst, X0);
                self.a.bind(done);
                self.cold_direct(i, slow, done, helpers::to_number);
            }

            Op::Jump { offset } => {
                let to = self.target(i, offset);
                self.a.b(to);
            }
            Op::Loop { offset } => {
                self.safe_point(self.pc(i) + 1);
                let to = self.target(i, offset);
                self.a.b(to);
            }
            Op::JumpIfTrue { condition, offset } | Op::JumpIfFalse { condition, offset } => {
                let to = self.target(i, offset);
                let next = self.a.label();
                let slow = self.a.label();
                let on_true = matches!(op, Op::JumpIfTrue { .. });
                let (truthy, falsy) = if on_true { (to, next) } else { (next, to) };
                self.load_reg(X0, condition);
                self.a.mov_imm64(X1, TRUE);
                self.a.cmp(true, X0, X1);
                self.a.bcond(EQ, truthy);
                self.a.add_imm(X1, X1, -1);
                self.a.cmp(true, X0, X1);
                self.a.bcond(EQ, falsy);
                // What else is quick to tell: an int is true unless it's 0, an object is, null and undefined aren't.
                // Doubles, strings and the rest by the helper.
                let not_int = self.a.label();
                self.a.eor(true, X9, X0, INT);
                self.a.lsr_imm(true, X9, X9, 32);
                self.a.cbz(true, true, X9, not_int);
                self.a.cbz(false, true, X0, truthy);
                self.a.b(falsy);
                self.a.bind(not_int);
                self.tag_of(X0);
                self.cmp_tag(TAG_OBJECT);
                self.a.bcond(EQ, truthy);
                self.a.orr_imm(true, X1, X0, 1);
                self.a.mov_imm64(X2, NULL);
                self.a.cmp(true, X1, X2);
                self.a.bcond(EQ, falsy);
                self.a.b(slow);
                self.a.set_cold(true);
                self.a.bind(slow);
                self.a.mov(X1, X0);
                self.a.mov(X0, VM);
                self.call_helper(helpers::truthy as *const () as usize);
                self.a.cbz(false, true, X0, truthy);
                self.a.b(falsy);
                self.a.set_cold(false);
                self.a.bind(next);
            }
            Op::JumpIfNullish { condition, offset } | Op::JumpIfNotNullish { condition, offset } => {
                let to = self.target(i, offset);
                self.load_reg(X0, condition);
                self.a.orr_imm(true, X0, X0, 1);
                self.a.mov_imm64(X1, NULL);
                self.a.cmp(true, X0, X1);
                self.a.bcond(if matches!(op, Op::JumpIfNullish { .. }) { EQ } else { NE }, to);
            }
            Op::JumpIfUndefined { condition, offset } | Op::JumpIfNotUndefined { condition, offset } => {
                let to = self.target(i, offset);
                self.load_reg(X0, condition);
                self.a.cmp(true, X0, UNDEF);
                self.a.bcond(if matches!(op, Op::JumpIfUndefined { .. }) { EQ } else { NE }, to);
            }
            Op::JumpIfNull { condition, offset } | Op::JumpIfNotNull { condition, offset } => {
                let to = self.target(i, offset);
                self.load_reg(X0, condition);
                self.a.mov_imm64(X1, NULL);
                self.a.cmp(true, X0, X1);
                self.a.bcond(if matches!(op, Op::JumpIfNull { .. }) { EQ } else { NE }, to);
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
            Op::IncLoopLt { i: counter, limit, offset } => self.inc_loop(i, counter, Src::Reg(limit), offset),
            Op::IncLoopLtInt { i: counter, limit, offset } => self.inc_loop(i, counter, int(limit), offset),
            Op::IncLoopLtLength { i: counter, object, offset } => self.inc_loop_length(i, counter, object, offset),
            Op::IterNext { value, iter, offset } => self.iter_next(i, value, iter, offset),
            Op::JumpTypeIs { a, kind, offset } => self.jump_type(a, kind, false, i32::from(offset), i),
            Op::JumpTypeIsNot { a, kind, offset } => self.jump_type(a, kind, true, i32::from(offset), i),
            Op::IterResult { offset, .. } => {
                let to = self.target(i, i32::from(offset));
                let next = self.a.label();
                self.generic_jump(i, to, next);
                self.a.bind(next);
            }

            Op::GetGlobal { dst, cache } => {
                let slow = self.a.label();
                let done = self.a.label();
                let data = self.a.label();
                let l = self.l;
                // The global object's shape and slot as the place knew them when this was compiled, in the code (see
                // `fixed_read`), then the cache's way.
                if let Some((shape, None, slot)) = self.fixed_read(cache) {
                    self.a.mov_imm64(X0, self.bases.objects + 64 * u64::from(self.job.global));
                    self.a.load32(X9, X0, l.obj.shape as i32);
                    self.a.cmp_imm(false, X9, i64::from(shape));
                    self.a.bcond(NE, data);
                    self.read_slot_at(X0, slot, X2);
                    self.store_reg(dst, X2);
                    self.a.b(done);
                }
                self.a.bind(data);
                self.a.load32(X0, VM, l.global);
                self.handle_address(X0);
                self.a.load32(X9, X0, l.obj.shape as i32);
                self.a.load32(X10, ICS, self.cache(cache, l.c_shape));
                self.a.cmp(false, X9, X10);
                self.a.bcond(NE, slow);
                self.a.load32(X10, ICS, self.cache(cache, l.c_slot));
                self.read_slot(X0, X10, X2);
                self.store_reg(dst, X2);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::GetCell { dst, cell } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.a.load32(X0, REGS, self.reg(cell));
                self.a.load_index(Size::B64, X0, CELLS, X0, true);
                self.a.cmp(true, X0, EMPTY_R);
                self.a.bcond(EQ, slow);
                self.store_reg(dst, X0);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::SetCell { cell, src } | Op::InitCell { cell, src } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.a.load32(X0, REGS, self.reg(cell));
                if matches!(op, Op::SetCell { .. }) {
                    self.a.load_index(Size::B64, X1, CELLS, X0, true);
                    self.a.cmp(true, X1, EMPTY_R);
                    self.a.bcond(EQ, slow);
                }
                self.load_reg(X1, src);
                self.a.store_index(Size::B64, X1, CELLS, X0, true);
                self.cell_barrier(done);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::GetUp { dst, index } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.a.load(X0, FRAME, self.l.f_upvalues);
                self.a.load32(X0, X0, 8 * i32::from(index));
                self.a.load_index(Size::B64, X0, CELLS, X0, true);
                self.a.cmp(true, X0, EMPTY_R);
                self.a.bcond(EQ, slow);
                self.store_reg(dst, X0);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::GetUpValue { dst, index } => {
                self.a.load(X0, FRAME, self.l.f_upvalues);
                self.a.load(X0, X0, 8 * i32::from(index));
                self.store_reg(dst, X0);
            }
            Op::SetUp { index, src } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.a.load(X0, FRAME, self.l.f_upvalues);
                self.a.load32(X0, X0, 8 * i32::from(index));
                self.a.load_index(Size::B64, X1, CELLS, X0, true);
                self.a.cmp(true, X1, EMPTY_R);
                self.a.bcond(EQ, slow);
                self.load_reg(X1, src);
                self.a.store_index(Size::B64, X1, CELLS, X0, true);
                self.cell_barrier(done);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::LoadCallee { dst } => {
                self.a.load32(X0, FRAME, self.l.f_closure);
                self.a.mov_imm64(X1, OBJECT_TAG);
                self.a.orr(true, X0, X0, X1);
                self.store_reg(dst, X0);
            }

            Op::GetProp { dst, object, cache } | Op::GetPropPoly { dst, object, cache } => {
                let slow = self.a.label();
                let done = self.a.label();
                self.load_reg(X0, object);
                self.get_prop(cache, slow, X2);
                self.store_reg(dst, X2);
                self.a.bind(done);
                self.cold_generic(i, slow, done);
            }
            Op::SetProp { object, value, cache } => {
                // A property the object has, where the place's first entry knows it is; anything else (adding one,
                // another shape) the generic way.
                let slow = self.a.label();
                let done = self.a.label();
                let l = self.l;
                self.load_reg(X0, object);
                self.object_address(X0, slow);
                self.a.load32(X9, X0, l.obj.shape as i32);
                self.a.load32(X10, ICS, self.cache(cache, l.c_shape));
                self.a.cmp(false, X9, X10);
                self.a.bcond(NE, slow);
                self.a.load8(X9, X0, l.obj.flags as i32);
                self.a.tbz(true, X9, crate::gc::OLD.trailing_zeros() as u8, slow);
                let adds = self.a.label();
                self.a.load32(X10, ICS, self.cache(cache, l.c_other));
                self.a.cmp_imm(false, X10, -1);
                self.a.bcond(NE, adds);
                self.a.load32(X10, ICS, self.cache(cache, l.c_slot));
                self.slot_place(X0, X10);
                self.load_reg(X1, value);
                self.a.store(X1, X11, 0);
                self.a.bind(done);
                // The cache says what shape adding it makes, which a constructor's `this.x = x` does each time.
                self.a.set_cold(true);
                self.a.bind(adds);
                self.add_slot(cache, object, value, slow, done);
                self.a.set_cold(false);
                self.cold_generic(i, slow, done);
            }
            Op::GetLength { dst, object, .. } => {
                let slow = self.a.label();
                let done = self.a.label();
                let not_object = self.a.label();
                let not_array = self.a.label();
                let length = self.a.label();
                let o = self.l.obj;
                self.load_reg(X0, object);
                self.object_address(X0, not_object);
                self.a.load8(X10, X0, o.kind as i32);
                self.a.cmp_imm(false, X10, i64::from(o.array_tag));
                self.a.bcond(NE, not_array);
                self.a.bind(length);
                self.a.load32(X1, X0, (o.kind + o.array_length) as i32);
                self.a.tbz(true, X1, 31, slow);
                self.a.orr(true, X1, X1, INT);
                self.store_reg(dst, X1);
                self.a.bind(done);
                self.a.set_cold(true);
                // `arguments.length` is how many there are, as for an array.
                self.a.bind(not_array);
                self.a.cmp_imm(false, X10, i64::from(o.arguments_tag));
                self.a.bcond(EQ, length);
                self.a.b(slow);
                // A string's, by a helper that does only that. (x9 has the value's tag.)
                self.a.bind(not_object);
                self.cmp_tag(TAG_STRING);
                self.a.bcond(NE, slow);
                self.a.mov(X1, X0);
                self.a.mov(X0, VM);
                self.call_helper(helpers::string_length as *const () as usize);
                self.store_reg(dst, X0);
                self.a.b(done);
                // Anything else (an object's `length` property) by a helper that does only that.
                self.a.bind(slow);
                self.a.mov(X0, VM);
                self.a.mov_imm64(X1, u64::from(self.pc(i)));
                self.load_reg(X2, object);
                self.call_helper(helpers::get_length as *const () as usize);
                self.leave_unless_ok();
                self.a.b(done);
                self.a.set_cold(false);
            }
            Op::GetElem { dst, object, key } => {
                let slow = self.a.label();
                let done = self.a.label();
                let not_array = self.a.label();
                let elements = self.a.label();
                let o = self.l.obj;
                self.load_reg(X0, object);
                self.load_reg(X1, key);
                self.object_address(X0, slow);
                self.guard_int(X1, slow);
                self.a.mov32(X1, X1);
                self.a.load8(X10, X0, o.kind as i32);
                self.a.cmp_imm(false, X10, i64::from(o.array_tag));
                self.a.bcond(NE, not_array);
                self.a.bind(elements);
                self.a.load32(X9, X0, (o.kind + o.array_length) as i32);
                self.a.cmp(false, X1, X9);
                self.a.bcond(HS, slow);
                self.a.load(X9, X0, (o.kind + o.array_start) as i32);
                self.a.load_index(Size::B64, X2, X9, X1, true);
                self.a.cmp(true, X2, EMPTY_R);
                self.a.bcond(EQ, slow);
                self.store_reg(dst, X2);
                self.a.bind(done);
                self.a.set_cold(true);
                // `arguments` keeps its elements where an array does.
                self.a.bind(not_array);
                self.a.cmp_imm(false, X10, i64::from(o.arguments_tag));
                self.a.bcond(EQ, elements);
                // Anything else (a typed array's, an element that isn't there, a key that's a name) by a helper that
                // does only that.
                self.a.bind(slow);
                self.a.mov(X0, VM);
                self.a.mov_imm64(X1, u64::from(self.pc(i)) | u64::from(dst) << 32);
                self.load_reg(X2, object);
                self.load_reg(X3, key);
                self.call_helper(helpers::get_elem as *const () as usize);
                self.leave_unless_ok();
                self.a.b(done);
                self.a.set_cold(false);
            }
            Op::SetElem { object, key, value } | Op::SetElemTyped { object, key, value } => {
                // An array's element that's there, in an array that can be written in place; anything else by the
                // helper for this instruction.
                let slow = self.a.label();
                let done = self.a.label();
                let o = self.l.obj;
                self.load_reg(X0, object);
                self.object_address(X0, slow);
                self.load_reg(X1, key);
                self.guard_int(X1, slow);
                self.a.mov32(X1, X1);
                self.a.load8(X10, X0, o.kind as i32);
                self.a.cmp_imm(false, X10, i64::from(o.array_tag));
                self.a.bcond(NE, slow);
                self.a.load8(X10, X0, o.flags as i32);
                self.a.tst_imm(false, X10, u64::from(crate::object::FROZEN_ELEMENTS | crate::gc::OLD));
                self.a.bcond(NE, slow);
                self.a.load32(X9, X0, (o.kind + o.array_length) as i32);
                self.a.cmp(false, X1, X9);
                self.a.bcond(HS, slow);
                self.a.load(X9, X0, (o.kind + o.array_start) as i32);
                self.a.add_lsl(X9, X9, X1, 3);
                self.a.load(X10, X9, 0);
                self.a.cmp(true, X10, EMPTY_R);
                self.a.bcond(EQ, slow);
                self.load_reg(X2, value);
                self.a.store(X2, X9, 0);
                self.a.bind(done);
                self.cold_direct(i, slow, done, helpers::set_elem);
            }

            Op::Call { dst, f, argc } => self.call(i, dst, f, argc, None),
            Op::CallPlain { dst, f, argc } => {
                self.store_reg(f + 1, UNDEF);
                self.call(i, dst, f, argc, None)
            }
            Op::CallMethod { dst, f, cache } | Op::CallMethodPoly { dst, f, cache } => {
                self.call(i, dst, f, 0, Some(cache))
            }
            Op::CallMath { dst, f, which } => self.call_math(i, dst, f, which),
            Op::InstanceOf { .. } => self.direct(i, helpers::instance_of),
            Op::New { .. }
            | Op::CallSpread { .. }
            | Op::NewSpread { .. }
            | Op::SuperCall { .. }
            | Op::SuperCallSpread { .. } => self.direct(i, helpers::call),
            Op::Return { src } => {
                self.load_reg(X0, src);
                self.ret(i);
            }
            Op::ReturnUndefined => {
                self.a.mov(X0, UNDEF);
                self.ret(i);
            }

            Op::NewObject { dst, capacity } => {
                self.a.mov(X0, VM);
                self.a.mov_imm64(X1, u64::from(capacity));
                self.call_helper(helpers::new_object as *const () as usize);
                self.store_reg(dst, X0);
            }
            Op::NewArray { dst, capacity } => {
                self.a.mov(X0, VM);
                self.a.mov_imm64(X1, u64::from(capacity));
                self.call_helper(helpers::new_array as *const () as usize);
                self.store_reg(dst, X0);
            }
            Op::ArrayPush { array, value } => {
                self.a.mov(X0, VM);
                self.load_reg(X1, array);
                self.load_reg(X2, value);
                self.call_helper(helpers::array_push as *const () as usize);
            }
            // Nothing unless something's rendering, when what was read is noted the generic way. A call that's been
            // given what it gave last time, or made by the Note, is jumped over with the Note after it (see
            // `keeping.rs`).
            Op::Note { kind, count, .. } if kind == crate::op::NOTE_KEEP && i + 3 + (count as usize) < self.labels.len() => {
                let note = self.a.label();
                let done = self.a.label();
                let past = self.labels[i + 3 + count as usize];
                self.a.load8(X9, VM, self.l.tracking);
                self.a.tbz(true, X9, 0, note);
                self.a.bind(done);
                let was = self.a.is_cold();
                self.a.set_cold(true);
                self.a.bind(note);
                self.generic_jump(i, past, done);
                self.a.set_cold(was);
            }
            Op::Note { .. } => {
                let note = self.a.label();
                let done = self.a.label();
                self.a.load8(X9, VM, self.l.tracking);
                self.a.tbz(true, X9, 0, note);
                self.a.bind(done);
                self.cold_generic(i, note, done);
            }
            Op::NoteKey { .. } | Op::NoteName { .. } => {}
            Op::Closure { dst, proto } => {
                self.a.mov(X0, VM);
                self.a.mov_imm64(X1, u64::from(proto));
                self.call_helper(helpers::closure as *const () as usize);
                self.store_reg(dst, X0);
            }
            Op::NewCell { dst, src } => {
                self.a.mov(X0, VM);
                self.load_reg(X1, src);
                self.call_helper(helpers::new_cell as *const () as usize);
                self.store_reg(dst, X0);
            }
            Op::CopyCell { reg } => {
                self.a.load32(X1, REGS, self.reg(reg));
                self.a.load_index(Size::B64, X1, CELLS, X1, true);
                self.a.mov(X0, VM);
                self.call_helper(helpers::new_cell as *const () as usize);
                self.store_reg(reg, X0);
            }
            Op::TypeOf { dst, src } => {
                self.a.mov(X0, VM);
                self.load_reg(X1, src);
                self.call_helper(helpers::type_of as *const () as usize);
                self.store_reg(dst, X0);
            }
            Op::GetIterator { .. } => self.direct(i, helpers::get_iterator),
            Op::ArraySpread { .. } => self.direct(i, helpers::array_spread),
            Op::Rest { .. } => self.direct(i, helpers::rest),
            Op::SuperGet { .. } => self.direct(i, helpers::super_get),
            Op::CreateArguments { .. } => self.direct(i, helpers::create_arguments),
            Op::SetHome { .. } => self.direct(i, helpers::set_home),
            Op::Concat { .. } => self.direct(i, helpers::concat),

            // A generator stops: the frame has gone, and what it hands back is the generator, or what it yields.
            Op::InitialYield => self.stop_here(i, helpers::initial_yield),
            Op::Yield { .. } => self.stop_here(i, helpers::yield_value),
            // Never in a function that's compiled (see `jit::compilable`).
            Op::Await { .. } => return false,
            // Everything else the generic way.
            _ => self.generic(i),
        }
        true
    }

    /// `+`, `-` or `*`: two ints here, two numbers as doubles out of the way, anything else by `helpers::add` for a
    /// `+` (which is mostly joining strings) or the generic way.
    fn arith(&mut self, i: usize, dst: u16, x: Src, y: Src, kind: Arith) {
        let doubles = self.a.label();
        let slow = self.a.label();
        let done = self.a.label();
        self.load(X0, x);
        self.load(X1, y);
        let both_known = matches!((x, y), (Src::Imm(_), Src::Imm(_)));
        let known_double = |src: Src| matches!(src, Src::Imm(value) if !value.is_int());
        if known_double(x) || known_double(y) || both_known {
            self.a.b(doubles);
        } else {
            match (x, y) {
                (Src::Reg(_), Src::Reg(_)) => self.guard_ints(X0, X1, doubles),
                (Src::Reg(_), Src::Imm(_)) => self.guard_int(X0, doubles),
                _ => self.guard_int(X1, doubles),
            }
            match kind {
                Arith::Add => {
                    self.a.adds(false, X2, X0, X1);
                    self.a.bcond(VS, doubles);
                }
                Arith::Sub => {
                    self.a.subs(false, X2, X0, X1);
                    self.a.bcond(VS, doubles);
                }
                Arith::Mul => {
                    // Zero is a double here, as it might be negative.
                    self.a.smull(X2, X0, X1);
                    self.a.sxtw(X3, X2);
                    self.a.cmp(true, X2, X3);
                    self.a.bcond(NE, doubles);
                    self.a.cbz(false, false, X2, doubles);
                    self.a.mov32(X2, X2);
                }
            }
            self.a.orr(true, X2, X2, INT);
            self.store_reg(dst, X2);
        }
        self.a.bind(done);
        self.a.set_cold(true);
        self.a.bind(doubles);
        self.src_double(x, X0, D0, slow);
        self.src_double(y, X1, D1, slow);
        let op = match kind {
            Arith::Add => Fop::Add,
            Arith::Sub => Fop::Sub,
            Arith::Mul => Fop::Mul,
        };
        self.a.fop(op, D0, D0, D1);
        self.a.fmov_from_d(X2, D0);
        self.store_reg(dst, X2);
        self.a.b(done);
        self.a.bind(slow);
        if kind == Arith::Add {
            self.load(X2, x);
            self.load(X3, y);
            self.a.mov(X0, VM);
            self.a.mov_imm64(X1, u64::from(self.pc(i)));
            self.call_helper(helpers::add as *const () as usize);
            self.leave_unless_ok();
        } else {
            self.generic(i);
        }
        self.a.b(done);
        self.a.set_cold(false);
    }

    fn arith_k(&mut self, i: usize, dst: u16, a: u16, k: u16, kind: Arith) {
        let value = self.constant(k as usize);
        if value.is_number() {
            self.arith(i, dst, Src::Reg(a), Src::Imm(value), kind);
        } else if kind == Arith::Add {
            // `x + "..."` joins strings, by the helper that does only `+`.
            self.load_reg(X2, a);
            self.a.mov_imm64(X3, value.0);
            self.a.mov(X0, VM);
            self.a.mov_imm64(X1, u64::from(self.pc(i)));
            self.call_helper(helpers::add as *const () as usize);
            self.leave_unless_ok();
        } else {
            self.generic(i);
        }
    }

    fn divide(&mut self, i: usize, dst: u16, x: Src, y: Src) {
        let slow = self.a.label();
        let done = self.a.label();
        self.load(X0, x);
        self.load(X1, y);
        self.src_double(x, X0, D0, slow);
        self.src_double(y, X1, D1, slow);
        self.a.fop(Fop::Div, D0, D0, D1);
        self.store_number(dst, D0);
        self.a.bind(done);
        self.cold_generic(i, slow, done);
    }

    /// Dividing by `value`, a power of two above zero: an int it divides exactly is shifted, a number is divided as
    /// doubles, anything else the generic way.
    fn divide_int(&mut self, i: usize, dst: u16, a: u16, value: i16) {
        let doubles = self.a.label();
        let slow = self.a.label();
        let done = self.a.label();
        let by = i32::from(value);
        self.load_reg(X0, a);
        if by > 0 && by & (by - 1) == 0 {
            self.guard_int(X0, doubles);
            if by > 1 {
                self.a.tst_imm(false, X0, (by - 1) as u64);
                self.a.bcond(NE, doubles);
            }
            self.a.asr_imm(false, X1, X0, by.trailing_zeros() as u8);
            self.a.orr(true, X1, X1, INT);
            self.store_reg(dst, X1);
        } else {
            self.a.b(doubles);
        }
        self.a.bind(done);
        self.a.set_cold(true);
        self.a.bind(doubles);
        self.to_double(X0, D0, slow);
        self.a.mov_imm64(X1, f64::from(by).to_bits());
        self.a.fmov_to_d(D1, X1);
        self.a.fop(Fop::Div, D0, D0, D1);
        self.store_number(dst, D0);
        self.a.b(done);
        self.a.set_cold(false);
        self.cold_generic(i, slow, done);
    }

    /// `%` on two ints when the left one isn't negative and the right one is above zero; anything else the generic
    /// way.
    fn modulo(&mut self, i: usize, dst: u16, a: u16, y: Src) {
        let slow = self.a.label();
        let done = self.a.label();
        self.load_reg(X0, a);
        match y {
            Src::Reg(b) => {
                self.load_reg(X1, b);
                self.guard_ints(X0, X1, slow);
                self.a.cmp_imm(false, X1, 0);
                self.a.bcond(LE, slow);
            }
            Src::Imm(value) if value.is_int() && value.as_int() > 0 => {
                self.guard_int(X0, slow);
                self.a.mov_imm64(X1, u64::from(value.as_int() as u32));
            }
            Src::Imm(_) => {
                self.generic(i);
                return;
            }
        }
        self.a.tbz(true, X0, 31, slow);
        self.a.sdiv(false, X2, X0, X1);
        self.a.msub(false, X0, X2, X1, X0);
        self.a.orr(true, X0, X0, INT);
        self.store_reg(dst, X0);
        self.a.bind(done);
        self.cold_generic(i, slow, done);
    }

    fn bitwise(&mut self, i: usize, dst: u16, a: u16, y: Src, kind: Bit) {
        let quick = self.a.label();
        let slow = self.a.label();
        let operate = self.a.label();
        let ushr_double = self.a.label();
        let done = self.a.label();
        self.load_reg(X0, a);
        match y {
            Src::Reg(b) => {
                self.load_reg(X1, b);
                self.guard_ints(X0, X1, quick);
            }
            Src::Imm(value) => {
                self.a.mov_imm64(X1, u64::from(value.as_int() as u32));
                self.guard_int(X0, quick);
            }
        }
        self.a.bind(operate);
        match kind {
            Bit::And => self.a.and(false, X0, X0, X1),
            Bit::Or => self.a.orr(false, X0, X0, X1),
            Bit::Xor => self.a.eor(false, X0, X0, X1),
            Bit::Shl => self.a.lslv(false, X0, X0, X1),
            Bit::Shr => self.a.asrv(false, X0, X0, X1),
            Bit::UShr => {
                self.a.lsrv(false, X0, X0, X1);
                self.a.tbz(true, X0, 31, ushr_double);
            }
        }
        self.a.orr(true, X0, X0, INT);
        self.store_reg(dst, X0);
        self.a.bind(done);
        self.a.set_cold(true);
        // Doubles that fit in 64 bits, as `quick_int32` does.
        self.a.bind(quick);
        self.to_int32(X0, slow);
        if let Src::Reg(_) = y {
            self.to_int32(X1, slow);
        }
        self.a.b(operate);
        if kind == Bit::UShr {
            self.a.bind(ushr_double);
            self.a.ucvtf(false, D0, X0);
            self.a.fmov_from_d(X0, D0);
            self.store_reg(dst, X0);
            self.a.b(done);
        }
        self.a.set_cold(false);
        self.cold_generic(i, slow, done);
    }

    /// Jumps to `on_true` or `on_false` on whether `x rel y`: ints here, doubles out of the way, anything else by a
    /// helper that only compares.
    fn branch_compare(&mut self, i: usize, x: Src, y: Src, rel: Rel, on_true: Label, on_false: Label) {
        let doubles = self.a.label();
        let slow = self.a.label();
        self.load(X0, x);
        self.load(X1, y);
        match (x, y) {
            (Src::Reg(_), Src::Reg(_)) => self.guard_ints(X0, X1, doubles),
            (Src::Reg(_), Src::Imm(value)) if value.is_int() => self.guard_int(X0, doubles),
            (Src::Imm(value), Src::Reg(_)) if value.is_int() => self.guard_int(X1, doubles),
            _ => self.a.b(doubles),
        }
        self.a.cmp(false, X0, X1);
        self.a.bcond(rel.int_cond(), on_true);
        self.a.b(on_false);
        self.a.set_cold(true);
        self.a.bind(doubles);
        self.src_double(x, X0, D0, slow);
        self.src_double(y, X1, D1, slow);
        self.a.fcmp(D0, D1);
        self.a.bcond(rel.double_cond(), on_true);
        self.a.b(on_false);
        self.a.bind(slow);
        self.load(X2, x);
        self.load(X3, y);
        self.a.mov(X0, VM);
        self.a.mov_imm64(X1, u64::from(self.pc(i)));
        self.call_helper(helpers::compare as *const () as usize);
        self.taken_or(on_true, on_false);
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
            self.safe_point(self.pc(i) + 1);
            self.a.b(to);
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
        self.a.b(done);
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
        self.load_reg(X0, a);
        self.load_reg(X1, b);
        self.a.cmp(true, X0, X1);
        self.a.bcond(EQ, same);
        // Different bits: unequal if either is only ever equal to itself (an object, null, undefined, a boolean, a
        // symbol), so its tag less the object tag is 0, 2, 3 or 4.
        self.only_itself(X0, differ, first_other);
        self.a.bind(first_other);
        self.only_itself(X1, differ, both_other);
        self.a.bind(same);
        // The same bits: equal, unless it's NaN.
        self.a.cmp(true, X0, INT);
        self.a.bcond(HS, equal);
        self.a.fmov_to_d(D0, X0);
        self.a.fcmp(D0, D0);
        self.a.bcond(VS, differ);
        self.a.b(equal);
        self.a.set_cold(true);
        // Both could be equal to something with other bits: two numbers are compared here, strings and bigints the
        // generic way. Two ints with different bits are different.
        self.a.bind(both_other);
        let not_ints = self.a.label();
        self.guard_ints(X0, X1, not_ints);
        self.a.b(differ);
        self.a.bind(not_ints);
        self.to_double(X0, D0, slow);
        self.to_double(X1, D1, slow);
        self.a.fcmp(D0, D1);
        self.a.bcond(EQ, equal);
        self.a.b(differ);
        self.a.bind(slow);
        self.a.mov(X2, X1);
        self.a.mov(X1, X0);
        self.a.mov(X0, VM);
        self.call_helper(helpers::strict_equals as *const () as usize);
        self.a.cbz(false, true, X0, equal);
        self.a.b(differ);
        self.a.set_cold(false);
    }

    /// Jumps to `itself` if `v` is only ever `===` to itself, else to `other`. Uses x9 and x10.
    fn only_itself(&mut self, v: R, itself: Label, other: Label) {
        self.a.lsr_imm(true, X9, v, 48);
        self.a.mov_imm64(X10, TAG_OBJECT as u64);
        self.a.sub(false, X9, X9, X10);
        self.a.cmp_imm(false, X9, 4);
        self.a.bcond(HI, other);
        self.a.cmp_imm(false, X9, 1);
        self.a.bcond(EQ, other);
        self.a.b(itself);
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
        self.a.b(done);
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
        self.safe_point(self.pc(i) + 1);
        self.a.b(to);
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
        self.load_reg(X0, a);
        self.a.mov_imm64(X1, Value::int(value.into()).0);
        self.a.cmp(true, X0, X1);
        self.a.bcond(EQ, equal);
        self.a.cmp(true, X0, INT);
        self.a.bcond(LO, double);
        self.a.b(differ);
        self.a.set_cold(true);
        self.a.bind(double);
        self.a.fmov_to_d(D0, X0);
        self.a.mov_imm64(X1, u64::from(i32::from(value) as u32));
        self.a.scvtf(false, D1, X1);
        self.a.fcmp(D0, D1);
        self.a.bcond(EQ, equal);
        self.a.b(differ);
        self.a.set_cold(false);
        if is_loop {
            self.a.bind(taken);
            self.safe_point(self.pc(i) + 1);
            self.a.b(to);
        }
        self.a.bind(next);
    }

    /// `typeof a == "..."` and its jump: told here from the tag, except whether an object is a function, which is the
    /// helper's.
    fn jump_type(&mut self, a: u16, kind: u16, negate: bool, offset: i32, i: usize) {
        use crate::op::*;
        let to = self.target(i, offset);
        let next = self.a.label();
        let slow = self.a.label();
        let (is, isnt) = if negate { (next, to) } else { (to, next) };
        self.load_reg(X0, a);
        self.tag_of(X0);
        let tag_is = |g: &mut Self, tag: i32| {
            g.cmp_tag(tag);
            g.a.bcond(EQ, is);
            g.a.b(isnt);
        };
        match kind {
            TYPE_NUMBER => {
                self.cmp_tag(TAG_OBJECT);
                self.a.bcond(LO, is);
                self.a.b(isnt);
            }
            TYPE_STRING => tag_is(self, 0xFFFB),
            TYPE_SYMBOL => tag_is(self, 0xFFFD),
            TYPE_BIGINT => tag_is(self, 0xFFFF),
            TYPE_UNDEFINED => {
                self.a.cmp(true, X0, UNDEF);
                self.a.bcond(EQ, is);
                self.a.b(isnt);
            }
            TYPE_BOOLEAN => {
                self.a.orr_imm(true, X0, X0, 1);
                self.a.mov_imm64(X1, TRUE);
                self.a.cmp(true, X0, X1);
                self.a.bcond(EQ, is);
                self.a.b(isnt);
            }
            TYPE_FUNCTION => {
                self.cmp_tag(TAG_OBJECT);
                self.a.bcond(EQ, slow);
                self.a.b(isnt);
            }
            _ => {
                // "object": null is, an object might be, numbers, strings, undefined, booleans, symbols and bigints
                // aren't.
                self.cmp_tag(TAG_OBJECT);
                self.a.bcond(EQ, slow);
                self.a.bcond(LO, isnt);
                self.a.mov_imm64(X1, NULL);
                self.a.cmp(true, X0, X1);
                self.a.bcond(EQ, is);
                for tag in [0xFFFB, 0xFFFD, 0xFFFF] {
                    self.cmp_tag(tag);
                    self.a.bcond(EQ, isnt);
                }
                self.a.cmp(true, X0, UNDEF);
                self.a.bcond(EQ, isnt);
                self.a.orr_imm(true, X1, X0, 1);
                self.a.mov_imm64(X2, TRUE);
                self.a.cmp(true, X1, X2);
                self.a.bcond(EQ, isnt);
                self.a.b(slow);
            }
        }
        // Whether an object is a function, by a helper that only says what `typeof` says. (x0 has the value.)
        self.a.set_cold(true);
        self.a.bind(slow);
        self.a.mov(X1, X0);
        self.a.mov(X0, VM);
        self.call_helper(helpers::type_kind as *const () as usize);
        self.a.cmp_imm(false, X0, i64::from(kind));
        self.a.bcond(EQ, is);
        self.a.b(isnt);
        self.a.set_cold(false);
        self.a.bind(next);
    }

    /// `==`: two ints here, anything else the generic way.
    fn jump_loose(&mut self, i: usize, a: u16, b: u16, negate: bool, offset: i16) {
        let to = self.target(i, i32::from(offset));
        let next = self.a.label();
        let slow = self.a.label();
        let (equal, differ) = if negate { (next, to) } else { (to, next) };
        self.load_reg(X0, a);
        self.load_reg(X1, b);
        self.guard_ints(X0, X1, slow);
        self.a.cmp(true, X0, X1);
        self.a.bcond(EQ, equal);
        self.a.b(differ);
        self.a.set_cold(true);
        self.a.bind(slow);
        self.generic_jump(i, to, next);
        self.a.set_cold(false);
        self.a.bind(next);
    }

    fn loose_value(&mut self, i: usize, dst: u16, a: u16, b: u16, negate: bool) {
        let slow = self.a.label();
        let done = self.a.label();
        self.load_reg(X0, a);
        self.load_reg(X1, b);
        self.guard_ints(X0, X1, slow);
        self.a.mov_imm64(X2, if negate { TRUE } else { FALSE });
        self.a.mov_imm64(X3, if negate { FALSE } else { TRUE });
        self.a.cmp(true, X0, X1);
        self.a.csel(true, X2, X3, X2, EQ);
        self.store_reg(dst, X2);
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
        self.load_reg(X0, counter);
        self.guard_int(X0, slow);
        self.a.adds_imm(false, X0, X0, 1);
        self.a.bcond(VS, slow);
        self.a.orr(true, X0, X0, INT);
        self.store_reg(counter, X0);
        match limit {
            Src::Reg(limit) => {
                self.load_reg(X1, limit);
                self.guard_int(X1, compare_slow);
                self.a.cmp(false, X0, X1);
            }
            Src::Imm(value) => self.a.cmp_imm(false, X0, i64::from(value.as_int())),
        }
        self.a.bcond(GE, next);
        self.a.bind(taken);
        self.safe_point(self.pc(i) + 1);
        self.a.b(to);
        self.a.set_cold(true);
        // The step the generic way does the test too.
        self.a.bind(slow);
        self.generic_jump(i, taken, next);
        if let Src::Reg(_) = limit {
            // `i` is a stepped int; the limit isn't one.
            self.a.bind(compare_slow);
            let not_number = self.a.label();
            self.a.scvtf(false, D0, X0);
            self.to_double(X1, D1, not_number);
            self.a.fcmp(D0, D1);
            self.a.bcond(MI, taken);
            self.a.b(next);
            self.a.bind(not_number);
            self.a.mov(X0, VM);
            self.a.mov_imm64(X1, u64::from(self.pc(i)));
            self.call_helper(helpers::loop_test as *const () as usize);
            self.taken_or(taken, next);
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
        self.load_reg(X0, counter);
        self.guard_int(X0, slow);
        self.a.adds_imm(false, X0, X0, 1);
        self.a.bcond(VS, slow);
        self.a.orr(true, X0, X0, INT);
        self.store_reg(counter, X0);
        self.load_reg(X1, object);
        self.object_address(X1, test_slow);
        self.a.load8(X10, X1, o.kind as i32);
        self.a.cmp_imm(false, X10, i64::from(o.array_tag));
        self.a.bcond(NE, test_slow);
        self.a.load32(X1, X1, (o.kind + o.array_length) as i32);
        self.a.tbz(true, X1, 31, test_slow);
        self.a.cmp(false, X0, X1);
        self.a.bcond(GE, next);
        self.a.bind(taken);
        self.safe_point(self.pc(i) + 1);
        self.a.b(to);
        self.a.set_cold(true);
        self.a.bind(slow);
        self.generic_jump(i, taken, next);
        // Stepped already: only the test is left, with the length of what isn't an array (a typed array's,
        // `arguments`') by a helper that only finds it if it's quick to, then the helper that does the whole test.
        let full = self.a.label();
        self.a.bind(test_slow);
        self.a.mov(X0, VM);
        self.load_reg(X1, object);
        self.call_helper(helpers::quick_length as *const () as usize);
        self.guard_int(X0, full);
        self.load_reg(X1, counter);
        self.a.cmp(false, X1, X0);
        self.a.bcond(GE, next);
        self.a.b(taken);
        self.a.bind(full);
        self.a.mov(X0, VM);
        self.a.mov_imm64(X1, u64::from(self.pc(i)));
        self.call_helper(helpers::loop_test as *const () as usize);
        self.taken_or(taken, next);
        self.a.set_cold(false);
        self.a.bind(next);
    }

    /// Reads property `cache` of the value in x0 into `out` if an entry of the place knows where it is (on the
    /// object, or on a prototype while none has changed), else on to `slow`: its first entry here, and out of the way
    /// the others it keeps once it's seen several shapes (which GetPropPoly's place has, and GetProp's may have come
    /// to since this was compiled).
    fn get_prop(&mut self, cache: u16, slow: Label, out: R) {
        let l = self.l;
        let holder = self.a.label();
        let read = self.a.label();
        let done = self.a.label();
        let others = self.a.label();
        self.object_address(X0, slow);
        self.a.load32(X9, X0, l.obj.shape as i32);
        self.a.load32(X10, ICS, self.cache(cache, l.c_shape));
        self.a.cmp(false, X9, X10);
        self.a.bcond(NE, others);
        self.a.load32(X10, ICS, self.cache(cache, l.c_other));
        self.a.cmp_imm(false, X10, -1);
        self.a.bcond(NE, holder);
        self.a.bind(read);
        self.a.load32(X10, ICS, self.cache(cache, l.c_slot));
        self.read_slot(X0, X10, out);
        self.a.b(done);
        // On a prototype, as long as no prototype has changed since.
        let was = self.a.is_cold();
        self.a.set_cold(true);
        self.a.bind(holder);
        self.a.load32(X9, ICS, self.cache(cache, l.c_epoch));
        self.a.load32(X11, VM, l.epoch);
        self.a.cmp(false, X9, X11);
        self.a.bcond(NE, slow);
        self.a.mov32(X0, X10);
        self.handle_address(X0);
        self.a.b(read);
        // Another shape than the first entry's (in w9): the entry of the others that knows it, in x12.
        let own = self.a.label();
        self.a.bind(others);
        self.poly_others(cache, slow);
        self.a.load32(X10, X12, l.c_other);
        self.a.cmp_imm(false, X10, -1);
        self.a.bcond(EQ, own);
        self.a.load32(X13, X12, l.c_epoch);
        self.a.load32(X11, VM, l.epoch);
        self.a.cmp(false, X13, X11);
        self.a.bcond(NE, slow);
        self.a.mov32(X0, X10);
        self.handle_address(X0);
        self.a.bind(own);
        self.a.load32(X10, X12, l.c_slot);
        self.read_slot(X0, X10, out);
        self.a.b(done);
        self.a.set_cold(was);
        self.a.bind(done);
    }

    /// With w9 the shape of an object, which isn't the first entry's of place `cache`: leaves in x12 the entry of the
    /// place's [`crate::op::POLY`] others that knows the shape, or goes on to `slow` if the place has never seen a
    /// second shape, has seen too many to keep (what reads remember then is the generic way's), or none knows this
    /// one. Uses x13 and x14.
    fn poly_others(&mut self, cache: u16, slow: Label) {
        let l = self.l;
        let found = self.a.label();
        self.a.add_imm(X12, ICS, i64::from(self.cache(cache, 0)));
        // Where the others are, in entries from this one, with the top bit set once it's megamorphic, or `NONE`.
        self.a.load32(X13, X12, l.c_more);
        self.a.cmp_imm(false, X13, -1);
        self.a.bcond(EQ, slow);
        self.a.tbz(true, X13, 31, slow);
        self.a.mov_imm64(X14, l.cache_size as u64);
        self.a.mul(true, X13, X13, X14);
        self.a.add(true, X12, X12, X13);
        for other in 0..crate::op::POLY {
            if other > 0 {
                self.a.add_imm(X12, X12, i64::from(l.cache_size));
            }
            self.a.load32(X13, X12, l.c_shape);
            self.a.cmp(false, X9, X13);
            self.a.bcond(EQ, found);
        }
        self.a.b(slow);
        self.a.bind(found);
    }

    /// Adds a property to the object at x0 (from register `object`) with the value in register `value`, as place
    /// `cache`'s first entry says (the shape that makes, in its `other`), as SetProp does: as long as no prototype has
    /// changed and the object can be added to, else on to `slow`. A plain object's first few values go inside it, as
    /// `Object::push_slot` puts them, and anything else by a helper.
    fn add_slot(&mut self, cache: u16, object: u16, value: u16, slow: Label, done: Label) {
        let l = self.l;
        let o = l.obj;
        let inside = self.a.label();
        let call = self.a.label();
        self.a.load32(X9, ICS, self.cache(cache, l.c_epoch));
        self.a.load32(X10, VM, l.epoch);
        self.a.cmp(false, X9, X10);
        self.a.bcond(NE, slow);
        self.a.load8(X9, X0, o.flags as i32);
        self.a.tst_imm(false, X9, u64::from(crate::object::NOT_EXTENSIBLE | crate::object::IS_PROTO));
        self.a.bcond(NE, slow);
        self.a.load8(X9, X0, o.slot_count as i32);
        self.a.cmp_imm(false, X9, i64::from(o.inline_count));
        self.a.bcond(HS, call);
        self.a.cbz(false, true, X9, inside);
        self.a.load8(X10, X0, o.kind as i32);
        self.a.cmp_imm(false, X10, i64::from(o.plain_tag));
        self.a.bcond(NE, call);
        self.a.bind(inside);
        self.load_reg(X10, value);
        self.a.add_imm(X11, X0, o.inline_slots as i64);
        self.a.store_index(Size::B64, X10, X11, X9, true);
        self.a.add_imm32(X9, X9, 1);
        self.a.store8(X9, X0, o.slot_count as i32);
        self.a.load32(X10, ICS, self.cache(cache, l.c_other));
        self.a.store32(X10, X0, o.shape as i32);
        self.a.load(X9, VM, l.debt);
        self.a.add_imm(X9, X9, 8);
        self.a.store(X9, VM, l.debt);
        self.a.b(done);
        self.a.bind(call);
        self.a.load32(X2, ICS, self.cache(cache, l.c_other));
        self.a.load32(X1, REGS, self.reg(object));
        self.load_reg(X3, value);
        self.a.mov(X0, VM);
        self.call_helper(helpers::add_slot as *const () as usize);
        self.a.b(done);
    }

    /// After cell w0 has had the value in x1 stored in it: anything but a number may need the cell remembered (the
    /// write barrier), which a helper does out of the way; then on to `done`.
    fn cell_barrier(&mut self, done: Label) {
        let barrier = self.a.label();
        self.a.mov_imm64(X9, OBJECT_TAG);
        self.a.cmp(true, X1, X9);
        self.a.bcond(HS, barrier);
        let was = self.a.is_cold();
        self.a.set_cold(true);
        self.a.bind(barrier);
        self.a.mov(X2, X1);
        self.a.mov32(X1, X0);
        self.a.mov(X0, VM);
        self.call_helper(helpers::cell_barrier as *const () as usize);
        self.a.b(done);
        self.a.set_cold(was);
    }

    /// `for of`'s next value: an array's here, by its count; anything else by the helper, which jumps if it's done, or
    /// for a generator either has the interpreter carry the frame on to its end, which is then gone, or puts the
    /// generator's frame on top for its code to carry on in (see `helpers::iter_next`).
    fn iter_next(&mut self, i: usize, value: u16, iter: u16, offset: i16) {
        let l = self.l;
        let o = l.obj;
        let to = self.target(i, i32::from(offset));
        let next = self.a.label();
        let slow = self.a.label();
        let ended = self.a.label();
        let over = self.a.label();
        self.load_reg(X0, iter);
        self.a.cmp(true, X0, EMPTY_R);
        self.a.bcond(EQ, ended);
        self.load_reg(X1, iter + 1);
        self.guard_int(X1, slow);
        self.object_address(X0, slow);
        self.a.load8(X10, X0, o.kind as i32);
        self.a.cmp_imm(false, X10, i64::from(o.array_tag));
        self.a.bcond(NE, slow);
        self.a.mov32(X2, X1);
        self.a.load32(X9, X0, (o.kind + o.array_length) as i32);
        self.a.cmp(false, X2, X9);
        self.a.bcond(HS, over);
        self.a.load(X3, X0, (o.kind + o.array_start) as i32);
        self.a.load_index(Size::B64, X3, X3, X2, true);
        // A hole is undefined.
        self.a.cmp(true, X3, EMPTY_R);
        self.a.csel(true, X3, UNDEF, X3, EQ);
        self.a.add_imm32(X2, X2, 1);
        self.a.orr(true, X2, X2, INT);
        self.store_reg(iter + 1, X2);
        self.store_reg(value, X3);
        self.a.bind(next);
        self.a.set_cold(true);
        // Past the end: it's finished with.
        self.a.bind(over);
        self.store_reg(iter, EMPTY_R);
        self.a.bind(ended);
        self.store_reg(value, UNDEF);
        self.a.b(to);
        let returned = self.a.label();
        let thrown = self.a.label();
        let resume = self.a.label();
        self.a.bind(slow);
        self.a.mov(X0, VM);
        self.a.mov_imm64(X1, u64::from(self.pc(i)));
        self.call_helper(helpers::iter_next as *const () as usize);
        self.a.cmp_imm(false, X0, i64::from(TAKEN));
        self.a.bcond(EQ, to);
        self.a.cbz(false, false, X0, next);
        self.a.cmp_imm(false, X0, i64::from(helpers::RESUME));
        self.a.bcond(EQ, resume);
        self.a.cmp_imm(false, X0, i64::from(helpers::RETURNED));
        self.a.bcond(EQ, returned);
        self.a.cmp_imm(false, X0, i64::from(helpers::THROWN));
        self.a.bcond(EQ, thrown);
        self.a.b(self.exit);
        // A compiled generator, whose frame the helper has put on top: carried on in its code, which is called as a
        // compiled function is, with its frame's registers and caches. It hands back what it yields, or that it's
        // ended (the helper has put the loop's end in place), or anything else as a call does.
        let entered = self.a.label();
        let yielded = self.a.label();
        self.a.bind(resume);
        self.a.load(X11, VM, l.value);
        self.a.load32(X9, FRAME, l.frame_size + l.f_base);
        self.a.mov_imm64(REGS, self.bases.stack);
        self.a.add_lsl(REGS, REGS, X9, 3);
        self.a.load32(X9, FRAME, l.frame_size + l.f_cache_base);
        self.a.mov_imm64(X10, l.cache_size as u64);
        self.a.mul(true, X9, X9, X10);
        self.a.mov_imm64(ICS, self.bases.caches);
        self.a.add(true, ICS, ICS, X9);
        self.a.add_imm(FRAME, FRAME, i64::from(l.frame_size));
        self.a.bl(entered);
        self.a.add_imm(FRAME, FRAME, -i64::from(l.frame_size));
        self.a.load32(X9, FRAME, l.f_base);
        self.a.mov_imm64(REGS, self.bases.stack);
        self.a.add_lsl(REGS, REGS, X9, 3);
        self.a.mov_imm64(ICS, self.ics);
        self.a.cbz(false, false, X1, yielded);
        self.a.cmp_imm(false, X1, i64::from(helpers::LEFT));
        self.a.bcond(EQ, to);
        self.a.b(self.propagate);
        self.a.bind(yielded);
        self.store_reg(value, X0);
        self.a.b(next);
        // As a compiled function's entry keeps its return address.
        self.a.bind(entered);
        self.a.push_pair(FP, LR);
        self.a.br(X11);
        self.a.bind(returned);
        self.a.load(X0, VM, l.value);
        self.leave(helpers::OK);
        // Its error is its caller's to deal with: a `catch` here was the interpreter's to find.
        self.a.bind(thrown);
        self.a.load(X0, VM, l.value);
        self.leave(helpers::THROW);
        self.a.set_cold(false);
    }

    /// A call of whatever register `f` has (after, for CallMethod, the read of the method into it at place `method`,
    /// from the value in the next): to compiled code directly, when it's an everyday function that's been compiled, a
    /// native by a helper that only calls it, else through `helpers::call`.
    fn call(&mut self, i: usize, dst: u16, f: u16, argc: u16, method: Option<u16>) {
        let l = self.l;
        let done = self.a.label();
        let slow = self.a.label();
        let not_function = self.a.label();
        // Too deep in Rust's stack to call natively: the interpreter makes the call, which needs none.
        let bail = self.bail_stub(i);
        self.a.mov_from_sp(X9);
        self.a.load(X10, VM, l.floor);
        self.a.cmp(true, X9, X10);
        self.a.bcond(LO, bail);
        if let Some(cache) = method {
            // The generic way does only the read, for these.
            let read_slow = self.a.label();
            let read = self.a.label();
            self.load_reg(X0, f + 1);
            self.get_prop(cache, read_slow, X2);
            self.store_reg(f, X2);
            self.a.bind(read);
            self.cold_generic(i, read_slow, read);
        }
        self.safe_point(self.pc(i) + 1);
        self.load_reg(X0, f);
        self.a.mov(X2, X0);
        self.object_address(X0, slow);
        self.a.load8(X9, X0, l.obj.kind as i32);
        self.a.cmp_imm(false, X9, i64::from(l.obj.function_tag));
        self.a.bcond(NE, not_function);
        self.a.load32(X9, X0, (l.obj.kind + l.obj.function_proto) as i32);
        self.a.mov_imm64(X11, l.proto_size as u64);
        self.a.mul(true, X11, X9, X11);
        self.a.mov_imm64(X12, self.bases.protos);
        self.a.add(true, X11, X11, X12);
        // A function that keeps its arguments does that in its prologue.
        self.a.load8(X12, X11, l.p_flags);
        self.a.tst_imm(false, X12, u64::from(IS_CLASS));
        self.a.bcond(NE, slow);
        self.a.mov_imm64(X12, self.bases.slots);
        self.a.load_index(Size::B64, X11, X12, X9, true);
        self.a.cmp_imm(true, X11, CODE_MIN as i64);
        self.a.bcond(LO, slow);
        self.a.and_imm(true, X11, X11, !1);
        // Its captured variables start after their count, if it has any.
        self.a.load(X3, X0, (l.obj.kind + l.obj.function_upvalues) as i32);
        self.a.add_imm(X12, X3, 8);
        self.a.cmp_imm(true, X3, 0);
        self.a.csel(true, X3, X12, X3, NE);
        self.a.mov32(X2, X2);
        self.a.load32(X0, FRAME, l.f_base);
        self.a.add_imm32(X0, X0, i64::from(f) + 1);
        self.a.mov_imm64(X1, u64::from(argc));
        self.a.mov_imm64(X4, u64::from(dst));
        self.a.mov_imm64(X9, u64::from(self.pc(i) + 1));
        self.a.store32(X9, FRAME, l.f_pc);
        let shift = 8 * (i64::from(f) + 1);
        self.a.add_imm(REGS, REGS, shift);
        self.a.add_imm(FRAME, FRAME, i64::from(l.frame_size));
        self.a.blr(X11);
        self.a.add_imm(FRAME, FRAME, -i64::from(l.frame_size));
        self.a.add_imm(REGS, REGS, -shift);
        self.a.mov_imm64(ICS, self.ics);
        self.a.cbz(false, true, X1, self.propagate);
        self.store_reg(dst, X0);
        self.a.bind(done);
        self.a.set_cold(true);
        // A native, by a helper that only calls it: its function, the frame at this call, and the top of the stack
        // where the native may use it, as `call_function!` has them.
        self.a.bind(not_function);
        self.a.cmp_imm(false, X9, i64::from(l.obj.native_tag));
        self.a.bcond(NE, slow);
        self.a.load(X10, X0, (l.obj.kind + l.obj.native_data) as i32);
        self.a.load(X1, X10, l.n_func);
        self.a.mov_imm64(X9, u64::from(self.pc(i) + 1));
        self.a.store32(X9, FRAME, l.f_pc);
        self.a.load32(X9, FRAME, l.f_top);
        self.a.store(X9, VM, l.top);
        self.a.load32(X2, FRAME, l.f_base);
        self.a.mov_imm64(X3, u64::from(f) | u64::from(argc) << 16 | u64::from(dst) << 32);
        self.a.mov(X0, VM);
        self.call_helper(helpers::call_native as *const () as usize);
        self.leave_unless_ok();
        self.a.b(done);
        self.a.bind(slow);
        self.direct(i, helpers::call);
        self.a.b(done);
        self.a.set_cold(false);
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
        self.load_reg(X0, f);
        self.object_address(X0, call);
        self.a.load8(X9, X0, o.kind as i32);
        self.a.cmp_imm(false, X9, i64::from(o.native_tag));
        self.a.bcond(NE, call);
        self.a.load(X10, X0, (o.kind + o.native_data) as i32);
        self.a.load(X10, X10, l.n_func);
        self.a.mov_imm64(X11, math_quick_native(which) as usize as u64);
        self.a.cmp(true, X10, X11);
        self.a.bcond(NE, call);
        self.load_reg(X0, f + 2);
        if argc == 2 {
            self.load_reg(X1, f + 3);
        }
        // Ints, where the answer is quick to tell.
        match name {
            "floor" | "ceil" | "trunc" | "round" => {
                self.guard_int(X0, doubles);
                self.store_reg(dst, X0);
                self.a.b(done);
            }
            "abs" => {
                self.guard_int(X0, doubles);
                // The most negative int's is a double.
                self.a.negs(false, X2, X0);
                self.a.bcond(VS, doubles);
                self.a.cmp_imm(false, X0, 0);
                self.a.csel(false, X2, X0, X2, GE);
                self.a.orr(true, X2, X2, INT);
                self.store_reg(dst, X2);
                self.a.b(done);
            }
            "min" | "max" | "imul" => {
                self.guard_ints(X0, X1, doubles);
                if name == "imul" {
                    self.a.mul(false, X0, X0, X1);
                } else {
                    self.a.cmp(false, X0, X1);
                    self.a.csel(false, X0, X0, X1, if name == "min" { LT } else { GT });
                }
                self.a.orr(true, X0, X0, INT);
                self.store_reg(dst, X0);
                self.a.b(done);
            }
            _ => {}
        }
        // Doubles (and ints, as doubles).
        self.a.bind(doubles);
        self.to_double(X0, D0, call);
        if argc == 2 {
            self.to_double(X1, D1, call);
        }
        match name {
            "sqrt" => self.a.fop1(Fop1::Sqrt, D0, D0),
            "abs" => self.a.fop1(Fop1::Abs, D0, D0),
            "floor" => self.a.fop1(Fop1::RoundDown, D0, D0),
            "ceil" => self.a.fop1(Fop1::RoundUp, D0, D0),
            "trunc" => self.a.fop1(Fop1::RoundZero, D0, D0),
            "fround" => {
                self.a.fcvt_to_single(D0, D0);
                self.a.fcvt_to_double(D0, D0);
            }
            // NaN if either is, and -0 the smaller of two zeros, as JavaScript has them.
            "min" => self.a.fop(Fop::Min, D0, D0, D1),
            "max" => self.a.fop(Fop::Max, D0, D0, D1),
            _ => {
                // `math_quick_apply(which, x, y)`, with the numbers where the calling convention has them.
                self.a.mov_imm64(X0, u64::from(which));
                self.call_helper(math_quick_apply as *const () as usize);
            }
        }
        self.a.bind(result);
        self.store_number(dst, D0);
        self.a.b(done);
        // What it calls isn't the built-in function, or not with numbers: the call, which the helper makes a Call
        // from now on for the interpreter.
        self.a.bind(call);
        self.call(i, dst, f, argc, None);
        self.a.bind(done);
    }

    /// The generator stops: `helper` has the frame go, and what it hands back (in `vm.jit.value`) is returned.
    fn stop_here(&mut self, i: usize, helper: Direct) {
        self.direct(i, helper);
        self.a.load(X0, VM, self.l.value);
        self.leave(helpers::OK);
    }

    /// Returns what's in x0, as `finish_return!` does: the frame goes, and the caller gets it.
    fn ret(&mut self, i: usize) {
        let l = self.l;
        let special = self.a.label();
        let finish = self.a.label();
        let helper = self.a.label();
        self.a.load8(X9, FRAME, l.f_flags);
        self.a.cbz(false, true, X9, special);
        self.a.bind(finish);
        self.a.load(X9, VM, l.frames_len);
        self.a.add_imm(X9, X9, -1);
        self.a.store(X9, VM, l.frames_len);
        self.leave(helpers::OK);
        self.a.set_cold(true);
        // A `new` hands back what was returned if it's an object, else its `this`, which a derived class's has to
        // have made by now.
        self.a.bind(special);
        self.a.cmp_imm(false, X9, i64::from(crate::vm::CONSTRUCTING));
        self.a.bcond(NE, helper);
        self.tag_of(X0);
        self.cmp_tag(TAG_OBJECT);
        self.a.bcond(EQ, finish);
        self.load_reg(X0, 0);
        self.a.cmp(true, X0, EMPTY_R);
        self.a.bcond(NE, finish);
        self.a.bind(helper);
        // What it throws is thrown at this instruction, as far as a `try` around it is concerned.
        self.a.mov_imm64(X9, u64::from(self.pc(i) + 1));
        self.a.store32(X9, FRAME, l.f_pc);
        self.a.mov(X1, X0);
        self.a.mov(X0, VM);
        self.call_helper(helpers::finish_return as *const () as usize);
        self.leave_unless_ok();
        self.a.load(X0, VM, l.value);
        self.a.b(finish);
        self.a.set_cold(false);
    }
}

fn int(value: i16) -> Src {
    Src::Imm(Value::int(value.into()))
}
