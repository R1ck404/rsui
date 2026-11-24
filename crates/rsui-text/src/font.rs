//! Reads TrueType fonts.
//!
//! Font files aren't trusted, so every read here is checked, and a damaged font gives
//! wrong results or an error at worst, never a panic or an endless loop.

use std::fmt;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::data::FontData;
use crate::gpos::PairKerning;
use crate::variations::{self, NamedInstance, Variation, VariationAxis};

/// How many parts one composite glyph may have, across all its levels. A part can itself
/// have parts, even including the glyph it's part of, so without a limit a small damaged
/// font could take forever. This also limits how deep they can go.
const MAX_COMPONENTS: u32 = 64;

pub(crate) fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = data.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

pub(crate) fn read_i16(data: &[u8], offset: usize) -> Option<i16> {
    read_u16(data, offset).map(|value| value as i16)
}

pub(crate) fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Why a font couldn't be loaded.
#[derive(Debug)]
pub enum FontError {
    Io(io::Error),
    /// The data isn't a font, or is damaged. This names the part that's wrong.
    Invalid(&'static str),
    /// A valid font of a kind this doesn't read. PostScript outlines are the usual reason.
    Unsupported(&'static str),
}

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "couldn't read the font: {error}"),
            Self::Invalid(what) => write!(f, "the font is damaged: {what}"),
            Self::Unsupported(what) => write!(f, "this kind of font isn't supported: {what}"),
        }
    }
}

impl std::error::Error for FontError {}

/// A font's vertical measurements, in font units. Multiply by `size / units_per_em` to
/// get pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontMetrics {
    pub units_per_em: u16,
    /// How far text rises above the baseline.
    pub ascent: i16,
    /// How far text falls below the baseline, as a negative number.
    pub descent: i16,
    /// Extra space between lines.
    pub line_gap: i16,
    /// How tall a lowercase `x` is, as its `OS/2` table says, which is what `vertical-align: middle` goes by, or 0 if it doesn't.
    pub x_height: i16,
}

/// Receives a glyph's outline, in font units, with y going up.
pub trait OutlineSink {
    fn move_to(&mut self, x: f32, y: f32);
    fn line_to(&mut self, x: f32, y: f32);
    fn quad_to(&mut self, control_x: f32, control_y: f32, x: f32, y: f32);
    fn close(&mut self);
}

#[derive(Clone, Copy)]
enum CharMap {
    /// Format 4: ranges of 16-bit characters.
    Segments { offset: u32, count: u16 },
    /// Format 12: ranges covering all of Unicode.
    Groups { offset: u32, count: u32 },
}

/// A loaded font. The file itself stays where it is, so this is small however large the
/// font is.
pub struct Font {
    data: FontData,
    /// Which font this is in its file, which can hold several.
    face: u32,
    /// The file it was opened from, if it's still as it is there.
    path: Option<PathBuf>,
    metrics: FontMetrics,
    /// What its `OS/2` table says text rises above and falls below the baseline as type is set, the second as a negative
    /// number, if it says.
    typo: Option<(i16, i16)>,
    glyph_count: u16,
    h_metric_count: u16,
    long_offsets: bool,
    hmtx: Range<usize>,
    loca: Range<usize>,
    glyf: Range<usize>,
    char_map: CharMap,
    kern: Option<Range<usize>>,
    gpos: Option<PairKerning>,
    gsub_table: Option<Range<usize>>,
    gpos_table: Option<Range<usize>>,
    gdef_table: Option<Range<usize>>,
    color: Option<ColorTables>,
    /// Where it says which glyphs a character has with a variation selector after it.
    variations: Option<usize>,
    hhea: Range<usize>,
    os2: Option<Range<usize>>,
    name: Option<Range<usize>>,
    stat: Option<Range<usize>>,
    /// The axes it varies along and where it's set on them, for a font that varies.
    vary: Option<Box<Variation>>,
}

/// One flat shape of an emoji or other colored glyph. Each is drawn over the ones before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorLayer {
    pub glyph: u16,
    /// Its color, as red, green, blue and how opaque. `None` is the color of the text.
    pub color: Option<[u8; 4]>,
}

/// Where a font keeps which shapes make up each colored glyph.
struct ColorTables {
    /// Six bytes for each glyph that has shapes, sorted by glyph.
    glyphs: Range<usize>,
    /// Four bytes for each shape.
    layers: Range<usize>,
    /// Four bytes for each color of the first palette.
    palette: Range<usize>,
    /// The whole table, and where its lists for glyphs made of gradients are in it, if it has them.
    table: Range<usize>,
    gradients: Option<(usize, usize, Option<usize>)>,
}

impl ColorTables {
    fn read(bytes: &[u8], colr: &Range<usize>, cpal: &Range<usize>) -> Option<Self> {
        let table = bytes.get(colr.clone())?;
        let glyph_count = read_u16(table, 2)? as usize;
        let glyphs_at = read_u32(table, 4)? as usize;
        let layers_at = read_u32(table, 8)? as usize;
        let layer_count = read_u16(table, 12)? as usize;
        let palettes = bytes.get(cpal.clone())?;
        let entries = read_u16(palettes, 2)? as usize;
        let colors_at = read_u32(palettes, 8)? as usize;
        let first_color = read_u16(palettes, 12)? as usize;
        let range = |table: &Range<usize>, at: usize, length: usize| {
            let start = table.start.checked_add(at)?;
            let end = start.checked_add(length)?;
            (end <= table.end).then_some(start..end)
        };
        let offset = |at: usize| {
            read_u32(table, at)
                .map(|value| value as usize)
                .filter(|at| *at != 0)
        };
        let gradients = if read_u16(table, 0)? >= 1 {
            offset(14).map(|list| (list, offset(18).unwrap_or(0), offset(22)))
        } else {
            None
        };
        Some(Self {
            table: colr.clone(),
            gradients,
            glyphs: range(colr, glyphs_at, glyph_count * 6)?,
            layers: range(colr, layers_at, layer_count * 4)?,
            palette: range(cpal, colors_at + first_color * 4, entries * 4)?,
        })
    }
}

struct Tables {
    head: Range<usize>,
    hhea: Range<usize>,
    maxp: Range<usize>,
    hmtx: Range<usize>,
    cmap: Range<usize>,
    loca: Option<Range<usize>>,
    glyf: Option<Range<usize>>,
    kern: Option<Range<usize>>,
    gpos: Option<Range<usize>>,
    gsub: Option<Range<usize>>,
    gdef: Option<Range<usize>>,
    colr: Option<Range<usize>>,
    cpal: Option<Range<usize>>,
    os2: Option<Range<usize>>,
    fvar: Option<Range<usize>>,
    avar: Option<Range<usize>>,
    gvar: Option<Range<usize>>,
    hvar: Option<Range<usize>>,
    mvar: Option<Range<usize>>,
    stat: Option<Range<usize>>,
    name: Option<Range<usize>>,
}

