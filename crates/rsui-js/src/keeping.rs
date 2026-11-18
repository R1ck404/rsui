//! What a component works out as it renders, kept for its next render. A plain call in a component's or a hook's own body,
//! as `const items = sortEntries(listing.entries, sort)`, gives what it gave last time, without being made again, when
//! it's given the same and what it was worked out from hasn't changed, so an app writes the calculation plainly and it
//! only runs again when what it depends on does. See "What a render works out" in rsui-tsx's DESIGN.md.
//!
//! The compiler puts a [`Op::Note`] of `NOTE_KEEP` before each such call, and one of `NOTE_KEPT` after it. While nothing
//! renders, each is a test of a byte. While something does, the first looks for what this place gave last time, in the
//! same component and the same run of it through the render (a loop in the body runs a place more than once):
//!
//! - **Given again** when the same function, or one made from the same code that captured the same, is given the same
//!   arguments, each the same value or one that's the same by [`Vm::props_equivalent`]'s rules, and nothing it was worked
//!   out from has changed since.
//! - **Worked out here** otherwise, when the last call made at this place took long enough or made enough to be worth
//!   keeping ([`WORTH_TIME`], [`WORTH_MADE`]): the Note makes the call itself, watching for anything that would make keeping
//!   it wrong, and keeps what it gave if nothing did.
//! - **Made the usual way** otherwise, timed by the two Notes, which is how a place is found to be worth keeping.
//!
//! What a call is worked out from is taken to be everything that can be reached from the function and what it's given:
//! what's in them, what's in that, and so on, through the prototypes, and through what each function reached captured, so
//! the variables at the top of a module that a function reads are part of it. Each object of that is watched through the
//! collector's write barrier (see `Watch` in gc.rs), so a change to any of them, by anything, is seen, and each cell a
//! function reached captures is kept with what was in it, which is looked at again before anything is given again. The
//! result is watched too, so a caller that changes what it was given back has it worked out again next time.
//!
//! What's worked out isn't kept when the call, or anything it called, changed something it didn't make (a watched object
//! changed, or a cell of what was watched holds something else afterwards), called something of the host's or something
//! that isn't pure (`Math.random`, `Date.now`, `new Date()`, `performance.now`, anything of a promise: see
//! `NativeData::pure`), threw, or reached what can't be watched (a typed array's bytes, which change without the barrier,
//! a promise, a generator, an iterator, a proxy, or what's held weakly), or more than [`MOST_WATCHED`] objects. A place that
//! isn't kept that way [`MOST_REFUSALS`] times in a row is only made the usual way from then on.
//!
//! What isn't covered is what's reached only through the global object, which isn't looked into, and the language's own
//! prototypes and what's on the global object as the engine starts, which aren't watched (see [`Keeping::built_in`]).

use crate::object::{Kind, NONE};
use crate::op::{IS_ASYNC, IS_CLASS, IS_GENERATOR, NOTE_KEEP, NOTE_KEPT, Op, SAYS_APP_FILE};
use crate::reads::NO_READS;
use crate::value::Value;
use crate::vm::{Run, Vm};
use foldhash::fast::FixedState;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a call has to take to be worth keeping what it gives: a fraction of a frame, which watching what it's worked
/// out from costs much less than.
pub(crate) const WORTH_TIME: Duration = Duration::from_micros(50);
/// How much a call has to make to be worth keeping what it gives, even when it's quick, as what it makes has to be
/// collected.
pub(crate) const WORTH_MADE: usize = 64 * 1024;
/// How many objects watching what one call is worked out from can look at, at most.
pub(crate) const MOST_WATCHED: usize = 400_000;
/// How many results are kept, at most, the least recently given going first.
const MOST_KEPT: usize = 16;
/// How long a result is kept without being given again, as the component it's for may have gone, which nothing says.
const KEPT_FOR: Duration = Duration::from_secs(30);
/// How many times in a row a place can be worked out without being kept before it's only made the usual way.
pub(crate) const MOST_REFUSALS: u8 = 3;
/// How many calls a place that wasn't kept too many times is made the usual way before it's tried again.
const RETRY_AFTER: u8 = 16;
/// How many cells can be kept with what they held, at most.
const MOST_CELLS: usize = 8192;

/// How many calls were given again, worked out and kept, and worked out and not kept, for tests and for comparing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeptCounts {
    pub given_again: u64,
    pub kept: u64,
    pub not_kept: u64,
}

