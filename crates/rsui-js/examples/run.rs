//! Runs a JavaScript or TypeScript file: `cargo run -p rsui-js --example run -- file.tsx`. With `--expose-gc` before the
//! file, there's a `gc()` that frees everything nothing can reach, as in Node.

use std::process::ExitCode;

#[global_allocator]
static ALLOCATOR: rsui_alloc::UiAlloc = rsui_alloc::UiAlloc;

const STACK: usize = 64 * 1024 * 1024;

fn run(path: &str, expose_gc: bool) -> ExitCode {
    let mut engine = rsui_js::Engine::new();
    engine.set_stack_limit(STACK / 8 * 7);
    if expose_gc {
        let vm = engine.vm();
        let gc = vm.host_function("gc", 0, |vm, _| {
            vm.collect_garbage_now();
            Ok(rsui_js::Value::UNDEFINED)
        });
        vm.set_global("gc", gc);
    }
    for (key, value) in std::env::vars() {
        engine.set_env(&key, &value);
    }
    let finished = engine
        .run_file(std::path::Path::new(path))
        .and_then(|_| engine.run_until_idle());
    if std::env::var_os("RSUI_JS_MEMORY").is_some() {
        let memory = engine.memory();
        eprintln!(
            "memory: {} bytes in all ({memory:?}), {} collections",
            memory.total(),
            engine.collections()
        );
    }
    match finished {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Uncaught {error}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let expose_gc = args.first().is_some_and(|arg| arg == "--expose-gc");
    if expose_gc {
        args.remove(0);
    }
    let Some(path) = args.into_iter().next() else {
        eprintln!("usage: run <file>");
        return ExitCode::FAILURE;
    };
    // A big stack only takes memory as it's used, and code can then nest as deeply as anyone would write it.
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || run(&path, expose_gc))
        .expect("the system has room for a thread")
        .join()
        .unwrap_or(ExitCode::FAILURE)
}
