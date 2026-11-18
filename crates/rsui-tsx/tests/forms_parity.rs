//! Text fields and the text around them as a browser has them, found with shadcn/ui's components (input, textarea,
//! label, field, input-group, native-select): the events typing sends, where the caret and selection are, what's an attribute,
//! and text that keeps its own font.

mod hands;

use std::path::PathBuf;

use hands::{Hands, KeyCode, KeyPress};

fn open(name: &str, source: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-forms-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    Hands::open(&file, (500, 400))
}

fn ctrl(code: KeyCode) -> KeyPress {
    let mut key = KeyPress::new(code);
    key.ctrl = true;
    key
}

fn shift(code: KeyCode) -> KeyPress {
    let mut key = KeyPress::new(code);
    key.shift = true;
    key
}

/// Typing, deleting and undoing in an `<input>` is heard as `beforeinput` and then `input`, with the `inputType` and `data` a
/// browser gives, by listeners on the window; selecting in it is heard as `select`; and `selectionStart`, `selectionEnd` and
/// `selectionDirection` say where Rust's caret is, as keys move it. A `beforeinput` that's stopped changes nothing.
#[test]
fn typing_in_a_field_sends_beforeinput_input_and_select() {
    let mut hands = open(
        "events",
        r#"
        import React, { useEffect, useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('');
            useEffect(() => {
                const heard: string[] = [];
                const tell = () => {
                    const field = document.activeElement as HTMLInputElement;
                    setSaid(`${heard.join(' ')} | ${field.value} ${field.selectionStart}-${field.selectionEnd} ${field.selectionDirection} attr=${field.getAttribute('value')}`);
                };
                for (const type of ['beforeinput', 'input', 'select']) {
                    window.addEventListener(type, (event: any) => {
                        heard.push(type === 'select' ? 'select' : `${type}:${event.inputType}:${event.data ?? ''}`);
                        if (type === 'beforeinput' && event.target.id === 'stop') {
                            event.preventDefault();
                        }
                        setTimeout(tell);
                    }, true);
                }
                window.addEventListener('keyup', () => setTimeout(tell), true);
            }, []);
            return (
                <div>
                    <input id="f" key="f" />
                    <input id="stop" key="stop" />
                    <div>{said}</div>
                </div>
            );
        }
        "#,
    );
    hands.settle();
    hands.click("f");
    hands.type_text("a");
    hands.type_text("b");
    hands.press_key(KeyPress::new(KeyCode::Left));
    hands.wait_for("| ab 1-1 forward attr=null");
    assert!(hands.says("beforeinput:insertText:a input:insertText:a beforeinput:insertText:b input:insertText:b |"), "{}", hands.app.outline());
    hands.press_key(shift(KeyCode::Right));
    hands.wait_for("| ab 1-2 forward");
    hands.press_key(shift(KeyCode::Home));
    hands.wait_for("| ab 0-1 backward");
    assert!(hands.says("select select |"), "{}", hands.app.outline());
    hands.press_key(KeyPress::new(KeyCode::End));
    hands.press_key(KeyPress::new(KeyCode::Backspace));
    hands.wait_for("beforeinput:deleteContentBackward: input:deleteContentBackward: | a 1-1");
    hands.press_key(ctrl(KeyCode::Character('z')));
    hands.wait_for("input:historyUndo: | ab");
    hands.press_key(ctrl(KeyCode::Character('a')));
    hands.wait_for("select | ab 0-2 forward");
    hands.click("stop");
    hands.type_text("x");
    hands.press_key(KeyPress::new(KeyCode::Left));
    // What the stopped field holds is nothing, which the text shows as one space.
    hands.wait_for("beforeinput:insertText:x | 0-0 forward");
    assert!(!hands.says(" input:insertText:x"), "{}", hands.app.outline());
}

