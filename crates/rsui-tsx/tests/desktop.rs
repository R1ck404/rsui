//! What a program asks of the desktop. None of it happens here: it's said, and the steps do what the system would've done back.

use std::path::PathBuf;
use std::process::Command;

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn app(name: &str, source: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-desktop-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    file
}

fn run(file: &PathBuf, steps: &str) -> (bool, String) {
    let output = Command::new(TOOL)
        .arg("--do")
        .arg(steps)
        .arg(file)
        .output()
        .unwrap();
    let said = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), said)
}

fn works(file: &PathBuf, steps: &str) -> String {
    let (ok, said) = run(file, steps);
    assert!(ok, "{said}");
    said
}

/// An app that shows what's happened to it.
const LOG: &str = r#"
    import { useState } from 'rsui';
    let said = '';
    let redraw = () => {};
    export const log = (text) => { said += text + ';'; redraw(); };
    export function Log() {
        const [, set] = useState(0);
        redraw = () => set((n) => n + 1);
        return <text>{'log ' + said}</text>;
    }
"#;

fn with_log(name: &str, source: &str) -> PathBuf {
    let file = app(name, source);
    std::fs::write(file.with_file_name("log.tsx"), LOG).unwrap();
    file
}

#[test]
fn a_global_shortcut_is_said_and_runs_when_its_keys_are_pressed() {
    let file = with_log(
        "shortcut",
        r#"
        import { shortcuts } from 'rsui/desktop';
        import { log, Log } from './log';
        const stop = shortcuts.register('CommandOrControl+Shift+K', () => log('pressed'));
        export default function App() {
            shortcuts.useShortcut('alt+f9', () => log('hook'));
            return (
                <window>
                    <Log />
                    <button key="stop" onClick={() => { stop(); log('stopped'); }}>Stop</button>
                </window>
            );
        }
        "#,
    );
    let said = works(
        &file,
        "global ctrl+shift+k; expect \"pressed;\"; global alt+f9; expect \"hook;\"; click #stop",
    );
    assert!(
        said.contains("the global shortcut CommandOrControl+Shift+K"),
        "{said}"
    );
    let (ok, said) = run(&file, "click #stop; global ctrl+shift+k");
    assert!(!ok);
    assert!(
        said.contains("nothing has asked for the global shortcut"),
        "{said}"
    );
}

#[test]
fn asking_for_keys_twice_or_for_keys_that_mean_nothing_fails() {
    let file = app(
        "shortcut-bad",
        r#"
        import { shortcuts } from 'rsui/desktop';
        shortcuts.register('ctrl+k', () => {});
        let twice = '';
        try { shortcuts.register('Control+K', () => {}); } catch (error) { twice = error.message; }
        let nonsense = '';
        try { shortcuts.register('ctrl+nothing', () => {}); } catch (error) { nonsense = error.message; }
        export default function App() {
            return <window><text>{twice}</text><text>{nonsense}</text></window>;
        }
        "#,
    );
    works(
        &file,
        "expect \"already in use\"; expect \"isn't a key this knows\"",
    );
}

#[test]
fn a_tray_icon_has_a_menu_that_can_be_used() {
    let file = with_log(
        "tray",
        r#"
        import { tray } from 'rsui/desktop';
        import { log, Log } from './log';
        tray.set({
            tooltip: 'My app',
            onClick: () => log('clicked'),
            menu: [
                { label: 'Open', onClick: () => log('open') },
                { separator: true },
                { label: 'More', items: [{ label: 'Deep', onClick: () => log('deep') }] },
            ],
        });
        export default function App() {
            return <window><Log /><button key="gone" onClick={() => tray.remove()}>Gone</button></window>;
        }
        "#,
    );
    let said = works(
        &file,
        "tray; expect \"clicked;\"; tray \"Open\"; expect \"open;\"; tray \"More/Deep\"; expect \"deep;\"",
    );
    assert!(said.contains("\"My app\""), "{said}");
    assert!(said.contains("More/Deep"), "{said}");
    let (ok, said) = run(&file, "tray \"Nope\"");
    assert!(!ok);
    assert!(said.contains("doesn't have \"Nope\""), "{said}");
    let (ok, said) = run(&file, "click #gone; tray");
    assert!(!ok);
    assert!(said.contains("there's no tray icon"), "{said}");
}

#[test]
fn a_tray_icon_reads_its_picture_or_says_it_cannot() {
    let file = app(
        "tray-icon",
        r#"
        import { tray } from 'rsui/desktop';
        let said = 'fine';
        try { tray.set({ icon: './not-there.png' }); } catch (error) { said = 'failed ' + error.code; }
        export default function App() { return <window><text>{said}</text></window>; }
        "#,
    );
    works(&file, "expect \"failed ENOENT\"");
}

