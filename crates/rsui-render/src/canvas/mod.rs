//! What a page's `<canvas>` draws, worked out here on the CPU and shown as a picture that's updated where it changed.
//!
//! A page's code draws through `canvas.js`, which writes what each call says into one list of numbers for every canvas, and
//! hands that over once what's running has finished, or as soon as something has to be read back. [`run`] goes through the list
//! and draws. Nothing's here for an app that never uses a canvas: the world below is only made the first time one is.
//!
//! The renderer shows a canvas as an image whose source is `canvas:ID:WxH` (see [`source`]): [`crate::Images`] asks here for its
//! size and pixels, and uploads only the part that changed since it last did.

mod blur;
pub(crate) mod geom;
mod paint;
mod pixels;
mod raster;
mod text;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub use geom::Matrix;
use geom::{Cap, Join, Path, StrokeStyle, Stroker};
use paint::{Gradient, GradientKind, Image, Op, Pattern, Repeat, Shader, Style};
use raster::{IRect, Rasterizer};
use text::{FontSpec, TextWorld};

/// What each number in the list means, as `canvas.js` writes them.
pub mod ops {
    pub const SELECT: u32 = 1;
    pub const SAVE: u32 = 2;
    pub const RESTORE: u32 = 3;
    pub const TRANSFORM: u32 = 4;
    pub const RESET: u32 = 5;
    pub const ALPHA: u32 = 6;
    pub const COMPOSITE: u32 = 7;
    pub const FILL_COLOR: u32 = 8;
    pub const FILL_GRADIENT: u32 = 9;
    pub const FILL_PATTERN: u32 = 10;
    pub const STROKE_COLOR: u32 = 11;
    pub const STROKE_GRADIENT: u32 = 12;
    pub const STROKE_PATTERN: u32 = 13;
    pub const LINE_WIDTH: u32 = 14;
    pub const LINE_CAP: u32 = 15;
    pub const LINE_JOIN: u32 = 16;
    pub const MITER: u32 = 17;
    pub const DASH: u32 = 18;
    pub const DASH_OFFSET: u32 = 19;
    pub const SHADOW_COLOR: u32 = 20;
    pub const SHADOW_BLUR: u32 = 21;
    pub const SHADOW_X: u32 = 22;
    pub const SHADOW_Y: u32 = 23;
    pub const SMOOTHING: u32 = 24;
    pub const FONT: u32 = 25;
    pub const ALIGN: u32 = 26;
    pub const BASELINE: u32 = 27;
    pub const DIRECTION: u32 = 28;
    pub const LETTER_SPACING: u32 = 29;
    pub const FILTER: u32 = 30;
    pub const BEGIN: u32 = 31;
    pub const MOVE: u32 = 32;
    pub const LINE: u32 = 33;
    pub const QUAD: u32 = 34;
    pub const BEZIER: u32 = 35;
    pub const ARC: u32 = 36;
    pub const ARC_TO: u32 = 37;
    pub const ELLIPSE: u32 = 38;
    pub const RECT: u32 = 39;
    pub const ROUND_RECT: u32 = 40;
    pub const CLOSE: u32 = 41;
    pub const FILL: u32 = 42;
    pub const STROKE: u32 = 43;
    pub const CLIP: u32 = 44;
    pub const FILL_PATH: u32 = 45;
    pub const STROKE_PATH: u32 = 46;
    pub const CLIP_PATH: u32 = 47;
    pub const FILL_RECT: u32 = 48;
    pub const STROKE_RECT: u32 = 49;
    pub const CLEAR_RECT: u32 = 50;
    pub const FILL_TEXT: u32 = 51;
    pub const STROKE_TEXT: u32 = 52;
    pub const DRAW_IMAGE: u32 = 53;
    pub const LINEAR: u32 = 54;
    pub const RADIAL: u32 = 55;
    pub const CONIC: u32 = 56;
    pub const STOP: u32 = 57;
    pub const PATTERN: u32 = 58;
    pub const PATTERN_TRANSFORM: u32 = 59;
    pub const PATH_DEFINE: u32 = 60;
    pub const FREE: u32 = 61;
    pub const IMAGE_SOURCE: u32 = 62;
    pub const ADD_PATH: u32 = 65;
}

/// How many arguments each kind of path step has, in a Path2D's own list.
fn path_args(op: u32) -> usize {
    match op {
        ops::MOVE | ops::LINE => 2,
        ops::QUAD => 4,
        ops::BEZIER => 6,
        ops::ARC => 6,
        ops::ARC_TO => 5,
        ops::ELLIPSE => 8,
        ops::RECT => 4,
        ops::ROUND_RECT => 12,
        ops::CLOSE => 0,
        ops::ADD_PATH => 7,
        _ => usize::MAX,
    }
}

/// What's clipped to: the pixels inside `bounds`, and how much of each, if not all.
#[derive(Debug)]
struct Clip {
    bounds: IRect,
    mask: Option<Vec<u8>>,
    /// Where the mask lets all of each pixel through, which is all of `bounds` when there's no mask, so that what's drawn
    /// there needn't look at it: a 255 in it changes nothing.
    full: IRect,
}

/// A rectangle of a clip's mask, `width` by `height`, that's all 255, as `[x0, y0, x1, y1]`: the longest run of them across its
/// middle row, and the rows above and below that are 255 all along it. `copied` says each row but the first and the last is the
/// same as the second, as a rectangle's are, so only those three need looking at.
fn full_part(mask: &[u8], width: usize, height: usize, copied: bool) -> Option<[usize; 4]> {
    if width == 0 || height == 0 {
        return None;
    }
    let row = |y: usize| &mask[y * width..(y + 1) * width];
    let middle = row(height / 2);
    let (mut best, mut at) = ((0, 0), 0);
    while at < width {
        if middle[at] != 255 {
            at += 1;
            continue;
        }
        let start = at;
        while at < width && middle[at] == 255 {
            at += 1;
        }
        if at - start > best.1 - best.0 {
            best = (start, at);
        }
    }
    let (x0, x1) = best;
    if x1 == x0 {
        return None;
    }
    let all = |y: usize| row(y)[x0..x1].iter().all(|v| *v == 255);
    let (mut y0, mut y1) = (height / 2, height / 2 + 1);
    if copied {
        // The rows between the first and the last are all the second.
        y0 = y0.min(1);
        y1 = y1.max(height - 1);
    }
    while y0 > 0 && all(y0 - 1) {
        y0 -= 1;
    }
    while y1 < height && all(y1) {
        y1 += 1;
    }
    Some([x0, y0, x1, y1])
}

impl Clip {
    #[inline]
    fn at(&self, x: i32, y: i32) -> f32 {
        match &self.mask {
            None => 1.0,
            Some(mask) => {
                let w = (self.bounds.x1 - self.bounds.x0) as usize;
                f32::from(mask[(y - self.bounds.y0) as usize * w + (x - self.bounds.x0) as usize]) / 255.0
            }
        }
    }
}

/// One `filter` function, as CSS says them.
#[derive(Debug, Clone, PartialEq)]
enum Filter {
    Blur(f32),
    /// A color matrix, four rows of five, for everything but blurs and shadows.
    Matrix([f32; 20]),
    Shadow { x: f32, y: f32, blur: f32, color: [u8; 4] },
}

/// What `save()` keeps.
#[derive(Debug, Clone)]
struct State {
    transform: Matrix,
    alpha: f32,
    op: Op,
    fill: Style,
    stroke: Style,
    line: StrokeStyle,
    shadow_color: [u8; 4],
    shadow_blur: f32,
    shadow_x: f32,
    shadow_y: f32,
    smoothing: bool,
    font: FontSpec,
    align: u8,
    baseline: u8,
    rtl: bool,
    letter_spacing: f32,
    filter: Option<Rc<Vec<Filter>>>,
    clip: Option<Rc<Clip>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            transform: Matrix::IDENTITY,
            alpha: 1.0,
            op: Op::SourceOver,
            fill: Style::Color([0, 0, 0, 255]),
            stroke: Style::Color([0, 0, 0, 255]),
            line: StrokeStyle::default(),
            shadow_color: [0, 0, 0, 0],
            shadow_blur: 0.0,
            shadow_x: 0.0,
            shadow_y: 0.0,
            smoothing: true,
            font: FontSpec::default(),
            align: 0,
            baseline: 0,
            rtl: false,
            letter_spacing: 0.0,
            filter: None,
            clip: None,
        }
    }
}

/// A canvas's pixels and what it's drawing with.
struct Surface {
    width: u32,
    height: u32,
    /// Red, green, blue and alpha, with colors multiplied by alpha.
    pixels: Vec<u8>,
    state: State,
    saved: Vec<State>,
    path: Path,
    /// What's changed since the GPU was last given it.
    dirty: IRect,
    version: u64,
    /// The version the GPU was last given, from which `dirty` counts.
    uploaded: u64,
    opaque: bool,
    /// A copy of its pixels as a picture, as `drawImage()` and patterns take it, kept until they change, so that a canvas drawn
    /// many times, as sprites are, is copied once.
    picture: Option<Rc<Image>>,
}

impl Surface {
    fn new(width: u32, height: u32, opaque: bool) -> Self {
        let mut surface = Self {
            width: 0,
            height: 0,
            pixels: Vec::new(),
            state: State::default(),
            saved: Vec::new(),
            path: Path::default(),
            dirty: IRect::new(0, 0, 0, 0),
            version: 1,
            uploaded: 0,
            opaque,
            picture: None,
        };
        surface.resize(width, height);
        surface
    }

    /// Makes it this big, and as it was when made, as setting `width` or `height` does, whether or not that changes them.
    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        let bytes = width as usize * height as usize * 4;
        self.pixels.clear();
        self.pixels.shrink_to(bytes);
        self.pixels.resize(bytes, 0);
        if self.opaque {
            for pixel in self.pixels.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
        }
        self.state = State::default();
        self.saved.clear();
        self.path.clear();
        self.picture = None;
        self.changed(self.whole());
    }

    fn whole(&self) -> IRect {
        IRect::new(0, 0, self.width as i32, self.height as i32)
    }

    fn changed(&mut self, area: IRect) {
        let area = area.intersect(&self.whole());
        if !area.is_empty() {
            self.dirty = self.dirty.union(&area);
            self.version += 1;
            self.picture = None;
            GENERATION.with(|g| g.set(g.get() + 1));
        }
    }

    /// What it shows now, as a picture, or `None` if it has no pixels.
    fn picture(&mut self) -> Option<Rc<Image>> {
        if self.width == 0 || self.height == 0 {
            return None;
        }
        Some(self.picture.get_or_insert_with(|| Rc::new(Image { width: self.width, height: self.height, pixels: self.pixels.clone() })).clone())
    }

    /// Where what's drawn can reach, as what's clipped to says.
    fn clip_bounds(&self) -> IRect {
        match &self.state.clip {
            Some(clip) => clip.bounds.intersect(&self.whole()),
            None => self.whole(),
        }
    }
}

