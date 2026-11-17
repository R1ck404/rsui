//! The Unicode Bidirectional Algorithm, UAX #9: which way each part of a text runs, when
//! right-to-left and left-to-right text are mixed.
//!
//! This follows the numbering of the rules in that document, and passes its two conformance
//! suites in full. See `tests/bidi.rs`.

use crate::unicode_tables::{BIDI_CLASSES, BRACKETS, MIRRORS};

/// A character's bidirectional class.
#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Class {
    L,
    R,
    AL,
    EN,
    ES,
    ET,
    AN,
    CS,
    NSM,
    BN,
    B,
    S,
    WS,
    ON,
    LRE,
    LRO,
    RLE,
    RLO,
    PDF,
    LRI,
    RLI,
    FSI,
    PDI,
}

const ALL_CLASSES: [Class; 23] = [
    Class::L,
    Class::R,
    Class::AL,
    Class::EN,
    Class::ES,
    Class::ET,
    Class::AN,
    Class::CS,
    Class::NSM,
    Class::BN,
    Class::B,
    Class::S,
    Class::WS,
    Class::ON,
    Class::LRE,
    Class::LRO,
    Class::RLE,
    Class::RLO,
    Class::PDF,
    Class::LRI,
    Class::RLI,
    Class::FSI,
    Class::PDI,
];

/// The most levels of embedding there can be, by rule BD2.
const MAX_DEPTH: u8 = 125;
/// How many brackets rule BD16 tracks at once.
const MAX_BRACKET_DEPTH: usize = 63;

/// The value for `at` in a table of runs, whose values are in the low `bits`.
pub(crate) fn lookup_run(table: &[u32], bits: u32, at: u32) -> u32 {
    let found = table.partition_point(|entry| entry >> bits <= at);
    table[found - 1] & ((1 << bits) - 1)
}

/// The class of a character.
pub fn class_of(character: char) -> Class {
    ALL_CLASSES[lookup_run(BIDI_CLASSES, 5, character as u32) as usize]
}

/// The character to show instead when this one appears in right-to-left text, such as `)` for `(`.
pub fn mirror_of(character: char) -> Option<char> {
    let at = character as u32;
    let (mut low, mut high) = (0, MIRRORS.len() / 2);
    while low < high {
        let middle = (low + high) / 2;
        match MIRRORS[middle * 2].cmp(&at) {
            std::cmp::Ordering::Equal => return char::from_u32(MIRRORS[middle * 2 + 1]),
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
        }
    }
    None
}

/// What a character is if it's a paired bracket: an identity shared with its pair, and whether it
/// opens.
fn bracket_of(character: char) -> Option<(u32, bool)> {
    let at = character as u32;
    let count = BRACKETS.len() / 3;
    let (mut low, mut high) = (0, count);
    while low < high {
        let middle = (low + high) / 2;
        let found = BRACKETS[middle * 3];
        match found.cmp(&at) {
            std::cmp::Ordering::Equal => {
                let pair = BRACKETS[middle * 3 + 1];
                let opens = BRACKETS[middle * 3 + 2] == 1;
                let opening = if opens { at } else { pair };
                // These two angle brackets are canonically the same as another pair.
                let identity = match opening {
                    0x2329 => 0x3008,
                    other => other,
                };
                return Some((identity, opens));
            }
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
        }
    }
    None
}

/// Whether rule X9 removes characters of this class from consideration. They have no level of their
/// own, and show nothing.
pub fn is_removed(class: Class) -> bool {
    matches!(
        class,
        Class::RLE | Class::LRE | Class::RLO | Class::LRO | Class::PDF | Class::BN
    )
}

fn is_isolate_initiator(class: Class) -> bool {
    matches!(class, Class::LRI | Class::RLI | Class::FSI)
}

fn is_neutral_or_isolate(class: Class) -> bool {
    matches!(
        class,
        Class::B
            | Class::S
            | Class::WS
            | Class::ON
            | Class::FSI
            | Class::LRI
            | Class::RLI
            | Class::PDI
    )
}

