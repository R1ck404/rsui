//! What compiled functions take up, and giving it back once nothing can run them any more.
//!
//! A function's code goes in one list for the whole engine (`Vm::code`), the places it reads and writes properties
//! through in another (`Vm::caches`), and what it is in a third (`Vm::protos`), each of which only ever grew before. That
//! made saving a file while an app runs keep every version of its code, as each one is compiled again. Now a full
//! collection works out which functions can still run, and the rest go back, for what's compiled next to use. Nothing
//! moves, as a function that can still run keeps its places.
//!
//! A function can run if a function object made from it is reached, if a frame or a generator is part way through it,
//! or if one of those can make a function object from it, as the code of a function that can run says which functions
//! inside it it makes. This is only worked out when nothing but JavaScript is running (see [`Vm::nested_runs`]), as a
//! native that has called back into it can be holding a function it compiled and hasn't run yet, which nothing else
//! knows of. Rust only ever compiles something to run it straight away, so that's the only time one's held that way,
//! apart from the CommonJS files an ES module imports, which are compiled with it and kept until their turn comes.
//!
//! The constants code uses (`Vm::consts`) stay, as they're shared between functions and are the same from one version
//! to the next: names, and numbers.
//!
//! A function's number is given to another once it's gone, unless the JIT has written it into machine code (see
//! [`CodeRoom::named`]): a call there checks what it calls by its number and then makes the frame from what it knew of
//! that function, so it would take another for it. That one's places still go back, as nothing can be it.

use crate::object::{Kind, NONE};
use crate::op::{Cache, MEGAMORPHIC, NOT_COMPILED, Op, POLY, Proto};
use crate::source_text::NO_SOURCE;
use crate::vm::Vm;

/// What a function's `start` is once its places have gone back.
pub(crate) const FREED: u32 = u32::MAX - 1;

/// The stretches of a list that are free, as where each starts and how long it is, in order, with none touching another.
#[derive(Default)]
pub(crate) struct Gaps {
    list: Vec<(u32, u32)>,
    total: usize,
}

impl Gaps {
    /// The start of a gap that's at least this long, which no longer counts as free.
    fn take(&mut self, length: u32) -> Option<u32> {
        let at = self.list.iter().position(|&(_, size)| size >= length)?;
        let (start, size) = self.list[at];
        if size == length {
            self.list.remove(at);
        } else {
            self.list[at] = (start + length, size - length);
        }
        self.total -= length as usize;
        Some(start)
    }

    /// Adds these, which mustn't overlap what's free already, putting together any that touch.
    fn give(&mut self, freed: &mut Vec<(u32, u32)>) {
        if freed.is_empty() {
            return;
        }
        self.total += freed.iter().map(|&(_, size)| size as usize).sum::<usize>();
        self.list.append(freed);
        self.list.sort_unstable_by_key(|&(start, _)| start);
        let mut kept: Vec<(u32, u32)> = Vec::with_capacity(self.list.len());
        for &(start, size) in &self.list {
            match kept.last_mut() {
                Some((last, last_size)) if *last + *last_size == start => *last_size += size,
                _ => kept.push((start, size)),
            }
        }
        self.list = kept;
    }

    /// How long a list `length` long can be once a gap reaching its end is taken off, which is taken off.
    fn trim(&mut self, length: usize) -> usize {
        match self.list.last() {
            Some(&(start, size)) if (start + size) as usize == length => {
                self.list.pop();
                self.total -= size as usize;
                start as usize
            }
            _ => length,
        }
    }
}

/// What's known about where compiled functions are, beyond what each says itself. See the top of this file.
#[derive(Default)]
pub(crate) struct CodeRoom {
    /// How many instructions and places in `caches` each function has, by its number, which its own 88 bytes have no
    /// room for. The places that a place in its code took on when it saw more shapes aren't counted (see
    /// `Vm::learn_another`), as that place says where they are.
    pub(crate) spans: Vec<(u32, u32)>,
    /// Where each function's text is, by its number: which source (see `source_text.rs`), and where in it it starts and
    /// ends, for `Function.prototype.toString`.
    texts: Vec<(u32, u32, u32)>,
    code: Gaps,
    caches: Gaps,
    /// The numbers of functions that have gone, to be used again.
    free_protos: Vec<u32>,
    /// The numbers the JIT may have written into machine code, for a function or one it calls, which aren't given to
    /// another function when theirs goes, as shapes' aren't (see `Shapes::pin`).
    named: Vec<u64>,
    /// Marks for which functions can still run, kept between collections so as not to be made again each time.
    pub(crate) marks: Vec<u64>,
}

