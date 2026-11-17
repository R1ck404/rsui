//! Core DOM causes found by the shadcn parity helpers: each part of a press as its own complete dispatch in a
//! browser's order, timers waiting until an event's been everywhere, focus when what has it is taken out, and what a field says
//! about its selection as it's typed in.

mod hands;

use std::path::PathBuf;

use hands::{Hands, KeyCode, KeyPress};

fn open(name: &str, source: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-core-events-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    let mut hands = Hands::open(&file, (500, 400));
    hands.settle();
    hands
}

/// `pointerdown` goes all the way down and back up before `mousedown` starts, and so on for up and click, each its own object
/// whatever hears it, with React's capture handlers before what's listening on the element and its bubble handlers after, as
/// on a page where React listens at its root. `pointerdown` says no clicks, `mousedown` one, and a `mousedown` has no `pointerId`.
#[test]
fn each_part_of_a_press_is_its_own_dispatch() {
    let mut hands = open(
        "press",
        r#"
        import React, { useEffect, useRef, useState } from 'react';
        const SHORT = { pointerdown: 'pd', mousedown: 'md', pointerup: 'pu', mouseup: 'mu', click: 'ck' };
        export default function App() {
            const [said, setSaid] = useState('');
            const heard = useRef([]);
            const objects = useRef(new Map());
            const add = (where, event) => {
                const map = objects.current;
                if (!map.has(event)) map.set(event, map.size + 1);
                let line = SHORT[event.type] + ':' + where;
                if (where === 'dc') line += '#' + map.get(event) + 'd' + event.detail + (event.pointerId === undefined ? '' : 'p');
                heard.current.push(line);
            };
            useEffect(() => {
                const box = document.getElementById('b');
                for (const type of Object.keys(SHORT)) {
                    document.addEventListener(type, (event) => event.target === box && add('dc', event), true);
                    box.addEventListener(type, (event) => add('n', event));
                    document.addEventListener(type, (event) => event.target === box && add('db', event));
                }
            }, []);
            const r = (where) => (event) => add(where, event.nativeEvent);
            return (
                <div>
                    <button key="b" id="b" onPointerDownCapture={r('rc')} onMouseDownCapture={r('rc')} onPointerDown={r('rb')} onMouseDown={r('rb')}
                        onPointerUp={r('rb')} onMouseUp={r('rb')} onClickCapture={r('rc')} onClick={r('rb')}>press</button>
                    <button key="show" onClick={() => { setSaid('said ' + heard.current.join(' ')); heard.current = []; objects.current = new Map(); }}>show</button>
                    <div>{said}</div>
                </div>
            );
        }
        "#,
    );
    hands.click("b");
    hands.click("show");
    hands.wait_for(
        "said pd:dc#1d0p pd:rc pd:n pd:rb pd:db md:dc#2d1 md:rc md:n md:rb md:db pu:dc#3d0p pu:n pu:rb pu:db mu:dc#4d1 mu:n mu:rb mu:db \
         ck:dc#5d1p ck:rc ck:n ck:rb ck:db",
    );
}

/// A timer set as a click comes down (Radix's DismissableLayer sets one in its capture listener) goes off only once the click's
/// been back up to `document`, even when what was clicked has handlers of its own in between, as a page's timers never run in the
/// middle of an event: Radix took the click as stopped, and a popover never closed on a click outside.
#[test]
fn timers_wait_until_the_click_is_back_up() {
    let mut hands = open(
        "timers",
        r#"
        import React, { useEffect, useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('waiting');
            useEffect(() => {
                let bubbled = false;
                document.addEventListener('click', () => { bubbled = false; setTimeout(() => setSaid('said bubbled=' + bubbled)); }, true);
                document.addEventListener('click', () => { bubbled = true; });
            }, []);
            return <div><button key="b" onClick={() => {}} onPointerDown={() => {}}>b</button><div key="plain" style={{ width: 100, height: 40 }} /><div>{said}</div></div>;
        }
        "#,
    );
    hands.click("b");
    hands.wait_for("said bubbled=true");
    hands.click("plain");
    hands.wait_for("said bubbled=true");
}

