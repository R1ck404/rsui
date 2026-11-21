//! `native` on rsui's own elements: drawn in the platform's look, Material 3 or Fluent, whatever the app says of how they look,
//! and working as the platform's do. See `rsui::native_look` and `src/native_look.rs`.

mod hands;

use std::path::PathBuf;
use std::process::Command;

use hands::{Hands, path_of};
use rsui::core::{Node, PropValue};
use rsui::native_look::{self, Theme};

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn folder(label: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("native-look-{}-{label}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The app, in this thread's look, light, which is chosen before it's loaded as the app's config would choose it.
fn open(label: &str, theme: &str, body: &str) -> Hands {
    let file = folder(label).join("app.tsx");
    std::fs::write(&file, body).unwrap();
    native_look::choose_theme(theme);
    native_look::set_dark(false);
    Hands::open(&file, (400, 300))
}

fn node<'a>(hands: &'a Hands, key: &str) -> &'a Node {
    let mut path = Vec::new();
    assert!(path_of(hands.app.runtime().root(), key, &mut path), "no {key} in\n{}", hands.app.outline());
    let mut node = hands.app.runtime().root();
    for at in path {
        node = &node.children[at as usize];
    }
    node
}

fn prop(hands: &Hands, key: &str, name: &str) -> Option<PropValue> {
    node(hands, key).props.get(name).cloned()
}

fn color(hex: u32) -> Option<PropValue> {
    Some(PropValue::Int(rsui::render::Color::hex(hex).to_prop()))
}

/// What's drawn, as red, green, blue and alpha for each pixel, 400 by 300.
fn pixels(hands: &mut Hands) -> Vec<u8> {
    hands.settle();
    let mut renderer = rsui::render::Renderer::offscreen((400, 300)).unwrap();
    renderer.render(hands.app.scene(), rsui::render::Color::WHITE).unwrap();
    renderer.read_pixels().unwrap()
}