impl Vm {
    /// Puts a function's instructions where there's room for them, and says where that is.
    pub(crate) fn place_code(&mut self, code: &[Op]) -> u32 {
        if let Some(start) = self.code_room.code.take(code.len() as u32) {
            self.code[start as usize..start as usize + code.len()].copy_from_slice(code);
            return start;
        }
        let start = self.code.len() as u32;
        self.code.extend_from_slice(code);
        start
    }

    /// Puts the places a function's code reads and writes properties through where there's room for them, and says where
    /// the first is.
    pub(crate) fn place_caches(&mut self, caches: &[Cache]) -> u32 {
        if caches.is_empty() {
            return self.caches.len() as u32;
        }
        if let Some(start) = self.code_room.caches.take(caches.len() as u32) {
            self.caches[start as usize..start as usize + caches.len()].copy_from_slice(caches);
            return start;
        }
        let start = self.caches.len() as u32;
        self.caches.extend_from_slice(caches);
        start
    }

    /// Adds a function, which has `code` instructions and `caches` places from where it says they start, and says its
    /// number.
    pub(crate) fn add_proto(&mut self, proto: Proto, code: u32, caches: u32) -> u32 {
        let id = self.add_proto_here(proto, code, caches);
        // What a file compiles to is written down as it's made, when packing (codecache.rs).
        if let Some(cache) = self.code_cache.as_mut().filter(|cache| cache.recording) {
            cache.protos.push(id);
        }
        id
    }

    fn add_proto_here(&mut self, proto: Proto, code: u32, caches: u32) -> u32 {
        if let Some(id) = self.code_room.free_protos.pop() {
            self.replace_proto(id, proto, code, caches);
            self.code_room.texts[id as usize] = (NO_SOURCE, 0, 0);
            return id;
        }
        self.protos.push(proto);
        self.code_room.spans.push((code, caches));
        self.code_room.texts.push((NO_SOURCE, 0, 0));
        #[cfg(feature = "jit")]
        self.jit.add_proto(self.protos.len());
        (self.protos.len() - 1) as u32
    }

    /// Fills in a function that was waiting to be compiled, as [`Vm::add_proto`] would have, or one whose number is
    /// used again. Its count towards being compiled to machine code starts again.
    pub(crate) fn replace_proto(&mut self, id: u32, proto: Proto, code: u32, caches: u32) {
        self.protos[id as usize] = proto;
        self.code_room.spans[id as usize] = (code, caches);
        #[cfg(feature = "jit")]
        self.jit.reset(id);
    }

    /// Says where a function's text is: in `source`, from and to these places in it.
    pub(crate) fn set_text(&mut self, id: u32, source: u32, (start, end): (u32, u32)) {
        self.code_room.texts[id as usize] = (source, start, end);
    }

    /// Where a function's text is, as [`Vm::set_text`] said, if it has any.
    pub(crate) fn text_of_proto(&self, id: u32) -> Option<(u32, u32, u32)> {
        self.code_room
            .texts
            .get(id as usize)
            .copied()
            .filter(|(source, ..)| *source != NO_SOURCE)
    }

    /// Keeps a function's number from being given to another once it's gone, as machine code may name it.
    #[cfg_attr(not(feature = "jit"), allow(dead_code))]
    pub(crate) fn keep_number(&mut self, id: u32) {
        let named = &mut self.code_room.named;
        let word = id as usize / 64;
        if named.len() <= word {
            named.resize(word + 1, 0);
        }
        named[word] |= 1 << (id % 64);
    }

    /// The bytes compiled code takes up, leaving out what's free.
    pub(crate) fn code_bytes(&self) -> usize {
        let room = &self.code_room;
        (self.code.len() - room.code.total) * size_of::<Op>()
            + (self.caches.len() - room.caches.total) * size_of::<Cache>()
            + self.consts.len() * size_of::<crate::value::Value>()
            + (self.protos.len() - room.free_protos.len()) * size_of::<Proto>()
    }