thread_local! {
    static WORLD: RefCell<Option<Box<World>>> = const { RefCell::new(None) };
    /// Changes whenever any canvas's pixels do, for a window to know to draw again.
    static GENERATION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Everything canvases have, made the first time one is.
#[derive(Default)]
struct World {
    surfaces: HashMap<u32, Surface>,
    gradients: HashMap<u32, Gradient>,
    patterns: HashMap<u32, Pattern>,
    paths: HashMap<u32, Vec<f64>>,
    images: HashMap<u32, Rc<Image>>,
    next_surface: u32,
    tools: Tools,
}

/// What drawing reuses from one shape to the next.
#[derive(Default)]
struct Tools {
    raster: Rasterizer,
    stroker: Stroker,
    text: TextWorld,
    lines: Vec<[f32; 4]>,
    path: Path,
    shaded: Vec<[f32; 4]>,
    shaded32: Vec<u32>,
    rect_lines: Vec<[f32; 4]>,
    /// How much of each pixel across a rectangle covers.
    across: Vec<f32>,
    /// Glyphs placed on the canvas, and how much of each pixel text covers, as the text being drawn has them.
    placed: Vec<(i32, i32, Rc<rsui_text::GlyphBitmap>)>,
    mask: Vec<f32>,
    mask8: Vec<u8>,
    /// A row of coverage from 0 to 255 as from 0 to 1, for what needs it so.
    floats: Vec<f32>,
    /// How much of each pixel a rectangle's rows that it only partly covers cover.
    down: Vec<f32>,
    /// A path's points with its arcs' on the arcs, for a hairline.
    on_arcs: Vec<(f32, f32)>,
    /// A hairline's color, and that color through each coverage with how much of what's under it stays.
    hair_table: (u32, Vec<(u32, u32)>),
}

fn with_world<R>(f: impl FnOnce(&mut World) -> R) -> R {
    WORLD.with(|world| {
        let mut world = world.borrow_mut();
        let world = world.get_or_insert_with(|| Box::new(World { next_surface: 1, ..World::default() }));
        f(world)
    })
}

/// Changes whenever any canvas's pixels do.
pub fn generation() -> u64 {
    GENERATION.with(std::cell::Cell::get)
}

/// Makes a canvas of this size, see-through unless `opaque`, and says its number.
pub fn create(width: u32, height: u32, opaque: bool) -> u32 {
    with_world(|world| {
        let id = world.next_surface;
        world.next_surface += 1;
        world.surfaces.insert(id, Surface::new(width.min(MAX_SIDE), height.min(MAX_SIDE), opaque));
        id
    })
}

/// The most a canvas can be on a side, as Chromium has it.
const MAX_SIDE: u32 = 32767;
/// The most pixels a canvas can have, as Chromium has it.
const MAX_AREA: u64 = 268_435_456;

/// Makes a canvas this big and empty, as setting its `width` or `height` does. Says whether it could be that big.
pub fn resize(id: u32, width: u32, height: u32) -> bool {
    let fits = width <= MAX_SIDE && height <= MAX_SIDE && u64::from(width) * u64::from(height) <= MAX_AREA;
    let (width, height) = if fits { (width, height) } else { (0, 0) };
    with_world(|world| {
        if let Some(surface) = world.surfaces.get_mut(&id) {
            surface.resize(width, height);
        }
    });
    fits
}

/// Lets go of a canvas nothing can reach any more.
pub fn release(id: u32) {
    if WORLD.with(|world| world.borrow().is_none()) {
        return;
    }
    with_world(|world| {
        world.surfaces.remove(&id);
    });
}

/// How big a canvas is, if it's there.
pub fn size(id: u32) -> Option<(u32, u32)> {
    with_world(|world| world.surfaces.get(&id).map(|s| (s.width, s.height)))
}

/// What a canvas is shown by: an image with this source, which says how big it is so that a new size is a new image.
pub fn source(id: u32, width: u32, height: u32) -> String {
    format!("canvas:{id}:{width}x{height}")
}

/// Which canvas an image's source is, and the size it says.
pub(crate) fn parse_source(source: &str) -> Option<(u32, u32, u32)> {
    let rest = source.strip_prefix("canvas:")?;
    let (id, size) = rest.split_once(':')?;
    let (w, h) = size.split_once('x')?;
    Some((id.parse().ok()?, w.parse().ok()?, h.parse().ok()?))
}

/// What the GPU needs of a canvas: all of it if `whole`, or what changed since it was last given it. The pixels, the part
/// they're of, and the version that is.
pub(crate) fn take_upload(id: u32, whole: bool, last: u64) -> Option<(IRectOut, Vec<u8>, u64)> {
    if WORLD.with(|world| world.borrow().is_none()) {
        return None;
    }
    with_world(|world| {
        let surface = world.surfaces.get_mut(&id)?;
        if surface.width == 0 || surface.height == 0 {
            return None;
        }
        if !whole && last == surface.version {
            return None;
        }
        let area = if whole || last != surface.uploaded { surface.whole() } else { surface.dirty };
        surface.dirty = IRect::new(0, 0, 0, 0);
        surface.uploaded = surface.version;
        if area.is_empty() {
            return Some((IRectOut { x: 0, y: 0, width: 0, height: 0 }, Vec::new(), surface.version));
        }
        let (w, h) = ((area.x1 - area.x0) as usize, (area.y1 - area.y0) as usize);
        let mut out = Vec::with_capacity(w * h * 4);
        let stride = surface.width as usize * 4;
        for y in area.y0 as usize..area.y1 as usize {
            let start = y * stride + area.x0 as usize * 4;
            out.extend_from_slice(&surface.pixels[start..start + w * 4]);
        }
        if surface.opaque {
            for pixel in out.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
        }
        Some((IRectOut { x: area.x0 as u32, y: area.y0 as u32, width: w as u32, height: h as u32 }, out, surface.version))
    })
}

/// The version of a canvas's pixels, which changes whenever they do.
pub(crate) fn version(id: u32) -> Option<u64> {
    if WORLD.with(|world| world.borrow().is_none()) {
        return None;
    }
    with_world(|world| world.surfaces.get(&id).map(|s| s.version))
}

/// A part of a canvas, in its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IRectOut {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Reads a canvas's pixels from `x, y`, `width` by `height`, as `getImageData()` gives them, without alpha multiplied in, into
/// `out`. What's outside it is nothing.
pub fn read(id: u32, x: i32, y: i32, width: u32, height: u32, out: &mut [u8]) {
    // 255 * 2^16 / alpha, so that taking alpha out is a multiply.
    static UNDO: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let undo = UNDO.get_or_init(|| {
        let mut table = [0u32; 256];
        for (a, value) in table.iter_mut().enumerate().skip(1) {
            *value = ((255u32 << 16) + a as u32 / 2) / a as u32;
        }
        table
    });
    with_world(|world| {
        let Some(surface) = world.surfaces.get(&id) else {
            return;
        };
        let (w, h) = (width as usize, height as usize);
        if out.len() < w * h * 4 {
            return;
        }
        let asked = IRect::new(x, y, x + width as i32, y + height as i32);
        let inside = asked.intersect(&surface.whole());
        // What's off the canvas is nothing, and the rest is all written below.
        if inside != asked {
            out[..w * h * 4].fill(0);
        }
        if inside.is_empty() {
            return;
        }
        let span = (inside.x1 - inside.x0) as usize;
        let stride = surface.width as usize * 4;
        for sy in inside.y0..inside.y1 {
            let from = sy as usize * stride + inside.x0 as usize * 4;
            let to = ((sy - y) as usize * w + (inside.x0 - x) as usize) * 4;
            let source = &surface.pixels[from..from + span * 4];
            let dest = &mut out[to..to + span * 4];
            // Pixels that are all there need nothing taken out, four at a time.
            let opaque = surface.opaque;
            pixels::each_unless_opaque(dest, source, opaque, |d, p| {
                let a = if opaque { 255 } else { p[3] };
                if a == 255 {
                    d[..3].copy_from_slice(&p[..3]);
                    d[3] = 255;
                } else if a != 0 {
                    let k = undo[usize::from(a)];
                    d[0] = ((u32::from(p[0]) * k + 32768) >> 16).min(255) as u8;
                    d[1] = ((u32::from(p[1]) * k + 32768) >> 16).min(255) as u8;
                    d[2] = ((u32::from(p[2]) * k + 32768) >> 16).min(255) as u8;
                    d[3] = a;
                } else {
                    d.fill(0);
                }
            });
        }
    });
}

/// Puts pixels on a canvas as they are, as `putImageData()` does, from `data`, which is `data_width` across, at `dx, dy`,
/// only the part of it `dirty` says, which is `[x, y, width, height]` in it.
pub fn write(id: u32, data: &[u8], data_width: u32, data_height: u32, dx: i32, dy: i32, dirty: [i32; 4]) {
    with_world(|world| {
        let Some(surface) = world.surfaces.get_mut(&id) else {
            return;
        };
        let [mut x0, mut y0, mut w, mut h] = dirty;
        if w < 0 {
            x0 += w;
            w = -w;
        }
        if h < 0 {
            y0 += h;
            h = -h;
        }
        let (x0, y0) = (x0.max(0), y0.max(0));
        let x1 = (dirty[0].max(dirty[0] + dirty[2])).min(data_width as i32);
        let y1 = (dirty[1].max(dirty[1] + dirty[3])).min(data_height as i32);
        let _ = (w, h);
        let target = IRect::new(dx + x0, dy + y0, dx + x1, dy + y1).intersect(&surface.whole());
        if target.is_empty() {
            return;
        }
        let sw = surface.width as usize;
        let span = (target.x1 - target.x0) as usize;
        for y in target.y0..target.y1 {
            let from = (((y - dy) as usize) * data_width as usize + (target.x0 - dx) as usize) * 4;
            let to = (y as usize * sw + target.x0 as usize) * 4;
            if from + span * 4 > data.len() {
                continue;
            }
            let source = &data[from..from + span * 4];
            let dest = &mut surface.pixels[to..to + span * 4];
            pixels::each_unless_opaque(dest, source, false, |d, p| {
                let a = u32::from(p[3]);
                if a == 255 {
                    d.copy_from_slice(p);
                } else {
                    d[0] = div255(u32::from(p[0]) * a) as u8;
                    d[1] = div255(u32::from(p[1]) * a) as u8;
                    d[2] = div255(u32::from(p[2]) * a) as u8;
                    d[3] = p[3];
                }
            });
        }
        surface.changed(target);
    });
}

/// Draws a picture with these premultiplied pixels into a canvas's image list as `id`, for `drawImage()` to use.
pub fn set_image(id: u32, width: u32, height: u32, pixels: Vec<u8>) {
    with_world(|world| {
        world.images.insert(id, Rc::new(Image { width, height, pixels }));
    });
}

/// Reads the picture at `source` for `drawImage()`, and keeps it as `id`. Says how big it is, or `None` if it can't be read.
pub fn load_image(id: u32, source: &str) -> Option<(u32, u32)> {
    let natural = crate::images::image_size(source)?;
    let picture = match crate::images::read_image(source, natural, false) {
        crate::images::Loaded::Picture(picture) => picture,
        _ => return None,
    };
    let size = (picture.width, picture.height);
    set_image(id, picture.width, picture.height, picture.pixels);
    Some(size)
}

/// Copies what a canvas shows now as a picture, for `createImageBitmap()` and `transferToImageBitmap()`, as `id`.
pub fn snapshot(canvas: u32, id: u32) -> Option<(u32, u32)> {
    with_world(|world| {
        let surface = world.surfaces.get_mut(&canvas)?;
        let size = (surface.width, surface.height);
        let image = surface.picture().unwrap_or_else(|| Rc::new(Image { width: size.0, height: size.1, pixels: Vec::new() }));
        world.images.insert(id, image);
        Some(size)
    })
}

/// Lets go of a picture `drawImage()` was given.
pub fn release_image(id: u32) {
    if WORLD.with(|world| world.borrow().is_none()) {
        return;
    }
    with_world(|world| {
        world.images.remove(&id);
    });
}

/// What `measureText()` says, for text in this font, aligned and on this baseline: its width, then the actual box's left,
/// right, ascent and descent, the font's box's ascent and descent, the em box's, and the hanging, alphabetic and ideographic
/// baselines.
pub fn measure(font: &[f64; 3], family: &str, letter_spacing: f32, rtl: bool, align: u8, baseline: u8, text: &str) -> [f64; 12] {
    with_world(|world| {
        let spec = text::font_of(font[0], font[1], font[2] != 0.0, family);
        let shaped = world.tools.text.shape(text, &spec, letter_spacing, rtl);
        let ink = world.tools.text.ink(&shaped, &spec).unwrap_or([0.0; 4]);
        let width = shaped.width;
        let dx = align_offset(align, rtl, width);
        let b = baseline_shift(baseline, &shaped);
        [
            f64::from(width),
            f64::from(-ink[0] - dx),
            f64::from(ink[2] + dx),
            f64::from(-ink[1] - b),
            f64::from(ink[3] + b),
            f64::from(shaped.ascent - b),
            f64::from(shaped.descent + b),
            f64::from(shaped.em_ascent - b),
            f64::from(shaped.em_descent + b),
            f64::from(shaped.ascent * 0.8 - b),
            f64::from(-b),
            f64::from(-shaped.descent - b),
        ]
    })
}

/// Text as an SVG path, in pixels from where its pen starts on its baseline, and how wide it is, for an `<svg>`'s `<text>`.
pub fn text_path(font: &[f64; 3], family: &str, letter_spacing: f32, text: &str) -> (String, f32) {
    with_world(|world| {
        let spec = text::font_of(font[0], font[1], font[2] != 0.0, family);
        let shaped = world.tools.text.shape(text, &spec, letter_spacing, false);
        (world.tools.text.svg_path(&shaped, &spec), shaped.width)
    })
}

/// How far right of where it's put text starts, as `textAlign` says: a negative number for text that ends there.
fn align_offset(align: u8, rtl: bool, width: f32) -> f32 {
    match (align, rtl) {
        (0, false) | (3, true) | (2, _) => 0.0,
        (1, false) | (0, true) | (3, false) => -width,
        (1, true) => 0.0,
        (4, _) => -width / 2.0,
        _ => 0.0,
    }
}

/// How far above where it's put the alphabetic baseline is, as `textBaseline` says, as Chromium works it out.
fn baseline_shift(baseline: u8, shaped: &text::Shaped) -> f32 {
    match baseline {
        1 => shaped.em_ascent,
        2 => shaped.ascent * 0.8,
        3 => (shaped.em_ascent - shaped.em_descent) / 2.0,
        4 => -shaped.descent,
        5 => -shaped.em_descent,
        _ => 0.0,
    }
}

/// Whether a point on the canvas is inside the path, or Path2D `path` if it isn't 0, or inside its stroke.
pub fn point_in(id: u32, path: u32, x: f32, y: f32, even_odd: bool, stroke: bool) -> bool {
    with_world(|world| {
        let World { surfaces, paths, tools, .. } = world;
        let Some(surface) = surfaces.get(&id) else {
            return false;
        };
        let mut built = Path::default();
        let used = if path != 0 {
            let Some(recorded) = paths.get(&path) else {
                return false;
            };
            replay(recorded, paths, &surface.state.transform, &mut built, 0);
            &built
        } else {
            &surface.path
        };
        tools.lines.clear();
        if stroke {
            tools.stroker.stroke(used, &surface.state.line, &surface.state.transform, &mut tools.lines);
            geom::contains(&tools.lines, x, y, false)
        } else {
            used.fill_lines(&mut tools.lines);
            geom::contains(&tools.lines, x, y, even_odd)
        }
    })
}

/// Draws what a list from `canvas.js` says: numbers, each step's kind and then what it's given, with `strings` for what isn't
/// a number, by where they are in it.
pub fn run(list: &[f64], strings: &[String]) {
    with_world(|world| {
        let mut current: Option<(u32, Surface)> = None;
        let mut at = 0;
        let read = |at: &mut usize| -> f64 {
            let value = list.get(*at).copied().unwrap_or(0.0);
            *at += 1;
            value
        };
        while at < list.len() {
            let op = read(&mut at) as u32;
            if op == ops::SELECT {
                let id = read(&mut at) as u32;
                if let Some((was, surface)) = current.take() {
                    world.surfaces.insert(was, surface);
                }
                current = world.surfaces.remove(&id).map(|surface| (id, surface));
                continue;
            }
            match op {
                ops::LINEAR | ops::RADIAL | ops::CONIC => {
                    let id = read(&mut at) as u32;
                    let kind = match op {
                        ops::LINEAR => GradientKind::Linear {
                            x0: read(&mut at) as f32,
                            y0: read(&mut at) as f32,
                            x1: read(&mut at) as f32,
                            y1: read(&mut at) as f32,
                        },
                        ops::RADIAL => GradientKind::Radial {
                            x0: read(&mut at) as f32,
                            y0: read(&mut at) as f32,
                            r0: read(&mut at) as f32,
                            x1: read(&mut at) as f32,
                            y1: read(&mut at) as f32,
                            r1: read(&mut at) as f32,
                        },
                        _ => GradientKind::Conic { angle: read(&mut at) as f32, x: read(&mut at) as f32, y: read(&mut at) as f32 },
                    };
                    world.gradients.insert(id, Gradient::new(kind));
                    continue;
                }
                ops::STOP => {
                    let id = read(&mut at) as u32;
                    let offset = read(&mut at) as f32;
                    let color = unpack(read(&mut at));
                    if let Some(gradient) = world.gradients.get_mut(&id) {
                        gradient.add_stop(offset, color);
                    }
                    continue;
                }
                ops::PATTERN => {
                    let id = read(&mut at) as u32;
                    let kind = read(&mut at) as u32;
                    let source = read(&mut at) as u32;
                    let repeat = match read(&mut at) as u32 {
                        1 => Repeat::X,
                        2 => Repeat::Y,
                        3 => Repeat::Neither,
                        _ => Repeat::Both,
                    };
                    let image = image_of(world, current.as_mut(), kind, source);
                    if let Some(image) = image {
                        world.patterns.insert(id, Pattern { image, repeat, transform: Matrix::IDENTITY });
                    }
                    continue;
                }
                ops::PATTERN_TRANSFORM => {
                    let id = read(&mut at) as u32;
                    let m = read_matrix(&mut at, &read);
                    if let Some(pattern) = world.patterns.get_mut(&id) {
                        pattern.transform = m;
                    }
                    continue;
                }
                ops::PATH_DEFINE => {
                    let id = read(&mut at) as u32;
                    let count = read(&mut at) as usize;
                    let end = (at + count).min(list.len());
                    world.paths.insert(id, list[at..end].to_vec());
                    at = end;
                    continue;
                }
                ops::FREE => {
                    let kind = read(&mut at) as u32;
                    let id = read(&mut at) as u32;
                    match kind {
                        1 => drop(world.gradients.remove(&id)),
                        2 => drop(world.patterns.remove(&id)),
                        3 => drop(world.paths.remove(&id)),
                        4 => drop(world.images.remove(&id)),
                        _ => {}
                    }
                    continue;
                }
                ops::IMAGE_SOURCE => {
                    let id = read(&mut at) as u32;
                    let index = read(&mut at) as usize;
                    if let Some(source) = strings.get(index) {
                        if !world.images.contains_key(&id) {
                            let _ = load_image(id, source);
                        }
                    }
                    continue;
                }
                _ => {}
            }
            let Some((surface_id, surface)) = current.as_mut() else {
                // Nothing chosen to draw on: what this step is given is skipped.
                at = skip(op, list, at);
                continue;
            };
            let surface_id = *surface_id;
            let mut drawing = Drawing { surface, world: &mut *world, surface_id };
            at = drawing.step(op, list, at, strings);
        }
        if let Some((was, surface)) = current.take() {
            world.surfaces.insert(was, surface);
        }
    });
}

/// Where the next step is, for one that's skipped.
fn skip(op: u32, list: &[f64], at: usize) -> usize {
    let fixed = match op {
        ops::SAVE | ops::RESTORE | ops::RESET | ops::BEGIN | ops::CLOSE | ops::STROKE => 0,
        ops::ALPHA | ops::COMPOSITE | ops::FILL_COLOR | ops::FILL_GRADIENT | ops::FILL_PATTERN | ops::STROKE_COLOR
        | ops::STROKE_GRADIENT | ops::STROKE_PATTERN | ops::LINE_WIDTH | ops::LINE_CAP | ops::LINE_JOIN | ops::MITER
        | ops::DASH_OFFSET | ops::SHADOW_COLOR | ops::SHADOW_BLUR | ops::SHADOW_X | ops::SHADOW_Y | ops::SMOOTHING | ops::ALIGN
        | ops::BASELINE | ops::DIRECTION | ops::LETTER_SPACING | ops::FILTER | ops::FILL | ops::CLIP | ops::STROKE_PATH => 1,
        ops::FILL_PATH | ops::CLIP_PATH | ops::MOVE | ops::LINE => 2,
        ops::FONT | ops::QUAD | ops::RECT | ops::FILL_RECT | ops::STROKE_RECT | ops::CLEAR_RECT | ops::FILL_TEXT | ops::STROKE_TEXT => 4,
        ops::ARC_TO => 5,
        ops::TRANSFORM | ops::BEZIER | ops::ARC => 6,
        ops::ELLIPSE => 8,
        ops::DRAW_IMAGE => 10,
        ops::ROUND_RECT => 12,
        ops::DASH => return at + 1 + list.get(at).copied().unwrap_or(0.0) as usize,
        _ => return list.len(),
    };
    at + fixed
}

fn read_matrix(at: &mut usize, read: &impl Fn(&mut usize) -> f64) -> Matrix {
    let mut values = [0.0f32; 6];
    for value in &mut values {
        *value = read(at) as f32;
    }
    Matrix::new(values)
}

/// A color as `canvas.js` packs it, red in the top byte and alpha in the bottom.
fn unpack(value: f64) -> [u8; 4] {
    let packed = value as u32;
    [(packed >> 24) as u8, (packed >> 16) as u8, (packed >> 8) as u8, packed as u8]
}

/// A picture to draw: one `drawImage()` was given by number, or a canvas, which may be the one being drawn on.
fn image_of(world: &mut World, current: Option<&mut (u32, Surface)>, kind: u32, id: u32) -> Option<Rc<Image>> {
    if kind == 1 {
        let surface = match current {
            Some((current_id, surface)) if *current_id == id => surface,
            _ => world.surfaces.get_mut(&id)?,
        };
        return surface.picture();
    }
    world.images.get(&id).cloned()
}

/// Builds a Path2D's path on the canvas, through `m`.
fn replay(recorded: &[f64], paths: &HashMap<u32, Vec<f64>>, m: &Matrix, out: &mut Path, depth: u32) {
    let _ = paths;
    let mut at = 0;
    let inverse = m.invert();
    let n = |at: &mut usize| -> f32 {
        let value = recorded.get(*at).copied().unwrap_or(0.0) as f32;
        *at += 1;
        value
    };
    while at < recorded.len() {
        let op = recorded[at] as u32;
        at += 1;
        if path_args(op) == usize::MAX {
            return;
        }
        match op {
            ops::ADD_PATH => {
                // What another path had when it was added, which follows, through its own transform.
                let count = n(&mut at) as usize;
                let mut values = [0.0; 6];
                for value in &mut values {
                    *value = n(&mut at);
                }
                let end = (at + count).min(recorded.len());
                if depth < 16 {
                    replay(&recorded[at..end], paths, &m.then(&Matrix::new(values)), out, depth + 1);
                }
                at = end;
            }
            _ => path_step(op, &mut at, &n, m, inverse.as_ref(), out),
        }
    }
}

/// Adds one step to a path, reading what it's given.
fn path_step(op: u32, at: &mut usize, n: &impl Fn(&mut usize) -> f32, m: &Matrix, inverse: Option<&Matrix>, path: &mut Path) {
    match op {
        ops::MOVE => {
            let (x, y) = (n(at), n(at));
            path.move_to(m.apply(x, y));
        }
        ops::LINE => {
            let (x, y) = (n(at), n(at));
            path.line_to(m.apply(x, y));
        }
        ops::QUAD => {
            let (cx, cy, x, y) = (n(at), n(at), n(at), n(at));
            path.quad_to(m, (cx, cy), (x, y));
        }
        ops::BEZIER => {
            let v: [f32; 6] = [n(at), n(at), n(at), n(at), n(at), n(at)];
            path.cubic_to(m, (v[0], v[1]), (v[2], v[3]), (v[4], v[5]));
        }
        ops::ARC => {
            let (x, y, r, a0, a1, ccw) = (n(at), n(at), n(at), n(at), n(at), n(at));
            path.ellipse(m, (x, y), (r, r), 0.0, a0, a1, ccw != 0.0);
        }
        ops::ARC_TO => {
            let (x1, y1, x2, y2, r) = (n(at), n(at), n(at), n(at), n(at));
            path.arc_to(m, inverse, (x1, y1), (x2, y2), r);
        }
        ops::ELLIPSE => {
            let v: [f32; 8] = [n(at), n(at), n(at), n(at), n(at), n(at), n(at), n(at)];
            path.ellipse(m, (v[0], v[1]), (v[2], v[3]), v[4], v[5], v[6], v[7] != 0.0);
        }
        ops::RECT => {
            let (x, y, w, h) = (n(at), n(at), n(at), n(at));
            path.rect(m, x, y, w, h);
        }
        ops::ROUND_RECT => {
            let (x, y, w, h) = (n(at), n(at), n(at), n(at));
            let mut radii = [[0.0; 2]; 4];
            for corner in &mut radii {
                *corner = [n(at), n(at)];
            }
            path.round_rect(m, x, y, w, h, radii);
        }
        ops::CLOSE => path.close(),
        _ => {}
    }
}

/// Drawing on one canvas, with everything else there is.
struct Drawing<'a> {
    surface: &'a mut Surface,
    world: &'a mut World,
    surface_id: u32,
}

