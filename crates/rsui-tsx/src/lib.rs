//! Writes rsui apps in TypeScript and JSX, with hooks like React's, and no webview.
//!
//! ```tsx
//! import { useState } from 'rsui';
//!
//! export default function App() {
//!     const [count, setCount] = useState(0);
//!     return (
//!         <window padding={24} gap={12} align="start">
//!             <text>Clicked {count} times</text>
//!             <button onClick={() => setCount(count + 1)} paddingX={14} paddingY={8}>Add one</button>
//!         </window>
//!     );
//! }
//! ```
//!
//! Each tag is an element of rsui's tree, and each component is a component of its runtime. So layout, text, the
//! keyboard, screen readers and agents all see what they see in an app written in Rust, and only what changed renders.
//!
//! Run a file with `rsui-tsx app.tsx`, which watches it and swaps in what changes while keeping the app's state, or use
//! [`run`] from a program of your own.

mod agent;
mod bridge;
mod canvas;
mod config;
mod css;
mod css_build;
mod debugger;
mod declare;
mod drive;
mod geometry;
mod layout;
mod live;
mod logpoint;
mod looks;
mod matching;
mod native;
mod native_more;
mod new;
mod pack;
mod plans;
mod png;
mod process_memory;
mod props;
mod react_compiler;
mod replay;
#[cfg(all(windows, feature = "sampling-profile"))]
mod sampling_profile;
mod selector_receipt;
mod trace_file;
mod variables;
mod vite_env;
mod watch;
mod webview;
mod why;

use std::path::{Path, PathBuf};

pub use bridge::{Entry, keep_state_across_reloads, load, request_reload, root};
/// For finding out why an app's memory grows. See `census.rs` in rsui-js.
#[doc(hidden)]
pub use bridge::{census, retainers, tables};
pub use react_compiler::ReactCompilerOptions;

/// Build transforms and explicitly requested artifact verification support.
#[derive(Clone, Debug)]
pub struct PackOptions {
    pub code_cache: bool,
    pub compiler: Option<ReactCompilerOptions>,
    /// Enables RSUI_PACK_STEPS and the live inspector in this artifact only.
    pub diagnostics: bool,
}

impl Default for PackOptions {
    fn default() -> Self {
        Self {
            code_cache: true,
            compiler: None,
            diagnostics: false,
        }
    }
}

pub fn packed_diagnostics() -> bool {
    rsui_js::packed_diagnostics()
}

/// How to run an app.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// What the window's called. Otherwise it's what the module exports as `title`, or the file's name.
    pub title: Option<String>,
    /// Swaps in changes to the app's files as they're saved, keeping its state.
    pub watch: bool,
}

