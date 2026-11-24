//! Making one program of an app, then running it somewhere its files aren't. It draws to a file rather
//! than opening a window, so nothing comes up on screen.

use std::path::{Path, PathBuf};
use std::process::Command;

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn folder(name: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("pack-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

fn write(path: PathBuf, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn program_type(program: &Path) -> u16 {
    let bytes = std::fs::read(program).unwrap();
    let header = u32::from_le_bytes(bytes[0x3C..0x40].try_into().unwrap()) as usize;
    u16::from_le_bytes(
        bytes[header + 4 + 20 + 68..header + 4 + 20 + 70]
            .try_into()
            .unwrap(),
    )
}

/// An app with something from everywhere an app gets things from.
fn make_app(source: &Path) -> PathBuf {
    write(
        source.join("app").join("main.tsx"),
        r#"
        import { message } from './parts/message';
        import shared from '../shared/words.js';
        import settings from './settings.json';
        import { fromPackage } from 'some-package';
        import { later } from '@scope/other';
        export const title = 'Packed';
        export default function App() {
            return (
                <window>
                    <text>{message + ' ' + shared + ' ' + settings.word + ' ' + fromPackage + ' ' + later()}</text>
                </window>
            );
        }
        "#,
    );
    write(
        source.join("app").join("parts").join("message.ts"),
        "export const message: string = 'from a part';",
    );
    write(
        source.join("shared").join("words.js"),
        "module.exports = 'from further up';",
    );
    write(
        source.join("app").join("settings.json"),
        r#"{ "word": "from json" }"#,
    );
    let package = source.join("node_modules").join("some-package");
    write(
        package.join("package.json"),
        r#"{ "name": "some-package", "main": "./lib/start.js" }"#,
    );
    write(
        package.join("lib").join("start.js"),
        "exports.fromPackage = 'from a package';",
    );
    let scoped = source.join("node_modules").join("@scope").join("other");
    write(
        scoped.join("package.json"),
        r#"{ "name": "@scope/other", "exports": { ".": "./index.js" } }"#,
    );
    // This part's only loaded once it's asked for, which is after the app has started.
    write(
        scoped.join("index.js"),
        "exports.later = () => require('./lazy.js').word;",
    );
    write(scoped.join("lazy.js"), "exports.word = 'from later';");
    source.join("app").join("main.tsx")
}

#[test]
fn a_packed_app_runs_with_none_of_its_files_there() {
    let source = folder("source");
    let main = make_app(&source);
    let place = folder("place");
    let program = place.join("the app.exe");
    let made = Command::new(TOOL)
        .arg("--pack")
        .arg(&program)
        .arg(&main)
        .output()
        .unwrap();
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    // Double clicking it shouldn't bring up a console.
    assert_eq!(program_type(&program), 2);
    std::fs::remove_dir_all(&source).unwrap();

    let picture = place.join("out.png");
    let ran = Command::new(&program)
        .arg("--png")
        .arg(&picture)
        .current_dir(&place)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&ran.stdout);
    let complained = String::from_utf8_lossy(&ran.stderr);
    assert!(ran.status.success(), "{said}\n{complained}");
    for word in [
        "from a part",
        "from further up",
        "from json",
        "from a package",
        "from later",
    ] {
        assert!(said.contains(word), "no {word} in {said}\n{complained}");
    }
    assert!(picture.exists());
}

/// A junction at `link` to the folder `target`, as pnpm makes, which needs no rights that a symbolic link would.
fn junction(link: &Path, target: &Path) {
    // `mklink` takes a `/` for the start of a switch.
    let (link, target) = (
        link.components().collect::<PathBuf>(),
        target.components().collect::<PathBuf>(),
    );
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    let made = Command::new("cmd")
        .arg("/c")
        .arg("mklink")
        .arg("/J")
        .arg(&link)
        .arg(&target)
        .output()
        .unwrap();
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
}

#[test]
fn a_packed_app_finds_the_packages_pnpm_installed_through_links() {
    let source = folder("pnpm");
    let store = source.join("node_modules").join(".pnpm");
    // Two packages that use a third, which pnpm puts next to each of them as a link to the one copy of it.
    for (name, uses) in [
        ("first", Some("shared")),
        ("@scope/second", Some("shared")),
        ("shared", None),
    ] {
        let place = store
            .join(format!("{}@1.0.0", name.replace('/', "+")))
            .join("node_modules");
        let package = place.join(name);
        write(
            package.join("package.json"),
            &format!(r#"{{ "name": "{name}", "main": "./lib/index.js" }}"#),
        );
        let code = match uses {
            Some(used) => {
                junction(
                    &place.join(used),
                    &store
                        .join(format!("{used}@1.0.0"))
                        .join("node_modules")
                        .join(used),
                );
                format!(
                    "const shared = require('{used}'); exports.word = () => '{name} ' + shared.count();"
                )
            }
            None => "let count = 0; exports.count = () => ++count;".to_owned(),
        };
        write(package.join("lib").join("index.js"), &code);
    }
    // The app's own `node_modules` has only what it says it uses, each a link into the store.
    for name in ["first", "@scope/second"] {
        junction(
            &source.join("node_modules").join(name),
            &store
                .join(format!("{}@1.0.0", name.replace('/', "+")))
                .join("node_modules")
                .join(name),
        );
    }
    let main = source.join("main.tsx");
    write(
        main.clone(),
        r#"
        import { word as first } from 'first';
        import { word as second } from '@scope/second';
        export default function App() {
            return <window><text>{first() + ', ' + second()}</text></window>;
        }
        "#,
    );
    let place = folder("pnpm-place");
    let program = place.join("pnpm app.exe");
    let made = Command::new(TOOL)
        .arg("--pack")
        .arg(&program)
        .arg(&main)
        .output()
        .unwrap();
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    std::fs::remove_dir_all(&source).unwrap();
    let ran = Command::new(&program)
        .arg("--png")
        .arg(place.join("out.png"))
        .current_dir(&place)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&ran.stdout);
    assert!(
        ran.status.success(),
        "{said}\n{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    // Both found the same one, as it's the same folder once the links are followed, as on Node.
    assert!(said.contains("first 1, @scope/second 2"), "{said}");
}

#[test]
fn the_program_it_comes_from_is_not_packed_and_stays_a_console_program() {
    assert_eq!(program_type(Path::new(TOOL)), 3);
    let source = folder("plain");
    let main = source.join("main.tsx");
    write(
        main.clone(),
        "export default function App() { return <window><text>Plain</text></window>; }",
    );
    let picture = source.join("out.png");
    let ran = Command::new(TOOL)
        .arg("--png")
        .arg(&picture)
        .arg(&main)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&ran.stdout).contains("Plain"));
}

#[test]
fn an_app_that_can_not_run_leaves_nothing_behind_and_says_why() {
    let source = folder("broken");
    let main = source.join("main.tsx");
    write(main.clone(), "export const nothing = 1;");
    let program = source.join("broken.exe");
    let made = Command::new(TOOL)
        .arg("--pack")
        .arg(&program)
        .arg(&main)
        .output()
        .unwrap();
    assert!(!made.status.success());
    assert!(
        String::from_utf8_lossy(&made.stderr).contains("default export"),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    assert!(!program.exists());
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut u8, flags: u32) -> *mut u8;
    fn FindResourceW(module: *mut u8, name: *const u16, kind: *const u16) -> *mut u8;
    fn FreeLibrary(module: *mut u8) -> i32;
}

/// One picture of one pixel. Only what's in the file's list of its pictures matters here.
fn tiny_icon() -> Vec<u8> {
    let mut icon = vec![0, 0, 1, 0, 1, 0];
    icon.extend_from_slice(&[1, 1, 0, 0, 1, 0, 32, 0]);
    icon.extend_from_slice(&4u32.to_le_bytes());
    icon.extend_from_slice(&22u32.to_le_bytes());
    icon.extend_from_slice(&[1, 2, 3, 4]);
    icon
}

#[test]
fn a_program_can_have_an_icon_and_still_run() {
    let source = folder("icon");
    let main = source.join("main.tsx");
    write(
        main.clone(),
        "export default function App() { return <window><text>With an icon</text></window>; }",
    );
    let icon = source.join("app.ico");
    std::fs::write(&icon, tiny_icon()).unwrap();
    let program = source.join("with icon.exe");
    let made = Command::new(TOOL)
        .arg("--pack")
        .arg(&program)
        .arg("--icon")
        .arg(&icon)
        .arg(&main)
        .output()
        .unwrap();
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    let wide: Vec<u16> = program
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain([0])
        .collect();
    const AS_DATA: u32 = 2;
    const ICON_GROUP: usize = 14;
    // SAFETY: the name ends in a terminator, and a small number can stand for a name or kind.
    unsafe {
        let module = LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), AS_DATA);
        assert!(!module.is_null());
        assert!(
            !FindResourceW(
                module,
                std::ptr::without_provenance(1),
                ICON_GROUP as *const u16
            )
            .is_null()
        );
        FreeLibrary(module);
    }
    let picture = source.join("out.png");
    let ran = Command::new(&program)
        .arg("--png")
        .arg(&picture)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&ran.stdout).contains("With an icon"),
        "adding the icon can't lose what's on the end of it"
    );
}

