//! The Unicode Bidirectional Algorithm against Unicode's own conformance suites.
//!
//! A sample of `BidiCharacterTest.txt` is in `tests/data`. To run both suites in full, set
//! `RSUI_UCD` to a folder holding `BidiCharacterTest.txt` and `BidiTest.txt`.

use rsui_text::bidi::{
    Class, class_of, is_removed, mirror_of, paragraph, paragraph_of_classes, reset_line_levels,
    visual_order,
};

fn check_levels_and_order(
    result_level: u8,
    mut levels: Vec<u8>,
    classes: &[Class],
    expected_levels: &[Option<u8>],
    expected_order: &[usize],
    context: &str,
) -> Result<(), String> {
    reset_line_levels(&mut levels, classes, result_level);
    for (index, expected) in expected_levels.iter().enumerate() {
        if let Some(expected) = expected {
            if levels[index] != *expected {
                return Err(format!(
                    "{context}: level at {index} is {}, expected {expected}, in {levels:?}",
                    levels[index]
                ));
            }
        }
    }
    let shown: Vec<usize> = (0..levels.len())
        .filter(|index| expected_levels[*index].is_some())
        .collect();
    let shown_levels: Vec<u8> = shown.iter().map(|index| levels[*index]).collect();
    let order: Vec<usize> = visual_order(&shown_levels)
        .into_iter()
        .map(|position| shown[position])
        .collect();
    if order != expected_order {
        return Err(format!(
            "{context}: order is {order:?}, expected {expected_order:?}"
        ));
    }
    Ok(())
}

fn parse_levels(text: &str) -> Vec<Option<u8>> {
    text.split_whitespace()
        .map(|part| {
            if part == "x" {
                None
            } else {
                Some(part.parse().expect("a level"))
            }
        })
        .collect()
}

fn parse_order(text: &str) -> Vec<usize> {
    text.split_whitespace()
        .map(|part| part.parse().expect("a position"))
        .collect()
}

/// Runs lines of `BidiCharacterTest.txt`, returning how many passed and the first few failures.
fn run_character_tests(text: &str) -> (usize, Vec<String>) {
    let mut passed = 0;
    let mut failures = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(';').collect();
        let characters: Vec<char> = fields[0]
            .split_whitespace()
            .map(|code| {
                char::from_u32(u32::from_str_radix(code, 16).expect("a code point"))
                    .expect("a character")
            })
            .collect();
        let level = match fields[1] {
            "0" => Some(0),
            "1" => Some(1),
            _ => None,
        };
        let result = paragraph(&characters, level);
        let expected_level: u8 = fields[2].parse().expect("a level");
        let outcome = if result.level != expected_level {
            Err(format!(
                "{line}: paragraph level is {}, expected {expected_level}",
                result.level
            ))
        } else {
            check_levels_and_order(
                result.level,
                result.levels.clone(),
                &result.classes,
                &parse_levels(fields[3]),
                &parse_order(fields[4]),
                line,
            )
        };
        match outcome {
            Ok(()) => passed += 1,
            Err(failure) => {
                if failures.len() < 5 {
                    failures.push(failure);
                }
            }
        }
    }
    (passed, failures)
}

fn class_by_name(name: &str) -> Class {
    match name {
        "L" => Class::L,
        "R" => Class::R,
        "AL" => Class::AL,
        "EN" => Class::EN,
        "ES" => Class::ES,
        "ET" => Class::ET,
        "AN" => Class::AN,
        "CS" => Class::CS,
        "NSM" => Class::NSM,
        "BN" => Class::BN,
        "B" => Class::B,
        "S" => Class::S,
        "WS" => Class::WS,
        "ON" => Class::ON,
        "LRE" => Class::LRE,
        "LRO" => Class::LRO,
        "RLE" => Class::RLE,
        "RLO" => Class::RLO,
        "PDF" => Class::PDF,
        "LRI" => Class::LRI,
        "RLI" => Class::RLI,
        "FSI" => Class::FSI,
        "PDI" => Class::PDI,
        other => panic!("unknown class {other}"),
    }
}