/// A `.woff` file is the font's tables, each packed by itself, and this puts them back as they'd be in a font file.
fn unpack_woff(bytes: &[u8]) -> Result<Vec<u8>, FontError> {
    const TOO_SHORT: FontError = FontError::Invalid("the .woff file is too short");
    /// Far more than any font, for a file that says it's enormous.
    const MOST: usize = 256 * 1024 * 1024;
    let long = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|four| u32::from_be_bytes([four[0], four[1], four[2], four[3]]))
    };
    let flavor = long(4).ok_or(TOO_SHORT)?;
    let count = usize::from(read_u16(bytes, 12).ok_or(TOO_SHORT)?);
    let mut out = Vec::new();
    out.extend_from_slice(&flavor.to_be_bytes());
    out.extend_from_slice(&(count as u16).to_be_bytes());
    let largest_power = count.max(1).ilog2();
    let search = (1u16 << largest_power).wrapping_mul(16);
    out.extend_from_slice(&search.to_be_bytes());
    out.extend_from_slice(&(largest_power as u16).to_be_bytes());
    out.extend_from_slice(&((count as u16).wrapping_mul(16).wrapping_sub(search)).to_be_bytes());
    let directory = out.len();
    out.resize(directory + count * 16, 0);
    for index in 0..count {
        let entry = 44 + index * 20;
        let (offset, packed, size, checksum) = (
            long(entry + 4).ok_or(TOO_SHORT)? as usize,
            long(entry + 8).ok_or(TOO_SHORT)? as usize,
            long(entry + 12).ok_or(TOO_SHORT)? as usize,
            long(entry + 16).ok_or(TOO_SHORT)?,
        );
        let held = bytes
            .get(offset..offset.checked_add(packed).ok_or(TOO_SHORT)?)
            .ok_or(TOO_SHORT)?;
        let place = out.len() as u32;
        if packed < size {
            let table = rsui_image::inflate_zlib(held, MOST).ok_or(FontError::Invalid(
                "a table in the .woff file can't be unpacked",
            ))?;
            out.extend_from_slice(&table);
        } else {
            out.extend_from_slice(held);
        }
        // Each table starts on a multiple of 4.
        out.resize(out.len().next_multiple_of(4), 0);
        let at = directory + index * 16;
        out[at..at + 4].copy_from_slice(&bytes[entry..entry + 4]);
        out[at + 4..at + 8].copy_from_slice(&checksum.to_be_bytes());
        out[at + 8..at + 12].copy_from_slice(&place.to_be_bytes());
        out[at + 12..at + 16].copy_from_slice(&(size as u32).to_be_bytes());
    }
    Ok(out)
}

