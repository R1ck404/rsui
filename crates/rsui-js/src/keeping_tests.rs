//! What a component works out as it renders, kept for the next render while what it was worked out from is the same: see
//! `keeping.rs`.

use crate::value::Value;
use crate::{Engine, KeptCounts};

/// An engine that's run `source` as a module of an app's own, in a folder of its own.
fn engine_with(name: &str, source: &str) -> Engine {
    let folder = std::env::temp_dir().join("rsui-keeping-tests").join(name);
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.js");
    std::fs::write(&file, source).unwrap();
    let mut engine = Engine::new();
    engine.run_module(&file).unwrap();
    engine
}

fn global(engine: &mut Engine, name: &str) -> Value {
    engine.vm().get_global(name).unwrap()
}

/// Renders `component` for `owner`, given the props the global `props` has, as rsui-tsx does, and gives what it gave.
fn render(engine: &mut Engine, owner: u32, component: &str, props: &str) -> Value {
    let function = global(engine, component);
    let props = global(engine, props);
    let vm = engine.vm();
    let id = vm.start_reads(owner, function, props, true, false);
    let given = vm.call_function(function, Value::UNDEFINED, &[props]);
    vm.end_reads();
    vm.drop_reads(id);
    given.unwrap()
}

fn run(engine: &mut Engine, code: &str) -> Value {
    engine.eval(code, "test.js").unwrap()
}

fn text(engine: &mut Engine, value: Value) -> String {
    engine.display(value)
}

/// What `code` gives with `value` as the global `it`.
fn of(engine: &mut Engine, value: Value, code: &str) -> String {
    engine.vm().set_global("it", value);
    let said = run(engine, code);
    text(engine, said)
}

fn counts(engine: &mut Engine) -> KeptCounts {
    engine.vm().kept_counts()
}

/// A sort of 20,000 numbers, which is worth keeping, as it makes far more than `WORTH_MADE`.
const SORTING: &str = r#"
    export function sorted(list, descending) {
        const copy = list.slice();
        copy.sort((a, b) => (descending ? b - a : a - b));
        return copy;
    }
    function List(props) {
        const order = sorted(props.list, props.descending);
        return order;
    }
    const list = [];
    for (let i = 0; i < 20000; i++) {
        list.push((i * 7919) % 20000);
    }
    Object.assign(globalThis, { List, list });
    globalThis.up = { list, descending: false };
    globalThis.down = { list, descending: true };
    globalThis.otherList = { list: list.slice(0, 15000), descending: false };
    globalThis.again = { list, descending: false };
"#;

#[test]
fn the_same_inputs_give_what_was_worked_out_last_time() {
    let mut engine = engine_with("same", SORTING);
    // The first time compiles what it calls, the second says it's worth keeping, the third keeps it, and the fourth gives it
    // again.
    render(&mut engine, 1, "List", "up");
    render(&mut engine, 1, "List", "up");
    let second = render(&mut engine, 1, "List", "up");
    let third = render(&mut engine, 1, "List", "up");
    assert_eq!(second, third, "the same array, not worked out again");
    // New props holding the same are the same.
    let fourth = render(&mut engine, 1, "List", "again");
    assert_eq!(third, fourth);
    let counts = counts(&mut engine);
    assert_eq!((counts.kept, counts.given_again, counts.not_kept), (1, 2, 0));
}

#[test]
fn what_changed_is_worked_out_again() {
    let mut engine = engine_with("changed", SORTING);
    render(&mut engine, 1, "List", "up");
    render(&mut engine, 1, "List", "up");
    let down = render(&mut engine, 1, "List", "down");
    assert_eq!(of(&mut engine, down, "it[0]"), "19999");
    let other = render(&mut engine, 1, "List", "otherList");
    assert_eq!(of(&mut engine, other, "it.length"), "15000");
    // And the first again is worked out again too, as only the last is kept.
    let up = render(&mut engine, 1, "List", "up");
    assert_eq!(of(&mut engine, up, "it[0]"), "0");
    assert_eq!(counts(&mut engine).given_again, 0);
}

#[test]
fn an_argument_changed_in_place_is_worked_out_again() {
    let mut engine = engine_with("in-place", SORTING);
    render(&mut engine, 1, "List", "up");
    render(&mut engine, 1, "List", "up");
    let kept = render(&mut engine, 1, "List", "up");
    assert_eq!(render(&mut engine, 1, "List", "up"), kept);
    // The same array, with one more in it.
    let given_before = counts(&mut engine).given_again;
    run(&mut engine, "list.push(-1)");
    let after = render(&mut engine, 1, "List", "up");
    assert_eq!(counts(&mut engine).given_again, given_before, "worked out again");
    assert_eq!(of(&mut engine, after, "it[0] + ' ' + it.length"), "-1 20001");
}

