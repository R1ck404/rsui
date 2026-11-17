//! Where lines of text end.

/// Lines are broken only if text is wider than this much over the limit. A width
/// measured earlier can pick up a tiny rounding error on its way back as a limit, and
/// that mustn't wrap text that was measured to fit.
pub(crate) const WIDTH_TOLERANCE: f32 = 0.01;

/// One line, found by [`next_line`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LineEnd {
    /// Where the line's visible text ends, as a byte position.
    pub(crate) end: usize,
    /// Where the next line starts, after any spaces or line break.
    pub(crate) next: usize,
    pub(crate) width: f32,
    /// Whether this line ends the text.
    pub(crate) last: bool,
}

/// Whether a line may end after this character.
pub(crate) fn can_break_after(character: char) -> bool {
    matches!(
        character,
        ' ' | '\t' | '-' | '\u{05BE}' | '\u{2010}' | '\u{2013}' | '\u{2014}' | '\u{200B}'
    ) || is_ideograph(character)
}

/// Whether a line may end between these two characters: where [`can_break_after`] says, and after a `?` or `!` that has a
/// word straight after it, as a browser breaks `lmnop?qrs`, which Unicode's rules for lines allow. A `/` isn't one, as a
/// browser keeps `defghijk/lmnop` together.
pub(crate) fn can_break_between(before: char, after: char) -> bool {
    can_break_after(before) || (matches!(before, '?' | '!') && after.is_alphanumeric())
}

/// Whether a line may start at this character regardless of what came before. Text in
/// these scripts doesn't use spaces, so lines can end between any two characters.
pub(crate) fn is_ideograph(character: char) -> bool {
    matches!(character as u32,
        0x2E80..=0x303F | 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF |
        0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FFFF)
}

pub(crate) fn is_space(character: char) -> bool {
    matches!(character, ' ' | '\t')
}

/// Finds the end of the line starting at character `start`, given how wide each character is. It
/// says where the line's visible characters end, and where the next line starts. A line can't
/// break just before something with no width, as that's a mark on the character before it. It can
/// also break at any of `word_starts`, for languages where the characters themselves can't tell.
pub(crate) fn next_measured_line(
    characters: &[(u32, char)],
    widths: &[f32],
    start: usize,
    max_width: f32,
    word_starts: &[usize],
) -> (usize, usize) {
    let limit = max_width + WIDTH_TOLERANCE;
    let mut width = 0.0;
    let mut last_break = None;
    let mut visible_end = start;
    for index in start..characters.len() {
        let character = characters[index].1;
        let new_width = width + widths[index];
        if !is_space(character) && new_width > limit && widths[index] > 0.0 {
            if let Some(found) = last_break {
                return found;
            }
            if index > start {
                return (index, index);
            }
        }
        width = new_width;
        let next_has_width = widths.get(index + 1).is_some_and(|width| *width > 0.0);
        if is_space(character) {
            last_break = Some((visible_end, index + 1));
        } else {
            visible_end = index + 1;
            let next_is_ideograph = characters
                .get(index + 1)
                .is_some_and(|next| is_ideograph(next.1));
            let starts_word =
                !word_starts.is_empty() && word_starts.binary_search(&(index + 1)).is_ok();
            let breaks = characters
                .get(index + 1)
                .map_or_else(|| can_break_after(character), |next| can_break_between(character, next.1));
            if next_has_width && (breaks || next_is_ideograph || starts_word) {
                last_break = Some((visible_end, visible_end));
            }
        }
    }
    (visible_end, characters.len())
}

/// Finds the end of the line starting at `start`.
///
/// `advance` gives the width of a character given the one before it on this line, which
/// lets kerning apply. Spaces at the end of a line don't count towards its width, so
/// they can't cause a wrap. A word too long for a line breaks between characters, and a
/// line always holds at least one character, so this always moves forward.
/// With `anywhere`, as CSS's `word-break: break-all` has it, a line that's full ends where it is, in a word or not.
pub(crate) fn next_line(
    text: &str,
    start: usize,
    max_width: f32,
    anywhere: bool,
    advance: &mut impl FnMut(Option<char>, char) -> f32,
) -> LineEnd {
    let limit = max_width + WIDTH_TOLERANCE;
    let mut width = 0.0;
    let mut previous = None;
    // The text up to the last place a line could end.
    let mut last_break: Option<(usize, usize, f32)> = None;
    // The width and end of the text before any spaces being passed over.
    let mut visible_width = 0.0;
    let mut visible_end = start;

    let mut characters = text[start..].char_indices();
    while let Some((offset, character)) = characters.next() {
        let position = start + offset;
        if character == '\n' {
            return LineEnd {
                end: visible_end,
                next: position + 1,
                width: visible_width,
                last: false,
            };
        }
        if character == '\r' {
            continue;
        }
        let new_width = width + advance(previous, character);
        if !is_space(character) && new_width > limit {
            if let Some((end, next, break_width)) =
                last_break.filter(|(_, next, _)| !anywhere || *next == position)
            {
                return LineEnd {
                    end,
                    next,
                    width: break_width,
                    last: false,
                };
            }
            // No place to break, so split the word, unless this is the first character.
            if position > start {
                return LineEnd {
                    end: position,
                    next: position,
                    width,
                    last: false,
                };
            }
        }
        width = new_width;
        previous = Some(character);
        if is_space(character) {
            last_break = Some((visible_end, position + character.len_utf8(), visible_width));
        } else {
            visible_width = width;
            visible_end = position + character.len_utf8();
            let next = characters.clone().next().map(|(_, c)| c);
            if next.map_or_else(|| can_break_after(character), |next| can_break_between(character, next)) {
                last_break = Some((visible_end, visible_end, visible_width));
            }
        }
        // A character that can start a line can also end the one before it.
        if let Some(next_character) = characters.clone().next().map(|(_, c)| c) {
            if is_ideograph(next_character) && !is_space(character) {
                last_break = Some((visible_end, visible_end, visible_width));
            }
        }
    }
    LineEnd {
        end: visible_end,
        next: text.len(),
        width: visible_width,
        last: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_can_end_after_a_question_mark_but_not_a_slash() {
        assert!(can_break_between('-', 'a'));
        assert!(can_break_between('?', 'q'));
        assert!(!can_break_between('?', '"'));
        assert!(!can_break_between('/', 'l'));
        assert!(!can_break_between('a', 'b'));
    }
}
