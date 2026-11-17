//! What shadcn/ui's form, item, empty, direction, attachment, marker, message,
//! message-scroller and bubble components needed of rsui: forms sent by a button outside them, textareas as elements of the page, what
//! a form library's controls ask of the DOM while they render, and a fieldset's legend apart from its flex layout.

mod hands;

use std::path::PathBuf;

use hands::{Hands, KeyCode};

fn open(name: &str, source: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-rich-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    Hands::open(&file, (500, 400))
}

/// A page (in a `PageWindow`, as the shadcn pages are) whose `Main` is given in `source`, with `css` as its stylesheet.
fn page(name: &str, css: &str, source: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-rich-page-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let app = format!(
        "import {{ PageWindow }} from 'rsui/web';
import React, {{ useEffect, useState }} from 'react';
import './app.css';
{source}
export default function App() {{ return <PageWindow><Main /></PageWindow>; }}
"
    );
    std::fs::write(folder.join("app.tsx"), app).unwrap();
    std::fs::write(folder.join("app.css"), format!("body {{ margin: 0; }}
{css}")).unwrap();
    let mut hands = Hands::open(&folder.join("app.tsx"), (500, 400));
    hands.settle();
    hands
}

/// A submit button outside its form, with `form="id"` (as every shadcn form example's Save is), sends that form: the page hears
/// `submit` (a `SubmitEvent` with its submitter) on the window as well as React's `onSubmit`. Enter in a field sends it by a
/// `click` on that button with `detail` 0, as a browser's implicit submission does, and its reset button with `form` resets it.
#[test]
fn a_button_outside_its_form_sends_it() {
    let mut hands = open(
        "form-attr",
        r#"
        import React, { useEffect, useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState<string[]>([]);
            const tell = (one: string) => setSaid((all) => [...all, one]);
            useEffect(() => {
                window.addEventListener('submit', (event: any) => tell(`win-submit:${event.target.id}:${event.submitter?.id}:${event instanceof SubmitEvent}`), true);
                window.addEventListener('click', (event: any) => { if (event.target.id === 'go') tell(`click:${event.detail}`); }, true);
            }, []);
            return (
                <div>
                    <form id="f" key="form" onSubmit={(event) => { event.preventDefault(); tell('onSubmit'); }} onReset={() => tell('onReset')}>
                        <input id="name" key="name" />
                    </form>
                    <button id="go" key="go" type="submit" form="f">Send</button>
                    <button id="again" key="again" type="reset" form="f">Reset</button>
                    <div key="said">{'said ' + said.join(' ')}</div>
                </div>
            );
        }
        "#,
    );
    hands.settle();
    hands.click("go");
    hands.wait_for("said click:1 win-submit:f:go:true onSubmit");
    hands.click("name");
    hands.press(KeyCode::Enter);
    hands.wait_for("said click:1 win-submit:f:go:true onSubmit click:0 win-submit:f:go:true onSubmit");
    hands.click("again");
    hands.wait_for("onSubmit onReset");
}

/// A `<textarea>` is an element of the page like any other: `querySelector` finds it with its attributes, what's typed in it is
/// heard as `beforeinput` and `input`, React's value is its text, `getComputedStyle` says how it's shown, and with
/// `field-sizing: content` it's as tall as what's in it rather than its `rows`.
#[test]
fn a_textarea_is_an_element_of_the_page() {
    let mut hands = page(
        "textarea",
        ".fit { field-sizing: content; font-size: 14px; line-height: 20px; padding: 0; border: 0 }",
        r#"
        function Main() {
            const [value, setValue] = useState('');
            const [said, setSaid] = useState('');
            const [heard, setHeard] = useState<string[]>([]);
            useEffect(() => {
                window.addEventListener('input', (event: any) => setHeard((all) => [...all, `${event.target.tagName}:${event.inputType}`]), true);
                const timer = setInterval(() => {
                    const found = document.querySelectorAll('textarea');
                    const one = document.getElementById('t') as HTMLTextAreaElement;
                    const fit = document.getElementById('fit') as HTMLTextAreaElement;
                    setSaid(`n=${found.length} slot=${one?.getAttribute('data-slot')} text=${one?.textContent} display=${one && getComputedStyle(one).display} fit=${fit && Math.round(fit.getBoundingClientRect().height)}`);
                }, 50);
                return () => clearInterval(timer);
            }, []);
            return (
                <div>
                    <textarea id="t" data-slot="textarea" rows={6} value={value} onChange={(event) => setValue(event.target.value)} />
                    <textarea id="fit" className="fit" rows={6} placeholder="one line" />
                    <div id="said">{said + ' heard ' + heard.join(' ')}</div>
                </div>
            );
        }
        "#,
    );
    hands.wait_for("n=2 slot=textarea text= display=");
    hands.wait_for("fit=20");
    let (x, y) = hands.middle_of_role(rsui::core::Role::TextInput);
    hands.click_at(x, y);
    hands.type_text("hi");
    hands.wait_for("text=hi");
    hands.wait_for("heard TEXTAREA:insertText TEXTAREA:insertText");
}

/// What a control asks of the DOM while it renders: a component that set its state from its ref renders again before the
/// tree's been changed, and `closest('form')` then finds the form it's in (Radix's switch, checkbox and radio decide by that
/// whether to have a hidden `<input>` for the form), even before anything's been laid out.
#[test]
fn closest_while_rendering_finds_what_its_in() {
    let mut hands = page(
        "closest",
        "",
        r#"
        function Probe() {
            const [node, setNode] = useState<HTMLElement | null>(null);
            const inForm = node === null ? 'none' : String(node.closest('form') !== null);
            return <button id="b" ref={setNode}>{'in form ' + inForm}</button>;
        }
        function Main() {
            return <form id="f"><div><Probe /></div></form>;
        }
        "#,
    );
    hands.wait_for("in form true");
}

/// A checkbox that keeps whether it's checked itself keeps what code set (`input.checked = true` is its dirty state, as Radix's
/// hidden form input is set), its `checked` attribute is still what it started with, and a plain `new Event('click')` ticks
/// nothing (only a mouse click runs what a click does). An `<output>`'s value is its text. `<body>` doesn't scroll in standards
/// mode (`document.scrollingElement` is `<html>`), and a shape in an `<svg>` is `display: inline`.
#[test]
fn dom_details_form_libraries_read() {
    let mut hands = open(
        "dom-details",
        r#"
        import React, { useEffect, useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('');
            useEffect(() => {
                const box = document.getElementById('box') as HTMLInputElement;
                const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'checked')!.set!;
                setter.call(box, true);
                box.dispatchEvent(new Event('click', { bubbles: true }));
                setTimeout(() => {
                    const out = document.createElement('output');
                    out.textContent = 'made';
                    document.body.appendChild(out);
                    const path = document.querySelector('path')!;
                    setSaid(`checked=${box.checked} attr=${box.getAttribute('checked')} out=${out.value} body=${document.body.scrollTop} scrolling=${document.scrollingElement === document.documentElement} path=${getComputedStyle(path).display}`);
                }, 50);
            }, []);
            return (
                <div>
                    <input id="box" key="box" type="checkbox" defaultChecked={false} />
                    <svg key="svg" width="10" height="10"><path d="M0 0L10 10" /></svg>
                    <div key="said">{said}</div>
                </div>
            );
        }
        "#,
    );
    hands.settle();
    hands.wait_for("checked=true attr=null out=made body=0 scrolling=true path=inline");
}

/// A fieldset that's a flex box with a gap lays its legend out apart, above the rest, which is what's flex: only the legend's
/// own margin is between it and what follows, as shadcn's `FieldSet` (`flex flex-col gap-6`, legend `mb-3`) has in a browser.
#[test]
fn a_fieldsets_legend_isnt_in_its_flex_gap() {
    let mut hands = page(
        "legend",
        ".set { display: flex; flex-direction: column; gap: 24px; border: 0; padding: 0; margin: 0 } .legend { margin-bottom: 12px; line-height: 20px; font-size: 14px } .one { height: 10px }",
        r#"
        function Legend({ children }: { children: React.ReactNode }) {
            return <legend id="legend" className="legend">{children}</legend>;
        }
        function Main() {
            const [said, setSaid] = useState('');
            useEffect(() => {
                const timer = setInterval(() => {
                    const legend = document.getElementById('legend')!.getBoundingClientRect();
                    const first = document.getElementById('first')!.getBoundingClientRect();
                    const second = document.getElementById('second')!.getBoundingClientRect();
                    setSaid(`after legend ${Math.round(first.top - legend.bottom)} between ${Math.round(second.top - first.bottom)}`);
                }, 50);
                return () => clearInterval(timer);
            }, []);
            return (
                <div>
                    <fieldset className="set">
                        <Legend>Plan</Legend>
                        <div id="first" className="one" />
                        <div id="second" className="one" />
                    </fieldset>
                    <div id="said">{said}</div>
                </div>
            );
        }
        "#,
    );
    hands.wait_for("after legend 12 between 24");
}

/// Sending a form checks its controls first, as a browser does: an `<input type=email>` holding "bad" hears `invalid`, gets the
/// keyboard, and the form isn't sent (react-hook-form's array example in Edge); `novalidate` skips that, and `validity`,
/// `checkValidity()` and `setCustomValidity()` say the same. Checking one radio of a group unchecks the others.
#[test]
fn a_form_checks_its_controls_before_its_sent() {
    let mut hands = page(
        "validity",
        "",
        r#"
        function Main() {
            const [said, setSaid] = useState<string[]>([]);
            const tell = (one: string) => setSaid((all) => [...all, one]);
            useEffect(() => {
                const mail = document.getElementById('mail') as HTMLInputElement;
                const a = document.getElementById('ra') as HTMLInputElement;
                const b = document.getElementById('rb') as HTMLInputElement;
                const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'checked')!.set!;
                setter.call(a, true);
                setter.call(b, true);
                tell(`radios ${a.checked} ${b.checked}`);
                tell(`check ${(document.getElementById('f') as HTMLFormElement).checkValidity()} ${mail.validity.typeMismatch} ${mail.validationMessage}`);
                (document.getElementById('g') as HTMLFormElement).requestSubmit();
            }, []);
            return (
                <div>
                    <form id="f" onSubmit={(event) => { event.preventDefault(); tell('sent'); }}>
                        <input id="mail" type="email" defaultValue="bad" onInvalid={() => tell('invalid')} />
                        <input id="ra" type="radio" name="r" defaultChecked={false} />
                        <input id="rb" type="radio" name="r" defaultChecked={false} />
                    </form>
                    <form id="g" noValidate onSubmit={(event) => { event.preventDefault(); tell('sent-g'); }}>
                        <input type="email" defaultValue="bad" />
                    </form>
                    <button id="go" type="submit" form="f">Send</button>
                    <button id="go-g" type="submit" form="g">Send g</button>
                    <div>{'said ' + said.join(' | ') + ' focus ' + (document.activeElement?.id ?? '')}</div>
                </div>
            );
        }
        "#,
    );
    hands.wait_for("said radios false true | invalid | check false true Please enter an email address. | sent-g");
    let (x, y) = hands.middle_of_role(rsui::core::Role::Button);
    hands.click_at(x, y);
    hands.wait_for("sent-g | invalid");
    hands.wait_for("focus mail");
    assert!(!hands.says("| sent |") && !hands.says("| sent focus"), "{}", hands.app.outline());
}

