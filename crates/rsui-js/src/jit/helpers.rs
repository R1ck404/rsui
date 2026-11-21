//! What compiled code calls for anything it doesn't do itself: each instruction the way `Vm::run_loop` does it, calls,
//! collections, and making room for a frame.
//!
//! Compiled code only does the usual case of an instruction itself (ints, doubles, a property a cache knows) and calls
//! [`op`] for everything else, which does the whole instruction again from the start, as the interpreter would. So
//! what each instruction means is said twice: in `run_loop` and here. Both call the same functions for the unusual
//! cases (`add_slow`, `get_cached`, ...), so only the glue between them is repeated, and a change to what an
//! instruction does has to be made in both places.
//!
//! Every helper is called with the frame of the function that's running on top, and says how it went: [`OK`],
//! [`THROW`] with the error in `vm.jit.value`, [`BAIL`] to have the interpreter carry on in that frame from where its
//! `pc` now is, or for a jump, [`TAKEN`].
#![allow(unsafe_code)]

use crate::object::*;
use crate::op::*;
use crate::text::{self, Atom, AtomKind};
use crate::value::Value;
use crate::vm::{Args, Frame, Vm};

/// Declares a function compiled code calls: with Windows' convention on x86-64, on every system (so that x64.rs has
/// one convention), and with the system's own elsewhere (AAPCS64 on Arm, which arm64.rs calls with).
macro_rules! jit_abi {
    ($(#[$meta:meta])* $vis:vis fn $name:ident($($params:tt)*) $(-> $ret:ty)? $body:block) => {
        #[cfg(target_arch = "x86_64")]
        $(#[$meta])*
        $vis extern "win64" fn $name($($params)*) $(-> $ret)? $body
        #[cfg(not(target_arch = "x86_64"))]
        $(#[$meta])*
        $vis extern "C" fn $name($($params)*) $(-> $ret)? $body
    };
}

pub(crate) const OK: u32 = 0;
pub(crate) const BAIL: u32 = 1;
pub(crate) const THROW: u32 = 2;
pub(crate) const TAKEN: u32 = 3;
/// The interpreter carried the frame on to its end, which returned the value in `vm.jit.value`: the frame is gone.
pub(crate) const RETURNED: u32 = 4;
/// The same, but it threw the error in `vm.jit.value`, and the frame is gone too.
pub(crate) const THROWN: u32 = 5;
/// The trampoline didn't go in, as Rust's stack is already below `Jit::floor`: the interpreter runs the frame.
pub(crate) const NOT_ENTERED: u32 = 6;
/// The frame has gone, and the interpreter carries on with the one on top from where it says: a generator the
/// interpreter's `for of` carried on in place has come to its end, so that loop has ended (see
/// `Vm::end_resumed_generator`).
pub(crate) const LEFT: u32 = 7;
/// A generator's frame has been put on top, for compiled code to carry it on in its own code, from where
/// `vm.jit.value` says (see `iter_next`).
pub(crate) const RESUME: u32 = 8;

/// The frame on top, after saying the instruction at `pc` is the one running, as `save_pc!` does, and with the top of
/// the stack where a native may use it, as `attempt!` does.
#[inline]
fn here(vm: &mut Vm, pc: u32) -> Frame {
    let frame = vm.frames.last_mut().expect("compiled code runs in a frame");
    frame.pc = pc + 1;
    let frame = *frame;
    vm.top = frame.top as usize;
    #[cfg(feature = "jsprof")]
    crate::jsprof::maybe_sample(vm);
    frame
}

#[cold]
fn throw(vm: &mut Vm, error: Value) -> u32 {
    vm.jit.value = error;
    THROW
}

/// What a bitwise operator makes of a number, as in `vm.rs`.
#[inline]
fn quick_int32(value: Value) -> Option<i32> {
    if value.is_int() {
        Some(value.as_int())
    } else if value.is_double() && value.as_double().abs() < 9_223_372_036_854_775_808.0 {
        Some(value.as_double() as i64 as i32)
    } else {
        None
    }
}

#[derive(Clone, Copy)]
enum Arith {
    Add,
    Sub,
}

fn arithmetic(vm: &mut Vm, x: Value, y: Value, kind: Arith) -> crate::vm::Run<Value> {
    Ok(if Value::both_ints(x, y) {
        let (a, b) = (x.as_int(), y.as_int());
        match kind {
            Arith::Add => match a.checked_add(b) {
                Some(value) => Value::int(value),
                None => Value::computed(f64::from(a) + f64::from(b)),
            },
            Arith::Sub => match a.checked_sub(b) {
                Some(value) => Value::int(value),
                None => Value::computed(f64::from(a) - f64::from(b)),
            },
        }
    } else if x.is_number() && y.is_number() {
        match kind {
            Arith::Add => Value::computed(x.as_number() + y.as_number()),
            Arith::Sub => Value::computed(x.as_number() - y.as_number()),
        }
    } else {
        match kind {
            Arith::Add => vm.add_slow(x, y)?,
            Arith::Sub => vm.sub_slow(x, y)?,
        }
    })
}

fn multiply(vm: &mut Vm, x: Value, y: Value) -> crate::vm::Run<Value> {
    Ok(if Value::both_ints(x, y) {
        match x.as_int().checked_mul(y.as_int()) {
            Some(value) if value != 0 => Value::int(value),
            _ => Value::computed(f64::from(x.as_int()) * f64::from(y.as_int())),
        }
    } else if x.is_number() && y.is_number() {
        Value::computed(x.as_number() * y.as_number())
    } else {
        vm.mul_slow(x, y)?
    })
}

fn divide(vm: &mut Vm, x: Value, y: Value) -> crate::vm::Run<Value> {
    if x.is_number() && y.is_number() {
        Ok(Value::number(x.as_number() / y.as_number()))
    } else {
        vm.div_slow(x, y)
    }
}

fn remainder(vm: &mut Vm, x: Value, y: Value) -> crate::vm::Run<Value> {
    if Value::both_ints(x, y) && x.as_int() >= 0 && y.as_int() > 0 {
        Ok(Value::int(x.as_int() % y.as_int()))
    } else if x.is_number() && y.is_number() {
        Ok(Value::number(x.as_number() % y.as_number()))
    } else {
        vm.mod_slow(x, y)
    }
}

fn bitwise(vm: &mut Vm, x: Value, y: Value, op: crate::builtins::BigOp) -> crate::vm::Run<Value> {
    use crate::builtins::BigOp;
    let (a, b) = match (quick_int32(x), quick_int32(y)) {
        (Some(a), Some(b)) => (a, b),
        _ => return vm.bitwise_slow(op, x, y),
    };
    Ok(match op {
        BigOp::And => Value::int(a & b),
        BigOp::Or => Value::int(a | b),
        BigOp::Xor => Value::int(a ^ b),
        BigOp::Shl => Value::int(a.wrapping_shl(b as u32 & 31)),
        BigOp::Shr => Value::int(a.wrapping_shr(b as u32 & 31)),
        BigOp::UShr => Value::int_or_double((a as u32).wrapping_shr(b as u32 & 31) as usize),
        _ => unreachable!("only bitwise operators come here"),
    })
}

type Compare = fn(&mut Vm, Value, Value) -> crate::vm::Run<bool>;

#[derive(Clone, Copy)]
enum Rel {
    Lt,
    Le,
    Gt,
    Ge,
}

fn relation(vm: &mut Vm, x: Value, y: Value, rel: Rel) -> crate::vm::Run<bool> {
    if Value::both_ints(x, y) {
        let (a, b) = (x.as_int(), y.as_int());
        return Ok(match rel {
            Rel::Lt => a < b,
            Rel::Le => a <= b,
            Rel::Gt => a > b,
            Rel::Ge => a >= b,
        });
    }
    if x.is_number() && y.is_number() {
        let (a, b) = (x.as_number(), y.as_number());
        return Ok(match rel {
            Rel::Lt => a < b,
            Rel::Le => a <= b,
            Rel::Gt => a > b,
            Rel::Ge => a >= b,
        });
    }
    let slow: Compare = match rel {
        Rel::Lt => Vm::less_than,
        Rel::Le => Vm::less_equal,
        Rel::Gt => Vm::greater_than,
        Rel::Ge => Vm::greater_equal,
    };
    slow(vm, x, y)
}

fn relation_int(vm: &mut Vm, x: Value, value: i16, rel: Rel) -> crate::vm::Run<bool> {
    let value = i32::from(value);
    if x.is_int() {
        let a = x.as_int();
        return Ok(match rel {
            Rel::Lt => a < value,
            Rel::Le => a <= value,
            Rel::Gt => a > value,
            Rel::Ge => a >= value,
        });
    }
    if x.is_double() {
        let (a, b) = (x.as_double(), f64::from(value));
        return Ok(match rel {
            Rel::Lt => a < b,
            Rel::Le => a <= b,
            Rel::Gt => a > b,
            Rel::Ge => a >= b,
        });
    }
    relation(vm, x, Value::int(value), rel)
}

fn strict_equal(vm: &Vm, x: Value, y: Value) -> bool {
    if x == y {
        !x.is_double() || !x.as_double().is_nan()
    } else if Value::both_ints(x, y) || !x.has_other_forms() || !y.has_other_forms() {
        false
    } else {
        vm.strict_equals(x, y)
    }
}

fn loose_equal(vm: &mut Vm, x: Value, y: Value) -> crate::vm::Run<bool> {
    if Value::both_ints(x, y) {
        Ok(x == y)
    } else {
        vm.loose_equals(x, y)
    }
}

fn strict_equal_int(x: Value, value: i16) -> bool {
    x == Value::int(i32::from(value)) || (x.is_double() && x.as_double() == f64::from(value))
}

fn step(vm: &mut Vm, x: Value, by: i32) -> crate::vm::Run<Value> {
    if by > 0 && x.is_int() && x.as_int() != i32::MAX {
        Ok(Value::int(x.as_int() + 1))
    } else if by < 0 && x.is_int() && x.as_int() != i32::MIN {
        Ok(Value::int(x.as_int() - 1))
    } else {
        vm.step_slow(x, by)
    }
}

/// Reads a property as GetProp does: the place's first entry, else `get_cached`.
fn get_prop(vm: &mut Vm, target: Value, cache: usize) -> crate::vm::Run<Value> {
    if target.is_object() {
        let entry = vm.caches[cache];
        let found = &vm.objects[target.index()];
        if found.shape == entry.shape {
            if entry.other == NONE {
                // SAFETY: a cache entry that matches an object's shape names a slot that shape has.
                return Ok(unsafe { found.read_slot(entry.slot as usize) });
            }
            if entry.epoch == vm.epoch {
                // SAFETY: as above, for the object the shape found it on.
                return Ok(unsafe { vm.objects[entry.other as usize].read_slot(entry.slot as usize) });
            }
        }
    }
    vm.get_cached(target, cache)
}

/// Reads a property as GetPropPoly does: the place's first entry, then its others, then what reads remember.
fn get_prop_poly(vm: &mut Vm, target: Value, cache: usize) -> crate::vm::Run<Value> {
    'slow: {
        if !target.is_object() {
            break 'slow;
        }
        let shape = vm.objects[target.index()].shape;
        let first = vm.caches[cache];
        let known = 'find: {
            if first.shape == shape {
                break 'find first;
            }
            if first.more & MEGAMORPHIC == 0 {
                let more = cache + first.more as usize;
                for at in more..more + POLY {
                    if vm.caches[at].shape == shape {
                        break 'find vm.caches[at];
                    }
                }
                break 'slow;
            }
            if vm.objects[target.index()].flags & LAZY_FUNCTION == 0
                && let Some((holder, slot)) = vm.remembered_value(shape, first.atom)
            {
                let epoch = vm.epoch;
                let entry = &mut vm.caches[cache];
                entry.shape = shape;
                entry.other = holder;
                entry.slot = slot;
                entry.epoch = epoch;
                let holder = if holder == NONE { target.handle() } else { holder };
                // SAFETY: what reads remember names a slot the shape has, on the object it was found on.
                return Ok(unsafe { vm.objects[holder as usize].read_slot(slot as usize) });
            }
            break 'slow;
        };
        if known.other == NONE {
            // SAFETY: a cache entry that matches an object's shape names a slot that shape has.
            return Ok(unsafe { vm.objects[target.index()].read_slot(known.slot as usize) });
        }
        if known.epoch == vm.epoch {
            // SAFETY: as above.
            return Ok(unsafe { vm.objects[known.other as usize].read_slot(known.slot as usize) });
        }
    }
    vm.get_cached(target, cache)
}

/// Writes a property as SetProp does.
fn set_prop(vm: &mut Vm, target: Value, value: Value, cache: usize) -> crate::vm::Run<()> {
    if target.is_object() {
        let entry = vm.caches[cache];
        let epoch = vm.epoch;
        // Not through `objects[..]`, which would remember an old object: that has to go the slow way.
        let found = unsafe { &mut *vm.objects.as_mut_ptr().add(target.index()) };
        if found.shape == entry.shape && found.flags & crate::gc::OLD == 0 {
            if entry.other == NONE {
                // SAFETY: a cache entry that matches an object's shape names a slot that shape has.
                unsafe { found.write_slot(entry.slot as usize, value) };
                return Ok(());
            }
            if entry.epoch == epoch && found.flags & (NOT_EXTENSIBLE | IS_PROTO) == 0 {
                found.shape = entry.other;
                found.push_slot(value);
                vm.debt += 8;
                return Ok(());
            }
        }
    }
    vm.set_cached(target, value, cache)
}

/// Writes a property as SetPropPoly does.
fn set_prop_poly(vm: &mut Vm, target: Value, value: Value, cache: usize) -> crate::vm::Run<()> {
    if target.is_object() {
        let epoch = vm.epoch;
        let first = vm.caches[cache];
        let found = unsafe { &mut *vm.objects.as_mut_ptr().add(target.index()) };
        let known = 'find: {
            if found.flags & crate::gc::OLD != 0 {
                break 'find None;
            }
            if first.shape == found.shape {
                break 'find Some(first);
            }
            if first.more & MEGAMORPHIC != 0 {
                if found.flags & LAZY_FUNCTION == 0
                    && let Some((to, slot)) = vm.remembered_store(found.shape, first.atom)
                {
                    if to == NONE {
                        // SAFETY: what writes remember names a slot the shape has.
                        unsafe { found.write_slot(slot as usize, value) };
                        return Ok(());
                    }
                    if found.flags & (NOT_EXTENSIBLE | IS_PROTO) == 0 {
                        found.shape = to;
                        found.push_slot(value);
                        vm.debt += 8;
                        return Ok(());
                    }
                }
                break 'find None;
            }
            let more = cache + first.more as usize;
            (more..more + POLY)
                .map(|at| vm.caches[at])
                .find(|entry| entry.shape == found.shape)
        };
        if let Some(known) = known {
            if known.other == NONE {
                // SAFETY: a cache entry that matches an object's shape names a slot that shape has.
                unsafe { found.write_slot(known.slot as usize, value) };
                return Ok(());
            }
            if known.epoch == epoch && found.flags & (NOT_EXTENSIBLE | IS_PROTO) == 0 {
                found.shape = known.other;
                found.push_slot(value);
                vm.debt += 8;
                return Ok(());
            }
        }
    }
    vm.set_cached(target, value, cache)
}

