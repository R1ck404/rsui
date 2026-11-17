//! Tests of the JIT: that compiled code does what the interpreter does, however it's entered and left. Each program
//! runs four times, interpreted, with every function compiled the first time it's called, and with functions
//! compiled once they're hot (so loops are entered partway), there and then and on the thread that compiles, and all
//! four have to print the same.
//!
//! With `RSUI_GC_STRESS=1` and `RSUI_GC_VERIFY=1` the collector runs at every safe point compiled code reaches too.

use std::cell::RefCell;
use std::rc::Rc;

use super::Mode;
use crate::Engine;
use crate::value::Value;

/// Runs a program with the JIT in `mode`, with a `gc()` that collects everything, and gives what it printed (and
/// what it threw, if it did) and how many functions were compiled. Compiled code must never have met an instruction
/// the generic helper doesn't do.
fn run_in(mode: Mode, source: &str) -> (String, u32) {
    run_with(mode, true, source)
}

/// `run_in`, with functions compiled on a thread of their own (`background`) or there and then.
fn run_with(mode: Mode, background: bool, source: &str) -> (String, u32) {
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    engine.vm.jit.mode = mode;
    engine.vm.jit.background = background;
    // The programs here are sized for a function to be compiled after a thousand runs.
    engine.vm.jit.threshold = 1000;
    let gc = engine.vm.host_function("gc", 0, |vm, _| {
        vm.collect_garbage_now();
        Ok(Value::UNDEFINED)
    });
    engine.vm.set_global("gc", gc);
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let result = engine
        .eval(source, "jit.js")
        .and_then(|_| engine.run_until_idle());
    let mut text = output.borrow().trim_end().to_owned();
    if let Err(error) = result {
        text.push_str(&format!("\nthrew: {error}"));
    }
    let stats = engine.vm.jit.stats();
    assert_eq!(stats.unknown, 0, "an instruction went to bail_at ({mode:?}):\n{source}");
    LAST.with(|last| last.set(stats));
    (text, stats.compiled)
}

thread_local! {
    /// What the last run's JIT counted, for a test that checks what compiled code did.
    static LAST: std::cell::Cell<super::Stats> = std::cell::Cell::new(super::Stats::default());
}

/// Checks the three ways of running a program print the same, and that something was compiled. Gives what it printed.
fn same(source: &str) -> String {
    let (off, _) = run_in(Mode::Off, source);
    let (eager, compiled) = run_in(Mode::Eager, source);
    let (on, _) = run_in(Mode::On, source);
    let (there, _) = run_with(Mode::On, false, source);
    assert!(compiled > 0, "nothing was compiled:\n{source}");
    assert_eq!(off, eager, "compiled at once:\n{source}");
    assert_eq!(off, on, "compiled once hot:\n{source}");
    assert_eq!(off, there, "compiled once hot, there and then:\n{source}");
    assert!(!off.contains("threw:"), "{off}");
    println!("{off}");
    off
}

#[test]
fn numbers_overflow_and_turn_into_doubles_as_in_the_interpreter() {
    let out = same(
        r#"
        function f(a, b) {
            return [a + b, a - b, a * b, a / b, a % b, a & b, a | b, a ^ b, a << b, a >> b, a >>> b, -a, a++, --b,
                a < b, a <= b, a > b, a >= b, a === b, a == b, a !== b];
        }
        const pairs = [[2147483647, 1], [-2147483648, 1], [-2147483648, -1], [65536, 65536], [0, -5], [-0, 5],
            [7, 0], [-7, 3], [7, -3], [1.5, 2.5], [0.1, 0.2], [NaN, 1], [Infinity, -Infinity], [2 ** 53, 3],
            [-1, 31], [1, 32], [3e10, 2], ['3', 4], [4, '3'], [true, null], [undefined, 1], [10n, 3n]];
        const out = [];
        for (let r = 0; r < 3; r++) for (const [a, b] of pairs) {
            try { out.push(f(a, b).map(String).join(',')); } catch (e) { out.push(e.constructor.name); }
        }
        let x = 2147483640, steps = 0;
        for (let i = 0; i < 20; i++) { x++; steps += x > 2147483647 ? 1 : 0; }
        let m = 1;
        for (let i = 0; i < 40; i++) m = m * 3 % 1000003;
        let z = 0;
        for (let i = 0; i < 2000; i++) z = (z * 1664525 + 1013904223) >>> 0;
        // A NaN an operation makes is stored as the interpreter would store it.
        const bits = new Float64Array(4);
        bits[0] = 0 / 0; bits[1] = Infinity - Infinity; bits[2] = Math.sqrt(-1); bits[3] = -0 * 1;
        console.log(out.length, x, steps, m, z, Array.from(new Uint8Array(bits.buffer)).join(''));
        console.log(out.slice(0, 22).join(' | '));
        "#,
    );
    assert!(out.starts_with("66 2147483660 13 "), "{out}");
}

#[test]
fn a_guard_that_fails_partway_through_a_loop_carries_on_the_long_way() {
    same(
        r#"
        const values = [];
        for (let i = 0; i < 3000; i++) values.push(i < 1000 ? i : i < 2000 ? i + 0.5 : i < 2500 ? String(i) : { valueOf() { return i; } });
        function total(list) {
            let s = 0, t = 0, n = 0;
            for (let i = 0; i < list.length; i++) { s += list[i]; t = t + (list[i] * 2) | 0; n += list[i] < 1500 ? 1 : 0; }
            return [typeof s, String(s).length, t, n];
        }
        console.log(total(values).join(' '));
        // An int counter that ends up a double, and a limit that's a string.
        let k = 0;
        for (let i = 2147483600; i < 2147483700; i++) k++;
        for (let i = 0; i < '50'; i++) k++;
        console.log(k);
        "#,
    );
}

#[test]
fn shapes_and_prototypes_changing_under_compiled_code() {
    same(
        r#"
        class P { get v() { return 1; } m() { return 'a'; } }
        const objects = [];
        for (let i = 0; i < 400; i++) objects.push(new P());
        let s = '';
        function read(o) { return o.m() + (o.x === undefined ? '-' : o.x) + o.v; }
        for (let i = 0; i < objects.length; i++) {
            const o = objects[i];
            if (i === 100) P.prototype.m = function () { return 'b'; };
            if (i === 200) Object.defineProperty(P.prototype, 'v', { get() { return 2; } });
            if (i === 250) o.x = 5;
            if (i === 300) { o.m = () => 'own'; delete o.x; }
            if (i === 350) Object.setPrototypeOf(o, { m() { return 'other'; }, v: 3 });
            if (i % 50 === 0 || i > 345 && i < 352) s += read(o) + ' ';
            else read(o);
        }
        console.log(s);
        // A place that sees many shapes, then one again.
        const many = [];
        for (let i = 0; i < 2000; i++) { const o = {}; o['k' + (i % 13)] = i; o.x = i; many.push(o); }
        let sum = 0;
        for (let r = 0; r < 3; r++) for (const o of many) { sum += o.x; o.x = o.x + 1; }
        console.log(sum, many[5].x, Object.keys(many[7]).join());
        "#,
    );
}

