//! What a page's components count on when they're used, which Mantine's kitchen sink showed was missing: a field's caret, the
//! element an `onChange` hears from, what a click went through, the page's own `<html>` and `<body>`, labels, radios' arrow keys,
//! hovering heard by `addEventListener`, and a `<select>` whose options come from a component. Each is used headless, with
//! `--do`, as a person would, in a sandbox of its own.

use std::path::PathBuf;
use std::process::Command;

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn app(name: &str, source: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-page-interaction-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    file
}

/// Does the steps, and says what was printed, failing if a step did or the app said something was wrong.
fn works(file: &PathBuf, steps: &str) -> String {
    let sandbox = file.with_file_name("sandbox");
    let output = Command::new(TOOL)
        .arg("--do")
        .arg(steps)
        .arg(file)
        .env("RSUI_SANDBOX", &sandbox)
        .env("APPDATA", &sandbox)
        .env("LOCALAPPDATA", &sandbox)
        .env("USERPROFILE", &sandbox)
        .env("HOME", &sandbox)
        .env("RSUI_STORAGE_FILE", sandbox.join("storage.json"))
        .env("RSUI_OFFLINE", "1")
        .output()
        .unwrap();
    let said = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "{said}");
    assert!(!said.contains("TypeError"), "{said}");
    said
}

#[test]
fn a_field_has_a_caret_that_code_can_read_and_move() {
    // As react-number-format and Mantine's NumberInput do, from `onChange` and later: without these, NumberInput's steppers threw,
    // and React took the whole app down.
    let file = app(
        "caret",
        r#"
        import { PageWindow } from 'rsui/web';
        import { useRef, useState } from 'react';
        export default function App() {
            const field = useRef(null);
            const [value, setValue] = useState('hello');
            const [said, setSaid] = useState('');
            return (
                <PageWindow>
                    <input id="field" ref={field} value={value} onChange={(event) => {
                        const target = event.target;
                        target.setSelectionRange(target.value.length, target.value.length);
                        setSaid('typed at ' + target.selectionStart + '-' + target.selectionEnd + ' of ' + target.value);
                        setValue(target.value);
                    }} />
                    <button id="check" onClick={() => {
                        const one = field.current;
                        one.setSelectionRange(2, 4, 'backward');
                        const first = [one.selectionStart, one.selectionEnd, one.selectionDirection].join(',');
                        one.select();
                        const all = [one.selectionStart, one.selectionEnd].join(',');
                        one.setRangeText('J', 0, 1);
                        setSaid('range ' + first + ' all ' + all + ' text ' + one.value);
                    }}>check</button>
                    <p>{said}</p>
                </PageWindow>
            );
        }
        "#,
    );
    works(&file, "click #check; expect \"range 2,4,backward all 0,5 text Jello\"; type #field \"abc\"; expect \"typed at 3-3 of abc\"");
}

#[test]
fn a_change_is_heard_from_the_field_itself() {
    // Checkbox.Group and Radio.Group read `event.currentTarget.value`, which is what the box stands for, and not whether it's ticked.
    let file = app(
        "change-target",
        r#"
        import { PageWindow } from 'rsui/web';
        import { useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('');
            const heard = (event) => setSaid(event.currentTarget.value + ' ' + event.currentTarget.checked + ' ' + (event.target === event.currentTarget));
            return (
                <PageWindow>
                    <input type="checkbox" id="svelte" value="svelte" onChange={heard} /><label htmlFor="svelte">Svelte</label>
                    <input type="radio" id="letter-b" name="letter" value="b" onChange={heard} /><label htmlFor="letter-b">Letter B</label>
                    <label><input type="checkbox" value="vue" onChange={(event) => setSaid('wrapped ' + event.target.value + ' ' + event.target.checked)} /> Vue</label>
                    <p>{said}</p>
                </PageWindow>
            );
        }
        "#,
    );
    works(&file, "click \"Svelte\"; expect \"svelte true true\"; click \"Letter B\"; expect \"b true true\"; click \"Vue\"; expect \"wrapped vue true\"");
}