fn direction(level: u8) -> Class {
    if level % 2 == 1 { Class::R } else { Class::L }
}

/// The level from the first strong character in this range, by rules P2 and P3, skipping what's
/// inside isolates. `None` if there isn't one.
fn first_strong_level(
    classes: &[Class],
    matching: &[Option<usize>],
    start: usize,
    end: usize,
) -> Option<u8> {
    let mut index = start;
    while index < end {
        match classes[index] {
            Class::L => return Some(0),
            Class::R | Class::AL => return Some(1),
            Class::LRI | Class::RLI | Class::FSI => match matching[index] {
                Some(close) => index = close,
                None => return None,
            },
            _ => {}
        }
        index += 1;
    }
    None
}

/// For each isolate initiator, the position of its matching PDI, by rule BD9.
fn match_isolates(classes: &[Class]) -> Vec<Option<usize>> {
    let mut matching = vec![None; classes.len()];
    let mut open = Vec::new();
    for (index, class) in classes.iter().enumerate() {
        if is_isolate_initiator(*class) {
            open.push(index);
        } else if *class == Class::PDI {
            if let Some(start) = open.pop() {
                matching[start] = Some(index);
            }
        }
    }
    matching
}

#[derive(Clone, Copy)]
struct Entry {
    level: u8,
    /// What rule X5 overrides characters to.
    overriding: Option<Class>,
    isolate: bool,
}

fn next_odd(level: u8) -> u8 {
    if level % 2 == 1 { level + 2 } else { level + 1 }
}

fn next_even(level: u8) -> u8 {
    if level % 2 == 0 { level + 2 } else { level + 1 }
}

/// The levels of each character in a paragraph, before any line breaking, with its own level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paragraph {
    pub level: u8,
    /// One for each character. Characters rule X9 removes take the level before them.
    pub levels: Vec<u8>,
    pub classes: Vec<Class>,
}

/// Works out the levels of a paragraph. `level` is 0 or 1 to say which way it runs, or `None` to
/// go by its first strong character.
pub fn paragraph(characters: &[char], level: Option<u8>) -> Paragraph {
    let classes: Vec<Class> = characters.iter().map(|c| class_of(*c)).collect();
    let brackets: Vec<Option<(u32, bool)>> = characters
        .iter()
        .zip(&classes)
        .map(|(character, class)| {
            if *class == Class::ON {
                bracket_of(*character)
            } else {
                None
            }
        })
        .collect();
    paragraph_of_classes(&classes, &brackets, level)
}

/// Like [`paragraph`], given the classes, and each bracket's identity and whether it opens. The
/// conformance tests use this to test classes that no particular characters stand for.
pub fn paragraph_of_classes(
    classes: &[Class],
    brackets: &[Option<(u32, bool)>],
    level: Option<u8>,
) -> Paragraph {
    let matching = match_isolates(classes);
    let paragraph_level = level
        .unwrap_or_else(|| first_strong_level(classes, &matching, 0, classes.len()).unwrap_or(0));
    let count = classes.len();
    let mut levels = vec![paragraph_level; count];
    let mut resolved = classes.to_vec();
    explicit_levels(
        classes,
        &matching,
        paragraph_level,
        &mut levels,
        &mut resolved,
    );

    // X10: the level runs, without what X9 removes.
    let mut runs: Vec<Vec<usize>> = Vec::new();
    for index in 0..count {
        if is_removed(classes[index]) {
            continue;
        }
        match runs.last_mut() {
            Some(run) if levels[*run.last().expect("runs aren't empty")] == levels[index] => {
                run.push(index);
            }
            _ => runs.push(vec![index]),
        }
    }
    let mut run_starting_at = std::collections::HashMap::new();
    for (position, run) in runs.iter().enumerate() {
        run_starting_at.insert(run[0], position);
    }
    let is_matched_pdi = {
        let mut matched = vec![false; count];
        for close in matching.iter().flatten() {
            matched[*close] = true;
        }
        matched
    };

    // BD13: isolating run sequences. These go by the levels from before any of them is resolved.
    let embedding = levels.clone();
    for run in &runs {
        let first = run[0];
        if classes[first] == Class::PDI && is_matched_pdi[first] {
            continue;
        }
        let mut sequence: Vec<usize> = run.clone();
        loop {
            let last = *sequence.last().expect("sequences aren't empty");
            let next = if is_isolate_initiator(classes[last]) {
                matching[last].and_then(|close| run_starting_at.get(&close))
            } else {
                None
            };
            match next {
                Some(&position) => sequence.extend_from_slice(&runs[position]),
                None => break,
            }
        }
        resolve_sequence(
            &sequence,
            &embedding,
            classes,
            brackets,
            &matching,
            paragraph_level,
            &mut levels,
            &mut resolved,
        );
    }

    // Characters X9 removes follow what's before them, so they don't break up runs later.
    let mut previous = paragraph_level;
    for index in 0..count {
        if is_removed(classes[index]) {
            levels[index] = previous;
        } else {
            previous = levels[index];
        }
    }
    Paragraph {
        level: paragraph_level,
        levels,
        classes: classes.to_vec(),
    }
}

