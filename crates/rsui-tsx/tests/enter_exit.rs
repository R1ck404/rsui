//! How an element comes in and goes out with CSS alone, as CSS Transitions 2 has it: `@starting-style` says where what's just
//! been drawn, or shown again after `display: none`, starts its transitions from, and `transition-behavior: allow-discrete` keeps
//! what's going to `display: none` (or `visibility: hidden`) shown while the rest of it moves away.

mod hands;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use hands::{path_of, Hands};
use rsui::core::PropValue;

fn write(name: &str, source: &str, css: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-enter-exit-{}-{name}", std::process::id()));
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

/// What the element with this key is drawn with, as it's drawn now, moving props and all.
fn prop_of(hands: &Hands, key: &str, prop: &str) -> Option<PropValue> {
    let root = hands.app.runtime().root();
    let mut path = Vec::new();
    if !path_of(root, key, &mut path) {
        return None;
    }
    let mut node = root;
    for index in path {
        node = &node.children[index as usize];
    }
    node.props.get(prop).cloned()
}

fn number(value: Option<PropValue>) -> Option<f64> {
    match value? {
        PropValue::Float(number) => Some(number),
        PropValue::Int(number) => Some(number as f64),
        _ => None,
    }
}

/// How see-through the element with this key is drawn: 1 where nothing says.
fn opacity_of(hands: &Hands, key: &str) -> f64 {
    number(prop_of(hands, key, "opacity")).unwrap_or(1.0)
}

/// Clicks `key`, then gives `look` the app every 16 ms for `frames` frames, from the click.
fn after_click<T>(hands: &mut Hands, key: &str, frames: u64, mut look: impl FnMut(&mut Hands) -> T) -> Vec<T> {
    let from = Instant::now();
    hands.app.tick(from);
    // However long a busy machine takes between a frame and the click, which a clock the test keeps doesn't see.
    std::thread::sleep(Duration::from_millis(120));
    hands.click(key);
    hands.settle();
    let mut seen = Vec::new();
    for step in 1..=frames {
        hands.app.tick(from + Duration::from_millis(step * 16));
        hands.settle();
        seen.push(look(hands));
    }
    seen
}

fn fades_in(seen: &[f64], to: f64, what: &str) {
    assert!(seen[0] < to * 0.5, "{what}: it starts from what `@starting-style` says: {seen:?}");
    assert!(seen.iter().any(|&one| one > 0.1 && one < to - 0.1), "{what}: it's partway at some point: {seen:?}");
    assert!(seen.windows(2).all(|pair| pair[1] >= pair[0] - 0.001), "{what}: only ever more there: {seen:?}");
    assert!((seen.last().unwrap() - to).abs() < 0.001, "{what}: and it gets there: {seen:?}");
}

fn stays(seen: &[f64], at: f64, what: &str) {
    assert!(seen.iter().all(|&one| (one - at).abs() < 0.001), "{what}: nothing moves: {seen:?}");
}

#[test]
fn what_starting_style_says_is_where_what_is_added_moves_from() {
    // A box added with `transition: opacity` starts from `@starting-style`'s `opacity: 0`, whether that's a rule of its own
    // (`@starting-style { .box { ... } }`, as Tailwind 4's `starting:` variant writes it) or inside the rule it's about. What
    // was there from the first doesn't, nor does what has no starting style, and drawing them again doesn't make them either.
    let file = write(
        "enter",
        r#"
        import './app.css';
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const [added, setAdded] = useState(false);
            return (
                <PageWindow><div>
                    <button key="add" onClick={() => setAdded(!added)}>add</button>
                    <div key="early" className="box" />
                    {added ? <div key="late" className="box" /> : null}
                    {added ? <div key="nested" className="nested" /> : null}
                    {added ? <div key="plain" className="plain" /> : null}
                    {added ? <div key="tw" className="starting:opacity-0 starting:scale-50 fade" /> : null}
                </div></PageWindow>
            );
        }
        "#,
        r#"
        .box, .nested, .plain, .fade { width: 20px; height: 10px; background: red; }
        .box { opacity: 0.8; transition: opacity 300ms linear; }
        @starting-style { .box { opacity: 0; } }
        .nested { transition: opacity 300ms linear; @starting-style { opacity: 0; } }
        .plain { opacity: 0.5; transition: opacity 300ms linear; }
        /* As Tailwind 4.3 writes `starting:opacity-0 starting:scale-50`. */
        .fade { transition-property: opacity, transform, translate, scale, rotate; transition-duration: 300ms; transition-timing-function: linear; }
        @starting-style {
          .starting\:scale-50 { --tw-scale-x: 50%; --tw-scale-y: 50%; --tw-scale-z: 50%; scale: var(--tw-scale-x) var(--tw-scale-y); }
          .starting\:opacity-0 { opacity: 0%; }
        }
        "#,
    );
    let mut hands = moving(&file);
    assert!((opacity_of(&hands, "early") - 0.8).abs() < 0.001, "what's there from the first is as it says");
    let seen = after_click(&mut hands, "add", 25, |hands| {
        let transform = match prop_of(hands, "tw", "transform") {
            Some(PropValue::Str(text)) => text.to_string(),
            _ => String::new(),
        };
        [opacity_of(hands, "late"), opacity_of(hands, "nested"), opacity_of(hands, "plain"), opacity_of(hands, "early"), opacity_of(hands, "tw")]
            .map(|one| (one, transform.clone()))
    });
    let column = |at: usize| seen.iter().map(|one| one[at].0).collect::<Vec<_>>();
    fades_in(&column(0), 0.8, "a rule of its own");
    fades_in(&column(1), 1.0, "nested in the rule");
    stays(&column(2), 0.5, "with no starting style");
    stays(&column(3), 0.8, "there from the first");
    fades_in(&column(4), 1.0, "Tailwind's `starting:opacity-0`");
    let transforms: Vec<&str> = seen.iter().map(|one| one[4].1.as_str()).collect();
    assert!(transforms[0].contains("scale"), "Tailwind's `starting:scale-50` starts it smaller: {transforms:?}");
    assert!(!transforms.last().unwrap().contains("0.5"), "and it grows to its size: {transforms:?}");
}

