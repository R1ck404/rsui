//! Page elements code made, drawn without a `Styled` of their own (`drawNative` in web.js): each page here is drawn
//! twice, with that on and with `globalThis.__rsuiNativeElements = false`, and what's drawn, where, and what the page's code finds
//! out about it has to be the same.

mod hands;

use std::fmt::Write as _;
use std::path::PathBuf;

use hands::Hands;
use rsui::core::{Node, PropValue};

/// A page whose `<div class="page">` has `body` put in it by code, with `css` in a `<style>`, then `script` run (an async function's
/// body, which has `root`, `style`, `out`, `later()` and `natives()`), and `done` and what `out` has said when it's finished.
fn page(name: &str, native: bool, css: &str, body: &str, script: &str) -> Hands {
    let source = r#"
        import React, { useEffect, useRef, useState } from 'react';
        OFF
        const style = document.createElement('style');
        style.textContent = CSS;
        document.head.appendChild(style);
        export default function App() {
            const ref = useRef<HTMLDivElement>(null);
            const [said, setSaid] = useState('');
            useEffect(() => {
                const root = ref.current!;
                root.innerHTML = BODY;
                const out: string[] = [];
                // Once what's been changed has been drawn and laid out, as the window does. It's a while, as drawing it all again after the CSS
                // changes takes long enough in a debug build for what's asked about too soon to find it still being laid out.
                const later = () => new Promise((done) => setTimeout(done, 400));
                // How many of the page's elements are drawn without a `Styled` of their own.
                const natives = () => Array.from(root.querySelectorAll('*')).filter((one: any) => one.__nativeLive !== undefined).length;
                // Where it's got to, for a test that doesn't finish to say.
                const at = (what: string) => setSaid('at ' + what + ' ' + String((globalThis as any).__rsuiNativeElements));
                const run = async () => {
                    // Once what's been put in has been drawn.
                    await new Promise((done) => setTimeout(done, 80));
                    SCRIPT
                };
                run().then(() => setSaid('done ' + out.join('|')), (error) => setSaid('done failed ' + error));
            }, []);
            return <div><div ref={ref} className="page" /><div className="said">{said}</div></div>;
        }
    "#
    .replace("OFF", if native { "" } else { "(globalThis as any).__rsuiNativeElements = false;" })
    .replace("CSS", &format!("{css:?}"))
    .replace("BODY", &format!("{body:?}"))
    .replace("SCRIPT", script);
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-native-{}-{name}-{native}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    let mut hands = Hands::open(&file, (480, 2400));
    hands.wait_for("done ");
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
            let _ = write!(
                out,
                " {:.2},{:.2},{:.2},{:.2}",
                rect.x, rect.y, rect.width, rect.height
            );
        }
        if !node.name.is_empty() {
            let _ = write!(out, " {:?}", node.name.as_str());
        }
        // Its key on the page is its own key when it's drawn without a `Styled` (see `nativeHost` in web.js), and a `dom_key` prop
        // under the key of the component that drew it when it's drawn by one. Rust finds it by either (`dom_key` in geometry.rs), so
        // it's said the same way for both, and a code-made node's component's key (`__domn` and a number) isn't said.
        let key = node.key.as_ref().map(|key| key.to_string());
        let page_key = key
            .as_deref()
            .filter(|key| key.starts_with("__dom") && !key.starts_with("__domn"));
        let dom = match node.props.get("dom_key") {
            Some(PropValue::Str(text)) => Some(text.as_str().to_owned()),
            _ => page_key.map(str::to_owned),
        };
        if let Some(key) = key.as_deref().filter(|key| !key.starts_with("__dom")) {
            let _ = write!(out, " key={}", plain(key));
        }
        if let Some(dom) = dom {
            let _ = write!(out, " dom={}", plain(&dom));
        }
        let mut props: Vec<String> = node
            .props
            .iter()
            .filter(|(name, _)| name.as_str() != "dom_key")
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
    walk(
        hands.app.runtime().root(),
        &hands.app.layout(),
        &mut Vec::new(),
        &mut out,
    );
    // What the page says it found out is compared on its own, as it says how many were drawn this way.
    out.lines()
        .filter(|line| !line.contains("\"done "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the page said when it was done, after `done `.
fn said(hands: &Hands) -> String {
    let outline = hands.app.outline();
    let at = outline.find("done ").expect("it's done");
    // It's quoted, with the props of what says it after it.
    outline[at + 5..]
        .split('"')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// The same page drawn both ways: what's drawn has to be the same, line for line. Gives what each said.
fn both(name: &str, css: &str, body: &str, script: &str) -> (String, String) {
    // One app at a time on a thread, as each has the thread's engine.
    let (a, said_on) = {
        let on = page(name, true, css, body, script);
        (dump(&on), said(&on))
    };
    let (b, said_off) = {
        let off = page(name, false, css, body, script);
        (dump(&off), said(&off))
    };
    if a != b {
        // Both in full, to compare.
        let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        let _ = std::fs::write(folder.join(format!("native-{name}-on.txt")), &a);
        let _ = std::fs::write(folder.join(format!("native-{name}-off.txt")), &b);
        let first = a
            .lines()
            .zip(b.lines())
            .position(|(x, y)| x != y)
            .unwrap_or(a.lines().count().min(b.lines().count()));
        let around = |text: &str| {
            text.lines()
                .skip(first.saturating_sub(8))
                .take(24)
                .collect::<Vec<_>>()
                .join("\n")
        };
        panic!("{name}: drawn without a Styled differs from with one at line {first}\n--- native\n{}\n--- Styled\n{}\n--- said\n{said_on}\n{said_off}", around(&a), around(&b));
    }
    (said_on, said_off)
}

/// The value after `name:` in what a page said.
fn value<'a>(said: &'a str, name: &str) -> &'a str {
    said.split('|')
        .find_map(|one| {
            one.strip_prefix(name)
                .and_then(|rest| rest.strip_prefix(':'))
        })
        .unwrap_or_else(|| panic!("no {name} in {said}"))
}

const CSS: &str = r#"
    .page { font-family: sans-serif; font-size: 15px; width: 460px; --accent: rgb(200, 0, 0); }
    p { margin: 6px 0; line-height: 1.4; }
    p.lead { font-size: 18px; color: rgb(0, 0, 120); }
    .doc p { margin: 2px 0; }
    .doc .note strong { color: var(--accent); }
    h1 { font-size: 24px; margin: 8px 0; }
    h2 { font-size: 20px; margin: 4px 0; }
    blockquote { border-left: 3px solid rgb(180, 180, 180); padding-left: 8px; margin: 4px 0; }
    pre { background: rgb(240, 240, 240); padding: 4px; font-family: monospace; }
    code { font-family: monospace; background: rgb(230, 230, 250); }
    mark { background: rgb(255, 255, 0); }
    ul, ol { padding-left: 24px; margin: 4px 0; }
    .center { text-align: center; }
    .big { font-size: 22px; }
    .wrap { white-space: pre-wrap; }
    .flexy { display: flex; gap: 6px; }
    .rtl { direction: rtl; }
    span.tag { color: rgb(0, 128, 0); }
    [data-kind="x"] { font-weight: bold; }
    .roman { list-style-type: upper-roman; }
    .square { list-style-type: square; }
"#;

/// Every kind of element this draws, in each of the contexts it's drawn in: on its own, with classes, styles, attributes, marks in
/// it and in each other, in what passes a font, an alignment, white space, a flex box or a direction down to it, and in lists.
const KINDS: &str = concat!(
    r#"<p>plain one</p><p>plain two, the same kind</p><p class="lead">a lead</p><p class="lead">another lead</p>"#,
    r#"<p style="color: rgb(10, 120, 10)">styled</p><p dir="rtl">right to left</p><p data-kind="x">data</p>"#,
    r#"<p>with <strong>strong</strong>, <em>em</em>, <b>b</b>, <i>i</i>, <u>u</u>, <s>s</s>, <code>code</code>, x<sub>2</sub>, y<sup>3</sup>, <mark>mark</mark> and <span class="tag">span</span></p>"#,
    r#"<p>nested <strong>bold <em>and italic</em></strong> marks</p><p>nested <strong>bold <em>and italic</em></strong> again</p>"#,
    r#"<p>a line<br>broken</p><p></p><p><br></p><p>a <a href="x.html">link</a> in it</p><p><span>only a span</span></p>"#,
    r#"<h1>heading one</h1><h2>heading two</h2><h3>three</h3><h4>four</h4><h5>five</h5><h6>six <em>em</em></h6>"#,
    r#"<blockquote>quoted text</blockquote><blockquote><p>quoted paragraph</p><p>and another</p></blockquote>"#,
    "<pre>pre  text\n  with spaces</pre><pre><code>let x = 1;\nlet y = 2;</code></pre>",
    r#"<ul><li>disc one</li><li>disc <strong>two</strong></li><li><p>with a paragraph</p></li></ul>"#,
    r#"<ol><li>first</li><li>second</li><li>third</li><li>fourth <em>em</em></li></ol>"#,
    r#"<ol><li value="1">valued</li><li value="2">valued too</li></ol><ol class="roman"><li>i</li><li>ii</li></ol><ul class="square"><li>sq</li><li>sq</li></ul>"#,
    r#"<div>text in a div</div><div>inline <em>marks</em> in a div</div><div><p>a paragraph in a div</p><p>and one more</p></div>"#,
    r#"<div class="doc"><p>doc one</p><p class="note">a <strong>note</strong></p><div><p>deeper</p><h3>deeper heading</h3></div></div>"#,
    r#"<div class="center"><p>centered</p><h2>centered heading</h2></div><div class="big"><p>big inherited</p><p>with <em>em</em></p></div>"#,
    r#"<div class="wrap"><p>pre   wrapped   text</p></div><div class="flexy"><p>in flex</p><p>beside</p></div><div class="rtl"><p>rtl container</p></div>"#,
);

#[test]
fn what_s_drawn_without_a_styled_is_what_a_styled_draws() {
    let body = format!("{KINDS}{KINDS}");
    let script = r#"
        // The last of each, in the second copy, where the first of its kind has been drawn already.
        const second = (selector: string) => { const all = root.querySelectorAll(selector); return (all[all.length - 1] as any)?.__nativeLive !== undefined; };
        out.push('natives:' + natives());
        for (const selector of ['p.lead', 'p[data-kind]', 'strong', 'em', 'h1', 'h6', 'blockquote', 'pre', 'code', 'li', 'mark', 'sub', 'div.doc', 'div.center', 'div.flexy p']) {
            out.push(selector + ':' + second(selector));
        }
    "#;
    let (on, off) = both("kinds", CSS, &body, script);
    let natives: usize = value(&on, "natives").parse().unwrap();
    // Of about 210 elements, all but the first of each kind, and lists, links, and what's in a flex box.
    assert!(natives > 80, "{on}");
    assert_eq!(value(&off, "natives"), "0", "{off}");
    for selector in [
        "p.lead",
        "p[data-kind]",
        "strong",
        "em",
        "h1",
        "h6",
        "blockquote",
        "pre",
        "code",
        "li",
        "mark",
        "sub",
        "div.doc",
        "div.center",
        "div.flexy p",
    ] {
        assert_eq!(
            value(&on, selector),
            "true",
            "{selector} isn't drawn this way: {on}"
        );
    }
}

#[test]
fn what_doesn_t_fit_is_a_styled() {
    let css = r#"
        p { margin: 4px 0; }
        p.hover:hover { color: rgb(255, 0, 0); }
        p.moving { transition: color 1s; }
        p.before::before { content: '> '; }
        p.upper { text-transform: uppercase; }
        p.grid { display: grid; }
        p.flex { display: flex; }
        span.box { display: inline-block; padding: 2px; border: 1px solid rgb(0, 0, 0); }
    "#;
    let one = concat!(
        r#"<p class="tab" tabindex="0">takes the keyboard</p><p class="edit" contenteditable="true">editable</p><p class="titled" title="said">with a title</p>"#,
        r#"<p class="hover">hovered</p><p class="moving">moving</p><p class="before">before</p><p class="upper">upper</p>"#,
        r#"<p class="grid">grid</p><p class="flex">flex</p><p>a <span class="box">box</span> in a line</p>"#,
        r#"<form><p>in a form</p></form><p>an <img src="x.png"> picture</p><p><button>button</button> beside</p><p>a <!-- comment --> here</p>"#,
    );
    let long: String = (0..70).map(|at| format!("<p>long {at}</p>")).collect();
    let body = format!("{one}{one}<div class=\"long\">{long}</div>");
    let script = r#"
        const drawn = (one: any) => one.__nativeLive !== undefined;
        const all = Array.from(root.querySelectorAll('p.tab, p.edit, p.titled, p.hover, p.moving, p.before, p.upper, p.grid, p.flex, form p'));
        out.push('styled:' + all.filter((one) => !drawn(one)).length + '/' + all.length + ' ' + all.filter(drawn).map((one) => one.className).join(' '));
        out.push('long:' + drawn(root.querySelector('div.long')) + ',' + drawn(root.querySelectorAll('div.long p')[5]));
    "#;
    let (on, _) = both("fallback", css, &body, script);
    assert_eq!(value(&on, "styled"), "20/20 ", "{on}");
    // A long list is a `Styled`, which keeps what's in it as it is, and each of what's in it is drawn this way.
    assert_eq!(value(&on, "long"), "false,true", "{on}");
}

/// A page whose effect puts this in its `<div class="page">` with `innerHTML`, as the first thing it does with the DOM, and
/// says what's in it once `wait` ms have gone by, then made into an app with its first frame made (which runs the effect), but
/// neither laid out nor updated since.
fn first_frame(name: &str, wait: u32) -> rsui::App {
    let source = r#"
        import React, { useEffect, useRef, useState } from 'react';
        export default function App() {
            const ref = useRef<HTMLDivElement>(null);
            const [said, setSaid] = useState('');
            useEffect(() => {
                const root = ref.current!;
                root.innerHTML = '<p class="one">zebra</p><div class="two"><p>okapi <em>em</em></p></div>';
                setTimeout(() => setSaid('done ' + root.querySelectorAll('*').length + ',' + (root.querySelector('div.two') !== null)), WAIT);
            }, []);
            return <div><div ref={ref} className="page" /><div className="said">{said}</div></div>;
        }
    "#
    .replace("WAIT", &wait.to_string());
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-native-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    let entry = rsui_tsx::load(&file, hands::STACK_LIMIT).unwrap();
    let text = rsui::default_text_engine().unwrap();
    rsui::App::new(rsui_tsx::root, entry, text, (480, 600), 1.0)
}

/// What code puts in a page's element with `innerHTML` is drawn when that's the first thing it does with the DOM. Until something
/// else had loaded `rsui/dom`, an element had nothing for `innerHTML`, and what was set only became a property of it.
#[test]
fn inner_html_first_is_drawn() {
    let mut hands = Hands {
        app: first_frame("html-first", 0),
    };
    hands.wait_for("done ");
    assert!(
        hands.says("zebra") && hands.says("okapi"),
        "{}",
        hands.app.outline()
    );
    assert_eq!(said(&hands), "4,true");
}

/// A page's timer that comes due while a frame is being made, as one does under load, runs at the start of the next frame, and
/// finds what its code put in the page and has been drawn, even when nothing's been laid out yet. It found nothing there, as the
/// tree had been let go of for the frame and only what had been laid out was left, which before the first layout is nothing:
/// `what_doesn_t_fit_is_a_styled`'s 80 ms timer came due there when the first frame took longer than that, and the page said
/// `Cannot read properties of null (reading '__nativeLive')`.
#[test]
fn a_timer_due_before_the_first_layout_finds_what_s_drawn() {
    let mut app = first_frame("early-timer", 40);
    // The frame takes long, and the timer's due by the time the next one starts, before anything's been laid out.
    std::thread::sleep(std::time::Duration::from_millis(120));
    app.update();
    let mut hands = Hands { app };
    hands.wait_for("done ");
    assert_eq!(said(&hands), "4,true");
}

#[test]
fn with_rules_about_where_each_is_among_the_rest_it_is_a_styled() {
    let css =
        "li:first-child { color: rgb(255, 0, 0); } p + p { margin-top: 10px; } p { margin: 0; }";
    let one = "<ul><li>one</li><li>two <em>em</em></li></ul><ol><li>a</li><li>b</li></ol><p>p one</p><p>p <strong>two</strong></p>";
    let body = format!("<div>{one}</div><div>{one}</div>");
    let (on, _) = both("structural", css, &body, "out.push('lis:' + Array.from(root.querySelectorAll('li')).filter((one: any) => one.__nativeLive !== undefined).length);");
    assert_eq!(value(&on, "lis"), "0", "{on}");
}

#[test]
fn the_page_s_code_finds_what_s_drawn_this_way() {
    let script = r#"
        const rect = (one: Element) => { const r = one.getBoundingClientRect(); return [r.left, r.top, r.width, r.height].map((n) => Math.round(n * 10) / 10).join(','); };
        const lead = root.querySelector('p.lead')!;
        out.push('found:' + [root.querySelectorAll('p.lead').length, root.querySelectorAll('strong').length, root.querySelectorAll('.doc p').length,
            root.querySelectorAll('li').length, document.querySelectorAll('[data-kind="x"]').length, root.querySelectorAll('blockquote > p').length].join(','));
        out.push('matches:' + lead.matches('.page p.lead') + ',' + (lead.closest('.page') === root) + ',' + (root.querySelector('.note strong')?.closest('.doc') !== null));
        out.push('text:' + root.querySelector('.doc')!.textContent + '/' + lead.textContent + '/' + root.querySelectorAll('ol')[0].textContent);
        out.push('rects:' + ['p.lead', 'strong', 'h1', 'li', 'blockquote p', 'pre code', 'div.doc', 'em'].map((selector) => rect(root.querySelectorAll(selector)[1])).join(';'));
        out.push('style:' + [getComputedStyle(lead).color, getComputedStyle(lead).fontSize, getComputedStyle(root.querySelectorAll('mark')[1]).backgroundColor].join(','));
        const strong = root.querySelectorAll('strong')[1] as HTMLElement;
        out.push('strong:' + strong.tagName + ',' + strong.parentElement!.tagName + ',' + (strong.firstChild as Text).data);
    "#;
    let body = format!("{KINDS}{KINDS}");
    let (on, off) = both("dom", CSS, &body, script);
    assert_eq!(on, off);
    assert!(value(&on, "found").starts_with("4,10,"), "{on}");
    assert_eq!(value(&on, "matches"), "true,true,true", "{on}");
    assert!(!value(&on, "rects").contains(",0,0;"), "{on}");
}

#[test]
fn the_keyboard_goes_where_it_does_with_a_styled() {
    // What can be edited, with paragraphs drawn this way in it, and a field in a block drawn this way.
    let script = r#"
        // What has the keyboard blinks its caret, which a long wait here doesn't get past in a test.
        const soon = () => new Promise((done) => setTimeout(done, 100));
        const editor = root.children[0] as HTMLElement;
        at('0');
        const input = document.createElement('input');
        input.value = 'x';
        root.children[1].appendChild(input);
        await soon();
        at('1');
        input.focus();
        await soon();
        at('2');
        out.push('input:' + (document.activeElement === input));
        editor.focus();
        await soon();
        at('3');
        out.push('editor:' + (document.activeElement === editor));
        const p = editor.querySelectorAll('p')[1];
        const range = document.createRange();
        range.setStart(p.firstChild!, 3);
        range.collapse(true);
        const selection = window.getSelection()!;
        selection.removeAllRanges();
        selection.addRange(range);
        await soon();
        at('4');
        out.push('caret:' + selection.anchorNode!.textContent + '@' + selection.anchorOffset);
        out.push('natives:' + natives());
    "#;
    let body = r#"<div contenteditable="true" class="ed"><p>first line</p><p>second line</p><p>third <strong>line</strong></p></div><div class="field"><p>a field</p></div>"#;
    let (on, off) = both("focus", "p { margin: 2px 0; }", body, script);
    assert_eq!(off, "input:true|editor:true|caret:second line@3|natives:0");
    assert_eq!(
        on.replace(&format!("natives:{}", value(&on, "natives")), "natives:0"),
        off
    );
    // The second paragraph, and the one the field is in.
    assert!(value(&on, "natives").parse::<usize>().unwrap() >= 2, "{on}");
    // Typing goes into the paragraph the caret is in, drawn this way, as it does into a `Styled`.
    let mut typed = Vec::new();
    for native in [true, false] {
        let mut hands = page("typing", native, "p { margin: 2px 0; }", body, script);
        hands.type_text("XY");
        hands.wait_for("secXYond line");
        typed.push(dump(&hands));
    }
    assert_eq!(typed[0], typed[1]);
}

#[test]
fn what_a_kind_came_to_goes_when_the_css_or_its_class_changes() {
    // Each change is seen to by what's drawn this way as by a `Styled`: the rules changing, a sheet put in, a rule put in it, a
    // class changing, and a style set on one of them.
    let script = r#"
        const look = (selector: string) => { const one = root.querySelectorAll(selector)[1]; if (one === undefined) { return 'missing ' + selector + ' ' + root.querySelectorAll(selector).length + ' ' + root.querySelectorAll('*').length; } const s = getComputedStyle(one); return [s.color, s.fontSize, s.marginLeft, Math.round(one.getBoundingClientRect().height)].join(','); };
        const record = () => out.push(['p', 'p.lead', 'strong', 'li', '.doc p', 'h2'].map(look).join(';'));
        const before = natives();
        record();
        style.textContent += ' p { color: rgb(1, 2, 3); } strong { color: rgb(4, 5, 6); }';
        await later();
        record();
        const added = document.createElement('style');
        added.textContent = '.lead { font-size: 30px; } li { margin-left: 5px; }';
        document.head.appendChild(added);
        await later();
        record();
        added.sheet!.insertRule('.doc p { margin-left: 20px; }', 0);
        await later();
        record();
        root.querySelectorAll('p')[1].className = 'lead';
        root.querySelectorAll('strong')[1].className = 'tag';
        root.querySelectorAll('h2')[1].setAttribute('class', 'big');
        await later();
        record();
        (root.querySelectorAll('p')[2] as HTMLElement).style.color = 'rgb(7, 8, 9)';
        root.querySelectorAll('em')[1].textContent = 'changed';
        await later();
        record();
        const p = root.querySelectorAll('p')[1] as any;
        out.push('p:' + p.className + ',' + getComputedStyle(p).fontSize);
        // Each change has the first of each kind drawn as a `Styled` to find out what it comes to now.
        out.push('natives:' + before + ',' + natives());
    "#;
    let body = format!("{KINDS}{KINDS}");
    let (on, off) = both(
        "stale",
        &format!("{CSS} span.tag {{ color: rgb(0, 128, 0); }}"),
        &body,
        script,
    );
    let counts = value(&on, "natives").to_owned();
    assert_eq!(
        on.replace(&format!("natives:{counts}"), ""),
        off.replace("natives:0,0", "")
    );
    // Each change has the first of each kind drawn as a `Styled` to find out what it comes to then, and the rest drawn from that.
    let (before, after) = counts.split_once(',').unwrap();
    let (before, after): (usize, usize) = (before.parse().unwrap(), after.parse().unwrap());
    assert!(before > 80 && after > 30, "{counts}");
    // Each change showed, and wasn't the same as before it.
    let records: Vec<&str> = on.split('|').take(6).collect();
    for pair in records.windows(2) {
        assert_ne!(pair[0], pair[1], "{on}");
    }
}

#[test]
fn unrelated_parent_transforms_preserve_explicit_inheritance_updates() {
    let script = r#"
        const candidate = root.querySelectorAll('p')[1] as any;
        const record = () => {
            const rect = candidate.getBoundingClientRect();
            out.push([Math.round(rect.x), Math.round(rect.width), getComputedStyle(candidate).width].join(','));
        };
        record();
        root.style.transform = 'translateX(11px)';
        await later();
        record();
        root.style.width = '23px';
        await later();
        record();
        // The third sibling can use the new declaration learned by the second.
        const third = document.createElement('p');
        third.textContent = 'another';
        root.appendChild(third);
        await later();
        out.push('inherited:' + getComputedStyle(third).width);
    "#;
    let css = "html { box-sizing: border-box; } * { box-sizing: inherit; } .page { width: 70px; transform: translateX(0px); } p { width: inherit; }";
    let (on, off) = both("parent-reads", css, "<p>first</p><p>second</p>", script);
    assert_eq!(on, off);
    assert_eq!(value(&on, "inherited"), "23px", "{on}");
}