/// Rules X1 to X8.
fn explicit_levels(
    classes: &[Class],
    matching: &[Option<usize>],
    paragraph_level: u8,
    levels: &mut [u8],
    resolved: &mut [Class],
) {
    let mut stack = vec![Entry {
        level: paragraph_level,
        overriding: None,
        isolate: false,
    }];
    let (mut overflow_isolates, mut overflow_embeddings, mut valid_isolates) =
        (0usize, 0usize, 0usize);
    for index in 0..classes.len() {
        let top = *stack.last().expect("the first entry is never popped");
        match classes[index] {
            class @ (Class::RLE | Class::LRE | Class::RLO | Class::LRO) => {
                let new_level = if matches!(class, Class::RLE | Class::RLO) {
                    next_odd(top.level)
                } else {
                    next_even(top.level)
                };
                if new_level <= MAX_DEPTH && overflow_isolates == 0 && overflow_embeddings == 0 {
                    let overriding = match class {
                        Class::RLO => Some(Class::R),
                        Class::LRO => Some(Class::L),
                        _ => None,
                    };
                    stack.push(Entry {
                        level: new_level,
                        overriding,
                        isolate: false,
                    });
                } else if overflow_isolates == 0 {
                    overflow_embeddings += 1;
                }
            }
            class @ (Class::RLI | Class::LRI | Class::FSI) => {
                levels[index] = top.level;
                if let Some(overriding) = top.overriding {
                    resolved[index] = overriding;
                }
                let right_to_left = match class {
                    Class::RLI => true,
                    Class::LRI => false,
                    _ => {
                        let end = matching[index].unwrap_or(classes.len());
                        first_strong_level(classes, matching, index + 1, end) == Some(1)
                    }
                };
                let new_level = if right_to_left {
                    next_odd(top.level)
                } else {
                    next_even(top.level)
                };
                if new_level <= MAX_DEPTH && overflow_isolates == 0 && overflow_embeddings == 0 {
                    valid_isolates += 1;
                    stack.push(Entry {
                        level: new_level,
                        overriding: None,
                        isolate: true,
                    });
                } else {
                    overflow_isolates += 1;
                }
            }
            Class::PDI => {
                if overflow_isolates > 0 {
                    overflow_isolates -= 1;
                } else if valid_isolates > 0 {
                    overflow_embeddings = 0;
                    while !stack.last().expect("an isolate is in there").isolate {
                        stack.pop();
                    }
                    stack.pop();
                    valid_isolates -= 1;
                }
                let top = *stack.last().expect("the first entry is never popped");
                levels[index] = top.level;
                if let Some(overriding) = top.overriding {
                    resolved[index] = overriding;
                }
            }
            Class::PDF => {
                if overflow_isolates == 0 {
                    if overflow_embeddings > 0 {
                        overflow_embeddings -= 1;
                    } else if !top.isolate && stack.len() >= 2 {
                        stack.pop();
                    }
                }
            }
            Class::B => levels[index] = paragraph_level,
            Class::BN => {}
            _ => {
                levels[index] = top.level;
                if let Some(overriding) = top.overriding {
                    resolved[index] = overriding;
                }
            }
        }
    }
}

