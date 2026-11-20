//! A component made by an app's own JSX doesn't render again when what it's given does the same as before, with no `memo` or
//! `useCallback` (see `JsProps` in bridge.rs), and everything that should still show does.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use rsui_headless::{Headless, Query};

/// A new folder for each test, as tests run side by side.
fn folder() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "auto-memo-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

fn start_in(folder: &Path, source: &str) -> Headless {
    let path = folder.join("app.tsx");
    std::fs::write(&path, source).unwrap();
    let entry = rsui_tsx::load(&path, 512 * 1024).unwrap();
    Headless::new(rsui_tsx::root, entry)
}

fn start(source: &str) -> Headless {
    start_in(&folder(), source)
}

fn key(name: &str) -> Query {
    Query::new().key(name)
}

fn line_with(ui: &Headless, part: &str) -> String {
    ui.outline()
        .lines()
        .find(|line| line.contains(part))
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("nothing says {part:?} in\n{}", ui.outline()))
}

/// How many components render for a click.
fn renders_for(ui: &mut Headless, name: &str) -> u64 {
    let before = ui.runtime().renders();
    ui.click(&key(name)).unwrap();
    ui.runtime().renders() - before
}

/// A list of rows, each given its item, a handler made in place, a style object made in place, and the same settings object,
/// as the file explorer's rows are without `memo` and `useCallback`.
const ROWS: &str = r#"
    import { useState } from 'rsui';
    const look = { color: 0x202020ff, size: 14 };
    function Row(props) {
        return (
            <button key={'row' + props.index} onClick={props.onPick} background={props.selected ? 0x3060c0ff : undefined}>
                {props.item.name}{props.selected ? ' (selected)' : ''} {props.style.weight}
            </button>
        );
    }
    export default function App() {
        const [selected, setSelected] = useState(-1);
        const [items] = useState(() => Array.from({ length: 50 }, (_, index) => ({ name: 'item ' + index })));
        const pick = (index) => setSelected(index);
        return (
            <window>
                <column>
                    {items.map((item, index) => (
                        <Row
                            key={index}
                            index={index}
                            item={item}
                            look={look}
                            style={{ weight: 'bold' }}
                            selected={index === selected}
                            onPick={() => pick(index)}
                        />
                    ))}
                </column>
            </window>
        );
    }
"#;

#[test]
fn moving_the_selection_renders_only_the_rows_that_changed() {
    let mut ui = start(ROWS);
    // The app, and the one row that's now selected.
    assert_eq!(renders_for(&mut ui, "row3"), 2);
    assert!(line_with(&ui, "item 3").contains("(selected)"));
    // The app, the row that was selected and the one that is now.
    assert_eq!(renders_for(&mut ui, "row7"), 3);
    assert!(line_with(&ui, "item 7").contains("(selected)"));
    assert!(!line_with(&ui, "item 3 ").contains("(selected)"));
    // The handler a row kept from before still picks it.
    assert_eq!(renders_for(&mut ui, "row3"), 3);
    assert!(line_with(&ui, "item 3").contains("(selected)"));
}

