//! The expenses example, used as a person would, on a file made for each test.

mod hands;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hands::Hands;

/// Where it keeps things is read from the environment, which belongs to the whole process, so these take turns.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn file(name: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("expenses-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder.join("expenses.json")
}

fn open(file: &std::path::Path) -> Hands {
    // SAFETY: the tests that read this take turns, and nothing else here touches the environment.
    unsafe {
        std::env::set_var("RSUI_EXPENSES_FILE", file);
    }
    let example = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/apps/expenses.tsx");
    Hands::open(&example, (1120, 720))
}

fn with_turn(run: impl FnOnce()) {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    run();
}

fn saved(when: &str) -> String {
    format!(
        r#"[{}]"#,
        when.lines()
            .enumerate()
            .map(|(id, line)| {
                let mut parts = line.split('|');
                format!(
                    r#"{{"id":{},"date":"{}","description":"{}","category":"{}","amount":{}}}"#,
                    id + 1,
                    parts.next().unwrap(),
                    parts.next().unwrap(),
                    parts.next().unwrap(),
                    parts.next().unwrap(),
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn type_in(hands: &mut Hands, key: &str, text: &str) {
    hands.click(key);
    hands.type_text(text);
}

const THREE: &str =
    "2026-01-05|Bread|Food|3.5\n2026-02-01|Rent|Home|800\n2026-02-10|A train|Travel|40.25";

#[test]
fn the_first_time_there_is_something_to_look_at() {
    with_turn(|| {
        let mut hands = open(&file("first"));
        hands.settle();
        let outline = hands.app.outline();
        assert!(outline.contains("Groceries"), "{outline}");
        assert!(hands.has("donut"), "{outline}");
        assert!(hands.has("trend"), "{outline}");
    });
}

#[test]
fn what_is_added_shows_everywhere_and_gets_saved() {
    with_turn(|| {
        let where_ = file("adding");
        std::fs::write(&where_, saved(THREE)).unwrap();
        let mut hands = open(&where_);
        hands.settle();
        assert!(hands.says(r#"text "843.75""#), "{}", hands.app.outline());
        type_in(&mut hands, "description", "Coffee, black");
        type_in(&mut hands, "amount", "1234.5");
        hands.click("add");
        assert!(
            hands.says(r#"text "2,078.25""#),
            "commas go in\n{}",
            hands.app.outline()
        );
        assert!(hands.says(r#"text "1,234.50""#), "{}", hands.app.outline());
        assert!(
            hands.says(r#"text "Coffee, black""#),
            "{}",
            hands.app.outline()
        );
        assert!(hands.says(r#"text "4""#), "{}", hands.app.outline());
        assert!(
            hands.says(r#"value="""#),
            "the form's clear\n{}",
            hands.app.outline()
        );
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(10) {
            hands.settle();
            if std::fs::read_to_string(&where_).is_ok_and(|text| text.contains("Coffee")) {
                break;
            }
            hands.app.runtime().wait(Duration::from_millis(20));
        }
        let text = std::fs::read_to_string(&where_).unwrap();
        assert!(text.contains(r#""amount":1234.5"#), "{text}");
    });
}

#[test]
fn what_is_wrong_with_the_form_says_so_and_adds_nothing() {
    with_turn(|| {
        let where_ = file("wrong");
        std::fs::write(&where_, saved(THREE)).unwrap();
        let mut hands = open(&where_);
        hands.click("add");
        assert!(
            hands.says("Say what it was for."),
            "{}",
            hands.app.outline()
        );
        type_in(&mut hands, "description", "Something");
        type_in(&mut hands, "amount", "nope");
        hands.click("add");
        assert!(hands.says("has to be a number"), "{}", hands.app.outline());
        assert!(hands.says(r#"text "843.75""#), "{}", hands.app.outline());
    });
}

fn rows_in_order(hands: &Hands) -> Vec<String> {
    hands
        .app
        .outline()
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("text \"")?;
            line.contains("#what-")
                .then(|| rest.split('"').next().unwrap().to_owned())
        })
        .collect()
}

#[test]
fn clicking_a_heading_sorts_by_it_and_again_turns_it_around() {
    with_turn(|| {
        let where_ = file("sorting");
        std::fs::write(&where_, saved(THREE)).unwrap();
        let mut hands = open(&where_);
        hands.settle();
        assert_eq!(
            rows_in_order(&hands),
            ["A train", "Rent", "Bread"],
            "newest first"
        );
        hands.click("sort-amount");
        assert_eq!(rows_in_order(&hands), ["Rent", "A train", "Bread"]);
        hands.click("sort-amount");
        assert_eq!(rows_in_order(&hands), ["Bread", "A train", "Rent"]);
        hands.click("sort-description");
        assert_eq!(rows_in_order(&hands), ["A train", "Bread", "Rent"]);
    });
}

#[test]
fn finding_and_choosing_a_kind_narrow_it_down() {
    with_turn(|| {
        let where_ = file("finding");
        std::fs::write(&where_, saved(THREE)).unwrap();
        let mut hands = open(&where_);
        type_in(&mut hands, "search", "TRA");
        assert_eq!(rows_in_order(&hands), ["A train"]);
        hands.click("search");
        for _ in 0..3 {
            hands.press(hands::KeyCode::Backspace);
        }
        hands.click("legend-Home");
        assert_eq!(rows_in_order(&hands), ["Rent"], "{}", hands.app.outline());
        hands.click("legend-Home");
        assert_eq!(rows_in_order(&hands).len(), 3, "and back");
    });
}

#[test]
fn removing_takes_it_out_of_everything() {
    with_turn(|| {
        let where_ = file("removing");
        std::fs::write(&where_, saved(THREE)).unwrap();
        let mut hands = open(&where_);
        hands.settle();
        hands.click("remove-2");
        assert_eq!(
            rows_in_order(&hands),
            ["A train", "Bread"],
            "{}",
            hands.app.outline()
        );
        assert!(hands.says(r#"text "43.75""#), "{}", hands.app.outline());
    });
}

#[test]
fn a_file_dragged_in_is_added_with_what_is_awkward_in_it_read_properly() {
    with_turn(|| {
        let where_ = file("dragging");
        std::fs::write(&where_, "[]").unwrap();
        let csv = where_.with_file_name("from elsewhere.csv");
        std::fs::write(
            &csv,
            "date,description,category,amount\r\n2026-03-01,\"Dinner, with \"\"friends\"\"\",Food,50.00\r\nnot a row\r\n2026-03-02,Unknown kind,Nonsense,5\r\n",
        )
        .unwrap();
        let mut hands = open(&where_);
        hands.settle();
        assert!(hands.app.accepts_files());
        let paths = [csv.to_string_lossy().into_owned()];
        hands.app.files_dropped(500.0, 300.0, &paths).unwrap();
        hands.wait_for("Added 2 from from elsewhere.csv");
        assert!(hands.says("Dinner, with"), "{}", hands.app.outline());
        assert!(hands.says(r#"text "55.00""#), "{}", hands.app.outline());
    });
}

#[test]
fn colors_follow_the_system() {
    with_turn(|| {
        let where_ = file("colors");
        std::fs::write(&where_, saved(THREE)).unwrap();
        let mut hands = open(&where_);
        hands.settle();
        hands.app.set_dark_mode(true);
        hands.settle();
        assert_eq!(hands.app.take_dark_title_bar_wanted(), Some(true));

        assert!(
            // The chart's lines are in the dark theme's color.
            hands.says("#30363d"),
            "{}",
            hands.app.outline()
        );
        // A ready-made widget goes dark too, though nothing about it has changed but that.
        let background = |hands: &Hands| {
            let mut path = Vec::new();
            assert!(hands::path_of(
                hands.app.runtime().root(),
                "category-button",
                &mut path
            ));
            let mut node = hands.app.runtime().root();
            for index in path {
                node = &node.children[index as usize];
            }
            node.props.get("background").cloned()
        };
        let dark = background(&hands);
        hands.app.set_dark_mode(false);
        hands.settle();
        let light = background(&hands);
        assert_ne!(dark, light);
    });
}

#[test]
fn ten_thousand_rows_stay_quick() {
    with_turn(|| {
        let where_ = file("many");
        let many: Vec<String> = (0..10_000)
            .map(|at| {
                format!(
                    "2026-{:02}-{:02}|Thing {at}|{}|{}",
                    at % 12 + 1,
                    at % 28 + 1,
                    ["Food", "Home", "Fun"][at % 3],
                    at % 100
                )
            })
            .collect();
        std::fs::write(&where_, saved(&many.join("\n"))).unwrap();
        let mut hands = open(&where_);
        hands.settle();
        let outline = hands.app.outline();
        let rows = outline.matches("#what-").count();
        assert!(rows < 100, "only what shows is built: {rows}");
        let started = Instant::now();
        hands.click("sort-amount");
        // A debug build's a lot slower than a real one, so this is only about not being hopeless.
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    });
}