/// What covers the pixels being drawn.
enum Coverage<'a> {
    /// What these lines go around, by the even-odd rule if it says.
    Lines(&'a [[f32; 4]], bool),
    /// How much of each pixel, `width` across from `x, y`.
    Mask { x: i32, y: i32, width: usize, height: usize, data: &'a [f32] },
    /// The same, from 0 to 255, as text has it.
    Mask8 { x: i32, y: i32, width: usize, height: usize, data: &'a [u8] },
    /// A rectangle on the canvas, from `[x0, y0]` to `[x1, y1]`, which needs nothing worked out but its edges.
    Rect([f32; 4]),
}

/// A row of how much of each pixel is covered, from 0 to 1, or from 0 to 255.
#[derive(Clone, Copy)]
enum Row<'a> {
    Float(&'a [f32]),
    Byte(&'a [u8]),
}

impl Row<'_> {
    fn len(&self) -> usize {
        match self {
            Row::Float(row) => row.len(),
            Row::Byte(row) => row.len(),
        }
    }

    #[inline]
    fn at(&self, i: usize) -> f32 {
        match self {
            Row::Float(row) => row[i],
            Row::Byte(row) => f32::from(row[i]) / 255.0,
        }
    }
}

/// The four lines round a rectangle, for what needs lines.
fn rect_lines([x0, y0, x1, y1]: [f32; 4], lines: &mut Vec<[f32; 4]>) {
    lines.extend([[x0, y0, x1, y0], [x1, y0, x1, y1], [x1, y1, x0, y1], [x0, y1, x0, y0]]);
}

/// The rectangle a path is on the canvas, if it's one whose sides go across and down, as `rect()` and `fillRect()` make with a
/// transform that only moves and resizes.
fn axis_rect(path: &Path) -> Option<[f32; 4]> {
    let mut found = None;
    for (points, _) in path.each() {
        if points.len() <= 1 {
            continue;
        }
        if found.is_some() || points.len() != 4 {
            return None;
        }
        let (xs, ys) = ([points[0].0, points[1].0, points[2].0, points[3].0], [points[0].1, points[1].1, points[2].1, points[3].1]);
        let axis = (xs[0] == xs[1] && ys[1] == ys[2] && xs[2] == xs[3] && ys[3] == ys[0])
            || (ys[0] == ys[1] && xs[1] == xs[2] && ys[2] == ys[3] && xs[3] == xs[0]);
        if !axis {
            return None;
        }
        found = Some([xs[0].min(xs[2]), ys[0].min(ys[2]), xs[0].max(xs[2]), ys[0].max(ys[2])]);
    }
    found
}

impl Drawing<'_> {
    fn step(&mut self, op: u32, list: &[f64], mut at: usize, strings: &[String]) -> usize {
        let read = |at: &mut usize| -> f64 {
            let value = list.get(*at).copied().unwrap_or(0.0);
            *at += 1;
            value
        };
        let f = |at: &mut usize| read(at) as f32;
        let state = &mut self.surface.state;
        match op {
            ops::SAVE => {
                let copy = state.clone();
                self.surface.saved.push(copy);
            }
            ops::RESTORE => {
                if let Some(saved) = self.surface.saved.pop() {
                    self.surface.state = saved;
                }
            }
            ops::TRANSFORM => {
                let m = read_matrix(&mut at, &read);
                if m.is_finite() {
                    state.transform = m;
                }
            }
            ops::RESET => {
                let (w, h) = (self.surface.width, self.surface.height);
                self.surface.resize(w, h);
            }
            ops::ALPHA => state.alpha = f(&mut at).clamp(0.0, 1.0),
            ops::COMPOSITE => state.op = Op::from_index(read(&mut at) as u32),
            ops::FILL_COLOR => state.fill = Style::Color(unpack(read(&mut at))),
            ops::FILL_GRADIENT => state.fill = Style::Gradient(read(&mut at) as u32),
            ops::FILL_PATTERN => state.fill = Style::Pattern(read(&mut at) as u32),
            ops::STROKE_COLOR => state.stroke = Style::Color(unpack(read(&mut at))),
            ops::STROKE_GRADIENT => state.stroke = Style::Gradient(read(&mut at) as u32),
            ops::STROKE_PATTERN => state.stroke = Style::Pattern(read(&mut at) as u32),
            ops::LINE_WIDTH => state.line.width = f(&mut at),
            ops::LINE_CAP => {
                state.line.cap = match read(&mut at) as u32 {
                    1 => Cap::Round,
                    2 => Cap::Square,
                    _ => Cap::Butt,
                }
            }
            ops::LINE_JOIN => {
                state.line.join = match read(&mut at) as u32 {
                    1 => Join::Round,
                    2 => Join::Bevel,
                    _ => Join::Miter,
                }
            }
            ops::MITER => state.line.miter_limit = f(&mut at),
            ops::DASH => {
                let count = read(&mut at) as usize;
                state.line.dash.clear();
                for _ in 0..count {
                    let value = f(&mut at);
                    state.line.dash.push(value);
                }
            }
            ops::DASH_OFFSET => state.line.dash_offset = f(&mut at),
            ops::SHADOW_COLOR => state.shadow_color = unpack(read(&mut at)),
            ops::SHADOW_BLUR => state.shadow_blur = f(&mut at),
            ops::SHADOW_X => state.shadow_x = f(&mut at),
            ops::SHADOW_Y => state.shadow_y = f(&mut at),
            ops::SMOOTHING => state.smoothing = read(&mut at) != 0.0,
            ops::FONT => {
                let (size, weight, italic) = (read(&mut at), read(&mut at), read(&mut at));
                let family = strings.get(read(&mut at) as usize).map_or("", String::as_str);
                state.font = text::font_of(size, weight, italic != 0.0, family);
            }
            ops::ALIGN => state.align = read(&mut at) as u8,
            ops::BASELINE => state.baseline = read(&mut at) as u8,
            ops::DIRECTION => state.rtl = read(&mut at) != 0.0,
            ops::LETTER_SPACING => state.letter_spacing = f(&mut at),
            ops::FILTER => {
                let index = read(&mut at);
                state.filter = if index < 0.0 {
                    None
                } else {
                    strings.get(index as usize).and_then(|text| parse_filter(text)).map(Rc::new)
                };
            }
            ops::BEGIN => self.surface.path.clear(),
            ops::MOVE | ops::LINE | ops::QUAD | ops::BEZIER | ops::ARC | ops::ARC_TO | ops::ELLIPSE | ops::RECT | ops::ROUND_RECT
            | ops::CLOSE => {
                let m = self.surface.state.transform;
                let inverse = m.invert();
                path_step(op, &mut at, &f, &m, inverse.as_ref(), &mut self.surface.path);
            }
            ops::FILL => {
                let even_odd = read(&mut at) != 0.0;
                let path = std::mem::take(&mut self.surface.path);
                self.fill_path(&path, even_odd);
                self.surface.path = path;
            }
            ops::STROKE => {
                let path = std::mem::take(&mut self.surface.path);
                self.stroke_path(&path);
                self.surface.path = path;
            }
            ops::CLIP => {
                let even_odd = read(&mut at) != 0.0;
                let path = std::mem::take(&mut self.surface.path);
                self.clip(&path, even_odd);
                self.surface.path = path;
            }
            ops::FILL_PATH | ops::STROKE_PATH | ops::CLIP_PATH => {
                let id = read(&mut at) as u32;
                let even_odd = op != ops::STROKE_PATH && read(&mut at) != 0.0;
                let mut path = std::mem::take(&mut self.world.tools.path);
                path.clear();
                if let Some(recorded) = self.world.paths.get(&id) {
                    replay(recorded, &self.world.paths, &self.surface.state.transform, &mut path, 0);
                }
                match op {
                    ops::FILL_PATH => self.fill_path(&path, even_odd),
                    ops::STROKE_PATH => self.stroke_path(&path),
                    _ => self.clip(&path, even_odd),
                }
                self.world.tools.path = path;
            }
            ops::FILL_RECT | ops::STROKE_RECT | ops::CLEAR_RECT => {
                let (x, y, w, h) = (f(&mut at), f(&mut at), f(&mut at), f(&mut at));
                if ![x, y, w, h].iter().all(|v| v.is_finite()) {
                    return at;
                }
                let mut path = std::mem::take(&mut self.world.tools.path);
                path.clear();
                let m = self.surface.state.transform;
                match op {
                    // The same rectangle `axis_rect()` would find in the path, without making it.
                    ops::FILL_RECT if m.b == 0.0 && m.c == 0.0 && {
                        let ((ax, ay), (bx, by)) = (m.apply(x, y), m.apply(x + w, y + h));
                        ax.is_finite() && ay.is_finite() && bx.is_finite() && by.is_finite()
                    } =>
                    {
                        let ((ax, ay), (bx, by)) = (m.apply(x, y), m.apply(x + w, y + h));
                        let shader = self.shader(self.surface.state.fill);
                        self.draw(Coverage::Rect([ax.min(bx), ay.min(by), ax.max(bx), ay.max(by)]), &shader, None);
                    }
                    ops::FILL_RECT => {
                        path.rect(&m, x, y, w, h);
                        self.fill_path(&path, false);
                    }
                    ops::STROKE_RECT => {
                        if w != 0.0 || h != 0.0 {
                            path.move_to(m.apply(x, y));
                            path.line_to(m.apply(x + w, y));
                            path.line_to(m.apply(x + w, y + h));
                            path.line_to(m.apply(x, y + h));
                            if w != 0.0 && h != 0.0 {
                                path.close();
                            } else {
                                path.line_to(m.apply(x, y));
                            }
                            self.stroke_path(&path);
                        }
                    }
                    _ => {
                        path.rect(&m, x, y, w, h);
                        self.clear(&path, (x, y, w, h));
                    }
                }
                self.world.tools.path = path;
            }
            ops::FILL_TEXT | ops::STROKE_TEXT => {
                let index = read(&mut at) as usize;
                let (x, y, most) = (f(&mut at), f(&mut at), read(&mut at));
                if let Some(text) = strings.get(index) {
                    self.text(text, x, y, most, op == ops::STROKE_TEXT);
                }
            }
            ops::DRAW_IMAGE => {
                let kind = read(&mut at) as u32;
                let id = read(&mut at) as u32;
                let v: [f32; 8] = [f(&mut at), f(&mut at), f(&mut at), f(&mut at), f(&mut at), f(&mut at), f(&mut at), f(&mut at)];
                let image = if kind == 1 && id == self.surface_id { self.surface.picture() } else { image_of(self.world, None, kind, id) };
                if let Some(image) = image {
                    self.draw_image(&image, v);
                }
            }
            _ => return skip(op, list, at),
        }
        at
    }

    /// What a fill or stroke style paints with, as it is now.
    fn shader(&mut self, style: Style) -> Shader {
        let state = &self.surface.state;
        if let Style::Color(c) = style {
            let a = f32::from(c[3]) / 255.0;
            return Shader::Solid([f32::from(c[0]) * a, f32::from(c[1]) * a, f32::from(c[2]) * a, f32::from(c[3])]);
        }
        let to_user = state.transform.invert();
        match style {
            Style::Color(_) => Shader::None,
            Style::Gradient(id) => {
                let (Some(gradient), Some(to_user)) = (self.world.gradients.get_mut(&id), to_user) else {
                    return Shader::None;
                };
                if !gradient.has_stops() {
                    return Shader::None;
                }
                let table = gradient.table();
                match gradient.kind {
                    GradientKind::Linear { x0, y0, x1, y1 } => {
                        let (dx, dy) = (x1 - x0, y1 - y0);
                        let len2 = dx * dx + dy * dy;
                        if len2 == 0.0 {
                            return Shader::None;
                        }
                        Shader::Linear { table, to_user, x0, y0, dx, dy, inv_len2: 1.0 / len2 }
                    }
                    GradientKind::Radial { x0, y0, r0, x1, y1, r1 } => {
                        if x0 == x1 && y0 == y1 && r0 == r1 {
                            return Shader::None;
                        }
                        Shader::Radial { table, to_user, kind: [x0, y0, r0, x1, y1, r1] }
                    }
                    GradientKind::Conic { angle, x, y } => Shader::Conic { table, to_user, angle, x, y },
                }
            }
            Style::Pattern(id) => {
                let (Some(pattern), Some(to_user)) = (self.world.patterns.get(&id), to_user) else {
                    return Shader::None;
                };
                let Some(from_pattern) = pattern.transform.invert() else {
                    return Shader::None;
                };
                let image = pattern.image.clone();
                let bounds = [0, 0, image.width as i32, image.height as i32];
                Shader::Image {
                    to_image: from_pattern.then(&to_user),
                    image,
                    smooth: state.smoothing,
                    repeat: pattern.repeat,
                    bounds,
                    pattern: true,
                }
            }
        }
    }

    fn fill_path(&mut self, path: &Path, even_odd: bool) {
        if let Some(rect) = axis_rect(path) {
            let shader = self.shader(self.surface.state.fill);
            self.draw(Coverage::Rect(rect), &shader, None);
            return;
        }
        let mut lines = std::mem::take(&mut self.world.tools.lines);
        lines.clear();
        path.fill_lines(&mut lines);
        let shader = self.shader(self.surface.state.fill);
        self.draw(Coverage::Lines(&lines, even_odd), &shader, None);
        self.world.tools.lines = lines;
    }

    fn stroke_path(&mut self, path: &Path) {
        if self.hairline(path) {
            return;
        }
        let mut lines = std::mem::take(&mut self.world.tools.lines);
        lines.clear();
        let state = &self.surface.state;
        self.world.tools.stroker.stroke(path, &state.line, &state.transform, &mut lines);
        let shader = self.shader(self.surface.state.stroke);
        self.draw(Coverage::Lines(&lines, false), &shader, None);
        self.world.tools.lines = lines;
    }

    /// Draws a stroke no wider than a pixel on the canvas as Skia does, a line a pixel across, as faint as it's thinner than that,
    /// with each pixel it crosses shared with the next. Says whether it could, which is for a plain color laid on as usual.
    fn hairline(&mut self, path: &Path) -> bool {
        let state = &self.surface.state;
        let m = state.transform;
        let (sx, sy) = ((m.a * m.a + m.b * m.b).sqrt(), (m.c * m.c + m.d * m.d).sqrt());
        let width = state.line.width * sx;
        let plain = state.op == Op::SourceOver
            && state.line.dash.is_empty()
            && state.filter.is_none()
            && state.shadow_color[3] == 0
            && (sx - sy).abs() <= sx * 0.001
            && (m.a * m.c + m.b * m.d).abs() <= sx * sy * 0.001;
        let Style::Color(color) = state.stroke else {
            return false;
        };
        let cap = state.line.cap;
        if !plain || !(width > 0.0 && width <= 1.0) {
            return false;
        }
        let Shader::Solid(premultiplied) = self.shader(Style::Color(color)) else {
            return false;
        };
        let color = paint::pack(quantize(premultiplied, self.surface.state.alpha));
        let modulate = (width * 256.0 + 0.5) as u32;
        let clipping = self.surface.clip_bounds();
        if clipping.is_empty() || modulate == 0 {
            return true;
        }
        let stride = self.surface.width as usize;
        // A clip that's soft at its edges lets through as much of each pixel as it covers, as Skia's does a hairline.
        let clip = self.surface.state.clip.clone();
        let soft = clip.as_ref().and_then(|clip| clip.mask.as_ref().map(|mask| (clip.bounds, mask)));
        // Where it lets all of each pixel through, along and across, as the walk has them; everywhere when it isn't soft.
        let full = clip.as_ref().filter(|_| soft.is_some()).map_or((i32::MIN, i32::MAX, i32::MIN, i32::MAX), |clip| {
            (clip.full.x0, clip.full.x1, clip.full.y0, clip.full.y1)
        });
        // The color through each coverage out of 256, and how much of what's under it stays, worked out once for each color.
        let mut table = std::mem::take(&mut self.world.tools.hair_table);
        if table.1.len() != 257 || table.0 != color {
            table.0 = color;
            table.1.clear();
            table.1.extend((0..=256u32).map(|c| {
                let s = if c >= 256 { color } else { paint::scale32(color, c) };
                let sa = s >> 24;
                (s, 256 - (sa + (sa >> 7)))
            }));
        }
        let shades = &table.1[..];
        let pixels = &mut self.surface.pixels;
        let mut touched = IRect::new(0, 0, 0, 0);
        let mut put = |across: bool, u: (i32, i32), v0: f32, slope: f32, ends: (f32, f32)| {
            // Only the part of the walk inside what's clipped to, along the longer way.
            let (u_low, u_high, v_low, v_high) = if across {
                (clipping.x0, clipping.x1, clipping.y0, clipping.y1)
            } else {
                (clipping.y0, clipping.y1, clipping.x0, clipping.x1)
            };
            let (full_u0, full_u1, full_v0, full_v1) = if across { full } else { (full.2, full.3, full.0, full.1) };
            let (start, end) = (u.0.max(u_low), u.1.min(u_high - 1));
            if start > end {
                return;
            }
            // Where its middle is across, in steps of 1/65536 of a pixel, from the middle of the first pixel along on.
            let first_middle = v0 + slope * ((start as f32 + 0.5) - ends.0) - 0.5;
            if !(first_middle.abs() < 30000.0 && slope.abs() <= 1.0) {
                return;
            }
            let mut v = (first_middle * 65536.0) as i64;
            let step = (slope * 65536.0) as i64;
            // How far a pixel along and a pixel across are in the canvas's bytes.
            let (along, side) = if across { (4, stride * 4) } else { (stride * 4, 4) };
            for p in start..=end {
                // Only the ends cover part of their pixel along.
                let part = if p == u.0 || p == u.1 {
                    (((p + 1) as f32).min(ends.1) - (p as f32).max(ends.0)).max(0.0)
                } else {
                    1.0
                };
                let weight = if part >= 1.0 { modulate } else { (part * modulate as f32) as u32 };
                let q = (v >> 16) as i32;
                let share = ((v >> 8) & 255) as u32;
                v += step;
                let (first, second) = ((weight * (256 - share)) >> 8, (weight * share) >> 8);
                // Both pixels across, when both are inside what's clipped to and a soft clip lets all of them through, as is
                // usual.
                let whole = soft.is_none() || (p >= full_u0 && p < full_u1 && q >= full_v0 && q + 1 < full_v1);
                if whole && q >= v_low && q + 1 < v_high {
                    let index = p as usize * along + q as usize * side;
                    let (s0, keep0) = shades[first as usize];
                    let (s1, keep1) = shades[second as usize];
                    let px = &mut pixels[index..index + side + 4];
                    // None of a pixel lays nothing on it and keeps all of it, which is what's there, as a line down the
                    // middle of pixels has on one side.
                    if first != 0 {
                        let d0 = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
                        px[..4].copy_from_slice(&(s0 + paint::scale32(d0, keep0)).to_le_bytes());
                    }
                    if second != 0 {
                        let d1 = u32::from_le_bytes([px[side], px[side + 1], px[side + 2], px[side + 3]]);
                        px[side..side + 4].copy_from_slice(&(s1 + paint::scale32(d1, keep1)).to_le_bytes());
                    }
                    continue;
                }
                for (at, cover) in [(q, first), (q + 1, second)] {
                    if cover == 0 || at < v_low || at >= v_high {
                        continue;
                    }
                    let cover = match soft {
                        Some((bounds, mask)) => {
                            let (x, y) = if across { (p, at) } else { (at, p) };
                            let m = u32::from(mask[(y - bounds.y0) as usize * (bounds.x1 - bounds.x0) as usize + (x - bounds.x0) as usize]);
                            (cover * m + 127) / 255
                        }
                        None => cover,
                    };
                    if cover == 0 {
                        continue;
                    }
                    // As `paint::blend_pixel` lays it on.
                    let index = p as usize * along + at as usize * side;
                    let (s, keep) = shades[cover as usize];
                    let px = &mut pixels[index..index + 4];
                    let d = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
                    px.copy_from_slice(&(s + paint::scale32(d, keep)).to_le_bytes());
                }
            }
        };
        let extend = if cap == Cap::Butt { 0.0 } else { 0.5 };
        // Arcs' points on the arcs themselves, as Skia has them for a hairline, rather than a little outside, as for filling.
        let mut on_arcs = std::mem::take(&mut self.world.tools.on_arcs);
        let all = if path.arcs.is_empty() {
            &path.points
        } else {
            path.points_on_arcs(&mut on_arcs);
            &on_arcs
        };
        for sub in &path.subpaths {
            let (points, closed) = (&all[sub.start as usize..sub.end as usize], sub.closed);
            let count = points.len();
            if count < 2 {
                continue;
            }
            let segments = if closed && count > 2 { count } else { count - 1 };
            for i in 0..segments {
                let (mut a, mut b) = (points[i], points[(i + 1) % count]);
                if extend > 0.0 && !(closed && count > 2) {
                    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                    let length = (dx * dx + dy * dy).sqrt();
                    if length > 0.0 {
                        let (ux, uy) = (dx / length * extend, dy / length * extend);
                        if i == 0 {
                            a = (a.0 - ux, a.1 - uy);
                        }
                        if i == segments - 1 {
                            b = (b.0 + ux, b.1 + uy);
                        }
                    }
                }
                hair_segment(a, b, &mut put);
                let reach = IRect::new(
                    raster::floor(a.0.min(b.0)) as i32 - 1,
                    raster::floor(a.1.min(b.1)) as i32 - 1,
                    raster::ceil(a.0.max(b.0)) as i32 + 2,
                    raster::ceil(a.1.max(b.1)) as i32 + 2,
                );
                touched = touched.union(&reach);
            }
        }
        self.world.tools.on_arcs = on_arcs;
        self.world.tools.hair_table = table;
        self.surface.changed(touched);
        true
    }

    fn clear(&mut self, path: &Path, rect: (f32, f32, f32, f32)) {
        let m = self.surface.state.transform;
        let simple = self.surface.state.clip.as_ref().is_none_or(|clip| clip.mask.is_none()) && m.is_axis_aligned();
        if simple {
            let (x0, y0) = m.apply(rect.0, rect.1);
            let (x1, y1) = m.apply(rect.0 + rect.2, rect.1 + rect.3);
            let (l, r) = (x0.min(x1), x0.max(x1));
            let (t, b) = (y0.min(y1), y0.max(y1));
            if l == l.round() && r == r.round() && t == t.round() && b == b.round() {
                let area = IRect::new(l as i32, t as i32, r as i32, b as i32).intersect(&self.surface.clip_bounds());
                if area.is_empty() {
                    return;
                }
                let stride = self.surface.width as usize * 4;
                let fill = if self.surface.opaque { [0, 0, 0, 255] } else { [0; 4] };
                for y in area.y0..area.y1 {
                    let start = y as usize * stride + area.x0 as usize * 4;
                    let row = &mut self.surface.pixels[start..start + (area.x1 - area.x0) as usize * 4];
                    if fill == [0; 4] {
                        row.fill(0);
                    } else {
                        for pixel in row.chunks_exact_mut(4) {
                            pixel.copy_from_slice(&fill);
                        }
                    }
                }
                self.surface.changed(area);
                return;
            }
        }
        if let Some(rect) = axis_rect(path) {
            self.draw(Coverage::Rect(rect), &Shader::Solid([0.0, 0.0, 0.0, 255.0]), Some(Op::Clear));
            return;
        }
        let mut lines = std::mem::take(&mut self.world.tools.lines);
        lines.clear();
        path.fill_lines(&mut lines);
        self.draw(Coverage::Lines(&lines, false), &Shader::Solid([0.0, 0.0, 0.0, 255.0]), Some(Op::Clear));
        self.world.tools.lines = lines;
    }

    fn clip(&mut self, path: &Path, even_odd: bool) {
        let bounds = self.surface.clip_bounds();
        let old = self.surface.state.clip.clone();
        // A rectangle on whole pixels is just where it is.
        if old.as_ref().is_none_or(|clip| clip.mask.is_none()) && path.subpaths.len() <= 2 {
            if let Some(rect) = whole_rect(path) {
                let bounds = bounds.intersect(&rect);
                self.surface.state.clip = Some(Rc::new(Clip { bounds, mask: None, full: bounds }));
                return;
            }
        }
        let mut lines = std::mem::take(&mut self.world.tools.lines);
        lines.clear();
        path.fill_lines(&mut lines);
        // What the rasterizer reaches, as it works it out, so that each row goes straight into the mask.
        let Some(reach) = IRect::around(&lines).map(|area| area.intersect(&bounds)).filter(|area| !area.is_empty()) else {
            self.world.tools.lines = lines;
            let none = IRect::new(0, 0, 0, 0);
            self.surface.state.clip = Some(Rc::new(Clip { bounds: none, mask: None, full: none }));
            return;
        };
        let width = (reach.x1 - reach.x0) as usize;
        let mut mask = vec![0u8; width * (reach.y1 - reach.y0) as usize];
        let under = old.as_ref().filter(|clip| clip.mask.is_some());
        let height = (reach.y1 - reach.y0) as usize;
        // A rectangle whose sides go across and down covers each row between its first and last the same, as its sides are
        // straight down: that row is worked out once, along with the first and the last, and copied to the rest.
        let rows: &[IRect] = &[bounds];
        let straight;
        let rows = if under.is_none() && height > 3 && axis_rect(path).is_some() {
            let row = |y: i32| IRect::new(bounds.x0, y, bounds.x1, y + 1);
            straight = [row(reach.y0), row(reach.y0 + 1), row(reach.y1 - 1)];
            &straight[..]
        } else {
            rows
        };
        for band in rows {
            self.world.tools.raster.fill(&lines, *band, even_odd, |y, x, row| {
                let base = (y - reach.y0) as usize * width + (x - reach.x0) as usize;
                let cells = &mut mask[base..base + row.len()];
                match under {
                    Some(clip) => {
                        for (i, (cell, amount)) in cells.iter_mut().zip(row).enumerate() {
                            *cell = (amount * clip.at(x + i as i32, y) * 255.0 + 0.5) as u8;
                        }
                    }
                    None => {
                        for (cell, amount) in cells.iter_mut().zip(row) {
                            *cell = (amount * 255.0 + 0.5) as u8;
                        }
                    }
                }
            });
        }
        if rows.len() == 3 {
            let (first, rest) = mask.split_at_mut(2 * width);
            let inside = &first[width..];
            for row in rest[..(height - 3) * width].chunks_exact_mut(width) {
                row.copy_from_slice(inside);
            }
        }
        self.world.tools.lines = lines;
        let full = full_part(&mask, width, height, rows.len() == 3).map_or(IRect::new(0, 0, 0, 0), |[x0, y0, x1, y1]| {
            IRect::new(reach.x0 + x0 as i32, reach.y0 + y0 as i32, reach.x0 + x1 as i32, reach.y0 + y1 as i32)
        });
        self.surface.state.clip = Some(Rc::new(Clip { bounds: reach, mask: Some(mask), full }));
    }

    /// Lays what `shader` paints through `coverage` on the canvas, with what the state says, and `op` in place of its own.
    fn draw(&mut self, coverage: Coverage, shader: &Shader, op: Option<Op>) {
        let state = &self.surface.state;
        let op = op.unwrap_or(state.op);
        if shader.is_none() && !op.unbounded() {
            return;
        }
        let clipping = self.surface.clip_bounds();
        if clipping.is_empty() {
            return;
        }
        let shadow = op != Op::Clear
            && state.shadow_color[3] != 0
            && (state.shadow_blur > 0.0 || state.shadow_x != 0.0 || state.shadow_y != 0.0);
        let filtered = op != Op::Clear && state.filter.is_some();
        if !shadow && !filtered && !op.unbounded() {
            self.draw_direct(coverage, shader, op, clipping);
        } else if let Coverage::Rect(rect) = coverage {
            let mut lines = std::mem::take(&mut self.world.tools.rect_lines);
            lines.clear();
            rect_lines(rect, &mut lines);
            self.draw_layered(Coverage::Lines(&lines, false), shader, op, clipping, shadow);
            self.world.tools.rect_lines = lines;
        } else if let Coverage::Mask8 { x, y, width, height, data } = coverage {
            let floats: Vec<f32> = data.iter().map(|v| f32::from(*v) / 255.0).collect();
            self.draw_layered(Coverage::Mask { x, y, width, height, data: &floats }, shader, op, clipping, shadow);
        } else {
            self.draw_layered(coverage, shader, op, clipping, shadow);
        }
    }

    /// The usual way: each row as it's worked out, straight on the canvas.
    fn draw_direct(&mut self, coverage: Coverage, shader: &Shader, op: Op, clipping: IRect) {
        let surface = &mut *self.surface;
        let alpha = surface.state.alpha;
        let clip = surface.state.clip.clone();
        let masked = clip.as_ref().is_some_and(|c| c.mask.is_some());
        let width = surface.width as usize;
        let pixels = &mut surface.pixels;
        let solid = shader.solid();
        let mut touched = IRect::new(0, 0, 0, 0);
        // A color's alpha with the global alpha in it, and its colors multiplied by that, in whole steps of 255, as Skia has a
        // paint's color before it's blended.
        let solid8 = solid.map(|s| quantize(s, alpha));
        let solid32 = solid8.map(paint::pack);
        let alpha8 = (alpha * 255.0 + 0.5) as u32;
        // A rectangle in a color, as `fillRect()` draws: its edges a pixel at a time, and the rest of each row as it is.
        if let (Coverage::Rect([x0, y0, x1, y1]), Some(color), Op::SourceOver, false) = (&coverage, solid32, op, masked) {
            let (x0, y0, x1, y1) = (*x0, *y0, *x1, *y1);
            if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
                return;
            }
            let reach = IRect::new(raster::floor(x0.max(-1.0e8)) as i32, raster::floor(y0.max(-1.0e8)) as i32, raster::ceil(x1.min(1.0e8)) as i32, raster::ceil(y1.min(1.0e8)) as i32)
                .intersect(&clipping);
            if reach.is_empty() {
                return;
            }
            if let Some((top, bottom)) = pixels::rect_solid(pixels, width, [x0, y0, x1, y1], [reach.x0, reach.y0, reach.x1, reach.y1], color) {
                surface.changed(IRect::new(reach.x0, top, reach.x1, bottom));
            }
            return;
        }
        // A narrow one through a clip that's soft at its edges, as a chart's bars are: the same, without a call for each row.
        if let (Coverage::Rect([x0, y0, x1, y1]), Some(color), Op::SourceOver, Some(clip)) = (&coverage, solid32, op, clip.as_ref())
            && let Some(mask) = clip.mask.as_ref()
            && x0.is_finite()
            && y0.is_finite()
            && x1.is_finite()
            && y1.is_finite()
        {
            let (x0, y0, x1, y1) = (*x0, *y0, *x1, *y1);
            let reach = IRect::new(raster::floor(x0.max(-1.0e8)) as i32, raster::floor(y0.max(-1.0e8)) as i32, raster::ceil(x1.min(1.0e8)) as i32, raster::ceil(y1.min(1.0e8)) as i32)
                .intersect(&clipping);
            if reach.is_empty() {
                return;
            }
            if (reach.x1 - reach.x0) as usize <= pixels::NARROW {
                let (b, f) = (clip.bounds, clip.full);
                let drawn = pixels::rect_masked(
                    pixels,
                    width,
                    [x0, y0, x1, y1],
                    [reach.x0, reach.y0, reach.x1, reach.y1],
                    color,
                    mask,
                    [b.x0, b.y0, b.x1, b.y1],
                    [f.x0, f.y0, f.x1, f.y1],
                );
                if let Some((top, bottom)) = drawn {
                    surface.changed(IRect::new(reach.x0, top, reach.x1, bottom));
                }
                return;
            }
        }
        let mut shaded = std::mem::take(&mut self.world.tools.shaded);
        let mut shaded32 = std::mem::take(&mut self.world.tools.shaded32);
        let mut floats = std::mem::take(&mut self.world.tools.floats);
        let mut row_of = |y: i32, x: i32, row: Row| {
            let len = row.len();
            touched = touched.union(&IRect::new(x, y, x + len as i32, y + 1));
            let start = (y as usize * width + x as usize) * 4;
            let out = &mut pixels[start..start + len * 4];
            if solid32.is_none() {
                shaded32.resize(len, 0);
                shader.row32(x, y, alpha8, &mut shaded32[..len], &mut shaded);
            }
            // The usual way, four pixels at a time.
            if op == Op::SourceOver && !masked {
                match (solid32, row) {
                    (Some(color), Row::Float(row)) => pixels::blend_solid(out, row, color),
                    (Some(color), Row::Byte(row)) => pixels::blend_solid8(out, row, color),
                    (None, Row::Float(row)) => pixels::blend_shaded(out, row, &shaded32[..len]),
                    (None, Row::Byte(row)) => {
                        floats.clear();
                        floats.extend(row.iter().map(|v| f32::from(*v) / 255.0));
                        pixels::blend_shaded(out, &floats, &shaded32[..len]);
                    }
                }
                return;
            }
            // Through a clip that's soft at its edges, as one on fractions of a pixel is: four pixels at a time too.
            if op == Op::SourceOver
                && let Some(clip) = clip.as_ref()
                && let Some(mask) = clip.mask.as_ref()
            {
                let w = (clip.bounds.x1 - clip.bounds.x0) as usize;
                let from = (y - clip.bounds.y0) as usize * w + (x - clip.bounds.x0) as usize;
                let row = match row {
                    Row::Float(row) => row,
                    Row::Byte(row) => {
                        floats.clear();
                        floats.extend(row.iter().map(|v| f32::from(*v) / 255.0));
                        &floats[..]
                    }
                };
                let source = solid32.is_none().then(|| &shaded32[..len]);
                // Where it lets all of each pixel through, it needn't be read.
                let full = clip.full;
                let all = y >= full.y0 && y < full.y1 && x >= full.x0 && x + len as i32 <= full.x1;
                let mask = (!all).then(|| &mask[from..from + len]);
                pixels::blend_masked(out, row, mask, solid32.unwrap_or(0), source);
                return;
            }
            for i in 0..len {
                let mut k = row.at(i);
                if masked {
                    k *= clip.as_ref().map_or(1.0, |c| c.at(x + i as i32, y));
                }
                if k <= 0.0 {
                    continue;
                }
                let d = &mut out[i * 4..i * 4 + 4];
                if op == Op::SourceOver {
                    let s8 = solid8.unwrap_or_else(|| paint::unpack(shaded32[i]));
                    let cover = (k * 255.0 + 0.5) as u32;
                    let s8 = if cover >= 255 {
                        s8
                    } else {
                        [div255(s8[0] * cover), div255(s8[1] * cover), div255(s8[2] * cover), div255(s8[3] * cover)]
                    };
                    if s8[3] == 255 {
                        d.copy_from_slice(&[s8[0] as u8, s8[1] as u8, s8[2] as u8, 255]);
                        continue;
                    }
                    let keep = 255 - s8[3];
                    for c in 0..4 {
                        d[c] = (s8[c] + div255(u32::from(d[c]) * keep)).min(255) as u8;
                    }
                } else if op == Op::Clear {
                    for value in d.iter_mut() {
                        *value = (f32::from(*value) * (1.0 - k) + 0.5) as u8;
                    }
                } else {
                    let s8 = solid8.unwrap_or_else(|| paint::unpack(shaded32[i]));
                    let sp = [s8[0] as f32 * k / 255.0, s8[1] as f32 * k / 255.0, s8[2] as f32 * k / 255.0, s8[3] as f32 * k / 255.0];
                    let dp = [f32::from(d[0]) / 255.0, f32::from(d[1]) / 255.0, f32::from(d[2]) / 255.0, f32::from(d[3]) / 255.0];
                    store(d, paint::composite(op, sp, dp));
                }
            }
        };
        let mut row_fn = |y: i32, x: i32, row: &[f32]| row_of(y, x, Row::Float(row));
        match coverage {
            Coverage::Lines(lines, even_odd) => {
                self.world.tools.raster.fill(lines, clipping, even_odd, &mut row_fn);
            }
            Coverage::Rect([x0, y0, x1, y1]) => {
                if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
                    self.world.tools.shaded = shaded;
                    self.world.tools.shaded32 = shaded32;
                    return;
                }
                let reach = IRect::new(raster::floor(x0.max(-1.0e8)) as i32, raster::floor(y0.max(-1.0e8)) as i32, raster::ceil(x1.min(1.0e8)) as i32, raster::ceil(y1.min(1.0e8)) as i32)
                    .intersect(&clipping);
                if reach.is_empty() {
                    self.world.tools.shaded = shaded;
                    self.world.tools.shaded32 = shaded32;
                    return;
                }
                let mut across = std::mem::take(&mut self.world.tools.across);
                across.clear();
                across.extend((reach.x0..reach.x1).map(|px| ((px + 1) as f32).min(x1) - (px as f32).max(x0)));
                let mut down = std::mem::take(&mut self.world.tools.down);
                for py in reach.y0..reach.y1 {
                    let part = ((py + 1) as f32).min(y1) - (py as f32).max(y0);
                    if part <= 0.0 {
                        continue;
                    }
                    if part >= 1.0 {
                        row_fn(py, reach.x0, &across);
                    } else {
                        down.clear();
                        down.extend(across.iter().map(|amount| amount * part));
                        row_fn(py, reach.x0, &down);
                    }
                }
                self.world.tools.across = across;
                self.world.tools.down = down;
            }
            Coverage::Mask { x, y, width: mw, height: mh, data } => {
                let area = IRect::new(x, y, x + mw as i32, y + mh as i32).intersect(&clipping);
                for py in area.y0..area.y1 {
                    if area.is_empty() {
                        break;
                    }
                    let from = (py - y) as usize * mw + (area.x0 - x) as usize;
                    let row = &data[from..from + (area.x1 - area.x0) as usize];
                    if row.iter().any(|v| *v > 0.0) {
                        row_fn(py, area.x0, row);
                    }
                }
            }
            Coverage::Mask8 { x, y, width: mw, height: mh, data } => {
                let area = IRect::new(x, y, x + mw as i32, y + mh as i32).intersect(&clipping);
                for py in area.y0..area.y1 {
                    if area.is_empty() {
                        break;
                    }
                    let from = (py - y) as usize * mw + (area.x0 - x) as usize;
                    let row = &data[from..from + (area.x1 - area.x0) as usize];
                    if row.iter().any(|v| *v != 0) {
                        row_of(py, area.x0, Row::Byte(row));
                    }
                }
            }
        }
        self.world.tools.floats = floats;
        self.world.tools.shaded = shaded;
        self.world.tools.shaded32 = shaded32;
        surface.changed(touched);
    }

    /// With a shadow, a filter, or a way of laying it on that changes what isn't drawn on: what's drawn is worked out by itself
    /// first, then laid on the canvas.
    fn draw_layered(&mut self, coverage: Coverage, shader: &Shader, op: Op, clipping: IRect, shadow: bool) {
        let state = self.surface.state.clone();
        let filters = state.filter.clone();
        // How far a filter's blurs and shadows reach past what's drawn.
        let mut margin = 0usize;
        if let Some(filters) = &filters {
            for filter in filters.iter() {
                margin += match filter {
                    Filter::Blur(sigma) => blur::reach(*sigma),
                    Filter::Shadow { x, y, blur: b, .. } => blur::reach(*b) + x.abs().max(y.abs()).ceil() as usize,
                    Filter::Matrix(_) => 0,
                };
            }
        }
        let whole = self.surface.whole();
        let reach = match &coverage {
            Coverage::Rect(_) => None,
            Coverage::Lines(lines, _) => IRect::around(lines),
            Coverage::Mask { x, y, width, height, .. } | Coverage::Mask8 { x, y, width, height, .. } => {
                Some(IRect::new(*x, *y, *x + *width as i32, *y + *height as i32))
            }
        };
        let Some(reach) = reach else {
            if op.unbounded() {
                self.composite_layer(&Layer::empty(), op, clipping, state.alpha, true);
            }
            return;
        };
        // What could show, which a shadow can bring in from off the canvas.
        let pad = margin as i32;
        let shadow_reach = if shadow {
            let r = blur::reach(state.shadow_blur / 2.0) as i32;
            let (sx, sy) = (state.shadow_x.round() as i32, state.shadow_y.round() as i32);
            IRect::new(whole.x0 - sx - r - pad, whole.y0 - sy - r - pad, whole.x1 - sx + r + pad, whole.y1 - sy + r + pad)
        } else {
            IRect::new(whole.x0 - pad, whole.y0 - pad, whole.x1 + pad, whole.y1 + pad)
        };
        let area = reach.intersect(&shadow_reach.union(&IRect::new(whole.x0 - pad, whole.y0 - pad, whole.x1 + pad, whole.y1 + pad)));
        if area.is_empty() {
            if op.unbounded() {
                self.composite_layer(&Layer::empty(), op, clipping, state.alpha, true);
            }
            return;
        }
        // Room for the shadow's blur too, which the shadow is worked out in before it's moved.
        let pad = pad + if shadow { blur::reach(state.shadow_blur / 2.0) as i32 } else { 0 };
        let area = IRect::new(area.x0 - pad, area.y0 - pad, area.x1 + pad, area.y1 + pad);
        let mut layer = Layer::new(area);
        let mut shaded = std::mem::take(&mut self.world.tools.shaded);
        {
            let mut put = |y: i32, x: i32, row: &[f32]| {
                shaded.resize(row.len(), [0.0; 4]);
                shader.row(x, y, &mut shaded[..row.len()]);
                for (i, &amount) in row.iter().enumerate() {
                    let px = x + i as i32;
                    if let Some(at) = layer.index(px, y) {
                        let s = shaded[i];
                        layer.pixels[at] = [s[0] * amount / 255.0, s[1] * amount / 255.0, s[2] * amount / 255.0, s[3] * amount / 255.0];
                    }
                }
            };
            match coverage {
                Coverage::Lines(lines, even_odd) => {
                    self.world.tools.raster.fill(lines, area, even_odd, &mut put);
                }
                // Made into a `Mask` by `draw()` before it comes here.
                Coverage::Rect(_) | Coverage::Mask8 { .. } => {}
                Coverage::Mask { x, y, width, height, data } => {
                    let inside = IRect::new(x, y, x + width as i32, y + height as i32).intersect(&area);
                    for py in inside.y0..inside.y1 {
                        if inside.is_empty() {
                            break;
                        }
                        let from = (py - y) as usize * width + (inside.x0 - x) as usize;
                        put(py, inside.x0, &data[from..from + (inside.x1 - inside.x0) as usize]);
                    }
                }
            }
        }
        self.world.tools.shaded = shaded;
        if let Some(filters) = &filters {
            for filter in filters.iter() {
                layer.filter(filter);
            }
        }
        if shadow {
            let mut shade = layer.shadow(state.shadow_color, state.shadow_blur / 2.0, state.shadow_x, state.shadow_y);
            shade.shift(state.shadow_x.round() as i32, state.shadow_y.round() as i32);
            self.composite_layer(&shade, op, clipping, state.alpha, false);
        }
        self.composite_layer(&layer, op, clipping, state.alpha, op.unbounded());
    }

    /// Lays a layer on the canvas, everywhere that's clipped to if `everywhere`, and only where the layer is otherwise.
    fn composite_layer(&mut self, layer: &Layer, op: Op, clipping: IRect, alpha: f32, everywhere: bool) {
        let surface = &mut *self.surface;
        let clip = surface.state.clip.clone();
        let area = if everywhere { clipping } else { layer.area.intersect(&clipping) };
        if area.is_empty() {
            return;
        }
        let width = surface.width as usize;
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                let s = layer.index(x, y).map_or([0.0; 4], |at| layer.pixels[at]);
                let m = clip.as_ref().map_or(1.0, |c| c.at(x, y));
                if m <= 0.0 {
                    continue;
                }
                let at = (y as usize * width + x as usize) * 4;
                let d = &mut surface.pixels[at..at + 4];
                let dp = [f32::from(d[0]) / 255.0, f32::from(d[1]) / 255.0, f32::from(d[2]) / 255.0, f32::from(d[3]) / 255.0];
                let sp = [s[0] * alpha, s[1] * alpha, s[2] * alpha, s[3] * alpha];
                let r = paint::composite(op, sp, dp);
                let out = [dp[0] + (r[0] - dp[0]) * m, dp[1] + (r[1] - dp[1]) * m, dp[2] + (r[2] - dp[2]) * m, dp[3] + (r[3] - dp[3]) * m];
                store(d, out);
            }
        }
        surface.changed(area);
    }

    fn draw_image(&mut self, image: &Rc<Image>, v: [f32; 8]) {
        let [mut sx, mut sy, mut sw, mut sh, mut dx, mut dy, mut dw, mut dh] = v;
        if !v.iter().all(|value| value.is_finite()) {
            return;
        }
        // A rectangle given backwards is the same rectangle.
        if sw < 0.0 {
            sx += sw;
            sw = -sw;
        }
        if sh < 0.0 {
            sy += sh;
            sh = -sh;
        }
        if dw < 0.0 {
            dx += dw;
            dw = -dw;
        }
        if dh < 0.0 {
            dy += dh;
            dh = -dh;
        }
        // What's asked for outside the picture is cut off, and so is as much of where it goes.
        let (iw, ih) = (image.width as f32, image.height as f32);
        if sw == 0.0 || sh == 0.0 || dw == 0.0 || dh == 0.0 {
            return;
        }
        let (scale_x, scale_y) = (dw / sw, dh / sh);
        if sx < 0.0 {
            dx -= sx * scale_x;
            dw += sx * scale_x;
            sw += sx;
            sx = 0.0;
        }
        if sy < 0.0 {
            dy -= sy * scale_y;
            dh += sy * scale_y;
            sh += sy;
            sy = 0.0;
        }
        if sx + sw > iw {
            let over = sx + sw - iw;
            sw -= over;
            dw -= over * scale_x;
        }
        if sy + sh > ih {
            let over = sy + sh - ih;
            sh -= over;
            dh -= over * scale_y;
        }
        if sw <= 0.0 || sh <= 0.0 || dw <= 0.0 || dh <= 0.0 {
            return;
        }
        let state = &self.surface.state;
        let m = state.transform;
        let Some(to_user) = m.invert() else {
            return;
        };
        let user_to_image = Matrix::new([sw / dw, 0.0, 0.0, sh / dh, sx - dx * sw / dw, sy - dy * sh / dh]);
        // Sampling between pixels reaches past the part asked for, into the rest of the picture, as Chromium's does.
        let bounds = [0, 0, image.width as i32, image.height as i32];
        // The quick way, for a picture that goes where it is, pixel for pixel, as sprites do.
        let simple = state.op == Op::SourceOver
            && state.clip.as_ref().is_none_or(|clip| clip.mask.is_none())
            && state.filter.is_none()
            && (state.shadow_color[3] == 0 || (state.shadow_blur == 0.0 && state.shadow_x == 0.0 && state.shadow_y == 0.0))
            && m.a == 1.0
            && m.d == 1.0
            && m.is_axis_aligned()
            && sw == dw
            && sh == dh
            && [sx, sy, sw, sh].iter().all(|v| *v == v.round())
            && (dx + m.e) == (dx + m.e).round()
            && (dy + m.f) == (dy + m.f).round();
        if simple {
            self.blit(image, (sx as i32, sy as i32, sw as i32, sh as i32), ((dx + m.e) as i32, (dy + m.f) as i32));
            return;
        }
        let shader = Shader::Image {
            image: image.clone(),
            to_image: user_to_image.then(&to_user),
            smooth: state.smoothing,
            repeat: Repeat::Neither,
            bounds,
            pattern: false,
        };
        let mut path = std::mem::take(&mut self.world.tools.path);
        path.clear();
        // A picture that isn't turned has whole pixels at its edges, rounded to the nearest, as Chromium draws it.
        let (x0, y0) = m.apply(dx, dy);
        let (x1, y1) = m.apply(dx + dw, dy + dh);
        let (l, r, t, b) = (x0.min(x1).round(), x0.max(x1).round(), y0.min(y1).round(), y0.max(y1).round());
        if m.is_axis_aligned() && (x1 - x0).abs() >= 1.0 && (y1 - y0).abs() >= 1.0 {
            if r <= l || b <= t {
                self.world.tools.path = path;
                return;
            }
            let shader = shader;
            self.world.tools.path = path;
            self.draw(Coverage::Rect([l, t, r, b]), &shader, None);
            return;
        } else {
            path.rect(&m, dx, dy, dw, dh);
        }
        let mut lines = std::mem::take(&mut self.world.tools.lines);
        lines.clear();
        path.fill_lines(&mut lines);
        self.draw(Coverage::Lines(&lines, false), &shader, None);
        self.world.tools.lines = lines;
        self.world.tools.path = path;
    }

    /// Lays part of a picture on the canvas pixel for pixel, at `to`.
    fn blit(&mut self, image: &Image, (sx, sy, sw, sh): (i32, i32, i32, i32), to: (i32, i32)) {
        let surface = &mut *self.surface;
        let target = IRect::new(to.0, to.1, to.0 + sw, to.1 + sh).intersect(&surface.clip_bounds());
        if target.is_empty() {
            return;
        }
        let alpha8 = (surface.state.alpha * 255.0 + 0.5) as u32;
        let width = surface.width as usize;
        let span = (target.x1 - target.x0) as usize;
        for y in target.y0..target.y1 {
            let iy = (sy + y - to.1) as usize;
            let from = (iy * image.width as usize + (sx + target.x0 - to.0) as usize) * 4;
            let source = &image.pixels[from..from + span * 4];
            let at = (y as usize * width + target.x0 as usize) * 4;
            pixels::blit_row(&mut surface.pixels[at..at + span * 4], source, alpha8);
        }
        surface.changed(target);
    }

    fn text(&mut self, text: &str, x: f32, y: f32, most: f64, stroke: bool) {
        if !(x.is_finite() && y.is_finite()) || most.is_nan() || most <= 0.0 {
            return;
        }
        // What's needed of the state, without copying the rest of it.
        let state = &self.surface.state;
        let (font, letter_spacing, rtl, align) = (state.font.clone(), state.letter_spacing, state.rtl, state.align);
        let (baseline_kind, m, style) = (state.baseline, state.transform, if stroke { state.stroke } else { state.fill });
        let shaped = self.world.tools.text.shape(text, &font, letter_spacing, rtl);
        if shaped.glyphs.is_empty() {
            return;
        }
        let squeeze = if most.is_finite() && f64::from(shaped.width) > most { most as f32 / shaped.width } else { 1.0 };
        let left = x + align_offset(align, rtl, shaped.width * squeeze);
        let baseline = y + baseline_shift(baseline_kind, &shaped);
        let shader = self.shader(style);
        let uniform = m.is_axis_aligned() && m.a == m.d && m.a > 0.0;
        if !stroke && squeeze == 1.0 && uniform {
            // Drawn the system's gray way, as a browser draws it, through Skia's gamma for the text's brightness.
            let brightness = match style {
                Style::Color(c) => ((u32::from(c[0]) * 54 + u32::from(c[1]) * 183 + u32::from(c[2]) * 19) / 256) as u8,
                _ => 0x7F,
            };
            let size = font.size * m.a;
            let tools = &mut self.world.tools;
            let mut placed = std::mem::take(&mut tools.placed);
            placed.clear();
            let pens = shaped.glyphs.iter().map(|g| m.apply(left + g.x, baseline + g.y));
            tools.text.draw_system(&shaped, size, pens, |px, py, bitmap| {
                placed.push((px + bitmap.left, py + bitmap.top, bitmap.clone()));
            });
            let area = placed.iter().fold(None::<IRect>, |acc, (px, py, b)| {
                let r = IRect::new(*px, *py, px + b.width as i32, py + b.height as i32);
                Some(acc.map_or(r, |a| a.union(&r)))
            });
            let Some(area) = area else {
                tools.placed = placed;
                return;
            };
            let (w, h) = ((area.x1 - area.x0) as usize, (area.y1 - area.y0) as usize);
            let mut mask = std::mem::take(&mut tools.mask8);
            mask.clear();
            mask.resize(w * h, 0);
            let gamma = tools.text.gamma(brightness);
            // Glyphs that overlap add up, to all of the pixel at most.
            for (px, py, bitmap) in &placed {
                let bw = bitmap.width as usize;
                for row in 0..bitmap.height as usize {
                    let base = (*py - area.y0) as usize * w + row * w + (*px - area.x0) as usize;
                    for (cell, &amount) in mask[base..base + bw].iter_mut().zip(&bitmap.coverage[row * bw..row * bw + bw]) {
                        *cell = cell.saturating_add(gamma[usize::from(amount)]);
                    }
                }
            }
            placed.clear();
            self.world.tools.placed = placed;
            self.draw(Coverage::Mask8 { x: area.x0, y: area.y0, width: w, height: h, data: &mask }, &shader, None);
            self.world.tools.mask8 = mask;
            return;
        }
        let place = m.then(&Matrix::new([squeeze, 0.0, 0.0, 1.0, left, baseline]));
        let mut path = std::mem::take(&mut self.world.tools.path);
        path.clear();
        self.world.tools.text.outlines(&shaped, &font, &place, &mut path);
        let mut lines = std::mem::take(&mut self.world.tools.lines);
        lines.clear();
        if stroke {
            let line = self.surface.state.line.clone();
            self.world.tools.stroker.stroke(&path, &line, &m, &mut lines);
            self.draw(Coverage::Lines(&lines, false), &shader, None);
        } else {
            // Through the same gamma as text the system draws, as Skia puts all text through it.
            path.fill_lines(&mut lines);
            let brightness = match style {
                Style::Color(c) => ((u32::from(c[0]) * 54 + u32::from(c[1]) * 183 + u32::from(c[2]) * 19) / 256) as u8,
                _ => 0x7F,
            };
            let clipping = self.surface.clip_bounds();
            if let Some(area) = IRect::around(&lines).map(|area| area.intersect(&clipping)).filter(|area| !area.is_empty()) {
                let (w, h) = ((area.x1 - area.x0) as usize, (area.y1 - area.y0) as usize);
                let mut mask = std::mem::take(&mut self.world.tools.mask);
                mask.clear();
                mask.resize(w * h, 0.0);
                let gamma = *self.world.tools.text.gamma(brightness);
                self.world.tools.raster.fill(&lines, area, false, |y, x, row| {
                    let base = (y - area.y0) as usize * w + (x - area.x0) as usize;
                    for (i, amount) in row.iter().enumerate() {
                        let byte = (amount * 255.0 + 0.5) as usize;
                        mask[base + i] = f32::from(gamma[byte.min(255)]) / 255.0;
                    }
                });
                self.draw(Coverage::Mask { x: area.x0, y: area.y0, width: w, height: h, data: &mask }, &shader, None);
                self.world.tools.mask = mask;
            }
        }
        self.world.tools.lines = lines;
        self.world.tools.path = path;
    }
}