    /// Marks what can still run, given the marks a full collection made of the functions behind the function objects
    /// and generators it reached, then frees what can't. See the top of this file.
    pub(crate) fn free_unused_code(&mut self) {
        let mut marks = std::mem::take(&mut self.code_room.marks);
        let mut work: Vec<u32> = Vec::new();
        let mark = |marks: &mut Vec<u64>, work: &mut Vec<u32>, id: u32| {
            let (word, bit) = (id as usize / 64, 1u64 << (id % 64));
            if marks[word] & bit == 0 {
                marks[word] |= bit;
                work.push(id);
            }
        };
        for frame in self.frames.iter() {
            mark(&mut marks, &mut work, frame.proto);
        }
        // CommonJS files that an ES module imports are compiled along with it, and run when it's their turn, which can be
        // after a collection while what comes before them runs.
        for proto in self.modules.prepared_protos() {
            mark(&mut marks, &mut work, proto);
        }
        for (word, bits) in marks.iter().enumerate() {
            let mut bits = *bits;
            while bits != 0 {
                work.push((word * 64) as u32 + bits.trailing_zeros());
                bits &= bits - 1;
            }
        }
        work.sort_unstable();
        work.dedup();
        // A function that can run can make the ones its code makes closures of.
        while let Some(id) = work.pop() {
            let start = self.protos[id as usize].start;
            if start == NOT_COMPILED || start == FREED {
                continue;
            }
            let length = self.code_room.spans[id as usize].0 as usize;
            for op in &self.code[start as usize..start as usize + length] {
                if let Op::Closure { proto, .. } = *op {
                    mark(&mut marks, &mut work, proto);
                }
            }
        }
        let mut code_freed = Vec::new();
        let mut caches_freed = Vec::new();
        for id in 0..self.protos.len() {
            if marks[id / 64] & (1 << (id % 64)) != 0 || self.protos[id].start == FREED {
                continue;
            }
            let (code, caches) = self.code_room.spans[id];
            let proto = &mut self.protos[id];
            if proto.start != NOT_COMPILED && code != 0 {
                code_freed.push((proto.start, code));
            }
            let base = proto.cache_base as usize;
            for at in base..base + caches as usize {
                let more = self.caches[at].more;
                if more != NONE {
                    caches_freed.push(((at as u32) + (more & !MEGAMORPHIC), POLY as u32));
                }
            }
            if caches != 0 {
                caches_freed.push((proto.cache_base, caches));
            }
            proto.start = FREED;
            proto.cache_base = 0;
            proto.captures = Box::new([]);
            proto.capture_names = Box::new([]);
            proto.handlers = Box::new([]);
            proto.lines = Box::new([]);
            self.code_room.spans[id] = (0, 0);
            self.code_room.texts[id] = (NO_SOURCE, 0, 0);
            if !is_named(&self.code_room.named, id) {
                self.code_room.free_protos.push(id as u32);
            }
            self.uncompiled.remove(&(id as u32));
            #[cfg(feature = "jit")]
            self.jit.forget(id as u32);
        }
        let room = &mut self.code_room;
        room.code.give(&mut code_freed);
        room.caches.give(&mut caches_freed);
        // What's free at the end of each list goes, rather than waiting there to be used again.
        let keep = room.code.trim(self.code.len());
        self.code.truncate(keep);
        self.code.release_unused(0);
        let keep = room.caches.trim(self.caches.len());
        self.caches.truncate(keep);
        self.caches.release_unused(0);
        while self.protos.last().is_some_and(|proto| proto.start == FREED)
            && !is_named(&room.named, self.protos.len() - 1)
        {
            self.protos.pop();
        }
        room.spans.truncate(self.protos.len());
        room.texts.truncate(self.protos.len());
        #[cfg(feature = "jit")]
        self.jit.slots.truncate(self.protos.len());
        let count = self.protos.len() as u32;
        room.free_protos.retain(|&id| id < count);
        // The lowest go first, so that what's at the end is the first to be free again.
        room.free_protos.sort_unstable_by(|a, b| b.cmp(a));
        self.protos.release_unused(0);
        if self.uncompiled.is_empty() && self.uncompiled.capacity() > 64 {
            self.uncompiled.shrink_to(0);
        }
        self.code_room.marks = marks;
        // The sources that what's left was compiled from stay.
        let mut used = vec![false; self.source_count()];
        for (source, ..) in &self.code_room.texts {
            if let Some(used) = used.get_mut(*source as usize) {
                *used = true;
            }
        }
        self.forget_unused_sources(&used);
    }

