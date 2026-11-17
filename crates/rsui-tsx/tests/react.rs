//! Code written for React, on rsui: what's in `react`, and what's in `localStorage`.

mod hands;

use std::path::PathBuf;
use std::process::Command;

use hands::Hands;

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn write(name: &str, source: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-react-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    file
}

fn open(name: &str, source: &str) -> Hands {
    Hands::open(&write(name, source), (400, 300))
}

#[test]
fn hooks_and_the_default_export_are_there() {
    let mut hands = open(
        "hooks",
        r#"
        import React, { useState, useEffect, useMemo, useCallback, useReducer, useRef, useLayoutEffect, useContext, createContext } from 'react';
        const Theme = createContext('light');
        function Inner() { const theme = useContext(Theme); return <text>{'theme ' + theme}</text>; }
        export default function App() {
            const [count, setCount] = React.useState(0);
            const doubled = useMemo(() => count * 2, [count]);
            const add = useCallback(() => setCount((c) => c + 1), []);
            const [text, dispatch] = useReducer((s, a) => s + a, 'r');
            const ran = useRef(0);
            useEffect(() => { ran.current += 1; }, [count]);
            return (
                <Theme.Provider value="dark">
                    <window>
                        <button key="add" onClick={() => { add(); dispatch('x'); }}>Add</button>
                        <text>{'doubled ' + doubled + ' ' + text + ' version ' + typeof React.version}</text>
                        <Inner />
                    </window>
                </Theme.Provider>
            );
        }
        "#,
    );
    hands.click("add");
    hands.settle();
    assert!(
        hands.says("doubled 2 rx version string"),
        "{}",
        hands.app.outline()
    );
    assert!(hands.says("theme dark"), "{}", hands.app.outline());
}

#[test]
fn memo_and_forward_ref_and_imperative_handles_work() {
    let mut hands = open(
        "memo",
        r#"
        import React, { memo, forwardRef, useRef, useImperativeHandle, useState, createRef } from 'react';
        const Plain = memo(({ label }) => <text>{'memo ' + label}</text>);
        const Fancy = forwardRef(function Fancy({ label }, ref) {
            const [times, setTimes] = useState(0);
            useImperativeHandle(ref, () => ({ poke() { setTimes((t) => t + 1); } }), []);
            return <text>{label + ' poked ' + times}</text>;
        });
        const made = createRef();
        export default function App() {
            const ref = useRef(null);
            return (
                <window>
                    <Plain label="one" />
                    <Fancy ref={ref} label="fancy" />
                    <button key="poke" onClick={() => ref.current.poke()}>Poke</button>
                    <text>{'made ' + made.current}</text>
                </window>
            );
        }
        "#,
    );
    assert!(hands.says("memo one"), "{}", hands.app.outline());
    hands.settle();
    hands.click("poke");
    hands.settle();
    assert!(hands.says("fancy poked 1"), "{}", hands.app.outline());
    assert!(hands.says("made null"), "{}", hands.app.outline());
}

