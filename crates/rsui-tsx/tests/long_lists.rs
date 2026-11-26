//! A long list made of a page's elements, as a page makes one: rows of `<div>`s in something that scrolls, the whole list rendered
//! again whenever the selection moves. What that costs follows what changed and what's seen rather than how many rows there are,
//! and what's skipped for being far from being seen (`content-visibility: auto`) still comes out as it would drawn: how far the
//! list scrolls, where each row is, and the text in each for whatever looks for it.
//!
//! What it costs is counted with `RSUI_SPLIT`, which says how many times each part ran in each step, as times vary.

use std::path::PathBuf;
use std::process::Command;

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

/// How many rows the list has.
const ROWS: usize = 2000;

/// The list, with rows 20 tall in an area that scrolls, which `css` can say more about. Up and End move the selection, which is
/// scrolled into view, as a file list's is.
fn app(name: &str, css: &str) -> PathBuf {
    app_with(name, css, r#"<span className="name">Row {index}</span>"#)
}

/// The same, with `content` in each row, which can use `index`.
fn app_with(name: &str, css: &str, content: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-long-lists-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("app.css"),
        format!(
            "body {{ margin: 0 }}\n.frame {{ height: 100vh; display: flex; flex-direction: column }}\n\
             .rows {{ flex: 1; min-height: 0; overflow-y: auto }}\n.row {{ height: 20px }}\n.selected {{ background: #cde }}\n{css}\n"
        ),
    )
    .unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(
        &file,
        format!(
            r#"
        import {{ memo, useLayoutEffect, useRef, useState }} from 'react';
        import {{ PageWindow }} from 'rsui/web';
        import './app.css';
        const COUNT = {ROWS};
        function Name({{ index }}: {{ index: number }}) {{
            return <span className="name">Row {{index}}</span>;
        }}
        const Row = memo(function Row({{ index, selected }}: {{ index: number; selected: boolean }}) {{
            const ref = useRef<HTMLDivElement>(null);
            useLayoutEffect(() => {{
                if (selected) {{
                    ref.current?.scrollIntoView({{ block: 'nearest' }});
                }}
            }}, [selected]);
            return (
                <div ref={{ref}} data-index={{index}} className={{'row' + (selected ? ' selected' : '')}}>
                    {content}
                </div>
            );
        }});
        function List() {{
            const [selected, setSelected] = useState(0);
            const onKeyDown = (event: any) => {{
                const next = {{ ArrowUp: selected - 1, End: COUNT - 1, Home: 0 }}[event.key as string];
                if (next !== undefined) {{
                    event.preventDefault();
                    setSelected(next);
                }}
            }};
            const rows = [];
            for (let index = 0; index < COUNT; index++) {{
                rows.push(<Row key={{index}} index={{index}} selected={{index === selected}} />);
            }}
            return (
                <div className="frame">
                    <div id="rows" tabIndex={{0}} onKeyDown={{onKeyDown}} className="rows">{{rows}}</div>
                </div>
            );
        }}
        export default function App() {{
            return <PageWindow><List /></PageWindow>;
        }}
    "#
        ),
    )
    .unwrap();
    file
}

/// Does the steps, sandboxed, with what each part costs said after each, and gives what was printed, failing if a step did.
fn run(file: &PathBuf, steps: &str) -> String {
    run_with(file, steps, &[])
}

/// The same, with more set in its environment.
fn run_with(file: &PathBuf, steps: &str, more: &[(&str, &str)]) -> String {
    let sandbox = file.with_file_name("sandbox");
    let output = Command::new(TOOL)
        .envs(more.iter().copied())
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
        .env("RSUI_SPLIT", "1")
        .env("RSUI_DO_SETTLE_MS", "0")
        .output()
        .unwrap();
    let said = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "{said}");
    assert!(!said.contains("TypeError") && !said.contains("ReferenceError"), "{said}");
    said
}

/// What the step that starts with `step` printed, up to the next step.
fn of_step<'a>(said: &'a str, step: &str) -> &'a str {
    let start = said.find(&format!("> {step}")).unwrap_or_else(|| panic!("no step {step:?} in\n{said}"));
    let rest = &said[start + 2..];
    &rest[..rest.find("\n> ").unwrap_or(rest.len())]
}

