//! Editing a line of text, checked against a simple model of what it should do.

use proptest::prelude::*;
use rsui::edit::{Change, Direction, TextEdit, Unit};

fn commit(edit: &mut TextEdit, value: &mut String, change: Option<Change>) {
    if let Some(change) = change {
        edit.commit(&change);
        *value = change.value.to_string();
    }
}

fn typed(text: &str) -> (TextEdit, String) {
    let mut edit = TextEdit::new();
    let mut value = String::new();
    for character in text.chars() {
        let change = edit.insert(&value, character.encode_utf8(&mut [0; 4]));
        commit(&mut edit, &mut value, change);
    }
    (edit, value)
}

#[test]
fn typing_goes_in_at_the_caret() {
    let (mut edit, mut value) = typed("ac");
    edit.go(&value, Direction::Back, Unit::Character, false);
    let change = edit.insert(&value, "b");
    commit(&mut edit, &mut value, change);
    assert_eq!(value, "abc");
    assert_eq!(edit.caret(), 2);
}

#[test]
fn typing_replaces_a_selection() {
    let (mut edit, mut value) = typed("hello");
    edit.select_all(&value);
    let change = edit.insert(&value, "x");
    commit(&mut edit, &mut value, change);
    assert_eq!(value, "x");
    assert_eq!(edit.selection(), (1, 1));
}

#[test]
fn moving_with_a_selection_goes_to_its_end() {
    let (mut edit, value) = typed("hello");
    edit.select_all(&value);
    edit.go(&value, Direction::Back, Unit::Character, false);
    assert_eq!(edit.selection(), (0, 0));
    edit.select_all(&value);
    edit.go(&value, Direction::Forward, Unit::Character, false);
    assert_eq!(edit.selection(), (5, 5));
}

#[test]
fn accents_and_joined_emoji_stay_with_their_character() {
    let family = "a\u{0301}\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{1F44B}\u{1F3FD}z";
    let mut edit = TextEdit::new();
    let mut stops = vec![0];
    edit.place(0);
    while edit.caret() < family.len() {
        edit.go(family, Direction::Forward, Unit::Character, false);
        stops.push(edit.caret());
    }
    assert_eq!(stops.len(), 5, "4 visible characters: {stops:?}");

    let mut back = vec![family.len()];
    while edit.caret() > 0 {
        edit.go(family, Direction::Back, Unit::Character, false);
        back.push(edit.caret());
    }
    back.reverse();
    assert_eq!(
        back, stops,
        "it stops in the same places in both directions"
    );
}

#[test]
fn words_are_skipped_as_they_are_on_windows() {
    let value = "one, two  three";
    let mut edit = TextEdit::new();
    let mut stops = Vec::new();
    while edit.caret() < value.len() {
        edit.go(value, Direction::Forward, Unit::Word, false);
        stops.push(edit.caret());
    }
    assert_eq!(stops, [3, 5, 10, 15]);

    let mut stops = Vec::new();
    while edit.caret() > 0 {
        edit.go(value, Direction::Back, Unit::Word, false);
        stops.push(edit.caret());
    }
    assert_eq!(stops, [10, 5, 3, 0]);
}

#[test]
fn a_double_click_selects_what_it_lands_on() {
    let value = "one two, three";
    let mut edit = TextEdit::new();
    edit.select_word_at(value, 5);
    assert_eq!(edit.selection(), (4, 7), "two");
    edit.select_word_at(value, 3);
    assert_eq!(edit.selection(), (3, 4), "the space");
    edit.select_word_at(value, value.len());
    assert_eq!(edit.selection(), (9, 14), "three, from the very end");
    edit.select_word_at("", 0);
    assert_eq!(edit.selection(), (0, 0));
}