/// The length that `IncLoopLtLength` compares with, as the interpreter finds it: an array's here, anything else by
/// `loop_length` (which knows `arguments`, typed arrays and strings).
fn length_of(vm: &mut Vm, target: Value) -> crate::vm::Run<Value> {
    if target.is_object()
        && let Kind::Array(items) = &vm.objects[target.index()].kind
    {
        return Ok(Value::int_or_double(items.len()));
    }
    vm.loop_length(target)
}

/// An error from a write that code that isn't strict ignores.
fn forgiven(vm: &mut Vm, result: crate::vm::Run<()>) -> u32 {
    match result {
        Ok(()) => OK,
        Err(error) => {
            if vm.forgive_refusal() {
                OK
            } else {
                throw(vm, error)
            }
        }
    }
}

jit_abi! {
/// Does the instruction at `pc`, in the frame on top, as the interpreter would, all of it. For CallMethod and
/// CallMethodPoly it's only the read of the method, which [`call`] then calls.
pub(crate) fn op(vm: &mut Vm, pc: u32) -> u32 {
    use crate::builtins::BigOp;
    #[cfg(feature = "jit-opt")]
    super::feedback::record(vm, pc);
    // Matched through a reference, so that each instruction's fields are read where they're used, as in
    // `run_loop`. Copying the instruction first has them all read before the match.
    // SAFETY: compiled code only passes the place of one of its own instructions, and the list never moves.
    let op: &Op = unsafe { &*vm.code.as_ptr().add(pc as usize) };
    if vm.jit.counting {
        vm.jit.count_generic(op);
        count_read(vm, op);
    }
    let frame = here(vm, pc);
    let base = frame.base as usize;
    let cache_index = |cache: u16| frame.cache_base as usize + cache as usize;
    macro_rules! reg {
        ($index:expr) => {
            vm.stack[base + $index as usize]
        };
    }
    macro_rules! attempt {
        ($result:expr) => {
            match $result {
                Ok(value) => value,
                Err(error) => return throw(vm, error),
            }
        };
    }
    macro_rules! jump_if {
        ($condition:expr) => {
            return if $condition { TAKEN } else { OK }
        };
    }
    macro_rules! konst {
        ($k:expr) => {
            vm.consts[$k as usize]
        };
    }
    match *op {
        Op::LoadConst { dst, index } => reg!(dst) = konst!(index),
        Op::LoadInt { dst, value } => reg!(dst) = Value::int(value),
        Op::LoadUndefined { dst } => reg!(dst) = Value::UNDEFINED,
        Op::LoadNull { dst } => reg!(dst) = Value::NULL,
        Op::LoadTrue { dst } => reg!(dst) = Value::TRUE,
        Op::LoadFalse { dst } => reg!(dst) = Value::FALSE,
        Op::LoadEmpty { dst } => reg!(dst) = Value::EMPTY,
        Op::Move { dst, src } => reg!(dst) = reg!(src),

        Op::Add { dst, a, b } => reg!(dst) = attempt!(arithmetic(vm, reg!(a), reg!(b), Arith::Add)),
        Op::Sub { dst, a, b } => reg!(dst) = attempt!(arithmetic(vm, reg!(a), reg!(b), Arith::Sub)),
        Op::AddInt { dst, a, value } => {
            let x = reg!(a);
            reg!(dst) = if x.is_int() {
                match x.as_int().checked_add(i32::from(value)) {
                    Some(sum) => Value::int(sum),
                    None => Value::computed(f64::from(x.as_int()) + f64::from(value)),
                }
            } else if x.is_double() {
                Value::computed(x.as_double() + f64::from(value))
            } else {
                attempt!(vm.add_slow(x, Value::int(i32::from(value))))
            };
        }
        Op::AddInt32 { dst, a, b } => {
            let (x, y) = (reg!(a), reg!(b));
            reg!(dst) = if Value::both_ints(x, y) {
                Value::int(x.as_int().wrapping_add(y.as_int()))
            } else {
                attempt!(vm.add_int32_slow(x, y))
            };
        }
        Op::DivInt { dst, a, value } => {
            let (x, by) = (reg!(a), i32::from(value));
            // A power of two above zero: what it divides exactly stays an int, of the same sign (0 too).
            reg!(dst) = if x.is_int() && x.as_int() & (by - 1) == 0 {
                Value::int(x.as_int() >> by.trailing_zeros())
            } else if x.is_number() {
                Value::number(x.as_number() / f64::from(by))
            } else {
                attempt!(vm.div_slow(x, Value::int(by)))
            };
        }
        Op::SubInt { dst, a, value } => {
            let x = reg!(a);
            reg!(dst) = if x.is_int() {
                match x.as_int().checked_sub(i32::from(value)) {
                    Some(sum) => Value::int(sum),
                    None => Value::computed(f64::from(x.as_int()) - f64::from(value)),
                }
            } else if x.is_double() {
                Value::computed(x.as_double() - f64::from(value))
            } else {
                attempt!(vm.sub_slow(x, Value::int(i32::from(value))))
            };
        }
        Op::MulInt { dst, a, value } => {
            reg!(dst) = attempt!(multiply(vm, reg!(a), Value::int(i32::from(value))))
        }
        Op::ModInt { dst, a, value } => {
            reg!(dst) = attempt!(remainder(vm, reg!(a), Value::int(i32::from(value))))
        }
        Op::BitAndInt { dst, a, value } => {
            reg!(dst) = attempt!(bitwise(vm, reg!(a), Value::int(i32::from(value)), BigOp::And))
        }
        Op::BitOrInt { dst, a, value } => {
            reg!(dst) = attempt!(bitwise(vm, reg!(a), Value::int(i32::from(value)), BigOp::Or))
        }
        Op::ShlInt { dst, a, value } => {
            reg!(dst) = attempt!(bitwise(vm, reg!(a), Value::int(i32::from(value)), BigOp::Shl))
        }
        Op::ShrInt { dst, a, value } => {
            reg!(dst) = attempt!(bitwise(vm, reg!(a), Value::int(i32::from(value)), BigOp::Shr))
        }
        Op::UShrInt { dst, a, value } => {
            reg!(dst) = attempt!(bitwise(vm, reg!(a), Value::int(i32::from(value)), BigOp::UShr))
        }
        Op::AddK { dst, a, k } => {
            reg!(dst) = attempt!(arithmetic(vm, reg!(a), konst!(k), Arith::Add))
        }
        Op::SubK { dst, a, k } => {
            reg!(dst) = attempt!(arithmetic(vm, reg!(a), konst!(k), Arith::Sub))
        }
        Op::MulK { dst, a, k } => reg!(dst) = attempt!(multiply(vm, reg!(a), konst!(k))),
        Op::DivK { dst, a, k } => reg!(dst) = attempt!(divide(vm, reg!(a), konst!(k))),
        Op::ModK { dst, a, k } => reg!(dst) = attempt!(remainder(vm, reg!(a), konst!(k))),
        Op::Mul { dst, a, b } => reg!(dst) = attempt!(multiply(vm, reg!(a), reg!(b))),
        Op::Div { dst, a, b } => reg!(dst) = attempt!(divide(vm, reg!(a), reg!(b))),
        Op::Mod { dst, a, b } => reg!(dst) = attempt!(remainder(vm, reg!(a), reg!(b))),
        Op::Pow { dst, a, b } => {
            let (x, y) = (reg!(a), reg!(b));
            reg!(dst) = attempt!(vm.pow_slow(x, y));
        }
        Op::BitAnd { dst, a, b } => reg!(dst) = attempt!(bitwise(vm, reg!(a), reg!(b), BigOp::And)),
        Op::BitOr { dst, a, b } => reg!(dst) = attempt!(bitwise(vm, reg!(a), reg!(b), BigOp::Or)),
        Op::BitXor { dst, a, b } => reg!(dst) = attempt!(bitwise(vm, reg!(a), reg!(b), BigOp::Xor)),
        Op::Shl { dst, a, b } => reg!(dst) = attempt!(bitwise(vm, reg!(a), reg!(b), BigOp::Shl)),
        Op::Shr { dst, a, b } => reg!(dst) = attempt!(bitwise(vm, reg!(a), reg!(b), BigOp::Shr)),
        Op::UShr { dst, a, b } => reg!(dst) = attempt!(bitwise(vm, reg!(a), reg!(b), BigOp::UShr)),
        Op::Lt { dst, a, b } => {
            reg!(dst) = Value::bool(attempt!(relation(vm, reg!(a), reg!(b), Rel::Lt)))
        }
        Op::Le { dst, a, b } => {
            reg!(dst) = Value::bool(attempt!(relation(vm, reg!(a), reg!(b), Rel::Le)))
        }
        Op::Gt { dst, a, b } => {
            reg!(dst) = Value::bool(attempt!(relation(vm, reg!(a), reg!(b), Rel::Gt)))
        }
        Op::Ge { dst, a, b } => {
            reg!(dst) = Value::bool(attempt!(relation(vm, reg!(a), reg!(b), Rel::Ge)))
        }
        Op::Eq { dst, a, b } => reg!(dst) = Value::bool(attempt!(loose_equal(vm, reg!(a), reg!(b)))),
        Op::Ne { dst, a, b } => {
            reg!(dst) = Value::bool(!attempt!(loose_equal(vm, reg!(a), reg!(b))))
        }
        Op::StrictEq { dst, a, b } => reg!(dst) = Value::bool(strict_equal(vm, reg!(a), reg!(b))),
        Op::StrictNe { dst, a, b } => reg!(dst) = Value::bool(!strict_equal(vm, reg!(a), reg!(b))),
        Op::In { dst, a, b } => {
            let (key, object) = (reg!(a), reg!(b));
            reg!(dst) = Value::bool(attempt!(vm.has_in(key, object)));
        }
        Op::InstanceOf { dst, a, b } => {
            let (value, target) = (reg!(a), reg!(b));
            reg!(dst) = Value::bool(attempt!(vm.instance_of(value, target)));
        }

        Op::Neg { dst, src } => {
            let x = reg!(src);
            reg!(dst) = if x.is_int() && x.as_int() != 0 && x.as_int() != i32::MIN {
                Value::int(-x.as_int())
            } else {
                attempt!(vm.negate_slow(x))
            };
        }
        Op::ToNumber { dst, src } => {
            let x = reg!(src);
            reg!(dst) = if x.is_number() {
                x
            } else {
                Value::number(attempt!(vm.to_number(x)))
            };
        }
        Op::ToNumeric { dst, src } => {
            let x = reg!(src);
            reg!(dst) = if x.is_number() {
                x
            } else {
                attempt!(vm.to_numeric(x))
            };
        }
        Op::Not { dst, src } => {
            let truth = vm.to_boolean(reg!(src));
            reg!(dst) = Value::bool(!truth);
        }
        Op::BitNot { dst, src } => {
            let x = reg!(src);
            if let Some(value) = quick_int32(x) {
                reg!(dst) = Value::int(!value);
                return OK;
            }
            let numeric = attempt!(vm.to_numeric(x));
            reg!(dst) = if numeric.is_bigint() {
                vm.bigint_not(numeric)
            } else {
                Value::int(!attempt!(vm.to_int32(numeric)))
            };
        }
        Op::TypeOf { dst, src } => {
            let kind = vm.type_of(reg!(src));
            reg!(dst) = kind;
        }
        Op::Inc { dst, src } => reg!(dst) = attempt!(step(vm, reg!(src), 1)),
        Op::Dec { dst, src } => reg!(dst) = attempt!(step(vm, reg!(src), -1)),
        Op::ToKey { dst, src } => {
            let key = reg!(src);
            reg!(dst) = if !key.is_string() && !key.is_symbol() && !key.is_int() {
                let atom = attempt!(vm.to_atom(key));
                vm.atom_value(atom)
            } else {
                key
            };
        }
        Op::Concat { dst, start, count } => {
            reg!(dst) = attempt!(vm.concat_registers(base + start as usize, count as usize));
        }

        Op::JumpIfTrue { condition, .. } => jump_if!(vm.to_boolean(reg!(condition))),
        Op::JumpIfFalse { condition, .. } => jump_if!(!vm.to_boolean(reg!(condition))),
        Op::JumpIfNullish { condition, .. } => jump_if!(reg!(condition).is_nullish()),
        Op::JumpIfNotNullish { condition, .. } => jump_if!(!reg!(condition).is_nullish()),
        Op::JumpIfNotUndefined { condition, .. } => jump_if!(!reg!(condition).is_undefined()),
        Op::JumpIfUndefined { condition, .. } => jump_if!(reg!(condition).is_undefined()),
        Op::JumpIfNull { condition, .. } => jump_if!(reg!(condition).is_null()),
        Op::JumpIfNotNull { condition, .. } => jump_if!(!reg!(condition).is_null()),
        Op::JumpLt { a, b, .. } => jump_if!(attempt!(relation(vm, reg!(a), reg!(b), Rel::Lt))),
        Op::JumpLe { a, b, .. } => jump_if!(attempt!(relation(vm, reg!(a), reg!(b), Rel::Le))),
        Op::JumpGt { a, b, .. } => jump_if!(attempt!(relation(vm, reg!(a), reg!(b), Rel::Gt))),
        Op::JumpGe { a, b, .. } => jump_if!(attempt!(relation(vm, reg!(a), reg!(b), Rel::Ge))),
        Op::JumpNotLt { a, b, .. } => jump_if!(!attempt!(relation(vm, reg!(a), reg!(b), Rel::Lt))),
        Op::JumpNotLe { a, b, .. } => jump_if!(!attempt!(relation(vm, reg!(a), reg!(b), Rel::Le))),
        Op::JumpNotGt { a, b, .. } => jump_if!(!attempt!(relation(vm, reg!(a), reg!(b), Rel::Gt))),
        Op::JumpNotGe { a, b, .. } => jump_if!(!attempt!(relation(vm, reg!(a), reg!(b), Rel::Ge))),
        Op::JumpStrictEq { a, b, .. } => jump_if!(strict_equal(vm, reg!(a), reg!(b))),
        Op::JumpStrictNe { a, b, .. } => jump_if!(!strict_equal(vm, reg!(a), reg!(b))),
        Op::JumpEq { a, b, .. } => jump_if!(attempt!(loose_equal(vm, reg!(a), reg!(b)))),
        Op::JumpNe { a, b, .. } => jump_if!(!attempt!(loose_equal(vm, reg!(a), reg!(b)))),
        Op::JumpLtInt { a, value, .. } => jump_if!(attempt!(relation_int(vm, reg!(a), value, Rel::Lt))),
        Op::JumpLeInt { a, value, .. } => jump_if!(attempt!(relation_int(vm, reg!(a), value, Rel::Le))),
        Op::JumpGtInt { a, value, .. } => jump_if!(attempt!(relation_int(vm, reg!(a), value, Rel::Gt))),
        Op::JumpGeInt { a, value, .. } => jump_if!(attempt!(relation_int(vm, reg!(a), value, Rel::Ge))),
        Op::JumpNotLtInt { a, value, .. } => {
            jump_if!(!attempt!(relation_int(vm, reg!(a), value, Rel::Lt)))
        }
        Op::JumpNotLeInt { a, value, .. } => {
            jump_if!(!attempt!(relation_int(vm, reg!(a), value, Rel::Le)))
        }
        Op::JumpNotGtInt { a, value, .. } => {
            jump_if!(!attempt!(relation_int(vm, reg!(a), value, Rel::Gt)))
        }
        Op::JumpNotGeInt { a, value, .. } => {
            jump_if!(!attempt!(relation_int(vm, reg!(a), value, Rel::Ge)))
        }
        Op::JumpStrictEqInt { a, value, .. } => jump_if!(strict_equal_int(reg!(a), value)),
        Op::JumpStrictNeInt { a, value, .. } => jump_if!(!strict_equal_int(reg!(a), value)),
        Op::JumpTypeIs { a, kind, .. } => jump_if!(vm.type_kind(reg!(a)) == kind),
        Op::JumpTypeIsNot { a, kind, .. } => jump_if!(vm.type_kind(reg!(a)) != kind),
        // The loops' own instructions: the step and the test. Compiled code collects garbage and jumps after.
        Op::IncLoopLt { i, limit, .. } => {
            reg!(i) = attempt!(step(vm, reg!(i), 1));
            jump_if!(attempt!(relation(vm, reg!(i), reg!(limit), Rel::Lt)));
        }
        Op::IncLoopLtInt { i, limit, .. } => {
            reg!(i) = attempt!(step(vm, reg!(i), 1));
            jump_if!(attempt!(relation_int(vm, reg!(i), limit, Rel::Lt)));
        }
        Op::IncLoopLtLength { i, object, .. } => {
            reg!(i) = attempt!(step(vm, reg!(i), 1));
            let length = attempt!(length_of(vm, reg!(object)));
            jump_if!(attempt!(relation(vm, reg!(i), length, Rel::Lt)));
        }
        Op::LoopLt { a, b, .. } => jump_if!(attempt!(relation(vm, reg!(a), reg!(b), Rel::Lt))),
        Op::LoopLe { a, b, .. } => jump_if!(attempt!(relation(vm, reg!(a), reg!(b), Rel::Le))),
        Op::LoopGt { a, b, .. } => jump_if!(attempt!(relation(vm, reg!(a), reg!(b), Rel::Gt))),
        Op::LoopGe { a, b, .. } => jump_if!(attempt!(relation(vm, reg!(a), reg!(b), Rel::Ge))),
        Op::LoopLtInt { a, value, .. } => jump_if!(attempt!(relation_int(vm, reg!(a), value, Rel::Lt))),
        Op::LoopLeInt { a, value, .. } => jump_if!(attempt!(relation_int(vm, reg!(a), value, Rel::Le))),
        Op::LoopGtInt { a, value, .. } => jump_if!(attempt!(relation_int(vm, reg!(a), value, Rel::Gt))),
        Op::LoopGeInt { a, value, .. } => jump_if!(attempt!(relation_int(vm, reg!(a), value, Rel::Ge))),
        Op::LoopStrictNe { a, b, .. } => jump_if!(!strict_equal(vm, reg!(a), reg!(b))),
        Op::LoopStrictNeInt { a, value, .. } => jump_if!(!strict_equal_int(reg!(a), value)),

        Op::GetGlobal { dst, cache } => {
            let entry = vm.caches[cache_index(cache)];
            let global = &vm.objects[vm.global as usize];
            reg!(dst) = if global.shape == entry.shape {
                // SAFETY: a cache entry that matches an object's shape names a slot that shape has.
                unsafe { global.read_slot(entry.slot as usize) }
            } else {
                attempt!(vm.get_global_slow(cache_index(cache)))
            };
        }
        Op::SetGlobal { src, cache } => {
            let value = reg!(src);
            attempt!(vm.set_global_slow(cache_index(cache), value));
        }
        Op::SetGlobalSloppy { src, cache } => {
            let value = reg!(src);
            let atom = vm.caches[cache_index(cache)].atom;
            let global = vm.global;
            let written = vm.set_on(global, atom, value);
            return forgiven(vm, written);
        }
        Op::DeclareGlobalVar { name } => {
            let atom = Atom(name);
            let global = vm.global;
            if !vm.has_own(global, atom) {
                vm.add_property(global, atom, Value::UNDEFINED, WRITABLE | ENUMERABLE);
            }
        }
        Op::DeclareGlobalFunction { src, name } => {
            let value = reg!(src);
            let global = vm.global;
            vm.define(global, Atom(name), value, WRITABLE | ENUMERABLE);
        }
        Op::TypeOfGlobal { dst, cache } => {
            let atom = vm.caches[cache_index(cache)].atom;
            let global = Value::object(vm.global);
            let value = attempt!(vm.get(global, atom));
            reg!(dst) = vm.type_of(value);
        }

        Op::NewCell { dst, src } => {
            let cell = vm.new_cell(reg!(src));
            reg!(dst) = cell;
        }
        Op::CopyCell { reg } => {
            let old = reg!(reg);
            let value = vm.cells[old.index()];
            let cell = vm.new_cell(value);
            reg!(reg) = cell;
        }
        Op::GetCell { dst, cell } => {
            let value = vm.cells[reg!(cell).index()];
            if value.is_empty() {
                let error = vm.reference_error("Cannot access a variable before initialization");
                return throw(vm, error);
            }
            reg!(dst) = value;
        }
        Op::SetCell { cell, src } => {
            let index = reg!(cell).index();
            if vm.cells[index].is_empty() {
                let error = vm.reference_error("Cannot access a variable before initialization");
                return throw(vm, error);
            }
            let value = reg!(src);
            vm.cells[index] = value;
        }
        Op::InitCell { cell, src } => {
            let index = reg!(cell).index();
            let value = reg!(src);
            vm.cells[index] = value;
        }
        Op::GetUp { dst, index } => {
            // SAFETY: the compiler only names captured variables the closure has.
            let cell = unsafe { *frame.upvalues.add(index as usize) }.index();
            let value = vm.cells[cell];
            if value.is_empty() {
                let error = vm.tdz_error(index);
                return throw(vm, error);
            }
            reg!(dst) = value;
        }
        Op::GetUpValue { dst, index } => {
            // SAFETY: as above.
            reg!(dst) = unsafe { *frame.upvalues.add(index as usize) };
        }
        Op::SetUp { index, src } => {
            // SAFETY: as above.
            let cell = unsafe { *frame.upvalues.add(index as usize) }.index();
            if vm.cells[cell].is_empty() {
                let error = vm.tdz_error(index);
                return throw(vm, error);
            }
            let value = reg!(src);
            vm.cells[cell] = value;
        }
        Op::Closure { dst, proto } => {
            let closure = vm.make_closure(proto, base);
            reg!(dst) = closure;
        }
        Op::LoadCallee { dst } => reg!(dst) = Value::object(frame.closure),
        Op::Note { kind, count, .. } => {
            if vm.tracking {
                if kind == crate::op::NOTE_KEEP || kind == crate::op::NOTE_KEPT {
                    // A call made here, or given what it gave last time, is stepped over with the Note after it, in the
                    // interpreter (see `keeping.rs`).
                    match vm.noted(pc as usize) {
                        // Compiled code goes on past the call, and the Note after it.
                        Ok(true) if kind == crate::op::NOTE_KEEP => return TAKEN,
                        Ok(true) => {
                            if let Some(frame) = vm.frames.last_mut() {
                                frame.pc = pc + 1 + u32::from(count) + 2;
                            }
                            return BAIL;
                        }
                        Ok(false) => {}
                        Err(error) => return throw(vm, error),
                    }
                } else {
                    vm.note(pc as usize);
                }
            }
        }
        Op::NoteKey { .. } | Op::NoteName { .. } => {}

        Op::NewObject { dst, capacity } => {
            let shape = vm.realm.object_shape;
            let object = vm.alloc_for_jit(shape, Kind::Plain, capacity as usize);
            reg!(dst) = object;
        }
        Op::NewArray { dst, capacity } => {
            let array = if capacity <= 4 {
                vm.small_array(&[])
            } else {
                vm.debt += capacity as usize * 8;
                let shape = vm.realm.array_shape;
                let items = Vec::with_capacity(capacity as usize);
                vm.alloc_for_jit(shape, Kind::Array(items.into()), 0)
            };
            reg!(dst) = array;
        }
        Op::ArrayPush { array, value } => {
            let value = reg!(value);
            let handle = reg!(array).index();
            if let Kind::Array(items) = &mut vm.objects[handle].kind {
                items.push(value);
            }
            vm.debt += 8;
        }
        Op::ArrayHole { array } => {
            let handle = reg!(array).index();
            if let Kind::Array(items) = &mut vm.objects[handle].kind {
                items.push(Value::EMPTY);
            }
        }
        Op::ArraySpread { array, source } => {
            let (array, source) = (reg!(array), reg!(source));
            attempt!(vm.spread_into(array, source));
        }

        Op::GetProp { dst, object, cache } => {
            reg!(dst) = attempt!(get_prop(vm, reg!(object), cache_index(cache)))
        }
        Op::GetPropPoly { dst, object, cache } => {
            reg!(dst) = attempt!(get_prop_poly(vm, reg!(object), cache_index(cache)))
        }
        Op::SetProp {
            object,
            value,
            cache,
        } => {
            let (target, value) = (reg!(object), reg!(value));
            let written = set_prop(vm, target, value, cache_index(cache));
            return forgiven(vm, written);
        }
        Op::SetPropPoly {
            object,
            value,
            cache,
        } => {
            let (target, value) = (reg!(object), reg!(value));
            let written = set_prop_poly(vm, target, value, cache_index(cache));
            return forgiven(vm, written);
        }
        Op::DefineField {
            object,
            value,
            cache,
        } => {
            let (target, value) = (reg!(object), reg!(value));
            let entry = vm.caches[cache_index(cache)];
            let found = unsafe { &mut *vm.objects.as_mut_ptr().add(target.index()) };
            if found.shape == entry.shape && found.flags & crate::gc::OLD == 0 {
                found.shape = entry.other;
                found.push_slot(value);
                return OK;
            }
            attempt!(vm.define_cached(target.handle(), value, cache_index(cache)));
        }
        Op::GetLength { dst, object, cache } => {
            let target = reg!(object);
            reg!(dst) = if target.is_object() {
                // `arguments.length` is always how many there are, as for an array (`own_special`).
                if let Kind::Array(items) | Kind::Arguments(items) = &vm.objects[target.index()].kind {
                    Value::int_or_double(items.len())
                } else {
                    attempt!(vm.get_cached(target, cache_index(cache)))
                }
            } else if target.is_string() {
                Value::int(vm.strings.items[target.index()].text.len() as i32)
            } else {
                attempt!(vm.get_cached(target, cache_index(cache)))
            };
        }
        Op::GetElem { dst, object, key } => {
            let (target, key) = (reg!(object), reg!(key));
            if target.is_object() && key.is_int() {
                // A negative index turns into a huge one, which is too big as well.
                let index = key.as_int() as u32 as usize;
                match &vm.objects[target.index()].kind {
                    Kind::Array(items) | Kind::Arguments(items) => {
                        if let Some(value) = items.get(index)
                            && !value.is_empty()
                        {
                            reg!(dst) = *value;
                            return OK;
                        }
                    }
                    Kind::TypedArray(data) => {
                        if data.epoch == vm.buffer_epoch && index < data.quick_length {
                            // SAFETY: while the epoch is the engine's, there are that many elements there.
                            reg!(dst) = unsafe { data.read_quick(index) };
                            return OK;
                        }
                        if let Some(value) = vm.typed_get_quick(target.handle(), index) {
                            reg!(dst) = value;
                            return OK;
                        }
                    }
                    _ => {}
                }
            }
            reg!(dst) = attempt!(vm.get_by_value(target, key));
        }
        // SetElemTyped is SetElem at a place that's written to a typed array, which the interpreter turns it into
        // and back (`go_typed_set`, `go_untyped_set`); this does the same, so that the interpreter finds each place
        // as it would have left it.
        Op::SetElem { object, key, value } | Op::SetElemTyped { object, key, value } => {
            let (target, key, value) = (reg!(object), reg!(key), reg!(value));
            let was_typed = matches!(*op, Op::SetElemTyped { .. });
            if target.is_object() && key.is_int() {
                let found = unsafe { &mut *vm.objects.as_mut_ptr().add(target.index()) };
                if found.flags & (FROZEN_ELEMENTS | crate::gc::OLD) == 0
                    && let Kind::Array(items) = &mut found.kind
                    && let Some(slot) = items.get_mut(key.as_int() as u32 as usize)
                    && !slot.is_empty()
                {
                    *slot = value;
                    if was_typed {
                        vm.go_untyped_set();
                    }
                    return OK;
                }
                if key.as_int() >= 0
                    && vm.typed_set_quick(target.handle(), key.as_int() as usize, value)
                {
                    if !was_typed {
                        vm.go_typed_set();
                    }
                    return OK;
                }
            }
            if was_typed {
                vm.go_untyped_set();
            }
            let written = vm.set_by_value(target, key, value);
            return forgiven(vm, written);
        }
        Op::DefineElem { object, key, value } => {
            let (target, key, value) = (reg!(object), reg!(key), reg!(value));
            let atom = attempt!(vm.to_atom(key));
            if vm.atoms.kind_of(atom) == AtomKind::Hidden {
                attempt!(vm.add_private(target.handle(), atom, value, WRITABLE));
            } else {
                vm.define(target.handle(), atom, value, PLAIN);
            }
        }
        Op::Delete { dst, object, key } => {
            let (target, key) = (reg!(object), reg!(key));
            reg!(dst) = match vm.delete_by_value(target, key) {
                Ok(done) => Value::bool(done),
                Err(error) => {
                    if !vm.forgive_refusal() {
                        return throw(vm, error);
                    }
                    Value::FALSE
                }
            };
        }
        Op::ObjectSpread { object, source } => {
            let (target, source) = (reg!(object), reg!(source));
            attempt!(vm.copy_properties(target, source, &[]));
        }
        Op::ObjectRest { dst, source, keys } => {
            let (source, keys) = (reg!(source), reg!(keys));
            let excluded = attempt!(vm.keys_to_atoms(keys));
            let target = vm.new_plain();
            reg!(dst) = target;
            attempt!(vm.copy_properties(target, source, &excluded));
        }
        Op::DefineGetter {
            object,
            key,
            function,
        } => {
            let (target, key, function) = (reg!(object), reg!(key), reg!(function));
            let atom = attempt!(vm.to_atom(key));
            vm.define_accessor(target.handle(), atom, Some(function), None, ENUMERABLE | CONFIGURABLE);
        }
        Op::DefineSetter {
            object,
            key,
            function,
        } => {
            let (target, key, function) = (reg!(object), reg!(key), reg!(function));
            let atom = attempt!(vm.to_atom(key));
            vm.define_accessor(target.handle(), atom, None, Some(function), ENUMERABLE | CONFIGURABLE);
        }
        Op::DefineMethod {
            object,
            key,
            function,
        } => {
            let (target, key, function) = (reg!(object), reg!(key), reg!(function));
            let atom = attempt!(vm.to_atom(key));
            let flags = if vm.atoms.kind_of(atom) == AtomKind::Hidden {
                HIDDEN & !WRITABLE
            } else {
                HIDDEN
            };
            vm.define(target.handle(), atom, function, flags);
        }
        Op::DefineClassGetter {
            object,
            key,
            function,
        } => {
            let (target, key, function) = (reg!(object), reg!(key), reg!(function));
            let atom = attempt!(vm.to_atom(key));
            vm.define_accessor(target.handle(), atom, Some(function), None, CONFIGURABLE);
        }
        Op::DefineClassSetter {
            object,
            key,
            function,
        } => {
            let (target, key, function) = (reg!(object), reg!(key), reg!(function));
            let atom = attempt!(vm.to_atom(key));
            vm.define_accessor(target.handle(), atom, None, Some(function), CONFIGURABLE);
        }
        Op::SetHome { function, home } => {
            let new_home = reg!(home).handle();
            let function_handle = reg!(function).index();
            if let Kind::Function { home, .. } = &mut vm.objects[function_handle].kind {
                *home = new_home;
            }
        }

        // The read half of a method call. `call` does the call.
        Op::CallMethod { f, cache, .. } => {
            reg!(f) = attempt!(get_prop(vm, reg!(f + 1), cache_index(cache)))
        }
        Op::CallMethodPoly { f, cache, .. } => {
            reg!(f) = attempt!(get_prop_poly(vm, reg!(f + 1), cache_index(cache)))
        }
        Op::Throw { src } => return throw(vm, reg!(src)),
        Op::CheckObjectCoercible { src } => {
            if reg!(src).is_nullish() {
                let shown = reg!(src);
                let shown = vm.brief(shown);
                let error =
                    vm.type_error(&format!("Cannot destructure '{shown}' as it is {shown}."));
                return throw(vm, error);
            }
        }

        Op::GetIterator { iter, source } => {
            let source = reg!(source);
            let (first, second) = attempt!(vm.get_iterator(source));
            reg!(iter) = first;
            reg!(iter + 1) = second;
        }
        Op::IterNext { value, iter, .. } => {
            let (first, second) = (reg!(iter), reg!(iter + 1));
            if first.is_empty() {
                reg!(value) = Value::UNDEFINED;
                return TAKEN;
            }
            if second.is_int() {
                if let Kind::Array(items) = &vm.objects[first.index()].kind {
                    let at = second.as_int() as usize;
                    if at < items.len() {
                        let item = items[at];
                        reg!(iter + 1) = Value::int(at as i32 + 1);
                        reg!(value) = if item.is_empty() {
                            Value::UNDEFINED
                        } else {
                            item
                        };
                        return OK;
                    }
                }
                reg!(iter) = Value::EMPTY;
                reg!(value) = Value::UNDEFINED;
                return TAKEN;
            }
            match attempt!(vm.iterator_step(first, second)) {
                Some(item) => reg!(value) = item,
                None => {
                    reg!(iter) = Value::EMPTY;
                    reg!(value) = Value::UNDEFINED;
                    return TAKEN;
                }
            }
        }
        Op::IterClose { iter } => {
            let (first, second) = (reg!(iter), reg!(iter + 1));
            if !first.is_empty() && !second.is_int() {
                attempt!(vm.iterator_close(first));
            }
        }
        Op::IterCloseQuietly { iter } => {
            let (first, second) = (reg!(iter), reg!(iter + 1));
            if !first.is_empty() && !second.is_int() {
                let _ = vm.iterator_close(first);
            }
        }
        Op::GetAsyncIterator { iter, source } => {
            let source = reg!(source);
            let (first, second) = attempt!(vm.get_async_iterator(source));
            reg!(iter) = first;
            reg!(iter + 1) = second;
        }
        Op::AsyncIterCall { dst, iter } => {
            let (first, second) = (reg!(iter), reg!(iter + 1));
            reg!(dst) = attempt!(vm.call(second, first, &[]));
        }
        Op::IterResult { value, .. } => {
            let result = reg!(value);
            if !result.is_object() {
                let error = vm.type_error("Iterator result is not an object");
                return throw(vm, error);
            }
            let done = attempt!(vm.get(result, text::DONE));
            reg!(value) = attempt!(vm.get(result, text::VALUE));
            jump_if!(vm.to_boolean(done));
        }
        Op::DelegateNext {
            dst,
            iter,
            received,
        } => {
            let (first, second, sent) = (reg!(iter), reg!(iter + 1), reg!(received));
            if second.is_int() {
                let at = second.as_int() as usize;
                let item = match &vm.objects[first.index()].kind {
                    Kind::Array(items) => items.get(at).copied(),
                    _ => None,
                };
                if item.is_some() {
                    reg!(iter + 1) = Value::int(at as i32 + 1);
                }
                let result = match item {
                    Some(found) if found.is_empty() => vm.iter_result(Value::UNDEFINED, false),
                    Some(found) => vm.iter_result(found, false),
                    None => vm.iter_result(Value::UNDEFINED, true),
                };
                reg!(dst) = result;
            } else {
                reg!(dst) = attempt!(vm.delegate_next(first, second, sent));
            }
        }
        Op::ForInKeys { dst, source } => {
            let source = reg!(source);
            reg!(dst) = attempt!(vm.for_in_keys(source));
        }

        Op::CreateArguments { dst } => {
            let arguments = vm.create_arguments();
            reg!(dst) = arguments;
        }
        Op::Rest { dst, start } => {
            let args = if frame.flags & crate::vm::KEPT_ARGS != 0 {
                vm.kept_args.last().map_or(&[][..], Vec::as_slice)
            } else {
                &[]
            };
            let rest = args.get(start as usize..).unwrap_or(&[]).to_vec();
            let array = if rest.len() <= 4 {
                vm.small_array(&rest)
            } else {
                vm.new_array(rest)
            };
            reg!(dst) = array;
        }
        Op::NewTarget { dst } => reg!(dst) = frame.new_target,
        Op::NewClass {
            dst,
            constructor,
            parent,
        } => {
            let (constructor, parent) = (reg!(constructor), reg!(parent));
            reg!(dst) = attempt!(vm.new_class(constructor, parent));
        }
        Op::SetFields {
            constructor,
            function,
        } => {
            let (constructor, function) = (reg!(constructor), reg!(function));
            vm.define(constructor.handle(), text::FIELDS, function, 0);
        }
        Op::SetPrivateMethods {
            constructor,
            methods,
        } => {
            let (constructor, methods) = (reg!(constructor), reg!(methods));
            vm.define(constructor.handle(), text::PRIVATE_METHODS, methods, 0);
        }
        Op::RunFields { this } => {
            let this = reg!(this);
            attempt!(vm.run_fields(frame.closure, this));
        }
        Op::SuperGet { dst, key, this } => {
            let (key, this) = (reg!(key), reg!(this));
            reg!(dst) = attempt!(vm.super_get(frame.closure, key, this));
        }
        Op::SuperSet { key, value, this } => {
            let (key, value, this) = (reg!(key), reg!(value), reg!(this));
            let atom = attempt!(vm.to_atom(key));
            attempt!(vm.set(this, atom, value));
        }
        Op::SloppyThis => {
            let this = reg!(0);
            if this.is_nullish() {
                reg!(0) = Value::object(vm.global);
            } else if !this.is_object() {
                reg!(0) = attempt!(vm.to_object(this));
            }
        }
        Op::ToObject { dst, src } => {
            let value = reg!(src);
            reg!(dst) = attempt!(vm.to_object(value));
        }
        Op::CheckThis { reg } => {
            if reg!(reg).is_empty() {
                let error = vm.reference_error(
                    "Must call super constructor in derived class before accessing 'this' or returning from derived constructor",
                );
                return throw(vm, error);
            }
        }
        Op::ThrowConstAssign { .. } => {
            let error = vm.type_error("Assignment to constant variable.");
            return throw(vm, error);
        }
        Op::ThrowTdz { name } => {
            let name = vm.atom_text(Atom(name));
            let error = vm.reference_error(&format!("Cannot access '{name}' before initialization"));
            return throw(vm, error);
        }

        // Compiled code does these itself, or never has them (see `jit::compilable`). The interpreter can.
        Op::Jump { .. }
        | Op::Loop { .. }
        | Op::Return { .. }
        | Op::ReturnUndefined
        | Op::Call { .. }
        | Op::CallMath { .. }
        | Op::CallPlain { .. }
        | Op::New { .. }
        | Op::CallSpread { .. }
        | Op::NewSpread { .. }
        | Op::SuperCall { .. }
        | Op::SuperCallSpread { .. }
        | Op::InitialYield
        | Op::Yield { .. }
        | Op::Await { .. }
        | Op::Step { .. } => return bail_at(vm, pc),
    }
    OK
}
}