#[test]
fn what_display_none_takes_away_stays_while_it_moves_away_and_comes_back_from_its_starting_style() {
    // `.closed { display: none; opacity: 0 }` with `transition: opacity 300ms, display 300ms allow-discrete`: it fades out, and
    // is only taken away once that's done. Shown again, it starts from `@starting-style`. Without `allow-discrete`, it goes at
    // once, and `visibility` is kept as it was until its transition's done, as it's always moved that way.
    let file = write(
        "exit",
        r#"
        import './app.css';
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const [open, setOpen] = useState(true);
            const shut = open ? '' : ' closed';
            return (
                <PageWindow><div>
                    <button key="toggle" onClick={() => setOpen(!open)}>toggle</button>
                    <div key="discrete" className={'panel discrete' + shut} />
                    <div key="sudden" className={'panel sudden' + shut} />
                    <div key="seen" className={'panel seen' + shut} />
                </div></PageWindow>
            );
        }
        "#,
        r#"
        .panel { width: 20px; height: 10px; background: red; opacity: 1; }
        .discrete { transition: opacity 300ms linear, display 300ms allow-discrete; }
        .sudden { transition: opacity 300ms linear, display 300ms; }
        .discrete.closed, .sudden.closed { display: none; opacity: 0; }
        .seen { transition: opacity 300ms linear, visibility 300ms; }
        .seen.closed { visibility: hidden; opacity: 0; }
        @starting-style { .discrete { opacity: 0; } }
        "#,
    );
    let mut hands = moving(&file);
    let shown = |hands: &mut Hands, key: &str| prop_of(hands, key, "unshown").is_none();
    let seen = after_click(&mut hands, "toggle", 12, |hands| {
        (opacity_of(hands, "discrete"), shown(hands, "discrete"), shown(hands, "sudden"), prop_of(hands, "seen", "inert").is_some(), opacity_of(hands, "seen"))
    });
    assert!(seen.iter().all(|one| one.1), "it's still shown while it fades: {seen:?}");
    assert!(seen.iter().any(|one| one.0 > 0.1 && one.0 < 0.9), "and it's partway at some point: {seen:?}");
    assert!(seen.iter().all(|one| !one.2), "without `allow-discrete` it's gone at once: {seen:?}");
    assert!(seen.iter().all(|one| !one.3), "what's going to `visibility: hidden` can still be seen: {seen:?}");
    assert!(seen.iter().any(|one| one.4 > 0.1 && one.4 < 0.9), "as it fades: {seen:?}");
    // Once its time's gone by, which JavaScript's timers go by as they always do, it's taken away.
    std::thread::sleep(Duration::from_millis(400));
    hands.app.tick(Instant::now() + Duration::from_secs(2));
    hands.settle();
    hands.app.tick(Instant::now() + Duration::from_secs(3));
    hands.settle();
    assert!(!shown(&mut hands, "discrete"), "once it's faded it's taken away");
    assert!(prop_of(&hands, "seen", "inert").is_some(), "and what's hidden can't be seen or clicked");
    // Shown again, it starts from its starting style.
    let back = after_click(&mut hands, "toggle", 25, |hands| opacity_of(hands, "discrete"));
    fades_in(&back, 1.0, "shown again");
}

#[test]
fn with_nothing_moving_what_display_none_takes_away_goes_at_once() {
    let file = write(
        "exit-still",
        r#"
        import './app.css';
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const [open, setOpen] = useState(true);
            return (
                <PageWindow><div>
                    <button key="toggle" onClick={() => setOpen(!open)}>toggle</button>
                    <div key="discrete" className={'panel' + (open ? '' : ' closed')} />
                </div></PageWindow>
            );
        }
        "#,
        r#"
        .panel { width: 20px; height: 10px; background: red; transition: opacity 300ms linear, display 300ms allow-discrete; }
        .closed { display: none; opacity: 0; }
        "#,
    );
    let mut hands = Hands::open(&file, (500, 400));
    hands.click("toggle");
    hands.settle();
    assert!(prop_of(&hands, "discrete", "unshown").is_some(), "it's gone at once");
}
