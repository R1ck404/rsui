//! A page's DOM as code changes it, and editing it with `contentEditable`: what's in `rsui/dom` and `rsui/editing`.

mod hands;

use std::path::PathBuf;

use hands::{Hands, KeyCode, KeyPress};

fn open(name: &str, source: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-editing-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    Hands::open(&file, (500, 400))
}

#[test]
fn ranges_selection_and_mutation_records_work_on_nodes_code_made() {
    let mut hands = open(
        "dom",
        r#"
        import React, { useEffect, useRef, useState } from 'react';
        export default function App() {
            const ref = useRef<HTMLDivElement>(null);
            const [said, setSaid] = useState('');
            useEffect(() => {
                const out: string[] = [];
                const div = document.createElement('div');
                ref.current!.appendChild(div);
                div.innerHTML = '<b>bold</b> text';
                const range = document.createRange();
                range.setStart(div.firstChild!.firstChild!, 1);
                range.setEnd(div.lastChild!, 3);
                out.push('range:' + range.toString());
                new MutationObserver((records) => {
                    out.push('records:' + records.map((one) => one.type).join(','));
                    setSaid(out.join('|'));
                }).observe(div, { childList: true, subtree: true, characterData: true });
                (div.lastChild as Text).data = ' TEXT';
                const selection = window.getSelection()!;
                selection.setBaseAndExtent(div.firstChild!.firstChild!, 0, div.lastChild!, 2);
                out.push('selection:' + selection.toString());
                out.push('parsed:' + new DOMParser().parseFromString('<p>a<i>b</i></p>', 'text/html').body.innerHTML);
            }, []);
            return <div><div ref={ref} /><div className="said">{said}</div></div>;
        }
        "#,
    );
    hands.wait_for("records:");
    assert!(
        hands.says("range:old te|selection:bold T|parsed:<p>a<i>b</i></p>|records:characterData"),
        "{}",
        hands.app.outline()
    );
}

#[test]
fn many_nodes_code_made_keep_their_order_and_ranges_across_them() {
    // Where each child is is kept on it, and found again once something's put in or taken out before it: see `ownIndex` in dom.js.
    // A range over many of them is worked out from where its ends are: see `reachOf`.
    let mut hands = open(
        "many",
        r#"
        import React, { useEffect, useRef, useState } from 'react';
        export default function App() {
            const ref = useRef<HTMLDivElement>(null);
            const [said, setSaid] = useState('');
            useEffect(() => {
                const div = document.createElement('div');
                ref.current!.appendChild(div);
                const ps: HTMLElement[] = [];
                for (let i = 0; i < 50; i++) {
                    const p = document.createElement('p');
                    p.textContent = 'p' + i;
                    div.appendChild(p);
                    ps.push(p);
                }
                for (const name of ['a', 'b', 'c']) {
                    const p = document.createElement('p');
                    p.textContent = name;
                    div.insertBefore(p, div.firstChild);
                }
                div.removeChild(ps[10]);
                ps[20].remove();
                div.insertBefore(ps[20], ps[5]);
                const forward: string[] = [];
                for (let at = div.firstChild; at !== null; at = at.nextSibling) forward.push(at.textContent!);
                const back: string[] = [];
                for (let at = div.lastChild; at !== null; at = at.previousSibling) back.unshift(at.textContent!);
                const listed = Array.from(div.childNodes).map((one) => one.textContent).join(',');
                const out = [`order:${forward.join(',') === listed && back.join(',') === listed}`, `count:${div.childNodes.length}`, `first:${forward.slice(0, 5).join(',')}`];
                const range = document.createRange();
                range.setStart(ps[2].firstChild!, 1);
                range.setEnd(ps[47].firstChild!, 2);
                const text = range.toString();
                out.push(`text:${text.slice(0, 6)}..${text.slice(-6)}:${text.length}`);
                out.push(`copied:${range.cloneContents().childNodes.length}`);
                out.push(`contains:${div.contains(ps[3].firstChild)},${ps[3].contains(ps[4])},${ps[3].contains(ps[3].firstChild)}`);
                const cut = document.createRange();
                cut.setStart(ps[1].firstChild!, 1);
                cut.setEnd(ps[3].firstChild!, 1);
                cut.deleteContents();
                out.push(`cut:${Array.from(div.childNodes).slice(3, 7).map((one) => one.textContent).join(',')}`);
                ps[0].style.color = 'red';
                out.push(`style:${(ps[0].getAttribute('style') ?? '').includes('red')},${ps[1].hasAttribute('style')},${ps[1].tagName}`);
                setSaid(out.join('|'));
            }, []);
            return <div><div ref={ref} /><div className="said">{said}</div></div>;
        }
        "#,
    );
    hands.wait_for("order:");
    assert!(
        hands.says("order:true|count:52|first:c,b,a,p0,p1|text:2p3p4p..5p46p4:125|copied:45|contains:true,false,true|cut:p0,p,3,p4|style:true,false,P"),
        "{}",
        hands.app.outline()
    );
}