/// Runs `BidiTest.txt`, which lists classes rather than characters.
fn run_class_tests(text: &str) -> (usize, Vec<String>) {
    let (mut passed, mut failures) = (0, Vec::new());
    let mut expected_levels = Vec::new();
    let mut expected_order = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("@Levels:") {
            expected_levels = parse_levels(rest);
            continue;
        }
        if let Some(rest) = line.strip_prefix("@Reorder:") {
            expected_order = parse_order(rest);
            continue;
        }
        let (classes, bits) = line.split_once(';').expect("two fields");
        let classes: Vec<Class> = classes.split_whitespace().map(class_by_name).collect();
        let brackets = vec![None; classes.len()];
        let bits: u8 = bits.trim().parse().expect("a bitset");
        for (bit, level) in [(1, None), (2, Some(0)), (4, Some(1))] {
            if bits & bit == 0 {
                continue;
            }
            let result = paragraph_of_classes(&classes, &brackets, level);
            let outcome = check_levels_and_order(
                result.level,
                result.levels.clone(),
                &result.classes,
                &expected_levels,
                &expected_order,
                line,
            );
            match outcome {
                Ok(()) => passed += 1,
                Err(failure) => {
                    if failures.len() < 5 {
                        failures.push(failure);
                    }
                }
            }
        }
    }
    (passed, failures)
}

#[test]
fn a_sample_of_the_official_tests_passes() {
    let (passed, failures) = run_character_tests(include_str!("data/bidi-sample.txt"));
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(passed > 500, "only {passed} ran");
}

#[test]
fn the_whole_of_the_official_character_tests_pass_if_they_are_there() {
    let Some(folder) = std::env::var_os("RSUI_UCD") else {
        return;
    };
    let path = std::path::Path::new(&folder).join("BidiCharacterTest.txt");
    let text = std::fs::read_to_string(path).expect("the file is there");
    let (passed, failures) = run_character_tests(&text);
    println!("character tests passed: {passed}");
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(passed > 90_000, "only {passed} ran");
}

#[test]
fn the_whole_of_the_official_class_tests_pass_if_they_are_there() {
    let Some(folder) = std::env::var_os("RSUI_UCD") else {
        return;
    };
    let path = std::path::Path::new(&folder).join("BidiTest.txt");
    let text = std::fs::read_to_string(path).expect("the file is there");
    let (passed, failures) = run_class_tests(&text);
    println!("class tests passed: {passed}");
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(passed > 500_000, "only {passed} ran");
}

#[test]
fn classes_come_from_the_tables() {
    assert_eq!(class_of('a'), Class::L);
    assert_eq!(class_of('\u{05D0}'), Class::R);
    assert_eq!(class_of('\u{0627}'), Class::AL);
    assert_eq!(class_of('1'), Class::EN);
    assert_eq!(class_of('\u{0661}'), Class::AN);
    assert_eq!(class_of(' '), Class::WS);
    assert_eq!(class_of('\u{2067}'), Class::RLI);
    assert!(is_removed(class_of('\u{202B}')));
    assert_eq!(class_of('\u{10FFFF}'), Class::BN);
}

#[test]
fn mirrors_come_from_the_tables() {
    assert_eq!(mirror_of('('), Some(')'));
    assert_eq!(mirror_of(')'), Some('('));
    assert_eq!(mirror_of('\u{00AB}'), Some('\u{00BB}'));
    assert_eq!(mirror_of('a'), None);
}

#[test]
fn nothing_breaks_on_an_empty_paragraph_or_deep_nesting() {
    assert!(paragraph(&[], None).levels.is_empty());
    assert!(visual_order(&[]).is_empty());
    // Far past the limit of 125 levels, which has to overflow gently.
    let deep: Vec<char> = "\u{202B}\u{2067}".repeat(1000).chars().collect();
    let result = paragraph(&deep, None);
    assert_eq!(result.levels.len(), deep.len());
    assert!(result.levels.iter().all(|level| *level <= 126));
}