#[test]
fn use_sync_external_store_follows_what_is_outside() {
    let mut hands = open(
        "store",
        r#"
        import { useSyncExternalStore, useId } from 'react';
        let value = 0;
        const listeners = new Set();
        const subscribe = (f) => { listeners.add(f); return () => listeners.delete(f); };
        const change = () => { value += 1; listeners.forEach((f) => f()); };
        export default function App() {
            const now = useSyncExternalStore(subscribe, () => value);
            const a = useId();
            const b = useId();
            return (
                <window>
                    <text>{'store ' + now + ' ' + (a !== b)}</text>
                    <button key="go" onClick={change}>Go</button>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    hands.click("go");
    hands.settle();
    assert!(hands.says("store 1 true"), "{}", hands.app.outline());
}

#[test]
fn class_components_work_with_their_lifecycle() {
    let mut hands = open(
        "class",
        r#"
        import React, { Component, PureComponent } from 'react';
        const log = [];
        class Counter extends Component {
            static defaultProps = { step: 2 };
            state = { count: 0 };
            componentDidMount() { log.push('mounted'); }
            componentDidUpdate(props, state) { log.push('updated from ' + state.count); }
            componentWillUnmount() { log.push('gone'); globalThis.afterGone = log.join(); }
            add = () => this.setState((s, p) => ({ count: s.count + p.step }), () => log.push('after'));
            render() {
                return (
                    <group>
                        <text>{'count ' + this.state.count}</text>
                        <button key="add" onClick={this.add}>Add</button>
                    </group>
                );
            }
        }
        class Named extends PureComponent {
            static getDerivedStateFromProps(props) { return { shout: props.name.toUpperCase() }; }
            state = {};
            render() { return <text>{'hello ' + this.state.shout}</text>; }
        }
        export default class App extends Component {
            state = { on: true };
            render() {
                return (
                    <window>
                        {this.state.on ? <Counter /> : <text>{'log ' + globalThis.afterGone}</text>}
                        <Named name="ann" />
                        <button key="off" onClick={() => this.setState({ on: false })}>Off</button>
                        <button key="again" onClick={() => this.forceUpdate()}>Again</button>
                    </window>
                );
            }
        }
        "#,
    );
    assert!(hands.says("hello ANN"), "{}", hands.app.outline());
    hands.settle();
    hands.click("add");
    hands.settle();
    assert!(hands.says("count 2"), "{}", hands.app.outline());
    hands.click("off");
    hands.settle();
    // It's gone once that render is done with, as in React, so it takes another one to show that.
    hands.click("again");
    hands.settle();
    let outline = hands.app.outline();
    assert!(
        outline.contains("mounted,updated from 0,after,gone"),
        "{outline}"
    );
}

#[test]
fn create_element_clone_element_and_children_work() {
    let hands = open(
        "elements",
        r#"
        import React, { createElement, cloneElement, Children, isValidElement, Fragment } from 'react';
        function Wrap({ children, tag }) {
            const kids = Children.toArray(children);
            return (
                <group>
                    {Children.map(children, (child, at) => cloneElement(child, { name: tag + at }))}
                    <text>{'there are ' + Children.count(children) + ' ' + kids.length + ' ' + isValidElement(kids[0]) + ' ' + isValidElement('no')}</text>
                </group>
            );
        }
        function Sized({ size }) { return createElement('text', null, 'sized ' + size); }
        Sized.defaultProps = { size: 7 };
        export default function App() {
            return createElement(
                'window',
                { key: 'root' },
                createElement(Wrap, { tag: 'n' }, createElement('group', { key: 'a' }), null, [createElement('group', { key: 'b' })]),
                createElement(Sized),
                createElement(Fragment, null, createElement('text', null, 'in a fragment')),
            );
        }
        "#,
    );
    let outline = hands.app.outline();
    assert!(outline.contains("there are 2 2 true false"), "{outline}");
    assert!(outline.contains("group \"n0\""), "{outline}");
    assert!(outline.contains("group \"n1\""), "{outline}");
    assert!(outline.contains("sized 7"), "{outline}");
    assert!(outline.contains("in a fragment"), "{outline}");
}

/// One context's Provider in the place of another's is a new provider: what's inside reads the new one's value, and the old
/// context is back to what it is by itself, as React has it.
#[test]
fn a_provider_of_another_context_in_the_same_place_provides_its_own() {
    let mut hands = open(
        "provider-swap",
        r#"
        import { createContext, useContext, useState } from 'react';
        const A = createContext('a by itself');
        const B = createContext('b by itself');
        function Read() { return <text>{'A=' + useContext(A) + ' B=' + useContext(B)}</text>; }
        export default function App() {
            const [swapped, setSwapped] = useState(false);
            const inside = swapped ? <B.Provider value="b given"><Read /></B.Provider> : <A.Provider value="a given"><Read /></A.Provider>;
            return <window>{inside}<button key="swap" onClick={() => setSwapped(true)}>Swap</button></window>;
        }
        "#,
    );
    assert!(hands.says("A=a given B=b by itself"), "{}", hands.app.outline());
    hands.click("swap");
    hands.settle();
    assert!(hands.says("A=a by itself B=b given"), "{}", hands.app.outline());
}

#[test]
fn a_context_has_a_consumer_and_a_transition_just_happens() {
    let mut hands = open(
        "consumer",
        r#"
        import { createContext, useTransition, useDeferredValue, startTransition, useState } from 'react';
        const Theme = createContext('none');
        export default function App() {
            const [pending, start] = useTransition();
            const [value, setValue] = useState(0);
            const later = useDeferredValue(value);
            return (
                <Theme.Provider value="dark">
                    <window>
                        <Theme.Consumer>{(theme) => <text>{'consumed ' + theme}</text>}</Theme.Consumer>
                        <text>{'value ' + later + ' ' + pending}</text>
                        <button key="go" onClick={() => start(() => startTransition(() => setValue(1)))}>Go</button>
                    </window>
                </Theme.Provider>
            );
        }
        "#,
    );
    assert!(hands.says("consumed dark"), "{}", hands.app.outline());
    hands.settle();
    hands.click("go");
    hands.settle();
    assert!(hands.says("value 1 false"), "{}", hands.app.outline());
}

#[test]
fn react_dom_has_what_libraries_call_and_says_how_an_app_starts() {
    let hands = open(
        "dom",
        r#"
        import ReactDOM, { createPortal, flushSync, unstable_batchedUpdates } from 'react-dom';
        import { createRoot } from 'react-dom/client';
        let said = '';
        try { createRoot(null); } catch (error) { said = error.message; }
        export default function App() {
            let ran = 0;
            flushSync(() => { ran += 1; });
            unstable_batchedUpdates(() => { ran += 1; });
            return (
                <window>
                    {createPortal(<text>In a portal</text>, null)}
                    <text>{'ran ' + ran + ' ' + typeof ReactDOM.findDOMNode}</text>
                    <text>{said}</text>
                </window>
            );
        }
        "#,
    );
    let outline = hands.app.outline();
    assert!(outline.contains("In a portal"), "{outline}");
    assert!(outline.contains("ran 2 function"), "{outline}");
    assert!(outline.contains("default export"), "{outline}");
}

#[test]
fn what_is_built_for_the_newer_jsx_runs() {
    let hands = open(
        "runtime",
        r#"
        import { jsx, jsxs, Fragment } from 'react/jsx-runtime';
        function Named({ label, children }) { return jsxs('group', { children: [jsx('text', { children: 'named ' + label }), children] }); }
        export default function App() {
            return jsxs('window', { children: [
                jsx(Named, { label: 'x', children: jsx('text', { children: 'inside' }) }),
                jsx(Fragment, { children: jsx('text', { children: 'keyed' }, 'k') }),
            ] });
        }
        "#,
    );
    let outline = hands.app.outline();
    assert!(outline.contains("named x"), "{outline}");
    assert!(outline.contains("inside"), "{outline}");
    assert!(outline.contains("keyed"), "{outline}");
}

#[test]
fn an_error_boundary_shows_what_it_says_instead() {
    let mut hands = open(
        "boundary",
        r#"
        import { Component, useState } from 'react';
        const seen = [];
        class Boundary extends Component {
            state = { failed: null };
            static getDerivedStateFromError(error) { return { failed: error.message }; }
            componentDidCatch(error) { seen.push('caught ' + error.message); }
            render() {
                if (this.state.failed !== null) {
                    return <text>{'instead: ' + this.state.failed + ' ' + seen.join()}</text>;
                }
                return this.props.children;
            }
        }
        function Fragile() {
            const [broken, setBroken] = useState(false);
            if (broken) { throw new Error('it broke'); }
            return <button key="break" onClick={() => setBroken(true)}>Break</button>;
        }
        export default function App() {
            const [, redraw] = useState(0);
            return (
                <window>
                    <Boundary><Fragile /></Boundary>
                    <text>Next to it</text>
                    <button key="again" onClick={() => redraw((n) => n + 1)}>Again</button>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    hands.click("break");
    hands.settle();
    assert!(hands.says("instead: it broke"), "{}", hands.app.outline());
    assert!(hands.says("Next to it"), "{}", hands.app.outline());
}

#[test]
fn an_error_without_a_boundary_still_shows_where_it_happened() {
    let hands = open(
        "no-boundary",
        r#"
        import { Component } from 'react';
        class Plain extends Component { render() { throw new Error('nobody catches this'); } }
        export default function App() { return <window><Plain /><text>Next to it</text></window>; }
        "#,
    );
    let outline = hands.app.outline();
    assert!(outline.contains("nobody catches this"), "{outline}");
    assert!(outline.contains("Next to it"), "{outline}");
}

#[test]
fn should_component_update_and_pure_components_skip_rendering() {
    let mut hands = open(
        "should",
        r#"
        import { Component, PureComponent } from 'react';
        let plain = 0;
        let pure = 0;
        class Stops extends Component {
            state = { n: 0 };
            shouldComponentUpdate(props, state) { return state.n !== 1; }
            render() {
                plain += 1;
                return (
                    <group>
                        <text>{'stops ' + this.state.n + ' rendered ' + plain}</text>
                        <button key="more" onClick={() => this.setState((s) => ({ n: s.n + 1 }))}>More</button>
                    </group>
                );
            }
        }
        class Pure extends PureComponent {
            state = { n: 0 };
            render() {
                pure += 1;
                return (
                    <group>
                        <text>{'pure ' + this.state.n + ' rendered ' + pure}</text>
                        <button key="same" onClick={() => this.setState({ n: 0 })}>Same</button>
                        <button key="changed" onClick={() => this.setState({ n: 1 })}>Changed</button>
                        <button key="forced" onClick={() => this.forceUpdate()}>Forced</button>
                    </group>
                );
            }
        }
        export default function App() { return <window><Stops /><Pure /></window>; }
        "#,
    );
    hands.settle();
    hands.click("more");
    hands.settle();
    assert!(hands.says("stops 0 rendered 1"), "{}", hands.app.outline());
    hands.click("more");
    hands.settle();
    assert!(hands.says("stops 2 rendered 2"), "{}", hands.app.outline());
    hands.click("same");
    hands.settle();
    assert!(hands.says("pure 0 rendered 1"), "{}", hands.app.outline());
    hands.click("changed");
    hands.settle();
    assert!(hands.says("pure 1 rendered 2"), "{}", hands.app.outline());
    hands.click("forced");
    hands.settle();
    assert!(hands.says("pure 1 rendered 3"), "{}", hands.app.outline());
}

#[test]
fn a_ref_on_a_class_is_the_instance_and_old_lifecycles_run() {
    let mut hands = open(
        "class-ref",
        r#"
        import { Component, createRef } from 'react';
        const log = [];
        class Named extends Component {
            UNSAFE_componentWillMount() { this.setState({ started: true }); log.push('will mount'); }
            UNSAFE_componentWillReceiveProps(props) { log.push('will receive ' + props.n); }
            componentWillUpdate(props) { log.push('will update ' + props.n); }
            state = { started: false };
            hello() { return 'hello from the instance'; }
            render() { return <text>{'started ' + this.state.started + ' ref in props ' + ('ref' in this.props)}</text>; }
        }
        export default class App extends Component {
            named = createRef();
            state = { n: 0, said: '' };
            render() {
                return (
                    <window>
                        <Named ref={this.named} n={this.state.n} />
                        <text>{this.state.said + ' ' + log.join()}</text>
                        <button key="go" onClick={() => this.setState({ n: 1, said: this.named.current.hello() })}>Go</button>
                    </window>
                );
            }
        }
        "#,
    );
    assert!(
        hands.says("started true ref in props false"),
        "{}",
        hands.app.outline()
    );
    hands.settle();
    hands.click("go");
    hands.settle();
    assert!(
        hands.says("hello from the instance"),
        "{}",
        hands.app.outline()
    );
    let outline = hands.app.outline();
    assert!(outline.contains("will mount"), "{outline}");
    // What a parent passes on gets as far as these once it's rendered, which comes after the parent's own text.
    hands.click("go");
    hands.settle();
    let outline = hands.app.outline();
    assert!(
        outline.contains("will receive 1,will update 1"),
        "{outline}"
    );
}

#[test]
fn a_class_compiled_for_old_browsers_works() {
    let hands = open(
        "old-class",
        r#"
        import { Component, PureComponent } from 'react';
        function Old() {
            const self = Component.apply(this, arguments) || this;
            self.state = { from: 'old' };
            return self;
        }
        Old.prototype = Object.create(Component.prototype);
        Old.prototype.constructor = Old;
        Old.prototype.render = function () { return <text>{'made the ' + this.state.from + ' way, ' + this.props.say}</text>; };
        function Older() { return PureComponent.apply(this, arguments) || this; }
        Older.prototype = Object.create(PureComponent.prototype);
        Older.prototype.constructor = Older;
        Older.prototype.render = function () { return <text>And a pure one</text>; };
        export default function App() { return <window><Old say="hi" /><Older /></window>; }
        "#,
    );
    let outline = hands.app.outline();
    assert!(outline.contains("made the old way, hi"), "{outline}");
    assert!(outline.contains("And a pure one"), "{outline}");
}

#[test]
fn default_props_apply_however_an_element_is_written() {
    let hands = open(
        "defaults",
        r#"
        import React, { createElement, forwardRef } from 'react';
        function Sized({ size, label }) { return <text>{label + ' ' + size}</text>; }
        Sized.defaultProps = { size: 7 };
        export default function App() {
            return (
                <window>
                    <Sized label="written" />
                    <Sized label="undefined" size={undefined} />
                    <Sized label="given" size={1} />
                    {createElement(Sized, { label: 'made' })}
                </window>
            );
        }
        "#,
    );
    let outline = hands.app.outline();
    for said in ["written 7", "undefined 7", "given 1", "made 7"] {
        assert!(outline.contains(said), "{said}: {outline}");
    }
}

#[test]
fn children_can_be_given_as_a_prop_to_what_is_built_in() {
    let hands = open(
        "children-prop",
        r#"
        import { createElement, cloneElement, Fragment } from 'react';
        export default function App() {
            const fragment = <Fragment><text>In a cloned fragment</text></Fragment>;
            return (
                <window>
                    {createElement('group', { children: createElement('text', null, 'as a prop') })}
                    {createElement(Fragment, { children: [createElement('text', { key: 'a' }, 'in a fragment prop')] })}
                    {cloneElement(fragment)}
                    {cloneElement(<group><text>In a cloned group</text></group>, { name: 'named' })}
                </window>
            );
        }
        "#,
    );
    let outline = hands.app.outline();
    for said in [
        "as a prop",
        "in a fragment prop",
        "In a cloned fragment",
        "In a cloned group",
        "group \"named\"",
    ] {
        assert!(outline.contains(said), "{said}: {outline}");
    }
}

#[test]
fn a_click_handler_is_given_what_web_code_looks_for() {
    let mut hands = open(
        "click-event",
        r#"
        import { useState } from 'rsui';
        export default function App() {
            const [said, setSaid] = useState('nothing');
            const click = (event) => {
                const before = event.defaultPrevented;
                event.preventDefault();
                event.stopPropagation();
                event.currentTarget.blur();
                setSaid(event.type + ' ' + before + ' ' + event.defaultPrevented + ' ' + event.isDefaultPrevented());
            };
            return (
                <window>
                    <button key="go" onClick={click}>Go</button>
                    <text>{said}</text>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    hands.click("go");
    hands.settle();
    assert!(
        hands.says("click false true true"),
        "{}",
        hands.app.outline()
    );
    // Each click has its own.
    hands.click("go");
    hands.settle();
    assert!(
        hands.says("click false true true"),
        "{}",
        hands.app.outline()
    );
}

fn run_with_storage(app: &PathBuf, storage: &PathBuf, steps: &str) -> String {
    let output = Command::new(TOOL)
        .env("RSUI_STORAGE_FILE", storage)
        .arg("--do")
        .arg(steps)
        .arg(app)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{said}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    said
}

#[test]
fn local_storage_lasts_and_session_storage_does_not() {
    let app = write(
        "storage",
        r#"
        import { useState } from 'rsui';
        export default function App() {
            const [, redraw] = useState(0);
            const show = 'local ' + localStorage.getItem('a') + ' ' + localStorage.b + ' ' + localStorage.length + ' session ' + sessionStorage.getItem('a');
            return (
                <window>
                    <text>{show}</text>
                    <button key="set" onClick={() => { localStorage.setItem('a', 1); localStorage.b = 'two'; sessionStorage.setItem('a', 'x'); redraw((n) => n + 1); }}>Set</button>
                    <button key="drop" onClick={() => { localStorage.removeItem('a'); delete localStorage.b; redraw((n) => n + 1); }}>Drop</button>
                    <button key="clear" onClick={() => { localStorage.setItem('c', '3'); localStorage.clear(); redraw((n) => n + 1); }}>Clear</button>
                </window>
            );
        }
        "#,
    );
    let storage = app.with_file_name("kept.json");
    let _ = std::fs::remove_file(&storage);
    run_with_storage(
        &app,
        &storage,
        "expect \"local null undefined 0 session null\"; click #set; expect \"local 1 two 2 session x\"",
    );
    assert!(storage.exists(), "it's in a file by now");
    // Another run of the program has the one, and not the other. What's kept is text, as in a browser.
    run_with_storage(
        &app,
        &storage,
        "expect \"local 1 two 2 session null\"; click #drop; expect \"local null undefined 0\"",
    );
    run_with_storage(
        &app,
        &storage,
        "expect \"local null undefined 0\"; click #set; click #clear; expect \"local null undefined 0\"",
    );
}

#[test]
fn nothing_is_lost_to_a_storage_file_that_is_not_readable() {
    let app = write(
        "bad-storage",
        "export default function App() { return <window><text>{'read ' + localStorage.getItem('a')}</text></window>; }",
    );
    let storage = app.with_file_name("bad.json");
    std::fs::write(&storage, "this isn't json").unwrap();
    run_with_storage(&app, &storage, "expect \"read null\"");
}

// React 19.

#[test]
fn use_reads_a_context_where_a_hook_could_not_and_a_context_is_its_own_provider() {
    let mut hands = open(
        "use-context",
        r#"
        import React, { createContext, use, useState } from 'react';
        const Theme = createContext('none');
        function Maybe({ show }) {
            if (!show) { return <text>hidden</text>; }
            return <text>{'used ' + use(Theme)}</text>;
        }
        export default function App() {
            const [show, setShow] = useState(false);
            const [theme, setTheme] = useState('dark');
            return (
                <Theme value={theme}>
                    <window>
                        <Maybe show={show} />
                        <text>{'outside ' + use(Theme) + ' version ' + React.version.split('.')[0]}</text>
                        <button key="show" onClick={() => setShow(true)}>Show</button>
                        <button key="light" onClick={() => setTheme('light')}>Light</button>
                    </window>
                </Theme>
            );
        }
        "#,
    );
    assert!(hands.says("hidden"), "{}", hands.app.outline());
    // What's around the provider has what there is without one.
    assert!(hands.says("outside none version 19"), "{}", hands.app.outline());
    hands.click("show");
    hands.settle();
    assert!(hands.says("used dark"), "{}", hands.app.outline());
    hands.click("light");
    hands.settle();
    assert!(hands.says("used light"), "{}", hands.app.outline());
}

#[test]
fn use_waits_for_a_promise_with_the_nearest_suspense() {
    let mut hands = open(
        "use-promise",
        r#"
        import { Component, Suspense, use, useState } from 'react';
        let resolve;
        const data = new Promise((done) => { resolve = done; });
        let reject;
        const failing = new Promise((_, failed) => { reject = failed; });
        class Boundary extends Component {
            state = { failed: null };
            static getDerivedStateFromError(error) { return { failed: error.message }; }
            render() { return this.state.failed === null ? this.props.children : <text>{'caught ' + this.state.failed}</text>; }
        }
        function Data() { return <text>{'loaded ' + use(data)}</text>; }
        function Fails() { return <text>{'never ' + use(failing)}</text>; }
        function Counter() {
            const [count, setCount] = useState(0);
            return <button key="count" onClick={() => setCount((n) => n + 1)}>{'counted ' + count}</button>;
        }
        export default function App() {
            return (
                <window>
                    <Suspense fallback={<text>waiting</text>}>
                        <Data />
                        <text>beside it</text>
                    </Suspense>
                    <Boundary>
                        <Suspense fallback={<text>failing soon</text>}><Fails /></Suspense>
                    </Boundary>
                    <Counter />
                    <button key="resolve" onClick={() => resolve('yes')}>Resolve</button>
                    <button key="reject" onClick={() => reject(new Error('no'))}>Reject</button>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    let outline = hands.app.outline();
    assert!(outline.contains("waiting"), "{outline}");
    assert!(outline.contains("failing soon"), "{outline}");
    // What's beside what waits waits with it, as what's in a Suspense shows all at once.
    assert!(!outline.contains("beside it"), "{outline}");
    // What's outside goes on as it would.
    hands.click("count");
    hands.settle();
    assert!(hands.says("counted 1"), "{}", hands.app.outline());
    hands.click("resolve");
    hands.wait_for("loaded yes");
    let outline = hands.app.outline();
    assert!(outline.contains("beside it"), "{outline}");
    assert!(!outline.contains("waiting"), "{outline}");
    hands.click("reject");
    hands.wait_for("caught no");
    assert!(hands.says("counted 1"), "{}", hands.app.outline());
}

#[test]
fn a_fallback_that_waits_waits_with_the_suspense_around_it_and_lazy_uses_suspense() {
    let mut hands = open(
        "nested-suspense",
        r#"
        import { Suspense, lazy, use } from 'react';
        let slowDone;
        const slow = new Promise((done) => { slowDone = done; });
        let loadDone;
        const Later = lazy(() => new Promise((done) => { loadDone = done; }));
        let bareDone;
        const bare = new Promise((done) => { bareDone = done; });
        function Slow() { return <text>{'slow ' + use(slow)}</text>; }
        function Bare() { return <text>{'bare ' + use(bare)}</text>; }
        export default function App() {
            return (
                <window>
                    <Suspense fallback={<text>outer waiting</text>}>
                        <Suspense fallback={<Slow />}>
                            <Later who="x" />
                        </Suspense>
                    </Suspense>
                    <Bare />
                    <text>after bare</text>
                    <button key="slow" onClick={() => slowDone('done')}>Slow</button>
                    <button key="load" onClick={() => loadDone({ default: ({ who }) => <text>{'lazy ' + who}</text> })}>Load</button>
                    <button key="bare" onClick={() => bareDone('ok')}>Bare</button>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    let outline = hands.app.outline();
    assert!(outline.contains("outer waiting"), "{outline}");
    // With no Suspense around it, it's only what waits that shows nothing.
    assert!(outline.contains("after bare"), "{outline}");
    assert!(!outline.contains("bare ok"), "{outline}");
    hands.click("slow");
    hands.wait_for("slow done");
    assert!(!hands.says("outer waiting"), "{}", hands.app.outline());
    hands.click("load");
    hands.wait_for("lazy x");
    assert!(!hands.says("slow done"), "{}", hands.app.outline());
    hands.click("bare");
    hands.wait_for("bare ok");
}

#[test]
fn a_promise_that_is_thrown_suspends_as_libraries_do_it() {
    let mut hands = open(
        "thrown-promise",
        r#"
        import { Suspense } from 'react';
        let value = null;
        let finish;
        const waiting = new Promise((done) => { finish = () => { value = 'ready'; done(); }; });
        function Reads() {
            if (value === null) { throw waiting; }
            return <text>{'thrown and ' + value}</text>;
        }
        export default function App() {
            return (
                <window>
                    <Suspense fallback={<text>thrown waiting</text>}><Reads /></Suspense>
                    <button key="finish" onClick={() => finish()}>Finish</button>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    assert!(hands.says("thrown waiting"), "{}", hands.app.outline());
    hands.click("finish");
    hands.wait_for("thrown and ready");
}

#[test]
fn use_action_state_runs_actions_in_turn_and_use_optimistic_shows_them_meanwhile() {
    let mut hands = open(
        "action-state",
        r#"
        import { Component, useActionState, useOptimistic, startTransition } from 'react';
        const finishing = [];
        const seen = [];
        async function add(count, step) {
            seen.push(count + '+' + step);
            await new Promise((done) => finishing.push(done));
            if (step === 0) { throw new Error('nothing to add'); }
            return count + step;
        }
        class Boundary extends Component {
            state = { failed: null };
            static getDerivedStateFromError(error) { return { failed: error.message }; }
            render() { return this.state.failed === null ? this.props.children : <text>{'action failed: ' + this.state.failed}</text>; }
        }
        function Counter() {
            const [count, dispatch, pending] = useActionState(add, 0);
            const [shown, addShown] = useOptimistic(count, (now, step) => now + step);
            const go = (step) => startTransition(() => { addShown(step); dispatch(step); });
            return (
                <group>
                    <text>{'count ' + count + ' shown ' + shown + ' pending ' + pending + ' seen ' + seen.join()}</text>
                    <button key="five" onClick={() => go(5)}>Five</button>
                    <button key="two" onClick={() => go(2)}>Two</button>
                    <button key="zero" onClick={() => go(0)}>Zero</button>
                    <button key="finish" onClick={() => finishing.shift()()}>Finish</button>
                </group>
            );
        }
        function Plain() {
            const [said, say] = useActionState((before, word) => before + ' ' + word, 'said');
            return <button key="say" onClick={() => say('hi')}>{said}</button>;
        }
        export default function App() {
            return <window><Boundary><Counter /></Boundary><Plain /></window>;
        }
        "#,
    );
    hands.settle();
    hands.click("five");
    hands.settle();
    assert!(
        hands.says("count 0 shown 5 pending true seen 0+5"),
        "{}",
        hands.app.outline()
    );
    // The second waits for the first, and is given what that made.
    hands.click("two");
    hands.settle();
    assert!(
        hands.says("count 0 shown 7 pending true"),
        "{}",
        hands.app.outline()
    );
    hands.click("finish");
    hands.wait_for("count 5 shown 7 pending true seen 0+5,5+2");
    hands.click("finish");
    hands.wait_for("count 7 shown 7 pending false");
    // One that isn't async is done straight away.
    hands.click("say");
    hands.settle();
    assert!(hands.says("said hi"), "{}", hands.app.outline());
    // What goes wrong in one goes to the error boundary.
    hands.click("zero");
    hands.settle();
    hands.click("finish");
    hands.wait_for("action failed: nothing to add");
}

#[test]
fn an_async_transition_is_pending_until_it_is_done_and_what_it_throws_is_caught() {
    let mut hands = open(
        "async-transition",
        r#"
        import { Component, useState, useTransition } from 'react';
        let finish;
        class Boundary extends Component {
            state = { failed: null };
            static getDerivedStateFromError(error) { return { failed: error.message }; }
            render() { return this.state.failed === null ? this.props.children : <text>{'transition failed: ' + this.state.failed}</text>; }
        }
        function Saver() {
            const [pending, start] = useTransition();
            const [saved, setSaved] = useState('nothing');
            const run = (fails) => start(async () => {
                await new Promise((done) => { finish = done; });
                if (fails) { throw new Error('could not save'); }
                setSaved('something');
            });
            return (
                <group>
                    <text>{'saved ' + saved + ' pending ' + pending}</text>
                    <button key="save" onClick={() => run(false)}>Save</button>
                    <button key="fail" onClick={() => run(true)}>Fail</button>
                    <button key="finish" onClick={() => finish()}>Finish</button>
                </group>
            );
        }
        export default function App() { return <window><Boundary><Saver /></Boundary></window>; }
        "#,
    );
    hands.settle();
    hands.click("save");
    hands.settle();
    assert!(
        hands.says("saved nothing pending true"),
        "{}",
        hands.app.outline()
    );
    hands.click("finish");
    hands.wait_for("saved something pending false");
    hands.click("fail");
    hands.settle();
    hands.click("finish");
    hands.wait_for("transition failed: could not save");
}

#[test]
fn an_optimistic_update_outside_a_transition_is_undone_straight_away() {
    let mut hands = open(
        "optimistic-outside",
        r#"
        import { useOptimistic, useState } from 'react';
        export default function App() {
            const [real, setReal] = useState('real');
            const [shown, show] = useOptimistic(real);
            return (
                <window>
                    <text>{'shows ' + shown}</text>
                    <button key="show" onClick={() => { show('hoped'); setReal('changed'); }}>Show</button>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    hands.click("show");
    hands.settle();
    assert!(hands.says("shows changed"), "{}", hands.app.outline());
}

#[test]
fn use_deferred_value_starts_with_its_initial_value() {
    let mut hands = open(
        "deferred",
        r#"
        import { useDeferredValue } from 'react';
        const seen = [];
        function Shows() {
            const value = useDeferredValue('final', 'initial');
            const plain = useDeferredValue('plain');
            seen.push(value);
            return <text>{'deferred ' + seen.join() + ' ' + plain}</text>;
        }
        export default function App() { return <window><Shows /></window>; }
        "#,
    );
    hands.settle();
    assert!(
        hands.says("deferred initial,final plain"),
        "{}",
        hands.app.outline()
    );
}

#[test]
fn a_function_component_is_given_ref_as_a_prop_and_a_ref_can_clean_up() {
    let mut hands = open(
        "ref-prop",
        r#"
        import { Component, createElement, useImperativeHandle, useRef, useState } from 'react';
        import { jsx } from 'react/jsx-runtime';
        const log = [];
        function Inner({ ref, label }) {
            useImperativeHandle(ref, () => 'handle ' + label, [label]);
            return <text>{'inner ' + label}</text>;
        }
        class Counted extends Component { render() { return <text>counted</text>; } }
        const callback = (value) => {
            log.push('set ' + value);
            return () => log.push('cleaned ' + value);
        };
        const classCallback = (value) => {
            log.push('class ' + (value instanceof Counted));
            return () => log.push('class cleaned');
        };
        export default function App() {
            const [on, setOn] = useState(true);
            const [, redraw] = useState(0);
            const written = useRef(null);
            const made = useRef(null);
            const compiled = useRef(null);
            return (
                <window>
                    {on ? <Inner key="a" ref={callback} label="a" /> : null}
                    {on ? <Counted key="counted" ref={classCallback} /> : null}
                    <Inner key="b" ref={written} label="b" />
                    {createElement(Inner, { key: 'c', ref: made, label: 'c' })}
                    {jsx(Inner, { ref: compiled, label: 'd' }, 'd')}
                    <text>{'refs ' + written.current + ', ' + made.current + ', ' + compiled.current + ' log ' + log.join()}</text>
                    <button key="off" onClick={() => setOn(false)}>Off</button>
                    <button key="again" onClick={() => redraw((n) => n + 1)}>Again</button>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    hands.click("again");
    hands.settle();
    assert!(
        hands.says("refs handle b, handle c, handle d log set handle a,class true"),
        "{}",
        hands.app.outline()
    );
    hands.click("off");
    hands.settle();
    hands.click("again");
    hands.settle();
    // What the callback gave back is called, rather than the callback with null.
    let outline = hands.app.outline();
    assert!(outline.contains("refs handle b, handle c, handle d"), "{outline}");
    assert!(outline.contains("cleaned handle a"), "{outline}");
    assert!(outline.contains("class cleaned"), "{outline}");
    assert!(!outline.contains("set null"), "{outline}");
    assert!(!outline.contains("class false"), "{outline}");
}

#[test]
fn react_dom_has_form_status_and_resource_hints() {
    let hands = open(
        "form-status",
        r#"
        import ReactDOM, { useFormStatus, preload, preconnect, prefetchDNS, preloadModule, preinit } from 'react-dom';
        function Status() {
            const { pending, data, method } = useFormStatus();
            return <text>{'pending ' + pending + ' ' + (data === null ? 'no data' : data.get('name')) + ' ' + method}</text>;
        }
        const sent = { get: (name) => 'sent ' + name };
        // What a page's form gives what's in it while its action runs: see `formSending` in 'rsui/web'.
        const status = { pending: true, data: sent, method: 'post', action: () => {} };
        const form = { subscribe: () => () => {}, getStatus: () => status };
        export default function App() {
            preload('/font.woff2', { as: 'font' });
            preconnect('https://example.com');
            prefetchDNS('https://example.com');
            preloadModule('/module.js');
            return (
                <window>
                    <Status />
                    <ReactDOM.__FormStatus.Provider value={form}>
                        <Status />
                    </ReactDOM.__FormStatus.Provider>
                    <text>{'dom ' + ReactDOM.version.split('.')[0] + ' ' + typeof preinit}</text>
                </window>
            );
        }
        "#,
    );
    let outline = hands.app.outline();
    assert!(outline.contains("pending false no data null"), "{outline}");
    assert!(outline.contains("pending true sent name post"), "{outline}");
    assert!(outline.contains("dom 19 function"), "{outline}");
}

#[test]
fn layout_effects_run_before_passive_effects_inside_out_and_insertion_effects_first() {
    let mut hands = open(
        "effect-order",
        r#"
        import { useEffect, useLayoutEffect, useInsertionEffect, useState } from 'react';
        const log = [];
        function Child({ name }) {
            useEffect(() => { log.push(name + ' effect'); }, []);
            useLayoutEffect(() => { log.push(name + ' layout'); }, []);
            useInsertionEffect(() => { log.push(name + ' insert'); }, []);
            return <text>{name}</text>;
        }
        export default function App() {
            const [shown, setShown] = useState('');
            // Written before the layout effect, which runs first all the same, as React's <Navigate> needs.
            useEffect(() => { log.push('app effect'); setShown(log.join(', ')); }, []);
            useLayoutEffect(() => { log.push('app layout'); }, []);
            return <window><Child name="a" /><Child name="b" /><text>{'order ' + shown}</text></window>;
        }
        "#,
    );
    hands.settle();
    assert!(
        hands.says(
            "order a insert, b insert, a layout, b layout, app layout, a effect, b effect, app effect"
        ),
        "{}",
        hands.app.outline()
    );
}

#[test]
fn state_set_in_a_layout_effect_is_drawn_with_the_first_frame() {
    let hands = open(
        "layout-state",
        r#"
        import { useLayoutEffect, useState } from 'react';
        export default function App() {
            const [measured, setMeasured] = useState('not yet');
            useLayoutEffect(() => { setMeasured('measured'); }, []);
            return <window><text>{measured}</text></window>;
        }
        "#,
    );
    // Opening it is one update, which is what's drawn first.
    assert!(hands.says("measured"), "{}", hands.app.outline());
}

#[test]
fn use_context_can_be_called_conditionally_as_it_takes_no_hook() {
    let mut hands = open(
        "conditional-context",
        r#"
        import { createContext, useContext, useState } from 'react';
        const Theme = createContext('light');
        function Presence({ on }) {
            const [count] = useState(0);
            const theme = on ? useContext(Theme) : 'none';
            return <text>{'theme ' + theme + ' ' + count}</text>;
        }
        export default function App() {
            const [on, setOn] = useState(false);
            return (
                <Theme.Provider value="dark">
                    <window>
                        <button key="add" onClick={() => setOn(true)}>Add</button>
                        <Presence on={on} />
                    </window>
                </Theme.Provider>
            );
        }
        "#,
    );
    assert!(hands.says("theme none 0"), "{}", hands.app.outline());
    hands.click("add");
    hands.settle();
    assert!(hands.says("theme dark 0"), "{}", hands.app.outline());
}

#[test]
fn react_19_2_has_use_effect_event_activity_and_cache() {
    let mut hands = open(
        "react-19-2",
        r#"
        import React, { Activity, useEffect, useEffectEvent, useState, cache, cacheSignal } from 'react';
        const effectsOf = [];
        function Counter() {
            const [count, setCount] = useState(0);
            useEffect(() => { effectsOf.push('on'); return () => effectsOf.push('off'); }, []);
            return <button key="count" onClick={() => setCount(count + 1)}>{'count ' + count + ' effects ' + effectsOf.join(',')}</button>;
        }
        const double = cache((n) => n * 2);
        export default function App() {
            const [hidden, setHidden] = useState(false);
            const [said, setSaid] = useState('');
            const latest = useEffectEvent(() => 'hidden ' + hidden);
            useEffect(() => { setSaid(latest()); }, [hidden]);
            return (
                <window>
                    <button key="toggle" onClick={() => setHidden(!hidden)}>Toggle</button>
                    <Activity mode={hidden ? 'hidden' : 'visible'}><Counter /></Activity>
                    <text>{'version ' + React.version + ' said ' + said + ' double ' + double(2) + ' signal ' + cacheSignal()}</text>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    assert!(
        hands.says("version 19.2.0 said hidden false double 4 signal null"),
        "{}",
        hands.app.outline()
    );
    hands.click("count");
    hands.settle();
    hands.click("toggle");
    hands.settle();
    // Hidden, it isn't there for anyone using the app, its effects are undone, and its state is kept.
    assert!(hands.says("group inert=true"), "{}", hands.app.outline());
    assert!(hands.says("said hidden true"), "{}", hands.app.outline());
    hands.click("toggle");
    hands.settle();
    hands.click("count");
    hands.settle();
    assert!(hands.says("count 2 effects on,off,on"), "{}", hands.app.outline());
    assert!(!hands.says("inert"), "{}", hands.app.outline());
}

#[test]
fn legacy_create_factory_and_a_forward_ref_without_a_name_of_its_own() {
    let hands = open(
        "legacy",
        r#"
        import React, { forwardRef } from 'react';
        class Box extends React.Component { render() { return <text>{'box ' + this.props.label}</text>; } }
        const make = React.createFactory(Box);
        const Fancy = forwardRef(function FancyInput(props, ref) { return <text>fancy</text>; });
        export default function App() {
            return <window>{make({ label: 'made' })}<Fancy /><text>{'named [' + Fancy.name + '] ' + Fancy.displayName}</text></window>;
        }
        "#,
    );
    let outline = hands.app.outline();
    assert!(outline.contains("box made"), "{outline}");
    assert!(outline.contains("fancy"), "{outline}");
    assert!(outline.contains("named [] FancyInput"), "{outline}");
}

#[test]
fn what_an_effect_or_a_lifecycle_throws_goes_to_the_error_boundary_with_where_it_was() {
    let mut hands = open(
        "effect-errors",
        r#"
        import React, { useEffect } from 'react';
        class Boundary extends React.Component {
            constructor(props) { super(props); this.state = { failed: null, stack: '' }; }
            static getDerivedStateFromError(error) { return { failed: error.message }; }
            componentDidCatch(error, info) { this.setState({ stack: info.componentStack }); }
            render() {
                if (this.state.failed !== null) {
                    return <text>{'caught ' + this.state.failed + ' at' + this.state.stack.replace(/\s+/g, ' ')}</text>;
                }
                return this.props.children;
            }
        }
        function Breaks() { useEffect(() => { throw new Error('in an effect'); }, []); return <text>fine</text>; }
        class Mounts extends React.Component { componentDidMount() { throw new Error('in a lifecycle'); } render() { return <text>mounted</text>; } }
        export default function App() {
            return (
                <window>
                    <Boundary><Breaks /></Boundary>
                    <Boundary><Mounts /></Boundary>
                    <text>still here</text>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    let outline = hands.app.outline();
    assert!(outline.contains("caught in an effect at in Breaks in Boundary"), "{outline}");
    assert!(outline.contains("caught in a lifecycle at in Mounts in Boundary"), "{outline}");
    assert!(outline.contains("still here"), "{outline}");
    assert!(!outline.contains("Something went wrong"), "{outline}");
}

#[test]
fn elements_made_through_react_have_their_children_in_their_props() {
    let hands = open(
        "props-children",
        r#"
        import { cloneElement, createElement } from 'react';
        const facts = [
            JSON.stringify(cloneElement({ dummy: true }, { key: 0 }, ['a', 'b']).props.children),
            JSON.stringify(createElement('text', null, 'x').props),
            JSON.stringify(cloneElement(createElement('text', null, 'x'), { children: 'y' }).props.children),
        ];
        export default function App() {
            return <window><text>{facts.join(' ').replace(/"/g, "'")}</text>{cloneElement(<text>old</text>, null, 'new')}</window>;
        }
        "#,
    );
    let outline = hands.app.outline();
    assert!(outline.contains("['a','b'] {'children':'x'} 'y'"), "{outline}");
    assert!(outline.contains("new"), "{outline}");
    assert!(!outline.contains("old"), "{outline}");
}

#[test]
fn state_set_back_to_what_was_rendered_does_not_render_again() {
    let mut hands = open(
        "bail-out",
        r#"
        import { useState, useLayoutEffect } from 'react';
        let renders = 0;
        function Measured({ kick }) {
            renders++;
            const [el, setEl] = useState(null);
            // As a callback ref does on each commit: null, and then what it had.
            useLayoutEffect(() => { setEl(null); setEl('node'); });
            return <text>{'el ' + el}</text>;
        }
        export default function App() {
            const [kick, setKick] = useState(0);
            return <window><button key="kick" onClick={() => setKick(kick + 1)}>Kick</button><Measured kick={kick} /><text>{'renders ' + renders}</text></window>;
        }
        "#,
    );
    hands.settle();
    assert!(hands.says("el node"), "{}", hands.app.outline());
    hands.click("kick");
    hands.settle();
    hands.click("kick");
    hands.settle();
    // Once to start with, once for 'node', and once for each kick, and never for state that ends where it was. The count's
    // shown as it was before the last kick rendered it.
    assert!(hands.says("renders 3"), "{}", hands.app.outline());
}

#[test]
fn a_timer_runs_while_every_frame_leaves_more_to_draw() {
    let mut hands = open(
        "dirty-frames",
        r#"
        import { useEffect, useState } from 'react';
        export default function App() {
            const [frames, setFrames] = useState(0);
            const [timer, setTimer] = useState('timer waiting');
            useEffect(() => { setTimeout(() => setTimer('timer ran'), 20); }, []);
            // Every render leaves state to set, which keeps something waiting to be drawn after every frame.
            queueMicrotask(() => setFrames((n) => n + 1));
            return <window><text>{timer}</text></window>;
        }
        "#,
    );
    hands.wait_for("timer ran");
}

#[test]
fn what_nothing_catches_shows_over_the_app_until_it_is_dismissed() {
    let mut hands = open(
        "dismiss-error",
        r#"
        import { useState, useEffect } from 'react';
        function Breaks() { useEffect(() => { throw new Error('nothing catches this'); }, []); return <text>broken</text>; }
        export default function App() {
            const [on, setOn] = useState(false);
            return <window><button key="break" onClick={() => setOn(true)}>Break</button>{on ? <Breaks /> : null}<text>the app</text></window>;
        }
        "#,
    );
    hands.click("break");
    hands.settle();
    assert!(hands.says("nothing catches this"), "{}", hands.app.outline());
    hands.click("error-dismiss");
    hands.settle();
    let outline = hands.app.outline();
    assert!(!outline.contains("Something went wrong"), "{outline}");
    assert!(outline.contains("the app"), "{outline}");
}
