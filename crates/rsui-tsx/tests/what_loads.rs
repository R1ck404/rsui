//! What an app pays for: a native app loads nothing of the web layer, a page's tags and CSS load `rsui/web` but not the DOM, the
//! page's globals (`rsui/dom-globals`) are only loaded once something touches one, and the nodes code makes (`rsui/dom`) only once
//! code makes one. Each app says what's been started, from `loadedBuiltins`.

use std::path::PathBuf;
use std::process::Command;

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn loaded(name: &str, source: &str) -> String {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-loads-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    let output = Command::new(TOOL)
        .arg("--do")
        .arg("wait \"loaded:\"")
        .arg(&file)
        .env("RSUI_SANDBOX", folder.join("sandbox"))
        .env("APPDATA", &folder)
        .env("LOCALAPPDATA", &folder)
        .env("USERPROFILE", &folder)
        .env("HOME", &folder)
        .env("RSUI_OFFLINE", "1")
        .output()
        .unwrap();
    let said = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "{said}");
    let start = said.rfind("loaded:").unwrap_or_else(|| panic!("{said}"));
    let line = &said[start..];
    line[..line.find('"').unwrap_or(line.len())].to_owned()
}

const REPORT: &str = r#"
    import * as rsui from 'rsui';
    export function Report() {
        const [said, set] = rsui.useState('');
        rsui.useEffect(() => set('loaded: ' + rsui.__host().loadedBuiltins().join(' ') + ' .'), []);
        return <text>{said}</text>;
    }
"#;

#[test]
fn a_native_app_loads_nothing_of_the_web() {
    let said = loaded(
        "native",
        &format!("{REPORT}\nimport {{ clipboard }} from 'rsui/desktop';\nexport default function App() {{ return <window><Report /><text>{{typeof clipboard.read}}</text></window>; }}"),
    );
    for web in [" rsui/web ", " rsui/dom-globals ", " rsui/dom ", " rsui/dom-events ", " rsui/web-routes "] {
        assert!(!said.contains(web), "{web} was loaded: {said}");
    }
    assert!(said.contains(" rsui/desktop/basics "), "{said}");
    assert!(!said.contains(" rsui/desktop/storage "), "only what's used is loaded: {said}");
    for unused in [" rsui/desktop/pictures ", " rsui/desktop/media "] {
        assert!(!said.contains(unused), "only what's used is loaded: {said}");
    }
}

#[test]
fn each_part_of_the_desktop_loads_only_when_it_s_used() {
    let said = loaded(
        "pictures",
        &format!("{REPORT}
import {{ files }} from 'rsui/desktop';
export default function App() {{ return <window><Report /><text>{{typeof files.icon}}</text></window>; }}"),
    );
    assert!(said.contains(" rsui/desktop/pictures "), "{said}");
    assert!(!said.contains(" rsui/desktop/media "), "{said}");
    let said = loaded(
        "media",
        &format!("{REPORT}
import {{ media }} from 'rsui/desktop';
export default function App() {{ return <window><Report /><text>{{typeof media.onKey}}</text></window>; }}"),
    );
    assert!(said.contains(" rsui/desktop/media "), "{said}");
    assert!(!said.contains(" rsui/desktop/pictures "), "{said}");
    assert!(!said.contains(" rsui/web-routes "), "{said}");
}

#[test]
fn a_page_that_never_touches_the_dom_does_not_load_it() {
    let said = loaded(
        "page",
        &format!("{REPORT}\nimport {{ PageWindow }} from 'rsui/web';\nexport default function App() {{ return <PageWindow><div className=\"a\"><Report /></div></PageWindow>; }}"),
    );
    assert!(said.contains(" rsui/web "), "{said}");
    assert!(!said.contains(" rsui/dom-globals "), "{said}");
    assert!(!said.contains(" rsui/dom "), "{said}");
}

#[test]
fn touching_document_loads_the_page_globals_and_the_event_classes_are_there() {
    let said = loaded(
        "dom",
        &format!("{REPORT}\nconst kinds = typeof document.querySelector + ' ' + (new MouseEvent('click', {{ clientX: 3 }}).clientX) + ' ' + (new KeyboardEvent('keydown', {{ key: 'a' }}) instanceof Event);\nimport {{ PageWindow }} from 'rsui/web';\nexport default function App() {{ return <PageWindow><div><Report /><p>{{kinds}}</p></div></PageWindow>; }}"),
    );
    assert!(said.contains(" rsui/dom-globals "), "{said}");
    assert!(said.contains(" rsui/dom-events "), "{said}");
    assert!(!said.contains(" rsui/dom "), "no node's been made: {said}");
}

#[test]
fn making_a_node_loads_the_dom() {
    let said = loaded(
        "nodes",
        &format!("{REPORT}
const made = document.createElement('div').nodeType + ' ' + document.createTextNode('a').nodeType;
import {{ PageWindow }} from 'rsui/web';
export default function App() {{ return <PageWindow><div><Report /><p>{{made}}</p></div></PageWindow>; }}"),
    );
    assert!(said.contains(" rsui/dom "), "{said}");
}