#[test]
fn a_picture_of_an_app_with_more_windows_has_a_picture_of_each() {
    let source = folder("pictures");
    let main = source.join("main.tsx");
    write(
        main.clone(),
        r#"
        import { Window } from 'rsui';
        export default function App() {
            return (
                <window>
                    <text>In the main one</text>
                    <Window title="Second" size={[100, 80]}><text>In the second one</text></Window>
                </window>
            );
        }
        "#,
    );
    let picture = source.join("out.png");
    let ran = Command::new(TOOL)
        .arg("--png")
        .arg(&picture)
        .arg(&main)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&ran.stdout);
    assert!(said.contains("In the main one"), "{said}");
    assert!(said.contains(r#"window 1, "Second""#), "{said}");
    assert!(said.contains("In the second one"), "{said}");
    assert!(picture.exists());
    let second = source.join("out.window-1.png");
    let bytes = std::fs::read(second).expect("there's a picture of it");
    // A PNG says how wide and tall it is, here.
    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 100);
    assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 80);
}

/// Packs `main`, with what its files compile to or without, and runs what's made to draw its first frame, saying what it
/// printed, what it drew, how big it is, and how many files it loaded from what it compiled when it was packed.
fn pack_and_draw(
    main: &Path,
    place: &Path,
    name: &str,
    code_cache: bool,
) -> (String, Vec<u8>, u64, String) {
    let program = place.join(format!("{name}.exe"));
    let mut pack = Command::new(TOOL);
    pack.arg("--pack").arg(&program);
    if !code_cache {
        pack.arg("--no-code-cache");
    }
    let made = pack.arg(main).output().unwrap();
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    let picture = place.join(format!("{name}.png"));
    let times = place.join(format!("{name}-times.txt"));
    let ran = Command::new(&program)
        .arg("--png")
        .arg(&picture)
        .current_dir(place)
        .env("RSUI_STARTUP_TIMES", &times)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&ran.stdout).into_owned();
    assert!(
        ran.status.success(),
        "{said}\n{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let times = std::fs::read_to_string(&times).unwrap();
    let cache = times
        .lines()
        .filter(|line| line.starts_with("cache "))
        .next_back()
        .unwrap_or_default()
        .to_owned();
    let size = std::fs::metadata(&program).unwrap().len();
    (said, std::fs::read(&picture).unwrap(), size, cache)
}

