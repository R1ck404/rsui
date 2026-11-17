//! Code moving what React drew, as cmdk sorts its items with `appendChild()`, and the geometry and styles that cmdk and Radix
//! read: each as small as it comes.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn open(name: &str, css: &str, before: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-dom-moves-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let css = format!("body {{ margin: 0; }}\n{css}");
    let app = format!(
        "import {{ PageWindow, dom }} from 'rsui/web';\nimport * as React from 'react';\nimport './app.css';\n{before}\nexport default function App() {{ return <PageWindow><Page /></PageWindow>; }}\n"
    );
    std::fs::write(folder.join("app.tsx"), app).unwrap();
    std::fs::write(folder.join("app.css"), css).unwrap();
    let mut hands = Hands::open(&folder.join("app.tsx"), (500, 400));
    hands.settle();
    hands
}

/// The keys given, from the top of the window down, by where each is drawn.
fn drawn_order(hands: &mut Hands, keys: &[&str]) -> Vec<String> {
    let mut found: Vec<(f32, String)> = keys.iter().map(|key| (hands.rect_of(key)[1], (*key).to_string())).collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().map(|(_, key)| key).collect()
}

#[test]
fn what_code_moves_is_drawn_where_it_put_it_until_react_changes_what_is_there() {
    let mut hands = open(
        "keyed",
        "",
        r#"
        function Page() {
            const [items, setItems] = React.useState(['a', 'b', 'c']);
            const [said, setSaid] = React.useState('');
            const [, setTick] = React.useState(0);
            const list = React.useRef(null);
            const order = () => Array.from(list.current.children).map((one) => one.textContent).join(',');
            const reverse = () => {
                for (const one of Array.from(list.current.children).reverse()) {
                    list.current.appendChild(one);
                }
                // The DOM says so at once, as a page's does.
                setSaid('now ' + order());
            };
            return (
                <dom.div>
                    <dom.button key="reverse" onClick={reverse}>reverse</dom.button>
                    <dom.button key="again" onClick={() => setTick((was) => was + 1)}>again</dom.button>
                    <dom.button key="add" onClick={() => setItems((was) => [...was, 'd'])}>add</dom.button>
                    <dom.div ref={list}>{items.map((one) => <dom.div key={one}>{one}</dom.div>)}</dom.div>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    hands.click("reverse");
    hands.settle();
    assert!(hands.says("now c,b,a"), "{}", hands.app.outline());
    assert_eq!(drawn_order(&mut hands, &["a", "b", "c"]), ["c", "b", "a"]);
    // React drawing the same again leaves it as it is.
    hands.click("again");
    hands.settle();
    assert_eq!(drawn_order(&mut hands, &["a", "b", "c"]), ["c", "b", "a"]);
    // What React puts in goes before the next of what it didn't move, or at the end: react-dom's `getHostSibling`.
    hands.click("add");
    hands.settle();
    assert_eq!(drawn_order(&mut hands, &["a", "b", "c", "d"]), ["c", "b", "a", "d"]);
}

#[test]
fn a_component_that_is_moved_keeps_its_state() {
    let mut hands = open(
        "parts",
        "",
        r#"
        function Part({ name }) {
            const [count, setCount] = React.useState(0);
            return <dom.li key={name} onClick={() => setCount((was) => was + 1)}>{name + ' ' + count}</dom.li>;
        }
        function Page() {
            const list = React.useRef(null);
            const reverse = () => {
                for (const one of Array.from(list.current.children).reverse()) {
                    list.current.appendChild(one);
                }
            };
            return (
                <dom.div>
                    <dom.button key="reverse" onClick={reverse}>reverse</dom.button>
                    <dom.ul ref={list}>
                        <Part name="p1" />
                        <Part name="p2" />
                        <Part name="p3" />
                    </dom.ul>
                </dom.div>
            );
        }
        "#,
    );
    hands.click("p1");
    hands.settle();
    assert!(hands.says("p1 1"), "{}", hands.app.outline());
    hands.click("reverse");
    hands.settle();
    assert_eq!(drawn_order(&mut hands, &["p1", "p2", "p3"]), ["p3", "p2", "p1"]);
    assert!(hands.says("p1 1"), "moved, not made again: {}", hands.app.outline());
    hands.click("p1");
    hands.settle();
    assert!(hands.says("p1 2"), "{}", hands.app.outline());
}

#[test]
fn what_a_provider_gives_is_moved_and_what_comes_back_goes_where_react_puts_it() {
    // cmdk's group: its items are in a context's provider inside the group's `<div>`, and an item that's filtered out draws
    // nothing until it's found again.
    let mut hands = open(
        "provider",
        "",
        r#"
        const Ctx = React.createContext(0);
        function Item({ name, shown }) {
            return shown ? <dom.div key={name}>{name}</dom.div> : null;
        }
        function Page() {
            const [hidden, setHidden] = React.useState(false);
            const list = React.useRef(null);
            const first = () => list.current.appendChild(list.current.firstElementChild);
            return (
                <dom.div>
                    <dom.button key="first" onClick={first}>first to the end</dom.button>
                    <dom.button key="hide" onClick={() => setHidden((was) => !was)}>hide a</dom.button>
                    <dom.div ref={list}>
                        <Ctx.Provider value={1}>
                            <Item name="a" shown={!hidden} />
                            <Item name="b" shown />
                            <Item name="c" shown />
                        </Ctx.Provider>
                    </dom.div>
                </dom.div>
            );
        }
        "#,
    );
    hands.click("first");
    hands.settle();
    assert_eq!(drawn_order(&mut hands, &["a", "b", "c"]), ["b", "c", "a"]);
    // Gone and back, it's put in before the next that stayed, as react-dom does.
    hands.click("hide");
    hands.settle();
    assert!(!hands.has("a"), "{}", hands.app.outline());
    hands.click("hide");
    hands.settle();
    assert_eq!(drawn_order(&mut hands, &["a", "b", "c"]), ["a", "b", "c"]);
}

#[test]
fn computed_overflow_says_what_css_says_each_way() {
    // react-remove-scroll lets a wheel through only to what says `auto` or `scroll` the way it goes: cmdk's list in a dialog.
    let hands = open(
        "overflow",
        ".down { overflow-y: auto; height: 20px; } .both { overflow: hidden; } .apart { overflow-x: hidden; overflow-y: scroll; height: 20px; } .clip { overflow-x: clip; }",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                const of = (id) => { const style = getComputedStyle(document.getElementById(id)); return style.overflowX + '/' + style.overflowY; };
                setSaid(['down', 'both', 'apart', 'clip', 'none'].map(of).join(' '));
            }, []);
            return (
                <dom.div>
                    <dom.div id="down" className="down">a</dom.div>
                    <dom.div id="both" className="both">b</dom.div>
                    <dom.div id="apart" className="apart">c</dom.div>
                    <dom.div id="clip" className="clip">d</dom.div>
                    <dom.div id="none">e</dom.div>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    assert!(
        hands.says("auto/auto hidden/hidden hidden/scroll clip/visible visible/visible"),
        "{}",
        hands.app.outline()
    );
}

#[test]
fn offsets_are_from_the_page_as_laid_out_however_it_scrolls_or_moves() {
    let hands = open(
        "offsets",
        "",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                window.scrollTo(0, 300);
                setTimeout(() => {
                    const outer = document.getElementById('outer');
                    const kid = document.getElementById('kid');
                    setSaid('outer ' + outer.offsetTop + ' moved ' + document.getElementById('moved').offsetLeft + ' kid ' + kid.offsetLeft + ' in ' + kid.offsetParent.id + ' at ' + Math.round(outer.getBoundingClientRect().y));
                }, 50);
            }, []);
            return (
                <dom.div style={{ padding: 20 }}>
                    <dom.div id="outer" style={{ height: 40 }}>outer</dom.div>
                    <dom.div id="moved" style={{ transform: 'translate(40px, 10px)', width: 200, padding: 10 }}>
                        <dom.div id="kid" style={{ transform: 'translateX(-50%)', width: 100 }}>kid</dom.div>
                    </dom.div>
                    <dom.p>{said}</dom.p>
                    <dom.div style={{ height: 1200 }} />
                </dom.div>
            );
        }
        "#,
    );
    let mut hands = hands;
    hands.wait_for("outer 20 moved 20 kid 10 in moved at -280");
}