/// Has the interpreter carry on from the instruction at `pc`, which hasn't been started. Compiled code only comes here
/// for an instruction the helpers don't do, which is a bug in what `Gen::op` sends them: it's counted, and with
/// `RSUI_JIT_STRICT` it ends the process.
#[cold]
fn bail_at(vm: &mut Vm, pc: u32) -> u32 {
    vm.jit.stats.unknown += 1;
    if super::strict() {
        eprintln!("jit: the generic way can't do {:?} (at {pc})", vm.code[pc as usize]);
        std::process::abort();
    }
    if let Some(frame) = vm.frames.last_mut() {
        frame.pc = pc;
    }
    BAIL
}

jit_abi! {
/// A call that compiled code doesn't make itself: to a function that isn't compiled, a native, anything else, a
/// `new` or a `super(...)`. For CallMethod the method has been read already. A JavaScript function runs to its end
/// before this returns, in the interpreter (which goes into compiled code again if it has some).
pub(crate) fn call(vm: &mut Vm, pc: u32) -> u32 {
    #[cfg(feature = "jit-opt")]
    super::feedback::record(vm, pc);
    let op = vm.code[pc as usize];
    if vm.jit.counting {
        count_call(vm, &op);
    }
    let frame = here(vm, pc);
    let base = frame.base as usize;
    macro_rules! attempt {
        ($result:expr) => {
            match $result {
                Ok(value) => value,
                Err(error) => return throw(vm, error),
            }
        };
    }
    // With a `new.target` for a `new` or a `super(...)`.
    let (dst, f, argc, new_target) = match op {
        Op::Call { dst, f, argc } => (dst, f, argc, None),
        // What it calls isn't the `Math` function it was made for, or not with numbers: as in the interpreter, it's
        // a Call from now on.
        Op::CallMath { dst, f, which } => {
            vm.go_plain_call();
            (dst, f, crate::builtins::math_quick_argc(which), None)
        }
        Op::CallPlain { dst, f, argc } => {
            vm.stack[base + f as usize + 1] = Value::UNDEFINED;
            (dst, f, argc, None)
        }
        Op::CallMethod { dst, f, .. } | Op::CallMethodPoly { dst, f, .. } => (dst, f, 0, None),
        Op::New { dst, f, argc } => (dst, f, argc, Some(vm.stack[base + f as usize])),
        Op::SuperCall { dst, f, argc } => {
            let (parent, target) = attempt!(vm.super_constructor(frame.closure, frame.new_target));
            vm.stack[base + f as usize] = parent;
            (dst, f, argc, Some(target))
        }
        Op::SuperCallSpread { dst, f, array } => {
            let array = vm.stack[base + array as usize];
            let (parent, target) = attempt!(vm.super_constructor(frame.closure, frame.new_target));
            let args = vm.array_items(array);
            vm.stack[base + f as usize] = parent;
            let value = attempt!(vm.construct_with_target(parent, &args, target));
            vm.stack[base + dst as usize] = value;
            return OK;
        }
        Op::CallSpread { dst, f, array } => {
            let (callee, this, array) = (
                vm.stack[base + f as usize],
                vm.stack[base + f as usize + 1],
                vm.stack[base + array as usize],
            );
            let args = vm.array_items(array);
            let value = attempt!(vm.call(callee, this, &args));
            vm.stack[base + dst as usize] = value;
            return OK;
        }
        Op::NewSpread { dst, f, array } => {
            let (callee, array) = (vm.stack[base + f as usize], vm.stack[base + array as usize]);
            let args = vm.array_items(array);
            let value = attempt!(vm.construct(callee, &args));
            vm.stack[base + dst as usize] = value;
            return OK;
        }
        _ => return bail_at(vm, pc),
    };
    let callee = vm.stack[base + f as usize];
    let result = 'call: {
        if callee.is_object() {
            match &vm.objects[callee.index()].kind {
                Kind::Function { proto, .. } => {
                    let proto = *proto;
                    let flags = vm.protos[proto as usize].flags;
                    let (frame_flags, target) = if let Some(target) = new_target {
                        if flags & IS_CONSTRUCTOR == 0 {
                            break 'call vm.call_at(base + f as usize, argc as usize, Some(target));
                        }
                        let this = if flags & IS_DERIVED != 0 {
                            Value::EMPTY
                        } else {
                            match vm.create_this(target) {
                                Ok(this) => this,
                                Err(error) => break 'call Err(error),
                            }
                        };
                        vm.stack[base + f as usize + 1] = this;
                        (crate::vm::CONSTRUCTING, target)
                    } else {
                        if flags & IS_CLASS != 0 {
                            break 'call vm.call_at(base + f as usize, argc as usize, None);
                        }
                        (0, Value::UNDEFINED)
                    };
                    vm.jit.count_called(proto);
                    let stop = vm.frames.len();
                    if let Err(error) = vm.enter_function(
                        callee.handle(),
                        proto,
                        base + f as usize + 1,
                        argc as usize,
                        dst,
                        frame_flags,
                        target,
                    ) {
                        break 'call Err(error);
                    }
                    break 'call vm.run(stop);
                }
                Kind::Native(native) if new_target.is_none() => {
                    let func = native.func;
                    if vm.keeping.working != 0 && !native.pure {
                        vm.keeping.spoiled = true;
                    }
                    let args = Args {
                        this: vm.stack[base + f as usize + 1],
                        start: base + f as usize + 2,
                        count: argc as usize,
                        new_target: Value::UNDEFINED,
                        callee: callee.handle(),
                    };
                    break 'call func(vm, args);
                }
                _ => {}
            }
        }
        vm.call_at(base + f as usize, argc as usize, new_target)
    };
    match result {
        Ok(value) => {
            vm.stack[base + dst as usize] = value;
            OK
        }
        Err(error) => throw(vm, error),
    }
}
}