#[test]
fn a_click_says_what_it_went_through_and_the_page_is_made_of_elements() {
    // Mantine's click-outside looks for its dropdown in `composedPath()`, and closed every dropdown when it was empty; Floating UI
    // takes `<html>` and `<body>` apart to know where a dropdown goes, and came to NaN.
    let file = app(
        "path",
        r#"
        import { PageWindow } from 'rsui/web';
        import { useEffect, useRef, useState } from 'react';
        import { createPortal } from 'react-dom';
        export default function App() {
            const box = useRef(null);
            const [said, setSaid] = useState('');
            useEffect(() => {
                const heard = (event) => setSaid('in box ' + event.composedPath().includes(box.current) + ', ends ' + event.composedPath().slice(-4).map((one) => one === window ? 'window' : one === document ? 'document' : one.nodeName).join(' '));
                document.addEventListener('mousedown', heard);
                return () => document.removeEventListener('mousedown', heard);
            }, []);
            const html = document.documentElement;
            const page = [html instanceof HTMLElement, document.body instanceof HTMLElement, html.ownerDocument === document, html.clientLeft, getComputedStyle(html).position, getComputedStyle(document.body).overflow].join(',');
            return (
                <PageWindow>
                    {createPortal(<div ref={box} style={{ position: 'absolute', top: 10, left: 300 }}><span>Inside</span></div>, document.body)}
                    <p>{'page ' + page}</p>
                    <button>Outside</button>
                    <p>{said}</p>
                </PageWindow>
            );
        }
        "#,
    );
    works(
        &file,
        "expect \"page true,true,true,0,static,visible\"; click \"Inside\"; expect \"in box true, ends BODY HTML document window\"; click \"Outside\"; expect \"in box false\"",
    );
}

#[test]
fn a_label_gives_its_field_the_keyboard_and_radios_go_on_with_the_arrows() {
    let file = app(
        "labels",
        r#"
        import { PageWindow } from 'rsui/web';
        import { useState } from 'react';
        export default function App() {
            const [text, setText] = useState('');
            const [letter, setLetter] = useState('a');
            return (
                <PageWindow>
                    <label htmlFor="name">Name</label>
                    <input id="name" value={text} onChange={(event) => setText(event.target.value)} />
                    <p>{'text ' + text}</p>
                    {['a', 'b', 'c'].map((one) => (
                        <span key={one}>
                            <input type="radio" id={'letter-' + one} name="letter" value={one} checked={letter === one} onChange={(event) => setLetter(event.currentTarget.value)} />
                            <label htmlFor={'letter-' + one}>{'Letter ' + one}</label>
                        </span>
                    ))}
                    <p>{'letter ' + letter}</p>
                </PageWindow>
            );
        }
        "#,
    );
    works(&file, "click \"Name\"; write \"via label\"; expect \"text via label\"; click \"Letter b\"; expect \"letter b\"; press right; expect \"letter c\"; press right; expect \"letter a\"; press left; expect \"letter c\"");
}

#[test]
fn hovering_is_heard_by_what_listens_with_add_event_listener() {
    // Floating UI's `useHover`, which Mantine's Tooltip uses, listens for `mouseleave` on the element itself.
    let file = app(
        "hover",
        r#"
        import { PageWindow } from 'rsui/web';
        import { useEffect, useRef, useState } from 'react';
        export default function App() {
            const target = useRef(null);
            const [said, setSaid] = useState('none');
            useEffect(() => {
                const one = target.current;
                const enter = () => setSaid('entered');
                const leave = () => setSaid('left');
                one.addEventListener('mouseenter', enter);
                one.addEventListener('mouseleave', leave);
                return () => {
                    one.removeEventListener('mouseenter', enter);
                    one.removeEventListener('mouseleave', leave);
                };
            }, []);
            return (
                <PageWindow>
                    <div ref={target} style={{ padding: 20 }}>Hover here</div>
                    <div style={{ padding: 20 }}>Somewhere else</div>
                    <p>{'heard ' + said}</p>
                </PageWindow>
            );
        }
        "#,
    );
    works(&file, "hover \"Hover here\"; expect \"heard entered\"; hover \"Somewhere else\"; expect \"heard left\"");
}

#[test]
fn a_select_takes_options_from_a_component_and_the_arrow_keys() {
    // Mantine's NativeSelect makes each `<option>` with a component of its own.
    let file = app(
        "native-select",
        r#"
        import { PageWindow } from 'rsui/web';
        import { useState } from 'react';
        function Choice({ data }) {
            return <option value={data.value}>{data.label}</option>;
        }
        export default function App() {
            const [value, setValue] = useState('b');
            return (
                <PageWindow>
                    <select id="pick" value={value} onChange={(event) => setValue(event.currentTarget.value)}>
                        {[{ value: 'a', label: 'Apple' }, { value: 'b', label: 'Banana' }, { value: 'c', label: 'Cherry' }].map((one) => <Choice key={one.value} data={one} />)}
                    </select>
                    <p>{'chose ' + value}</p>
                </PageWindow>
            );
        }
        "#,
    );
    works(&file, "expect \"Banana\"; press down #pick; expect \"chose c\"; press up; press up; expect \"chose a\"");
}
