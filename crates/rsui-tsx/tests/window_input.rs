//! What a page hears from the window it's in: the pointer held down on something and taken off it, the window being put behind
//! another or minimized and brought back, and CSS's own `translate`, `rotate` and `scale`, as Tailwind 4 writes them.

mod hands;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use hands::Hands;
use rsui::replay::Input;

fn write(name: &str, source: &str, css: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-window-input-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    std::fs::write(folder.join("app.css"), css).unwrap();
    file
}

/// What the page has written into the element with this key's text, as `<p key="log">`.
fn text_of(hands: &Hands, key: &str) -> String {
    let outline = hands.app.outline();
    let line = outline
        .lines()
        .find(|line| line.split_whitespace().any(|word| word == format!("#{key}")))
        .unwrap_or_else(|| panic!("no {key} in\n{outline}"));
    line.to_owned()
}

#[test]
fn a_tab_held_down_hears_the_pointer_after_it_leaves_the_tab() {
    // The file explorer's tabs, in a strip the window is dragged by: a tab hears `pointermove` after the pointer's gone off it
    // onto the strip, and off the window, because a press gives the pointer to what was pressed (Pointer Events 3, 9.5).
    let file = write(
        "held",
        r#"
        import './app.css';
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const [moves, setMoves] = useState<string[]>([]);
            return (
                <PageWindow><div>
                    <div key="strip" className="strip [app-region:drag]">
                        <div key="tab" className="tab" onClick={() => {}}
                            onPointerMove={(event: any) => setMoves((all) => [...all, event.clientX + ',' + event.clientY + ',' + event.buttons])} />
                    </div>
                    <p key="log">{'moves ' + moves.join(' ')}</p>
                </div></PageWindow>
            );
        }
        "#,
        r#"
        body { margin: 0; }
        .strip { height: 40px; width: 400px; }
        .tab { margin-left: 10px; width: 50px; height: 30px; }
        "#,
    );
    let mut hands = Hands::open(&file, (500, 400));
    hands.settle();
    hands.down_at(20.0, 15.0);
    // Along the strip, then down into the page, then out of the window altogether, which a captured pointer still reports.
    for (x, y) in [(120.0, 15.0), (250.0, 20.0), (250.0, 200.0), (600.0, -30.0)] {
        std::thread::sleep(Duration::from_millis(40));
        hands.move_at(x, y);
    }
    std::thread::sleep(Duration::from_millis(40));
    hands.settle();
    let log = text_of(&hands, "log");
    for heard in ["120,15,1", "250,20,1", "250,200,1", "600,-30,1"] {
        assert!(log.contains(heard), "the tab hears {heard}: {log}");
    }
    // And with the system saying the pointer left between moves, as it does while it's held over where the window is dragged by.
    hands.up_at(600.0, -30.0);
    hands.down_at(20.0, 15.0);
    for (x, y) in [(130.0, 15.0), (260.0, 20.0), (260.0, 210.0)] {
        std::thread::sleep(Duration::from_millis(40));
        hands.move_at(x, y);
        let _ = hands.app.feed(&Input::Leave);
        hands.settle();
    }
    let log = text_of(&hands, "log");
    for heard in ["130,15,1", "260,20,1", "260,210,1"] {
        assert!(log.contains(heard), "the tab hears {heard} with leaves between: {log}");
    }
}

