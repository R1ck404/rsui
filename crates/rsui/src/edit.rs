//! Editing one line of text, apart from any window or tree.
//!
//! The value isn't kept here. Each operation is given the current value and works out what
//! should change, which the app offers to the component that owns the value. Only if the
//! component accepts it does the [`TextEdit`] move on, with [`TextEdit::commit`].
//!
//! History holds only what each change replaced, not copies of the value, so undo costs next to
//! nothing however long the value is.

use rsui_core::Text;

/// Which way to move or delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Back,
    Forward,
}

/// How far to move or delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// One visible character, keeping accents and joined emoji with what they belong to.
    Character,
    Word,
    /// To the start or end of the text.
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangeKind {
    Typing,
    Deleting,
    Other,
    Undo,
    Redo,
}

/// A change to the value that has been worked out, but not yet made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The whole value after this change.
    pub value: Text,
    /// Where the replaced part starts.
    at: usize,
    removed: Text,
    inserted: Text,
    caret: usize,
    anchor: usize,
    /// Where they were beforehand, for undo. The value can change before the change is
    /// committed, which would otherwise have moved them.
    before: (usize, usize),
    kind: ChangeKind,
    /// What a browser's `InputEvent.inputType` says this is, as `insertText` or `deleteWordBackward`.
    input_type: &'static str,
}

impl Change {
    /// What a browser's `InputEvent.inputType` calls this change.
    pub fn input_type(&self) -> &'static str {
        self.input_type
    }

    /// What a browser's `InputEvent.data` says for it: the text put in by typing it, and nothing otherwise.
    pub fn data(&self) -> Option<&str> {
        matches!(self.input_type, "insertText" | "insertCompositionText").then_some(self.inserted.as_str())
    }

    /// Where it leaves the selection held from, and the caret, as byte positions in the value after it.
    pub fn ends(&self) -> (usize, usize) {
        (self.anchor, self.caret)
    }

    /// The same change, said to be this kind of input, as a paste (`insertFromPaste`) or a cut (`deleteByCut`) is.
    pub fn said_as(mut self, input_type: &'static str) -> Self {
        self.input_type = input_type;
        self
    }

    /// This change, given that what actually came of it was `value`, changed from `before`. A
    /// component can take a change and alter it, such as to make it uppercase, so what was
    /// replaced is worked out again, and the caret stays as close as it can.
    pub fn settled_as(mut self, before: &str, value: &str) -> Self {
        if self.value != value {
            let prefix = floor_boundary(
                value,
                common_length(before.bytes(), value.bytes()).min(before.len()),
            );
            let prefix = floor_boundary(before, prefix);
            let longest_suffix = before.len().min(value.len()) - prefix;
            let mut suffix =
                common_length(before.bytes().rev(), value.bytes().rev()).min(longest_suffix);
            while !before.is_char_boundary(before.len() - suffix)
                || !value.is_char_boundary(value.len() - suffix)
            {
                suffix -= 1;
            }
            self.at = prefix;
            self.removed = before[prefix..before.len() - suffix].into();
            self.inserted = value[prefix..value.len() - suffix].into();
            self.value = value.into();
        }
        self.caret = floor_boundary(value, self.caret);
        self.anchor = floor_boundary(value, self.anchor);
        self
    }
}

fn common_length(a: impl Iterator<Item = u8>, b: impl Iterator<Item = u8>) -> usize {
    a.zip(b).take_while(|(a, b)| a == b).count()
}

/// One change that can be undone: at `at`, `removed` was replaced with `inserted`.
#[derive(Debug, Clone)]
struct Entry {
    at: usize,
    removed: Text,
    inserted: Text,
    before: (usize, usize),
    after: (usize, usize),
}

impl Entry {
    fn size(&self) -> usize {
        self.removed.len() + self.inserted.len()
    }
}

/// How many changes can be undone.
const UNDO_LIMIT: usize = 100;
/// How much replaced text history keeps, in bytes, apart from the latest change, which is always
/// kept however large it is.
const UNDO_BYTES: usize = 256 * 1024;