impl Font {
    /// Opens a font file. For a file holding a collection of fonts, this is the first.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, FontError> {
        Self::open_face(path, 0)
    }

    /// Opens a font from a file holding a collection of fonts, counting from 0.
    pub fn open_face(path: impl AsRef<Path>, face: u32) -> Result<Self, FontError> {
        let data = FontData::open(path.as_ref()).map_err(FontError::Io)?;
        if data.bytes().starts_with(b"wOFF") {
            return Self::from_bytes(unpack_woff(data.bytes())?);
        }
        // What's only in memory, as what's inside a packed app is, has no file for others to read it from.
        let on_the_disk = data.is_on_the_disk();
        let mut font = Self::parse(data, face)?;
        if on_the_disk {
            font.path = Some(path.as_ref().to_owned());
        }
        Ok(font)
    }

    /// Reads a font from its bytes.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, FontError> {
        Self::parse(FontData::Owned(bytes.into()), 0)
    }

    /// Reads a font built into the program, with `include_bytes!`.
    pub fn from_static(bytes: &'static [u8]) -> Result<Self, FontError> {
        Self::parse(FontData::Static(bytes), 0)
    }

    fn parse(data: FontData, face: u32) -> Result<Self, FontError> {
        let bytes = data.bytes();
        let tables = read_tables(bytes, face)?;
        let table = |range: &Range<usize>| &bytes[range.clone()];

        let head = table(&tables.head);
        let units_per_em =
            read_u16(head, 18).ok_or(FontError::Invalid("the head table is too short"))?;
        if units_per_em == 0 {
            return Err(FontError::Invalid("the head table has no units per em"));
        }
        let long_offsets =
            read_i16(head, 50).ok_or(FontError::Invalid("the head table is too short"))? != 0;

        let hhea = table(&tables.hhea);
        let too_short = || FontError::Invalid("the hhea table is too short");
        let h_metric_count = read_u16(hhea, 34).ok_or_else(too_short)?;
        let metrics = vertical_metrics(
            bytes,
            units_per_em,
            &tables.hhea,
            tables.os2.as_ref(),
            &|_| 0.0,
        )
        .ok_or_else(too_short)?;

        let glyph_count = read_u16(table(&tables.maxp), 4)
            .ok_or(FontError::Invalid("the maxp table is too short"))?;

        let char_map = read_char_map(table(&tables.cmap))
            .ok_or(FontError::Invalid("no usable character map"))?;

        let (Some(loca), Some(glyf)) = (tables.loca, tables.glyf) else {
            return Err(FontError::Unsupported("it has no TrueType outlines"));
        };

        let gpos = tables
            .gpos
            .as_ref()
            .and_then(|range| PairKerning::read(table(range)));

        let variations = read_variations(table(&tables.cmap)).map(|at| tables.cmap.start + at);
        let color = match (&tables.colr, &tables.cpal) {
            (Some(colr), Some(cpal)) => ColorTables::read(bytes, colr, cpal),
            _ => None,
        };

        let typo = tables.os2.as_ref().map(&table).and_then(|os2| Some((read_i16(os2, 68)?, read_i16(os2, 70)?)));
        let vary = tables.fvar.clone().and_then(|fvar| {
            let axes = variations::axes(table(&fvar));
            if axes.is_empty() {
                return None;
            }
            // `GDEF`'s store, from version 1.3 on.
            let gdef_store = tables.gdef.clone().and_then(|gdef| {
                let at = (read_u32(table(&gdef), 0)? >= 0x0001_0003)
                    .then(|| read_u32(table(&gdef), 14))??;
                (at != 0).then(|| gdef.start + at as usize..gdef.end)
            });
            let values = axes.iter().map(|axis| axis.default).collect();
            let coordinates = vec![0; axes.len()];
            Some(Box::new(Variation {
                axes,
                values,
                coordinates,
                fvar,
                avar: tables.avar.clone(),
                gvar: tables.gvar.clone(),
                hvar: tables.hvar.clone(),
                mvar: tables.mvar.clone(),
                gdef_store,
                hvar_scalars: Vec::new(),
                mvar_scalars: Vec::new(),
                gdef_scalars: Vec::new(),
            }))
        });
        Ok(Self {
            face,
            path: None,
            typo,
            variations,
            color,
            metrics,
            hhea: tables.hhea.clone(),
            os2: tables.os2.clone(),
            name: tables.name.clone(),
            stat: tables.stat.clone(),
            vary,
            glyph_count,
            h_metric_count,
            long_offsets,
            hmtx: tables.hmtx,
            loca,
            glyf,
            char_map: rebase(char_map, tables.cmap.start),
            kern: tables.kern,
            gpos: gpos.map(|gpos| gpos.at(tables.gpos.as_ref().map_or(0, |range| range.start))),
            gsub_table: tables.gsub,
            gpos_table: tables.gpos,
            gdef_table: tables.gdef,
            data,
        })
    }

    pub fn metrics(&self) -> FontMetrics {
        self.metrics
    }

    /// How far above and below the baseline type is set, as its `OS/2` table says, the second as a negative number, if it says.
    pub fn typo_metrics(&self) -> Option<(i16, i16)> {
        self.typo.filter(|(above, below)| *above > 0 || *below < 0)
    }

    /// The whole of the file this font came from, which can hold others too, as what draws it some other way needs it.
    pub fn file_bytes(&self) -> &[u8] {
        self.data.bytes()
    }

    /// The file this font was opened from, if it was, and that has what [`Self::file_bytes`] does.
    pub fn file_path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Which font this is in its file, counting from 0.
    pub fn face_index(&self) -> u32 {
        self.face
    }

    pub fn glyph_count(&self) -> u16 {
        self.glyph_count
    }

    /// The axes this font varies along, such as weight, in the order its `fvar` table has them, or none for a font that
    /// doesn't vary.
    pub fn variation_axes(&self) -> &[VariationAxis] {
        self.vary.as_ref().map_or(&[], |vary| &vary.axes)
    }

    /// Where this font is set along each of its axes, by tag, which is each one's default until
    /// [`Self::set_variations`] says otherwise.
    pub fn variations(&self) -> impl Iterator<Item = ([u8; 4], f32)> + '_ {
        self.vary.iter().flat_map(|vary| {
            vary.axes
                .iter()
                .zip(&vary.values)
                .map(|(axis, value)| (axis.tag, *value))
        })
    }

    /// Whether this font is set anywhere along its axes but at their defaults, so that its glyphs, advances and
    /// measurements aren't the ones in its tables as they are.
    pub fn is_varied(&self) -> bool {
        self.moved().is_some()
    }

    /// Sets this font along its axes, such as `(*b"wght", 600.0)` for semibold, each within what the font has, and
    /// leaving those not given at their defaults. A font that doesn't vary stays as it is.
    pub fn set_variations(&mut self, settings: &[([u8; 4], f32)]) {
        let Some(mut vary) = self.vary.take() else {
            return;
        };
        vary.set(self.bytes(), settings);
        let bytes = self.bytes();
        let mvar = vary.mvar.clone().and_then(|range| bytes.get(range));
        let delta = |tag: &[u8; 4]| {
            mvar.map_or(0.0, |mvar| {
                variations::metric_delta(mvar, &vary.mvar_scalars, tag)
            })
        };
        let metrics = vertical_metrics(
            bytes,
            self.metrics.units_per_em,
            &self.hhea,
            self.os2.as_ref(),
            &delta,
        );
        let moved = |value: i16, tag: &[u8; 4]| (f32::from(value) + delta(tag)).round() as i16;
        let typo = self
            .os2
            .clone()
            .and_then(|range| bytes.get(range))
            .and_then(|os2| Some((moved(read_i16(os2, 68)?, b"hasc"), moved(read_i16(os2, 70)?, b"hdsc"))));
        if let Some(metrics) = metrics {
            self.metrics = metrics;
        }
        self.typo = typo;
        self.vary = Some(vary);
    }

    /// Where the font is set along its axes, if that's anywhere but their defaults.
    #[inline]
    fn moved(&self) -> Option<&Variation> {
        self.vary.as_deref().filter(|vary| vary.is_moved())
    }

    /// What `GDEF` says values in `GPOS` change by, where the font is set, if it's been moved along its axes.
    pub(crate) fn gdef_variations(&self) -> Option<variations::Store<'_>> {
        let vary = self.moved()?;
        Some(variations::Store {
            data: self.bytes().get(vary.gdef_store.clone()?)?,
            scalars: &vary.gdef_scalars,
        })
    }

    /// The places along its axes this font names, such as "Semibold Text", for a font that varies.
    pub(crate) fn named_instances(&self) -> Vec<NamedInstance> {
        self.vary
            .as_ref()
            .and_then(|vary| self.bytes().get(vary.fvar.clone()))
            .map(variations::named_instances)
            .unwrap_or_default()
    }

    /// The font's `STAT` table, which names what each place along its axes is, if it has one.
    pub(crate) fn stat_table(&self) -> Option<&[u8]> {
        self.bytes().get(self.stat.clone()?)
    }

    /// One of the names in the font's `name` table, by its number, in English if it has that.
    pub(crate) fn name(&self, id: u16) -> Option<String> {
        let table = self.bytes().get(self.name.clone()?)?;
        let count = read_u16(table, 2)? as usize;
        let strings = read_u16(table, 4)? as usize;
        let mut best: Option<(u8, usize)> = None;
        for index in 0..count {
            let record = 6 + index * 12;
            if read_u16(table, record + 6)? != id {
                continue;
            }
            let platform = read_u16(table, record)?;
            let encoding = read_u16(table, record + 2)?;
            let language = read_u16(table, record + 4)?;
            let rank = match (platform, encoding, language) {
                (3, 1 | 10, 0x409) => 4,
                (3, 1 | 10, _) => 3,
                (0, _, _) => 2,
                (1, 0, 0) => 1,
                _ => continue,
            };
            if best.is_none_or(|(best, _)| rank > best) {
                best = Some((rank, record));
            }
        }
        let (rank, record) = best?;
        let length = read_u16(table, record + 8)? as usize;
        let at = strings + read_u16(table, record + 10)? as usize;
        let text = table.get(at..at + length)?;
        Some(if rank == 1 {
            text.iter().map(|&byte| char::from(byte)).collect()
        } else {
            let units: Vec<u16> = text
                .chunks_exact(2)
                .map(|two| u16::from_be_bytes([two[0], two[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        })
    }

    fn bytes(&self) -> &[u8] {
        self.data.bytes()
    }

    /// Whether any of this font's glyphs are made of colored shapes.
    #[inline]
    pub fn has_color_layers(&self) -> bool {
        self.color.is_some()
    }

    /// The colored shapes that make up this glyph, from the bottom up, if it's made of any.
    pub fn color_layers(&self, glyph: u16) -> impl Iterator<Item = ColorLayer> + '_ {
        let found = self.color.as_ref().and_then(|color| {
            let bytes = self.bytes();
            let glyphs = bytes.get(color.glyphs.clone())?;
            let (mut low, mut high) = (0, glyphs.len() / 6);
            while low < high {
                let middle = (low + high) / 2;
                match read_u16(glyphs, middle * 6)?.cmp(&glyph) {
                    std::cmp::Ordering::Less => low = middle + 1,
                    std::cmp::Ordering::Greater => high = middle,
                    std::cmp::Ordering::Equal => {
                        let first = read_u16(glyphs, middle * 6 + 2)? as usize;
                        let count = read_u16(glyphs, middle * 6 + 4)? as usize;
                        return Some((color, first..first + count));
                    }
                }
            }
            None
        });
        found.into_iter().flat_map(move |(color, range)| {
            let bytes = self.bytes();
            range.filter_map(move |layer| {
                let layers = bytes.get(color.layers.clone())?;
                let glyph = read_u16(layers, layer * 4)?;
                let index = read_u16(layers, layer * 4 + 2)? as usize;
                // The last number there is, or anything past the end, means the color of the text.
                let color = bytes
                    .get(color.palette.clone())
                    .and_then(|palette| palette.get(index * 4..index * 4 + 4))
                    .filter(|_| index != 0xFFFF)
                    .map(|bgra| [bgra[2], bgra[1], bgra[0], bgra[3]]);
                Some(ColorLayer { glyph, color })
            })
        })
    }

    /// Whether this font has glyphs made of gradients.
    #[inline]
    pub(crate) fn gradient_tables(&self) -> Option<(&[u8], usize, usize, Option<usize>)> {
        let color = self.color.as_ref()?;
        let (glyphs, layers, clips) = color.gradients?;
        Some((
            self.bytes().get(color.table.clone())?,
            glyphs,
            layers,
            clips,
        ))
    }

    /// A color from the font's first palette, as red, green, blue and how opaque.
    pub(crate) fn palette_color(&self, index: u16) -> Option<[u8; 4]> {
        let color = self.color.as_ref()?;
        let palette = self.bytes().get(color.palette.clone())?;
        let at = index as usize * 4;
        let bgra = palette.get(at..at + 4)?;
        Some([bgra[2], bgra[1], bgra[0], bgra[3]])
    }

    /// The tables that say how to shape text.
    pub(crate) fn shaping_tables(&self) -> crate::ot::Tables<'_> {
        let bytes = self.bytes();
        let get = |range: &Option<Range<usize>>| range.clone().and_then(|range| bytes.get(range));
        crate::ot::Tables {
            gsub: get(&self.gsub_table),
            gpos: get(&self.gpos_table),
            gdef: get(&self.gdef_table),
            variations: self.gdef_variations(),
        }
    }

    /// Whether this font has special glyphs for characters followed by a variation selector.
    #[inline]
    pub(crate) fn has_variations(&self) -> bool {
        self.variations.is_some()
    }

    /// The glyph for a character with a variation selector after it, if the font has that combination.
    pub(crate) fn variation_glyph(&self, character: char, selector: char) -> Option<u16> {
        let bytes = self.bytes();
        let table = self.variations?;
        let count = read_u32(bytes, table + 6)? as usize;
        let selector = selector as u32;
        let code = character as u32;
        let read_u24 = |at: usize| -> Option<u32> {
            Some(u32::from(*bytes.get(at)?) << 16 | u32::from(read_u16(bytes, at + 1)?))
        };
        let (mut low, mut high) = (0, count);
        while low < high {
            let middle = (low + high) / 2;
            let record = table + 10 + middle * 11;
            match read_u24(record)?.cmp(&selector) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => {
                    let special = read_u32(bytes, record + 7)? as usize;
                    if special != 0 {
                        let list = table + special;
                        let (mut low, mut high) = (0, read_u32(bytes, list)? as usize);
                        while low < high {
                            let middle = (low + high) / 2;
                            let item = list + 4 + middle * 5;
                            match read_u24(item)?.cmp(&code) {
                                std::cmp::Ordering::Less => low = middle + 1,
                                std::cmp::Ordering::Greater => high = middle,
                                std::cmp::Ordering::Equal => {
                                    let glyph = read_u16(bytes, item + 3)?;
                                    return (glyph != 0 && glyph < self.glyph_count)
                                        .then_some(glyph);
                                }
                            }
                        }
                    }
                    // Some combinations are only saying the usual glyph is the right one.
                    let usual = read_u32(bytes, record + 3)? as usize;
                    if usual != 0 {
                        let list = table + usual;
                        for range in 0..read_u32(bytes, list)? as usize {
                            let item = list + 4 + range * 4;
                            let first = read_u24(item)?;
                            let extra = u32::from(*bytes.get(item + 3)?);
                            if (first..=first + extra).contains(&code) {
                                return self.glyph(character);
                            }
                        }
                    }
                    return None;
                }
            }
        }
        None
    }

    /// The glyph for a character, or `None` if the font doesn't have one.
    pub fn glyph(&self, character: char) -> Option<u16> {
        let code = character as u32;
        let glyph = match self.char_map {
            CharMap::Segments { offset, count } => {
                self.segments_glyph(offset as usize, count as usize, code)
            }
            CharMap::Groups { offset, count } => {
                self.groups_glyph(offset as usize, count as usize, code)
            }
        }?;
        (glyph != 0 && glyph < self.glyph_count).then_some(glyph)
    }

    fn segments_glyph(&self, map: usize, count: usize, code: u32) -> Option<u16> {
        if code > 0xFFFF {
            return None;
        }
        let code = code as u16;
        let bytes = self.bytes();
        let ends = map + 14;
        let starts = ends + count * 2 + 2;
        let deltas = starts + count * 2;
        let offsets = deltas + count * 2;

        // Find the first segment that ends at or after this character.
        let (mut low, mut high) = (0, count);
        while low < high {
            let middle = (low + high) / 2;
            if read_u16(bytes, ends + middle * 2)? < code {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        if low >= count {
            return None;
        }
        let start = read_u16(bytes, starts + low * 2)?;
        if code < start {
            return None;
        }
        let delta = read_u16(bytes, deltas + low * 2)?;
        let offset_position = offsets + low * 2;
        let range_offset = read_u16(bytes, offset_position)? as usize;
        if range_offset == 0 {
            return Some(code.wrapping_add(delta));
        }
        let glyph = read_u16(
            bytes,
            offset_position + range_offset + (code - start) as usize * 2,
        )?;
        (glyph != 0).then(|| glyph.wrapping_add(delta))
    }

    fn groups_glyph(&self, map: usize, count: usize, code: u32) -> Option<u16> {
        let bytes = self.bytes();
        let groups = map + 16;
        let (mut low, mut high) = (0, count);
        while low < high {
            let middle = (low + high) / 2;
            let group = groups + middle * 12;
            if read_u32(bytes, group + 4)? < code {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        if low >= count {
            return None;
        }
        let group = groups + low * 12;
        let start = read_u32(bytes, group)?;
        if code < start {
            return None;
        }
        let first_glyph = read_u32(bytes, group + 8)?;
        u16::try_from(first_glyph.checked_add(code - start)?).ok()
    }

    /// How far to move after a glyph, in font units.
    pub fn advance(&self, glyph: u16) -> u16 {
        if self.h_metric_count == 0 {
            return 0;
        }
        let index = glyph.min(self.h_metric_count - 1) as usize;
        let advance = read_u16(&self.bytes()[self.hmtx.clone()], index * 4).unwrap_or(0);
        match self.moved() {
            None => advance,
            Some(vary) => self.varied_advance(vary, glyph, advance),
        }
    }

    /// A glyph's advance where the font is set along its axes, which is what `HVAR` says, or how far apart `gvar`
    /// moves the points where the glyph starts and ends, rounded to whole units, as DirectWrite and HarfBuzz have them.
    fn varied_advance(&self, vary: &Variation, glyph: u16, advance: u16) -> u16 {
        let bytes = self.bytes();
        let delta = if let Some(hvar) = vary.hvar.clone().and_then(|range| bytes.get(range)) {
            variations::advance_delta(hvar, &vary.hvar_scalars, glyph)
        } else if let Some(gvar) = vary.gvar.clone().and_then(|range| bytes.get(range)) {
            let count = self.glyph_data(glyph).map_or(0, outline_point_count);
            // Only where they're named moves the points after the outline's, so where the outline's are doesn't matter.
            let points = vec![(0.0, 0.0); count + 4];
            let mut deltas = points.clone();
            variations::glyph_deltas(gvar, glyph, &vary.coordinates, &points, &[], &mut deltas);
            deltas[count + 1].0 - deltas[count].0
        } else {
            0.0
        };
        (f32::from(advance) + delta.round()).clamp(0.0, f32::from(u16::MAX)) as u16
    }

    /// The adjustment between two glyphs next to each other, in font units. Usually
    /// negative, as in "AV", which would look too far apart otherwise. In a font that
    /// varies, it can be a fraction of a unit where the font is set.
    pub fn kerning(&self, left: u16, right: u16) -> f32 {
        if let Some(gpos) = &self.gpos {
            return gpos.kerning(self.bytes(), left, right, self.gdef_variations());
        }
        f32::from(
            self.kern
                .as_ref()
                .and_then(|range| kern_table_pair(&self.bytes()[range.clone()], left, right))
                .unwrap_or(0),
        )
    }

    /// How far apart two glyphs go, by the font's old `kern` table only. Kerning in the newer place is
    /// for the scripts it's listed under, which is what shaping goes by.
    pub(crate) fn old_kerning(&self, left: u16, right: u16) -> i16 {
        self.kern
            .as_ref()
            .and_then(|range| kern_table_pair(&self.bytes()[range.clone()], left, right))
            .unwrap_or(0)
    }

    /// Sends a glyph's outline to `sink`. A glyph with no outline, like a space, sends
    /// nothing. Damaged data ends the outline early.
    pub fn outline(&self, glyph: u16, sink: &mut impl OutlineSink) {
        let mut budget = MAX_COMPONENTS;
        let identity = Transform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            dx: 0.0,
            dy: 0.0,
        };
        self.outline_at(glyph, identity, &mut budget, sink);
    }

    fn glyph_data(&self, glyph: u16) -> Option<&[u8]> {
        if glyph >= self.glyph_count {
            return None;
        }
        let bytes = self.bytes();
        let loca = &bytes[self.loca.clone()];
        let index = glyph as usize;
        let (start, end) = if self.long_offsets {
            (
                read_u32(loca, index * 4)? as usize,
                read_u32(loca, index * 4 + 4)? as usize,
            )
        } else {
            (
                read_u16(loca, index * 2)? as usize * 2,
                read_u16(loca, index * 2 + 2)? as usize * 2,
            )
        };
        bytes[self.glyf.clone()].get(start..end)
    }

    fn outline_at(
        &self,
        glyph: u16,
        transform: Transform,
        budget: &mut u32,
        sink: &mut impl OutlineSink,
    ) {
        let Some(data) = self.glyph_data(glyph) else {
            return;
        };
        let Some(contours) = read_i16(data, 0) else {
            return;
        };
        let varied = self.moved().and_then(|vary| {
            let gvar = self.bytes().get(vary.gvar.clone()?)?;
            Some((gvar, &vary.coordinates[..]))
        });
        if contours >= 0 {
            let _ = simple_outline(data, contours as usize, transform, sink, |points, ends| {
                if let Some((gvar, coordinates)) = varied {
                    move_points(gvar, glyph, coordinates, points, ends);
                }
            });
        } else {
            let _ = self.composite_outline(glyph, data, transform, budget, varied, sink);
        }
    }

    fn composite_outline(
        &self,
        glyph: u16,
        data: &[u8],
        parent: Transform,
        budget: &mut u32,
        varied: Option<(&[u8], &[i32])>,
        sink: &mut impl OutlineSink,
    ) -> Option<()> {
        let mut parts = Vec::new();
        let read = read_components(data, budget, &mut parts);
        if let Some((gvar, coordinates)) = varied {
            // Each part is a point to `gvar`, which can move it, and the four after them say where the glyph starts and
            // ends.
            let mut points: Vec<(f32, f32)> = parts.iter().map(|(_, own)| (own.dx, own.dy)).collect();
            move_points(gvar, glyph, coordinates, &mut points, &[]);
            for ((_, own), (x, y)) in parts.iter_mut().zip(points) {
                own.dx = x;
                own.dy = y;
            }
        }
        for (component, own) in parts {
            self.outline_at(component, parent.then(own), budget, sink);
        }
        read
    }
}

/// Moves a glyph's points as `gvar` says, for where the font is set along its axes. `ends` are the last point of each of
/// its contours, for a glyph with an outline.
fn move_points(gvar: &[u8], glyph: u16, coordinates: &[i32], points: &mut Vec<(f32, f32)>, ends: &[usize]) {
    let count = points.len();
    points.resize(count + 4, (0.0, 0.0));
    let mut deltas = vec![(0.0, 0.0); points.len()];
    variations::glyph_deltas(gvar, glyph, coordinates, points, ends, &mut deltas);
    points.truncate(count);
    for (point, delta) in points.iter_mut().zip(deltas) {
        point.0 += delta.0;
        point.1 += delta.1;
    }
}

/// How many points a glyph has, as `gvar` counts them: those of its outline, or one for each part of a glyph made of
/// others.
fn outline_point_count(data: &[u8]) -> usize {
    match read_i16(data, 0) {
        Some(contours @ 1..) => {
            read_u16(data, 10 + (contours as usize - 1) * 2).map_or(0, |last| last as usize + 1)
        }
        Some(i16::MIN..=-1) => {
            let mut budget = MAX_COMPONENTS;
            let mut parts = Vec::new();
            let _ = read_components(data, &mut budget, &mut parts);
            parts.len()
        }
        _ => 0,
    }
}

/// Reads the parts of a glyph made of others, each with how it's placed, into `parts`, as far as `budget` allows and
/// the data isn't damaged, which is `None`.
fn read_components(data: &[u8], budget: &mut u32, parts: &mut Vec<(u16, Transform)>) -> Option<()> {
    {
        const ARGS_ARE_WORDS: u16 = 1 << 0;
        const ARGS_ARE_XY: u16 = 1 << 1;
        const HAS_SCALE: u16 = 1 << 3;
        const MORE_COMPONENTS: u16 = 1 << 5;
        const HAS_XY_SCALE: u16 = 1 << 6;
        const HAS_MATRIX: u16 = 1 << 7;

        let mut position = 10;
        loop {
            if *budget == 0 {
                return None;
            }
            *budget -= 1;
            let flags = read_u16(data, position)?;
            let component = read_u16(data, position + 2)?;
            position += 4;
            let (arg1, arg2) = if flags & ARGS_ARE_WORDS != 0 {
                let args = (read_i16(data, position)?, read_i16(data, position + 2)?);
                position += 4;
                args
            } else {
                let args = (
                    *data.get(position)? as i8 as i16,
                    *data.get(position + 1)? as i8 as i16,
                );
                position += 2;
                args
            };
            let fixed =
                |position: usize| read_i16(data, position).map(|value| f32::from(value) / 16384.0);
            let (mut a, mut b, mut c, mut d) = (1.0, 0.0, 0.0, 1.0);
            if flags & HAS_SCALE != 0 {
                a = fixed(position)?;
                d = a;
                position += 2;
            } else if flags & HAS_XY_SCALE != 0 {
                a = fixed(position)?;
                d = fixed(position + 2)?;
                position += 4;
            } else if flags & HAS_MATRIX != 0 {
                a = fixed(position)?;
                b = fixed(position + 2)?;
                c = fixed(position + 4)?;
                d = fixed(position + 6)?;
                position += 8;
            }
            // Offsets given as point numbers are only for fine placement of accents,
            // and rare, so such a part stays at the origin.
            let (dx, dy) = if flags & ARGS_ARE_XY != 0 {
                (f32::from(arg1), f32::from(arg2))
            } else {
                (0.0, 0.0)
            };
            parts.push((component, Transform { a, b, c, d, dx, dy }));
            if flags & MORE_COMPONENTS == 0 {
                return Some(());
            }
        }
    }
}

fn rebase(map: CharMap, table: usize) -> CharMap {
    match map {
        CharMap::Segments { offset, count } => CharMap::Segments {
            offset: offset + table as u32,
            count,
        },
        CharMap::Groups { offset, count } => CharMap::Groups {
            offset: offset + table as u32,
            count,
        },
    }
}

fn read_tables(bytes: &[u8], face: u32) -> Result<Tables, FontError> {
    const TTC: u32 = u32::from_be_bytes(*b"ttcf");
    const OTTO: u32 = u32::from_be_bytes(*b"OTTO");
    const TRUE: u32 = u32::from_be_bytes(*b"true");
    const TRUETYPE: u32 = 0x0001_0000;

    let not_a_font = FontError::Invalid("the file doesn't start like a font");
    let tag = read_u32(bytes, 0).ok_or(not_a_font)?;
    let start = if tag == TTC {
        let at = 12 + face as usize * 4;
        read_u32(bytes, at).ok_or(FontError::Invalid("the font collection is too short"))? as usize
    } else {
        0
    };
    match read_u32(bytes, start) {
        Some(TRUETYPE | TRUE) => {}
        Some(OTTO) => return Err(FontError::Unsupported("it has PostScript outlines")),
        _ => return Err(FontError::Invalid("the file doesn't start like a font")),
    }
    let count =
        read_u16(bytes, start + 4).ok_or(FontError::Invalid("the table list is too short"))?;

    let (mut head, mut hhea, mut maxp, mut hmtx, mut cmap) = (None, None, None, None, None);
    let (mut loca, mut glyf, mut kern, mut gpos) = (None, None, None, None);
    let (mut gsub, mut gdef) = (None, None);
    let (mut colr, mut cpal, mut os2) = (None, None, None);
    let (mut fvar, mut avar, mut gvar, mut hvar, mut mvar) = (None, None, None, None, None);
    let (mut stat, mut name) = (None, None);
    for index in 0..count as usize {
        let record = start + 12 + index * 16;
        let Some(tag) = bytes.get(record..record + 4) else {
            return Err(FontError::Invalid("the table list is too short"));
        };
        let offset = read_u32(bytes, record + 8).map(|value| value as usize);
        let length = read_u32(bytes, record + 12).map(|value| value as usize);
        let (Some(offset), Some(length)) = (offset, length) else {
            return Err(FontError::Invalid("the table list is too short"));
        };
        let end = offset.checked_add(length);
        // A table past the end of the file is treated as missing, which is an error
        // only if the font needs that table.
        let Some(range) = end.filter(|end| *end <= bytes.len()).map(|end| offset..end) else {
            continue;
        };
        match tag {
            b"head" => head = Some(range),
            b"hhea" => hhea = Some(range),
            b"maxp" => maxp = Some(range),
            b"hmtx" => hmtx = Some(range),
            b"cmap" => cmap = Some(range),
            b"loca" => loca = Some(range),
            b"glyf" => glyf = Some(range),
            b"kern" => kern = Some(range),
            b"GPOS" => gpos = Some(range),
            b"GSUB" => gsub = Some(range),
            b"GDEF" => gdef = Some(range),
            b"COLR" => colr = Some(range),
            b"CPAL" => cpal = Some(range),
            b"OS/2" => os2 = Some(range),
            b"fvar" => fvar = Some(range),
            b"avar" => avar = Some(range),
            b"gvar" => gvar = Some(range),
            b"HVAR" => hvar = Some(range),
            b"MVAR" => mvar = Some(range),
            b"STAT" => stat = Some(range),
            b"name" => name = Some(range),
            _ => {}
        }
    }
    Ok(Tables {
        head: head.ok_or(FontError::Invalid("there's no head table"))?,
        hhea: hhea.ok_or(FontError::Invalid("there's no hhea table"))?,
        maxp: maxp.ok_or(FontError::Invalid("there's no maxp table"))?,
        hmtx: hmtx.ok_or(FontError::Invalid("there's no hmtx table"))?,
        cmap: cmap.ok_or(FontError::Invalid("there's no cmap table"))?,
        loca,
        glyf,
        kern,
        gpos,
        gsub,
        gdef,
        colr,
        cpal,
        os2,
        fvar,
        avar,
        gvar,
        hvar,
        mvar,
        stat,
        name,
    })
}

/// Finds the part of a character map that's about variation selectors, if there is one.
fn read_variations(cmap: &[u8]) -> Option<usize> {
    let count = read_u16(cmap, 2)? as usize;
    (0..count).find_map(|index| {
        let record = 4 + index * 8;
        let (platform, encoding) = (read_u16(cmap, record)?, read_u16(cmap, record + 2)?);
        let at = read_u32(cmap, record + 4)? as usize;
        (platform == 0 && encoding == 5 && read_u16(cmap, at)? == 14).then_some(at)
    })
}

/// Picks the character map that covers the most: full Unicode if there is one.
fn read_char_map(cmap: &[u8]) -> Option<CharMap> {
    let count = read_u16(cmap, 2)? as usize;
    let mut best: Option<(u8, CharMap)> = None;
    for index in 0..count {
        let record = 4 + index * 8;
        let platform = read_u16(cmap, record)?;
        let encoding = read_u16(cmap, record + 2)?;
        let offset = read_u32(cmap, record + 4)? as usize;
        let unicode = matches!((platform, encoding), (3, 1 | 10) | (0, _));
        if !unicode {
            continue;
        }
        let map = match read_u16(cmap, offset)? {
            4 => CharMap::Segments {
                offset: offset as u32,
                count: read_u16(cmap, offset + 6)? / 2,
            },
            12 => CharMap::Groups {
                offset: offset as u32,
                count: read_u32(cmap, offset + 12)?,
            },
            _ => continue,
        };
        let rank = match map {
            CharMap::Groups { .. } => 2,
            CharMap::Segments { .. } => 1,
        };
        if best.as_ref().is_none_or(|(best_rank, _)| rank > *best_rank) {
            best = Some((rank, map));
        }
    }
    best.map(|(_, map)| map)
}

/// Finds a pair in the old-style kerning table.
fn kern_table_pair(kern: &[u8], left: u16, right: u16) -> Option<i16> {
    let tables = read_u16(kern, 2)?;
    let mut position = 4;
    for _ in 0..tables {
        let length = read_u16(kern, position + 2)? as usize;
        let coverage = read_u16(kern, position + 4)?;
        // Only horizontal kerning in format 0.
        if coverage & 1 != 0 && coverage >> 8 == 0 {
            let pairs = read_u16(kern, position + 6)? as usize;
            let wanted = (u32::from(left) << 16) | u32::from(right);
            let (mut low, mut high) = (0, pairs);
            while low < high {
                let middle = (low + high) / 2;
                let pair = position + 14 + middle * 6;
                let found = read_u32(kern, pair)?;
                match found.cmp(&wanted) {
                    std::cmp::Ordering::Equal => return read_i16(kern, pair + 4),
                    std::cmp::Ordering::Less => low = middle + 1,
                    std::cmp::Ordering::Greater => high = middle,
                }
            }
        }
        if length == 0 {
            return None;
        }
        position += length;
    }
    None
}

#[derive(Clone, Copy)]
struct Transform {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    dx: f32,
    dy: f32,
}

impl Transform {
    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.dx,
            self.b * x + self.d * y + self.dy,
        )
    }

    /// `inner` applied first, then this one.
    fn then(&self, inner: Transform) -> Transform {
        let (dx, dy) = self.apply(inner.dx, inner.dy);
        Transform {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            dx,
            dy,
        }
    }
}

/// Reads a glyph's outline, has `adjust` move its points, given the last point of each contour, and sends it to `sink`.
fn simple_outline(
    data: &[u8],
    contours: usize,
    transform: Transform,
    sink: &mut impl OutlineSink,
    adjust: impl FnOnce(&mut Vec<(f32, f32)>, &[usize]),
) -> Option<()> {
    const X_SHORT: u8 = 1 << 1;
    const Y_SHORT: u8 = 1 << 2;
    const REPEAT: u8 = 1 << 3;
    const X_SAME_OR_POSITIVE: u8 = 1 << 4;
    const Y_SAME_OR_POSITIVE: u8 = 1 << 5;

    if contours == 0 {
        return Some(());
    }
    let ends_start = 10;
    let instructions_at = ends_start + contours * 2;
    let point_count = read_u16(data, instructions_at - 2)? as usize + 1;
    let instructions = read_u16(data, instructions_at)? as usize;
    let mut position = instructions_at + 2 + instructions;

    // Flags are compressed with repeats, so read them first to find where x starts.
    let mut flags = Vec::with_capacity(point_count);
    while flags.len() < point_count {
        let flag = *data.get(position)?;
        position += 1;
        let mut copies = 1;
        if flag & REPEAT != 0 {
            copies += *data.get(position)? as usize;
            position += 1;
        }
        flags.extend(std::iter::repeat_n(
            flag,
            copies.min(point_count - flags.len()),
        ));
    }

    let mut points = Vec::with_capacity(point_count);
    let mut x = 0i32;
    for &flag in &flags {
        if flag & X_SHORT != 0 {
            let delta = i32::from(*data.get(position)?);
            position += 1;
            x += if flag & X_SAME_OR_POSITIVE != 0 {
                delta
            } else {
                -delta
            };
        } else if flag & X_SAME_OR_POSITIVE == 0 {
            x += i32::from(read_i16(data, position)?);
            position += 2;
        }
        points.push((x, 0i32));
    }
    let mut y = 0i32;
    for (point, &flag) in points.iter_mut().zip(&flags) {
        if flag & Y_SHORT != 0 {
            let delta = i32::from(*data.get(position)?);
            position += 1;
            y += if flag & Y_SAME_OR_POSITIVE != 0 {
                delta
            } else {
                -delta
            };
        } else if flag & Y_SAME_OR_POSITIVE == 0 {
            y += i32::from(read_i16(data, position)?);
            position += 2;
        }
        point.1 = y;
    }

    let mut ends = Vec::with_capacity(contours);
    let mut start = 0;
    for contour in 0..contours {
        let end = read_u16(data, ends_start + contour * 2)? as usize;
        // Ends must increase, and stay within the points.
        if end < start || end >= point_count {
            break;
        }
        ends.push(end);
        start = end + 1;
    }
    let mut points: Vec<(f32, f32)> = points
        .into_iter()
        .map(|(x, y)| (x as f32, y as f32))
        .collect();
    adjust(&mut points, &ends);
    let mut start = 0;
    for &end in &ends {
        emit_contour(&points[start..=end], &flags[start..=end], transform, sink);
        start = end + 1;
    }
    (ends.len() == contours).then_some(())
}

/// Turns one contour's points into curves. Two control points in a row have an implied
/// point on the curve halfway between them.
fn emit_contour(
    points: &[(f32, f32)],
    flags: &[u8],
    transform: Transform,
    sink: &mut impl OutlineSink,
) {
    const ON_CURVE: u8 = 1;
    let count = points.len();
    if count < 2 {
        return;
    }
    let at = |index: usize| {
        let (x, y) = points[index % count];
        transform.apply(x, y)
    };
    let on_curve = |index: usize| flags[index % count] & ON_CURVE != 0;
    let middle = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);

    // Start from a point on the curve, working one out if none is given first.
    let (first, begin) = if on_curve(0) {
        (at(0), 1)
    } else if on_curve(count - 1) {
        (at(count - 1), 0)
    } else {
        (middle(at(0), at(count - 1)), 0)
    };
    sink.move_to(first.0, first.1);

    let remaining = if on_curve(0) || !on_curve(count - 1) {
        count - begin
    } else {
        count - 1
    };
    let mut control: Option<(f32, f32)> = None;
    for offset in 0..remaining {
        let index = begin + offset;
        let point = at(index);
        if on_curve(index) {
            match control.take() {
                Some(c) => sink.quad_to(c.0, c.1, point.0, point.1),
                None => sink.line_to(point.0, point.1),
            }
        } else {
            if let Some(c) = control {
                let implied = middle(c, point);
                sink.quad_to(c.0, c.1, implied.0, implied.1);
            }
            control = Some(point);
        }
    }
    match control {
        Some(c) => sink.quad_to(c.0, c.1, first.0, first.1),
        None => sink.line_to(first.0, first.1),
    }
    sink.close();
}