/// Why an app didn't run.
#[derive(Debug)]
pub enum Error {
    /// The app's code has something wrong with it.
    Script(String),
    /// The window couldn't be made.
    Window(rsui::RunError),
    /// The thread it runs on couldn't be started.
    Thread(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Script(message) => f.write_str(message),
            Self::Window(error) => error.fmt(f),
            Self::Thread(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

/// A thread's worth of stack, for deep component trees and recursive scripts. It only takes memory as it's used.
pub(crate) const STACK: usize = 32 * 1024 * 1024;

/// Runs an app from a file, whose default export is its root component, until its window closes.
pub fn run(path: impl AsRef<Path>, options: Options) -> Result<(), Error> {
    // The graphics driver loads while the app's scripts do, rather than after, and a packed app's scripts are read
    // while the ones before them run.
    rsui::render::start_gpu_early();
    rsui_js::know_modules(bridge::HOST_MODULES);
    rsui_js::read_packed_files_ahead();
    // A packed app's fonts and pictures are inside it.
    if rsui_js::running_packed() {
        rsui::text::read_files_with(rsui_js::read_packed_file);
    }
    let path: PathBuf = path.as_ref().to_owned();
    let thread = std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || run_here(&path, options))
        .map_err(Error::Thread)?;
    thread
        .join()
        .unwrap_or_else(|_| Err(Error::Script("the app's thread panicked".to_owned())))
}

fn run_here(path: &Path, options: Options) -> Result<(), Error> {
    // This is a program that's really running, and what it asks for at the top of its main file really happens.
    native::allow_real_effects();
    // Windows knows it as this app, and not only as the program running it, before it has a window: see `set_app_id`.
    // That's the id its `rsui.config.ts` gives, which a packed app has written in; one that isn't packed has its config
    // read on a thread of its own while its scripts load, as it's only wanted once there's a window.
    let config_id = if rsui_js::running_packed() || config::find(path).is_none() {
        Ok(config::app_id(path))
    } else {
        let path = path.to_owned();
        Err(std::thread::spawn(move || config::app_id(&path)))
    };
    let tell_app_id = move |config_id: Result<Option<String>, std::thread::JoinHandle<Option<String>>>, path: &Path| {
        let id = match config_id {
            Ok(id) => id,
            Err(reading) => reading.join().ok().flatten(),
        };
        let id = id.unwrap_or_else(|| format!("rsui.{}", bridge::name_of_app(path)));
        if let Err(error) = rsui::desktop::set_app_id(&id) {
            eprintln!("rsui-tsx: Windows wasn't told which app this is: {error}");
        }
    };
    let config_id = match config_id {
        Ok(id) => {
            tell_app_id(Ok(id), path);
            None
        }
        reading => Some(reading),
    };
    // Someone working on it can see what renders and why, and save it. Someone it's been handed to has no use for that.
    let worked_on = !rsui_js::running_packed() || rsui_js::packed_diagnostics();
    // The viewer is a tool, not something being worked on: it has no inspector of its own, nor the pill.
    rsui::allow_inspector(worked_on && !trace_file::is_viewer());
    if worked_on && !trace_file::is_viewer() {
        // Where each frame's time goes is kept from the start, lightly, so it's there once something's been slow.
        rsui::inspect::keep_spans_lightly();
        rsui::inspect::set_opener(live::open_viewer);
    }
    rsui::inspect::set_trace_saver(trace_file::save_beside_app);
    // The viewer can look at it while it runs, unless it's the viewer, which has nothing to look at in itself.
    if (!rsui_js::running_packed() || rsui_js::packed_diagnostics()) && !trace_file::is_viewer() {
        live::start(path, &file_title(path));
        // What comes in is kept, with the clock and where random numbers start, so it can be replayed.
        replay::begin().map_err(Error::Script)?;
    }
    rsui::mark("thread");
    let entry = load(path, STACK / 8 * 7).map_err(Error::Script)?;
    if let Some(reading) = config_id {
        tell_app_id(reading, path);
    }
    // A replay is given to the window's app when it first updates, which is once it's there and can be woken.
    if let Some(plan) = replay::take_plan() {
        rsui::inspect::run_in_app(Box::new(move |app| {
            replay::play(app, &plan);
        }));
    }
    rsui::mark("scripts");
    let title = options
        .title
        .or_else(|| entry.title())
        .unwrap_or_else(|| file_title(path));
    live::named(&title);
    if options.watch {
        watch::start(path);
    }
    #[cfg(all(windows, feature = "sampling-profile"))]
    let profile = sampling_profile::NativeProfile::start();
    // HOST's engine remains on this UI thread until its thread-local storage is
    // dropped. Keep the entry too, and finish/match JIT addresses before returning.
    #[cfg(all(windows, feature = "sampling-profile"))]
    let profile_entry = profile.as_ref().map(|_| entry.clone());
    let ran = rsui::run_sized(&title, bridge::wanted_size(), root, entry).map_err(Error::Window);
    #[cfg(all(windows, feature = "sampling-profile"))]
    if let Some(profile) = profile {
        profile.finish();
    }
    #[cfg(all(windows, feature = "sampling-profile"))]
    drop(profile_entry);
    live::stop();
    ran
}

/// Draws an app's first frame to a PNG file, with no window, and prints what's in its tree. This is for looking at an app
/// without running it, and it uses the software GPU, so it works anywhere.
pub fn draw_to_file(path: &Path, png: &Path, size: (u32, u32)) -> Result<(), Error> {
    read_ahead_headless();
    let path = path.to_owned();
    let png = png.to_owned();
    let thread = std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || -> Result<(), Error> {
            let mut desk = drive::Desk::open(&path, STACK / 8 * 7, size).map_err(Error::Script)?;
            rsui::mark("app made");
            rsui_js::write_startup_times("app made");
            // What's on another thread can lead to more of it, as a folder being read can show another to
            // read too, so this goes on until it's all settled, or it's been long enough.
            desk.settle(std::time::Duration::from_secs(10))
                .map_err(Error::Script)?;
            rsui::mark("settled");
            // Drawing is what asks for pictures, and one arriving can change how things are laid out.
            for one in &mut desk.windows {
                one.draw().map_err(Error::Script)?;
            }
            rsui::mark("first drawn");
            rsui_js::write_startup_times("first drawn");
            desk.settle(std::time::Duration::from_secs(1))
                .map_err(Error::Script)?;
            for (number, one) in desk.windows.iter_mut().enumerate() {
                let pixels = one.draw().map_err(Error::Script)?;
                // The main window goes where it's asked to, and the rest next to it, numbered.
                let place = if number == 0 {
                    png.clone()
                } else {
                    let stem = png.file_stem().unwrap_or_default().to_string_lossy();
                    png.with_file_name(format!("{stem}.window-{number}.png"))
                };
                write_png(&place, one.size.0 as usize, one.size.1 as usize, &pixels)
                    .map_err(Error::Thread)?;
                if one.id != rsui::WindowId::MAIN.0 {
                    println!(
                        "\nwindow {number}, {:?}, in {}:",
                        one.title,
                        place.display()
                    );
                }
                println!("{}", one.app.outline());
            }
            Ok(())
        })
        .map_err(Error::Thread)?;
    thread
        .join()
        .unwrap_or_else(|_| Err(Error::Script("the app's thread panicked".to_owned())))
}

/// Gets the viewer ready to show a trace saved by rsui, and says which app file to run for it. `--png` and `--do` work on it
/// as on any other app.
pub fn trace_viewer(trace: &Path) -> Result<PathBuf, Error> {
    trace_file::check(trace).map_err(Error::Script)?;
    let trace = std::path::absolute(trace).map_err(|error| Error::Script(error.to_string()))?;
    // Set before anything else runs, and read by the viewer's own code, on the thread it starts.
    // SAFETY: this is the main thread, before any other has been started.
    unsafe { std::env::set_var(trace_file::TRACE_VAR, &trace) };
    trace_file::viewer_file().map_err(Error::Script)
}

/// Gets the viewer ready to look at an app while it runs, and says which app file to run for it. It looks for apps run with
/// rsui-tsx, and `pid` picks one of them, or it offers what it finds.
pub fn live_viewer(pid: Option<u32>) -> Result<PathBuf, Error> {
    live::forget_gone();
    // SAFETY: this is the main thread, before any other has been started.
    unsafe {
        std::env::set_var(trace_file::LIVE_VAR, live::folder());
        if let Some(pid) = pid {
            std::env::set_var(trace_file::PID_VAR, pid.to_string());
        }
    }
    trace_file::viewer_file().map_err(Error::Script)
}

/// Has the app that's run next replay what came in to it in a trace, up to input `to` if that's said, and stop in what that
/// input does if `pause` says, by way of the environment, so a process it starts doesn't. It's to be said on the main
/// thread before any other has started.
pub fn ask_for_replay(trace: &Path, to: Option<usize>, pause: bool) -> Result<(), Error> {
    trace_file::check(trace).map_err(Error::Script)?;
    let trace = std::path::absolute(trace).map_err(|error| Error::Script(error.to_string()))?;
    // SAFETY: this is the main thread, before any other has been started.
    unsafe {
        std::env::set_var(replay::REPLAY_VAR, &trace);
        match to {
            Some(to) => std::env::set_var(replay::TO_VAR, to.to_string()),
            None => std::env::remove_var(replay::TO_VAR),
        }
        if pause {
            std::env::set_var(replay::PAUSE_VAR, "1");
        } else {
            std::env::remove_var(replay::PAUSE_VAR);
        }
    }
    Ok(())
}

/// What the steps for [`do_steps`] are.
pub const STEPS: &str = drive::STEPS;

/// Makes a new app in this folder, with what it takes to run it, check it, and have an agent work on it. The folder can't
/// already have anything in it. It says what it made.
/// The app's `rsui.config.ts` for `platform` (`windows`, `android`, `macos` or `linux`; this system's if it's `None`), as
/// JSON with everything it leaves out filled in, or what's wrong with it. `app` is its main file or its folder.
pub fn app_config(app: &Path, platform: Option<&str>) -> Result<String, Error> {
    let config = config::load(app, platform.unwrap_or(config::this_platform())).map_err(Error::Script)?;
    serde_json::to_string_pretty(&config).map_err(|error| Error::Script(error.to_string()))
}

pub fn new_project(place: &Path) -> Result<Vec<PathBuf>, Error> {
    new::new_project(place).map_err(Error::Script)
}

/// Serves an app to an agent over the Model Context Protocol, on the standard streams, with no window, until the client
/// closes them. Saving a file changes what's running by the next tool call. Nothing the app writes can reach the client
/// other than through a tool's result, as its standard output goes to the standard error.
pub fn serve_agent(path: &Path) -> Result<(), Error> {
    let path = path.to_owned();
    let thread = std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || agent::serve(&path, STACK / 8 * 7).map_err(Error::Thread))
        .map_err(Error::Thread)?;
    thread
        .join()
        .unwrap_or_else(|_| Err(Error::Script("the app's thread panicked".to_owned())))
}