/// For `RSUI_JIT_LOG`: what a property read compiled code left to the generic helper read from.
#[cold]
fn count_read(vm: &mut Vm, op: &Op) {
    let target = match *op {
        Op::GetProp { object, .. } | Op::GetPropPoly { object, .. } => object,
        Op::CallMethod { f, .. } | Op::CallMethodPoly { f, .. } => f + 1,
        _ => return,
    };
    let base = vm.frames.last().map_or(0, |frame| frame.base as usize);
    let value = vm.stack[base + target as usize];
    let what = if value.is_string() {
        "a string"
    } else if value.is_number() {
        "a number"
    } else if !value.is_object() {
        "another primitive"
    } else {
        let found = &vm.objects[value.index()];
        if found.flags & LAZY_FUNCTION != 0 {
            "a function with properties yet to be made"
        } else {
            match &found.kind {
                Kind::Function { .. } => "a function",
                Kind::Native(_) => "a native",
                Kind::Array(_) => "an array",
                Kind::Plain => "a plain object",
                _ => "another object",
            }
        }
    };
    *vm.jit.generic.entry(format!("call: read of {what}")).or_default() += 1;
}

/// For `RSUI_JIT_LOG`: what a call compiled code didn't make itself was, by the instruction and what it called.
#[cold]
fn count_call(vm: &mut Vm, op: &Op) {
    let f = match *op {
        Op::Call { f, .. }
        | Op::CallMath { f, .. }
        | Op::CallPlain { f, .. }
        | Op::CallMethod { f, .. }
        | Op::CallMethodPoly { f, .. }
        | Op::New { f, .. }
        | Op::CallSpread { f, .. }
        | Op::NewSpread { f, .. }
        | Op::SuperCall { f, .. }
        | Op::SuperCallSpread { f, .. } => f,
        _ => return,
    };
    let base = vm.frames.last().map_or(0, |frame| frame.base as usize);
    let callee = vm.stack[base + f as usize];
    let what = if !callee.is_object() {
        "not an object"
    } else {
        match &vm.objects[callee.index()].kind {
            Kind::Function { proto, .. } => {
                let flags = vm.protos[*proto as usize].flags;
                let slot = vm.jit.slots[*proto as usize];
                if flags & IS_DERIVED != 0 {
                    "a derived class"
                } else if flags & IS_CLASS != 0 {
                    "a class"
                } else if flags & (IS_GENERATOR | IS_ASYNC) != 0 {
                    "a generator or async"
                } else if slot == 0 {
                    "a function never compiled"
                } else if slot < super::plan::CODE_MIN {
                    "a function not compiled yet"
                } else {
                    "a compiled function"
                }
            }
            Kind::Native(native) if native.forwards != 0 => "call or apply",
            Kind::Native(_) => "a native",
            Kind::Bound(_) => "a bound function",
            _ => "something else",
        }
    };
    let text = format!("{op:?}");
    let name = text.split([' ', '{']).next().unwrap_or_default();
    *vm.jit.generic.entry(format!("call: {name} of {what}")).or_default() += 1;
}