#[test]
fn errors_from_helpers_and_callees_reach_the_right_catch() {
    same(
        r#"
        const log = [];
        function bad(i) {
            if (i % 5 === 1) return { valueOf() { throw new RangeError('valueOf ' + i); } } + 1;
            if (i % 5 === 2) return { get p() { throw new TypeError('getter ' + i); } }.p;
            if (i % 5 === 3) return JSON.parse('{' + i);
            if (i % 5 === 4) return null.x;
            return i;
        }
        function middle(i) { try { return bad(i); } finally { log.push('f' + i); } }
        for (let i = 0; i < 1200; i++) {
            try { middle(i); } catch (e) { if (i < 10 || i > 1195) log.push(e.constructor.name + ':' + e.message.slice(0, 12)); }
        }
        console.log(log.filter((x) => x[0] !== 'f').join(' '), log.length);
        function tdz() { try { use(); let late = 1; function use() { return late; } } catch (e) { return e.constructor.name; } }
        function notCallable() { try { const o = {}; o.nope(); } catch (e) { return e.message; } }
        function rethrow() { try { try { throw 1; } catch (e) { throw e + 1; } } catch (e) { return e; } }
        let r = [];
        for (let i = 0; i < 1100; i++) r = [tdz(), notCallable(), rethrow()];
        console.log(r.join(' '));
        // Where an error was made is where its stack says.
        function make() { return new Error('here'); }
        let stack;
        for (let i = 0; i < 1100; i++) stack = make().stack;
        console.log(stack.split('\n').slice(0, 2).join(' / '));
        "#,
    );
}

#[test]
fn collections_while_compiled_code_runs_keep_what_it_uses() {
    same(
        r#"
        function chain(n) {
            let head = null;
            for (let i = 0; i < n; i++) {
                head = { i, next: head, s: 'n' + i, f: () => i };
                if (i % 1000 === 0) gc();
            }
            return head;
        }
        function sum(head) { let s = 0; while (head !== null) { s += head.i + head.s.length + head.f(); head = head.next; } return s; }
        let total = 0;
        for (let r = 0; r < 4; r++) { const h = chain(5000); gc(); total += sum(h); }
        const keep = [];
        for (let i = 0; i < 20000; i++) { const a = [i, { v: i }, 'x' + i]; if (i % 7 === 0) keep.push(a); }
        gc();
        let check = 0;
        for (const a of keep) check += a[0] + a[1].v + a[2].length;
        console.log(total, keep.length, check);
        "#,
    );
}

#[test]
fn deep_recursion_and_the_stack_limit() {
    same(
        r#"
        function depth(n) { return n === 0 ? 0 : 1 + depth(n - 1); }
        function tooDeep() { try { return depth(1e7); } catch (e) { return e instanceof RangeError; } }
        function* gen(n) { yield n === 0 ? 0 : 1 + through(n - 1); }
        function through(n) { return gen(n).next().value; }
        const out = [];
        for (let i = 0; i < 3; i++) out.push(depth(9000), tooDeep(), through(4));
        // (Generators nest on Rust's stack, which a debug build's frames use a lot of, so only a few here.)
        // Recursion that goes on after a RangeError, and then deep again.
        let ok = 0;
        for (let i = 0; i < 3; i++) { try { depth(1e7); } catch (e) { ok += depth(5000); } }
        console.log(out.join(' '), ok);
        "#,
    );
}

#[test]
fn constructors_super_and_new_target() {
    same(
        r#"
        function Plain(a) { this.a = a; }
        function Gives(a) { this.a = a; return { b: a }; }
        function Primitive(a) { this.a = a; return 5; }
        function Who() { return new.target === Who; }
        class Base { constructor(x) { this.x = x; } get twice() { return this.x * 2; } }
        class Derived extends Base { constructor(x) { super(x + 1); this.y = new.target.name; } }
        class Empty extends Base {}
        class Skips extends Base { constructor(go) { if (go) super(1); } }
        let s = 0, names = '';
        for (let i = 0; i < 2000; i++) {
            s += new Plain(i).a + new Gives(i).b + new Primitive(i).a + (new Who() instanceof Who ? 1 : 0);
            const d = new Derived(i);
            s += d.x + d.twice + new Empty(i).x;
            if (i === 1999) names = d.y + ' ' + Object.getPrototypeOf(d).constructor.name;
            try { new Skips(i % 2 === 0); } catch (e) { s += 1000; }
        }
        console.log(s, names, Who());
        "#,
    );
}

#[test]
fn methods_of_object_literals_find_super_through_their_home() {
    // Each named method of a literal gets the literal as its home (SetHome), which is where `super` looks from.
    let out = same(
        r#"
        const base = { greet() { return 'base'; }, get size() { return 1; } };
        const other = { greet() { return 'other'; }, get size() { return 2; } };
        function make(i) {
            const o = {
                __proto__: i % 3 === 0 ? other : base,
                greet() { return super.greet() + i; },
                'quoted name'() { return super.size; },
                twice() { return this.greet() + this['quoted name'](); },
            };
            return o.twice();
        }
        let s = '';
        for (let i = 0; i < 3000; i++) s = make(i) + ' ' + s.slice(0, 40);
        console.log(s);
        "#,
    );
    assert!(out.starts_with("base29991 base29981 other29972"), "{out}");
}

#[test]
fn calls_of_every_kind() {
    same(
        r#"
        function add3(a, b, c) { return a + b + c; }
        function count() { let s = 0; for (let i = 0; i < arguments.length; i++) s += arguments[i]; return s; }
        function rest(first, ...more) { return first + more.length; }
        function defaults(a, b = 2, c = a + b) { return a + b + c; }
        const o = { base: 10, get g() { return this.base; }, set g(v) { this.base = v; }, m(k) { return this.base + k; } };
        const bound = o.m.bind({ base: 100 });
        let t = 0;
        for (let i = 0; i < 3000; i++) {
            t += add3(i) !== add3(i) ? 1 : 0;
            t += add3(1, 2, 3, 4) + add3.call(null, i, 1, 2) + add3.apply(null, [i, 1, 2]);
            t += count(1, 2, i) + rest(i, 1, 2) + defaults(i) + defaults(1, 2, 3);
            t += o.g + o.m(i) + bound(i) + Math.max(i, 5) + [i, 1].length;
            o.g = i % 10;
            t += add3(...[1, 2, i]);
            t = t % 1000003;
        }
        const counter = (() => { let c = 0; return { inc() { return ++c; } }; })();
        for (let i = 0; i < 5000; i++) counter.inc();
        // Functions that keep their arguments, called directly from compiled code, with too few and too many.
        function Kept() { this.n = arguments.length; this.last = arguments[arguments.length - 1]; }
        function sloppy(a) { arguments[0] = 'changed'; return [a, arguments[0], arguments.length].join(); }
        let k = 0, last = '';
        for (let i = 0; i < 3000; i++) { const o = new Kept(i, 2, 3); k += o.n + o.last + count() + count(i); last = sloppy(i, 5); }
        console.log(t, counter.inc(), [3, 1, 2].sort((a, b) => a - b).join(), [1, 2, 3].map((x) => x * 2).join(), k, last);
        "#,
    );
}