/// What a place gave, kept for the next time it runs in the same component.
struct Kept {
    owner: u32,
    pc: u32,
    run: u32,
    function: Value,
    args: Box<[Value]>,
    result: Value,
    /// What `Watch::changes` was when it was kept: it's given again only while it's the same.
    changes: u32,
    used: u64,
    /// When it was last given or kept.
    given_at: Instant,
}

/// What's known of a place.
#[derive(Default, Clone, Copy)]
struct Site {
    /// Whether the last call made there was worth keeping.
    worth: bool,
    /// How many calls made there have been timed, as the first, which compiles what it calls, says little.
    timed: u8,
    /// How many times in a row it was worked out and not kept.
    refusals: u8,
    /// How many calls have been made there the usual way since that was too many, after [`RETRY_AFTER`] of which it's
    /// tried again, as what it's given can be different now.
    rested: u8,
}

/// A call made the usual way that's being timed.
struct Timing {
    depth: u32,
    pc: u32,
    began: Instant,
    made: usize,
    collections: u32,
}

#[derive(Default)]
pub(crate) struct Keeping {
    kept: Vec<Kept>,
    sites: HashMap<u32, Site, FixedState>,
    timing: Vec<Timing>,
    /// How many times each place has run in each render so far, by what [`Vm::start_reads`] gave it and where it is.
    runs: HashMap<(u32, u32), u32, FixedState>,
    /// The cells captured by the functions that are watched, with what each held then: anything kept is only given again
    /// while they all hold the same.
    cells: Vec<(Value, Value)>,
    cell_numbers: HashMap<u32, (), FixedState>,
    /// What `Watch::changes` was when everything watched was last looked through: nothing watched has changed since while
    /// it's the same, so what's watched can be taken to have been looked through already.
    clean_at: u32,
    /// What's being worked out, held for the collector while it is.
    working_on: Vec<Value>,
    /// How many calls are being worked out, inside which a place is only made the usual way.
    pub(crate) working: u32,
    /// Whether something that isn't pure has been called in what's being worked out.
    pub(crate) spoiled: bool,
    clock: u64,
    /// The language's own prototypes and what's on the global object as the engine starts, which aren't watched or looked
    /// into: the engine changes some of them itself, as a method written in JavaScript is put in place the first time it's
    /// called, and a program doesn't.
    built_in: HashMap<u32, (), FixedState>,
    /// Which objects a walk has been to, by number, and the list of them, kept between walks for their room.
    seen: Vec<u64>,
    found: Vec<u32>,
    counts: KeptCounts,
}

/// Whether results are kept: `RSUI_KEEP=0` turns it off, for comparing.
fn keeping_enabled() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("RSUI_KEEP").map_or(true, |value| value != "0"))
}

/// Whether `RSUI_KEEP_LOG` asks for a line about each call worked out and each given again.
fn logging() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RSUI_KEEP_LOG").is_some_and(|value| value != "0"))
}

impl Keeping {
    /// Starts counting runs of each place again for a render that's starting, or starting again.
    pub(crate) fn restart(&mut self, id: u32) {
        if !self.runs.is_empty() {
            self.runs.retain(|&(render, _), _| render != id);
        }
    }

    /// Forgets how many times each place ran, as nothing's rendering now.
    pub(crate) fn rendered(&mut self) {
        self.runs.clear();
        self.timing.clear();
    }

    /// Everything held, for the collector.
    pub(crate) fn each_value(&self, each: &mut impl FnMut(Value)) {
        for kept in &self.kept {
            each(kept.function);
            each(kept.result);
            for arg in &kept.args {
                each(*arg);
            }
        }
        for (cell, value) in &self.cells {
            each(*cell);
            each(*value);
        }
        for value in &self.working_on {
            each(*value);
        }
    }
}