#[test]
fn rows_in_a_file_written_for_a_page_skip_too() {
    // A `<div>` with a tag rsui has too, `<button>`, has the file's JSX call `__jsxPageMemo`. Each row says how often it's
    // rendered, as what the page puts around each of the `<div>`'s children renders too.
    let source = ROWS
        .replace("import { useState } from 'rsui';", "import { useRef, useState } from 'rsui';")
        .replace("function Row(props) {", "function Row(props) {
        const renders = useRef(0);
        renders.current++;")
        .replace("{props.style.weight}", "{props.style.weight} renders {renders.current}")
        .replace("<column>", "<column><div className=\"rows\">")
        .replace("</column>", "</div></column>");
    let mut ui = start(&source);
    ui.click(&key("row3")).unwrap();
    ui.click(&key("row7")).unwrap();
    assert!(line_with(&ui, "item 7").contains("(selected) bold renders 2"));
    assert!(line_with(&ui, "item 3 ").contains("bold renders 3"));
    assert!(line_with(&ui, "item 10").contains("bold renders 1"));
}

#[test]
fn saying_use_no_memo_in_the_file_renders_every_row() {
    let mut ui = start(&format!("'use no memo';\n{ROWS}"));
    assert_eq!(renders_for(&mut ui, "row3"), 51);
}

#[test]
fn saying_use_no_memo_in_the_parent_renders_every_row() {
    let source = ROWS.replacen(
        "export default function App() {",
        "export default function App() {
        'use no memo';",
        1,
    );
    let mut ui = start(&source);
    assert_eq!(renders_for(&mut ui, "row3"), 51);
}

#[test]
fn a_child_that_says_use_no_memo_renders_whenever_its_parent_does() {
    let source = ROWS.replacen("function Row(props) {", "function Row(props) {\n        'use no memo';", 1);
    let mut ui = start(&source);
    assert_eq!(renders_for(&mut ui, "row3"), 51);
}

#[test]
fn a_handler_that_sees_a_value_that_changed_is_given_again() {
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        function Child(props) {
            return <button key="child" onClick={props.onShow}>child</button>;
        }
        export default function App() {
            const [count, setCount] = useState(0);
            const [shown, setShown] = useState('none');
            return (
                <window>
                    <button key="add" onClick={() => setCount(count + 1)}>add</button>
                    <Child onShow={() => setShown('saw ' + count)} />
                    <text>shown {shown}</text>
                </window>
            );
        }
        "#,
    );
    ui.click(&key("add")).unwrap();
    ui.click(&key("add")).unwrap();
    ui.click(&key("child")).unwrap();
    assert!(line_with(&ui, "shown").contains("shown saw 2"));
}

#[test]
fn a_handler_seeing_a_let_reads_what_it_holds_now() {
    // The `let` is in a cell, a new one each render, and two handlers are the same when what's in their cells is.
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        let renders = 0;
        function Child(props) {
            renders++;
            return <button key="child" onClick={props.onShow}>child renders {renders}</button>;
        }
        export default function App() {
            const [count, setCount] = useState(0);
            const [shown, setShown] = useState('none');
            let half = Math.floor(count / 2);
            half += 0;
            return (
                <window>
                    <button key="add" onClick={() => setCount(count + 1)}>add</button>
                    <Child onShow={() => setShown('half ' + half)} />
                    <text>shown {shown}</text>
                </window>
            );
        }
        "#,
    );
    // 0 to 1 leaves half at 0, so the child keeps what it had.
    assert_eq!(renders_for(&mut ui, "add"), 1);
    // 1 to 2 makes it 1, which the child's handler is handed without it rendering, as it's a handler (`onShow`) made from the
    // same code.
    assert_eq!(renders_for(&mut ui, "add"), 1);
    ui.click(&key("child")).unwrap();
    assert!(line_with(&ui, "shown").contains("shown half 1"));
}

#[test]
fn children_that_changed_show() {
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        function Box(props) {
            return <column>{props.children}</column>;
        }
        export default function App() {
            const [count, setCount] = useState(0);
            const [other, setOther] = useState(0);
            return (
                <window>
                    <button key="add" onClick={() => setCount(count + 1)}>add</button>
                    <button key="other" onClick={() => setOther(other + 1)}>other {other}</button>
                    <Box><text>count {count}</text></Box>
                </window>
            );
        }
        "#,
    );
    ui.click(&key("add")).unwrap();
    assert!(line_with(&ui, "count ").contains("count 1"));
    // The box is given the same element as before, so only the app renders.
    assert_eq!(renders_for(&mut ui, "other"), 1);
    assert!(line_with(&ui, "count ").contains("count 1"));
    assert_eq!(renders_for(&mut ui, "add"), 2);
    assert!(line_with(&ui, "count ").contains("count 2"));
}

#[test]
fn what_reads_a_context_renders_when_it_changes() {
    let mut ui = start(
        r#"
        import { createContext, useContext, useState } from 'rsui';
        const Theme = createContext('light');
        function Label(props) {
            const theme = useContext(Theme);
            return <text>theme {theme} {props.text}</text>;
        }
        export default function App() {
            const [theme, setTheme] = useState('light');
            return (
                <window>
                    <button key="flip" onClick={() => setTheme(theme === 'light' ? 'dark' : 'light')}>flip</button>
                    <Theme.Provider value={theme}>
                        <column>
                            <Label text="a" onHover={() => {}} />
                        </column>
                    </Theme.Provider>
                </window>
            );
        }
        "#,
    );
    ui.click(&key("flip")).unwrap();
    assert!(line_with(&ui, "theme").contains("theme dark a"));
    ui.click(&key("flip")).unwrap();
    assert!(line_with(&ui, "theme").contains("theme light a"));
}