#[test]
fn the_window_going_behind_another_is_blur_and_coming_back_is_focus() {
    // Alt-Tab away and back: the window hears `blur` and `focus`, `document.hasFocus()` follows, and what has the keyboard hears
    // `blur` before the window does and `focus` after it, as in a browser. Minimized, `visibilitychange` says it's hidden.
    let file = write(
        "activation",
        r#"
        import './app.css';
        import { useEffect, useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const [heard, setHeard] = useState<string[]>([]);
            useEffect(() => {
                const note = (what: string) => setHeard((all) => [...all, what]);
                window.addEventListener('blur', () => note('window-blur:' + document.hasFocus()));
                window.addEventListener('focus', () => note('window-focus:' + document.hasFocus()));
                document.addEventListener('visibilitychange', () => note('visibility:' + document.visibilityState + ':' + document.hidden));
            }, []);
            return (
                <PageWindow><div>
                    <input key="field" onBlur={() => setHeard((all) => [...all, 'field-blur'])}
                        onFocus={() => setHeard((all) => [...all, 'field-focus'])} />
                    <p key="log">{'heard ' + heard.join(' ')}</p>
                </div></PageWindow>
            );
        }
        "#,
        "body { margin: 0; }",
    );
    let mut hands = Hands::open(&file, (500, 400));
    hands.settle();
    hands.click("field");
    hands.settle();
    let _ = hands.app.feed(&Input::Active(false));
    hands.settle();
    let _ = hands.app.feed(&Input::Shown(false));
    hands.settle();
    let _ = hands.app.feed(&Input::Shown(true));
    hands.settle();
    let _ = hands.app.feed(&Input::Active(true));
    hands.settle();
    let log = text_of(&hands, "log");
    let wanted = "heard field-focus field-blur window-blur:false visibility:hidden:true visibility:visible:false window-focus:true field-focus";
    assert!(log.contains(wanted), "{log}");
    // Said again, it's nothing new.
    let _ = hands.app.feed(&Input::Active(true));
    hands.settle();
    assert!(text_of(&hands, "log").contains(&format!("{wanted}\"")) || text_of(&hands, "log").ends_with(wanted), "{}", text_of(&hands, "log"));
}

/// The `transform` rsui has for what has this key now, which is part of the way there while it moves.
fn transform_of(hands: &Hands, key: &str) -> String {
    let mut path = Vec::new();
    assert!(hands::path_of(hands.app.runtime().root(), key, &mut path), "no {key}");
    let mut node = hands.app.runtime().root();
    for &step in &path {
        node = &node.children[step as usize];
    }
    match node.props.get("transform") {
        Some(rsui::core::PropValue::Str(text)) => text.to_string(),
        other => format!("{other:?}"),
    }
}

#[test]
fn tailwind_translate_rotate_and_scale_come_before_transform() {
    // Tailwind 4's `translate-x-4 rotate-45 scale-150` with a `transform` of its own: CSS Transforms 2 has them moved, then
    // turned, then scaled, then transformed.
    let file = write(
        "parts",
        r#"
        import './app.css';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            return <PageWindow><div><div key="box" className="box translate-x-4 rotate-45 scale-150 own" /></div></PageWindow>;
        }
        "#,
        r#"
        :root { --spacing: 0.25rem; }
        *, ::before, ::after { --tw-translate-x: 0; --tw-translate-y: 0; --tw-rotate-x: initial; --tw-scale-x: 1; --tw-scale-y: 1; }
        .box { width: 20px; height: 20px; }
        .translate-x-4 { --tw-translate-x: calc(var(--spacing) * 4); translate: var(--tw-translate-x) var(--tw-translate-y); }
        .rotate-45 { rotate: 45deg; }
        .scale-150 { --tw-scale-x: 150%; --tw-scale-y: 150%; --tw-scale-z: 150%; scale: var(--tw-scale-x) var(--tw-scale-y); }
        .own { transform: skewX(10deg); }
        "#,
    );
    let mut hands = Hands::open(&file, (500, 400));
    hands.settle();
    let transform = transform_of(&hands, "box");
    assert_eq!(transform, "translate(16px, 0) rotate(45deg) scale(1.5, 1.5) skewX(10deg)", "{}", hands.app.outline());
}

