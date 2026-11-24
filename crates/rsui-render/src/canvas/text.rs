//! Text on a canvas: what `font` says, set by `rsui-text`, and drawn gray by the system as a browser draws it, or from the
//! glyphs' outlines when it's turned, stretched or stroked.

use std::collections::HashMap;
use std::rc::Rc;

use rsui_core::Symbol;
use rsui_text::{Font, FontStyle, GlyphBitmap, OutlineSink, PlacedGlyph, TextEngine, TextLayout, TextStyle};

use super::geom::{Matrix, Path};

/// What `font` comes to.
#[derive(Debug, Clone, PartialEq)]
pub struct FontSpec {
    /// In CSS pixels.
    pub size: f32,
    pub weight: u16,
    pub italic: bool,
    /// The families, as CSS lists them.
    pub family: Symbol,
}

impl Default for FontSpec {
    fn default() -> Self {
        Self { size: 10.0, weight: 400, italic: false, family: Symbol::new("sans-serif") }
    }
}

impl FontSpec {
    fn style(&self, letter_spacing: f32, right_to_left: bool) -> TextStyle {
        let mut style = TextStyle::new(self.size);
        style.font = FontStyle::of_weight(&self.weight.to_string());
        style.italic = self.italic;
        style.family = Some(self.family);
        style.letter_spacing = letter_spacing;
        style.right_to_left = right_to_left;
        style
    }
}

/// Text that's been set, ready to draw or measure, with the pen at its left on the alphabetic baseline.
#[derive(Debug, Default)]
pub struct Shaped {
    pub glyphs: Vec<PlacedGlyph>,
    pub width: f32,
    /// How far the font's lines go above and below the baseline, as a browser on Windows has them.
    pub ascent: f32,
    pub descent: f32,
    /// The same, made to add up to the size, as the spec's em box and Chromium's `top`, `middle` and `bottom` go by.
    pub em_ascent: f32,
    pub em_descent: f32,
}

/// What draws and measures text, for every canvas.
pub struct TextWorld {
    engine: Option<TextEngine>,
    rasterizer: rsui_text::Rasterizer,
    layout: TextLayout,
    /// Glyphs drawn gray by the system, by font, glyph, size and how far right of a whole pixel they are.
    glyphs: HashMap<(u16, u16, u32, u8), Rc<GlyphBitmap>, Quick>,
    glyph_bytes: usize,
    /// Each glyph's box in its font's units, as left, top, right and bottom with y going down.
    bounds: HashMap<(u16, u16), [f32; 4]>,
    /// Skia's mask gamma, for each of the eight steps of brightness.
    gamma: Option<Box<[[u8; 256]; 8]>>,
    /// Text already set, by how it was set and then the text itself, as a chart's labels are set again every frame.
    set: HashMap<SetKey, HashMap<Box<str>, Rc<Shaped>, Quick>, Quick>,
    set_count: usize,
}

/// How text was set: the font's size, weight, whether it's italic and its families, the letter spacing and whether it's right
/// to left.
type SetKey = (u32, u16, bool, Symbol, u32, bool);

/// A quicker hash than the standard one, for what's looked up for every glyph.
type Quick = foldhash::fast::RandomState;

/// The most glyphs kept take, before they're let go of.
const GLYPH_BUDGET: usize = 4 << 20;
/// The most pieces of set text kept, before they're let go of.
const SET_BUDGET: usize = 2048;

impl Default for TextWorld {
    fn default() -> Self {
        let mut rasterizer = rsui_text::Rasterizer::default();
        rasterizer.set_system(crate::dwrite::gray_glyphs());
        Self {
            engine: None,
            rasterizer,
            layout: TextLayout::default(),
            glyphs: HashMap::default(),
            glyph_bytes: 0,
            bounds: HashMap::new(),
            gamma: None,
            set: HashMap::default(),
            set_count: 0,
        }
    }
}

/// The system's UI font, as an app's text starts with.
#[cfg(windows)]
const REGULAR_FONT: &str = "C:/Windows/Fonts/segoeui.ttf";
#[cfg(windows)]
const BOLD_FONT: &str = "C:/Windows/Fonts/segoeuib.ttf";

impl TextWorld {
    fn engine(&mut self) -> Option<&mut TextEngine> {
        if self.engine.is_none() {
            #[cfg(windows)]
            let (regular, bold) = (REGULAR_FONT, Some(BOLD_FONT));
            #[cfg(not(windows))]
            let (regular, bold) = {
                let (regular, bold) = rsui_text::system_ui_font_files();
                (regular?, bold)
            };
            let mut engine = TextEngine::new(Font::open(regular).ok()?);
            if let Some(Ok(bold)) = bold.map(Font::open) {
                engine = engine.with_bold(bold);
            }
            self.engine = Some(engine.with_system_fallbacks());
        }
        self.engine.as_mut()
    }