/// How far above and below the baseline a font's lines go, and the gap between them, as Windows has them, which is what a browser
/// on Windows uses: its typographic ones if it says to use those, and its Windows ones otherwise, with what `hhea` has more than
/// those as the gap. `None` if the `OS/2` table is too short to say.
fn windows_metrics(
    os2: &[u8],
    (ascent, descent, gap): (i16, i16, i16),
    delta: &dyn Fn(&[u8; 4]) -> f32,
) -> Option<(i16, i16, i16)> {
    let moved = |value: i16, tag: &[u8; 4]| -> i16 { (f32::from(value) + delta(tag)).round() as i16 };
    let selection = read_u16(os2, 62)?;
    if selection & (1 << 7) != 0 {
        return Some((
            moved(read_i16(os2, 68)?, b"hasc"),
            moved(read_i16(os2, 70)?, b"hdsc"),
            moved(read_i16(os2, 72)?, b"hlgp"),
        ));
    }
    let clipping = |at: usize, tag: &[u8; 4]| -> Option<i32> {
        Some((f32::from(read_u16(os2, at)?) + delta(tag)).round() as i32)
    };
    let (above, below) = (clipping(74, b"hcla")?, clipping(76, b"hcld")?);
    if above == 0 && below == 0 {
        return None;
    }
    let total = i32::from(ascent) - i32::from(descent) + i32::from(gap);
    let more = (total - above - below).max(0);
    let narrow = |value: i32| i16::try_from(value).ok();
    Some((narrow(above)?, narrow(-below)?, narrow(more)?))
}