#[test]
fn what_reads_a_store_renders_when_it_changes() {
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        import { useSyncExternalStore } from 'react';
        let value = 0;
        const listeners = new Set();
        const store = {
            subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
            get: () => value,
            bump() { value++; for (const listener of listeners) listener(); },
        };
        function Reading(props) {
            const seen = useSyncExternalStore(store.subscribe, store.get);
            return <text>store {seen}</text>;
        }
        export default function App() {
            return (
                <window>
                    <button key="bump" onClick={() => store.bump()}>bump</button>
                    <Reading onHover={() => {}} />
                </window>
            );
        }
        "#,
    );
    ui.click(&key("bump")).unwrap();
    ui.click(&key("bump")).unwrap();
    assert!(line_with(&ui, "store").contains("store 2"));
}

#[test]
fn a_child_given_a_ref_renders_whenever_its_parent_does() {
    let mut ui = start(
        r#"
        import { useRef, useState } from 'rsui';
        function Shows(props) {
            return <text>ref {props.held.current}</text>;
        }
        export default function App() {
            const [count, setCount] = useState(0);
            const held = useRef(0);
            return (
                <window>
                    <button key="add" onClick={() => { held.current = count + 1; setCount(count + 1); }}>add</button>
                    <Shows held={held} />
                </window>
            );
        }
        "#,
    );
    assert_eq!(renders_for(&mut ui, "add"), 2);
    assert!(line_with(&ui, "ref ").contains("ref 1"));
}

#[test]
fn state_in_a_child_that_skipped_still_renders_it() {
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        function Counter(props) {
            const [count, setCount] = useState(0);
            return <button key={'count' + props.name} onClick={() => setCount(count + 1)}>{props.name} {count}</button>;
        }
        export default function App() {
            const [other, setOther] = useState(0);
            return (
                <window>
                    <button key="other" onClick={() => setOther(other + 1)}>other {other}</button>
                    <Counter name="a" onHover={() => setOther(other)} />
                </window>
            );
        }
        "#,
    );
    ui.click(&key("other")).unwrap();
    ui.click(&key("counta")).unwrap();
    ui.click(&key("counta")).unwrap();
    assert!(line_with(&ui, "a 2").contains("a 2"));
}

#[test]
fn elements_made_in_node_modules_render_as_react_has_them() {
    let folder = folder();
    let package = folder.join("node_modules").join("rows");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.json"), r#"{ "name": "rows", "main": "index.jsx" }"#).unwrap();
    std::fs::write(
        package.join("index.jsx"),
        r#"
        export function Rows(props) {
            const Row = props.Row;
            const out = [];
            for (let index = 0; index < 20; index++) {
                out.push(<Row key={index} index={index} onPick={() => props.onPick(index)} />);
            }
            return <column>{out}</column>;
        }
        "#,
    )
    .unwrap();
    let mut ui = start_in(
        &folder,
        r#"
        import { useState } from 'rsui';
        import { Rows } from 'rows';
        function Row(props) {
            return <button key={'row' + props.index} onClick={props.onPick}>row {props.index}</button>;
        }
        export default function App() {
            const [picked, setPicked] = useState(-1);
            return (
                <window>
                    <text>picked {picked}</text>
                    <Rows Row={Row} picked={picked} onPick={setPicked} />
                </window>
            );
        }
        "#,
    );
    // The app, `Rows`, which is given another `picked`, and each of its rows, as their elements are its own.
    assert_eq!(renders_for(&mut ui, "row4"), 22);
    assert!(line_with(&ui, "picked").contains("picked 4"));
}

#[test]
fn a_component_from_node_modules_renders_whenever_its_parent_does() {
    // It can read what changes with no render, as rsui's own page elements read the page's CSS, so only the app's own components
    // are taken to show the same for the same.
    let folder = folder();
    let package = folder.join("node_modules").join("label");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.json"), r#"{ "name": "label", "main": "index.js" }"#).unwrap();
    std::fs::write(
        package.join("index.js"),
        "let renders = 0; exports.Label = function Label(props) { renders++; return __jsx('text', null, ['label ' + renders]); };",
    )
    .unwrap();
    let mut ui = start_in(
        &folder,
        r#"
        import { useState } from 'rsui';
        import { Label } from 'label';
        export default function App() {
            const [count, setCount] = useState(0);
            return (
                <window>
                    <button key="add" onClick={() => setCount(count + 1)}>add</button>
                    <Label onHover={() => {}} />
                </window>
            );
        }
        "#,
    );
    assert_eq!(renders_for(&mut ui, "add"), 2);
    assert!(line_with(&ui, "label ").contains("label 2"));
}

