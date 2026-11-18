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
    // The child renders once what its `step` sees has changed, which isn't a handler's name, so it's given the new one.
    assert_eq!(renders_for(&mut ui, "child"), 2);
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
    // Each time the app, the `<div>`, the button, which says something else, and three providers around them that aren't the
    // app's: but not the label's `<span>`, which a new place would draw again.
    assert_eq!(ui.runtime().renders() - before, 12);
}
