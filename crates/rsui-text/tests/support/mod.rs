//! Builds a tiny font in code, so tests know exactly what's in it and don't depend on
//! any installed font.
//!
//! It has 1000 units per em, an ascent of 800, a descent of -200 and a line gap of 100:
//!
//! | Character | Glyph | Advance | Shape |
//! |---|---|---|---|
//! | (missing) | 0 | 300 | none |
//! | `A` | 1 | 600 | a square from (0, 0) to (500, 500) |
//! | `B` | 2 | 500 | a right triangle with sides of 500 |
//! | space | 3 | 250 | none |
//! | `C` | 4 | 800 | a circle of radius 400 around (400, 400), from curves |
//! | `D` | 5 | 700 | parts: a square moved right by 100, and a half-size triangle |

#![allow(dead_code)]

pub const UNITS: f32 = 1000.0;

fn u16s(values: &[u16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_be_bytes())
        .collect()
}

/// A glyph with one contour per list of `(x, y, on_curve)`.
fn simple_glyph(contours: &[&[(i16, i16, bool)]]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend((contours.len() as i16).to_be_bytes());
    out.extend([0u8; 8]); // The bounding box, which nothing here relies on.
    let mut end = 0i32;
    for contour in contours {
        end += contour.len() as i32;
        out.extend(((end - 1) as u16).to_be_bytes());
    }
    out.extend(0u16.to_be_bytes()); // No instructions.
    let points: Vec<_> = contours.iter().flat_map(|contour| contour.iter()).collect();
    for point in &points {
        out.push(u8::from(point.2));
    }
    let mut last = 0i16;
    for point in &points {
        out.extend((point.0 - last).to_be_bytes());
        last = point.0;
    }
    last = 0;
    for point in &points {
        out.extend((point.1 - last).to_be_bytes());
        last = point.1;
    }
    if out.len() % 2 == 1 {
        out.push(0);
    }
    out
}

fn composite_glyph() -> Vec<u8> {
    const WORDS: u16 = 1;
    const XY: u16 = 2;
    const SCALE: u16 = 8;
    const MORE: u16 = 32;
    let mut out = Vec::new();
    out.extend((-1i16).to_be_bytes());
    out.extend([0u8; 8]);
    // The square, moved right by 100.
    out.extend(u16s(&[WORDS | XY | MORE, 1, 100, 0]));
    // The triangle at half size. 0.5 is 8192 in 2.14 fixed point.
    out.extend(u16s(&[WORDS | XY | SCALE, 2, 0, 0, 8192]));
    out
}

/// A character map in format 4, for `A` to `D` and space.
fn cmap_format_4() -> Vec<u8> {
    let segments: [(u16, u16, i16); 4] = [
        (0x20, 0x20, 3 - 0x20),
        (0x41, 0x42, 1 - 0x41),
        (0x43, 0x44, 4 - 0x43),
        (0xFFFF, 0xFFFF, 1),
    ];
    let count = segments.len() as u16;
    let mut sub = u16s(&[4, 0, 0, count * 2, 0, 0, 0]);
    sub.extend(segments.iter().flat_map(|segment| segment.1.to_be_bytes()));
    sub.extend(0u16.to_be_bytes());
    sub.extend(segments.iter().flat_map(|segment| segment.0.to_be_bytes()));
    sub.extend(segments.iter().flat_map(|segment| segment.2.to_be_bytes()));
    sub.extend(segments.iter().flat_map(|_| 0u16.to_be_bytes()));
    let length = sub.len() as u16;
    sub[2..4].copy_from_slice(&length.to_be_bytes());
    sub
}

/// A character map in format 12, which also maps an emoji to the triangle.
fn cmap_format_12() -> Vec<u8> {
    let groups: [(u32, u32, u32); 4] = [
        (0x20, 0x20, 3),
        (0x41, 0x42, 1),
        (0x43, 0x44, 4),
        (0x1F600, 0x1F600, 2),
    ];
    let mut sub = u16s(&[12, 0]);
    sub.extend((16 + groups.len() as u32 * 12).to_be_bytes());
    sub.extend(0u32.to_be_bytes());
    sub.extend((groups.len() as u32).to_be_bytes());
    for group in groups {
        sub.extend(group.0.to_be_bytes());
        sub.extend(group.1.to_be_bytes());
        sub.extend(group.2.to_be_bytes());
    }
    sub
}

