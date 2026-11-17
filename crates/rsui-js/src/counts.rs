//! Counting what the interpreter runs, for measuring, only in builds with the `counts` feature:
//! each instruction by kind, natives by name, each function's calls, loop turns and instructions, the objects made at
//! each place in the code, and each time `Vm::epoch` moves (a prototype changing), by where. It's written to the file
//! `RSUI_COUNTS` names, a part at a time: whoever runs the engine says where a part starts with [`mark`] (rsui-tsx
//! does at each step of `--do`, and a script with `__rsuiCount(name)`), and each part says what was counted from its
//! mark to the next.
//!
//! Compiled code isn't counted, so it's for `RSUI_JIT=0` (or a build without the JIT). What's counted is the program's,
//! not the engine's: how often a function runs is the same whichever tier runs it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;

use crate::op::Op;
use crate::vm::Vm;

/// How hot a function has to have got, in calls and loop turns, for the hotness lines (see `write_part`).
const THRESHOLDS: [u64; 5] = [1_000, 3_000, 10_000, 13_000, 30_000];

#[derive(Default)]
struct Counts {
    /// The part being counted.
    label: String,
    /// Each instruction kind, with one of them for its name.
    ops: HashMap<std::mem::Discriminant<Op>, (u64, Op), foldhash::fast::FixedState>,
    /// Each native, by its function and name.
    natives: HashMap<(usize, u32), (u64, String), foldhash::fast::FixedState>,
    /// By proto: calls, loop turns and instructions in this part; calls and turns since the start.
    calls: Vec<u64>,
    turns: Vec<u64>,
    instructions: Vec<u64>,
    total: Vec<u64>,
    /// By proto: its name and where it's from, found the first time it's counted.
    names: Vec<Option<String>>,
    /// Where the instruction running is, for the objects it makes: its proto and its place in it.
    at: (u32, u32),
    /// Objects made, by the instruction's proto, place and op, and the object's kind.
    made: HashMap<(u32, u32, &'static str), u64, foldhash::fast::FixedState>,
    /// The epoch's moves, by where and the property's name (an atom, or `u32::MAX` for none).
    epochs: HashMap<(&'static str, u32), u64, foldhash::fast::FixedState>,
}

thread_local! {
    static COUNTS: RefCell<Counts> = RefCell::new(Counts { label: "start".into(), ..Counts::default() });
}

fn on() -> bool {
    use std::sync::atomic::{AtomicU8, Ordering};
    static ON: AtomicU8 = AtomicU8::new(0);
    match ON.load(Ordering::Relaxed) {
        0 => {
            let on = std::env::var_os("RSUI_COUNTS").is_some();
            ON.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        known => known == 2,
    }
}

fn grow(list: &mut Vec<u64>, proto: u32) -> &mut u64 {
    let at = proto as usize;
    if list.len() <= at {
        list.resize(at + 1, 0);
    }
    &mut list[at]
}

impl Counts {
    fn name(&mut self, vm: &Vm, proto: u32) {
        let at = proto as usize;
        if self.names.len() <= at {
            self.names.resize(at + 1, None);
        }
        if self.names[at].is_none() {
            // A function that's gone by now (its proto's place given back) is named by its number.
            let Some(found) = vm.protos.get(at) else {
                self.names[at] = Some(format!("proto {at}, gone"));
                return;
            };
            let name = vm.atom_text(found.name);
            let line = found.lines.first().map_or(0, |&(_, line)| line);
            let file = vm.atom_text(found.file);
            let file = file.rsplit(['/', '\\']).next().unwrap_or_default().to_owned();
            self.names[at] = Some(format!(
                "{} ({file}:{line})",
                if name.is_empty() { "<anonymous>" } else { &name }
            ));
        }
    }
}

/// An instruction about to run, in a frame of `proto`, at `pc` in the engine's code.
#[inline]
pub(crate) fn op(op: &Op, proto: u32, pc: u32) {
    if !on() {
        return;
    }
    COUNTS.with(|counts| {
        let mut counts = counts.borrow_mut();
        counts.ops.entry(std::mem::discriminant(op)).or_insert((0, *op)).0 += 1;
        *grow(&mut counts.instructions, proto) += 1;
        counts.at = (proto, pc);
    });
}

/// A frame made for a call of `proto`.
#[inline]
pub(crate) fn call(vm: &Vm, proto: u32) {
    if !on() {
        return;
    }
    COUNTS.with(|counts| {
        let mut counts = counts.borrow_mut();
        *grow(&mut counts.calls, proto) += 1;
        *grow(&mut counts.total, proto) += 1;
        counts.name(vm, proto);
    });
}

/// A jump back to the top of a loop, in a frame of `proto`.
#[inline]
pub(crate) fn turn(vm: &Vm, proto: u32) {
    if !on() {
        return;
    }
    COUNTS.with(|counts| {
        let mut counts = counts.borrow_mut();
        *grow(&mut counts.turns, proto) += 1;
        *grow(&mut counts.total, proto) += 1;
        counts.name(vm, proto);
    });
}

/// A native called from the interpreter.
#[inline]
pub(crate) fn native(vm: &Vm, func: usize, name: crate::text::Atom) {
    if !on() {
        return;
    }
    COUNTS.with(|counts| {
        let mut counts = counts.borrow_mut();
        counts
            .natives
            .entry((func, name.0))
            .or_insert_with(|| (0, vm.atom_text(name)))
            .0 += 1;
    });
}

/// An object made, of this kind, by the instruction running.
#[inline]
pub(crate) fn made(kind: &crate::object::Kind) {
    if !on() {
        return;
    }
    use crate::object::Kind;
    let name = match kind {
        Kind::Plain => "object",
        Kind::Array(_) => "array",
        Kind::Arguments(_) => "arguments",
        Kind::Function { .. } => "closure",
        Kind::Native(_) => "native",
        Kind::Bound(_) => "bound",
        Kind::Iter(_) => "iterator",
        Kind::Generator(_) => "generator",
        Kind::Promise(_) => "promise",
        Kind::Map(_) | Kind::Set(_) => "map or set",
        Kind::Proxy(_) => "proxy",
        Kind::Accessor { .. } => "accessor",
        Kind::Error => "error",
        _ => "other",
    };
    COUNTS.with(|counts| {
        let mut counts = counts.borrow_mut();
        let (proto, pc) = counts.at;
        *counts.made.entry((proto, pc, name)).or_insert(0) += 1;
    });
}

/// `Vm::epoch` moved, at `place`, for the property named `key` (or `u32::MAX`).
#[inline]
pub(crate) fn epoch(place: &'static str, key: u32) {
    if !on() {
        return;
    }
    COUNTS.with(|counts| *counts.borrow_mut().epochs.entry((place, key)).or_insert(0) += 1);
}

/// Writes what's been counted since the last mark as a part of its own, and starts a part named `label`.
pub fn mark(vm: &Vm, label: &str) {
    if !on() {
        return;
    }
    COUNTS.with(|counts| {
        let mut counts = counts.borrow_mut();
        write_part(vm, &mut counts);
        counts.label = label.to_owned();
    });
}

/// The name of an op, from its `Debug`.
fn op_name(op: &Op) -> String {
    let text = format!("{op:?}");
    text.split([' ', '{', '(']).next().unwrap_or("?").to_owned()
}

fn write_part(vm: &Vm, counts: &mut Counts) {
    let Some(path) = std::env::var_os("RSUI_COUNTS") else {
        return;
    };
    let mut out = String::new();
    let instructions: u64 = counts.instructions.iter().sum();
    let calls: u64 = counts.calls.iter().sum();
    let turns: u64 = counts.turns.iter().sum();
    let made: u64 = counts.made.values().sum();
    let epochs: u64 = counts.epochs.values().sum();
    out.push_str(&format!(
        "== part {}\ninstructions {instructions} calls {calls} loop-turns {turns} objects-made {made} epoch-moves {epochs}\n",
        counts.label
    ));
    if instructions == 0 && made == 0 && epochs == 0 {
        out.push('\n');
        let _ = std::fs::OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| f.write_all(out.as_bytes()));
        return;
    }
    let mut epoch_rows: Vec<_> = counts.epochs.iter().collect();
    epoch_rows.sort_by(|a, b| b.1.cmp(a.1));
    for (&(place, key), count) in epoch_rows {
        let atom = crate::text::Atom(key);
        let name = if key == u32::MAX || vm.atoms.is_free(atom) { String::new() } else { format!(": {}", vm.atom_text(atom)) };
        out.push_str(&format!("epoch {count:8} {place}{name}\n"));
    }
    let mut ops: Vec<(u64, String)> = counts.ops.values().map(|(count, op)| (*count, op_name(op))).collect();
    ops.sort_by(|a, b| b.0.cmp(&a.0));
    for (count, name) in ops.iter().take(40) {
        out.push_str(&format!("op {count:10} {name}\n"));
    }
    let mut natives: Vec<_> = counts.natives.values().collect();
    natives.sort_by(|a, b| b.0.cmp(&a.0));
    for (count, name) in natives.iter().take(30) {
        out.push_str(&format!("native {count:8} {name}\n"));
    }
    // Each function, by how hot it was in this part, with how hot it is in all.
    let protos = counts.calls.len().max(counts.turns.len()).max(counts.instructions.len());
    let get = |list: &Vec<u64>, at: usize| list.get(at).copied().unwrap_or(0);
    let mut rows: Vec<usize> = (0..protos)
        .filter(|&at| get(&counts.calls, at) + get(&counts.turns, at) + get(&counts.instructions, at) > 0)
        .collect();
    rows.sort_by_key(|&at| std::cmp::Reverse(get(&counts.calls, at) + get(&counts.turns, at)));
    out.push_str(&format!("functions {} (calls turns instructions total-so-far name)\n", rows.len()));
    for &at in rows.iter().take(80) {
        let name = counts.names.get(at).cloned().flatten().unwrap_or_else(|| format!("proto {at}"));
        out.push_str(&format!(
            "fn {:8} {:8} {:9} {:9} {name}\n",
            get(&counts.calls, at),
            get(&counts.turns, at),
            get(&counts.instructions, at),
            get(&counts.total, at)
        ));
    }
    // How much of this part's instructions are in functions that were that hot before it started, and by its end.
    for threshold in THRESHOLDS {
        let mut before = 0u64;
        let mut by_end = 0u64;
        let mut functions_before = 0;
        let mut functions_by_end = 0;
        for at in 0..protos {
            let here = get(&counts.instructions, at);
            let total = get(&counts.total, at);
            let earlier = total - get(&counts.calls, at) - get(&counts.turns, at);
            if earlier >= threshold {
                before += here;
                functions_before += 1;
            }
            if total >= threshold {
                by_end += here;
                functions_by_end += 1;
            }
        }
        out.push_str(&format!(
            "hot {threshold:6}: before {functions_before} functions, {:.1}% of instructions; by end {functions_by_end}, {:.1}%\n",
            100.0 * before as f64 / instructions.max(1) as f64,
            100.0 * by_end as f64 / instructions.max(1) as f64
        ));
    }
    // Objects made, by kind and by place.
    let mut kinds: HashMap<&str, u64> = HashMap::new();
    for (&(_, _, kind), &count) in &counts.made {
        *kinds.entry(kind).or_insert(0) += count;
    }
    let mut kinds: Vec<_> = kinds.into_iter().collect();
    kinds.sort_by(|a, b| b.1.cmp(&a.1));
    for (kind, count) in kinds {
        out.push_str(&format!("made {count:8} {kind}\n"));
    }
    let mut places: Vec<((u32, u32, &'static str), u64)> = counts.made.iter().map(|(&at, &count)| (at, count)).collect();
    places.sort_by(|a, b| b.1.cmp(&a.1));
    for ((proto, pc, kind), count) in places.into_iter().take(60) {
        counts.name(vm, proto);
        let name = counts.names[proto as usize].clone().unwrap_or_default();
        let start = vm.protos.get(proto as usize).map_or(0, |found| found.start);
        let op = vm.code.get(pc as usize).map(op_name).unwrap_or_default();
        out.push_str(&format!("place {count:8} {kind} {name} @{} {op}\n", pc.wrapping_sub(start)));
    }
    out.push('\n');
    let _ = std::fs::OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| f.write_all(out.as_bytes()));
    counts.ops.clear();
    counts.natives.clear();
    counts.calls.iter_mut().for_each(|count| *count = 0);
    counts.turns.iter_mut().for_each(|count| *count = 0);
    counts.instructions.iter_mut().for_each(|count| *count = 0);
    counts.made.clear();
    counts.epochs.clear();
}

/// Gives a script `__rsuiCount(name)`, which marks a part (see [`mark`]).
pub(crate) fn install(vm: &mut Vm) {
    let function = vm.host_function("__rsuiCount", 1, |vm, args| {
        let label = args.get(vm, 0);
        let label = if label.is_string() { vm.to_rust_string(label)? } else { "part".to_owned() };
        mark(vm, &label);
        Ok(crate::value::Value::UNDEFINED)
    });
    vm.set_global("__rsuiCount", function);
}
