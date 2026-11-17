//! What's been added to `rsui/desktop`, and the web's ways into the same things. None of it happens here: it's said,
//! and what the system would say back is pretended. What lasts goes in a sandbox folder of the test's own.

use std::path::PathBuf;
use std::process::Command;

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn folder(name: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-desktop-more-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

fn app(name: &str, source: &str) -> PathBuf {
    let file = folder(name).join("app.tsx");
    std::fs::write(&file, source).unwrap();
    file
}

fn run(file: &PathBuf, steps: &str) -> (bool, String) {
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
    (output.status.success(), said)
}

fn works(file: &PathBuf, steps: &str) -> String {
    let (ok, said) = run(file, steps);
    assert!(ok, "{said}");
    said
}

/// An app whose `run` is called once it's there, which says what it found, as `said ...`.
fn checks(name: &str, imports: &str, body: &str) -> PathBuf {
    app(
        name,
        &format!(
            r#"
            import {{ useEffect, useState }} from 'rsui';
            {imports}
            async function check() {{
                {body}
            }}
            export default function App() {{
                const [said, setSaid] = useState('');
                useEffect(() => {{ check().then((text) => setSaid('said ' + text), (error) => setSaid('failed ' + error.message)); }}, []);
                return <window><text>{{said}}</text></window>;
            }}
            "#
        ),
    )
}

#[test]
fn the_shell_says_what_it_would_open_and_won_t_open_what_is_not_there() {
    let file = checks(
        "shell",
        "import { shell } from 'rsui/desktop'; import { writeFileSync } from 'fs'; import { join, dirname } from 'path';",
        r#"
        const here = join(process.env.RSUI_SANDBOX, 'note.txt');
        writeFileSync(here, 'hi');
        shell.open(here); shell.reveal(here); shell.trash(here); shell.openWith(here); shell.properties(here);
        let missing = '';
        try { shell.open(join(process.env.RSUI_SANDBOX, 'nothing.txt')); } catch (error) { missing = error.message; }
        return missing.includes("there's nothing at") ? 'ok' : 'missing said ' + missing;
        "#,
    );
    std::fs::create_dir_all(file.with_file_name("sandbox")).unwrap();
    let said = works(&file, "wait \"said\"; expect \"said ok\"");
    for expected in ["asked to open", "show in its folder", "Recycle Bin", "choose what opens", "properties of"] {
        assert!(said.contains(expected), "{expected}: {said}");
    }
}

#[test]
fn the_clipboard_keeps_each_format_and_says_which_there_are() {
    let file = checks(
        "clipboard",
        "import { clipboard } from 'rsui/desktop';",
        r#"
        await clipboard.write({ text: 'plain', html: '<b>bold</b>', image: new Uint8Array([137, 80, 78, 71]) });
        const got = await clipboard.read();
        const formats = clipboard.formats().join(',');
        await clipboard.clear();
        const after = await clipboard.read();
        return [got.text, got.html, Array.from(got.image ?? []).join('.'), formats, after.html === undefined].join(' ');
        "#,
    );
    let said = works(&file, "wait \"said\"");
    assert!(said.contains("said plain <b>bold</b> 137.80.78.71 text/plain,text/html,image/png true"), "{said}");
}

#[test]
fn secrets_are_kept_in_the_sandbox_and_never_in_the_credential_manager_while_checking() {
    let file = checks(
        "secrets",
        "import { secrets } from 'rsui/desktop';",
        r#"
        await secrets.set('token', 'shh');
        const got = await secrets.get('token');
        const gone = await secrets.delete('token');
        const after = await secrets.get('token');
        return [got, gone, after === undefined].join(' ');
        "#,
    );
    let said = works(&file, "wait \"said\"");
    assert!(said.contains("said shh true true"), "{said}");
    assert!(said.contains("kept a secret as \"token\""), "{said}");
}

#[test]
fn a_notification_can_be_changed_and_taken_away_and_clicks_are_heard() {
    let file = checks(
        "notifications",
        "import { notifications } from 'rsui/desktop';",
        r#"
        const heard = [];
        notifications.onClick(({ id }) => heard.push('any ' + id));
        const first = notifications.show({ title: 'One', onClick: () => heard.push('own') });
        const changed = notifications.update(first, { body: 'more' });
        const second = notifications.show({ title: 'Two' });
        const stale = notifications.update(first, { body: 'late' });
        globalThis.heard = heard;
        notifications.dismiss(second);
        return [first, second, changed, stale].join(' ');
        "#,
    );
    let said = works(&file, "wait \"said\"");
    assert!(said.contains("said 1 2 true false"), "{said}");
    assert!(said.contains("took the notification away"), "{said}");
}

#[test]
fn links_and_kinds_of_file_are_said_to_be_this_program_s_in_the_sandbox() {
    let file = checks(
        "protocols",
        "import { protocols, fileAssociations } from 'rsui/desktop';",
        r#"
        let bad = '';
        try { protocols.register('https'); } catch (error) { bad = 'refused'; }
        protocols.register('myapp:');
        const on = protocols.isRegistered('myapp');
        protocols.unregister('myapp');
        const off = protocols.isRegistered('myapp');
        fileAssociations.register({ extension: '.mynote', description: 'My note' });
        fileAssociations.unregister('mynote');
        return [bad, on, off].join(' ');
        "#,
    );
    let said = works(&file, "wait \"said\"");
    assert!(said.contains("said refused true false"), "{said}");
    assert!(said.contains("opens myapp: links"), "{said}");
    assert!(said.contains("opens .mynote files"), "{said}");
}

#[test]
fn what_the_system_says_about_itself_has_the_shape_it_should() {
    let file = checks(
        "system",
        "import { system, network, power, displays, app } from 'rsui/desktop';",
        r#"
        const access = system.accessibility();
        const battery = power.battery();
        const status = network.status();
        const info = app.getInfo();
        const paths = app.getPaths();
        return [
            /^[a-z]{2,3}(-[A-Za-z0-9]+)*$/.test(system.locale()), typeof system.timeZone(), typeof system.idleTime(),
            typeof access.reducedMotion, typeof access.textScale, typeof battery.onBattery, typeof status.online,
            displays.forWindow() === null || displays.forWindow().width > 0, typeof info.version, typeof paths.temp,
        ].join(' ');
        "#,
    );
    let said = works(&file, "wait \"said\"");
    assert!(said.contains("said true string number boolean number boolean boolean true string string"), "{said}");
}

#[test]
fn dragging_files_out_is_said_and_what_is_not_there_is_refused_as_a_promise() {
    // A real drag needs a button held, and was tried in a real window; here it's said, and nothing's dropped.
    let file = checks(
        "drag",
        "import { drag } from 'rsui/desktop'; import { writeFileSync } from 'fs'; import { join } from 'path';",
        r#"
        const here = join(process.env.RSUI_SANDBOX, 'dragged.txt');
        writeFileSync(here, 'hi');
        const done = await drag.startFiles(here);
        let refused = '';
        const asked = drag.startFiles(join(process.env.RSUI_SANDBOX, 'nothing.txt'));
        await asked.catch((error) => { refused = error.message; });
        return done + ' ' + (refused.includes("there's nothing at") ? 'refused' : 'refused ' + refused);
        "#,
    );
    std::fs::create_dir_all(file.with_file_name("sandbox")).unwrap();
    let said = works(&file, "wait \"said\"");
    assert!(said.contains("said none refused") && said.contains("started dragging 1 file(s)"), "{said}");
}

#[test]
fn the_time_zone_is_a_place_s_and_a_window_s_picture_is_refused_with_a_reason_while_checking() {
    // These are read for real while checking, as they change nothing. Windows 10 1903 and later have the ICU that names the
    // zone. A real window showed Etc/GMT-1, the offset alone, for Europe/Amsterdam before.
    let file = checks(
        "zone",
        "import { mainWindow } from 'rsui'; import { system } from 'rsui/desktop';",
        r#"
        const zone = system.timeZone();
        const accent = system.accentColor();
        let refused = '';
        await mainWindow.capture().catch((error) => { refused = error.message; });
        return [/^[A-Z][A-Za-z_]+\/[A-Za-z_\/+-]+$/.test(zone) && !zone.startsWith('Etc/') ? 'zone' : 'zone ' + zone,
            accent === null || /^#[0-9a-f]{6}$/.test(accent) ? 'accent' : 'accent ' + accent,
            refused.includes('`shot` step') ? 'refused' : 'refused ' + refused].join(' ');
        "#,
    );
    let said = works(&file, "wait \"said\"");
    assert!(said.contains("said zone accent refused"), "{said}");
}

#[test]
fn a_menu_where_the_pointer_is_can_be_chosen_from_in_steps() {
    let file = app(
        "popup",
        r#"
        import { useState } from 'rsui';
        import { menus } from 'rsui/desktop';
        export default function App() {
            const [said, setSaid] = useState('nothing');
            return (
                <window>
                    <text>{'chose ' + said}</text>
                    <button key="open" onClick={() => menus.popup([{ label: 'Copy', onClick: () => setSaid('copy') }, { separator: true }, { label: 'More', items: [{ label: 'Deep', onClick: () => setSaid('deep') }] }])}>Open</button>
                </window>
            );
        }
        "#,
    );
    let said = works(&file, "click #open; popup \"More/Deep\"; expect \"chose deep\"");
    assert!(said.contains("showed a menu of Copy, More/Deep"), "{said}");
    let (ok, said) = run(&file, "popup \"Copy\"");
    assert!(!ok && said.contains("no menu is showing"), "{said}");
}

#[test]
fn the_taskbar_and_the_window_s_own_controls_are_asked_of_the_window() {
    let file = app(
        "taskbar",
        r#"
        import { mainWindow } from 'rsui';
        import { taskbar } from 'rsui/desktop';
        export default function App() {
            return (
                <window>
                    <button key="go" onClick={() => {
                        taskbar.setProgress(0.5, 'paused'); taskbar.requestAttention();
                        mainWindow.setOpacity(0.8); mainWindow.setResizable(false); mainWindow.setSizeLimits({ minWidth: 300, maxWidth: 900 });
                        mainWindow.setSkipTaskbar(true); mainWindow.setClickThrough(true); mainWindow.setContentProtection(true);
                        mainWindow.showWithoutActivation(); mainWindow.startResize('left');
                    }}>Go</button>
                </window>
            );
        }
        "#,
    );
    let said = works(&file, "click #go");
    for expected in ["Progress(0.5, \"paused\")", "Flash(\"critical\")", "Opacity(0.8)", "Resizable(false)", "SizeLimits(Some((300, 0)), Some((900, 0)))", "SkipTaskbar(true)", "ClickThrough(true)", "ContentProtection(true)", "ShowWithoutActivation", "StartResize(\"left\")"] {
        assert!(said.contains(expected), "{expected}: {said}");
    }
}

#[test]
fn the_title_is_one_thing_the_page_and_the_window_both_set_and_read() {
    let file = app(
        "title",
        r#"
        import { PageWindow } from 'rsui/web';
        import { mainWindow } from 'rsui';
        import { useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('');
            return (
                <PageWindow>
                    <p>{said}</p>
                    <button id="page" onClick={() => { document.title = 'From the page'; setSaid('rsui reads ' + mainWindow.getState().title); }}>page</button>
                    <button id="rsui" onClick={() => { mainWindow.setTitle('From rsui'); setSaid('page reads ' + document.title); }}>rsui</button>
                </PageWindow>
            );
        }
        "#,
    );
    works(&file, "click \"page\"; expect \"rsui reads From the page\"; click \"rsui\"; expect \"page reads From rsui\"");
}

#[test]
fn local_storage_and_storage_are_one_store_and_each_hears_the_other() {
    let file = app(
        "storage",
        r#"
        import { PageWindow } from 'rsui/web';
        import { storage } from 'rsui/desktop';
        import { useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('');
            return (
                <PageWindow>
                    <p>{said}</p>
                    <button id="go" onClick={() => {
                        const heard = [];
                        window.addEventListener('storage', (event) => heard.push('page heard ' + event.key + '=' + event.newValue));
                        storage.onChange(({ key, from }) => heard.push('rsui heard ' + key + ' from ' + from));
                        localStorage.setItem('a', '1');
                        storage.set('b', { n: 2 });
                        setSaid([storage.get('a'), localStorage.getItem('b'), ...heard].join('; '));
                    }}>go</button>
                </PageWindow>
            );
        }
        "#,
    );
    // Each side hears each change once, the rsui way's listener first here as it was added first.
    works(&file, "click \"go\"; expect \"1; {\\\"n\\\":2}; rsui heard a from localStorage; rsui heard b from storage; page heard b={\\\"n\\\":2}\"");
}

#[test]
fn hiding_the_window_and_covering_the_screen_are_heard_by_the_page_once_each() {
    let file = app(
        "visibility",
        r#"
        import { PageWindow } from 'rsui/web';
        import { mainWindow } from 'rsui';
        import { useState } from 'react';
        const heard = [];
        export default function App() {
            const [said, setSaid] = useState('');
            return (
                <PageWindow>
                    <p>{said}</p>
                    <button id="go" onClick={() => {
                        document.addEventListener('visibilitychange', () => heard.push('hidden ' + document.hidden));
                        document.addEventListener('fullscreenchange', () => heard.push('fullscreen ' + (document.fullscreenElement !== null)));
                        mainWindow.onStateChange((state) => heard.push('state ' + state.minimized + ' ' + state.fullscreen));
                        mainWindow.minimize();
                        mainWindow.restore();
                        document.body.requestFullscreen();
                        setSaid(heard.join('; '));
                    }}>go</button>
                </PageWindow>
            );
        }
        "#,
    );
    works(&file, "click \"go\"; expect \"hidden true; state true false; hidden false; state false false; fullscreen true; state false true\"");
}

#[test]
fn match_media_answers_as_css_does_and_says_when_the_window_changes_size() {
    let file = app(
        "media",
        r#"
        import { PageWindow } from 'rsui/web';
        import { useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('');
            return (
                <PageWindow>
                    <p>{said}</p>
                    <button id="go" onClick={() => {
                        const wide = matchMedia('(min-width: 700px)');
                        const heard = [];
                        wide.addEventListener('change', (event) => heard.push('now ' + event.matches));
                        globalThis.heard = heard;
                        setSaid('wide ' + wide.matches + ' ' + (matchMedia('(prefers-color-scheme: dark)').matches || matchMedia('(prefers-color-scheme: light)').matches));
                    }}>go</button>
                    <button id="say" onClick={() => setSaid(globalThis.heard.join(' '))}>say</button>
                </PageWindow>
            );
        }
        "#,
    );
    works(&file, "size 800 600; click \"go\"; expect \"wide true true\"; size 500 600; wait 50; click \"say\"; expect \"now false\"");
}

#[test]
fn a_window_that_remembers_its_place_keeps_it_in_the_app_s_store() {
    let file = app(
        "placement",
        r#"
        import { Window, useState } from 'rsui';
        export default function App() {
            const [open, setOpen] = useState(false);
            return (
                <window>
                    <button key="open" onClick={() => setOpen(true)}>Open</button>
                    {open && <Window title="Settings" rememberPlacement="settings"><text>Inside</text></Window>}
                </window>
            );
        }
        "#,
    );
    let sandbox = file.with_file_name("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();
    std::fs::write(sandbox.join("storage.json"), r#"{"__placement:settings":{"x":40,"y":50,"width":320,"height":240,"maximized":false}}"#).unwrap();
    let said = works(&file, "click #open; wait 50");
    assert!(said.contains("Resize(320, 240)") && said.contains("MoveTo(40, 50)"), "{said}");
}

#[test]
fn an_update_is_newer_by_its_version_as_semver_has_it() {
    let file = checks(
        "updates",
        "import { updates } from 'rsui/desktop';",
        r#"
        const newer = updates.__newer;
        return [newer('1.10.0', '1.9.2'), newer('1.0.0', '1.0.0'), newer('1.0.0', '1.0.0-beta'), newer('1.0.0-beta', '1.0.0'), updates.waitingToConfirm()].join(' ');
        "#,
    );
    let said = works(&file, "wait \"said\"");
    assert!(said.contains("said true false true false false"), "{said}");
}

// Tier 2.

/// A small picture file, in the folder, which Windows makes a thumbnail of: four by four pixels, as a BMP.
fn picture_file(folder: &std::path::Path) -> PathBuf {
    let (width, height) = (4u32, 4u32);
    let row = (width * 3).div_ceil(4) * 4;
    let pixels = row * height;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&(54 + pixels).to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&54u32.to_le_bytes());
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&24u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]);
    for y in 0..height {
        for x in 0..width {
            bytes.extend_from_slice(&[(x * 60) as u8, (y * 60) as u8, 200]);
        }
        bytes.extend(std::iter::repeat_n(0, (row - width * 3) as usize));
    }
    let file = folder.join("picture.bmp");
    std::fs::write(&file, bytes).unwrap();
    file
}