#[test]
fn for_of_over_arrays_maps_generators_and_closing_early() {
    same(
        r#"
        const closed = [];
        function iterable(n) {
            return { [Symbol.iterator]() { let i = 0; return { next: () => ({ value: i, done: i++ >= n }), return() { closed.push(n); return {}; } }; } };
        }
        function* gen(n) { for (let i = 0; i < n; i++) yield i * 2; }
        function walk(n) {
            let s = 0;
            for (const v of [1, , 3]) s += v === undefined ? 100 : v;
            for (const [k, v] of new Map([[1, 2], [3, 4]])) s += k * v;
            for (const v of new Set([5, 6])) s += v;
            for (const v of gen(n)) s += v;
            for (const v of iterable(n)) { if (v === 3) break; s += v; }
            for (const c of 'abc') s += c.charCodeAt(0);
            return s;
        }
        let t = 0;
        for (let i = 0; i < 1500; i++) t += walk(i % 8);
        console.log(t, closed.length, closed.slice(-3).join());
        "#,
    );
}

#[test]
fn generators_under_for_of_called_from_compiled_code() {
    // The interpreter carries a generator on in place for `for of` (`step_in_place`, `RESUMED_HERE`); compiled code
    // gets the same values, ends, errors and early exits, with generators that call compiled code themselves.
    same(
        r#"
        function twice(x) { return x * 2; }
        function* count(n) { for (let i = 0; i < n; i++) yield twice(i); return 'done'; }
        function* inner() { yield 1; yield 2; }
        function* outer() { yield* inner(); yield* [3, 4]; yield 5; }
        function* throws(n) { for (let i = 0; i < n; i++) { if (i === 2) throw new Error('at ' + i); yield i; } }
        let finallies = 0;
        function* guarded() { try { yield 1; yield 2; yield 3; } finally { finallies++; } }
        function* empty() {}
        function walk(n) {
            let s = 0;
            for (const v of count(n)) s += v;
            for (const v of outer()) s += v * 10;
            try { for (const v of throws(5)) s += v; } catch (e) { s += e.message.length; }
            for (const v of guarded()) { if (v === 2) break; s += v; }
            for (const v of empty()) s += 1000;
            const g = count(3);
            g.next();
            for (const v of g) s += v * 100;
            for (const a of count(2)) for (const b of count(2)) s += a + b;
            return s;
        }
        let t = 0;
        for (let i = 0; i < 1500; i++) t += walk(i % 6);
        // Frames that go on in the interpreter from a `for of` over a generator: a constructor's, one that keeps its
        // arguments, one whose error a caller catches, one with a `finally`, and loops after it in the same frame.
        function Summed(n) { this.total = 0; for (const v of count(n)) this.total += v; }
        function kept() { let s = arguments.length; for (const v of count(arguments[0])) s += v + arguments[1]; return s; }
        function fails(n) { for (const v of count(n)) if (v > 4) throw new Error('big ' + v); return -1; }
        function after(n) { let s = 0; try { for (const v of count(n)) s += v; } finally { s += 1000; } for (let i = 0; i < n; i++) s += i; return s; }
        let u = 0;
        for (let i = 0; i < 1500; i++) {
            u += new Summed(i % 5).total + kept(i % 4, 3) + after(i % 7);
            try { u += fails(i % 6); } catch (e) { u += e.message.length; }
        }
        console.log(t, finallies, u);
        "#,
    );
}

#[test]
fn generators_compiled_and_carried_on_in_their_code() {
    // A generator's own code is compiled, and carried on where it stopped from Rust: by `next` with a value sent in,
    // `return` through a `finally`, `throw` into a `catch`, spreading, destructuring, `Array.from`, and from a `for of`.
    // One whose frame the interpreter is carrying on in place gets compiled partway through its loop.
    same(
        r#"
        function* echo() { let got = 0, sum = 0; while (true) { got = yield got * 2; if (got === undefined) return sum; sum += got; } }
        function* guarded(log) { try { yield 1; yield 2; } finally { log.push('finally'); } }
        function* caught() { let n = 0; while (true) { try { yield n++; } catch (e) { n += e; } } }
        function* range(n) { for (let i = 0; i < n; i++) yield i; }
        function* args() { for (let i = 0; i < arguments.length; i++) yield arguments[i]; }
        function* nested(n) { yield* range(n); yield* [n, n]; return yield 'last'; }
        class Bag { constructor(n) { this.n = n; } *[Symbol.iterator]() { for (let i = 0; i < this.n; i++) yield this.n - i; } }
        const object = { *pairs() { yield [1, 2]; yield [3, 4]; } };
        let total = 0;
        const log = [];
        for (let r = 0; r < 1200; r++) {
            const e = echo();
            e.next();
            total += e.next(3).value + e.next(r % 5).value;
            total += e.next().value;
            const g = guarded(log);
            total += g.next().value;
            total += g.return(7).value + (g.next().done ? 1 : 0);
            const c = caught();
            c.next(); c.next();
            total += c.throw(10).value;
            total += [...range(r % 9)].length + Array.from(range(4)).reduce((a, b) => a + b, 0);
            const [a, b, ...rest] = range(6);
            total += a + b + rest.length;
            for (const v of args(r, 1, 2)) total += v;
            const n = nested(3);
            let s = 0;
            for (let x = n.next(); !x.done; x = n.next(x.value)) s += typeof x.value === 'number' ? x.value : 1;
            total += s + [...new Bag(r % 4)].length;
            for (const [p, q] of object.pairs()) total += p * q;
        }
        // One generator, carried on in place from an interpreted loop, long enough to be compiled meanwhile; and a
        // chain of them, each carried on in place by the one after until they're compiled, one by one.
        let long = 0;
        for (const v of range(5000)) long += v;
        function* map(it, f) { for (const v of it) yield f(v); }
        function* filter(it, f) { for (const v of it) if (f(v)) yield v; }
        function* take(it, n) { if (n <= 0) return; for (const v of it) { yield v; if (--n <= 0) return; } }
        let chained = 0, wrong = 0, at = 0;
        for (const v of take(filter(map(range(30000), (x) => x * 3), (x) => x % 2 === 0), 9000)) {
            if (v !== at * 6) wrong++;
            at++;
            chained += v & 0xff;
        }
        long += chained * 1000 + wrong;
        // A generator can't be carried on while it's running.
        let running;
        function* selfish() { try { running.next(); } catch (err) { yield err.constructor.name; } }
        running = selfish();
        console.log(total, log.length, long, running.next().value);
        "#,
    );
}