#[test]
fn a_menu_bar_can_be_used_and_taken_away() {
    let file = with_log(
        "menu",
        r#"
        import { mainWindow } from 'rsui';
        import { log, Log } from './log';
        mainWindow.setMenu([
            { label: 'File', items: [{ label: 'Open', shortcut: 'ctrl+o', onClick: () => log('opening') }] },
        ]);
        export default function App() {
            return <window><Log /><button key="off" onClick={() => mainWindow.setMenu(null)}>Off</button></window>;
        }
        "#,
    );
    let said = works(&file, "menubar \"File/Open\"; expect \"opening;\"");
    assert!(said.contains("File/Open"), "{said}");
    let (ok, said) = run(&file, "click #off; menubar \"File/Open\"");
    assert!(!ok);
    assert!(said.contains("doesn't have"), "{said}");
}

#[test]
fn a_notification_is_said_and_clicking_on_it_only_works_once() {
    let file = with_log(
        "notification",
        r#"
        import { notifications } from 'rsui/desktop';
        import { log, Log } from './log';
        export default function App() {
            return (
                <window>
                    <Log />
                    <button key="one" onClick={() => notifications.show({ title: 'Done', body: 'All of it', onClick: () => log('one') })}>One</button>
                    <button key="web" onClick={() => { const made = new Notification('Web', { body: 'way' }); made.onclick = () => log('web ' + Notification.permission); }}>Web</button>
                </window>
            );
        }
        "#,
    );
    let said = works(
        &file,
        "click #one; notification; expect \"one;\"; click #web; notification; expect \"web granted;\"",
    );
    assert!(said.contains("\"Done\", \"All of it\""), "{said}");
    let (ok, said) = run(&file, "click #one; notification; notification");
    assert!(!ok);
    assert!(said.contains("nothing's listening"), "{said}");
}

#[test]
fn notification_and_audio_and_the_like_are_there_without_importing_them() {
    let file = app(
        "globals",
        r#"
        const before = typeof Notification + ' ' + typeof Audio + ' ' + typeof alert + ' ' + typeof confirm + ' ' + typeof screen;
        export default function App() {
            return <window><text>{before}</text><text>{'answer ' + confirm('Sure?')}</text></window>;
        }
        "#,
    );
    let said = works(
        &file,
        "dialog \"ok\"; expect \"function function function function object\"",
    );
    assert!(
        said.contains("got cancel"),
        "nothing had said what to answer yet: {said}"
    );
}

#[test]
fn sound_plays_to_its_end_without_being_heard() {
    let file = with_log(
        "sound",
        r#"
        import { sound } from 'rsui/desktop';
        import { log, Log } from './log';
        export default function App() {
            return (
                <window>
                    <Log />
                    <button key="play" onClick={() => sound.play('./bell.mp3', { volume: 0.5, onEnded: () => log('ended') })}>Play</button>
                    <button key="web" onClick={() => { const a = new Audio('./bell.wav'); a.onended = () => log('web ended ' + a.ended + ' ' + a.paused); a.play().then(() => log('playing ' + a.paused)); }}>Web</button>
                    <button key="loop" onClick={() => { const a = new Audio('./bell.wav'); a.loop = true; a.addEventListener('ended', () => log('should not end')); a.play(); log('looping'); }}>Loop</button>
                    <button key="beep" onClick={() => sound.beep('warning')}>Beep</button>
                </window>
            );
        }
        "#,
    );
    let said = works(
        &file,
        "click #play; expect no \"ended\"; soundends; expect \"ended;\"; click #web; expect \"playing false;\"; soundends; expect \"web ended true true;\"; click #beep",
    );
    assert!(said.contains("bell.mp3"), "{said}");
    assert!(said.contains("wasn't heard"), "{said}");
    assert!(said.contains("beeped: warning"), "{said}");
    works(
        &file,
        "click #loop; soundends; expect no \"should not end\"; soundends; expect no \"should not end\"",
    );
}

#[test]
fn starting_at_login_is_said_and_nothing_is_changed() {
    let file = app(
        "login",
        r#"
        import { useState } from 'rsui';
        import { app } from 'rsui/desktop';
        export default function App() {
            const [on, setOn] = useState(app.startsAtLogin());
            return (
                <window>
                    <text>{'starts ' + on}</text>
                    <button key="on" onClick={() => { app.setStartAtLogin(true, { args: ['--hidden'] }); setOn(app.startsAtLogin()); }}>On</button>
                    <button key="off" onClick={() => { app.setStartAtLogin(false); setOn(app.startsAtLogin()); }}>Off</button>
                </window>
            );
        }
        "#,
    );
    let said = works(
        &file,
        "expect \"starts false\"; click #on; expect \"starts true\"; click #off; expect \"starts false\"",
    );
    assert!(said.contains("--hidden"), "{said}");
    assert!(said.contains("app.tsx"), "it says which app: {said}");
    assert!(said.contains("not to start at login"), "{said}");
}

#[test]
fn being_the_only_one_of_a_program_hears_about_others() {
    let file = with_log(
        "single",
        r#"
        import { app } from 'rsui/desktop';
        import { log, Log } from './log';
        const first = app.singleInstance((args) => log('second with ' + args.length + ' ' + args.join(',')));
        export default function App() { return <window><text>{'first ' + first}</text><Log /></window>; }
        "#,
    );
    works(
        &file,
        "expect \"first true\"; secondinstance \"one\" \"two words\"; expect \"second with 2 one,two words;\"; secondinstance; expect \"second with 0 ;\"",
    );
}

