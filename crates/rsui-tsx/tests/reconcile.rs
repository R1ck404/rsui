//! Which component of the last render a child is, as React has it: by where it is in the array of children it's in,
//! holes and all, and by what type it is, or by its key and type in that array.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn open(name: &str, source: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-reconcile-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    Hands::open(&file, (500, 400))
}

/// A counter that says who it is, and a log of what mounted and unmounted, which every test here uses.
const COUNTER: &str = r#"
    import { useState, useEffect, Fragment } from 'react';
    const log = [];
    let shown = null;
    function say(what) { log.push(what); shown?.((n) => n + 1); }
    function Counter({ id }) {
        const [count, setCount] = useState(0);
        useEffect(() => { say('mount ' + id); return () => say('unmount ' + id); }, []);
        return <button key={'add-' + id} onClick={() => setCount((c) => c + 1)}>{id + ' is ' + count}</button>;
    }
    function Other({ id }) {
        const [count, setCount] = useState(0);
        useEffect(() => { say('mount other ' + id); return () => say('unmount other ' + id); }, []);
        return <button key={'other-' + id} onClick={() => setCount((c) => c + 1)}>{'other ' + id + ' is ' + count}</button>;
    }
    function Log() {
        const [, setShown] = useState(0);
        shown = setShown;
        return <text>{'log: ' + log.join(', ')}</text>;
    }
"#;

fn app(name: &str, body: &str) -> Hands {
    let mut hands = open(name, &format!("{COUNTER}\n{body}"));
    hands.settle();
    hands
}

fn assert_says(hands: &Hands, text: &str) {
    assert!(hands.says(text), "wanted {text:?} in\n{}", hands.app.outline());
}

#[test]
fn state_stays_with_its_component_when_a_conditional_sibling_before_it_comes_and_goes() {
    let mut hands = app(
        "conditional",
        r#"
        export default function App() {
            const [open, setOpen] = useState(false);
            return (
                <window>
                    <button key="toggle" onClick={() => setOpen((o) => !o)}>Toggle</button>
                    {open && <Counter id="a" />}
                    <Counter id="b" />
                    <Counter id="c" />
                    <Log />
                </window>
            );
        }
        "#,
    );
    hands.click("add-b");
    hands.click("add-c");
    hands.click("add-c");
    hands.settle();
    assert_says(&hands, "b is 1");
    assert_says(&hands, "c is 2");
    hands.click("toggle");
    hands.settle();
    assert_says(&hands, "a is 0");
    assert_says(&hands, "b is 1");
    assert_says(&hands, "c is 2");
    hands.click("add-a");
    hands.click("toggle");
    hands.settle();
    assert!(!hands.has("add-a"), "{}", hands.app.outline());
    assert_says(&hands, "b is 1");
    assert_says(&hands, "c is 2");
    // Only `a` came and went. `b` and `c` mounted once, and nothing unmounted them.
    assert_says(&hands, "log: mount b, mount c, mount a, unmount a");
}

#[test]
fn a_hole_of_null_undefined_or_false_keeps_its_place_inside_an_element() {
    let mut hands = app(
        "holes",
        r#"
        export default function App() {
            const [step, setStep] = useState(0);
            const first = [null, undefined, false, true, <Other id="x" />][step % 5];
            return (
                <window>
                    <button key="next" onClick={() => setStep((s) => s + 1)}>Next</button>
                    <group>
                        {first}
                        <Counter id="b" />
                        {step > 1 ? null : <Counter id="c" />}
                        <Counter id="d" />
                    </group>
                    <Log />
                </window>
            );
        }
        "#,
    );
    hands.click("add-b");
    hands.click("add-d");
    hands.click("add-d");
    for _ in 0..4 {
        hands.click("next");
        hands.settle();
        assert_says(&hands, "b is 1");
        assert_says(&hands, "d is 2");
    }
    assert_says(&hands, "other x is 0");
    assert_says(
        &hands,
        "log: mount b, mount c, mount d, unmount c, mount other x",
    );
}

#[test]
fn a_different_type_in_the_same_place_starts_again() {
    let mut hands = app(
        "types",
        r#"
        export default function App() {
            const [flip, setFlip] = useState(false);
            return (
                <window>
                    <button key="flip" onClick={() => setFlip((f) => !f)}>Flip</button>
                    {flip ? <Other id="1" /> : <Counter id="1" />}
                    {flip ? <row><Counter id="2" /></row> : <group><Counter id="2" /></group>}
                    {flip ? <group><Counter id="3" /></group> : <group><Counter id="3" /></group>}
                    <Log />
                </window>
            );
        }
        "#,
    );
    hands.click("add-1");
    hands.click("add-2");
    hands.click("add-3");
    hands.settle();
    hands.click("flip");
    hands.settle();
    // A component of another type, and one inside an element of another type, start again. One inside the same type doesn't.
    assert_says(&hands, "other 1 is 0");
    assert_says(&hands, "2 is 0");
    assert_says(&hands, "3 is 1");
    hands.click("flip");
    hands.settle();
    assert_says(&hands, "1 is 0");
    assert_says(&hands, "3 is 1");
    assert_says(
        &hands,
        "log: mount 1, mount 2, mount 3, unmount 1, unmount 2, mount other 1, mount 2, unmount other 1, unmount 2, mount 1, mount 2",
    );
}