#[test]
fn call_apply_and_bound_functions_from_compiled_code() {
    // `f.call`, `f.apply` and bound functions are made the call they stand for, in place (`forward_in_place`);
    // through each other, with `arguments`, holes, natives, classes and too few or many arguments.
    same(
        r#"
        function add3(a, b, c) { return (this && this.base || 0) + a + b + c; }
        function args() { return arguments.length + ':' + Array.prototype.join.call(arguments, '-'); }
        class K { constructor() { this.k = 1; } }
        const o = { base: 1000 };
        const bound = add3.bind(o, 1);
        const bound2 = bound.bind(null, 2);
        const callOfCall = Function.prototype.call.bind(add3);
        function viaArguments() { return add3.apply(o, arguments); }
        let t = 0, last = '';
        for (let i = 0; i < 3000; i++) {
            t += add3.call(o, i, 1, 2) + add3.call(null, i) + add3.call() + add3.apply(o, [i, 2, 3]) + add3.apply(null);
            t += bound(i, 3) + bound2(i) + bound2() + callOfCall(o, i, 1, 1) + viaArguments(i, 1, 1);
            t += add3.call.call(add3, o, 1, 2, 3) + add3.apply.call(add3, o, [1, 2, i]);
            t += Math.max.apply(null, [i, 5, 7]) + Math.max.call(null, 1, i) + String.fromCharCode.apply(null, [65 + i % 26]).length;
            last = args.apply(null, [1, , 3]) + ' ' + args.call(null, i) + ' ' + args.apply(null, { length: 2, 0: 'a', 1: 'b' });
            try { K.call({}); } catch (e) { t += e instanceof TypeError ? 1 : 0; }
            try { add3.apply(null, 5); } catch (e) { t += e instanceof TypeError ? 1 : 0; }
            t = t % 1000003;
        }
        console.log(t, last);
        "#,
    );
}

#[test]
fn arguments_read_as_lists_from_compiled_code() {
    same(
        r#"
        function sum() { let s = 0; for (let i = 0; i < arguments.length; i++) s += arguments[i]; return s; }
        function sloppy(a, b) { arguments[0] = 10; return a + b + arguments.length + arguments[5]; }
        function strict(a) { 'use strict'; arguments[0] = 10; return a + arguments[0] + arguments.length; }
        function grown() { arguments.length = 1; return arguments.length + ',' + arguments[1]; }
        function callee() { return typeof arguments.callee; }
        function spread() { return [...arguments].join('') + Array.from(arguments).length; }
        let t = 0, text = '';
        for (let i = 0; i < 3000; i++) {
            t += sum(1, 2, i) + sum() + sum(i) + (sloppy(i, 2) || 0) + strict(i);
            text = grown(1, 2, 3) + ' ' + callee() + ' ' + spread(1, 2, 3);
        }
        console.log(t, text);
        "#,
    );
}

#[test]
fn typed_arrays_and_their_buffers_moving_under_compiled_code() {
    // Compiled code reads and writes typed arrays' elements where the interpreter keeps them (`TypedData::quick`),
    // which a resize or a transfer of their buffer moves (`buffer_epoch`).
    same(
        r#"
        const kinds = [Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array, Uint32Array,
            Float32Array, Float64Array];
        function fill(a, n) { for (let i = 0; i < n; i++) a[i] = i * 37 - 300.75; }
        // Ints of every size, which each kind wraps or clamps, and doubles that are whole.
        function fillInts(a, n) { for (let i = 0; i < n; i++) a[i] = i % 3 === 0 ? (i * 2654435761) | 0 : i % 3 === 1 ? i * 1000 - 20000 : i / 2; }
        function check(a) { let s = 0; for (let i = 0; i < a.length; i++) s = (s * 31 + a[i]) % 1000003; return s; }
        function total(a) { let s = 0; for (let i = 0; i < a.length; i++) s += a[i]; return s; }
        function reads(a, n) { let s = 0; for (let i = -2; i < n + 2; i++) { const v = a[i]; s += v === undefined ? 0.5 : v; } return s; }
        const out = [];
        for (let r = 0; r < 4; r++) {
            for (const Kind of kinds) {
                const a = new Kind(40);
                fill(a, 40);
                a[3] = 'x'; a[4] = NaN; a[5] = -0; a[6] = 1e10; a[7] = true;
                out.push(total(a), reads(a, 40));
                fillInts(a, 40);
                a[1] = 4294967295; a[2] = -1; a[8] = 255.5; a[9] = 254.5; a[10] = -0.5;
                out.push(check(a), Object.is(a[5], -0));
            }
            const buffer = new ArrayBuffer(16, { maxByteLength: 64 });
            const view = new Int32Array(buffer);
            const fixed = new Int32Array(buffer, 0, 4);
            fill(view, 4);
            out.push(total(view), reads(fixed, 4));
            buffer.resize(64);
            fill(view, 16);
            out.push(total(view), total(fixed), reads(view, 16));
            buffer.resize(8);
            out.push(total(view), reads(fixed, 4));
            const moved = buffer.transfer(32);
            out.push(view.length, reads(view, 2), total(new Int32Array(moved)));
            const plain = [1, 2, 3];
            // The same place writes to an array and a typed array, in turn.
            function put(x, i, v) { x[i] = v; }
            for (let i = 0; i < 10; i++) { put(i % 2 ? plain : view, 0, i); }
            out.push(plain[0], view[0]);
        }
        console.log(out.join(' '));
        "#,
    );
}