    /// Checks that nothing that can run is a function that's gone, nor makes one. Panics if not. See `RSUI_GC_VERIFY`.
    #[cold]
    #[inline(never)]
    pub(crate) fn verify_code(&self, when: &str) {
        let gone = |id: u32| {
            self.protos
                .get(id as usize)
                .is_none_or(|proto| proto.start == FREED)
        };
        for index in 0..self.objects.len() {
            if self.free_objects.holes().has(index) {
                continue;
            }
            let object = &self.objects[index];
            let id = match &object.kind {
                Kind::Function { proto, .. } => *proto,
                Kind::Generator(data) if data.proto != NONE => data.proto,
                _ => continue,
            };
            assert!(
                !gone(id),
                "{when}: object {index} runs compiled function {id}, which is gone"
            );
        }
        for frame in self.frames.iter() {
            assert!(
                !gone(frame.proto),
                "{when}: a frame runs compiled function {}, which is gone",
                frame.proto
            );
        }
        for (id, proto) in self.protos.iter().enumerate() {
            if proto.start == NOT_COMPILED || proto.start == FREED {
                continue;
            }
            let length = self.code_room.spans[id].0 as usize;
            for op in &self.code[proto.start as usize..proto.start as usize + length] {
                if let Op::Closure { proto: inner, .. } = *op {
                    assert!(
                        !gone(inner),
                        "{when}: compiled function {id} makes {inner}, which is gone"
                    );
                }
            }
        }
    }
}

/// Whether machine code may name the function of this number (see [`CodeRoom::named`]).
fn is_named(named: &[u64], id: usize) -> bool {
    named.get(id / 64).is_some_and(|word| word >> (id % 64) & 1 != 0)
}

#[cfg(test)]
mod tests {
    use super::Gaps;
    use crate::Engine;

    /// How long the lists of compiled code are, which only what's free at their ends can make shorter.
    fn lengths(engine: &Engine) -> (usize, usize, usize) {
        (
            engine.vm.code.len(),
            engine.vm.caches.len(),
            engine.vm.protos.len(),
        )
    }

    fn text(engine: &mut Engine, source: &str) -> String {
        let value = engine.eval(source, "check.js").unwrap();
        engine.display(value)
    }

    #[test]
    fn a_new_version_of_the_same_code_uses_the_room_the_last_one_left() {
        let mut engine = Engine::new();
        let mut seen = Vec::new();
        for version in 0..40 {
            let source = format!(
                "globalThis.app = {{ total(list) {{ return list.map((x) => x * {version}).reduce((a, b) => a + b, 0) + this.extra.size; }}, \
                 extra: {{ size: {version} }} }}; app.total([1, 2, 3]);"
            );
            assert_eq!(text(&mut engine, &source), (version * 7).to_string());
            engine.vm.collect_all_garbage();
            seen.push(lengths(&engine));
        }
        // The old version is still there while the new one is compiled, so the lists go between having one and two of
        // them, and never more.
        let most = |from: &[(usize, usize, usize)]| from.iter().copied().max().unwrap();
        assert_eq!(most(&seen[30..]), most(&seen[..10]), "{seen:?}");
    }

