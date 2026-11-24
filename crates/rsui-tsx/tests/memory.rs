//! What an app keeps as it's used for a long time: drawn again and again, the engine keeps no more than it did after the
//! first few times, and nor does anything kept for it here.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn write(name: &str, source: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-memory-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    file
}

/// Fewer times round when the collector runs at every safe point, which makes each far slower.
fn shorter() -> u32 {
    if std::env::var_os("RSUI_GC_STRESS").is_some_and(|value| value != "0") {
        50
    } else {
        1
    }
}

/// What's kept: the engine's census, and the sizes of everything kept for the app in Rust.
struct Kept {
    census: rsui_js::Census,
    tables: Vec<(&'static str, usize)>,
}

fn kept(hands: &Hands) -> Kept {
    let mut tables = rsui_tsx::tables();
    let [places, mounted, effects, cleanups] = hands.app.runtime().sizes();
    tables.extend([
        ("component places", places),
        ("mounted components", mounted),
        ("waiting effects", effects),
        ("waiting cleanups", cleanups),
    ]);
    Kept {
        census: rsui_tsx::census(),
        tables,
    }
}

fn show(times: u32, kept: &Kept) -> String {
    let tables: Vec<String> = kept
        .tables
        .iter()
        .map(|(name, size)| format!("{name} {size}"))
        .collect();
    format!(
        "after {times}: {}tables: {}\n",
        kept.census.report(12),
        tables.join(", ")
    )
}

const NEW_COMPONENT_EACH_TIME: &str = r#"
import { useState } from 'react';
export default function App() {
    const [count, setCount] = useState(0);
    // A new component each time this is drawn, so what was there is unmounted and this is mounted in its place.
    function Inner({ label }) {
        const [shown] = useState(label);
        return <text key="inner">{'inner ' + shown}</text>;
    }
    return (
        <window>
            <button key="more" onClick={() => setCount((was) => was + 1)}>more</button>
            <text key="count">{'count ' + count}</text>
            <Inner label={'at ' + count} />
        </window>
    );
}
"#;

/// Clicks `key` this many times, as a window does it.
fn click_times(hands: &mut Hands, key: &str, times: u32) {
    let (x, y) = hands.middle_of(key);
    hands.app.pointer_move(x, y);
    hands.settle();
    for _ in 0..times {
        hands.app.pointer_down(x, y, 1);
        let _ = hands.app.pointer_up(x, y);
        hands.settle();
    }
}

/// Shows what a component made anew each time it's drawn keeps, after 100, 1,000 and 10,000 times, and what keeps the
/// most recent of what grew. It's for looking at with `--ignored --nocapture`, not a test.
#[test]
#[ignore = "for looking at with --nocapture, not a test"]
fn census_of_a_new_component_each_time() {
    let mut hands = Hands::open(&write("census", NEW_COMPONENT_EACH_TIME), (400, 300));
    let mut done = 0;
    let mut before: Option<rsui_js::Census> = None;
    for times in [100, 1_000, 10_000] {
        click_times(&mut hands, "more", times - done);
        done = times;
        assert!(
            hands.says(&format!("count {times}")),
            "{}",
            hands.app.outline()
        );
        let now = kept(&hands);
        print!("{}", show(times, &now));
        if let Some(before) = &before {
            print!("grew: {}", now.census.growth_since(before, 12));
        }
        before = Some(now.census);
    }
    for label in ["Inner", "Object"] {
        for path in rsui_tsx::retainers(label, 3) {
            println!("{label}: {path}");
        }
    }
}

/// A page that makes a new component each time it's drawn, drawn 5,000 times: the engine keeps no more than after
/// 1,000, nothing a click made is left on the list of what natives are holding (which only empties when JavaScript runs
/// from the top, as jobs and timers do, and this page has none), and nothing kept for the app in Rust has grown at all.
/// The engine's list of objects only goes as far as the most it's had at once, which settles at less than twice what's
/// alive; while each click's new component was kept, it grew by one place each time, to six times as much by now.
#[test]
fn a_new_component_each_time_it_is_drawn_keeps_nothing_more() {
    let mut hands = Hands::open(&write("new-each-time", NEW_COMPONENT_EACH_TIME), (400, 300));
    let (some, more) = (1_000 / shorter(), 5_000 / shorter());
    click_times(&mut hands, "more", some);
    let after_some = kept(&hands);
    click_times(&mut hands, "more", more - some);
    assert!(hands.says(&format!("count {more}")), "{}", hands.app.outline());
    let after_more = kept(&hands);
    let shown = format!("{}{}", show(some, &after_some), show(more, &after_more));
    assert!(after_more.census.nursery <= after_some.census.nursery, "{shown}");
    assert_eq!(after_some.census.objects, after_more.census.objects, "{shown}");
    assert_eq!(after_some.census.strings, after_more.census.strings, "{shown}");
    assert_eq!(after_some.census.cells, after_more.census.cells, "{shown}");
    assert!(
        after_more.census.object_places <= after_more.census.objects * 3,
        "{shown}"
    );
    assert_eq!(after_some.tables, after_more.tables, "{shown}");
}

/// Structural metadata belongs to the current page snapshot; filtered ranks belong to one query.
/// Repeatedly replacing the snapshot must not retain earlier metadata or query maps.
#[test]
fn selector_queries_release_weak_targets_between_sync_events() {
    let mut hands = Hands::open(
        &write(
            "selector-retention",
            r#"
        import { useState } from 'react';
        export default function App() {
            const [flipped, setFlipped] = useState(false);
            const [said, setSaid] = useState('ready');
            const ids = Array.from({ length: 96 }, (_, i) => i);
            if (flipped) ids.reverse();
            const ask = () => {
                try {
                    const check = (selector, expected) => {
                        const found = [...document.querySelectorAll(selector)].map(e => e.getAttribute('id'));
                        if (found.join(',') !== expected.map(i => 'i' + i).join(',')) throw new Error(selector);
                    };
                    check('#items > :nth-child(2n)', ids.filter((_, i) => i % 2 === 1));
                    check('#items > span:nth-of-type(2n)', ids.filter(i => i % 2 === 0).filter((_, i) => i % 2 === 1));
                    check('#items > :nth-child(2n of .chosen)', ids.filter(i => i % 3 === 0).filter((_, i) => i % 2 === 1));
                    check('#items > :nth-last-child(2n of .chosen)', ids.filter(i => i % 3 === 0).filter((_, i) => i % 2 === 0));
                    if (document.querySelector('#items:has(> .chosen)')?.getAttribute('id') !== 'items') throw new Error('has');
                    setSaid('PASS');
                } catch (error) { setSaid('FAIL ' + error); }
            };
            return <div>
                <button key="query" onClick={ask}>query</button>
                <button key="flip" onClick={() => setFlipped(was => !was)}>flip</button>
                <p>{said}</p>
                <div id="items">{ids.map(i => {
                    const Tag = i % 2 === 0 ? 'span' : 'div';
                    return <Tag key={i} id={'i' + i} className={i % 3 === 0 ? 'chosen' : ''} />;
                })}</div>
            </div>;
        }
        "#,
        ),
        (500, 400),
    );
    let mut exercise = |times| {
        for _ in 0..times {
            // Collect at the same event boundary each cycle. The next page snapshot
            // sweeps dead weak-cache entries before creating its replacement entries.
            rsui_tsx::census();
            hands.click("flip");
            hands.click("query");
            assert!(hands.says("PASS"), "{}", hands.app.outline());
        }
    };
    exercise(10);
    let warm = rsui_tsx::census();
    // How much room the objects take goes up and down by a step from one cycle to the next, with the same objects, as a
    // `Map` or `Set` that has things taken out and put back grows its table and compacts it again. So it's the most of a
    // few cycles here against the least of a few later, which only something that keeps growing goes over.
    let mut warm_bytes = warm.object_bytes;
    for _ in 0..4 {
        exercise(1);
        warm_bytes = warm_bytes.max(rsui_tsx::census().object_bytes);
    }
    exercise(50);
    let later = rsui_tsx::census();
    let mut later_bytes = later.object_bytes;
    for _ in 0..4 {
        exercise(1);
        later_bytes = later_bytes.min(rsui_tsx::census().object_bytes);
    }
    let detail = format!("warm: {}later: {}", warm.report(8), later.report(8));
    println!("{detail}");
    assert_eq!(later.objects, warm.objects, "{detail}");
    assert!(later_bytes <= warm_bytes, "{later_bytes} > {warm_bytes}: {detail}");
    assert_eq!(later.strings, warm.strings, "{detail}");
    assert_eq!(later.cells, warm.cells, "{detail}");
    assert_eq!(later.rooted, warm.rooted, "{detail}");
}

/// Pending microtasks share the current WeakRef lifetime; the following click does not.
#[test]
fn weak_targets_survive_microtasks_and_expire_before_the_next_event() {
    let mut hands = Hands::open(
        &write(
            "weak-checkpoint",
            r#"
        import { useState } from 'react';
        let weak;
        export default function App() {
            const [said, setSaid] = useState('ready');
            const start = () => {
                weak = new WeakRef({ message: 'alive' });
                queueMicrotask(() => {
                    // Exercise collection safe points with RSUI_GC_STRESS enabled.
                    for (let i = 0; i < 32; i++) Array.from({ length: 8 }, () => ({}));
                    const first = weak.deref()?.message;
                    Promise.resolve().then(() => {
                        setSaid(first === 'alive' && weak.deref()?.message === 'alive' ? 'microtasks PASS' : 'microtasks FAIL');
                    });
                });
            };
            return <div>
                <button key="start" onClick={start}>start</button>
                <button key="check" onClick={() => setSaid(weak.deref() === undefined ? 'released PASS' : 'released FAIL')}>check</button>
                <p>{said}</p>
            </div>;
        }
        "#,
        ),
        (400, 300),
    );
    hands.click("start");
    assert!(hands.says("microtasks PASS"), "{}", hands.app.outline());
    rsui_tsx::census();
    hands.click("check");
    assert!(hands.says("released PASS"), "{}", hands.app.outline());
}