#[test]
fn keyed_children_keep_their_state_when_they_are_reordered() {
    let mut hands = app(
        "keyed",
        r#"
        export default function App() {
            const [ids, setIds] = useState(['p', 'q', 'r']);
            return (
                <window>
                    <button key="reverse" onClick={() => setIds((ids) => [...ids].reverse())}>Reverse</button>
                    <button key="drop" onClick={() => setIds((ids) => ids.slice(1))}>Drop</button>
                    <group>{ids.map((id) => <Counter key={id} id={id} />)}</group>
                    <group>{ids.map((id) => <row key={id}><Counter id={'row ' + id} /></row>)}</group>
                    <group>{ids.map((id) => <Fragment key={id}><text>{'in ' + id}</text><Counter id={'frag ' + id} /></Fragment>)}</group>
                    <Log />
                </window>
            );
        }
        "#,
    );
    hands.click("add-q");
    hands.click("add-r");
    hands.click("add-r");
    hands.click("add-row q");
    hands.click("add-frag r");
    hands.settle();
    hands.click("reverse");
    hands.settle();
    for what in [
        "p is 0",
        "q is 1",
        "r is 2",
        "row q is 1",
        "row r is 0",
        "frag r is 1",
        "frag q is 0",
    ] {
        assert_says(&hands, what);
    }
    hands.click("drop");
    hands.settle();
    assert!(!hands.has("add-r"), "{}", hands.app.outline());
    assert_says(&hands, "q is 1");
    assert_says(&hands, "row q is 1");
    assert_says(&hands, "frag q is 0");
    // Nothing remounted on the way: the reverse only moved them, and dropping `r` only unmounted it.
    assert_says(
        &hands,
        "log: mount p, mount q, mount r, mount row p, mount row q, mount row r, mount frag p, mount frag q, mount frag r, \
         unmount r, unmount row r, unmount frag r",
    );
}

#[test]
fn an_array_or_a_fragment_among_children_is_one_place() {
    let mut hands = app(
        "arrays",
        r#"
        export default function App() {
            const [items, setItems] = useState([]);
            const [open, setOpen] = useState(false);
            return (
                <window>
                    <button key="more" onClick={() => setItems((items) => [...items, 'i' + items.length])}>More</button>
                    <button key="open" onClick={() => setOpen((o) => !o)}>Open</button>
                    <group>
                        {items.map((item) => <Counter key={item} id={item} />)}
                        <Counter id="after list" />
                        <>
                            {open && <Counter id="in fragment" />}
                            <Counter id="fragment end" />
                        </>
                        {[open && <Counter key="k" id="keyed in array" />, <Counter id="unkeyed in array" />]}
                        <Counter id="last" />
                    </group>
                    <Log />
                </window>
            );
        }
        "#,
    );
    for id in ["after list", "fragment end", "unkeyed in array", "last"] {
        hands.click(&format!("add-{id}"));
    }
    hands.click("more");
    hands.settle();
    hands.click("more");
    hands.settle();
    hands.click("open");
    hands.settle();
    for id in ["after list", "fragment end", "unkeyed in array", "last"] {
        assert_says(&hands, &format!("{id} is 1"));
    }
    assert_says(&hands, "i1 is 0");
    assert_says(&hands, "in fragment is 0");
    assert_says(&hands, "keyed in array is 0");
    assert_says(
        &hands,
        "log: mount after list, mount fragment end, mount unkeyed in array, mount last, mount i0, mount i1, \
         mount in fragment, mount keyed in array",
    );
}

#[test]
fn what_a_component_returns_is_placed_as_its_children_are() {
    let mut hands = app(
        "returned",
        r#"
        function Parts({ open }) {
            // An array, or a fragment with no key, at the top is the children themselves.
            return open ? [<Counter id="first" />, <Counter id="second" />] : <><Counter id="first" /></>;
        }
        function Passes({ children }) {
            return <group>{children}</group>;
        }
        export default function App() {
            const [open, setOpen] = useState(false);
            return (
                <window>
                    <button key="open" onClick={() => setOpen((o) => !o)}>Open</button>
                    <Parts open={open} />
                    <Passes>{open && <Counter id="given" />}<Counter id="passed" /></Passes>
                    <Log />
                </window>
            );
        }
        "#,
    );
    hands.click("add-first");
    hands.click("add-passed");
    hands.settle();
    hands.click("open");
    hands.settle();
    assert_says(&hands, "first is 1");
    assert_says(&hands, "second is 0");
    assert_says(&hands, "passed is 1");
    assert_says(&hands, "given is 0");
    assert_says(
        &hands,
        "log: mount first, mount passed, mount second, mount given",
    );
}

#[test]
fn a_ref_stays_on_its_own_element_when_one_before_it_comes_and_goes() {
    let mut hands = open(
        "refs",
        r#"
        import { useState, useRef } from 'react';
        const log = [];
        const first = (element) => log.push('first ' + (element === null ? 'off' : 'on'));
        const second = (element) => log.push('second ' + (element === null ? 'off' : 'on'));
        export default function App() {
            const [open, setOpen] = useState(false);
            const [, render] = useState(0);
            const held = useRef(null);
            return (
                <window>
                    <button key="open" onClick={() => setOpen((o) => !o)}>Open</button>
                    <button key="look" onClick={() => render((n) => n + 1)}>Look</button>
                    {open && <div ref={first} id="first">First</div>}
                    <div ref={second} id="second">Second</div>
                    <div ref={held} id="held">Held</div>
                    <text>{'log: ' + log.join(', ') + '; held ' + (held.current ? 'set' : 'none')}</text>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    hands.click("open");
    hands.settle();
    hands.click("open");
    hands.settle();
    hands.click("look");
    hands.settle();
    let outline = hands.app.outline();
    assert!(
        outline.contains("log: second on, first on, first off;"),
        "{outline}"
    );
    assert!(!outline.contains("held none"), "{outline}");
}
