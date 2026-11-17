//! Real apps in real TSX files, run with no window, and checked through the tree an agent would see.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use rsui::core::ActionKind;
use rsui::prelude::*;
use rsui_headless::{Headless, Query};

/// A new file for each test, as tests run side by side.
fn write_app(source: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "tsx-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("app.tsx");
    std::fs::write(&path, source).unwrap();
    path
}

const STACK_LIMIT: usize = 512 * 1024;

fn start(source: &str) -> Headless {
    let entry = rsui_tsx::load(&write_app(source), STACK_LIMIT).unwrap();
    Headless::new(rsui_tsx::root, entry)
}

fn key(name: &str) -> Query {
    Query::new().key(name)
}

fn outline_of(ui: &Headless) -> String {
    ui.outline()
}

#[test]
fn a_component_renders_and_a_click_changes_its_state() {
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        export default function App() {
            const [count, setCount] = useState(0);
            return <window><button key="add" onClick={() => setCount(count + 1)}>Clicked {count} times</button></window>;
        }
        "#,
    );
    assert!(outline_of(&ui).contains(r#"button "Clicked 0 times" #add"#));
    ui.click(&key("add")).unwrap();
    ui.click(&key("add")).unwrap();
    assert!(outline_of(&ui).contains(r#"button "Clicked 2 times" #add"#));
}

#[test]
fn only_the_component_that_changed_renders() {
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        function Other() { return <text>Other</text>; }
        function Counter() {
            const [count, setCount] = useState(0);
            return <button key="add" onClick={() => setCount(count + 1)}>{count}</button>;
        }
        export default function App() {
            return <window><Other /><Other /><Other /><Counter /></window>;
        }
        "#,
    );
    let before = ui.runtime().renders();
    ui.click(&key("add")).unwrap();
    assert_eq!(ui.runtime().renders() - before, 1);
}

#[test]
fn a_child_with_the_same_props_does_not_render_again() {
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        let renders = 0;
        function Child({ label }) {
            renders++;
            return <text>{label} {renders}</text>;
        }
        export default function App() {
            const [count, setCount] = useState(0);
            return (
                <window>
                    <button key="add" onClick={() => setCount(count + 1)}>{count}</button>
                    <Child label="same" />
                    <Child label={'count ' + count} />
                </window>
            );
        }
        "#,
    );
    assert!(outline_of(&ui).contains(r#"text "same 1""#));
    ui.click(&key("add")).unwrap();
    let outline = outline_of(&ui);
    // The first child kept what it had, while the second one rendered again, as the third render overall.
    assert!(outline.contains(r#"text "same 1""#), "{outline}");
    assert!(outline.contains(r#"text "count 1 3""#), "{outline}");
}

#[test]
fn hooks_work_as_they_do_in_react() {
    let mut ui = start(
        r#"
        import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'rsui';
        const log = [];
        function Child() {
            useEffect(() => {
                log.push('child mounted');
                return () => log.push('child unmounted');
            }, []);
            return <text>child</text>;
        }
        function reducer(state, action) { return action === 'up' ? state + 10 : state; }
        export default function App() {
            const [count, setCount] = useState(() => 1);
            const [total, dispatch] = useReducer(reducer, 5);
            const renders = useRef(0);
            renders.current++;
            const doubled = useMemo(() => { log.push('memo'); return count * 2; }, [count]);
            const stable = useCallback(() => 1, []);
            const first = useRef(stable);
            useEffect(() => {
                log.push('effect ' + count);
                return () => log.push('cleanup ' + count);
            }, [count]);
            return (
                <window>
                    <button key="add" onClick={() => setCount((old) => old + 1)}>add</button>
                    <button key="same" onClick={() => setCount(count)}>same</button>
                    <button key="up" onClick={() => dispatch('up')}>up</button>
                    <text>values {count},{doubled},{total},{renders.current},{first.current === stable ? 'same' : 'new'}</text>
                    {count < 3 ? <Child /> : null}
                    <text>log {log.join(';')}</text>
                </window>
            );
        }
        "#,
    );
    // Setting state to what it already is doesn't render anything.
    ui.click(&key("same")).unwrap();
    ui.click(&key("add")).unwrap();
    ui.click(&key("up")).unwrap();
    ui.click(&key("add")).unwrap();
    // Render it once more, for the log to have everything in it.
    ui.click(&key("up")).unwrap();
    let values = text_with(&ui, "values").unwrap();
    assert!(values.contains("values 3,6,25,5,same"), "{values}");
    let log = text_with(&ui, "log ").unwrap();
    assert!(
        log.contains(
            "log memo;child mounted;effect 1;memo;cleanup 1;effect 2;memo;child unmounted;cleanup 2;effect 3"
        ),
        "{log}"
    );
}

fn text_with(ui: &Headless, part: &str) -> Option<String> {
    outline_of(ui)
        .lines()
        .find(|line| line.contains(part))
        .map(str::to_owned)
}

#[test]
fn effects_run_after_rendering_and_clean_up_in_order() {
    let mut ui = start(
        r#"
        import { useEffect, useState } from 'rsui';
        const log = [];
        function Child() {
            useEffect(() => {
                log.push('child on');
                return () => log.push('child off');
            }, []);
            return <text>child</text>;
        }
        export default function App() {
            const [count, setCount] = useState(0);
            const [, rerender] = useState(0);
            useEffect(() => {
                log.push('run ' + count);
                return () => log.push('clean ' + count);
            }, [count]);
            return (
                <window>
                    <button key="add" onClick={() => setCount(count + 1)}>add</button>
                    <button key="again" onClick={() => rerender((n) => n + 1)}>again</button>
                    {count === 0 ? <Child /> : null}
                    <text>log: {log.join(', ')}</text>
                </window>
            );
        }
        "#,
    );
    ui.click(&key("again")).unwrap();
    assert!(
        text_with(&ui, "log:").unwrap().contains("child on, run 0"),
        "{}",
        outline_of(&ui)
    );
    ui.click(&key("add")).unwrap();
    ui.click(&key("again")).unwrap();
    let log = text_with(&ui, "log:").unwrap();
    assert!(log.contains("child off"), "{log}");
    assert!(log.contains("clean 0, run 1"), "{log}");
}

#[test]
fn context_reaches_what_is_below_and_changes_tell_it_to_render() {
    let mut ui = start(
        r#"
        import { createContext, useContext, useState } from 'rsui';
        const Theme = createContext('fallback');
        function Deep() {
            return <text>theme {useContext(Theme)}</text>;
        }
        function Between() {
            // This has no reason to render again, but what's inside it has to.
            return <group><Deep /></group>;
        }
        export default function App() {
            const [theme, setTheme] = useState('light');
            return (
                <window>
                    <button key="change" onClick={() => setTheme('dark')}>change</button>
                    <Theme.Provider value={theme}><Between /></Theme.Provider>
                    <Deep />
                </window>
            );
        }
        "#,
    );
    assert!(outline_of(&ui).contains(r#"text "theme light""#));
    assert!(outline_of(&ui).contains(r#"text "theme fallback""#));
    ui.click(&key("change")).unwrap();
    assert!(outline_of(&ui).contains(r#"text "theme dark""#));
    assert!(outline_of(&ui).contains(r#"text "theme fallback""#));
}

#[test]
fn keys_keep_state_with_what_it_belongs_to() {
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        function Item({ id }) {
            const [clicks, setClicks] = useState(0);
            return <button key={'b' + id} onClick={() => setClicks(clicks + 1)}>{id}: {clicks}</button>;
        }
        export default function App() {
            const [order, setOrder] = useState(['a', 'b', 'c']);
            return (
                <window>
                    <button key="flip" onClick={() => setOrder([...order].reverse())}>flip</button>
                    {order.map((id) => <Item key={id} id={id} />)}
                </window>
            );
        }
        "#,
    );
    ui.click(&Query::new().name("a: 0")).unwrap();
    ui.click(&key("flip")).unwrap();
    assert!(outline_of(&ui).contains(r#""a: 1""#), "{}", outline_of(&ui));
    assert!(ui.find(&Query::new().name("a: 1")).is_ok());
    let names: Vec<String> = ui.root().children[1..]
        .iter()
        .map(|child| child.name.to_string())
        .collect();
    assert_eq!(names, ["c: 0", "b: 0", "a: 1"]);
}

#[test]
fn another_kind_of_component_in_the_same_place_starts_again() {
    let mut ui = start(
        r#"
        import { useEffect, useState } from 'rsui';
        const log = [];
        function First() {
            const [count, setCount] = useState(0);
            useEffect(() => () => log.push('first gone'), []);
            return <button key="first" onClick={() => setCount(count + 1)}>first {count}</button>;
        }
        function Second() {
            const [count] = useState(100);
            return <text>second {count} {log.join()}</text>;
        }
        export default function App() {
            const [swapped, setSwapped] = useState(false);
            return (
                <window>
                    <button key="swap" onClick={() => setSwapped(true)}>swap</button>
                    {swapped ? <Second /> : <First />}
                </window>
            );
        }
        "#,
    );
    ui.click(&key("first")).unwrap();
    assert!(outline_of(&ui).contains(r#""first 1""#));
    ui.click(&key("swap")).unwrap();
    assert!(
        outline_of(&ui).contains(r#""second 100"#),
        "{}",
        outline_of(&ui)
    );
}

#[test]
fn timers_and_promises_reach_the_screen() {
    let mut ui = start(
        r#"
        import { useEffect, useState } from 'rsui';
        export default function App() {
            const [timer, setTimer] = useState('waiting');
            const [promised, setPromised] = useState('waiting');
            const [awaited, setAwaited] = useState('waiting');
            useEffect(() => {
                const id = setTimeout(() => setTimer('fired'), 10);
                Promise.resolve('resolved').then(setPromised);
                (async () => {
                    await new Promise((done) => setTimeout(done, 20));
                    setAwaited('awaited');
                })();
                return () => clearTimeout(id);
            }, []);
            return <window><text>{timer}</text><text>{promised}</text><text>{awaited}</text></window>;
        }
        "#,
    );
    let (_, settled) = ui.settle(Duration::from_secs(5));
    assert!(settled);
    let outline = outline_of(&ui);
    assert!(outline.contains(r#""fired""#), "{outline}");
    assert!(outline.contains(r#""resolved""#), "{outline}");
    assert!(outline.contains(r#""awaited""#), "{outline}");
}

#[test]
fn inputs_take_what_is_typed() {
    let mut ui = start(
        r#"
        import { TextInput, useState } from 'rsui';
        export default function App() {
            const [value, setValue] = useState('start');
            const [key, setKey] = useState('none');
            return (
                <window onKeyPress={setKey}>
                    <TextInput key="field" name="Field" value={value} onChange={setValue} />
                    <text>{value.toUpperCase()}</text>
                    <text>key {key}</text>
                </window>
            );
        }
        "#,
    );
    ui.set_value(&key("field"), "changed").unwrap();
    assert!(outline_of(&ui).contains(r#""CHANGED""#));
    assert!(outline_of(&ui).contains(r#"value="changed""#));
}

#[test]
fn numbers_and_conditions_as_children_do_what_jsx_says() {
    let ui = start(
        r#"
        export default function App() {
            const items = [1, 2];
            return (
                <window>
                    <group>
                        some text
                        {false}{null}{undefined}{true}
                        {items.map((item) => <text key={item}>item {item}</text>)}
                        <>
                            <text>in a fragment</text>
                            {[<text key="nested">in an array</text>]}
                        </>
                        {0}
                    </group>
                </window>
            );
        }
        "#,
    );
    let names: Vec<String> = ui.root().children[0]
        .children
        .iter()
        .map(|child| child.name.to_string())
        .collect();
    assert_eq!(
        names,
        [
            "some text",
            "item 1",
            "item 2",
            "in a fragment",
            "in an array",
            "0"
        ]
    );
}

#[test]
fn style_props_become_what_layout_and_paint_read() {
    let ui = start(
        r##"
        const shared = { gap: 4, background: '#102030' };
        export default function App() {
            return (
                <window>
                    <row key="a" style={shared} padding={2} width="50%" flex={1} radius={3} color={0xFF0000} align="center" />
                </window>
            );
        }
        "##,
    );
    let row = &ui.root().children[0];
    let number = |name: &str| match row.props.get(name) {
        Some(PropValue::Float(value)) => *value,
        other => panic!("{name} is {other:?}"),
    };
    assert_eq!(number("gap"), 4.0);
    assert_eq!(number("padding_left"), 2.0);
    assert_eq!(number("padding_bottom"), 2.0);
    assert_eq!(number("width_percent"), 50.0);
    assert_eq!(number("grow"), 1.0);
    assert_eq!(number("corner_radius"), 3.0);
    assert_eq!(row.props.get("align"), Some(&PropValue::from("center")));
    assert_eq!(row.props.get("direction"), Some(&PropValue::from("row")));
    assert_eq!(
        row.props.get("background"),
        Some(&PropValue::Int(Color::hex(0x102030).to_prop()))
    );
    assert_eq!(
        row.props.get("text_color"),
        Some(&PropValue::Int(Color::hex(0xFF0000).to_prop()))
    );
}

#[test]
fn something_that_renders_nothing_stays_mounted_and_takes_no_room() {
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        function Maybe() {
            const [shown, setShown] = useState(false);
            globalThis.show = () => setShown(true);
            return shown ? <text>now shown</text> : null;
        }
        export default function App() {
            return <window><button key="go" onClick={() => globalThis.show()}>go</button><Maybe /></window>;
        }
        "#,
    );
    let empty = &ui.root().children[1];
    assert!(empty.props.get("overlay").is_some());
    ui.click(&key("go")).unwrap();
    assert!(outline_of(&ui).contains(r#""now shown""#));
}

#[test]
fn a_dialog_opens_and_closes_when_dismissed() {
    let mut ui = start(
        r#"
        import { Dialog, useState } from 'rsui';
        export default function App() {
            const [open, setOpen] = useState(false);
            return (
                <window>
                    <button key="open" onClick={() => setOpen(true)}>open</button>
                    <Dialog id="about" name="About" open={open} onClose={() => setOpen(false)}>
                        <text>inside</text>
                    </Dialog>
                </window>
            );
        }
        "#,
    );
    assert!(!outline_of(&ui).contains("inside"));
    ui.click(&key("open")).unwrap();
    assert!(outline_of(&ui).contains("inside"));
    ui.act(&key("about"), ActionKind::Dismiss, None).unwrap();
    assert!(!outline_of(&ui).contains("inside"));
}

#[test]
fn menus_and_selects_call_what_they_are_given() {
    let mut ui = start(
        r#"
        import { MenuButton, Select, useState } from 'rsui';
        export default function App() {
            const [said, setSaid] = useState('nothing');
            const [chosen, setChosen] = useState(0);
            return (
                <window>
                    <MenuButton id="menu" label="Menu" items={[{ label: 'Cut', onSelect: () => setSaid('cut') }]} />
                    <Select id="fruit" options={['Apple', 'Pear']} value={chosen} onChange={setChosen} />
                    <text>{said}</text>
                </window>
            );
        }
        "#,
    );
    ui.click(&key("menu-button")).unwrap();
    ui.click(&key("menu-item-0")).unwrap();
    assert!(outline_of(&ui).contains(r#""cut""#));
    assert!(!outline_of(&ui).contains("menu-item-0"));
    ui.click(&key("fruit-button")).unwrap();
    ui.click(&key("fruit-item-1")).unwrap();
    assert!(outline_of(&ui).contains(r#"button "Pear""#));
}

#[test]
fn a_long_list_only_builds_what_is_in_view_and_scrolls() {
    let mut ui = start(
        r#"
        function Row({ index }) { return <item>Row {index}</item>; }
        const renderRow = (index) => <Row index={index} />;
        export default function App() {
            return <window><list key="rows" count={1000000} visible={5} overscan={1} rowHeight={30} renderRow={renderRow} /></window>;
        }
        "#,
    );
    let list = &ui.root().children[0];
    assert_eq!(list.props.get("count"), Some(&PropValue::Int(1_000_000)));
    assert_eq!(list.children.len(), 6);
    assert_eq!(list.children[0].name, "Row 0");
    ui.act(
        &key("rows"),
        ActionKind::Scroll,
        Some(PropValue::Int(500_000)),
    )
    .unwrap();
    let list = &ui.root().children[0];
    assert_eq!(list.children[1].name, "Row 500000");
    assert_eq!(list.children.len(), 7);
}

#[test]
fn a_scrolling_area_keeps_the_state_of_what_is_inside() {
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        function Inner() {
            const [count, setCount] = useState(0);
            return <button key="inner" onClick={() => setCount(count + 1)}>{count}</button>;
        }
        export default function App() {
            return <window><scroll key="area" flex={1} gap={4}><Inner /></scroll></window>;
        }
        "#,
    );
    ui.click(&key("inner")).unwrap();
    ui.act(
        &key("area"),
        ActionKind::Scroll,
        Some(PropValue::Float(40.0)),
    )
    .unwrap();
    let area = &ui.root().children[0];
    assert_eq!(area.props.get("scroll_y"), Some(&PropValue::Float(40.0)));
    assert_eq!(area.children[0].name, "1");
}

#[test]
fn an_error_while_rendering_shows_where_it_happened_and_the_rest_carries_on() {
    let ui = start(
        r#"
        function Broken() { throw new Error('this is broken'); }
        export default function App() {
            return <window><Broken /><text>still here</text></window>;
        }
        "#,
    );
    let outline = outline_of(&ui);
    assert!(outline.contains("this is broken"), "{outline}");
    assert!(outline.contains("still here"), "{outline}");
}

#[test]
fn a_function_for_an_event_that_does_not_exist_is_an_error_that_says_which_do() {
    let ui = start(
        r#"
        export default function App() {
            return <window><button onPress={() => {}}>Press</button></window>;
        }
        "#,
    );
    let outline = outline_of(&ui);
    assert!(outline.contains("onPress"), "{outline}");
    assert!(outline.contains("onClick"), "{outline}");
}

#[test]
fn breaking_the_rules_of_hooks_is_an_error_rather_than_a_crash() {
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        export default function App() {
            const [more, setMore] = useState(false);
            if (more) { useState(1); }
            return <window><button key="go" onClick={() => setMore(true)}>go</button></window>;
        }
        "#,
    );
    ui.click(&key("go")).unwrap();
    assert!(
        outline_of(&ui).contains("same hooks"),
        "{}",
        outline_of(&ui)
    );
}

#[test]
fn an_error_in_a_handler_shows_on_top_and_the_app_carries_on() {
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        export default function App() {
            const [count, setCount] = useState(0);
            return (
                <window>
                    <button key="bad" onClick={() => { throw new Error('bad handler'); }}>bad</button>
                    <button key="good" onClick={() => setCount(count + 1)}>count {count}</button>
                </window>
            );
        }
        "#,
    );
    ui.click(&key("bad")).unwrap();
    assert!(
        outline_of(&ui).contains("bad handler"),
        "{}",
        outline_of(&ui)
    );
    ui.click(&key("good")).unwrap();
    assert!(outline_of(&ui).contains("count 1"), "{}", outline_of(&ui));
}

#[test]
fn state_stays_alive_through_a_lot_of_garbage() {
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        function Item({ value }) {
            const [held] = useState(() => ({ value, list: [value, value + 1] }));
            return <text>{held.value + held.list[1]}</text>;
        }
        export default function App() {
            const [count, setCount] = useState(0);
            const [kept] = useState(() => ({ name: 'kept', list: ['a', 'b'] }));
            let junk = [];
            for (let i = 0; i < 20000; i++) { junk.push({ i, text: 'junk ' + i }); }
            const rows = [];
            for (let i = 0; i < 50; i++) { rows.push(<Item key={i} value={i} />); }
            return (
                <window>
                    <button key="add" onClick={() => setCount(count + junk.length)}>{kept.name} {kept.list[1]} {count}</button>
                    {rows}
                </window>
            );
        }
        "#,
    );
    for _ in 0..40 {
        ui.click(&key("add")).unwrap();
    }
    assert!(
        outline_of(&ui).contains(r#"button "kept b 800000""#),
        "{}",
        outline_of(&ui)
    );
    assert!(outline_of(&ui).contains(r#"text "19""#));
}

#[test]
fn modules_can_import_each_other_and_their_own_components() {
    let path = write_app(
        r#"
        import { Label } from './label';
        export const title = 'Named';
        export default function App() { return <window><Label text="hello" /></window>; }
        "#,
    );
    std::fs::write(
        path.with_file_name("label.tsx"),
        r#"
        import { useMemo } from 'rsui';
        export function Label({ text }: { text: string }) {
            const shouted = useMemo(() => text.toUpperCase(), [text]);
            return <text>{shouted}</text>;
        }
        "#,
    )
    .unwrap();
    let entry = rsui_tsx::load(&path, STACK_LIMIT).unwrap();
    assert_eq!(entry.title().as_deref(), Some("Named"));
    let ui = Headless::new(rsui_tsx::root, entry);
    assert!(outline_of(&ui).contains(r#""HELLO""#));
}

#[test]
fn a_file_with_no_component_says_so() {
    let path = write_app("export const nothing = 1;");
    let error = rsui_tsx::load(&path, STACK_LIMIT).unwrap_err();
    assert!(error.contains("default export"), "{error}");
}

#[test]
fn a_syntax_error_says_where() {
    let path = write_app("export default function App( { return 1 }");
    let error = rsui_tsx::load(&path, STACK_LIMIT).unwrap_err();
    assert!(
        error.to_lowercase().contains("syntax") || error.contains("app.tsx"),
        "{error}"
    );
}

const VERSION_ONE: &str = r#"
    import { useState } from 'rsui';
    function Counter() {
        const [count, setCount] = useState(0);
        return <button key="add" onClick={() => setCount(count + 1)}>one {count}</button>;
    }
    export default function App() { return <window><Counter /></window>; }
"#;

#[test]
fn saving_a_new_version_swaps_it_in_and_keeps_the_state() {
    let path = write_app(VERSION_ONE);
    let entry = rsui_tsx::load(&path, STACK_LIMIT).unwrap();
    rsui_tsx::keep_state_across_reloads();
    let mut ui = Headless::new(rsui_tsx::root, entry);
    ui.click(&key("add")).unwrap();
    ui.click(&key("add")).unwrap();
    assert!(outline_of(&ui).contains(r#""one 2""#));

    std::fs::write(&path, VERSION_ONE.replace("one {count}", "two {count}")).unwrap();
    assert!(rsui_tsx::request_reload(&path));
    ui.settle(Duration::from_secs(5));
    assert!(
        outline_of(&ui).contains(r#""two 2""#),
        "{}",
        outline_of(&ui)
    );

    // It still works, with the new code.
    ui.click(&key("add")).unwrap();
    assert!(
        outline_of(&ui).contains(r#""two 3""#),
        "{}",
        outline_of(&ui)
    );
}

#[test]
fn a_new_version_with_different_hooks_starts_that_component_again() {
    let path = write_app(VERSION_ONE);
    let entry = rsui_tsx::load(&path, STACK_LIMIT).unwrap();
    rsui_tsx::keep_state_across_reloads();
    let mut ui = Headless::new(rsui_tsx::root, entry);
    ui.click(&key("add")).unwrap();
    std::fs::write(
        &path,
        VERSION_ONE
            .replace(
                "const [count, setCount] = useState(0);",
                "const [other] = useState('new'); const [count, setCount] = useState(0);",
            )
            .replace("one {count}", "{other} {count}"),
    )
    .unwrap();
    assert!(rsui_tsx::request_reload(&path));
    ui.settle(Duration::from_secs(5));
    // Each hook is still a `useState`, so the old count is what `other` now has. That's what fast refresh does too.
    assert!(outline_of(&ui).contains("button"), "{}", outline_of(&ui));

    std::fs::write(
        &path,
        VERSION_ONE
            .replace(
                "const [count, setCount] = useState(0);",
                "const count = 7; const setCount = useRef(0).current;",
            )
            .replace("{ useState }", "{ useRef }"),
    )
    .unwrap();
    assert!(rsui_tsx::request_reload(&path));
    ui.settle(Duration::from_secs(5));
    assert!(
        outline_of(&ui).contains(r#""one 7""#),
        "{}",
        outline_of(&ui)
    );
}

#[test]
fn a_new_version_with_a_mistake_in_it_leaves_the_old_one_running() {
    let path = write_app(VERSION_ONE);
    let entry = rsui_tsx::load(&path, STACK_LIMIT).unwrap();
    rsui_tsx::keep_state_across_reloads();
    let mut ui = Headless::new(rsui_tsx::root, entry);
    ui.click(&key("add")).unwrap();

    std::fs::write(&path, "export default function App( { oops").unwrap();
    assert!(rsui_tsx::request_reload(&path));
    ui.settle(Duration::from_secs(5));
    assert!(
        outline_of(&ui).contains(r#"dialog "Something went wrong""#),
        "{}",
        outline_of(&ui)
    );
    assert!(
        outline_of(&ui).contains(r#""one 1""#),
        "{}",
        outline_of(&ui)
    );

    // Fixing it takes the message away, and nothing was lost.
    std::fs::write(&path, VERSION_ONE.replace("one {count}", "fixed {count}")).unwrap();
    assert!(rsui_tsx::request_reload(&path));
    ui.settle(Duration::from_secs(5));
    assert!(
        !outline_of(&ui).contains("Something went wrong"),
        "{}",
        outline_of(&ui)
    );
    assert!(
        outline_of(&ui).contains(r#""fixed 1""#),
        "{}",
        outline_of(&ui)
    );
}

#[test]
fn an_effect_still_runs_when_a_child_renders_at_the_same_time() {
    let mut ui = start(
        r#"
        import { useEffect, useState } from 'rsui';
        const log = [];
        function Child({ n }) {
            return <text>child {n}</text>;
        }
        export default function App() {
            const [n, setN] = useState(0);
            const [, rerender] = useState(0);
            useEffect(() => {
                log.push('run ' + n);
            }, [n]);
            return (
                <window>
                    <button key="add" onClick={() => setN(n + 1)}>add</button>
                    <button key="again" onClick={() => rerender((x) => x + 1)}>again</button>
                    <Child n={n} />
                    <text>log: {log.join(', ')}</text>
                </window>
            );
        }
        "#,
    );
    ui.click(&key("add")).unwrap();
    ui.click(&key("add")).unwrap();
    ui.click(&key("again")).unwrap();
    let log = text_with(&ui, "log:").unwrap();
    assert!(log.contains("run 0, run 1, run 2"), "{log}");
}

#[test]
fn an_effect_that_never_settles_is_stopped_and_the_rest_of_the_app_carries_on() {
    let mut ui = start(
        r#"
        import { useEffect, useState } from 'rsui';
        function Runaway() {
            const [count, setCount] = useState(0);
            useEffect(() => setCount(count + 1), [count]);
            return <text>running away</text>;
        }
        function Fine() {
            const [count, setCount] = useState(0);
            useEffect(() => {
                globalThis.ranEffect = count;
            }, [count]);
            return <button key="fine" onClick={() => setCount(count + 1)}>fine {count} {globalThis.ranEffect}</button>;
        }
        export default function App() {
            return (
                <window>
                    <Runaway />
                    <Fine />
                </window>
            );
        }
        "#,
    );
    // It's said in the log, as a page's console would, and nothing covers the app.
    assert!(
        !outline_of(&ui).contains("Something went wrong"),
        "{}",
        outline_of(&ui)
    );
    assert!(outline_of(&ui).contains("running away"), "{}", outline_of(&ui));
    ui.click(&key("fine")).unwrap();
    ui.click(&key("fine")).unwrap();
    assert!(outline_of(&ui).contains("fine 2 1"), "{}", outline_of(&ui));
}

#[test]
fn a_component_whose_effects_were_stopped_can_run_them_again_later() {
    let mut ui = start(
        r#"
        import { useEffect, useState } from 'rsui';
        const log = [];
        export default function App() {
            const [loop, setLoop] = useState(0);
            const [other, setOther] = useState(0);
            useEffect(() => {
                if (loop > 0 && loop < 1000) {
                    setLoop(loop + 1);
                }
            }, [loop]);
            useEffect(() => {
                log.push(other);
            }, [other]);
            return (
                <window>
                    <button key="loop" onClick={() => setLoop(1)}>loop</button>
                    <button key="other" onClick={() => setOther(other + 1)}>other</button>
                    <text>log: {log.join(',')}</text>
                </window>
            );
        }
        "#,
    );
    ui.click(&key("loop")).unwrap();
    ui.click(&key("other")).unwrap();
    ui.click(&key("other")).unwrap();
    let log = text_with(&ui, "log:").unwrap();
    assert!(log.contains("0,1"), "{log}");
}

#[test]
fn text_is_what_shows_and_a_name_is_what_it_is_called() {
    let ui = start(
        r#"
        export default function App() {
            return (
                <window>
                    <button key="remove" name="Remove the first row" color="red">Remove</button>
                    <button key="plain">Plain</button>
                    <button key="named" name="Only a name" />
                </window>
            );
        }
        "#,
    );
    let outline = outline_of(&ui);
    assert!(
        outline.contains(r#"button "Remove the first row" #remove"#),
        "{outline}"
    );
    assert!(outline.contains(r#"text "Remove""#), "{outline}");
    assert!(outline.contains(r#"button "Plain" #plain"#), "{outline}");
    assert!(
        outline.contains(r#"button "Only a name" #named"#),
        "{outline}"
    );
    // How the text looks goes with it, whatever that takes.
    let ui = start(
        r#"
        export default function App() {
            return <window><button key="b" name="Called this" fontFamily="Consolas" fontStyle="italic" ellipsis underline textAlign="right" color="red" fontSize={9} fontWeight="bold">Shows this</button></window>;
        }
        "#,
    );
    // These are what it looks like, which an outline leaves out, so it's on the node that they're looked for.
    let found = ui.find(&Query::new().name("Shows this")).unwrap();
    for look in [
        "font_family",
        "font_style",
        "ellipsis",
        "underline",
        "text_align",
    ] {
        assert!(found.node.props.get(look).is_some(), "{look} isn't on it");
    }
}

#[test]
fn a_checkbox_says_whether_it_is_ticked_and_calls_back() {
    let mut ui = start(
        r#"
        import { Checkbox, useState } from 'rsui';
        export default function App() {
            const [on, setOn] = useState(false);
            return (
                <window>
                    <Checkbox id="tick" label="Send updates" checked={on} onChange={setOn} />
                    <text>{on ? 'yes' : 'no'}</text>
                </window>
            );
        }
        "#,
    );
    assert!(outline_of(&ui).contains(r#""no""#), "{}", outline_of(&ui));
    ui.toggle(&key("tick")).unwrap();
    assert!(outline_of(&ui).contains(r#""yes""#), "{}", outline_of(&ui));
    ui.toggle(&key("tick")).unwrap();
    assert!(outline_of(&ui).contains(r#""no""#), "{}", outline_of(&ui));
}

fn window_app(source: &str) -> rsui::App {
    let entry = rsui_tsx::load(&write_app(source), STACK_LIMIT).unwrap();
    let text = rsui::default_text_engine().unwrap();
    let mut app = rsui::App::new(rsui_tsx::root, entry, text, (600, 400), 1.0);
    app.update();
    app.scene();
    app
}

#[test]
fn focus_gives_the_keyboard_to_what_has_that_key() {
    let mut app = window_app(
        r#"
        import { focus, TextInput, useState } from 'rsui';
        export default function App() {
            const [value, setValue] = useState('');
            return (
                <window>
                    <button key="go" onClick={() => focus('field')}>go</button>
                    <button key="none" onClick={() => focus('nothing has this')}>none</button>
                    <TextInput key="field" name="Field" value={value} onChange={setValue} />
                </window>
            );
        }
        "#,
    );
    assert_eq!(app.focused(), None);
    // The button at the top.
    app.pointer_down(5.0, 5.0, 1);
    app.pointer_up(5.0, 5.0).unwrap();
    app.update();
    assert_eq!(app.focused(), Some(&[2u32][..]));
    app.type_text("hello").unwrap();
    app.update();
    assert!(
        app.outline().contains(r#"value="hello""#),
        "{}",
        app.outline()
    );
}

#[test]
fn an_input_that_asks_for_it_has_the_keyboard_when_it_first_shows() {
    let mut app = window_app(
        r#"
        import { Dialog, TextInput, useState } from 'rsui';
        export default function App() {
            const [open, setOpen] = useState(false);
            const [value, setValue] = useState('');
            return (
                <window>
                    <button key="open" onClick={() => setOpen(true)}>open</button>
                    <TextInput key="other" name="Other" />
                    {open ? <TextInput autoFocus key="wanted" name="Wanted" value={value} onChange={setValue} /> : null}
                </window>
            );
        }
        "#,
    );
    assert_eq!(app.focused(), None, "nothing's asked yet");
    app.pointer_down(5.0, 5.0, 1);
    app.pointer_up(5.0, 5.0).unwrap();
    app.update();
    app.type_text("straight in").unwrap();
    app.update();
    assert!(
        app.outline().contains(r#"value="straight in""#),
        "{}",
        app.outline()
    );
}

#[test]
fn keys_go_on_to_what_is_around_them_unless_something_says_it_dealt_with_them() {
    let mut app = window_app(
        r#"
        import { focus, TextInput, useState } from 'rsui';
        const log = [];
        export default function App() {
            const [, rerender] = useState(0);
            const note = (where) => (key) => {
                log.push(where + ' ' + key);
                rerender((n) => n + 1);
            };
            const dealWith = (key) => {
                note('inner')(key);
                return key === 'ctrl+d';
            };
            return (
                <window onKeyPress={note('window')}>
                    <group onKeyPress={dealWith}>
                        <TextInput autoFocus key="field" name="Field" />
                    </group>
                    <text>log: {log.join(', ')}</text>
                </window>
            );
        }
        "#,
    );
    app.update();
    let ctrl = |letter| rsui::KeyPress::new(rsui::KeyCode::Character(letter)).ctrl();
    app.key(ctrl('e')).unwrap();
    app.key(ctrl('d')).unwrap();
    app.update();
    app.scene();
    assert!(
        app.outline()
            .contains("log: inner ctrl+e, window ctrl+e, inner ctrl+d"),
        "{}",
        app.outline()
    );
}

#[test]
fn what_finishes_on_another_thread_reaches_the_screen() {
    let file = write_app("the file's text");
    let source = format!(
        r#"
        import {{ useEffect, useState }} from 'rsui';
        import {{ promises }} from 'fs';
        export default function App() {{
            const [text, setText] = useState('reading');
            useEffect(() => {{
                promises.readFile({file:?}, 'utf8').then(setText, (error) => setText(error.code));
            }}, []);
            return <window><text>{{text}}</text></window>;
        }}
        "#
    );
    let mut ui = start(&source);
    assert!(outline_of(&ui).contains("reading"), "{}", outline_of(&ui));
    for _ in 0..1000 {
        ui.settle(Duration::from_millis(10));
        if outline_of(&ui).contains("the file's text") {
            return;
        }
        // Settling doesn't wait for what the runtime doesn't know about, like a file being read.
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("it never showed\n{}", outline_of(&ui));
}

/// Where something with this key is, after what's happening has been applied.
fn middle_of(app: &mut rsui::App, wanted: &str) -> (f32, f32) {
    fn find(node: &rsui::core::Node, wanted: &str, path: &mut Vec<u32>) -> bool {
        if node
            .key
            .as_ref()
            .is_some_and(|key| key.to_string() == wanted)
        {
            return true;
        }
        for (index, child) in node.children.iter().enumerate() {
            path.push(index as u32);
            if find(child, wanted, path) {
                return true;
            }
            path.pop();
        }
        false
    }
    app.update();
    app.scene();
    let mut path = Vec::new();
    assert!(
        find(app.runtime().root(), wanted, &mut path),
        "{}",
        app.outline()
    );
    let rect = app.layout().rect(&path).unwrap();
    (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
}

const POINTER_APP: &str = r#"
    import { useState } from 'rsui';
    const log = [];
    export default function App() {
        const [, rerender] = useState(0);
        const note = (text) => {
            log.push(text);
            rerender((n) => n + 1);
        };
        return (
            <window>
                <button key="target" width={100} height={40}
                    onClick={() => note('click')}
                    onDoubleClick={() => note('double')}
                    onContextMenu={(x, y) => note('menu ' + x + ' ' + y)}>
                    target
                </button>
                <group key="area" width={200} height={100}
                    onDrag={(phase, x, y, dx, dy) => note(phase + ' ' + x + ' ' + y + ' ' + dx + ' ' + dy)} />
                <text>log: {log.join(', ')}</text>
            </window>
        );
    }
"#;

#[test]
fn a_double_click_is_two_clicks_and_then_a_double_click() {
    let mut app = window_app(POINTER_APP);
    let (x, y) = middle_of(&mut app, "target");
    app.pointer_down(x, y, 1);
    app.pointer_up(x, y).unwrap();
    app.pointer_down(x, y, 2);
    app.pointer_up(x, y).unwrap();
    app.update();
    assert!(
        app.outline().contains("log: click, click, double"),
        "{}",
        app.outline()
    );
}

#[test]
fn the_other_button_says_where_it_was_clicked() {
    let mut app = window_app(POINTER_APP);
    let (x, y) = middle_of(&mut app, "target");
    app.secondary_click(x, y).unwrap();
    app.update();
    assert!(
        app.outline().contains(&format!("log: menu {x} {y}")),
        "{}",
        app.outline()
    );
    // Somewhere nothing's listening is nothing.
    let (x, y) = middle_of(&mut app, "area");
    app.secondary_click(x, y).unwrap();
    app.update();
    assert!(
        !app.outline().contains(&format!("menu {x}")),
        "{}",
        app.outline()
    );
}

#[test]
fn dragging_says_where_the_pointer_is_from_start_to_end() {
    let mut app = window_app(POINTER_APP);
    let (x, y) = middle_of(&mut app, "area");
    app.pointer_down(x, y, 1);
    app.pointer_move(x + 10.0, y + 5.0);
    app.pointer_move(x + 10.0, y + 5.0);
    app.pointer_move(x + 30.0, y + 5.0);
    app.pointer_up(x + 30.0, y + 5.0).unwrap();
    // After it's let go of, moving is only moving.
    app.pointer_move(x + 50.0, y);
    app.update();
    let outline = app.outline();
    assert!(
        outline
            .contains("log: start 100 50 0 0, move 110 55 10 5, move 130 55 20 0, end 130 55 0 0"),
        "{outline}"
    );
}

#[test]
fn a_slider_follows_dragging_and_clicking_and_the_arrow_keys() {
    let mut app = window_app(
        r#"
        import { Slider, useState } from 'rsui';
        export default function App() {
            const [value, setValue] = useState(0);
            return (
                <window>
                    <Slider id="volume" value={value} onChange={setValue} width={216} />
                    <text>value {value}</text>
                </window>
            );
        }
        "#,
    );
    let (x, y) = middle_of(&mut app, "volume");
    let says = |app: &rsui::App, value: i32| {
        assert!(
            app.outline().contains(&format!("value {value}\"")),
            "{value}\n{}",
            app.outline()
        );
    };
    says(&app, 0);
    // The middle of it is half way.
    app.pointer_down(x, y, 1);
    app.update();
    says(&app, 50);
    // On past the end stops at the end, and it carries on hearing though the pointer's left it.
    app.pointer_move(x + 500.0, y + 200.0);
    app.update();
    says(&app, 100);
    app.pointer_move(x - 500.0, y);
    app.update();
    says(&app, 0);
    app.pointer_up(x - 500.0, y).unwrap();
    app.update();
    // It has the keyboard from being clicked.
    app.key(rsui::KeyPress::new(rsui::KeyCode::Right)).unwrap();
    app.update();
    says(&app, 1);
    app.key(rsui::KeyPress::new(rsui::KeyCode::End)).unwrap();
    app.update();
    says(&app, 100);
    app.key(rsui::KeyPress::new(rsui::KeyCode::Right)).unwrap();
    app.update();
    says(&app, 100);
}

#[test]
fn the_clipboard_can_be_written_and_read_and_the_window_renamed() {
    let mut app = window_app(
        r#"
        import { mainWindow, useState } from 'rsui';
        import { clipboard } from 'rsui/desktop';
        export default function App() {
            const [read, setRead] = useState('nothing');
            return (
                <window>
                    <button key="copy" onClick={() => clipboard.write({ text: 'from the app' })}>copy</button>
                    <button key="paste" onClick={() => clipboard.read().then((got) => setRead(got.text))}>paste</button>
                    <button key="name" onClick={() => mainWindow.setTitle('A new name')}>name</button>
                    <text>read {read}</text>
                </window>
            );
        }
        "#,
    );
    let press = |app: &mut rsui::App, key: &str| {
        let (x, y) = middle_of(app, key);
        app.pointer_down(x, y, 1);
        app.pointer_up(x, y).unwrap();
        app.update();
    };
    assert_eq!(app.take_title_wanted(), None);
    press(&mut app, "name");
    assert_eq!(app.take_title_wanted().as_deref(), Some("A new name"));
    press(&mut app, "copy");
    press(&mut app, "paste");
    assert!(
        app.outline().contains("read from the app"),
        "{}",
        app.outline()
    );
}

#[test]
fn a_click_on_rsuis_own_elements_says_where_it_was_which_button_and_what_was_held() {
    let mut app = window_app(
        r#"
        import { useState } from 'rsui';
        export default function App() {
            const [said, setSaid] = useState('nothing');
            return (
                <window padding={10}>
                    <button key="pick" width={100} height={30}
                        onClick={(e) => setSaid([e.clientX, e.clientY, e.offsetX, e.button, e.ctrlKey, e.shiftKey, e.detail].join(' '))}>Pick</button>
                    <text>{'said ' + said}</text>
                </window>
            );
        }
        "#,
    );
    let (x, y) = middle_of(&mut app, "pick");
    app.set_modifiers(rsui::Modifiers { ctrl: true, ..Default::default() });
    app.pointer_down(x, y, 1);
    app.pointer_up(x, y).unwrap();
    app.update();
    let wanted = format!("said {x} {y} {} 0 true false 1", x - 10.0);
    assert!(app.outline().contains(&wanted), "{wanted}\n{}", app.outline());
    // With the keyboard, it's in the middle, and nothing was clicked.
    app.set_modifiers(rsui::Modifiers::default());
    app.focus_on(&[0]);
    app.key(rsui::KeyPress::new(rsui::KeyCode::Enter)).unwrap();
    app.update();
    assert!(app.outline().contains(" 0 false false 0"), "{}", app.outline());
}

#[test]
fn shortcuts_hear_function_keys_and_alt() {
    let mut app = window_app(
        r#"
        import { useState } from 'rsui';
        export default function App() {
            const [keys, setKeys] = useState([]);
            return (
                <window key="root" onKeyPress={(key) => setKeys((was) => [...was, key])}>
                    <text>{'keys ' + keys.join(' ')}</text>
                </window>
            );
        }
        "#,
    );
    for press in [
        rsui::KeyPress::new(rsui::KeyCode::Function(2)),
        rsui::KeyPress::new(rsui::KeyCode::Character('s')).alt(),
        rsui::KeyPress::new(rsui::KeyCode::Character('k')).ctrl().meta(),
        // Only held along with the rest, so it isn't a shortcut by itself.
        rsui::KeyPress::new(rsui::KeyCode::Shift),
    ] {
        app.key(press).unwrap();
        app.update();
    }
    assert!(app.outline().contains("keys f2 alt+s ctrl+win+k\""), "{}", app.outline());
}

#[test]
fn rsuis_own_elements_hear_events_as_the_web_has_them() {
    let mut app = window_app(
        r#"
        import { useState, TextInput } from 'rsui';
        export default function App() {
            const [log, setLog] = useState([]);
            const [value, setValue] = useState('');
            const note = (said) => setLog((was) => [...was, said]);
            return (
                <window key="root" padding={10} gap={10} align="start">
                    <row key="track" width={200} height={20} background={0xcccccc}
                        onPointerDown={(e) => note('down ' + e.offsetX + ' ' + e.buttons)}
                        onPointerMove={(e) => e.buttons === 1 && note('move ' + e.offsetX)}
                        onPointerUp={(e) => note('up ' + e.offsetX)}
                        onMouseEnter={() => note('enter')} onMouseLeave={(e) => note('leave ' + e.type)} />
                    <row key="list" width={200} height={30} onClickCapture={() => note('list first')} onClick={(e) => note('list ' + e.ctrlKey)}>
                        <button key="stays" width={60} height={30} onClick={() => note('stays')}>Stays</button>
                        <button key="passes" width={60} height={30} onClick={(e) => { note('passes'); e.passOn(); }}>Passes</button>
                    </row>
                    <input key="field" width={200} height={30} value={value} onChange={setValue}
                        onKeyDown={(e) => { if (e.key === 'x') { e.preventDefault(); note('no x'); } }}
                        onKeyUp={(e) => note('up ' + e.code)} onBlur={() => note('blur')}
                        onPaste={(e) => { note('paste ' + e.clipboardData.getData('text')); e.preventDefault(); }} />
                    <text key="away">{'log ' + log.join(', ')}</text>
                </window>
            );
        }
        "#,
    );
    let rect = |app: &mut rsui::App, key: &str| {
        let (x, y) = middle_of(app, key);
        (x, y)
    };
    let (_, y) = rect(&mut app, "track");
    // The track starts 10 in, at the padding.
    app.pointer_move(20.0, y);
    app.pointer_down(20.0, y, 1);
    app.pointer_move(60.0, y);
    app.pointer_move(300.0, y + 200.0);
    app.pointer_up(300.0, y + 200.0).unwrap();
    app.update();
    for (key, held) in [("stays", false), ("passes", true)] {
        let (x, y) = rect(&mut app, key);
        app.set_modifiers(rsui::Modifiers { ctrl: held, ..Default::default() });
        app.pointer_move(x, y);
        app.pointer_down(x, y, 1);
        app.pointer_up(x, y).unwrap();
        app.update();
    }
    app.set_modifiers(rsui::Modifiers::default());
    let (x, y) = rect(&mut app, "field");
    app.pointer_move(x, y);
    app.pointer_down(x, y, 1);
    app.pointer_up(x, y).unwrap();
    app.set_clipboard(rsui::MemoryClipboard::default());
    for letter in ['a', 'x', 'b'] {
        app.key(rsui::KeyPress::new(rsui::KeyCode::Character(letter))).unwrap();
        app.type_text(&letter.to_string()).unwrap();
        app.key_up(rsui::KeyPress::new(rsui::KeyCode::Character(letter))).unwrap();
        app.update();
    }
    app.key(rsui::KeyPress::new(rsui::KeyCode::Character('a')).ctrl()).unwrap();
    app.key(rsui::KeyPress::new(rsui::KeyCode::Character('c')).ctrl()).unwrap();
    app.key(rsui::KeyPress::new(rsui::KeyCode::Character('v')).ctrl()).unwrap();
    app.update();
    let (x, y) = rect(&mut app, "away");
    app.pointer_down(x, y, 1);
    app.pointer_up(x, y).unwrap();
    app.update();
    let outline = app.outline();
    for said in [
        // It's left while it's still held, and the moves still come to it.
        "enter, down 10 1, move 50, leave mouseleave, move 290, up 290",
        "list first, stays, list first, passes, list true",
        "up KeyA, no x, up KeyX, up KeyB",
        "paste ab, blur",
    ] {
        assert!(outline.contains(said), "{said}\n{outline}");
    }
    assert!(outline.contains("value=\"ab\""), "{outline}");
}

#[test]
fn a_handler_for_something_that_never_happens_is_still_an_error_that_says_what_does() {
    let entry = rsui_tsx::load(
        &write_app(
            r#"
            export default function App() {
                return <window><row onTouchStart={() => {}} /></window>;
            }
            "#,
        ),
        STACK_LIMIT,
    )
    .unwrap();
    let ui = Headless::new(rsui_tsx::root, entry);
    let outline = ui.outline();
    assert!(outline.contains("onTouchStart isn't something that happens"), "{outline}");
    assert!(outline.contains("onPointerDown"), "{outline}");
}

#[test]
fn rsuis_own_elements_can_be_dragged_and_dropped_on() {
    let mut app = window_app(
        r#"
        import { useState } from 'rsui';
        export default function App() {
            const [said, setSaid] = useState('nothing');
            return (
                <window padding={10} gap={10} align="start">
                    <row key="card" width={80} height={30} draggable onDragStart={(e) => e.dataTransfer.setData('text', 'the card')} />
                    <row key="zone" width={200} height={60} onDragOver={(e) => e.preventDefault()} onDrop={(e) => setSaid(e.dataTransfer.getData('text'))} />
                    <text>{'said ' + said}</text>
                </window>
            );
        }
        "#,
    );
    let (from_x, from_y) = middle_of(&mut app, "card");
    let (to_x, to_y) = middle_of(&mut app, "zone");
    app.pointer_move(from_x, from_y);
    app.pointer_down(from_x, from_y, 1);
    for step in 1..=4 {
        let along = step as f32 / 4.0;
        app.pointer_move(from_x + (to_x - from_x) * along, from_y + (to_y - from_y) * along);
    }
    app.pointer_up(to_x, to_y).unwrap();
    app.update();
    assert!(app.outline().contains("said the card"), "{}", app.outline());
}

#[test]
fn an_app_that_waits_at_its_top_starts_once_it_has() {
    let mut ui = start(
        r#"
        import { useState } from 'rsui';
        const settings = await new Promise((done) => setTimeout(() => done({ greeting: 'Loaded' }), 20));
        export const title = settings.greeting;
        export default function App() {
            const [count, setCount] = useState(0);
            return <window><button key="add" onClick={() => setCount(count + 1)}>{settings.greeting} {count}</button></window>;
        }
        "#,
    );
    assert!(outline_of(&ui).contains(r#"button "Loaded 0" #add"#), "{}", outline_of(&ui));
    ui.click(&key("add")).unwrap();
    assert!(outline_of(&ui).contains(r#"button "Loaded 1" #add"#));
}
