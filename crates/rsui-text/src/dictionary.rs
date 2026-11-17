//! Where lines can end in Thai, Lao, Khmer and Burmese, which don't put spaces between words, so that
//! takes knowing the words. Windows has dictionaries for them, in its copy of ICU, which is only loaded
//! when a paragraph in one of these languages has to wrap. Without it, Thai and Lao can still break
//! before a vowel written ahead of its consonant, and anything else breaks when it runs out of room.

/// Whether finding the words in this character's language takes a dictionary.
#[inline]
pub(crate) fn needs_dictionary(character: char) -> bool {
    matches!(character as u32,
        0x0E00..=0x0EFF | 0x1000..=0x109F | 0x1780..=0x17FF | 0xA9E0..=0xA9FF | 0xAA60..=0xAA7F)
}

/// Which characters of a paragraph can start a line, by being the start of a word in one of those
/// languages, from first to last.
pub(crate) fn word_starts(characters: &[(u32, char)]) -> Vec<usize> {
    let mut starts = system::line_starts(characters).unwrap_or_else(|| by_rule(characters));
    // Everything else already has its rules. These are only for where they can't say.
    starts.retain(|at| {
        *at > 0
            && *at < characters.len()
            && (needs_dictionary(characters[*at - 1].1) || needs_dictionary(characters[*at].1))
    });
    starts
}

/// What can be told without a dictionary.
fn by_rule(characters: &[(u32, char)]) -> Vec<usize> {
    characters
        .iter()
        .enumerate()
        .filter(
            |(_, (_, character))| matches!(*character as u32, 0x0E40..=0x0E44 | 0x0EC0..=0x0EC4),
        )
        .map(|(at, _)| at)
        .collect()
}

#[cfg(windows)]
mod system {
    use std::ffi::{c_char, c_void};
    use std::sync::OnceLock;

    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x800;
    /// Breaks where a line can end, rather than between words or characters.
    const BREAK_LINE: i32 = 2;
    const DONE: i32 = -1;

    type Open = unsafe extern "C" fn(i32, *const c_char, *const u16, i32, *mut i32) -> *mut c_void;
    type Next = unsafe extern "C" fn(*mut c_void) -> i32;
    type Close = unsafe extern "C" fn(*mut c_void);

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    #[derive(Clone, Copy)]
    struct Icu {
        open: Open,
        next: Next,
        close: Close,
    }

    /// Finds Windows's copy of ICU. It's been there since 2017, under the second name at first.
    fn icu() -> Option<Icu> {
        static ICU: OnceLock<Option<Icu>> = OnceLock::new();
        *ICU.get_or_init(|| {
            ["icu.dll", "icuuc.dll"].iter().find_map(|name| {
                let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
                // SAFETY: the name ends in a zero, and it's only looked for among Windows's own files.
                let module = unsafe {
                    LoadLibraryExW(
                        wide.as_ptr(),
                        std::ptr::null_mut(),
                        LOAD_LIBRARY_SEARCH_SYSTEM32,
                    )
                };
                if module.is_null() {
                    return None;
                }
                let find = |name: &[u8]| {
                    // SAFETY: the module is loaded, and stays so, and each name ends in a zero.
                    let found = unsafe { GetProcAddress(module, name.as_ptr().cast()) };
                    (!found.is_null()).then_some(found)
                };
                let (open, next, close) = (
                    find(b"ubrk_open\0")?,
                    find(b"ubrk_next\0")?,
                    find(b"ubrk_close\0")?,
                );
                // SAFETY: these are the types ICU has always declared these with.
                unsafe {
                    Some(Icu {
                        open: std::mem::transmute::<*mut c_void, Open>(open),
                        next: std::mem::transmute::<*mut c_void, Next>(next),
                        close: std::mem::transmute::<*mut c_void, Close>(close),
                    })
                }
            })
        })
    }

    /// Asks ICU where lines can start. `None` if it can't say.
    pub(super) fn line_starts(characters: &[(u32, char)]) -> Option<Vec<usize>> {
        let icu = icu()?;
        let mut text: Vec<u16> = Vec::with_capacity(characters.len());
        // Where each unit of its text is in the paragraph.
        let mut character_at: Vec<u32> = Vec::with_capacity(characters.len());
        let mut units = [0u16; 2];
        for (index, (_, character)) in characters.iter().enumerate() {
            for unit in character.encode_utf16(&mut units) {
                text.push(*unit);
                character_at.push(index as u32);
            }
        }
        let length = i32::try_from(text.len()).ok()?;
        let mut status = 0;
        // SAFETY: the text stays put until this is closed below, and the empty name means no language
        // in particular, as the text itself says which dictionaries it needs.
        let breaker = unsafe {
            (icu.open)(
                BREAK_LINE,
                c"".as_ptr(),
                text.as_ptr(),
                length,
                &raw mut status,
            )
        };
        if status > 0 || breaker.is_null() {
            return None;
        }
        let mut starts = Vec::new();
        loop {
            // SAFETY: it's open, and this is only asked of it from here.
            let at = unsafe { (icu.next)(breaker) };
            if at == DONE {
                break;
            }
            if let Some(index) = usize::try_from(at).ok().and_then(|at| character_at.get(at)) {
                starts.push(*index as usize);
            }
        }
        // SAFETY: it's open, and isn't used again.
        unsafe { (icu.close)(breaker) };
        Some(starts)
    }
}

#[cfg(not(windows))]
mod system {
    pub(super) fn line_starts(_characters: &[(u32, char)]) -> Option<Vec<usize>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn characters(text: &str) -> Vec<(u32, char)> {
        text.char_indices()
            .map(|(byte, c)| (byte as u32, c))
            .collect()
    }

    #[test]
    fn a_vowel_written_first_can_start_a_line_even_with_no_dictionary() {
        // Pa, Sara E, Ka: the E is said after the Ka, but is written before it.
        let text = characters("\u{0E1B}\u{0E40}\u{0E01}");
        assert_eq!(by_rule(&text), vec![1]);
    }

    #[test]
    fn only_these_languages_need_a_dictionary() {
        assert!(needs_dictionary('\u{0E01}'));
        assert!(needs_dictionary('\u{0E81}'));
        assert!(needs_dictionary('\u{1780}'));
        assert!(needs_dictionary('\u{1000}'));
        assert!(!needs_dictionary('a'));
        assert!(!needs_dictionary('\u{0915}'));
        assert!(!needs_dictionary('\u{4E00}'));
    }

    #[test]
    fn words_in_thai_are_found_by_the_dictionary_if_windows_has_it() {
        // "Sawasdee" and "khrap", which have nothing between them.
        let word = "\u{0E2A}\u{0E27}\u{0E31}\u{0E2A}\u{0E14}\u{0E35}";
        let text: String = format!("{word}\u{0E04}\u{0E23}\u{0E31}\u{0E1A}");
        let chars = characters(&text);
        if system::line_starts(&chars).is_none() {
            return;
        }
        assert_eq!(word_starts(&chars), vec![word.chars().count()]);
    }

    #[test]
    fn breaks_in_other_languages_are_left_to_the_other_rules() {
        let chars = characters("hello there \u{0E01}");
        // The one after "hello " is nothing to do with this, but the one next to Thai is.
        assert!(word_starts(&chars).iter().all(|at| *at >= 12));
    }
}
