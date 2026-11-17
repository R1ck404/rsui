//! A window that's been resized since it opened, as by maximizing it, is still that size for a new version of its code: a page that
//! lays itself out from `innerWidth` was left laid out for the size it opened at, in the corner of a maximized window.
mod hands;

const SIZED: &str = r#"
    import { PageWindow, dom } from 'rsui/web';
    import { useEffect, useState } from 'react';
    export const size = [300, 200];
    export default function App() {
        // A resize only draws it again: what it says is what `innerWidth` is as it's drawn, so a new version says what that is then.
        const [, redraw] = useState(0);
        useEffect(() => {
            const resized = () => redraw((count) => count + 1);
            addEventListener('resize', resized);
            return () => removeEventListener('resize', resized);
        }, []);
        return <PageWindow><dom.p>{'one ' + innerWidth + ' by ' + innerHeight}</dom.p></PageWindow>;
    }
"#;

#[test]
fn saving_a_new_version_keeps_the_size_the_window_is_now() {
    let folder = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-reload-size-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, SIZED).unwrap();
    rsui_tsx::keep_state_across_reloads();
    let mut ui = hands::Hands::open(&file, (300, 200));
    ui.app.resize((640, 480), 1.0);
    ui.settle();
    ui.wait_for("one 640 by 480");

    std::fs::write(&file, SIZED.replace("'one '", "'two '")).unwrap();
    assert!(rsui_tsx::request_reload(&file));
    ui.wait_for("two 640 by 480");
}