jit_abi! {
/// The test of `IncLoopLt`, `IncLoopLtInt` or `IncLoopLtLength` at `pc`, once compiled code has stepped the
/// counter itself: [`TAKEN`] if the loop goes round again.
pub(crate) fn loop_test(vm: &mut Vm, pc: u32) -> u32 {
    let op = vm.code[pc as usize];
    let frame = here(vm, pc);
    let base = frame.base as usize;
    let going_on = match op {
        Op::IncLoopLt { i, limit, .. } => {
            let (x, y) = (vm.stack[base + i as usize], vm.stack[base + limit as usize]);
            relation(vm, x, y, Rel::Lt)
        }
        Op::IncLoopLtInt { i, limit, .. } => {
            let x = vm.stack[base + i as usize];
            relation_int(vm, x, limit, Rel::Lt)
        }
        Op::IncLoopLtLength { i, object, .. } => {
            let target = vm.stack[base + object as usize];
            match length_of(vm, target) {
                Ok(length) => {
                    let x = vm.stack[base + i as usize];
                    relation(vm, x, length, Rel::Lt)
                }
                Err(error) => Err(error),
            }
        }
        _ => unreachable!("only the loops' instructions are tested here"),
    };
    match going_on {
        Ok(true) => TAKEN,
        Ok(false) => OK,
        Err(error) => throw(vm, error),
    }
}
}