#[test]
fn the_same_object_with_methods_renders_it_again() {
    // TanStack Table gives the same `row` each time, which says it's selected once it is.
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        const store = { selected: false };
        const row = { isSelected: () => store.selected };
        function Cell(props) {
            return <text>{props.row.isSelected() ? 'cell on' : 'cell off'}</text>;
        }
        export default function App() {
            const [, setVersion] = useState(0);
            return (
                <window>
                    <button key="toggle" onClick={() => { store.selected = !store.selected; setVersion((v) => v + 1); }}>toggle</button>
                    <Cell row={row} />
                </window>
            );
        }
        "#,
    );
    ui.click(&key("toggle")).unwrap();
    assert!(line_with(&ui, "cell ").contains("cell on"));
}

#[test]
fn a_handler_that_sees_new_state_is_handed_it_without_the_child_rendering() {
    // As `useEvent` would have it by hand: the child keeps its handler, which now does what the new one does.
    let mut ui = start(
        r#"
        import { useEffect, useState } from 'rsui';
        let renders = 0;
        let effects = 0;
        function Child(props) {
            renders++;
            useEffect(() => { effects++; }, [props.onShow]);
            return <button key="child" onClick={props.onShow}>child renders {renders}</button>;
        }
        function Shows(props) {
            return <text>says {props.say()}</text>;
        }
        export default function App() {
            const [count, setCount] = useState(0);
            const [shown, setShown] = useState('none');
            return (
                <window>
                    <button key="add" onClick={() => setCount(count + 1)}>add</button>
                    <Child onShow={() => setShown('saw ' + count + ' after ' + effects + ' effects')} />
                    <Shows say={() => 'count ' + count} />
                    <text>shown {shown}</text>
                </window>
            );
        }
        "#,
    );
    // The app, and `Shows`, whose `say` is called as it renders, which isn't named as a handler is.
    assert_eq!(renders_for(&mut ui, "add"), 2);
    assert!(line_with(&ui, "says").contains("says count 1"));
    assert_eq!(renders_for(&mut ui, "add"), 2);
    assert!(line_with(&ui, "says").contains("says count 2"));
    assert!(line_with(&ui, "child renders").contains("child renders 1"));
    // What it was handed sees what the app's last render did, and its effect, which only runs when the handler is another,
    // ran once.
    ui.click(&key("child")).unwrap();
    assert!(line_with(&ui, "shown").contains("shown saw 2 after 1 effects"));
}

#[test]
fn a_handler_read_from_an_object_given_whole_is_handed_on_without_rendering_again_and_again() {
    // As the browser demo's `browser`, made by a hook, is given whole to what draws `onClick={browser.addSpace}`.
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        function useModel() {
            const [n, setN] = useState(0);
            return { n, add: () => setN((before) => before + 1) };
        }
        function Bar(props) {
            return <button key="add" onClick={props.model.add}>add</button>;
        }
        function Middle(props) {
            return (
                <div>
                    <p>{'n=' + props.model.n}</p>
                    <Bar model={props.model} />
                </div>
            );
        }
        function Inner() {
            const model = useModel();
            return <Middle model={model} />;
        }
        export default function App() {
            return (
                <window>
                    <Inner />
                </window>
            );
        }
        "#,
    );
    for n in 1..=3 {
        // What has the model, and `Bar`, which read a function from it, so the new one is only the same as itself: each once.
        // Looking again at `Bar` once its button was handed the new handler had it render until it was stopped.
        let rendered = renders_for(&mut ui, "add");
        assert!(rendered <= 4, "{rendered} rendered");
        assert!(line_with(&ui, "n=").contains(&format!("n={n}")));
    }
    assert_eq!(ui.runtime().runaways(), 0);
}

