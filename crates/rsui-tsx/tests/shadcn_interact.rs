//! What the shadcn/ui kitchen sink's interaction steps needed of rsui/web, each as
//! small as it comes: cmdk's selectors, Radix Presence's animation events, Radix Slider's pointer capture, input-otp's
//! `pointer-events`, and Radix menus that a transform puts where they're shown.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn open(name: &str, css: &str, before: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-shadcn-interact-{}-{name}", std::process::id()));
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

#[test]
fn an_attribute_said_to_be_empty_isnt_matched_by_what_has_none() {
    // cmdk finds an item's group with `item.closest('[cmdk-group-items=""]')`, and put the item in itself when that found the
    // item: a browser's HierarchyRequestError, and nothing filtered after it.
    let hands = open(
        "empty-attribute",
        "",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                const item = document.getElementById('item');
                const group = item.closest('[cmdk-group-items=""]');
                const all = document.querySelectorAll('[cmdk-group-items=""]').length;
                setSaid('found ' + group?.id + ' of ' + all + ', item matches ' + item.matches('[cmdk-group-items=""]') + ', any ' + item.matches('[cmdk-item]'));
            }, []);
            return (
                <dom.div>
                    <dom.div id="group" cmdk-group-items="">
                        <dom.div id="item" cmdk-item="">Calendar</dom.div>
                    </dom.div>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    assert!(hands.says("found group of 1, item matches false, any true"), "{}", hands.app.outline());
}