fn cmap(with_format_12: bool) -> Vec<u8> {
    let (encoding, sub) = if with_format_12 {
        (10, cmap_format_12())
    } else {
        (1, cmap_format_4())
    };
    let mut table = u16s(&[0, 1, 3, encoding]);
    table.extend(12u32.to_be_bytes());
    table.extend(sub);
    table
}

/// `AB` closer by 100, in an old-style kerning table.
fn kern_table() -> Vec<u8> {
    let mut table = u16s(&[0, 1, 0, 20, 1, 1, 6, 0, 0]);
    table.extend(u16s(&[1, 2]));
    table.extend((-100i16).to_be_bytes());
    table
}

/// `AB` closer by 50 through a list of pairs, and `BA` closer by 70 through classes.
fn gpos_table() -> Vec<u8> {
    let mut table = Vec::new();
    // Header: version, then offsets to the script, feature and lookup lists.
    table.extend(u16s(&[1, 0, 10, 30, 44]));
    // Script list at 10: one script, "latn", with a default language.
    table.extend(u16s(&[1]));
    table.extend(b"latn");
    table.extend(u16s(&[8]));
    // The script at 18, whose language follows it.
    table.extend(u16s(&[4, 0]));
    // The language at 22: no reordering table, no required feature, one feature.
    table.extend(u16s(&[0, 0xFFFF, 1, 0]));
    assert_eq!(table.len(), 30);
    // Feature list at 30: one feature, "kern".
    table.extend(u16s(&[1]));
    table.extend(b"kern");
    table.extend(u16s(&[8]));
    // The feature at 38: no parameters, two lookups.
    table.extend(u16s(&[0, 2, 0, 1]));
    assert_eq!(table.len(), 46);
    table.splice(8..10, 46u16.to_be_bytes()).for_each(drop);
    // Lookup list at 46: two lookups, then the lookups at offsets from here.
    let lookup_list = table.len();
    table.extend(u16s(&[2, 6, 0]));

    // First lookup: pair adjustment, with two subtables that both cover glyph 1. A pair
    // that isn't in the first must still be looked for in the second.
    table.extend(u16s(&[2, 0, 2, 10, 36]));
    for (second_glyph, value) in [(2u16, -50i16), (4, -30)] {
        // Format 1: coverage of glyph 1, moving the next glyph, with one set of pairs.
        let subtable = table.len();
        table.extend(u16s(&[1, 14, 4, 0, 1, 20]));
        assert_eq!(table.len() - subtable, 12);
        table.extend(u16s(&[0])); // Padding, to show offsets are honoured.
        // Coverage, 14 from the subtable: format 1, listing glyph 1.
        table.extend(u16s(&[1, 1, 1]));
        assert_eq!(table.len() - subtable, 20);
        // The set of pairs: just this one.
        table.extend(u16s(&[1, second_glyph]));
        table.extend(value.to_be_bytes());
        assert_eq!(table.len() - subtable, 26);
    }

    // Second lookup: pair adjustment by classes.
    let second = table.len();
    let offset = (second - lookup_list) as u16;
    table[lookup_list + 4..lookup_list + 6].copy_from_slice(&offset.to_be_bytes());
    table.extend(u16s(&[2, 0, 2, 10, 0]));
    let last_offset_at = table.len() - 2;
    let subtable = table.len();
    // Format 2: coverage at 24, classes at 30 and 40, two classes of each.
    table.extend(u16s(&[2, 24, 4, 0, 30, 40, 2, 2]));
    // Records for class pairs (0,0), (0,1), (1,0) and (1,1).
    for value in [0i16, 0, 0, -70] {
        table.extend(value.to_be_bytes());
    }
    assert_eq!(table.len() - subtable, 24);
    // Coverage in ranges: glyph 2 only.
    table.extend(u16s(&[2, 1, 2, 2, 0]));
    table.extend(u16s(&[0]));
    assert_eq!(table.len() - subtable, 36);
    let classes_1 = 36u16;
    table[subtable + 8..subtable + 10].copy_from_slice(&classes_1.to_be_bytes());
    // First classes in format 1: glyph 2 is class 1.
    table.extend(u16s(&[1, 2, 1, 1]));
    let classes_2 = (table.len() - subtable) as u16;
    table[subtable + 10..subtable + 12].copy_from_slice(&classes_2.to_be_bytes());
    // Second classes in format 2: glyph 1 is class 1.
    table.extend(u16s(&[2, 1, 1, 1, 1]));

    // A list of pairs after the classes, with a pair the classes give no adjustment to.
    // It must never apply: classes cover glyph 2, so they end this lookup.
    let last = (table.len() - second) as u16;
    table[last_offset_at..last_offset_at + 2].copy_from_slice(&last.to_be_bytes());
    table.extend(u16s(&[1, 12, 4, 0, 1, 18]));
    table.extend(u16s(&[1, 1, 2]));
    table.extend(u16s(&[1, 2]));
    table.extend((-99i16).to_be_bytes());
    table
}

