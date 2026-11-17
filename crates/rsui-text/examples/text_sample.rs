//! Draws a page of text to a PNG, for looking at what the rasterizer really produces.
//!
//! Run with `cargo run --release -p rsui-text --example text_sample -- output.png`.

use std::io::Write;

use rsui_text::{Font, Rasterizer, TextEngine, TextStyle};

const WIDTH: usize = 720;
const HEIGHT: usize = 520;
const PARAGRAPH: &str = "The quick brown fox jumps over the lazy dog. Sphinx of black quartz, \
judge my vow! AVATAR Type Wave. 0123456789 fi fl ffi. éàüñß €£¥ — “quoted” text.";

fn main() -> std::io::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "text_sample.png".into());
    let fonts = "C:/Windows/Fonts";
    let regular = Font::open(format!("{fonts}/segoeui.ttf")).expect("Segoe UI is installed");
    let bold = Font::open(format!("{fonts}/segoeuib.ttf")).expect("Segoe UI Bold is installed");
    let mut engine = TextEngine::new(regular).with_bold(bold);
    let mut rasterizer = Rasterizer::new();
    let mut pixels = vec![255u8; WIDTH * HEIGHT];

    let mut top = 16.0;
    for (size, bold) in [
        (11.0, false),
        (12.0, false),
        (13.0, false),
        (14.0, false),
        (14.0, true),
        (16.0, false),
        (20.0, false),
        (28.0, true),
    ] {
        let style = if bold {
            TextStyle::new(size).bold()
        } else {
            TextStyle::new(size)
        };
        let layout = engine.layout(PARAGRAPH, style, WIDTH as f32 - 32.0);
        for glyph in &layout.glyphs {
            let font = engine.font(glyph.font).expect("the glyph's font exists");
            let pen_x = 16.0 + glyph.x;
            // A quarter of a pixel at a time, as the atlas does.
            let snapped = (pen_x * 4.0).floor() / 4.0;
            let bitmap = rasterizer.rasterize(font, glyph.glyph, size, snapped.fract());
            let origin_x = snapped.floor() as i32 + bitmap.left;
            let origin_y = (top + glyph.y) as i32 + bitmap.top;
            for row in 0..bitmap.height as i32 {
                for column in 0..bitmap.width as i32 {
                    let (x, y) = (origin_x + column, origin_y + row);
                    if x < 0 || y < 0 || x >= WIDTH as i32 || y >= HEIGHT as i32 {
                        continue;
                    }
                    let coverage = bitmap.coverage[(row * bitmap.width as i32 + column) as usize];
                    let target = &mut pixels[y as usize * WIDTH + x as usize];
                    let blended = u32::from(*target) * (255 - u32::from(coverage)) / 255;
                    *target = blended as u8;
                }
            }
        }
        top += layout.height() + 12.0;
    }
    write_png(&path, WIDTH, HEIGHT, &pixels)
}

/// The smallest valid grayscale PNG writer: no compression, one row at a time.
fn write_png(path: &str, width: usize, height: usize, pixels: &[u8]) -> std::io::Result<()> {
    let mut raw = Vec::with_capacity((width + 1) * height);
    for row in pixels.chunks(width) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut zlib = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    while let Some(block) = blocks.next() {
        zlib.push(u8::from(blocks.peek().is_none()));
        zlib.extend((block.len() as u16).to_le_bytes());
        zlib.extend((!(block.len() as u16)).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in &raw {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    zlib.extend(((b << 16) | a).to_be_bytes());

    let mut header = Vec::new();
    header.extend((width as u32).to_be_bytes());
    header.extend((height as u32).to_be_bytes());
    header.extend([8, 0, 0, 0, 0]);

    let mut file = std::fs::File::create(path)?;
    file.write_all(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])?;
    for (kind, data) in [(b"IHDR", &header), (b"IDAT", &zlib), (b"IEND", &Vec::new())] {
        file.write_all(&(data.len() as u32).to_be_bytes())?;
        file.write_all(kind)?;
        file.write_all(data)?;
        let mut crc = u32::MAX;
        for byte in kind.iter().chain(data.iter()) {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        file.write_all(&(!crc).to_be_bytes())?;
    }
    Ok(())
}