impl Vm {
    /// Notes the language's own prototypes, and what's on the global object, as the engine has just made them: see
    /// [`Keeping::built_in`].
    pub(crate) fn note_built_ins(&mut self) {
        let realm = &self.realm;
        let mut built_in: Vec<u32> = vec![
            realm.object_proto,
            realm.function_proto,
            realm.array_proto,
            realm.iterator_proto,
            realm.native_iterator_proto,
            realm.generator_proto,
            realm.async_generator_proto,
            realm.generator_function_proto,
            realm.async_function_proto,
            realm.async_generator_function_proto,
        ];
        let globals: Vec<Value> = self.objects[self.global as usize].slots().to_vec();
        for value in globals {
            if !value.is_object() || matches!(self.objects[value.index()].kind, Kind::Accessor { .. }) {
                continue;
            }
            built_in.push(value.handle());
            let shape = self.objects[value.index()].shape;
            if let Some((slot, _)) = self.shapes.lookup(shape, crate::text::PROTOTYPE)
                && let Some(prototype) = self.objects[value.index()].slots().get(slot as usize)
                && prototype.is_object()
            {
                built_in.push(prototype.handle());
            }
        }
        self.keeping.built_in = built_in.into_iter().filter(|&handle| handle != NONE).map(|handle| (handle, ())).collect();
    }

    /// How many calls have been given again, kept and not kept: see `keeping.rs`.
    pub fn kept_counts(&self) -> KeptCounts {
        self.keeping.counts
    }

    /// What a Note does while something renders: a `NOTE_KEEP` gives true when it made the call itself, or gave what it
    /// gave last time, in which case the call and the `NOTE_KEPT` after it are to be stepped over.
    #[cold]
    pub(crate) fn noted(&mut self, at: usize) -> Run<bool> {
        let Op::Note { value, root, kind, .. } = self.code[at] else {
            return Ok(false);
        };
        match kind {
            NOTE_KEEP => self.keep_call(at, value, root),
            NOTE_KEPT => {
                self.kept_call_ended(at);
                Ok(false)
            }
            _ => {
                self.note(at);
                Ok(false)
            }
        }
    }

    fn keep_call(&mut self, at: usize, dst: u16, f: u16) -> Run<bool> {
        if self.keeping.working != 0 || !keeping_enabled() {
            return Ok(false);
        }
        let current = self.current_reads();
        if current == NO_READS {
            return Ok(false);
        }
        let Some(owner) = self.reads.owner_of(current) else {
            return Ok(false);
        };
        let (Some(frame), Some(Op::NoteKey { reg: argc })) = (self.frames.last(), self.code.get(at + 1).copied()) else {
            return Ok(false);
        };
        let (base, depth) = (frame.base as usize, self.frames.len() as u32);
        let start = base + f as usize;
        let argc = argc as usize;
        if !self.keepable(self.stack[start]) {
            return Ok(false);
        }
        // What a call that threw was timing is gone.
        while self.keeping.timing.last().is_some_and(|timing| timing.depth >= depth) {
            self.keeping.timing.pop();
        }
        let pc = at as u32;
        let run = {
            let runs = self.keeping.runs.entry((current, pc)).or_insert(0);
            *runs += 1;
            *runs - 1
        };
        if let Some(index) = self
            .keeping
            .kept
            .iter()
            .position(|kept| kept.owner == owner && kept.pc == pc && kept.run == run)
        {
            if self.kept_holds(index, start, argc) {
                self.keeping.clock += 1;
                let clock = self.keeping.clock;
                let kept = &mut self.keeping.kept[index];
                kept.used = clock;
                kept.given_at = Instant::now();
                let result = kept.result;
                self.stack[base + dst as usize] = result;
                self.keeping.counts.given_again += 1;
                if logging() {
                    let name = self.name_of_callee(self.stack[start]);
                    eprintln!("keep: gave {name} again");
                }
                return Ok(true);
            }
            if let Some(index) = self
                .keeping
                .kept
                .iter()
                .position(|kept| kept.owner == owner && kept.pc == pc && kept.run == run)
            {
                self.keeping.kept.swap_remove(index);
            }
        }
        let site = self.keeping.sites.get(&pc).copied().unwrap_or_default();
        if site.worth && site.refusals < MOST_REFUSALS {
            return self.work_out(owner, pc, run, start, argc, base + dst as usize);
        }
        self.keeping.timing.push(Timing {
            depth,
            pc,
            began: Instant::now(),
            made: self.debt,
            collections: self.gc_count,
        });
        Ok(false)
    }

    /// A function made from the app's own code that can be called and give its answer straight away.
    fn keepable(&self, callee: Value) -> bool {
        if !callee.is_object() {
            return false;
        }
        let Kind::Function { proto, .. } = self.objects[callee.index()].kind else {
            return false;
        };
        let proto = &self.protos[proto as usize];
        proto.flags & (IS_ASYNC | IS_GENERATOR | IS_CLASS) == 0 && proto.says & SAYS_APP_FILE != 0
    }