#[test]
fn what_react_draws_has_its_text_as_text_nodes() {
    let mut hands = open(
        "drawn",
        r#"
        import React, { useEffect, useRef, useState } from 'react';
        export default function App() {
            const ref = useRef<HTMLParagraphElement>(null);
            const [said, setSaid] = useState('');
            useEffect(() => {
                const p = ref.current!;
                const kinds = Array.from(p.childNodes).map((one) => (one.nodeType === 3 ? (one as Text).data : (one as Element).localName));
                setSaid(`kids:${kinds.join(',')} text:${p.textContent} same:${p.firstChild === p.childNodes[0]}`);
            }, []);
            return <div><p ref={ref}>a<b>bold</b>c<br />d</p><div>{said}</div></div>;
        }
        "#,
    );
    hands.wait_for("kids:");
    assert!(hands.says("kids:a,b,c,br,d text:aboldcd same:true"), "{}", hands.app.outline());
}

#[test]
fn typing_enter_and_backspace_in_something_that_can_be_edited() {
    let mut hands = open(
        "typing",
        r#"
        import React, { useEffect, useRef, useState } from 'react';
        export default function App() {
            const ref = useRef<HTMLDivElement>(null);
            const [said, setSaid] = useState('');
            useEffect(() => {
                const root = ref.current!;
                root.innerHTML = '<p>Hello</p><p>World</p>';
                const types: string[] = [];
                root.addEventListener('beforeinput', (event: any) => types.push(event.inputType));
                root.addEventListener('input', () => setSaid(`html=${root.innerHTML} types=${types.join(',')}`));
            }, []);
            return (
                <div>
                    <div ref={ref} contentEditable suppressContentEditableWarning style={{ fontFamily: 'Arial', fontSize: 16, lineHeight: '20px', width: 300 }} />
                    <div>{said}</div>
                </div>
            );
        }
        "#,
    );
    hands.settle();
    // After "Hello", on the first paragraph's line.
    hands.click_at(250.0, 26.0);
    hands.type_text("!");
    hands.wait_for("html=<p>Hello!</p>");
    hands.press(KeyCode::Enter);
    hands.type_text("x");
    hands.press(KeyCode::Backspace);
    hands.press(KeyCode::Backspace);
    hands.wait_for("deleteContentBackward,deleteContentBackward");
    assert!(
        hands.says("html=<p>Hello!</p><p>World</p> types=insertText,insertParagraph,insertText,deleteContentBackward,deleteContentBackward"),
        "{}",
        hands.app.outline()
    );
}

