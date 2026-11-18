//! What moves by itself in a page, as CSS's `transition` and `animation` have it, against how Mantine's parts use them.

mod hands;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use hands::Hands;

fn write(name: &str, source: &str, css: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-motion-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    std::fs::write(folder.join("app.css"), css).unwrap();
    file
}

/// Opens it with things moving, on a clock the test keeps.
fn moving(file: &PathBuf) -> Hands {
    let mut hands = Hands::open(file, (500, 400));
    hands.app.set_animations(true);
    hands.app.set_own_clock(true);
    hands.app.tick(Instant::now());
    hands.settle();
    hands
}

/// However long a slow or busy machine might take between a frame and what's done after it, which a clock the test keeps
/// doesn't see: these failed under load, when a click's transition started at the real time, after the frames it was looked at.
fn lag() {
    std::thread::sleep(Duration::from_millis(120));
}

/// Clicks `key`, then says how wide `box` is every 16 ms for `frames` frames, from the click.
fn widths_after_click(hands: &mut Hands, key: &str, frames: u64, mut between: impl FnMut(&mut Hands, u64)) -> Vec<f32> {
    let from = Instant::now();
    hands.app.tick(from);
    lag();
    hands.click(key);
    hands.settle();
    let mut widths = Vec::new();
    for step in 1..=frames {
        between(hands, step);
        hands.app.tick(from + Duration::from_millis(step * 16));
        hands.settle();
        widths.push(hands.rect_of("box")[2]);
    }
    widths
}

fn moves_from_20_to_120(widths: &[f32], what: &str) {
    assert!(widths.iter().any(|&w| w > 30.0 && w < 110.0), "{what}: it's partway at some point: {widths:?}");
    assert!(widths.windows(2).all(|pair| pair[1] >= pair[0] - 0.01), "{what}: only ever wider: {widths:?}");
    assert!((widths.last().unwrap() - 120.0).abs() < 0.01, "{what}: and it gets there: {widths:?}");
}

#[test]
fn transition_longhands_said_apart_are_put_together() {
    // Mantine's FloatingIndicator (a SegmentedControl's indicator): `transition-property: transform, width, height;
    // transition-duration: 0ms`, then `:where([data-initialized]) { transition-duration: var(--transition-duration) }`, with the
    // variable on the element. Each rule's longhands took the place of all the ones before, so nothing was left to move.
    let file = write(
        "longhands",
        r#"
        import './app.css';
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const [wide, setWide] = useState(false);
            const [which, setWhich] = useState('ready');
            return (
                <PageWindow><div>
                    <button key="go" onClick={() => setWide(!wide)}>go</button>
                    <button key="vars" onClick={() => setWhich('vars')}>vars</button>
                    <button key="inline" onClick={() => setWhich('inline')}>inline</button>
                    <div key="box" className={'box ' + which} data-ready=""
                        style={{ width: wide ? 120 : 20, ...(which === 'inline' ? { transitionDuration: '300ms' } : null) }} />
                </div></PageWindow>
            );
        }
        "#,
        r#"
        .box { position: absolute; top: 40px; height: 10px; background: red; transition-property: width;
            transition-duration: 0ms; transition-timing-function: linear; }
        .ready[data-ready] { transition-duration: 300ms; }
        .vars { --d: 300ms; }
        .vars[data-ready] { transition-duration: var(--d); }
        "#,
    );
    let mut hands = moving(&file);
    let there = widths_after_click(&mut hands, "go", 25, |_, _| {});
    moves_from_20_to_120(&there, "a later rule's duration");
    let back = widths_after_click(&mut hands, "go", 25, |_, _| {});
    assert!((back.last().unwrap() - 20.0).abs() < 0.01, "and back: {back:?}");
    for which in ["vars", "inline"] {
        hands.click(which);
        hands.settle();
        let there = widths_after_click(&mut hands, "go", 25, |_, _| {});
        moves_from_20_to_120(&there, which);
        widths_after_click(&mut hands, "go", 25, |_, _| {});
    }
}

#[test]
fn a_width_that_is_a_share_moves() {
    // Mantine's Progress: a section is `width: var(--progress-section-size)` with `transition: width 100ms ease`, and the
    // variable, on its `style`, says `20%` then `80%`. A share is `width_percent` to layout, which nothing said could move.
    let file = write(
        "share",
        r#"
        import './app.css';
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const [wide, setWide] = useState(false);
            return (
                <PageWindow><div>
                    <button key="go" onClick={() => setWide(!wide)}>go</button>
                    <div key="track" className="track"><div key="box" className="box" style={{ '--size': wide ? '60%' : '10%' }} /></div>
                </div></PageWindow>
            );
        }
        "#,
        r#"
        .track { width: 200px; height: 10px; display: flex; }
        .box { width: var(--size); height: 10px; background: red; transition: width 300ms linear; }
        "#,
    );
    let mut hands = moving(&file);
    assert!((hands.rect_of("box")[2] - 20.0).abs() < 0.01, "{:?}", hands.rect_of("box"));
    let there = widths_after_click(&mut hands, "go", 25, |_, _| {});
    assert!(there.iter().any(|&w| w > 30.0 && w < 110.0), "it's partway at some point: {there:?}");
    assert!(there.windows(2).all(|pair| pair[1] >= pair[0] - 0.01), "only ever wider: {there:?}");
    assert!((there.last().unwrap() - 120.0).abs() < 0.01, "and it gets there: {there:?}");
}