jit_abi! {
/// `===` of two values that could each be equal to something with other bits than theirs, and aren't both numbers:
/// two strings or two bigints, by what they hold. 1 if they're equal. It can't fail or make anything.
pub(crate) fn strict_equals(vm: &mut Vm, x: u64, y: u64) -> u32 {
    u32::from(vm.strict_equals(Value(x), Value(y)))
}
}

jit_abi! {
/// The comparison at `pc` (`<`, `<=`, `>` or `>=`, as a value, a jump or a loop's test) of `x` and `y`, which aren't
/// two numbers, as the interpreter's `less_than` and the rest work it out: [`TAKEN`] if it holds, OK if not.
pub(crate) fn compare(vm: &mut Vm, pc: u32, x: u64, y: u64) -> u32 {
    #[cfg(feature = "jit-opt")]
    super::feedback::record(vm, pc);
    let rel = match vm.code[pc as usize] {
        Op::Lt { .. }
        | Op::JumpLt { .. }
        | Op::JumpNotLt { .. }
        | Op::JumpLtInt { .. }
        | Op::JumpNotLtInt { .. }
        | Op::LoopLt { .. }
        | Op::LoopLtInt { .. } => Rel::Lt,
        Op::Le { .. }
        | Op::JumpLe { .. }
        | Op::JumpNotLe { .. }
        | Op::JumpLeInt { .. }
        | Op::JumpNotLeInt { .. }
        | Op::LoopLe { .. }
        | Op::LoopLeInt { .. } => Rel::Le,
        Op::Gt { .. }
        | Op::JumpGt { .. }
        | Op::JumpNotGt { .. }
        | Op::JumpGtInt { .. }
        | Op::JumpNotGtInt { .. }
        | Op::LoopGt { .. }
        | Op::LoopGtInt { .. } => Rel::Gt,
        Op::Ge { .. }
        | Op::JumpGe { .. }
        | Op::JumpNotGe { .. }
        | Op::JumpGeInt { .. }
        | Op::JumpNotGeInt { .. }
        | Op::LoopGe { .. }
        | Op::LoopGeInt { .. } => Rel::Ge,
        _ => unreachable!("only comparisons come here"),
    };
    here(vm, pc);
    match relation(vm, Value(x), Value(y), rel) {
        Ok(true) => TAKEN,
        Ok(false) => OK,
        Err(error) => throw(vm, error),
    }
}
}

jit_abi! {
/// Concat at `pc`: the strings of its registers joined, as the interpreter makes it.
pub(crate) fn concat(vm: &mut Vm, pc: u32) -> u32 {
    let Op::Concat { dst, start, count } = vm.code[pc as usize] else {
        unreachable!("only Concat comes here");
    };
    let base = here(vm, pc).base as usize;
    match vm.concat_registers(base + start as usize, count as usize) {
        Ok(value) => {
            vm.stack[base + dst as usize] = value;
            OK
        }
        Err(error) => throw(vm, error),
    }
}
}

// Helpers for instructions that are a call of one `Vm` function, which saves the generic helper's work of finding
// which instruction it is: each finds its operands in the one instruction it's for, as `op` would.

jit_abi! {
/// InstanceOf at `pc`.
pub(crate) fn instance_of(vm: &mut Vm, pc: u32) -> u32 {
    let Op::InstanceOf { dst, a, b } = vm.code[pc as usize] else {
        return op(vm, pc);
    };
    let base = here(vm, pc).base as usize;
    let (value, target) = (vm.stack[base + a as usize], vm.stack[base + b as usize]);
    match vm.instance_of(value, target) {
        Ok(found) => {
            vm.stack[base + dst as usize] = Value::bool(found);
            OK
        }
        Err(error) => throw(vm, error),
    }
}
}

jit_abi! {
/// SuperGet at `pc`.
pub(crate) fn super_get(vm: &mut Vm, pc: u32) -> u32 {
    let Op::SuperGet { dst, key, this } = vm.code[pc as usize] else {
        return op(vm, pc);
    };
    let frame = here(vm, pc);
    let base = frame.base as usize;
    let (key, this) = (vm.stack[base + key as usize], vm.stack[base + this as usize]);
    match vm.super_get(frame.closure, key, this) {
        Ok(value) => {
            vm.stack[base + dst as usize] = value;
            OK
        }
        Err(error) => throw(vm, error),
    }
}
}

jit_abi! {
/// CreateArguments at `pc`, which can't fail.
pub(crate) fn create_arguments(vm: &mut Vm, pc: u32) -> u32 {
    let Op::CreateArguments { dst } = vm.code[pc as usize] else {
        return op(vm, pc);
    };
    let base = here(vm, pc).base as usize;
    let arguments = vm.create_arguments();
    vm.stack[base + dst as usize] = arguments;
    OK
}
}

jit_abi! {
/// SetHome at `pc`, which gives a method an object literal makes the literal as its home, as the interpreter does.
/// Each method with a name of its own has one (which defines such a method as a field is), and chart.js
/// runs about 275,000 of them in the canvas benchmark (whose line chart's update took 12.65 ms with them done the
/// generic way, and 12.2-12.5 with this). It can't fail, allocate or call anything.
pub(crate) fn set_home(vm: &mut Vm, pc: u32) -> u32 {
    let Op::SetHome { function, home } = vm.code[pc as usize] else {
        return op(vm, pc);
    };
    let base = vm.frames.last().expect("compiled code runs in a frame").base as usize;
    let (function, home) = (vm.stack[base + function as usize], vm.stack[base + home as usize]);
    if let Kind::Function { home: slot, .. } = &mut vm.objects[function.index()].kind {
        *slot = home.handle();
    }
    OK
}
}

jit_abi! {
/// ToNumber or ToNumeric at `pc` of what isn't a number.
pub(crate) fn to_number(vm: &mut Vm, pc: u32) -> u32 {
    let (dst, src, numeric) = match vm.code[pc as usize] {
        Op::ToNumber { dst, src } => (dst, src, false),
        Op::ToNumeric { dst, src } => (dst, src, true),
        _ => return op(vm, pc),
    };
    let base = here(vm, pc).base as usize;
    let value = vm.stack[base + src as usize];
    let converted = if numeric {
        vm.to_numeric(value)
    } else {
        vm.to_number(value).map(Value::number)
    };
    match converted {
        Ok(value) => {
            vm.stack[base + dst as usize] = value;
            OK
        }
        Err(error) => throw(vm, error),
    }
}
}

jit_abi! {
/// SetElem at `pc` of what isn't an array's or a typed array's element (`o[key] = v` for a name), as the
/// interpreter does it then.
pub(crate) fn set_elem(vm: &mut Vm, pc: u32) -> u32 {
    #[cfg(feature = "jit-opt")]
    super::feedback::record(vm, pc);
    // (SetElemTyped turns back into SetElem the generic way, as in the interpreter.)
    let Op::SetElem { object, key, value } = vm.code[pc as usize] else {
        return op(vm, pc);
    };
    let base = here(vm, pc).base as usize;
    let (target, key, value) = (
        vm.stack[base + object as usize],
        vm.stack[base + key as usize],
        vm.stack[base + value as usize],
    );
    if !(target.is_object() && key.is_int()) {
        let written = vm.set_by_value(target, key, value);
        return forgiven(vm, written);
    }
    op(vm, pc)
}
}

jit_abi! {
/// `x % y` of two doubles, as Rust's `%` (and the interpreter's `remainder!`) works it out.
pub(crate) fn fmod(x: f64, y: f64) -> f64 {
    x % y
}
}

jit_abi! {
/// The write barrier for cell `index`, which compiled code has just stored `value` in, as `Cells::write_barrier`
/// does after the interpreter's stores. Only called for what isn't a number.
pub(crate) fn cell_barrier(vm: &mut Vm, index: u32, value: u64) {
    vm.cells.write_barrier(index as usize, Value(value));
}
}

jit_abi! {
/// The `TYPE_` number of what `typeof` says of `value`. It can't fail or make anything.
pub(crate) fn type_kind(vm: &mut Vm, value: u64) -> u32 {
    u32::from(vm.type_kind(Value(value)))
}
}

jit_abi! {
/// Whether `value` is truthy, 1 if it is. It can't fail or make anything.
pub(crate) fn truthy(vm: &mut Vm, value: u64) -> u32 {
    u32::from(vm.to_boolean(Value(value)))
}
}

jit_abi! {
/// A string's length, as GetLength gives it. It can't fail or make anything.
pub(crate) fn string_length(vm: &mut Vm, string: u64) -> u64 {
    Value::int(vm.strings.items[Value(string).index()].text.len() as i32).0
}
}

