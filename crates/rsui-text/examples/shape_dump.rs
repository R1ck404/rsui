//! Shapes each line of a file, with one font, and writes out what comes of it, for comparing with other
//! programs. Sizes are in the font's own units, so nothing's lost to rounding.
//!
//! `shape_dump <font> <lines> <out>` writes a line of `glyph x y` triples for each line of text.

use std::fmt::Write;

use rsui_text::{Font, TextEngine, TextStyle};

fn main() {
    let mut arguments = std::env::args().skip(1);
    let (Some(font), Some(lines), Some(out)) =
        (arguments.next(), arguments.next(), arguments.next())
    else {
        eprintln!("usage: shape_dump <font> <lines> <out>");
        std::process::exit(2);
    };
    let font = Font::open(&font).expect("the font can be read");
    let units = f32::from(font.metrics().units_per_em);
    let mut engine = TextEngine::new(font);
    let style = TextStyle::new(units);
    let text = std::fs::read_to_string(lines).expect("the text can be read");
    let mut output = String::new();
    for line in text.lines() {
        let layout = engine.layout(line, style, f32::INFINITY);
        for glyph in &layout.glyphs {
            let _ = write!(
                output,
                "{} {} {} ",
                glyph.glyph,
                glyph.x.round(),
                glyph.y.round()
            );
        }
        output.push('\n');
        // Each line is by itself, and long runs of them don't need to be remembered.
        engine.forget_caches();
    }
    std::fs::write(out, output).expect("the result can be written");
}