#[test]
fn what_is_placed_by_itself_on_a_line_without_sides_is_after_what_is_before_it() {
    // Radix's NavigationMenu trigger's focus proxy: a visually hidden `position: absolute` span after the trigger in its `<li>`.
    let mut hands = open(
        "static-on-line",
        ".item { position: relative; } .trigger { display: inline-flex; width: 120px; height: 30px; } .proxy { position: absolute; width: 1px; height: 1px; overflow: hidden; }",
        r#"
        function Page() {
            return (
                <dom.ul style={{ listStyle: 'none', padding: 0, margin: 0 }}>
                    <dom.li className="item" key="item">
                        <dom.button className="trigger" key="trigger">Components</dom.button>
                        <dom.span className="proxy" key="proxy" tabIndex={0} />
                    </dom.li>
                </dom.ul>
            );
        }
        "#,
    );
    let [item_x, _, _, _] = hands.rect_of("item");
    let [trigger_x, _, trigger_width, _] = hands.rect_of("trigger");
    let [proxy_x, _, _, _] = hands.rect_of("proxy");
    assert!(
        (proxy_x - (trigger_x + trigger_width)).abs() < 0.5,
        "after the trigger ({trigger_x} + {trigger_width}), not at the item's start ({item_x}): {proxy_x}
{}",
        hands.app.outline()
    );
}
