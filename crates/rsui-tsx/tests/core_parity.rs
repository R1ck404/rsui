//! Engine causes found under the dialog, sheet and drawer at once: `:focus-visible` as
//! Chromium has it, rounded corners that are infinite or shares (Tailwind 4's `rounded-full`) drawn and hit as CSS has them,
//! what's placed by itself over what's in the flow, `setPointerCapture()`, a finished animation not played again when what it's on
//! is made again, and `getComputedStyle().transform` as a matrix.

mod hands;

use std::path::PathBuf;

use hands::{Hands, KeyCode};

fn open(name: &str, css: &str, before: &str, jsx: &str, size: (u32, u32)) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-core-parity-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let css = format!("body {{ margin: 0; }}\n{css}");
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

/// Chromium's `:focus-visible` (`SelectorChecker::MatchesFocusVisiblePseudoClass`): focus from code before anything's been done
/// shows it, as does focus from code after the keyboard; the pointer's focus doesn't, nor code's focus after it, until a key goes
/// down (then what has the focus shows it at once); `focus({ focusVisible })` says which, whatever came before.
#[test]
fn focus_visible_is_chromiums() {
    let before = format!(
        "{SAYS}\nfunction FocusFirst() {{ React.useEffect(() => document.getElementById('a').focus(), []); return null; }}
const measure = () => ['a', 'b'].map((id) => document.getElementById(id).getBoundingClientRect().width).join(',') + ' ' + (document.activeElement?.id || 'none');"
    );
    let css = "button { display: block; width: 50px; height: 20px; padding: 0; border: 0; } button:focus-visible { width: 77px; }";
    let jsx = r#"<dom.div>
        <dom.button id="a" onClick={() => {}}>a</dom.button>
        <dom.button id="b" onClick={() => document.getElementById('a').focus()}>b</dom.button>
        <dom.button id="c" onClick={() => document.getElementById('a').focus({ focusVisible: true })}>c</dom.button>
        <dom.button id="d" onClick={() => {}}>d</dom.button>
        <FocusFirst />
        <Says measure={measure} />
    </dom.div>"#;
    let mut hands = open("focus-visible", css, &before, jsx, (400, 300));
    // Before anything's been done, focus from code shows.
    hands.wait_for("said 77,50 a");
    // The pointer's focus doesn't show, and code's focus after it doesn't either.
    hands.click_at(10.0, 30.0);
    hands.wait_for("said 50,50 a");
    // A key going down shows it on what has the focus at once.
    hands.press(KeyCode::Character('x'));
    hands.wait_for("said 77,50 a");
    // `focusVisible: true` says it shows, after the pointer.
    hands.click_at(10.0, 70.0);
    hands.wait_for("said 50,50 d");
    hands.click_at(10.0, 50.0);
    hands.wait_for("said 77,50 a");
}

/// Tailwind 4's `rounded-full` is `calc(infinity * 1px)`: the corners are made to fit the box (CSS Backgrounds 3, 5.5), and the
/// box isn't hit outside them, nor is one with `50%`, so a click in a round button's corner is on what's under it.
#[test]
fn infinite_and_share_radii_fit_and_are_hit_inside_their_curves() {
    let before = r#"function Log() {
    const [said, setSaid] = React.useState('none');
    return <dom.div id="under" style={{ width: 300, height: 120, position: 'relative' }} onClick={(event) => setSaid(event.target.id)}>
        <dom.div id="full" className="full" />
        <dom.div id="half" className="half" />
        <dom.div id="said">{'hit ' + said}</dom.div>
    </dom.div>;
}"#;
    let css = ".full { position: absolute; left: 0; top: 0; width: 100px; height: 100px; border-radius: calc(infinity * 1px); background: red; }
