//! The tasks example, used the way a person would use it, with no window. Each test has a file of its own to save to.

mod hands;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use rsui_headless::{Headless, Query};

const STACK_LIMIT: usize = 512 * 1024;

/// Where it saves is read from the environment, which belongs to the whole process, so these take turns.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/apps/tasks.tsx")
}

fn data_file(name: &str) -> PathBuf {
    let folder =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tasks-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join(format!("{name}.json"));
    let _ = std::fs::remove_file(&file);
    file
}

fn start(file: &std::path::Path) -> Headless {
    // SAFETY: tests that read this hold the lock above, and nothing else here touches the environment.
    unsafe {
        std::env::set_var("RSUI_TASKS_FILE", file);
    }
    let entry = rsui_tsx::load(&example(), STACK_LIMIT).unwrap();
    Headless::new(rsui_tsx::root, entry)
}

fn key(name: &str) -> Query {
    Query::new().key(name)
}

fn add_task(ui: &mut Headless, text: &str) {
    ui.set_value(&key("draft"), text).unwrap();
    ui.click(&key("add")).unwrap();
}

/// Long enough for a change to be saved.
fn wait_for_save(ui: &mut Headless, file: &std::path::Path, contains: &str) {
    for _ in 0..50 {
        ui.settle(Duration::from_millis(50));
        if std::fs::read_to_string(file).is_ok_and(|text| text.contains(contains)) {
            return;
        }
    }
    panic!("{contains} was never saved");
}

#[test]
fn it_starts_with_something_to_look_at() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let ui = start(&data_file("first"));
    let outline = ui.outline();
    assert!(outline.contains("Try adding a task"), "{outline}");
    assert!(outline.contains("3 left, 0 done"), "{outline}");
}

#[test]
fn adding_and_ticking_off_change_the_counts() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let mut ui = start(&data_file("counts"));
    add_task(&mut ui, "Buy milk");
    assert!(ui.outline().contains("Buy milk"), "{}", ui.outline());
    assert!(ui.outline().contains("4 left, 0 done"), "{}", ui.outline());
    assert!(
        ui.find(&key("draft"))
            .unwrap()
            .line()
            .contains(r#"value="""#)
            || !ui.find(&key("draft")).unwrap().line().contains("Buy milk")
    );

    ui.toggle(&key("done-4")).unwrap();
    assert!(ui.outline().contains("3 left, 1 done"), "{}", ui.outline());
    ui.toggle(&key("done-4")).unwrap();
    assert!(ui.outline().contains("4 left, 0 done"), "{}", ui.outline());
}

#[test]
fn nothing_is_added_for_a_blank_task() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let mut ui = start(&data_file("blank"));
    add_task(&mut ui, "   ");
    assert!(ui.outline().contains("3 left, 0 done"), "{}", ui.outline());
}

#[test]
fn projects_show_what_is_in_them_and_new_tasks_go_there() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let mut ui = start(&data_file("projects"));
    ui.click(&key("project-Home")).unwrap();
    assert!(ui.outline().contains("Plan the week"), "{}", ui.outline());
    assert!(
        !ui.outline().contains("Try adding a task"),
        "{}",
        ui.outline()
    );
    add_task(&mut ui, "Fix the door");
    assert!(ui.outline().contains("Fix the door"), "{}", ui.outline());

    ui.click(&key("project-Everything")).unwrap();
    assert!(
        ui.outline().contains("Try adding a task"),
        "{}",
        ui.outline()
    );
    assert!(ui.outline().contains("Fix the door"), "{}", ui.outline());

    ui.click(&key("project-Work")).unwrap();
    assert!(
        ui.outline().contains("Nothing here yet."),
        "{}",
        ui.outline()
    );
}

#[test]
fn a_project_can_be_added_and_removed_and_its_tasks_are_kept() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let mut ui = start(&data_file("remove-project"));
    ui.click(&key("project-Home")).unwrap();
    ui.click(&key("project-menu-button")).unwrap();
    ui.click(&key("project-menu-item-0")).unwrap();
    assert!(ui.find(&key("project-Home")).is_err(), "{}", ui.outline());
    assert!(ui.outline().contains("Plan the week"), "{}", ui.outline());
}

