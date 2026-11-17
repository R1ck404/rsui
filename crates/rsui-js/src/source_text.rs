//! Where each function's source text is, for `Function.prototype.toString`, which gives it back exactly as it was
//! written, as the language says.
//!
//! Keeping every file's text for that would cost as much memory again as the text itself, for something that's rarely
//! asked. So each function keeps only where its text starts and ends (`CodeRoom::texts`, beside its 88 bytes), and which
//! source that's in, and the sources are kept as cheaply as each kind allows:
//!
//! - What's in the program, as the bundled modules are, is already there, and is only pointed to.
//! - A file is read again when it's asked for, through the same steps as when it was loaded, and only used if it's as
//!   long and hashes the same as what was compiled. If it's changed since, there's no text to give, and it says what a
//!   built-in function says. The last file read is kept until the next full collection, as a package can ask for many
//!   of its functions' text at once.
//! - Code given as a string, to `eval`, `new Function` or [`crate::Engine::eval`], is copied, as there's nowhere else
//!   to find it again. Like the rest, it goes once nothing compiled from it can run.

use std::hash::BuildHasher;
use std::path::PathBuf;
use std::rc::Rc;

use crate::text::Text;
use crate::vm::Vm;

/// No source: a function that was made up rather than written, or one whose source isn't kept.
pub(crate) const NO_SOURCE: u32 = u32::MAX;

pub(crate) enum SourceText {
    /// Its number is free to be given to another.
    Gone,
    Static(&'static str),
    Owned(Rc<str>),
    File { path: PathBuf, length: usize, hash: u64 },
}

/// Every source that functions' text is in, by number.
#[derive(Default)]
pub(crate) struct Sources {
    list: Vec<SourceText>,
    free: Vec<u32>,
    /// The last file read again, by its number.
    read: Option<(u32, Rc<str>)>,
}

/// A hash of bytes, as [`hash`] is of text, for checking the code cache's block isn't damaged.
pub(crate) fn hash_bytes(bytes: &[u8]) -> u64 {
    foldhash::fast::FixedState::with_seed(0x5eed).hash_one(bytes)
}

pub(crate) fn hash(text: &str) -> u64 {
    foldhash::fast::FixedState::with_seed(0x5eed).hash_one(text)
}

impl SourceText {
    /// A file's text, as it was compiled.
    pub(crate) fn file(path: PathBuf, text: &str) -> Self {
        Self::File {
            path,
            length: text.len(),
            hash: hash(text),
        }
    }
}

impl Vm {
    /// Keeps a source for the functions compiled from it, and says its number.
    pub(crate) fn add_source(&mut self, text: SourceText) -> u32 {
        let sources = &mut self.sources;
        match sources.free.pop() {
            Some(id) => {
                sources.list[id as usize] = text;
                id
            }
            None => {
                sources.list.push(text);
                (sources.list.len() - 1) as u32
            }
        }
    }

    /// What a function compiled from source was written as, if that can still be found.
    pub(crate) fn function_text(&mut self, proto: u32) -> Option<Text> {
        let (source, start, end) = self.text_of_proto(proto)?;
        let (start, end) = (start as usize, end as usize);
        // A function that was made up, as a file's own is, has no text.
        if start >= end {
            return None;
        }
        let whole: Rc<str> = match self.sources.list.get(source as usize)? {
            SourceText::Gone => return None,
            SourceText::Static(text) => return text.get(start..end).map(Text::from_str),
            SourceText::Owned(text) => text.clone(),
            SourceText::File { path, length, hash: expected } => match &self.sources.read {
                Some((id, text)) if *id == source => text.clone(),
                _ => {
                    let (path, length, expected) = (path.clone(), *length, *expected);
                    let text = crate::modules::read_source(&path).ok()?;
                    if text.len() != length || hash(&text) != expected {
                        return None;
                    }
                    let text: Rc<str> = Rc::from(text);
                    self.sources.read = Some((source, text.clone()));
                    text
                }
            },
        };
        // Build the VM's string storage directly: an intermediate Rust String would copy the same source twice.
        whole.get(start..end).map(Text::from_str)
    }

    /// Lets go of the sources no function that's left was compiled from, and of the file last read again. A full
    /// collection does this once it's freed the code that can't run.
    pub(crate) fn forget_unused_sources(&mut self, used: &[bool]) {
        self.sources.read = None;
        for (id, text) in self.sources.list.iter_mut().enumerate() {
            if !matches!(text, SourceText::Gone) && !used.get(id).copied().unwrap_or(false) {
                *text = SourceText::Gone;
                self.sources.free.push(id as u32);
            }
        }
        while matches!(self.sources.list.last(), Some(SourceText::Gone)) {
            self.sources.list.pop();
        }
        let count = self.sources.list.len() as u32;
        self.sources.free.retain(|id| *id < count);
        self.sources.free.sort_unstable_by(|a, b| b.cmp(a));
    }

    /// The source numbered `id`, for the code cache, which keeps what a file's functions' text is in.
    pub(crate) fn source_entry(&self, id: u32) -> Option<&SourceText> {
        self.sources.list.get(id as usize)
    }

    /// How many sources there are, counting the numbers that are free.
    pub(crate) fn source_count(&self) -> usize {
        self.sources.list.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_no_function_is_from_goes() {
        let mut vm = Vm::new();
        let base = vm.source_count();
        let a = vm.add_source(SourceText::Owned(Rc::from("a")));
        let b = vm.add_source(SourceText::Static("b"));
        let mut used = vec![true; base + 2];
        used[a as usize] = false;
        vm.forget_unused_sources(&used);
        assert!(matches!(vm.sources.list[a as usize], SourceText::Gone));
        assert!(matches!(vm.sources.list[b as usize], SourceText::Static("b")));
        assert_eq!(vm.add_source(SourceText::Static("c")), a);
    }
}