.half { position: absolute; left: 150px; top: 0; width: 100px; height: 100px; border-radius: 50%; background: blue; }
#said { position: absolute; top: 105px; }";
    let mut hands = open("radii", css, before, "<Log />", (400, 300));
    hands.click_at(50.0, 50.0);
    hands.wait_for("hit full");
    hands.click_at(4.0, 4.0);
    hands.wait_for("hit under");
    hands.click_at(200.0, 50.0);
    hands.wait_for("hit half");
    hands.click_at(246.0, 96.0);
    hands.wait_for("hit under");
}

/// In something fixed (a sheet), what's placed by itself (its close button, `absolute top-4 right-4`) is over what's in the flow
/// that it's put over (the sheet's header), so it's what's hit there, as CSS paints them.
#[test]
fn what_is_placed_by_itself_is_hit_over_the_flow() {
    let before = r#"function Sheet() {
    const [said, setSaid] = React.useState('none');
    return <dom.div className="sheet">
        <dom.div id="header" className="header" onClick={() => setSaid('header')}>header</dom.div>
        <dom.button id="close" className="close" onClick={() => setSaid('close')}>x</dom.button>
        <dom.div id="said">{'hit ' + said}</dom.div>
    </dom.div>;
}"#;
    let css = ".sheet { position: fixed; top: 0; right: 0; bottom: 0; width: 200px; }
.header { padding: 16px; height: 40px; }
.close { position: absolute; top: 16px; right: 16px; width: 16px; height: 16px; padding: 0; border: 0; }";
    let mut hands = open("placed-over-flow", css, before, "<Sheet />", (400, 300));
    hands.click_at(400.0 - 24.0, 24.0);
    hands.wait_for("hit close");
    hands.click_at(250.0, 24.0);
    hands.wait_for("hit header");
}

/// `setPointerCapture()` in `pointerdown` has what the pointer does go to that element until the button comes up, wherever the
/// pointer is, with `gotpointercapture` before the next pointer event and `lostpointercapture` after `pointerup`, and the click
/// there too (vaul's drawer has its Cancel button's click by it).
#[test]
fn pointer_capture_keeps_the_pointer_until_the_button_comes_up() {
    let before = r#"function Capture() {
    const [log, setLog] = React.useState([]);
    const note = (what) => setLog((was) => [...was, what]);
    React.useEffect(() => {
        for (const type of ['gotpointercapture', 'pointerup', 'lostpointercapture', 'click']) {
            window.addEventListener(type, (event) => note(type + ':' + event.target.id), true);
        }
    }, []);
    return <dom.div>
        <dom.div id="grab" className="box" onPointerDown={(event) => event.target.setPointerCapture(event.pointerId)}>grab</dom.div>
        <dom.div id="other" className="box">other</dom.div>
        <dom.div id="log">{'log ' + log.join(' ')}</dom.div>
    </dom.div>;
}"#;
    let css = ".box { width: 100px; height: 40px; }";
    let mut hands = open("capture", css, before, "<Capture />", (400, 300));
    hands.down_at(20.0, 20.0);
    hands.move_at(20.0, 60.0);
    hands.up_at(20.0, 60.0);
    hands.wait_for("log gotpointercapture:grab pointerup:grab lostpointercapture:grab click:grab");
}

#[test]
fn releasing_capture_in_pointer_up_notifies_after_bubbling() {
    let before = r#"function Capture() {
    const [log, setLog] = React.useState([]);
    const note = (what) => setLog((was) => [...was, what]);
    React.useEffect(() => {
        for (const type of ['pointerup', 'lostpointercapture', 'click']) {
            window.addEventListener(type, (event) => note(type + ':' + event.target.id));
        }
    }, []);
    return <dom.div>
        <dom.div id="grab" className="box"
            onPointerDown={(e) => e.currentTarget.setPointerCapture(e.pointerId)}
            onPointerUp={(e) => {
                e.currentTarget.releasePointerCapture(e.pointerId);
                note('captured:' + e.currentTarget.hasPointerCapture(e.pointerId));
            }}>grab</dom.div>
        <dom.div className="box">other</dom.div>
        <dom.div>{'log ' + log.join(' ')}</dom.div>
    </dom.div>;
}"#;
    let mut hands = open("release-on-up", ".box { width: 100px; height: 40px; }", before, "<Capture />", (400, 300));
    hands.down_at(20.0, 20.0);
    hands.move_at(20.0, 60.0);
    hands.up_at(20.0, 60.0);
    hands.wait_for("log captured:false pointerup:grab lostpointercapture:grab click:grab");
}

