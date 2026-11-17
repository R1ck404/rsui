//! The file browser example, used as a person would, on a folder made for each test. Reading folders happens on other
//! threads, so this waits for what it expects to see.

mod hands;

use std::path::PathBuf;
use std::sync::Mutex;

use hands::{Hands, KeyCode};

/// Where it starts is read from the environment, which belongs to the whole process, so these take turns.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn folder(name: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("files-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(folder.join("inner")).unwrap();
    std::fs::write(folder.join("inner").join("deep.txt"), "from deep down").unwrap();
    std::fs::write(folder.join("notes.txt"), "the words in the notes").unwrap();
    std::fs::write(folder.join("big.bin"), vec![0u8; 3000]).unwrap();
    folder
}

fn open(start: &std::path::Path) -> Hands {
    // SAFETY: the tests that read this take turns, and nothing else here touches the environment.
    unsafe {
        std::env::set_var("RSUI_FILES_START", start);
    }
    let example = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/apps/files.tsx");
    Hands::open(&example, (960, 640))
}

fn with_turn(run: impl FnOnce()) {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    run();
}

#[test]
fn a_folder_shows_up_with_folders_first_and_the_chosen_one_is_previewed() {
    with_turn(|| {
        let mut hands = open(&folder("first"));
        hands.wait_for("3 of 3 items");
        let outline = hands.app.outline();
        let place = |text: &str| {
            outline
                .find(text)
                .unwrap_or_else(|| panic!("{text}\n{outline}"))
        };
        assert!(
            place(r#"list_item "inner""#) < place(r#"list_item "big.bin""#),
            "{outline}"
        );
        assert!(
            place(r#"list_item "big.bin""#) < place(r#"list_item "notes.txt""#),
            "{outline}"
        );
        assert!(outline.contains(r#"text "2.9 KB""#), "{outline}");
    });
}

#[test]
fn the_keyboard_moves_and_opens_and_goes_back() {
    with_turn(|| {
        let mut hands = open(&folder("keys"));
        hands.wait_for("3 of 3 items");
        hands.press(KeyCode::Down);
        hands.press(KeyCode::Down);
        hands.wait_for("the words in the notes");
        hands.press(KeyCode::Up);
        hands.press(KeyCode::Up);
        hands.press(KeyCode::Enter);
        hands.wait_for("1 of 1 items");
        assert!(
            hands.says(r#"list_item "deep.txt""#),
            "{}",
            hands.app.outline()
        );
        hands.wait_for("from deep down");
        hands.press(KeyCode::Backspace);
        hands.wait_for("3 of 3 items");
        assert!(
            hands.says(r#"list_item "inner""#),
            "{}",
            hands.app.outline()
        );
    });
}

#[test]
fn clicking_chooses_and_double_clicking_opens() {
    with_turn(|| {
        let mut hands = open(&folder("clicks"));
        hands.wait_for("3 of 3 items");
        hands.click("row-2");
        hands.wait_for("the words in the notes");
        hands.click("row-0");
        hands.wait_for(r#"text "inner""#);
        let (x, y) = hands.middle_of("row-0");
        hands.app.pointer_down(x, y, 2);
        hands.app.pointer_up(x, y).unwrap();
        hands.wait_for("1 of 1 items");
        hands.click("up");
        hands.wait_for("3 of 3 items");
        hands.click("back");
        hands.wait_for("1 of 1 items");
    });
}

#[test]
fn the_filter_narrows_it_and_typing_there_leaves_the_arrows_for_the_text() {
    with_turn(|| {
        let mut hands = open(&folder("filter"));
        hands.wait_for("3 of 3 items");
        hands.press_with_ctrl('f');
        hands.type_text("note");
        hands.wait_for("1 of 3 items");
        assert!(
            hands.says(r#"list_item "notes.txt""#),
            "{}",
            hands.app.outline()
        );
        // Backspace is for the filter while it has the keyboard, and doesn't go to the last folder.
        hands.press(KeyCode::Backspace);
        assert!(hands.says(r#"value="not""#), "{}", hands.app.outline());
        hands.type_text("hing like this");
        hands.wait_for("Nothing matches that.");
    });
}

#[test]
fn a_folder_that_cannot_be_read_says_so() {
    with_turn(|| {
        let start = folder("gone").join("not there");
        let mut hands = open(&start);
        hands.wait_for("ENOENT");
    });
}

#[test]
fn moving_past_the_end_of_what_shows_keeps_the_chosen_row_in_view() {
    with_turn(|| {
        let start = folder("long");
        for index in 0..2000 {
            std::fs::write(start.join(format!("file{index:04}.txt")), "").unwrap();
        }
        let mut hands = open(&start);
        hands.wait_for("2003 items");
        hands.settle();
        for _ in 0..200 {
            hands.press(KeyCode::Down);
        }
        // Folders first, and then big.bin and the rest in order. Number 200 is a long way off the first screen.
        let chosen = "file0197.txt";
        hands.wait_for(chosen);
        let (_, y) = hands.middle_of("row-200");
        assert!(
            y > 0.0 && y < 640.0,
            "the chosen row is at {y}, out of sight"
        );
        assert!(hands.has("row-200"), "{}", hands.app.outline());
        // A long way up again goes back to the top.
        hands.press(KeyCode::Home);
        assert!(hands.has("row-0"), "{}", hands.app.outline());
        let (_, y) = hands.middle_of("row-0");
        assert!(
            y > 0.0 && y < 640.0,
            "the first row is at {y}, out of sight"
        );
    });
}