#[test]
fn a_width_kept_between_a_least_and_a_most_moves() {
    // From `min(300px, 50%)` to `clamp(400px, 60%, 800px)` in 200 wide, which are 100 and 400 there (the least wins): it goes
    // from what the one comes to to what the other does, as a browser has it, where it used to go there at once. Turned back
    // part way, it goes back from where it is.
    let file = write(
        "clamp",
        r#"
        import './app.css';
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const [wide, setWide] = useState(false);
            return (
                <PageWindow><div>
                    <button key="go" onClick={() => setWide(!wide)}>go</button>
                    <div key="track" className="track"><div key="box" className={wide ? 'box wide' : 'box'} /></div>
                </div></PageWindow>
            );
        }
        "#,
        r#"
        .track { width: 200px; height: 10px; }
        .box { width: min(300px, 50%); height: 10px; background: red; transition: width 300ms linear; }
        .box.wide { width: clamp(400px, 60%, 800px); }
        "#,
    );
    let mut hands = moving(&file);
    assert!((hands.rect_of("box")[2] - 100.0).abs() < 0.01, "{:?}", hands.rect_of("box"));
    let there = widths_after_click(&mut hands, "go", 25, |_, _| {});
    assert!(there.iter().any(|&w| w > 120.0 && w < 380.0), "it's partway at some point: {there:?}");
    assert!(there.windows(2).all(|pair| pair[1] >= pair[0] - 0.01), "only ever wider: {there:?}");
    assert!((there.last().unwrap() - 400.0).abs() < 0.01, "and it gets there: {there:?}");
    // Part way there, then back.
    let mut turned = None;
    let both = widths_after_click(&mut hands, "go", 25, |hands, step| {
        if step == 8 {
            turned = Some(hands.rect_of("box")[2]);
            hands.click("go");
            hands.settle();
        }
    });
    let turned = turned.unwrap();
    assert!(turned > 120.0 && turned < 380.0, "it was part way back: {both:?}");
    let after = both[7];
    assert!((after - turned).abs() < 60.0, "it goes back from where it was: {turned} then {both:?}");
    assert!((both.last().unwrap() - 400.0).abs() < 0.01, "and gets there again: {both:?}");
}

#[test]
fn shares_of_a_flex_box_move() {
    // Panes split by `flex-grow` shares, in a row 300 wide: going from 1 to 5 against a neighbour of 1 takes this one from
    // 150 to 250, which it glides to with `transition: flex-grow`, as a browser has it.
    let file = write(
        "grow",
        r#"
        import './app.css';
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const [wide, setWide] = useState(false);
            return (
                <PageWindow><div>
                    <button key="go" onClick={() => setWide(!wide)}>go</button>
                    <div key="track" className="track">
                        <div key="box" className="pane" style={{ flexGrow: wide ? 5 : 1 }} />
                        <div key="other" className="pane" style={{ flexGrow: 1 }} />
                    </div>
                </div></PageWindow>
            );
        }
        "#,
        r#"
        .track { width: 300px; height: 10px; display: flex; }
        .pane { flex-basis: 0; height: 10px; background: red; transition: flex-grow 300ms linear; }
        "#,
    );
    let mut hands = moving(&file);
    assert!((hands.rect_of("box")[2] - 150.0).abs() < 0.01, "{:?}", hands.rect_of("box"));
    let there = widths_after_click(&mut hands, "go", 25, |_, _| {});
    assert!(there.iter().any(|&w| w > 160.0 && w < 240.0), "it's partway at some point: {there:?}");
    assert!(there.windows(2).all(|pair| pair[1] >= pair[0] - 0.01), "only ever wider: {there:?}");
    assert!((there.last().unwrap() - 250.0).abs() < 0.01, "and it gets there: {there:?}");
}

