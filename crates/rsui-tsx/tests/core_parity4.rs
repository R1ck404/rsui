//! Cross-cutting causes found under many components: the document's selection of page text
//! made by the pointer, the window's own `focus` and `blur` as Tab leaves and comes back into the page, `performance.now()` counting
//! from the page's start, and `getBoundingClientRect()` through `scale()` and `rotate()`.

mod hands;

use std::path::PathBuf;

use hands::{Hands, KeyCode};

fn open(name: &str, css: &str, before: &str, jsx: &str, size: (u32, u32)) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-core-parity4-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let css = format!("body {{ margin: 0; font: 16px sans-serif; }}\n{css}");
    let app = format!(
        "import {{ PageWindow, dom }} from 'rsui/web';\nimport * as React from 'react';\nimport './app.css';\n{before}\nexport default function App() {{ return <PageWindow>{jsx}</PageWindow>; }}\n"
    );
    std::fs::write(folder.join("app.tsx"), app).unwrap();
    std::fs::write(folder.join("app.css"), css).unwrap();
    let mut hands = Hands::open(&folder.join("app.tsx"), size);
    hands.settle();
    hands
}

/// A component that shows what `measure` says, every 30 ms.
const SAYS: &str = r#"function Says({ measure }) {
    const [said, setSaid] = React.useState('waiting');
    React.useEffect(() => {
        const timer = setInterval(() => {
            try {
                setSaid('said ' + measure());
            } catch (error) {
                setSaid('threw ' + error.message);
            }
        }, 30);
        return () => clearInterval(timer);
    }, []);
    return <dom.div id="says">{said}</dom.div>;
}"#;

/// A press on page text leaves a collapsed selection there, two select the word with the space after it, a drag selects what it
/// went over, a press on what's selected that doesn't move collapses it, a press on a button or on `user-select: none` text leaves
/// it as it is, and `selectionchange` is heard once for each change.
#[test]
fn page_text_is_selected_by_the_pointer() {
    let before = format!(
        "{SAYS}\nlet changes = 0;\ndocument.addEventListener('selectionchange', () => changes++);
const measure = () => {{ const s = getSelection(); return s.rangeCount === 0 ? 'none ' + changes : [s.anchorNode.nodeType, s.anchorOffset, s.focusOffset, '[' + s.toString() + ']', changes].join(' '); }};"
    );
    let css = "p { margin: 0; height: 20px; line-height: 20px; } #none { user-select: none; } button { display: block; height: 20px; }";
    let jsx = r#"<dom.div>
        <dom.p id="p">Hello there world</dom.p>
        <dom.p id="none">Not this</dom.p>
        <dom.button id="b">button</dom.button>
        <Says measure={measure} />
    </dom.div>"#;
    let mut hands = open("selection", css, &before, jsx, (400, 300));
    hands.wait_for("said none 0");
    // A press at the start: a caret in the text node, at 0.
    hands.click_at(1.0, 10.0);
    hands.wait_for("said 3 0 0 [] 1");
    // Two presses on the first word: a caret, then it and the space after it, each heard.
    hands.click_at(10.0, 10.0);
    hands.app.pointer_down(10.0, 10.0, 2);
    hands.settle();
    let _ = hands.app.pointer_up(10.0, 10.0);
    hands.wait_for("said 3 0 6 [Hello ] 3");
    // A press on a button, and on what can't be selected, changes nothing.
    hands.click_at(10.0, 50.0);
    hands.click_at(10.0, 30.0);
    hands.settle();
    hands.wait_for("said 3 0 6 [Hello ] 3");
    // A press on what's selected that comes up without moving leaves a caret there.
    hands.click_at(10.0, 10.0);
    hands.wait_for("said 3 1 1 [] ");
    // A drag from the start to past the end selects it all.
    hands.down_at(1.0, 10.0);
    hands.move_at(200.0, 10.0);
    hands.move_at(390.0, 10.0);
    hands.up_at(390.0, 10.0);
    hands.wait_for("said 3 0 17 [Hello there world]");
}