use paint::div255;

/// A color from 0 to 255, multiplied by alpha, with `alpha` more of it, as whole steps: alpha first, then each color from the
/// color without alpha times that, as Skia makes a paint's color.
#[inline]
fn quantize(s: [f32; 4], alpha: f32) -> [u32; 4] {
    let a = (s[3] * alpha + 0.5).clamp(0.0, 255.0) as u32;
    if s[3] <= 0.0 || a == 0 {
        return [0; 4];
    }
    // The colors without alpha, then with the alpha it has now.
    let un = |c: f32| ((c * 255.0 / s[3]) + 0.5).clamp(0.0, 255.0) as u32;
    [div255(un(s[0]) * a), div255(un(s[1]) * a), div255(un(s[2]) * a), a]
}

/// Walks a line a pixel across from `a` to `b`: along its longer way a pixel at a time, and across, shared between the two pixels
/// its middle falls between, which `put` is given as whether that's across, the pixels along, where it starts across and how it
/// slopes, and where along it starts and ends.
fn hair_segment(a: (f32, f32), b: (f32, f32), put: &mut impl FnMut(bool, (i32, i32), f32, f32, (f32, f32))) {
    if ![a.0, a.1, b.0, b.1].iter().all(|v| v.is_finite() && v.abs() < 1.0e7) {
        return;
    }
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let across = dx.abs() >= dy.abs();
    let (u0, v0, u1, v1) = if across { (a.0, a.1, b.0, b.1) } else { (a.1, a.0, b.1, b.0) };
    let (u0, v0, u1, v1) = if u0 <= u1 { (u0, v0, u1, v1) } else { (u1, v1, u0, v0) };
    let length = u1 - u0;
    if length <= 0.0 {
        return;
    }
    let slope = (v1 - v0) / length;
    put(across, (raster::floor(u0) as i32, raster::floor(u1) as i32), v0, slope, (u0, u1));
}

