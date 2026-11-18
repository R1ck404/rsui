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
    globalThis.tooDeep = [{ v: deep(20) }, { v: deep(20) }];
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
    function actions(n) { const act = { next: () => act.go(n), go: (i) => i + n }; return act; }
    globalThis.cycle = [{ onNext: actions(1).next }, { onNext: actions(1).next }];
    globalThis.cycleOther = [{ onNext: actions(1).next }, { onNext: actions(2).next }];
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
fn what_refers_to_itself_is_the_same_when_everything_else_in_it_is() {
    // An object of handlers that each see the object, as the file explorer's `act` is, rather than running out of depth.
    let mut engine = engine_with(PAIRS, "pairs.js");
    assert!(equivalent(&mut engine, "cycle", true));
    assert!(!equivalent(&mut engine, "cycleOther", true));
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

/// Handlers in an app's own file, each pair made by two renders that saw different state.
const HANDLERS: &str = r#"
    globalThis.opaque = Symbol('opaque');
    function bar(state) {
        const run = (what) => what + ' ' + state;
        return { label: 'bar', onRefresh: () => run('refresh'), refresh: () => run('refresh') };
    }
    globalThis.handler = [{ onRefresh: bar(1).onRefresh, label: 'a' }, { onRefresh: bar(2).onRefresh, label: 'a' }];
    globalThis.notNamedAsOne = [{ refresh: bar(1).refresh }, { refresh: bar(2).refresh }];
    globalThis.labelChanged = [{ onRefresh: bar(1).onRefresh, label: 'a' }, { onRefresh: bar(2).onRefresh, label: 'b' }];
    globalThis.otherCode = [{ onClick: () => 1 }, { onClick: () => 2 }];
    function waiting(state) { return async () => state; }
    globalThis.async = [{ onClick: waiting(1) }, { onClick: waiting(2) }];
    const button = (state) => ({ type: 'button', key: undefined, props: { onClick: bar(state).onRefresh }, kids: ['go'], memo: true });
    globalThis.inChildren = [{ children: [button(1), 'text'] }, { children: [button(2), 'text'] }];
"#;

fn handlers() -> Engine {
    let file = std::env::temp_dir().join("app").join("handlers.js");
    engine_with(HANDLERS, file.to_str().unwrap())
}

fn handing_on(engine: &mut Engine, name: &str) -> bool {
    let (a, b) = pair(engine, name);
    let vm = engine.vm();
    let opaque = vm.get_global("opaque").unwrap();
    vm.props_equivalent_handing_on(a, b, true, opaque)
}

/// What the old props' handler at `path` (a property, or `children.0.props.onClick`) gives when it's called.
fn old_says(engine: &mut Engine, name: &str, path: &str) -> String {
    let (mut at, _) = pair(engine, name);
    let vm = engine.vm();
    for part in path.split('.') {
        at = match part.parse::<usize>() {
            Ok(index) => vm.array_elements(at).unwrap()[index],
            Err(_) => {
                let key = vm.atom(part);
                vm.get_property(at, key).unwrap()
            }
        };
    }
    let said = vm.call_function(at, Value::UNDEFINED, &[]).unwrap();
    vm.plain_text(said).unwrap()
}

#[test]
fn a_handler_made_from_the_same_code_is_handed_what_the_new_one_sees_and_nothing_else_is() {
    let mut engine = handlers();
    assert!(!equivalent(&mut engine, "handler", true));
    assert_eq!(old_says(&mut engine, "handler", "onRefresh"), "refresh 1");
    assert!(handing_on(&mut engine, "handler"));
    // The old function is still the one the child has, doing what the new one does.
    assert_eq!(old_says(&mut engine, "handler", "onRefresh"), "refresh 2");
    // Inside an element that's given as children, as a button's `onClick` is.
    assert!(handing_on(&mut engine, "inChildren"));
    assert_eq!(old_says(&mut engine, "inChildren", "children.0.props.onClick"), "refresh 2");

    // A function that isn't named as a handler, as `renderRow`, is handed on too, as nothing rendering called it without its
    // reads being noted: what did, with them noted, is looked at again (see `reads_tests.rs`).
    assert!(handing_on(&mut engine, "notNamedAsOne"));
    // Other code does something else, and an async one can be waiting part of the way through.
    assert!(!handing_on(&mut engine, "otherCode"));
    assert!(!handing_on(&mut engine, "async"));
    // When something else isn't the same, the child renders with the new one, and the old one is left as it was.
    assert!(!handing_on(&mut engine, "labelChanged"));
    assert_eq!(old_says(&mut engine, "labelChanged", "onRefresh"), "refresh 1");
}