/// Tab past the last stop takes the keyboard out of the page, as into a browser's toolbar: what had it hears `blur` and then the
/// window does, and `document.hasFocus()` is false until Tab, a click or `focus()` gives it back, which the window hears first.
#[test]
fn tab_out_of_the_page_blurs_the_window() {
    let before = format!(
        "{SAYS}\nconst heard = [];\nfor (const type of ['focus', 'blur']) window.addEventListener(type, (event) => heard.push(type + ':' + (event.target === window ? 'window' : event.target.id)), true);
const measure = () => heard.join(',') + ' has ' + document.hasFocus();"
    );
    let jsx = r#"<dom.div>
        <dom.button id="a" onClick={() => {}}>a</dom.button>
        <dom.button id="b" onClick={() => {}}>b</dom.button>
        <Says measure={measure} />
    </dom.div>"#;
    let mut hands = open("window-focus", "", &before, jsx, (400, 300));
    hands.wait_for("has true");
    hands.press(KeyCode::Tab);
    hands.press(KeyCode::Tab);
    hands.press(KeyCode::Tab);
    hands.wait_for("said focus:a,blur:a,focus:b,blur:b,blur:window has false");
    hands.press(KeyCode::Tab);
    hands.wait_for("blur:window,focus:window,focus:a has true");
}

/// `performance.now()` is the milliseconds since the page started, as `performance.timeOrigin` says on the epoch's clock, and an
/// event's `timeStamp` is on the same clock.
#[test]
fn performance_now_counts_from_the_start() {
    let before = format!(
        "{SAYS}\nlet stamp = -1;\nwindow.addEventListener('pointerdown', (event) => {{ stamp = event.timeStamp; }}, true);
const measure = () => [performance.now() < 60000, Math.abs(performance.timeOrigin + performance.now() - Date.now()) < 100, stamp >= 0 && stamp <= performance.now()].join(',');"
    );
    let jsx = r#"<dom.div style={{ height: 100 }}><Says measure={measure} /></dom.div>"#;
    let mut hands = open("performance", "", &before, jsx, (400, 300));
    hands.click_at(10.0, 50.0);
    hands.wait_for("said true,true,true");
}

/// `getBoundingClientRect()` is the box around where an element is drawn, through its own transform and every one above it, each
/// about its `transform-origin`, as browsers have it, while `offsetWidth` and a computed `width` are as it's laid out.
#[test]
fn rects_are_around_transformed_boxes() {
    let before = format!(
        "{SAYS}\nconst rect = (id) => {{ const b = document.getElementById(id).getBoundingClientRect(); return [b.x, b.y, b.width, b.height].map((n) => Math.round(n)).join(':'); }};
const measure = () => [rect('scaled'), rect('kid'), rect('turned'), rect('prop'), document.getElementById('scaled').offsetWidth, getComputedStyle(document.getElementById('scaled')).width].join(' ');"
    );
    let css = ".box { position: absolute; width: 100px; height: 40px; }";
    let jsx = r#"<dom.div>
        <dom.div id="scaled" className="box" style={{ left: 0, top: 0, transform: 'scale(0.5)' }}><dom.div id="kid" style={{ width: 40, height: 20 }} /></dom.div>
        <dom.div id="turned" className="box" style={{ left: 200, top: 0, transform: 'rotate(90deg)' }} />
        <dom.div id="prop" className="box" style={{ left: 0, top: 100, rotate: '180deg', scale: '2 1', transformOrigin: 'top left' }} />
        <dom.div style={{ position: 'absolute', top: 200 }}><Says measure={measure} /></dom.div>
    </dom.div>"#;
    let mut hands = open("rects", css, &before, jsx, (400, 300));
    hands.wait_for("said 25:10:50:20 25:10:20:10 230:-30:40:100 -200:60:200:40 100 100px");
}