#[test]
fn translate_rotate_and_scale_transition_by_themselves() {
    // `transition: scale 300ms` (and `translate`, `rotate`) moves what Tailwind's `hover:scale-110` changes, which a transition of
    // `transform` alone didn't name.
    let file = write(
        "moving-parts",
        r#"
        import './app.css';
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const [on, setOn] = useState(false);
            return (
                <PageWindow><div>
                    <button key="go" onClick={() => setOn(!on)}>go</button>
                    <div key="scaled" className={'box scaled' + (on ? ' on' : '')} />
                    <div key="moved" className={'box moved' + (on ? ' on' : '')} />
                    <div key="turned" className={'box turned' + (on ? ' on' : '')} />
                </div></PageWindow>
            );
        }
        "#,
        r#"
        :root { --spacing: 0.25rem; }
        *, ::before, ::after { --tw-translate-x: 0; --tw-translate-y: 0; --tw-scale-x: 1; --tw-scale-y: 1; }
        .box { width: 20px; height: 20px; }
        .scaled { transition: scale 300ms linear; scale: var(--tw-scale-x) var(--tw-scale-y); }
        .scaled.on { --tw-scale-x: 200%; --tw-scale-y: 200%; }
        .moved { transition-property: translate; transition-duration: 300ms; transition-timing-function: linear;
            translate: var(--tw-translate-x) var(--tw-translate-y); }
        .moved.on { --tw-translate-x: calc(var(--spacing) * 25); }
        .turned { transition: rotate 300ms linear; rotate: 0deg; }
        .turned.on { rotate: 90deg; }
        "#,
    );
    let mut hands = Hands::open(&file, (500, 400));
    hands.app.set_animations(true);
    hands.app.set_own_clock(true);
    hands.app.tick(Instant::now());
    hands.settle();
    let from = Instant::now();
    hands.app.tick(from);
    std::thread::sleep(Duration::from_millis(120));
    hands.click("go");
    hands.settle();
    let mut seen: Vec<[String; 3]> = Vec::new();
    for step in 1..=25 {
        hands.app.tick(from + Duration::from_millis(step * 16));
        hands.settle();
        seen.push(["scaled", "moved", "turned"].map(|key| moving_value(&hands, key)));
    }
    for (at, (key, end)) in [("scaled", "scale(2, 2)"), ("moved", "translate(100px, 0)"), ("turned", "rotate(90deg)")].into_iter().enumerate() {
        let values: Vec<&String> = seen.iter().map(|three| &three[at]).collect();
        assert_eq!(values.last().unwrap().as_str(), end, "{key} gets there: {values:?}");
        assert!(values.len() > 3 && values.iter().filter(|value| value.as_str() != end).count() >= 3, "{key} is partway for a while: {values:?}");
    }
}

/// What's drawn of a moving transform now: rsui's runtime puts the part of the way it's got to in the outline as `transform`.
fn moving_value(hands: &Hands, key: &str) -> String {
    transform_of(hands, key)
}

#[test]
fn keyframes_of_scale_and_rotate_play() {
    // `@keyframes` that say `scale` and `rotate`, as tw-animate-css's `zoom-in` and `spin` do, play as the transform they make.
    let file = write(
        "keyframes-parts",
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
        @keyframes grow { from { scale: 0.5; rotate: 0deg; } to { scale: 1; rotate: 90deg; } }
        .box { width: 20px; height: 20px; }
        .box.on { animation: grow 1s linear; }
        "#,
    );
    let mut hands = Hands::open(&file, (500, 400));
    hands.app.set_animations(true);
    hands.app.set_own_clock(true);
    let from = Instant::now();
    hands.app.tick(from);
    hands.settle();
    hands.click("go");
    hands.settle();
    let mut seen = Vec::new();
    for step in 1..=8 {
        hands.app.tick(from + Duration::from_millis(step * 100));
        hands.settle();
        seen.push(transform_of(&hands, "box"));
    }
    assert!(
        seen.iter().any(|one| one.starts_with("rotate(") && one.contains("scale(") && !one.contains("rotate(0deg)") && !one.contains("rotate(90deg)")),
        "partway, turned and scaled: {seen:?}"
    );
}