#[test]
fn handlers_that_see_an_object_of_handlers_that_sees_itself_are_the_same() {
    // As the file explorer's `act` is: each of what's in it can call the others through it.
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        let renders = 0;
        function Child(props) {
            renders++;
            return <button key="child" onClick={() => props.step()}>child renders {renders}</button>;
        }
        export default function App() {
            const [count, setCount] = useState(0);
            const [other, setOther] = useState(0);
            const act = { go: (n) => setCount(n), next: () => act.go(count + 1), back: () => act.go(count - 1) };
            return (
                <window>
                    <button key="other" onClick={() => setOther(other + 1)}>other {other}</button>
                    <Child step={act.next} />
                    <text>count {count}</text>
                </window>
            );
        }
        "#,
    );
    assert_eq!(renders_for(&mut ui, "other"), 1);
    // What the child's `step` sees has changed, which isn't a handler's name, but nothing rendering called it, so it's handed
    // on as a handler is, rather than the child rendering (see `reads.rs` in rsui-js).
    assert_eq!(renders_for(&mut ui, "child"), 1);
    assert!(line_with(&ui, "count ").contains("count 1"));
    ui.click(&key("child")).unwrap();
    assert!(line_with(&ui, "count ").contains("count 2"));
}

/// A page's `<div>`s around components given handlers made in place, a keyed list among them, and what reads a context.
const PAGE: &str = r#"
    import { createContext, useContext, useState } from 'rsui';
    const Theme = createContext('light');
    let renders = 0;
    function Tab(props) {
        renders++;
        return <button key={'tab' + props.id} onClick={props.onPick}>tab {props.label} renders {renders}</button>;
    }
    function Themed() {
        return <span>theme {useContext(Theme)}</span>;
    }
    export default function App() {
        const [other, setOther] = useState(0);
        const [label, setLabel] = useState('a');
        const [order, setOrder] = useState(['x', 'y', 'z']);
        const [theme, setTheme] = useState('light');
        const [picked, setPicked] = useState('none');
        return (
            <window>
                <button key="other" onClick={() => setOther(other + 1)}>other {other}</button>
                <button key="rename" onClick={() => setLabel(label + 'b')}>rename</button>
                <button key="reverse" onClick={() => setOrder([...order].reverse())}>reverse</button>
                <button key="flip" onClick={() => setTheme(theme === 'light' ? 'dark' : 'light')}>flip</button>
                <Theme.Provider value={theme}>
                    <div className="bar" style={{ padding: 4 }}>
                        <span>tabs</span>
                        <div className="list">
                            {order.map((id) => (
                                <Tab key={id} id={id} label={label + id} onPick={() => setPicked(label + id + ' ' + other)} />
                            ))}
                        </div>
                        <div className="themed"><Themed /></div>
                    </div>
                </Theme.Provider>
                <text>picked {picked}</text>
            </window>
        );
    }
"#;

#[test]
fn a_page_element_given_the_same_again_isnt_drawn_again() {
    let mut ui = start(PAGE);
    // The app, the button that says what `other` is now, and the provider, which isn't the app's: the `<div>`s are given what
    // does the same, handlers seeing `other` and all.
    assert_eq!(renders_for(&mut ui, "other"), 3);
    assert_eq!(renders_for(&mut ui, "other"), 3);
    ui.click(&key("taby")).unwrap();
    assert!(line_with(&ui, "picked").contains("picked ay 2"));
    // What changed in them still shows, a keyed list's order, and what reads a context.
    ui.click(&key("rename")).unwrap();
    assert!(line_with(&ui, "tab abx").contains("tab abx"));
    ui.click(&key("reverse")).unwrap();
    let outline = ui.outline();
    let at = |name: &str| outline.find(name).unwrap_or_else(|| panic!("no {name} in\n{outline}"));
    assert!(at("tab abz") < at("tab aby") && at("tab aby") < at("tab abx"));
    ui.click(&key("flip")).unwrap();
    assert!(line_with(&ui, "theme ").contains("theme dark"));
    ui.click(&key("tabz")).unwrap();
    assert!(line_with(&ui, "picked").contains("picked abz 2"));
}

