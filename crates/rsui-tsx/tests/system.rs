//! What a program asks of the system, with what would really happen left to the window.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn open(name: &str, source: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("system-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    Hands::open(&file, (400, 300))
}

#[test]
fn the_color_scheme_follows_the_system_and_takes_the_title_bar_with_it() {
    let mut hands = open(
        "scheme",
        r#"
        import { useColorScheme } from 'rsui';
        export default function App() {
            const scheme = useColorScheme();
            return <window><text>{'It is ' + scheme}</text></window>;
        }
        "#,
    );
    hands.settle();
    assert!(hands.says("It is "), "{}", hands.app.outline());
    assert!(hands.app.take_dark_title_bar_wanted().is_some());
    hands.app.set_dark_mode(true);
    hands.settle();
    assert!(hands.says("It is dark"), "{}", hands.app.outline());
    assert_eq!(hands.app.take_dark_title_bar_wanted(), Some(true));
    hands.app.set_dark_mode(false);
    hands.settle();
    assert!(hands.says("It is light"), "{}", hands.app.outline());
    assert_eq!(hands.app.take_dark_title_bar_wanted(), Some(false));
    // Saying it again changes nothing, which matters as the system says this a lot.
    hands.app.set_dark_mode(false);
    hands.settle();
    assert_eq!(hands.app.take_dark_title_bar_wanted(), None);
}

#[test]
fn a_program_that_does_not_ask_about_colors_keeps_its_title_bar() {
    let mut hands = open(
        "no-scheme",
        "export default function App() { return <window><text>Hello</text></window>; }",
    );
    hands.settle();
    hands.app.set_dark_mode(true);
    hands.settle();
    assert_eq!(hands.app.take_dark_title_bar_wanted(), None);
}

#[test]
fn links_can_be_opened_and_anything_else_is_an_error_that_says_why() {
    let mut hands = open(
        "urls",
        r#"
        import { shell } from 'rsui/desktop';
        export default function App() {
            return (
                <window>
                    <button key="good" onClick={() => shell.open('https://example.com/a?b=c')}>Good</button>
                    <button key="bad" onClick={() => shell.open('file:///C:/Windows/System32/calc.exe')}>Bad</button>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    hands.click("good");
    let wanted = hands.app.take_urls_wanted();
    assert_eq!(wanted.len(), 1);
    assert_eq!(wanted[0].as_str(), "https://example.com/a?b=c");
    hands.click("bad");
    assert!(hands.app.take_urls_wanted().is_empty());
    assert!(hands.says("calc.exe"), "{}", hands.app.outline());
    assert!(hands.says("Only http"), "{}", hands.app.outline());
}

#[test]
fn dropped_files_go_to_the_nearest_thing_that_takes_them_with_their_spaces() {
    let mut hands = open(
        "drop",
        r#"
        import { useState } from 'rsui';
        export default function App() {
            const [got, setGot] = useState('nothing');
            return (
                <window onDropFiles={(paths) => setGot('window ' + paths.length + ' ' + paths[1])}>
                    <column key="inner" height={100} onDropFiles={(paths) => setGot('inner ' + paths[0])}>
                        <text>{got}</text>
                    </column>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    assert!(hands.app.accepts_files());
    let paths = [
        "C:\\a folder\\one file.txt".to_owned(),
        "C:\\two.md".to_owned(),
    ];
    hands.app.files_dropped(10.0, 10.0, &paths).unwrap();
    hands.settle();
    assert!(
        hands.says("inner C:\\\\a folder\\\\one file.txt"),
        "{}",
        hands.app.outline()
    );
    hands.app.files_dropped(10.0, 250.0, &paths).unwrap();
    hands.settle();
    assert!(
        hands.says("window 2 C:\\\\two.md"),
        "{}",
        hands.app.outline()
    );
}

#[test]
fn a_window_only_says_it_takes_files_if_it_does() {
    let mut hands = open(
        "no-drop",
        "export default function App() { return <window><text>Hello</text></window>; }",
    );
    hands.settle();
    assert!(!hands.app.accepts_files());
    // Nothing to give them to is nothing to go wrong.
    hands
        .app
        .files_dropped(10.0, 10.0, &["C:\\one.txt".to_owned()])
        .unwrap();
}

const WIDGETS: &str = r#"
    import { ProgressBar, RadioGroup, Spinner, Switch, Tabs, useState } from 'rsui';
    export const minSize = [320, 200];
    export default function App() {
        const [on, setOn] = useState(false);
        const [choice, setChoice] = useState(0);
        const [tab, setTab] = useState(0);
        return (
            <window>
                <Switch id="power" label="Power" checked={on} onChange={setOn} />
                <Switch id="stuck" label="Stuck" checked={false} disabled onChange={setOn} />
                <RadioGroup id="size" name="Size" options={['Small', 'Medium', 'Large']} value={choice} onChange={setChoice} />
                <Tabs id="pages" options={['One', 'Two']} value={tab} onChange={setTab} />
                <ProgressBar id="done" value={3} max={4} width={100} name="Done" />
                <Spinner />
                <text>{(on ? 'on' : 'off') + ' choice ' + choice + ' tab ' + tab}</text>
            </window>
        );
    }
"#;

#[test]
fn the_ready_made_widgets_do_what_they_say() {
    let mut hands = open("widgets", WIDGETS);
    hands.settle();
    assert!(hands.says("off choice 0 tab 0"), "{}", hands.app.outline());
    hands.click("power");
    assert!(hands.says("on choice 0 tab 0"), "{}", hands.app.outline());
    hands.click("stuck");
    assert!(
        hands.says("on choice 0 tab 0"),
        "a disabled one does nothing\n{}",
        hands.app.outline()
    );
    hands.click("size-2");
    assert!(hands.says("on choice 2 tab 0"), "{}", hands.app.outline());
    // The arrow keys go between them, and round again.
    hands.press(hands::KeyCode::Down);
    assert!(hands.says("on choice 0 tab 0"), "{}", hands.app.outline());
    hands.press(hands::KeyCode::Up);
    assert!(hands.says("on choice 2 tab 0"), "{}", hands.app.outline());
    hands.click("pages-1");
    assert!(hands.says("on choice 2 tab 1"), "{}", hands.app.outline());
    hands.press(hands::KeyCode::Left);
    assert!(hands.says("on choice 2 tab 0"), "{}", hands.app.outline());
    assert!(hands.says(r#"value="75%""#), "{}", hands.app.outline());
    assert!(hands.says(r#"image "Busy""#), "{}", hands.app.outline());
}

#[test]
fn a_password_field_from_tsx_is_one() {
    let mut hands = open(
        "password",
        r#"
        import { TextInput, useState } from 'rsui';
        export default function App() {
            const [secret, setSecret] = useState('');
            const [short, setShort] = useState('');
            return (
                <window>
                    <TextInput key="secret" password value={secret} onChange={setSecret} />
                    <TextInput key="short" maxLength={2} value={short} onChange={setShort} />
                    <TextInput key="fixed" readOnly value="fixed" />
                </window>
            );
        }
        "#,
    );
    hands.settle();
    hands.click("secret");
    hands.type_text("abc");
    assert!(hands.says(r#"value="abc""#), "{}", hands.app.outline());
    hands.click("short");
    hands.type_text("abc");
    assert!(hands.says(r#"value="ab""#), "{}", hands.app.outline());
    hands.click("fixed");
    hands.type_text("x");
    assert!(hands.says(r#"value="fixed""#), "{}", hands.app.outline());
}
