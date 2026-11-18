//! What `Vm::props_equivalent` says is the same, and where JSX calls `__jsxMemo`.

use crate::Engine;
use crate::value::Value;

fn engine_with(source: &str, file: &str) -> Engine {
    let mut engine = Engine::new();
    engine.eval(source, file).unwrap();
    engine
}

fn pair(engine: &mut Engine, name: &str) -> (Value, Value) {
    let vm = engine.vm();
    let list = vm.get_global(name).unwrap();
    let both = vm.array_elements(list).unwrap();
    (both[0], both[1])
}

fn equivalent(engine: &mut Engine, name: &str, refs_change: bool) -> bool {
    let (a, b) = pair(engine, name);
    let vm = engine.vm();
    let opaque = vm.get_global("opaque").unwrap();
    vm.props_equivalent(a, b, refs_change, opaque)
}

const PAIRS: &str = r#"
    globalThis.opaque = Symbol('opaque');
    const select = (i) => i;
    function props(i, select) { return { onPick: () => select(i), style: { weight: 'bold' }, list: [1, 'a'] }; }
    globalThis.same = [props(1, select), props(1, select)];
    globalThis.otherValue = [props(1, select), props(2, select)];
    globalThis.otherFunction = [props(1, select), props(1, (i) => i)];
    function make(n) { return (i) => i + n; }
    globalThis.made = [{ f: make(1) }, { f: make(1) }];
    globalThis.madeOther = [{ f: make(1) }, { f: make(2) }];
    function changing() { let seen = 1; const f = () => seen; return [f, (v) => { seen = v; }]; }
    const [first, setFirst] = changing();
    const [second] = changing();
    globalThis.cells = [{ f: first }, { f: second }];
    const [third, setThird] = changing();
    const [fourth] = changing();
    setThird(5);
    globalThis.cellChanged = [{ f: third }, { f: fourth }];
    globalThis.bound = [{ f: select.bind(null, 1) }, { f: select.bind(null, 1) }];
    globalThis.boundOther = [{ f: select.bind(null, 1) }, { f: select.bind(null, 2) }];
    class Point { constructor(x) { this.x = x; } }
    globalThis.instances = [{ p: new Point(1) }, { p: new Point(1) }];
    const point = new Point(1);
    globalThis.sameInstance = [{ p: point }, { p: point }];
    const ref = { current: 1 };
    globalThis.refs = [{ r: ref }, { r: ref }];
    function reading(held) { return () => held.current; }
    globalThis.seeingRefs = [{ f: reading(ref) }, { f: reading(ref) }];
    const methods = { isSelected: () => false };
    globalThis.methods = [{ row: methods }, { row: methods }];
    const data = { name: 'a', size: 1 };
    globalThis.data = [{ entry: data }, { entry: data }];
    globalThis.getters = [{ g: { get x() { return 1; } } }, { g: { get x() { return 1; } } }];
    const deep = (n) => (n === 0 ? 1 : { d: deep(n - 1) });
    globalThis.tooDeep = [{ v: deep(10) }, { v: deep(10) }];
    globalThis.shallowEnough = [{ v: deep(3) }, { v: deep(3) }];
    const element = (i) => ({ type: 'text', key: undefined, props: { onClick: () => select(i) }, kids: ['a'] });
    globalThis.elements = [{ children: element(1) }, { children: element(1) }];
    globalThis.elementsOther = [{ children: element(1) }, { children: element(2) }];
    const row = { selected: false };
    const foreign = () => ({ type: select, key: undefined, props: { row }, kids: null, memo: opaque });
    globalThis.foreign = [{ children: foreign() }, { children: foreign() }];
    const kept = foreign();
    globalThis.foreignKept = [{ children: kept }, { children: kept }];
    globalThis.long = [{ list: Array.from({ length: 100 }, () => 1) }, { list: Array.from({ length: 100 }, () => 1) }];
"#;

#[test]
fn handlers_made_again_from_the_same_code_seeing_the_same_values_are_the_same() {
    let mut engine = engine_with(PAIRS, "pairs.js");
    assert!(equivalent(&mut engine, "same", true));
    assert!(!equivalent(&mut engine, "otherValue", true));
    assert!(!equivalent(&mut engine, "otherFunction", true));
    assert!(equivalent(&mut engine, "made", true));
    assert!(!equivalent(&mut engine, "madeOther", true));
    assert!(equivalent(&mut engine, "bound", true));
    assert!(!equivalent(&mut engine, "boundOther", true));
}

