//! Saving an app again and again doesn't keep every version of its code. `--ignored --nocapture` runs a longer soak of
//! the examples, which prints what the engine keeps as it goes.

use std::path::Path;
use std::time::Duration;

use rsui_headless::Headless;

/// The bytes of compiled code the engine has after each of `saves` saves of the same file, with the first before any.
fn code_after_saves(file: &str, saves: usize, print: bool) -> Vec<usize> {
    // The examples keep what they're given in files under the user's profile unless told where: these go in this test's own
    // folder, so running the tests never rewrites a real `%APPDATA%\rsui-tasks\tasks.json`.
    static SCRATCH: std::sync::Once = std::sync::Once::new();
    SCRATCH.call_once(|| {
        let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("reload-soak-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        // SAFETY: set once, before any app in this test binary is loaded, and nothing else here touches the environment.
        unsafe {
            std::env::set_var("RSUI_TASKS_FILE", folder.join("tasks.json"));
            std::env::set_var("RSUI_STORAGE_FILE", folder.join("storage.json"));
        }
    });
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    let entry = rsui_tsx::load(&path, 512 * 1024).unwrap();
    rsui_tsx::keep_state_across_reloads();
    let mut ui = Headless::new(rsui_tsx::root, entry);
    ui.settle(Duration::from_secs(5));
    let mut sizes = Vec::new();
    for at in 0..=saves {
        if at > 0 {
            assert!(rsui_tsx::request_reload(&path));
            ui.settle(Duration::from_secs(5));
        }
        let census = rsui_tsx::census();
        let memory = &census.memory;
        sizes.push(memory.code);
        if print && at % (saves / 5).max(1) == 0 {
            println!(
                "{file} saves {at:4}: code {:7} KB, objects {:6} KB, strings {:6} KB, shapes {:5} KB, atoms {:6}, total {:7} KB",
                memory.code / 1024,
                memory.objects / 1024,
                memory.strings / 1024,
                memory.shapes / 1024,
                census.atoms,
                memory.total() / 1024,
            );
        }
    }
    sizes
}

#[test]
fn saving_again_and_again_keeps_one_version_of_the_code() {
    let sizes = code_after_saves("tests/apps/tasks.tsx", 8, false);
    // Each save compiled about 33 KB more of this app before its code was freed.
    let most = sizes.iter().max().unwrap();
    assert!(*most <= sizes[0] + 4096, "{sizes:?}");
}

#[test]
#[ignore = "a longer soak of the examples that prints what the engine keeps, for looking at with --nocapture"]
fn soak_the_examples() {
    code_after_saves("tests/apps/tasks.tsx", 50, true);
    code_after_saves("tests/apps/gallery.tsx", 50, true);
}
