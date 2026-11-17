//! More than one window, with no real ones: what would be asked of the system is done by hand here, so that
//! each window is an [`App`] that's used the way a person would.

mod hands;

use std::collections::HashMap;
use std::path::PathBuf;

use hands::{App, Hands};
use rsui::{WindowId, take_window_requests};

/// All the windows of a program.
struct Desk {
    main: Hands,
    others: HashMap<u32, Hands>,
    titles: HashMap<u32, String>,
    options: HashMap<u32, rsui::WindowOptions>,
    main_closed: bool,
}

impl Desk {
    fn open(name: &str, source: &str) -> Self {
        let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("windows-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("app.tsx");
        std::fs::write(&file, source).unwrap();
        // Anything left over belongs to a test that's finished.
        let _ = take_window_requests();
        let mut desk = Self {
            main: Hands::open(&file, (400, 300)),
            others: HashMap::new(),
            titles: HashMap::new(),
            options: HashMap::new(),
            main_closed: false,
        };
        desk.settle();
        desk
    }

    /// Does what's been asked of the windows, until nothing's going on anywhere.
    fn settle(&mut self) {
        for _ in 0..8 {
            self.main.settle();
            for other in self.others.values_mut() {
                other.settle();
            }
            let requests = take_window_requests();
            for wanted in requests.open {
                let text = rsui::default_text_engine().unwrap();
                let app: App = (wanted.make)(text, (300, 200), 1.0);
                self.titles
                    .insert(wanted.id.0, wanted.options.title.clone());
                self.options.insert(wanted.id.0, wanted.options);
                self.others.insert(wanted.id.0, Hands { app });
            }
            for (id, title) in requests.titles {
                self.titles.insert(id.0, title);
            }
            for id in requests.close {
                if id == WindowId::MAIN {
                    self.main_closed = true;
                } else if let Some(mut gone) = self.others.remove(&id.0) {
                    // As a window does before it goes.
                    gone.settle();
                }
            }
        }
    }

    fn only_other(&mut self) -> &mut Hands {
        assert_eq!(self.others.len(), 1, "there should be one other window");
        self.others.values_mut().next().unwrap()
    }

    fn click_in_main(&mut self, key: &str) {
        self.main.click(key);
        self.settle();
    }

    fn click_in_other(&mut self, key: &str) {
        self.only_other().click(key);
        self.settle();
    }
}

const SETTINGS: &str = r#"
    import { createContext, useContext, useEffect, useState, Window } from 'rsui';
    const Theme = createContext('no theme');
    function Inside({ count, onAdd }) {
        const theme = useContext(Theme);
        const [own, setOwn] = useState(0);
        useEffect(() => () => console.log('inside has gone'), []);
        return (
            <column>
                <text>{'count ' + count + ', theme ' + theme + ', own ' + own}</text>
                <button key="add-there" onClick={onAdd}>Add</button>
                <button key="own" onClick={() => setOwn(own + 1)}>Own</button>
            </column>
        );
    }
    export default function App() {
        const [open, setOpen] = useState(false);
        const [count, setCount] = useState(0);
        return (
            <Theme.Provider value="dark">
                <window>
                    <text>{'main ' + count + (open ? ' open' : ' shut')}</text>
                    <button key="toggle" onClick={() => setOpen(!open)}>Toggle</button>
                    <button key="add-here" onClick={() => setCount(count + 1)}>Add</button>
                    {open ? (
                        <Window title={'Settings ' + count} size={[320, 240]} minSize={[100, 100]} modal onClose={() => setOpen(false)}>
                            <Inside count={count} onAdd={() => setCount(count + 1)} />
                        </Window>
                    ) : null}
                </window>
            </Theme.Provider>
        );
    }
"#;

#[test]
fn a_window_opens_and_closes_with_what_is_in_the_tree_and_says_how_it_should_be() {
    let mut desk = Desk::open("open", SETTINGS);
    assert!(desk.others.is_empty(), "it's not there to begin with");
    desk.click_in_main("toggle");
    assert_eq!(desk.others.len(), 1, "{}", desk.main.app.outline());
    assert!(desk.main.says("main 0 open"), "{}", desk.main.app.outline());
    let options = desk.options.values().next().unwrap();
    assert_eq!(options.size, Some((320, 240)));
    assert_eq!(options.smallest, Some((100, 100)));
    assert!(options.modal);
    assert!(options.owned, "it does unless it says otherwise");
    assert_eq!(options.title, "Settings 0");
    desk.click_in_main("toggle");
    assert!(desk.others.is_empty(), "it goes when it's taken away");
}

#[test]
fn what_is_in_a_window_has_the_same_state_and_context_as_the_rest() {
    let mut desk = Desk::open("shared", SETTINGS);
    desk.click_in_main("toggle");
    let other = desk.only_other();
    assert!(
        other.says("count 0, theme dark, own 0"),
        "{}",
        other.app.outline()
    );
    // From the main window to the other one.
    desk.click_in_main("add-here");
    assert!(desk.main.says("main 1 open"), "{}", desk.main.app.outline());
    let other = desk.only_other();
    assert!(
        other.says("count 1, theme dark, own 0"),
        "{}",
        other.app.outline()
    );
    // From that one back.
    desk.click_in_other("add-there");
    assert!(desk.main.says("main 2 open"), "{}", desk.main.app.outline());
    let other = desk.only_other();
    assert!(
        other.says("count 2, theme dark, own 0"),
        "{}",
        other.app.outline()
    );
    // Its own state stays through all of that, and doesn't touch the main window.
    desk.click_in_other("own");
    let other = desk.only_other();
    assert!(
        other.says("count 2, theme dark, own 1"),
        "{}",
        other.app.outline()
    );
    desk.click_in_main("add-here");
    let other = desk.only_other();
    assert!(
        other.says("count 3, theme dark, own 1"),
        "{}",
        other.app.outline()
    );
}

#[test]
fn its_title_follows_what_it_is_told() {
    let mut desk = Desk::open("title", SETTINGS);
    desk.click_in_main("toggle");
    desk.click_in_main("add-here");
    assert_eq!(desk.titles.values().next().unwrap(), "Settings 1");
}

#[test]
fn clicking_the_close_button_asks_what_is_in_the_window() {
    let mut desk = Desk::open("asked", SETTINGS);
    desk.click_in_main("toggle");
    let other = desk.only_other();
    assert!(other.app.asked_to_close().unwrap(), "it has an answer");
    desk.settle();
    assert!(desk.others.is_empty(), "and that was to close it");
    assert!(desk.main.says("main 0 shut"), "{}", desk.main.app.outline());
    // Which can be opened again.
    desk.click_in_main("toggle");
    assert_eq!(desk.others.len(), 1);
    let other = desk.only_other();
    assert!(
        other.says("own 0"),
        "it starts again\n{}",
        other.app.outline()
    );
}

#[test]
fn with_nothing_to_ask_the_close_button_just_closes_it() {
    let mut desk = Desk::open(
        "unasked",
        r#"
        import { useState, Window } from 'rsui';
        export default function App() {
            const [renders, setRenders] = useState(0);
            return (
                <window>
                    <button key="again" onClick={() => setRenders(renders + 1)}>{'rendered ' + renders}</button>
                    <Window title="On its own"><text>{'Inside ' + renders}</text></Window>
                </window>
            );
        }
        "#,
    );
    assert_eq!(desk.others.len(), 1);
    assert!(desk.only_other().app.asked_to_close().unwrap());
    desk.settle();
    assert!(desk.others.is_empty());
    // The main window carrying on doesn't bring it back.
    desk.click_in_main("again");
    assert!(desk.others.is_empty());
    assert!(desk.main.says("rendered 1"), "{}", desk.main.app.outline());
}

#[test]
fn timers_and_effects_reach_what_is_in_another_window() {
    let mut desk = Desk::open(
        "timers",
        r#"
        import { useEffect, useState, Window } from 'rsui';
        function Ticking() {
            const [ticks, setTicks] = useState(0);
            useEffect(() => {
                const timer = setInterval(() => setTicks((now) => now + 1), 1);
                return () => clearInterval(timer);
            }, []);
            return <text>{ticks > 2 ? 'ticked' : 'waiting'}</text>;
        }
        export default function App() {
            return <window><text>Main</text><Window title="Clock"><Ticking /></Window></window>;
        }
        "#,
    );
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(10) {
        desk.settle();
        desk.main
            .app
            .runtime()
            .wait(std::time::Duration::from_millis(5));
        if desk.only_other().says("ticked") {
            return;
        }
    }
    panic!("it never ticked\n{}", desk.only_other().app.outline());
}

#[test]
fn the_main_window_can_be_asked_and_closed_from_code() {
    let mut desk = Desk::open(
        "main",
        r#"
        import { mainWindow, useState } from 'rsui';
        export default function App() {
            const [asked, setAsked] = useState(false);
            return (
                <window onClose={() => setAsked(true)}>
                    <text>{asked ? 'Are you sure?' : 'Hello'}</text>
                    <button key="really" onClick={() => mainWindow.close()}>Close</button>
                </window>
            );
        }
        "#,
    );
    assert!(desk.main.app.asked_to_close().unwrap());
    desk.settle();
    assert!(
        desk.main.says("Are you sure?"),
        "{}",
        desk.main.app.outline()
    );
    assert!(!desk.main_closed, "asking doesn't close it");
    desk.click_in_main("really");
    assert!(desk.main_closed);
}

#[test]
fn a_window_with_no_onclose_of_its_own_closes_when_asked() {
    let mut hands = Desk::open(
        "plain",
        "export default function App() { return <window><text>Plain</text></window>; }",
    );
    assert!(
        !hands.main.app.asked_to_close().unwrap(),
        "it has nothing to say about it"
    );
}

#[test]
fn several_windows_can_be_open_each_with_their_own_things() {
    let mut desk = Desk::open(
        "several",
        r#"
        import { useState, Window } from 'rsui';
        export default function App() {
            const [names, setNames] = useState(['One', 'Two', 'Three']);
            return (
                <window>
                    <button key="drop" onClick={() => setNames(names.slice(1))}>Drop</button>
                    {names.map((name) => (
                        <Window key={name} title={name}><text>{'Inside ' + name}</text></Window>
                    ))}
                </window>
            );
        }
        "#,
    );
    assert_eq!(desk.others.len(), 3);
    let shown: Vec<bool> = desk
        .others
        .values()
        .map(|other| other.says("Inside "))
        .collect();
    assert_eq!(shown, [true, true, true]);
    desk.click_in_main("drop");
    assert_eq!(desk.others.len(), 2);
    assert!(desk.others.values().all(|other| !other.says("Inside One")));
}

#[test]
fn an_error_in_another_window_shows_in_the_main_one() {
    let mut desk = Desk::open(
        "error",
        r#"
        import { Window } from 'rsui';
        export default function App() {
            return (
                <window>
                    <text>Main</text>
                    <Window title="Broken"><button key="bad" onClick={() => { throw new Error('it broke'); }}>Bad</button></Window>
                </window>
            );
        }
        "#,
    );
    desk.click_in_other("bad");
    assert!(desk.main.says("it broke"), "{}", desk.main.app.outline());
}

#[test]
fn the_example_works_as_it_says() {
    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/apps/windows.tsx"),
    )
    .unwrap();
    let mut desk = Desk::open("example", &source);
    assert!(desk.others.is_empty());
    desk.click_in_main("settings");
    assert_eq!(desk.others.len(), 1, "settings");
    assert!(
        desk.only_other().says(r#"text "Settings""#),
        "{}",
        desk.only_other().app.outline()
    );
    desk.click_in_main("note");
    desk.click_in_main("note");
    assert_eq!(desk.others.len(), 3, "and two notes");
    // Asking to close is a question in a window of its own, which isn't owned by being asked.
    assert!(desk.main.app.asked_to_close().unwrap());
    desk.settle();
    assert_eq!(desk.others.len(), 4);
    assert!(
        desk.options.values().any(|options| options.modal),
        "that one has to be answered"
    );
    let (id, _) = desk
        .others
        .iter()
        .find(|(_, other)| other.says("Close everything?"))
        .map(|(id, other)| (*id, other.app.outline()))
        .expect("it's asking");
    desk.others.get_mut(&id).unwrap().click("no");
    desk.settle();
    assert_eq!(desk.others.len(), 3, "it stays open");
    assert!(!desk.main_closed);
    assert!(desk.main.app.asked_to_close().unwrap());
    desk.settle();
    let (id, _) = desk
        .others
        .iter()
        .find(|(_, other)| other.says("Close everything?"))
        .map(|(id, other)| (*id, other.app.outline()))
        .expect("it's asking again");
    desk.others.get_mut(&id).unwrap().click("yes");
    desk.settle();
    assert!(desk.main_closed);
}

#[test]
fn a_window_can_start_with_no_frame_on_top_and_fullscreen() {
    let desk = Desk::open(
        "kinds",
        r#"
        import { Window } from 'rsui';
        export default function App() {
            return (
                <window>
                    <Window title="Kinds" frameless alwaysOnTop fullscreen><text>Inside</text></Window>
                    <Window title="Plain"><text>Inside</text></Window>
                </window>
            );
        }
        "#,
    );
    let options: Vec<_> = desk.options.values().collect();
    assert_eq!(options.len(), 2);
    let kinds = options.iter().find(|one| one.title == "Kinds").unwrap();
    assert!(kinds.frameless && kinds.on_top && kinds.fullscreen);
    let plain = options.iter().find(|one| one.title == "Plain").unwrap();
    assert!(!plain.frameless && !plain.on_top && !plain.fullscreen);
}

#[test]
fn what_is_asked_of_a_window_reaches_the_right_one() {
    let _ = take_window_requests();
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("windows-{}-controls", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(
        &file,
        r#"
        import { mainWindow, useCurrentWindow, Window } from 'rsui';
        function Inside() {
            const here = useCurrentWindow();
            return <button key="fit" onClick={() => { here.setAlwaysOnTop(); here.resize(320, 200); }}>Fit</button>;
        }
        function Main() {
            const here = useCurrentWindow();
            return <button key="small" onClick={() => { here.minimize(); mainWindow.setFullscreen(false); mainWindow.moveTo(5, 6); here.center(); }}>Small</button>;
        }
        export default function App() {
            return <window><Main /><Window title="Other"><Inside /></Window></window>;
        }
        "#,
    )
    .unwrap();
    let mut main = Hands::open(&file, (400, 300));
    main.settle();
    let requests = take_window_requests();
    let wanted = requests.open.into_iter().next().unwrap();
    let id = wanted.id;
    let text = rsui::default_text_engine().unwrap();
    let mut other = Hands {
        app: (wanted.make)(text, (300, 200), 1.0),
    };
    other.settle();
    main.click("small");
    main.settle();
    other.click("fit");
    other.settle();
    use rsui::WindowChange::*;
    assert_eq!(
        take_window_requests().changes,
        [
            (WindowId::MAIN, Minimize),
            (WindowId::MAIN, Fullscreen(false)),
            (WindowId::MAIN, MoveTo(5, 6)),
            (WindowId::MAIN, Center),
            (id, OnTop(true)),
            (id, Resize(320, 200)),
        ]
    );
}

#[test]
fn a_window_can_open_as_a_widget_a_bar_or_an_overlay() {
    let desk = Desk::open(
        "desktop-kinds",
        r#"
        import { Window } from 'rsui';
        export default function App() {
            return (
                <window>
                    <Window title="Widget" frameless transparent layer="desktop" skipTaskbar allDesktops><text>Clock</text></Window>
                    <Window title="Bar" alwaysOnTop appBar={{ edge: 'top', size: 32, monitor: 1 }}><text>Bar</text></Window>
                    <Window title="Beneath" layer="bottom" alwaysOnTop><text>Under</text></Window>
                    <Window title="Plain"><text>Inside</text></Window>
                </window>
            );
        }
        "#,
    );
    let find = |title: &str| desk.options.values().find(|one| one.title == title).unwrap().clone();
    let widget = find("Widget");
    assert!(widget.frameless && widget.transparent && widget.skip_taskbar && widget.all_desktops);
    assert_eq!(widget.effective_layer(), rsui::WindowLayer::Desktop);
    assert_eq!(widget.app_bar, None);
    let bar = find("Bar");
    assert_eq!(bar.effective_layer(), rsui::WindowLayer::Top, "alwaysOnTop is the top layer");
    assert_eq!(
        bar.app_bar,
        Some(rsui::AppBar { edge: rsui::ScreenEdge::Top, size: 32, monitor: Some(1) })
    );
    assert_eq!(find("Beneath").effective_layer(), rsui::WindowLayer::Bottom, "a layer that's said wins");
    let plain = find("Plain");
    assert_eq!(plain.effective_layer(), rsui::WindowLayer::Normal);
    assert!(!plain.transparent && !plain.skip_taskbar && !plain.all_desktops && plain.app_bar.is_none());
}

#[test]
fn what_a_window_is_follows_its_props_one_at_a_time() {
    let mut desk = Desk::open(
        "desktop-props",
        r#"
        import { useState, Window } from 'rsui';
        export default function App() {
            const [bar, setBar] = useState(true);
            const [frameless, setFrameless] = useState(false);
            const [layer, setLayer] = useState('desktop');
            return (
                <window>
                    <button key="bar" onClick={() => setBar(!bar)}>Bar</button>
                    <button key="frame" onClick={() => setFrameless(!frameless)}>Frame</button>
                    <button key="layer" onClick={() => setLayer(layer === 'desktop' ? 'bottom' : 'desktop')}>Layer</button>
                    <Window title="Changing" frameless={frameless} layer={layer} appBar={bar ? { edge: 'left', size: 50 } : null}>
                        <text>Inside</text>
                    </Window>
                </window>
            );
        }
        "#,
    );
    let id = *desk.options.keys().next().unwrap();
    let id = WindowId(id);
    let _ = take_window_requests();
    use rsui::WindowChange::*;
    let changes = |desk: &mut Desk, key: &str| {
        desk.main.click(key);
        desk.main.settle();
        take_window_requests().changes
    };
    assert_eq!(changes(&mut desk, "frame"), [(id, Frameless(true))], "the frame alone, leaving its layer as it is");
    assert_eq!(changes(&mut desk, "bar"), [(id, AppBar(None))]);
    assert_eq!(
        changes(&mut desk, "bar"),
        [(id, AppBar(Some(rsui::AppBar { edge: rsui::ScreenEdge::Left, size: 50, monitor: None })))]
    );
    assert_eq!(changes(&mut desk, "layer"), [(id, Layer(rsui::WindowLayer::Bottom))]);
}

#[test]
fn a_window_can_be_moved_between_layers_and_made_a_bar_from_code() {
    let _ = take_window_requests();
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("windows-{}-desktop-controls", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(
        &file,
        r#"
        import { mainWindow } from 'rsui';
        export default function App() {
            return (
                <window>
                    <button key="go" onClick={() => {
                        mainWindow.setLayer('desktop');
                        mainWindow.setTransparent();
                        mainWindow.setAllDesktops();
                        mainWindow.setSkipTaskbar();
                        mainWindow.setAppBar({ edge: 'right', size: 64.6, monitor: 0 });
                        mainWindow.setAppBar(null);
                        mainWindow.setLayer('top');
                        mainWindow.setAlwaysOnTop(false);
                        mainWindow.setTransparent(false);
                        mainWindow.setAllDesktops(false);
                        mainWindow.setLayer();
                    }}>Go</button>
                    <button key="wrong" onClick={() => {
                        for (const bad of [() => mainWindow.setLayer('sideways'), () => mainWindow.setAppBar({ edge: 'middle', size: 4 }), () => mainWindow.setAppBar({ edge: 'top', size: 0 })]) {
                            try { bad(); console.log('no error'); } catch (error) { console.log('error: ' + error.message); }
                        }
                    }}>Wrong</button>
                </window>
            );
        }
        "#,
    )
    .unwrap();
    let mut main = Hands::open(&file, (400, 300));
    main.settle();
    let _ = take_window_requests();
    main.click("go");
    main.settle();
    use rsui::WindowChange::*;
    use rsui::WindowLayer;
    assert_eq!(
        take_window_requests().changes,
        [
            (WindowId::MAIN, Layer(WindowLayer::Desktop)),
            (WindowId::MAIN, Transparent(true)),
            (WindowId::MAIN, AllDesktops(true)),
            (WindowId::MAIN, SkipTaskbar(true)),
            (
                WindowId::MAIN,
                AppBar(Some(rsui::AppBar { edge: rsui::ScreenEdge::Right, size: 65, monitor: Some(0) }))
            ),
            (WindowId::MAIN, AppBar(None)),
            (WindowId::MAIN, Layer(WindowLayer::Top)),
            (WindowId::MAIN, OnTop(false)),
            (WindowId::MAIN, Transparent(false)),
            (WindowId::MAIN, AllDesktops(false)),
            (WindowId::MAIN, Layer(WindowLayer::Normal)),
        ]
    );
    main.click("wrong");
    main.settle();
    assert!(take_window_requests().changes.is_empty(), "nothing wrong reaches the window");
}

#[test]
fn the_main_window_says_how_it_opens_with_what_it_exports() {
    let _ = take_window_requests();
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("windows-{}-desktop-exports", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(
        &file,
        r#"
        export const frameless = true;
        export const layer = 'desktop';
        export const transparent = true;
        export const skipTaskbar = true;
        export const allDesktops = true;
        export const appBar = { edge: 'bottom', size: 40 };
        export default function App() {
            return <window><text>Wallpaper</text></window>;
        }
        "#,
    )
    .unwrap();
    let main = Hands::open(&file, (400, 300));
    let options = rsui::main_window_options();
    assert!(options.frameless && options.transparent && options.skip_taskbar && options.all_desktops);
    assert_eq!(options.effective_layer(), rsui::WindowLayer::Desktop);
    assert_eq!(options.app_bar, Some(rsui::AppBar { edge: rsui::ScreenEdge::Bottom, size: 40, monitor: None }));
    drop(main);
    // Another app on the same thread starts from nothing.
    std::fs::write(&file, "export default function App() { return <window><text>Plain</text></window>; }").unwrap();
    let _plain = Hands::open(&file, (400, 300));
    let options = rsui::main_window_options();
    assert!(!options.transparent && options.app_bar.is_none());
    assert_eq!(options.effective_layer(), rsui::WindowLayer::Normal);
}
