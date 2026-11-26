//! Runs an app written in TSX: `rsui-tsx app.tsx`. It watches the app's files, and swaps in what changes.
//!
//! `rsui-tsx new my-app` makes a new one to start from. `--help` says what else there is.
//!
//! `--once` leaves out the watching. `--png file.png` draws one frame to that file and prints what's in the tree, with
//! no window, which is how an agent can look at it.
//!
//! `--do "click #add; expect \"1\"; shot out.png"` uses it with no window, the way a person would, and says what happens.
//! It's `--do @file` for steps in a file, and `--do -` for steps from the standard input.
//!
//! `--agent` serves it to an agent over the Model Context Protocol, on the standard streams, with no window. Saving a file
//! changes what's running by the next tool call.
//!
//! `--pack out.exe app.tsx` makes one program out of the app, with `--icon app.ico` for what it shows, to hand to someone
//! with nothing else to install. That program runs its app, and doesn't take any of this but `--png`.

use std::process::ExitCode;

#[cfg(not(any(feature = "alloc-owners", feature = "system-alloc")))]
#[global_allocator]
static ALLOCATOR: rsui::UiAlloc = rsui::UiAlloc;

// What each stage holds is counted, for `census` in steps: see `owners.rs` in rsui-alloc.
#[cfg(feature = "alloc-owners")]
#[global_allocator]
static ALLOCATOR: rsui::alloc_owners::Owned = rsui::alloc_owners::Owned;

fn help() -> String {
    format!(
        "Runs apps written in TypeScript and JSX, with no web view.

  rsui-tsx new my-app            makes a new app in a new folder, which says how to work on it
  rsui-tsx app.tsx               runs it in a window, and swaps in what you save
  rsui-tsx --once app.tsx        the same, without watching its files

With no window, for checking your work or having an agent do it:

  rsui-tsx --png out.png app.tsx      draws it to a file, and prints what's in it
  rsui-tsx --do \"STEPS\" app.tsx       uses it, and says what changed, with exit code 1 at the first step that fails
  rsui-tsx --do @steps.txt app.tsx    steps from a file, which makes a test. `--do -` is from the standard input
  rsui-tsx --agent app.tsx            serves it to a client over the Model Context Protocol, on the standard streams
  rsui-tsx --profile 5s app.tsx       runs it for that long, and says what took the time and what rendered for nothing
  rsui-tsx --view trace.rsuitrace     shows a trace saved with ctrl+shift+S in a window, or `save` with --do
  rsui-tsx --view [PID]               looks at an app that's running in a window, as it runs, to stop it at breakpoints too
  rsui-tsx --replay trace.rsuitrace [--to N] [--pause] app.tsx
                                      runs it again with what came in to it then, up to input N, and stops in what N does

In a window, ctrl+shift+I shows what renders, why, and where each frame's time goes. `--do` has `why TARGET` and `frames` for
the same, and `--agent` has `ui_why` and `ui_frames`.

To hand it to someone:

  rsui-tsx --pack out.exe [--icon app.ico] app.tsx    makes one program, with nothing else to install
  rsui-tsx config [--platform android] [app]          prints what its rsui.config.ts says, as JSON, or what's wrong with it
  rsui-tsx --react-compiler --pack out.exe app.tsx   compiles project React code at build time (requires Node on the build machine)
    --pack-diagnostics                            enables environment-driven test steps and live inspection in the artifact
    --react-compiler-root DIR                      source root (repeatable; defaults to nearest project folder)
    --react-compiler-incompatible-api PKG:EXPORT    additional API bailout (repeatable)
                                      (--no-code-cache leaves out what its files compiled to, which it then compiles)

Steps end at a new line or a `;`. {}
",
        rsui_tsx::STEPS
    )
}

/// How long `--profile` runs for, in milliseconds, from `5s`, `500ms` or a number of seconds.
fn profile_time(given: &str) -> Option<u64> {
    if let Some(ms) = given.strip_suffix("ms") {
        return ms.parse().ok();
    }
    let seconds: f64 = given.strip_suffix('s').unwrap_or(given).parse().ok()?;
    (seconds > 0.0).then_some((seconds * 1000.0) as u64)
}

/// The steps as they're given: right there, in a file after `@`, or from the standard input for `-`.
fn read_steps(given: &str) -> Result<String, String> {
    if given == "-" {
        return std::io::read_to_string(std::io::stdin())
            .map_err(|error| format!("the steps couldn't be read: {error}"));
    }
    match given.strip_prefix('@') {
        Some(file) => std::fs::read_to_string(file).map_err(|error| format!("{file}: {error}")),
        None => Ok(given.to_owned()),
    }
}