#[test]
fn a_result_the_caller_changed_is_worked_out_again() {
    let mut engine = engine_with("result-changed", SORTING);
    render(&mut engine, 1, "List", "up");
    render(&mut engine, 1, "List", "up");
    let kept = render(&mut engine, 1, "List", "up");
    // What was given back, changed by what had it.
    let given = render(&mut engine, 1, "List", "up");
    assert_eq!(given, kept);
    let vm = engine.vm();
    vm.set_global("given", given);
    let given_before = counts(&mut engine).given_again;
    run(&mut engine, "given.reverse()");
    let after = render(&mut engine, 1, "List", "up");
    assert_eq!(counts(&mut engine).given_again, given_before, "worked out again");
    assert_eq!(of(&mut engine, after, "it[0]"), "0");
}

#[test]
fn what_a_function_reads_at_the_top_of_its_module_counts() {
    let mut engine = engine_with(
        "module-state",
        r#"
        let scale = 1;
        const options = { offset: 0 };
        export function scaled(list) {
            return list.map((value) => value * scale + options.offset);
        }
        function Scaled(props) {
            return scaled(props.list);
        }
        Object.assign(globalThis, {
            Scaled,
            props: { list: Array.from({ length: 20000 }, (_, at) => at) },
            setScale: (to) => { scale = to; },
            setOffset: (to) => { options.offset = to; },
        });
        "#,
    );
    render(&mut engine, 1, "Scaled", "props");
    render(&mut engine, 1, "Scaled", "props");
    let kept = render(&mut engine, 1, "Scaled", "props");
    assert_eq!(render(&mut engine, 1, "Scaled", "props"), kept);
    let given_before = counts(&mut engine).given_again;
    run(&mut engine, "setScale(2)");
    let scaled = render(&mut engine, 1, "Scaled", "props");
    assert_eq!(counts(&mut engine).given_again, given_before, "worked out again");
    engine.vm().set_global("seen", scaled);
    let last = run(&mut engine, "seen[3]");
    assert_eq!(text(&mut engine, last), "6");
    let kept = render(&mut engine, 1, "Scaled", "props");
    assert_eq!(render(&mut engine, 1, "Scaled", "props"), kept);
    let given_before = counts(&mut engine).given_again;
    run(&mut engine, "setOffset(10)");
    let offset = render(&mut engine, 1, "Scaled", "props");
    assert_eq!(counts(&mut engine).given_again, given_before, "worked out again");
    engine.vm().set_global("seen", offset);
    let last = run(&mut engine, "seen[3]");
    assert_eq!(text(&mut engine, last), "16");
}

#[test]
fn what_isnt_pure_is_never_kept() {
    let mut engine = engine_with(
        "impure",
        r#"
        let calls = 0;
        const log = [];
        export function counting(list) {
            calls++;
            return list.map((value) => value + 1);
        }
        export function logging(list) {
            log.push(list.length);
            return list.map((value) => value + 1);
        }
        export function changingWhatItsGiven(list) {
            const out = list.map((value) => value + 1);
            list.length = list.length;
            list.touched = (list.touched ?? 0) + 1;
            return out;
        }
        export function random(list) {
            return list.map((value) => value + Math.random());
        }
        export function now(list) {
            return list.map((value) => value + Date.now());
        }
        export function throwing(list) {
            list.map((value) => value + 1);
            throw new Error('no');
        }
        function Counting(props) { return counting(props.list); }
        function Logging(props) { return logging(props.list); }
        function Changing(props) { return changingWhatItsGiven(props.list); }
        function Random(props) { return random(props.list); }
        function Now(props) { return now(props.list); }
        function Throwing(props) { try { return throwing(props.list); } catch { return null; } }
        Object.assign(globalThis, {
            Counting, Logging, Changing, Random, Now, Throwing,
            props: { list: Array.from({ length: 20000 }, (_, at) => at) },
            calls: () => calls,
            logged: () => log.length,
        });
        "#,
    );
    for component in ["Counting", "Logging", "Changing", "Random", "Now", "Throwing"] {
        for _ in 0..5 {
            render(&mut engine, 1, component, "props");
        }
    }
    let calls = run(&mut engine, "calls()");
    assert_eq!(text(&mut engine, calls), "5");
    let logged = run(&mut engine, "logged()");
    assert_eq!(text(&mut engine, logged), "5");
    let touched = run(&mut engine, "props.list.touched");
    assert_eq!(text(&mut engine, touched), "5");
    let counts = counts(&mut engine);
    assert_eq!(counts.given_again, 0);
    assert_eq!(counts.kept, 0);
    assert!(counts.not_kept > 0);
}

