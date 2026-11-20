//! Tests of the whole engine, running real code.

use std::cell::RefCell;
use std::rc::Rc;

use super::*;

/// Runs a program to its end, and gives what it logged, one line to each call.
fn run(source: &str) -> String {
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let result = engine
        .eval(source, "test.js")
        .and_then(|_| engine.run_until_idle());
    if let Err(error) = result {
        panic!("{source}\nthrew: {error}");
    }
    output.borrow().trim_end().to_owned()
}

fn error_of(source: &str) -> String {
    let mut engine = Engine::new();
    engine.set_printer(|_| {});
    match engine
        .eval(source, "test.js")
        .and_then(|_| engine.run_until_idle())
    {
        Ok(_) => panic!("{source}\nshould have thrown"),
        Err(error) => error.message,
    }
}

#[test]
fn arithmetic_works() {
    assert_eq!(
        run("console.log(1 + 2 * 3, 7 % 3, 2 ** 10, -5 / 2, 7 >> 1, 1 << 31, -1 >>> 28)"),
        "7 1 1024 -2.5 3 -2147483648 15"
    );
    assert_eq!(
        run("console.log(2147483647 + 1, -2147483648 - 1, 65536 * 65536)"),
        "2147483648 -2147483649 4294967296"
    );
    assert_eq!(
        run("console.log(0.1 + 0.2, 1 / 0, -1 / 0, 0 / 0)"),
        "0.30000000000000004 Infinity -Infinity NaN"
    );
    assert_eq!(
        run("console.log(0 * -1, -0 + 0, 5 % -3, -5 % 3)"),
        "-0 0 2 -2"
    );
    assert_eq!(
        run("let i = 2147483647; i++; console.log(i); let j = -2147483648; j--; console.log(j)"),
        "2147483648\n-2147483649"
    );
}

/// The instructions for a power of two on the right of `/`, a number on the left of `*`, `&` and `|`, a `?:` that
/// goes straight into its variable, and `while (a !== b)` with its test at the bottom.
#[test]
fn shorter_ways_with_numbers_give_the_same_answers() {
    assert_eq!(
        run(
            "const xs = [-6, 0, -0, 7, -2147483648, 2147483647, 6.5, '6', null, -7, 1e300]; console.log(xs.map(x => [x / 2, x / 1, x / 4, x / 16384, Object.is(x / 2, -0)].join()).join(' '))"
        ),
        "-3,-6,-1.5,-0.0003662109375,false 0,0,0,0,false 0,0,0,0,true 3.5,7,1.75,0.00042724609375,false -1073741824,-2147483648,-536870912,-131072,false 1073741823.5,2147483647,536870911.75,131071.99993896484,false 3.25,6.5,1.625,0.000396728515625,false 3,6,1.5,0.0003662109375,false 0,0,0,0,false -3.5,-7,-1.75,-0.00042724609375,false 5e+299,1e+300,2.5e+299,6.103515625e+295,false"
    );
    assert_eq!(
        run(
            "let calls = 0; const o = { valueOf() { calls++; return 5; } }; console.log(3 * o, 6 & o, 1 | o, calls, 3 * -0, Object.is(0 * -1, -0), Object.is(-1 * 0, -0), 2 * 3, 4 & 6)"
        ),
        "15 4 5 3 -0 true true 6 4"
    );
    assert_eq!(
        run(
            "let n = 5; n = n > 0 ? n - 1 : n + 1; let m = 3; m = m > 9 ? m : (m = 7) + m; console.log(n, m); let steps = 0, k = 27; while (k !== 1) { k = (k & 1) === 0 ? k / 2 : 3 * k + 1; steps++; } console.log(steps)"
        ),
        "4 14\n111"
    );
    assert_eq!(
        run(
            "let d = 0.5; while (d !== 4.5) d += 1; let s = 'a'; const t = 'aaa'; while (s !== t) s += 'a'; let e = 0; do { e++ } while (e !== 3); let f = 4; const four = 4.0 / 1; while (f !== four) f++; console.log(d, s, e, f)"
        ),
        "4.5 aaa 3 4"
    );
    assert_eq!(
        run(
            "const f = (x, y) => (x + y) | 0; const g = (x, y) => (x + y) | -0; console.log([f(2147483647, 1), f(1.5, 2.7), f('1', 2), f({ valueOf() { return 3; } }, 4), f(-0, -0), f(NaN, 1), f(2 ** 31, 2 ** 31), f(-2147483648, -1), g(5, 6), f(1e21, 1)].join()); try { f(1n, 2n) } catch (e) { console.log(e.name) }"
        ),
        "-2147483648,4,12,7,0,0,0,2147483647,11,-559939584\nTypeError"
    );
}

/// `Math.sqrt(x)` and the like are worked out by the interpreter itself while they're the built-in functions and
/// given numbers, and are called as usual otherwise.
#[test]
fn math_functions_called_straight_from_the_interpreter_give_what_they_would() {
    assert_eq!(
        run(
            "const r = []; for (const x of [4, -1.5, 2.5, -0.5, 0, -0, NaN, 1e300, -2147483648, 7]) r.push([Math.sqrt(x), Math.floor(x), Math.ceil(x), Math.round(x), Math.abs(x), Math.trunc(x), Math.sign(x), Object.is(Math.round(x), -0), Math.fround(x), Math.exp(x) > 0].join()); console.log(r.join(' '))"
        ),
        "2,4,4,4,4,4,1,false,4,true NaN,-2,-1,-1,1.5,-1,-1,false,-1.5,true 1.5811388300841898,2,3,3,2.5,2,1,false,2.5,true NaN,-1,0,0,0.5,0,-1,true,-0.5,true 0,0,0,0,0,0,0,false,0,true 0,0,0,0,0,0,0,true,0,true NaN,NaN,NaN,NaN,NaN,NaN,NaN,false,NaN,false 1e+150,1e+300,1e+300,1e+300,1e+300,1e+300,1,false,Infinity,true NaN,-2147483648,-2147483648,-2147483648,2147483648,-2147483648,-1,false,-2147483648,false 2.6457513110645907,7,7,7,7,7,1,false,7,true"
    );
    assert_eq!(
        run(
            "console.log(Object.is(Math.max(-0, 0), 0), Object.is(Math.min(0, -0), -0), Math.max(NaN, 1), Math.min(3, 2), Math.imul(2 ** 31, 3), Math.pow(2, 10), Math.hypot(3, 4), Math.atan2(1, 1), Math.max(1, '5'), Math.sqrt('16'), Math.floor(null))"
        ),
        "true true NaN 2 -2147483648 1024 5 0.7853981633974483 5 4 0"
    );
    assert_eq!(
        run(
            "const out = []; for (let i = 0; i < 4; i++) { if (i === 2) Math.sqrt = (x) => x * 2; out.push(Math.sqrt(9)); } console.log(out.join()); { const Math = { floor: (x) => 'mine ' + x }; console.log(Math.floor(2.5)); } let v = 0; const o = { valueOf() { v++; return 2.5; } }; console.log(Math.floor(o), v, Math.max(o, o), v); globalThis.Math = { cos: () => 'other' }; console.log(Math.cos(0))"
        ),
        "3,3,18,18\nmine 2.5\n2 1 2.5 3\nother"
    );
}

#[test]
fn strings_join_and_compare() {
    assert_eq!(
        run("console.log('a' + 'b', 'a' + 1, 1 + 'a', `x${1 + 1}y${'z'}`)"),
        "ab a1 1a x2yz"
    );
    assert_eq!(
        run("console.log('a' < 'b', 'b' < 'a', '10' < 9, 'abc' === 'ab' + 'c')"),
        "true false false true"
    );
    assert_eq!(
        run("console.log('h\\u00e9llo'.length, 'h\\u00e9llo'.toUpperCase())"),
        "5 H\u{c9}LLO"
    );
}

#[test]
fn variables_have_scopes() {
    assert_eq!(
        run("let a = 1; { let a = 2; console.log(a) } console.log(a)"),
        "2\n1"
    );
    assert_eq!(run("var a = 1; { var a = 2 } console.log(a)"), "2");
    assert_eq!(run("console.log(typeof f); function f() {}"), "function");
    assert_eq!(run("console.log(a); var a = 1"), "undefined");
    assert!(error_of("console.log(a); let a = 1").contains("before initialization"));
    assert!(error_of("const a = 1; a = 2").contains("constant"));
    assert!(error_of("nothing").contains("not defined"));
    assert_eq!(run("console.log(typeof nothing)"), "undefined");
}

#[test]
fn closures_capture_variables() {
    assert_eq!(
        run("function c() { let n = 0; return () => ++n } const f = c(); f(); console.log(f())"),
        "2"
    );
    assert_eq!(
        run(
            "const fs = []; for (let i = 0; i < 3; i++) fs.push(() => i); console.log(fs.map(f => f()).join())"
        ),
        "0,1,2"
    );
    assert_eq!(
        run(
            "const fs = []; for (var i = 0; i < 3; i++) fs.push(() => i); console.log(fs.map(f => f()).join())"
        ),
        "3,3,3"
    );
    assert_eq!(
        run(
            "const fs = []; for (const x of [1, 2]) fs.push(() => x); console.log(fs[0](), fs[1]())"
        ),
        "1 2"
    );
    assert_eq!(
        run(
            "function a(x) { return function () { return function () { return x } } } console.log(a(5)()())"
        ),
        "5"
    );
    assert!(error_of("function f() { return x } f(); let x = 1").contains("before initialization"));
}

#[test]
fn loops_compare_with_big_numbers() {
    assert_eq!(
        run(
            "let a = 0; for (let i = 0; i < 100000; i++) a++;
             let b = 0, j = 0; while (j < 70000) { j += 3; b++ }
             let c = 0, k = 50000; do { k -= 7; c++ } while (k >= -40000.5);
             let d = 0; for (let i = 0; i <= 40000; i += 10000) d += i;
             let e = 0; for (let i = 1e10; i > 9999999990; i--) e++;
             console.log(a, b, j, c, k, d, e)"
        ),
        "100000 23334 70002 12858 -40006 100000 10"
    );
}

#[test]
fn a_function_that_declares_a_name_doesnt_capture_the_one_outside() {
    // Each `i` inside is the function's own, so the loop's `i` can stay in a register.
    assert_eq!(
        run(
            "function p(i) { return () => i * 2 } function q() { var i = 7; return () => i } function r() { let i = 8; return i }
             let s = 0; for (let i = 0; i < 3; i++) s += p(i)() + q()() + r(); console.log(s)"
        ),
        "51"
    );
    // A default sees the variable outside, not the body's.
    assert_eq!(
        run("let x = 1; function f(a = () => x) { var x = 2; return a() } console.log(f())"),
        "1"
    );
    assert_eq!(
        run("let x = 1; function f(a = () => x) { let y = 2; return a() + y } x = 5; console.log(f())"),
        "7"
    );
    // A block's declaration covers only the block.
    assert_eq!(
        run("let i = 3; function f() { { let i = 1 } return () => i } i = 4; console.log(f()())"),
        "4"
    );
    assert_eq!(
        run("let i = 3; const f = (j) => { if (j) { var i = j } return i }; console.log(f(0), f(2), i)"),
        "undefined 2 3"
    );
    // The function's own name, said inside it, is still the variable outside.
    assert_eq!(
        run("function g(n) { return n ? g(n - 1) + 1 : 0 } const h = g; g = () => 100; console.log(h(3))"),
        "101"
    );
}

#[test]
fn a_variable_read_before_an_assignment_in_the_same_expression_keeps_its_value() {
    assert_eq!(run("let a = 1; console.log(a + (a = 5), a)"), "6 5");
    assert_eq!(
        run("let a = 1; a = a || 2; console.log(a); let b = 0; b = b || 3; console.log(b)"),
        "1\n3"
    );
    assert_eq!(
        run(
            "let x = 1; x = [x, x + 1]; console.log(x.join()); let y = 2; y = { y }; console.log(y.y)"
        ),
        "1,2\n2"
    );
    assert_eq!(run("let n = 1; n = n ? n + 1 : n; console.log(n)"), "2");
}

#[test]
fn functions_take_arguments_in_every_way() {
    assert_eq!(
        run(
            "function f(a, b = a + 1, ...c) { return [a, b, c.length].join() } console.log(f(1), f(1, 5), f(1, 2, 3, 4))"
        ),
        "1,2,0 1,5,0 1,2,2"
    );
    assert_eq!(
        run("function f() { return arguments.length + arguments[0] } console.log(f(1, 2))"),
        "3"
    );
    assert_eq!(
        run("function f(a) { return a } console.log(f(), f(1, 2))"),
        "undefined 1"
    );
    assert_eq!(
        run("console.log(((a, b) => a + b)(1, 2), (a => a * 2)(4), (() => 5)())"),
        "3 8 5"
    );
    assert_eq!(
        run(
            "function f({ a, b: [c] }, [d, ...e]) { return a + c + d + e.length } console.log(f({ a: 1, b: [2] }, [3, 4, 5]))"
        ),
        "8"
    );
    assert_eq!(
        run(
            "console.log(((a, b) => 0).length, (function (a, b = 1, c) {}).length, f.name); function f() {}"
        ),
        "2 1 f"
    );
    assert_eq!(
        run(
            "const g = () => {}; const h = function () {}; const o = { m() {}, n: () => {} }; console.log(g.name, h.name, o.m.name, o.n.name)"
        ),
        "g h m n"
    );
    assert_eq!(
        run("console.log(Math.max(...[1, 5, 3]), Math.max(0, ...[1, 2], 9))"),
        "5 9"
    );
    assert_eq!(
        run(
            "const fact = function me(n) { return n <= 1 ? 1 : n * me(n - 1) }; console.log(fact(5))"
        ),
        "120"
    );
}

#[test]
fn this_is_what_it_should_be() {
    assert_eq!(
        run(
            "const o = { v: 1, f() { return this.v }, g() { return (() => this.v)() } }; console.log(o.f(), o.g())"
        ),
        "1 1"
    );
    assert_eq!(
        run(
            "'use strict'; function f() { return this } console.log(f(), f.call(1), f.apply(2), f.bind(3)())"
        ),
        "undefined 1 2 3"
    );
    assert_eq!(
        run(
            "function f() { return this } console.log(f() === globalThis, typeof f.call(1), f.call(null) === globalThis)"
        ),
        "true object true"
    );
    assert_eq!(
        run("function f() { 'use strict'; return this } console.log(f())"),
        "undefined"
    );
    assert_eq!(
        run("class A { m() { return (function () { return this })() } } console.log(new A().m())"),
        "undefined"
    );
    assert_eq!(
        run("x = 5; console.log(x, globalThis.x); function f() { y = 6 } f(); console.log(y)"),
        "5 5\n6"
    );
    assert!(error_of("'use strict'; x = 5").contains("not defined"));
    assert_eq!(
        run(
            "const o = { f() { return [1].map(function () { 'use strict'; return this }, 5)[0] } }; console.log(o.f())"
        ),
        "5"
    );
}

#[test]
fn objects_hold_properties() {
    assert_eq!(
        run(
            "const o = { a: 1, 'b c': 2, [`d`]: 3 }; o.e = 4; console.log(o.a, o['b c'], o.d, o.e, o.nope)"
        ),
        "1 2 3 4 undefined"
    );
    assert_eq!(
        run(
            "const o = { a: 1, b: 2 }; delete o.a; console.log(Object.keys(o).join(), 'a' in o, 'b' in o)"
        ),
        "b false true"
    );
    assert_eq!(
        run("const o = { b: 1, 2: 1, a: 1, 1: 1 }; console.log(Object.keys(o).join())"),
        "1,2,b,a"
    );
    assert_eq!(
        run("const a = { x: 1 }; const b = { ...a, y: 2, x: 3 }; console.log(JSON.stringify(b))"),
        "{\"x\":3,\"y\":2}"
    );
    assert_eq!(
        run("const { a, ...r } = { a: 1, b: 2, c: 3 }; console.log(a, JSON.stringify(r))"),
        "1 {\"b\":2,\"c\":3}"
    );
    assert_eq!(
        run(
            "const o = { get a() { return 1 }, set a(v) { this.b = v } }; o.a = 5; console.log(o.a, o.b)"
        ),
        "1 5"
    );
    assert_eq!(
        run(
            "const o = Object.create({ p: 1 }); o.q = 2; console.log(o.p, o.q, Object.keys(o).join()); for (const k in o) console.log(k)"
        ),
        "1 2 q\nq\np"
    );
    assert_eq!(
        run(
            "const o = {}; for (let i = 0; i < 300; i++) o['k' + i] = i; delete o.k5; let s = 0; for (const k in o) s += o[k]; console.log(s, o.k299)"
        ),
        "44845 299"
    );
}

#[test]
fn a_prototype_changing_is_seen_by_code_that_already_read_through_it() {
    assert_eq!(
        run(
            "function A() {} A.prototype.v = 1; const a = new A(); function r() { return a.v } r(); r(); A.prototype.v = 2; console.log(r()); a.v = 3; console.log(r()); delete a.v; console.log(r())"
        ),
        "2\n3\n2"
    );
    assert_eq!(
        run(
            "const p = {}; const o = Object.create(p); function r() { return o.x } console.log(r()); p.x = 1; console.log(r()); Object.setPrototypeOf(o, { x: 2 }); console.log(r())"
        ),
        "undefined\n1\n2"
    );
    assert_eq!(
        run(
            "class A { m() { return 1 } } class B extends A {} const b = new B(); function r() { return b.m() } r(); r(); A.prototype.m = () => 2; console.log(r()); B.prototype.m = () => 3; console.log(r())"
        ),
        "2\n3"
    );
}

#[test]
fn a_property_that_isnt_there_is_seen_once_it_is() {
    // A place that reads a property nothing has remembers that, until a prototype changes or the object gets it.
    assert_eq!(
        run(
            "function A() {} const a = new A(); const r = () => a.v; r(); r(); const out = [r()]; A.prototype.v = 1; out.push(r()); a.v = 2; out.push(r()); delete a.v; out.push(r()); delete A.prototype.v; out.push(r()); Object.prototype.v = 3; out.push(r()); delete Object.prototype.v; out.push(r()); Object.defineProperty(A.prototype, 'v', { get() { return 4 }, configurable: true }); out.push(r()); Object.setPrototypeOf(a, { v: 5 }); out.push(r()); console.log(out.join())"
        ),
        ",1,2,1,,3,,4,5"
    );
    assert_eq!(
        run(
            "const list = [{ k: 1 }, { j: 1 }, { i: 1 }, { h: 1 }, { g: 1 }, { f: 1 }]; const r = (o) => o.missing; for (let n = 0; n < 3; n++) list.forEach(r); const before = list.map(r).join(); Object.prototype.missing = 'p'; console.log(before, list.map(r).join())"
        ),
        ",,,,, p,p,p,p,p,p"
    );
    assert_eq!(
        run(
            "class P { get x() { return this._x } } const o = new P(); const r = () => o.y; r(); r(); P.prototype.y = 'y'; console.log(r())"
        ),
        "y"
    );
}

#[test]
fn instanceof_sees_what_changes_after_it_has_looked_before() {
    // `instanceof` remembers where a function's `Symbol.hasInstance` and `prototype` are, for functions of its shape.
    assert_eq!(
        run(
            "function F() {} const a = new F(); const t = () => a instanceof F; t(); t(); console.log(t()); F.prototype = {}; console.log(t(), new F() instanceof F); Object.setPrototypeOf(a, F.prototype); console.log(t())"
        ),
        "true\nfalse true\ntrue"
    );
    assert_eq!(
        run(
            "class A {} class B extends A {} const b = new B(); const t = () => [b instanceof A, b instanceof B, 1 instanceof B].join(); t(); console.log(t()); Object.defineProperty(A, Symbol.hasInstance, { value: () => false }); console.log(t()); Object.defineProperty(B, Symbol.hasInstance, { value: () => true }); console.log(t())"
        ),
        "true,true,false\nfalse,false,false\nfalse,true,true"
    );
    assert_eq!(
        run(
            "function G() {} const g = new G(); const t = () => g instanceof G; t(); t(); Object.setPrototypeOf(G, { [Symbol.hasInstance]: () => 0 }); console.log(t()); Object.setPrototypeOf(G, null); console.log(t())"
        ),
        "false\ntrue"
    );
    assert!(
        error_of("function H() {} const t = () => ({}) instanceof H; t(); t(); H.prototype = 1; t()")
            .contains("non-object prototype")
    );
}

#[test]
fn writes_through_setters_and_read_only_properties_are_respected() {
    assert_eq!(
        run(
            "let s; const p = { set x(v) { s = v } }; const o = Object.create(p); function w(t) { t.x = 1 } w(o); w(o); console.log(s, Object.keys(o).length)"
        ),
        "1 0"
    );
    assert!(
        error_of("'use strict'; const o = Object.freeze({ a: 1 }); o.a = 2").contains("read only")
    );
    assert!(
        error_of("'use strict'; const o = Object.freeze({ a: 1 }); o.b = 2")
            .contains("not extensible")
    );
    assert!(error_of("'use strict'; const a = Object.freeze([1]); a[0] = 2").contains("read only"));
    assert_eq!(
        run(
            "'use strict'; function w(o) { o.a = 1 } class C {} for (let i = 0; i < 3; i++) w(new C()); const f = new C(); Object.freeze(f); try { w(f) } catch (e) { console.log(e.name) }"
        ),
        "TypeError"
    );
    assert_eq!(
        run(
            "console.log(Object.isFrozen(Object.freeze({})), Object.isFrozen({}), Object.isSealed(Object.seal({ a: 1 })))"
        ),
        "true false true"
    );
}

#[test]
fn arrays_hold_elements() {
    assert_eq!(
        run(
            "const a = [1, 2, 3]; a.push(4); console.log(a.length, a.pop(), a.shift(), a.join('-'))"
        ),
        "4 4 1 2-3"
    );
    assert_eq!(
        run("const a = []; a[3] = 1; console.log(a.length, 2 in a, 3 in a, JSON.stringify(a))"),
        "4 false true [null,null,null,1]"
    );
    assert_eq!(
        run(
            "const a = [1, 2, 3]; a.length = 1; console.log(a.join()); a.length = 2; console.log(a[1])"
        ),
        "1\nundefined"
    );
    assert_eq!(
        run(
            "console.log([3, 1, 2].sort().join(), [10, 9, 1].sort().join(), [10, 9, 1].sort((a, b) => a - b).join())"
        ),
        "1,2,3 1,10,9 1,9,10"
    );
    assert_eq!(
        run("console.log([1, 2, 3].map(x => x * 2).filter(x => x > 2).reduce((a, b) => a + b, 0))"),
        "10"
    );
    assert_eq!(
        run("console.log([1, [2, [3]]].flat(Infinity).join(), [1, 2].flatMap(x => [x, x]).join())"),
        "1,2,3 1,1,2,2"
    );
    assert_eq!(
        run(
            "console.log([1, 2, 3].slice(1).join(), [1, 2, 3].slice(-1).join(), [1, 2, 3, 4].splice(1, 2).join(), [1, 2].concat(3, [4]).join())"
        ),
        "2,3 3 2,3 1,2,3,4"
    );
    assert_eq!(
        run(
            "console.log([1, 2, 3].indexOf(2), [1, NaN].includes(NaN), [1, NaN].indexOf(NaN), [1, 2].at(-1), [1, 2].find(x => x > 1), [1, 2].findIndex(x => x > 5))"
        ),
        "1 true -1 2 2 -1"
    );
    assert_eq!(
        run(
            "console.log(Array.from('abc').join(), Array.from({ length: 2 }, (_, i) => i).join(), Array.of(1, 2).length, new Array(3).length, Array.isArray([]))"
        ),
        "a,b,c 0,1 2 3 true"
    );
    assert_eq!(
        run(
            "const s = [{ k: 1, v: 'a' }, { k: 0, v: 'b' }, { k: 1, v: 'c' }, { k: 0, v: 'd' }]; console.log(s.sort((x, y) => x.k - y.k).map(x => x.v).join(''))"
        ),
        "bdac"
    );
    assert_eq!(
        run(
            "const a = []; for (let i = 0; i < 100; i++) a.push((i * 37) % 101); a.sort((x, y) => y - x); let good = true; for (let i = 1; i < 100; i++) if (a[i - 1] < a[i]) good = false; console.log(good)"
        ),
        "true"
    );
}

#[test]
fn destructuring_gets_things_out() {
    assert_eq!(
        run("const [a, , b = 5, ...c] = [1, 2, undefined, 4, 5]; console.log(a, b, c.join())"),
        "1 5 4,5"
    );
    assert_eq!(
        run("let a = 1, b = 2; [a, b] = [b, a]; console.log(a, b)"),
        "2 1"
    );
    assert_eq!(
        run(
            "const o = {}; [o.x, o['y']] = [1, 2]; ({ a: o.z } = { a: 3 }); console.log(o.x, o.y, o.z)"
        ),
        "1 2 3"
    );
    assert_eq!(
        run("const { a: { b }, c = 2 } = { a: { b: 1 } }; console.log(b, c)"),
        "1 2"
    );
    assert!(error_of("const { a } = null").contains("destructure"));
    assert_eq!(
        run("function* g() { yield 1; yield 2; yield 3 } const [x, y] = g(); console.log(x, y)"),
        "1 2"
    );
    assert_eq!(
        run(
            "const [a, b] = new Set([1, 2]); const [[k, v]] = new Map([['x', 1]]); console.log(a, b, k, v)"
        ),
        "1 2 x 1"
    );
    assert_eq!(run("const [c] = 'hi'; console.log(c)"), "h");
}

