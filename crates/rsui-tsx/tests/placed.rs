//! A page's plain elements drawn in place, as host elements of the component whose output they're in, rather than each as a
//! component of its own (`placeElement` in web.js): each page here is drawn twice, with that on and with
//! `globalThis.__rsuiPlaced = false`, and what's drawn, where, and what the page's code finds out about it has to be the same,
//! while far fewer components are made and rendered.

mod hands;

use std::fmt::Write as _;
use std::path::PathBuf;

use hands::{Hands, KeyCode};
use rsui::core::{Node, PropValue};

/// A page made of `app`, a module whose default export is the app, with `globalThis.__rsuiPlaced = false` first when it's not to be
/// drawn in place. It says `done ` and what it found out once it's finished.
fn page(name: &str, placed: bool, app: &str) -> Hands {
    let source = format!(
        "{}\n{app}",
        if placed { "" } else { "(globalThis as any).__rsuiPlaced = false;" }
    );
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-placed-{}-{name}-{placed}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    let mut hands = Hands::open(&file, (480, 1200));
    hands.wait_for("done");
    hands.settle();
    hands
}

/// Everything that's drawn, with where it is, its key, and all its props and actions, one to a line, with each page element's own
/// key (`__dom` and a number, which depends on the order things are first drawn in) left out.
fn dump(hands: &Hands) -> String {
    fn plain(text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(at) = rest.find("__dom") {
            out.push_str(&rest[..at + 5]);
            rest = rest[at + 5..].trim_start_matches(|c: char| c.is_ascii_digit());
        }
        out.push_str(rest);
        out
    }
    fn walk(node: &Node, layout: &rsui::layout::Layout, path: &mut Vec<u32>, out: &mut String) {
        let _ = write!(out, "{}{}", " ".repeat(path.len()), node.role.name());
        if let Some(rect) = layout.rect(path) {
            let _ = write!(out, " {:.2},{:.2},{:.2},{:.2}", rect.x, rect.y, rect.width, rect.height);
        }
        if !node.name.is_empty() {
            let _ = write!(out, " {:?}", node.name.as_str());
        }
        // What the runtime numbers by itself, a host with handlers that has no key of its own, is numbered through what a component
        // gives back, which is more of them in place, so it's only said that it's numbered.
        if let Some(key) = node.key.as_ref().map(|key| key.to_string()) {
            let key = if key.chars().all(|c| c.is_ascii_digit()) { "numbered".to_owned() } else { plain(&key) };
            let _ = write!(out, " key={key}");
        }
        let mut props: Vec<String> = node
            .props
            .iter()
            .map(|(name, value)| {
                let value = match value {
                    PropValue::Bool(on) => on.to_string(),
                    PropValue::Int(number) => number.to_string(),
                    PropValue::Float(number) => format!("{number:.3}"),
                    PropValue::Str(text) => format!("{:?}", plain(text.as_str())),
                };
                format!("{}={value}", name.as_str())
            })
            .collect();
        props.sort();
        for prop in props {
            let _ = write!(out, " {prop}");
        }
        for action in node.actions.iter() {
            let _ = write!(out, " on:{action:?}");
        }
        out.push('\n');
        for (index, child) in node.children.iter().enumerate() {
            path.push(index as u32);
            walk(child, layout, path, out);
            path.pop();
        }
    }
    let mut out = String::new();
    walk(hands.app.runtime().root(), &hands.app.layout(), &mut Vec::new(), &mut out);
    out
}

/// What the page said when it was done, after `done`.
fn said(hands: &Hands) -> String {
    let outline = hands.app.outline();
    let at = outline.find("done").expect("it's done");
    outline[at + 4..].trim_start_matches(' ').split('"').next().unwrap_or_default().to_owned()
}

/// What a page came to one way: what was drawn after it started and after each step, what it said in the end, and how many
/// components rendered to start it.
struct Run {
    dumps: Vec<(String, String)>,
    said: String,
    renders: u64,
    outline: String,
}

/// Steps done to a page, each named.
type Steps<'a> = &'a [(&'a str, &'a dyn Fn(&mut Hands))];