#[test]
fn math_calls_and_math_replaced_after_compiling() {
    // CallMath works the usual ones out itself while they're the built-in functions and given numbers.
    same(
        r#"
        function f(x, y) {
            return Math.sqrt(x) + Math.abs(y) + Math.floor(x / 3) + Math.ceil(y / 7) + Math.round(x / 9) +
                Math.trunc(-x / 5) + Math.min(x, y) + Math.max(x, y) + Math.sin(x) + Math.pow(x, 0.5) +
                Math.atan2(y, x) + Math.imul(x, y) + Math.sign(-x) + Math.hypot(x, y) + Math.fround(x / 3) +
                Math.log(x + 1) + Math.exp(-x / 100) + Math.cbrt(y);
        }
        const inputs = [0, -0, 1, -1, 2.5, -2.5, 1e300, -1e300, NaN, Infinity, -Infinity, 0.49999999999999994, 2 ** 31, '7', null];
        const out = [];
        for (let r = 0; r < 200; r++) for (const x of inputs) out.push(f(x, -x - 1));
        const before = out.slice(0, inputs.length).join();
        Math.floor = (x) => 42;
        const realMax = Math.max;
        Math.max = function () { return 'max' + arguments.length; };
        const after = inputs.map((x) => f(x, 1)).join();
        Math.max = realMax;
        const saved = Math;
        globalThis.Math = { sqrt: () => -1, abs: Math.abs, floor: Math.floor, ceil: Math.ceil, round: Math.round,
            trunc: Math.trunc, min: Math.min, max: Math.max, sin: Math.sin, pow: Math.pow, atan2: Math.atan2,
            imul: Math.imul, sign: Math.sign, hypot: Math.hypot, fround: Math.fround, log: Math.log, exp: Math.exp,
            cbrt: Math.cbrt };
        const replaced = inputs.map((x) => f(x, 2)).join();
        globalThis.Math = saved;
        console.log(before, '|', after, '|', replaced, out.length);
        "#,
    );
}

#[test]
fn absent_properties_that_appear_later() {
    // A read of what isn't there is remembered as being on `Vm::absent`, guarded by the epoch: it has to see the
    // property once it's added to the object, its prototype or `Object.prototype`, or once a getter is.
    same(
        r#"
        function C() { this.a = 1; }
        function read(o) { return o.missing; }
        function method(o) { return typeof o.later === 'function' ? o.later() : -1; }
        let t = 0;
        const objects = [];
        for (let i = 0; i < 3000; i++) {
            const o = new C();
            objects.push(o);
            t += read(o) === undefined ? 1 : 0;
            t += method(o);
            if (i === 1000) C.prototype.missing = 5;
            if (i === 1500) o.missing = 7;
            if (i === 2000) Object.prototype.later = function () { return 3; };
            if (i === 2500) Object.defineProperty(C.prototype, 'missing', { get() { return 11; }, configurable: true });
            const v = read(o);
            t += v === undefined ? 0 : v;
        }
        delete Object.prototype.later;
        // The same for strings and numbers, whose places remember a property on their prototype, further up, or
        // nowhere.
        function key(v) { return v.key; }
        function own(v) { return typeof v.hasOwnProperty + typeof v.toFixed + typeof v.at; }
        const prims = ['a', 'bc', 5, 2.5, -0];
        let p = '';
        for (let i = 0; i < 3000; i++) {
            const v = prims[i % prims.length];
            p += String(key(v)) + own(v);
            if (i === 1000) Object.prototype.key = 'O';
            if (i === 1500) String.prototype.key = 'S';
            if (i === 2000) Number.prototype.key = 'N';
            if (i === 2500) { delete String.prototype.key; delete Object.prototype.key; Number.prototype.toFixed = undefined; }
            if (p.length > 2000) p = p.slice(-200);
        }
        // One place that sees objects of several shapes, strings and numbers (`child.key` over a list of children).
        function keyOf(c, i) { return c && c.key != null ? c.key : 'i' + i; }
        const children = [{ key: 'a' }, 'text', { key: 'b', x: 1 }, 7, { y: 2 }, 'more', { key: null }, 0.5];
        let k = '';
        for (let r = 0; r < 600; r++) {
            for (let i = 0; i < children.length; i++) k += keyOf(children[i], i);
            if (r === 300) String.prototype.key = 'sk';
            if (k.length > 500) k = k.slice(-100);
        }
        console.log(t, read(objects[0]), method(objects[0]), p.slice(-120), k.slice(-60));
        "#,
    );
}

#[test]
fn the_new_interpreter_instructions_in_compiled_code() {
    // AddInt32, DivInt, LoopStrictNe and LoopStrictNeInt on ints, doubles and what isn't a number.
    same(
        r#"
        function hash(s) { let h = 0; for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0; return h; }
        function mix(a, b) { return (a + b) | 0; }
        function halves(x) { return [x / 2, x / 4, x / 1024, x / 1]; }
        function untilEqual(a, b) { let n = 0; while (a !== b) { a++; n++; } return n; }
        function untilFive(x) { let n = 0; while (x !== 5) { x = x + 1; n++; if (n > 100) break; } return n; }
        const values = [0, -0, 1, -1, 7, -7, 8, -8, 2147483647, -2147483648, 1.5, -2.5, NaN, Infinity, '12', null, undefined, 3n, true];
        const out = [];
        for (let r = 0; r < 300; r++) {
            out.push(hash('abcdefghij' + r));
            for (const a of values) {
                for (const b of values.slice(0, 12)) { try { out.push(mix(a, b)); } catch (e) { out.push(e.name); } }
                try { out.push(halves(a).join()); } catch (e) { out.push(e.name); }
            }
            out.push(untilEqual(0, r), untilEqual(0.5, r + 0.5), untilFive(0), untilFive(4.5), untilFive('3'));
        }
        console.log(out.length, out.slice(0, 40).join(' '), out.slice(-300).join(' '));
        "#,
    );
}

#[test]
fn strings_truthiness_typeof_and_cells_out_of_the_way() {
    // The small helpers compiled code calls instead of the generic one: a string's methods and length, `+` with a
    // string, comparing strings, `===` of strings, truthiness and `typeof` of every kind, and cells holding objects
    // (the write barrier) while collections happen.
    same(
        r#"
        const values = ['', 'a', 'abc', 0, -0, 0.5, NaN, 1n, 0n, null, undefined, true, false, {}, [], () => 1,
            class {}, Symbol('s'), new Map(), Math.max];
        function truth(v) { let t = ''; if (v) t += 'T'; else t += 'F'; t += !v ? '!' : '.'; return t + (v ? 1 : 0); }
        function kinds(v) {
            return [typeof v === 'function', typeof v === 'object', typeof v === 'string', typeof v !== 'number',
                typeof v === 'undefined', typeof v === 'boolean', typeof v === 'symbol', typeof v === 'bigint'].map(Number).join('');
        }
        function strings(s, i) {
            const t = s + '-' + i;
            return [t.length, t.toUpperCase(), t.slice(1, 3), t.indexOf('-'), t < 'b', t >= s, t === s + '-' + i,
                t !== 'x', `${t}:${i}`, t.charCodeAt(0), s.length === 0].join('|');
        }
        function counter() { let box = { n: 0 }; let list = []; return () => { box = { n: box.n + 1, prev: box }; list = [box, ...list.slice(0, 3)]; return box.n + list.length; }; }
        const out = [];
        let c = counter();
        for (let r = 0; r < 400; r++) {
            for (const v of values) out.push(truth(v), kinds(v));
            out.push(strings(['ab', 'zz', ''][r % 3], r));
            out.push(c());
            if (r % 50 === 0) { gc(); c = counter(); }
        }
        console.log(out.length, out.slice(0, 60).join(' '), out.slice(-20).join(' '));
        "#,
    );
}