#[test]
fn copies_of_objects_and_arrays_go_the_quick_way_only_when_they_may() {
    // Each runs a few times, so the second and later go the way that was remembered the first time.
    assert_eq!(
        run("const out = [];
             for (let r = 0; r < 3; r++) {
                 const k = 'b';
                 const { a, [k]: b, ...rest } = { a: 1, b: 2, c: 3, d: r };
                 const { x, ...others } = { x: 1, y: 2, ['z' + r]: 3 };
                 out.push(a, b, JSON.stringify(rest), x, JSON.stringify(others));
             }
             console.log(out.join(' '))"),
        r#"1 2 {"c":3,"d":0} 1 {"y":2,"z0":3} 1 2 {"c":3,"d":1} 1 {"y":2,"z1":3} 1 2 {"c":3,"d":2} 1 {"y":2,"z2":3}"#
    );
    // A setter that turns up on the prototype after Object.assign has remembered how to add the property is called.
    assert_eq!(
        run("const seen = [], out = [], proto = {};
             for (let r = 0; r < 3; r++) {
                 if (r === 2) Object.defineProperty(proto, 'v', { set(value) { seen.push(value) }, configurable: true });
                 out.push(JSON.stringify(Object.assign(Object.create(proto), { v: r, w: r + 1 }, { w: 9 })));
             }
             console.log(out.join(' '), seen.join())"),
        r#"{"v":0,"w":9} {"v":1,"w":9} {"w":9} 2"#
    );
    assert_eq!(
        run("const holes = [1, , 3];
             const spread = [0, ...holes, ...[4, 5, 6, 7, 8, 9, 10, 11, 12], ...'ab'];
             console.log(spread.length, 2 in spread, spread[2], spread.join())"),
        "15 true undefined 0,1,,3,4,5,6,7,8,9,10,11,12,a,b"
    );
}

#[test]
fn control_flow_goes_where_it_should() {
    assert_eq!(
        run(
            "let s = ''; for (let i = 0; i < 5; i++) { if (i == 1) continue; if (i == 3) break; s += i } console.log(s)"
        ),
        "02"
    );
    assert_eq!(
        run(
            "let s = ''; a: for (let i = 0; i < 3; i++) { for (let j = 0; j < 3; j++) { if (j == 1) continue a; if (i == 2) break a; s += i + '' + j + ' ' } } console.log(s)"
        ),
        "00 10"
    );
    assert_eq!(
        run("let i = 0; while (i < 3) i++; do { i++ } while (i < 2); console.log(i)"),
        "4"
    );
    assert_eq!(
        run(
            "function f(x) { switch (x) { case 1: return 'a'; case 2: case 3: return 'b'; default: return 'c' } } console.log(f(1), f(3), f(9))"
        ),
        "a b c"
    );
    assert_eq!(
        run(
            "let s = ''; switch (2) { case 1: s += 'a'; default: s += 'd'; case 3: s += 'c'; break; case 4: s += 'x' } console.log(s)"
        ),
        "dc"
    );
    assert_eq!(
        run("console.log(1 && 2, 0 && 2, 1 || 2, 0 || 2, null ?? 1, 0 ?? 1, !0, !1)"),
        "2 0 1 2 1 0 true false"
    );
    assert_eq!(
        run("let a = null; a ??= 1; let b = 0; b ||= 2; let c = 1; c &&= 3; console.log(a, b, c)"),
        "1 2 3"
    );
    assert_eq!(
        run(
            "const o = { a: { b() { return 1 } } }; console.log(o?.a?.b(), o.x?.b(), o.x?.[1], o.a.c?.())"
        ),
        "1 undefined undefined undefined"
    );
    assert_eq!(run("block: { console.log(1); break block; }"), "1");
}

#[test]
fn errors_can_be_thrown_and_caught() {
    assert_eq!(run("try { throw 1 } catch (e) { console.log(e) }"), "1");
    assert_eq!(
        run(
            "function f() { throw new Error('x') } try { f() } catch (e) { console.log(e.message, e.name, e instanceof Error) }"
        ),
        "x Error true"
    );
    assert_eq!(
        run("try { null.x } catch (e) { console.log(e.name, e.message) }"),
        "TypeError Cannot read properties of null (reading 'x')"
    );
    assert_eq!(
        run("try { undefined.x = 1 } catch (e) { console.log(e.message) }"),
        "Cannot set properties of undefined (setting 'x')"
    );
    assert_eq!(
        run("try { (1)() } catch (e) { console.log(e.message) }"),
        "1 is not a function"
    );
    assert_eq!(
        run(
            "class E extends Error { constructor(m) { super(m); this.name = 'E' } } try { throw new E('m') } catch (e) { console.log(e.name, e.message, e instanceof E, e instanceof Error, String(e)) }"
        ),
        "E m true true E: m"
    );
    assert_eq!(
        run("function r() { return r() } try { r() } catch (e) { console.log(e.name) }"),
        "RangeError"
    );
    assert_eq!(
        run("const e = new Error('a', { cause: 1 }); console.log(e.cause, e.toString())"),
        "1 Error: a"
    );
}

#[test]
fn finally_always_runs() {
    assert_eq!(
        run("function f() { try { return 1 } finally { console.log('f') } } console.log(f())"),
        "f\n1"
    );
    assert_eq!(
        run("function f() { try { return 1 } finally { return 2 } } console.log(f())"),
        "2"
    );
    assert_eq!(
        run(
            "function f() { try { throw 1 } catch (e) { return e + 1 } finally { console.log('f') } } console.log(f())"
        ),
        "f\n2"
    );
    assert_eq!(
        run("try { try { throw 1 } finally { console.log('a') } } catch (e) { console.log(e) }"),
        "a\n1"
    );
    assert_eq!(
        run(
            "for (let i = 0; i < 2; i++) { try { if (i == 0) continue; break } finally { console.log(i) } } console.log('e')"
        ),
        "0\n1\ne"
    );
    assert_eq!(
        run(
            "function f() { for (const x of [1, 2]) { try { try { return x } finally { console.log('in') } } finally { console.log('out') } } } console.log(f())"
        ),
        "in\nout\n1"
    );
    assert_eq!(
        run(
            "let i = 0; a: while (true) { try { try { break a } finally { i++ } } finally { i += 10 } } console.log(i)"
        ),
        "11"
    );
}

#[test]
fn classes_work() {
    assert_eq!(
        run(
            "class A { constructor(x) { this.x = x } get d() { return this.x * 2 } static s() { return 's' } m() { return this.x } } const a = new A(2); console.log(a.x, a.d, A.s(), a.m(), a instanceof A)"
        ),
        "2 4 s 2 true"
    );
    assert_eq!(
        run(
            "class A { m() { return 'A' } } class B extends A { m() { return super.m() + 'B' } } console.log(new B().m())"
        ),
        "AB"
    );
    assert_eq!(
        run(
            "class A { constructor() { this.n = new.target.name } } class B extends A {} console.log(new A().n, new B().n)"
        ),
        "A B"
    );
    assert_eq!(
        run(
            "class A { x = 1; y = this.x + 1; static z = 5; #p = 9; p() { return this.#p } static #q = 8; static q() { return A.#q } } const a = new A(); console.log(a.x, a.y, A.z, a.p(), A.q())"
        ),
        "1 2 5 9 8"
    );
    assert_eq!(
        run(
            "class A { x = 1 } class B extends A { y = this.x + 1; constructor() { super(); this.z = this.y + 1 } } const b = new B(); console.log(b.x, b.y, b.z)"
        ),
        "1 2 3"
    );
    assert_eq!(
        run("class A { f = () => this.v; v = 3 } const { f } = new A(); console.log(f())"),
        "3"
    );
    assert_eq!(
        run(
            "class A { static #c = 0; static inc() { return ++A.#c } } A.inc(); console.log(A.inc())"
        ),
        "2"
    );
    assert_eq!(
        run(
            "const C = class { m() { return 1 } }; console.log(C.name, new C().m()); const D = class E { n() { return E.name } }; console.log(new D().n())"
        ),
        "C 1\nE"
    );
    assert!(error_of("class A {} A()").contains("without 'new'"));
    assert!(
        error_of("class A extends Object { constructor() { this.x = 1 } } new A()")
            .contains("super")
    );
    assert!(error_of("class A extends 5 {}").contains("not a constructor"));
    assert_eq!(
        run(
            "class A { static #x(v) { return v } static y(v) { return A.#x(v) } #m() { return 1 } n() { return this.#m() } has(o) { return #m in o } } console.log(A.y(2), new A().n())"
        ),
        "2 1"
    );
    assert_eq!(
        run(
            "class A { get [Symbol.toStringTag]() { return 'T' } *[Symbol.iterator]() { yield 1 } } console.log(String(new A()), [...new A()].join())"
        ),
        "[object T] 1"
    );
    assert_eq!(
        run(
            "function F() { this.a = 1 } F.prototype.b = function () { return this.a + 1 }; class G extends F { c() { return this.b() } } console.log(new G().c(), new F().constructor === F)"
        ),
        "2 true"
    );
    assert_eq!(
        run(
            "class A { static x = 1; static { A.y = A.x + 1 } } class B extends A {} console.log(A.y, B.x)"
        ),
        "2 1"
    );
    assert_eq!(
        run(
            "class M extends Map { size2() { return this.size * 2 } } const m = new M([[1, 2]]); console.log(m.size2(), m.get(1), m instanceof Map); class Arr extends Array { sum() { return this.reduce((a, b) => a + b, 0) } } const r = new Arr(); r.push(1, 2); console.log(r.sum(), r.length)"
        ),
        "2 2 true\n3 2"
    );
}

#[test]
fn generators_stop_and_carry_on() {
    assert_eq!(
        run(
            "function* g() { const x = yield 1; console.log(x); yield x + 1; return 9 } const i = g(); console.log(i.next().value); console.log(i.next(5).value); const e = i.next(); console.log(e.value, e.done, i.next().done)"
        ),
        "1\n5\n6\n9 true true"
    );
    assert_eq!(
        run("function* g() { yield* [1, 2]; yield 3 } console.log([...g()].join())"),
        "1,2,3"
    );
    assert_eq!(
        run(
            "function* g() { try { yield 1; yield 2 } finally { console.log('done') } } for (const x of g()) { console.log(x); break }"
        ),
        "1\ndone"
    );
    assert_eq!(
        run(
            "function* g() { try { yield 1 } catch (e) { yield e + 1 } } const i = g(); i.next(); console.log(i.throw(5).value)"
        ),
        "6"
    );
    assert_eq!(
        run(
            "function* g() { yield 1 } const i = g(); console.log(i.return(7).value, i.next().done)"
        ),
        "7 true"
    );
    assert_eq!(
        run(
            "function* g() { try { yield 1; console.log('no') } catch (e) { console.log('no') } finally { console.log('f') } } const i = g(); i.next(); console.log(i.return(7).value)"
        ),
        "f\n7"
    );
    assert_eq!(
        run(
            "function* g(n) { let a = 0, b = 1; while (n-- > 0) { yield a; [a, b] = [b, a + b] } } console.log([...g(7)].join())"
        ),
        "0,1,1,2,3,5,8"
    );
    assert_eq!(
        run(
            "const o = { *g() { yield this.v }, v: 1 }; console.log(o.g().next().value); class C { *h() { yield 2 } } console.log(new C().h().next().value)"
        ),
        "1\n2"
    );
    assert_eq!(
        run("function* g() { yield 1 } const i = g(); console.log(i[Symbol.iterator]() === i)"),
        "true"
    );
    assert!(
        error_of("function* g() { i.next() } const i = g(); i.next()").contains("already running")
    );
}

#[test]
fn iterators_can_be_written_by_hand() {
    assert_eq!(
        run(
            "const it = { [Symbol.iterator]() { let n = 0; return { next: () => ({ value: n, done: n++ >= 2 }) } } }; console.log([...it].join()); for (const x of it) console.log(x)"
        ),
        "0,1\n0\n1"
    );
    assert_eq!(
        run(
            "let closed = 0; const it = { [Symbol.iterator]() { return { next: () => ({ value: 1 }), return() { closed++; return {} } } } }; for (const x of it) break; const [a] = it; console.log(closed)"
        ),
        "2"
    );
    assert_eq!(
        run(
            "const a = [1, 2]; const i = a[Symbol.iterator](); console.log(i.next().value, [...i].join())"
        ),
        "1 2"
    );
    assert_eq!(
        run(
            "console.log([...'a\\u{1F600}b'].length, [...new Map([[1, 2]]).keys()].join(), [...new Set([1, 1, 2]).values()].join(), [...[5, 6].entries()].join('|'), [...[5].keys()].join())"
        ),
        "3 1 1,2 0,5|1,6 0"
    );
    assert!(error_of("for (const x of 5) {}").contains("not iterable"));
    assert_eq!(
        run(
            "const a = [1, 2, 3]; const seen = []; for (const x of a) { seen.push(x); if (x == 1) a.push(4) } console.log(seen.join())"
        ),
        "1,2,3,4"
    );
    assert_eq!(
        run(
            "function f() { return [...arguments].join() } console.log(f(1, 2)); function g() { for (const x of arguments) console.log(x) } g(3)"
        ),
        "1,2\n3"
    );
}

#[test]
fn promises_and_async_run_in_order() {
    assert_eq!(
        run("console.log(1); Promise.resolve().then(() => console.log(3)); console.log(2)"),
        "1\n2\n3"
    );
    assert_eq!(
        run(
            "async function f() { console.log(1); const x = await 2; console.log(x); return 3 } f().then(console.log); console.log('a')"
        ),
        "1\na\n2\n3"
    );
    assert_eq!(
        run("async function f() { throw new Error('x') } f().catch(e => console.log(e.message))"),
        "x"
    );
    assert_eq!(
        run(
            "async function f() { try { await Promise.reject(1) } catch (e) { console.log(e) } finally { console.log('f') } } f()"
        ),
        "1\nf"
    );
    assert_eq!(
        run(
            "Promise.all([1, Promise.resolve(2), new Promise(r => r(3))]).then(v => console.log(v.join()))"
        ),
        "1,2,3"
    );
    assert_eq!(
        run("Promise.allSettled([1, Promise.reject(2)]).then(v => console.log(JSON.stringify(v)))"),
        "[{\"status\":\"fulfilled\",\"value\":1},{\"status\":\"rejected\",\"reason\":2}]"
    );
    assert_eq!(
        run(
            "Promise.race([new Promise(() => {}), Promise.resolve(1)]).then(console.log); Promise.any([Promise.reject(1), 2]).then(console.log)"
        ),
        "1\n2"
    );
    assert_eq!(
        run("Promise.any([Promise.reject(1)]).catch(e => console.log(e.name, e.errors.join()))"),
        "AggregateError 1"
    );
    assert_eq!(
        run(
            "Promise.resolve(1).finally(() => console.log('f')).then(console.log); Promise.reject(2).finally(() => {}).catch(console.log)"
        ),
        "f\n1\n2"
    );
    assert_eq!(
        run("Promise.resolve({ then(r) { r(5) } }).then(console.log)"),
        "5"
    );
    assert_eq!(
        run(
            "setTimeout(() => console.log(2), 5); setTimeout(console.log, 0, 1); Promise.resolve().then(() => console.log(0))"
        ),
        "0\n1\n2"
    );
    assert_eq!(
        run(
            "const t = setTimeout(() => console.log('no'), 0); clearTimeout(t); let n = 0; const i = setInterval(() => { if (++n == 2) { clearInterval(i); console.log(n) } }, 1)"
        ),
        "2"
    );
    assert_eq!(
        run("queueMicrotask(() => console.log(2)); console.log(1)"),
        "1\n2"
    );
    assert_eq!(
        run(
            "const f = async () => { await null; return 1 }; class C { async m() { return await f() + 1 } } new C().m().then(console.log)"
        ),
        "2"
    );
    assert_eq!(
        run(
            "async function a() { for (let i = 0; i < 3; i++) await i; return 'd' } a().then(console.log)"
        ),
        "d"
    );
    assert!(error_of("Promise.reject(new Error('lost'))").contains("lost"));
    assert_eq!(
        run(
            "const p = Promise.reject(1); setTimeout(() => {}, 0); p.catch(() => console.log('fine'))"
        ),
        "fine"
    );
}

#[test]
fn native_map_comparison_observes_mutation_and_same_value_without_user_code() {
    let mut engine = Engine::new();
    engine.eval(r#"
        const objectKey = {}, objectValue = {};
        globalThis.firstMap = new Map([[objectKey, objectValue], [NaN, 'nan'], [0, undefined]]);
        globalThis.sameMap = new Map([[0, undefined], ['deleted', 9], [NaN, 'nan'], [objectKey, objectValue]]);
        sameMap.delete('deleted');
        globalThis.missingMap = new Map([[objectKey, objectValue], [NaN, 'nan'], ['missing', undefined]]);
        globalThis.changedMap = new Map(firstMap); changedMap.set(objectKey, {});
        globalThis.nanOne = new Map([['value', NaN]]); globalThis.nanTwo = new Map(nanOne);
        globalThis.zeroOne = new Map([['value', -0]]); globalThis.zeroTwo = new Map([['value', 0]]);
        globalThis.notMap = new Set(firstMap.keys());
        firstMap.get = sameMap.get = () => { throw new Error('get must not run') };
        firstMap[Symbol.iterator] = sameMap[Symbol.iterator] = () => { throw new Error('iterator must not run') };
    "#, "native-map-comparison.js").unwrap();
    let vm = engine.vm();
    let first = vm.get_global("firstMap").unwrap();
    let same = vm.get_global("sameMap").unwrap();
    let missing = vm.get_global("missingMap").unwrap();
    let changed = vm.get_global("changedMap").unwrap();
    let nan_one = vm.get_global("nanOne").unwrap();
    let nan_two = vm.get_global("nanTwo").unwrap();
    let zero_one = vm.get_global("zeroOne").unwrap();
    let zero_two = vm.get_global("zeroTwo").unwrap();
    let not_map = vm.get_global("notMap").unwrap();
    assert!(vm.same_map_entries(first, same));
    assert!(!vm.same_map_entries(first, missing));
    assert!(!vm.same_map_entries(first, changed));
    assert!(vm.same_map_entries(nan_one, nan_two));
    assert!(!vm.same_map_entries(zero_one, zero_two));
    assert!(!vm.same_map_entries(first, not_map));
    assert!(!vm.same_map_entries(Value::UNDEFINED, Value::UNDEFINED));
    engine.eval("sameMap.set('late', 1)", "native-map-mutation.js").unwrap();
    assert!(!engine.vm().same_map_entries(first, same));
}

#[test]
fn map_copy_preserves_entries_and_observable_constructor_hooks() {
    assert_eq!(run(r#"
        const key = {}, value = {};
        const original = new Map([[key, value], ['gone', 1], [NaN, 2], [-0, 3]]);
        original.delete('gone'); original.set('last', 4);
        const copy = new Map(original);
        original.set(key, 'changed'); copy.set('copy-only', 5);
        console.log(copy.get(key) === value, copy.get(NaN), copy.get(0),
            [...copy.keys()].map(k => k === key ? 'object' : String(k)).join(), original.has('copy-only'));
        const it = original.entries(); it.next();
        const resumed = new Map({ [Symbol.iterator]() { return it } });
        console.log([...resumed.values()].join(), it.next().done);
        original[Symbol.iterator] = function* () { yield ['custom', 7] };
        console.log([...new Map(original)].join());
        const calls = [];
        class Custom extends Map { set(k, v) { calls.push(k); return super.set(k, v * 2) } }
        console.log([...new Custom(new Map([['a', 1], ['b', 2]])).values()].join(), calls.join());
    "#), "true 2 3 object,NaN,0,last,copy-only false\n2,3,4 true\ncustom,7\n2,4 a,b");
    assert_eq!(run(r#"
        const source = new Map([['a', 1]]), order = [];
        const set = Map.prototype.set;
        Object.defineProperty(Map.prototype, 'set', { configurable: true, get() {
            order.push('adder'); set.call(this, 'prepopulated', 2); return set;
        }});
        Object.defineProperty(source, Symbol.iterator, { get() {
            order.push('iterator'); set.call(source, 'after-getter', 3);
            return Map.prototype.entries;
        }});
        const copy = new Map(source);
        console.log(order.join(), [...copy].map(([k,v]) => k + ':' + v).join());
        delete Map.prototype.set;
        Map.prototype.set = set;
        let closed = false;
        class Throws extends Map { set() { throw new Error('stop') } }
        try { new Throws({ *[Symbol.iterator]() { try { yield ['a', 1] } finally { closed = true } } }) } catch {}
        console.log(closed);
    "#), "adder,iterator prepopulated:2,a:1,after-getter:3\ntrue");
}

#[test]
fn map_and_set_keep_order_and_find_by_value() {
    assert_eq!(
        run(
            "const m = new Map(); m.set('a', 1).set(1, 2).set(NaN, 3); const k = {}; m.set(k, 4); console.log(m.get('a'), m.get(1), m.get(NaN), m.get(k), m.get({}), m.size, m.has('a')); m.delete('a'); console.log(m.has('a'), [...m.keys()].length)"
        ),
        "1 2 3 4 undefined 4 true\nfalse 3"
    );
    assert_eq!(
        run(
            "const m = new Map(); m.set('a' + 'b', 1); console.log(m.get('ab'), m.get(['a', 'b'].join(''))); m.set(0, 'z'); console.log(m.get(-0), m.get(1 - 1))"
        ),
        "1 1\nz z"
    );
    assert_eq!(
        run(
            "const s = new Set([1, 2, 2, 3]); s.delete(2); s.add(2); console.log([...s].join(), s.size); s.clear(); console.log(s.size)"
        ),
        "1,3,2 3\n0"
    );
    assert_eq!(
        run(
            "const m = new Map([[1, 'a'], [2, 'b']]); m.forEach((v, k) => console.log(k, v)); for (const [k, v] of m) console.log(k + v)"
        ),
        "1 a\n2 b\n1a\n2b"
    );
    assert_eq!(
        run(
            "const w = new WeakMap(); const k = {}; w.set(k, 1); console.log(w.get(k), w.has({})); const ws = new WeakSet(); ws.add(k); console.log(ws.has(k))"
        ),
        "1 false\ntrue"
    );
    assert_eq!(
        run(
            "const m = new Map(); for (let i = 0; i < 1000; i++) m.set(i, i); for (let i = 0; i < 999; i++) m.delete(i); console.log(m.size, m.get(999), m.get(5))"
        ),
        "1 999 undefined"
    );
    assert_eq!(
        run(
            "const m = new Map(); for (let i = 0; i < 5; i++) m.set(i, i); let s = ''; for (const [k] of m) { s += k; if (k == 1) { m.delete(2); m.set(9, 9) } } console.log(s)"
        ),
        "01349"
    );
}

#[test]
fn json_goes_there_and_back() {
    assert_eq!(
        run(
            "console.log(JSON.stringify({ a: 1, b: [1, 'x', null, true], c: { d: undefined, e() {} } }))"
        ),
        "{\"a\":1,\"b\":[1,\"x\",null,true],\"c\":{}}"
    );
    assert_eq!(
        run("console.log(JSON.stringify({ a: [1, { b: 2 }], c: {} , d: []}, null, 2))"),
        "{\n  \"a\": [\n    1,\n    {\n      \"b\": 2\n    }\n  ],\n  \"c\": {},\n  \"d\": []\n}"
    );
    assert_eq!(
        run(
            "console.log(JSON.stringify('a\"b\\n'), JSON.stringify(undefined), JSON.stringify(NaN), JSON.stringify(new Date(0)), JSON.stringify([undefined]))"
        ),
        "\"a\\\"b\\n\" undefined null \"1970-01-01T00:00:00.000Z\" [null]"
    );
    assert_eq!(
        run(
            "console.log(JSON.stringify({ a: 1, b: 2 }, ['a']), JSON.stringify({ a: 1 }, (k, v) => typeof v == 'number' ? v + 1 : v), JSON.stringify({ toJSON() { return 5 } }))"
        ),
        "{\"a\":1} {\"a\":2} 5"
    );
    assert_eq!(
        run(
            "const o = JSON.parse('{\"a\":[1,2,{\"b\":null}],\"c\":\"x\\\\n\\\\u0041\",\"d\":-1.5e2,\"e\":true}'); console.log(o.a[1], o.a[2].b, o.c, o.d, o.e)"
        ),
        "2 null x\nA -150 true"
    );
    assert_eq!(
        run("console.log(JSON.parse('[1,2]', (k, v) => typeof v == 'number' ? v * 2 : v).join())"),
        "2,4"
    );
    for bad in ["{", "[1,]", "{a:1}", "tru", "01", "'a'", "1 2", ""] {
        assert!(
            error_of(&format!("JSON.parse({bad:?})")).contains("SyntaxError"),
            "{bad}"
        );
    }
    assert!(error_of("const a = {}; a.a = a; JSON.stringify(a)").contains("circular"));
}

#[test]
fn numbers_become_text_and_back() {
    assert_eq!(
        run(
            "console.log((255).toString(16), (0.5).toString(2), (1.005).toFixed(2), (2.5).toFixed(0), (1234.5678).toFixed(1), (0).toFixed(1))"
        ),
        "ff 0.1 1.00 3 1234.6 0.0"
    );
    assert_eq!(
        run(
            "console.log((1.5).toPrecision(3), (123456).toPrecision(2), (0.00001).toPrecision(1), (12).toExponential(1))"
        ),
        "1.50 1.2e+5 0.00001 1.2e+1"
    );
    assert_eq!(
        run(
            "console.log(parseInt('42px'), parseInt('ff', 16), parseInt('0x10'), parseInt('z'), parseFloat('1.5e3x'), parseFloat('.5'), Number('12'), Number(''), Number('x'), Number(null), Number(true), +'3')"
        ),
        "42 255 16 NaN 1500 0.5 12 0 NaN 0 1 3"
    );
    assert_eq!(
        run(
            "console.log(String(1), String(null), String(undefined), String(true), String(1e21), String(-0), `${[1, 2]}`, `${{}}`)"
        ),
        "1 null undefined true 1e+21 0 1,2 [object Object]"
    );
    assert_eq!(
        run(
            "console.log(Number.isInteger(1), Number.isInteger(1.5), Number.isSafeInteger(2 ** 53), Number.MAX_SAFE_INTEGER, isNaN('x'), Number.isNaN('x'))"
        ),
        "true false false 9007199254740991 true false"
    );
    assert_eq!(
        run(
            "console.log(Math.floor(-1.5), Math.ceil(1.1), Math.round(2.5), Math.round(-2.5), Math.trunc(-1.9), Math.abs(-1), Math.sign(-3), Math.max(), Math.min(1, 2), Math.hypot(3, 4), Math.imul(65536, 65536), Math.clz32(1))"
        ),
        "-2 2 3 -2 -1 1 -1 -Infinity 1 5 0 31"
    );
    assert_eq!(
        run("const r = Math.random(); console.log(r >= 0 && r < 1, Math.random() !== r)"),
        "true true"
    );
    assert_eq!(
        run(
            "console.log(1 == '1', null == undefined, null == 0, '' == 0, [] == '', [1] == 1, ({}) == '[object Object]', NaN == NaN, 1 === 1.0)"
        ),
        "true true false true true true true false true"
    );
    assert_eq!(
        run(
            "const o = { valueOf() { return 5 } }; const s = { toString() { return 'x' } }; console.log(o + 1, s + 1, o * 2, `${s}`, new Date(5) - 0)"
        ),
        "6 x1 10 x 5"
    );
    assert_eq!(
        run("const o = { [Symbol.toPrimitive](h) { return h } }; console.log(`${o}`, o + '', +o)"),
        "string default NaN"
    );
}

#[test]
fn strings_have_their_methods() {
    assert_eq!(
        run(
            "console.log('abc'.charAt(1), 'abc'[1], 'abc'.charCodeAt(1), 'abc'.at(-1), 'abc'.indexOf('c'), 'abcabc'.lastIndexOf('b'), 'abc'.includes('b'), 'abc'.startsWith('a'), 'abc'.endsWith('c'))"
        ),
        "b b 98 c 2 4 true true true"
    );
    assert_eq!(
        run(
            "console.log('abc'.slice(1), 'abc'.slice(-1), 'abcd'.substring(2, 1), 'abcd'.substr(1, 2), ' a '.trim() + '|', 'a'.padStart(3, 'xy'), 'a'.padEnd(2), 'ab'.repeat(2), 'aB'.toUpperCase(), 'aB'.toLowerCase())"
        ),
        "bc c b bc a| xya a  abab AB ab"
    );
    assert_eq!(
        run(
            "console.log('a,b'.split(',').join('|'), 'abc'.split('').join('|'), 'a'.split().length, 'a,b,c'.split(',', 2).join(), 'a1b2'.split(/\\d/).join('|'), 'a1b'.split(/(\\d)/).join('|'))"
        ),
        "a|b a|b|c 1 a,b a|b| a|1|b"
    );
    assert_eq!(
        run(
            "console.log('aXbX'.replace('X', '-'), 'aXbX'.replaceAll('X', '-'), 'aXbX'.replace(/X/g, '-'), 'ab'.replace(/(a)(b)/, '$2$1$$'), 'abc'.replace('b', '[$&$`$\\']'), 'a'.replace('a', m => m + m))"
        ),
        "a-bX a-b- a-b- ba$ a[bac]c aa"
    );
    assert_eq!(
        run(
            "console.log('x1y22'.match(/\\d+/)[0], 'x1y22'.match(/\\d+/).index, 'x1y22'.match(/\\d+/g).join(), 'x'.match(/\\d/), 'x1'.search(/\\d/), [...'a1b2'.matchAll(/\\d/g)].map(m => m[0] + m.index).join())"
        ),
        "1 1 1,22 null 1 11,23"
    );
    assert_eq!(
        run(
            "console.log(String.fromCharCode(65, 66), String.fromCodePoint(0x1F600).length, '\\u{1F600}'.codePointAt(0), 'a'.localeCompare('b'), 'a'.localeCompare('a'), String.raw`a\\n${1}`)"
        ),
        "AB 2 128512 -1 0 a\\n1"
    );
    assert_eq!(
        run(
            "const t = (s, ...v) => s.raw.join('|') + v.join(); console.log(t`a${1}b${2}c`); const f = s => s; console.log(f`x` === f`x`); const g = () => f`y`; console.log(g() === g())"
        ),
        "a|b|c1,2\nfalse\ntrue"
    );
    assert_eq!(
        run(
            "console.log(encodeURIComponent('a b&c/\\u00e9'), decodeURIComponent('a%20b%C3%A9'), encodeURI('a b/c'), decodeURI('a%2Fb%20c'))"
        ),
        "a%20b%26c%2F%C3%A9 a b\u{e9} a%20b/c a%2Fb c"
    );
    assert!(error_of("decodeURIComponent('%')").contains("URI malformed"));
}

#[test]
fn regular_expressions_work() {
    assert_eq!(
        run(
            "const r = /(\\d+)-(?<n>\\d+)/; const m = r.exec('a 12-34'); console.log(m[0], m[1], m.groups.n, m.index, r.source, r.flags, String(r), r.global)"
        ),
        "12-34 12 34 2 (\\d+)-(?<n>\\d+)  /(\\d+)-(?<n>\\d+)/ false"
    );
    assert_eq!(
        run(
            "const r = /a/g; console.log(r.test('aa'), r.lastIndex, r.test('aa'), r.lastIndex, r.test('aa'), r.lastIndex)"
        ),
        "true 1 true 2 false 0"
    );
    assert_eq!(
        run(
            "const r = /a/y; console.log(r.test('ba')); r.lastIndex = 1; console.log(r.test('ba'))"
        ),
        "false\ntrue"
    );
    assert_eq!(
        run(
            "console.log(new RegExp('a+', 'i').test('AA'), new RegExp(/x/g).flags, /[/]/.test('/'), /a|b/.test('b'), /^$/.test(''))"
        ),
        "true g true true true"
    );
    assert_eq!(
        run(
            "console.log('2020-01-02'.replace(/(?<y>\\d+)-(?<m>\\d+)-(?<d>\\d+)/, '$<d>/$<m>/$<y>'), 'abc'.replace(/b/, (m, i, s) => m + i + s))"
        ),
        "02/01/2020 ab1abcc"
    );
    assert!(error_of("new RegExp('(')").contains("Invalid regular expression"));
    assert!(error_of("new RegExp('a', 'qq')").contains("Invalid flags"));
}

#[test]
fn objects_have_their_methods() {
    assert_eq!(
        run(
            "const o = { a: 1, b: 2 }; console.log(Object.values(o).join(), Object.entries(o).join('|'), JSON.stringify(Object.assign({}, o, { c: 3 })), JSON.stringify(Object.fromEntries([['x', 1]])))"
        ),
        "1,2 a,1|b,2 {\"a\":1,\"b\":2,\"c\":3} {\"x\":1}"
    );
    assert_eq!(
        run(
            "const o = {}; Object.defineProperty(o, 'a', { value: 1 }); console.log(o.a, Object.keys(o).length, JSON.stringify(Object.getOwnPropertyDescriptor(o, 'a'))); Object.defineProperty(o, 'g', { get() { return 2 }, enumerable: true }); console.log(o.g, Object.keys(o).join())"
        ),
        "1 0 {\"value\":1,\"writable\":false,\"enumerable\":false,\"configurable\":false}\n2 g"
    );
    assert!(
        error_of(
            "'use strict'; const o = {}; Object.defineProperty(o, 'a', { value: 1 }); o.a = 2"
        )
        .contains("read only")
    );
    assert!(error_of("const o = {}; Object.defineProperty(o, 'a', { value: 1 }); Object.defineProperty(o, 'a', { value: 2 })").contains("redefine"));
    assert_eq!(
        run(
            "const p = {}; const o = Object.create(p); console.log(Object.getPrototypeOf(o) === p, p.isPrototypeOf(o), o.__proto__ === p, ({ __proto__: p }).__proto__ === p, Object.getPrototypeOf(Object.create(null)))"
        ),
        "true true true true null"
    );
    assert_eq!(
        run(
            "console.log({}.toString(), [].toString(), Object.prototype.toString.call([]), Object.prototype.toString.call(null), ({}).hasOwnProperty('x'), Object.hasOwn({ x: 1 }, 'x'), Object.is(NaN, NaN), Object.is(0, -0))"
        ),
        "[object Object]  [object Array] [object Null] false true true false"
    );
    assert_eq!(
        run(
            "const s = Symbol('d'); const o = { [s]: 1, a: 2 }; console.log(o[s], s.description, s.toString(), Object.keys(o).length, Object.getOwnPropertySymbols(o).length, typeof s, Symbol.for('x') === Symbol.for('x'), Symbol.keyFor(Symbol.for('x')))"
        ),
        "1 d Symbol(d) 1 1 symbol true x"
    );
    assert_eq!(
        run(
            "console.log(typeof 1, typeof 'a', typeof true, typeof undefined, typeof null, typeof {}, typeof [], typeof (() => 1), typeof class {})"
        ),
        "number string boolean undefined object object object function function"
    );
    assert_eq!(
        run(
            "console.log(Object.getOwnPropertyNames(function f(a) {}).sort().join(), Object.getOwnPropertyNames(() => 1).sort().join(), Object.getOwnPropertyNames([1]).join(), Object.getOwnPropertyNames('ab').join())"
        ),
        "length,name,prototype length,name 0,length 0,1,length"
    );
    assert_eq!(
        run(
            "function F() {} console.log(F.prototype.constructor === F, new F() instanceof F, Object.keys(F.prototype).length, F.hasOwnProperty('prototype'), (() => 1).prototype)"
        ),
        "true true 0 true undefined"
    );
    assert_eq!(
        run(
            "const g = Object.groupBy([1, 2, 3], x => x % 2 ? 'odd' : 'even'); console.log(g.odd.join(), g.even.join())"
        ),
        "1,3 2"
    );
    assert_eq!(
        run(
            "console.log(Reflect.ownKeys({ a: 1 }).join(), Reflect.has({ a: 1 }, 'a'), Reflect.get({ a: 1 }, 'a'), Reflect.apply(Math.max, null, [1, 2]), new (Reflect.construct(function () { this.v = 1 }, []).constructor)().v)"
        ),
        "a true 1 2 1"
    );
    assert_eq!(
        run(
            "const f = function (a, b) { return [this, a, b].join() }.bind(1, 2); console.log(f(3), f.name, f.length); class A { constructor(x) { this.x = x } } const B = A.bind(null, 5); console.log(new B().x)"
        ),
        "1,2,3 bound  1\n5"
    );
    assert_eq!(
        run("console.log(new Function('a', 'b', 'return a + b')(1, 2))"),
        "3"
    );
}

#[test]
fn dates_work_in_utc() {
    assert_eq!(
        run(
            "const d = new Date(Date.UTC(2020, 1, 29, 12, 30, 15, 500)); console.log(d.toISOString(), d.getTime(), d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate(), d.getUTCDay(), d.getUTCHours())"
        ),
        "2020-02-29T12:30:15.500Z 1582979415500 2020 1 29 6 12"
    );
    assert_eq!(
        run(
            "console.log(new Date('2000-01-01T00:00:00Z').getTime(), Date.parse('1970-01-02'), new Date('nope').getTime(), JSON.stringify(new Date(NaN)), typeof Date.now())"
        ),
        "946684800000 86400000 NaN null number"
    );
    assert_eq!(
        run(
            "const d = new Date(0); d.setUTCFullYear(2001, 0, 2); d.setUTCHours(3); console.log(d.toISOString())"
        ),
        "2001-01-02T03:00:00.000Z"
    );
}

#[test]
fn console_log_shows_things_as_node_does() {
    assert_eq!(
        run("console.log([1, 'a', null, undefined, true, -0])"),
        "[ 1, 'a', null, undefined, true, -0 ]"
    );
    assert_eq!(
        run("console.log({ a: 1, 'b c': 'x', d: [1], e: { f: {} } })"),
        "{ a: 1, 'b c': 'x', d: [ 1 ], e: { f: {} } }"
    );
    assert_eq!(
        run("console.log({ a: { b: { c: { d: 1 } } } }, [[[[1]]]])"),
        "{ a: { b: { c: [Object] } } } [ [ [ [Array] ] ] ]"
    );
    assert_eq!(
        run("console.log(function f() {}, () => 1, class A {}, Math.max, Symbol('s'))"),
        "[Function: f] [Function (anonymous)] [class A] [Function: max] Symbol(s)"
    );
    assert_eq!(
        run(
            "console.log(new Map([[1, 'a']]), new Set([1]), [, 1], new (class Foo { x = 1 })(), Object.create(null), [])"
        ),
        "Map(1) { 1 => 'a' } Set(1) { 1 } [ <1 empty item>, 1 ] Foo { x: 1 } [Object: null prototype] {} []"
    );
    assert_eq!(
        run("const a = { n: 1 }; a.self = a; console.log(a)"),
        "{ n: 1, self: [Circular *1] }"
    );
    assert_eq!(
        run(
            "console.log(Promise.resolve(1), new Date(0), /a/g, new Number(1), { get a() { return 1 } })"
        ),
        "Promise { 1 } 1970-01-01T00:00:00.000Z /a/g [Number: 1] { a: [Getter] }"
    );
}

#[test]
fn declarations_of_a_whole_block_are_only_types() {
    assert_eq!(
        run(
            "declare global { interface String { shout(): string; } interface Window { a: { b: () => void } } }
             declare module 'thing' { export function f(): void; export const g: number; }
             declare module 'bare';
             declare namespace Space.Inner { const x: number; }
             String.prototype.shout = function () { return this.toUpperCase() + '!' };
             const global = 'still a name', module2 = 1;
             console.log('hi'.shout(), global, module2)"
        ),
        "HI! still a name 1"
    );
}

#[test]
fn typescript_runs_with_its_types_gone() {
    assert_eq!(
        run(
            "let a: number = 1; const b: Array<string> = []; function f<T>(x: T, y?: number): T { return x } console.log(a, b.length, f<number>(2))"
        ),
        "1 0 2"
    );
    assert_eq!(
        run(
            "interface I { a: string } type T = { b: number } | string; const v = 1 as unknown as number; const w = (v as any)!; console.log(v, w, v satisfies number)"
        ),
        "1 1 1"
    );
    assert_eq!(
        run("enum E { A, B = 5, C } enum S { X = 'x' } console.log(E.A, E.B, E.C, E[5], S.X)"),
        "0 5 6 B x"
    );
    assert_eq!(
        run(
            "class P<T> implements Q { private x: number = 1; readonly y?: string; static z: number = 2; constructor(public a: T, private readonly b = 3) {} get(): number { return this.x + this.b } declare d: string; abstract q(): void; [k: string]: any } const p = new P<string>('s'); console.log(p.a, p.get(), P.z, 'd' in p)"
        ),
        "s 4 2 false"
    );
    assert_eq!(
        run(
            "class B { constructor(public v: number) {} } class D extends B { constructor(public w: number) { super(w + 1) } } const d = new D(1); console.log(d.v, d.w)"
        ),
        "2 1"
    );
    assert_eq!(
        run(
            "function o(a: string): void; function o(a: number): void; function o(a: any) { console.log(a) } o(1); const g = <T,>(x: T): T => x; console.log(g(2)); const h = async <T,>(x: T) => x; h(3).then(console.log)"
        ),
        "1\n2\n3"
    );
    assert_eq!(
        run(
            "declare const x: number; declare function y(): void; export {}; let z: (a: number) => void = (a) => console.log(a); z(1); const t: [number, string] = [1, 'a']; console.log(t[1]); const c = { a: 1 } as const; console.log(c.a)"
        ),
        "1\na\n1"
    );
    assert_eq!(
        run(
            "const m = new Map<string, number>(); m.set('a', 1); console.log(m.get('a')!); const arr = [1, 2] as number[]; for (const n of arr as number[]) console.log(n); console.log(1 < 2, 2 > 1, (1 as number) < 2)"
        ),
        "1\n1\n2\ntrue true true"
    );
    assert_eq!(
        run(
            "type A<T> = T extends string ? 'a' : T extends number ? `b${T}` : never; const f = (x: number): x is number => true; console.log(f(1)); function g(this: any, a: number) { return a } console.log(g(2))"
        ),
        "true\n2"
    );
}

#[test]
fn typescript_class_modifiers_are_read_wherever_they_go() {
    // `export default abstract class`, named or not.
    assert_eq!(
        run_as_file(
            "abstract_default",
            "export default abstract class Base { abstract x(): number; y() { return this.x() + 1 } } console.log(new (class extends Base { x() { return 1 } })().y())"
        ),
        "2"
    );
    assert_eq!(
        run_as_file(
            "abstract_anonymous",
            "export default abstract class { static n = 3 } console.log('read')"
        ),
        "read"
    );
    // A name that's only `abstract`, on a line of its own, is still a name.
    assert_eq!(
        run_as_file(
            "abstract_name",
            "const abstract = 4; export default abstract\nconsole.log(abstract)"
        ),
        "4"
    );
    assert_eq!(
        run_as_file(
            "abstract_export",
            "export abstract class A { abstract f(): void; g() { return 'g' } } export declare abstract class B {} console.log(class extends A { f() {} }.prototype.g())"
        ),
        "g"
    );
    // Abstract and declared members are only types. `override`, and parameter properties with every modifier, are
    // what they'd be without them.
    assert_eq!(
        run(
            "abstract class Shape { abstract readonly sides: number; protected abstract area(): number; abstract get label(): string; describe() { return `${this.sides}:${this.area()}` } }
            class Square extends Shape {
                override readonly sides = 4;
                declare readonly extra: string;
                declare static s: number;
                constructor(private readonly size: number, public override name: string = 'sq', protected tag?: string) { super() }
                protected override area(): number { return this.size * this.size }
                override get label() { return 'L' + this.size }
            }
            const s = new Square(3);
            console.log(s.describe(), s.label, Object.keys(s).join(), 'extra' in s, 's' in Square)"
        ),
        "4:9 L3 sides,size,name,tag false false"
    );
}

#[test]
fn an_export_default_without_a_name_ends_where_its_body_does() {
    assert_eq!(
        run_as_file(
            "default_function",
            "export default function () { return 1 } console.log('after function')"
        ),
        "after function"
    );
    assert_eq!(
        run_as_file(
            "default_async",
            "export default async function () {} (1, console.log('after async'))"
        ),
        "after async"
    );
    assert_eq!(
        run_as_file(
            "default_call",
            "export default (function () { console.log('called') })()"
        ),
        "called"
    );
}

#[test]
fn accessor_fields_are_a_getter_and_a_setter_over_a_value_of_their_own() {
    assert_eq!(
        run(
            "let made = 0;
            class A {
                accessor x = ++made;
                static accessor y: number = 10;
                accessor #z = 'z';
                accessor 'quoted' = 'q';
                get z() { return this.#z }
                set z(value) { this.#z = value }
            }
            const a = new A();
            const b = new A();
            a.x = 7;
            a.z = 'Z';
            const described = Object.getOwnPropertyDescriptor(A.prototype, 'x');
            console.log(a.x, b.x, A.y, a.z, b.z, a.quoted, Object.keys(a).length, typeof described.get, typeof described.set);
            A.y = 11;
            console.log(A.y, Object.getOwnPropertyNames(A).includes('y'), Object.hasOwn(a, 'x'));"
        ),
        "7 2 10 Z z q 0 function function\n11 true false"
    );
    // `accessor` on its own, or before a new line, is a field called that.
    assert_eq!(
        run(
            "class B { accessor; declare = 1; abstract = 2; accessor\n other = 3 } console.log(Object.keys(new B()).join())"
        ),
        "accessor,declare,abstract,other"
    );
}

#[test]
fn jsx_becomes_calls() {
    let prelude = "function __jsx(t, p, c) { return { t: typeof t == 'function' ? t.name : t, p, c: c ?? [] } } const __Fragment = 'F';";
    let show = |code: &str| run(&format!("{prelude} console.log(JSON.stringify({code}))"));
    assert_eq!(show("<div />"), "{\"t\":\"div\",\"p\":null,\"c\":[]}");
    assert_eq!(
        show("<a b=\"c\" d={1} e>x {2} <i>y</i></a>"),
        "{\"t\":\"a\",\"p\":{\"b\":\"c\",\"d\":1,\"e\":true},\"c\":[\"x \",2,\" \",{\"t\":\"i\",\"p\":null,\"c\":[\"y\"]}]}"
    );
    assert_eq!(
        show("<><b />{[1].map(n => <i>{n}</i>)}</>"),
        "{\"t\":\"F\",\"p\":null,\"c\":[{\"t\":\"b\",\"p\":null,\"c\":[]},[{\"t\":\"i\",\"p\":null,\"c\":[1]}]]}"
    );
    assert_eq!(
        run(&format!(
            "{prelude} const Foo = () => 1; const n = {{ Bar: () => 2 }}; console.log(<Foo x={{1}} />.t, <n.Bar />.t, <a-b />.t, <a {{...{{ z: 1 }}}} y={{2}} />.p.z)"
        )),
        "Foo Bar a-b 1"
    );
    assert_eq!(
        show("<a>\n   line one\n   line two\n</a>"),
        "{\"t\":\"a\",\"p\":null,\"c\":[\"line one line two\"]}"
    );
    assert_eq!(
        show("1 < 2 ? <a>{'>'}</a> : null"),
        "{\"t\":\"a\",\"p\":null,\"c\":[\">\"]}"
    );
}

#[test]
fn with_finds_names_on_its_object() {
    assert_eq!(
        run(
            "const o = { a: 1 }; let b = 2; with (o) { console.log(a, b); a = 3; b = 4 } console.log(o.a, b)"
        ),
        "1 2\n3 4"
    );
    assert_eq!(
        run(
            "with ({ x: 1 }) { const f = () => x; console.log(f()); function g() { return x } console.log(g()) }"
        ),
        "1\n1"
    );
    assert_eq!(
        run(
            "with ({ a: 1 }) { with ({ b: 2 }) { console.log(a + b) } { let a = 5; console.log(a) } }"
        ),
        "3\n5"
    );
    assert_eq!(
        run("let i = 0; with ({ i: 5 }) { i++ } console.log(i)"),
        "0"
    );
    assert_eq!(
        run("with ({ x: 1 }) { console.log(typeof x, typeof y) }"),
        "number undefined"
    );
}

#[test]
fn a_scripts_top_level_is_the_global_scope() {
    assert_eq!(
        run(
            "var a = 1; function f() { return 2 } console.log(globalThis.a, globalThis.f(), this === globalThis); console.log(Object.getOwnPropertyDescriptor(globalThis, 'a').configurable)"
        ),
        "1 2 true\nfalse"
    );
    assert_eq!(
        run("console.log(typeof f, a); var a = 1; function f() {}"),
        "function undefined"
    );
    assert_eq!(
        run("let l = 1; const fn = () => l + 1; console.log(fn(), globalThis.l)"),
        "2 undefined"
    );
    assert_eq!(
        run("var a = 1; { function inBlock() { return a } console.log(inBlock()) }"),
        "1"
    );
    let mut engine = Engine::new();
    engine
        .eval("var shared = 5; function f() { return shared + 1 }", "a.js")
        .unwrap();
    let result = engine.eval("f()", "b.js").unwrap();
    assert_eq!(engine.display(result), "6");
    assert!(error_of("let a; let a").contains("already been declared"));
    assert!(error_of("let a; var a").contains("already been declared"));
    assert!(error_of("{ let a; { var a } }").contains("already been declared"));
    assert!(error_of("let a; function a() {}").contains("already been declared"));
    assert!(error_of("try {} catch (e) { let e }").contains("already been declared"));
    assert_eq!(
        run("var a; var a; function b() {} function b() {} { function c() {} function c() {} }"),
        ""
    );
}

#[test]
fn code_that_is_not_strict_carries_on_past_refused_writes() {
    assert_eq!(
        run(
            "const o = Object.freeze({ a: 1 }); o.a = 2; o.b = 3; delete o.a; console.log(o.a, o.b)"
        ),
        "1 undefined"
    );
    assert_eq!(
        run(
            "const a = Object.freeze([1]); a[0] = 2; a[1] = 3; console.log(a[0], a.length, delete a[0])"
        ),
        "1 1 false"
    );
    assert_eq!(
        run(
            "const g = { get x() { return 1 } }; g.x = 2; console.log(g.x); (1).y = 2; 'a'.z = 1; console.log('ok')"
        ),
        "1\nok"
    );
    assert_eq!(
        run(
            "console.log(delete Object.prototype); undefined = 1; console.log(typeof undefined); NaN = 1; console.log(NaN !== NaN)"
        ),
        "false\nundefined\ntrue"
    );
    assert_eq!(
        run(
            "const o = { set s(v) { throw new TypeError('mine') } }; try { o.s = 1 } catch (e) { console.log(e.message) }"
        ),
        "mine"
    );
    assert!(error_of("'use strict'; delete Object.prototype").contains("Cannot delete"));
    assert!(error_of("'use strict'; (1).y = 2").contains("Cannot create"));
    assert!(
        error_of("function f() { 'use strict'; Object.freeze({}).a = 1 } f()")
            .contains("not extensible")
    );
    // A built-in always fails out loud, whoever called it.
    assert!(error_of("[].push.call(Object.freeze([]), 1)").contains("frozen"));
}

#[test]
fn elements_of_an_array_can_be_more_than_plain_values() {
    assert_eq!(
        run(
            "const a = [1, 2, 3]; Object.defineProperty(a, 1, { get() { return 'g' } }); console.log(a[1], a.join(), a.length, JSON.stringify(Object.getOwnPropertyDescriptor(a, '1').set === undefined), Object.keys(a).join())"
        ),
        "g 1,g,3 3 true 0,1,2"
    );
    assert_eq!(
        run(
            "const a = [1]; let s; Object.defineProperty(a, 3, { set(v) { s = v }, configurable: true }); a[3] = 9; console.log(s, a.length, 2 in a, 3 in a, a[3]); delete a[3]; console.log(3 in a); a[3] = 5; console.log(a[3])"
        ),
        "9 4 false true undefined\nfalse\n5"
    );
    assert_eq!(
        run(
            "'use strict'; const a = [1, 2]; Object.defineProperty(a, 0, { value: 7, writable: false }); console.log(a[0], JSON.stringify(Object.getOwnPropertyDescriptor(a, 0))); try { a[0] = 8 } catch (e) { console.log(e.name) } console.log(a[0]); Object.defineProperty(a, 1, { value: 5 }); console.log(a[1], JSON.stringify(Object.getOwnPropertyDescriptor(a, 1)))"
        ),
        "7 {\"value\":7,\"writable\":false,\"enumerable\":true,\"configurable\":true}\nTypeError\n7\n5 {\"value\":5,\"writable\":true,\"enumerable\":true,\"configurable\":true}"
    );
    assert_eq!(
        run(
            "const a = [1, 2, 3]; Object.defineProperty(a, 1, { value: 2, configurable: false }); try { a.length = 0 } catch (e) { console.log(e.name) } console.log(a.length)"
        ),
        "2"
    );
    assert_eq!(
        run(
            "'use strict'; const a = [1]; Object.defineProperty(a, 'length', { writable: false }); try { a.push(2) } catch (e) { console.log(e.name) } try { a[5] = 1 } catch (e) { console.log(e.name) } console.log(a.length, JSON.stringify(Object.getOwnPropertyDescriptor(a, 'length')))"
        ),
        "TypeError\nTypeError\n1 {\"value\":1,\"writable\":false,\"enumerable\":false,\"configurable\":false}"
    );
    assert_eq!(
        run(
            "Array.prototype[1] = 'p'; const a = [0, , 2]; console.log(a[1], 1 in a, a.join(), [, ,].length); const seen = []; a.forEach(x => seen.push(x)); console.log(seen.join()); delete Array.prototype[1]; console.log(a[1])"
        ),
        "p true 0,p,2 2\n0,p,2\nundefined"
    );
    assert_eq!(
        run(
            "let s; Object.defineProperty(Array.prototype, 2, { set(v) { s = v }, configurable: true }); const a = []; a[2] = 1; console.log(s, a.length); delete Array.prototype[2]"
        ),
        "1 0"
    );
}

#[test]
fn array_methods_work_on_anything_array_like() {
    assert_eq!(
        run(
            "const o = { length: 3, 0: 'a', 2: 'c' }; const seen = []; [].forEach.call(o, x => seen.push(x)); console.log(seen.join(), [].map.call(o, x => x + x).join('-'), [].indexOf.call(o, 'c'), [].includes.call(o, undefined), [].indexOf.call(o, undefined), [].slice.call(o, 1).length, [].join.call(o, '+'))"
        ),
        "a,c aa--cc 2 true -1 2 a++c"
    );
    assert_eq!(
        run(
            "const huge = { length: 2 ** 53 - 1, [2 ** 53 - 2]: 'end' }; console.log([].lastIndexOf.call(huge, 'end'), [].indexOf.call(huge, 'end', -1), [].includes.call(huge, 'end', -1), [].at.call(huge, -1))"
        ),
        "9007199254740990 9007199254740990 true end"
    );
    assert_eq!(
        run(
            "const o = { length: 2 ** 53 - 1 }; for (const f of ['map', 'filter', 'slice', 'splice', 'concat']) { try { if (f !== 'filter' && f !== 'concat') [][f].call(o, x => x); console.log(f, 'fine') } catch (e) { console.log(f, e.name) } }"
        ),
        "map RangeError\nfilter fine\nslice RangeError\nsplice RangeError\nconcat fine"
    );
    assert_eq!(
        run(
            "const o = { length: 2, 0: 'x', 1: 'y' }; [].reverse.call(o); console.log(o[0], o[1]); [].unshift.call(o, 'w'); console.log(o.length, o[0], o[2]); console.log([].shift.call(o), o.length); console.log([].pop.call(o), o.length); console.log([].push.call(o, 'z'), o[1]); console.log([].splice.call(o, 0, 1), o.length, o[0])"
        ),
        "y x\n3 w x\nw 2\nx 1\n2 z\n[ 'y' ] 1 z"
    );
}

#[test]
fn iterators_are_closed_when_something_goes_wrong() {
    let closes = "let closed = 0; const it = { [Symbol.iterator]() { return this }, next() { return { value: [1, 2], done: false } }, return() { closed++; return {} } };";
    assert_eq!(
        run(&format!(
            "{closes} const set = Map.prototype.set; Map.prototype.set = () => {{ throw new Error('x') }}; try {{ new Map(it) }} catch (e) {{ console.log(e.message, closed) }} Map.prototype.set = set"
        )),
        "x 1"
    );
    assert_eq!(
        run(&format!(
            "{closes} const add = Set.prototype.add; Set.prototype.add = () => {{ throw new Error('x') }}; try {{ new Set(it) }} catch (e) {{ console.log(e.message, closed) }} Set.prototype.add = add"
        )),
        "x 1"
    );
    assert_eq!(
        run(&format!(
            "{closes} try {{ Array.from(it, () => {{ throw new Error('x') }}) }} catch (e) {{ console.log(e.message, closed) }}"
        )),
        "x 1"
    );
    assert_eq!(
        run(&format!(
            "{closes} const n = {{ [Symbol.iterator]() {{ return this }}, next() {{ return {{ value: 1, done: false }} }}, return() {{ closed++; return {{}} }} }}; try {{ new Map(n) }} catch (e) {{ console.log(e.name, closed) }}"
        )),
        "TypeError 1"
    );
    assert_eq!(
        run(&format!(
            "{closes} try {{ Object.fromEntries({{ [Symbol.iterator]() {{ return this }}, next() {{ return {{ value: 1, done: false }} }}, return() {{ closed++; return {{}} }} }}) }} catch (e) {{ console.log(e.name, closed) }}"
        )),
        "TypeError 1"
    );
    assert_eq!(
        run(&format!(
            "{closes} const r = Promise.resolve; Promise.resolve = () => {{ throw new Error('x') }}; Promise.all(it).catch(e => console.log(e.message, closed)); Promise.resolve = r"
        )),
        "x 1"
    );
    assert_eq!(
        run(&format!(
            "{closes} const t = Promise.prototype.then; Promise.prototype.then = () => {{ throw new Error('x') }}; const p = Promise.race(it); Promise.prototype.then = t; p.catch(e => console.log(e.message, closed))"
        )),
        "x 1"
    );
    assert_eq!(
        run(
            "Promise.all(5).catch(e => console.log(e.name)); Promise.race().catch(e => console.log(e.name)); Promise.allSettled(null).catch(e => console.log(e.name)); Promise.any(1).catch(e => console.log(e.name))"
        ),
        "TypeError\nTypeError\nTypeError\nTypeError"
    );
}

#[test]
fn prototypes_stay_sensible() {
    assert_eq!(
        run(
            "const a = {}; const b = Object.create(a); try { Object.setPrototypeOf(a, b) } catch (e) { console.log(e.name) } console.log(Reflect.setPrototypeOf(a, b), Reflect.setPrototypeOf(Object.prototype, {}), Reflect.setPrototypeOf(Object.prototype, null), Reflect.setPrototypeOf(Object.preventExtensions({}), {}), Reflect.setPrototypeOf(a, Object.prototype))"
        ),
        "TypeError\nfalse false true false true"
    );
    assert!(error_of("Object.prototype.__proto__ = {}").contains("TypeError"));
    assert!(error_of("Object.setPrototypeOf(null, {})").contains("TypeError"));
    assert!(
        error_of("Reflect.construct(function () {}, [], () => {})").contains("not a constructor")
    );
    assert!(error_of("Reflect.construct(() => {}, [])").contains("not a constructor"));
    assert!(error_of("new Math.max()").contains("not a constructor"));
    assert_eq!(
        run(
            "function isConstructor(f) { try { Reflect.construct(function () {}, [], f) } catch { return false } return true } console.log(isConstructor(Array.prototype.map), isConstructor(Math.max), isConstructor(Object), isConstructor(class {}), isConstructor(() => 1), isConstructor(function () {}), isConstructor({ m() {} }.m), isConstructor(function* () {}), isConstructor(async function () {}))"
        ),
        "false false true true false true false false false"
    );
}

#[test]
fn generator_and_async_functions_have_prototypes_of_their_own() {
    assert_eq!(
        run(
            "function* g() { yield 1 } const G = Object.getPrototypeOf(g); console.log(G.constructor.name, G.prototype === Object.getPrototypeOf(g.prototype), g.prototype.constructor === G, Object.getPrototypeOf(g()) === g.prototype, String(g()), G[Symbol.toStringTag]); g.prototype.extra = () => 'e'; console.log(g().extra()); console.log(new G.constructor('a', 'yield a')(5).next().value)"
        ),
        "GeneratorFunction true true true [object Generator] GeneratorFunction\ne\n5"
    );
    assert_eq!(
        run(
            "async function f() {} const A = Object.getPrototypeOf(f); console.log(A.constructor.name, A[Symbol.toStringTag], f.prototype, f.hasOwnProperty('prototype')); new A.constructor('return 1')().then(console.log)"
        ),
        "AsyncFunction AsyncFunction undefined false\n1"
    );
    assert_eq!(
        run(
            "async function* a() { yield 1 } const AG = Object.getPrototypeOf(a); console.log(AG.constructor.name, AG[Symbol.toStringTag], String(a()), Object.getPrototypeOf(a()) === a.prototype, a.prototype.constructor === AG); new AG.constructor('yield 2')().next().then(r => console.log(r.value))"
        ),
        "AsyncGeneratorFunction AsyncGeneratorFunction [object AsyncGenerator] true true\n2"
    );
    assert!(error_of("new (function* () {})()").contains("not a constructor"));
    assert!(error_of("new (async function () {})()").contains("not a constructor"));
}

#[test]
fn newer_library_features_are_there() {
    assert_eq!(
        run(
            "console.log([1, 2, 3].values().map(x => x * 2).filter(x => x > 2).toArray().join(), [1, 2, 3, 4].values().drop(1).take(2).toArray().join(), [1, 2].values().flatMap(x => [x, x]).toArray().join(), [1, 2].values().reduce((a, b) => a + b), [1, 2].values().some(x => x > 1), [1, 2].values().every(x => x > 1), [1, 2].values().find(x => x > 1))"
        ),
        "4,6 2,3 1,1,2,2 3 true false 2"
    );
    assert_eq!(
        run(
            "function* g() { yield 1; yield 2 } console.log(g().map(x => x + 1).toArray().join(), Iterator.from(g()).toArray().length, Iterator.from({ next() { return { done: true } } }).toArray().length, typeof Iterator, new (class extends Iterator {})() instanceof Iterator, g() instanceof Iterator, Object.prototype.toString.call([].values().map(x => x)))"
        ),
        "2,3 2 0 function true true [object Iterator Helper]"
    );
    assert!(error_of("new Iterator()").contains("TypeError"));
    assert_eq!(
        run(
            "const a = new Set([1, 2, 3]), b = new Set([2, 3, 4]); console.log([...a.union(b)].join(), [...a.intersection(b)].join(), [...a.difference(b)].join(), [...a.symmetricDifference(b)].join(), a.isSubsetOf(new Set([1, 2, 3, 4])), a.isSupersetOf(new Set([1])), a.isDisjointFrom(new Set([9])))"
        ),
        "1,2,3,4 2,3 1 1,4 true true true"
    );
    assert_eq!(
        run(
            "Array.fromAsync([1, Promise.resolve(2)]).then(r => console.log(r.join())); Promise.try(() => 5).then(console.log); console.log(Error.isError(new TypeError()), Error.isError({}), RegExp.escape('a.b'), Math.f16round(1.337))"
        ),
        "true false \\x61\\.b 1.3369140625\n5\n1,2"
    );
    assert_eq!(
        run(
            "const w = new WeakRef({ a: 1 }); console.log(w.deref().a, Object.prototype.toString.call(w)); const r = new FinalizationRegistry(() => {}); console.log(r.register({}, 1), r.unregister({}), Object.prototype.toString.call(r))"
        ),
        "1 [object WeakRef]\nundefined false [object FinalizationRegistry]"
    );
    assert_eq!(
        run(
            "const a = new BigInt64Array(2); a[0] = -1n; console.log(a[0], new BigUint64Array(a.buffer)[0], a.length); const v = new DataView(new ArrayBuffer(8)); v.setBigInt64(0, -2n); console.log(v.getBigUint64(0), v.getBigInt64(0), v.getUint8(7)); const f = new Float16Array([1.5, 100000]); console.log(f[0], f[1]); console.log(new BigInt64Array([3n, 1n, 2n]).sort().join())"
        ),
        "-1n 18446744073709551615n 2\n18446744073709551614n -2n 254\n1.5 Infinity\n1,2,3"
    );
    assert!(error_of("new BigInt64Array(1)[0] = 1").contains("BigInt"));
    assert!(error_of("new Uint8Array(new BigInt64Array(1))").contains("mix"));
    assert_eq!(
        run(
            "const b = new ArrayBuffer(4, { maxByteLength: 8 }); const t = new Uint8Array(b); const fixed = new Uint8Array(b, 0, 4); console.log(b.resizable, b.maxByteLength, t.length); b.resize(8); console.log(t.length, fixed.length); b.resize(2); console.log(t.length, fixed.length, fixed.byteLength); const c = new ArrayBuffer(2); new Uint8Array(c)[0] = 7; const d = c.transfer(4); console.log(c.detached, c.byteLength, d.byteLength, new Uint8Array(d)[0], d.resizable); try { new Uint8Array(c) } catch (e) { console.log(e.name) }"
        ),
        "true 8 4\n8 4\n2 0 0\ntrue 0 4 7 false\nTypeError"
    );
}

/// The interpreter reads and writes typed arrays' elements straight from where they are in memory, which has to be
/// worked out again whenever a buffer is resized or transferred.
#[test]
fn typed_arrays_find_their_elements_after_buffers_move() {
    assert_eq!(
        run(
            "const kinds = [Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array, Uint32Array, Float32Array, Float64Array]; const out = []; for (const K of kinds) { const a = new K(4); a[0] = -1.5; a[1] = 300; a[2] = 2.5; a[3] = NaN; a[4] = 9; out.push(a.join() + '|' + a[4] + '|' + a[-1]); } console.log(out.join(' '))"
        ),
        "-1,44,2,0|undefined|undefined 255,44,2,0|undefined|undefined 0,255,2,0|undefined|undefined -1,300,2,0|undefined|undefined 65535,300,2,0|undefined|undefined -1,300,2,0|undefined|undefined 4294967295,300,2,0|undefined|undefined -1.5,300,2.5,NaN|undefined|undefined -1.5,300,2.5,NaN|undefined|undefined"
    );
    assert_eq!(
        run(
            "const b = new ArrayBuffer(8, { maxByteLength: 16 }); const u = new Uint8Array(b, 0, 8); const all = new Uint8Array(b); u[7] = 5; b.resize(4); console.log(u[7], u.length, all[3], all.length); u[1] = 1; b.resize(16); u[7] = 6; all[15] = 9; console.log(u[7], u[1], all[15], all.length)"
        ),
        "undefined 0 0 4\n6 0 9 16"
    );
    assert_eq!(
        run(
            "const c = new ArrayBuffer(4); const v = new Int16Array(c); v[1] = -2; const d = c.transfer(8); console.log(v[1], v.length); v[1] = 3; const w = new Int16Array(d); console.log(w[1], w[3]); w[3] = 70000; console.log(w[3], new Uint16Array(d)[3]); const s = new Float64Array(3); const t = new Uint8Array(s.buffer, 8, 8); t[7] = 64; console.log(s[1], t[7])"
        ),
        "undefined 0\n-2 0\n4464 4464\n2 64"
    );
    // `i < list.length` at the end of a loop reads a typed array's length the short way only while it's the getter's.
    assert_eq!(
        run(
            "const sum = (a) => { let n = 0; for (let i = 0; i < a.length; i++) n += a[i]; return n; }; const a = new Int8Array([1, 2, 3]); const b = new Int8Array([4, 5, 6, 7]); const out = [sum(a), sum(b)]; Object.defineProperty(b, 'length', { value: 2 }); out.push(sum(a), sum(b)); const base = Object.getPrototypeOf(Int8Array.prototype); const was = Object.getOwnPropertyDescriptor(base, 'length'); Object.defineProperty(base, 'length', { get() { return 1; }, configurable: true }); out.push(sum(a), sum(new Int8Array([9, 9]))); Object.defineProperty(base, 'length', was); const r = new ArrayBuffer(2, { maxByteLength: 4 }); const t = new Uint8Array(r); t[0] = 1; t[1] = 2; const all = new Uint8Array(r); out.push(sum(all)); r.resize(4); t[3] = 3; out.push(sum(all), sum(a)); console.log(out.join())"
        ),
        "6,22,6,9,1,9,3,6,6"
    );
}

#[test]
fn regular_expressions_use_the_symbols_they_should() {
    assert_eq!(
        run(
            "const o = { [Symbol.replace]: (s, r) => s + r, [Symbol.match]: () => 'm', [Symbol.search]: () => 7, [Symbol.split]: () => ['s'], [Symbol.matchAll]: () => 'all' }; console.log('a'.replace(o, 'b'), 'a'.match(o), 'a'.search(o), 'a'.split(o).join(), 'a'.matchAll(o), 'a'.replaceAll(o, 'c'))"
        ),
        "ab m 7 s all ac"
    );
    assert_eq!(
        run(
            "console.log(/b/[Symbol.replace]('abc', 'x'), /b/[Symbol.match]('abc')[0], /b/[Symbol.search]('abc'), /b/[Symbol.split]('abc').join(), [.../b/g[Symbol.matchAll]('abcb')].length, typeof RegExp[Symbol.species], Symbol.species.toString(), Symbol.unscopables.description)"
        ),
        "axc b 1 a,c 2 function Symbol(Symbol.species) Symbol.unscopables"
    );
    assert_eq!(
        run(
            "console.log([1].concat({ length: 1, 0: 2, [Symbol.isConcatSpreadable]: true }).join(), (() => { const a = [3]; a[Symbol.isConcatSpreadable] = false; return [1].concat(a).length })())"
        ),
        "1,2 2"
    );
    for bad in [
        "/\\p{Nope}/u",
        "/\\p{Script=}/u",
        "/\\p{ASCII=F}/u",
        "/(?<a>)(?<a>)/",
        "/(?<1a>)/",
        "/\\k<a>(?<b>)/",
        "/a{2,1}/",
        "/(?<=a)+/",
        "/(?=a)+/u",
        "/]/u",
        "/{/u",
        "/\\-/u",
        "/[\\d-a]/u",
        "/\\1/u",
        "/(?<a/",
    ] {
        assert!(error_of(bad).contains("SyntaxError"), "{bad}");
    }
    assert_eq!(
        run(
            "console.log(/\\p{Lu}+/u.exec('aBCd')[0], /\\p{Script=Greek}+/u.exec('a\\u03b1\\u03b2')[0].length, /\\p{General_Category=Decimal_Number}/u.test('5'), /\\P{L}/u.test('a'), /\\p{ASCII}/u.test('a'), /\\p{Any}/u.test('a'), /\\p{L}/.test('p{L}'), /\\k<a>/.test('k<a>'), /(?=a)+/.test('a'), /]/.test(']'), /{/.test('{'), /\\-/.test('-'), /(?i:a)b/.test('Ab'), /(?i:a)b/.test('AB'))"
        ),
        "BC 2 true false true true true true true true true true true false"
    );
}

#[test]
fn arguments_has_what_it_should() {
    assert_eq!(
        run(
            "function f() { return [arguments.callee === f, [...arguments].length, arguments[Symbol.iterator] === Array.prototype.values, Object.prototype.toString.call(arguments), Object.getOwnPropertyDescriptor(arguments, 'length').enumerable, Array.prototype.slice.call(arguments, 1).length] } console.log(f(1, 2).join())"
        ),
        "true,2,true,[object Arguments],false,1"
    );
    assert!(
        error_of("function f() { 'use strict'; return arguments.callee } f()")
            .contains("TypeError")
    );
}

#[test]
fn generators_check_their_arguments_when_called() {
    assert_eq!(
        run(
            "function* g([x]) { yield x } try { g(null); console.log('no') } catch (e) { console.log(e.name) } function* d(a = (() => { throw new Error('x') })()) {} try { d(); console.log('no') } catch (e) { console.log(e.message) } let n = 0; function* c(a = n++) { yield a } const i = c(); console.log(n, i.next().value, n)"
        ),
        "TypeError\nx\n1 0 1"
    );
    assert_eq!(
        run(
            "async function* a([x]) {} try { a(null); console.log('no') } catch (e) { console.log(e.name) } async function f([x]) {} f(null).catch(e => console.log('rejected', e.name)); function* s(...r) { yield r.length; yield arguments.length } const i = s(1, 2); console.log(i.next().value, i.next().value)"
        ),
        "TypeError\n2 2\nrejected TypeError"
    );
}

#[test]
fn private_names_have_to_be_there() {
    assert_eq!(
        run(
            "class A { #x = 1; static read(o) { return o.#x } static write(o) { o.#x = 2 } static has(o) { return #x in o } } console.log(A.read(new A()), A.has(new A()), A.has({})); try { A.read({}) } catch (e) { console.log(e.name) } try { A.write({}) } catch (e) { console.log(e.name) } const a = new A(); A.write(a); console.log(A.read(a), Object.keys(a).length, JSON.stringify(a))"
        ),
        "1 true false\nTypeError\nTypeError\n2 0 {}"
    );
    assert!(
        error_of("class A { #m() {} static w(o) { o.#m = 1 } } A.w(new A())").contains("TypeError")
    );
}

#[test]
fn each_class_has_its_own_private_names() {
    assert_eq!(
        run(
            "class Base { #value = 'base'; get fromBase() { return this.#value } }
             class Derived extends Base { #value = 'derived'; get fromDerived() { return this.#value } }
             const d = new Derived();
             console.log(d.fromBase, d.fromDerived);
             class A { #secret = 1; static read(o) { return o.#secret } }
             class B { #secret = 2 }
             try { A.read(new B()); console.log('read it') } catch (e) { console.log(e.name) }
             class Outer { #x = 'outer'; inner() { const self = this; return new (class { #y = 'inner'; show() { return self.#x + this.#y } })().show() } }
             console.log(new Outer().inner())"
        ),
        "base derived\nTypeError\nouterinner"
    );
}

#[test]
fn private_names_do_not_come_through_inheritance() {
    assert_eq!(
        run(
            "class M { #m() { return 'ok' } #f = 1; static call(o) { return o.#m() } static has(o) { return #m in o } static read(o) { return o.#f } }
             const child = Object.create(new M());
             for (const test of [() => M.call(child), () => M.read(child), () => M.call(new Proxy(new M(), {}))]) {
                 try { console.log(test()) } catch (e) { console.log(e.name) }
             }
             console.log(M.has(child), M.has(new M()));
             class S { static #s = 5; static #sm() { return 6 } static get v() { return this.#s + this.#sm() } }
             console.log(S.v);
             class Sub extends S {}
             try { console.log(Sub.v) } catch (e) { console.log(e.name) }"
        ),
        "TypeError\nTypeError\nTypeError\nfalse true\n11\nTypeError"
    );
}

#[test]
fn private_accessors_work_and_refuse_what_they_cannot_do() {
    assert_eq!(
        run(
            "class P { set #x(v) { this.y = v } get #x() { return this.y * 2 } go() { this.#x = 21; this.#x += 1; return this.#x } }
             console.log(new P().go());
             class G { get #g() { return 1 } w() { this.#g = 2 } }
             try { new G().w(); console.log('wrote') } catch (e) { console.log(e.name) }
             class O { set #o(v) {} r() { return this.#o } }
             try { new O().r(); console.log('read') } catch (e) { console.log(e.name) }
             class N { #n() {} static name(o) { return o.#n.name } }
             console.log(N.name(new N()));
             class E { #field = 1; get #a() { return this.#field } read = this.#a; }
             console.log(new E().read)"
        ),
        "86\nTypeError\nTypeError\n#n\n1"
    );
}

#[test]
fn a_private_name_can_only_be_put_on_an_object_once() {
    assert_eq!(
        run("class R { constructor(o) { return o } }
             class T extends R { #t = 1; static has(o) { return #t in o } }
             const plain = Object.freeze({});
             new T(plain);
             console.log(T.has(plain));
             try { new T(plain); console.log('again') } catch (e) { console.log(e.name) }
             class U extends R { #u() {} }
             const other = {};
             new U(other);
             try { new U(other); console.log('again') } catch (e) { console.log(e.name) }"),
        "true\nTypeError\nTypeError"
    );
}

#[test]
fn a_subclass_of_promise_gets_its_own_kind_back() {
    assert_eq!(
        run(
            "class P extends Promise {}
             const kinds = [
                 P.resolve(1).then(() => {}), P.resolve(1).catch(() => {}), P.resolve(1).finally(() => {}),
                 P.reject(1).catch(() => {}), P.all([]), P.allSettled([]), P.race([]), P.any([1]),
                 P.withResolvers().promise,
             ];
             console.log(kinds.map((one) => one instanceof P).join());
             const plain = Promise.resolve(1);
             console.log(Promise.resolve(plain) === plain, P.resolve(plain) === plain);
             class Q extends Promise { static get [Symbol.species]() { return Promise } }
             console.log(new Q(() => {}).then() instanceof Q);
             const out = [];
             P.all([P.resolve(1), 2]).then((all) => out.push('all ' + all));
             P.any([P.reject(1), P.resolve(2)]).then((any) => out.push('any ' + any));
             P.resolve(5).finally(() => 6).then((value) => out.push('finally ' + value));
             setTimeout(() => console.log(out.sort().join()), 0);"
        ),
        "true,true,true,true,true,true,true,true,true\ntrue false\nfalse\nall 1,2,any 2,finally 5"
    );
}

#[test]
fn promise_helpers_work_with_anything_that_looks_like_a_promise_constructor() {
    assert_eq!(
        run(
            "const seen = [];
             function Thing(executor) { executor((value) => seen.push('resolved ' + value), (value) => seen.push('rejected ' + value)) }
             Thing.resolve = (value) => { if (value === 'bad') throw 'bad one'; return { then(done) { done(value) } } };
             const closing = { [Symbol.iterator]() { return { next: () => ({ value: 'bad', done: false }), return() { seen.push('closed'); return {} } } } };
             console.log(Promise.all.call(Thing, closing) instanceof Thing);
             Promise.all.call(Thing, [1, 2]);
             console.log(seen.join());
             try { Promise.all.call(1, []) } catch (e) { console.log(e.name) }
             try { Promise.resolve.call(1) } catch (e) { console.log(e.name) }"
        ),
        "true\nclosed,rejected bad one,resolved 1,2\nTypeError\nTypeError"
    );
}

#[test]
fn a_helper_of_promise_all_only_counts_the_first_time() {
    assert_eq!(
        run("let settled = false;
             function Thing(executor) { executor(() => { settled = true }, () => {}) }
             Thing.resolve = (value) => value;
             let twice;
             Promise.all.call(Thing, [{ then(done) { twice = done } }, { then() {} }]);
             twice(1);
             twice(2);
             console.log(settled)"),
        "false"
    );
}

#[test]
fn array_methods_read_the_length_before_looking_at_the_callback() {
    assert_eq!(
        run(
            "const seen = [];
             const like = { get length() { seen.push('length'); throw new RangeError('from length') } };
             for (const name of ['every', 'some', 'forEach', 'map', 'filter', 'reduce', 'reduceRight', 'find']) {
                 try { Array.prototype[name].call(like); seen.push('no error') } catch (e) { seen.push(e.name) }
             }
             console.log(seen.filter((one) => one !== 'length').join())"
        ),
        "RangeError,RangeError,RangeError,RangeError,RangeError,RangeError,RangeError,RangeError"
    );
}

#[test]
fn a_subclass_of_array_gets_its_own_kind_back() {
    assert_eq!(
        run(
            "class Stack extends Array { top() { return this[this.length - 1] } }
             const s = Stack.from([1, 2, 3]);
             const made = {
                 map: s.map((x) => x * 2), filter: s.filter((x) => x > 1), slice: s.slice(1), concat: s.concat([4]),
                 splice: Stack.from([1, 2, 3]).splice(0, 2), flat: s.flat(), flatMap: s.flatMap((x) => [x, x]),
             };
             for (const [name, result] of Object.entries(made)) {
                 console.log(name, result instanceof Stack, result.length, result.top());
             }
             class Plain extends Array { static get [Symbol.species]() { return Array } }
             console.log(new Plain(1, 2).map((x) => x) instanceof Plain, Array.isArray(new Plain(1, 2).map((x) => x)));
             const odd = [1, 2];
             odd.constructor = null;
             try { odd.map((x) => x); console.log('no error') } catch (e) { console.log(e.name) }
             const none = [1];
             none.constructor = undefined;
             console.log(none.map((x) => x + 1)[0]);
             const frozen = [1];
             frozen.constructor = { [Symbol.species]: function () { return Object.freeze({}) } };
             try { frozen.map((x) => x); console.log('no error') } catch (e) { console.log(e.name) }"
        ),
        "map true 3 6\nfilter true 2 3\nslice true 2 3\nconcat true 4 4\nsplice true 2 2\nflat true 3 3\nflatMap true 6 3\nfalse true\nTypeError\n2\nTypeError"
    );
}

#[test]
fn what_is_remembered_about_constructors_does_not_outlast_a_change() {
    assert_eq!(
        run(
            "class Other extends Array {}
             const a = [1, 2];
             console.log(a.slice() instanceof Other, a.map((x) => x) instanceof Other);
             const before = Array.prototype.constructor;
             Array.prototype.constructor = Other;
             console.log(a.slice() instanceof Other, a.map((x) => x) instanceof Other, [3].concat(4) instanceof Other);
             delete Array.prototype.constructor;
             console.log(a.slice() instanceof Other, Array.isArray(a.slice()));
             Array.prototype.constructor = before;
             console.log(a.slice() instanceof Other);
             a.constructor = Other;
             console.log(a.slice() instanceof Other, [1].slice() instanceof Other);
             class P extends Promise {}
             const q = Promise.resolve(1);
             console.log(q.then() instanceof P);
             const was = Promise.prototype.constructor;
             Promise.prototype.constructor = P;
             console.log(q.then() instanceof P, Promise.resolve(q) === q);
             Promise.prototype.constructor = was;
             console.log(q.then() instanceof P, Promise.resolve(q) === q);"
        ),
        "false false\ntrue true true\nfalse true\nfalse\ntrue false\nfalse\ntrue false\nfalse true"
    );
}

#[test]
fn methods_are_only_for_what_they_are_for() {
    for code in [
        "Map.prototype.get.call(new Set(), 1)",
        "Set.prototype.add.call(new Map(), 1)",
        "WeakMap.prototype.get.call(new Map(), {})",
        "Map.prototype.keys.call(new Set())",
        "Set.prototype.values.call(new Map())",
        "new WeakMap().set(1, 1)",
        "new WeakSet().add('a')",
        "Uint8Array.prototype.join.call([])",
        "Uint8Array.prototype.values.call({})",
        "Object.getOwnPropertyDescriptor(RegExp.prototype, 'global').get.call({})",
    ] {
        assert!(error_of(code).contains("TypeError"), "{code}");
    }
    assert_eq!(
        run(
            "console.log(RegExp.prototype.global, RegExp.prototype.source, new Uint8Array([1]).values().next().value, new Uint8Array([1])[Symbol.iterator]().next().value, [...new Uint8Array([2])].join(), new Uint8Array(2).join('-'))"
        ),
        "undefined (?:) 1 1 2 0-0"
    );
}

#[test]
fn old_code_still_means_what_it_meant() {
    assert_eq!(run("console.log(010, 0777, 089)"), "8 511 89");
    assert_eq!(
        run("const f = function g() { g = 1; return typeof g }; console.log(f())"),
        "function"
    );
    assert!(error_of("'use strict'; const f = function g() { g = 1 }; f()").contains("constant"));
    assert_eq!(
        run(
            "function _t(o) { return _t = function (x) { return x + 1 }, _t(o) } console.log(_t(1), _t(2))"
        ),
        "2 3"
    );
}

#[test]
fn typed_arrays_share_their_memory() {
    assert_eq!(
        run(
            "const a = new Uint8Array(3); a[0] = 257; a[1] = -1; a[5] = 1; console.log(a[0], a[1], a[2], a[5], a.length, a.byteLength, 5 in a, 0 in a)"
        ),
        "1 255 0 undefined 3 3 false true"
    );
    assert_eq!(
        run(
            "const b = new ArrayBuffer(8); const u = new Uint8Array(b); const f = new Float64Array(b); f[0] = 1; console.log(u[7], u[6], b.byteLength); const v = new DataView(b); console.log(v.getUint8(7), v.getFloat64(0, true)); v.setInt16(0, -2); console.log(u[0], u[1])"
        ),
        "63 240 8\n63 1\n255 254"
    );
    assert_eq!(
        run(
            "const a = Int16Array.from([1, 2, 3]); console.log([...a].join(), a.map(x => x * 2).join(), a.subarray(1).length, a.slice(0, 1).length, a.join('-'), a.indexOf(2), Int16Array.BYTES_PER_ELEMENT, a.buffer.byteLength, a.includes(9), Array.from(a).length)"
        ),
        "1,2,3 2,4,6 2 1 1-2-3 1 2 6 false 3"
    );
    assert_eq!(
        run(
            "const s = new Uint8Array([1, 2, 3, 4]); const t = s.subarray(1, 3); t[0] = 9; console.log(s[1], t.length, t.byteOffset); s.set([7, 8], 2); console.log(s.join()); console.log(new Uint8ClampedArray([300, -5, 1.5, 2.5]).join(), new Int8Array([200])[0], new Uint32Array([-1])[0], new Float32Array([1.5])[0])"
        ),
        "9 2 1\n1,9,7,8\n255,0,2,2 -56 4294967295 1.5"
    );
    assert_eq!(
        run(
            "console.log(Object.prototype.toString.call(new Uint8Array(1)), new Uint8Array(2), ArrayBuffer.isView(new Uint8Array(1)), ArrayBuffer.isView([]), Object.keys(new Uint8Array(2)).join())"
        ),
        "[object Uint8Array] Uint8Array(2) [ 0, 0 ] true false 0,1"
    );
    assert_eq!(
        run(
            "console.log(new TextDecoder().decode(new TextEncoder().encode('h\\u00e9')), new TextEncoder().encode('h\\u00e9').length, btoa('hi'), atob('aGk='), crypto.getRandomValues(new Uint8Array(4)).length, crypto.randomUUID().length)"
        ),
        "h\u{e9} 3 aGk= hi 4 36"
    );
    assert_eq!(
        run(
            "const a = { d: new Date(0), m: new Map([[1, { x: 1 }]]), l: [1] }; a.self = a; const c = structuredClone(a); console.log(c.d.getTime(), c.m.get(1).x, c.l !== a.l, c.self === c)"
        ),
        "0 1 true true"
    );
}

#[test]
fn a_get_trap_called_in_place_is_called_as_any_other() {
    // The interpreter calls a `get` trap written in JavaScript in place, for `p.x`, `p[key]` and their polymorphic
    // kind; a method read (`p.m()`), a trap that isn't, and a private name go the long way. Both have to agree.
    assert_eq!(
        run(
            "const log = []; const h = { get(t, k, r) { log.push(typeof k === 'symbol' ? 'sym' : k, arguments.length, this === h, r === p); if (k === 'boom') throw new Error('no ' + k); return k === 'm' ? () => 'called' : t[k] } }; const p = new Proxy({ a: 1, 0: 'zero' }, h); const read = (o) => o.a; console.log(p.a, p['a'], p[0], p[Symbol.iterator], p.m(), read({ a: 2 }), read({ b: 1, a: 3 }), read(p)); try { p.boom } catch (e) { console.log(e.message, e.stack.includes('get')) } console.log(log.join())"
        ),
        "1 1 zero undefined called 2 3 1\nno boom true\na,3,true,true,a,3,true,true,0,3,true,true,sym,3,true,true,m,3,true,true,a,3,true,true,boom,3,true,true"
    );
    assert_eq!(
        run(
            "const deep = new Proxy({}, { get(t, k) { return k.length > 3000 ? k.length : deep[k + 'x'] } }); console.log(deep.x); let n = 0; const loop = new Proxy({}, { get(t, k) { n++; return loop[k] } }); try { loop.x } catch (e) { console.log(e.constructor.name, n > 1000) }"
        ),
        "3001\nRangeError true"
    );
    assert_eq!(
        run(
            "const bound = new Proxy({}, { get: function (t, k) { return k + this.tag }.bind({ tag: '!' }) }); const arrow = new Proxy({ v: 5 }, { get: (t, k) => t[k] * 2 }); const gen = new Proxy({}, { *get(t, k) { yield k } }); const none = new Proxy({ m: 3 }, {}); const cls = new Proxy({}, { get: class {} }); console.log(bound.k, arrow.v, gen.w.next().value, none.m); try { cls.z } catch (e) { console.log(e.constructor.name) } const looked = {}; Object.defineProperty(looked, 'get', { get() { console.log('trap looked up'); return (t, k) => 'v' + k } }); console.log(new Proxy({}, looked).z); new Proxy({}, { async get(t, k) { return k } }).u.then((v) => console.log('async', v))"
        ),
        "k! 10 w 3\nTypeError\ntrap looked up\nvz\nasync u"
    );
    assert_eq!(
        run(
            "class Base { constructor() { return new Proxy(this, { get(t, k) { return k === 'hello' ? 'hi' : Reflect.get(t, k) } }) } } class Sub extends Base { #secret = 7; reveal() { return this.#secret } } const s = new Sub(); console.log(s.hello, s.reveal())"
        ),
        "hi 7"
    );
}

#[test]
fn getters_called_in_place_see_every_change() {
    // Once what reads remember says a property is a getter written in JavaScript, the interpreter calls it in place.
    // Changing the getter, or the property to a value, or the prototype, has to be seen.
    assert_eq!(
        run(
            "const out = []; class Node { constructor(n) { this.n = n } get next() { return this.n + 1 } get self() { return this } } const nodes = [new Node(1), new Node(2), new Node(3)]; let s = 0; for (let i = 0; i < 50; i++) for (const node of nodes) s += node.next; out.push(s, nodes[0].self === nodes[0]); const own = {}; let calls = 0; Object.defineProperty(own, 'v', { get() { calls++; return 'own' + calls }, configurable: true }); for (let i = 0; i < 5; i++) own.v; out.push(own.v, calls); Object.defineProperty(own, 'v', { get() { return 'replaced' }, configurable: true }); out.push(own.v); Object.defineProperty(own, 'v', { value: 'data now', configurable: true }); out.push(own.v); const proto = { get p() { return 'proto ' + this.tag } }; const kids = [1, 2, 3].map((tag) => { const o = Object.create(proto); o.tag = tag; return o }); const read = (o) => o.p; for (let i = 0; i < 20; i++) kids.forEach(read); out.push(kids.map(read).join('')); Object.defineProperty(proto, 'p', { get() { return 'changed ' + this.tag } }); out.push(kids.map(read).join('')); const throwing = { get bad() { throw new Error('threw') } }; for (let i = 0; i < 3; i++) { try { throwing.bad } catch (e) { out.push(e.message) } } let depth = 0; const rec = { get r() { depth++; return depth < 2000 ? this.r : depth } }; out.push(rec.r); const inf = { get x() { return this.x } }; try { inf.x } catch (e) { out.push(e.constructor.name) } class Static { static get s() { return this.name } } for (let i = 0; i < 2; i++) out.push(Static.s); const bound = {}; Object.defineProperty(bound, 'b', { get: function () { return this.k }.bind({ k: 'bound' }) }); for (let i = 0; i < 2; i++) out.push(bound.b); const cls = {}; Object.defineProperty(cls, 'c', { get: class X {} }); for (let i = 0; i < 2; i++) { try { cls.c } catch (e) { out.push(e.constructor.name) } } const gen = {}; Object.defineProperty(gen, 'g', { get: function* () { yield 'gen' } }); for (let i = 0; i < 2; i++) out.push(gen.g.next().value); Object.defineProperty(Number.prototype, 'twice', { get() { return this * 2 }, configurable: true }); for (let i = 0; i < 2; i++) out.push((5).twice); delete Number.prototype.twice; console.log(out.join())"
        ),
        "450,true,own6,6,replaced,data now,proto 1proto 2proto 3,changed 1changed 2changed 3,threw,threw,threw,2000,RangeError,Static,Static,bound,bound,TypeError,TypeError,gen,gen,10,10"
    );
}

#[test]
fn a_parameter_nothing_changes_is_captured_as_its_value() {
    // A closure captures a parameter's value when nothing could give it another; anything that might keeps it in a
    // cell, in this function or one inside it, by any way of assigning or declaring.
    assert_eq!(
        run(
            "const out = []; function plain(a, b) { return () => a + b } out.push(plain(1, 2)()); function assigned(a) { const f = () => a; a = 5; return f() } out.push(assigned(1)); function inside(a) { const set = () => { a = 9 }; const get = () => a; set(); return get() } out.push(inside(1)); function inc(a) { const f = () => a; a++; return f() } out.push(inc(1)); function swap(a, b) { const f = () => a + b; [a, b] = [b, a]; return f() } out.push(swap('x', 'y')); function forOf(a) { const fs = []; for (a of [1, 2]) fs.push(() => a); return fs.map((f) => f()).join() } out.push(forOf(0)); function again(a) { const f = () => a; var a = 3; return f() } out.push(again(1)); function fn(a) { const f = () => typeof a; function a() {} return f() } out.push(fn(1)); function deep(a) { return () => () => () => a } out.push(deep('deep')()()()); function deepSet(a) { const get = () => () => a; const set = () => () => { a = 'changed' }; set()(); return get()() } out.push(deepSet('orig')); function own(a) { const g = (a) => { a = 1; return a }; return g(5) + (() => a)() } out.push(own(10)); function later(a) { function inner() { return a } function setter() { a = 'set' } setter(); return inner() } out.push(later('no')); function lazy(a) { function inner() { return a } return inner } out.push(lazy('lazy')()); function* gen(a) { yield () => a } out.push(gen('g').next().value()); function cls(a) { return class { m() { return a } static s = a } } out.push(new (cls('c'))().m(), cls('d').s); function inField(a) { class C { f = (a = 'field') } new C(); return (() => a)() } out.push(inField('y')); function inKey(a) { const o = { [(a = 'key')]: 1 }; return (() => a)() } out.push(inKey('v')); function withIt(a) { const o = { a: 'o' }; with (o) { a = 'w' } return (() => a)() + o.a } out.push(withIt('p')); function dup(a, b = () => a) { return b() } out.push(dup(6)); console.log(out.join())"
        ),
        "3,5,9,2,yx,2,2,3,function,deep,changed,11,set,lazy,g,c,d,field,key,pw,6"
    );
}

#[test]
fn classes_are_constructed_in_every_way() {
    // Fields, constructors that aren't written, `super(...args)` and fields are run in place by the interpreter; a
    // constructor that isn't written hands on its arguments without asking the array for its iterator.
    assert_eq!(
        run(
            "const out = []; class A { origin; x = 1; #p = 'p'; constructor(o) { this.origin = o } get p() { return this.#p } } class B extends A { y = this.x + 1 } class C extends B { z = this.y + 1; static s = 's'; #q = 'q'; q() { return this.#q } } const c = new C(5); out.push(c.origin, c.x, c.y, c.z, c.p, c.q(), C.s, Object.keys(c).join(''), c instanceof A); class D extends A { constructor(...args) { super(...args); this.w = args.length } } out.push(new D(1, 2, 3).w, new D().origin); const saved = Array.prototype[Symbol.iterator]; let iterated = 0; Array.prototype[Symbol.iterator] = function () { iterated++; return saved.call(this) }; new C(1); const unwritten = iterated; new D(1); const written = iterated - unwritten; Array.prototype[Symbol.iterator] = saved; out.push(unwritten, written > 0); class G { constructor() { return { other: true } } } class H extends G { field = 'h' } out.push(JSON.stringify(new H())); class I { constructor() { throw new Error('I throws') } } class J extends I { f = out.push('never') } try { new J() } catch (e) { out.push(e.message) } class K { f = (() => { throw new Error('field throws') })() } try { new K() } catch (e) { out.push(e.message) } class M extends A { constructor() { try { super(1) } finally { out.push('finally') } } } out.push(new M().origin); function Old(a) { this.a = a } class N extends Old { b = 2 } const n = new N(4); out.push(n.a, n.b); class P extends A { constructor(...rest) { super(...rest, 'extra') } } out.push(new P().origin); const proxied = new Proxy(A, { construct(t, args, nt) { return Reflect.construct(t, ['proxied'], nt) } }); class V extends proxied { v = 1 } out.push(new V().origin, new V().v); class E extends Array {} const e = new E(3); out.push(e.length, e instanceof E); console.log(out.join())"
        ),
        "5,1,2,3,p,q,s,originxyz,true,3,,0,true,{\"other\":true,\"field\":\"h\"},I throws,field throws,finally,1,4,2,extra,proxied,1,3,true"
    );
}

#[test]
fn methods_in_an_object_literal_are_its_own_properties() {
    // A method with a name of its own is defined as a field is; one with a computed, numbered or `__proto__` key by
    // its key.
    assert_eq!(
        run(
            "const k = 'c'; const o = { a() { return super.toString === Object.prototype.toString }, 'b-b'() { return 2 }, 0() { return 0 }, [k]() { return 3 }, __proto__() { return 9 }, get g() { return 4 }, a2: 1 }; console.log(Object.keys(o).join(), o.a.name, o['b-b'].name, o.a(), o['b-b'](), o[0](), o.c(), o.__proto__(), o.g, Object.getPrototypeOf(o) === Object.prototype, JSON.stringify(Object.getOwnPropertyDescriptor(o, 'a')), 'prototype' in o.a); const dup = { x: 1, x() { return 'm' } }, dup2 = { get y() { return 1 }, y() { return 'm2' } }; console.log(dup.x(), dup2.y(), typeof Object.getOwnPropertyDescriptor(dup2, 'y').value); const made = []; for (let i = 0; i < 3; i++) made.push({ i, get() { return this.i }, set(v) { this.i = v } }); made[1].set(7); console.log(made.map((m) => m.get()).join())"
        ),
        "0,a,b-b,c,__proto__,g,a2 a b-b true 2 0 3 9 4 true {\"writable\":true,\"enumerable\":true,\"configurable\":true} false\nm m2 function\n0,7,2"
    );
}

#[test]
fn to_string_tags_are_the_built_in_ones_unless_something_says_otherwise() {
    assert_eq!(
        run(
            "const ts = Object.prototype.toString; console.log([undefined, null, 1, 1.5, 'a', true, {}, [], () => 1, new Error(), new Date(0), /a/, (function () { return arguments })(), new Proxy([], {}), Object.create(null), new String('s'), new Number(1), Math].map((v) => ts.call(v)).join()); Object.defineProperty(Number.prototype, Symbol.toStringTag, { value: 'Num', configurable: true }); console.log(ts.call(1), ts.call(new Number(2))); delete Number.prototype[Symbol.toStringTag]; Object.defineProperty(String.prototype, Symbol.toStringTag, { get() { 'use strict'; return typeof this }, configurable: true }); console.log(ts.call('x')); delete String.prototype[Symbol.toStringTag]; Boolean.prototype[Symbol.toStringTag] = 5; console.log(ts.call(true), ts.call({ [Symbol.toStringTag]: 'Mine' }), ts.call(1) === ts.call(2))"
        ),
        "[object Undefined],[object Null],[object Number],[object Number],[object String],[object Boolean],[object Object],[object Array],[object Function],[object Error],[object Date],[object RegExp],[object Arguments],[object Array],[object Object],[object String],[object Number],[object Math]\n[object Num] [object Num]\n[object object]\n[object Boolean] [object Mine] true"
    );
}

#[test]
fn proxies_handle_everything() {
    assert_eq!(
        run(
            "const log = []; const p = new Proxy({ a: 1 }, { get(t, k, r) { log.push('get ' + String(k)); return Reflect.get(t, k, r) }, set(t, k, v) { log.push('set ' + k); t[k] = v * 2; return true }, has(t, k) { log.push('has ' + k); return k in t }, deleteProperty(t, k) { log.push('del ' + k); return delete t[k] }, ownKeys(t) { log.push('keys'); return Reflect.ownKeys(t) } }); p.a; p.b = 2; 'a' in p; delete p.a; Object.keys(p); console.log(log.join(), p.b)"
        ),
        "get a,set b,has a,del a,keys 4"
    );
    assert_eq!(
        run(
            "function f(p) { return p.x } const o = { x: 1 }; const p = new Proxy(o, { get: () => 2 }); console.log(f(o), f(o), f(p), f(o)); const q = Object.create(p); console.log(q.x); function w(t) { t.y = 1 } let s = 0; const r = new Proxy({}, { set() { s++; return true } }); w({}); w({}); w(r); console.log(s, Object.create(r).y = 5, s)"
        ),
        "1 1 2 1\n2\n1 5 2"
    );
    assert_eq!(
        run(
            "const f = new Proxy(function (a) { return a + 1 }, { apply(t, th, args) { return t(args[0]) * 10 }, construct(t, args) { return { made: args[0] } } }); console.log(f(1), new f(5).made, typeof f); const c = new Proxy(class { constructor() { this.v = 1 } }, {}); console.log(new c().v)"
        ),
        "20 5 function\n1"
    );
    assert_eq!(
        run(
            "const p = new Proxy([1, 2], {}); console.log(Array.isArray(p), p.length, JSON.stringify(p), [...p].join(), p.map(x => x * 2).join(), Object.prototype.toString.call(p)); p.push(3); console.log(p.length, p.pop(), p.shift(), p.join()); p.unshift(5); p.splice(1, 0, 6); p.reverse(); console.log(p.join())"
        ),
        "true 2 [1,2] 1,2 2,4 [object Array]\n3 3 1 2\n2,6,5"
    );
    assert_eq!(
        run(
            "const { proxy, revoke } = Proxy.revocable({}, {}); proxy.a = 1; console.log(proxy.a); revoke(); try { proxy.a } catch (e) { console.log(e.name) }"
        ),
        "1\nTypeError"
    );
    assert_eq!(
        run(
            "const p = new Proxy({}, { getOwnPropertyDescriptor: () => ({ value: 1, enumerable: true, configurable: true }), ownKeys: () => ['z'], getPrototypeOf: () => Array.prototype, defineProperty(t, k, d) { t[k] = d.value + 1; return true } }); console.log(Object.keys(p).join(), JSON.stringify(Object.getOwnPropertyDescriptor(p, 'q')), Object.getPrototypeOf(p) === Array.prototype); Object.defineProperty(p, 'n', { value: 1 });"
        ),
        "z {\"value\":1,\"writable\":false,\"enumerable\":true,\"configurable\":true} true"
    );
    assert!(error_of("new Proxy({}, { set: () => false }).x = 1").contains("falsish"));
    assert!(error_of("new Proxy(1, {})").contains("non-object"));
}

#[test]
fn async_generators_and_for_await_work() {
    assert_eq!(
        run(
            "async function* g() { yield 1; const x = await Promise.resolve(2); yield x; return 3 } (async () => { const i = g(); console.log(JSON.stringify(await i.next()), JSON.stringify(await i.next()), JSON.stringify(await i.next()), JSON.stringify(await i.next())) })()"
        ),
        "{\"value\":1,\"done\":false} {\"value\":2,\"done\":false} {\"value\":3,\"done\":true} {\"done\":true}"
    );
    assert_eq!(
        run(
            "async function* g() { for (let i = 0; i < 3; i++) { await null; yield i } } (async () => { let s = ''; for await (const x of g()) s += x; console.log(s); for await (const y of [Promise.resolve(5), 6]) console.log(y); for await (const z of (function* () { yield 7 })()) console.log(z) })()"
        ),
        "012\n5\n6\n7"
    );
    assert_eq!(
        run(
            "async function* g() { try { yield 1; yield 2 } finally { console.log('f') } } (async () => { for await (const x of g()) { console.log(x); break } const i = g(); await i.next(); console.log(JSON.stringify(await i.return(9))) })()"
        ),
        "1\nf\nf\n{\"value\":9,\"done\":true}"
    );
    assert_eq!(
        run(
            "async function* g() { throw new Error('x') } g().next().catch(e => console.log(e.message)); async function* h() { yield 1 } const i = h(); const a = i.next(); const b = i.next(); Promise.all([a, b]).then(r => console.log(r[0].value, r[1].done)); console.log(i[Symbol.asyncIterator]() === i)"
        ),
        "true\nx\n1 true"
    );
    assert_eq!(
        run(
            "const o = { async *g() { yield this.v }, v: 1 }; o.g().next().then(r => console.log(r.value)); class C { async *m() { yield 2 } } new C().m().next().then(r => console.log(r.value))"
        ),
        "1\n2"
    );
    assert!(
        error_of("(async () => { for await (const x of 5) {} })()").contains("not async iterable")
    );
}

#[test]
fn bigints_work() {
    assert_eq!(
        run(
            "console.log(1n + 2n, 2n ** 64n, -7n / 2n, -7n % 2n, 5n * 5n, 2n - 3n, typeof 1n, 0x10n, 0b11n, 1_000n)"
        ),
        "3n 18446744073709551616n -3n -1n 25n -1n bigint 16n 3n 1000n"
    );
    assert_eq!(
        run(
            "console.log(1n < 2n, 2n > 1, 1 < 2n, 1n == 1, 1n === 1, 2n == '2', 2n > '1', 1n < 1.5, 1.5 < 1n, NaN < 1n, 0n ? 1 : 2, !0n, !!1n)"
        ),
        "true true true true false true true true false false 2 true true"
    );
    assert_eq!(
        run(
            "let a = 1n; a++; a += 2n; console.log(a, -a, ~a, a << 2n, a >> 1n, a & 6n, a | 1n, a ^ 1n, String(a), a.toString(2), `${a}`, BigInt(5), BigInt('0x10'), BigInt(true), Number(5n), parseInt('5'))"
        ),
        "4n -4n -5n 16n 2n 4n 5n 5n 4 100 4 5n 16n 1n 5 5"
    );
    assert_eq!(
        run(
            "console.log(BigInt.asUintN(8, 257n), BigInt.asUintN(8, -1n), BigInt.asIntN(8, 255n), BigInt.asIntN(8, 127n), BigInt.asIntN(64, 2n ** 63n))"
        ),
        "1n 255n -1n 127n -9223372036854775808n"
    );
    assert_eq!(
        run(
            "const m = new Map([[1n, 'a']]); console.log(m.get(1n), m.get(1), new Set([1n, 1n]).size, [1n].includes(1n), [1n].indexOf(1n), Object(1n) instanceof BigInt, typeof Object(1n), { [1n]: 2 }['1'], [3n, 1n].sort().join())"
        ),
        "a undefined 1 true 0 true object 2 1,3"
    );
    for (code, message) in [
        ("1n + 1", "mix"),
        ("+1n", "convert"),
        ("1n / 0n", "zero"),
        ("BigInt(1.5)", "integer"),
        ("BigInt('x')", "SyntaxError"),
        ("JSON.stringify(1n)", "serialize"),
        ("new BigInt(1)", "not a constructor"),
        ("2n ** -1n", "non-negative"),
        ("1n >>> 1n", "shift"),
        ("Math.max(1n)", "convert"),
    ] {
        assert!(error_of(code).contains(message), "{code}");
    }
    assert_eq!(
        run(
            "let big = []; for (let i = 0; i < 2000; i++) big.push(BigInt(i) ** 40n); let s = 0n; for (const b of big) s += b; console.log(String(s).length)"
        ),
        "134"
    );
}

#[test]
fn eval_runs_code_and_gives_its_last_value() {
    assert_eq!(
        run(
            "console.log(eval('1 + 1'), eval('var a = 5; a * 2'), eval(7), eval(''), (0, eval)('typeof globalThis'))"
        ),
        "2 10 7 undefined object"
    );
    assert!(error_of("eval('let = 1 2')").contains("SyntaxError"));
    assert_eq!(
        run(
            "console.log(escape('a b+\\u00e9\\u1234'), unescape('a%20b%E9%u1234') === 'a b\\u00e9\\u1234')"
        ),
        "a%20b+%E9%u1234 true"
    );
}

#[test]
fn yield_star_gives_back_what_it_went_through_returned() {
    assert_eq!(
        run(
            "function* a() { const x = yield 1; return x + 1 } function* b() { const r = yield* a(); yield r * 10 } const i = b(); i.next(); console.log(i.next(4).value, i.next().done)"
        ),
        "50 true"
    );
    assert_eq!(
        run(
            "function* g() { const r = yield* [1, 2]; console.log(r); return 3 } console.log([...g()].join())"
        ),
        "undefined\n1,2"
    );
    assert_eq!(
        run("function* g() { yield* 'ab'; yield* new Set([1]) } console.log([...g()].join())"),
        "a,b,1"
    );
    assert_eq!(
        run(
            "function* a() { try { yield 1 } finally { console.log('closed') } } function* b() { yield* a() } const i = b(); i.next(); i.return(5)"
        ),
        "closed"
    );
    assert_eq!(
        run(
            "async function* a() { yield 1; return 2 } async function* b() { const r = yield* a(); yield r } (async () => { const out = []; for await (const x of b()) out.push(x); console.log(out.join()) })()"
        ),
        "1,2"
    );
    assert_eq!(
        run(
            "function* w() { let s = 0; while (true) s += yield s } function* o() { return yield* w() } const i = o(); i.next(); i.next(2); console.log(i.next(3).value)"
        ),
        "5"
    );
}

#[test]
fn syntax_errors_say_where() {
    let error = error_of("let a = 1;\nlet b = ;");
    assert!(
        error.contains("SyntaxError") && error.contains(":2"),
        "{error}"
    );
    assert!(error_of("break").contains("isn't inside a loop"));
    assert!(error_of("a: while (1) { break b }").contains("isn't inside a loop"));
}

#[test]
fn errors_know_where_they_came_from() {
    let stack = run(
        "function inner() { return new Error('x') }\nfunction outer() {\n  return inner()\n}\nconsole.log(outer().stack)",
    );
    assert!(stack.starts_with("Error: x\n"), "{stack}");
    assert!(stack.contains("at inner (test.js:1)"), "{stack}");
    assert!(stack.contains("at outer (test.js:3)"), "{stack}");
}

#[test]
fn deep_nesting_is_an_error_and_not_a_crash() {
    let deep = format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000));
    assert!(error_of(&deep).contains("too deeply"));
    let deep = format!("{}1{}", "[".repeat(100_000), "]".repeat(100_000));
    assert!(error_of(&deep).contains("too deeply"));
    let long = format!("1{}", " + 1".repeat(100_000));
    assert!(error_of(&long).contains("too long"));
    let chain = format!("a{}", ".b".repeat(100_000));
    assert!(error_of(&chain).contains("too long"));
    assert_eq!(
        run(
            "function d(n) { return [1].map(() => n ? d(n - 1) : 0)[0] } try { d(1e6) } catch (e) { console.log(e.name) }"
        ),
        "RangeError"
    );
    assert_eq!(
        run("const a = []; a[0] = a; console.log(String(a) === '', JSON.stringify([[[[1]]]]))"),
        "true [[[[1]]]]"
    );
}

#[test]
fn garbage_is_collected_and_what_is_used_stays() {
    let mut engine = Engine::new();
    let output = Rc::new(RefCell::new(String::new()));
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    engine
        .eval(
            "const kept = [];
             let last;
             for (let i = 0; i < 200000; i++) {
                 const o = { i, s: 'x' + i, a: [i], f: () => i };
                 if (i % 10000 == 0) kept.push(o);
                 last = o;
             }
             const seen = kept.map(o => o.s + o.a[0] + o.f()).join();
             console.log(seen.length, last.f(), kept.length);",
            "gc.js",
        )
        .unwrap();
    assert!(engine.collections() > 0, "nothing was collected");
    let text = output.borrow().clone();
    assert!(text.ends_with("199999 20\n"), "{text}");
    // It shouldn't still be holding on to all that.
    assert!(
        engine.memory().total() < 16 * 1024 * 1024,
        "{:?}",
        engine.memory()
    );
}

/// Text built a piece at a time shares one buffer between its steps, and what's alive counts that buffer once, however
/// many of the steps are kept. Counting each step as text of its own had Lexical's 10,000 paragraphs, which keep about
/// a thousand steps of the document's text while it reconciles, look like 0.8 to 6.5 GB to a collection that came then,
/// and the next one wait for hundreds of MB more.
#[test]
fn text_built_a_piece_at_a_time_counts_once_however_many_steps_are_kept() {
    let (mut engine, _) = engine_with_gc();
    engine
        .eval(
            "globalThis.steps = [];
             globalThis.text = '';
             for (let i = 0; i < 2000; i++) { text += 'x'.repeat(1000); steps.push(text); }
             gc();",
            "steps.js",
        )
        .unwrap();
    // 2 MB of text, and the room it has to grow. The steps' own lengths add up to 2 GB.
    let strings = engine.memory().strings;
    assert!(strings < 16 * 1024 * 1024, "{strings} bytes of strings");
    // And what the collection took to be alive is as little, so with the steps gone, making garbage soon collects again.
    let before = engine.collections();
    engine
        .eval(
            "globalThis.steps = null; globalThis.text = null;
             for (let i = 0; i < 100000; i++) { globalThis.last = { i, s: 'y' + i }; }",
            "garbage.js",
        )
        .unwrap();
    let collections = engine.collections() - before;
    assert!(
        collections >= 2,
        "{collections} collections while making 100,000 objects"
    );
}

#[test]
fn strings_kept_with_the_same_text_share_it_and_stay_as_they_were() {
    crate::share_same_text(true);
    let (mut engine, _) = engine_with_gc();
    engine
        .eval(
            "globalThis.kept = [];
             globalThis.wide = '\u{e9}'.repeat(40);
             for (let i = 0; i < 4000; i++) {
                 kept.push('flex h-7 items-center gap-2 border-b px-3 text-sm ' + (i % 2 ? 'odd' : 'even'));
                 kept.push(wide + (i % 2));
             }
             gc(); gc();",
            "same.js",
        )
        .unwrap();
    // 8,000 strings of 50 to 80 bytes, which are four texts.
    let strings = engine.memory().strings;
    assert!(strings < 600 * 1024, "{strings} bytes of strings");
    let checked = engine
        .eval(
            "const a = kept[0], b = kept[4];
             const longer = a + ' more', other = b + ' else';
             gc();
             [a === b, a.length, longer, other, kept[1] === kept[5], kept[1].length, kept[1].charCodeAt(0),
              kept.every((s, i) => s === (i % 2 ? wide + ((i >> 1) % 2) : 'flex h-7 items-center gap-2 border-b px-3 text-sm ' + ((i >> 1) % 2 ? 'odd' : 'even')))].join()",
            "check.js",
        )
        .unwrap();
    let text = engine.vm().plain_text(checked).unwrap();
    assert_eq!(
        text,
        "true,54,flex h-7 items-center gap-2 border-b px-3 text-sm even more,flex h-7 items-center gap-2 border-b px-3 text-sm even else,true,41,233,true"
    );
}

#[test]
fn a_quiet_collection_gives_back_and_shrinks_and_an_ordinary_one_does_not() {
    let (mut engine, _) = engine_with_gc();
    let make = "globalThis.kept = [];
         for (let i = 0; i < 1000; i++) { const o = {}; for (let k = 0; k < 9; k++) o['p' + k] = k; kept.push(o); }
         globalThis.junk = [];
         for (let i = 0; i < 100000; i++) junk.push({ i });
         for (let i = 0; i < 1000; i++) kept.push({ after: i });
         junk = null;";
    engine.eval(make, "make.js").unwrap();
    engine.collect_garbage();
    let ordinary = engine.footprint();
    // Unless the environment asks for them on every full collection, as a run of all the tests with them on does.
    if std::env::var_os("RSUI_GIVE_BACK").is_none_or(|on| on == "0") {
        assert_eq!(ordinary.objects.given_back_pages, 0, "{ordinary}");
    }
    if std::env::var_os("RSUI_EXACT").is_none_or(|on| on == "0") {
        assert!(ordinary.slots.classed_bytes > ordinary.slots.classed_at_length, "{ordinary}");
    }
    engine.eval(make, "make.js").unwrap();
    engine.collect_garbage_when_quiet();
    let quiet = engine.footprint();
    assert!(quiet.objects.given_back_pages > 1000, "{quiet}");
    assert_eq!(quiet.slots.classed_bytes, quiet.slots.classed_at_length, "{quiet}");
    // And the next ordinary one is ordinary again.
    assert!(!crate::gc::giveback::enabled() || std::env::var_os("RSUI_GIVE_BACK").is_some());
}

#[test]
fn storage_outside_objects_is_shrunk_to_fit_and_still_grows() {
    crate::gc::exact::FORCE.with(|on| on.set(true));
    let (mut engine, output) = engine_with_gc();
    engine
        .eval(
            "globalThis.wide = [];
             for (let i = 0; i < 1000; i++) {
                 const o = {};
                 for (let k = 0; k < 9; k++) o['p' + k] = i * 10 + k;
                 const a = [];
                 for (let k = 0; k < 33; k++) a.push(i + k);
                 wide.push({ o, a });
             }",
            "wide.js",
        )
        .unwrap();
    let before = engine.footprint();
    engine.eval("gc();", "gc.js").unwrap();
    let after = engine.footprint();
    assert!(after.slots.classed_bytes < before.slots.classed_bytes, "{before}
{after}");
    assert!(after.items.classed_bytes < before.items.classed_bytes, "{before}
{after}");
    assert_eq!(after.slots.classed_bytes, after.slots.classed_at_length, "{after}");
    engine
        .eval(
            "let bad = 0;
             for (let i = 0; i < 1000; i++) {
                 const { o, a } = wide[i];
                 for (let k = 0; k < 9; k++) if (o['p' + k] !== i * 10 + k) bad++;
                 for (let k = 0; k < 33; k++) if (a[k] !== i + k) bad++;
                 o.extra = 1; a.push(-1);
                 if (o.extra !== 1 || a.length !== 34 || a[33] !== -1 || a[32] !== i + 32) bad++;
             }
             console.log('bad', bad);",
            "check.js",
        )
        .unwrap();
    assert!(output.borrow().contains("bad 0"), "{}", output.borrow());
    crate::gc::exact::FORCE.with(|on| on.set(false));
}

#[test]
fn pages_with_nothing_left_on_them_are_given_back_and_used_again() {
    crate::gc::giveback::FORCE.with(|on| on.set(true));
    let (mut engine, output) = engine_with_gc();
    engine
        .eval(
            "globalThis.kept = [];
             for (let i = 0; i < 2000; i++) kept.push({ i, name: 'kept ' + i });
             globalThis.junk = [];
             for (let i = 0; i < 200000; i++) { let n = i; junk.push({ i, s: 'junk ' + i, f: () => n }); }
             for (let i = 0; i < 2000; i++) kept.push({ i: i + 2000, name: 'kept ' + (i + 2000) });
             junk = null;
             gc();",
            "give.js",
        )
        .unwrap();
    let given = engine.footprint();
    assert!(given.objects.given_back_pages > 5000, "{given}");
    assert!(given.strings.given_back_pages > 500, "{given}");
    // Making as much again uses the pages given back before the list grows, and what was kept is as it was.
    let places = given.objects.places;
    engine
        .eval(
            "globalThis.again = [];
             for (let i = 0; i < 200000; i++) { let n = i; again.push({ i, s: 'again ' + i, f: () => n }); }
             let bad = 0;
             for (let i = 0; i < 4000; i++) if (kept[i].i !== i || kept[i].name !== 'kept ' + i) bad++;
             for (let i = 0; i < 200000; i += 997) if (again[i].i !== i || again[i].s !== 'again ' + i || again[i].f() !== i) bad++;
             console.log('bad', bad);
             gc();
             again = null;
             gc();",
            "again.js",
        )
        .unwrap();
    assert!(output.borrow().contains("bad 0"), "{}", output.borrow());
    let after = engine.footprint();
    assert!(after.objects.places <= places + 1024, "{given}
{after}");
    assert!(after.objects.given_back_pages > 5000, "{after}");
    // A census walks every place, so it takes them all back first.
    let census = engine.census();
    assert!(census.objects >= 4000);
    assert_eq!(engine.footprint().objects.given_back_pages, 0);
    crate::gc::giveback::FORCE.with(|on| on.set(false));
}

/// An engine with a `gc()` that frees everything nothing can reach, as Node's does when it's run with `--expose-gc`,
/// and what it logs.
fn engine_with_gc() -> (Engine, Rc<RefCell<String>>) {
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let vm = engine.vm();
    let gc = vm.host_function("gc", 0, |vm, _| {
        vm.collect_garbage_now();
        Ok(Value::UNDEFINED)
    });
    vm.set_global("gc", gc);
    (engine, output)
}

/// [`run`], with `gc()` as in Node run with `--expose-gc`.
fn run_with_gc(source: &str) -> String {
    let (mut engine, output) = engine_with_gc();
    let result = engine
        .eval(source, "test.js")
        .and_then(|_| engine.run_until_idle());
    if let Err(error) = result {
        panic!("{source}\nthrew: {error}");
    }
    output.borrow().trim_end().to_owned()
}

/// How many objects are alive, after collecting everything that isn't.
fn live_objects(engine: &mut Engine) -> usize {
    engine.collect_garbage();
    let vm = &engine.vm;
    (0..vm.objects.len())
        .filter(|index| !vm.free_objects.holes().has(*index))
        .filter(|index| !matches!(vm.objects[*index].kind, object::Kind::Free))
        .count()
}

#[test]
fn what_a_module_imports_by_name_is_kept_while_it_is_asked_for() {
    // Asking a host module for a name runs its getter, which collects garbage and then makes a variable a closure keeps.
    // The cell for the import has to be held by then, or that variable's cell is the same one, and is overwritten with
    // what's imported, as `rsui/desktop`'s `updates` once came to be `process.env`'s helper.
    let (mut engine, output) = engine_with_gc();
    engine.add_module(
        "lazy-host",
        "let keep = null;
        Object.defineProperty(exports, 'thing', { enumerable: true, get() { gc(); let mine = 'mine'; keep = () => mine; return 'right'; } });
        exports.check = () => keep();",
    );
    let folder = std::env::temp_dir().join(format!("rsui-js-import-held-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("main.js"),
        "import { thing, check } from 'lazy-host';\nconsole.log(thing, check());",
    )
    .unwrap();
    let ran = engine.run_file(&folder.join("main.js"));
    let _ = std::fs::remove_dir_all(&folder);
    ran.unwrap();
    assert_eq!(output.borrow().trim_end(), "right mine");
}

// What these give is what Node 24 gives with `--expose-gc`. Everything that has to stay is reachable from something
// the callbacks use, as in Node a module's variables that no function uses go when it has run.
#[test]
fn weak_maps_and_weak_refs_let_go_of_what_nothing_else_holds() {
    assert_eq!(
        run_with_gc(
            r#"
            // WeakRef: kept for the rest of the job, gone in the next one.
            let target = { name: 't' };
            const ref = new WeakRef(target);
            target = null;
            gc();
            console.log('same job', ref.deref() !== undefined);

            // A WeakMap's value lives as long as its key does, and no longer.
            const map = new WeakMap();
            const key = {};
            let value = { v: 42 };
            const valueRef = new WeakRef(value);
            map.set(key, value);
            value = null;
            let key2 = {};
            let value2 = { v: 2 };
            const key2Ref = new WeakRef(key2);
            const value2Ref = new WeakRef(value2);
            map.set(key2, value2);
            key2 = value2 = null;

            // A value that refers to its own key doesn't keep it alive.
            let selfKey = {};
            const selfRef = new WeakRef(selfKey);
            map.set(selfKey, { selfKey });
            selfKey = null;

            // Chains: each value is the next key, set up last to first, across two maps.
            const other = new WeakMap();
            let a = { n: 'a' }, b = { n: 'b' }, c = { n: 'c' }, d = { n: 'd' };
            const dRef = new WeakRef(d);
            other.set(c, d);
            map.set(b, c);
            other.set(a, b);
            let head = a;
            a = b = c = d = null;

            // A WeakSet's members go too.
            const set = new WeakSet();
            let member = {};
            const memberRef = new WeakRef(member);
            set.add(member);
            member = null;

            // Symbols can be keys, but not registered ones.
            const sym = Symbol('s');
            map.set(sym, 'by symbol');
            try { map.set(Symbol.for('r'), 1); } catch (e) { console.log(e.constructor.name); }

            setTimeout(() => {
              gc();
              console.log('next job', ref.deref());
              console.log('value kept by key', valueRef.deref() && valueRef.deref().v, map.get(key).v);
              console.log('key and value gone', key2Ref.deref(), value2Ref.deref());
              console.log('self key gone', selfRef.deref());
              console.log('chain kept', dRef.deref() && dRef.deref().n, other.has(head));
              console.log('member gone', memberRef.deref(), set.has({}));
              console.log('symbol', map.get(sym), map.has(sym));
              head = null;
              setTimeout(() => {
                gc();
                console.log('chain gone', dRef.deref());
              }, 0);
            }, 0);
            "#
        ),
        "same job true\nTypeError\nnext job undefined\nvalue kept by key 42 42\nkey and value gone undefined undefined\n\
         self key gone undefined\nchain kept d true\nmember gone undefined false\nsymbol by symbol true\nchain gone undefined"
    );
}

#[test]
fn finalization_registries_call_back_once_their_targets_have_gone() {
    assert_eq!(
        run_with_gc(
            r#"
            // The callback gets what was registered, once the target has gone, as a job of its own.
            const calls = [];
            const registry = new FinalizationRegistry((held) => calls.push(held));
            let one = {}, two = {}, three = {};
            const token = {};
            registry.register(one, 'one');
            registry.register(two, 'two', token);
            registry.register(three, 'three', three);
            console.log('unregister unknown', registry.unregister({}));
            console.log('unregister token', registry.unregister(token));
            one = two = three = null;

            // A registry that has gone calls nothing. Its target outlives it.
            let lostTarget = {};
            (function () { new FinalizationRegistry(() => console.log('lost registry called')).register(lostTarget, 'x'); })();

            // What a WeakRef handed out stays across an await, which carries on in the same turn.
            async function across() {
              let o = {};
              const r = new WeakRef(o);
              o = null;
              await null;
              gc();
              console.log('after await', r.deref() !== undefined);
              return r;
            }

            across().then((r) => {
              setTimeout(() => {
                gc();
                console.log('next turn', r.deref());
              }, 0);
            });

            setTimeout(() => {
              // Not while collecting, nor in the turn it collected in (with a collection at every safe point, these
              // targets can have gone before this, which is why it's a comparison).
              const before = calls.length;
              gc();
              lostTarget = null;
              console.log('not while collecting', calls.length === before, registry.unregister(token));
              Promise.resolve().then(() => console.log('nor in that turn', calls.length === before));
              setTimeout(() => {
                console.log('calls', calls.sort().join());
                // A registry only its own callback would use goes before it's called.
                let t = {};
                const r2 = new FinalizationRegistry((h) => console.log('called', h));
                r2.register(t, 'held', t);
                t = null;
                setTimeout(() => {
                  gc();
                  setTimeout(() => console.log('done'), 0);
                }, 0);
              }, 10);
            }, 0);
            "#
        ),
        "unregister unknown false\nunregister token true\nafter await true\nnot while collecting true false\n\
         nor in that turn true\nnext turn undefined\ncalls one,three\ndone"
    );
}

#[test]
fn a_registry_can_be_unregistered_from_while_its_callback_is_due() {
    // Each callback is called once, in an order the language leaves open (and Node's has no pattern to).
    assert_eq!(
        run_with_gc(
            "const order = [];
             const registry = new FinalizationRegistry((held) => order.push(held));
             (function () { for (let i = 0; i < 6; i++) registry.register({}, i); })();
             setTimeout(() => { gc(); setTimeout(() => console.log(order.sort().join(), registry.unregister({})), 5); }, 0);"
        ),
        "0,1,2,3,4,5 false"
    );
    // Two targets go together, and the first callback takes the other off: it's not called.
    assert_eq!(
        run_with_gc(
            r#"
            const seen = [];
            const tokens = { first: {}, second: {} };
            const registry = new FinalizationRegistry((held) => {
              seen.push(held);
              console.log('unregisters the other', registry.unregister(held === 'first' ? tokens.second : tokens.first));
            });
            let a = {}, b = {};
            registry.register(a, 'first', tokens.first);
            registry.register(b, 'second', tokens.second);
            a = b = null;
            setTimeout(() => { gc(); setTimeout(() => console.log(seen.length, registry.unregister({})), 5); }, 0);
            "#
        ),
        "unregisters the other true\n1 false"
    );
    // A callback that throws stops that turn, and the error is reported, as an uncaught one is.
    let (mut engine, _) = engine_with_gc();
    let result = engine
        .eval(
            "const r = new FinalizationRegistry(() => { throw new Error('from the callback'); });
             (function () { r.register({}, 1); })();
             setTimeout(() => { gc(); setTimeout(() => console.log('timer', r), 5); }, 0);",
            "test.js",
        )
        .and_then(|_| engine.run_until_idle());
    assert!(
        result.is_err_and(|error| error.message.contains("from the callback")),
        "the error should have come out"
    );
}

#[test]
fn a_long_lived_weak_map_does_not_keep_short_lived_keys() {
    let (mut engine, output) = engine_with_gc();
    engine
        .eval(
            "var rendered = new WeakMap();
             var kept = [];
             function render(from) {
               let total = 0;
               for (let i = from; i < from + 100000; i++) {
                 const node = { i, children: [{ i }] };
                 rendered.set(node, { tag: 'div', node });
                 if (i % 25000 == 0) kept.push(node);
                 total += node.i;
               }
               return total;
             }
             gc(); gc();",
            "test.js",
        )
        .unwrap();
    let before = live_objects(&mut engine);
    engine.eval("console.log(render(0))", "a.js").unwrap();
    // Small collections have been letting go of the entries all along.
    let map = weak_map_of(&mut engine);
    let object::Kind::WeakMap(data) = &engine.vm.objects[map].kind else {
        panic!("not a weak map");
    };
    assert!(
        (data.live as usize) < 50000,
        "{} entries are still there",
        data.live
    );
    let after = live_objects(&mut engine);
    engine.eval("console.log(render(100000))", "b.js").unwrap();
    let again = live_objects(&mut engine);
    // What's kept is four nodes each time, each with its child and its entry's value.
    assert!(after <= before + 4 * 3 + 16, "{before} became {after}");
    assert!(again <= after + 4 * 3 + 16, "{after} became {again}");
    let map = weak_map_of(&mut engine);
    let object::Kind::WeakMap(data) = &engine.vm.objects[map].kind else {
        panic!("not a weak map");
    };
    assert_eq!(data.live, 8);
    assert!(
        data.entries.capacity() < 1024,
        "{}",
        data.entries.capacity()
    );
    engine
        .eval(
            "console.log(kept.every((node) => rendered.get(node).node === node && rendered.get(node).tag === 'div'))",
            "c.js",
        )
        .unwrap();
    assert_eq!(output.borrow().as_str(), "4999950000\n14999950000\ntrue\n");
}

/// A function's variables that a small collection kept while it was running stay young, so once it's gone, what they
/// held dies young too. They used to be old as soon as they were kept, and an old variable that holds something young is
/// looked at by every small collection until that's old, whether anything still has the variable or not.
#[test]
fn what_a_finished_function_s_variables_held_dies_young() {
    let (mut engine, output) = engine_with_gc();
    let turn = |engine: &mut Engine, source: &str| {
        engine.eval(source, "test.js").unwrap();
        engine.run_jobs().unwrap();
    };
    turn(&mut engine, "var ref; var it;");
    engine.collect_garbage();
    // The generator waits with `held` in a variable a closure has, which the small collection keeps.
    turn(
        &mut engine,
        "function* work() { let held = { big: 'x'.repeat(100) + 'y' }; ref = new WeakRef(held); const touch = () => held; yield touch; }
         it = work(); it.next();",
    );
    engine.vm.collect_young_garbage();
    turn(&mut engine, "it = null;");
    engine.vm.collect_young_garbage();
    engine.vm.collect_young_garbage();
    // With a collection at every safe point, what it held may well have become old already.
    if std::env::var_os("RSUI_GC_STRESS").is_some() {
        engine.collect_garbage();
    }
    turn(&mut engine, "console.log(ref.deref() === undefined);");
    assert_eq!(output.borrow().as_str(), "true\n");
}

/// A string kept once by a small collection stays young, so one that only a young object had dies with it at the next.
#[test]
fn a_string_kept_once_by_a_small_collection_is_still_young() {
    let (mut engine, _output) = engine_with_gc();
    engine.eval("var holder = null;", "test.js").unwrap();
    engine.collect_garbage();
    engine
        .eval("holder = Array.from({ length: 100 }, (_, at) => 'kept for a while, number ' + at);", "test.js")
        .unwrap();
    let strings = |engine: &mut Engine| {
        let strings = &engine.vm.strings;
        (0..strings.items.len()).filter(|&at| !strings.free.is_free(at)).count()
    };
    engine.vm.collect_young_garbage();
    let kept = strings(&mut engine);
    engine.eval("holder = null;", "test.js").unwrap();
    engine.vm.collect_young_garbage();
    if std::env::var_os("RSUI_GC_STRESS").is_none() {
        assert!(strings(&mut engine) + 100 <= kept, "{} strings, {kept} before", strings(&mut engine));
    }
}

#[test]
fn weak_holders_that_are_old_let_go_of_young_things_in_small_collections() {
    let (mut engine, output) = engine_with_gc();
    let turn = |engine: &mut Engine, source: &str| {
        engine.eval(source, "test.js").unwrap();
        // What `WeakRef`s handed out is let go of at the end of the turn.
        engine.run_jobs().unwrap();
    };
    turn(
        &mut engine,
        "var map = new WeakMap(); var set = new WeakSet(); var oldKey = { old: true }; var keep = {};
         var calls = []; var registry = new FinalizationRegistry((held) => calls.push(held));",
    );
    // Everything there is now is old.
    engine.collect_garbage();
    // The old map, set and registry are given young keys, values and targets, one of which nothing else holds, and a
    // chain of young keys and values that only the first key leads to.
    turn(
        &mut engine,
        "var youngKey = { young: true };
         map.set(youngKey, { v: 'young value' }); map.set(oldKey, { v: 'young value of an old key' });
         set.add(youngKey); registry.register(youngKey, 'young key', youngKey);
         (function () { const dying = {}; map.set(dying, { v: 'dies' }); set.add(dying); registry.register(dying, 'dying'); })();
         var chainHead = {};
         (function () { let key = chainHead; for (let i = 0; i < 5; i++) { const next = { i }; map.set(key, next); key = next; } keep.last = new WeakRef(key); })();",
    );
    engine.vm.collect_young_garbage();
    turn(
        &mut engine,
        "console.log(map.get(youngKey).v, map.get(oldKey).v, set.has(youngKey), keep.last.deref().i);",
    );
    let weak_live = |engine: &mut Engine, name: &str| {
        let handle = engine.vm().get_global(name).unwrap().index();
        match &engine.vm.objects[handle].kind {
            object::Kind::WeakMap(data) | object::Kind::WeakSet(data) => data.live,
            _ => panic!("{name} isn't a weak map or set"),
        }
    };
    // The one that died, and nothing else: oldKey, youngKey and the chain's five.
    assert_eq!(weak_live(&mut engine, "map"), 7);
    assert_eq!(weak_live(&mut engine, "set"), 1);
    // The young ones have been kept once, and die before they're old.
    turn(&mut engine, "youngKey = null; chainHead = null;");
    engine.vm.collect_young_garbage();
    // With a collection at every safe point, they may well have become old already.
    if std::env::var_os("RSUI_GC_STRESS").is_some() {
        engine.collect_garbage();
    }
    assert_eq!(weak_live(&mut engine, "map"), 1);
    assert_eq!(weak_live(&mut engine, "set"), 0);
    turn(&mut engine, "");
    turn(
        &mut engine,
        "console.log(calls.sort().join(), keep.last.deref(), map.get(oldKey).v, registry.unregister(oldKey));",
    );
    assert_eq!(
        output.borrow().as_str(),
        "young value young value of an old key true 4\ndying,young key undefined young value of an old key false\n"
    );
}

/// Code that makes a class, or a prototype, each time it's called keeps none of them once they're no longer used: not
/// the prototypes, which the shapes of what they made used to keep, nor the shapes. What it prints is what Node prints.
#[test]
fn prototypes_and_shapes_made_by_each_call_go_with_what_used_them() {
    let (mut engine, output) = engine_with_gc();
    engine
        .eval(
            "function make(i) { class PerCall { constructor() { this.i = i; } get twice() { return this.i * 2; } } return new PerCall().twice; }
             function fresh(i) { const proto = { kind: 'p' + i, hello() { return this.kind; } }; return Object.create(proto).hello(); }
             // An object used as a dictionary, with a new name each time, which gives each a shape of its own.
             function named(i) { const o = {}; o['key' + i] = i; o['other' + (i % 7)] = 1; return Object.keys(o).length; }
             var total = 0, last = '';
             function run(from, to) { for (let i = from; i < to; i++) { total += make(i) + named(i); last = fresh(i); } }
             run(0, 1000);",
            "test.js",
        )
        .unwrap();
    let before = engine.census();
    engine.eval("run(1000, 11000);", "more.js").unwrap();
    let after = engine.census();
    engine.eval("console.log(total, last)", "show.js").unwrap();
    let shown = format!("before:\n{before}\nafter:\n{after}");
    assert_eq!(before.unexplained, 0, "{shown}");
    assert_eq!(after.unexplained, 0, "{shown}");
    assert_eq!(after.objects, before.objects, "{shown}");
    assert_eq!(after.shapes, before.shapes, "{shown}");
    assert_eq!(after.count("PerCall"), 0, "{shown}");
    assert!(
        engine.vm.shapes.items.len() < before.shapes * 2 + 64,
        "the list of shapes has {} places for {} shapes",
        engine.vm.shapes.items.len(),
        after.shapes
    );
    assert_eq!(output.borrow().as_str(), "121011000 p10999\n");
}

/// A place in the code that remembers objects of a shape, a read that found something on a prototype, one that has seen
/// so many shapes that it goes by what reads remember, and a write that adds a property, all still get it right once the
/// shapes they remember have gone and their places have been given to new shapes. Node prints the same.
#[test]
fn what_places_in_the_code_remember_about_shapes_that_have_gone_is_forgotten() {
    let (mut engine, output) = engine_with_gc();
    let vm = engine.vm();
    let shape_of = vm.host_function("shapeOf", 1, |vm, args| {
        let value = args.get(vm, 0);
        Ok(Value::int(vm.objects[value.index()].shape as i32))
    });
    vm.set_global("shapeOf", shape_of);
    let turn = |engine: &mut Engine, source: &str| {
        engine.eval(source, "test.js").unwrap();
        engine.run_jobs().unwrap();
    };
    turn(
        &mut engine,
        r#"
            var shapes = [], seen = [], fill = [];
            function make(kind) {
              const p = { tag: kind };
              const o = Object.create(p);
              if (kind === 1) { o.a = 1; o.b = 2; }
              else if (kind === 2) { o.b = 20; o.a = 10; }
              else { o.c = 300; o.b = 200; o.a = 100; }
              return o;
            }
            function read(o) { return o.a * 1000 + o.b; }
            function readTag(o) { return o.tag; }
            function readMany(o) { return o.a; }
            function setE(o) { o.e = 9; return o; }
            // A place that's seen more shapes than it has room for goes by what reads remember.
            var kept = [];
            for (let i = 0; i < 6; i++) { const many = { ['x' + i]: i, a: i }; kept.push(many); readMany(many); }
            function step(kind) {
              const o = make(kind);
              shapes.push(shapeOf(o));
              seen.push(read(o), readTag(o), readMany(o), Object.keys(setE(o)).join(''), o.e);
            }
            step(1);
            "#,
    );
    let shapes = engine.vm().get_global("shapes").unwrap();
    let first = engine.vm().get_index(shapes, 0).unwrap().as_int() as u32;
    let remembers = |engine: &Engine, shape: u32| {
        engine
            .vm
            .caches
            .iter()
            .any(|entry| entry.shape == shape || entry.other == shape)
            || engine
                .vm
                .lookups
                .iter()
                .any(|found| found.shape == shape || found.holder == shape)
    };
    assert!(
        remembers(&engine, first),
        "nothing remembers the first object's shape"
    );
    // The first object has gone, and so have its prototype and shapes, and nothing remembers them.
    engine.collect_garbage();
    assert!(engine.vm.shapes.is_free(first), "its shape is still there");
    assert!(!remembers(&engine, first), "its shape is still remembered");
    // New objects with prototypes of their own, and shapes, take the places the first one's had.
    turn(
        &mut engine,
        "for (let i = 0; i < 50; i++) fill.push(make(2 + i % 2));
         let sum = 0;
         for (const o of fill) sum += read(o) + readTag(o) + readMany(o);
         step(2); step(3); step(1);
         console.log(seen.join(' '), sum, Object.keys(setE(fill[0])).join(''), Object.keys(setE(fill[1])).join(''));",
    );
    assert!(
        !engine.vm.shapes.is_free(first),
        "nothing has been given the first shape's place"
    );
    assert_eq!(
        output.borrow().as_str(),
        "1002 1 1 abe 9 10020 2 10 bae 9 100200 3 100 cbae 9 1002 1 1 abe 9 2758375 bae cbae\n"
    );
}

/// Names worked out while running (set and deleted, or read from JSON), symbols, and private names in a class made by
/// each call are kept only while something has them, along with their strings. What it prints is what Node prints.
#[test]
fn names_and_symbols_made_by_each_call_go_once_nothing_has_them() {
    let (mut engine, output) = engine_with_gc();
    engine
        .eval(
            r#"var total = 0, last = '';
             function keys(i) { const o = {}; o['key' + i] = i; delete o['key' + i]; o['k' + i] = 2; return o['k' + i]; }
             function parsed(i) { const o = JSON.parse('{"j' + i + '": ' + i + ', "same": 1}'); return o['j' + i] + o.same; }
             function symbols(i) { const s = Symbol('s' + i); const o = { [s]: i }; return o[s] + (Symbol().description === undefined ? 1 : 0); }
             function privates(i) { class P { #x = i; static read(p) { return p.#x; } } return P.read(new P()); }
             function run(from, to) { for (let i = from; i < to; i++) { total += keys(i) + parsed(i) + symbols(i) + privates(i); last = Object.keys(JSON.parse('{"z' + i + '":1}'))[0]; } }
             run(0, 1000);"#,
            "test.js",
        )
        .unwrap();
    let before = engine.census();
    // The same file name, as the name of a file stays.
    engine.eval("run(1000, 11000);", "test.js").unwrap();
    let after = engine.census();
    engine.eval("console.log(total, last)", "test.js").unwrap();
    let shown = format!("before:\n{before}\nafter:\n{after}");
    eprintln!("{shown}");
    assert_eq!(after.unexplained, 0, "{shown}");
    assert_eq!(after.atoms, before.atoms, "{shown}");
    assert_eq!(after.strings, before.strings, "{shown}");
    assert!(
        after.atom_places < before.atoms + 256,
        "the list of atoms has {} places for {} atoms",
        after.atom_places,
        after.atoms
    );
    assert_eq!(output.borrow().as_str(), "181527500 z10999\n");
}

/// An async function that a job carries on with runs as the job does, with no native holding anything for it, so the
/// names and symbols it makes go while it runs, not only once it's done: the list of atoms doesn't get anywhere near
/// the 30,000 it makes.
#[test]
fn atoms_go_while_an_async_function_runs_after_it_awaits() {
    let (mut engine, output) = engine_with_gc();
    engine
        .eval(
            r#"var total = 0;
             async function work(from, to) {
               await null;
               for (let i = from; i < to; i++) {
                 const o = {}; o['a' + i] = i; delete o['a' + i];
                 total += JSON.parse('{"b' + i + '": 1}')['b' + i] + Symbol('c' + i).description.length;
               }
             }
             work(0, 1000);"#,
            "test.js",
        )
        .unwrap();
    engine.run_jobs().unwrap();
    let before = engine.census();
    engine.eval("work(1000, 11000);", "test.js").unwrap();
    engine.run_jobs().unwrap();
    let most = engine.vm.atoms.len();
    let after = engine.census();
    engine.eval("console.log(total)", "test.js").unwrap();
    eprintln!("atoms {} -> {}, {most} places at the end of the loop", before.atoms, after.atoms);
    assert_eq!(after.atoms, before.atoms);
    assert!(
        most < before.atoms + 10_000,
        "{most} places for atoms at the end, from {}",
        before.atoms
    );
    assert_eq!(output.borrow().as_str(), "65890\n");
}

/// A string that remembers an atom that's gone forgets it, so once its place is given to another name, the string isn't
/// taken for that name. Without that, reading `made[0][kept]` would find `fresh0`.
#[test]
fn an_atom_that_has_gone_is_not_taken_for_the_one_given_its_place() {
    let (mut engine, output) = engine_with_gc();
    engine
        .eval(
            "var kept = 'name' + 1; var probe = {}; probe[kept] = 1; delete probe[kept];",
            "test.js",
        )
        .unwrap();
    let kept = engine.vm().get_global("kept").unwrap();
    let atom = Atom(engine.vm.strings.items[kept.index()].atom);
    assert_ne!(atom.0, 0, "the string doesn't remember its atom");
    engine.collect_garbage();
    assert!(engine.vm.atoms.is_free(atom), "the atom is still there");
    assert_eq!(engine.vm.strings.items[kept.index()].atom, 0);
    engine
        .eval(
            "var made = []; for (let i = 0; i < 50; i++) { const o = {}; o['fresh' + i] = i; made.push(o); }",
            "more.js",
        )
        .unwrap();
    assert!(
        !engine.vm.atoms.is_free(atom),
        "nothing has been given the atom's place"
    );
    engine
        .eval(
            "const t = {}; t[kept] = 7;
             console.log(probe[kept], made.filter(o => o[kept] !== undefined).length, Object.keys(made[0]).join(), Object.keys(t).join(), t[kept], kept in made[0]);",
            "show.js",
        )
        .unwrap();
    assert_eq!(output.borrow().as_str(), "undefined 0 fresh0 name1 7 false\n");
}

/// A symbol nothing else has is held weakly by a `WeakMap`, a `WeakSet` and a `WeakRef`, and goes from them once it has
/// gone, before its place is given to another symbol.
#[test]
fn weak_holders_let_go_of_symbols_that_have_gone() {
    let (mut engine, output) = engine_with_gc();
    engine
        .eval(
            "var map = new WeakMap(), set = new WeakSet(), kept = Symbol('kept'), ref;
             (function () { const gone = Symbol('gone'); map.set(gone, { big: 1 }); set.add(gone); ref = new WeakRef(gone); })();
             // A value that has its own key doesn't keep the entry.
             (function () { const own = Symbol('own'); map.set(own, { own }); })();
             map.set(kept, 'still'); set.add(kept); var keptRef = new WeakRef(kept);",
            "test.js",
        )
        .unwrap();
    engine.run_jobs().unwrap();
    engine.collect_garbage();
    engine.vm.verify_atoms("after the symbols went");
    let map = engine.vm().get_global("map").unwrap();
    let object::Kind::WeakMap(data) = &engine.vm.objects[map.index()].kind else {
        panic!("map isn't a WeakMap");
    };
    assert_eq!(data.live, 1, "the map still has entries for symbols that went");
    engine
        .eval(
            "const fresh = []; for (let i = 0; i < 20; i++) fresh.push(Symbol('fresh' + i));
             console.log(ref.deref(), keptRef.deref().description, map.get(kept), set.has(kept), fresh.filter(s => map.has(s) || set.has(s)).length);",
            "show.js",
        )
        .unwrap();
    assert_eq!(output.borrow().as_str(), "undefined kept still true 0\n");
}

/// A native that's going through names, or was given one, keeps it while what it calls frees everything else that
/// nothing has: the names a proxy's `ownKeys` gave, and the keys `Object.assign` found before a getter deleted them.
/// Node prints the same.
#[test]
fn natives_keep_the_names_they_hold_while_javascript_they_call_collects() {
    assert_eq!(
        run_with_gc(
            "const src = {};
             for (let i = 0; i < 5; i++) src['p' + i + 'x'] = i;
             Object.defineProperty(src, 'p0x', { get() { for (let j = 1; j < 5; j++) delete src['p' + j + 'x']; gc(); for (let j = 0; j < 50; j++) ({})['q' + j] = 1; return 'g'; }, enumerable: true, configurable: true });
             const target = Object.assign({}, src);
             let n = 5;
             const p = new Proxy({}, { ownKeys() { return ['dyn' + n, 'dyn' + (n + 1)]; }, getOwnPropertyDescriptor(t, k) { gc(); for (let j = 0; j < 30; j++) ({})['junk' + j + n] = 1; return { value: k, enumerable: true, configurable: true }; } });
             console.log(JSON.stringify(target), Object.keys(p).join());"
        ),
        r#"{"p0x":"g"} dyn5,dyn6"#
    );
}

/// `JSON.stringify` remembers how to write objects of each shape it sees, and what it writes can run code that frees
/// shapes and gives their places to others, which it then mustn't take for the shapes it remembers.
#[test]
fn json_doesnt_take_a_new_shape_for_one_that_has_gone() {
    assert_eq!(
        run_with_gc(
            "function piece(i) { const o = Object.create({ from: i }); o['k' + (i % 3)] = i; o.v = 'v' + i; return o; }
             const list = [];
             for (let i = 0; i < 8; i++) list.push(i % 2 ? piece(i) : { toJSON() { gc(); return [piece(i), piece(i + 10)]; } });
             console.log(JSON.stringify({ list, after: piece(99) }));"
        ),
        r#"{"list":[[{"k0":0,"v":"v0"},{"k1":10,"v":"v10"}],{"k1":1,"v":"v1"},[{"k2":2,"v":"v2"},{"k0":12,"v":"v12"}],{"k0":3,"v":"v3"},[{"k1":4,"v":"v4"},{"k2":14,"v":"v14"}],{"k2":5,"v":"v5"},[{"k0":6,"v":"v6"},{"k1":16,"v":"v16"}],{"k1":7,"v":"v7"}],"after":{"k0":99,"v":"v99"}}"#
    );
}

/// Where the `rendered` weak map of `a_long_lived_weak_map_does_not_keep_short_lived_keys` is.
fn weak_map_of(engine: &mut Engine) -> usize {
    let vm = engine.vm();
    vm.get_global("rendered").unwrap().index()
}

#[test]
fn collecting_during_a_callback_keeps_what_natives_are_holding() {
    assert_eq!(
        run("const r = Array.from({ length: 2000 }, (_, i) => i).map(i => { const junk = []; for (let j = 0; j < 200; j++) junk.push({ j }); return { i, s: 'v' + i } }); console.log(r[1999].s, r[0].i, r.length);
             const sorted = Array.from({ length: 3000 }, (_, i) => ({ v: 'k' + (3000 - i) })).sort((a, b) => { const j = { a, b }; return a.v < b.v ? -1 : a.v > b.v ? 1 : 0 }); console.log(sorted[0].v);
             function* gen() { for (let i = 0; i < 2000; i++) { const junk = new Array(100).fill({ i }); yield 'g' + junk[0].i } } let n = 0; for (const s of gen()) n += s.length; console.log(n);
             const m = new Map(); for (let i = 0; i < 3000; i++) m.set('key' + i, { i }); let t = 0; m.forEach((v, k) => { const junk = new Array(50).fill(k); t += v.i + junk.length }); console.log(t, m.get('key5').i);
             const big = {}; for (let i = 0; i < 5000; i++) big['p' + i] = { i, n: 'x'.repeat(50) }; let c = 0; for (const k in big) c += big[k].n.length; console.log(c, JSON.stringify(big).length > 1000);"),
        "v1999 0 2000\nk1\n8890\n4648500 5\n250000 true"
    );
}

#[test]
fn async_functions_survive_collections_while_they_wait() {
    assert_eq!(
        run(
            "async function work(n) { const here = { n, s: 'held' + n }; await null; const junk = []; for (let i = 0; i < 3000; i++) junk.push({ i }); await null; return here.s + here.n } Promise.all(Array.from({ length: 50 }, (_, i) => work(i))).then(r => console.log(r[49], r.length))"
        ),
        "held4949 50"
    );
}

#[test]
fn an_empty_engine_stays_small() {
    let engine = Engine::new();
    let memory = engine.memory();
    assert!(
        memory.total() < 300 * 1024,
        "{memory:?} is {} bytes",
        memory.total()
    );
}

#[test]
fn files_read_ahead_run_as_they_would_have() {
    let folder = std::env::temp_dir().join(format!("rsui-js-ahead-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let files = [
        (
            "main.js",
            "import { a } from './a.js'; import b from './b.cjs'; import c from './c.js'; import d from './d.cjs';\nconsole.log(a, b.b, c, d.broken, d.late);",
        ),
        (
            "d.cjs",
            "let broken; try { require('./broken.js'); } catch (error) { broken = error.name; }\nmodule.exports = { broken, late: typeof require('./late.js') };",
        ),
        ("a.js", "\u{FEFF}export const a = 'module';"),
        ("b.cjs", "#!/usr/bin/env node\nexports.b = 'commonjs';"),
        // It says `import`, but only in a string, so it's CommonJS.
        (
            "c.js",
            "module.exports = 'not a module, though it says import';",
        ),
        ("broken.js", "this is not javascript"),
        ("late.js", "module.exports = {};"),
    ];
    for (name, text) in files {
        std::fs::write(folder.join(name), text).unwrap();
    }
    // In the order they're run, but for the last, which is wanted before it would be read.
    let order: Vec<std::path::PathBuf> = ["main.js", "a.js", "b.cjs", "c.js", "d.cjs", "broken.js"]
        .iter()
        .map(|name| folder.join(name))
        .chain([folder.join("late.js")])
        .collect();
    crate::ahead::start(order);
    crate::ahead::wait_until_read();
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let ran = engine.run_file(&folder.join("main.js"));
    let _ = std::fs::remove_dir_all(&folder);
    ran.unwrap();
    assert_eq!(
        output.borrow().trim_end(),
        "module commonjs not a module, though it says import SyntaxError object"
    );
    // All but the one that isn't JavaScript, which is read again where it's run, to say why.
    assert_eq!(
        crate::ahead::USED.load(std::sync::atomic::Ordering::Relaxed),
        6
    );
}

#[test]
fn a_file_written_after_its_folder_was_looked_in_can_still_be_imported() {
    let said = run_modules(
        "written-later",
        &[
            (
                "main.js",
                "const fs = require('fs'); const path = require('path');
let before; try { require('./later.js'); } catch (error) { before = error.code; }
fs.writeFileSync(path.join(__dirname, 'later.js'), 'module.exports = \"there\";');
const result = before + ' ' + require('./later.js');",
            ),
            ("other.js", "module.exports = 1;"),
        ],
    );
    assert_eq!(said, "MODULE_NOT_FOUND there");
}

#[test]
fn a_file_asked_for_with_other_letters_is_the_same_module() {
    let said = run_modules(
        "letters",
        &[
            (
                "main.js",
                "const a = require('./Thing.js'); const b = require('./thing.js'); const c = require('./sub/../THING');
const result = (a === b) + ' ' + (b === c) + ' ' + a.count;",
            ),
            ("thing.js", "globalThis.loads = (globalThis.loads || 0) + 1; exports.count = globalThis.loads;"),
        ],
    );
    assert_eq!(said, "true true 1");
}

#[test]
fn a_package_is_found_past_folders_that_have_none() {
    let folder = std::env::temp_dir().join(format!("rsui-js-packages-{}", std::process::id()));
    let deep = folder.join("src").join("a").join("b");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::create_dir_all(folder.join("node_modules").join("thing").join("lib")).unwrap();
    std::fs::write(
        folder
            .join("node_modules")
            .join("thing")
            .join("package.json"),
        r#"{ "main": "lib/start" }"#,
    )
    .unwrap();
    std::fs::write(
        folder
            .join("node_modules")
            .join("thing")
            .join("lib")
            .join("start.js"),
        "module.exports = 'found';",
    )
    .unwrap();
    std::fs::write(
        deep.join("main.js"),
        "console.log(require('thing'), require('thing') === require('../../../node_modules/thing/lib/start.js'));",
    )
    .unwrap();
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let ran = engine.run_file(&deep.join("main.js"));
    let _ = std::fs::remove_dir_all(&folder);
    ran.unwrap();
    assert_eq!(output.borrow().trim_end(), "found true");
}

#[test]
fn modules_import_and_export() {
    let dir = std::env::temp_dir().join(format!("rsui_js_modules_{}", std::process::id()));
    let package = dir.join("node_modules").join("pkg");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        dir.join("main.ts"),
        "import dflt, { named as other, count, inc } from './lib';\n\
         import * as all from './lib';\n\
         import cjs, { hello } from './cjs.js';\n\
         import data from './data.json';\n\
         import pkg from 'pkg';\n\
         export const out: number = 1;\n\
         inc();\n\
         console.log(dflt, other, all.named, cjs.hello, hello, data.a, pkg);\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("lib.ts"),
        "export default 'd'; export const named = 'n'; export let count = 0; export function inc() { count++ }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("cjs.js"),
        "const self = require('./cjs2'); module.exports = { hello: self.v + __filename.length * 0 };\n",
    )
    .unwrap();
    std::fs::write(dir.join("cjs2.js"), "exports.v = 'hi';\n").unwrap();
    std::fs::write(dir.join("data.json"), "{\"a\": 5}").unwrap();
    std::fs::write(package.join("package.json"), "{\"main\": \"lib/start.js\"}").unwrap();
    std::fs::create_dir_all(package.join("lib")).unwrap();
    std::fs::write(
        package.join("lib").join("start.js"),
        "module.exports = 'from a package';\n",
    )
    .unwrap();

    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let result = engine.run_file(&dir.join("main.ts"));
    let shown = result.map(|_| ()).map_err(|error| error.message);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(shown, Ok(()));
    assert_eq!(output.borrow().trim_end(), "d n n hi0 hi0 5 from a package");
}

#[test]
fn files_can_be_written_read_and_removed() {
    let dir = std::env::temp_dir().join(format!("rsui_js_files_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.js");
    let folder = dir.join("made").join("deep");
    let source = format!(
        r#"
        const fs = require('fs');
        const path = require('path');
        const folder = {folder:?};
        fs.mkdirSync(folder, {{ recursive: true }});
        const file = path.join(folder, 'a.txt');
        console.log(fs.existsSync(file));
        fs.writeFileSync(file, 'héllo');
        fs.appendFileSync(file, ' wörld');
        console.log(fs.readFileSync(file, 'utf8'), fs.statSync(file).isFile(), fs.statSync(folder).isDirectory());
        console.log(fs.statSync(file).size);
        fs.renameSync(file, file + '2');
        console.log(fs.readdirSync(folder).join());
        try {{ fs.readFileSync(file); }} catch (error) {{ console.log(error.code, error.syscall, error instanceof Error); }}
        fs.rmSync(file, {{ force: true }});
        fs.rmSync(path.dirname(folder), {{ recursive: true }});
        console.log(fs.existsSync(folder));
        fs.promises.readFile(file).catch((error) => console.log('rejected', error.code));
        "#
    );
    std::fs::write(&main, source).unwrap();
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let result = engine
        .run_file(&main)
        .map(|_| ())
        .and_then(|()| engine.run_until_idle())
        .map_err(|error| error.message);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(result, Ok(()));
    let shown = output.borrow().replace("\r\n", "\n");
    assert_eq!(
        shown.trim_end(),
        "false\nhéllo wörld true true\n13\na.txt2\nENOENT open true\nfalse\nrejected ENOENT"
    );
}

/// Runs a file with all that it needs, and says what it printed.
fn run_as_file(name: &str, source: &str) -> String {
    let dir = std::env::temp_dir().join(format!("rsui_js_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.js");
    std::fs::write(&main, source).unwrap();
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let result = engine
        .run_file(&main)
        .map(|_| ())
        .and_then(|()| engine.run_until_idle())
        .map_err(|error| error.message);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(result, Ok(()));
    let shown = output.borrow().replace("\r\n", "\n");
    shown.trim_end().to_owned()
}

#[test]
fn files_can_be_read_and_written_on_another_thread() {
    let folder = std::env::temp_dir().join(format!("rsui_js_files_later_{}", std::process::id()));
    let source = format!(
        r#"
        const {{ promises: fs }} = require('fs');
        const path = require('path');
        (async () => {{
            const folder = {folder:?};
            await fs.mkdir(folder, {{ recursive: true }});
            const file = path.join(folder, 'a.txt');
            await fs.writeFile(file, 'hello');
            await fs.appendFile(file, ' there');
            console.log(await fs.readFile(file, 'utf8'));
            console.log((await fs.stat(file)).size, (await fs.stat(folder)).isDirectory());
            console.log((await fs.readdir(folder)).join());
            await fs.rename(file, file + '2');
            await fs.readFile(file).catch((error) => console.log(error.code, error instanceof Error));
            const all = await Promise.all([1, 2, 3, 4, 5, 6, 7, 8].map((n) => fs.writeFile(path.join(folder, n + '.txt'), String(n))));
            console.log(all.length, (await fs.readdir(folder)).length);
            await fs.rm(folder, {{ recursive: true }});
            console.log('done');
        }})();
        console.log('this comes first');
        "#
    );
    assert_eq!(
        run_as_file("later", &source),
        "this comes first\nhello there\n11 true\na.txt\nENOENT true\n8 9\ndone"
    );
}

#[test]
fn require_finds_fs_promises_by_its_own_name() {
    assert_eq!(
        run_as_file(
            "promises",
            "console.log(typeof require('fs/promises').readFile, require('fs/promises') === require('fs').promises);"
        ),
        "function true"
    );
}

/// A very small server for one test: it answers each connection with what `answer` says, from what it was sent.
fn serve(answer: impl Fn(&str) -> String + Send + 'static) -> u16 {
    use std::io::{Read as _, Write as _};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut connection) = connection else {
                return;
            };
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            while let Ok(read) = connection.read(&mut buffer) {
                request.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&request);
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .and_then(|length| length.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length || read == 0 {
                        break;
                    }
                }
                if read == 0 {
                    break;
                }
            }
            let reply = answer(&String::from_utf8_lossy(&request));
            let _ = connection.write_all(reply.as_bytes());
        }
    });
    port
}

#[test]
fn fetch_gets_what_a_server_says() {
    let port = serve(|request| {
        let first = request.lines().next().unwrap_or("").to_owned();
        let carried = request
            .lines()
            .find(|line| line.to_ascii_lowercase().starts_with("x-wanted:"))
            .unwrap_or("x-wanted: nothing")
            .to_owned();
        let sent = request.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
        let body = format!("{{\"first\":{first:?},\"carried\":{carried:?},\"sent\":{sent:?}}}");
        if first.contains("/missing") {
            "HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\nConnection: close\r\n\r\nno".to_owned()
        } else {
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-Reply: yes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        }
    });
    let source = format!(
        r#"
        (async () => {{
            const base = 'http://127.0.0.1:{port}';
            const got = await fetch(base + '/hello?a=1', {{ headers: {{ 'X-Wanted': 'this' }} }});
            console.log(got.ok, got.status, got.statusText, got.headers.get('x-reply'), got.headers.get('nope'));
            const answer = await got.json();
            console.log(answer.first, '|', answer.carried);
            console.log(got.bodyUsed);
            await got.text().catch((error) => console.log(error.name));
            const posted = await (await fetch(base + '/post', {{ method: 'POST', body: 'héllo' }})).json();
            console.log(posted.first, '|', posted.sent);
            const missing = await fetch(base + '/missing');
            console.log(missing.ok, missing.status, missing.statusText, await missing.text());
            await fetch('ftp://nope/').catch((error) => console.log(error.name));
            await fetch(base, {{ method: 'GET', body: 'x' }}).catch((error) => console.log(error.name));
            await fetch('http://127.0.0.1:1/').catch((error) => console.log(error.name, error.message.startsWith('fetch failed')));
            const made = new Response('{{"a":1}}', {{ status: 201 }});
            console.log(made.ok, made.status, (await made.json()).a, new Headers({{ A: 'b' }}).get('a'));
        }})();
        "#
    );
    assert_eq!(
        run_as_file("fetch", &source),
        "true 200 OK yes null\n\
         GET /hello?a=1 HTTP/1.1 | x-wanted: this\n\
         true\n\
         TypeError\n\
         POST /post HTTP/1.1 | héllo\n\
         false 404 Not Found no\n\
         TypeError\n\
         TypeError\n\
         TypeError true\n\
         true 201 1 b"
    );
}

#[test]
fn urls_come_apart_and_go_back_together() {
    assert_eq!(
        run(r#"
            const u = new URL('HTTPS://User:p%40ss@EXAMPLE.com:443/a/../b c?x=1&y=é#top');
            console.log(u.href);
            console.log(u.protocol, u.username, u.password, u.host, u.port === '', u.pathname, u.search, u.hash, u.origin);
            u.searchParams.append('z', 'a b');
            console.log(u.search);
            console.log(new URL('../c?d', 'http://a.com/x/y/z').href, new URL('http://a.com/?').href);
            try { new URL('nothing'); } catch (error) { console.log(error.name, error.code); }
        "#),
        "https://User:p%40ss@example.com/b%20c?x=1&y=%C3%A9#top\n\
         https: User p%40ss example.com true /b%20c ?x=1&y=%C3%A9 #top https://example.com\n\
         ?x=1&y=%C3%A9&z=a+b\n\
         http://a.com/x/c?d http://a.com/?\n\
         TypeError ERR_INVALID_URL"
    );
}

#[test]
fn search_params_read_write_and_sort() {
    assert_eq!(
        run(r#"
            const p = new URLSearchParams('?b=2&a=1&a=3&c&e=x%20y+z');
            console.log(p.get('a'), p.getAll('a').join(), p.get('c') === '', p.get('none'), p.has('b'), p.size);
            p.sort();
            console.log(p.toString());
            p.set('a', 'new');
            p.delete('b');
            console.log(JSON.stringify([...p]));
            console.log(new URLSearchParams({ q: 'a b&c' }).toString());
        "#),
        "1 1,3 true null true 5\n\
         a=1&a=3&b=2&c=&e=x+y+z\n\
         [[\"a\",\"new\"],[\"c\",\"\"],[\"e\",\"x y z\"]]\n\
         q=a+b%26c"
    );
}

/// CommonJS that an ES module imports is compiled along with it, and only runs once what it imports before it has, which
/// can collect everything first, as react-router's server chunk does before `cookie` and `set-cookie-parser` run.
#[test]
fn commonjs_an_es_module_imports_still_runs_after_a_collection_before_its_turn() {
    let dir = std::env::temp_dir().join(format!("rsui_js_prepared_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("first.mjs"), "gc();\nexport const first = 'first';\n").unwrap();
    std::fs::write(
        dir.join("one.js"),
        "function parse(text) { return text.split(';').map((part) => part.trim()); }\nexports.parse = parse;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("two.js"),
        "function split(text) { return text.split(',').length; }\nmodule.exports = split;\nmodule.exports.split = split;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.mjs"),
        "import { first } from './first.mjs';\nimport { parse } from './one.js';\nimport { split } from './two.js';\nconsole.log(first, parse('a; b').join('|'), split('x,y,z'));\n",
    )
    .unwrap();
    let (mut engine, output) = engine_with_gc();
    let result = engine
        .run_file(&dir.join("main.mjs"))
        .map(|_| ())
        .and_then(|()| engine.run_until_idle())
        .map_err(|error| error.message);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(result, Ok(()));
    assert_eq!(output.borrow().trim_end(), "first a|b 3");
}

#[test]
fn file_urls_and_paths_turn_into_each_other_on_windows_and_elsewhere() {
    assert_eq!(
        run_as_file("file_urls", r#"
            const { pathToFileURL, fileURLToPath } = require('node:url');
            const windows = { windows: true };
            const posix = { windows: false };
            console.log(pathToFileURL('C:\\a b\\#x?.txt', windows).href);
            console.log(pathToFileURL('C:\\folder\\', windows).href);
            console.log(pathToFileURL('\\\\server\\share\\dir\\f.txt', windows).href);
            console.log(pathToFileURL('\\\\?\\UNC\\server\\share\\f', windows).href);
            console.log(pathToFileURL('/tmp/a b%/c\\d/', posix).href);
            console.log(fileURLToPath('file:///C:/a%20b/c', windows), fileURLToPath(new URL('file://server/share/f'), windows));
            console.log(fileURLToPath('file:///tmp/a%20b', posix), fileURLToPath('file://localhost/tmp/x', posix));
            for (const [url, options] of [['file://host/x', posix], ['file:///a%2Fb', posix], ['file:///a%5Cb', windows], ['file:///a/b', windows], ['http://x/', windows], [5, windows]]) {
                try { fileURLToPath(url, options); } catch (error) { console.log(error.name, error.code); }
            }
        "#),
        "file:///C:/a%20b/%23x%3F.txt\n\
         file:///C:/folder/\n\
         file://server/share/dir/f.txt\n\
         file://server/share/f\n\
         file:///tmp/a%20b%25/c%5Cd/\n\
         C:\\a b\\c \\\\server\\share\\f\n\
         /tmp/a b /tmp/x\n\
         TypeError ERR_INVALID_FILE_URL_HOST\n\
         TypeError ERR_INVALID_FILE_URL_PATH\n\
         TypeError ERR_INVALID_FILE_URL_PATH\n\
         TypeError ERR_INVALID_FILE_URL_PATH\n\
         TypeError ERR_INVALID_URL_SCHEME\n\
         TypeError ERR_INVALID_ARG_TYPE"
    );
}

#[test]
fn the_older_url_functions_take_urls_apart_as_node_does() {
    assert_eq!(
        run_as_file("older_url", r#"
            const url = require('url');
            const parsed = url.parse('http://user:pw@Host.com:8080/p/a/t/h?query=string#hash');
            console.log(parsed instanceof url.Url, JSON.stringify(parsed));
            console.log(JSON.stringify(url.parse('/a/b?x=1&x=2', true).query), url.parse('//foo/bar', false, true).host, url.parse('//foo/bar').host);
            console.log(url.parse('mailto:me@example.com').href, url.parse('http://[::1]:3000/x').hostname, url.parse('HTTP://bücher.de/a b').href);
            console.log(url.parse('http:\\\\evil.com\\path?a\\b').href);
            console.log(url.format({ protocol: 'https', hostname: 'example.com', pathname: '/some/path', query: { page: 1, format: 'json' } }));
            console.log(url.format(new URL('https://a:b@xn--g6w251d.com/x?y#z'), { unicode: true, auth: false }), url.format(new URL('https://a:b@c.com/x?y#z'), { fragment: false, search: false }));
            console.log([url.resolve('/one/two/three', 'four'), url.resolve('http://example.com/', '/one'), url.resolve('http://example.com/one', '/two'), url.resolve('http://a/b/c/d;p?q', '../../g'), url.resolve('mailto:a@b', 'c@d')].join(' '));
        "#),
        "true {\"protocol\":\"http:\",\"slashes\":true,\"auth\":\"user:pw\",\"host\":\"host.com:8080\",\"port\":\"8080\",\"hostname\":\"host.com\",\"hash\":\"#hash\",\"search\":\"?query=string\",\"query\":\"query=string\",\"pathname\":\"/p/a/t/h\",\"path\":\"/p/a/t/h?query=string\",\"href\":\"http://user:pw@host.com:8080/p/a/t/h?query=string#hash\"}\n\
         {\"x\":[\"1\",\"2\"]} foo null\n\
         mailto:me@example.com ::1 http://xn--bcher-kva.de/a%20b\n\
         http://evil.com/path?a%5Cb\n\
         https://example.com/some/path?page=1&format=json\n\
         https://測試.com/x?y#z https://a:b@c.com/x\n\
         /one/two/four http://example.com/one http://example.com/two http://a/g mailto:c@d"
    );
}

#[test]
fn domains_go_into_punycode_and_back() {
    assert_eq!(
        run_as_file("domains", r#"
            const url = require('url');
            console.log(url.domainToASCII('español.com'), url.domainToASCII('中文.com'), url.domainToASCII('EXAMPLE.org'), JSON.stringify(url.domainToASCII('xn--iñvalid.com')));
            console.log(url.domainToUnicode('xn--espaol-zwa.com'), JSON.stringify(url.domainToUnicode('xn--iñvalid.com')), url.domainToASCII('a\u3002b'));
            console.log(new URL('http://Bücher.de/').host, URL.parse('nope'), URL.parse('/x', 'http://a').href, new URL('file://localhost/x').href);
            const options = url.urlToHttpOptions(new URL('https://a:b@[::1]:8443/p?q=1#h'));
            console.log(Object.getPrototypeOf(options), options.hostname, options.port, options.path, options.auth);
        "#),
        "xn--espaol-zwa.com xn--fiq228c.com example.org \"\"\n\
         español.com \"\" a.b\n\
         xn--bcher-kva.de null http://a/x file:///x\n\
         null ::1 8443 /p?q=1 a:b"
    );
}

#[test]
fn paths_on_windows_can_be_network_shares() {
    assert_eq!(
        run_as_file("unc_paths", r#"
            const { win32, posix } = require('path');
            console.log(win32.resolve('\\\\server\\share\\x\\..\\y'), win32.normalize('//server/share'), win32.dirname('\\\\server\\share\\a'));
            console.log(win32.parse('\\\\server\\share\\a.txt').root, win32.join('\\\\server', 'share', 'f'), win32.isAbsolute('\\\\server\\share'));
            console.log(win32.relative('C:\\A\\b', 'c:\\a\\c'), win32.relative('C:\\a', 'D:\\b'), win32.relative('\\\\s\\h\\a', '\\\\s\\h\\b'));
            console.log(win32.toNamespacedPath('C:\\a'), win32.toNamespacedPath('\\\\s\\h\\f'), posix.toNamespacedPath('/a'), win32.toNamespacedPath(''));
            console.log(posix.resolve('rel').startsWith('/'), posix.resolve('rel').includes('\\'));
        "#),
        "\\\\server\\share\\y \\\\server\\share\\ \\\\server\\share\\\n\
         \\\\server\\share\\ \\\\server\\share\\f true\n\
         ..\\c D:\\b ..\\b\n\
         \\\\?\\C:\\a \\\\?\\UNC\\s\\h\\f /a \n\
         true false"
    );
}

#[test]
fn node_says_what_its_modules_are_and_what_process_is() {
    assert_eq!(
        run_as_file(
            "builtins",
            "const m = require('module'); console.log(m.isBuiltin('http'), m.isBuiltin('node:zlib'), m.isBuiltin('url'), m.isBuiltin('left-pad'), Object.prototype.toString.call(process))"
        ),
        "true true true false [object process]"
    );
}

#[test]
fn finished_streams_say_so_once_and_after_it_returns() {
    assert_eq!(
        run_as_file("finished_once", r#"
            const { Readable, PassThrough, finished } = require('stream');
            const done = Readable.from([]);
            done.resume();
            done.on('end', () => {
                let stop = null;
                stop = finished(done, (error) => {
                    stop();
                    console.log('told', error, typeof stop);
                });
                console.log('returned first');
            });
            const open = new PassThrough();
            const stop = finished(open, () => console.log('not told'));
            stop();
            open.end('x');
            open.resume();
        "#),
        "returned first\ntold undefined function"
    );
}

#[test]
fn abort_controllers_and_events_tell_who_is_listening() {
    assert_eq!(
        run(r#"
            const controller = new AbortController();
            const heard = [];
            controller.signal.addEventListener('abort', (event) => heard.push(event.type));
            controller.abort();
            controller.abort();
            console.log(controller.signal.aborted, controller.signal.reason.name, heard.join());
            const target = new EventTarget();
            target.addEventListener('x', () => heard.push('once'), { once: true });
            target.dispatchEvent(new Event('x'));
            target.dispatchEvent(new Event('x'));
            console.log(heard.join());
        "#),
        "true AbortError abort\nabort,once"
    );
}

#[test]
fn a_fetch_that_has_been_aborted_says_so() {
    assert_eq!(
        run_as_file(
            "aborted",
            r#"
            const controller = new AbortController();
            controller.abort();
            fetch('http://127.0.0.1:1/', { signal: controller.signal }).catch((error) => console.log(error.name));
            "#
        ),
        "AbortError"
    );
}

#[test]
fn buffers_turn_into_text_and_back() {
    assert_eq!(
        run(r#"
            const b = Buffer.from('héllo');
            console.log(b.toString('hex'), b.toString('base64'), b.toString(), b.length, b instanceof Uint8Array);
            console.log(Buffer.from('68656c6c6f', 'hex').toString(), Buffer.from('aGVsbG8=', 'base64').toString());
            console.log(Buffer.concat([Buffer.from('a'), Buffer.from('bc')]).toString(), Buffer.isBuffer(b), Buffer.alloc(2).toString('hex'));
        "#),
        "68c3a96c6c6f aMOpbGxv héllo 6 true\nhello hello\nabc true 0000"
    );
}

#[test]
fn something_can_bring_its_own_url_instead() {
    assert_eq!(
        run(
            "globalThis.URL = 'mine'; console.log(URL); console.log(typeof AbortController, typeof Nothing);"
        ),
        "mine\nfunction undefined"
    );
}

#[test]
fn the_web_globals_do_not_show_up_among_the_keys_of_the_global_object() {
    assert_eq!(
        run("console.log(Object.keys(globalThis).includes('URL'));"),
        "false"
    );
}

#[test]
fn a_locale_that_is_not_one_is_the_users_and_digits_follow_the_numbering_system() {
    // shadcn's calendar writes its month dropdown with `toLocaleString("default", { month: "short" })`, and react-day-picker
    // its Persian digits with `Intl.NumberFormat("en-US", { numberingSystem: "arabext" })`.
    assert_eq!(
        run("const d = new Date(2025, 5, 12); console.log(d.toLocaleString('default', { month: 'short' }), d.toLocaleString('default', { month: 'long' }), d.toLocaleString('xx', { month: 'short' }));"),
        "Jun June Jun"
    );
    assert_eq!(
        run("console.log([0, 5, 9].map((n) => new Intl.NumberFormat('en-US', { numberingSystem: 'arabext' }).format(n)).join(''), new Intl.NumberFormat('en-US', { numberingSystem: 'arab' }).format(-1234.5), new Intl.NumberFormat('en-US-u-nu-thai').format(12), (12).toLocaleString('en-US', { numberingSystem: 'bogus' }));"),
        "\u{6F0}\u{6F5}\u{6F9} \u{61C}-\u{661}\u{66C}\u{662}\u{663}\u{664}\u{66B}\u{665} \u{E51}\u{E52} 12"
    );
    assert_eq!(
        run("console.log(new Date(2025, 5, 12).toLocaleDateString('en-US', { numberingSystem: 'arabext' }), new Intl.NumberFormat('en-US', { numberingSystem: 'arabext' }).resolvedOptions().numberingSystem);"),
        "\u{6F6}/\u{6F1}\u{6F2}/\u{6F2}\u{6F0}\u{6F2}\u{6F5} arabext"
    );
}

#[test]
fn numbers_are_written_as_they_are_asked_to_be() {
    assert_eq!(
        run(
            "console.log((1238).toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 }));"
        ),
        "1,238.00"
    );
    assert_eq!(
        run("console.log((1234.5678).toLocaleString('en-US'));"),
        "1,234.568"
    );
    assert_eq!(
        run("console.log((0.25).toLocaleString('en-US', { style: 'percent' }));"),
        "25%"
    );
    assert_eq!(
        run(
            "console.log((-1234.5).toLocaleString('en-US', { style: 'currency', currency: 'USD' }));"
        ),
        "-$1,234.50"
    );
}

#[test]
fn intl_is_only_compiled_when_it_is_looked_for_and_is_there_when_it_is() {
    assert_eq!(
        run(
            "console.log(Object.keys(globalThis).includes('Intl'), typeof Intl, new Intl.NumberFormat('en-US').format(1234.5));"
        ),
        "false object 1,234.5"
    );
}

/// `Intl.Collator` and `localeCompare` put text in the order Node's (and Edge's) ICU does, with their options; what's
/// expected is what Node printed.
#[test]
fn collators_order_text_as_icu_does() {
    assert_eq!(
        run(r#"
const names = ['b', 'Résumé', 'resume', 'a', 'ä', '_x', 'Z', 'file10', 'file9', 'côte', 'coté', '東京', 'あ', 'ア', 'é'];
console.log([...names].sort(new Intl.Collator('en').compare).join(' '));
console.log([...names].sort(new Intl.Collator('en', { numeric: true, sensitivity: 'base' }).compare).join(' '));
console.log([...names].sort((a, b) => a.localeCompare(b, 'en', { caseFirst: 'upper' })).join(' '));
console.log('a'.localeCompare('á', undefined, { sensitivity: 'base' }), 'résumé'.localeCompare('resumes'), 'a'.localeCompare('A'));
const c = new Intl.Collator('en-u-kn', { sensitivity: 'case' });
const o = c.resolvedOptions();
console.log(o.numeric, o.sensitivity, o.caseFirst, o.ignorePunctuation, o.usage, c.compare('x2', 'x10'), typeof c.compare, c.compare.length);
try { new Intl.Collator('en', { sensitivity: 'nope' }); } catch (e) { console.log(e.name); }
"#),
        "_x a ä b coté côte é file10 file9 resume Résumé Z あ ア 東京\n\
         _x a ä b côte coté é file9 file10 Résumé resume Z あ ア 東京\n\
         _x a ä b coté côte é file10 file9 resume Résumé Z あ ア 東京\n\
         0 -1 -1\n\
         true case false false sort -1 function 2\n\
         RangeError"
    );
}

#[test]
fn an_arrow_function_as_the_first_choice_is_not_taken_for_one_with_a_return_type() {
    assert_eq!(
        run("let s = 0; const f = true ? v => (s += v) : v => (s -= v); console.log(f(2));"),
        "2"
    );
    assert_eq!(
        run("const a = 1; const f = false ? (a) : b => b + 1; console.log(f(2));"),
        "3"
    );
    assert_eq!(
        run("const g = true ? (a): number => a + 1 : 0; console.log(g(2));"),
        "3"
    );
    assert_eq!(
        run("const h = (x) : number => x * 2; console.log(h(2));"),
        "4"
    );
    assert_eq!(
        run("const i = true ? (false ? (1) : 2) : 3; console.log(i);"),
        "2"
    );
}

#[test]
fn what_is_in_a_file_that_is_not_really_utf8_still_loads() {
    let dir = std::env::temp_dir().join(format!("rsui-latin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("old.js");
    std::fs::write(&file, b"module.exports = 'caf\xE9'.length;").unwrap();
    let source = format!("console.log(require({:?}));", file.to_string_lossy());
    assert_eq!(run_as_file("latin", &source), "4");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn old_style_calls_to_what_is_written_for_node_work() {
    assert_eq!(
        run_as_file(
            "old_calls",
            "const util = require('util'); const EventEmitter = require('events'); const { Readable } = require('stream');
             function Mine() { EventEmitter.call(this); }
             util.inherits(Mine, EventEmitter);
             const mine = new Mine(); mine.on('x', (v) => console.log(v)); mine.emit('x', 1);
             function Source() { Readable.call(this, { read() {} }); }
             util.inherits(Source, Readable);
             console.log(new Source() instanceof Readable, Buffer('ab').toString('hex'), Buffer(2).length, Buffer.from('a') instanceof Buffer);"
        ),
        "1\ntrue 6162 2 true"
    );
}

#[test]
fn the_environment_is_still_there_after_collecting_garbage() {
    let mut engine = crate::Engine::new();
    engine.set_env("SOME_NAME", "its value");
    let file = std::env::temp_dir().join(format!("rsui-env-{}.js", std::process::id()));
    // This makes a lot of garbage before anything has looked at it.
    std::fs::write(
        &file,
        "let keep = []; for (let i = 0; i < 3000000; i++) { keep = [{ i }]; }
         if (process.env.some_name !== 'its value') throw new Error('it has gone');
         if (!Object.keys(process.env).includes('SOME_NAME')) throw new Error('its name has gone');",
    )
    .unwrap();
    let result = engine.run_file(&file);
    let _ = std::fs::remove_file(&file);
    assert!(
        engine.collections() > 0,
        "this needs garbage to have been collected"
    );
    if let Err(error) = result {
        panic!("{error}");
    }
}

#[test]
fn something_can_say_how_it_is_shown() {
    assert_eq!(
        run(
            "console.log({ [Symbol.for('nodejs.util.inspect.custom')]() { return 'mine'; } }, Buffer.from([1, 255]));"
        ),
        "mine <Buffer 01 ff>"
    );
}

/// Answers one request on this machine with these bytes, whatever it's asked, and says what it was asked.
fn answer_once(body: &'static [u8]) -> (u16, std::thread::JoinHandle<String>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let waiting = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut asked = Vec::new();
        let mut chunk = [0u8; 1024];
        // As far as the end of the headers, and then whatever they said is coming.
        loop {
            let read = stream.read(&mut chunk).unwrap();
            asked.extend_from_slice(&chunk[..read]);
            let text = String::from_utf8_lossy(&asked).into_owned();
            if let Some(end) = text.find("\r\n\r\n") {
                let length = text
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|rest| rest.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if asked.len() >= end + 4 + length {
                    break;
                }
            }
        }
        let head = format!(
            "HTTP/1.1 201 Created\r\nContent-Length: {}\r\nX-Answer: yes\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        String::from_utf8_lossy(&asked).into_owned()
    });
    (port, waiting)
}

#[test]
fn http_asks_for_things_and_gets_back_exactly_what_comes() {
    let (port, waiting) = answer_once(&[0, 255, 104, 105]);
    let said = run_as_file("http", &format!(
        "const http = require('http');
         const asking = http.request({{ hostname: '127.0.0.1', port: {port}, path: '/there?a=1', method: 'POST', headers: {{ 'X-Mine': 'yes' }} }}, (answer) => {{
             const parts = [];
             answer.on('data', (part) => parts.push(part));
             answer.on('end', () => console.log(answer.statusCode, answer.statusMessage, answer.headers['x-answer'], Buffer.concat(parts).toString('hex')));
         }});
         asking.write('some ');
         asking.end('words');"
    ));
    assert_eq!(said, "201 Created yes 00ff6869");
    let asked = waiting.join().unwrap();
    assert!(asked.starts_with("POST /there?a=1 HTTP/1.1"), "{asked}");
    assert!(asked.to_lowercase().contains("x-mine: yes"), "{asked}");
    assert!(asked.ends_with("some words"), "{asked}");
}

#[test]
fn fetch_gets_bytes_as_they_are_and_can_send_them() {
    let (port, waiting) = answer_once(&[0, 255, 104, 105]);
    let said = run(&format!(
        "fetch('http://127.0.0.1:{port}/', {{ method: 'PUT', body: new Uint8Array([1, 0, 2]) }})
             .then((answer) => answer.arrayBuffer())
             .then((bytes) => console.log(new Uint8Array(bytes).join(',')));"
    ));
    assert_eq!(said, "0,255,104,105");
    let asked = waiting.join().unwrap();
    assert!(asked.ends_with("\u{1}\u{0}\u{2}"), "{asked:?}");
}

#[test]
fn something_that_cannot_be_reached_is_an_error_that_can_be_listened_for() {
    let port = {
        // There's nothing here once this has gone.
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    };
    let said = run_as_file(
        "unreachable",
        &format!(
            "require('http').get('http://127.0.0.1:{port}/', () => console.log('answered')).on('error', (error) => console.log(typeof error.code));"
        ),
    );
    assert_eq!(said, "string");
}

/// Runs a little app of modules from files, and gives what the first one has as `result`.
fn run_modules(name: &str, files: &[(&str, &str)]) -> String {
    let folder = std::env::temp_dir().join(format!("rsui-js-cycles-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    for (index, (file, text)) in files.iter().enumerate() {
        let shows = if index == 0 {
            "
console.log(result);"
        } else {
            ""
        };
        std::fs::write(folder.join(file), format!("{text}{shows}")).unwrap();
    }
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let ran = engine.run_file(&folder.join(files[0].0));
    let _ = std::fs::remove_dir_all(&folder);
    ran.unwrap();
    output.borrow().trim_end().to_owned()
}

#[test]
fn what_is_imported_in_a_circle_is_there_by_the_time_it_is_used() {
    let said = run_modules(
        "plain",
        &[
            (
                "main.js",
                "import { read } from './a.js'; import './b.js'; export const result = read();",
            ),
            (
                "a.js",
                "import { value } from './b.js'; export function read() { return value; }",
            ),
            (
                "b.js",
                "import { read } from './a.js'; export const value = 'real value'; export const seen = typeof read;",
            ),
        ],
    );
    assert_eq!(said, "real value");
}

#[test]
fn a_default_import_in_a_circle_gets_there_too() {
    let said = run_modules(
        "default",
        &[
            (
                "main.js",
                "import './b.js'; import { read } from './a.js'; export const result = read();",
            ),
            (
                "b.js",
                "import { read } from './a.js'; export default 'the default'; export const read2 = read;",
            ),
            (
                "a.js",
                "import value from './b.js'; export function read() { return value; }",
            ),
        ],
    );
    assert_eq!(said, "the default");
}

#[test]
fn export_star_in_a_circle_gets_everything_in_the_end() {
    // What's first here finishes before it's got what `index.js` was going to give it.
    let said = run_modules(
        "star",
        &[
            (
                "main.js",
                "import { read, value, again } from './index.js'; export const result = read() + ' ' + value + ' ' + again();",
            ),
            (
                "index.js",
                "export * from './a.js'; export * from './b.js';",
            ),
            (
                "a.js",
                "import { value } from './index.js'; export function read() { return String(value); }",
            ),
            (
                "b.js",
                "import { read } from './index.js'; export const value = 'seen'; export const again = () => read();",
            ),
        ],
    );
    assert_eq!(said, "seen seen seen");
}

#[test]
fn a_module_that_starts_in_the_middle_of_a_star_gets_it_too() {
    let said = run_modules(
        "middle",
        &[
            (
                "main.js",
                "import { read } from './a.js'; export const result = read();",
            ),
            (
                "a.js",
                "import { value } from './index.js'; export function read() { return value + ' ' + (typeof value); }",
            ),
            (
                "index.js",
                "export * from './a.js'; export * from './b.js';",
            ),
            ("b.js", "export const value = 'given';"),
        ],
    );
    assert_eq!(said, "given string");
}

#[test]
fn a_renamed_export_in_a_circle_gets_there_too() {
    let said = run_modules(
        "renamed",
        &[
            (
                "main.js",
                "import { read } from './a.js'; export const result = read();",
            ),
            (
                "a.js",
                "import { other } from './index.js'; import './b.js'; export function read() { return other; } export const name = 'first';",
            ),
            ("index.js", "export { name as other } from './a.js';"),
            ("b.js", "export const b = 1;"),
        ],
    );
    assert_eq!(said, "first");
}


/// What Node prints for this program: places in the code that see a few shapes, or many, and everything that can make
/// what they remembered wrong (shapes changing, prototypes changing, getters and setters added or replaced, deletes,
/// frozen and non-extensible objects, proxies, typed arrays' `length`, `in` and `hasOwnProperty`).
#[test]
fn places_that_see_several_shapes_notice_when_what_they_remembered_changes() {
    let program = r#"
'use strict';
const out = [];
const log = (...a) => out.push(a.join(' '));
function getX(o) { return o.x; }
function setX(o, v) { o.x = v; }
function addZ(o) { o.z = 1; }
function callM(o) { return o.m(); }
function lengthOf(o) { return o.length; }
function hasY(o) { return 'y' in o; }
function ownY(o) { return Object.prototype.hasOwnProperty.call(o, 'y'); }
function run(f, list) { let r = []; for (let k = 0; k < 3; k++) r = list.map(f); return r.join(','); }

// Own properties at a site that sees a few shapes, then shapes changing under it.
const a = { x: 1 }, b = { y: 0, x: 2 }, c = { y: 0, z: 0, x: 3 }, d = { w: 0, x: 4 };
log('own', run(getX, [a, b, c, d]));
delete b.x; c.x = 30; Object.defineProperty(d, 'x', { get() { return 40; } });
log('own changed', run(getX, [a, b, c, d]));

// Through prototypes, which then change.
class P { m() { return 'p'; } } class Q extends P { } class R extends Q { m() { return 'r'; } }
const S = { m() { return 's'; } };
const objs = [new P(), new Q(), new R(), Object.create(S)];
log('proto', run(callM, objs));
P.prototype.m = () => 'p2';
log('proto changed', run(callM, objs));
Q.prototype.m = () => 'q';
objs[2].m = () => 'own';
Object.setPrototypeOf(objs[3], { m() { return 's2'; } });
log('proto changed again', run(callM, objs));
P.prototype.x = 'px';
log('inherited x', run(getX, objs));
Object.defineProperty(Q.prototype, 'x', { get() { return 'qget'; }, configurable: true });
log('getter later', run(getX, objs));
Object.defineProperty(Q.prototype, 'x', { get() { return 'qget2'; }, configurable: true });
log('getter replaced', run(getX, objs));
delete Q.prototype.x; delete P.prototype.x;
log('gone', run(getX, objs));

// Writes to own properties, then one object frozen, sealed or made non-extensible.
const w = [{ x: 0 }, { a: 0, x: 0 }, { b: 0, x: 0 }, { c: 0, x: 0 }];
for (let k = 0; k < 3; k++) w.forEach((o, i) => setX(o, i + k));
log('writes', run(getX, w));
Object.freeze(w[1]);
try { setX(w[1], 99); } catch (e) { log('frozen', e.constructor.name); }
log('after freeze', run(getX, w));

// Adding a property, which a setter or a read-only one on a prototype stops.
class A1 { } class A2 { } class A3 { } class A4 { }
const adds = () => [new A1(), new A2(), new A3(), new A4(), { k: 1 }];
for (let k = 0; k < 3; k++) adds().forEach(addZ);
log('added', adds().map(o => (addZ(o), o.z)).join(','));
let seen = 0;
Object.defineProperty(A2.prototype, 'z', { set(v) { seen += v; }, get() { return 'setter'; } });
Object.defineProperty(A3.prototype, 'z', { value: 'ro', writable: false });
const after = adds();
after.forEach(o => { try { addZ(o); } catch (e) { log('read-only', e.constructor.name); } });
log('after setter', after.map(o => o.z).join(','), seen, after.map(o => Object.keys(o).join('')).join('|'));
const ne = new A1(); Object.preventExtensions(ne);
try { addZ(ne); } catch (e) { log('not extensible', e.constructor.name); }
log('ne', ne.z, Object.keys(ne).length);

// Proxies at a site that's seen plain objects.
const proxy = new Proxy({ x: 'target' }, {
  get(t, k) { return k === 'x' ? 'trap' : t[k]; },
  has(t, k) { return k === 'y'; },
  set(t, k, v) { log('set trap', k, v); return true; },
});
log('proxy', run(getX, [a, proxy, c, proxy]), hasY(proxy), ownY(proxy));
setX(proxy, 7);

// Many shapes: past the few a place keeps for itself.
const many = [];
for (let i = 0; i < 12; i++) { const o = {}; o['k' + i] = i; o.x = i; if (i % 2) o.y = i; many.push(o); }
log('many', run(getX, many));
for (let k = 0; k < 3; k++) many.forEach((o, i) => setX(o, i * 10 + k));
log('many writes', run(getX, many));
delete many[3].x; Object.freeze(many[4]); Object.defineProperty(many[5], 'x', { get() { return 'g'; } });
try { many.forEach((o, i) => setX(o, 'n' + i)); } catch (e) { log('many frozen', e.constructor.name); }
log('many changed', run(getX, many));
const manyProto = [];
for (let i = 0; i < 10; i++) { const p = { ['p' + i]: 1 }; manyProto.push(Object.create(p)); }
log('many absent', run(getX, manyProto));
Object.prototype.x = 'op';
log('many from Object.prototype', run(getX, manyProto));
delete Object.prototype.x;
log('many absent again', run(getX, manyProto));
const manyAdd = () => { const r = []; for (let i = 0; i < 10; i++) r.push({ ['q' + i]: i }); return r; };
for (let k = 0; k < 3; k++) manyAdd().forEach(addZ);
let protoSeen = 0;
Object.defineProperty(Object.prototype, 'z', { set(v) { protoSeen += v; }, get() { return 'oz'; }, configurable: true });
const ma = manyAdd(); ma.forEach(addZ);
log('many add with setter', ma.map(o => o.z).join(','), protoSeen);
delete Object.prototype.z;

// Fields defined by a base class on instances of its subclasses, and literals.
class F { f = 1; g = 2; }
class F1 extends F { } class F2 extends F { } class F3 extends F { h = 3; } class F4 extends F { }
const fs = []; for (let k = 0; k < 3; k++) for (const C of [F, F1, F2, F3, F4]) fs.push(new C());
log('fields', fs.slice(-5).map(o => Object.keys(o).join('')).join(','));
const lit = (o) => ({ ...o, x: 9 });
log('literal', [a, b, c, { q: 1 }].map(o => JSON.stringify(lit(o))).join(' '));

// Lengths: arrays, strings, typed arrays and objects with a length at one place.
const lens = [[1, 2], 'abc', new Float64Array(4), new Int8Array(5), { length: 'own' }, function (p, q) { }, new Uint8Array(new ArrayBuffer(8), 2)];
log('lengths', run(lengthOf, lens));
let i8 = 0; for (let k = 0; k < 10; k++) for (let i = 0; i < lens[3].length; i++) i8++;
log('loop', i8);
try { lengthOf(Object.create(Float64Array.prototype)); } catch (e) { log('not typed', e.constructor.name); }
const TA = Object.getPrototypeOf(Int8Array.prototype);
const real = Object.getOwnPropertyDescriptor(TA, 'length');
Object.defineProperty(TA, 'length', { get() { return 'patched'; }, configurable: true });
log('patched', run(lengthOf, lens));
Object.defineProperty(TA, 'length', real);
Object.defineProperty(lens[2], 'length', { value: 'own length' });
log('restored', run(lengthOf, lens));
Object.defineProperty(Int8Array.prototype, 'length', { value: 'int8 proto' });
log('shadowed', run(lengthOf, lens));

// Lazy function properties at a site that's read them on something else of the same shape.
const fakeFn = Object.create(Function.prototype);
function named() { }
log('names', [fakeFn, named, () => 1, class K { }].map(f => f.name).join(','));

// in and hasOwnProperty, also for what isn't there.
const ins = [{ y: 1 }, { x: 1 }, Object.create({ y: 2 }), [], Object.create(null), { a: 1, y: undefined }];
log('in', run(hasY, ins));
log('own', run(ownY, ins));
Object.prototype.y = 1;
log('in after', run(hasY, ins));
log('own after', run(ownY, ins));
delete Object.prototype.y;
ins[1].y = 1; delete ins[0].y;
log('in changed', run(hasY, ins), run(ownY, ins));
console.log(out.join('\n'));
"#;
    let expected = r#"
own 1,2,3,4
own changed 1,,30,40
proto p,p,r,s
proto changed p2,p2,r,s
proto changed again p2,q,own,s2
inherited x px,px,px,
getter later px,qget,qget,
getter replaced px,qget2,qget2,
gone ,,,
writes 2,3,4,5
frozen TypeError
after freeze 2,3,4,5
added 1,1,1,1,1
read-only TypeError
after setter 1,setter,ro,1,1 1 z|||z|kz
not extensible TypeError
ne  0
proxy 1,trap,30,trap true false
set trap x 7
many 0,1,2,3,4,5,6,7,8,9,10,11
many writes 2,12,22,32,42,52,62,72,82,92,102,112
many frozen TypeError
many changed n0,n1,n2,n3,42,g,62,72,82,92,102,112
many absent ,,,,,,,,,
many from Object.prototype op,op,op,op,op,op,op,op,op,op
many absent again ,,,,,,,,,
many add with setter oz,oz,oz,oz,oz,oz,oz,oz,oz,oz 10
fields fg,fg,fg,fgh,fg
literal {"x":9} {"y":0,"x":9} {"y":0,"z":0,"x":9} {"q":1,"x":9}
lengths 2,3,4,5,own,2,6
loop 50
not typed TypeError
patched 2,3,patched,patched,own,2,patched
restored 2,3,own length,5,own,2,6
shadowed 2,3,own length,int8 proto,own,2,6
names ,named,,K
in true,false,true,false,false,true
own true,false,false,false,false,true
in after true,true,true,true,false,true
own after true,false,false,false,false,true
in changed false,true,true,false,false,true false,true,false,false,false,true
"#;
    assert_eq!(run(program), expected.trim());
}
/// What a program logs when its functions are compiled when first called, and when they're all compiled at once,
/// which have to be the same.
fn run_both_ways(source: &str) -> String {
    run_both_ways_with_stack(source, None)
}

/// [`run_both_ways`], with the engine told how much of Rust's stack it may use, for a thread with more than a test's.
fn run_both_ways_with_stack(source: &str, stack_limit: Option<usize>) -> String {
    let output = |lazily: bool| {
        let output = Rc::new(RefCell::new(String::new()));
        let mut engine = Engine::new();
        engine.compile_lazily(lazily);
        if let Some(limit) = stack_limit {
            engine.set_stack_limit(limit);
        }
        let sink = output.clone();
        engine.set_printer(move |text| sink.borrow_mut().push_str(text));
        if let Err(error) = engine
            .eval(source, "test.js")
            .and_then(|_| engine.run_until_idle())
        {
            panic!("{source}\nthrew: {error}");
        }
        output.borrow().trim_end().to_owned()
    };
    let lazily = output(true);
    assert_eq!(lazily, output(false), "{source}");
    lazily
}

#[test]
fn a_function_compiled_when_first_called_sees_what_it_would_have() {
    // Variables from several functions out, some written after the inner function is made.
    assert_eq!(
        run_both_ways(
            "function outer(a) { let b = a + 1; const f = () => { let c = 10; return () => a + b + c + d; }; let d = 100; return f()(); }
             console.log(outer(1));"
        ),
        "113"
    );
    // Something that's still in its temporal dead zone when the function is called.
    assert_eq!(
        run_both_ways(
            "function f() { const read = () => later; try { read(); } catch (e) { console.log(e.constructor.name); } let later = 1; return read(); }
             console.log(f());"
        ),
        "ReferenceError\n1"
    );
    // `this`, `arguments` and `new.target` through arrows, and a function's own name.
    assert_eq!(
        run_both_ways(
            "function F() { const g = () => () => [this.x, arguments.length, new.target === F]; this.x = 7; return g()(); }
             console.log(F.call({}, 1, 2).join(), new F(1).join());
             const fact = function me(n) { return n < 2 ? 1 : n * me(n - 1); }; console.log(fact(5));"
        ),
        "7,2,false 7,1,true\n120"
    );
    // A variable the inner function has its own of, with the same name as one outside.
    assert_eq!(
        run_both_ways(
            "let x = 'outer'; function f() { const g = (x) => x + '!'; const h = () => x; return g('inner') + h(); } console.log(f());"
        ),
        "inner!outer"
    );
    // Writing to what's outside, and a constant that can't be.
    assert_eq!(
        run_both_ways(
            "function counter() { let n = 0; const k = 1; return [() => ++n, () => { try { k = 2; } catch (e) { return e.constructor.name; } }]; }
             const [step, bad] = counter(); step(); console.log(step(), bad());"
        ),
        "2 TypeError"
    );
    // Private names, strictness from a class, fields and `super`.
    assert_eq!(
        run_both_ways(
            "class A { hi() { return 'A'; } }
             class B extends A { #secret = 3; static count = 0; value = this.#secret * 2;
               hi() { return super.hi() + this.#secret + (() => this.value)(); }
               loose() { try { undeclared = 1; } catch (e) { return e.constructor.name; } } }
             const b = new B(); console.log(b.hi(), b.loose(), B.count);"
        ),
        "A36 ReferenceError 0"
    );
    // Generators, async functions, defaults that see earlier parameters, and the rest.
    assert_eq!(
        run_both_ways(
            "function* g(a, b = a * 2, ...rest) { yield a; yield b; yield rest.length; }
             console.log([...g(1, undefined, 9, 9)].join());
             async function later(v) { await null; return v + 1; } later(1).then((v) => console.log(v));"
        ),
        "1,2,2\n2"
    );
    // A function's name and length, read before it's ever called.
    assert_eq!(
        run_both_ways(
            "const o = { method(a, b) {}, get value() { return 1; } }; const anon = (a, b, c = 1) => {};
             console.log(o.method.name, o.method.length, anon.name, anon.length, Object.getOwnPropertyDescriptor(o, 'value').get.name);"
        ),
        "method 2 anon 2 get value"
    );
}

#[test]
fn a_function_that_waits_is_not_compiled_until_called() {
    let mut engine = Engine::new();
    engine.compile_lazily(true);
    engine.set_printer(|_| {});
    engine
        .eval("function never() { return 1; } function once() { return 2; } once();", "test.js")
        .unwrap();
    assert_eq!(engine.vm.uncompiled.len(), 1);
    engine.eval("never();", "test2.js").unwrap();
    assert!(engine.vm.uncompiled.is_empty());
}

#[test]
fn mistakes_in_a_function_that_waits_are_found_before_anything_runs() {
    for bad in [
        "function f() { break; }",
        "function f() { a: while (true) { continue b; } }",
        "function f() { let a; { var a; } }",
        "function f() { try {} catch (e) { let e; } }",
        "function f() { const g = () => { let [a]; }; }",
        "function f() { const o = { m() { let x; let x; } }; }",
        "class C { m() { (() => { switch (1) { case 1: continue; } })(); } }",
    ] {
        let source = format!("console.log('ran'); {bad}");
        let mut engine = Engine::new();
        engine.compile_lazily(true);
        engine.set_printer(move |_| panic!("{bad} ran"));
        let error = engine.eval(&source, "test.js").unwrap_err().message;
        assert!(error.contains("SyntaxError"), "{bad}: {error}");
    }
}

#[test]
fn every_bundled_module_compiles() {
    let node = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/node");
    let mut checked = 0;
    for folder in [node.clone(), node.join("misc")] {
        for entry in std::fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|extension| extension == "js") {
                let source: &'static str =
                    Box::leak(std::fs::read_to_string(&path).unwrap().into_boxed_str());
                if let Err(error) = check_compiles(source, &MODULE_PARAMS) {
                    panic!("{}: {error}", path.display());
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 30);
    for name in ["internal/env", "fs/promises", "os"] {
        let source = crate::modules::node_source(name).unwrap();
        check_compiles(source, &MODULE_PARAMS).unwrap();
    }
    check_compiles(
        include_str!("js/helpers.js"),
        &["__iteratorPrototype", "__disposal"],
    )
    .unwrap();
}

#[test]
fn only_packages_wait_to_be_compiled() {
    let folder = std::env::temp_dir().join(format!("rsui-js-lazy-{}", std::process::id()));
    let package = folder.join("node_modules").join("tool");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("index.js"),
        "exports.used = () => 'used'; exports.unused = () => 'unused'; exports.broken = function () { return 1; };",
    )
    .unwrap();
    std::fs::write(
        folder.join("main.js"),
        "const tool = require('tool'); function mine() {} console.log(tool.used());",
    )
    .unwrap();
    // A mistake in a package's function that never runs is still found when it loads.
    std::fs::write(folder.join("node_modules").join("bad.js"), "exports.f = () => { break; };").unwrap();
    std::fs::write(folder.join("uses_bad.js"), "require('./node_modules/bad.js');").unwrap();
    let mut engine = Engine::new();
    // Packages waiting is off unless it's asked for.
    engine.set_lazy_compile(crate::LazyCompile::NodeModules);
    let output = Rc::new(RefCell::new(String::new()));
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let ran = engine.run_file(&folder.join("main.js"));
    let waiting: Vec<String> = engine
        .vm
        .uncompiled
        .keys()
        .map(|proto| engine.vm.atom_text(engine.vm.protos[*proto as usize].file))
        .collect();
    let mut other = Engine::new();
    other.set_lazy_compile(crate::LazyCompile::NodeModules);
    let bad = other.run_file(&folder.join("uses_bad.js"));
    let _ = std::fs::remove_dir_all(&folder);
    ran.unwrap();
    assert_eq!(output.borrow().trim_end(), "used");
    // `unused` and `broken`, not `mine` or anything in the bundled modules.
    assert_eq!(waiting.len(), 2, "{waiting:?}");
    assert!(waiting.iter().all(|file| file.contains("node_modules")), "{waiting:?}");
    assert!(bad.unwrap_err().message.contains("SyntaxError"));
}

/// What Node prints for this program: places in the code that see two to four shapes keep what they found for each in
/// entries of their own, and those have to notice everything that makes them wrong as the first one does (shapes
/// changing, prototypes changing or written, getters and setters appearing, frozen and non-extensible objects), as do
/// places that see more shapes than that. Each part has functions of its own, so its places see only its shapes.
#[test]
fn polymorphic_places_notice_when_any_of_what_they_keep_changes() {
    let program = r#"
'use strict';
const out = [];
const log = (...a) => out.push(a.join(' '));
function getX(o) { return o.x; }
function setX(o, v) { o.x = v; }
function addW(o, v) { o.w = v; }
function callM(o) { return o.m(); }
function lengthOf(o) { return o.length; }
function each(f, list, times) { let r; for (let k = 0; k < times; k++) r = list.map(f); return r.join(','); }

// A place that sees two, three and four shapes, so what it saw first is kept in its other entries, and then those
// shapes change: what's in the other entries has to be noticed as out of date, not just what's first.
const s1 = { x: 1 }, s2 = { a: 0, x: 2 }, s3 = { a: 0, b: 0, x: 3 }, s4 = { a: 0, b: 0, c: 0, x: 4 };
function getX1(o) { return o.x; }
log('two', each(getX1, [s1, s2], 3));
log('four', each(getX1, [s1, s2, s3, s4], 3));
s1.x = 10; delete s2.x; s3.x = 30;
log('four changed', each(getX1, [s1, s2, s3, s4], 3));
Object.defineProperty(s4, 'x', { get() { return 'g4'; }, configurable: true });
log('a getter on one', each(getX1, [s1, s2, s3, s4], 3));

// Found on prototypes, in the other entries, then the prototypes change (the epoch).
const pa = { x: 'pa' }, pb = { x: 'pb' }, pc = { x: 'pc' };
const ka = Object.create(pa), kb = Object.create(pb), kc = Object.create(pc), kd = { x: 'own' };
function getX2(o) { return o.x; }
log('protos', each(getX2, [ka, kb, kc, kd], 3));
pb.x = 'pb2';
log('one proto written', each(getX2, [ka, kb, kc, kd], 3));
delete pa.x; Object.setPrototypeOf(pc, { x: 'pc-parent' }); delete pc.x;
log('protos changed', each(getX2, [ka, kb, kc, kd], 3));
ka.x = 'ka-own';
log('shadowed', each(getX2, [ka, kb, kc, kd], 3));

// Method calls at a place that sees several shapes, then a method replaced on one prototype.
class C1 { m() { return 1; } } class C2 { m() { return 2; } } class C3 extends C1 { } class C4 extends C2 { m() { return 4; } }
const cs = [new C1(), new C2(), new C3(), new C4()];
log('methods', each(callM, cs, 3));
C1.prototype.m = function () { return 'one'; };
log('method replaced', each(callM, cs, 3));
cs[3].m = () => 'own4';
log('own method', each(callM, cs, 3));

// More shapes than a place keeps: it's megamorphic, and still notices changes.
const many = [];
for (let i = 0; i < 12; i++) { const o = {}; o['k' + i] = i; o.x = i; many.push(o); }
log('many', each(getX, many, 3));
many[5].x = 'five'; delete many[7].x; Object.setPrototypeOf(many[9], { x: 'nine' }); delete many[9].x;
log('many changed', each(getX, many, 3));
const deep = [];
for (let i = 0; i < 8; i++) { const p = {}; p['p' + i] = 0; p.x = 'p' + i; deep.push(Object.create(p)); }
log('many protos', each(getX, deep, 3));
Object.getPrototypeOf(deep[2]).x = 'changed';
log('many protos changed', each(getX, deep, 3));

// Writes at a place that sees several shapes, and many: own properties, and adding one, which a setter or a
// read-only one on a prototype then stops, and frozen or non-extensible objects refuse.
function setX3(o, v) { o.x = v; }
function getX3(o) { return o.x; }
const ws = [{ x: 0 }, { a: 0, x: 0 }, { b: 0, x: 0 }, { c: 0, x: 0 }];
for (let k = 0; k < 3; k++) ws.forEach((o, i) => setX3(o, i * 10 + k));
log('writes', each(getX3, ws, 1));
Object.freeze(ws[2]);
try { setX3(ws[2], 99); } catch (e) { log('frozen', e.constructor.name); }
log('after freeze', each(getX3, ws, 1));
const wm = [];
for (let i = 0; i < 10; i++) { const o = {}; o['q' + i] = i; o.x = 0; wm.push(o); }
for (let k = 0; k < 3; k++) wm.forEach((o, i) => setX(o, i + k));
log('many writes', each(getX, wm, 1));
Object.preventExtensions(wm[4]);
setX(wm[4], 'still writable');
log('non-extensible', wm[4].x);
class B1 { } class B2 { } class B3 { } class B4 { } class B5 { } class B6 { }
const fresh = () => [new B1(), new B2(), new B3(), new B4(), new B5(), new B6(), { z: 1 }];
for (let k = 0; k < 3; k++) fresh().forEach((o, i) => addW(o, i));
log('added', fresh().map(o => (addW(o, 7), o.w)).join(','));
Object.defineProperty(B3.prototype, 'w', { set(v) { this.seen = v; }, get() { return 'set-' + this.seen; } });
Object.defineProperty(B5.prototype, 'w', { value: 'ro', writable: false });
const after = fresh();
after.forEach((o, i) => { try { addW(o, i); } catch (e) { o.err = e.constructor.name; } });
log('added after', after.map(o => o.w + (o.err ? '!' + o.err : '')).join(','));
function addW4(o, v) { o.w = v; }
const four = () => [new B1(), new B2(), new B4(), new B6()];
for (let k = 0; k < 3; k++) four().forEach((o, i) => addW4(o, i));
Object.defineProperty(B2.prototype, 'w', { set(v) { this.seen2 = v; }, get() { return 'set2-' + this.seen2; } });
log('four added after', four().map(o => (addW4(o, 5), o.w)).join(','));

// `length` at a place that sees arrays, strings, typed arrays and objects with their own.
log('lengths', each(lengthOf, [[1, 2], 'abc', new Uint8Array(4), { length: 9 }, { a: 1, length: 8 }, function (a, b) { }], 3));

// Fields a base class defines on objects of its subclasses, which have shapes of their own.
class F { f = 1; g = 2; } class G extends F { h = 3; } class H extends F { i = 4; } class I extends G { j = 5; }
for (let k = 0; k < 3; k++) [new F(), new G(), new H(), new I()];
log('fields', [new F(), new G(), new H(), new I()].map(o => Object.keys(o).join('')).join(','));

// Getters found through the other entries, and a lazy function's own name at a polymorphic place.
function getName(o) { return o.name; }
const named = [function alpha() { }, { name: 'obj' }, class Beta { }, Object.create({ get name() { return 'got'; } })];
log('names', each(getName, named, 3));
log('names fresh', each(getName, [function gamma() { }, function () { }, () => 1, { a: 1, name: 'o2' }], 3));

// After many collections, with old objects written at a polymorphic place.
const olds = [{ x: 1 }, { y: 1, x: 2 }, { z: 1, x: 3 }];
for (let i = 0; i < 20000; i++) { const junk = { a: i, b: [i] }; if (junk.a < 0) log('never'); }
for (let k = 0; k < 3; k++) olds.forEach((o, i) => setX(o, { v: i + k }));
for (let i = 0; i < 20000; i++) { const junk = { a: i, b: [i] }; if (junk.a < 0) log('never'); }
log('old', olds.map(o => o.x.v).join(','));

console.log(out.join('\n'));
"#;
    let expected = r#"
two 1,2
four 1,2,3,4
four changed 10,,30,4
a getter on one 10,,30,g4
protos pa,pb,pc,own
one proto written pa,pb2,pc,own
protos changed ,pb2,pc-parent,own
shadowed ka-own,pb2,pc-parent,own
methods 1,2,1,4
method replaced one,2,one,4
own method one,2,one,own4
many 0,1,2,3,4,5,6,7,8,9,10,11
many changed 0,1,2,3,4,five,6,,8,nine,10,11
many protos p0,p1,p2,p3,p4,p5,p6,p7
many protos changed p0,p1,changed,p3,p4,p5,p6,p7
writes 2,12,22,32
frozen TypeError
after freeze 2,12,22,32
many writes 2,3,4,5,6,7,8,9,10,11
non-extensible still writable
added 7,7,7,7,7,7,7
added after 0,1,set-2,3,ro!TypeError,5,6
four added after 5,set2-5,5,5
lengths 2,3,4,9,8,2
fields fg,fgh,fgi,fghj
names alpha,obj,Beta,got
names fresh gamma,,,o2
old 2,3,4
"#;
    assert_eq!(run(program), expected.trim());
}

/// The ops of the function named `name` that `source` made, up to its first return.
fn ops_of(engine: &Engine, name: &str) -> Vec<crate::op::Op> {
    let vm = &engine.vm;
    let proto = vm
        .protos
        .iter()
        .find(|proto| vm.atom_text(proto.name) == name)
        .unwrap_or_else(|| panic!("no function {name}"));
    let mut ops = Vec::new();
    for op in &vm.code[proto.start as usize..] {
        ops.push(*op);
        if matches!(op, crate::op::Op::Return { .. } | crate::op::Op::ReturnUndefined) {
            break;
        }
    }
    ops
}

/// A place that has only ever seen one shape keeps the instruction for one, and one that sees a second becomes the
/// kind that looks at the others too, for reads, writes and method calls.
#[test]
fn a_place_becomes_polymorphic_when_it_sees_a_second_shape() {
    use crate::op::Op;
    let mut engine = Engine::new();
    engine.set_printer(|_| {});
    engine
        .eval(
            r#"
function readOne(o) { return o.x; }
function readMany(o) { return o.x; }
function writeMany(o) { o.x = 1; }
function callMany(o) { return o.m(); }
const one = [{ x: 1 }, { x: 2 }];
const many = [{ x: 1, m() { return 1; } }, { a: 1, x: 2, m() { return 2; } }, { b: 1, x: 3, m() { return 3; } }];
for (let k = 0; k < 3; k++) {
    one.forEach(readOne);
    many.forEach(readMany);
    many.forEach(writeMany);
    many.forEach(callMany);
}
"#,
            "test.js",
        )
        .unwrap();
    let has = |name: &str, wanted: fn(&Op) -> bool| ops_of(&engine, name).iter().any(wanted);
    assert!(has("readOne", |op| matches!(op, Op::GetProp { .. })));
    assert!(has("readMany", |op| matches!(op, Op::GetPropPoly { .. })));
    assert!(has("writeMany", |op| matches!(op, Op::SetPropPoly { .. })));
    assert!(has("callMany", |op| matches!(op, Op::CallMethodPoly { .. })));
}

/// Callbacks that Rust calls (array methods, sorts, Map and Set, `replace`, JSON) are called the quick way when they're
/// everyday functions. Whatever they do, things have to carry on as they would: throwing, calling the same method again,
/// changing what's gone through, being generators or async, getters, `this`, running out of stack. Expected from Node.
#[test]
fn callbacks_from_rust_can_do_anything() {
    let program = r#"
const log = [];
const out = (...parts) => log.push(parts.join(' '));
// A callback that throws leaves things as they were.
try { [1, 2, 3].map((x) => { if (x === 2) throw new Error('two'); return x; }); } catch (e) { out('threw', e.message); }
out('after', [1, 2, 3].map((x) => x * 2).join());
// The same method from inside its own callback, and deeper.
out('nested', [1, 2].map((x) => [10, 20].map((y) => [x, y].map((z) => z + 1).join('+')).join('|')).join(' / '));
const deep = (n) => (n === 0 ? [0] : [n].flatMap((x) => deep(x - 1).map((y) => y + x)));
out('deep', deep(10).join());
// Changing the array while it's gone through: what's added isn't visited, what's removed is skipped.
const grow = [1, 2, 3];
const seen = [];
grow.forEach((x, i, a) => { seen.push(x); if (i === 0) { a.push(99); a.pop(); a.pop(); } });
out('grow', seen.join(), grow.length);
const holes = [1, , 3, , 5];
out('holes', holes.map((x) => x * 2).length, Object.keys(holes.map((x) => x * 2)).join(), holes.filter(() => true).join());
const shrink = [1, 2, 3, 4, 5];
out('shrink', shrink.map((x, i, a) => { a.length = 3; return x; }).join(':'), shrink.length);
const swap = [3, 1, 2];
out('reduce mutating', swap.reduce((acc, x, i, a) => { if (i === 0) a[2] = 100; return acc + x; }, 0));
// Generators and async functions as callbacks.
function* gen(x) { yield x; yield x * 2; }
out('gen', [1, 2].map(gen).map((g) => [...g].join('+')).join());
const asyncs = [1, 2, 3].map(async (x) => { await null; return x * 3; });
Promise.all(asyncs).then((v) => out('async', v.join()));
// Getters on what's gone through, including one that calls the same method.
const arrayLike = { length: 3, get 0() { return 'a'; }, get 1() { return [1, 2].map((x) => x + 1).join(''); }, 2: 'c' };
out('getters', Array.prototype.map.call(arrayLike, (x) => x.toUpperCase()).join());
// Comparisons that throw, and sorting inside one.
try { [3, 1, 2].sort((a, b) => { if (a === 2 || b === 2) throw new Error('cmp'); return a - b; }); } catch (e) { out('sort threw', e.message); }
out('sort nested', [3, 1, 2].sort((a, b) => [b, a].sort((x, y) => x - y)[0] === a ? -1 : 1).join());
out('sort', [5, 1, 4, 2, 3].sort((a, b) => b - a).join());
// What `this` is.
out('this strict', [1].map(function () { 'use strict'; return typeof this; }).join(), [1].map(function () { 'use strict'; return this; }, 5)[0]);
out('this sloppy', [1].map(function () { return this === globalThis; }).join(), typeof [1].map(function () { return this; }, 5)[0]);
out('this arrow', [1].map(() => typeof this).join());
// Rest parameters, `arguments`, defaults, fewer and more parameters.
out('rest', [1, 2].map((...all) => all.length).join(), [7].map(function () { return arguments.length; }).join());
out('defaults', [1, 2].map((x, i = 9, a, extra = 'e') => `${x}${i}${extra}`).join());
out('reduce rest', [1, 2, 3].reduce((...all) => all[0] + all[1] + all.length, 0));
// What isn't a plain function.
try { [1].map(class A {}); } catch (e) { out('class', e.constructor.name); }
out('bound', [1, 2].map(function (x) { return this.k + x; }.bind({ k: 10 })).join());
out('proxy', [1, 2].map(new Proxy((x) => x * 7, {})).join());
out('native', ['1', '2', '3'].map(Number).join(), ['a', 'b'].map(String.prototype.toUpperCase.call, String.prototype.toUpperCase).join());
// Map and Set forEach, changing what they go through.
const m = new Map([[1, 'a'], [2, 'b'], [3, 'c']]);
const mv = [];
m.forEach((v, k, map) => { mv.push(k + v); if (k === 1) { map.delete(2); map.set(4, 'd'); } });
out('map', mv.join());
const s = new Set([1, 2]);
const sv = [];
s.forEach((v) => { sv.push(v); if (v < 4) s.add(v + 2); });
out('set', sv.join());
try { new Map([[1, 1]]).forEach(() => { throw new Error('mapthrow'); }); } catch (e) { out('map threw', e.message); }
// replace with a function, inside itself.
out('replace', 'a-b-c'.replace(/[a-z]/g, (c) => c.toUpperCase() + 'xy'.replace(/y/, (y) => y + c)));
out('replace groups', '2024-01'.replace(/(?<y>\d+)-(?<m>\d+)/, (...a) => JSON.stringify(a[a.length - 1])));
// JSON with a reviver and a replacer.
out('json', JSON.stringify(JSON.parse('{"a":[1,2,{"b":3}]}', (k, v) => (typeof v === 'number' ? v * 10 : v)), (k, v) => (k === 'b' ? undefined : v)));
// Running out of stack inside callbacks is an error that can be caught, after which things carry on.
const down = (n) => [n].map((x) => down(x + 1))[0];
try { down(0); } catch (e) { out('overflow', e instanceof RangeError); }
out('after overflow', [1, 2, 3].map((x) => x + 1).join());
// Lots of calls, each making something, with collections going on.
let made = [];
for (let r = 0; r < 20; r++) made = Array.from({ length: 2000 }, (_, i) => ({ i, s: 'v' + i })).map((o) => ({ ...o, t: o.s + r })).filter((o) => o.i % 7 === 0);
out('made', made.length, made[100].t, made.reduce((a, o) => a + o.i, 0));
out('every some find', [1, 2, 3].every((x) => x > 0), [1, 2, 3].some((x) => x > 2), [1, 2, 3].find((x) => x > 1), [1, 2, 3].findLast((x) => x < 3), [1, 2, 3].findIndex((x) => x === 3));
setTimeout(() => console.log(log.join('\n')), 0);
"#;
    let expected = r#"
threw two
after 2,4,6
nested 2+11|2+21 / 3+11|3+21
deep 55
grow 1,2 2
holes 5 0,2,4 1,3,5
shrink 1:2:3:: 3
reduce mutating 104
gen 1+2,2+4
getters A,23,C
sort threw cmp
sort nested 1,2,3
sort 5,4,3,2,1
this strict undefined 5
this sloppy true object
this arrow object
rest 3,3 3
defaults 10e,21e
reduce rest 18
class TypeError
bound 11,12
proxy 7,14
native 1,2,3 A,B
map 1a,3c,4d
set 1,2,3,4,5
map threw mapthrow
replace Axya-Bxyb-Cxyc
replace groups {"y":"2024","m":"01"}
json {"a":[10,20,{}]}
overflow true
after overflow 2,3,4
made 286 v70019 285285
every some find true true 2 2 2
async 3,6,9
"#;
    // With as much stack as an app has (rsui-tsx runs on a 64 MB thread and says so): built for testing, each callback
    // from Rust takes tens of kilobytes, so the engine's default of 850 KB, meant for any thread, runs out in this
    // program's nesting.
    const STACK: usize = 64 * 1024 * 1024;
    let ran = std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || run_both_ways_with_stack(program, Some(STACK / 8 * 7)))
        .expect("the system has room for a thread")
        .join()
        .expect("the program ran");
    assert_eq!(ran, expected.trim());
}

/// `call`, `apply` and bound functions are made in place in the interpreter (`Vm::forward_in_place`): every way of
/// writing one should still give what calling the function directly does, including one inside another, `this`,
/// missing and extra arguments, what isn't a plain list, errors, and a deep stack of them.
#[test]
fn call_apply_and_bound_functions_call_what_they_stand_for() {
    assert_eq!(
        run(r#"
            function show(a, b, c) { return [this === undefined ? 'u' : this.n, a, b, c, arguments.length].join(); }
            function sloppy() { return typeof this; }
            const o = { n: 1 };
            const bound = show.bind(o, 'x');
            const bound2 = bound.bind(null, 'y');
            const out = [
                show.call(o, 1, 2, 3), show.call(o), show.call(), show.call(o, 1, 2, 3, 4),
                show.apply(o, [1, 2]), show.apply(o), show.apply(o, null), show.apply(o, { length: 2, 0: 'p', 1: 'q' }),
                show.apply(o, [1, , 3]), (function () { return show.apply(o, arguments); })(7, 8),
                bound(1, 2), bound2(3), bound2.call({ n: 9 }, 4), bound.apply(null, [5, 6]),
                Function.prototype.call.call(show, o, 1), Function.prototype.apply.call(show, o, [2]),
                show.call.call(show.call, show, o, 'deep'),
                sloppy.call(5), sloppy.call(undefined), [1, 2].map(show.bind(o)).join('|'),
                new (show.bind(o, 1))() instanceof show,
            ];
            console.log(out.join(' / '));
            try { (1).call(); } catch (e) { console.log(e.constructor.name); }
            try { show.call.call(5); } catch (e) { console.log(e.constructor.name); }
            class K {}
            try { K.call(null); } catch (e) { console.log(e.message); }
            try { (() => { throw new Error('inside'); }).apply(null, []); } catch (e) { console.log(e.message); }
            function down(n) { return n === 0 ? 0 : 1 + down.call(null, n - 1); }
            console.log(down(5000));
            const sum = (...xs) => xs.reduce((a, b) => a + b, 0);
            console.log(sum.apply(null, Array.from({ length: 1000 }, (_, i) => i)));
            "#),
        "1,1,2,3,3 / 1,,,,0 / ,,,,0 / 1,1,2,3,4 / 1,1,2,,2 / 1,,,,0 / 1,,,,0 / 1,p,q,,2 / 1,1,,3,3 / 1,7,8,,2 / \
         1,x,1,2,3 / 1,x,y,3,3 / 1,x,y,4,3 / 1,x,5,6,3 / 1,1,,,1 / 1,2,,,1 / 1,deep,,,1 / object / object / \
         1,1,0,1,2,3|1,2,1,1,2,3 / true\n\
         TypeError\nTypeError\nClass constructor K cannot be invoked without 'new'\ninside\n5000\n499500"
    );
}

/// `for of` carries a generator on in the interpreter that's running (`Vm::step_in_place`) rather than from Rust:
/// what it yields, where it ends, `break` running its `finally`, errors thrown out of it or caught in it, one that's
/// running already, `yield*`, many inside each other, its arguments, one started with `next()`, and one that's done.
#[test]
fn for_of_carries_generators_on_in_place() {
    assert_eq!(
        run(r#"
            const log = [];
            function* range(n) { for (let i = 0; i < n; i++) yield i; return 'end'; }
            function* map(it, f) { for (const v of it) yield f(v); }
            function* take(it, n) { for (const v of it) { if (n-- <= 0) return; yield v; } }
            let sum = 0;
            for (const v of take(map(range(100), (x) => x * 3), 10)) sum += v;
            log.push(sum);
            function* guarded() { try { yield 1; yield 2; yield 3; } finally { log.push('finally'); } }
            for (const v of guarded()) { if (v === 2) break; log.push(v); }
            function* thrower() { yield 1; throw new Error('boom'); }
            try { for (const v of thrower()) log.push('got ' + v); } catch (e) { log.push(e.message); }
            function* catcher() { try { yield 1; null.x; } catch (e) { yield 'caught'; } yield 'after'; }
            for (const v of catcher()) log.push(v);
            const g = range(3);
            for (const v of g) log.push('a' + v);
            for (const v of g) log.push('again' + v);
            log.push(JSON.stringify(g.next()));
            function* self() { for (const v of me) yield v; }
            const me = self();
            try { for (const v of me) log.push(v); } catch (e) { log.push(e.constructor.name); }
            function* outer() { yield* range(2); yield 'x'; const r = yield* range(1); yield r; }
            for (const v of outer()) log.push('o' + v);
            function* deep(n) { if (n > 0) for (const v of deep(n - 1)) yield v + 1; else yield 0; }
            for (const v of deep(500)) log.push('deep' + v);
            function* withArgs() { yield arguments.length; yield arguments[1]; }
            for (const v of withArgs(4, 5, 6)) log.push('args' + v);
            function* resumed() { const x = yield 1; log.push('x=' + x); yield 2; }
            const r = resumed(); log.push(r.next().value);
            for (const v of r) log.push('rest' + v);
            function* inner() { yield 1; yield 2; }
            function* uses() { for (const a of inner()) { for (const b of inner()) yield a * 10 + b; } }
            log.push([...uses()].join());
            for (const v of uses()) log.push('u' + v);
            const [a, b, ...rest] = range(5);
            log.push(a, b, rest.length);
            function* thrower2() { yield 1; return thrower2.missing.x; }
            try { for (const v of thrower2()) log.push('t' + v); } catch (e) { log.push(e.constructor.name); }
            const t = thrower2(); try { for (const v of t) {} } catch (e) {} log.push(JSON.stringify(t.next()));
            console.log(log.join(' '));
            "#),
        "135 1 finally got 1 boom 1 caught after a0 a1 a2 {\"done\":true} TypeError o0 o1 ox o0 oend deep500 args3 \
         args5 1 x=undefined rest2 11,12,21,22 u11 u12 u21 u22 0 1 3 t1 TypeError {\"done\":true}"
    );
}

/// `arguments` in sloppy code is made with the shape the last one had (`Vm::create_arguments`), and its length and
/// elements are read as an array's are: each one should still have its own values, `callee`, iterator and keys.
#[test]
fn arguments_objects_have_their_own_values() {
    assert_eq!(
        run(r#"
            const out = [];
            function f() { return [arguments.length, arguments[0], arguments[5], [...arguments].join('+'), arguments.callee === f, Object.keys(arguments).join('')]; }
            for (let i = 0; i < 3; i++) out.push(f(1, 2, 3).join());
            out.push(f().join());
            function g() { 'use strict'; try { return arguments.callee; } catch (e) { return e.constructor.name + arguments.length; } }
            out.push(g(1), g());
            function h() { arguments[0] = 9; arguments.extra = 1; return arguments[0] + arguments.length + Object.keys(arguments).length; }
            out.push(h(1, 2), h(1, 2));
            console.log(out.join(' / '));
            "#),
        "3,1,,1+2+3,true,012 / 3,1,,1+2+3,true,012 / 3,1,,1+2+3,true,012 / 0,,,,true, / TypeError1 / TypeError0 / 14 / 14"
    );
}

/// Runs `main.js` among these files as a program, to its end, and gives what it printed, and what went wrong after it.
fn run_program(name: &str, files: &[(&str, &str)]) -> String {
    let folder = std::env::temp_dir().join(format!("rsui-js-modules-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    for (file, text) in files {
        std::fs::write(folder.join(file), text).unwrap();
    }
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let ran = engine
        .run_file(&folder.join(files[0].0))
        .and_then(|_| engine.run_until_idle());
    let _ = std::fs::remove_dir_all(&folder);
    let mut said = output.borrow().replace("\r\n", "\n").trim_end().to_owned();
    if let Err(error) = ran {
        if !said.is_empty() {
            said.push('\n');
        }
        said.push_str("threw ");
        said.push_str(error.message.lines().next().unwrap_or_default());
    }
    said
}

#[test]
fn an_import_is_the_variable_it_names_not_a_copy() {
    let said = run_program(
        "live",
        &[
            (
                "main.js",
                "import count, { count as same, inc } from './counter.js';\nimport * as all from './counter.js';\n\
                 console.log(count, same, all.count);\ninc(); inc();\nconsole.log(count, same, all.count, all.default);\n\
                 try { same = 5; } catch (error) { console.log(error.constructor.name); }\n\
                 console.log(Object.keys(all).join(), Object.prototype.toString.call(all), Object.getPrototypeOf(all));",
            ),
            (
                "counter.js",
                "export let count = 0;\nexport function inc() { count++; }\nexport { count as default };",
            ),
        ],
    );
    assert_eq!(
        said,
        "0 0 0\n2 2 2 2\nTypeError\ncount,default,inc [object Module] null"
    );
}

#[test]
fn modules_that_import_each_other_can_call_each_others_functions_before_they_run() {
    let said = run_program(
        "cycle",
        &[
            (
                "main.js",
                "import { a, readB } from './a.js';\nconsole.log(a, readB());",
            ),
            (
                "a.js",
                "import { b, early } from './b.js';\nexport const a = 'A';\nexport function hoisted() { return 'hoisted'; }\n\
                 export function readB() { return b + ' ' + early; }",
            ),
            (
                "b.js",
                "import { hoisted, a } from './a.js';\nexport const b = 'B';\nexport const early = hoisted();\n\
                 let seen; try { seen = a; } catch (error) { seen = error.constructor.name; }\nconsole.log('b first', seen);",
            ),
        ],
    );
    assert_eq!(said, "b first ReferenceError\nA B hoisted");
}

#[test]
fn a_module_can_wait_at_its_top_and_what_imports_it_waits_too() {
    let said = run_program(
        "await",
        &[
            (
                "main.js",
                "import { late } from './late.js';\nimport './slow.js';\nimport './quick.js';\nconsole.log('main', late);\n\
                 const config = await new Promise((done) => setTimeout(() => done({ port: 8080 }), 5));\n\
                 console.log('config', config.port);\nconst { value } = await import('./later.js');\nconsole.log('later', value);",
            ),
            ("late.js", "export const late = await Promise.resolve('waited');"),
            (
                "slow.js",
                "console.log('slow starts'); await 0; console.log('slow ends'); export {};",
            ),
            ("quick.js", "console.log('quick'); export {};"),
            ("later.js", "await null;\nexport const value = 42;"),
        ],
    );
    assert_eq!(
        said,
        "slow starts\nquick\nslow ends\nmain waited\nconfig 8080\nlater 42"
    );
}

#[test]
fn a_main_module_that_fails_after_waiting_says_so() {
    let said = run_program(
        "fails-late",
        &[(
            "main.js",
            "export {};\nconsole.log('before');\nawait null;\nthrow new RangeError('too late');",
        )],
    );
    assert_eq!(said, "before\nthrew RangeError: too late");
}

#[test]
fn commonjs_and_es_modules_import_each_other_as_node_has_them() {
    let said = run_program(
        "commonjs",
        &[
            (
                "main.js",
                "import lib, { named, counter, bump } from './lib.cjs';\nimport * as all from './lib.cjs';\n\
                 bump();\nconsole.log(named, counter, lib.counter, typeof all['module.exports']);\n\
                 import('./waits.js').then(() => console.log('waited'));\n\
                 try { require; } catch (error) { console.log(error.constructor.name); }",
            ),
            (
                "lib.cjs",
                "exports.named = 'named'; exports.counter = 0; exports.bump = () => { exports.counter++; };\n\
                 const esm = require('./esm.js');\nconsole.log(esm.value, esm.default, esm.__esModule);\n\
                 import('./waits.js');\n\
                 try { require('./waits.js'); } catch (error) { console.log(error.code); }",
            ),
            ("esm.js", "export const value = 'value';\nexport default 'default';"),
            ("waits.js", "await null;\nexport const done = true;"),
        ],
    );
    assert_eq!(
        said,
        "value default true\nERR_REQUIRE_ASYNC_MODULE\nnamed 0 1 object\nReferenceError\nwaited"
    );
}

/// TypeScript takes away an import that's only used as a type, so a type can be imported by name without saying `type`, as
/// apps do, whether it's an `export type`, an `interface`, passed on from elsewhere, or only in a package's `.d.ts`. In
/// TypeScript, a name that isn't exported can only be a type, as the app wouldn't have compiled otherwise.
#[test]
fn a_type_imported_by_its_name_is_left_out_as_typescript_does() {
    let said = run_program(
        "types",
        &[
            (
                "main.ts",
                "import { Shape, Named, Other, Passed, real } from './shapes.ts';\nconst one = { kind: 'a' } as Shape;\nconst two: Named = { name: real };\nconsole.log(one.kind, two.name, typeof Other, typeof Passed);",
            ),
            (
                "shapes.ts",
                "export type Shape = { kind: string };\nexport interface Named { name: string }\ntype Hidden = number;\nexport type { Hidden as Other };\nexport { Passed } from './more.ts';\nexport const real = 'real';",
            ),
            ("more.ts", "export interface Passed { at: number }"),
        ],
    );
    assert_eq!(said, "a real undefined undefined");
    // JavaScript has no types to import, so a name that isn't there is still an error there.
    let missing = run_program(
        "not-a-type",
        &[
            ("main.js", "import { nope } from './a.ts';\nconsole.log(nope);"),
            ("a.ts", "export type yes = 1;\nexport const no = 0;"),
        ],
    );
    assert!(missing.starts_with("threw SyntaxError"), "{missing}");
}

#[test]
fn what_a_module_does_wrong_is_found_before_any_of_it_runs() {
    let missing = run_program(
        "missing",
        &[
            ("main.js", "console.log('ran');\nimport { nope } from './a.js';"),
            ("a.js", "export const yes = 1;"),
        ],
    );
    assert!(
        missing.starts_with(
            "threw SyntaxError: The requested module './a.js' does not provide an export named 'nope'"
        ),
        "{missing}"
    );
    let nested = run_program(
        "nested",
        &[("main.js", "console.log('ran');\nif (true) { import x from './a.js'; }")],
    );
    assert!(nested.starts_with("threw SyntaxError"), "{nested}");
    let twice = run_program(
        "twice",
        &[("main.js", "console.log('ran');\nconst a = 1;\nexport { a, a as b, a as b };")],
    );
    assert!(twice.starts_with("threw SyntaxError"), "{twice}");
}

#[test]
fn a_module_that_threw_throws_the_same_again() {
    let said = run_program(
        "throws",
        &[
            (
                "main.js",
                "const seen = [];\nfor (let i = 0; i < 2; i++) {\n  try { await import('./bad.js'); } catch (error) { seen.push(error); }\n}\n\
                 console.log(seen.length, seen[0] === seen[1], seen[0].message);",
            ),
            (
                "bad.js",
                "console.log('runs once');\nexport const x = 1;\nthrow new Error('bad');",
            ),
        ],
    );
    assert_eq!(said, "runs once\n2 true bad");
}

#[test]
fn import_meta_and_attributes_say_what_node_says() {
    let said = run_program(
        "meta",
        &[
            (
                "main.js",
                "import data from './data.json' with { type: 'json' };\nimport { other } from './other.js';\n\
                 console.log(import.meta.url.startsWith('file:///'), import.meta.url.endsWith('/main.js'), import.meta === other);\n\
                 console.log(import.meta.filename.endsWith('main.js'), typeof import.meta.dirname, import.meta.resolve('./other.js').endsWith('/other.js'));\n\
                 console.log(data.name);",
            ),
            ("other.js", "export const other = import.meta;"),
            ("data.json", "{ \"name\": \"json\" }"),
        ],
    );
    assert_eq!(said, "true true false\ntrue string true\njson");
}

#[test]
fn leaving_a_for_await_early_waits_for_its_iterator_to_finish() {
    assert_eq!(
        run(
            "async function* g() { try { yield 1; yield 2; } finally { await null; console.log('cleaned up'); } }\n\
             (async () => {\n\
               for await (const x of g()) { console.log(x); break; }\n\
               console.log('after the loop');\n\
               const first = await (async () => { for await (const x of g()) { return x; } })();\n\
               console.log('returned', first);\n\
             })();"
        ),
        "1\ncleaned up\nafter the loop\ncleaned up\nreturned 1"
    );
}

#[test]
fn what_one_case_of_a_switch_declares_another_cant() {
    assert!(
        error_of("switch (1) { case 1: const a = 1; break; case 2: const a = 2; }")
            .contains("already been declared")
    );
    assert_eq!(
        run("switch (2) { case 1: const a = 1; break; case 2: { const a = 2; console.log(a); } }"),
        "2"
    );
}

/// What `using` declares is disposed of when its block, its turn of a loop or its loop is left, in any way. What it prints is what Node 24 prints for the same program.
#[test]
fn using_disposes_last_first_however_its_block_is_left() {
    assert_eq!(
        run(r#"const log = [];
const res = (name, fail) => ({ [Symbol.dispose]() { log.push(name); if (fail) throw new Error(name); } });
function returns() { using a = res('a'), b = res('b'); log.push('body'); return 'r'; }
log.push(returns());
for (const i of [0, 1, 2]) { using x = res('turn' + i); if (i === 0) continue; if (i === 1) break; }
for (using x of [res('of1'), null, res('of2')]) log.push('of');
for (using x = res('head'), y = null; ;) { log.push('loop'); break; }
try {
    { using a = res('a', true); using b = res('b', true); throw new Error('body'); }
} catch (e) {
    log.push(`${e.name}: ${e.message} | ${e.error.message} | ${e.suppressed.name} ${e.suppressed.error.message} ${e.suppressed.suppressed.message}`);
}
try { { using a = res('only', true); } } catch (e) { log.push(e.constructor === Error, e.message); }
function* gen() { using a = res('gen'); yield 1; yield 2; }
const g = gen(); g.next(); log.push(JSON.stringify(g.return(9)));
for (const bad of [1, {}, { [Symbol.dispose]: 1 }]) {
    try { using x = bad; } catch (e) { log.push(e.name); }
}
{ using f = class { static [Symbol.dispose]() { log.push("class"); } }; log.push(f.name); }
{ let using = 2; log.push(using); }
var using = [5]; using[0] = 6; log.push(using[0]);
{ using
  x = 7; log.push(typeof x); }
const e = new SuppressedError(1, 2, 'm');
log.push(e.message, e.error, e.suppressed, Object.getOwnPropertyNames(e).filter((n) => n !== 'stack').join(), SuppressedError.length);
log.push(Object.getPrototypeOf(SuppressedError) === Error, e instanceof Error, SuppressedError.prototype.name);
const iter = [1, 2][Symbol.iterator]();
log.push(typeof Iterator.prototype[Symbol.dispose], String(Symbol.dispose), String(Symbol.asyncDispose));
const closing = { next() { return { done: false }; }, return() { log.push('closed'); return {}; } };
Object.setPrototypeOf(closing, Iterator.prototype);
closing[Symbol.dispose]();
console.log(log.join('\n'));"#),
        "body\nb\na\nr\nturn0\nturn1\nof\nof1\nof\nof\nof2\nloop\nhead\nb\na\nSuppressedError: An error was suppressed during disposal | a | SuppressedError b body\nonly\ntrue\nonly\ngen\n{\"value\":9,\"done\":true}\nTypeError\nTypeError\nTypeError\nf\nclass\n2\n6\nnumber\nm\n1\n2\nmessage,error,suppressed\n3\ntrue\ntrue\nSuppressedError\nfunction\nSymbol(Symbol.dispose)\nSymbol(Symbol.asyncDispose)\nclosed"
    );
}

/// `await using` waits for each disposal, and for one of nothing, at the same moments as Node. What it prints is what Node 24 prints for the same program.
#[test]
fn await_using_waits_for_each_disposal_as_node_does() {
    assert_eq!(
        run(r#"const log = [];
const ares = (name) => ({ async [Symbol.asyncDispose]() { log.push('async ' + name); } });
const res = (name) => ({ [Symbol.dispose]() { log.push('sync ' + name); } });
const tick = (label, n) => { let p = Promise.resolve(); for (let i = 1; i <= n; i++) p = p.then(() => log.push(label + i)); };
(async () => {
    tick('t', 4);
    { await using a = ares('a'), b = res('b'), c = null; log.push('body'); }
    log.push('after');
    tick('u', 3);
    { await using n = null; log.push('null body'); }
    log.push('after null');
    await null; await null; await null;
    try {
        await using a = { [Symbol.asyncDispose]() { return Promise.reject(new Error('rejected')); } };
        await using b = { [Symbol.dispose]() { throw new Error('thrown'); } };
    } catch (e) {
        log.push(`${e.name} ${e.error.message} ${e.suppressed.message}`);
    }
    for await (using x of [res('fa1'), res('fa2')]) log.push('for await');
    for (await using x of [ares('fu1'), null]) log.push('for await using');
    async function f() { await using a = ares('in f'); return 'f done'; }
    log.push(await f());
    async function* ag() { try { yield 1; } finally { log.push('ag finally'); } }
    const it = ag(); await it.next(); await it[Symbol.asyncDispose](); log.push('ag disposed');
    console.log(log.join('\n'));
})();"#),
        "body\nsync b\nt1\nasync a\nt2\nafter\nnull body\nt3\nu1\nafter null\nt4\nu2\nu3\nSuppressedError rejected thrown\nfor await\nsync fa1\nfor await\nsync fa2\nfor await using\nasync fu1\nfor await using\nasync in f\nf done\nag finally\nag disposed"
    );
}

/// `DisposableStack` and `AsyncDisposableStack`. What it prints is what Node 24 prints for the same program.
#[test]
fn disposable_stacks_dispose_as_using_does() {
    assert_eq!(
        run(r#"const log = [];
const res = (name) => ({ [Symbol.dispose]() { log.push('dispose ' + name); } });
const stack = new DisposableStack();
log.push(stack.use(null), stack.use(res('a')) !== undefined);
log.push(stack.adopt('v', (value) => log.push('adopt ' + value)));
stack.defer(() => log.push('defer'));
log.push(stack.disposed, Object.prototype.toString.call(stack));
const moved = stack.move();
log.push(stack.disposed, moved.disposed, moved instanceof DisposableStack);
try { stack.use(res('late')); } catch (e) { log.push(e.name, e.message); }
moved.dispose();
moved.dispose();
log.push(moved.disposed, DisposableStack.prototype[Symbol.dispose] === DisposableStack.prototype.dispose);
try { DisposableStack(); } catch (e) { log.push(e.name); }
try { DisposableStack.prototype.dispose.call({}); } catch (e) { log.push(e.name); }
try { stack.defer(1); } catch (e) { log.push(e.name); }
class Mine extends DisposableStack {}
const mine = new Mine();
log.push(mine instanceof Mine, mine.move() instanceof Mine);
{
    using s = new DisposableStack();
    s.defer(() => log.push('stack in using'));
}
(async () => {
    const as = new AsyncDisposableStack();
    as.use(res('sync in async'));
    as.use({ async [Symbol.asyncDispose]() { log.push('async one'); } });
    as.adopt(3, async (v) => log.push('adopt async ' + v));
    as.defer(() => { throw new Error('deferred failure'); });
    as.defer(() => { throw new Error('second failure'); });
    try { await as.disposeAsync(); } catch (e) { log.push(`${e.name} ${e.error.message} ${e.suppressed.message}`); }
    log.push(as.disposed, await as.disposeAsync());
    const rejected = await AsyncDisposableStack.prototype.disposeAsync.call({}).catch((e) => e.name);
    log.push(rejected, AsyncDisposableStack.prototype[Symbol.asyncDispose] === AsyncDisposableStack.prototype.disposeAsync);
    log.push(Object.prototype.toString.call(as), AsyncDisposableStack.length);
    console.log(log.join('\n'));
})();"#),
        "\ntrue\nv\nfalse\n[object DisposableStack]\ntrue\nfalse\ntrue\nReferenceError\nCannot call DisposableStack.prototype.use on an already-disposed DisposableStack\ndefer\nadopt v\ndispose a\ntrue\ntrue\nTypeError\nTypeError\nReferenceError\ntrue\nfalse\nstack in using\nadopt async 3\nasync one\ndispose sync in async\nSuppressedError deferred failure second failure\ntrue\n\nTypeError\ntrue\n[object AsyncDisposableStack]\n0"
    );
}

/// What the language says can't be written with `using`, and what still means the variable called `using`.
#[test]
fn using_is_only_a_declaration_where_the_language_says() {
    for source in [
        "{ using x; }",
        "{ using x = null, y; }",
        "{ using {a} = null; }",
        "for (using x in []) {}",
        "switch (1) { case 1: using x = null; }",
        "if (true) using x = null;",
        "label: using x = null;",
        "using x = null;",
        "{ using let = null; }",
        "async function f() { if (true) await using x = null; }",
        "function f() { { const x = 1; using x = null; } }",
    ] {
        // None of it runs, as it can't be compiled.
        let message = error_of(&format!("throw new Error('it ran');\n{source}"));
        assert!(!message.contains("it ran"), "{source}");
    }
    // What it declares is a `const`.
    assert_eq!(
        error_of("{ using x = null; x = 1; }"),
        error_of("{ const x = null; x = 1; }")
    );
    assert_eq!(
        run("var using = [1, 2], of = [[9], [8], [7]], out = [];\n\
             for (using of of [0, 1, 2]) out.push(using);\n\
             var before = using;\n\
             using = [0];\n\
             { using[0] = 3; }\n\
             { using\n x = 1; }\n\
             async function f() { await using\n y = 2; return y; }\n\
             f().then((y) => console.log(out.length, out[0], before, using[0], x, y));"),
        "1 7 7 3 1 2"
    );
}

/// A module can `await using` at its top, and the objects of Node's that have a way to be disposed of in Node 24 do.
#[test]
fn node_objects_are_disposed_of_as_in_node() {
    let folder = std::env::temp_dir().join(format!("rsui-using-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let main = folder.join("main.mjs");
    std::fs::write(
        &main,
        r#"import fs from 'node:fs';
const log = [];
let handle;
{
    await using file = await fs.promises.open(import.meta.filename);
    handle = file;
    log.push(typeof file.fd);
}
log.push(handle.fd);
await handle.close();
await using last = { async [Symbol.asyncDispose]() { console.log(log.join(' ')); } };
log.push('end of module');"#,
    )
    .unwrap();
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let ran = engine.run_file(&main).and_then(|_| engine.run_until_idle());
    let _ = std::fs::remove_dir_all(&folder);
    ran.unwrap();
    assert_eq!(
        output.borrow().trim_end(),
        "number -1 end of module"
    );
}

#[test]
fn a_namespace_is_the_languages_own_kind_of_object() {
    let said = run_program(
        "namespace",
        &[
            (
                "main.js",
                "import * as ns from './lib.js';\n\
                 console.log(JSON.stringify(Object.getOwnPropertyDescriptor(ns, 'a')), Object.isExtensible(ns), Object.isSealed(ns), Object.isFrozen(ns));\n\
                 console.log(Reflect.set(ns, 'a', 5), Reflect.defineProperty(ns, 'a', { value: 1 }), Reflect.defineProperty(ns, 'a', { value: 2 }), Reflect.deleteProperty(ns, 'a'), delete ns.missing);\n\
                 try { ns.a = 3; } catch (error) { console.log(error.constructor.name, error.message); }\n\
                 console.log(Reflect.ownKeys(ns).map(String).join(), Reflect.setPrototypeOf(ns, null), Reflect.setPrototypeOf(ns, {}));\n\
                 console.log(ns);",
            ),
            ("lib.js", "export const a = 1;\nexport default 2;\nexport let later = 3;"),
        ],
    );
    assert_eq!(
        said,
        "{\"value\":1,\"writable\":true,\"enumerable\":true,\"configurable\":false} false true false\n\
         false true false false true\n\
         TypeError Cannot assign to read only property 'a' of object '[object Module]'\n\
         a,default,later,Symbol(Symbol.toStringTag) true false\n\
         [Module: null prototype] { a: 1, default: 2, later: 3 }"
    );
}

#[test]
fn require_of_an_es_module_gives_what_node_gives() {
    let said = run_program(
        "require-esm",
        &[
            (
                "main.cjs",
                "const withDefault = require('./default.mjs');\n\
                 console.log(Object.keys(withDefault).join(), withDefault.__esModule, withDefault === require('./default.mjs'));\n\
                 const without = require('./plain.mjs');\n\
                 console.log(Object.keys(without).join(), without.__esModule);\n\
                 console.log(JSON.stringify(require('./whole.mjs')));\n\
                 import('./plain.mjs').then((ns) => console.log(ns === without));",
            ),
            ("default.mjs", "export const a = 1;\nexport default 2;"),
            ("plain.mjs", "export const b = 1;"),
            (
                "whole.mjs",
                "const value = { custom: true };\nexport { value as 'module.exports' };\nexport const c = 3;",
            ),
        ],
    );
    assert_eq!(said, "__esModule,a,default true true\nb undefined\n{\"custom\":true}\ntrue");
}

#[test]
fn a_package_can_say_its_files_are_es_modules() {
    let folder = std::env::temp_dir().join(format!("rsui-js-type-{}", std::process::id()));
    std::fs::create_dir_all(folder.join("esm")).unwrap();
    std::fs::create_dir_all(folder.join("plain")).unwrap();
    std::fs::write(folder.join("esm").join("package.json"), "{ \"type\": \"module\" }").unwrap();
    std::fs::write(folder.join("esm").join("side.js"), "console.log('module', typeof require, this);").unwrap();
    std::fs::write(folder.join("plain").join("side.js"), "console.log('commonjs', typeof require);").unwrap();
    std::fs::write(folder.join("main.js"), "import './esm/side.js';\nimport './plain/side.js';").unwrap();
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let ran = engine.run_file(&folder.join("main.js")).map(|_| ());
    let _ = std::fs::remove_dir_all(&folder);
    ran.unwrap();
    assert_eq!(output.borrow().replace("\r\n", "\n").trim_end(), "module undefined undefined\ncommonjs function");
}

#[test]
fn file_urls_and_the_older_url_functions_are_what_node_has() {
    let said = run_program(
        "url",
        &[(
            "main.mjs",
            "import { fileURLToPath, pathToFileURL, parse, format, resolve, domainToASCII } from 'node:url';\n\
             console.log(fileURLToPath('file:///C:/Users/a%20b/c.txt'), fileURLToPath('file://server/share/x.txt'));\n\
             console.log(pathToFileURL('C:\\\\Users\\\\a b\\\\c#d.txt').href, fileURLToPath(import.meta.url).endsWith('main.mjs'));\n\
             const p = parse('http://user:pw@Example.COM:8080/p/a/t/h?query=string#hash', true);\n\
             console.log(p.auth, p.host, p.pathname, p.query.query, p.hash, parse('mailto:someone@example.com').hostname);\n\
             console.log(format({ protocol: 'https', hostname: 'example.com', pathname: '/a b', query: { x: 1 } }), resolve('/one/two/three', 'four'), resolve('http://a.com/b/c', '../d'));\n\
             console.log(domainToASCII('espa\u{f1}ol.com'), new URL('http://Espa\u{f1}ol.com/').hostname);",
        )],
    );
    assert_eq!(
        said,
        "C:\\Users\\a b\\c.txt \\\\server\\share\\x.txt\n\
         file:///C:/Users/a%20b/c%23d.txt true\n\
         user:pw example.com:8080 /p/a/t/h string #hash example.com\n\
         https://example.com/a b?x=1 /one/two/four http://a.com/d\n\
         xn--espaol-zwa.com xn--espaol-zwa.com"
    );
}

/// What a pause saw: why, the line and function, and the innermost function's variables as `console.log` shows them.
#[derive(Debug)]
struct Seen {
    reason: crate::PauseReason,
    line: u32,
    function: String,
    locals: Vec<String>,
    outer: Vec<String>,
    worked_out: Option<String>,
}

fn locals_of(vm: &mut Vm, frame: &crate::FrameView) -> Vec<String> {
    frame
        .locals
        .iter()
        .filter(|(name, _)| name != "this")
        .map(|(name, value)| format!("{name}={}", vm.describe(*value, 40)))
        .collect()
}

#[test]
fn a_debugger_stops_at_breakpoints_steps_and_sees_variables() {
    let folder = std::env::temp_dir().join(format!("rsui-js-debugger-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let main = folder.join("main.js");
    std::fs::write(
        &main,
        "function add(a, b) {
    const sum = a + b;
    return sum;
}
let total = 0;
for (let i = 0; i < 3; i++) {
    total = add(total, i);
}
console.log(`total ${total}`);
",
    )
    .unwrap();
    let elsewhere = folder.join("other.js");
    std::fs::write(&elsewhere, "let n = 0;\nfor (let i = 0; i < 10; i++) { n += i; }\n").unwrap();

    let seen = Rc::new(RefCell::new(Vec::<Seen>::new()));
    let resumes = Rc::new(RefCell::new(
        vec![
            crate::Resume::Into,
            crate::Resume::Over,
            crate::Resume::Out,
            crate::Resume::Continue,
            crate::Resume::Continue,
        ]
        .into_iter(),
    ));
    let debugged = main.clone();
    crate::set_debug_files(Some(Rc::new(move |path| path == debugged)));
    let kept = seen.clone();
    let next = resumes.clone();
    crate::set_pause_handler(Some(Rc::new(move |vm, pause| {
        let frames = vm.paused_frames(4);
        let locals = locals_of(vm, &frames[0]);
        let outer = frames.get(1).map(|frame| locals_of(vm, frame)).unwrap_or_default();
        let worked_out = (pause.at.line == 3).then(|| {
            let value = vm.evaluate_in_frame(0, "a + b + sum + 100").unwrap();
            vm.describe(value, 40)
        });
        kept.borrow_mut().push(Seen {
            reason: pause.reason,
            line: pause.at.line,
            function: pause.at.function.clone(),
            locals,
            outer,
            worked_out,
        });
        next.borrow_mut().next().unwrap_or(crate::Resume::Continue)
    })));
    crate::set_breakpoints(&[(main.to_string_lossy().into_owned(), 7)]);

    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let ran = engine.run_file(&main);
    let other = engine.run_file(&elsewhere);
    let mut steps = |file: &std::path::Path| {
        let atom = engine.vm.intern_str(&file.to_string_lossy());
        engine
            .vm
            .protos
            .iter()
            .filter(|proto| proto.file == atom && proto.start != crate::op::NOT_COMPILED)
            .map(|proto| {
                engine.vm.code[proto.start as usize..]
                    .iter()
                    .take_while(|op| !matches!(op, crate::op::Op::Return { .. } | crate::op::Op::ReturnUndefined))
                    .filter(|op| matches!(op, crate::op::Op::Step { .. }))
                    .count()
            })
            .sum::<usize>()
    };
    let (in_main, in_other) = (steps(&main), steps(&elsewhere));

    // Asked from another thread, a pause stops a loop that's busy, at its next statement.
    let busy = folder.join("busy.js");
    std::fs::write(&busy, "let n = 0;\nwhile (!globalThis.done) {\n    n++;\n}\nconsole.log(`stopped ${n > 0}`);\n").unwrap();
    let debugged = busy.clone();
    crate::set_debug_files(Some(Rc::new(move |path| path == debugged)));
    crate::set_breakpoints(&[]);
    let asked = Rc::new(RefCell::new(None));
    let kept = asked.clone();
    crate::set_pause_handler(Some(Rc::new(move |vm, pause| {
        let n = vm.evaluate_in_frame(0, "n").unwrap();
        *kept.borrow_mut() = Some((pause.reason, pause.at.line, n.as_number() > 0.0));
        vm.evaluate_in_frame(0, "globalThis.done = true").unwrap();
        crate::Resume::Continue
    })));
    let handle = crate::pause_handle();
    let asker = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        handle.pause();
    });
    let stopped = engine.run_file(&busy);
    asker.join().unwrap();
    crate::set_pause_handler(None);
    crate::set_debug_files(None);
    let _ = std::fs::remove_dir_all(&folder);
    ran.unwrap();
    other.unwrap();
    stopped.unwrap();

    let seen = seen.borrow();
    let summary: Vec<String> = seen
        .iter()
        .map(|one| format!("{:?} {} {} [{}]", one.reason, one.function, one.line, one.locals.join(" ")))
        .collect();
    assert_eq!(seen.len(), 5, "{summary:#?}");
    assert_eq!((seen[0].reason, seen[0].line), (crate::PauseReason::Breakpoint, 7), "{summary:#?}");
    assert!(seen[0].locals.contains(&"i=0".to_owned()), "{summary:#?}");
    assert!(seen[0].locals.contains(&"total=0".to_owned()), "{summary:#?}");
    // Into goes into `add`, over goes to its next line, and out comes back to the loop, once round it.
    assert_eq!((seen[1].reason, seen[1].function.as_str(), seen[1].line), (crate::PauseReason::Step, "add", 2));
    // `sum` hasn't been given its value yet, which a register that a call cleared shows as undefined.
    assert_eq!(seen[1].locals, ["a=0", "b=0", "sum=undefined"], "{summary:#?}");
    assert!(seen[1].outer.contains(&"i=0".to_owned()), "what called it is seen too: {:?}", seen[1].outer);
    assert_eq!(seen[2].line, 3);
    assert!(seen[2].locals.contains(&"sum=0".to_owned()), "{summary:#?}");
    assert_eq!(seen[2].worked_out.as_deref(), Some("100"));
    assert_eq!((seen[3].reason, seen[3].line), (crate::PauseReason::Step, 7), "{summary:#?}");
    assert!(seen[3].locals.contains(&"i=1".to_owned()), "{summary:#?}");
    // A breakpoint in a loop stops each time round.
    assert_eq!((seen[4].reason, seen[4].line), (crate::PauseReason::Breakpoint, 7), "{summary:#?}");
    assert!(seen[4].locals.contains(&"i=2".to_owned()), "{summary:#?}");
    assert!(seen[4].locals.contains(&"total=1".to_owned()), "{summary:#?}");
    assert!(output.borrow().contains("total 3"), "{}", output.borrow());
    assert!(in_main >= 5, "each statement of the debugged file has somewhere to stop: {in_main}");
    assert_eq!(in_other, 0, "a file that isn't being debugged has nowhere to stop");

    assert_eq!(*asked.borrow(), Some((crate::PauseReason::Asked, 3, true)));
    assert!(output.borrow().contains("stopped true"), "{}", output.borrow());
}

#[test]
fn a_rejection_nothing_catches_goes_to_what_asked_for_them_and_is_no_error() {
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let handler = engine
        .eval(
            "(reason, promise) => { console.log('heard ' + reason.message + ' ' + (promise instanceof Promise)); if (reason.message === 'first') Promise.reject(new Error('from the handler')); }",
            "test.js",
        )
        .unwrap();
    engine.report_rejections_to(handler);
    engine
        .eval(
            "Promise.reject(new Error('first')); Promise.reject(new Error('caught')).catch(() => console.log('caught'));",
            "test.js",
        )
        .unwrap();
    // It's no error, and what the handler leaves is heard too.
    engine.run_jobs().unwrap();
    assert_eq!(output.borrow().trim_end(), "caught\nheard first true\nheard from the handler true");
}

#[test]
fn pbkdf2_matches_frozen_node_and_rfc_vectors() {
    assert_eq!(
        run_as_file(
            "pbkdf2_vectors",
            include_str!("../tests/data/crypto-vectors.js")
        ),
        "crypto checks 250"
    );
}

#[test]
fn pbkdf2_preserves_fallback_coercions_and_validation_order() {
    assert_eq!(
        run_as_file(
            "pbkdf2_fallback",
            r#"
        const c = require('crypto');
        const { Buffer } = require('buffer');
        const assert = require('assert');
        const derive = (p, s, r, l, d = 'sha256') => c.pbkdf2Sync(p, s, r, l, d).toString('hex');
        const normal = derive('pw', 'salt', 2, 40);
        assert.strictEqual(derive('pw', 'salt', '2', 40), normal);
        assert.strictEqual(derive('pw', 'salt', 1.5, 40), normal);
        assert.strictEqual(derive('pw', 'salt', { valueOf() { return 2; } }, 40), normal);
        for (const rounds of [0, -1, NaN, undefined]) {
            assert.strictEqual(derive('pw', 'salt', rounds, 40), derive('pw', 'salt', 1, 40));
        }
        assert.strictEqual(derive('pw', 'salt', 2, '40'), normal);
        assert.strictEqual(derive('pw', 'salt', 2, 40.5), normal);
        assert.strictEqual(derive('pw', 'salt', 2, 40, { toString() { return 'SHA-256'; } }), normal);
        assert.strictEqual(derive(new Uint16Array([0x170, 0x177]), 'salt', 2, 40), normal);
        assert.strictEqual(derive([112, 119], 'salt', 2, 40), normal);
        assert.strictEqual(derive(new Uint8Array([112, 119]).buffer, 'salt', 2, 40), normal);
        for (const hash of c.getHashes()) {
            assert.strictEqual(derive('pw', 'salt', 2, 0, hash), '');
            assert.strictEqual(derive('pw', null, 2, 0, hash), '');
        }
        const converted = new Uint8Array([1, 2]);
        converted.valueOf = () => 'pw';
        assert.strictEqual(derive(converted, 'salt', 2, 40), normal);
        let order = [];
        const password = { valueOf() { order.push('password'); return 'pw'; } };
        const salt = { valueOf() { order.push('salt'); return 'salt'; } };
        const digest = { toString() { order.push('digest'); return 'bogus'; } };
        assert.throws(() => derive(password, salt, 2, -1, digest), /Digest method not supported/);
        assert.strictEqual(order.join(','), 'digest');
        assert.throws(() => derive(null, salt, 2, -1), /first argument/);
        assert.throws(() => derive('pw', salt, 2, -1), RangeError);
        assert.strictEqual(order.join(','), 'digest');
        assert.throws(() => derive('pw', new DataView(new ArrayBuffer(4)), 2, 1), /first argument/);
        assert.throws(() => derive(new DataView(new ArrayBuffer(4)), 'salt', 2, 0), /first argument/);
        console.log('fallback ok');
    "#
        ),
        "fallback ok"
    );
}

#[test]
fn pbkdf2_preserves_inherited_input_conversions() {
    assert_eq!(
        run_as_file(
            "pbkdf2_inherited",
            r#"
        const c = require('crypto');
        const { Buffer } = require('buffer');
        const assert = require('assert');
        const derive = (p, s = 'salt', length = 40) => c.pbkdf2Sync(p, s, 2, length, 'sha256').toString('hex');
        const expected = derive('pw');
        for (const proto of [Buffer.prototype, Uint8Array.prototype, Object.getPrototypeOf(Uint8Array.prototype), Object.prototype]) {
            const password = Buffer.from([1, 2]);
            const previous = Object.getOwnPropertyDescriptor(proto, 'valueOf');
            Object.defineProperty(proto, 'valueOf', {
                configurable: true,
                value() { return this === password ? 'pw' : this; }
            });
            let result;
            try { result = derive(password); }
            finally {
                if (previous) Object.defineProperty(proto, 'valueOf', previous);
                else delete proto.valueOf;
            }
            assert.strictEqual(result, expected);
        }
        // Strings also pass through temporary Buffers on the compatibility path.
        Buffer.prototype.valueOf = function () { return this.toString() === 'source' ? 'pw' : this; };
        let result;
        try { result = derive('source'); }
        finally { delete Buffer.prototype.valueOf; }
        assert.strictEqual(result, expected);
        const password = new Uint8Array([1, 2]);
        password.data = [112, 119];
        let saltReads = 0;
        const salt = new Uint8Array([3, 4]);
        Object.defineProperty(Uint8Array.prototype, 'type', {
            configurable: true,
            get() {
                if (this === salt) saltReads++;
                return this === password ? 'Buffer' : undefined;
            }
        });
        let empty;
        try { result = derive(password); empty = derive('pw', salt, 0); }
        finally { delete Uint8Array.prototype.type; }
        assert.strictEqual(result, expected);
        assert.strictEqual(empty, '');
        assert.strictEqual(saltReads, 0);
        console.log('inherited conversions ok');
    "#
        ),
        "inherited conversions ok"
    );
}

#[test]
fn pbkdf2_callbacks_remain_deferred_once_and_snapshot_in_the_microtask() {
    assert_eq!(
        run_as_file(
            "pbkdf2_async",
            r#"
        const c = require('crypto');
        const assert = require('assert');
        const { Buffer } = require('buffer');
        let inline = true, success = 0, failure = 0;
        const password = Buffer.from('pw');
        c.pbkdf2(password, 'salt', 2, 40, 'sha256', (error, result) => {
            assert.strictEqual(inline, false);
            assert.strictEqual(error, null);
            assert.strictEqual(result.toString('hex'), c.pbkdf2Sync('xw', 'salt', 2, 40, 'sha256').toString('hex'));
            success++;
        });
        c.pbkdf2(null, null, 2, -1, 'bogus', error => {
            assert.strictEqual(inline, false);
            assert.strictEqual(error.message, 'Digest method not supported');
            failure++;
        });
        password[0] = 120;
        inline = false;
        queueMicrotask(() => console.log(success, failure));
    "#
        ),
        "1 1"
    );
}

#[test]
fn numbers_turned_into_strings_again_are_the_same_strings_through_collections() {
    // Each number's string is made once between collections and handed out again after, so it has to be the right
    // one even when a collection came in between, and when what it was made for has let go of it.
    assert_eq!(
        run(r#"
            const seen = [];
            let junk = [];
            for (let round = 0; round < 40; round++) {
                for (let i = 0; i < 2000; i++) junk.push({ i, s: '' + (i + 0.5) });
                junk = [];
                seen.push(String(1.2) + (round % 7) + ('' + -0) + (1e21 + '') + String(NaN) + String(2 ** 31) + (0.1 + 0.2));
            }
            console.log(new Set(seen).size, seen[39], String(3.25) === '' + 3.25, [1.5, 1.5, 2.5].join());
        "#),
        "7 1.2401e+21NaN21474836480.30000000000000004 true 1.5,1.5,2.5"
    );
}

#[test]
fn closures_with_any_number_of_captured_variables_keep_them_through_collections() {
    assert_eq!(
        run(r#"
            function make(n, base) {
                const a = base, b = base + 1, c = base + 2, d = base + 3, e = base + 4, f = base + 5, g = base + 6,
                    h = base + 7, i = base + 8, j = base + 9;
                switch (n) {
                    case 1: return () => a;
                    case 2: return () => a + b;
                    case 4: return () => a + b + c + d;
                    case 8: return () => a + b + c + d + e + f + g + h;
                    default: return () => a + b + c + d + e + f + g + h + i + j;
                }
            }
            let total = 0;
            const kept = [];
            for (let round = 0; round < 3000; round++) {
                const fns = [1, 2, 4, 8, 10].map((n) => make(n, round));
                for (const f of fns) total += f();
                if (round % 300 === 0) kept.push(fns);
            }
            const later = kept.map((fns) => fns.map((f) => f()).join('/'));
            console.log(total, later[0], later[9]);
        "#),
        "112702500 0/1/6/28/45 2700/5401/10806/21628/27045"
    );
}

#[test]
fn call_of_a_native_gives_it_the_arguments_after_this() {
    assert_eq!(
        run(r#"
            const o = { a: 1 };
            const has = Object.prototype.hasOwnProperty;
            const arr = [1];
            Array.prototype.push.call(arr, 2, 3);
            console.log(
                has.call(o, 'a'), has.call(o, 'b'), Object.prototype.toString.call([]), Object.prototype.toString.call(null),
                Math.max.call(null, 1, 5, 3), Math.max.call(null), String.prototype.slice.call('hello', 1, 3), arr.join(),
                Function.prototype.call.call(Math.min, null, 4, 2), [].concat.call([1], [2], 3).join(),
            );
            try { has.call(null, 'a'); } catch (e) { console.log(e instanceof TypeError); }
        "#),
        "true false [object Array] [object Null] 5 -Infinity el 1,2,3 2 1,2,3\ntrue"
    );
}

#[test]
fn a_proxy_reads_through_its_handler_as_it_is_now() {
    assert_eq!(
        run(r#"
            const handler = { get(target, key) { return 'first ' + String(key); } };
            const p = new Proxy({ x: 1 }, handler);
            const out = [];
            for (let i = 0; i < 3; i++) out.push(p.x);
            handler.get = (target, key) => 'second ' + String(key);
            out.push(p.y);
            delete handler.get;
            out.push(p.x);
            Object.defineProperty(handler, 'get', { get() { return () => 'from a getter'; }, configurable: true });
            out.push(p.z);
            const other = new Proxy({}, { has() { return true; }, get(t, k) { return k === 'length' ? 3 : undefined; } });
            out.push(other.length, 'q' in other);
            class C { #secret = 4; static peek(o) { return o.#secret; } }
            try { C.peek(new Proxy(new C(), {})); } catch (e) { out.push(e instanceof TypeError); }
            console.log(out.join('|'));
        "#),
        "first x|first x|first x|second y|1|from a getter|3|true|true"
    );
}

#[test]
fn match_reads_flags_through_a_getter_that_was_put_in_place_of_the_built_in_one() {
    assert_eq!(
        run(r#"
            const before = 'a1a2a3'.match(/a\d/);
            const flags = Object.getOwnPropertyDescriptor(RegExp.prototype, 'flags');
            Object.defineProperty(RegExp.prototype, 'flags', { get() { return ''; }, configurable: true });
            const after = 'a1a2a3'.match(/a\d/g);
            Object.defineProperty(RegExp.prototype, 'flags', flags);
            const again = 'a1a2a3'.match(/a\d/g);
            console.log(before.join(), before.index, after.join(), again.join(), ('' + 1.6).match(/^(\d+)\.(\d+)$/).slice(1).join());
        "#),
        "a1 0 a1 a1,a2,a3 1,6"
    );
}

#[test]
fn the_same_string_matched_again_gives_a_new_array_of_the_same_captures() {
    assert_eq!(
        run(r#"
            const re = /^(normal|(\d+(?:\.\d+)?)(px|em|%)?)$/;
            const out = [];
            let junk = [];
            for (let i = 0; i < 3000; i++) {
                const m = ('' + 1.6).match(re);
                if (i === 0) { m[2] = 'changed'; m.extra = 1; }
                if (i % 1000 === 1) out.push(m.join('/'), m.index, m.extra, m.input);
                for (let j = 0; j < 20; j++) junk.push({ j });
                if (junk.length > 5000) junk = [];
            }
            const g = /a(\d)/g;
            out.push('a1a2'.match(g).join(), g.exec('a1a2')[1], g.exec('a1a2')[1], g.exec('a1a2'), /b/.exec('abc').index, /z/.exec('abc'));
            console.log(out.join(' '));
        "#),
        "1.6/1.6/1.6/ 0  1.6 1.6/1.6/1.6/ 0  1.6 1.6/1.6/1.6/ 0  1.6 a1,a2 1 2  1"
    );
}

#[test]
fn slicing_the_same_string_the_same_way_again_gives_the_same_text_through_collections() {
    assert_eq!(
        run(r#"
            const out = [];
            let junk = [];
            for (let i = 0; i < 3000; i++) {
                const tag = Object.prototype.toString.call(i % 2 ? [] : {});
                out.push(tag.slice(0, 7) + tag.slice(-6) + tag.substring(8, 9) + 'abc'.slice(1) + String(i).slice(-1));
                for (let j = 0; j < 20; j++) junk.push({ j });
                if (junk.length > 5000) junk = [];
            }
            console.log(new Set(out).size, out[0], out[1], out[2999], 'hello'.slice(1, 3), 'hello'.slice(1, 3) === 'el');
        "#),
        "10 [objectbject]Obc0 [objectArray]Abc1 [objectArray]Abc9 el true"
    );
}