/// Runs an app with no window, does these steps to it the way a person would, and prints what happens: what changes
/// after each, and what's there at the end. It stops at the first that fails, and that's the error, so a list of steps
/// is a test. It says what the steps are if they don't make sense.
pub fn do_steps(path: &Path, steps: &str) -> Result<(), Error> {
    read_ahead_headless();
    let path = path.to_owned();
    let steps = steps.to_owned();
    let thread = std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || drive::run_steps(&path, STACK / 8 * 7, &steps).map_err(Error::Script))
        .map_err(Error::Thread)?;
    thread
        .join()
        .unwrap_or_else(|_| Err(Error::Script("the app's thread panicked".to_owned())))
}

/// For measuring how a packed app starts: with `RSUI_HEADLESS_AHEAD` set, a packed app run with
/// `--png` or `--do` reads its files ahead as it does in a window, so that its start can be timed without one. Unset,
/// nothing changes.
fn read_ahead_headless() {
    if std::env::var_os("RSUI_HEADLESS_AHEAD").is_some() {
        rsui_js::know_modules(bridge::HOST_MODULES);
        rsui_js::read_packed_files_ahead();
    }
}

fn write_png(path: &Path, width: usize, height: usize, rgba: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, png::encode_drawn(width, height, rgba))
}