fn run(name: &str, placed: bool, app: &str, steps: Steps) -> Run {
    let mut hands = page(name, placed, app);
    let renders = hands.app.runtime().renders();
    let mut dumps = vec![("started".to_owned(), dump(&hands))];
    for (step, action) in steps {
        action(&mut hands);
        hands.settle();
        dumps.push((step.to_string(), dump(&hands)));
    }
    Run { dumps, said: said(&hands), renders, outline: hands.app.outline() }
}

/// The same page drawn both ways, one after the other, as one app at a time has a thread's engine: what's drawn has to be the
/// same, after it starts and after each step. Gives both.
fn both(name: &str, app: &str, steps: Steps) -> (Run, Run) {
    let placed = run(name, true, app, steps);
    let not = run(name, false, app, steps);
    for ((step, a), (_, b)) in placed.dumps.iter().zip(&not.dumps) {
        if a != b {
            let first = a.lines().zip(b.lines()).position(|(x, y)| x != y).unwrap_or(a.lines().count().min(b.lines().count()));
            let around = |text: &str| text.lines().skip(first.saturating_sub(6)).take(20).collect::<Vec<_>>().join("
");
            panic!("{name}, {step}: drawn in place differs at line {first}
--- in place
{}
--- as components
{}", around(a), around(b));
        }
    }
    assert_eq!(placed.said, not.said, "{name}: what the page found out");
    (placed, not)
}

/// Finds what's drawn by the text it shows, as a person would.
trait ByText {
    fn middle_of_text(&mut self, text: &str) -> (f32, f32);
    fn click_text(&mut self, text: &str);
}

impl ByText for Hands {
    fn middle_of_text(&mut self, text: &str) -> (f32, f32) {
        fn find(node: &Node, text: &str, path: &mut Vec<u32>) -> bool {
            if node.name.as_str() == text {
                return true;
            }
            for (at, child) in node.children.iter().enumerate() {
                path.push(at as u32);
                if find(child, text, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
        self.settle();
        let mut path = Vec::new();
        assert!(find(self.app.runtime().root(), text, &mut path), "nothing shows {text:?} in
{}", self.app.outline());
        let rect = self.app.layout().rect(&path).expect("it's been laid out");
        (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
    }

    fn click_text(&mut self, text: &str) {
        let (x, y) = self.middle_of_text(text);
        self.click_at(x, y);
    }
}

/// The value after `name:` in what a page said.
fn value<'a>(said: &'a str, name: &str) -> &'a str {
    said.split('|')
        .find_map(|one| one.strip_prefix(name).and_then(|rest| rest.strip_prefix(':')))
        .unwrap_or_else(|| panic!("no {name} in {said}"))
}

/// Rows of plain elements, each in a component of the app's with a ref and a handler, as a file list's are: what's drawn, what a
/// ref holds and what it says of where it is, and that a press picks a row, are the same; and only the app's own components are
/// made, not one for each of the page's elements.
#[test]
fn rows_with_refs_and_handlers() {
    const APP: &str = r#"
        import { useLayoutEffect, useRef, useState } from 'react';
        import { PageWindow } from 'rsui/web';
        const css = document.createElement('style');
        css.textContent = '.row { display: flex; height: 24px; padding: 0 8px; gap: 4px } .row:hover { background: #eee } .picked { background: #cde; color: #123 } .name { flex: 1 } .size { width: 60px; text-align: right }';
        document.head.appendChild(css);
        const out: string[] = [];
        function Row({ index, picked, onPick }: { index: number; picked: boolean; onPick: (index: number) => void }) {
            const ref = useRef<HTMLDivElement>(null);
            useLayoutEffect(() => {
                if (index === 3) {
                    const box = ref.current!.getBoundingClientRect();
                    out.push('tag:' + ref.current!.tagName + '|data:' + ref.current!.dataset.index + '|width:' + Math.round(box.width));
                }
            }, [index]);
            return (
                <div ref={ref} data-index={index} className={'row' + (picked ? ' picked' : '')} onMouseDown={() => onPick(index)} key={index}>
                    <span className="name">{'file ' + index}</span>
                    <span className="size">{index * 3} KB</span>
                </div>
            );
        }
        export default function App() {
            const [picked, setPicked] = useState(0);
            const [said, setSaid] = useState('');
            const rows = [];
            for (let index = 0; index < 40; index++) {
                rows.push(<Row key={index} index={index} picked={index === picked} onPick={setPicked} />);
            }
            useLayoutEffect(() => {
                setTimeout(() => {
                    const found = document.querySelectorAll('.row').length;
                    const second = document.querySelector('[data-index="2"] .name')?.textContent;
                    setSaid('done ' + out.join('|') + '|rows:' + found + '|second:' + second);
                }, 50);
            }, []);
            return (
                <PageWindow>
                    <div className="list">{rows}</div>
                    <p className="said">{said}</p>
                    <p className="picked-is">picked {picked}</p>
                </PageWindow>
            );
        }
    "#;
    let press_row: &dyn Fn(&mut Hands) = &|hands| {
        let (x, y) = hands.middle_of_text("file 5");
        hands.down_at(x, y);
        hands.up_at(x, y);
    };
    let (placed, not) = both("rows", APP, &[("a row pressed", press_row)]);
    let said = placed.said.clone();
    assert_eq!(value(&said, "tag"), "DIV", "{said}");
    assert_eq!(value(&said, "data"), "3", "{said}");
    assert_eq!(value(&said, "rows"), "40", "{said}");
    assert_eq!(value(&said, "second"), "file 2", "{said}");
    assert!(value(&said, "width").parse::<u32>().unwrap() > 400, "{said}");
    // Each of 40 rows is a `div` with two `span`s in it, which as components are 120 of them, and 3 more around them.
    assert!(not.renders - placed.renders >= 120, "drawn in place, {} components rendered to start, against {}", placed.renders, not.renders);
    // A press on a row picked it, which drew it and the one that was picked again, the same way.
    assert!(placed.outline.contains("picked 5"), "{}", placed.outline);
}

/// What an element drawn in place hears and passes on: a click on what's inside goes through what's around it, which hears it on
/// the way down with `onClickCapture` and on the way back up, unless it's stopped; and a callback ref is given its element, and
/// what it gave back is called once the element goes.
#[test]
fn events_and_callback_refs() {
    const APP: &str = r#"
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        const heard: string[] = [];
        const refs: string[] = [];
        export default function App() {
            const [shown, setShown] = useState(true);
            const [count, setCount] = useState(0);
            return (
                <PageWindow>
                    <div className="outer" onClickCapture={() => heard.push('outer down')} onClick={() => heard.push('outer up')}>
                        <div className="inner" onClick={(event) => { heard.push('inner'); if (count > 0) event.stopPropagation(); setCount(count + 1); }}>
                            <span className="press">press {count}</span>
                        </div>
                    </div>
                    {shown ? (
                        <div className="kept" ref={(element) => { refs.push('given ' + (element?.className ?? 'nothing')); return () => refs.push('let go'); }}>kept</div>
                    ) : null}
                    <div className="hide" onClick={() => setShown(false)}>hide</div>
                    <p className="said">done {heard.join(',')}|{refs.join(',')}</p>
                </PageWindow>
            );
        }
    "#;
    let (placed, not) = both("events", APP, &[
        ("pressed once", &|hands| hands.click_text("press 0")),
        ("pressed twice", &|hands| hands.click_text("press 1")),
        ("hidden", &|hands| hands.click_text("hide")),
        ("pressed again", &|hands| hands.click_text("press 2")),
    ]);
    assert!(not.renders > placed.renders, "{} against {}", placed.renders, not.renders);
    let said = placed.said;
    assert!(said.starts_with("outer down,inner,outer up,outer down,inner,outer down,inner|given kept,let go"), "{said}");
}

/// What's in an element drawn in place is under it: a component in it reads what a provider around the element gives, inherits
/// its color, and reads where it is on a line; and a sheet added later restyles both what's drawn in place and what isn't.
#[test]
fn contexts_inheritance_and_sheets() {
    const APP: &str = r#"
        import { createContext, useContext, useEffect, useState } from 'react';
        import { PageWindow } from 'rsui/web';
        const Name = createContext('none');
        const base = document.createElement('style');
        base.textContent = '.box { color: rgb(200, 0, 0); padding: 4px } .label { font-weight: 700 }';
        document.head.appendChild(base);
        function Label() {
            const name = useContext(Name);
            return <span className="label">{name}</span>;
        }
        export default function App() {
            const [said, setSaid] = useState('');
            useEffect(() => {
                setTimeout(() => {
                    const later = document.createElement('style');
                    later.textContent = '.box .label { color: rgb(0, 0, 200) } .tail { margin-left: 12px }';
                    document.head.appendChild(later);
                    setTimeout(() => {
                        const label = document.querySelector('.label')!;
                        setSaid('done color:' + getComputedStyle(label).color + '|margin:' + getComputedStyle(document.querySelector('.tail')!).marginLeft);
                    }, 50);
                }, 50);
            }, []);
            return (
                <PageWindow>
                    <Name.Provider value="inner name">
                        <div className="box">
                            <p>
                                before <Label /> after <span className="tail">tail</span>
                            </p>
                        </div>
                    </Name.Provider>
                    <p className="said">{said}</p>
                </PageWindow>
            );
        }
    "#;
    let (placed, not) = both("contexts", APP, &[]);
    assert!(not.renders > placed.renders, "{} against {}", placed.renders, not.renders);
    let said = placed.said.clone();
    assert_eq!(value(&said, "color"), "rgb(0, 0, 200)", "{said}");
    assert_eq!(value(&said, "margin"), "12px", "{said}");
    assert!(placed.outline.contains("inner name"), "{}", placed.outline);
}

/// Where an element is among what's around it, as `:nth-child()` and `+` ask, and what's hovered, for an element drawn in place:
/// one put in before the rest moves what each of them is, and hovering one draws only it as hovered.
#[test]
fn siblings_and_hover() {
    const APP: &str = r#"
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        const css = document.createElement('style');
        css.textContent = '.item { height: 20px } .item:nth-child(2) { color: rgb(0, 128, 0) } .item + .item { border-top: 1px solid black } .item:hover { background: rgb(255, 255, 0) }';
        document.head.appendChild(css);
        export default function App() {
            const [items, setItems] = useState(['b', 'c', 'd']);
            return (
                <PageWindow>
                    <div className="items">
                        {items.map((item) => <div key={item} className="item">item {item}</div>)}
                    </div>
                    <div className="add" onClick={() => setItems(['a', ...items])}>add</div>
                    <p>done</p>
                </PageWindow>
            );
        }
    "#;
    let (placed, not) = both("siblings", APP, &[
        ("added", &|hands| hands.click_text("add")),
        ("hovered", &|hands| {
            let (x, y) = hands.middle_of_text("item c");
            hands.move_at(x, y);
        }),
    ]);
    assert!(not.renders > placed.renders, "{} against {}", placed.renders, not.renders);
    let outline = &placed.dumps.last().unwrap().1;
    let hovered = outline.lines().filter(|line| line.contains("255, 255, 0") || line.contains("#ffff00")).count();
    assert!(hovered <= 1, "{outline}");
}

/// Keys pressed in an element drawn in place that has the keyboard reach its handler.
#[test]
fn keys_reach_what_has_the_keyboard() {
    const APP: &str = r#"
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const [pressed, setPressed] = useState<string[]>([]);
            return (
                <PageWindow>
                    <div className="keys" tabIndex={0} onKeyDown={(event) => setPressed([...pressed, event.key])}>keys {pressed.join(' ')}</div>
                    <p>done</p>
                </PageWindow>
            );
        }
    "#;
    let (placed, not) = both("keys", APP, &[
        ("focused", &|hands| hands.click_text("keys ")),
        ("pressed", &|hands| hands.press(KeyCode::Down)),
    ]);
    assert!(placed.outline.contains("keys ArrowDown"), "{}", placed.outline);
    assert!(not.renders > placed.renders, "{} against {}", placed.renders, not.renders);
}
