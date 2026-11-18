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
        const Row = memo(function Row({{ index, selected }}: {{ index: number; selected: boolean }}) {{
            const ref = useRef<HTMLDivElement>(null);
            useLayoutEffect(() => {{
                if (selected) {{
                    ref.current?.scrollIntoView({{ block: 'nearest' }});
                }}
            }}, [selected]);
            return (
                <div ref={{ref}} data-index={{index}} className={{'row' + (selected ? ' selected' : '')}}>
                    <span className="name">Row {{index}}</span>
                </div>
            );
        }});
        function List() {{
            const [selected, setSelected] = useState(0);
            const onKeyDown = (event: any) => {{
                const next = {{ ArrowUp: selected - 1, End: COUNT - 1 }}[event.key as string];
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
    let sandbox = file.with_file_name("sandbox");
    let output = Command::new(TOOL)
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
         eval \"document.querySelector('[data-index=\\\"1998\\\"]').getBoundingClientRect().bottom\"",
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

/// Rows that skip what's in them until they're near being seen, taken to be as tall as they are.
const SKIPPING: &str = ".row { content-visibility: auto; contain-intrinsic-size: auto 20px }";

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
        &format!(
            "size 600 400\nwait 100\nclickat 100 10\npress end\n\
             eval \"document.querySelectorAll('.row .name').length\"\n\
             eval \"document.querySelector('[data-index=\\\"1999\\\"] .name').textContent\"\n\
             eval \"document.querySelector('[data-index=\\\"1000\\\"] .name') === null\"\n\
             eval \"document.querySelector('[data-index=\\\"0\\\"] .name') !== null\"\n\
             eval \"document.getElementById('rows').scrollTop\""
        ),
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
