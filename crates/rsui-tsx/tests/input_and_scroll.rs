//! What was found missing in the kitchen sinks, used headless with `--do` as a person would, each in a sandbox of its own:
//! clicks and hovers that were lost or told twice when what was pressed set off something put in before it; variables and
//! `inherit` that still reach what uses them when what's in between isn't drawn again; and `<html>`'s box, which floating-ui
//! places dropdowns from, moving up as the page scrolls.

use std::path::PathBuf;
use std::process::Command;

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn app(name: &str, source: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-input-scroll-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    file
}

/// Does the steps, and says what was printed, failing if a step did or the app said something was wrong.
fn works(file: &PathBuf, steps: &str) -> String {
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
        .output()
        .unwrap();
    let said = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "{said}");
    assert!(!said.contains("TypeError"), "{said}");
    said
}

#[test]
fn clicks_and_hovers_are_neither_lost_nor_told_twice_when_a_press_puts_something_before_them() {
    // Each press puts something in before the rows, or takes it out, as a page's `onMouseDown` can, while a timer keeps the page
    // busy. The press and the release are each applied before the next comes, as a window does, with no frame between them,
    // and some presses come where no move said the pointer had got to. The base lost 7 of the 8 clicks, as the release was
    // found at another place among what's around it than the press, and told each row it was entered twice.
    let file = app(
        "presses",
        r#"
        import { PageWindow } from 'rsui/web';
        import { useEffect, useState } from 'react';
        const busy = (ms: number) => { const end = performance.now() + ms; while (performance.now() < end) {} };
        const ROWS = [0, 1, 2, 3, 4, 5];
        export default function App() {
            const [clicks, setClicks] = useState(ROWS.map(() => 0));
            const [enters, setEnters] = useState(ROWS.map(() => 0));
            const [downs, setDowns] = useState(0);
            const [dbl, setDbl] = useState(0);
            useEffect(() => { const timer = setInterval(() => busy(8), 10); return () => clearInterval(timer); }, []);
            const bump = (list: number[], at: number) => list.map((n, i) => (i === at ? n + 1 : n));
            return (
                <PageWindow>
                    <div style={{ margin: 0 }}>
                        {downs % 2 === 1 ? <div style={{ height: 0, overflow: 'hidden' }}>pressed</div> : null}
                        {ROWS.map((i) => (
                            <button key={i} style={{ display: 'block', width: 300, height: 40 }}
                                onMouseDown={() => setDowns((n) => n + 1)}
                                onClick={() => setClicks((list) => bump(list, i))}
                                onMouseEnter={() => setEnters((list) => bump(list, i))}
                                onDoubleClick={() => setDbl((n) => n + 1)}>row {i}</button>
                        ))}
                    </div>
                    <p>clicks {clicks.join(',')} enters {enters.join(',')} dbl {dbl}</p>
                </PageWindow>
            );
        }
        "#,
    );
    works(
        &file,
        "wait 300; burst move 100 28 down 100 28 up 100 28; wait 100; expect \"clicks 1,0,0,0,0,0 enters 1,0,0,0,0,0\"; \
         burst move 100 68 down 100 68 up 100 68; wait 100; expect \"clicks 1,1,0,0,0,0 enters 1,1,0,0,0,0\"; \
         burst down 100 108 up 100 108; wait 100; expect \"clicks 1,1,1,0,0,0 enters 1,1,1,0,0,0\"; \
         burst move 100 148 move 100 188 move 100 228 move 100 188; wait 100; expect \"enters 1,1,1,1,2,1\"; \
         burst move 100 28 down 100 28 up 100 28 dblclick 100 28; wait 100; expect \"clicks 3,1,1,0,0,0 enters 2,1,1,1,2,1 dbl 1\"; \
         burst move 100 68 down 100 68 move 100 70 up 100 70 move 100 108 down 100 108 up 100 108 move 100 148 down 100 148 up 100 148; \
         wait 100; expect \"clicks 3,2,2,1,0,0 enters 2,2,2,2,2,1\"",
    );
}

#[test]
fn what_uses_a_variable_or_inherit_hears_it_change_through_what_is_not_drawn_again() {
    // A variable set two elements up, and `inherit` from the element around, changed by a click: what's in between uses neither,
    // so it isn't drawn again, and what uses them still has to be (`toldChanged` in layer.js).
    let file = app(
        "variables",
        r#"
        import { PageWindow } from 'rsui/web';
        import { useState } from 'react';
        export default function App() {
            const [wide, setWide] = useState(false);
            return (
                <PageWindow>
                    <style>{'.kid { width: inherit; height: 5px; } .deep { width: var(--w); height: 5px; }'}</style>
                    <div style={{ ['--w' as any]: wide ? '80px' : '40px' }}>
                        <section><div><div className="deep" /></div></section>
                    </div>
                    <div style={{ width: wide ? 120 : 60 }}><div className="kid" /></div>
                    <button id="toggle" onClick={() => setWide((was) => !was)}>toggle</button>
                </PageWindow>
            );
        }
        "#,
    );
    let widths = "eval \"document.querySelector('.deep').getBoundingClientRect().width + ' ' + document.querySelector('.kid').getBoundingClientRect().width\"";
    let said = works(&file, &format!("wait 200; {widths}; click #toggle; wait 200; {widths}; click #toggle; wait 200; {widths}"));
    let seen: Vec<&str> = said.lines().filter_map(|line| line.trim().strip_prefix("= ")).collect();
    assert_eq!(seen, ["40 60", "80 120", "40 60"], "{said}");
}

#[test]
fn the_page_element_moves_up_as_the_page_scrolls() {
    // As on a page: floating-ui places a dropdown that's `position: absolute` from where `<html>` is and how far the page is
    // scrolled, which with `<html>` always at the top put Mantine's dropdowns as far from their buttons as the page was scrolled.
    let file = app(
        "scrolled",
        r#"
        import { PageWindow } from 'rsui/web';
        export default function App() {
            return (
                <PageWindow>
                    <div style={{ height: 3000 }}>tall</div>
                </PageWindow>
            );
        }
        "#,
    );
    let said = works(
        &file,
        "wait 200; wheelat 100 100 3; wheelat 100 100 3; wait 200; eval \"(() => { const r = document.documentElement.getBoundingClientRect(); return [window.scrollY > 100, r.top === -window.scrollY, r.left, r.width > 0, r.height >= 3000].join(' '); })()\"",
    );
    assert!(said.contains("= true true 0 true true"), "{said}");
}