/// An animation that's played to its end isn't played again because what it's on is made again with the same animation, as CSS
/// has it: a drawer that's pressed (and re-rendered) stays where its slide-in left it.
#[test]
fn a_finished_animation_is_not_played_again_by_a_render() {
    let before = r#"function Again() {
    const [count, setCount] = React.useState(0);
    const [starts, setStarts] = React.useState(0);
    return <dom.div>
        <dom.div id="moving" className="moving" data-count={count} onAnimationStart={() => setStarts((was) => was + 1)}>moving</dom.div>
        <dom.button id="again" className="again" onClick={() => setCount((was) => was + 1)}>again</dom.button>
        <dom.div id="starts">{'starts ' + starts + ' count ' + count}</dom.div>
    </dom.div>;
}"#;
    let css = "@keyframes come { from { opacity: 0; } to { opacity: 1; } }
.moving { animation: come 40ms linear; height: 20px; }
.again { display: block; width: 80px; height: 20px; padding: 0; border: 0; }";
    let mut hands = open("animation-again", css, before, "<Again />", (400, 300));
    hands.app.set_animations(true);
    hands.wait_for("starts 1 count 0");
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_millis(150) {
        hands.settle();
        hands.app.runtime().wait(std::time::Duration::from_millis(5));
    }
    hands.click_at(10.0, 30.0);
    hands.wait_for("starts 1 count 1");
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_millis(100) {
        hands.settle();
        hands.app.runtime().wait(std::time::Duration::from_millis(5));
    }
    hands.wait_for("starts 1 count 1");
}

/// `getComputedStyle().transform` is the matrix it comes to, as a browser says it: vaul reads how far a drawer's been dragged
/// from it.
#[test]
fn computed_transform_is_a_matrix() {
    let before = format!(
        "{SAYS}\nconst measure = () => ['t', 'r', 'n', 'z'].map((id) => getComputedStyle(document.getElementById(id)).transform).join(' | ');"
    );
    let css = "#t { transform: translate3d(0, 143px, 0); } #r { transform: scale(2, 3) translate(10px, 5px); } #z { transform: translateZ(5px); }";
    let jsx = r#"<dom.div>
        <dom.div id="t">t</dom.div>
        <dom.div id="r">r</dom.div>
        <dom.div id="n">n</dom.div>
        <dom.div id="z">z</dom.div>
        <Says measure={measure} />
    </dom.div>"#;
    let mut hands = open("transform-matrix", css, &before, jsx, (400, 300));
    hands.wait_for(
        "said matrix(1, 0, 0, 1, 0, 143) | matrix(2, 0, 0, 3, 20, 15) | none | matrix3d(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 5, 1)",
    );
}

/// `document` is the top of the tree, as in a browser: walking up `parentNode` ends at `null` (react-remove-scroll does it on
/// every wheel in a drawer, and threw at `undefined`).
#[test]
fn walking_up_parent_nodes_ends_at_null() {
    let before = format!(
        "{SAYS}\nconst measure = () => {{ let node = document.getElementById('deep'); const names = []; while (node !== null) {{ names.push(node.nodeName); node = node.parentNode; }} return names.join('>'); }};"
    );
    let jsx = r#"<dom.div><dom.p id="deep">deep</dom.p><Says measure={measure} /></dom.div>"#;
    let mut hands = open("parent-nodes", "", &before, jsx, (400, 300));
    hands.wait_for("said P>DIV");
    hands.wait_for(">BODY>HTML>#document");
}