#[test]
fn a_handler_made_again_before_something_on_a_page_doesnt_move_it() {
    // What's after a button whose handler sees what changed is still where it was, as far as CSS can tell, so it isn't told
    // so again and drawn again (see `sameToCss` in web.js), though CSS asks what's before it.
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        let renders = 0;
        function Label(props) {
            renders++;
            return <span className="label">label renders {renders}</span>;
        }
        export default function App() {
            const [other, setOther] = useState(0);
            return (
                <window>
                    <style>{'.row button + .label { color: red; }'}</style>
                    <div className="row">
                        <button key="other" onClick={() => setOther(other + 1)}>other {other}</button>
                        <Label />
                    </div>
                </window>
            );
        }
        "#,
    );
    let label = line_with(&ui, "label renders");
    let before = ui.runtime().renders();
    ui.click(&key("other")).unwrap();
    ui.click(&key("other")).unwrap();
    assert!(line_with(&ui, "text \"other").contains("other 2"));
    assert_eq!(line_with(&ui, "label renders"), label);
    // Each time the app, the button, which says something else, and the provider around the line they're on, which isn't the
    // app's, with the `<div>` drawn in place in the app's output (see `placeElement` in web.js): but not the label's `<span>`,
    // which a new place would draw again.
    assert_eq!(ui.runtime().renders() - before, 6);
}

/// Panes each given the whole session, as the file explorer's were before it gave them only what they use: the session's
/// `tabs` change whenever another pane gets the keyboard, and a pane only reads its own folder's view and what's pinned, in a
/// menu it makes as it renders and calls later.
const SESSION: &str = r#"
    import { useEffect, useState } from 'rsui';
    const renders = { a: 0, b: 0, c: 0 };
    let effects = 0;
    function Pane({ session, id }) {
        renders[id]++;
        const [said, setSaid] = useState('nothing');
        const own = session.views[id];
        const menu = () => setSaid('pinned ' + session.pinned.join(',') + ' at ' + session.tabs);
        return (
            <column>
                <text>{id} shows {own ? own.view : 'list'} renders {renders[id]}</text>
                <button key={'menu-' + id} onClick={menu}>menu {id}</button>
                <text>{id} said {said}</text>
            </column>
        );
    }
    function Watching(props) {
        useEffect(() => { effects++; }, [props.session]);
        return <text>effects {effects}</text>;
    }
    export default function App() {
        const [session, setSession] = useState({ tabs: 0, pinned: ['C:'], views: { a: { view: 'grid' } } });
        return (
            <window>
                <button key="focus" onClick={() => setSession({ ...session, tabs: session.tabs + 1 })}>focus</button>
                <button key="pin" onClick={() => setSession({ ...session, pinned: [...session.pinned, 'D:'] })}>pin</button>
                <button key="view" onClick={() => setSession({ ...session, views: { ...session.views, b: { view: 'details' } } })}>view</button>
                <Pane session={session} id="a" />
                <Pane session={session} id="b" />
                <Pane session={session} id="c" />
                <Watching session={session} />
            </window>
        );
    }
"#;

#[test]
fn panes_given_the_whole_session_render_only_when_what_they_read_of_it_changes() {
    let mut ui = start(SESSION);
    // The app, and what watches the session in an effect, which renders as the session is another, as it would in React.
    // (What it shows is how many times its effect ran before it rendered.)
    assert_eq!(renders_for(&mut ui, "focus"), 2);
    assert!(line_with(&ui, "effects").contains("effects 1"));
    assert!(line_with(&ui, "a shows").contains("renders 1"));
    // What's pinned is read only when the menu is used, which sees the session the app has now.
    assert_eq!(renders_for(&mut ui, "pin"), 2);
    assert!(line_with(&ui, "effects").contains("effects 2"));
    ui.click(&key("menu-b")).unwrap();
    assert!(line_with(&ui, "b said").contains("pinned C:,D: at 1"));
    // Only the pane whose own view changed renders for it.
    assert_eq!(renders_for(&mut ui, "view"), 3);
    assert!(line_with(&ui, "b shows").contains("b shows details renders 3"));
    assert!(line_with(&ui, "a shows").contains("renders 1"));
    assert!(line_with(&ui, "c shows").contains("renders 1"));
}