    /// Sets `text` in `font`, on one line, with what a canvas does to white space, or gives what it came to last time.
    pub fn shape(&mut self, text: &str, font: &FontSpec, letter_spacing: f32, right_to_left: bool) -> Rc<Shaped> {
        let key = (font.size.to_bits(), font.weight, font.italic, font.family, letter_spacing.to_bits(), right_to_left);
        if let Some(found) = self.set.get(&key).and_then(|texts| texts.get(text)) {
            return found.clone();
        }
        let shaped = Rc::new(self.shape_now(text, font, letter_spacing, right_to_left));
        if self.set_count >= SET_BUDGET {
            self.set.clear();
            self.set_count = 0;
        }
        self.set.entry(key).or_default().insert(text.into(), shaped.clone());
        self.set_count += 1;
        shaped
    }

    fn shape_now(&mut self, text: &str, font: &FontSpec, letter_spacing: f32, right_to_left: bool) -> Shaped {
        let style = font.style(letter_spacing, right_to_left);
        // Tabs and line breaks are spaces on a canvas.
        let cleaned: std::borrow::Cow<str> = if text.contains(['\t', '\n', '\r', '\u{c}']) {
            text.replace(['\t', '\n', '\r', '\u{c}'], " ").into()
        } else {
            text.into()
        };
        let mut layout = std::mem::take(&mut self.layout);
        let Some(engine) = self.engine() else {
            return Shaped::default();
        };
        engine.layout_into(&cleaned, style, f32::INFINITY, &mut layout);
        if letter_spacing != 0.0 {
            rsui_text::space_letters(&mut layout, letter_spacing);
        }
        let (_, first) = engine.first_font(style);
        let metrics = first.metrics();
        let per_unit = font.size / f32::from(metrics.units_per_em.max(1));
        // Whole pixels, as Chromium rounds them.
        let ascent = (f32::from(metrics.ascent) * per_unit).round();
        let descent = (-f32::from(metrics.descent) * per_unit).round();
        let (typo_above, typo_below) = first
            .typo_metrics()
            .map_or((ascent, descent), |(a, d)| (f32::from(a) * per_unit, -f32::from(d) * per_unit));
        let total = typo_above + typo_below;
        let (em_ascent, em_descent) = if total > 0.0 {
            let above = font.size * typo_above / total;
            (above, font.size - above)
        } else {
            (font.size * 0.8, font.size * 0.2)
        };
        let mut shaped = Shaped { ascent, descent, em_ascent, em_descent, ..Shaped::default() };
        if let Some(line) = layout.lines.first() {
            shaped.width = line.width;
            let range = line.first_glyph as usize..(line.first_glyph + line.glyph_count) as usize;
            shaped.glyphs.extend(layout.glyphs[range].iter().map(|glyph| PlacedGlyph {
                x: glyph.x - line.x,
                y: glyph.y - line.baseline,
                ..*glyph
            }));
        }
        self.layout = layout;
        shaped
    }

    /// The box the ink of set text covers, from the pen at its left on the baseline, as left, top, right and bottom with y going
    /// down, in CSS pixels.
    pub fn ink(&mut self, shaped: &Shaped, font: &FontSpec) -> Option<[f32; 4]> {
        let mut out: Option<[f32; 4]> = None;
        for glyph in &shaped.glyphs {
            let Some(box_) = self.glyph_bounds(glyph.font, glyph.glyph) else {
                continue;
            };
            let Some(engine) = self.engine.as_ref() else {
                return None;
            };
            let Some(face) = engine.font(glyph.font) else {
                continue;
            };
            let per_unit = font.size / f32::from(face.metrics().units_per_em.max(1));
            // A glyph's box is the whole pixels its picture covers at this size, as Skia gives a browser it.
            let placed = [
                glyph.x + (box_[0] * per_unit).floor(),
                glyph.y + (box_[1] * per_unit).floor(),
                glyph.x + (box_[2] * per_unit).ceil(),
                glyph.y + (box_[3] * per_unit).ceil(),
            ];
            out = Some(match out {
                None => placed,
                Some(o) => [o[0].min(placed[0]), o[1].min(placed[1]), o[2].max(placed[2]), o[3].max(placed[3])],
            });
        }
        out
    }