fn file_title(path: &Path) -> String {
    path.file_stem().map_or_else(
        || "rsui".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// What says a packed program was just replaced with a new version of itself that hasn't said it works yet, next to the program,
/// with how many times it's been started since.
pub(crate) fn unconfirmed_marker(program: &Path) -> PathBuf {
    program.with_extension("unconfirmed")
}

/// For a packed program that's just been updated: the first time it starts, that's noted. If it starts again without having said
/// it works, with `updates.confirmHealthy()`, the version before it is put back and started, and this says so, as the program
/// should end. It's nothing for a program that wasn't updated.
pub fn roll_back_an_unconfirmed_update() -> bool {
    let Ok(program) = std::env::current_exe() else {
        return false;
    };
    let marker = unconfirmed_marker(&program);
    let Ok(said) = std::fs::read_to_string(&marker) else {
        return false;
    };
    let starts: u32 = said.trim().parse().unwrap_or(0);
    let old = program.with_extension("old");
    if starts == 0 || !old.exists() {
        let _ = std::fs::write(&marker, (starts + 1).to_string());
        return false;
    }
    // It's started before and never said it works: the one before it goes back where it was.
    let bad = program.with_extension("failed");
    let _ = std::fs::remove_file(&bad);
    if std::fs::rename(&program, &bad).is_err() || std::fs::rename(&old, &program).is_err() {
        let _ = std::fs::remove_file(&marker);
        return false;
    }
    let _ = std::fs::remove_file(&marker);
    let _ = std::process::Command::new(&program)
        .args(std::env::args_os().skip(1))
        .spawn();
    true
}

/// Looks for an app inside this program, as [`pack`] puts one, and says which file to run if there is one.
pub fn packed_app() -> Option<PathBuf> {
    rsui_js::open_packed_files()
}

/// Shows that something went wrong, for a packed app, which has no console to say so on.
pub fn show_error(message: &str) {
    pack::show_error(message);
}

/// Makes one program at `out`, from this program and the app at `app`, to hand to someone with nothing else to
/// install. It runs the app's main file to see what it loads, without showing anything. `icon` is a `.ico` file for
/// the program to show. With `code_cache`, what each file compiled to as it ran goes in the program too, which then loads
/// that rather than compiling them as it starts (their top levels still run then, as they would have).
pub fn pack(app: &Path, out: &Path, icon: Option<&Path>, code_cache: bool) -> Result<(), Error> {
    pack_with_react_compiler(app, out, icon, code_cache, None)
}

/// Packs an app with optional build-time React Compiler output, retaining its
/// original module paths. The resulting program does not need Node or Babel.
pub fn pack_with_react_compiler(
    app: &Path,
    out: &Path,
    icon: Option<&Path>,
    code_cache: bool,
    compiler: Option<&ReactCompilerOptions>,
) -> Result<(), Error> {
    pack_with_options(
        app,
        out,
        icon,
        &PackOptions {
            code_cache,
            compiler: compiler.cloned(),
            diagnostics: false,
        },
    )
}

/// Builds a standalone app, optionally with metadata-gated diagnostics.
pub fn pack_with_options(
    app: &Path,
    out: &Path,
    icon: Option<&Path>,
    options: &PackOptions,
) -> Result<(), Error> {
    // A config that's wrong stops it before any of the app has run, with what's wrong with it.
    if config::find(app).is_some() {
        config::load(app, config::this_platform()).map_err(Error::Script)?;
    }
    let code_cache = options.code_cache;
    let diagnostics = options.diagnostics;
    let mut build = options
        .compiler
        .as_ref()
        .map(|options| react_compiler::prepare(app, options))
        .transpose()
        .map_err(Error::Script)?;
    let (app, out, icon) = (app.to_owned(), out.to_owned(), icon.map(Path::to_owned));
    let thread = std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || {
            let overlay = build
                .as_ref()
                .map(|build| build.overlay.clone())
                .unwrap_or_default();
            rsui_js::with_source_overlay(overlay, || {
                if code_cache {
                    bridge::keep_compiled_code();
                }
                bridge::start_packing();
                load(&app, STACK / 8 * 7).map_err(Error::Script)?;
                // What's only loaded once it's wanted, as `lazy(() => import("./Page"))` has it, is packed too, with what it loads.
                let graph = if let Some(build) = &mut build {
                    build
                        .collect_graph(bridge::loaded_files())
                        .map_err(Error::Script)?
                } else {
                    let mut looked_at = std::collections::HashSet::new();
                    loop {
                        let more: Vec<PathBuf> = bridge::loaded_files()
                            .into_iter()
                            .filter(|file| looked_at.insert(file.clone()))
                            .flat_map(|file| pack::dynamic_imports(&file))
                            .filter(|file| !bridge::loaded_files().contains(file))
                            .collect();
                        if more.is_empty() {
                            break;
                        }
                        for file in more {
                            looked_at.insert(file.clone());
                            bridge::load_also(&file);
                        }
                    }
                    Vec::new()
                };
                bridge::before_packing();
                if let Some(build) = &build {
                    build.validate().map_err(Error::Script)?;
                }
                let code = bridge::take_compiled_code();
                pack::pack(
                    &app,
                    &out,
                    icon.as_deref(),
                    &bridge::loaded_files(),
                    &bridge::packed_also(),
                    &bridge::loaded_builtins(),
                    code.as_ref(),
                    &graph,
                    diagnostics,
                )
                .map_err(Error::Script)
            })
        })
        .map_err(Error::Thread)?;
    thread
        .join()
        .unwrap_or_else(|_| Err(Error::Script("the app's thread panicked".to_owned())))
}