/// Panes laid out by splits, each drawn by a function the app makes, which sees which pane has the keyboard, as the file
/// explorer's `drawPane` is: the splits and their parts pass it down, and only what's at the bottom calls it.
const SPLITS: &str = r#"
    import { useState } from 'rsui';
    const renders = {};
    const count = (name) => { renders[name] = (renders[name] ?? 0) + 1; return renders[name]; };
    function Pane(props) {
        const times = count('pane ' + props.id);
        return <button key={'pane-' + props.id} onClick={props.onActivate}>{props.id}{props.active ? ' active' : ''} renders {times}</button>;
    }
    function Slot(props) {
        count('slot ' + props.id);
        return props.pane(props.id);
    }
    function Part(props) {
        count('part ' + props.id);
        return <column><Slot {...props} /></column>;
    }
    function Splits(props) {
        count('splits');
        return <row>{props.ids.map((id) => <Part key={id} {...props} id={id} />)}</row>;
    }
    export default function App() {
        const [focus, setFocus] = useState('a');
        const [ids] = useState(['a', 'b', 'c']);
        const drawPane = (id) => <Pane id={id} active={id === focus} onActivate={() => setFocus(id)} />;
        return (
            <window>
                <Splits ids={ids} pane={drawPane} />
                <text>splits {renders.splits}</text>
            </window>
        );
    }
"#;

#[test]
fn a_function_that_draws_each_pane_renders_only_the_panes_whose_drawing_changed() {
    let mut ui = start(SPLITS);
    // The app, the two slots whose function now gives something else, and their panes: not the splits, their parts, or the
    // third slot.
    assert_eq!(renders_for(&mut ui, "pane-b"), 5);
    assert!(line_with(&ui, "pane-b").contains("b active renders 2"));
    assert!(line_with(&ui, "pane-a").contains("a renders 2"));
    assert!(line_with(&ui, "pane-c").contains("c renders 1"));
    assert!(line_with(&ui, "splits").contains("splits 1"));
    // The panes' handlers, which the splits kept from the first render, still set what they should.
    assert_eq!(renders_for(&mut ui, "pane-c"), 5);
    assert!(line_with(&ui, "pane-c").contains("c active renders 2"));
    assert!(!line_with(&ui, "pane-b").contains("active"));
}

/// The file explorer's panes as they are: page elements around nested splits, each split's parts spreading what they were
/// given into what's inside them, panes added one at a time, and a function that draws each pane, which sees which pane has
/// the keyboard and hands each pane the app's handlers.
const PAGE_SPLITS: &str = r#"
    import { useRef, useState } from 'rsui';
    const renders = {};
    const count = (name) => { renders[name] = (renders[name] ?? 0) + 1; return renders[name]; };
    function Pane(props) {
        const times = count('pane ' + props.id);
        return (
            <div className="pane">
                <button key={'pane-' + props.id} onClick={() => props.onActivate(props.id)}>
                    {props.id}{props.active ? ' active' : ''}{props.marked ? ' marked' : ''} renders {times}
                </button>
            </div>
        );
    }
    function Slot(props) {
        return <div className="slot">{props.children}</div>;
    }
    function Part(props) {
        const { share, children } = props;
        return <div className="part" style={{ flexGrow: share }}>{children}</div>;
    }
    function Inside(props) {
        const { node } = props;
        if (node.kind === 'pane') {
            return props.pane(node.id);
        }
        return node.children.map((child) => (
            <Part key={child.id} share={1} grows={child.kind === 'pane' && props.fresh(child.id)}>
                <Inside {...props} node={child} />
            </Part>
        ));
    }
    function Splits(props) {
        const { layout } = props;
        return (
            <div className="splits">
                <Inside {...props} node={layout} />
            </div>
        );
    }
    export default function App() {
        const [view, setView] = useState({ panes: [{ id: 'a' }], focus: 'a', layout: { kind: 'split', id: 's', children: [{ kind: 'pane', id: 'a' }] } });
        const fresh = useRef(new Set());
        const viewFocus = view.focus;
        const on = {
            activate: (id) => setView((old) => ({ ...old, focus: id })),
            split: () => {
                const id = String.fromCharCode(97 + view.panes.length);
                setView({ panes: [...view.panes, { id }], focus: id, layout: { ...view.layout, children: [...view.layout.children, { kind: 'pane', id }] } });
            },
        };
        const drawPane = (id) => {
            const state = view.panes.find((one) => one.id === id);
            if (!state) {
                return null;
            }
            return (
                <Slot fresh={fresh.current.has(state.id)}>
                    <Pane id={state.id} active={state.id === viewFocus} marked={view.panes.length > 1 && state.id === viewFocus} onActivate={on.activate} />
                </Slot>
            );
        };
        return (
            <window>
                <div className="root">
                    <div className="bar">
                        <button key="split" onClick={on.split}>split</button>
                    </div>
                    <div className="panes">
                        <Splits layout={view.layout} pane={drawPane} fresh={(id) => fresh.current.has(id)} onStart={() => on.split()} />
                    </div>
                </div>
            </window>
        );
    }
