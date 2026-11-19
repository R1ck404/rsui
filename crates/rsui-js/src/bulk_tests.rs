//! Tests of bulk.rs: what Rust makes, reads and copies many properties at a time is what JavaScript would have, and a
//! measure of how long each takes against JavaScript doing the same (`cargo test --profile fastrel -p rsui-js bulk_speed --
//! --ignored --nocapture`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use super::*;

thread_local! {
    static KEYS12: Cell<Option<Keys>> = const { Cell::new(None) };
    static KEYS6: Cell<Option<Keys>> = const { Cell::new(None) };
    static STARTED: Cell<Option<Instant>> = const { Cell::new(None) };
}

const TWELVE: [&str; 12] = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"];

fn keys12(vm: &mut Vm) -> Keys {
    KEYS12.with(|keys| {
        keys.get().unwrap_or_else(|| {
            let made = vm.keys(&TWELVE);
            keys.set(Some(made));
            made
        })
    })
}

fn keys6(vm: &mut Vm) -> Keys {
    KEYS6.with(|keys| {
        keys.get().unwrap_or_else(|| {
            let made = vm.keys(&TWELVE[..6]);
            keys.set(Some(made));
            made
        })
    })
}

fn engine() -> (Engine, Rc<RefCell<String>>) {
    KEYS12.with(|keys| keys.set(None));
    KEYS6.with(|keys| keys.set(None));
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let vm = engine.vm();
    let hosts: [(&str, HostFunction); 14] = [
        ("deleteKeys", |vm, args| {
            let object = args.get(vm, 0);
            let mut keys = Vec::new();
            for at in 1..args.count() {
                let name = args.get(vm, at);
                keys.push(vm.property_atom(name)?);
            }
            vm.delete_keys(object, &keys)?;
            Ok(object)
        }),
        ("gc", |vm, _| {
            vm.collect_garbage_now();
            Ok(Value::UNDEFINED)
        }),
        ("now", |_, _| {
            let started = STARTED.with(|at| *at.get().get_or_insert_with(Instant::now));
            let micros = started.elapsed().as_secs_f64() * 1e6;
            STARTED.with(|at| at.set(Some(started)));
            Ok(Value::number(micros))
        }),
        ("nothing", |_, _| Ok(Value::UNDEFINED)),
        ("make12", |vm, args| {
            let keys = keys12(vm);
            let first = args.get(vm, 0);
            let base = if first.is_number() { first.as_number() } else { 0.0 };
            let values: [Value; 12] = std::array::from_fn(|at| Value::number(base + at as f64));
            Ok(vm.new_object_with(keys, &values))
        }),
        ("make12Slowly", |vm, args| {
            let first = args.get(vm, 0);
            let base = if first.is_number() { first.as_number() } else { 0.0 };
            let object = vm.new_object();
            for (at, name) in TWELVE.iter().enumerate() {
                let atom = vm.atom(name);
                vm.set_property(object, atom, Value::number(base + at as f64))?;
            }
            Ok(object)
        }),
        ("make", |vm, args| {
            // make(names, values): through a list made each time, for what the tests ask of it.
            let names = args.get(vm, 0);
            let values = args.get(vm, 1);
            let names: Vec<String> =
                vm.array_elements(names).unwrap_or(&[]).to_vec().into_iter().map(|name| vm.plain_text(name).unwrap_or_default()).collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let keys = vm.keys(&names);
            let values = vm.array_elements(values).unwrap_or(&[]).to_vec();
            Ok(vm.new_object_with(keys, &values))
        }),
        ("read6", |vm, args| {
            let keys = keys6(vm);
            let object = args.get(vm, 0);
            let mut out = [Value::UNDEFINED; 6];
            vm.read_keys(object, keys, &mut out)?;
            Ok(vm.new_array(out.to_vec()))
        }),
        ("sum6", |vm, args| {
            let keys = keys6(vm);
            let object = args.get(vm, 0);
            let mut out = [Value::UNDEFINED; 6];
            vm.read_keys(object, keys, &mut out)?;
            Ok(Value::number(out.iter().map(|value| if value.is_number() { value.as_number() } else { 0.0 }).sum()))
        }),
        ("sum6Own", |vm, args| {
            let keys = keys6(vm);
            let object = args.get(vm, 0);
            let mut out = [Value::UNDEFINED; 6];
            vm.read_own_keys(object, keys, &mut out)?;
            Ok(Value::number(out.iter().map(|value| if value.is_number() { value.as_number() } else { 0.0 }).sum()))
        }),
        ("sum6Slowly", |vm, args| {
            let object = args.get(vm, 0);
            let mut sum = 0.0;
            for name in &TWELVE[..6] {
                let atom = vm.atom(name);
                let value = vm.get_property(object, atom)?;
                if value.is_number() {
                    sum += value.as_number();
                }
            }
            Ok(Value::number(sum))
        }),
        ("assign", |vm, args| {
            let target = args.get(vm, 0);
            for at in 1..args.count() {
                let source = args.get(vm, at);
                vm.assign(target, source)?;
            }
            Ok(target)
        }),
        ("assignAll", |vm, args| {
            // assignAll(target, sources): each source in an array, as a rule's props are put on a look one after another.
            let target = args.get(vm, 0);
            let sources = args.get(vm, 1);
            let count = vm.array_elements(sources).map_or(0, <[Value]>::len);
            for at in 0..count {
                let source = vm.array_elements(sources).expect("an array")[at];
                vm.assign(target, source)?;
            }
            Ok(target)
        }),
        ("assignAllSlowly", |vm, args| {
            let target = args.get(vm, 0);
            let sources = args.get(vm, 1);
            let count = vm.array_elements(sources).map_or(0, <[Value]>::len);
            let mut props = Vec::new();
            for at in 0..count {
                let source = vm.array_elements(sources).expect("an array")[at];
                vm.properties_into(source, &mut props)?;
                for (key, value) in props.drain(..) {
                    vm.set_property(target, key, value)?;
                }
            }
            Ok(target)
        }),
    ];
    for (name, host) in hosts {
        let function = vm.host_function(name, 1, host);
        vm.set_global(name, function);
    }
    (engine, output)
}

