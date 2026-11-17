//! What's drawn again when a page's CSS changes is only what that's about, as a browser restyles: a sheet that's added
//! (as a scroll lock adds one when a dialog opens), an attribute or class on `body` that rules about what's inside it ask about,
//! and a variable on the page that rules use. Each still comes out as drawing everything again would, and only a few of the
//! page's elements are told.

use std::path::PathBuf;
use std::process::Command;

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn app(name: &str, source: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-restyle-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    file
}

/// Does the steps with `RSUI_RESTYLE_LOG` and `RSUI_RESTYLE_CHECK`, and says what was printed, failing if a step did or the app said something was wrong.
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
        .env("RSUI_RESTYLE_LOG", "1")
        .env("RSUI_RESTYLE_CHECK", "1")
        .output()
        .unwrap();
    let said = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "{said}");
    assert!(!said.contains("TypeError"), "{said}");
    said
}

const PAGE: &str = r#"
    import { PageWindow } from 'rsui/web';
    import { useState } from 'react';
    export default function App() {
        const [count] = useState(40);
        const [on, setOn] = useState(false);
        (globalThis as any).addSheet = () => setOn(true);
        return (
            <PageWindow>
                <div>
                    {on ? <style>{'.target { width: 120px } body[data-locked] .deep { width: 77px } .dark .kid { width: 66px } .uses-var { width: var(--w, 10px) }'}</style> : null}
                    {Array.from({ length: count }, (_, i) => <div key={i} className="plain">plain {i}</div>)}
                    <div id="t" className="target">t</div>
                    <div className="outer"><div id="d" className="deep">d</div></div>
                    <div className="outer"><span><div id="k" className="kid">k</div></span></div>
                    <div id="v" className="uses-var">v</div>
                </div>
            </PageWindow>
        );
    }
"#;

#[test]
fn resize_restyles_viewport_consumers_and_crossed_media_conditions() {
    let file = app("resize-dependencies", r#"
        import { PageWindow } from 'rsui/web';
        export default function App() {
            return <PageWindow>
                <style>{`.responsive { width: 33px; height: 20px; }
                    @media (min-width: 700px) { .responsive { width: 77px; } }
                    :root { --view-width: 20vw; }
                    .viewport { width: 10vw; height: 20px; }
                    .variable { width: var(--view-width); height: 20px; }`}</style>
                {Array.from({length: 40}, (_, i) => <div className="plain" key={i}>plain {i}</div>)}
                <div id="media" className="responsive">Media</div>
                <div id="viewport" className="viewport">Viewport</div>
                <div id="variable" className="variable">Variable</div>
                <div id="inline" style={{width: '15vw', height: 20}}>Inline</div>
                <div style={{'--local-width': '25vw'}}>
                    <div id="local" style={{width: 'var(--local-width)', height: 20}}>Local variable</div>
                </div>
            </PageWindow>;
        }
    "#);
    let probe = "eval \"JSON.stringify(['media','viewport','variable','inline','local'].map(id => Math.round(document.getElementById(id).getBoundingClientRect().width)))\"";
    let said = works(&file, &format!(
        "size 1000 800; wait 100; {probe}; eval \"console.error('MARK same-media')\"; size 1010 800; wait 100; {probe}; eval \"console.error('MARK crossed-media')\"; size 600 800; wait 100; {probe}"
    ));
    assert!(said.contains("[77,100,200,150,250]"), "{said}");
    assert!(said.contains("[77,101,202,152,253]"), "{said}");
    assert!(said.contains("[33,60,120,90,150]"), "{said}");
    let same = said.split("MARK same-media").last().unwrap().split("MARK crossed-media").next().unwrap();
    let counts = told_after(&format!("MARK same-media\n{same}"), "MARK same-media");
    assert!(!counts.is_empty(), "viewport consumers must be told: {said}");
    assert!(counts.iter().any(|(told, all)| told < all), "unchanged media must not redraw the entire page: {said}");
}

/// How many of the page's elements were told the last time something changed, as `RSUI_RESTYLE_LOG` says, after `mark`.
fn told_after(said: &str, mark: &str) -> Vec<(usize, usize)> {
    let from = said.find(mark).unwrap_or_else(|| panic!("no {mark} in {said}"));
    said[from..]
        .lines()
        .filter_map(|line| line.strip_prefix("rsui restyle: told "))
        .map(|rest| {
            let mut parts = rest.split(" of ");
            (parts.next().unwrap().trim().parse().unwrap(), parts.next().unwrap().trim().parse().unwrap())
        })
        .collect()
}

#[test]
fn a_sheet_an_attribute_on_body_and_a_page_variable_restyle_only_what_they_are_about() {
    let file = app("selective", PAGE);
    let width = |id: &str| format!("eval \"'{id}=' + document.getElementById('{id}').getBoundingClientRect().width\"");
    let steps = [
        "wait 300".to_owned(),
        "eval \"console.error('MARK sheet'); addSheet(); 'added'\"".to_owned(),
        "wait 200".to_owned(),
        width("t"),
        width("d"),
        width("v"),
        "eval \"console.error('MARK attr'); document.body.setAttribute('data-locked', '1'); 'set'\"".to_owned(),
        "wait 200".to_owned(),
        width("d"),
        "eval \"console.error('MARK class'); document.body.classList.add('dark'); 'dark'\"".to_owned(),
        "wait 200".to_owned(),
        width("k"),
        "eval \"console.error('MARK var'); document.documentElement.style.setProperty('--w', '55px'); 'var'\"".to_owned(),
        "wait 200".to_owned(),
        width("v"),
        "eval \"console.error('MARK unset'); document.body.removeAttribute('data-locked'); document.body.classList.remove('dark'); 'unset'\"".to_owned(),
        "wait 200".to_owned(),
        width("d"),
        width("k"),
    ]
    .join("; ");
    let said = works(&file, &steps);
    // What each comes to is what a browser has.
    let order: Vec<&str> = said.lines().filter_map(|line| line.trim().strip_prefix("= ")).filter(|line| line.contains('=')).collect();
    let page_width = order[1].trim_start_matches("d=").parse::<f64>().unwrap();
    assert!(page_width > 200.0, "{said}");
    assert_eq!(order[0], "t=120", "{said}");
    assert_eq!(order[2], "v=10", "{said}");
    assert_eq!(order[3], "d=77", "{said}");
    assert_eq!(order[4], "k=66", "{said}");
    assert_eq!(order[5], "v=55", "{said}");
    assert_eq!(order[6], format!("d={page_width}"), "{said}");
    assert_ne!(order[7], "k=66", "{said}");
    // Drawing everything again after each comes to the same.
    assert!(said.contains("rsui restyle check: drawing"), "{said}");
    assert!(!said.contains("MISSED"), "{said}");
    // And only what they're about was told, and what that's in, of the forty-odd elements on the page.
    for mark in ["MARK sheet", "MARK attr", "MARK class", "MARK var", "MARK unset"] {
        let told = told_after(&said, mark);
        assert!(!told.is_empty(), "nothing told after {mark}: {said}");
        let (count, of) = told[0];
        assert!(of > 40 && count <= 8, "{mark}: told {count} of {of}: {said}");
    }
}