    #[test]
    fn what_can_still_run_stays_while_the_rest_goes() {
        let mut engine = Engine::new();
        // A closure of an old version, a generator part way through, an async function waiting, and a function inside
        // one that can still run, which hasn't been made yet.
        text(
            &mut engine,
            "globalThis.kept = (() => { const n = 41; return () => n + 1; })();
             globalThis.counting = (function* () { let i = 0; while (true) yield i++; })();
             counting.next();
             globalThis.done = 'no';
             let finish;
             (async () => { await new Promise((resolve) => { finish = resolve; }); done = 'yes'; })();
             globalThis.finish = finish;
             globalThis.maker = (k) => () => k * 2;
             'ok'",
        );
        for version in 0..20 {
            text(
                &mut engine,
                &format!("globalThis.junk = function () {{ return () => {version}; }}; junk()()"),
            );
            engine.vm.collect_all_garbage();
        }
        assert_eq!(text(&mut engine, "kept()"), "42");
        assert_eq!(text(&mut engine, "counting.next().value"), "1");
        assert_eq!(text(&mut engine, "maker(21)()"), "42");
        text(&mut engine, "finish(); 'ok'");
        engine.run_jobs().unwrap();
        engine.vm.collect_all_garbage();
        assert_eq!(text(&mut engine, "done"), "yes");
        engine.vm.verify_code("at the end");
    }

    #[test]
    fn a_function_that_waits_to_be_compiled_can_be_compiled_after_others_have_gone() {
        let mut engine = Engine::new();
        engine.set_lazy_compile(crate::LazyCompile::Always);
        text(
            &mut engine,
            "globalThis.later = function () { return [1, 2].map((x) => x + 1).join(); }; 'ok'",
        );
        for version in 0..10 {
            text(
                &mut engine,
                &format!("(function () {{ return {version}; }})()"),
            );
            engine.vm.collect_all_garbage();
        }
        assert_eq!(text(&mut engine, "later()"), "2,3");
        engine.vm.collect_all_garbage();
        assert_eq!(text(&mut engine, "later()"), "2,3");
        engine.vm.verify_code("at the end");
    }

    /// Machine code checks what a call calls by its number, then makes the frame from what it knew of that function. So
    /// once a function it guessed at has gone, its number mustn't be another's, which the check would take for it.
    #[cfg(feature = "jit")]
    #[test]
    fn a_number_machine_code_knows_is_not_given_to_another_function() {
        use crate::object::Kind;
        let mut engine = Engine::new();
        engine.vm.jit.mode = crate::jit::Mode::Eager;
        engine.vm.jit.background = false;
        let proto_of = |engine: &mut Engine, name: &str| {
            let value = engine.eval(name, "check.js").unwrap();
            match engine.vm.objects[value.index()].kind {
                Kind::Function { proto, .. } => proto,
                _ => panic!("{name} is a function"),
            }
        };
        text(
            &mut engine,
            "globalThis.helper = (x) => x + 0;
             globalThis.caller = () => { let total = 0; for (let i = 0; i < 4; i++) total += helper(i); return total; };
             caller()",
        );
        let first = proto_of(&mut engine, "helper");
        for version in 1..30 {
            // Functions nothing compiles, whose numbers are given out again, and the next version of the one the
            // caller's code guessed at.
            let source = format!(
                "(function () {{ return [1, 2].map((x) => x * {version}).join(); }})();
                 globalThis.helper = (x) => x + {version}; caller()"
            );
            assert_eq!(text(&mut engine, &source), (6 + 4 * version).to_string());
            engine.vm.collect_all_garbage();
            assert_eq!(engine.vm.protos[first as usize].start, super::FREED);
        }
        assert!(engine.vm.jit.stats().compiled > 0);
        engine.vm.verify_code("at the end");
    }

    #[test]
    fn gaps_put_together_what_touches_and_hand_out_the_first_that_fits() {
        let mut gaps = Gaps::default();
        gaps.give(&mut vec![(10, 5), (0, 4), (4, 2)]);
        assert_eq!(gaps.list, [(0, 6), (10, 5)]);
        assert_eq!(gaps.total, 11);
        assert_eq!(gaps.take(5), Some(0));
        assert_eq!(gaps.list, [(5, 1), (10, 5)]);
        assert_eq!(gaps.take(5), Some(10));
        assert_eq!(gaps.take(2), None);
        assert_eq!(gaps.total, 1);
        gaps.give(&mut vec![(6, 4)]);
        assert_eq!(gaps.list, [(5, 5)]);
        assert_eq!(gaps.trim(10), 5);
        assert_eq!(gaps.trim(5), 5);
        assert_eq!(gaps.total, 0);
    }
}