    fn kept_call_ended(&mut self, at: usize) {
        let depth = self.frames.len() as u32;
        let Some(timing) = self.keeping.timing.last() else {
            return;
        };
        if timing.depth != depth || timing.pc as usize + 3 != at {
            return;
        }
        let timing = self.keeping.timing.pop().expect("there's one");
        let made = if self.gc_count == timing.collections { self.debt.saturating_sub(timing.made) } else { usize::MAX };
        let worth = timing.began.elapsed() >= WORTH_TIME || made >= WORTH_MADE;
        let site = self.keeping.sites.entry(timing.pc).or_default();
        site.timed = site.timed.saturating_add(1);
        site.worth = worth && site.timed > 1;
        if site.refusals >= MOST_REFUSALS {
            site.rested += 1;
            if site.rested >= RETRY_AFTER {
                site.refusals = 0;
                site.rested = 0;
            }
        }
    }

    /// Whether what's kept at `index` can be given again to the call whose callee is at `start` on the stack, with `argc`
    /// arguments after where its `this` goes.
    fn kept_holds(&mut self, index: usize, start: usize, argc: usize) -> bool {
        if !self.still_clean() || self.keeping.kept[index].changes != self.objects.watch.changes {
            return false;
        }
        let kept = &self.keeping.kept[index];
        if kept.args.len() != argc {
            return false;
        }
        let callee = self.stack[start];
        if !self.same_function(kept.function, callee) {
            return false;
        }
        let kept = &self.keeping.kept[index];
        kept.args.iter().zip(&self.stack[start + 2..start + 2 + argc]).all(|(&old, &new)| self.same_arg(old, new))
    }

    /// Whether nothing watched has changed since it was all looked through, and each cell watched holds what it did. If
    /// either has, everything is forgotten, as none of it can be given again.
    fn still_clean(&mut self) -> bool {
        let cells = &self.cells;
        if self.keeping.clean_at == self.objects.watch.changes
            && self.keeping.cells.iter().all(|(cell, value)| cells[cell.index()] == *value)
        {
            return true;
        }
        if logging() && !self.keeping.kept.is_empty() {
            let why = if self.keeping.clean_at != self.objects.watch.changes {
                let changed = self.objects.watch.last_changed;
                format!("an object changed, {}", self.what_changed(changed))
            } else {
                "a variable a function captured holds something else".to_owned()
            };
            eprintln!("keep: forgot what was kept, as {why}");
        }
        self.forget_kept();
        false
    }

    /// What an object is, in a few words, for the log.
    fn what_changed(&mut self, handle: usize) -> String {
        if handle >= self.objects.len() {
            return "which has gone".to_owned();
        }
        let kind = match &self.objects[handle].kind {
            Kind::Array(items) => return format!("an array of {}", items.len()),
            Kind::Function { proto, .. } => return format!("function {}", self.atom_text(self.protos[*proto as usize].name)),
            Kind::Map(data) => return format!("a Map of {}", data.entries.len()),
            Kind::Plain => "an object",
            _ => "something",
        };
        let keys: Vec<String> =
            self.own_keys(handle as u32).iter().take(6).map(|(atom, _)| self.atom_text(*atom).to_string()).collect();
        format!("{kind} with {}", keys.join(", "))
    }

    /// Forgets everything kept, and stops watching.
    fn forget_kept(&mut self) {
        self.objects.stop_watching();
        let keeping = &mut self.keeping;
        keeping.kept.clear();
        keeping.cells.clear();
        keeping.cell_numbers.clear();
        keeping.clean_at = self.objects.watch.changes;
    }

    fn same_function(&self, old: Value, new: Value) -> bool {
        if old == new {
            return true;
        }
        if !old.is_object() || !new.is_object() {
            return false;
        }
        let (
            Kind::Function {
                proto: old_proto,
                upvalues: old_captured,
                home: old_home,
                ..
            },
            Kind::Function {
                proto: new_proto,
                upvalues: new_captured,
                home: new_home,
                ..
            },
        ) = (&self.objects[old.index()].kind, &self.objects[new.index()].kind)
        else {
            return false;
        };
        old_proto == new_proto
            && old_home == new_home
            && old_captured.len() == new_captured.len()
            && old_captured.iter().zip(new_captured.iter()).all(|(&x, &y)| {
                x == y || (x.is_cell() && y.is_cell() && self.cells[x.index()] == self.cells[y.index()])
            })
    }