/// A font's measurements, as Windows has them, with what `delta` says each of the font's own measurements changes by, by
/// `MVAR`'s tags for them, for a font that varies. `None` if `hhea` is too short.
fn vertical_metrics(
    bytes: &[u8],
    units_per_em: u16,
    hhea: &Range<usize>,
    os2: Option<&Range<usize>>,
    delta: &dyn Fn(&[u8; 4]) -> f32,
) -> Option<FontMetrics> {
    let moved = |value: i16, tag: &[u8; 4]| -> i16 { (f32::from(value) + delta(tag)).round() as i16 };
    let hhea = bytes.get(hhea.clone())?;
    let ascent = moved(read_i16(hhea, 4)?, b"hasc");
    let descent = moved(read_i16(hhea, 6)?, b"hdsc");
    let line_gap = moved(read_i16(hhea, 8)?, b"hlgp");
    let os2 = os2.and_then(|range| bytes.get(range.clone()));
    let (ascent, descent, line_gap) = os2
        .and_then(|os2| windows_metrics(os2, (ascent, descent, line_gap), delta))
        .unwrap_or((ascent, descent, line_gap));
    Some(FontMetrics {
        units_per_em,
        ascent,
        descent,
        line_gap,
        // `sxHeight`, which only an `OS/2` table of version 2 or later has.
        x_height: os2
            .filter(|os2| read_u16(os2, 0).is_some_and(|version| version >= 2))
            .and_then(|os2| read_i16(os2, 86))
            .map_or(0, |height| moved(height, b"xhgt")),
    })
}