/// Puts a color from 0 to 1, multiplied by alpha, in a pixel.
#[inline]
fn store(d: &mut [u8], c: [f32; 4]) {
    let a = c[3].clamp(0.0, 1.0);
    d[3] = (a * 255.0 + 0.5) as u8;
    for i in 0..3 {
        d[i] = (c[i].clamp(0.0, a) * 255.0 + 0.5) as u8;
    }
}

/// The rectangle a path is, if it's one on whole pixels, as `rect()` makes.
fn whole_rect(path: &Path) -> Option<IRect> {
    let mut found = None;
    for (points, closed) in path.each() {
        if points.len() <= 1 {
            continue;
        }
        if found.is_some() || !closed || points.len() != 4 {
            return None;
        }
        let xs = [points[0].0, points[1].0, points[2].0, points[3].0];
        let ys = [points[0].1, points[1].1, points[2].1, points[3].1];
        let axis = (xs[0] == xs[1] && ys[1] == ys[2] && xs[2] == xs[3] && ys[3] == ys[0])
            || (ys[0] == ys[1] && xs[1] == xs[2] && ys[2] == ys[3] && xs[3] == xs[0]);
        if !axis || xs.iter().chain(ys.iter()).any(|v| *v != v.round()) {
            return None;
        }
        let (x0, x1) = (xs.iter().copied().fold(f32::MAX, f32::min), xs.iter().copied().fold(f32::MIN, f32::max));
        let (y0, y1) = (ys.iter().copied().fold(f32::MAX, f32::min), ys.iter().copied().fold(f32::MIN, f32::max));
        found = Some(IRect::new(x0 as i32, y0 as i32, x1 as i32, y1 as i32));
    }
    found
}