/// Deleting a word with Ctrl goes no further than its line, as in a browser's textarea: at the start of a line it's only the
/// line break that goes, and at the end of one only the line break after it.
#[test]
fn deleting_a_word_stops_at_a_line_break() {
    let (mut edit, mut value) = typed("one two
three four
");
    let change = edit.delete(&value, Direction::Back, Unit::Word);
    commit(&mut edit, &mut value, change);
    assert_eq!(value, "one two
three four");
    let change = edit.delete(&value, Direction::Back, Unit::Word);
    commit(&mut edit, &mut value, change);
    assert_eq!(value, "one two
three ");
    // From the end of the first line, forward: only the break goes.
    edit.place(7);
    let change = edit.delete(&value, Direction::Forward, Unit::Word);
    commit(&mut edit, &mut value, change);
    assert_eq!(value, "one twothree ");
    // And a word before a break doesn't take the break with it.
    let (mut edit, mut value) = typed("ab   
cd");
    edit.place(2);
    let change = edit.delete(&value, Direction::Forward, Unit::Word);
    commit(&mut edit, &mut value, change);
    assert_eq!(value, "ab
cd");
}

#[test]
fn a_run_of_typing_undoes_as_one_step() {
    let (mut edit, mut value) = typed("hello");
    let change = edit.undo(&value);
    commit(&mut edit, &mut value, change);
    assert_eq!(value, "");
    assert!(edit.undo(&value).is_none());

    let change = edit.redo(&value);
    commit(&mut edit, &mut value, change);
    assert_eq!(value, "hello");
    assert_eq!(edit.caret(), 5);
}

#[test]
fn moving_the_caret_starts_a_new_undo_step() {
    let (mut edit, mut value) = typed("ab");
    edit.go(&value, Direction::Back, Unit::Character, false);
    let change = edit.insert(&value, "x");
    commit(&mut edit, &mut value, change);
    assert_eq!(value, "axb");
    let change = edit.undo(&value);
    commit(&mut edit, &mut value, change);
    assert_eq!(value, "ab");
    assert_eq!(edit.caret(), 1, "back where it was before typing");
}

#[test]
fn undo_brings_back_a_selection_even_if_the_value_moved_it_first() {
    let (mut edit, mut value) = typed("hello");
    edit.select_all(&value);
    let change = edit
        .delete(&value, Direction::Back, Unit::Character)
        .unwrap();
    // The app sees the new value before it commits, and keeps the caret inside it.
    edit.clamp(&change.value);
    commit(&mut edit, &mut value, Some(change));
    assert_eq!(value, "");

    let change = edit.undo(&value);
    commit(&mut edit, &mut value, change);
    assert_eq!(value, "hello");
    assert_eq!(edit.selection(), (0, 5));
}

#[test]
fn new_changes_forget_what_could_be_redone() {
    let (mut edit, mut value) = typed("a");
    let change = edit.undo(&value);
    commit(&mut edit, &mut value, change);
    let change = edit.insert(&value, "b");
    commit(&mut edit, &mut value, change);
    assert!(edit.redo(&value).is_none());
}

#[test]
fn clamping_keeps_the_caret_on_a_character() {
    let mut edit = TextEdit::new();
    edit.place(100);
    edit.clamp("aé");
    assert_eq!(edit.caret(), 3);
    edit.place(2);
    edit.clamp("aé");
    assert_eq!(edit.caret(), 1, "byte 2 is inside the é");
}

#[test]
fn history_holds_what_changed_and_not_copies_of_the_value() {
    // A megabyte, with room in history for a hundred steps of it if it kept copies.
    let (mut edit, mut value) = typed("start");
    let change = edit.insert(&value, &"x".repeat(1 << 20));
    commit(&mut edit, &mut value, change);
    for _ in 0..50 {
        edit.go(&value, Direction::Back, Unit::Character, false);
        let change = edit.insert(&value, "y");
        commit(&mut edit, &mut value, change);
    }
    // Each undo step after the large paste is one byte, so all of these work, and only the
    // oldest ones are lost once there's too much to keep.
    for _ in 0..50 {
        let change = edit.undo(&value);
        commit(&mut edit, &mut value, change);
    }
    assert_eq!(value.len(), 5 + (1 << 20));
}

#[test]
fn history_stops_rather_than_breaking_if_the_value_changes_elsewhere() {
    let (mut edit, value) = typed("hello");
    assert!(edit.undo("something else entirely").is_none());
    assert!(edit.undo("").is_none());
    let change = edit.undo(&value);
    assert!(change.is_some());
    edit.clamp("");
}

#[test]
fn a_change_settled_as_something_else_still_undoes_properly() {
    // A component that makes everything uppercase.
    let (mut edit, mut value) = typed("AB");
    edit.go(&value, Direction::Back, Unit::Character, false);
    let before = value.clone();
    let change = edit
        .insert(&before, "x")
        .unwrap()
        .settled_as(&before, "AXB");
    edit.commit(&change);
    value = change.value.to_string();
    assert_eq!(value, "AXB");
    assert_eq!(edit.caret(), 2);
    let change = edit.undo(&value);
    commit(&mut edit, &mut value, change);
    assert_eq!(value, "AB");
    assert_eq!(edit.caret(), 1);
    let change = edit.redo(&value);
    commit(&mut edit, &mut value, change);
    assert_eq!(value, "AXB");
}

#[test]
fn settling_never_cuts_through_a_character() {
    let (mut edit, _) = typed("");
    edit.place(0);
    let change = edit.insert("é", "é").unwrap().settled_as("é", "è");
    edit.commit(&change);
    assert!(change.value.is_char_boundary(edit.caret()));
    assert_eq!(change.value, "è");
}

#[derive(Debug, Clone)]
enum Op {
    Type(char),
    Paste(String),
    Move(Direction, Unit, bool),
    Delete(Direction, Unit),
    SelectAll,
    Undo,
    Redo,
}

fn direction() -> impl Strategy<Value = Direction> {
    prop_oneof![Just(Direction::Back), Just(Direction::Forward)]
}

fn unit() -> impl Strategy<Value = Unit> {
    prop_oneof![Just(Unit::Character), Just(Unit::Word), Just(Unit::Line)]
}

fn character() -> impl Strategy<Value = char> {
    prop_oneof![
        Just('a'),
        Just('Z'),
        Just(' '),
        Just('.'),
        Just('é'),
        Just('\u{0301}'),
        Just('\u{200D}'),
        Just('日'),
        Just('\u{1F44B}'),
    ]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => character().prop_map(Op::Type),
        1 => proptest::collection::vec(character(), 0..5)
            .prop_map(|text| Op::Paste(text.into_iter().collect())),
        3 => (direction(), unit(), any::<bool>()).prop_map(|(d, u, e)| Op::Move(d, u, e)),
        2 => (direction(), unit()).prop_map(|(d, u)| Op::Delete(d, u)),
        1 => Just(Op::SelectAll),
        2 => Just(Op::Undo),
        1 => Just(Op::Redo),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Whatever happens, the positions stay valid, and only what was asked for changes.
    #[test]
    fn edits_stay_valid_and_change_only_the_selection(ops in proptest::collection::vec(op(), 0..60)) {
        let mut edit = TextEdit::new();
        let mut value = String::new();
        for op in ops {
            let (start, end) = edit.selection();
            let change = match &op {
                Op::Type(character) => edit.insert(&value, character.encode_utf8(&mut [0; 4])),
                Op::Paste(text) => edit.insert(&value, text),
                Op::Delete(direction, unit) => edit.delete(&value, *direction, *unit),
                Op::Undo => edit.undo(&value),
                Op::Redo => edit.redo(&value),
                Op::Move(direction, unit, extend) => {
                    edit.go(&value, *direction, *unit, *extend);
                    None
                }
                Op::SelectAll => {
                    edit.select_all(&value);
                    None
                }
            };
            let before = value.clone();
            commit(&mut edit, &mut value, change);

            let (new_start, new_end) = edit.selection();
            prop_assert!(new_end <= value.len());
            prop_assert!(value.is_char_boundary(new_start) && value.is_char_boundary(new_end));
            match &op {
                Op::Type(_) | Op::Paste(_) => {
                    let text = match &op {
                        Op::Type(character) => character.to_string(),
                        Op::Paste(text) => text.clone(),
                        _ => unreachable!(),
                    };
                    let expected = format!("{}{text}{}", &before[..start], &before[end..]);
                    prop_assert_eq!(&value, &expected);
                }
                Op::Delete(..) => {
                    // What's left is the same text with one piece cut out of it.
                    let at = edit.caret();
                    let removed = before.len() - value.len();
                    prop_assert_eq!(&value[..at], &before[..at]);
                    prop_assert_eq!(&value[at..], &before[at + removed..]);
                }
                Op::Move(..) | Op::SelectAll => prop_assert_eq!(&value, &before),
                Op::Undo | Op::Redo => {}
            }
        }
    }

    /// Undoing everything gets back to the start, and redoing it all gets back to the end.
    #[test]
    fn undo_and_redo_retrace_every_step(ops in proptest::collection::vec(op(), 0..60)) {
        let mut edit = TextEdit::new();
        let mut value = String::new();
        for op in ops {
            let change = match op {
                Op::Type(character) => edit.insert(&value, character.encode_utf8(&mut [0; 4])),
                Op::Paste(text) => edit.insert(&value, &text),
                Op::Delete(direction, unit) => edit.delete(&value, direction, unit),
                Op::Move(direction, unit, extend) => {
                    edit.go(&value, direction, unit, extend);
                    None
                }
                Op::SelectAll => {
                    edit.select_all(&value);
                    None
                }
                Op::Undo | Op::Redo => None,
            };
            commit(&mut edit, &mut value, change);
        }
        let end = value.clone();
        while let Some(change) = edit.undo(&value) {
            commit(&mut edit, &mut value, Some(change));
        }
        prop_assert_eq!(&value, "");
        while let Some(change) = edit.redo(&value) {
            commit(&mut edit, &mut value, Some(change));
        }
        prop_assert_eq!(&value, &end);
    }

    /// Plain text has one stop per character, in both directions.
    #[test]
    fn plain_characters_each_take_one_step(text in "[a-z ]{0,30}") {
        let mut edit = TextEdit::new();
        let boundaries: Vec<usize> = (0..=text.len()).collect();
        let mut stops = vec![0];
        while edit.caret() < text.len() {
            edit.go(&text, Direction::Forward, Unit::Character, false);
            stops.push(edit.caret());
        }
        prop_assert_eq!(&stops, &boundaries);
    }
}

#[test]
fn marks_in_any_script_stay_with_their_letter() {
    // Hebrew shin with a point, then Arabic beh with a vowel, then a Latin letter with an accent.
    let value = "\u{05E9}\u{05B8}\u{0628}\u{064E}e\u{0301}";
    let mut edit = TextEdit::new();
    for expected in [4, 8, 11] {
        edit.go(value, Direction::Forward, Unit::Character, false);
        assert_eq!(edit.caret(), expected);
    }
    for expected in [8, 4, 0] {
        edit.go(value, Direction::Back, Unit::Character, false);
        assert_eq!(edit.caret(), expected);
    }
}

#[test]
fn a_word_with_marks_in_it_is_one_word() {
    let value = "\u{05E9}\u{05B8}\u{05DC}\u{05D5}\u{05B9}\u{05DD} x";
    let mut edit = TextEdit::new();
    edit.select_word_at(value, 0);
    assert_eq!(edit.selection(), (0, value.len() - 2));
}

/// What's left after pressing backspace at the end of this.
fn after_backspace(value: &str) -> String {
    let mut edit = TextEdit::new();
    edit.place(value.len());
    edit.delete(value, Direction::Back, Unit::Character)
        .map_or_else(|| value.to_owned(), |change| change.value.to_string())
}

#[test]
fn backspace_takes_a_mark_and_leaves_its_letter() {
    assert_eq!(after_backspace("e\u{0301}"), "e");
    // A vowel in Arabic, Hebrew and Hindi.
    assert_eq!(after_backspace("\u{0628}\u{064E}"), "\u{0628}");
    assert_eq!(after_backspace("\u{05D1}\u{05B8}"), "\u{05D1}");
    assert_eq!(after_backspace("\u{0915}\u{093F}"), "\u{0915}");
    // One at a time, when there are several.
    assert_eq!(after_backspace("e\u{0301}\u{0302}"), "e\u{0301}");
}

#[test]
fn backspace_takes_a_whole_letter_when_there_is_no_mark() {
    assert_eq!(after_backspace("ab"), "a");
    assert_eq!(after_backspace("a"), "");
    assert_eq!(after_backspace(""), "");
    assert_eq!(after_backspace("\u{0301}"), "");
}

#[test]
fn backspace_takes_a_stacked_letter_and_leaves_the_others() {
    // Ka and Ssa stacked, as they're written in Hindi.
    assert_eq!(
        after_backspace("\u{0915}\u{094D}\u{0937}"),
        "\u{0915}\u{094D}"
    );
    assert_eq!(after_backspace("\u{0915}\u{094D}"), "\u{0915}");
    // With a joiner asking for a half form, which isn't an emoji.
    assert_eq!(
        after_backspace("\u{0915}\u{094D}\u{200D}\u{0937}"),
        "\u{0915}\u{094D}\u{200D}"
    );
}

#[test]
fn the_arrow_keys_go_over_a_stack_of_letters_at_once() {
    let value = "\u{0915}\u{094D}\u{0937}\u{093F}\u{0915}";
    let mut edit = TextEdit::new();
    edit.place(0);
    for expected in [12, 15] {
        edit.go(value, Direction::Forward, Unit::Character, false);
        assert_eq!(edit.caret(), expected);
    }
    for expected in [12, 0] {
        edit.go(value, Direction::Back, Unit::Character, false);
        assert_eq!(edit.caret(), expected);
    }
}

#[test]
fn tamil_letters_after_its_dot_stand_alone() {
    // Its sign is drawn as a dot, rather than stacking what's after it.
    let value = "\u{0B95}\u{0BCD}\u{0BB7}";
    let mut edit = TextEdit::new();
    edit.place(0);
    edit.go(value, Direction::Forward, Unit::Character, false);
    assert_eq!(edit.caret(), 6);
}

#[test]
fn the_arrow_keys_and_backspace_treat_a_flag_as_one_thing() {
    // Two flags, each of two characters of four bytes.
    let value = "\u{1F1F3}\u{1F1F1}\u{1F1FA}\u{1F1F8}x";
    let mut edit = TextEdit::new();
    edit.place(0);
    for expected in [8, 16, 17] {
        edit.go(value, Direction::Forward, Unit::Character, false);
        assert_eq!(edit.caret(), expected);
    }
    for expected in [16, 8, 0] {
        edit.go(value, Direction::Back, Unit::Character, false);
        assert_eq!(edit.caret(), expected);
    }
    assert_eq!(after_backspace(&value[..16]), &value[..8]);
    assert_eq!(after_backspace(&value[..8]), "");
    // On its own, half of one is just itself.
    assert_eq!(after_backspace("\u{1F1F3}"), "");
    assert_eq!(after_backspace("a\u{1F1F3}"), "a");
}

#[test]
fn backspace_takes_all_of_an_emoji() {
    assert_eq!(after_backspace("a\u{1F44B}\u{1F3FD}"), "a");
    assert_eq!(after_backspace("a\u{2764}\u{FE0F}"), "a");
    assert_eq!(after_backspace("a1\u{FE0F}\u{20E3}"), "a");
    assert_eq!(
        after_backspace("a\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}"),
        "a"
    );
}

#[test]
fn delete_still_takes_a_letter_and_its_marks_together() {
    let value = "e\u{0301}z";
    let mut edit = TextEdit::new();
    edit.place(0);
    let change = edit
        .delete(value, Direction::Forward, Unit::Character)
        .unwrap();
    assert_eq!(change.value, "z");
}