/// What `eval` gave in the step that starts with `step`.
fn value<'a>(said: &'a str, step: &str) -> &'a str {
    let part = of_step(said, step);
    part.lines()
        .find_map(|line| line.strip_prefix("  = "))
        .unwrap_or_else(|| panic!("{step:?} gave nothing in\n{part}"))
        .trim()
}

/// How many times `part` ran in the step that starts with `step`, as `RSUI_SPLIT` says, which is 0 if it didn't.
fn times(said: &str, step: &str, part: &str) -> u64 {
    of_step(said, step)
        .lines()
        .filter_map(|line| line.trim().strip_prefix("split "))
        .flat_map(|line| line.split(' '))
        .filter_map(|one| one.strip_prefix(part)?.strip_prefix('='))
        .filter_map(|one| one.split('/').nth(1)?.parse::<u64>().ok())
        .sum()
}

/// How many rows have what's in them drawn, as their names.
const NAMES: &str = r#"eval "document.querySelectorAll('.row .name').length""#;

#[test]
fn moving_the_selection_in_a_long_list_looks_at_what_changed() {
    let file = app("selection", "");
    let said = run(
        &file,
        "size 600 400\nwait 100\nclickat 100 10\npress end\npress up\n\
         eval \"document.querySelector('.selected').dataset.index\"\n\
         eval \"document.getElementById('rows').scrollTop\"\n\
         eval \"document.querySelector('[data-index=\\\"1998\\\"]').getBoundingClientRect().bottom\"\n\
         press home",
    );
    // Row 1999 is the last, at the bottom of the 400 that show, and Up selects the one above it, which shows already.
    assert_eq!(value(&said, "eval \"document.querySelector('.selected')"), "1998", "{said}");
    assert_eq!(value(&said, "eval \"document.getElementById('rows').scrollTop"), format!("{}", ROWS * 20 - 400), "{said}");
    assert_eq!(value(&said, "eval \"document.querySelector('[data-index"), "380", "{said}");
    // Rendering the list again is a few calls into JavaScript, rather than one for each of its rows, which asked the kind of each
    // again (`keptKids` in web.js). The first time it's rendered again it still asks, as what its rows are is only learned as
    // they're first drawn.
    let calls = times(&said, "press up", "js");
    assert!(calls < 200, "{calls} calls into JavaScript for {ROWS} rows\n{said}");
    // And what's in the area that scrolls is kept as it was, apart from the two rows that changed, rather than each row described
    // and built again (`scroll_component` in bridge.rs).
    let built = times(&said, "press up", "n:built");
    assert!(built < 20, "{built} rows built again of {ROWS}\n{said}");
    // Each row is compared with what it was once each time the list's rendered again, once it's been kept, and not again as it's drawn
    // again to be scrolled to the top (`keptKids` and `sameElements` in web.js).
    for step in ["press up", "press home"] {
        let compared = times(&said, step, "host:propsSame");
        assert!(compared < ROWS as u64 + 100, "{step}: rows compared {compared} times for {ROWS} rows\n{said}");
    }
    for step in ["press end", "press up"] {
        // Bringing the row into view finds what's around it, rather than making the whole page again (`nodeUpOf` in web.js).
        assert_eq!(times(&said, step, "pageTree-build"), 0, "{step}: the whole page was made again\n{said}");
    }
}

#[test]
fn going_to_the_end_of_a_long_list_lays_out_none_of_its_rows() {
    let file = app("end", "");
    let said = run(&file, "size 600 400\nwait 100\nclickat 100 10\npress end");
    // Scrolling to the end, with a classic scrollbar beside the rows, laid every row out twice, at the area's width and then at
    // what the scrollbar leaves (`BAR_DOWN` in rsui-layout).
    let solved = times(&said, "press end", "n:solved");
    assert!(solved < 100, "{solved} boxes laid out again for {ROWS} rows\n{said}");
}