#[test]
fn places_that_turn_polymorphic_or_megamorphic_after_compiling() {
    // Compiled while each place saw one shape; the interpreter would have rewritten them for more. Reads, writes,
    // adds and method calls at places with two, four and twenty-four shapes.
    same(
        r#"
        function read(o) { return o.x + o.y; }
        function write(o, v) { o.x = v; }
        function add(o, v) { o.added = v; return o; }
        function call(o) { return o.area(); }
        class A { constructor(i) { this.x = i; this.y = 1; } area() { return this.x; } }
        class B { constructor(i) { this.y = 2; this.x = i; } area() { return this.x * 2; } }
        let t = 0;
        for (let i = 0; i < 3000; i++) { const o = new A(i); t += read(o) + call(o); write(o, i); add({ q: 1 }, i); }
        const many = [];
        for (let k = 0; k < 24; k++) { const o = { ['k' + k]: k }; o.x = k; o.y = 1; o.area = () => k; many.push(o); }
        for (let r = 0; r < 300; r++) {
            for (const o of [new A(r), new B(r), { x: r, y: 3, area() { return 7; } }, { y: 4, x: r, area: () => 9 }]) {
                t += read(o) + call(o); write(o, r + 1); t += o.x; t += add(o, r).added;
            }
            for (const o of many) { t += read(o) + call(o); write(o, r); t += o.x; t += add({ ...o }, r).added; }
            t %= 1000003;
        }
        console.log(t);
        "#,
    );
}

#[test]
fn compiled_code_is_used_from_rust_and_from_the_interpreter() {
    // Callbacks from natives, code entered at the top of a loop, and a function too small to be worth going into
    // from outside compiled code, which compiled callers still call.
    same(
        r#"
        const small = (x) => x + 1;
        function big(list) { let s = 0; for (let i = 0; i < list.length; i++) s += small(list[i]); return s; }
        const list = Array.from({ length: 500 }, (_, i) => i);
        let t = 0;
        for (let r = 0; r < 30; r++) {
            t += list.map(small).reduce((a, b) => a + b, 0) + big(list);
            t += list.filter((x) => x % 3 === 0).length;
        }
        let u = 0;
        for (let i = 0; i < 100000; i++) u = (u + i * 7) % 1000003;
        console.log(t, u);
        "#,
    );
}

#[test]
fn calls_compiled_in_line_that_have_to_go_on_in_the_callee() {
    // Short methods and functions are compiled in line at a call that knows them. Each case here makes one of
    // their instructions leave the line partway (a number turning into a string, a shape the place hasn't seen, a
    // getter, an error, a variable read before it's set, a callee replaced), after earlier ones have had their
    // effect, which must happen once.
    same(
        r#"
        class P { constructor(x) { this.x = x; this.count = 0; } bump(k) { this.count = this.count + 1; return this.x + k; } }
        function plus(a, b) { return a + b; }
        function twice(o) { return o.v * 2; }
        let late = 0;
        function readLate() { return late + tdz; }
        function bump(c) { c.n++; return c.n; }
        const calls = { n: 0 };
        let t = 0, s = '';
        const p = new P(1);
        for (let i = 0; i < 6000; i++) {
            t += p.bump(i);
            t += plus(i, 1);
            if (i > 3000) s = plus(s.length > 20 ? '' : s, 'x');
            const o = i % 1000 === 999 ? { w: 1, v: 'q' } : { v: i };
            const r = twice(o);
            t += r === r ? r : 7;
            if (i === 4000) p.x = 'str';
            if (i === 5000) p.x = 2 ** 40;
            t = typeof t === 'number' ? t % 1000003 : 0;
            bump(calls);
        }
        let tdz = 1;
        console.log(t, s, p.count, p.x, calls.n);
        let errors = 0;
        function field(o) { return o.deep.x; }
        for (let i = 0; i < 4000; i++) {
            try { errors += field(i % 500 === 0 ? {} : { deep: { x: 1 } }); } catch (e) { errors += 1000; s = e.constructor.name; }
        }
        console.log(errors, s);
        // A getter where a plain property was, and a method replaced on the prototype after compiling.
        const obj = { get v() { return 5; } };
        let g = 0;
        for (let i = 0; i < 4000; i++) {
            g += twice(i < 3000 ? { v: i } : obj);
            if (i === 3500) P.prototype.bump = function (k) { return -k; };
            g += p.bump(1);
            g %= 1000003;
        }
        console.log(g, p.count);
        "#,
    );
    // (The run compiled once hot is the last.)
    let stats = LAST.with(|last| last.get());
    assert!(stats.inlined > 0 && stats.left_line > 0, "{stats:?}");
}

#[test]
fn calls_compiled_in_line_with_captured_variables_and_collections() {
    same(
        r#"
        function make() {
            let c = 0;
            return { inc() { c++; return c; }, add(k) { c = c + k; return c; }, get() { return c; } };
        }
        const m = make();
        const adders = [];
        for (let k = 0; k < 4; k++) adders.push((x) => x + k);
        const keep = [];
        let t = 0;
        for (let i = 0; i < 8000; i++) {
            m.inc();
            t += m.add(i & 3) & 0xffff;
            t += adders[0](i);
            if (i % 1000 === 0) { keep.push({ i }); gc(); }
            if (i === 6000) m.add('s');
            t %= 1000003;
        }
        // A `Math` function replaced by one that collects, called from a callee compiled in line.
        function root(x) { return Math.sqrt(x) + 1; }
        let r = 0;
        for (let i = 0; i < 4000; i++) {
            if (i === 3000) Math.sqrt = (x) => { gc(); return x; };
            r += root(i);
        }
        console.log(t, m.get(), keep.length, Math.round(r));
        "#,
    );
    // (The run compiled once hot is the last.)
    let stats = LAST.with(|last| last.get());
    assert!(stats.inlined > 0 && stats.left_line > 0, "{stats:?}");
}