/// The caret, the selection and the undo history of one text input being edited.
#[derive(Debug, Clone, Default)]
pub struct TextEdit {
    caret: usize,
    anchor: usize,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    /// What the last change was, if the caret hasn't moved since, so that typing a word
    /// undoes as one step.
    last: Option<ChangeKind>,
}

impl TextEdit {
    pub fn new() -> Self {
        Self::default()
    }

    /// Where the caret is, as a byte position.
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selected bytes, from first to last. Empty if nothing is selected.
    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    /// Where the selection is held from, as a byte position: the caret is the other end.
    pub fn anchor(&self) -> usize {
        self.anchor
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    /// Makes sure the caret and selection are still inside `value`, in case it was changed by
    /// something else.
    pub fn clamp(&mut self, value: &str) {
        self.caret = floor_boundary(value, self.caret);
        self.anchor = floor_boundary(value, self.anchor);
    }

    /// Puts the caret at a position, with nothing selected.
    pub fn place(&mut self, at: usize) {
        self.caret = at;
        self.anchor = at;
        self.last = None;
    }

    /// Moves the caret to a position, selecting what it passes over.
    pub fn extend_to(&mut self, at: usize) {
        self.caret = at;
        self.last = None;
    }

    pub fn select_all(&mut self, value: &str) {
        self.anchor = 0;
        self.caret = value.len();
        self.last = None;
    }

    /// Selects the word, or the run of spaces or punctuation, around a position.
    pub fn select_word_at(&mut self, value: &str, at: usize) {
        let at = floor_boundary(value, at);
        let Some(class) = value[at..]
            .chars()
            .next()
            .or_else(|| value[..at].chars().next_back())
            .map(class_of)
        else {
            self.place(at);
            return;
        };
        let start = value[..at]
            .char_indices()
            .rev()
            .take_while(|(_, character)| class_of(*character) == class)
            .last()
            .map_or(at, |(byte, _)| byte);
        let end = value[at..]
            .char_indices()
            .find(|(_, character)| class_of(*character) != class)
            .map_or(value.len(), |(byte, _)| at + byte);
        self.anchor = start;
        self.caret = end;
        self.last = None;
    }

    /// Moves the caret. `extend` keeps the other end of the selection where it is.
    pub fn go(&mut self, value: &str, direction: Direction, unit: Unit, extend: bool) {
        let at = if !extend && self.has_selection() && unit == Unit::Character {
            // Moving by one with something selected goes to that end of the selection.
            let (start, end) = self.selection();
            match direction {
                Direction::Back => start,
                Direction::Forward => end,
            }
        } else {
            target(value, self.caret, direction, unit)
        };
        if extend {
            self.extend_to(at);
        } else {
            self.place(at);
        }
    }

    fn replacing(
        &self,
        value: &str,
        start: usize,
        end: usize,
        text: &str,
        kind: ChangeKind,
        input_type: &'static str,
    ) -> Change {
        Change {
            input_type,
            value: splice(value, start, end, text),
            at: start,
            removed: value[start..end].into(),
            inserted: text.into(),
            caret: start + text.len(),
            anchor: start + text.len(),
            before: (self.caret, self.anchor),
            kind,
        }
    }

    /// What typing or pasting this would do, replacing any selection.
    pub fn insert(&self, value: &str, text: &str) -> Option<Change> {
        if text.is_empty() && !self.has_selection() {
            return None;
        }
        let kind = if text.chars().count() == 1 && !self.has_selection() {
            ChangeKind::Typing
        } else {
            ChangeKind::Other
        };
        let (start, end) = self.selection();
        Some(self.replacing(value, start, end, text, kind, "insertText"))
    }

    /// What deleting would do: the selection if there is one, or else this far from the caret.
    pub fn delete(&self, value: &str, direction: Direction, unit: Unit) -> Option<Change> {
        let (start, end) = if self.has_selection() {
            self.selection()
        } else {
            let other = if direction == Direction::Back && unit == Unit::Character {
                previous_to_delete(value, self.caret)
            } else if unit == Unit::Word {
                // A word goes no further than its line: at the line's edge, only the line break goes, as in a browser.
                word_to_delete(value, self.caret, direction)
            } else {
                target(value, self.caret, direction, unit)
            };
            (self.caret.min(other), self.caret.max(other))
        };
        let input_type = match (direction, unit) {
            (Direction::Back, Unit::Character) => "deleteContentBackward",
            (Direction::Forward, Unit::Character) => "deleteContentForward",
            (Direction::Back, Unit::Word) => "deleteWordBackward",
            (Direction::Forward, Unit::Word) => "deleteWordForward",
            (Direction::Back, Unit::Line) => "deleteSoftLineBackward",
            (Direction::Forward, Unit::Line) => "deleteSoftLineForward",
        };
        (start < end).then(|| self.replacing(value, start, end, "", ChangeKind::Deleting, input_type))
    }

    /// What undoing the last change would do. Nothing, if the value has since been changed by
    /// something else, so that the history no longer fits it.
    pub fn undo(&self, value: &str) -> Option<Change> {
        let entry = self.undo.last()?;
        if value.get(entry.at..entry.at + entry.inserted.len())? != entry.inserted.as_str() {
            return None;
        }
        Some(Change {
            value: splice(
                value,
                entry.at,
                entry.at + entry.inserted.len(),
                &entry.removed,
            ),
            at: entry.at,
            removed: entry.inserted.clone(),
            inserted: entry.removed.clone(),
            caret: entry.before.0,
            anchor: entry.before.1,
            before: (self.caret, self.anchor),
            kind: ChangeKind::Undo,
            input_type: "historyUndo",
        })
    }

    /// What redoing the last undone change would do, if it still fits the value.
    pub fn redo(&self, value: &str) -> Option<Change> {
        let entry = self.redo.last()?;
        if value.get(entry.at..entry.at + entry.removed.len())? != entry.removed.as_str() {
            return None;
        }
        Some(Change {
            value: splice(
                value,
                entry.at,
                entry.at + entry.removed.len(),
                &entry.inserted,
            ),
            at: entry.at,
            removed: entry.removed.clone(),
            inserted: entry.inserted.clone(),
            caret: entry.after.0,
            anchor: entry.after.1,
            before: (self.caret, self.anchor),
            kind: ChangeKind::Redo,
            input_type: "historyRedo",
        })
    }

    /// Records that a change has been made.
    pub fn commit(&mut self, change: &Change) {
        match change.kind {
            ChangeKind::Undo => {
                if let Some(entry) = self.undo.pop() {
                    self.redo.push(entry);
                }
                self.last = None;
            }
            ChangeKind::Redo => {
                if let Some(entry) = self.redo.pop() {
                    self.undo.push(entry);
                }
                self.last = None;
            }
            kind => {
                self.redo.clear();
                let after = (change.caret, change.anchor);
                if !self.merge(change, kind, after) {
                    self.undo.push(Entry {
                        at: change.at,
                        removed: change.removed.clone(),
                        inserted: change.inserted.clone(),
                        before: change.before,
                        after,
                    });
                }
                self.keep_history_small();
                self.last = Some(kind);
            }
        }
        self.caret = change.caret;
        self.anchor = change.anchor;
    }

    /// Adds this change on to the last one, if it carries straight on from it.
    fn merge(&mut self, change: &Change, kind: ChangeKind, after: (usize, usize)) -> bool {
        if self.last != Some(kind) {
            return false;
        }
        let Some(last) = self.undo.last_mut() else {
            return false;
        };
        match kind {
            ChangeKind::Typing
                if change.removed.is_empty() && change.at == last.at + last.inserted.len() =>
            {
                last.inserted.push_str(&change.inserted);
            }
            // Pressing backspace again removes what's just before the last removal.
            ChangeKind::Deleting
                if change.inserted.is_empty()
                    && last.inserted.is_empty()
                    && change.at + change.removed.len() == last.at =>
            {
                let mut removed = change.removed.clone();
                removed.push_str(&last.removed);
                last.removed = removed;
                last.at = change.at;
            }
            // Pressing delete again removes what's now at the same place.
            ChangeKind::Deleting
                if change.inserted.is_empty()
                    && last.inserted.is_empty()
                    && change.at == last.at =>
            {
                last.removed.push_str(&change.removed);
            }
            _ => return false,
        }
        last.after = after;
        true
    }

    fn keep_history_small(&mut self) {
        let mut bytes: usize = self.undo.iter().map(Entry::size).sum();
        let mut drop = 0;
        while self.undo.len() - drop > 1
            && (self.undo.len() - drop > UNDO_LIMIT || bytes > UNDO_BYTES)
        {
            bytes -= self.undo[drop].size();
            drop += 1;
        }
        self.undo.drain(..drop);
    }
}

fn splice(value: &str, start: usize, end: usize, text: &str) -> Text {
    let mut out = Text::with_capacity(value.len() - (end - start) + text.len());
    out.push_str(&value[..start]);
    out.push_str(text);
    out.push_str(&value[end..]);
    out
}

/// The nearest character boundary at or before `at`.
fn floor_boundary(value: &str, at: usize) -> usize {
    let mut at = at.min(value.len());
    while !value.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn target(value: &str, from: usize, direction: Direction, unit: Unit) -> usize {
    match (unit, direction) {
        (Unit::Character, Direction::Back) => previous_character(value, from),
        (Unit::Character, Direction::Forward) => next_character(value, from),
        (Unit::Word, Direction::Back) => previous_word(value, from),
        (Unit::Word, Direction::Forward) => next_word(value, from),
        (Unit::Line, Direction::Back) => 0,
        (Unit::Line, Direction::Forward) => value.len(),
    }
}

/// Whether a character joins on to the one before it, such as an accent or a skin tone.
fn joins_previous(character: char) -> bool {
    !character.is_ascii()
        && (rsui_text::is_mark(character)
            || matches!(
                character as u32,
                0x200D | 0xFE00..=0xFE0F | 0x1F3FB..=0x1F3FF | 0xE0100..=0xE01EF
            ))
}

const JOINER: char = '\u{200D}';

/// The sign in a script of India that stacks the next letter on this one, rather than saying a vowel
/// after it. Tamil has one too, but it's drawn as a dot, so what's after it stands alone.
fn is_stacking_sign(character: char) -> bool {
    matches!(
        character as u32,
        0x094D
            | 0x09CD
            | 0x0A4D
            | 0x0ACD
            | 0x0B4D
            | 0x0C4D
            | 0x0CCD
            | 0x0D4D
            | 0x0DCA
            | 0x1039
            | 0x17D2
            | 0x1A60
            | 0x1B44
            | 0x1BAB
            | 0xA9C0
            | 0x11046
            | 0x11133
            | 0x111C0
            | 0x11235
            | 0x11442
            | 0x114C2
            | 0x115BF
            | 0x1163F
            | 0x116B6
    )
}

fn is_letter_of_india(character: char) -> bool {
    matches!(character as u32,
        0x0900..=0x0DFF | 0x1000..=0x109F | 0x1780..=0x17FF | 0x1A20..=0x1AAF | 0x1B00..=0x1BFF
        | 0xA980..=0xA9DF | 0x11000..=0x116FF)
        && character.is_alphabetic()
        && !rsui_text::is_mark(character)
}

/// Where backspace deletes back to. A mark typed on a letter goes by itself, so that a wrong vowel in
/// Arabic, Hebrew or Hindi doesn't take its letter with it, and so does the last letter stacked on
/// others in Hindi. What's joined up to make one emoji goes all at once, as its parts mean nothing alone.
fn previous_to_delete(value: &str, from: usize) -> usize {
    let start = previous_character(value, from);
    let whole = &value[start..from];
    let in_india = whole.chars().next().is_some_and(is_letter_of_india);
    let is_emoji = !in_india
        && whole.chars().any(|character| {
            matches!(
                character as u32,
                0x200D | 0x20E3 | 0xFE00..=0xFE0F | 0x1F3FB..=0x1F3FF | 0xE0100..=0xE01EF
            )
        });
    match whole.char_indices().next_back() {
        Some((last, character))
            if last > 0 && !is_emoji && (in_india || rsui_text::is_mark(character)) =>
        {
            start + last
        }
        _ => start,
    }
}

/// Half of a flag. Two of them make one, counting from the first in a row.
fn is_half_a_flag(character: char) -> bool {
    matches!(character as u32, 0x1F1E6..=0x1F1FF)
}

fn next_character(value: &str, from: usize) -> usize {
    let mut characters = value[from..].char_indices().peekable();
    let Some((_, mut last)) = characters.next() else {
        return value.len();
    };
    if is_half_a_flag(last) {
        if let Some((byte, other)) = characters.next() {
            if !is_half_a_flag(other) {
                return next_after_marks(value, from, byte);
            }
            last = other;
        }
    }
    while let Some(&(byte, character)) = characters.peek() {
        let stacked = is_stacking_sign(last) && is_letter_of_india(character);
        if !joins_previous(character) && last != JOINER && !stacked {
            return from + byte;
        }
        last = character;
        characters.next();
    }
    value.len()
}

/// Where the next character starts, given where the one after a flag's first half is.
fn next_after_marks(value: &str, from: usize, byte: usize) -> usize {
    let rest = &value[from + byte..];
    let first = rest.chars().next();
    if first.is_some_and(joins_previous) {
        next_character(value, from + byte)
    } else {
        from + byte
    }
}

/// Moves back over a whole flag, if that's what's found, as it's two characters.
fn previous_flag(value: &str, start: usize) -> usize {
    if !value[start..].chars().next().is_some_and(is_half_a_flag) {
        return start;
    }
    let before = value[..start]
        .chars()
        .rev()
        .take_while(|character| is_half_a_flag(*character))
        .count();
    if before % 2 == 1 { start - 4 } else { start }
}

fn previous_character(value: &str, from: usize) -> usize {
    previous_flag(value, previous_single(value, from))
}

fn previous_single(value: &str, from: usize) -> usize {
    let mut characters = value[..from].char_indices().rev().peekable();
    while let Some((byte, character)) = characters.next() {
        let joined = characters.peek().is_some_and(|(_, before)| {
            *before == JOINER || (is_stacking_sign(*before) && is_letter_of_india(character))
        });
        if !joins_previous(character) && !joined {
            return byte;
        }
    }
    0
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Class {
    Word,
    Space,
    Other,
}

fn class_of(character: char) -> Class {
    if character.is_alphanumeric() || character == '_' || joins_previous(character) {
        Class::Word
    } else if character.is_whitespace() {
        Class::Space
    } else {
        Class::Other
    }
}

/// The start of the next word, as control and the right arrow goes on Windows.
fn next_word(value: &str, from: usize) -> usize {
    let mut characters = value[from..].char_indices().peekable();
    let Some(&(_, first)) = characters.peek() else {
        return value.len();
    };
    let class = class_of(first);
    while characters
        .next_if(|(_, character)| class_of(*character) == class)
        .is_some()
    {}
    while characters
        .next_if(|(_, character)| class_of(*character) == Class::Space)
        .is_some()
    {}
    characters
        .peek()
        .map_or(value.len(), |(byte, _)| from + byte)
}

/// Where deleting a word from `from` goes to: the next or previous word, but not past the line break on the way, which
/// goes by itself when it's right there.
fn word_to_delete(value: &str, from: usize, direction: Direction) -> usize {
    match direction {
        Direction::Back => {
            if value[..from].ends_with('\n') {
                return from - 1;
            }
            let line_start = value[..from].rfind('\n').map_or(0, |at| at + 1);
            previous_word(value, from).max(line_start)
        }
        Direction::Forward => {
            if value[from..].starts_with('\n') {
                return from + 1;
            }
            let line_end = value[from..].find('\n').map_or(value.len(), |at| from + at);
            next_word(value, from).min(line_end)
        }
    }
}

/// The start of the word before, skipping any spaces on the way.
fn previous_word(value: &str, from: usize) -> usize {
    let mut at = from;
    let mut characters = value[..from].char_indices().rev().peekable();
    while let Some((byte, _)) =
        characters.next_if(|(_, character)| class_of(*character) == Class::Space)
    {
        at = byte;
    }
    let Some(&(_, first)) = characters.peek() else {
        return at;
    };
    let class = class_of(first);
    while let Some((byte, _)) = characters.next_if(|(_, character)| class_of(*character) == class) {
        at = byte;
    }
    at
}