fn pixel(pixels: &[u8], x: f32, y: f32) -> [u8; 3] {
    let at = (y as usize * 400 + x as usize) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

const BUTTON: &str = r#"
    import { useState } from 'rsui';
    export default function App() {
        const [count, setCount] = useState(0);
        return (
            <window native padding={16} gap={8} align="start">
                <button key="save" native onClick={() => setCount(count + 1)}>Save</button>
                <button key="send" native android={{ variant: 'outlined' }} windows={{ variant: 'accent' }}>Send</button>
                <text>{'pressed ' + count}</text>
            </window>
        );
    }
"#;

#[test]
fn a_native_button_is_materials_and_clicks() {
    let mut ui = open("button", "material", BUTTON);
    assert_eq!(native_look::theme(), Theme::Material);
    // Material 3's filled button: the primary color, round ends, 40 high, with a ripple and its state layers.
    assert_eq!(prop(&ui, "save", "background"), color(0x6750A4));
    assert_eq!(prop(&ui, "save", "text_color"), color(0xFFFFFF));
    assert_eq!(prop(&ui, "save", "corner_radius"), Some(PropValue::Float(20.0)));
    assert!(prop(&ui, "save", "ripple").is_some());
    assert_eq!(ui.rect_of("save")[3], 40.0);
    // `android` says this one's outlined, and `windows` is left alone.
    assert_eq!(prop(&ui, "send", "border_color"), color(0x79747E));
    assert_eq!(prop(&ui, "send", "background"), None);
    let middle = ui.middle_of("save");
    let shown = pixels(&mut ui);
    assert_eq!(pixel(&shown, middle.0 - 25.0, middle.1), [0x67, 0x50, 0xA4]);
    ui.click("save");
    ui.click("save");
    assert!(ui.says("pressed 2"), "{}", ui.app.outline());
}

#[test]
fn a_native_button_is_fluents_with_what_windows_says() {
    let ui = open("fluent-button", "fluent", BUTTON);
    assert_eq!(native_look::theme(), Theme::Fluent);
    // WinUI's standard button: corners of 4, 32 high, a border darker along the bottom.
    assert_eq!(prop(&ui, "save", "corner_radius"), Some(PropValue::Float(4.0)));
    assert_eq!(prop(&ui, "save", "border_width"), Some(PropValue::Float(1.0)));
    assert!(prop(&ui, "save", "border_bottom_color").is_some());
    assert!(prop(&ui, "save", "ripple").is_none());
    assert!(matches!(prop(&ui, "save", "focus_ring"), Some(PropValue::Str(_))));
    // `windows` says this one's the accent button, whose text is white on a light background.
    assert_eq!(prop(&ui, "send", "text_color"), color(0xFFFFFF));
    assert!(prop(&ui, "send", "pressed_text_color").is_some());
}

#[test]
fn what_says_how_a_native_control_looks_is_left_out_and_where_it_goes_is_kept() {
    let ui = open(
        "left-out",
        "material",
        r##"
        export default function App() {
            return (
                <window padding={10} align="start">
                    <button key="red" native background="#ff0000" color="#00ff00" radius={3} fontSize={30} paddingX={50}
                        width={150} marginTop={7} style={{ borderColor: '#0000ff', marginLeft: 5 }}>Red</button>
                    <button key="plain" background="#ff0000" radius={3}>Plain</button>
                </window>
            );
        }
        "##,
    );
    assert_eq!(prop(&ui, "red", "background"), color(0x6750A4));
    assert_eq!(prop(&ui, "red", "text_color"), color(0xFFFFFF));
    assert_eq!(prop(&ui, "red", "corner_radius"), Some(PropValue::Float(20.0)));
    assert_eq!(prop(&ui, "red", "font_size"), Some(PropValue::Float(14.0)));
    assert_eq!(prop(&ui, "red", "padding_left"), Some(PropValue::Float(24.0)));
    assert_eq!(prop(&ui, "red", "border_color"), None);
    assert_eq!(prop(&ui, "red", "width"), Some(PropValue::Float(150.0)));
    assert_eq!(prop(&ui, "red", "margin_top"), Some(PropValue::Float(7.0)));
    assert_eq!(prop(&ui, "red", "margin_left"), Some(PropValue::Float(5.0)));
    assert_eq!(prop(&ui, "red", "native"), None);
    // Without `native`, it's as it was.
    assert_eq!(prop(&ui, "plain", "background"), color(0xFF0000));
    assert_eq!(prop(&ui, "plain", "corner_radius"), Some(PropValue::Float(3.0)));
}

#[test]
fn a_native_field_is_typed_in_and_shows_its_placeholder_and_its_line() {
    let mut ui = open(
        "field",
        "material",
        r##"
        import { useState } from 'rsui';
        export default function App() {
            const [name, setName] = useState('');
            return (
                <window padding={16} gap={8} align="start" background="#ffffff">
                    <input key="name" native width={300} value={name} onChange={setName} placeholder="Name" />
                    <input key="other" native width={300} value={name} onInput={setName} android={{ variant: 'filled' }} />
                    <text>{'name ' + name}</text>
                </window>
            );
        }
        "##,
    );
    assert_eq!(prop(&ui, "name", "placeholder"), Some(PropValue::from("Name")));
    assert_eq!(prop(&ui, "name", "placeholder_color"), color(0x49454F));
    assert_eq!(ui.rect_of("name")[3], 56.0);
    let [x, y, width, height] = ui.rect_of("name");
    // Its outline is 1 wide in Material's outline color until it's typed in, when it's 2 wide in the primary color.
    let edge = (x + width / 2.0, y + height - 1.5);
    let before = pixels(&mut ui);
    assert_ne!(pixel(&before, edge.0, edge.1), [0x67, 0x50, 0xA4]);
    ui.click("name");
    ui.type_text("Ada");
    assert!(ui.says("name Ada"), "{}", ui.app.outline());
    let after = pixels(&mut ui);
    assert_eq!(pixel(&after, edge.0, edge.1), [0x67, 0x50, 0xA4]);
    // `onInput` is what's typed, as `onChange` is.
    ui.click("other");
    ui.type_text("!");
    assert!(ui.says("name Ada!"), "{}", ui.app.outline());
}

#[test]
fn native_checkboxes_switches_and_sliders_say_what_they_change_to() {
    let mut ui = open(
        "toggles",
        "fluent",
        r#"
        import { useState } from 'rsui';
        export default function App() {
            const [tick, setTick] = useState(false);
            const [on, setOn] = useState(true);
            const [level, setLevel] = useState(5);
            return (
                <window padding={16} gap={8} align="start">
                    <checkbox key="tick" native checked={tick} onChange={setTick}>Remember me</checkbox>
                    <switch key="on" native checked={on} onChange={setOn}>Wi-Fi</switch>
                    <slider key="level" native width={220} value={level} min={0} max={10} onChange={setLevel} />
                    <progress key="done" native width={220} value={level} max={10} />
                    <text>{'tick ' + tick + ' on ' + on + ' level ' + level}</text>
                </window>
            );
        }
        "#,
    );
    assert!(ui.says("tick false on true level 5"), "{}", ui.app.outline());
    // A box, then its label, which is its name.
    assert_eq!(node(&ui, "tick").children.len(), 2);
    assert_eq!(&*node(&ui, "tick").children[1].name, "Remember me");
    ui.click("tick");
    assert!(ui.says("tick true on true"), "{}", ui.app.outline());
    // Ticked, there's a check mark in the box.
    assert_eq!(node(&ui, "tick").children[0].children.len(), 1);
    ui.click("on");
    assert!(ui.says("tick true on false"), "{}", ui.app.outline());
    ui.click("level");
    ui.press(rsui::KeyCode::Right);
    assert!(ui.says("level 6"), "{}", ui.app.outline());
    ui.press(rsui::KeyCode::End);
    assert!(ui.says("level 10"), "{}", ui.app.outline());
    ui.press(rsui::KeyCode::Home);
    assert!(ui.says("level 0"), "{}", ui.app.outline());
}

#[test]
fn the_look_comes_from_the_apps_config_and_what_is_left_out_is_said_once() {
    let app = r##"
        export default function App() {
            return (
                <window>
                    <button key="one" native background="#ff0000">One</button>
                    <button key="two" native background="#00ff00" className="rounded">Two</button>
                </window>
            );
        }
    "##;
    // Material's button ripples, and Fluent's doesn't.
    for (theme, ripples) in [("material", true), ("fluent", false)] {
        let dir = folder(&format!("config-{theme}"));
        std::fs::write(dir.join("app.tsx"), app).unwrap();
        std::fs::write(
            dir.join("rsui.config.ts"),
            format!("import {{ defineConfig }} from 'rsui/config';\nexport default defineConfig({{ id: 'dev.rsui.looks', theme: '{theme}' }});\n"),
        )
        .unwrap();
        let output = Command::new(TOOL)
            .env_remove("RSUI_THEME")
            .env("RSUI_COLOR_SCHEME", "light")
            .arg("--do")
            .arg("look")
            .arg(dir.join("app.tsx"))
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&output.stdout);
        let complained = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{said}\n{complained}");
        let button = said.lines().find(|line| line.contains("button \"One\"")).unwrap_or_default();
        assert_eq!(button.contains("ripple="), ripples, "{theme}: {said}");
        assert!(button.contains("focus_ring="), "{theme}: {said}");
        assert_eq!(complained.matches("leaves out `background`").count(), 1, "{complained}");
        assert_eq!(complained.matches("leaves out `className`").count(), 1, "{complained}");
        // `className` doesn't make it a page's button.
        assert!(said.contains("button \"Two\""), "{said}");
    }
}

#[test]
fn native_controls_go_dark_with_the_system() {
    let mut ui = open("dark", "material", BUTTON);
    assert_eq!(prop(&ui, "save", "background"), color(0x6750A4));
    // What the window was told last may be the system's own: it's told light first, as what's drawn is.
    ui.app.set_dark_mode(false);
    ui.settle();
    ui.app.set_dark_mode(true);
    ui.settle();
    // Material's dark scheme: its primary is lighter, and what's on it darker.
    assert_eq!(prop(&ui, "save", "background"), color(0xD0BCFF));
    assert_eq!(prop(&ui, "save", "text_color"), color(0x381E72));
    ui.app.set_dark_mode(false);
    ui.settle();
    assert_eq!(prop(&ui, "save", "background"), color(0x6750A4));
}