    fn glyph_bounds(&mut self, font: u16, glyph: u16) -> Option<[f32; 4]> {
        if let Some(found) = self.bounds.get(&(font, glyph)) {
            return Some(*found);
        }
        let face = self.engine.as_ref()?.font(font)?;
        let mut sink = Bounds { min: (f32::MAX, f32::MAX), max: (f32::MIN, f32::MIN), pen: (0.0, 0.0) };
        face.outline(glyph, &mut sink);
        let found = if sink.min.0 <= sink.max.0 { [sink.min.0, -sink.max.1, sink.max.0, -sink.min.1] } else { [0.0; 4] };
        self.bounds.insert((font, glyph), found);
        Some(found)
    }

    /// Adds the coverage of set text drawn the system's way, at `size` canvas pixels, with its pen at `pen` on the canvas, to
    /// `mask`, which is `width` across from `origin`. Says it couldn't, for text the system can't draw.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_system(
        &mut self,
        shaped: &Shaped,
        size: f32,
        pens: impl Iterator<Item = (f32, f32)>,
        mut put: impl FnMut(i32, i32, &Rc<GlyphBitmap>),
    ) -> bool {
        let size_bits = size.to_bits();
        for (glyph, (x, y)) in shaped.glyphs.iter().zip(pens) {
            let whole = x.floor();
            // Four places between pixels, as Skia has them.
            let quarter = (((x - whole) * 4.0).round() as u8).min(4);
            let (whole, quarter) = if quarter == 4 { (whole + 1.0, 0) } else { (whole, quarter) };
            let key = (glyph.font, glyph.glyph, size_bits, quarter);
            let bitmap = match self.glyphs.get(&key) {
                Some(found) => found.clone(),
                None => {
                    let Some(engine) = self.engine.as_ref() else {
                        return false;
                    };
                    let Some(face) = engine.font(glyph.font) else {
                        continue;
                    };
                    let drawn = Rc::new(self.rasterizer.rasterize(face, glyph.glyph, size, f32::from(quarter) / 4.0));
                    if self.glyph_bytes > GLYPH_BUDGET {
                        self.glyphs.clear();
                        self.glyph_bytes = 0;
                    }
                    self.glyph_bytes += drawn.coverage.len() + 64;
                    self.glyphs.insert(key, drawn.clone());
                    drawn
                }
            };
            if bitmap.width == 0 {
                continue;
            }
            put(whole as i32, y.round() as i32, &bitmap);
        }
        true
    }

    /// The outlines of set text, through `m` from where its pen starts, made into lines on the canvas.
    pub fn outlines(&mut self, shaped: &Shaped, font: &FontSpec, m: &Matrix, path: &mut Path) {
        let Some(engine) = self.engine.as_ref() else {
            return;
        };
        for glyph in &shaped.glyphs {
            let Some(face) = engine.font(glyph.font) else {
                continue;
            };
            let per_unit = font.size / f32::from(face.metrics().units_per_em.max(1));
            let place = m.then(&Matrix::new([per_unit, 0.0, 0.0, -per_unit, glyph.x, glyph.y]));
            let mut sink = ToPath { path, m: place, start: None };
            face.outline(glyph.glyph, &mut sink);
            if sink.start.is_some() {
                sink.path.close();
            }
        }
    }

    /// The outlines of set text as an SVG path, in pixels from where its pen starts on the baseline, with y going down, as an
    /// `<svg>`'s `<text>` is drawn: see `addLayers` in `layer.js`.
    pub fn svg_path(&mut self, shaped: &Shaped, font: &FontSpec) -> String {
        let mut out = String::new();
        let Some(engine) = self.engine.as_ref() else {
            return out;
        };
        for glyph in &shaped.glyphs {
            let Some(face) = engine.font(glyph.font) else {
                continue;
            };
            let per_unit = font.size / f32::from(face.metrics().units_per_em.max(1));
            let mut sink = ToSvg { out: &mut out, per_unit, x: glyph.x, y: glyph.y, open: false };
            face.outline(glyph.glyph, &mut sink);
            if sink.open {
                sink.out.push('Z');
            }
        }
        out
    }

    /// How much of a pixel a glyph covers, for text this bright, as Skia has it for a browser's text.
    pub fn gamma(&mut self, brightness: u8) -> &[u8; 256] {
        let tables = self.gamma.get_or_insert_with(|| {
            let mut tables = Box::new([[0u8; 256]; 8]);
            for (step, table) in tables.iter_mut().enumerate() {
                for (coverage, out) in table.iter_mut().enumerate() {
                    *out = (browser_curve(coverage as f32 / 255.0, step) * 255.0).round() as u8;
                }
            }
            tables
        });
        &tables[usize::from(brightness / 32)]
    }
}

