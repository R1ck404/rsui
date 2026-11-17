//! Reads an image and writes out what was read, for comparing with other programs.
//!
//! `dump <file> <out>` writes each frame as a little-endian width, height and delay, followed by red,
//! green, blue and alpha with colors multiplied by alpha.

use std::io::Write;

fn main() {
    let mut arguments = std::env::args().skip(1);
    let (Some(path), Some(out)) = (arguments.next(), arguments.next()) else {
        eprintln!("usage: dump <file> <out>");
        std::process::exit(2);
    };
    let bytes = std::fs::read(&path).expect("the file can be read");
    let animation = match rsui_image::decode_animation(&bytes, (u32::MAX, u32::MAX)) {
        Ok(animation) => animation,
        Err(error) => {
            eprintln!("{path}: {error}");
            std::process::exit(1);
        }
    };
    let mut file = std::io::BufWriter::new(std::fs::File::create(&out).expect("it can be written"));
    for frame in &animation.frames {
        file.write_all(&animation.width.to_le_bytes()).unwrap();
        file.write_all(&animation.height.to_le_bytes()).unwrap();
        file.write_all(&frame.delay.to_le_bytes()).unwrap();
        file.write_all(&frame.pixels).unwrap();
    }
}