#[test]
fn a_file_s_thumbnail_and_icon_are_read_for_real_as_pngs() {
    let file = checks(
        "files",
        "import { files } from 'rsui/desktop'; import { join, dirname } from 'path';",
        r#"
        const here = dirname(process.env.RSUI_SANDBOX);
        const looks = (one, most) => one !== null && one.width > 0 && one.width <= most && one.height > 0 && one.height <= most
            && one.png[0] === 137 && one.png[1] === 80 && one.url.startsWith('data:image/png;base64,');
        const thumbnail = await files.thumbnail(join(here, 'picture.bmp'), { size: 64 });
        const icon = await files.icon(join(here, 'note.txt'));
        const none = await files.thumbnail(join(here, 'note.txt'));
        let missing = '';
        try { await files.icon(join(here, 'nothing.txt')); } catch (error) { missing = error.message; }
        return [looks(thumbnail, 64), looks(icon, 32), none === null || looks(none, 256), missing.includes("there's nothing at")].join(' ');
        "#,
    );
    let here = file.parent().unwrap();
    picture_file(here);
    std::fs::write(here.join("note.txt"), "hi").unwrap();
    let said = works(&file, "wait \"said\" 10000");
    assert!(said.contains("said true true true true"), "{said}");
}

#[test]
fn a_picture_of_the_screen_is_said_and_blank_while_checking() {
    let file = checks(
        "capture",
        "import { capture, displays } from 'rsui/desktop';",
        r#"
        const part = await capture.region({ x: 10, y: 20, width: 12, height: 5 });
        const main = displays.primary();
        const whole = await capture.screen();
        let none = '';
        try { await capture.screen(99); } catch (error) { none = error.message; }
        return [part.width, part.height, part.png[0] === 137, part.url.startsWith('data:image/png;base64,'),
            main === null || (whole.width === main.width && whole.height === main.height), none.includes('no screen 99')].join(' ');
        "#,
    );
    let said = works(&file, "wait \"said\" 10000");
    assert!(said.contains("said 12 5 true true true true"), "{said}");
    assert!(said.contains("took a picture of the screen at 10, 20, 12 by 5, which is blank here"), "{said}");
}