fn run(source: &str) -> String {
    let (mut engine, output) = engine();
    let result = engine.eval(source, "test.js").and_then(|_| engine.run_until_idle());
    if let Err(error) = result {
        panic!("{source}\nthrew: {error}");
    }
    output.borrow().trim_end().to_owned()
}

#[test]
fn an_object_made_with_keys_is_what_a_literal_makes() {
    assert_eq!(
        run("const o = make12(1); console.log(JSON.stringify(o), Object.keys(o).length, o instanceof Object);
             const p = make12(2); console.log(p.l, p.a, Object.getPrototypeOf(p) === Object.prototype);
             o.z = 3; delete o.a; console.log(JSON.stringify(o)); gc(); console.log(JSON.stringify(make12(0)));"),
        "{\"a\":1,\"b\":2,\"c\":3,\"d\":4,\"e\":5,\"f\":6,\"g\":7,\"h\":8,\"i\":9,\"j\":10,\"k\":11,\"l\":12} 12 true\n\
         13 2 true\n\
         {\"b\":2,\"c\":3,\"d\":4,\"e\":5,\"f\":6,\"g\":7,\"h\":8,\"i\":9,\"j\":10,\"k\":11,\"l\":12,\"z\":3}\n\
         {\"a\":0,\"b\":1,\"c\":2,\"d\":3,\"e\":4,\"f\":5,\"g\":6,\"h\":7,\"i\":8,\"j\":9,\"k\":10,\"l\":11}"
    );
    // A few, and names that aren't simply in a shape.
    assert_eq!(
        run("console.log(JSON.stringify(make(['x', 'y'], [1, 'two'])), JSON.stringify(make(['1', 'x', '0'], [1, 2, 3])))"),
        "{\"x\":1,\"y\":\"two\"} {\"0\":3,\"1\":1,\"x\":2}"
    );
}

#[test]
fn keys_read_what_a_read_would() {
    assert_eq!(
        run("console.log(JSON.stringify(read6({ a: 1, c: 3, f: 6, z: 0 })), JSON.stringify(read6({ f: 1, e: 2, d: 3, c: 4, b: 5, a: 6 })));
             console.log(JSON.stringify(read6({ a: 1, c: 3, f: 6, z: 0 })), JSON.stringify(read6({ get a() { return 'got' } })));
             Object.prototype.b = 'above'; console.log(JSON.stringify(read6({ a: 1, c: 3, f: 6, z: 0 })));
             delete Object.prototype.b; console.log(JSON.stringify(read6({ a: 1, c: 3, f: 6, z: 0 })), JSON.stringify(read6(null === 1 ? 0 : [1])));
             Object.defineProperty(Object.prototype, 'c', { get() { return 'getter' }, configurable: true });
             console.log(JSON.stringify(read6({ a: 1 })));"),
        "[1,null,3,null,null,6] [6,5,4,3,2,1]\n\
         [1,null,3,null,null,6] [\"got\",null,null,null,null,null]\n\
         [1,\"above\",3,null,null,6]\n\
         [1,null,3,null,null,6] [null,null,null,null,null,null]\n\
         [1,null,\"getter\",null,null,null]"
    );
}

#[test]
fn keys_deleted_together_are_what_deleting_each_does() {
    let cases = [
        "const t = {}; for (let i = 0; i < 40; i++) t['p' + i] = i; deleteKeys(t, 'p3', 'p39', 'nope'); t.q = 1; console.log(JSON.stringify(t), Object.keys(t).length, 'p3' in t)",
        "const t = { a: 1, b: 2 }; deleteKeys(t, 'a'); console.log(JSON.stringify(t)); const u = { a: 3, b: 4 }; deleteKeys(u, 'a'); console.log(JSON.stringify(u))",
        "const t = { a: 1, get g() { return 2 }, c: 3 }; deleteKeys(t, 'a'); console.log(JSON.stringify(t), Object.keys(t))",
        "'use strict'; const t = Object.freeze({ a: 1, b: 2 }); try { deleteKeys(t, 'a') } catch (e) { console.log(e instanceof TypeError) } console.log(JSON.stringify(t))",
        "const t = [1, 2]; t.x = 1; deleteKeys(t, 'x', '0'); console.log(JSON.stringify(t), t.x)",
        "const t = {}; Object.defineProperty(t, 'h', { value: 1, enumerable: false, configurable: true }); t.v = 2; deleteKeys(t, 'v'); console.log(t.h, Object.keys(t), Object.getOwnPropertyNames(t))",
    ];
    for case in cases {
        let one_at_a_time = case.replace("deleteKeys(t, ", "((o, ...k) => k.forEach((n) => delete o[n]))(t, ").replace("deleteKeys(u, ", "((o, ...k) => k.forEach((n) => delete o[n]))(u, ");
        assert_eq!(run(case), run(&one_at_a_time), "{case}");
    }
}

#[test]
fn assign_is_what_object_assign_does() {
    let cases = [
        // Added, replaced, and both, again and again over the same shapes.
        "const out = []; for (let i = 0; i < 3; i++) { const t = { a: 1, b: 2 }; assign(t, { b: 3, c: 4 }, { d: 5, a: 6 }); out.push(JSON.stringify(t)); } console.log(out.join(' '))",
        // Into an empty one, from an empty one, and from itself.
        "const t = {}; assign(t, {}, { x: 1 }, null, undefined); assign(t, t); console.log(JSON.stringify(t))",
        // A setter on the way up is called, and a read-only one isn't written.
        "Object.defineProperty(Object.prototype, 'seen', { set(v) { console.log('set', v) }, configurable: true }); const t = {}; assign(t, { seen: 1, x: 2 }); console.log(JSON.stringify(t), Object.keys(t))",
        "const t = {}; assign(t, { y: 1 }); Object.defineProperty(Object.prototype, 'y2', { set(v) { console.log('set y2', v) }, configurable: true }); const u = {}; assign(u, { y: 1, y2: 2 }); console.log(JSON.stringify(u))",
        // What's frozen, getters on the source, symbols and numbers.
        "'use strict'; const t = Object.freeze({ a: 1 }); try { assign(t, { a: 2 }) } catch (e) { console.log(e instanceof TypeError) } console.log(t.a)",
        "const t = {}; assign(t, { get g() { return 'got' }, h: 1 }); console.log(JSON.stringify(t), Object.getOwnPropertyDescriptor(t, 'g').value)",
        "const s = Symbol('s'); const t = {}; assign(t, { [s]: 1, 2: 'two', x: 3, 1: 'one' }); console.log(t[s], JSON.stringify(t))",
        // A non-writable one in the target.
        "const t = {}; Object.defineProperty(t, 'w', { value: 1, writable: false, enumerable: true }); try { assign(t, { w: 2 }) } catch (e) { console.log('threw') } console.log(t.w)",
        // Over a collection, which can free shapes and give their numbers to others.
        "const keep = []; for (let i = 0; i < 50; i++) { const t = { ['k' + i]: i }; assign(t, { q: i }); keep.push(t); if (i % 10 === 0) gc(); } console.log(JSON.stringify(keep[49]), Object.keys(keep[3]).join())",
        // Into one that's had something deleted, which has a shape of its own then.
        "const t = {}; for (let i = 0; i < 40; i++) t['p' + i] = i; delete t.p1; assign(t, { p2: 'x', q: 1 }, { q: 2, p1: 3 }); console.log(JSON.stringify(t))",
        "Object.defineProperty(Object.prototype, 'r', { set(v) { console.log('set r', v) }, configurable: true }); const t = {}; for (let i = 0; i < 40; i++) t['p' + i] = i; delete t.p1; assign(t, { p2: 'x', r: 1 }); console.log(JSON.stringify(t))",
        // An array, a function, and many properties, which make a dictionary.
        "const a = [1]; assign(a, { x: 1 }); const f = function () {}; assign(f, { y: 2 }); console.log(a.x, f.y, f.name); const big = {}; for (let i = 0; i < 200; i++) big['p' + i] = i; const t = {}; assign(t, big); console.log(Object.keys(t).length, t.p199)",
    ];
    for case in cases {
        let expected = run(&case.replace("assign(", "Object.assign("));
        assert_eq!(run(case), expected, "{case}");
    }
}

/// How long each of these takes, in nanoseconds a time, against JavaScript doing the same, for DESIGN.md.
#[test]
#[ignore]
fn bulk_speed() {
    let source = r#"
        const N = 200000;
        function time(name, f) {
            let best = Infinity;
            for (let round = 0; round < 7; round++) {
                const start = now();
                f();
                best = Math.min(best, now() - start);
            }
            console.log(name.padEnd(44) + (best * 1000 / N).toFixed(1) + ' ns');
        }
        let sink = 0;
        time('a host call that does nothing', () => { for (let i = 0; i < N; i++) nothing(i); });
        time('12 numbers: an object literal in JS', () => { for (let i = 0; i < N; i++) sink = { a: i, b: i + 1, c: i + 2, d: i + 3, e: i + 4, f: i + 5, g: i + 6, h: i + 7, i: i + 8, j: i + 9, k: i + 10, l: i + 11 }; });
        time('12 numbers: new_object_with', () => { for (let i = 0; i < N; i++) sink = make12(i); });
        time('12 numbers: new_object + set_property', () => { for (let i = 0; i < N; i++) sink = make12Slowly(i); });
        const o = { z: 0, a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, x: 7, y: 8, w: 9 };
        time('6 reads: in JS', () => { let s = 0; for (let i = 0; i < N; i++) s += o.a + o.b + o.c + o.d + o.e + o.f; sink = s; });
        time('6 reads: read_keys', () => { let s = 0; for (let i = 0; i < N; i++) s += sum6(o); sink = s; });
        time('6 reads: get_property', () => { let s = 0; for (let i = 0; i < N; i++) s += sum6Slowly(o); sink = s; });
        // As a look has rules put on it: a few props each, some said again.
        const rules = [];
        const names = ['display', 'position', 'color', 'paddingLeft', 'paddingRight', 'marginTop', 'width', 'height', 'gap', 'borderColor', 'borderWidth', 'fontSize', 'lineHeight', 'opacity', 'cursor', 'background'];
        for (let r = 0; r < 20; r++) {
            const props = {};
            for (let k = 0; k < 4; k++) props[names[(r * 3 + k * 5) % names.length]] = r * 10 + k;
            rules.push(props);
        }
        time('20 rules onto a look: Object.assign in JS', () => { for (let i = 0; i < N / 20; i++) { const look = {}; for (const props of rules) Object.assign(look, props); sink = look; } });
        time('20 rules onto a look: assign', () => { for (let i = 0; i < N / 20; i++) sink = assignAll({}, rules); });
        time('20 rules onto a look: properties + set_property', () => { for (let i = 0; i < N / 20; i++) sink = assignAllSlowly({}, rules); });
        // As a look's few marks are read: thousands of shapes, each of many props, the names mostly not there.
        const many = [];
        for (let s = 0; s < 4000; s++) {
            const o = {};
            for (let k = 0; k < 40; k++) o['p' + ((s * 7 + k * 13) % 97)] = k;
            if (s % 3 === 0) o.c = 1;
            many.push(o);
        }
        time('6 reads, 4,000 shapes: in JS', () => { let s = 0; for (let i = 0; i < N; i++) { const o = many[i % 4000]; s += (o.a === undefined) + (o.b === undefined) + (o.c === undefined) + (o.d === undefined) + (o.e === undefined) + (o.f === undefined); } sink = s; });
        time('6 reads, 4,000 shapes: read_keys', () => { let s = 0; for (let i = 0; i < N; i++) s += sum6(many[i % 4000]); sink = s; });
        time('6 reads, 4,000 shapes: read_own_keys', () => { let s = 0; for (let i = 0; i < N; i++) s += sum6Own(many[i % 4000]); sink = s; });
        time('6 reads: read_own_keys', () => { let s = 0; for (let i = 0; i < N; i++) s += sum6Own(o); sink = s; });
        time('6 reads, 4,000 shapes: get_property', () => { let s = 0; for (let i = 0; i < N; i++) s += sum6Slowly(many[i % 4000]); sink = s; });
    "#;
    println!("{}", run(source));
}