#[test]
fn batching_leaf_rows_preserves_ordinary_and_memo_component_props() {
    let file = app("leaf-props", "");
    std::fs::write(&file, r#"
        'use no memo';
        import { memo, useState } from 'react';
        import { PageWindow } from 'rsui/web';
        const box = { current: 0 };
        const ids = Array.from({length: 80}, (_, i) => i);
        function Row({id, box}) { return <div className="plain">{id + ':' + box.current}</div> }
        const MemoRow = memo(function({id, box}) { return <div className="memo">{id + ':' + box.current}</div> });
        function List() {
            const [revision, setRevision] = useState(0);
            const order = revision === 2 ? [...ids].reverse() : ids;
            return <div>
                <button onClick={() => { box.current++; setRevision(revision + 1) }}>Advance</button>
                <div>{order.map(id => <Row key={id} id={id} box={box}/>)}</div>
                <div>{order.map(id => <MemoRow key={id} id={id} box={box}/>)}</div>
            </div>
        }
        export default function Root() { return <PageWindow><List/></PageWindow> }
    "#).unwrap();
    let said = run(
        &file,
        r#"click "Advance"
        eval "document.querySelector('.plain').textContent + '/' + document.querySelector('.memo').textContent"
        click "Advance"
        eval "document.querySelector('.plain').textContent + ' vs ' + document.querySelector('.memo').textContent""#,
    );
    assert_eq!(
        value(
            &said,
            r#"eval "document.querySelector('.plain').textContent + '/'"#
        ),
        "0:1/0:0",
        "{said}"
    );
    assert_eq!(
        value(
            &said,
            r#"eval "document.querySelector('.plain').textContent + ' vs '"#
        ),
        "79:2 vs 79:0",
        "{said}"
    );
}

/// Rows that skip what's in them until they're near being seen, taken to be as tall as they are.
const SKIPPING: &str = ".row { content-visibility: auto; contain-intrinsic-size: auto 20px }";

fn grouped_app(name: &str) -> PathBuf {
    let file = app(name, "");
    std::fs::write(
        &file,
        r#"
        import { PageWindow } from 'rsui/web';
        import { useLayoutEffect, useState } from 'react';
        globalThis.mounted = 0;
        function Counter() {
            const [count, update] = useState(0);
            useLayoutEffect(() => { globalThis.mounted += 1 }, []);
            return <button id="stateful" onClick={() => update(count + 1)}>Count {count}</button>;
        }
        export default function App() {
            return <PageWindow><div style={{ height: 360, overflowY: 'auto' }} id="scrollport">
                <div>
                    {Array.from({ length: 12 }, (_, group) => (
                        <section key={group} id={'group-' + group} tabIndex={0} aria-label={'Group ' + group}>
                            <h2>Heading {group}</h2>
                            {Array.from({ length: 16 }, (_, row) => (
                                <div key={row} tabIndex={row === 15 ? -1 : undefined} className="group-row"
                                    id={'row-' + group + '-' + row} style={{ height: 64 }}>
                                    <span><strong>{group}/{row}</strong></span>
                                </div>
                            ))}
                            {group === 10 ? <div><Counter/></div> : null}
                        </section>
                    ))}
                </div>
            </div></PageWindow>;
        }
        "#,
    )
    .unwrap();
    file
}

#[test]
fn grouped_scroll_views_defer_plain_rows_and_keep_dom_targets() {
    let file = grouped_app("grouped-dom");
    let steps = r#"size 600 400
wait 100
eval "document.querySelectorAll('.group-row').length"
eval "document.querySelector('#group-7').getBoundingClientRect().height < 1000"
eval "document.querySelector('#group-0').getBoundingClientRect().height > 1000"
eval "globalThis.target = document.querySelector('#row-7-15'); target.isConnected && target.closest('section').id === 'group-7'"
eval "target.setAttribute('data-test', 'kept'); target.style.color = 'red'; target.addEventListener('click', () => globalThis.heard = true)"
wait 100
eval "target === document.querySelector('#row-7-15') && target.dataset.test === 'kept' && getComputedStyle(target).color === 'rgb(255, 0, 0)'"
eval "target.click(); globalThis.heard === true"
eval "globalThis.viewport = document.querySelector('#scrollport'); globalThis.end = document.querySelector('#row-11-15'); end.scrollIntoView({block:'end'})"
wait 100
eval "Math.abs(end.getBoundingClientRect().bottom - viewport.getBoundingClientRect().bottom) < 1 && end === document.querySelector('#row-11-15')"
eval "globalThis.focusRow = document.querySelector('#row-4-15'); focusRow.focus({preventScroll:true})"
wait 100
eval "document.activeElement === focusRow && Math.abs(viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop) < 1"
eval "globalThis.mounted === 1 && document.querySelector('#stateful').textContent === 'Count 0'"
eval "document.querySelector('#stateful').click()"
wait 100
eval "viewport.scrollTop = 0"
wait 100
eval "document.querySelector('#stateful').textContent === 'Count 1' && globalThis.mounted === 1"
"#;
    let said = run(&file, steps);
    let answers = values(&said);
    assert_eq!(answers[0..4], ["192", "true", "true", "true"], "{said}");
    for index in [5, 6, 8, 10, 11, 14] {
        assert_eq!(answers[index], "true", "answer {index}\n{said}");
    }
    let eager = run_with(&file, steps, &[("RSUI_AUTO_SKIP", "0")]);
    assert_eq!(values(&eager)[1], "false", "{eager}");
}

#[test]
fn grouped_scroll_views_anchor_resolved_estimates_and_follow_tab_order() {
    let file = grouped_app("grouped-anchor");
    let said = run(
        &file,
        r#"size 600 400
wait 100
eval "globalThis.viewport = document.querySelector('#scrollport'); globalThis.group = document.querySelector('#group-7'); viewport.scrollTop = group.getBoundingClientRect().top - viewport.getBoundingClientRect().top + 100"
wait 100
eval "Math.round(group.getBoundingClientRect().top - viewport.getBoundingClientRect().top)"
eval "[6, 7].every(i => document.querySelector('#group-'+i).getBoundingClientRect().height > 1000) && document.querySelector('#group-4').getBoundingClientRect().height < 1000"
eval "group.focus()"
wait 40
press Tab
wait 100
eval "document.activeElement.id"
press Shift+Tab
wait 100
eval "document.activeElement.id"
wheel 200 250 0 -2100
wait 100
eval "document.querySelector('#group-4').getBoundingClientRect().height > 1000"
"#,
    );
    assert_eq!(
        values(&said)[1..],
        ["-100", "true", "undefined", "group-8", "group-7", "true"],
        "{said}"
    );
}

#[test]
fn rows_far_from_being_seen_are_as_tall_as_they_say_with_nothing_in_them() {
    let file = app("skipping", SKIPPING);
    let said = run(
        &file,
        &format!(
            "size 600 400\nwait 100\n{NAMES}\n\
             eval \"document.getElementById('rows').scrollHeight\"\n\
             eval \"document.querySelector('[data-index=\\\"1500\\\"]').getBoundingClientRect().height\"\n\
             eval \"document.querySelectorAll('.row').length\"\n\
             expect \"Row 1500\""
        ),
    );
    // What shows and what's within half the window below it, which starts at 960 by 640 before `size`: 32 rows show, and 24
    // more are within 480 of them. The rest aren't drawn.
    let drawn: usize = value(&said, "eval \"document.querySelectorAll('.row .name')").parse().unwrap();
    assert!((20..=100).contains(&drawn), "{drawn} rows drawn\n{said}");
    // The list is as long as it would be with all of them drawn, and each row is where it would be.
    assert_eq!(value(&said, "eval \"document.getElementById('rows').scrollHeight"), format!("{}", ROWS * 20), "{said}");
    assert_eq!(value(&said, "eval \"document.querySelector('[data-index"), "20", "{said}");
    // Every row is an element of the page, drawn or not, and what's in one that isn't is found by its text.
    assert_eq!(value(&said, "eval \"document.querySelectorAll('.row').length"), format!("{ROWS}"), "{said}");
}

#[test]
fn rows_brought_near_are_drawn_and_stay_drawn() {
    let file = app("brought-near", SKIPPING);
    let said = run(
        &file,
        &"size 600 400\nwait 100\nclickat 100 10\npress end\n\
             eval \"document.querySelectorAll('.row .name').length\"\n\
             eval \"document.querySelector('[data-index=\\\"1999\\\"] .name').textContent\"\n\
             eval \"document.querySelector('[data-index=\\\"1000\\\"] .name') === null\"\n\
             eval \"document.querySelector('[data-index=\\\"0\\\"] .name') !== null\"\n\
             eval \"document.getElementById('rows').scrollTop\"".to_string(),
    );
    // The last rows are drawn once the selection's gone to them, and the first stay as they were, while what's between them,
    // which was never near, isn't.
    assert_eq!(value(&said, "eval \"document.querySelector('[data-index=\\\"1999\\\"]"), "Row 1999", "{said}");
    assert_eq!(value(&said, "eval \"document.querySelector('[data-index=\\\"1000\\\"]"), "true", "{said}");
    assert_eq!(value(&said, "eval \"document.querySelector('[data-index=\\\"0\\\"]"), "true", "{said}");
    let drawn: usize = value(&said, "eval \"document.querySelectorAll('.row .name').length").parse().unwrap();
    assert!((40..=150).contains(&drawn), "{drawn} rows drawn\n{said}");
    assert_eq!(value(&said, "eval \"document.getElementById('rows').scrollTop"), format!("{}", ROWS * 20 - 400), "{said}");
}

#[test]
fn a_row_that_is_not_the_size_it_says_is_that_size_once_drawn() {
    // Rows are 30 tall, and say they're 20 while they skip what's in them.
    let file = app("other-size", ".row { height: auto; content-visibility: auto; contain-intrinsic-size: 20px } .name { display: block; height: 30px }");
    let said = run(
        &file,
        "size 600 400\nwait 100\n\
         eval \"document.querySelector('[data-index=\\\"0\\\"]').getBoundingClientRect().height\"\n\
         eval \"document.querySelector('[data-index=\\\"1500\\\"]').getBoundingClientRect().height\"\n\
         eval \"document.getElementById('rows').scrollHeight\"\n\
         eval \"document.querySelectorAll('.row .name').length\"",
    );
    assert_eq!(value(&said, "eval \"document.querySelector('[data-index=\\\"0\\\"]"), "30", "{said}");
    assert_eq!(value(&said, "eval \"document.querySelector('[data-index=\\\"1500\\\"]"), "20", "{said}");
    let drawn: usize = value(&said, "eval \"document.querySelectorAll('.row .name')").parse().unwrap();
    let height: usize = value(&said, "eval \"document.getElementById('rows').scrollHeight").parse().unwrap();
    assert_eq!(height, drawn * 30 + (ROWS - drawn) * 20, "{said}");
}

#[test]
fn hidden_rows_never_draw_what_is_in_them() {
    let file = app("hidden", ".row { content-visibility: hidden; contain-intrinsic-size: 0 12px }");
    let said = run(
        &file,
        "size 600 400\nwait 100\n\
         eval \"document.querySelectorAll('.row .name').length\"\n\
         eval \"document.querySelector('[data-index=\\\"0\\\"]').getBoundingClientRect().height\"\n\
         eval \"document.getElementById('rows').scrollHeight\"",
    );
    assert_eq!(value(&said, "eval \"document.querySelectorAll('.row .name')"), "0", "{said}");
    // The rows are 20 tall as `.row` says, whatever's said for what's in them.
    assert_eq!(value(&said, "eval \"document.querySelector('[data-index"), "20", "{said}");
    assert_eq!(value(&said, "eval \"document.getElementById('rows').scrollHeight"), format!("{}", ROWS * 20), "{said}");
}

/// Rows that are as tall as they say whatever's in them: flex boxes of a height of their own, in a block that scrolls.
const OWN_BOX: &str = ".row { display: flex; align-items: center }";

/// How many rows have what's in them drawn, as the boxes of their names in the window's tree as it's printed at the end.
fn drawn(said: &str) -> usize {
    said[said.rfind("\nwindow ").unwrap_or(0)..].matches("class=\"name\"").count()
}

/// What each `eval` gave, in order.
fn values(said: &str) -> Vec<&str> {
    said.lines().filter_map(|line| line.strip_prefix("  = ")).collect()
}

/// Steps that ask what's in the list as the page has it, wherever it's drawn.
const ASKED: &str = "eval \"document.querySelectorAll('.row .name').length\"\n\
    eval \"document.querySelector('[data-index=\\\"1500\\\"]').textContent\"\n\
    eval \"document.querySelector('[data-index=\\\"1500\\\"] .name').parentElement.dataset.index\"\n\
    eval \"[...document.querySelectorAll('.name')].slice(1498, 1502).map((one) => one.textContent).join('|')\"\n\
    eval \"document.getElementById('rows').scrollHeight\"\n\
    eval \"JSON.stringify(document.querySelector('[data-index=\\\"1500\\\"]').getBoundingClientRect())\"\n\
    eval \"document.getElementById('rows').textContent.length\"";

#[test]
fn rows_of_a_long_list_far_from_being_seen_skip_what_is_in_them_by_themselves() {
    let file = app("by-themselves", OWN_BOX);
    let steps = format!("size 600 400\nwait 100\n{ASKED}");
    let said = run(&file, &steps);
    // What shows and what's within half the window of it is drawn, and the rest of the 2,000 isn't, with nothing said in CSS.
    let shown = drawn(&said);
    assert!((20..=100).contains(&shown), "{shown} rows drawn\n{said}");
    // And the page is as it would be with all of them drawn: each row's element is there with what's in it, in order, with its text,
    // and each row is where it would be.
    assert_eq!(value(&said, "eval \"document.querySelectorAll('.row .name')"), format!("{ROWS}"), "{said}");
    assert_eq!(value(&said, "eval \"document.querySelector('[data-index=\\\"1500\\\"]').textContent"), "Row 1500", "{said}");
    assert_eq!(value(&said, "eval \"document.querySelector('[data-index=\\\"1500\\\"] .name')"), "1500", "{said}");
    assert_eq!(value(&said, "eval \"[...document"), "Row 1498|Row 1499|Row 1500|Row 1501", "{said}");
    assert_eq!(value(&said, "eval \"document.getElementById('rows').scrollHeight"), format!("{}", ROWS * 20), "{said}");
    let text: usize = (0..ROWS).map(|at| format!("Row {at}").len()).sum();
    assert_eq!(value(&said, "eval \"document.getElementById('rows').textContent"), format!("{text}"), "{said}");
    // All of which is what it says with every row drawn, where the row is too.
    let whole = run_with(&file, &steps, &[("RSUI_AUTO_SKIP", "0")]);
    assert_eq!(drawn(&whole), ROWS, "{whole}");
    assert_eq!(values(&said), values(&whole), "{said}\n{whole}");
}

#[test]
fn rows_skipped_by_themselves_are_drawn_as_they_are_brought_near() {
    let file = app("skipped-brought-near", OWN_BOX);
    let steps = "size 600 400\nwait 100\nclickat 100 10\npress end\npress up\n\
         eval \"document.querySelector('.selected').dataset.index\"\n\
         eval \"document.getElementById('rows').scrollTop\"\n\
         eval \"[...document.querySelectorAll('.name')].slice(1996).map((one) => one.textContent).join('|')\"\n\
         eval \"document.querySelector('[data-index=\\\"1999\\\"] .name').getBoundingClientRect().bottom\"";
    let said = run(&file, steps);
    assert_eq!(value(&said, "eval \"document.querySelector('.selected')"), "1998", "{said}");
    assert_eq!(value(&said, "eval \"document.getElementById('rows').scrollTop"), format!("{}", ROWS * 20 - 400), "{said}");
    // What's at the end is in its place among the rest, drawn, where the window shows it.
    assert_eq!(value(&said, "eval \"[...document"), "Row 1996|Row 1997|Row 1998|Row 1999", "{said}");
    let bottom: f64 = value(&said, "eval \"document.querySelector('[data-index=\\\"1999\\\"] .name')").parse().unwrap();
    assert!(bottom > 380.0 && bottom <= 400.0, "{bottom}\n{said}");
    // The rows at the start were drawn, and the ones at the end, and few between.
    let shown = drawn(&said);
    assert!((40..=150).contains(&shown), "{shown} rows drawn\n{said}");
    assert_eq!(values(&said), values(&run_with(&file, steps, &[("RSUI_AUTO_SKIP", "0")])), "{said}");
}

#[test]
fn rows_whose_size_could_follow_what_is_in_them_are_drawn_whole() {
    // A row that's a block, or as tall as what's in it, or that has a component in it, which only says what it makes once it's
    // drawn, is drawn with all that's in it, as before.
    for (name, css, content) in [
        ("block", "", r#"<span className="name">Row {index}</span>"#),
        ("auto", ".row { display: flex; height: auto }", r#"<span className="name">Row {index}</span>"#),
        ("component", OWN_BOX, "<Name index={index} />"),
    ] {
        let file = app_with(name, css, content);
        let said = run(&file, "size 600 400\nwait 100\neval \"document.querySelectorAll('.row .name').length\"");
        assert_eq!(drawn(&said), ROWS, "{name}\n{said}");
        assert_eq!(value(&said, "eval \"document.querySelectorAll('.row .name')"), format!("{ROWS}"), "{name}\n{said}");
    }
}

#[test]
fn what_code_asks_of_a_skipped_row_has_it_drawn() {
    let file = app("asked", OWN_BOX);
    // Where something in a row that's skipped is, and how it looks, is only known once it's drawn, which asking has it be: the next
    // time it's asked, it says what it would with every row drawn. Bringing it into view brings its row, which draws it as it comes
    // near. Its HTML is what's written in it.
    let ask = "eval \"document.querySelector('[data-index=\\\"1500\\\"]').innerHTML\"\n\
         eval \"document.querySelector('[data-index=\\\"1400\\\"] .name').offsetHeight > -1\"\n\
         wait 100\n\
         eval \"JSON.stringify(document.querySelector('[data-index=\\\"1400\\\"] .name').getBoundingClientRect())\"\n\
         eval \"getComputedStyle(document.querySelector('[data-index=\\\"1400\\\"] .name')).display\"";
    let steps = format!("size 600 400\nwait 100\n{ask}\neval \"document.querySelector('[data-index=\\\"1500\\\"] .name').scrollIntoView()\"\nwait 100\n\
         eval \"Math.abs(document.getElementById('rows').scrollTop - 30000) <= 20\"\n\
         eval \"document.querySelector('[data-index=\\\"1500\\\"] .name').getBoundingClientRect().height > 0\"");
    let said = run(&file, &steps);
    let whole = run_with(&file, &steps, &[("RSUI_AUTO_SKIP", "0")]);
    assert_eq!(values(&said), values(&whole), "{said}\n{whole}");
    assert_eq!(values(&said)[values(&said).len() - 2..], ["true", "true"], "{said}");
}

#[test]
fn a_long_list_in_the_page_as_it_scrolls_skips_rows_too() {
    // The rows are in the page, which scrolls, rather than in a box of their own that does.
    let file = app("in-the-page", ".frame { height: auto; display: block } .rows { overflow: visible } .row { display: flex; align-items: center }");
    let steps = format!("size 600 400\nwait 100\n{ASKED}");
    let said = run(&file, &steps);
    let shown = drawn(&said);
    assert!((20..=100).contains(&shown), "{shown} rows drawn\n{said}");
    let whole = run_with(&file, &steps, &[("RSUI_AUTO_SKIP", "0")]);
    assert_eq!(drawn(&whole), ROWS, "{whole}");
    assert_eq!(values(&said), values(&whole), "{said}\n{whole}");
}

#[test]
fn rows_that_can_be_edited_are_drawn_whole() {
    let file = app_with("edited", OWN_BOX, r#"<span className="name" contentEditable>Row {index}</span>"#);
    let said = run(&file, "size 600 400\nwait 100");
    assert_eq!(drawn(&said), ROWS, "{said}");
}

/// The tree of the window as it's printed at the end, which is everything drawn, as it's drawn.
fn tree(said: &str) -> &str {
    &said[said.rfind("\nwindow ").unwrap_or(0)..]
}

/// Rows that are one of a few kinds, with what Tailwind 4 says of every element, which is about it alone, and a state of their own.
const KINDS: &str = ".row { display: flex; align-items: center } .row:hover { background: #eee } \
    *:where(:not(iframe)) { box-sizing: border-box; --tw-x: 0 } .name { color: #333 }";

#[test]
fn rows_of_a_long_list_start_from_another_of_their_kind() {
    let file = app("kinds", KINDS);
    let steps = format!("size 600 400\nwait 100\nhoverat 100 50\nwait 50\neval \"globalThis.__rsuiSharedLooksUsed\"\n{ASKED}");
    let said = run(&file, &steps);
    // Every row but the first of each kind, selected or not, starts from that one's look rather than working its own out.
    let shared: usize = value(&said, "eval \"globalThis.__rsuiSharedLooksUsed").parse().unwrap();
    assert!(shared >= ROWS - 10, "{shared} of {ROWS} rows shared a look\n{said}");
    // And everything is drawn as it is with each working its own out, the hovered row too.
    let alone = run_with(&file, &steps, &[("RSUI_SHARED_LOOKS", "0")]);
    assert_eq!(value(&alone, "eval \"globalThis.__rsuiSharedLooksUsed"), "0", "{alone}");
    assert_eq!(values(&said)[1..], values(&alone)[1..], "{said}\n{alone}");
    assert_eq!(tree(&said), tree(&alone));
}

#[test]
fn a_row_with_more_to_it_than_its_kind_has_its_own_look() {
    // Rules about where a row is, about what it's given, and about what it's in, and a row with a `style` of its own, each make what a
    // row comes to more than its kind, so each such row works it out itself, and comes out as it does with nothing shared.
    for (name, css) in [
        ("attribute", "[data-index=\"7\"] { background: #f00 }"),
        ("position", ".row:nth-child(3n) { background: #0f0 }"),
        ("inside", ".rows > .row { margin-left: 2px } .frame .selected { background: #00f }"),
        ("inherit", ".row { padding-left: inherit } .rows { padding-left: 3px }"),
    ] {
        let file = app(name, &format!("{KINDS} {css}"));
        let steps = format!("size 600 400\nwait 100\nclickat 100 10\npress end\npress home\neval \"globalThis.__rsuiSharedLooksUsed\"\n{ASKED}");
        let said = run(&file, &steps);
        let alone = run_with(&file, &steps, &[("RSUI_SHARED_LOOKS", "0")]);
        assert_eq!(values(&said)[1..], values(&alone)[1..], "{name}\n{said}\n{alone}");
        assert_eq!(tree(&said), tree(&alone), "{name}");
    }
}

#[test]
fn a_rows_element_does_what_a_page_element_does() {
    let file = app("element", KINDS);
    let row = "document.querySelector('[data-index=\\\"3\\\"]')";
    let steps = format!(
        "size 600 400\nwait 100\n\
         eval \"Object.getOwnPropertyNames({row}).length < 16\"\n\
         eval \"{row} instanceof HTMLDivElement && {row}.tagName === 'DIV' && {row}.localName === 'div'\"\n\
         eval \"{row}.classList.contains('row') && {row}.dataset.index === '3' && {row}.textContent === 'Row 3'\"\n\
         eval \"let heard = 0; {row}.addEventListener('click', () => heard++); {row}.click(); heard\"\n\
         eval \"{row}.setAttribute('data-x', 'y'); {row}.getAttribute('data-x')\"\n\
         eval \"{row}.style.color = 'rgb(255, 0, 0)'; {row}.style.color\"\n\
         wait 50\n\
         eval \"getComputedStyle({row}).color\"\n\
         eval \"{row} === document.querySelector('[data-index=\\\"3\\\"]') && {row}.parentElement.id\""
    );
    let said = run(&file, &steps);
    // Each element's handle has only what's its own, as its key, its classes and where it was found, and gets the rest from what every
    // element with its tag shares, where each had all of what an element does of its own, 70 and more methods and accessors.
    assert_eq!(values(&said), ["true", "true", "true", "1", "y", "rgb(255, 0, 0)", "rgb(255, 0, 0)", "rows"], "{said}");
}