"#;

/// What pane `id` shows, as in `a active marked renders 2`.
fn pane_shows(ui: &Headless, id: &str) -> String {
    line_with(ui, &format!("text \"{id} "))
}

#[test]
fn moving_the_keyboard_between_panes_in_page_splits_renders_the_two_it_moved_between() {
    let mut ui = start(PAGE_SPLITS);
    ui.click(&key("split")).unwrap();
    ui.click(&key("split")).unwrap();
    assert!(pane_shows(&ui, "c").contains("c active marked"), "{}", ui.outline());
    // Clicking a pane that doesn't have the keyboard gives it the keyboard: only that pane, and the one that had it, render,
    // with what calls the function that draws each, the slots, and what the page puts around each of their `<div>`s, which
    // all show something else, and the app: 12, not 31, as nothing passing the function down renders, nor the third pane, and
    // the `<div>`s are drawn in place in what each pane gives back (see `placeElement` in web.js).
    let before = ui.runtime().renders();
    ui.click(&key("pane-a")).unwrap();
    assert!(pane_shows(&ui, "a").contains("a active marked"), "{}", ui.outline());
    assert!(!pane_shows(&ui, "c").contains("active"), "{}", ui.outline());
    let b = pane_shows(&ui, "b");
    assert_eq!(ui.runtime().renders() - before, 12, "{}", ui.outline());
    // Again and again, from what each has been handed since.
    ui.click(&key("pane-b")).unwrap();
    assert!(pane_shows(&ui, "b").contains("b active marked"), "{}", ui.outline());
    assert!(!pane_shows(&ui, "a").contains("active"), "{}", ui.outline());
    ui.click(&key("pane-c")).unwrap();
    assert!(pane_shows(&ui, "c").contains("c active marked"), "{}", ui.outline());
    assert!(!pane_shows(&ui, "b").contains("active"), "{}", ui.outline());
    assert!(b.contains("renders 2"), "{b}");
}

/// A list given what's picked, which each of many rows compares its item with, as a file list compares each file with the
/// selection: what it read is kept in a table once there's much of it, and it still renders exactly when what it compared
/// comes out otherwise, and not when only something it didn't read changed.
const COMPARED_ROWS: &str = r#"
    import { useState } from 'rsui';
    function List(props) {
        return (
            <column>
                {props.items.map((item) => (
                    <text key={'row' + item.id}>{item.id === props.selected ? 'item ' + item.id + ' (picked)' : 'item ' + item.id}</text>
                ))}
            </column>
        );
    }
    export default function App() {
        const [selected, setSelected] = useState(3);
        const [other, setOther] = useState(0);
        const [items] = useState(() => Array.from({ length: 60 }, (_, id) => ({ id })));
        return (
            <window>
                <button key="next" onClick={() => setSelected(selected + 20)}>next</button>
                <button key="other" onClick={() => setOther(other + 1)}>{'other ' + other}</button>
                <List items={items} selected={selected} unread={other} />
            </window>
        );
    }
"#;

#[test]
fn a_list_comparing_many_items_with_what_is_picked_renders_when_that_moves() {
    let mut ui = start(COMPARED_ROWS);
    assert!(ui.outline().contains("item 3 (picked)"), "{}", ui.outline());
    ui.click(&key("next")).unwrap();
    assert!(ui.outline().contains("item 23 (picked)"), "{}", ui.outline());
    assert!(!ui.outline().contains("item 3 (picked)"), "{}", ui.outline());
    ui.click(&key("next")).unwrap();
    assert!(ui.outline().contains("item 43 (picked)"), "{}", ui.outline());
    // Only what it didn't read changed, so the list doesn't render: the button does, with the app.
    let before = ui.runtime().renders();
    ui.click(&key("other")).unwrap();
    assert!(ui.outline().contains("other 1"), "{}", ui.outline());
    assert_eq!(ui.runtime().renders() - before, 1, "only the app renders\n{}", ui.outline());
    assert!(ui.outline().contains("item 43 (picked)"), "{}", ui.outline());
}