#[test]
fn searching_finds_tasks_by_their_text_and_notes() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let mut ui = start(&data_file("search"));
    ui.set_value(&key("search"), "PLAN").unwrap();
    assert!(ui.outline().contains("Plan the week"), "{}", ui.outline());
    assert!(
        !ui.outline().contains("Try adding a task"),
        "{}",
        ui.outline()
    );
    ui.set_value(&key("search"), "notes go").unwrap();
    assert!(
        ui.outline().contains("Tick this one off"),
        "{}",
        ui.outline()
    );
    ui.set_value(&key("search"), "nothing like this").unwrap();
    assert!(
        ui.outline().contains("Nothing matches that."),
        "{}",
        ui.outline()
    );
    ui.set_value(&key("search"), "").unwrap();
    assert!(
        ui.outline().contains("Try adding a task"),
        "{}",
        ui.outline()
    );
}

#[test]
fn sorting_keeps_what_is_in_each_row() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let mut ui = start(&data_file("sort"));
    ui.toggle(&key("done-1")).unwrap();
    ui.click(&key("sort-button")).unwrap();
    ui.click(&key("sort-item-2")).unwrap();
    let outline = ui.outline();
    let place = |text: &str| {
        outline
            .find(text)
            .unwrap_or_else(|| panic!("{text}\n{outline}"))
    };
    assert!(
        place("Plan the week") < place("Tick this one off"),
        "{outline}"
    );
    assert!(
        place("Tick this one off") < place("Try adding a task"),
        "{outline}"
    );
    assert!(outline.contains("1 done"), "{outline}");
}

#[test]
fn details_edit_the_task_that_is_chosen() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let mut ui = start(&data_file("details"));
    assert!(ui.outline().contains("Choose a task"), "{}", ui.outline());
    ui.click(&key("task-1")).unwrap();
    ui.set_value(&key("title-1"), "A better name").unwrap();
    assert!(
        ui.outline().contains(r#""A better name""#),
        "{}",
        ui.outline()
    );
    ui.set_value(&key("notes-1"), "Remember this").unwrap();
    assert!(ui.outline().contains("Remember this"), "{}", ui.outline());
    // Something else shows its own, and coming back still has them.
    ui.click(&key("task-2")).unwrap();
    assert!(ui.outline().contains("Notes go here."), "{}", ui.outline());
    ui.click(&key("task-1")).unwrap();
    assert!(ui.outline().contains("Remember this"), "{}", ui.outline());
}

#[test]
fn removing_can_be_undone_and_the_chance_goes_away() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let mut ui = start(&data_file("undo"));
    ui.click(&key("task-1")).unwrap();
    ui.click(&key("remove-1")).unwrap();
    assert!(ui.find(&key("task-1")).is_err(), "{}", ui.outline());
    assert!(ui.outline().contains("Choose a task"), "{}", ui.outline());
    assert!(ui.find(&key("undo")).is_ok(), "{}", ui.outline());
    ui.click(&key("undo")).unwrap();
    assert!(ui.find(&key("task-1")).is_ok(), "{}", ui.outline());
    assert!(ui.find(&key("undo")).is_err(), "{}", ui.outline());
}

#[test]
fn clearing_what_is_done_asks_first() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let mut ui = start(&data_file("clear"));
    assert!(
        ui.find(&key("clear")).is_err(),
        "there's nothing to clear yet"
    );
    ui.toggle(&key("done-1")).unwrap();
    ui.toggle(&key("done-2")).unwrap();
    ui.click(&key("clear")).unwrap();
    assert!(
        ui.outline().contains("Remove 2 finished tasks?"),
        "{}",
        ui.outline()
    );
    ui.click(&key("confirm-cancel")).unwrap();
    assert!(ui.find(&key("task-1")).is_ok(), "{}", ui.outline());
    ui.click(&key("clear")).unwrap();
    ui.click(&key("confirm-clear")).unwrap();
    assert!(ui.find(&key("task-1")).is_err(), "{}", ui.outline());
    assert!(ui.find(&key("task-2")).is_err(), "{}", ui.outline());
    assert!(ui.find(&key("task-3")).is_ok(), "{}", ui.outline());
}

