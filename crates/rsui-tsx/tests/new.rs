//! Starting from nothing, the way the README says to.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn folder(name: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("new-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

fn tool(args: &[&str]) -> Output {
    Command::new(TOOL).args(args).output().unwrap()
}

fn said(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn complained(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn make(place: &Path) -> Output {
    let output = tool(&["new", place.to_str().unwrap()]);
    assert!(output.status.success(), "{}", complained(&output));
    output
}

#[test]
fn a_new_app_works_as_it_comes() {
    let place = folder("works").join("My App");
    let output = make(&place);
    let told = said(&output);
    assert!(
        told.contains("rsui-tsx app.tsx"),
        "it says what's next: {told}"
    );
    let app = place.join("app.tsx");
    let used = tool(&[
        "--do",
        "expect \"My App\"; expect \"1 left\"; type #draft \"Buy milk\"; click #add; expect \"Buy milk\"; expect \"2 left\"; expect no \"Buy milk\\\" #draft\"; click #task-1; expect \"1 left\"",
        app.to_str().unwrap(),
    ]);
    assert!(
        used.status.success(),
        "{}\n{}",
        said(&used),
        complained(&used)
    );
    let picture = place.join("out.png");
    let drawn = tool(&["--png", picture.to_str().unwrap(), app.to_str().unwrap()]);
    assert!(drawn.status.success(), "{}", complained(&drawn));
    assert!(picture.exists());
}

#[test]
fn what_is_in_the_instructions_works() {
    let place = folder("instructions").join("app");
    make(&place);
    let guide = std::fs::read_to_string(place.join("AGENTS.md")).unwrap();
    // The steps it gives as an example of a test.
    let at = guide.find("// Adding a task.").expect("it has an example");
    let steps = &guide[at..];
    let steps = &steps[..steps.find("```").unwrap()];
    // Its picture goes in a place of its own, which takes quotes, as a path can have anything in it.
    let picture = place
        .join("after.png")
        .to_str()
        .unwrap()
        .replace('\\', "\\\\");
    let file = place.join("steps.txt");
    std::fs::write(&file, steps.replace("after.png", &format!("\"{picture}\""))).unwrap();
    let used = tool(&[
        "--do",
        &format!("@{}", file.display()),
        place.join("app.tsx").to_str().unwrap(),
    ]);
    assert!(
        used.status.success(),
        "{}\n{}",
        said(&used),
        complained(&used)
    );
    assert!(place.join("after.png").exists());
    assert!(guide.contains("#add"), "it talks about what the app has");
}

#[test]
fn everything_an_editor_and_a_client_read_is_as_it_should_be() {
    let place = folder("files").join("Don't Panic!");
    make(&place);
    for file in ["tsconfig.json", ".mcp.json"] {
        let text = std::fs::read_to_string(place.join(file)).unwrap();
        let _: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|error| panic!("{file}: {error}\n{text}"));
    }
    let mcp = std::fs::read_to_string(place.join(".mcp.json")).unwrap();
    assert!(mcp.contains("\"don-t-panic\""), "{mcp}");
    assert!(mcp.contains("--agent"), "{mcp}");
    assert_eq!(
        std::fs::read_to_string(place.join("rsui.d.ts")).unwrap(),
        include_str!("../rsui.d.ts")
    );
    assert_eq!(
        std::fs::read_to_string(place.join("CLAUDE.md")).unwrap(),
        "@AGENTS.md\n"
    );
    // A title with a quote in it still has to be code that runs.
    let app = place.join("app.tsx");
    let used = tool(&["--do", "expect \"Don't Panic!\"", app.to_str().unwrap()]);
    assert!(
        used.status.success(),
        "{}\n{}",
        said(&used),
        complained(&used)
    );
}

#[test]
fn nothing_that_is_already_there_gets_changed() {
    let place = folder("taken");
    std::fs::write(place.join("mine.txt"), "keep this").unwrap();
    let output = tool(&["new", place.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(
        complained(&output).contains("already has things in it"),
        "{}",
        complained(&output)
    );
    assert_eq!(
        std::fs::read_to_string(place.join("mine.txt")).unwrap(),
        "keep this"
    );
    assert!(!place.join("app.tsx").exists());
}

#[test]
fn an_empty_folder_can_be_used() {
    let place = folder("empty");
    make(&place);
    assert!(place.join("app.tsx").exists());
}

#[test]
fn a_new_app_needs_a_name() {
    let output = tool(&["new"]);
    assert!(!output.status.success());
    assert!(
        complained(&output).contains("new my-app"),
        "{}",
        complained(&output)
    );
}

#[test]
fn help_says_what_there_is_and_a_mistake_says_to_look_there() {
    let output = tool(&["--help"]);
    assert!(output.status.success());
    let help = said(&output);
    for part in [
        "new",
        "--once",
        "--png",
        "--do",
        "--agent",
        "--pack",
        "click TARGET",
        "@path",
    ] {
        assert!(help.contains(part), "{part} isn't in {help}");
    }
    assert!(said(&tool(&["-h"])).contains("--agent"));
    let version = tool(&["--version"]);
    assert!(
        said(&version).starts_with("rsui-tsx "),
        "{}",
        said(&version)
    );
    let wrong = tool(&["--nope", "app.tsx"]);
    assert!(!wrong.status.success());
    assert!(
        complained(&wrong).contains("--nope"),
        "{}",
        complained(&wrong)
    );
    assert!(
        complained(&wrong).contains("--help"),
        "{}",
        complained(&wrong)
    );
    let nothing = tool(&[]);
    assert!(!nothing.status.success());
    assert!(
        complained(&nothing).contains("--help"),
        "{}",
        complained(&nothing)
    );
}