#[derive(Clone, Copy, Default)]
pub struct Options {
    pub format_12: bool,
    pub kern: bool,
    pub gpos: bool,
    /// Makes `D` nothing but many references to itself, as a damaged font might.
    pub endless_composite: bool,
}

/// A composite glyph with 100 parts, all of which are glyph 5 itself.
fn self_referencing_glyph() -> Vec<u8> {
    const WORDS: u16 = 1;
    const XY: u16 = 2;
    const MORE: u16 = 32;
    let mut out = Vec::new();
    out.extend((-1i16).to_be_bytes());
    out.extend([0u8; 8]);
    for part in 0..100 {
        let more = if part < 99 { MORE } else { 0 };
        out.extend(u16s(&[WORDS | XY | more, 5, 0, 0]));
    }
    out
}

pub fn font_bytes(options: Options) -> Vec<u8> {
    let glyphs: Vec<Vec<u8>> = vec![
        Vec::new(),
        simple_glyph(&[&[
            (0, 0, true),
            (0, 500, true),
            (500, 500, true),
            (500, 0, true),
        ]]),
        simple_glyph(&[&[(0, 0, true), (0, 500, true), (500, 0, true)]]),
        Vec::new(),
        // Four control points in a row: every point on the curve is implied.
        simple_glyph(&[&[
            (0, 0, false),
            (0, 800, false),
            (800, 800, false),
            (800, 0, false),
        ]]),
        if options.endless_composite {
            self_referencing_glyph()
        } else {
            composite_glyph()
        },
    ];
    let advances = [300u16, 600, 500, 250, 800, 700];

    let mut loca = Vec::new();
    let mut glyf = Vec::new();
    for glyph in &glyphs {
        loca.extend((glyf.len() as u32).to_be_bytes());
        glyf.extend(glyph);
    }
    loca.extend((glyf.len() as u32).to_be_bytes());

    let mut head = vec![0u8; 54];
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[50..52].copy_from_slice(&1i16.to_be_bytes());

    let mut hhea = vec![0u8; 36];
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[8..10].copy_from_slice(&100i16.to_be_bytes());
    hhea[34..36].copy_from_slice(&(advances.len() as u16).to_be_bytes());

    let mut maxp = vec![0u8; 6];
    maxp[4..6].copy_from_slice(&(glyphs.len() as u16).to_be_bytes());

    let hmtx: Vec<u8> = advances
        .iter()
        .flat_map(|advance| u16s(&[*advance, 0]))
        .collect();

    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"cmap", cmap(options.format_12)),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
    ];
    if options.kern {
        tables.push((b"kern", kern_table()));
    }
    if options.gpos {
        tables.push((b"GPOS", gpos_table()));
    }

    let mut file = 0x0001_0000u32.to_be_bytes().to_vec();
    file.extend(u16s(&[tables.len() as u16, 0, 0, 0]));
    let mut offset = 12 + tables.len() * 16;
    for (tag, table) in &tables {
        file.extend(*tag);
        file.extend(0u32.to_be_bytes());
        file.extend((offset as u32).to_be_bytes());
        file.extend((table.len() as u32).to_be_bytes());
        offset += table.len();
    }
    for (_, table) in tables {
        file.extend(table);
    }
    file
}

pub fn test_font(options: Options) -> rsui_text::Font {
    rsui_text::Font::from_bytes(font_bytes(options)).expect("the test font is valid")
}

pub fn plain_font() -> rsui_text::Font {
    test_font(Options::default())
}

/// An installed font, for the tests that want real data. These only run if it's there.
pub fn system_font_bytes(name: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("C:/Windows/Fonts/{name}")).ok()
}