/// In right-to-left text a grid's first column is at the right and `justify-self: end` at the left (shadcn's CardHeader in
/// `dir="rtl"`), and a flex item that's `overflow: hidden` can be smaller than what's in it, so `h-full` inside it is its
/// flexed height (the message scroller's viewport in a `flex-1 overflow-hidden` CardContent scrolls rather than grows).
#[test]
fn rtl_grids_and_clipping_flex_items() {
    let mut hands = page(
        "rtl-grid",
        ".grid { display: grid; grid-template-columns: 1fr auto; width: 300px } .end { justify-self: end; width: 50px; height: 10px } \
         .one { height: 10px } .col { display: flex; flex-direction: column; height: 300px } .top { height: 40px } \
         .item { flex: 1 1 0%; overflow: hidden } .full { height: 100%; overflow-y: auto } .tall { height: 600px; flex-shrink: 0 }",
        r#"
        function Main() {
            const [said, setSaid] = useState('');
            useEffect(() => {
                const timer = setInterval(() => {
                    const grid = document.getElementById('grid')!.getBoundingClientRect();
                    const first = document.getElementById('first')!.getBoundingClientRect();
                    const action = document.getElementById('action')!.getBoundingClientRect();
                    const full = document.getElementById('full')!;
                    setSaid(`first ${Math.round(grid.right - first.right)} action ${Math.round(action.left - grid.left)} full ${full.clientHeight}`);
                }, 50);
                return () => clearInterval(timer);
            }, []);
            return (
                <div>
                    <div id="grid" className="grid" dir="rtl"><div id="first" className="one" /><div id="action" className="end" /></div>
                    <div className="col"><div className="top" /><div className="item"><div id="full" className="full"><div className="tall" /></div></div></div>
                    <div>{said}</div>
                </div>
            );
        }
        "#,
    );
    hands.wait_for("first 0 action 0 full 260");
}