#[test]
fn the_jump_list_is_said_and_kept_in_the_sandbox() {
    let file = checks(
        "jumplist",
        "import { taskbar } from 'rsui/desktop'; import { readFileSync } from 'fs'; import { join } from 'path';",
        r#"
        taskbar.setJumpList({
            tasks: [{ title: 'New note', args: '--new', description: 'Starts a note' }],
            categories: [{ name: 'Recent', items: [{ title: 'a.md', args: 'a.md' }, { title: 'b.md', args: 'b.md' }] }],
        });
        const kept = JSON.parse(readFileSync(join(process.env.RSUI_SANDBOX, 'rsui-desktop.json'), 'utf8')).jumpList;
        taskbar.clearJumpList();
        const after = JSON.parse(readFileSync(join(process.env.RSUI_SANDBOX, 'rsui-desktop.json'), 'utf8')).jumpList;
        return [kept.tasks[0].args, kept.categories[0].items[1].title, after === null].join(' ');
        "#,
    );
    let said = works(&file, "wait \"said\"");
    assert!(said.contains("said --new b.md true"), "{said}");
    assert!(said.contains("set its jump list to Tasks (New note); Recent (a.md, b.md)"), "{said}");
    assert!(said.contains("cleared its jump list"), "{said}");
}

#[test]
fn thumbnail_buttons_and_the_backdrop_are_asked_of_the_window_and_a_button_can_be_clicked() {
    let file = app(
        "thumbnails",
        r#"
        import { useState, mainWindow } from 'rsui';
        export default function App() {
            const [said, setSaid] = useState('nothing');
            return (
                <window>
                    <text>{'heard ' + said}</text>
                    <button key="go" onClick={() => {
                        mainWindow.setThumbnailButtons([
                            { id: 'play', tooltip: 'Play', onClick: () => setSaid('play') },
                            { id: 'next', tooltip: 'Next', disabled: true, onClick: () => setSaid('next') },
                        ]);
                        mainWindow.setBackdrop('mica');
                        let refused = [];
                        try { mainWindow.setThumbnailButtons(new Array(8).fill({ tooltip: 'x' })); } catch (error) { refused.push('eight'); }
                        try { mainWindow.setBackdrop('glass'); } catch (error) { refused.push('glass'); }
                        setSaid('refused ' + refused.join(' '));
                    }}>Go</button>
                </window>
            );
        }
        "#,
    );
    let said = works(&file, "click #go; expect \"heard refused eight glass\"; thumb play; expect \"heard play\"");
    assert!(said.contains("ThumbnailButtons([\"Play\" (0 bytes of icon), \"Next\" (0 bytes of icon, disabled)])"), "{said}");
    assert!(said.contains("Backdrop(\"mica\")"), "{said}");
    let (ok, said) = run(&file, "click #go; thumb next");
    assert!(!ok && said.contains("is disabled"), "{said}");
    let (ok, said) = run(&file, "click #go; thumb pause");
    assert!(!ok && said.contains("no button \"pause\""), "{said}");
}

