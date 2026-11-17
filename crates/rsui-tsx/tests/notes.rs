//! The notes example, used as a person would, on a folder made for each test.

mod hands;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hands::{Hands, KeyCode, KeyPress};

/// Where it keeps its notes is read from the environment, which belongs to the whole process, so these take turns.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn folder(name: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("notes-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

fn open(folder: &std::path::Path) -> Hands {
    // SAFETY: the tests that read this take turns, and nothing else here touches the environment.
    unsafe {
        std::env::set_var("RSUI_NOTES_FOLDER", folder);
    }
    let example = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/apps/notes.tsx");
    Hands::open(&example, (1100, 700))
}

fn with_turn(run: impl FnOnce()) {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    run();
}

fn saved_to(hands: &mut Hands, file: &std::path::Path, wanted: &str) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        hands.settle();
        if std::fs::read_to_string(file).is_ok_and(|text| text.contains(wanted)) {
            return;
        }
        hands.app.runtime().wait(Duration::from_millis(10));
    }
    panic!("{wanted} never got to {}", file.display());
}

#[test]
fn the_first_time_there_is_a_note_and_a_preview_of_it() {
    with_turn(|| {
        let mut hands = open(&folder("first"));
        hands.settle();
        assert_eq!(
            hands.app.take_title_wanted().as_deref(),
            Some("Welcome - Notes")
        );
        let outline = hands.app.outline();
        assert!(outline.contains(r#"text "Welcome""#), "{outline}");
        assert!(
            outline.contains(r#"text "a note""#),
            "the bold part is its own\n{outline}"
        );
        assert!(outline.contains(r#"text "some code""#), "{outline}");
        assert!(outline.contains("words"), "{outline}");
    });
}

#[test]
fn what_is_typed_shows_in_the_preview_and_gets_saved() {
    with_turn(|| {
        let where_ = folder("typing");
        let mut hands = open(&where_);
        hands.click("add");
        hands.settle();
        // A new note has the keyboard straight away.
        hands.type_text("# Big");
        hands.press(KeyCode::Enter);
        hands.type_text("- one thing");
        assert!(hands.says(r#"text "Big""#), "{}", hands.app.outline());
        assert!(hands.says(r#"text "one thing""#), "{}", hands.app.outline());
        saved_to(&mut hands, &where_.join("Note 1.md"), "one thing");
    });
}

#[test]
fn notes_that_were_saved_are_there_next_time() {
    with_turn(|| {
        let where_ = folder("again");
        std::fs::write(where_.join("Older.md"), "from before").unwrap();
        let mut hands = open(&where_);
        hands.settle();
        assert!(
            hands.says(r#"text "from before""#),
            "{}",
            hands.app.outline()
        );
    });
}

#[test]
fn tabs_open_switch_and_close() {
    with_turn(|| {
        let where_ = folder("tabs");
        std::fs::write(where_.join("One.md"), "first").unwrap();
        std::fs::write(where_.join("Two.md"), "second").unwrap();
        let mut hands = open(&where_);
        hands.settle();
        assert!(hands.has("tab-open-One"));
        assert!(!hands.has("tab-open-Two"));
        hands.click("note-Two");
        assert!(hands.has("tab-open-Two"), "{}", hands.app.outline());
        assert!(hands.says(r#"value="second""#), "{}", hands.app.outline());
        hands.click("tab-open-One");
        assert!(hands.says(r#"value="first""#), "{}", hands.app.outline());
        hands.click("tab-close-One");
        assert!(!hands.has("tab-open-One"), "{}", hands.app.outline());
        assert!(
            hands.says(r#"value="second""#),
            "the one that's left shows\n{}",
            hands.app.outline()
        );
        hands.click("tab-close-Two");
        assert!(hands.says("Choose a note"), "{}", hands.app.outline());
    });
}

#[test]
fn dragging_the_line_between_the_panes_resizes_them() {
    with_turn(|| {
        let mut hands = open(&folder("drag"));
        let width = |hands: &mut Hands| {
            hands.settle();
            let mut path = Vec::new();
            assert!(hands::path_of(
                hands.app.runtime().root(),
                "editor",
                &mut path
            ));
            hands.app.layout().rect(&path).unwrap().width
        };
        let before = width(&mut hands);
        let (x, y) = hands.middle_of("splitter");
        hands.app.pointer_move(x, y);
        hands.app.pointer_down(x, y, 1);
        hands.settle();
        // It follows each step, and the line moves away under the pointer while this happens.
        for step in 1..=10 {
            hands.app.pointer_move(x + step as f32 * 10.0, y);
            hands.settle();
        }
        hands.app.pointer_up(x + 100.0, y).unwrap();
        hands.settle();
        assert_eq!(width(&mut hands), before + 100.0);
        // Moving afterwards doesn't.
        hands.app.pointer_move(x + 300.0, y);
        assert_eq!(width(&mut hands), before + 100.0);
    });
}

#[test]
fn a_right_click_opens_a_menu_where_the_pointer_is_and_it_does_what_it_says() {
    with_turn(|| {
        let where_ = folder("menu");
        std::fs::write(where_.join("One.md"), "first").unwrap();
        std::fs::write(where_.join("Two.md"), "second").unwrap();
        let mut hands = open(&where_);
        let (x, y) = hands.middle_of("note-Two");
        hands.app.secondary_click(x, y).unwrap();
        hands.settle();
        assert!(hands.has("note-menu"), "{}", hands.app.outline());
        let (menu_x, menu_y) = hands.middle_of("note-menu");
        assert!(
            menu_x > x && menu_y > y,
            "it's below and to the right of ({x}, {y}), at ({menu_x}, {menu_y})"
        );
        hands.click("menu-copy");
        assert!(!hands.has("note-menu"), "choosing something closes it");
        // Clicking away closes it too.
        hands.app.secondary_click(x, y).unwrap();
        hands.settle();
        hands.press(KeyCode::Escape);
        assert!(!hands.has("note-menu"), "{}", hands.app.outline());
        // And remove means it.
        hands.app.secondary_click(x, y).unwrap();
        hands.settle();
        hands.click("menu-remove");
        assert!(!hands.has("note-Two"), "{}", hands.app.outline());
        let gone = where_.join("Two.md");
        let started = Instant::now();
        while gone.exists() && started.elapsed() < Duration::from_secs(5) {
            hands.settle();
            hands.app.runtime().wait(Duration::from_millis(10));
        }
        assert!(!gone.exists());
    });
}

#[test]
fn renaming_asks_for_a_name_with_the_keyboard_ready() {
    with_turn(|| {
        let where_ = folder("rename");
        std::fs::write(where_.join("Old.md"), "the words").unwrap();
        let mut hands = open(&where_);
        let (x, y) = hands.middle_of("note-Old");
        hands.app.secondary_click(x, y).unwrap();
        hands.settle();
        hands.click("menu-rename");
        hands.settle();
        // What's there is selected, so typing replaces it.
        hands.type_text("New");
        assert!(hands.says(r#"value="New""#), "{}", hands.app.outline());
        hands.press(KeyCode::Enter);
        hands.settle();
        assert!(hands.has("note-New"), "{}", hands.app.outline());
        assert!(!hands.has("note-Old"), "{}", hands.app.outline());
        assert_eq!(
            hands.app.take_title_wanted().as_deref(),
            Some("New - Notes")
        );
        saved_to(&mut hands, &where_.join("New.md"), "the words");
        assert!(!where_.join("Old.md").exists());
    });
}

#[test]
fn a_note_of_a_megabyte_stays_quick_to_type_in() {
    with_turn(|| {
        let where_ = folder("large");
        let line = "This line is one of a great many, as this note is rather long.\n";
        std::fs::write(where_.join("Long.md"), line.repeat(16_000)).unwrap();
        let mut hands = open(&where_);
        hands.click("editor");
        hands.settle();
        hands.app.key(KeyPress::new(KeyCode::End).ctrl()).unwrap();
        let started = Instant::now();
        hands.type_text("abc");
        let each = started.elapsed() / 3;
        assert!(hands.says("words"), "{}", hands.app.outline());
        // A debug build's a lot slower than a real one, so this is only about not being hopeless.
        assert!(each < Duration::from_secs(2), "each key took {each:?}");
    });
}

#[test]
fn shortcuts_work_from_anywhere() {
    with_turn(|| {
        let mut hands = open(&folder("shortcuts"));
        hands.press_with_ctrl('n');
        assert!(hands.has("note-Note 1"), "{}", hands.app.outline());
        hands.type_text("typed");
        assert!(
            hands.says(r#"value="typed""#),
            "it went to the new note\n{}",
            hands.app.outline()
        );
        hands.press_with_ctrl('f');
        hands.type_text("zzz");
        assert!(
            hands.says("Nothing matches that."),
            "{}",
            hands.app.outline()
        );
        hands.press_with_ctrl('w');
        assert!(!hands.has("tab-open-Note 1"), "{}", hands.app.outline());
    });
}

#[test]
fn files_dragged_in_become_notes_and_other_kinds_are_left_alone() {
    with_turn(|| {
        let where_ = folder("dropped");
        std::fs::write(where_.join("Here.md"), "was here").unwrap();
        let elsewhere = folder("dropped-from");
        let file = elsewhere.join("From elsewhere.txt");
        let other = elsewhere.join("picture.png");
        std::fs::write(&file, "came from elsewhere").unwrap();
        std::fs::write(&other, "not text").unwrap();
        let mut hands = open(&where_);
        hands.settle();
        assert!(hands.app.accepts_files());
        let paths = [
            file.to_string_lossy().into_owned(),
            other.to_string_lossy().into_owned(),
        ];
        hands.app.files_dropped(300.0, 300.0, &paths).unwrap();
        hands.wait_for(r#"value="came from elsewhere""#);
        assert!(hands.has("note-From elsewhere"), "{}", hands.app.outline());
        assert!(!hands.has("note-picture"), "{}", hands.app.outline());
    });
}