    fn same_arg(&self, old: Value, new: Value) -> bool {
        if old == new {
            return true;
        }
        if old.is_object() && new.is_object() {
            return self.props_equivalent(old, new, true, Value::UNDEFINED);
        }
        !old.is_object() && !new.is_object() && self.same_value(old, new)
    }

    /// Makes the call whose callee is at `start` here, watching what it's worked out from, and keeps what it gives if
    /// nothing made that wrong. What it gives goes to `dst` on the stack.
    fn work_out(&mut self, owner: u32, pc: u32, run: u32, start: usize, argc: usize, dst: usize) -> Run<bool> {
        self.stack[start + 1] = Value::UNDEFINED;
        // What's watched has to be as it was when it was looked through, or it's all looked through again.
        self.still_clean();
        let mut given: Vec<Value> = Vec::with_capacity(argc + 1);
        given.push(self.stack[start]);
        given.extend_from_slice(&self.stack[start + 2..start + 2 + argc]);
        let held = self.keeping.working_on.len();
        self.keeping.working_on.extend_from_slice(&given);
        let looking = Instant::now();
        let watching = self.watch_from(&given);
        let looked = looking.elapsed();
        let changes = self.objects.watch.changes;
        let cells = self.keeping.cells.len();
        self.keeping.working += 1;
        self.keeping.spoiled = false;
        let began = Instant::now();
        let (made, collections) = (self.debt, self.gc_count);
        let result = self.call_at(start, argc, None);
        let took = began.elapsed();
        self.keeping.working -= 1;
        self.keeping.working_on.truncate(held);
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                self.refuse(pc);
                return Err(error);
            }
        };
        self.stack[dst] = value;
        let why = if std::mem::take(&mut self.keeping.spoiled) {
            Some("it called something that isn't pure")
        } else if !watching {
            Some("what it's worked out from can't be watched")
        } else if self.objects.watch.changes != changes {
            Some("it changed something it didn't make")
        } else if !self.keeping.cells[..cells].iter().all(|(cell, value)| self.cells[cell.index()] == *value) {
            Some("it changed a variable it captured")
        } else {
            None
        };
        let name = if logging() { self.name_of_callee(given[0]) } else { String::new() };
        if let Some(why) = why {
            self.refuse(pc);
            self.keeping.counts.not_kept += 1;
            if logging() {
                eprintln!("keep: worked out {name} in {:.1} ms and didn't keep it, as {why}", took.as_secs_f64() * 1e3);
            }
            return Ok(true);
        }
        let made = if self.gc_count == collections { self.debt.saturating_sub(made) } else { usize::MAX };
        let site = self.keeping.sites.entry(pc).or_default();
        site.worth = took >= WORTH_TIME || made >= WORTH_MADE;
        // Looking through what it's given took longer than the call, as when it only reads a little of something big
        // that's new each time: that isn't worth it either.
        if looked > took {
            site.refusals = site.refusals.saturating_add(1);
        } else {
            site.refusals = 0;
        }
        // Too quick this time to be worth what keeping it costs, as a folder that's empty is.
        if !site.worth {
            return Ok(true);
        }
        if !self.watch_from(&[value]) {
            self.refuse(pc);
            return Ok(true);
        }
        self.keeping.kept.retain(|kept| kept.given_at.elapsed() < KEPT_FOR);
        if self.keeping.kept.len() >= MOST_KEPT
            && let Some(oldest) = (0..self.keeping.kept.len()).min_by_key(|&at| self.keeping.kept[at].used)
        {
            self.keeping.kept.swap_remove(oldest);
        }
        self.keeping.clock += 1;
        let used = self.keeping.clock;
        self.keeping.kept.push(Kept {
            owner,
            pc,
            run,
            function: given[0],
            args: given[1..].into(),
            result: value,
            changes: self.objects.watch.changes,
            used,
            given_at: Instant::now(),
        });
        self.keeping.counts.kept += 1;
        if logging() {
            eprintln!(
                "keep: worked out {name} in {:.1} ms, watching {} objects and {} cells",
                took.as_secs_f64() * 1e3,
                self.objects.watch.count,
                self.keeping.cells.len()
            );
        }
        Ok(true)
    }

    fn refuse(&mut self, pc: u32) {
        let site = self.keeping.sites.entry(pc).or_default();
        site.refusals = site.refusals.saturating_add(1);
    }

    /// Watches everything that can be reached from `roots` that isn't watched already, and keeps each cell a function
    /// reached captures with what it holds. Gives false, and watches nothing, if something can't be watched, or there's too
    /// much of it. What's watched already was looked through when it was watched, and hasn't changed since (see
    /// `clean_at`), so what it reaches is watched too.
    fn watch_from(&mut self, roots: &[Value]) -> bool {
        let mut seen = std::mem::take(&mut self.keeping.seen);
        let mut found = std::mem::take(&mut self.keeping.found);
        let mut cells: Vec<(Value, Value)> = Vec::new();
        let mut work: Vec<u32> = roots.iter().filter(|value| value.is_object()).map(|value| value.handle()).collect();
        let length = self.objects.len();
        if seen.len() < length / 64 + 1 {
            seen.resize(length / 64 + 1, 0);
        }
        let global = self.global;
        let mut fine = true;
        while let Some(handle) = work.pop() {
            let at = handle as usize;
            if at >= length || self.objects.watch.is_watched(at) || self.keeping.built_in.contains_key(&handle) {
                continue;
            }
            let (word, bit) = (at / 64, 1u64 << (at % 64));
            if seen[word] & bit != 0 {
                continue;
            }
            seen[word] |= bit;
            found.push(handle);
            if found.len() > MOST_WATCHED {
                fine = false;
                break;
            }
            // The global object is watched, so what's put on it is seen, but not looked into: that's everything.
            if handle == global {
                continue;
            }

            let object = &self.objects[at];
            let mut push = |value: Value| {
                if value.is_object() {
                    work.push(value.handle());
                }
            };
            let proto = self.shapes.items[object.shape as usize].proto;
            if proto != NONE {
                push(Value::object(proto));
            }
            for value in object.slots() {
                push(*value);
            }
            match &object.kind {
                Kind::Plain | Kind::Error | Kind::Date(_) | Kind::RegExp(_) | Kind::BigInt(_) => {}
                Kind::Boxed(value) => push(*value),
                Kind::Array(items) | Kind::Arguments(items) => {
                    for value in items.iter() {
                        push(*value);
                    }
                }
                Kind::Function { home, upvalues, .. } => {
                    if *home != NONE {
                        push(Value::object(*home));
                    }
                    // Captures shared with another function, by handing on, can change through it.
                    if self.reads.shares_captures(upvalues.block()) {
                        fine = false;
                        break;
                    }
                    for value in upvalues.iter() {
                        if value.is_cell() {
                            let inside = self.cells[value.index()];
                            if !self.keeping.cell_numbers.contains_key(&value.handle())
                                && !cells.iter().any(|(cell, _)| cell == value)
                            {
                                cells.push((*value, inside));
                            }
                            push(inside);
                        } else {
                            push(*value);
                        }
                    }
                }
                Kind::Native(native) => push(native.data),
                Kind::Bound(bound) => {
                    push(bound.target);
                    push(bound.this);
                    for value in &bound.args {
                        push(*value);
                    }
                }
                Kind::Accessor { get, set } => {
                    push(*get);
                    push(*set);
                }
                Kind::Map(data) | Kind::Set(data) => {
                    for (key, value) in &data.entries {
                        push(*key);
                        push(*value);
                    }
                }
                _ => {
                    fine = false;
                    break;
                }
            }
        }
        if fine && self.keeping.cells.len() + cells.len() > MOST_CELLS {
            fine = false;
        }
        for &handle in &found {
            seen[handle as usize / 64] = 0;
        }
        if fine {
            for &handle in &found {
                self.objects.watch(handle as usize);
            }
            for &(cell, _) in &cells {
                self.keeping.cell_numbers.insert(cell.handle(), ());
            }
            self.keeping.cells.extend(cells);
        }
        found.clear();
        self.keeping.seen = seen;
        self.keeping.found = found;
        fine
    }

    fn name_of_callee(&self, callee: Value) -> String {
        match callee.is_object().then(|| &self.objects[callee.index()].kind) {
            Some(Kind::Function { proto, .. }) => self.atom_text(self.protos[*proto as usize].name).to_string(),
            _ => "?".to_owned(),
        }
    }
}