jit_abi! {
/// A call from compiled code at `pc` to a native, whose function compiled code has found (`func`), and has said the
/// frame is at that call, with the top of the stack where the native may use it. `operands` has the callee's
/// register, how many arguments there are and where the result goes, in 16 bits each from the bottom, of the frame
/// whose registers start at `base` on the stack.
pub(crate) fn call_native(vm: &mut Vm, func: usize, base: u32, operands: u64) -> u32 {
    // SAFETY: compiled code read it from a native's `NativeData::func`.
    let func: NativeFn = unsafe { std::mem::transmute::<usize, NativeFn>(func) };
    let base = base as usize;
    let f = operands as u16 as usize;
    let argc = (operands >> 16) as u16 as usize;
    let dst = (operands >> 32) as u16 as usize;
    // SAFETY: the frame's registers are inside the stack, which doesn't move.
    let (callee, this) = unsafe {
        let at = vm.stack.as_ptr().add(base + f);
        (*at, *at.add(1))
    };
    let args = Args {
        this,
        start: base + f + 2,
        count: argc,
        new_target: Value::UNDEFINED,
        callee: callee.handle(),
    };
    #[cfg(feature = "jit-opt")]
    super::feedback::native_called(vm, callee);
    if vm.keeping.working != 0
        && callee.is_object()
        && matches!(&vm.objects[callee.index()].kind, Kind::Native(native) if !native.pure)
    {
        vm.keeping.spoiled = true;
    }
    match func(vm, args) {
        Ok(value) => {
            // SAFETY: as above; the frame is still there.
            unsafe { *vm.stack.as_mut_ptr().add(base + dst) = value };
            OK
        }
        Err(error) => throw(vm, error),
    }
}
}

jit_abi! {
/// The `length` IncLoopLtLength compares with, for what isn't an array, when it's quick to find (`arguments`, a
/// typed array whose getter is the built-in one, a string): the value, or `EMPTY` for the long way. It can't fail or
/// run anything, so it needs no frame.
pub(crate) fn quick_length(vm: &mut Vm, target: u64) -> u64 {
    vm.quick_loop_length(Value(target)).unwrap_or(Value::EMPTY).0
}
}

jit_abi! {
/// IterNext at `pc` when what it goes through isn't an array. Any iterator but a generator: its `next`, as
/// `iterator_step` calls it.
///
/// A generator with the built-in `next` is what the interpreter's `for of` carries on in place (`Vm::step_in_place`),
/// with nothing to start or stop for each value, which compiled code can't: it would have the interpreter run the
/// generator from here for each one, which cost more than the interpreter going through the whole loop. So the
/// interpreter carries this frame on from this instruction to the frame's end (marked [`crate::vm::INTERPRETED`],
/// so that it doesn't come back into this code at the top of the loop), and this says the function has returned
/// ([`RETURNED`], with its value) or thrown ([`THROWN`]). The functions it calls are compiled as ever.
pub(crate) fn iter_next(vm: &mut Vm, pc: u32) -> u32 {
    let Op::IterNext { value, iter, .. } = vm.code[pc as usize] else {
        unreachable!("only IterNext comes here");
    };
    let frame = here(vm, pc);
    let base = frame.base as usize;
    let (first, second) = (vm.stack[base + iter as usize], vm.stack[base + iter as usize + 1]);
    // A count over what isn't an array (compiled code does arrays itself), or nothing more: the generic way.
    if first.is_empty() || second.is_int() {
        return op(vm, pc);
    }
    match resume_in_code(vm, first, second, value) {
        Ok(Some(target)) => {
            vm.jit.value = Value(target as u64);
            return RESUME;
        }
        Ok(None) => {}
        Err(error) => return throw(vm, error),
    }
    // Not a generator's frame that a `for of` is carrying on in place itself: its `yield` would hand its value to that
    // loop and end the frame, which isn't the frame returning.
    if vm.resumes_in_place(first, second) && frame.flags & crate::vm::RESUMED_HERE == 0 {
        let depth = vm.frames.len() - 1;
        let frame = vm.frames.last_mut().expect("compiled code runs in a frame");
        frame.flags |= crate::vm::INTERPRETED;
        frame.pc = pc;
        vm.jit.stats.handed += 1;
        return match vm.run(depth) {
            Ok(returned) => {
                vm.jit.value = returned;
                RETURNED
            }
            Err(error) => {
                vm.jit.value = error;
                THROWN
            }
        };
    }
    match vm.iterator_step(first, second) {
        Ok(Some(item)) => {
            vm.stack[base + value as usize] = item;
            OK
        }
        Ok(None) => {
            vm.stack[base + iter as usize] = Value::EMPTY;
            vm.stack[base + value as usize] = Value::UNDEFINED;
            TAKEN
        }
        Err(error) => throw(vm, error),
    }
}
}

jit_abi! {
/// ArraySpread at `pc`, `[...source]` into an array literal, as the interpreter does. This, Rest and GetIterator of
/// what isn't an array have helpers of their own, rather than the generic one, whose start and end cost more than the
/// interpreter's handler does: code that makes small trees of elements each frame (react_render) does them hundreds of
/// thousands of times.
pub(crate) fn array_spread(vm: &mut Vm, pc: u32) -> u32 {
    let Op::ArraySpread { array, source } = vm.code[pc as usize] else {
        unreachable!("only ArraySpread comes here");
    };
    let base = here(vm, pc).base as usize;
    let (array, source) = (vm.stack[base + array as usize], vm.stack[base + source as usize]);
    match vm.spread_into(array, source) {
        Ok(()) => OK,
        Err(error) => throw(vm, error),
    }
}
}

jit_abi! {
/// Rest at `pc`, a rest parameter, as the interpreter does (see `array_spread`).
pub(crate) fn rest(vm: &mut Vm, pc: u32) -> u32 {
    let Op::Rest { dst, start } = vm.code[pc as usize] else {
        unreachable!("only Rest comes here");
    };
    let frame = here(vm, pc);
    let args = if frame.flags & crate::vm::KEPT_ARGS != 0 {
        vm.kept_args.last().map_or(&[][..], Vec::as_slice)
    } else {
        &[]
    };
    let rest = args.get(start as usize..).unwrap_or(&[]).to_vec();
    let array = if rest.len() <= 4 {
        vm.small_array(&rest)
    } else {
        vm.new_array(rest)
    };
    vm.stack[frame.base as usize + dst as usize] = array;
    OK
}
}

jit_abi! {
/// GetIterator at `pc` of what compiled code doesn't go through by a count itself, as the interpreter does (see
/// `array_spread`).
pub(crate) fn get_iterator(vm: &mut Vm, pc: u32) -> u32 {
    let Op::GetIterator { iter, source } = vm.code[pc as usize] else {
        unreachable!("only GetIterator comes here");
    };
    let base = here(vm, pc).base as usize;
    match vm.get_iterator(vm.stack[base + source as usize]) {
        Ok((first, second)) => {
            vm.stack[base + iter as usize] = first;
            vm.stack[base + iter as usize + 1] = second;
            OK
        }
        Err(error) => throw(vm, error),
    }
}
}

/// For `iter_next`: if what a `for of` goes through is a suspended generator with the built-in `next` and compiled
/// code that can carry on where it stopped, puts its frame on top as the interpreter does to carry it on in place
/// (`RESUMED_HERE`, its `yield` going to register `dst` of the frame under it), and says where in its code to go on.
/// Compiled code then calls that directly: a pipeline of generators runs in compiled code end to end, with nothing
/// started or stopped for each value but the generator's frame.
fn resume_in_code(vm: &mut Vm, iterator: Value, next: Value, dst: u16) -> crate::vm::Run<Option<usize>> {
    // Compiled code uses Rust's stack for the call, as for any.
    if !iterator.is_object() || !next.is_object() || crate::stack_position() < vm.jit.floor {
        return Ok(None);
    }
    if !matches!(&vm.objects[next.index()].kind,
        Kind::Native(native) if native.func as usize == Vm::generator_next as NativeFn as usize)
    {
        return Ok(None);
    }
    let Kind::Generator(data) = &vm.objects[iterator.index()].kind else {
        return Ok(None);
    };
    if data.state != GeneratorState::Suspended || vm.protos[data.proto as usize].flags & IS_ASYNC != 0 {
        return Ok(None);
    }
    let Some(target) = vm.jit_entry(data.proto, data.pc) else {
        return Ok(None);
    };
    vm.enter_in_place(iterator, dst)?;
    vm.jit.stats.resumes += 1;
    Ok(Some(target))
}

jit_abi! {
/// A read at a megamorphic place `cache` (its index in the list of caches) of the object `target`, from what reads
/// remember, as GetPropPoly does: [`OK`] with the value in `vm.jit.value`, or [`BAIL`] for the generic way to do it.
/// It can't fail or run anything, so it needs no frame.
pub(crate) fn mega_get(vm: &mut Vm, target: u64, cache: u32) -> u32 {
    let target = Value(target);
    let cache = cache as usize;
    let (shape, flags) = {
        let found = &vm.objects[target.index()];
        (found.shape, found.flags)
    };
    let atom = vm.caches[cache].atom;
    if flags & LAZY_FUNCTION != 0 {
        return BAIL;
    }
    let Some((holder, slot)) = vm.remembered_value(shape, atom) else {
        return BAIL;
    };
    let epoch = vm.epoch;
    let entry = &mut vm.caches[cache];
    entry.shape = shape;
    entry.other = holder;
    entry.slot = slot;
    entry.epoch = epoch;
    let holder = if holder == NONE { target.handle() } else { holder };
    // SAFETY: what reads remember names a slot the shape has, on the object it was found on.
    vm.jit.value = unsafe { vm.objects[holder as usize].read_slot(slot as usize) };
    OK
}
}

jit_abi! {
/// A write at a megamorphic place `cache` (its index in the list of caches) of `value` to the object `target`, from
/// where writes to its shape went before, as SetPropPoly does: [`OK`] if it wrote, or [`BAIL`] for the generic way. It
/// can't fail or run anything, so it needs no frame. Compiled code has checked the object isn't old.
pub(crate) fn mega_set(vm: &mut Vm, target: u64, value: u64, cache: u32) -> u32 {
    let (target, value) = (Value(target), Value(value));
    let atom = vm.caches[cache as usize].atom;
    let (shape, flags) = {
        let found = &vm.objects[target.index()];
        (found.shape, found.flags)
    };
    if flags & LAZY_FUNCTION != 0 {
        return BAIL;
    }
    let Some((to, slot)) = vm.remembered_store(shape, atom) else {
        return BAIL;
    };
    // Not through `objects[..]`, which would remember an old object.
    let found = unsafe { &mut *vm.objects.as_mut_ptr().add(target.index()) };
    if to == NONE {
        // SAFETY: what writes remember names a slot the shape has.
        unsafe { found.write_slot(slot as usize, value) };
        return OK;
    }
    if found.flags & (NOT_EXTENSIBLE | IS_PROTO) != 0 {
        return BAIL;
    }
    found.shape = to;
    found.push_slot(value);
    vm.debt += 8;
    OK
}
}

jit_abi! {
/// The `+` at `pc` (Add, AddK or AddInt) of `x` and `y`, when they aren't two numbers, as `arithmetic!` does.
pub(crate) fn add(vm: &mut Vm, pc: u32, x: u64, y: u64) -> u32 {
    #[cfg(feature = "jit-opt")]
    super::feedback::record(vm, pc);
    let dst = match vm.code[pc as usize] {
        Op::Add { dst, .. } | Op::AddK { dst, .. } | Op::AddInt { dst, .. } => dst,
        _ => unreachable!("only a `+` comes here"),
    };
    let frame = here(vm, pc);
    match arithmetic(vm, Value(x), Value(y), Arith::Add) {
        Ok(value) => {
            vm.stack[frame.base as usize + dst as usize] = value;
            OK
        }
        Err(error) => throw(vm, error),
    }
}
}

jit_abi! {
/// GetLength at `pc` of `target` when that isn't an array, as the interpreter does: a string's here, anything else
/// through its cache.
pub(crate) fn get_length(vm: &mut Vm, pc: u32, target: u64) -> u32 {
    let Op::GetLength { dst, cache, .. } = vm.code[pc as usize] else {
        unreachable!("only GetLength comes here");
    };
    let target = Value(target);
    let frame = here(vm, pc);
    let place = frame.base as usize + dst as usize;
    if target.is_string() {
        vm.stack[place] = Value::int(vm.strings.items[target.index()].text.len() as i32);
        return OK;
    }
    if target.is_object()
        && let Kind::Array(items) = &vm.objects[target.index()].kind
    {
        vm.stack[place] = Value::int_or_double(items.len());
        return OK;
    }
    match vm.get_cached(target, frame.cache_base as usize + cache as usize) {
        Ok(value) => {
            vm.stack[place] = value;
            OK
        }
        Err(error) => throw(vm, error),
    }
}
}