/// Skia's mask gamma for a step of brightness, as `quad.hlsl` has it with a browser's gamma and contrast.
fn browser_curve(coverage: f32, step: usize) -> f32 {
    let [gamma, contrast] = crate::dwrite::BROWSER_TEXT_LOOK;
    let step = step as f32;
    let src = (step * 32.0 + step * 4.0 + (step / 2.0).floor()) / 255.0;
    let dst = 1.0 - src;
    let lin_src = src.powf(gamma);
    let lin_dst = dst.powf(gamma);
    let a = coverage + (1.0 - coverage) * contrast * lin_dst * coverage;
    if (src - dst).abs() < 1.0 / 256.0 {
        return (a * 255.0).round() / 255.0;
    }
    let lin_out = lin_src * a + (1.0 - a) * lin_dst;
    let out = lin_out.powf(1.0 / gamma);
    ((255.0 * (out - dst) / (src - dst)).round() / 255.0).clamp(0.0, 1.0)
}

struct Bounds {
    min: (f32, f32),
    max: (f32, f32),
    pen: (f32, f32),
}

impl OutlineSink for Bounds {
    fn move_to(&mut self, x: f32, y: f32) {
        self.add(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.add(x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        // Where the curve turns back, each way, is the furthest it goes.
        let (x0, y0) = self.pen;
        for (a, b, c, across) in [(x0, cx, x, true), (y0, cy, y, false)] {
            let denominator = a - 2.0 * b + c;
            if denominator != 0.0 {
                let t = (a - b) / denominator;
                if t > 0.0 && t < 1.0 {
                    let u = 1.0 - t;
                    let along = |p: f32, q: f32, r: f32| u * u * p + 2.0 * u * t * q + t * t * r;
                    let point = if across { (along(a, b, c), along(y0, cy, y)) } else { (along(x0, cx, x), along(a, b, c)) };
                    self.min = (self.min.0.min(point.0), self.min.1.min(point.1));
                    self.max = (self.max.0.max(point.0), self.max.1.max(point.1));
                }
            }
        }
        self.add(x, y);
    }
    fn close(&mut self) {}
}

impl Bounds {
    fn add(&mut self, x: f32, y: f32) {
        self.min = (self.min.0.min(x), self.min.1.min(y));
        self.max = (self.max.0.max(x), self.max.1.max(y));
        self.pen = (x, y);
    }
}

struct ToPath<'a> {
    path: &'a mut Path,
    m: Matrix,
    start: Option<(f32, f32)>,
}

impl OutlineSink for ToPath<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.start.is_some() {
            self.path.close();
        }
        self.path.move_to(self.m.apply(x, y));
        self.start = Some((x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to(self.m.apply(x, y));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.path.quad_to(&self.m, (cx, cy), (x, y));
    }
    fn close(&mut self) {
        self.path.close();
        self.start = None;
    }
}

/// Writes a glyph's outline as SVG path commands, from its font's units, y up, to pixels from `x` and `y`, y down.
struct ToSvg<'a> {
    out: &'a mut String,
    per_unit: f32,
    x: f32,
    y: f32,
    open: bool,
}

impl ToSvg<'_> {
    fn point(&mut self, x: f32, y: f32) {
        use std::fmt::Write;
        let _ = write!(self.out, "{} {} ", round_to(self.x + x * self.per_unit), round_to(self.y - y * self.per_unit));
    }
}

/// To a thousandth, which is plenty for a path and keeps it short.
fn round_to(value: f32) -> f32 {
    (value * 1000.0).round() / 1000.0
}

impl OutlineSink for ToSvg<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.open {
            self.out.push('Z');
        }
        self.out.push('M');
        self.point(x, y);
        self.open = true;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.out.push('L');
        self.point(x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.out.push('Q');
        self.point(cx, cy);
        self.point(x, y);
    }
    fn close(&mut self) {
        self.out.push('Z');
        self.open = false;
    }
}

/// Reads a canvas's `font` from what the page's code gave it, as numbers and the families: see `setFont` in `canvas.js`.
pub fn font_of(size: f64, weight: f64, italic: bool, family: &str) -> FontSpec {
    FontSpec {
        size: if size.is_finite() && size > 0.0 { size as f32 } else { 10.0 },
        weight: (weight as u16).clamp(1, 1000),
        italic,
        family: Symbol::new(if family.is_empty() { "sans-serif" } else { family }),
    }
}