#[test]
fn super_calls_derived_classes_and_instanceof_in_compiled_code() {
    same(
        r#"
        class A { constructor(x) { this.x = x; } }
        class B extends A { constructor(x) { super(x + 1); this.y = 2; } }
        class C extends B { constructor(x) { super(x); this.z = arguments.length; } }
        class Gives extends A { constructor(x) { super(x); return { given: x }; } }
        class Args { constructor() { this.n = arguments.length; } }
        class FromArgs extends Args { constructor(a, b) { super(a, b, 3); } }
        class Late extends A { constructor(x) { if (x < 0) { this.bad = 1; } super(x); } }
        class Twice extends A { constructor(x) { super(x); if (x === 77) super(x); } }
        class Arr extends Array { constructor(n) { super(n); this.extra = 1; } }
        class Err extends Error { constructor(m) { super(m); } }
        function Old(x) { this.old = x; }
        class FromOld extends Old { constructor(x) { super(x); } }
        let t = 0, notes = [];
        for (let i = 0; i < 3000; i++) {
            const c = new C(i), b = new B(i), g = new Gives(i);
            t += c.x + c.y + c.z + b.x + g.given + new FromArgs(1, 2).n + new FromOld(i).old;
            t += new Arr(3).length + (new Err('m') instanceof Error ? 1 : 0);
            t += (c instanceof A) + (c instanceof B) + (b instanceof C) * 10 + (g instanceof A) * 100;
            t += (i instanceof A) + ('s' instanceof Object) + (null instanceof Object);
            if (i % 500 === 0) {
                try { new Late(-1); } catch (e) { notes.push(e.constructor.name); }
                try { new Twice(77); } catch (e) { notes.push(e.constructor.name); }
                try { i instanceof {}; } catch (e) { notes.push(e.constructor.name); }
            }
            // A prototype changing, and an instanceof made special.
            if (i === 2000) Object.setPrototypeOf(C.prototype, A.prototype);
            if (i === 2500) Object.defineProperty(B, Symbol.hasInstance, { value: (v) => v === 5 });
            t += (5 instanceof B) + (c instanceof B) * 2;
            t %= 1000003;
        }
        // The parent of a class changed after compiling.
        class P1 { constructor() { this.p = 1; } }
        class P2 { constructor() { this.p = 2; } }
        class K extends P1 { constructor() { super(); } }
        let k = 0;
        for (let i = 0; i < 3000; i++) { if (i === 2000) Object.setPrototypeOf(K, P2); k += new K().p; }
        const bound = A.bind(null);
        let bt = 0;
        for (let i = 0; i < 2000; i++) bt += (new A(i) instanceof bound) ? 1 : 0;
        console.log(t, k, bt, notes.join());
        "#,
    );
}

#[test]
fn code_compiled_before_its_later_loops_ran_is_compiled_again() {
    // The script is compiled at its first loop, when the calls in its second have seen nothing, so they're made the
    // long way until the function is compiled again, with what they've learned, and goes on in the new code partway.
    same(
        r#"
        let t = 0;
        for (let i = 0; i < 3000; i++) t = (t + i * 3) % 1000003;
        const base = { greet() { return 1; }, level: 2 };
        let chain = base;
        for (let i = 0; i < 4; i++) chain = Object.create(chain);
        class Counter { constructor() { this.n = 0; } inc() { this.n++; return this.n; } }
        const c = new Counter();
        for (let i = 0; i < 5000; i++) {
            t += chain.greet() + chain.level + c.inc();
            if (i === 4000) base.greet = function () { return 10; };
            t %= 1000003;
        }
        console.log(t, c.n);
        "#,
    );
    let stats = LAST.with(|last| last.get());
    assert!(stats.recompiled > 0 && stats.inlined > 0, "{stats:?}");
}

#[test]
fn methods_at_places_that_have_seen_several_shapes() {
    // Each method a polymorphic place has found is called directly or compiled in line, by its proto; another one,
    // one replaced, and one that can't go on in line go the other ways.
    same(
        r#"
        class Circle { constructor(r) { this.r = r; } area() { return 3 * this.r * this.r; } }
        class Square { constructor(s) { this.s = s; } area() { return this.s * this.s; } }
        class Tri { constructor(b, h) { this.b = b; this.h = h; } area() { return (this.b * this.h) >> 1; } }
        class Odd { constructor() { this.k = 1; } area() { let t = 0; for (let i = 0; i < 3; i++) t += i; return t + this.k; } }
        class Late { area() { return 1000; } }
        const shapes = [];
        for (let i = 0; i < 40; i++) shapes.push(i % 4 === 0 ? new Circle(i) : i % 4 === 1 ? new Square(i) : i % 4 === 2 ? new Tri(i, 2) : new Odd());
        let area = 0;
        for (let r = 0; r < 300; r++) {
            for (let i = 0; i < shapes.length; i++) area += shapes[i].area();
            if (r === 200) shapes.push(new Late());
            if (r === 250) Square.prototype.area = function () { return -1; };
            if (r === 270) shapes[0].r = 'x';
            area %= 1000003;
        }
        console.log(area);
        "#,
    );
    let stats = LAST.with(|last| last.get());
    assert!(stats.inlined > 0 && stats.left_line > 0, "{stats:?}");
}

#[test]
fn shapes_written_into_the_code_and_what_changes_under_them() {
    // Reads and writes compare with the shapes their places knew when compiled, as constants. Here the objects with
    // those shapes all die and collections run (a freed shape's number can be given to another, which mustn't be
    // taken for it), prototypes change under reads of what they hold, a setter appears where adds went, and objects
    // turn into dictionaries.
    same(
        r#"
        function readX(o) { return o.x; }
        function readM(o) { return o.m(); }
        function make(i) { const o = {}; o.x = i; o.y = i + 1; return o; }
        function grow(o, v) { o.z = v; return o; }
        class P { m() { return 1; } }
        let t = 0;
        for (let i = 0; i < 3000; i++) t += readX(make(i)) + readM(new P()) + grow(make(i), i).z;
        gc(); gc();
        // Other shapes now, made in another order, after the old ones' objects are gone.
        for (let i = 0; i < 3000; i++) {
            const o = i % 2 ? { y: i, x: 2 * i } : { q: 1, r: 2, x: i };
            t += readX(o);
            if (i % 700 === 0) gc();
        }
        P.prototype.m = function () { return 5; };
        for (let i = 0; i < 2000; i++) t += readM(new P());
        Object.defineProperty(Object.prototype, 'z', { set(v) { this.zz = v * 3; }, configurable: true });
        for (let i = 0; i < 2000; i++) { const g = grow(make(i), 2); t += (g.z === undefined ? 1 : 0) + g.zz; }
        delete Object.prototype.z;
        for (let i = 0; i < 2000; i++) { const o = make(i); delete o.y; o.w = 1; t += readX(o) + grow(o, 1).z; }
        console.log(t);
        "#,
    );
}

/// The feedback compiled code records (`jit::feedback`): each kind of record says what its instruction met.
#[cfg(feature = "jit-opt")]
mod feedback {
    use super::super::feedback::*;
    use super::super::{Mode, feedback_of};
    use crate::Engine;

    /// Runs a program with every function compiled as it's first called, and gives the records of the function
    /// named `name`, by its instructions' names, with the JIT's counts.
    fn records(source: &str, name: &str) -> (Vec<(String, Vec<u8>)>, super::super::Stats) {
        let mut engine = Engine::new();
        engine.vm.jit.mode = Mode::Eager;
        engine.vm.jit.background = false;
        engine.set_printer(|_| {});
        engine
            .eval(source, "feedback.js")
            .and_then(|_| engine.run_until_idle())
            .expect("the program runs");
        (feedback_of(&engine.vm, name), engine.vm.jit.stats())
    }

