//! What shadcn's calendar, date picker, combobox and OTP input showed rsui doing differently from a browser, each
//! used headless with `--do`, in a sandbox of its own.

use std::path::PathBuf;
use std::process::Command;

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn app(name: &str, source: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-shadcn-dates-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    // Each element of the page says its tag, as the parity harness's pages have it.
    std::fs::write(&file, format!("{source}\n(globalThis as any).__rsuiDomTags = true;\n")).unwrap();
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
fn react_leaves_out_data_attributes_given_nothing() {
    // react-day-picker gives every day `data-selected={selected || undefined}`: a day that isn't has no attribute at all.
    let file = app(
        "data-undefined",
        r#"
        import { PageWindow } from 'rsui/web';
        export default function Main() {
            return <PageWindow><div id="d" data-selected={undefined} data-today={null} data-on={true} data-off={false} data-day="12">x</div></PageWindow>;
        }
        "#,
    );
    let said = works(
        &file,
        r#"eval "const d = document.getElementById('d'); [d.getAttribute('data-selected'), d.getAttribute('data-today'), d.getAttribute('data-on'), d.getAttribute('data-off'), d.getAttribute('data-day'), JSON.stringify(Object.keys(d.dataset).sort())].map(String).join('|')""#,
    );
    assert!(said.contains(r#"= null|null|true|false|12|["day","off","on"]"#), "{said}");
}

#[test]
fn a_select_has_no_value_attribute_from_react() {
    let file = app(
        "select-value",
        r#"
        import { PageWindow } from 'rsui/web';
        export default function Main() {
            return <PageWindow><main><select id="s" value="2025" onChange={() => {}}><option value="2024">2024</option><option value="2025">2025</option></select></main></PageWindow>;
        }
        "#,
    );
    let said = works(
        &file,
        r#"eval "[document.getElementById('s').getAttribute('value'), document.getElementById('s').value].map(String).join('|')""#,
    );
    assert!(said.contains("= null|2025"), "{said}");
}

#[test]
fn visibility_hidden_is_what_the_computed_style_says_and_is_handed_down() {
    // react-day-picker's hidden days have `invisible` (`visibility: hidden`), which is how they're told apart.
    let file = app(
        "visibility",
        r#"
        import { PageWindow, addCss } from 'rsui/web';
        addCss('.invisible { visibility: hidden; } .shown { visibility: visible; }');
        export default function Main() {
            return <PageWindow><div className="invisible" id="a"><span id="b">in</span><span className="shown" id="c">back</span></div><div id="d">x</div></PageWindow>;
        }
        "#,
    );
    let said = works(&file, r#"eval "['a', 'b', 'c', 'd'].map((id) => getComputedStyle(document.getElementById(id)).visibility).join('|')""#);
    assert!(said.contains("= hidden|hidden|visible|visible"), "{said}");
}

#[test]
fn growing_buttons_in_a_wrapping_row_go_on_as_many_lines_as_their_words_need() {
    // The calendar's presets: five `flex: 1` buttons in a 300px row that wraps. Each is never narrower than its words, as CSS's
    // `min-width: auto` has it, so they wrap rather than all squeezing onto one line.
    let file = app(
        "wrap-grow",
        r#"
        import { PageWindow } from 'rsui/web';
        const b = { flex: '1 1 0%', whiteSpace: 'nowrap', padding: '0 12px', fontSize: 14 } as const;
        export default function Main() {
            return <PageWindow><div id="row" style={{ display: 'flex', flexWrap: 'wrap', gap: 8, width: 300 }}>
                {['Today', 'Tomorrow', 'In 3 days', 'In a week', 'In 2 weeks'].map((label) => <button key={label} style={b}>{label}</button>)}
            </div></PageWindow>;
        }
        "#,
    );
    let said = works(
        &file,
        r#"size 600 400; eval "((all) => new Set(all.map((r) => Math.round(r.top))).size + ' lines, ' + all.every((r) => r.width >= 40) + ' ' + all.every((r) => r.right <= document.getElementById('row').getBoundingClientRect().right + 1))([...document.querySelectorAll('#row button')].map((one) => one.getBoundingClientRect()))""#,
    );
    let lines: usize = said.split("\n  = ").nth(1).and_then(|rest| rest.split(' ').next()).and_then(|n| n.parse().ok()).unwrap_or(0);
    assert!(lines >= 2, "{said}");
    assert!(said.contains("lines, true true"), "{said}");
}

#[test]
fn focus_given_by_code_from_outside_has_the_keyboard_before_the_next_key() {
    // A console's `button.focus()` (the parity harness's `focus` step): React hears `onFocus` and draws what that set, and the next
    // key goes to the button, rather than to the page.
    let file = app(
        "eval-focus",
        r#"
        import { PageWindow } from 'rsui/web';
        import { useState } from 'react';
        const log: string[] = ((globalThis as any).__log = []);
        function B() {
            const [on, setOn] = useState(false);
            return <button id="b" data-focused={on || undefined} onFocus={() => setOn(true)} onKeyDown={(e) => log.push(e.key)}>b</button>;
        }
        export default function Main() { return <PageWindow><main><B /></main></PageWindow>; }
        "#,
    );
    let said = works(
        &file,
        r#"eval "document.getElementById('b').focus()"; press right; eval "'keys ' + __log.join(',')"; eval "'focused ' + document.getElementById('b').getAttribute('data-focused')""#,
    );
    assert!(said.contains("= focused true"), "{said}");
    assert!(said.contains("= keys ArrowRight"), "{said}");
}

#[test]
fn a_text_field_takes_no_more_than_its_max_length() {
    // input-otp's one hidden input is `maxLength={6}`: typing past it adds nothing, as in a browser.
    let file = app(
        "max-length",
        r#"
        import { PageWindow } from 'rsui/web';
        export default function Main() { return <PageWindow><main><input id="i" maxLength={3} defaultValue="" /></main></PageWindow>; }
        "#,
    );
    let said = works(&file, r#"clickat css:#i; write "abcdef"; eval "document.getElementById('i').value""#);
    assert!(said.lines().any(|line| line.trim() == "= abc"), "{said}");
}