/// A block's own font is what its text is written in, even when it's all there is in a `<div>`, and a `line-height` that's a
/// `calc()` of numbers alone is times the font's size, as Tailwind's `text-2xl` says it.
#[test]
fn a_heading_alone_in_a_div_keeps_its_font_and_its_line_height() {
    let mut hands = open(
        "heading",
        r#"
        import React, { useEffect, useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('');
            useEffect(() => {
                const h1 = document.querySelector('h1')!;
                setSaid(`height=${Math.round(h1.getBoundingClientRect().height)}`);
            }, []);
            return (
                <div>
                    <div>
                        <h1 style={{ fontSize: 24, fontWeight: 700, lineHeight: 'calc(2 / 1.5)', margin: 0 }}>Literal</h1>
                    </div>
                    <p>{said}</p>
                </div>
            );
        }
        "#,
    );
    hands.wait_for("height=");
    assert!(hands.says("height=32"), "{}", hands.app.outline());
    // The heading's text is its own, not a piece of text inside it written in what's around it.
    let outline = hands.app.outline();
    let line = outline.lines().find(|line| line.contains("\"Literal\"")).unwrap();
    assert!(line.contains("dom_key"), "{outline}");
}

/// A `<textarea>` is an element of the page, found by selectors and matched by CSS's states as an `<input>` is; with
/// `field-sizing: content` it's as tall as its lines; End on a line that wraps leaves the caret at that line's end, so Down
/// goes to the next line, not the one after; and Ctrl+Backspace at the start of a line takes only the line break.
#[test]
fn a_textarea_is_a_page_element_that_grows_with_its_lines() {
    let mut hands = open(
        "textarea",
        r#"
        import React, { useEffect, useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('');
            useEffect(() => {
                let told = 0;
                const tell = () => setTimeout(() => {
                    told += 1;
                    const area = document.querySelector('textarea') as HTMLTextAreaElement | null;
                    setSaid(area === null ? 'none' : `told=${told} h=${Math.round(area.getBoundingClientRect().height)} at=${area.selectionStart} v=${JSON.stringify(area.value)}`);
                });
                window.addEventListener('keyup', tell, true);
                window.addEventListener('input', tell, true);
                tell();
            }, []);
            return (
                <div>
                    <textarea key="area" id="area" style={{ fieldSizing: 'content', width: 120, minHeight: 20, padding: 0, border: 'none', font: '16px monospace', lineHeight: '20px' }} />
                    <p>{said}</p>
                </div>
            );
        }
        "#,
    );
    hands.click("area");
    hands.press_key(KeyPress::new(KeyCode::End));
    hands.wait_for("h=20 at=0");
    hands.press(KeyCode::Enter);
    hands.press(KeyCode::Enter);
    hands.wait_for("at=2");
    // Three lines of 20 px, as `field-sizing: content` makes it.
    let rect = hands.rect_of("area");
    assert!((rect[3] - 60.0).abs() < 1.0, "{rect:?}\n{}", hands.app.outline());
    hands.press_key(ctrl(KeyCode::Character('a')));
    hands.press(KeyCode::Backspace);
    // Two lines of text that wraps: the first line ends after "aaaa ", where the second starts.
    hands.type_text("aaaa bbbb cccc dddd eeee ffff");
    hands.wait_for("h=");
    press_and_hear(&mut hands, ctrl(KeyCode::Home));
    press_and_hear(&mut hands, KeyPress::new(KeyCode::End));
    let first_end = said_number(&hands, " at=");
    press_and_hear(&mut hands, KeyPress::new(KeyCode::Down));
    let below = said_number(&hands, " at=");
    // Down from the end of the first line lands at the end of the second (or on it), never further than the line after.
    press_and_hear(&mut hands, KeyPress::new(KeyCode::Down));
    let further = said_number(&hands, " at=");
    assert!(first_end > 0 && below > first_end && further >= below, "{first_end} {below} {further}\n{}", hands.app.outline());
    hands.press_key(ctrl(KeyCode::Character('a')));
    hands.type_text("one");
    hands.press(KeyCode::Enter);
    hands.type_text("two");
    hands.press_key(KeyPress::new(KeyCode::Home));
    hands.press_key(ctrl(KeyCode::Backspace));
    hands.wait_for(r#"v=\"onetwo\""#);
}

/// Presses a key and waits until the app has said where the caret is since, which it does from a `setTimeout` its `keyup`
/// listener starts. That timer comes back from the timer thread, so it may be a few frames later: reading what's said straight
/// after the key would read what the key before it left, which it did, as it raced that thread.
fn press_and_hear(hands: &mut Hands, key: KeyPress) {
    let before = said_number(hands, "told=");
    hands.press_key(key);
    let started = std::time::Instant::now();
    while said_number(hands, "told=") <= before {
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "nothing said after {key:?}
{}", hands.app.outline());
        hands.app.runtime().wait(std::time::Duration::from_millis(5));
        hands.settle();
    }
}

fn said_number(hands: &Hands, after: &str) -> usize {
    let outline = hands.app.outline();
    let at = outline.find(after).unwrap_or_else(|| panic!("no {after} in\n{outline}")) + after.len();
    outline[at..].chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap()
}

/// What a browser sends for clicks: a press on what takes no clicks itself, as a textarea, says how many clicks in a row it
/// was and is followed by `dblclick` on the second, heard by `window` on the way down; a label's click passes its count on; a
/// `<button>` with no handler is clicked by Enter; and a control that can't be used hears the pointer but not the mouse.
#[test]
fn clicks_say_how_many_and_reach_the_window() {
    let mut hands = open(
        "clicks",
        r#"
        import React, { useEffect, useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('');
            useEffect(() => {
                const heard: string[] = [];
                for (const type of ['click', 'dblclick', 'mousedown', 'pointerdown', 'focusin', 'keydown']) {
                    window.addEventListener(type, (event: any) => {
                        heard.push(`${type}:${event.target.id || event.target.localName}:${event.detail}`);
                        setTimeout(() => setSaid(heard.join(' ')));
                    }, true);
                }
            }, []);
            return (
                <div>
                    <textarea key="area" id="area" style={{ width: 200, height: 60 }} />
                    <label key="label" id="label" htmlFor="area">Label</label>
                    <button key="plain" id="plain">Plain</button>
                    <button key="off" id="off" disabled style={{ width: 80, height: 30 }}>Off</button>
                    <p>{said}</p>
                </div>
            );
        }
        "#,
    );
    hands.settle();
    // Tab goes to the textarea, then past the label to the button, and Enter clicks that with no clicks in a row.
    hands.press(KeyCode::Tab);
    hands.press(KeyCode::Tab);
    hands.press(KeyCode::Enter);
    hands.wait_for("keydown:plain:0 click:plain:0");
    let (x, y) = hands.middle_of("area");
    hands.app.pointer_move(x, y);
    hands.app.pointer_down(x, y, 1);
    let _ = hands.app.pointer_up(x, y);
    hands.app.pointer_down(x, y, 2);
    let _ = hands.app.pointer_up(x, y);
    hands.wait_for("click:area:2 dblclick:area:2");
    hands.click("label");
    hands.wait_for("click:label:1 focusin:area:0 click:area:1");
    hands.click("off");
    hands.wait_for("pointerdown:off:");
    hands.settle();
    assert!(!hands.says("mousedown:off") && !hands.says("click:off"), "{}", hands.app.outline());
}

/// Tab to a page's field below what it's scrolled in puts it in the middle, as Chromium does, and once that's stopped
/// scrolling `scrollend` is heard.
#[test]
fn tab_puts_a_field_below_in_the_middle_and_scrollend_follows() {
    let mut hands = open(
        "scrollend",
        r#"
        import React, { useEffect, useRef, useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('');
            const area = useRef<HTMLDivElement>(null);
            useEffect(() => {
                const box = area.current!;
                box.addEventListener('scrollend', () => setSaid(`end y=${Math.round(box.scrollTop)}`));
            }, []);
            return (
                <div>
                    <p>{said}</p>
                    <div key="area" ref={area} style={{ height: 300, overflow: 'auto' }}>
                        <input key="first" id="first" style={{ display: 'block', height: 20, padding: 0, border: 'none', marginTop: 30 }} />
                        <input key="far" id="far" style={{ display: 'block', height: 20, padding: 0, border: 'none', marginTop: 1000 }} />
                        <div style={{ height: 1000 }} />
                    </div>
                </div>
            );
        }
        "#,
    );
    hands.click("first");
    hands.press(KeyCode::Tab);
    // The field's middle is in the middle of the 300 px area: about 1050 + 10 - 150.
    hands.wait_for("end y=");
    let y = said_number(&hands, "end y=");
    assert!((880..=940).contains(&y), "{y}\n{}", hands.app.outline());
}

/// A page's `<select>` is an element with its options in the DOM (an optgroup's too), `value`, `selectedIndex`, `options`,
/// `selectedOptions` and `disabled` as a browser has them (their text is checked by the shadcn parity runs, as `textContent`
/// loads `rsui/dom`, which a debug build can't parse); on the closed select the arrows choose the next option that can be
/// chosen (skipping a disabled one, nothing past the end) and typing chooses by how they start, each heard as `input` and then
/// `change`, as Chromium does on Windows.
#[test]
fn a_select_is_a_page_element_and_its_keys_choose() {
    let mut hands = open(
        "select",
        r#"
        import React, { useEffect, useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('');
            useEffect(() => {
                const s = document.getElementById('s') as HTMLSelectElement;
                const heard: string[] = [];
                const tell = () => setSaid(`${heard.join(' ')} v=${s.value} i=${s.selectedIndex} n=${s.options.length} t=${s.selectedOptions[0]?.value} g=${document.querySelectorAll('optgroup').length} d=${s.options[2].disabled}`);
                for (const type of ['input', 'change']) {
                    s.addEventListener(type, () => { heard.push(type); setTimeout(tell); });
                }
                s.focus();
                tell();
            }, []);
            return (
                <div>
                    <select id="s" key="s">
                        <option value="">Pick</option>
                        <optgroup label="G">
                            <option value="a">Apple</option>
                            <option value="b" disabled>Banana</option>
                            <option value="c">Cherry</option>
                        </optgroup>
                    </select>
                    <p>{said}</p>
                </div>
            );
        }
        "#,
    );
    hands.wait_for("v= i=0 n=4 t= g=1 d=true");
    hands.press(KeyCode::Down);
    hands.wait_for("input change v=a i=1");
    hands.press(KeyCode::Down);
    hands.wait_for("input change input change v=c i=3 n=4 t=c");
    hands.press(KeyCode::Down);
    hands.press(KeyCode::Home);
    hands.wait_for("input change input change input change v= i=0");
    hands.press(KeyCode::Character('a'));
    hands.wait_for("v=a i=1 n=4 t=a");
    assert!(!hands.says("input change input change input change input change input change"), "{}", hands.app.outline());
}

/// Code that asks about an element straight after React's put it in, before it's drawn, finds what it's in, as Radix's Select
/// does with its trigger's `closest('form')`; an `<svg>`'s shapes have boxes, from their geometry through the `viewBox`; Escape
/// in a page's field keeps the keyboard there; and a fieldset's legend that a component makes is out of the fieldset's `gap`.
#[test]
fn elements_are_where_react_put_them_and_shapes_have_boxes() {
    let mut hands = open(
        "unplaced",
        r#"
        import React, { useEffect, useState } from 'react';
        function Early() {
            const [element, setElement] = useState<HTMLElement | null>(null);
            return <button ref={setElement} key="early">{element ? `form=${!!element.closest('form')}` : 'none'}</button>;
        }
        function Legend(props: any) {
            return <legend style={{ margin: 0, marginBottom: 12, padding: 0, fontSize: 16, lineHeight: '24px' }} {...props} />;
        }
        export default function App() {
            const [said, setSaid] = useState('');
            const [typed, setTyped] = useState('');
            useEffect(() => {
                setTimeout(() => {
                    const path = document.querySelector('path')!.getBoundingClientRect();
                    const svg = document.querySelector('svg')!.getBoundingClientRect();
                    const fieldset = document.querySelector('fieldset')!.getBoundingClientRect();
                    const p = document.querySelector('fieldset p')!.getBoundingClientRect();
                    setSaid(`path=${Math.round(path.x - svg.x)},${Math.round(path.y - svg.y)},${Math.round(path.width)},${Math.round(path.height)} p=${Math.round(p.y - fieldset.y)}`);
                }, 100);
            }, []);
            return (
                <div>
                    <form><div><Early /></div></form>
                    <svg width="48" height="48" viewBox="0 0 24 24"><path d="M20 6 9 17l-5-5" /></svg>
                    <fieldset style={{ display: 'flex', flexDirection: 'column', gap: 24, border: 0, padding: 0, margin: 0 }}>
                        <Legend>Payment</Legend>
                        <p style={{ margin: 0, lineHeight: '20px' }}>one</p>
                    </fieldset>
                    <input key="field" value={typed} onChange={(event) => setTyped(event.target.value)} />
                    <p>{said} typed={typed}</p>
                </div>
            );
        }
        "#,
    );
    hands.wait_for("form=true");
    // The tick's box is 16 by 11 units, at 4,6, twice as big.
    hands.wait_for("path=8,12,32,22");
    // The legend's 24 and its 12 below, and no gap after it.
    hands.wait_for("p=36");
    hands.click("field");
    hands.press(KeyCode::Escape);
    hands.type_text("x");
    hands.wait_for("typed=x");
}