#[test]
fn a_message_box_gets_the_answer_it_was_said_to_get() {
    let file = with_log(
        "message",
        r#"
        import { dialogs } from 'rsui/desktop';
        import { log, Log } from './log';
        export default function App() {
            return (
                <window>
                    <Log />
                    <button key="later" onClick={async () => log('later ' + await dialogs.message({ title: 'T', message: 'Sure?', kind: 'question', buttons: 'yes-no' }))}>Later</button>
                    <button key="now" onClick={() => log('now ' + confirm('Well?'))}>Now</button>
                    <button key="yes" onClick={() => log('confirmed ' + confirm('Go on?'))}>Yes</button>
                </window>
            );
        }
        "#,
    );
    let said = works(
        &file,
        "dialog \"yes\"; click #later; wait \"later yes;\"; click #now; expect \"now false;\"; dialog \"ok\"; click #yes; expect \"confirmed true;\"; click #yes; expect \"confirmed false;\"",
    );
    assert!(said.contains("\"T\", \"Sure?\", and got yes"), "{said}");
}

#[test]
fn the_screens_are_really_asked_about_as_that_changes_nothing() {
    let file = app(
        "screens",
        r#"
        import { displays, power } from 'rsui/desktop';
        const list = displays.list();
        const good = list.every((one) => one.width > 0 && one.scale >= 1 && one.workHeight <= one.height);
        export default function App() {
            return (
                <window>
                    <text>{'good ' + good + ' ' + (list.length === 0 || screen.width > 0) + ' ' + displays.pointer().length + ' ' + typeof power.battery().onBattery}</text>
                </window>
            );
        }
        "#,
    );
    works(&file, "expect \"good true true 2 boolean\"");
}

#[test]
fn keeping_awake_and_power_are_said_and_heard() {
    let file = with_log(
        "power",
        r#"
        import { power } from 'rsui/desktop';
        import { log, Log } from './log';
        const stop = power.onChange((what) => log('power ' + what));
        export default function App() {
            return (
                <window>
                    <Log />
                    <button key="awake" onClick={() => power.keepAwake()}>Awake</button>
                    <button key="stop" onClick={stop}>Stop</button>
                </window>
            );
        }
        "#,
    );
    let said = works(
        &file,
        "click #awake; power suspend; expect \"power suspend;\"; click #stop; power resume; expect no \"power resume\"",
    );
    assert!(said.contains("keep the machine awake: true"), "{said}");
}

#[test]
fn a_program_only_updates_itself_with_exactly_the_file_it_meant() {
    let file = app(
        "update",
        r#"
        import { useState } from 'rsui';
        import { app, updates } from 'rsui/desktop';
        import { writeFileSync } from 'fs';
        import { join } from 'path';
        import { createHash } from 'crypto';
        const update = join(process.env.TEMP ?? '.', 'rsui-update-test-' + process.pid + '.bin');
        writeFileSync(update, 'a new program');
        const right = createHash('sha256').update('a new program').digest('hex');
        function attempt(hash) {
            try { updates.install(update, hash); return 'replaced'; } catch (error) { return error.message; }
        }
        export default function App() {
            const [said, setSaid] = useState('');
            return (
                <window>
                    <text>{said}</text>
                    <button key="none" onClick={() => setSaid(attempt())}>None</button>
                    <button key="wrong" onClick={() => setSaid(attempt('0'.repeat(64)))}>Wrong</button>
                    <button key="right" onClick={() => setSaid(attempt(right))}>Right</button>
                    <button key="again" onClick={() => app.relaunch()}>Again</button>
                </window>
            );
        }
        "#,
    );
    let said = works(
        &file,
        "click #none; expect \"needs the SHA-256\"; click #wrong; expect \"not the update that was meant\"; click #right; expect \"replaced\"; click #again",
    );
    assert!(said.contains("which wasn't done"), "{said}");
    assert!(said.contains("asked to start again"), "{said}");
}

#[test]
fn checking_an_app_never_brings_up_anything_real() {
    // What would come up is only ever said, which is all that can be checked from here, along with the tool still being here.
    let file = app(
        "nothing-real",
        r#"
        import { mainWindow } from 'rsui';
        import { app, notifications, power, sound, tray } from 'rsui/desktop';
        notifications.show({ title: 'T' }); tray.set({ tooltip: 'x' }); mainWindow.setMenu([{ label: 'L' }]); sound.beep(); sound.play('./x.mp3'); power.keepAwake(); app.setStartAtLogin();
        export default function App() { return <window><text>Fine</text></window>; }
        "#,
    );
    let said = works(&file, "expect \"Fine\"");
    for expected in [
        "a notification",
        "a tray icon",
        "a menu bar",
        "beeped",
        "a sound",
        "keep the machine",
        "start at login",
    ] {
        assert!(said.contains(expected), "{expected}: {said}");
    }
}