/// Pixels drawn by themselves, from 0 to 1 and multiplied by alpha, over `area` of the canvas.
struct Layer {
    area: IRect,
    pixels: Vec<[f32; 4]>,
}

impl Layer {
    fn new(area: IRect) -> Self {
        let size = ((area.x1 - area.x0).max(0) as usize) * ((area.y1 - area.y0).max(0) as usize);
        Self { area, pixels: vec![[0.0; 4]; size] }
    }

    fn empty() -> Self {
        Self { area: IRect::new(0, 0, 0, 0), pixels: Vec::new() }
    }

    #[inline]
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let a = &self.area;
        (x >= a.x0 && x < a.x1 && y >= a.y0 && y < a.y1).then(|| (y - a.y0) as usize * (a.x1 - a.x0) as usize + (x - a.x0) as usize)
    }

    fn width(&self) -> usize {
        (self.area.x1 - self.area.x0) as usize
    }

    fn height(&self) -> usize {
        (self.area.y1 - self.area.y0) as usize
    }

    fn shift(&mut self, x: i32, y: i32) {
        self.area = IRect::new(self.area.x0 + x, self.area.y0 + y, self.area.x1 + x, self.area.y1 + y);
    }

    /// Its shadow: its alpha, blurred, in `color`, not yet moved.
    fn shadow(&self, color: [u8; 4], sigma: f32, _x: f32, _y: f32) -> Layer {
        let (w, h) = (self.width(), self.height());
        let mut alpha: Vec<f32> = self.pixels.iter().map(|p| p[3]).collect();
        blur::blur(&mut alpha, w, h, 1, sigma, sigma);
        let ca = f32::from(color[3]) / 255.0;
        let c = [f32::from(color[0]) / 255.0 * ca, f32::from(color[1]) / 255.0 * ca, f32::from(color[2]) / 255.0 * ca, ca];
        Layer { area: self.area, pixels: alpha.iter().map(|a| [c[0] * a, c[1] * a, c[2] * a, c[3] * a]).collect() }
    }

    fn filter(&mut self, filter: &Filter) {
        let (w, h) = (self.width(), self.height());
        match filter {
            Filter::Blur(sigma) => {
                let mut flat: Vec<f32> = self.pixels.iter().flat_map(|p| *p).collect();
                blur::blur(&mut flat, w, h, 4, *sigma, *sigma);
                for (pixel, four) in self.pixels.iter_mut().zip(flat.chunks_exact(4)) {
                    *pixel = [four[0], four[1], four[2], four[3]];
                }
            }
            Filter::Matrix(m) => {
                for pixel in &mut self.pixels {
                    let a = pixel[3];
                    if a <= 0.0 && m[18] == 0.0 && m[19] == 0.0 {
                        continue;
                    }
                    let un = if a > 0.0 { [pixel[0] / a, pixel[1] / a, pixel[2] / a] } else { [0.0; 3] };
                    let mut out = [0.0f32; 4];
                    for row in 0..4 {
                        out[row] = (m[row * 5] * un[0] + m[row * 5 + 1] * un[1] + m[row * 5 + 2] * un[2] + m[row * 5 + 3] * a + m[row * 5 + 4])
                            .clamp(0.0, 1.0);
                    }
                    *pixel = [out[0] * out[3], out[1] * out[3], out[2] * out[3], out[3]];
                }
            }
            Filter::Shadow { x, y, blur: b, color } => {
                // A filter's shadow is blurred by its length itself, as `blur()` is, where a canvas's own shadow is by half.
                let mut shade = self.shadow(*color, *b, *x, *y);
                let (sx, sy) = (x.round() as i32, y.round() as i32);
                shade.shift(sx, sy);
                // The shadow goes under what's there.
                let mut out = vec![[0.0f32; 4]; self.pixels.len()];
                for py in self.area.y0..self.area.y1 {
                    for px in self.area.x0..self.area.x1 {
                        let at = self.index(px, py).unwrap_or(0);
                        let under = shade.index(px, py).map_or([0.0; 4], |i| shade.pixels[i]);
                        out[at] = paint::composite(Op::SourceOver, self.pixels[at], under);
                    }
                }
                self.pixels = out;
            }
        }
    }
}