/// `rsui-tsx config [--platform P] [APP]`: prints the app's `rsui.config.ts` as JSON, with what it leaves out filled in, for
/// scripts that can't run TypeScript, or says what's wrong with it.
fn print_config(args: impl Iterator<Item = String>) -> ExitCode {
    let mut platform = None;
    let mut app = None;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--platform" => platform = args.next(),
            // JSON is all it prints, so this is only for saying so.
            "--json" => {}
            _ if arg.starts_with('-') => {
                eprintln!("`rsui-tsx config` takes --platform windows|android|macos|linux and the app, not {arg}");
                return ExitCode::FAILURE;
            }
            _ => app = Some(arg),
        }
    }
    let app = app.unwrap_or_else(|| ".".to_owned());
    match rsui_tsx::app_config(app.as_ref(), platform.as_deref()) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn new_app(name: Option<String>) -> ExitCode {
    let Some(name) = name else {
        eprintln!("say what it's called, as in `rsui-tsx new my-app`.");
        return ExitCode::FAILURE;
    };
    match rsui_tsx::new_project(name.as_ref()) {
        Ok(made) => {
            println!("Made {name}, with {} files. Now:\n", made.len());
            println!("  cd {name}");
            println!("  rsui-tsx app.tsx\n");
            println!(
                "AGENTS.md says how to check your work, with no window. Agents read it for themselves."
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    #[cfg(windows)]
    if let Some(status) = rsui_tsx::crash_report::reporter_main() {
        return status;
    }
    rsui::mark("start");
    #[cfg(feature = "alloc-owners")]
    if std::env::var_os("RSUI_ALLOC_SITES").is_some() {
        // Where a sample of blocks are made, for `census` in steps to say.
        rsui::alloc_owners::start_sites();
    }
    let mut file = None;
    let mut watch = true;
    let mut png = None;
    let mut pack = None;
    let mut pack_diagnostics = false;
    let mut react_compiler = false;
    let mut compiler_options = rsui_tsx::ReactCompilerOptions::default();
    let mut icon = None;
    let mut code_cache = true;
    let mut steps = None;
    let mut agent = false;
    // `--view` alone looks at an app while it runs, and with a file shows a trace that was saved.
    let mut view: Option<Option<String>> = None;
    let mut replay = None;
    let mut replay_to = None;
    let mut replay_pause = false;
    let packed = rsui_tsx::packed_app();
    if packed.is_some() && rsui_tsx::roll_back_an_unconfirmed_update() {
        // The version before it is running instead.
        return ExitCode::SUCCESS;
    }
    let mut args = std::env::args().skip(1).peekable();
    if packed.is_none() && args.peek().is_some_and(|arg| arg == "new") {
        return new_app(args.nth(1));
    }
    if packed.is_none() && args.peek().is_some_and(|arg| arg == "config") {
        args.next();
        return print_config(args);
    }
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--png" => png = args.next(),
            // What's on a packed app's command line is its own, and not for here.
            _ if packed.is_some() => {}
            "--once" => watch = false,
            "--pack" => pack = args.next(),
            "--pack-diagnostics" => pack_diagnostics = true,
            "--react-compiler" => react_compiler = true,
            "--react-compiler-root" => {
                let Some(root) = args.next().filter(|value| !value.starts_with('-')) else {
                    eprintln!("--react-compiler-root requires a directory");
                    return ExitCode::FAILURE;
                };
                compiler_options.roots.push(root.into());
            }
            "--react-compiler-incompatible-api" => {
                let Some(api) = args.next().filter(|value| !value.starts_with('-')) else {
                    eprintln!("--react-compiler-incompatible-api requires PACKAGE:EXPORT");
                    return ExitCode::FAILURE;
                };
                compiler_options.incompatible_apis.push(api);
            }
            "--icon" => icon = args.next(),
            "--no-code-cache" => code_cache = false,
            "--do" => steps = args.next(),
            "--view" => view = Some(args.next_if(|next| !next.starts_with('-'))),
            "--agent" => agent = true,
            "--replay" => replay = args.next(),
            "--to" => {
                let Some(to) = args.next().and_then(|to| to.parse::<usize>().ok()) else {
                    eprintln!(
                        "say which input to replay up to, as in `--to 12`. The viewer's Replay tab numbers them."
                    );
                    return ExitCode::FAILURE;
                };
                replay_to = Some(to);
            }
            "--pause" => replay_pause = true,
            "--profile" => {
                let Some(time) = args.next().and_then(|time| profile_time(&time)) else {
                    eprintln!("say for how long, as in `rsui-tsx --profile 5s app.tsx`.");
                    return ExitCode::FAILURE;
                };
                steps = Some(format!("wait {time}; profile"));
            }
            "--help" | "-h" => {
                print!("{}", help());
                return ExitCode::SUCCESS;
            }
            "--version" | "-V" => {
                println!("rsui-tsx {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ if arg.starts_with('-') && arg != "-" => {
                eprintln!("there's no {arg}. `rsui-tsx --help` says what there is.");
                return ExitCode::FAILURE;
            }
            _ => file = Some(arg),
        }
    }
    if pack_diagnostics && pack.is_none() {
        eprintln!("--pack-diagnostics requires --pack OUT ENTRY");
        return ExitCode::FAILURE;
    }
    if rsui_tsx::packed_diagnostics() {
        steps = std::env::var("RSUI_PACK_STEPS").ok();
    }
    if (react_compiler
        || !compiler_options.roots.is_empty()
        || !compiler_options.incompatible_apis.is_empty())
        && (!react_compiler
            || pack.is_none()
            || png.is_some()
            || steps.is_some()
            || agent
            || view.is_some()
            || replay.is_some())
    {
        eprintln!(
            "--react-compiler and its options are build-only: use --react-compiler --pack OUT ENTRY; ordinary HMR is unchanged"
        );
        return ExitCode::FAILURE;
    }
    if let Some(trace) = replay {
        if let Err(error) = rsui_tsx::ask_for_replay(trace.as_ref(), replay_to, replay_pause) {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    } else if replay_to.is_some() || replay_pause {
        eprintln!(
            "--to and --pause are for --replay, as in `rsui-tsx --replay trace.rsuitrace --to 12 app.tsx`."
        );
        return ExitCode::FAILURE;
    }
    // A trace is shown by an app of its own, which reads it from where this says.
    if let Some(what) = view {
        let viewer = match what {
            Some(pid) if pid.parse::<u32>().is_ok() => rsui_tsx::live_viewer(pid.parse().ok()),
            Some(trace) => rsui_tsx::trace_viewer(trace.as_ref()),
            None => rsui_tsx::live_viewer(None),
        };
        match viewer {
            Ok(viewer) => file = Some(viewer.to_string_lossy().into_owned()),
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        }
        watch = false;
    }
    let is_packed = packed.is_some();
    let Some(file) = packed
        .map(|file| file.to_string_lossy().into_owned())
        .or(file)
    else {
        eprintln!(
            "say which app to run, as in `rsui-tsx app.tsx`, or make one with `rsui-tsx new my-app`. `rsui-tsx --help` says what else there is."
        );
        return ExitCode::FAILURE;
    };
    // What renders, and why, is kept from the start for these, so that asking why later finds an answer, unless `RSUI_INSPECT=0`
    // says not to, as for measuring what an app takes as it would in a window.
    if (agent || steps.is_some()) && std::env::var_os("RSUI_INSPECT").is_none_or(|said| said != "0")
    {
        rsui::inspect_from_start(true);
    }
    if agent {
        return match rsui_tsx::serve_agent(file.as_ref()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    if let Some(steps) = steps {
        let steps = match read_steps(&steps) {
            Ok(steps) => steps,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        };
        return match rsui_tsx::do_steps(file.as_ref(), &steps) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    let result = match (pack, png) {
        (Some(out), _) => rsui_tsx::pack_with_options(
            file.as_ref(),
            out.as_ref(),
            icon.as_deref().map(AsRef::as_ref),
            &rsui_tsx::PackOptions {
                code_cache,
                compiler: react_compiler.then_some(compiler_options),
                diagnostics: pack_diagnostics,
            },
        ),
        (None, Some(png)) => rsui_tsx::draw_to_file(file.as_ref(), png.as_ref(), (960, 640)),
        (None, None) => {
            let options = rsui_tsx::Options {
                title: None,
                watch: watch && !is_packed,
                write_code_cache: None,
            };
            #[cfg(windows)]
            if is_packed {
                rsui_tsx::crash_report::run_packed(file.as_ref(), options)
            } else {
                rsui_tsx::run(&file, options)
            }
            #[cfg(not(windows))]
            rsui_tsx::run(&file, options)
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            if is_packed {
                rsui_tsx::show_error(&error.to_string());
            }
            ExitCode::FAILURE
        }
    }
}
