//! `<webview>`: web pages from elsewhere in the system's browser view. These run with `--do`, so no
//! browser is started: a load comes back as one that worked, and the `webview` steps say what the page did, which is what
//! these check the app hears.

use std::path::PathBuf;
use std::process::Command;

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn app(name: &str, source: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("webview-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    file
}

/// Runs `steps` and gives what was said, failing if a step did.
fn works(file: &PathBuf, steps: &str) -> String {
    let data = file.parent().unwrap().join("data");
    let output = Command::new(TOOL)
        .arg("--do")
        .arg(steps)
        .arg(file)
        // Whatever it would keep goes here, never in the person's own folders.
        .env("APPDATA", &data)
        .env("LOCALAPPDATA", &data)
        .env("RSUI_SANDBOX", &data)
        .env("RSUI_OFFLINE", "1")
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    assert!(output.status.success(), "{steps}\n{said}\n{}", String::from_utf8_lossy(&output.stderr));
    said
}

/// Says each of `wanted` is in `said`, in that order.
fn in_order(said: &str, wanted: &[&str]) {
    let mut from = 0;
    for one in wanted {
        match said[from..].find(one) {
            Some(at) => from += at + one.len(),
            None => panic!("{one:?} isn't after what came before it in:\n{said}"),
        }
    }
}

const BROWSER: &str = r#"
    import { useRef } from 'react';
    import { PageWindow } from 'rsui/web';
    import type { WebViewHandle } from 'rsui/webview';

    export default function App() {
        const view = useRef<WebViewHandle | null>(null);
        return (
            <PageWindow>
                <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
                    <button onClick={() => view.current?.back()}>Back</button>
                    <button onClick={() => view.current?.go('https://example.org/')}>Go</button>
                    <webview
                        ref={view}
                        src="https://example.com/start"
                        profile="work"
                        keys={['Ctrl+T', 'Alt+ArrowLeft']}
                        style={{ flex: 1 }}
                        onState={(s) => console.log(`state ${s.url} title=${s.title} loading=${s.loading} back=${s.canGoBack} forward=${s.canGoForward} error=${s.error?.description ?? ''}`)}
                        onNewWindow={(r) => console.log('new window ' + r.url)}
                        onKey={(k) => console.log('key ' + k.name)}
                        onClose={() => console.log('asked to close')}
                        onUnavailable={(why) => console.log('unavailable: ' + why)}
                    >
                        <p>Loading</p>
                    </webview>
                </div>
            </PageWindow>
        );
    }
"#;

#[test]
fn a_view_goes_where_it_is_told_and_says_how_its_page_is() {
    let file = app("goes", BROWSER);
    let said = works(&file, "webview load https://example.com/next; click \"Back\"; click \"Go\"");
    in_order(
        &said,
        &[
            "it would show a web page in WebView2, in the profile work",
            "it would load https://example.com/start",
            "state https://example.com/start title=example.com loading=false back=false",
            "state https://example.com/next title=example.com loading=false back=true",
            "it would go back to https://example.com/start",
            "state https://example.com/start title=example.com loading=false back=false forward=true",
            "it would load https://example.org/",
            "state https://example.org/ title=example.org loading=false back=true forward=false",
        ],
    );
    // Its element is where the browser's window will be kept, by its number.
    assert!(works(&file, "look").contains("embed=1"));
}

#[test]
fn what_the_page_does_reaches_the_app() {
    let file = app("page", BROWSER);
    let said = works(
        &file,
        "webview title Start here; webview popup https://example.com/elsewhere; webview key Ctrl+T; \
         webview fail https://nope.invalid/ -105; webview close",
    );
    in_order(
        &said,
        &[
            "title=Start here",
            "new window https://example.com/elsewhere",
            "key Ctrl+T",
            "state https://nope.invalid/ title= loading=false back=false forward=false error=ERR_NAME_NOT_RESOLVED",
            "asked to close",
        ],
    );
}

#[test]
fn a_view_updates_its_kept_keys_without_loading_the_page_again() {
    let file = app(
        "remap",
        r#"
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';

        export default function App() {
            const [key, setKey] = useState('Ctrl+T');
            return (
                <PageWindow>
                    <button onClick={() => setKey('Ctrl+K')}>Remap</button>
                    <webview src="https://example.com/start" keys={[key]} onKey={(k) => console.log('kept ' + k.name)} />
                </PageWindow>
            );
        }
        "#,
    );
    let said = works(&file, "webview key ctrl+t; click Remap; webview key ctrl+t; webview key ctrl+k");
    assert_eq!(said.matches("kept Ctrl+T").count(), 1, "{said}");
    assert_eq!(said.matches("kept Ctrl+K").count(), 1, "{said}");
    assert_eq!(said.matches("it would load https://example.com/start").count(), 1, "{said}");
}

#[test]
fn a_computer_without_webview2_is_told_what_to_do() {
    let file = app("missing", BROWSER);
    let said = works(&file, "webview unavailable");
    let line = said.lines().find(|line| line.starts_with("unavailable: ")).unwrap_or_else(|| panic!("{said}"));
    assert!(line.contains("WebView2"), "{line}");
}

#[test]
fn a_view_is_one_of_rsuis_own_elements_outside_a_page_too() {
    let file = app(
        "native",
        r#"
        export default function App() {
            return (
                <window title="Native">
                    <column grow={1}>
                        <webview src="https://example.com/" grow={1} onState={(s) => console.log('native ' + s.url + ' ' + s.title)} />
                    </column>
                </window>
            );
        }
        "#,
    );
    let said = works(&file, "look");
    in_order(&said, &["it would load https://example.com/", "native https://example.com/ example.com", "embed=1"]);
}