jit_abi! {
/// GetElem of `target[key]` when that isn't an array's element, as the interpreter does: a typed array's the quick
/// way, anything else `get_by_value`. `at` is the instruction's place, with the register the result goes in from bit
/// 32.
pub(crate) fn get_elem(vm: &mut Vm, at: u64, target: u64, key: u64) -> u32 {
    let (pc, dst) = (at as u32, (at >> 32) as usize);
    let (target, key) = (Value(target), Value(key));
    #[cfg(feature = "jit-opt")]
    super::feedback::elem_read(vm, pc, target, key);
    // The quick ways can't fail or run anything, so they need nothing of the frame but where it is.
    let base = vm.frames.last().expect("compiled code runs in a frame").base as usize;
    if target.is_object() && key.is_int() {
        if let Some(value) = vm.typed_get_quick(target.handle(), key.as_int() as u32 as usize) {
            vm.stack[base + dst] = value;
            return OK;
        }
        if let Kind::Array(items) = &vm.objects[target.index()].kind
            && let Some(value) = items.get(key.as_int() as u32 as usize)
            && !value.is_empty()
        {
            vm.stack[base + dst] = *value;
            return OK;
        }
    }
    here(vm, pc);
    match vm.get_by_value(target, key) {
        Ok(value) => {
            vm.stack[base + dst] = value;
            OK
        }
        Err(error) => throw(vm, error),
    }
}
}

jit_abi! {
/// Writes a typed array's element, as SetElem's quick way does: [`OK`] if it did, or [`BAIL`] for the generic way.
pub(crate) fn typed_set(vm: &mut Vm, target: u64, key: u64, value: u64) -> u32 {
    let (target, key) = (Value(target), Value(key));
    if target.is_object()
        && key.is_int()
        && key.as_int() >= 0
        && vm.typed_set_quick(target.handle(), key.as_int() as usize, Value(value))
    {
        #[cfg(feature = "jit-opt")]
        super::feedback::typed_written(vm, target);
        OK
    } else {
        #[cfg(feature = "jit-opt")]
        {
            vm.jit.site = 0;
        }
        BAIL
    }
}
}

jit_abi! {
/// An error was thrown in the compiled function whose frame is `frame`, or in compiled code it called, whose frames
/// are still above it. As `Vm::unwind` does for that frame: if it has a `catch` or `finally` around where it is, the
/// frames above it go, the error is put where the handler wants it, and this says which of the function's
/// instructions the handler starts at; else `u32::MAX`, with the error in `vm.jit.value`.
pub(crate) fn catch(vm: &mut Vm, error: u64, frame: *const Frame) -> u32 {
    let error = Value(error);
    let index = (frame as usize - vm.frames.as_ptr() as usize) / size_of::<Frame>();
    while vm.frames.len() > index + 1 {
        vm.pop_frame();
    }
    let frame = vm.frames[index];
    let proto = &vm.protos[frame.proto as usize];
    let at = frame.pc.wrapping_sub(proto.start).wrapping_sub(1);
    let is_generator_return = error == Value::GENERATOR_RETURN;
    let found = proto
        .handlers
        .iter()
        .find(|handler| {
            handler.start <= at && at < handler.end && !(is_generator_return && handler.is_catch)
        })
        .map(|handler| (handler.target, handler.reg, proto.start));
    match found {
        Some((target, reg, start)) => {
            vm.frames[index].pc = start + target;
            vm.stack[frame.base as usize + reg as usize] = error;
            target
        }
        None => {
            vm.jit.value = error;
            u32::MAX
        }
    }
}
}

// Small helpers for what the generic way would do the same as, taking what they need in registers. None of them can
// fail, or collect garbage (that's only at safe points), so they need no frame.

jit_abi! {
/// NewObject's object.
pub(crate) fn new_object(vm: &mut Vm, capacity: u32) -> u64 {
    let shape = vm.realm.object_shape;
    vm.alloc_for_jit(shape, Kind::Plain, capacity as usize).0
}
}

jit_abi! {
/// NewArray's array.
pub(crate) fn new_array(vm: &mut Vm, capacity: u32) -> u64 {
    if capacity <= 4 {
        return vm.small_array(&[]).0;
    }
    vm.debt += capacity as usize * 8;
    let shape = vm.realm.array_shape;
    let items = Vec::with_capacity(capacity as usize);
    vm.alloc_for_jit(shape, Kind::Array(items.into()), 0).0
}
}

jit_abi! {
/// ArrayPush, onto the array an array literal is making.
pub(crate) fn array_push(vm: &mut Vm, array: u64, value: u64) {
    if let Kind::Array(items) = &mut vm.objects[Value(array).index()].kind {
        items.push(Value(value));
    }
    vm.debt += 8;
}
}

jit_abi! {
/// Closure's function, made in the frame on top.
pub(crate) fn closure(vm: &mut Vm, proto: u32) -> u64 {
    let base = vm.frames.last().expect("compiled code runs in a frame").base as usize;
    vm.make_closure(proto, base).0
}
}

jit_abi! {
/// NewCell's and CopyCell's cell, holding `value`.
pub(crate) fn new_cell(vm: &mut Vm, value: u64) -> u64 {
    vm.new_cell(Value(value)).0
}
}

jit_abi! {
/// TypeOf's string.
pub(crate) fn type_of(vm: &mut Vm, value: u64) -> u64 {
    vm.type_of(Value(value)).0
}
}

jit_abi! {
/// Gives the object `handle` a property at the end, moving it to `shape`, as SetProp does when its cache says how.
pub(crate) fn add_slot(vm: &mut Vm, handle: u32, shape: u32, value: u64) {
    // Not through `objects[..]`: compiled code has checked it isn't old.
    let found = unsafe { &mut *vm.objects.as_mut_ptr().add(handle as usize) };
    found.shape = shape;
    found.push_slot(Value(value));
    vm.debt += 8;
}
}

jit_abi! {
/// The same for DefineField, which doesn't count it as made.
pub(crate) fn define_slot(vm: &mut Vm, handle: u32, shape: u32, value: u64) {
    // As above.
    let found = unsafe { &mut *vm.objects.as_mut_ptr().add(handle as usize) };
    found.shape = shape;
    found.push_slot(Value(value));
}
}

jit_abi! {
/// Keeps the arguments of the frame compiled code has just made, for a function that keeps them, as
/// `enter_function` does.
pub(crate) fn keep_args(vm: &mut Vm, argc: u32) {
    let base = vm.frames.last().expect("a frame was just made").base as usize;
    let mut args = vm.spare_args.pop().unwrap_or_default();
    args.extend_from_slice(&vm.stack[base + 1..base + 1 + argc as usize]);
    vm.kept_args.push(args);
}
}

jit_abi! {
/// Makes the `this` of the `new` at `pc`, whose callee compiled code has found to be one it can call itself, or of
/// the `super(...)` there, whose parent isn't derived: for its `new.target`, which is the frame's.
pub(crate) fn this_for(vm: &mut Vm, pc: u32) -> u32 {
    #[cfg(feature = "jit-opt")]
    let site = super::feedback::site_of(vm, pc);
    let frame = here(vm, pc);
    let (f, target) = match vm.code[pc as usize] {
        Op::New { f, .. } => (f, vm.stack[frame.base as usize + f as usize]),
        Op::SuperCall { f, .. } => (f, frame.new_target),
        _ => unreachable!("only a `new` or a `super(...)` makes its `this` here"),
    };
    let place = frame.base as usize + f as usize;
    match vm.create_this(target) {
        Ok(this) => {
            vm.stack[place + 1] = this;
            #[cfg(feature = "jit-opt")]
            super::feedback::made(vm, site, vm.stack[place], this);
            OK
        }
        Err(error) => throw(vm, error),
    }
}
}

jit_abi! {
/// The calls at `pc` and the others like it that knew nothing of what they'd call have counted `counter` (the
/// running code's) down: compiles the function again, or looks whether it has been (`Vm::jit_recompile`), and says
/// where its new code for that instruction is, or 0 to go on in this.
pub(crate) fn recompile(vm: &mut Vm, pc: u32, counter: *mut u64) -> u64 {
    let frame = here(vm, pc);
    vm.jit_recompile(frame.proto, pc, counter) as u64
}
}

jit_abi! {
/// The count towards the optimizing tier of the compiled function on top has come to 0 (see `jit::feedback::due`).
#[cfg(feature = "jit-opt")]
pub(crate) fn opt_due(vm: &mut Vm) {
    let proto = vm.frames.last().expect("compiled code runs in a frame").proto;
    vm.opt_due(proto);
}
}

jit_abi! {
/// A safe point that's due: collects garbage, with the frame's `pc` saying it's at the instruction before `pc`.
pub(crate) fn collect(vm: &mut Vm, pc: u32) -> u32 {
    let frame = vm.frames.last_mut().expect("compiled code runs in a frame");
    frame.pc = pc;
    vm.top = frame.top as usize;
    vm.collect_garbage();
    OK
}
}

jit_abi! {
/// A compiled function's frame doesn't fit: makes room for it, as `enter_function` would, or says why not. The
/// caller's frame is on top, and `needed` is how far the stack has to reach.
pub(crate) fn grow(vm: &mut Vm, needed: u32) -> u32 {
    let frame = *vm.frames.last().expect("compiled code runs in a frame");
    vm.top = frame.top as usize;
    if vm.frames.len() >= crate::vm::MAX_FRAMES {
        let error = vm.range_error("Maximum call stack size exceeded");
        return throw(vm, error);
    }
    if let Err(error) = vm.reserve_stack(needed as usize) {
        return throw(vm, error);
    }
    if vm.frames.len() >= vm.frames.capacity() {
        // Pushing one asks for more memory, and the place is then left for the frame that's coming.
        vm.frames.push(frame);
        vm.frames.pop();
    }
    OK
}
}

jit_abi! {
/// Finishes returning from a frame with flags (a `new`, or one that keeps its arguments), as `finish_return!` does
/// before the frame goes. The value to hand back goes in `vm.jit.value`.
pub(crate) fn finish_return(vm: &mut Vm, value: u64) -> u32 {
    match vm.finish_special_frame(Value(value)) {
        Ok(value) => {
            vm.jit.value = value;
            OK
        }
        Err(error) if error == Value::ENTERED => LEFT,
        Err(error) => throw(vm, error),
    }
}
}

jit_abi! {
/// InitialYield at `pc`, at the start of a generator: makes the generator, with the frame as it is, which then goes,
/// as the interpreter's InitialYield does. The generator goes in `vm.jit.value`, for compiled code to return.
pub(crate) fn initial_yield(vm: &mut Vm, pc: u32) -> u32 {
    let frame = here(vm, pc);
    let generator = vm.suspend_new_generator();
    vm.pop_frame();
    let flags = vm.protos[frame.proto as usize].flags;
    debug_assert!(flags & IS_GENERATOR != 0 && flags & IS_ASYNC == 0, "only a generator is compiled");
    let default = vm.realm.generator_proto;
    let chosen = match vm.get(Value::object(frame.closure), crate::text::PROTOTYPE) {
        Ok(found) if found.is_object() => found.handle(),
        _ => default,
    };
    if chosen != default {
        vm.set_proto(generator.handle(), chosen);
    }
    vm.jit.value = generator;
    OK
}
}

jit_abi! {
/// Yield at `pc`: the generator stops, with its frame as it is, and the frame goes, as the interpreter's Yield does.
/// What it yields goes in `vm.jit.value`, for compiled code to return: to `resume_generator`, or, for a generator a
/// `for of` carried on in place, to whoever went into this code, which puts it where that loop wants it.
pub(crate) fn yield_value(vm: &mut Vm, pc: u32) -> u32 {
    let Op::Yield { dst, src } = vm.code[pc as usize] else {
        unreachable!("only Yield comes here");
    };
    let frame = here(vm, pc);
    let value = vm.stack[frame.base as usize + src as usize];
    if frame.flags & crate::vm::RESUMED_HERE != 0 {
        vm.yield_here(dst, value);
    } else {
        vm.suspend_generator(dst);
        vm.pop_frame();
        vm.yielded = crate::generator::Suspended::Yield;
    }
    vm.jit.value = value;
    OK
}
}