#[test]
fn animations_say_they_start_and_end_to_what_listens_and_computed_styles_say_which() {
    // As Radix Presence does as a dialog closes: it keeps the computed style it got when it was put in, reads `animationName`
    // from it once it's closing, and takes it away when `animationend` says that animation's done.
    let hands = open(
        "animation-events",
        r#"
        @keyframes enter { from { opacity: 0; } }
        @keyframes exit { to { opacity: 0; } }
        .in { animation-name: enter; animation-duration: 150ms; }
        .out { animation-name: exit; animation-duration: 200ms; }
        "#,
        r#"
        function Page() {
            const [open, setOpen] = React.useState(true);
            const [log, setLog] = React.useState([]);
            const node = React.useRef(null);
            const style = React.useRef(null);
            const note = (said) => setLog((was) => [...was, said]);
            React.useLayoutEffect(() => {
                const element = node.current;
                style.current = getComputedStyle(element);
                note('mounted ' + style.current.animationName);
                const heard = (event) => note(event.type + ' ' + event.animationName + (event.target === element ? '' : ' elsewhere'));
                element.addEventListener('animationstart', heard);
                element.addEventListener('animationend', heard);
                setTimeout(() => setOpen(false), 50);
                return () => {
                    element.removeEventListener('animationstart', heard);
                    element.removeEventListener('animationend', heard);
                };
            }, []);
            React.useLayoutEffect(() => {
                if (!open) {
                    note('closing ' + style.current.animationName + ' ' + style.current.animationDuration);
                }
            }, [open]);
            return (
                <dom.div>
                    <dom.div ref={node} className={open ? 'in' : 'out'}><dom.span>Dialog</dom.span></dom.div>
                    <dom.p>{log.join(', ')}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    let mut hands = hands;
    hands.wait_for("animationend exit");
    // With nothing moving, each ends as soon as it starts.
    assert!(hands.says("mounted enter, "), "{}", hands.app.outline());
    assert!(
        hands.says("closing exit 0.2s, animationstart exit, animationend exit"),
        "{}",
        hands.app.outline()
    );
}

#[test]
fn a_keyframe_takes_the_variables_the_element_sets_itself() {
    // As Radix's accordion's `height: var(--radix-accordion-content-height)`, which it sets on the content it opens. With
    // nothing moving, an animation that's said to stay where it ends is there at once.
    let mut hands = open(
        "keyframe-own-variable",
        r#"
        @keyframes down { from { height: 0; } to { height: var(--content-height); } }
        .content { animation: down 0.2s ease-out forwards; overflow: hidden; }
        "#,
        r#"
        function Page() {
            return (
                <dom.div>
                    <dom.div id="content" className="content" style={{ '--content-height': '77px' }}>
                        <dom.div style={{ height: 20 }}>Inside</dom.div>
                    </dom.div>
                </dom.div>
            );
        }
        "#,
    );
    hands.settle();
    let laid_out = hands.app.rectangles();
    let line = laid_out.lines().find(|line| line.contains("dom=") && line.contains(",77")).unwrap_or("");
    assert!(!line.is_empty(), "nothing is 77 high\n{laid_out}");
}

#[test]
fn what_has_the_pointer_captured_says_so_until_it_comes_up() {
    // Radix's Slider follows the pointer only while the thumb it went down on says `hasPointerCapture()`.
    let mut hands = open(
        "pointer-capture",
        "",
        r#"
        function Page() {
            const [log, setLog] = React.useState([]);
            const note = (said) => setLog((was) => [...was, said]);
            return (
                <dom.div>
                    <dom.div
                        style={{ width: 300, height: 40, background: '#ccc' }}
                        onPointerDown={(event) => { event.target.setPointerCapture(event.pointerId); note('down ' + event.target.hasPointerCapture(event.pointerId)); }}
                        onPointerMove={(event) => { if (event.target.hasPointerCapture(event.pointerId)) note('moved'); }}
                        onPointerUp={(event) => note('up ' + event.target.hasPointerCapture(event.pointerId))}
                        onClick={(event) => note('after ' + event.target.hasPointerCapture(1))}
                    >Track</dom.div>
                    <dom.p>{'log ' + [...new Set(log)].join(', ')}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    hands.move_at(20.0, 20.0);
    hands.down_at(20.0, 20.0);
    hands.move_at(80.0, 20.0);
    hands.move_at(120.0, 20.0);
    hands.up_at(120.0, 20.0);
    hands.settle();
    assert!(hands.says("log down true, moved, up true, after false"), "{}", hands.app.outline());
}

#[test]
fn what_takes_the_pointer_in_what_doesnt_gets_the_click() {
    // input-otp: a `pointer-events: none` box over the slots, with a `pointer-events: all` input over all of it, which is what
    // a click anywhere there lands on.
    let mut hands = open(
        "pointer-events-all",
        "",
        r#"
        function Page() {
            const [log, setLog] = React.useState('nothing');
            return (
                <dom.div style={{ position: 'relative', width: 300, height: 40, pointerEvents: 'none' }}>
                    <dom.div style={{ width: 300, height: 40 }} onClick={() => setLog('slot')}>Slots</dom.div>
                    <dom.div style={{ position: 'absolute', inset: 0, pointerEvents: 'none' }}>
                        <dom.input style={{ position: 'absolute', inset: 0, pointerEvents: 'all', opacity: 1, color: 'transparent' }} onClick={() => setLog('input')} />
                    </dom.div>
                    <dom.p style={{ marginTop: 50 }}>{'clicked ' + log}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    hands.click_at(150.0, 20.0);
    hands.settle();
    assert!(hands.says("clicked input"), "{}", hands.app.outline());
}

#[test]
fn a_fixed_box_moved_by_its_transform_is_clicked_where_its_shown() {
    // Radix's menus are `position: fixed` at 0, 0 in a wrapper that `transform: translate()` puts where they're shown, as a
    // context menu at the pointer.
    let mut hands = open(
        "fixed-transformed",
        "",
        r#"
        function Page() {
            const [log, setLog] = React.useState('nothing');
            return (
                <dom.div>
                    <dom.div style={{ position: 'fixed', left: 0, top: 0, transform: 'translate(200px, 150px)', zIndex: 50 }}>
                        <dom.div style={{ width: 120, height: 30, background: '#eee' }} onClick={() => setLog('item')}>Item</dom.div>
                    </dom.div>
                    <dom.p>{'clicked ' + log}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    hands.click_at(260.0, 165.0);
    hands.settle();
    assert!(hands.says("clicked item"), "{}", hands.app.outline());
}