/// What a `filter` string says, as CSS has it, or `None` for `none` or something that isn't one.
fn parse_filter(text: &str) -> Option<Vec<Filter>> {
    let text = text.trim();
    if text.is_empty() || text == "none" {
        return None;
    }
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.trim().is_empty() {
        rest = rest.trim_start();
        let open = rest.find('(')?;
        let name = rest[..open].trim().to_ascii_lowercase();
        let mut depth = 0;
        let mut close = None;
        for (i, c) in rest.char_indices().skip(open) {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close?;
        let args = rest[open + 1..close].trim();
        rest = &rest[close + 1..];
        let amount = |default: f32| -> Option<f32> {
            if args.is_empty() {
                return Some(default);
            }
            if let Some(percent) = args.strip_suffix('%') {
                return percent.trim().parse::<f32>().ok().map(|v| v / 100.0);
            }
            args.parse::<f32>().ok()
        };
        let length = |text: &str| -> Option<f32> {
            let text = text.trim();
            if text == "0" {
                return Some(0.0);
            }
            text.strip_suffix("px").and_then(|v| v.trim().parse().ok())
        };
        let filter = match name.as_str() {
            "blur" => Filter::Blur(if args.is_empty() { 0.0 } else { length(args)? }),
            "brightness" => {
                let v = amount(1.0)?;
                Filter::Matrix([v, 0.0, 0.0, 0.0, 0.0, 0.0, v, 0.0, 0.0, 0.0, 0.0, 0.0, v, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0])
            }
            "contrast" => {
                let v = amount(1.0)?;
                let o = 0.5 - 0.5 * v;
                Filter::Matrix([v, 0.0, 0.0, 0.0, o, 0.0, v, 0.0, 0.0, o, 0.0, 0.0, v, 0.0, o, 0.0, 0.0, 0.0, 1.0, 0.0])
            }
            "opacity" => {
                let v = amount(1.0)?.clamp(0.0, 1.0);
                Filter::Matrix([1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, v, 0.0])
            }
            "invert" => {
                let v = amount(1.0)?.clamp(0.0, 1.0);
                let s = 1.0 - 2.0 * v;
                Filter::Matrix([s, 0.0, 0.0, 0.0, v, 0.0, s, 0.0, 0.0, v, 0.0, 0.0, s, 0.0, v, 0.0, 0.0, 0.0, 1.0, 0.0])
            }
            "grayscale" => {
                let v = 1.0 - amount(1.0)?.clamp(0.0, 1.0);
                Filter::Matrix([
                    0.2126 + 0.7874 * v, 0.7152 - 0.7152 * v, 0.0722 - 0.0722 * v, 0.0, 0.0,
                    0.2126 - 0.2126 * v, 0.7152 + 0.2848 * v, 0.0722 - 0.0722 * v, 0.0, 0.0,
                    0.2126 - 0.2126 * v, 0.7152 - 0.7152 * v, 0.0722 + 0.9278 * v, 0.0, 0.0,
                    0.0, 0.0, 0.0, 1.0, 0.0,
                ])
            }
            "sepia" => {
                let v = 1.0 - amount(1.0)?.clamp(0.0, 1.0);
                Filter::Matrix([
                    0.393 + 0.607 * v, 0.769 - 0.769 * v, 0.189 - 0.189 * v, 0.0, 0.0,
                    0.349 - 0.349 * v, 0.686 + 0.314 * v, 0.168 - 0.168 * v, 0.0, 0.0,
                    0.272 - 0.272 * v, 0.534 - 0.534 * v, 0.131 + 0.869 * v, 0.0, 0.0,
                    0.0, 0.0, 0.0, 1.0, 0.0,
                ])
            }
            "saturate" => {
                let s = amount(1.0)?;
                Filter::Matrix([
                    0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s, 0.0, 0.0,
                    0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s, 0.0, 0.0,
                    0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s, 0.0, 0.0,
                    0.0, 0.0, 0.0, 1.0, 0.0,
                ])
            }
            "hue-rotate" => {
                let degrees = if args.is_empty() {
                    0.0
                } else if let Some(v) = args.strip_suffix("deg") {
                    v.trim().parse::<f32>().ok()?
                } else if let Some(v) = args.strip_suffix("turn") {
                    v.trim().parse::<f32>().ok()? * 360.0
                } else if let Some(v) = args.strip_suffix("rad") {
                    v.trim().parse::<f32>().ok()?.to_degrees()
                } else if args == "0" {
                    0.0
                } else {
                    return None;
                };
                let (sin, cos) = degrees.to_radians().sin_cos();
                Filter::Matrix([
                    0.213 + cos * 0.787 - sin * 0.213, 0.715 - cos * 0.715 - sin * 0.715, 0.072 - cos * 0.072 + sin * 0.928, 0.0, 0.0,
                    0.213 - cos * 0.213 + sin * 0.143, 0.715 + cos * 0.285 + sin * 0.140, 0.072 - cos * 0.072 - sin * 0.283, 0.0, 0.0,
                    0.213 - cos * 0.213 - sin * 0.787, 0.715 - cos * 0.715 + sin * 0.715, 0.072 + cos * 0.928 + sin * 0.072, 0.0, 0.0,
                    0.0, 0.0, 0.0, 1.0, 0.0,
                ])
            }
            "drop-shadow" => {
                // Lengths, then a color, in either order.
                let mut lengths = Vec::new();
                let mut color = [0, 0, 0, 255];
                for part in split_args(args) {
                    if let Some(v) = length(&part) {
                        lengths.push(v);
                    } else if let Some(c) = parse_color(&part) {
                        color = c;
                    } else {
                        return None;
                    }
                }
                if lengths.len() < 2 {
                    return None;
                }
                Filter::Shadow { x: lengths[0], y: lengths[1], blur: lengths.get(2).copied().unwrap_or(0.0), color }
            }
            _ => return None,
        };
        out.push(filter);
    }
    (!out.is_empty()).then_some(out)
}

/// What's between spaces, keeping what's in brackets together.
fn split_args(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth -= 1;
                current.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// A color in a filter, as far as one's needed: hex, `rgb()` and `rgba()`, and a few names.
fn parse_color(text: &str) -> Option<[u8; 4]> {
    let text = text.trim().to_ascii_lowercase();
    if let Some(color) = crate::Color::from_text(&text) {
        return Some([color.r, color.g, color.b, color.a]);
    }
    let named = match text.as_str() {
        "red" => Some([255, 0, 0, 255]),
        "green" => Some([0, 128, 0, 255]),
        "blue" => Some([0, 0, 255, 255]),
        "gray" | "grey" => Some([128, 128, 128, 255]),
        "yellow" => Some([255, 255, 0, 255]),
        "orange" => Some([255, 165, 0, 255]),
        "purple" => Some([128, 0, 128, 255]),
        "lime" => Some([0, 255, 0, 255]),
        "cyan" | "aqua" => Some([0, 255, 255, 255]),
        "magenta" | "fuchsia" => Some([255, 0, 255, 255]),
        "silver" => Some([192, 192, 192, 255]),
        "navy" => Some([0, 0, 128, 255]),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    let inner = text.strip_prefix("rgba(").or_else(|| text.strip_prefix("rgb("))?.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(|c: char| c == ',' || c == '/' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
    if parts.len() < 3 {
        return None;
    }
    let channel = |p: &str| p.parse::<f32>().ok().map(|v| v.round().clamp(0.0, 255.0) as u8);
    let alpha = parts.get(3).and_then(|p| p.parse::<f32>().ok()).unwrap_or(1.0);
    Some([channel(parts[0])?, channel(parts[1])?, channel(parts[2])?, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8])
}

#[cfg(test)]
mod tests;
