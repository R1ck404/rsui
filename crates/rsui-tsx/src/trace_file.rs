//! A trace file: what was kept about an app's frames and state, saved to hand to someone or attach to a bug, and opened
//! again with `rsui-tsx --view`, which shows it the same way wherever it's opened.
//!
//! It's JSON, so anything can read it too: `frames` each with what was carried out and what rendered and why, `history`
//! with each change of state, and `summary` as `profile` says it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rsui::{App, FrameRecord};
use rsui::runtime::Cause;
use serde_json::{Value, json};

/// What it says it is, and which version of what's in it.
const KIND: &str = "rsui trace";
/// 2 has `spans`, where each frame's time went, as the viewer's Performance tab shows it.
const VERSION: u32 = 2;

fn ms(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 1_000_000.0).round() / 1000.0
}

/// A frame, as a trace file has it.
pub(crate) fn frame_json(frame: &FrameRecord) -> Value {
    json!({
        "number": frame.number,
        "at": ms(frame.at),
        "script": ms(frame.script),
        "layout": ms(frame.layout),
        "paint": ms(frame.paint),
        "gpu": ms(frame.gpu),
        "present": ms(frame.present),
        "painted": frame.painted,
        "handled": frame.handled.iter().map(|handled| json!({
            "action": handled.action.name(),
            "key": handled.key.to_string(),
            "took": ms(handled.took),
            "note": handled.note,
            "rejected": handled.rejected,
        })).collect::<Vec<_>>(),
        "renders": frame.renders.iter().map(|render| json!({
            "name": render.name.as_ref().map(ToString::to_string),
            "cause": match render.cause {
                Cause::Mounted => "mounted",
                Cause::Props => "props",
                Cause::State => "state",
            },
            "took": ms(render.took),
            "own": ms(render.own),
            "changed": render.changed,
            "depth": render.depth,
            "path": render.path,
            "why": rsui::inspect::describe_render(render)
                .split_once(": ")
                .map(|(_, why)| why.to_owned()),
        })).collect::<Vec<_>>(),
    })
}

/// Each change of state kept, as a trace file has them.
pub(crate) fn history_json() -> Vec<Value> {
    crate::why::history_entries()
        .into_iter()
        .map(|(number, component, said)| json!({ "number": number, "component": component, "said": said }))
        .collect()
}

/// What's kept about an app, as a trace file has it.
pub(crate) fn trace_of(app: &App, title: &str) -> Value {
    let frames: Vec<Value> = app.frames().map(frame_json).collect();
    let history = history_json();
    json!({
        "kind": KIND,
        "version": VERSION,
        "app": title,
        "file": crate::bridge::app_file().file_name().map(|name| name.to_string_lossy().into_owned()),
        "frames": frames,
        "history": history,
        "summary": rsui::summary(app.frames()),
        "replay": crate::replay::json(app),
        "spans": crate::live::spans_news(&json!({ "spans": 0, "labels": 0, "stacks": 0 }), usize::MAX),
    })
}

/// Saves what's kept about an app to a file.
pub(crate) fn save(app: &App, title: &str, path: &Path) -> Result<(), String> {
    let text =
        serde_json::to_string_pretty(&trace_of(app, title)).map_err(|error| error.to_string())?;
    std::fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))
}

/// Saves it next to the app, under a name no other has, for ctrl+shift+S in a window. Says where.
pub(crate) fn save_beside_app(app: &App) -> Result<PathBuf, String> {
    let main = crate::bridge::app_file();
    let folder = std::path::absolute(&main)
        .ok()
        .and_then(|main| main.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let stem = main.file_stem().map_or_else(
        || "app".to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let path = folder.join(format!("{stem}-{seconds}.rsuitrace"));
    save(app, &stem, &path)?;
    Ok(path)
}

/// Checks a file is a trace this can show, and says why not if it isn't.
pub(crate) fn check(path: &Path) -> Result<(), String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("{} isn't a trace: {error}", path.display()))?;
    if value["kind"] != KIND {
        return Err(format!("{} isn't a trace saved by rsui", path.display()));
    }
    if value["version"]
        .as_u64()
        .is_none_or(|version| version > u64::from(VERSION))
    {
        return Err(format!(
            "{} was saved by a newer rsui than this",
            path.display()
        ));
    }
    Ok(())
}

/// What tells the viewer which trace to show, or the folder where apps say they can be looked at while they run, and which
/// of those it's to look at.
pub(crate) const TRACE_VAR: &str = "RSUI_TRACE";
pub(crate) const LIVE_VAR: &str = "RSUI_LIVE";
pub(crate) const PID_VAR: &str = "RSUI_LIVE_PID";
/// Has the viewer dock itself to the app it's looking at, as it does when F12 opened it.
pub(crate) const DOCK_VAR: &str = "RSUI_VIEW_DOCK";

/// Whether this is the viewer, which isn't itself to be looked at.
pub(crate) fn is_viewer() -> bool {
    std::env::var_os(TRACE_VAR).is_some() || std::env::var_os(LIVE_VAR).is_some()
}

/// The viewer, as an app, which reads the trace that `RSUI_TRACE` names, or looks at an app in `RSUI_LIVE`: each of its files,
/// by its name in the folder it's run from, and what's in it. `main.tsx` is the one that's run.
pub(crate) const VIEWER_FILES: &[(&str, &str)] = &[
    ("main.tsx", include_str!("viewer/main.tsx")),
    ("theme.ts", include_str!("viewer/theme.ts")),
    ("link.ts", include_str!("viewer/link.ts")),
    ("ui.tsx", include_str!("viewer/ui.tsx")),
    ("elements.tsx", include_str!("viewer/elements.tsx")),
    ("console.tsx", include_str!("viewer/console.tsx")),
    ("sources.tsx", include_str!("viewer/sources.tsx")),
    ("state.tsx", include_str!("viewer/state.tsx")),
    ("replay.tsx", include_str!("viewer/replay.tsx")),
    ("performance.tsx", include_str!("viewer/performance.tsx")),
    ("rendering.tsx", include_str!("viewer/rendering.tsx")),
    ("spans.ts", include_str!("viewer/spans.ts")),
    ("timeline.tsx", include_str!("viewer/timeline.tsx")),
    ("palette.tsx", include_str!("viewer/palette.tsx")),
];

/// Puts the viewer where it can be run from, and says where its `main.tsx` is.
pub(crate) fn viewer_file() -> Result<PathBuf, String> {
    let folder = std::env::temp_dir().join("rsui-viewer");
    std::fs::create_dir_all(&folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    for (name, code) in VIEWER_FILES {
        let file = folder.join(name);
        if std::fs::read_to_string(&file).ok().as_deref() != Some(*code) {
            std::fs::write(&file, code).map_err(|error| format!("{}: {error}", file.display()))?;
        }
    }
    Ok(folder.join("main.tsx"))
}
