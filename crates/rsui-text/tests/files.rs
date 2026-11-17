//! Fonts that aren't on the disk, as a packed app's aren't.

use std::path::Path;

use rsui_text::{Font, read_file, read_files_with};

fn inside_the_program(path: &Path) -> Option<Vec<u8>> {
    (path == Path::new(r"C:\packed\app\font.ttf"))
        .then(|| std::fs::read(r"C:\Windows\Fonts\arial.ttf").expect("Windows has Arial"))
}

#[test]
fn a_font_is_read_from_where_files_are_said_to_be_before_the_disk() {
    read_files_with(inside_the_program);
    let font = Font::open(r"C:\packed\app\font.ttf").expect("it's inside the program");
    assert!(font.glyph('A').is_some_and(|glyph| glyph != 0));
    // It isn't on the disk for anything else to read it from.
    assert!(font.file_path().is_none());
    // What isn't inside it is on the disk, as it would be.
    assert!(Font::open(r"C:\Windows\Fonts\arial.ttf").is_ok());
    assert!(Font::open(r"C:\packed\app\other.ttf").is_err());
    assert!(read_file(Path::new(r"C:\packed\app\font.ttf")).is_ok_and(|bytes| bytes.len() > 1000));
}