#[test]
fn a_transition_on_its_way_goes_on_when_only_its_duration_changes() {
    // Mantine's FloatingIndicator sets where it's going on its `style`, then, told its target's size by a `ResizeObserver`,
    // `transitionDuration = '0ms'` and the same place again. As CSS has it, that doesn't stop what's moving: only a new place to
    // go, or the prop not being said to move any more, does. The indicator jumped.
    let file = write(
        "duration-zero",
        r#"
        import './app.css';
        import { PageWindow } from 'rsui/web';
        const box = () => document.querySelector('.box');
        export default function App() {
            return (
                <PageWindow><div>
                    <button key="go" onClick={() => { box().style.width = '120px'; }}>go</button>
                    <button key="zero" onClick={() => { box().style.transitionDuration = '0ms'; box().style.width = '120px'; }}>zero</button>
                    <button key="jump" onClick={() => { box().style.width = '60px'; }}>jump</button>
                    <div key="box" className="box" />
                </div></PageWindow>
            );
        }
        "#,
        r#"
        .box { position: absolute; top: 40px; width: 20px; height: 10px; background: red;
            transition-property: width; transition-duration: 300ms; transition-timing-function: linear; }
        "#,
    );
    let mut hands = moving(&file);
    let widths = widths_after_click(&mut hands, "go", 25, |hands, step| {
        if step == 4 {
            lag();
            hands.click("zero");
            hands.settle();
        }
    });
    moves_from_20_to_120(&widths, "with its duration made 0 on the way");
    assert!(widths[5] < 100.0, "still partway after the duration's 0: {widths:?}");
    // A new place to go with no time to get there is where it is at once.
    hands.click("jump");
    hands.settle();
    assert!((hands.rect_of("box")[2] - 60.0).abs() < 0.01, "{:?}", hands.rect_of("box"));
}

#[test]
fn what_a_collapsed_box_holds_is_as_tall_as_it_is_inside_a_wrapper() {
    // Mantine's Collapse opens to its `scrollHeight`. With a Mantine `Box` in it, what it holds is in a box with none of its own
    // (`display: contents`), which `scrollHeight` didn't look into, so it opened to nothing.
    let file = write(
        "collapsed",
        r#"
        import './app.css';
        import { useEffect, useRef, useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const plain = useRef(null);
            const wrapped = useRef(null);
            const [said, setSaid] = useState('');
            useEffect(() => { setTimeout(() => setSaid(plain.current.scrollHeight + ' and ' + wrapped.current.scrollHeight), 50); }, []);
            return (
                <PageWindow><div>
                    <div key="plain" ref={plain} className="shut"><div className="inside">In it.</div></div>
                    <div key="wrapped" ref={wrapped} className="shut"><div className="wrap"><div className="inside">In it.</div></div></div>
                    <p>{'heights: ' + said}</p>
                </div></PageWindow>
            );
        }
        "#,
        r#"
        .shut { height: 0; overflow: hidden; }
        .wrap { display: contents; }
        .inside { height: 40px; padding: 5px 0; }
        "#,
    );
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let entry = rsui_tsx::load(&file, 8 << 20).unwrap();
            let text = rsui::default_text_engine().unwrap();
            let mut hands = Hands { app: rsui::App::new(rsui_tsx::root, entry, text, (500, 400), 1.0) };
            hands.settle();
            hands.wait_for("heights: 50 and 50");
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn keyframes_with_variables_in_them_move() {
    // Mantine's striped, animated Progress: `@keyframes { 0% { background-position: 0 0 } 100% { background-position:
    // calc(2.5rem * var(--mantine-scale)) 0 } }`. The variable wasn't put in, `calc(` was cut in two at its space, and
    // `background-position` wasn't something that moved.
    let file = write(
        "keyframes-vars",
        r#"
        import './app.css';
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const [on, setOn] = useState(false);
            return (
                <PageWindow><div>
                    <button key="go" onClick={() => setOn(true)}>go</button>
                    <div key="box" className={on ? 'box on' : 'box'} />
                </div></PageWindow>
            );
        }
        "#,
        r#"
        :root { --scale: 1; }
        @keyframes slide { 0% { background-position: 0 0; } 100% { background-position: calc(2.5rem * var(--scale)) 0; } }
        .box { width: 100px; height: 10px; background-size: 20px 20px; }
        .box.on { animation: slide 1s linear infinite; }
        "#,
    );
    let mut hands = moving(&file);
    let from = Instant::now();
    hands.app.tick(from);
    hands.click("go");
    hands.settle();
    let outline = hands.app.outline();
    assert!(outline.contains("background_position s 40px 0px"), "where it goes is in pixels: {outline}");
    let mut places = Vec::new();
    for step in 1..=12 {
        hands.app.tick(from + Duration::from_millis(step * 70));
        hands.settle();
        let outline = hands.app.outline();
        let place = outline.split("background_position=\"").nth(1).and_then(|rest| rest.split('"').next()).unwrap_or("");
        places.push(place.to_owned());
    }
    assert!(
        places.iter().any(|place| place.ends_with("px 0px") && !["0px 0px", "40px 0px"].contains(&place.as_str())),
        "it's partway at some point: {places:?}"
    );
}