#[test]
fn a_variable_in_a_cell_is_compared_by_what_it_holds() {
    let mut engine = engine_with(PAIRS, "pairs.js");
    assert!(equivalent(&mut engine, "cells", true));
    assert!(!equivalent(&mut engine, "cellChanged", true));
}

#[test]
fn objects_made_again_are_the_same_only_when_plainly_made() {
    let mut engine = engine_with(PAIRS, "pairs.js");
    assert!(equivalent(&mut engine, "elements", true));
    assert!(!equivalent(&mut engine, "elementsOther", true));
    assert!(equivalent(&mut engine, "shallowEnough", true));
    assert!(!equivalent(&mut engine, "tooDeep", true));
    assert!(!equivalent(&mut engine, "long", true));
    // An instance of a class can keep more than its properties say, and a getter can say anything.
    assert!(!equivalent(&mut engine, "instances", true));
    assert!(equivalent(&mut engine, "sameInstance", true));
    assert!(!equivalent(&mut engine, "getters", true));
    // An element that says it wasn't made by the app's own JSX is only the same as itself, and that only when it's not
    // cautious, as an element holds a function, its component.
    assert!(!equivalent(&mut engine, "foreign", true));
    assert!(!equivalent(&mut engine, "foreign", false));
    assert!(equivalent(&mut engine, "foreignKept", false));
}

#[test]
fn a_ref_or_an_object_with_methods_is_only_the_same_as_itself_when_not_cautious() {
    let mut engine = engine_with(PAIRS, "pairs.js");
    assert!(!equivalent(&mut engine, "refs", true));
    assert!(equivalent(&mut engine, "refs", false));
    assert!(!equivalent(&mut engine, "methods", true));
    assert!(equivalent(&mut engine, "methods", false));
    // The same data is the same, and so is a ref that handlers see, as they only read it when they're called.
    assert!(equivalent(&mut engine, "data", true));
    assert!(equivalent(&mut engine, "seeingRefs", true));
}

const JSX: &str = r#"
    globalThis.__jsxMemo = () => 'memo';
    globalThis.__jsx = () => 'plain';
    globalThis.made = <a />;
    function Quiet() { 'use no memo'; return [<a />, (() => <b />)()]; }
    globalThis.quiet = Quiet();
    globalThis.Quiet = Quiet;
    globalThis.Loud = function Loud() { return <a />; };
"#;

fn global_text(engine: &mut Engine, name: &str) -> String {
    let vm = engine.vm();
    let value = vm.get_global(name).unwrap();
    crate::builtins::inspect_value(vm, value)
}

#[test]
fn jsx_in_an_app_file_calls_jsx_memo_unless_it_says_use_no_memo() {
    let file = std::env::temp_dir().join("app").join("a.jsx");
    let mut engine = engine_with(JSX, file.to_str().unwrap());
    assert_eq!(global_text(&mut engine, "made"), "'memo'");
    assert_eq!(global_text(&mut engine, "quiet"), "[ 'plain', 'plain' ]");
    let vm = engine.vm();
    let (quiet, loud) = (vm.get_global("Quiet").unwrap(), vm.get_global("Loud").unwrap());
    assert!(vm.says_no_memo(quiet));
    assert!(!vm.says_no_memo(loud));

    let mut engine = engine_with(&format!("'use no memo';\n{JSX}"), file.to_str().unwrap());
    assert_eq!(global_text(&mut engine, "made"), "'plain'");
    let vm = engine.vm();
    let loud = vm.get_global("Loud").unwrap();
    assert!(vm.says_no_memo(loud));
}

#[test]
fn jsx_outside_an_app_file_calls_plain_jsx() {
    let package = std::env::temp_dir().join("app").join("node_modules").join("lib").join("a.jsx");
    let mut engine = engine_with(JSX, package.to_str().unwrap());
    assert_eq!(global_text(&mut engine, "made"), "'plain'");
    let mut engine = engine_with(JSX, "a.jsx");
    assert_eq!(global_text(&mut engine, "made"), "'plain'");
}