#[test]
fn a_packed_app_loads_what_it_compiled_when_it_was_packed() {
    let source = folder("cached-source");
    let main = make_app(&source);
    let place = folder("cached-place");
    let (said, drawn, size, cache) = pack_and_draw(&main, &place, "cached", true);
    let (plain_said, plain_drawn, plain_size, plain_cache) =
        pack_and_draw(&main, &place, "plain", false);
    // It does just what it does without, having loaded rsui's own module and the app's five files rather than compiled
    // them. `lazy.js` is only loaded after it's started, so it wasn't when it was packed, and it's compiled.
    assert_eq!(said, plain_said);
    assert_eq!(drawn, plain_drawn);
    assert_eq!(plain_cache, "cache 0 loaded 0 refused 0 resolved");
    // "cache N loaded N refused N resolved"
    let counts: Vec<u32> = cache
        .split(' ')
        .filter_map(|word| word.parse().ok())
        .collect();
    let [loaded, refused, resolved] = counts[..] else {
        panic!("{cache}");
    };
    assert!(loaded >= 6, "{cache}");
    assert_eq!(refused, 0, "{cache}");
    // What the app's imports and `require`s were found to be when it was packed was known, rather than looked for.
    assert!(resolved >= 4, "{cache}");
    assert!(size > plain_size);
    std::fs::remove_dir_all(&source).unwrap();
    let _ = std::fs::remove_dir_all(&place);
}

/// rsui's own modules, which every app's code imports, go through the code cache and back as they're compiled, and the
/// app does what it does without (`RSUI_CODE_CACHE_CHECK` panics where what's read back isn't what was compiled).
#[test]
fn rsuis_own_modules_read_back_from_the_code_cache_as_they_compiled() {
    let source = folder("own-modules");
    let names = [
        "rsui",
        "rsui/desktop",
        "rsui/web-routes",
        "rsui/dom",
        "rsui/web",
        "rsui/canvas",
        "rsui/editing",
        "rsui/events",
        "react",
        "react-dom/client",
        "react/jsx-runtime",
    ];
    let imports: String = names
        .iter()
        .enumerate()
        .map(|(at, name)| format!("import * as m{at} from '{name}';\n"))
        .collect();
    let counts: Vec<String> = (0..names.len())
        .map(|at| format!("Object.keys(m{at}).length"))
        .collect();
    let main = source.join("main.tsx");
    write(
        main.clone(),
        &format!(
            "{imports}export default function App() {{ return <window><text>{{[{}].join(' ')}}</text></window>; }}\n",
            counts.join(", ")
        ),
    );
    let run = |check: bool| {
        let picture = source.join(format!("{check}.png"));
        let mut command = Command::new(TOOL);
        command
            .arg("--png")
            .arg(&picture)
            .arg(&main)
            .current_dir(&source);
        if check {
            command.env("RSUI_CODE_CACHE_CHECK", "1");
        }
        let ran = command.output().unwrap();
        let said = String::from_utf8_lossy(&ran.stdout).into_owned();
        assert!(
            ran.status.success(),
            "{said}\n{}",
            String::from_utf8_lossy(&ran.stderr)
        );
        (said, std::fs::read(&picture).unwrap())
    };
    let (checked, checked_drawn) = run(true);
    let (plain, plain_drawn) = run(false);
    assert_eq!(checked, plain);
    assert_eq!(checked_drawn, plain_drawn);
    std::fs::remove_dir_all(&source).unwrap();
}