    /// The first record of an instruction whose name starts with that.
    fn first<'a>(records: &'a [(String, Vec<u8>)], op: &str) -> &'a [u8] {
        &records.iter().find(|(name, _)| name.starts_with(op)).unwrap_or_else(|| panic!("no {op} in {records:?}")).1
    }

    fn word(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[4 * at..4 * at + 4].try_into().unwrap())
    }

    // Each function has a loop, so that no call of it is compiled in line (where it records nothing), and is called
    // from what an array gives, which a call can't guess.
    const LOOP: &str = "for (let k = 0; k < 1; k++) {}";

    #[test]
    fn number_records_say_what_kinds_of_numbers_were_met() {
        let program = |calls: &str| format!("function f(a, b) {{ {LOOP} return a + b; }} const g = [f][0]; {calls}");
        let kinds = |calls: &str| first(&records(&program(calls), "f").0, "Add")[0];
        assert_eq!(kinds("g(1, 2); g(3, 4);"), 0);
        assert_eq!(kinds("g(1, 2); g(1.5, 2);"), DOUBLE);
        // An int result that isn't an int32 is a double's, as V8's feedback has it.
        assert_eq!(kinds("g(2147483647, 1);"), DOUBLE);
        assert_ne!(kinds("g(1, 2); g('a', 2);") & STRING, 0);
        assert_eq!(kinds("g(1, 2); g('a', 2);") & OTHER, 0);
        assert_ne!(kinds("g(1, 2); g(undefined, 2);") & OTHER, 0);
        let product = format!("function f(a, b) {{ {LOOP} return a * b; }} const g = [f][0]; g(0, -5); g(2, 3);");
        assert_eq!(first(&records(&product, "f").0, "Mul")[0], DOUBLE);
        let ints = format!("function f(a, b) {{ {LOOP} return a * b; }} const g = [f][0]; g(4, -5); g(2, 3);");
        assert_eq!(first(&records(&ints, "f").0, "Mul")[0], 0);
    }

    #[test]
    fn branch_records_say_which_ways_they_went() {
        let program = |calls: &str| {
            format!("function f(x) {{ {LOOP} if (x > 0) return 1; return 2; }} const g = [f][0]; {calls}")
        };
        let ways = |calls: &str| {
            let records = records(&program(calls), "f").0;
            // The second compare-and-jump: the first is the loop's test before it starts.
            records.iter().filter(|(name, _)| name.starts_with("JumpNot")).nth(1).expect("the `if`").1[1]
        };
        let one = ways("g(1); g(2);");
        assert!(one == TAKEN || one == FELL, "{one}");
        assert_eq!(ways("g(1); g(-1);"), TAKEN | FELL);
        let truthy = format!("function f(x) {{ {LOOP} if (x) return 1; return 2; }} const g = [f][0]; g(true);");
        let records = records(&truthy, "f").0;
        let way = first(&records, "JumpIfFalse")[0];
        assert!(way == TAKEN || way == FELL, "{way}");
    }

    #[test]
    fn call_records_say_what_was_called() {
        let program = |calls: &str| {
            format!(
                "function via(f) {{ {LOOP} return f(); }} function one() {{ return 1; }} function two() {{ return 2; }}
                 const v = [via][0]; {calls}"
            )
        };
        let call = |calls: &str| first(&records(&program(calls), "via").0, "Call").to_vec();
        let once = call("v(one); v(one); v(one);");
        assert!(word(&once, 0) != 0 && word(&once, 0) != MANY && word(&once, 0) & NATIVE == 0, "{once:?}");
        assert_eq!(word(&call("v(one); v(two); v(one);"), 0), MANY);
        let native = call("v(Date.now); v(Date.now);");
        assert_ne!(word(&native, 0) & NATIVE, 0, "{native:?}");
        let bound = call("v(one.bind(null));");
        assert_ne!(word(&bound, 1) & OTHER_CALLEE, 0, "{bound:?}");
    }

    #[test]
    fn new_records_say_what_was_made() {
        let program = format!(
            "class P {{ constructor(x) {{ this.x = x; }} }} function make(x) {{ {LOOP} return new P(x); }}
             const m = [make][0]; m(1); m(2); m(3);"
        );
        let made = first(&records(&program, "make").0, "New").to_vec();
        assert!(word(&made, 0) != 0 && word(&made, 0) != MANY, "{made:?}");
        assert!(word(&made, 1) != 0 && word(&made, 1) != MANY, "{made:?}");
    }

    #[test]
    fn element_records_say_what_was_read_and_written() {
        let read = |args: &str| {
            let program = format!("function get(a, i) {{ {LOOP} return a[i]; }} const g = [get][0]; g({args}); g({args});");
            first(&records(&program, "get").0, "GetElem").to_vec()
        };
        assert_eq!(read("[1, 2, 3], 1")[0], 0);
        assert_eq!(read("[1, , 3], 1")[0], HOLE);
        assert_eq!(read("[1], 5")[0], PAST_END);
        let typed = read("new Int16Array(4), 1");
        assert_eq!(typed[0], TYPED);
        assert_ne!(typed[1], read("new Float64Array(4), 1")[1], "the kinds differ");
        let named = read("{ x: 1 }, 'x'");
        assert_eq!(named[0], NAMED | NOT_ARRAY);
        assert!(word(&named, 1) != 0 && word(&named, 1) != MANY, "{named:?}");
        assert_eq!(read("'abc', 1")[0], PRIMITIVE);
        let written = |args: &str| {
            let program =
                format!("function put(a, i) {{ {LOOP} a[i] = 2; }} const p = [put][0]; p({args}); p({args});");
            first(&records(&program, "put").0, "SetElem").to_vec()
        };
        assert_eq!(written("new Uint8Array(4), 1")[0], TYPED);
        assert_eq!(written("[], 3")[0], PAST_END);
    }

    #[test]
    fn a_function_is_due_once_its_count_has_gone_and_its_feedback_has_settled() {
        let program = "function hot(n) { let s = 0; for (let i = 0; i < n; i++) s += i; return s; }
                       const h = [hot][0]; for (let r = 0; r < 12000; r++) h(1);";
        let (_, stats) = records(program, "hot");
        assert!(stats.counted_out >= 2, "{stats:?}");
        assert!(stats.due >= 1, "{stats:?}");
        let few = "function hot(n) { let s = 0; for (let i = 0; i < n; i++) s += i; return s; }
                   const h = [hot][0]; for (let r = 0; r < 5000; r++) h(1);";
        assert_eq!(records(few, "hot").1.due, 0);
    }
}