/// What has the keyboard hears `blur` and `focusout` when it's taken out, or remounted with another key, and the focus is then
/// `<body>`'s: a remounted element is a new element.
#[test]
fn taking_out_what_has_the_keyboard_blurs_it() {
    let mut hands = open(
        "remount",
        r#"
        import React, { useEffect, useRef, useState } from 'react';
        export default function App() {
            const [n, setN] = useState(0);
            const [said, setSaid] = useState('');
            const heard = useRef([]);
            useEffect(() => {
                for (const type of ['blur', 'focusout']) {
                    window.addEventListener(type, () => heard.current.push(type), true);
                }
            }, []);
            return (
                <div>
                    <button key={'k' + n} id="k" onClick={() => setN(n + 1)}>keyed {n}</button>
                    <button key="show" onMouseDown={(event) => event.preventDefault()}
                        onClick={() => setSaid('said ' + heard.current.join(' ') + ' active=' + document.activeElement.tagName)}>show</button>
                    <div>{said}</div>
                </div>
            );
        }
        "#,
    );
    hands.click("k0");
    hands.click("show");
    hands.wait_for("said blur focusout active=BODY");
}

/// `maxLength` stops typing, and each time the caret moves the field is sent `selectionchange`, whose listener can select the
/// character after the caret for the next key to type over, as input-otp's one field for all its digits does.
#[test]
fn a_field_keeps_to_max_length_and_says_selectionchange() {
    let mut hands = open(
        "otp",
        r#"
        import React, { useEffect, useRef, useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('');
            useEffect(() => {
                let changes = 0;
                document.addEventListener('selectionchange', () => {
                    changes += 1;
                    const f = document.getElementById('f');
                    if (document.activeElement === f && f.selectionStart === f.selectionEnd && f.selectionStart === f.maxLength) {
                        f.setSelectionRange(f.maxLength - 1, f.maxLength, 'backward');
                    }
                    setTimeout(() => setSaid('said ' + f.value + ' ' + f.selectionStart + '-' + f.selectionEnd + ' ' + f.selectionDirection + ' changes=' + (changes > 0)));
                }, true);
            }, []);
            return <div><input key="f" id="f" maxLength={3} defaultValue="" /><div>{said}</div></div>;
        }
        "#,
    );
    hands.click("f");
    hands.type_text("123");
    hands.wait_for("said 123 2-3 backward changes=true");
    // The next digit types over the last, which is selected.
    hands.type_text("9");
    hands.wait_for("said 129 2-3 backward");
    // Moved to the end, it's selected again, and typed over again.
    hands.press_key(KeyPress::new(KeyCode::End));
    hands.type_text("5");
    hands.wait_for("said 125 2-3 backward");
}

/// A field takes no more than its `maxLength` as it's typed in.
#[test]
fn typing_stops_at_max_length() {
    let mut hands = open(
        "max-length",
        r#"
        import React, { useState } from 'react';
        export default function App() {
            const [value, setValue] = useState('');
            return <div><input key="f" maxLength={2} value={value} onChange={(event) => setValue(event.target.value)} /><div>{'said ' + value + '.'}</div></div>;
        }
        "#,
    );
    hands.click("f");
    hands.type_text("abc");
    hands.wait_for("said ab.");
}

/// `body` and `html` can be asked `matches()` and `closest()`, as Base UI's `closest(target, 'a')` asks of everything up from what
/// was clicked, which threw, so a combobox item was never selected.
#[test]
fn body_and_html_match_selectors() {
    let mut hands = open(
        "matches",
        r#"
        import React, { useEffect, useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('');
            useEffect(() => {
                const b = document.body, h = document.documentElement;
                setSaid('said ' + [b.matches('body'), b.matches('a'), h.matches(':root'), h.matches('html, a'), b.closest('html') === h,
                    String(b.closest('a')), document.nodeName].join(' '));
            }, []);
            return <div>{said}</div>;
        }
        "#,
    );
    hands.wait_for("said true false true true true null #document");
}

/// A component made anew each render, as shadcn's Calendar makes its `Root` inside its props, is another kind each time, and what
/// it drew is new elements, which don't keep the focus: while files are watched, when components are told apart by name, as much
/// as otherwise. Only a new version of the code keeps what's there.
#[test]
fn a_component_made_each_render_draws_new_elements() {
    rsui_tsx::keep_state_across_reloads();
    let mut hands = open(
        "inline-kind",
        r#"
        import React, { useState } from 'react';
        export default function App() {
            const [n, setN] = useState(0);
            const [said, setSaid] = useState('');
            const Inner = () => <button key="in" onClick={() => setN(n + 1)}>in {n}</button>;
            return (
                <div>
                    <Inner />
                    <button key="show" onMouseDown={(event) => event.preventDefault()} onClick={() => setSaid('said ' + document.activeElement.tagName)}>show</button>
                    <div>{said}</div>
                </div>
            );
        }
        "#,
    );
    hands.click("in");
    hands.click("show");
    hands.wait_for("said BODY");
}