/// The first frame of an app with a blue box and white words, drawn with `--png`, as its size and its pixels, red first and
/// not multiplied by alpha. `effect` is what it does once it's there, and `root` what its window says of its background.
fn backdrop_picture(name: &str, web: bool, root: &str, effect: &str) -> (Vec<u8>, rsui_image::Picture) {
    let file = app(
        name,
        &format!(
            r#"
            {}
            import {{ useEffect, mainWindow }} from 'rsui';
            export default function App() {{
                useEffect(() => {{ {effect} }}, []);
                return (
                    <window{root}>
                        <row key="box" width={{40}} height={{30}} background={{0x3366cc}} />
                        <text color={{0xffffff}}>White words on what's seen through</text>
                    </window>
                );
            }}
            "#,
            if web { "import 'rsui/web';" } else { "" }
        ),
    );
    let picture = file.with_file_name("out.png");
    let sandbox = file.with_file_name("sandbox");
    let output = Command::new(TOOL)
        .arg("--png")
        .arg(&picture)
        .arg(&file)
        .env("RSUI_SANDBOX", &sandbox)
        .env("APPDATA", &sandbox)
        .env("LOCALAPPDATA", &sandbox)
        .env("USERPROFILE", &sandbox)
        .env("HOME", &sandbox)
        .env("RSUI_OFFLINE", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let bytes = std::fs::read(&picture).unwrap();
    let decoded = rsui_image::decode(&bytes, (960, 640)).unwrap();
    // What's decoded has its colors multiplied by alpha, as blending wants.
    let mut plain = decoded.pixels.clone();
    for pixel in plain.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for part in &mut pixel[..3] {
                *part = ((u32::from(*part) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    (bytes, rsui_image::Picture { pixels: plain, ..decoded })
}

fn pixel(picture: &rsui_image::Picture, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * picture.width + x) * 4) as usize;
    picture.pixels[at..at + 4].try_into().unwrap()
}

/// How many pixels of the white words have a color, as ClearType's edges do, where gray text's don't.
fn colored_edges(picture: &rsui_image::Picture, background: [u8; 3]) -> usize {
    picture
        .pixels
        .chunks_exact(4)
        .filter(|pixel| pixel[..3] != background && pixel[..3] != [0x33, 0x66, 0xcc] && pixel[3] != 0)
        .filter(|pixel| pixel[..3].iter().max().unwrap() - pixel[..3].iter().min().unwrap() > 6)
        .count()
}

#[test]
fn a_backdrop_shows_through_where_the_window_has_no_background_and_not_through_what_has_a_color() {
    const BLUE: [u8; 4] = [0x33, 0x66, 0xcc, 255];
    const RSUI_BACKGROUND: [u8; 4] = [0xf6, 0xf8, 0xfa, 255];
    let (plain_bytes, plain) = backdrop_picture("backdrop-none", false, "", "");
    assert_eq!((plain.width, plain.height), (960, 640));
    assert_eq!(pixel(&plain, 959, 639), RSUI_BACKGROUND, "without a backdrop the window is as it was");
    assert_eq!(pixel(&plain, 5, 5), BLUE);
    assert!(plain.pixels.chunks_exact(4).all(|pixel| pixel[3] == 255), "and it's all opaque");

    // A backdrop and then none is the same as never having one, to the byte.
    let (off_bytes, _) = backdrop_picture("backdrop-off", false, "", "mainWindow.setBackdrop('mica'); mainWindow.setBackdrop('none');");
    assert_eq!(off_bytes, plain_bytes, "taking the backdrop away puts it all back");

    for (name, root) in [("backdrop-mica", ""), ("backdrop-transparent", " background=\"transparent\"")] {
        let (_, seen) = backdrop_picture(name, false, root, "mainWindow.setBackdrop('mica');");
        assert_eq!(pixel(&seen, 959, 639)[3], 0, "{name}: where nothing's drawn, the backdrop shows");
        assert_eq!(pixel(&seen, 5, 5), BLUE, "{name}: what has a color is as opaque as it is");
        let edges = seen.pixels.chunks_exact(4).filter(|pixel| pixel[3] != 0 && pixel[3] != 255).count();
        assert!(edges > 100, "{name}: the words are there, with {edges} partly covered pixels");
    }

    // A window that has a color of its own covers its backdrop with it.
    let (_, white) = backdrop_picture("backdrop-white", false, " background=\"#ffffff\"", "mainWindow.setBackdrop('mica');");
    assert_eq!(pixel(&white, 959, 639), [255, 255, 255, 255]);
    assert!(white.pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));

    // Text drawn as a browser draws it is ClearType's when the system's set to that, which only works over something opaque, so
    // on what's seen through it's gray.
    let (_, page) = backdrop_picture("backdrop-page", true, " background=\"#000000\"", "");
    let (_, seen_page) = backdrop_picture("backdrop-page-mica", true, "", "mainWindow.setBackdrop('mica');");
    assert_eq!(pixel(&seen_page, 959, 639)[3], 0);
    let clear_type = colored_edges(&page, [0, 0, 0]);
    let on_backdrop = colored_edges(&seen_page, [0, 0, 0]);
    assert_eq!(on_backdrop, 0, "gray on the backdrop, where it has {clear_type} colored pixels on black");
}

#[test]
fn a_color_and_a_font_are_asked_for_and_the_dialog_step_answers() {
    let file = app(
        "pickers",
        r#"
        import { useState } from 'rsui';
        import { dialogs } from 'rsui/desktop';
        export default function App() {
            const [said, setSaid] = useState('nothing');
            return (
                <window>
                    <text>{'got ' + said}</text>
                    <button key="color" onClick={() => dialogs.pickColor({ initial: '#000000' }).then((color) => setSaid('color ' + color))}>Color</button>
                    <button key="font" onClick={() => dialogs.pickFont({ initial: { family: 'Segoe UI', size: 12 } }).then((font) => setSaid(font === null ? 'no font' : ['font', font.family, font.size, font.weight, font.italic, font.color].join(' ')))}>Font</button>
                </window>
            );
        }
        "#,
    );
    let said = works(
        &file,
        r##"dialog "#FF8000"; click #color; expect "got color #ff8000"; click #color; expect "got color null"; dialog "{\"family\":\"Arial\",\"size\":14,\"weight\":700,\"italic\":true,\"color\":\"#112233\"}"; click #font; expect "got font Arial 14 700 true #112233"; dialog cancel; click #font; expect "got no font""##,
    );
    assert!(said.contains("a color was asked for, and got #FF8000"), "{said}");
    assert!(said.contains("a color was asked for, and was cancelled"), "{said}");
    assert!(said.contains("a font was asked for, and got"), "{said}");
}

#[test]
fn the_sound_devices_have_the_shape_they_should_both_ways_in() {
    let file = app(
        "audio",
        r#"
        import { PageWindow } from 'rsui/web';
        import { audio } from 'rsui/desktop';
        import { useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('');
            return (
                <PageWindow>
                    <p>{said}</p>
                    <button id="go" onClick={async () => {
                        const devices = audio.devices();
                        const ours = Array.isArray(devices) && devices.every((one) => typeof one.id === 'string' && typeof one.name === 'string'
                            && (one.kind === 'output' || one.kind === 'input') && typeof one.isDefault === 'boolean');
                        const web = await navigator.mediaDevices.enumerateDevices();
                        const theirs = Array.isArray(web) && web.length === devices.length && web.every((one) => typeof one.deviceId === 'string'
                            && typeof one.label === 'string' && (one.kind === 'audiooutput' || one.kind === 'audioinput'));
                        const stop = audio.onChange(() => {});
                        setSaid(['shape', ours, theirs, typeof stop].join(' '));
                    }}>go</button>
                </PageWindow>
            );
        }
        "#,
    );
    works(&file, "click \"go\"; expect \"shape true true function\"");
}

#[test]
fn media_keys_are_heard_the_rsui_way_and_the_web_s() {
    let file = app(
        "media-keys",
        r#"
        import { PageWindow } from 'rsui/web';
        import { media } from 'rsui/desktop';
        import { useState } from 'react';
        const heard = [];
        export default function App() {
            const [said, setSaid] = useState('');
            return (
                <PageWindow>
                    <p>{said}</p>
                    <button id="listen" onClick={() => {
                        media.onKey((key) => heard.push('rsui ' + key));
                        navigator.mediaSession.metadata = new MediaMetadata({ title: 'Song' });
                        navigator.mediaSession.playbackState = 'playing';
                        navigator.mediaSession.setActionHandler('pause', () => heard.push('web pause'));
                        navigator.mediaSession.setActionHandler('nexttrack', () => heard.push('web next'));
                        setSaid('listening to ' + navigator.mediaSession.metadata.title);
                    }}>listen</button>
                    <button id="say" onClick={() => setSaid(heard.join(', '))}>say</button>
                </PageWindow>
            );
        }
        "#,
    );
    works(&file, "click \"listen\"; expect \"listening to Song\"; media next; media playpause; click \"say\"; expect \"rsui next, web next, rsui playPause, web pause\"");
    let (ok, said) = run(&file, "click \"listen\"; media louder");
    assert!(!ok && said.contains("isn't a media key"), "{said}");
    let (ok, said) = run(&file, "media next");
    assert!(!ok && said.contains("nothing's listening for the media keys"), "{said}");
}
