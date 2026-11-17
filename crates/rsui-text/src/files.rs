//! Files an app reads for itself, such as its fonts and pictures, which a packed app has inside it rather than on the
//! disk.

use std::path::Path;
use std::sync::OnceLock;

/// What reads a file, if it has it.
pub type FileReader = fn(&Path) -> Option<Vec<u8>>;

/// Where files are read from before the disk, if anywhere.
static READER: OnceLock<FileReader> = OnceLock::new();

/// Has an app's files be read by `reader` first, and from the disk only if it doesn't have them. Only the first of
/// these counts.
pub fn read_files_with(reader: FileReader) {
    let _ = READER.set(reader);
}

/// What's in a file, from where [`read_files_with`] says, if it has it, and otherwise from the disk.
pub fn read_file(path: &Path) -> std::io::Result<Vec<u8>> {
    match read_by_reader(path) {
        Some(bytes) => Ok(bytes),
        None => std::fs::read(path),
    }
}

/// What's in a file, if it's somewhere [`read_files_with`] says to read it from.
pub fn read_by_reader(path: &Path) -> Option<Vec<u8>> {
    READER.get().and_then(|reader| reader(path))
}