/// Undo and redo of what the page left to the browser: typing keeps only the block it went into (so starting to type again in
/// a big document doesn't write all of it out), typing into another block is undone apart, and Enter is undone as a whole.
#[test]
fn undo_and_redo_of_typing_and_enter() {
    let mut hands = open(
        "undo",
        r#"
        import React, { useEffect, useRef, useState } from 'react';
        export default function App() {
            const ref = useRef<HTMLDivElement>(null);
            const [said, setSaid] = useState('');
            useEffect(() => {
                const root = ref.current!;
                root.innerHTML = '<p>Hello</p><p>World</p>';
                const show = () => setSaid(`html=${root.innerHTML};`);
                root.addEventListener('input', show);
            }, []);
            return (
                <div>
                    <div ref={ref} contentEditable suppressContentEditableWarning style={{ fontFamily: 'Arial', fontSize: 16, lineHeight: '20px', width: 300 }} />
                    <div>{said}</div>
                </div>
            );
        }
        "#,
    );
    let ctrl = |letter: char| KeyPress::new(KeyCode::Character(letter)).ctrl();
    hands.settle();
    // After "Hello", then after "World", within the time typing goes together.
    hands.click_at(250.0, 26.0);
    hands.type_text("ab");
    hands.click_at(250.0, 62.0);
    hands.type_text("c");
    hands.wait_for("html=<p>Helloab</p><p>Worldc</p>;");
    hands.press_key(ctrl('z'));
    hands.wait_for("html=<p>Helloab</p><p>World</p>;");
    hands.press_key(ctrl('z'));
    hands.wait_for("html=<p>Hello</p><p>World</p>;");
    hands.press_key(ctrl('y'));
    hands.wait_for("html=<p>Helloab</p><p>World</p>;");
    hands.press_key(ctrl('y'));
    hands.wait_for("html=<p>Helloab</p><p>Worldc</p>;");
    // Enter changes the blocks, and is undone with all of them; the typing before it still is after.
    hands.press(KeyCode::Enter);
    hands.type_text("d");
    hands.wait_for("html=<p>Helloab</p><p>Worldc</p><p>d</p>;");
    hands.press_key(ctrl('z'));
    hands.wait_for("html=<p>Helloab</p><p>Worldc</p><p><br></p>;");
    hands.press_key(ctrl('z'));
    hands.wait_for("html=<p>Helloab</p><p>Worldc</p>;");
    hands.press_key(ctrl('z'));
    hands.wait_for("html=<p>Helloab</p><p>World</p>;");
}

#[test]
fn nodes_code_made_have_the_page_s_focus_and_style() {
    let mut hands = open(
        "focus",
        r#"
        import React, { useEffect, useRef, useState } from 'react';
        export default function App() {
            const ref = useRef<HTMLDivElement>(null);
            const [said, setSaid] = useState('');
            useEffect(() => {
                const made = document.createElement('div');
                made.setAttribute('tabindex', '0');
                made.textContent = 'made';
                made.style.cssText = 'background: url(data:image/png;base64,AAAA); color: red';
                ref.current!.appendChild(made);
                const heard: string[] = [];
                for (const type of ['focus', 'focusin', 'blur', 'focusout']) {
                    made.addEventListener(type, () => heard.push(type));
                }
                // Once it's drawn, which is when it can have the keyboard.
                setTimeout(() => {
                    made.focus();
                    const focused = document.activeElement === made;
                    made.blur();
                    const after = document.activeElement === document.body;
                    setSaid(`focused:${focused} after:${after} heard:${heard.join(',')} style:${made.getAttribute('style')} color:${made.style.color}`);
                }, 50);
            }, []);
            return <div><div ref={ref} /><div>{said}</div></div>;
        }
        "#,
    );
    hands.wait_for("focused:");
    assert!(
        hands.says("focused:true after:true heard:focus,focusin,blur,focusout style:background: url(data:image/png;base64,AAAA); color: red; color:red"),
        "{}",
        hands.app.outline()
    );
}