/// Rules W1 to I2, for one isolating run sequence.
#[allow(clippy::too_many_arguments)]
fn resolve_sequence(
    sequence: &[usize],
    embedding: &[u8],
    classes: &[Class],
    brackets: &[Option<(u32, bool)>],
    matching: &[Option<usize>],
    paragraph_level: u8,
    levels: &mut [u8],
    resolved: &mut [Class],
) {
    let level = embedding[sequence[0]];
    // What's on either side, skipping what X9 removes.
    let before = classes[..sequence[0]]
        .iter()
        .rposition(|class| !is_removed(*class))
        .map_or(paragraph_level, |at| embedding[at]);
    let last = *sequence.last().expect("sequences aren't empty");
    let after = if is_isolate_initiator(classes[last]) && matching[last].is_none() {
        paragraph_level
    } else {
        classes[last + 1..]
            .iter()
            .position(|class| !is_removed(*class))
            .map_or(paragraph_level, |at| embedding[last + 1 + at])
    };
    let sos = direction(level.max(before));
    let eos = direction(level.max(after));

    let mut types: Vec<Class> = sequence.iter().map(|index| resolved[*index]).collect();
    let count = types.len();

    // W1
    let mut previous = sos;
    for class in &mut types {
        if *class == Class::NSM {
            *class = if is_isolate_initiator(previous) || previous == Class::PDI {
                Class::ON
            } else {
                previous
            };
        }
        previous = *class;
    }
    // W2
    let mut strong = sos;
    for class in &mut types {
        match *class {
            Class::L | Class::R | Class::AL => strong = *class,
            Class::EN if strong == Class::AL => *class = Class::AN,
            _ => {}
        }
    }
    // W3
    for class in &mut types {
        if *class == Class::AL {
            *class = Class::R;
        }
    }
    // W4
    for index in 1..count.saturating_sub(1) {
        let (left, right) = (types[index - 1], types[index + 1]);
        match types[index] {
            Class::ES if left == Class::EN && right == Class::EN => types[index] = Class::EN,
            Class::CS if left == right && matches!(left, Class::EN | Class::AN) => {
                types[index] = left;
            }
            _ => {}
        }
    }
    // W5
    let mut index = 0;
    while index < count {
        if types[index] == Class::ET {
            let start = index;
            while index < count && types[index] == Class::ET {
                index += 1;
            }
            let touches_number = (start > 0 && types[start - 1] == Class::EN)
                || (index < count && types[index] == Class::EN);
            if touches_number {
                types[start..index].fill(Class::EN);
            }
        } else {
            index += 1;
        }
    }
    // W6
    for class in &mut types {
        if matches!(*class, Class::ES | Class::ET | Class::CS) {
            *class = Class::ON;
        }
    }
    // W7
    let mut strong = sos;
    for class in &mut types {
        match *class {
            Class::L | Class::R => strong = *class,
            Class::EN if strong == Class::L => *class = Class::L,
            _ => {}
        }
    }
    resolve_brackets(sequence, classes, brackets, level, sos, &mut types);

    // N1 and N2
    let mut index = 0;
    while index < count {
        if is_neutral_or_isolate(types[index]) {
            let start = index;
            while index < count && is_neutral_or_isolate(types[index]) {
                index += 1;
            }
            let strong_side = |class: Class| match class {
                Class::L => Class::L,
                _ => Class::R,
            };
            let left = if start == 0 {
                sos
            } else {
                strong_side(types[start - 1])
            };
            let right = if index == count {
                eos
            } else {
                strong_side(types[index])
            };
            let chosen = if left == right {
                left
            } else {
                direction(level)
            };
            types[start..index].fill(chosen);
        } else {
            index += 1;
        }
    }
    // I1 and I2
    for (position, class) in types.iter().enumerate() {
        let raised = if level % 2 == 0 {
            match class {
                Class::R => 1,
                Class::AN | Class::EN => 2,
                _ => 0,
            }
        } else {
            match class {
                Class::L | Class::EN | Class::AN => 1,
                _ => 0,
            }
        };
        levels[sequence[position]] = level + raised;
    }
}