#[test]
fn each_place_and_each_run_of_it_keeps_its_own() {
    let mut engine = engine_with(
        "places",
        r#"
        export function doubled(list) { return list.map((value) => value * 2); }
        function Two(props) {
            const a = doubled(props.a);
            const b = doubled(props.b);
            const each = [];
            for (const list of props.lists) {
                each.push(doubled(list));
            }
            return [a, b, ...each];
        }
        const big = (from) => Array.from({ length: 20000 }, (_, at) => at + from);
        const [a, b, c, d] = [big(0), big(1), big(2), big(3)];
        Object.assign(globalThis, { Two });
        globalThis.props = { a, b, lists: [c, d] };
        globalThis.swapped = { a: b, b: a, lists: [d, c] };
        "#,
    );
    render(&mut engine, 1, "Two", "props");
    render(&mut engine, 1, "Two", "props");
    render(&mut engine, 1, "Two", "props");
    let before = counts(&mut engine).given_again;
    let given = render(&mut engine, 1, "Two", "props");
    // Four places, counting each run of the loop's as one, each given again.
    assert_eq!(counts(&mut engine).given_again - before, 4);
    engine.vm().set_global("given", given);
    let firsts = run(&mut engine, "given.map((list) => list[0]).join()");
    assert_eq!(text(&mut engine, firsts), "0,2,4,6");
    // Swapped, each works out its own again.
    let given = render(&mut engine, 1, "Two", "swapped");
    engine.vm().set_global("given", given);
    let firsts = run(&mut engine, "given.map((list) => list[0]).join()");
    assert_eq!(text(&mut engine, firsts), "2,0,6,4");
}

#[test]
fn each_component_keeps_its_own_and_hooks_are_always_called() {
    let mut engine = engine_with(
        "hooks",
        r#"
        let hooked = 0;
        export function doubled(list) { return list.map((value) => value * 2); }
        function useCount() { hooked++; return hooked; }
        function useDoubled(list) {
            const count = useCount();
            return [count, doubled(list)];
        }
        function Shown(props) {
            const [count, list] = useDoubled(props.list);
            return list;
        }
        const big = (from) => Array.from({ length: 20000 }, (_, at) => at + from);
        Object.assign(globalThis, { Shown, hooked: () => hooked });
        globalThis.first = { list: big(0) };
        globalThis.second = { list: big(5) };
        "#,
    );
    for _ in 0..3 {
        render(&mut engine, 1, "Shown", "first");
        render(&mut engine, 2, "Shown", "second");
    }
    let one = render(&mut engine, 1, "Shown", "first");
    let two = render(&mut engine, 2, "Shown", "second");
    assert_eq!(of(&mut engine, one, "it[0]"), "0");
    engine.vm().set_global("two", two);
    let first = run(&mut engine, "two[0]");
    assert_eq!(text(&mut engine, first), "10");
    // The hook ran each time, though what was worked out inside it was given again.
    let hooked = run(&mut engine, "hooked()");
    assert_eq!(text(&mut engine, hooked), "8");
    assert!(counts(&mut engine).given_again >= 4);
}

#[test]
fn a_collection_doesnt_hide_a_change() {
    let mut engine = engine_with("collected", SORTING);
    render(&mut engine, 1, "List", "up");
    render(&mut engine, 1, "List", "up");
    let kept = render(&mut engine, 1, "List", "up");
    // Small collections while what's watched is young, and a full one.
    for _ in 0..4 {
        run(&mut engine, "globalThis.junk = Array.from({ length: 50000 }, (_, at) => ({ at }))");
        assert_eq!(render(&mut engine, 1, "List", "up"), kept);
    }
    engine.collect_garbage();
    assert_eq!(render(&mut engine, 1, "List", "up"), kept);
    let given_before = counts(&mut engine).given_again;
    run(&mut engine, "list[0] = -5");
    let after = render(&mut engine, 1, "List", "up");
    assert_eq!(counts(&mut engine).given_again, given_before, "worked out again");
    engine.vm().set_global("after", after);
    let first = run(&mut engine, "after[0]");
    assert_eq!(text(&mut engine, first), "-5");
}

#[test]
fn a_quick_call_isnt_kept() {
    let mut engine = engine_with(
        "quick",
        r#"
        export function add(a, b) { return { sum: a + b }; }
        function Sum(props) { return add(props.a, props.b); }
        Object.assign(globalThis, { Sum });
        globalThis.props = { a: 1, b: 2 };
        "#,
    );
    for _ in 0..3 {
        render(&mut engine, 1, "Sum", "props");
    }
    assert_eq!(counts(&mut engine), KeptCounts::default());
}

#[test]
fn a_date_of_a_given_time_is_pure_and_the_time_now_isnt() {
    let mut engine = engine_with(
        "dates",
        r#"
        export function years(list) { return list.map((at) => new Date(at * 86400000).getUTCFullYear()); }
        export function ages(list) { return list.map((at) => new Date().getTime() - at); }
        function Years(props) { return years(props.list); }
        function Ages(props) { return ages(props.list); }
        Object.assign(globalThis, { Years, Ages, props: { list: Array.from({ length: 20000 }, (_, at) => at) } });
        "#,
    );
    for _ in 0..4 {
        render(&mut engine, 1, "Years", "props");
    }
    assert_eq!(counts(&mut engine).given_again, 1);
    for _ in 0..4 {
        render(&mut engine, 1, "Ages", "props");
    }
    let counts = counts(&mut engine);
    assert_eq!((counts.given_again, counts.not_kept), (1, 2));
}