/// A long list of what code made, as an editor's document is, where one is put in, one taken out and one made longer at a time:
/// each time, what's drawn for it is what's changed, and the rest is kept as it was (`keptKids` in web.js, `build_long` in
/// bridge.rs), and where the rest are is moved rather than worked out again (`solve_moved` in rsui-layout, which a debug build
/// checks against laying it all out again). It's all where a page has it, one under another, in order. With a rule about where
/// each is among the rest, as `:nth-child` is, what's after a change is drawn again, as that's changed for it.
fn long_list(name: &str, css: &str) -> Hands {
    let source = r#"
        import React, { useEffect, useRef, useState } from 'react';
        const style = document.createElement('style');
        style.textContent = 'CSS';
        document.head.appendChild(style);
        export default function App() {
            const ref = useRef<HTMLDivElement>(null);
            const [said, setSaid] = useState('');
            useEffect(() => {
                const div = document.createElement('div');
                ref.current!.appendChild(div);
                const ps: HTMLElement[] = [];
                for (let i = 0; i < 100; i++) {
                    const p = document.createElement('p');
                    p.textContent = 'p' + i;
                    div.appendChild(p);
                    ps.push(p);
                }
                // Each what's on the page then, as text, and how far apart each is from the one before.
                const look = () => {
                    const all = Array.from(div.children) as HTMLElement[];
                    const tops = all.map((one) => one.getBoundingClientRect().top);
                    const gaps = new Set(tops.slice(1).map((top, at) => Math.round((top - tops[at] - all[at].getBoundingClientRect().height) * 10) / 10));
                    const colors = all.slice(0, 6).map((one) => getComputedStyle(one).color).join(',');
                    return `${all.length}:${all.slice(0, 3).map((one) => one.textContent).join(',')}..${all.slice(-2).map((one) => one.textContent).join(',')}:gaps ${[...gaps].join(',')}:${colors}`;
                };
                const out: string[] = [];
                const steps = [
                    () => { const p = document.createElement('p'); p.textContent = 'new'; div.insertBefore(p, ps[50]); },
                    () => { ps[10].remove(); },
                    () => { ps[30].textContent = 'a paragraph that goes on long enough to take more than one line in a box this wide, and more'; },
                    () => { const p = document.createElement('p'); p.textContent = 'first'; div.insertBefore(p, div.firstChild); },
                    () => { const p = document.createElement('p'); p.textContent = 'last'; div.appendChild(p); },
                ];
                const next = (at: number) => {
                    if (at === steps.length) {
                        const said = Array.from(div.children).map((one) => one.textContent);
                        out.push(`order:${said.indexOf('new') === 50 && said.indexOf('p11') === 11 && !said.includes('p10')}`);
                        setSaid(out.join('|'));
                        return;
                    }
                    steps[at]();
                    // Once it's been drawn and laid out, as the window does.
                    setTimeout(() => {
                        out.push(look());
                        next(at + 1);
                    }, 100);
                };
                setTimeout(() => {
                    out.push(look());
                    next(0);
                }, 20);
            }, []);
            return <div><div ref={ref} style={{ width: 300 }} /><div className="said">{said}</div></div>;
        }
    "#
    .replace("CSS", css);
    open(name, &source)
}

#[test]
fn a_long_list_code_made_draws_and_lays_out_what_changed() {
    let mut hands = long_list("long", "p { margin: 4px 0; color: rgb(0, 0, 255); }");
    hands.wait_for("order:");
    let blue = "rgb(0, 0, 255),".repeat(6);
    let blue = &blue[..blue.len() - 1];
    let want = [
        format!("100:p0,p1,p2..p98,p99:gaps 4:{blue}"),
        format!("101:p0,p1,p2..p98,p99:gaps 4:{blue}"),
        format!("100:p0,p1,p2..p98,p99:gaps 4:{blue}"),
        format!("100:p0,p1,p2..p98,p99:gaps 4:{blue}"),
        format!("101:first,p0,p1..p98,p99:gaps 4:{blue}"),
        format!("102:first,p0,p1..p99,last:gaps 4:{blue}"),
        "order:true".to_owned(),
    ]
    .join("|");
    assert!(hands.says(&want), "wanted {want}\n{}", hands.app.outline());
}

#[test]
fn a_long_list_code_made_keeps_where_each_is_among_the_rest() {
    let mut hands = long_list("nth", "p { margin: 4px 0; } p:nth-child(odd) { color: rgb(255, 0, 0); } p:nth-child(even) { color: rgb(0, 128, 0); }");
    hands.wait_for("order:");
    let (red, green) = ("rgb(255, 0, 0)", "rgb(0, 128, 0)");
    let colors = [red, green, red, green, red, green].join(",");
    let want = [
        format!("100:p0,p1,p2..p98,p99:gaps 4:{colors}"),
        format!("101:p0,p1,p2..p98,p99:gaps 4:{colors}"),
        format!("100:p0,p1,p2..p98,p99:gaps 4:{colors}"),
        format!("100:p0,p1,p2..p98,p99:gaps 4:{colors}"),
        format!("101:first,p0,p1..p98,p99:gaps 4:{colors}"),
        format!("102:first,p0,p1..p99,last:gaps 4:{colors}"),
        "order:true".to_owned(),
    ]
    .join("|");
    assert!(hands.says(&want), "wanted {want}\n{}", hands.app.outline());
}