#[test]
fn what_was_done_is_there_next_time() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let file = data_file("saved");
    let mut ui = start(&file);
    add_task(&mut ui, "Still here later");
    wait_for_save(&mut ui, &file, "Still here later");
    assert!(
        !file.with_extension("json.new").exists(),
        "nothing's left behind"
    );
    drop(ui);

    let handle = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn({
            let file = file.clone();
            move || start(&file).outline()
        })
        .unwrap();
    let outline = handle.join().unwrap();
    assert!(outline.contains("Still here later"), "{outline}");
}

#[test]
fn a_damaged_file_is_not_the_end_of_the_world() {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let file = data_file("damaged");
    std::fs::write(&file, "{ this is not what it should be").unwrap();
    let ui = start(&file);
    assert!(
        ui.outline().contains("Try adding a task"),
        "{}",
        ui.outline()
    );
}

// By hand: real clicks where things are on screen, and keys, as a person's would be.

fn open_tasks(file: &std::path::Path) -> hands::Hands {
    // SAFETY: as in `start`.
    unsafe {
        std::env::set_var("RSUI_TASKS_FILE", file);
    }
    hands::Hands::open(&example(), (960, 640))
}

fn with_turn(run: impl FnOnce()) {
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    run();
}

#[test]
fn typing_a_task_and_pressing_enter_adds_it_and_the_box_is_ready_for_the_next() {
    with_turn(|| {
        let mut hands = open_tasks(&data_file("enter"));
        hands.click("draft");
        hands.type_text("First thing");
        assert!(
            hands.says(r#"value="First thing""#),
            "{}",
            hands.app.outline()
        );
        hands.press(hands::KeyCode::Enter);
        assert!(hands.has("task-4"), "{}", hands.app.outline());
        assert!(hands.says(r#""First thing""#), "{}", hands.app.outline());
        assert!(
            !hands.says(r#"value="First thing""#),
            "the box is empty again\n{}",
            hands.app.outline()
        );
        hands.type_text("Second thing");
        hands.press(hands::KeyCode::Enter);
        assert!(hands.has("task-5"), "{}", hands.app.outline());
        assert!(hands.says(r#""Second thing""#), "{}", hands.app.outline());
    });
}

#[test]
fn clicking_something_in_a_row_does_only_that() {
    with_turn(|| {
        let mut hands = open_tasks(&data_file("in-a-row"));
        hands.click("done-1");
        assert!(hands.says("2 left, 1 done"), "{}", hands.app.outline());
        assert!(
            hands.says("Choose a task"),
            "ticking it didn't choose it\n{}",
            hands.app.outline()
        );
        hands.click("remove-2");
        assert!(!hands.has("task-2"), "{}", hands.app.outline());
        assert!(
            hands.says("Choose a task"),
            "removing it didn't choose it\n{}",
            hands.app.outline()
        );
        hands.click("task-3");
        assert!(hands.has("title-3"), "{}", hands.app.outline());
    });
}

#[test]
fn what_is_typed_in_the_details_shows_in_the_list_as_it_goes() {
    with_turn(|| {
        let mut hands = open_tasks(&data_file("details-by-hand"));
        hands.click("task-1");
        hands.click("title-1");
        hands.type_text("!!");
        assert!(
            hands.says(r#"value="Try adding a task!!""#),
            "{}",
            hands.app.outline()
        );
        assert!(
            hands.says(r#"text "Try adding a task!!""#),
            "{}",
            hands.app.outline()
        );
        hands.press(hands::KeyCode::Backspace);
        assert!(
            hands.says(r#"text "Try adding a task!""#),
            "{}",
            hands.app.outline()
        );
        // The keyboard hasn't gone anywhere, as the value came back from the app.
        hands.type_text("?");
        assert!(
            hands.says(r#"value="Try adding a task!?""#),
            "{}",
            hands.app.outline()
        );
    });
}

#[test]
fn the_keyboard_can_do_all_of_it() {
    with_turn(|| {
        let mut hands = open_tasks(&data_file("keyboard"));
        hands.click("draft");
        hands.press(hands::KeyCode::Tab);
        assert_eq!(
            hands.app.focused().map(<[u32]>::to_vec),
            {
                let mut path = Vec::new();
                assert!(hands::path_of(hands.app.runtime().root(), "add", &mut path));
                Some(path)
            },
            "tab goes to the button after the box"
        );
        // On through the list, which can take the keyboard to scroll, and the first row.
        let mut first_box = Vec::new();
        assert!(hands::path_of(
            hands.app.runtime().root(),
            "done-1",
            &mut first_box
        ));
        for _ in 0..4 {
            if hands.app.focused() == Some(first_box.as_slice()) {
                break;
            }
            hands.press(hands::KeyCode::Tab);
        }
        assert_eq!(hands.app.focused(), Some(first_box.as_slice()));
        hands.press(hands::KeyCode::Space);
        assert!(
            hands.says("2 left, 1 done"),
            "the next thing is the first checkbox\n{}",
            hands.app.outline()
        );
    });
}

#[test]
fn a_dialog_keeps_the_rest_out_and_escape_closes_it() {
    with_turn(|| {
        let mut hands = open_tasks(&data_file("dialog"));
        hands.click("done-1");
        let behind = hands.middle_of("task-3");
        hands.click("clear");
        assert!(hands.has("confirm-clear"), "{}", hands.app.outline());
        hands.app.pointer_move(behind.0, behind.1);
        let _ = hands.app.click(behind.0, behind.1);
        hands.settle();
        assert!(
            !hands.has("title-3"),
            "the row behind it wasn't clicked\n{}",
            hands.app.outline()
        );
        if hands.has("confirm-clear") {
            hands.press(hands::KeyCode::Escape);
        }
        // It fades, and takes no more input from when it starts to.
        assert!(hands.says("2 left, 1 done"), "{}", hands.app.outline());
        hands.click("task-3");
        assert!(
            hands.has("title-3"),
            "the rest works again\n{}",
            hands.app.outline()
        );
    });
}

#[test]
fn after_scrolling_a_click_lands_on_what_is_under_the_pointer() {
    with_turn(|| {
        let file = data_file("scrolling");
        let tasks: Vec<String> = (1..=80)
            .map(|id| format!(r#"{{"id":{id},"text":"Task {id}","done":false,"project":"Inbox","notes":"","added":{id}}}"#))
            .collect();
        std::fs::write(
            &file,
            format!(
                r#"{{"tasks":[{}],"projects":["Inbox"],"nextId":81}}"#,
                tasks.join(",")
            ),
        )
        .unwrap();
        let mut hands = open_tasks(&file);
        let (x, y) = hands.middle_of("tasks");
        let before = hands.middle_of("task-30");
        hands.app.pointer_move(x, y);
        hands.app.scroll(x, y, 30).unwrap();
        hands.settle();
        let after = hands.middle_of("task-30");
        assert!(
            after.1 < before.1 - 500.0,
            "it moved, from {before:?} to {after:?}"
        );
        hands.click_at(after.0, after.1);
        assert!(
            hands.has("title-30"),
            "the row where the click was got it\n{}",
            hands.app.outline()
        );
    });
}

#[test]
fn clicking_away_from_a_menu_closes_it_and_the_click_still_counts() {
    with_turn(|| {
        let mut hands = open_tasks(&data_file("menu"));
        hands.click("sort-button");
        assert!(hands.has("sort-item-0"), "{}", hands.app.outline());
        hands.click("task-3");
        assert!(hands.has("title-3"), "{}", hands.app.outline());
    });
}

#[test]
fn shortcuts_give_the_keyboard_to_what_they_are_for() {
    with_turn(|| {
        let mut hands = open_tasks(&data_file("shortcuts"));
        hands
            .app
            .key(hands::KeyPress::new(hands::KeyCode::Character('n')).ctrl())
            .unwrap();
        hands.settle();
        hands.type_text("From the keyboard");
        assert!(
            hands.says(r#"value="From the keyboard""#),
            "{}",
            hands.app.outline()
        );
        hands.press(hands::KeyCode::Enter);
        assert!(hands.has("task-4"), "{}", hands.app.outline());
        hands
            .app
            .key(hands::KeyPress::new(hands::KeyCode::Character('f')).ctrl())
            .unwrap();
        hands.settle();
        hands.type_text("keyboard");
        assert!(
            hands.says(r#"#search placeholder="Search" value="keyboard""#),
            "{}",
            hands.app.outline()
        );
        assert!(!hands.has("task-1"), "{}", hands.app.outline());
    });
}