/// Rule N0: paired brackets take the direction of what's in or around them.
fn resolve_brackets(
    sequence: &[usize],
    classes: &[Class],
    brackets: &[Option<(u32, bool)>],
    level: u8,
    sos: Class,
    types: &mut [Class],
) {
    // BD16
    let mut open: Vec<(u32, usize)> = Vec::new();
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (position, index) in sequence.iter().enumerate() {
        if types[position] != Class::ON {
            continue;
        }
        let Some((identity, opens)) = brackets.get(*index).copied().flatten() else {
            continue;
        };
        if opens {
            if open.len() == MAX_BRACKET_DEPTH {
                pairs.clear();
                break;
            }
            open.push((identity, position));
        } else if let Some(found) = open.iter().rposition(|(other, _)| *other == identity) {
            pairs.push((open[found].1, position));
            open.truncate(found);
        }
    }
    pairs.sort_unstable();

    let embedding = direction(level);
    let strong_of = |class: Class| match class {
        Class::L => Some(Class::L),
        Class::R | Class::EN | Class::AN => Some(Class::R),
        _ => None,
    };
    for (start, end) in pairs {
        let mut found_embedding = false;
        let mut found_opposite = false;
        for class in &types[start + 1..end] {
            match strong_of(*class) {
                Some(strong) if strong == embedding => {
                    found_embedding = true;
                    break;
                }
                Some(_) => found_opposite = true,
                None => {}
            }
        }
        let chosen = if found_embedding {
            Some(embedding)
        } else if found_opposite {
            let context = types[..start]
                .iter()
                .rev()
                .find_map(|class| strong_of(*class))
                .unwrap_or(sos);
            Some(if context != embedding {
                context
            } else {
                embedding
            })
        } else {
            None
        };
        if let Some(chosen) = chosen {
            types[start] = chosen;
            types[end] = chosen;
            // Marks on a bracket stay with it, going by what they were before rule W1.
            for bracket in [start, end] {
                let mut position = bracket + 1;
                while position < sequence.len() && classes[sequence[position]] == Class::NSM {
                    types[position] = chosen;
                    position += 1;
                }
            }
        }
    }
}

/// Rule L1 for one line: whitespace at the end, and before any separator, goes back to the
/// paragraph's level. Takes the levels of the line, with its classes before overrides.
pub fn reset_line_levels(levels: &mut [u8], classes: &[Class], paragraph_level: u8) {
    let mut trailing = true;
    for index in (0..levels.len()).rev() {
        match classes[index] {
            Class::B | Class::S => {
                levels[index] = paragraph_level;
                trailing = true;
            }
            Class::WS | Class::FSI | Class::LRI | Class::RLI | Class::PDI => {
                if trailing {
                    levels[index] = paragraph_level;
                }
            }
            class if is_removed(class) => {
                if trailing {
                    levels[index] = paragraph_level;
                }
            }
            _ => trailing = false,
        }
    }
}

/// Rule L2: the order to show a line's characters in, as their positions, from left to right.
pub fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let Some(&highest) = levels.iter().max() else {
        return order;
    };
    let lowest_odd = levels
        .iter()
        .copied()
        .filter(|level| level % 2 == 1)
        .min()
        .unwrap_or(highest + 1);
    let mut level = highest;
    while level >= lowest_odd && level > 0 {
        let mut index = 0;
        while index < order.len() {
            if levels[order[index]] >= level {
                let start = index;
                while index < order.len() && levels[order[index]] >= level {
                    index += 1;
                }
                order[start..index].reverse();
            } else {
                index += 1;
            }
        }
        level -= 1;
    }
    order
}
