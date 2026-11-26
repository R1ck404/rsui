//! Keeps images on the GPU at the size they're shown, reading them on another thread.
//!
//! The pixels of an image only exist on the GPU, as there's no need for a second copy. If the GPU is
//! let go of, or the image is needed at another size, it's simply read again from its file.

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rsui_core::Text;
use rsui_image::{Animation, Picture, decode, decode_animation, fitted, info, is_animated};
use rsui_text::ColorBitmap;

/// How much GPU memory images can use before those not on screen start being let go of.
const BUDGET: usize = 64 * 1024 * 1024;
/// How much of a file to read to find an image's size. Nearly every file has it well before this.
const HEADER_BYTES: u64 = 256 * 1024;
/// How much to try first.
const FIRST_HEADER_BYTES: u64 = 16 * 1024;
/// An image up to this much bigger than it's shown is used as it is, rather than read again. Reading
/// again for every small change, while a window's being resized, would never catch up.
const SPARE: f32 = 1.5;
/// A second thread only exists while something's being read, and doesn't do much that needs a stack.
const WORKER_STACK: usize = 256 * 1024;

/// Pixels on their way to the GPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageUpload {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    /// Red, green, blue and alpha, with colors multiplied by alpha.
    pub pixels: Vec<u8>,
    /// For an image that changes, as a canvas does: where in it these pixels go, which makes it the first time it's given,
    /// at `(0, 0)` and its whole size. `None` for one that never changes.
    pub changes: Option<(u32, u32)>,
}

/// Images built into the program, by name.
fn built_in() -> &'static Mutex<HashMap<String, &'static [u8]>> {
    static BUILT_IN: OnceLock<Mutex<HashMap<String, &'static [u8]>>> = OnceLock::new();
    BUILT_IN.get_or_init(Mutex::default)
}

/// Lets an image built into the program be used by this name, which an image element's `src` then
/// gives instead of a file's path. Use it with `include_bytes!`.
pub fn register_image(name: &str, bytes: &'static [u8]) {
    if let Ok(mut images) = built_in().lock() {
        images.insert(name.to_owned(), bytes);
    }
}

fn built_in_bytes(source: &str) -> Option<&'static [u8]> {
    built_in().lock().ok()?.get(source).copied()
}

/// An encoded image kept in memory for the lifetime of this handle. The name can be used as an
/// image source, including in CSS. A decode already in flight keeps the bytes until it finishes.
pub struct MemoryImage(String);

type MemoryImages = Mutex<HashMap<String, Arc<Vec<u8>>>>;

fn memory_images() -> &'static MemoryImages {
    static IMAGES: OnceLock<MemoryImages> = OnceLock::new();
    IMAGES.get_or_init(Mutex::default)
}

impl MemoryImage {
    pub fn new(bytes: Vec<u8>) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let name = format!("rsui-image:{}", NEXT.fetch_add(1, Ordering::Relaxed));
        memory_images().lock().expect("image registry").insert(name.clone(), Arc::new(bytes));
        Self(name)
    }

    pub fn source(&self) -> &str {
        &self.0
    }
}

impl Drop for MemoryImage {
    fn drop(&mut self) {
        memory_images().lock().expect("image registry").remove(&self.0);
    }
}

/// Where a path that doesn't say where it's from is from.
static FILES_FROM: OnceLock<PathBuf> = OnceLock::new();

/// Says where a picture's path is from, if it doesn't say itself. It's where it's run from otherwise. Only the first of these counts.
pub fn set_files_from(folder: PathBuf) {
    let _ = FILES_FROM.set(folder);
}

fn file_of(source: &str) -> PathBuf {
    match FILES_FROM.get() {
        Some(folder) => folder.join(source),
        None => PathBuf::from(source),
    }
}

fn with_bytes<T>(source: &str, read: impl FnOnce(&[u8]) -> T) -> Option<T> {
    let source = source.split_once(DRAWN_AT).map_or(source, |(from, _)| from);
    if source.starts_with("rsui-image:") {
        let bytes = memory_images().lock().ok()?.get(source)?.clone();
        return Some(read(&bytes));
    }
    if let Some(bytes) = built_in_bytes(source) {
        return Some(read(bytes));
    }
    if let Some(bytes) = data_url(source) {
        return Some(read(&bytes));
    }
    Some(read(&rsui_text::read_file(&file_of(source)).ok()?))
}

/// What separates where a picture drawn at a size of its own is from, and that size, in what it's kept by.
const DRAWN_AT: char = '\u{1}';

/// What a `data:` URL holds, as `data:image/png;base64,...` or `data:image/svg+xml;utf8,<svg ...` writes it.
fn data_url(source: &str) -> Option<Vec<u8>> {
    let rest = source.strip_prefix("data:")?;
    let (kind, payload) = rest.split_once(',')?;
    if kind
        .split(';')
        .any(|part| part.trim().eq_ignore_ascii_case("base64"))
    {
        return base64(payload);
    }
    Some(percent_decoded(payload))
}

/// Bytes written as base64, leaving out anything else in it, as a browser does.
fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut bits = 0u32;
    let mut count = 0;
    for byte in percent_decoded(text) {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => continue,
        };
        bits = (bits << 6) | u32::from(value);
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Text with `%XX` turned back into the bytes those are.
fn percent_decoded(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    let hex = |byte: u8| char::from(byte).to_digit(16);
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[at + 1]), hex(bytes[at + 2])) {
                out.push((high * 16 + low) as u8);
                at += 3;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    out
}

/// What's known about an image from the start of it.
#[derive(Clone, Copy)]
struct Found {
    size: (u32, u32),
    /// Whether it's drawn rather than made of pixels, which makes it as sharp at any size.
    scalable: bool,
    /// Whether all it says is what shape it is, as an SVG with only a `viewBox` does.
    shape_only: bool,
}

/// How big the picture at `source` is, in its own pixels, from only the start of it, or `None` if it can't be read.
pub fn image_size(source: &str) -> Option<(u32, u32)> {
    read_size(source).map(|found| found.size)
}

/// What an image is, which usually only takes the start of its file.
fn read_size(source: &str) -> Option<Found> {
    let pixels = |bytes: &[u8]| {
        if crate::svg_image::is_svg(bytes) {
            return crate::svg_image::svg_size(bytes).map(|(size, own)| Found {
                size,
                scalable: true,
                shape_only: !own,
            });
        }
        let found = info(bytes).ok()?;
        Some(Found {
            size: (found.width, found.height),
            scalable: false,
            shape_only: false,
        })
    };
    if source.starts_with("rsui-image:") || source.starts_with("data:") || source.ends_with(".svg") || source.contains(".svg?") {
        return with_bytes(source, pixels)?;
    }
    match read_pixel_size(source) {
        Some(size) => Some(Found {
            size,
            scalable: false,
            shape_only: false,
        }),
        // Not made of pixels, or not of a kind that's known, which could be an SVG by any name.
        None => with_bytes(source, pixels)?,
    }
}

fn read_pixel_size(source: &str) -> Option<(u32, u32)> {
    if let Some(bytes) = built_in_bytes(source) {
        let found = info(bytes).ok()?;
        return Some((found.width, found.height));
    }
    if let Some(bytes) = rsui_text::read_by_reader(&file_of(source)) {
        let found = info(&bytes).ok()?;
        return Some((found.width, found.height));
    }
    let mut file = std::fs::File::open(file_of(source)).ok()?;
    let mut start = Vec::new();
    // Most files say in their first few bytes, so only some need the rest of this.
    for read_up_to in [FIRST_HEADER_BYTES, HEADER_BYTES] {
        let more = read_up_to - start.len() as u64;
        std::io::Read::by_ref(&mut file)
            .take(more)
            .read_to_end(&mut start)
            .ok()?;
        if let Ok(found) = info(&start) {
            return Some((found.width, found.height));
        }
    }
    let found = info(&std::fs::read(file_of(source)).ok()?).ok()?;
    Some((found.width, found.height))
}

/// Whether pixels of this size are enough for what's wanted, and not too many more than that.
fn good_for(size: (u32, u32), wanted: (u32, u32)) -> bool {
    let enough = size.0 >= wanted.0 && size.1 >= wanted.1;
    let spare = size.0 as f32 <= (wanted.0 as f32 * SPARE).ceil()
        && size.1 as f32 <= (wanted.1 as f32 * SPARE).ceil();
    enough && spare
}

struct Job {
    source: Text,
    target: (u32, u32),
    /// Whether to read all of an animated image, or only what it shows first.
    animations: bool,
}

/// What came of reading something.
pub(crate) enum Loaded {
    Picture(Picture),
    Animation(Animation),
    Failed,
}

struct Finished {
    source: Text,
    target: (u32, u32),
    read: Loaded,
}

/// Reads an image, as an animation if it's one and they're wanted.
pub(crate) fn read_image(source: &str, target: (u32, u32), animations: bool) -> Loaded {
    with_bytes(source, |bytes| {
        if crate::svg_image::is_svg(bytes) {
            return crate::svg_image::draw_svg(bytes, target).map_or(Loaded::Failed, Loaded::Picture);
        }
        if animations && is_animated(bytes) {
            if let Ok(animation) = decode_animation(bytes, target) {
                if animation.frames.len() > 1 {
                    return Loaded::Animation(animation);
                }
            }
        }
        decode(bytes, target).map_or(Loaded::Failed, Loaded::Picture)
    }).unwrap_or(Loaded::Failed)
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    finished: Vec<Finished>,
    worker_running: bool,
}

type Waker = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    changed: Condvar,
    waker: Mutex<Option<Waker>>,
}

/// What a thread does for as long as there's something to read, after which it's gone, and costs
/// nothing.
fn work(shared: &Shared) {
    loop {
        let job = {
            let Ok(mut queue) = shared.queue.lock() else {
                return;
            };
            match queue.jobs.pop_front() {
                Some(job) => job,
                None => {
                    queue.worker_running = false;
                    shared.changed.notify_all();
                    return;
                }
            }
        };
        let read = read_image(&job.source, job.target, job.animations);
        if let Ok(mut queue) = shared.queue.lock() {
            queue.finished.push(Finished {
                source: job.source,
                target: job.target,
                read,
            });
        }
        shared.changed.notify_all();
        let waker = shared.waker.lock().ok().and_then(|waker| waker.clone());
        if let Some(waker) = waker {
            waker();
        }
    }
}

/// One image on the GPU.
#[derive(Clone, Copy, PartialEq, Eq)]
struct OnGpu {
    id: u32,
    width: u32,
    height: u32,
    /// What it was read to fit.
    target: (u32, u32),
}

/// An animated image that's playing, all of whose frames are on the GPU.
struct Playing {
    /// Which image each frame is, and when it ends, in milliseconds from the start.
    frames: Vec<(u32, u64)>,
    started: Instant,
    /// How many times it plays, where 0 means forever.
    repeats: u32,
    /// Which frame was last drawn.
    shown: usize,
    bytes_per_frame: usize,
}

impl Playing {
    fn length(&self) -> u64 {
        self.frames.last().map_or(0, |frame| frame.1).max(1)
    }

    /// Which frame it's on at this time, and how long until it changes, if it's going to.
    fn at(&self, now: Instant) -> (usize, Option<u64>) {
        let elapsed = now.saturating_duration_since(self.started).as_millis() as u64;
        let length = self.length();
        if self.repeats > 0 && elapsed >= length.saturating_mul(u64::from(self.repeats)) {
            return (self.frames.len() - 1, None);
        }
        let into = elapsed % length;
        let index = self.frames.partition_point(|frame| frame.1 <= into);
        let index = index.min(self.frames.len() - 1);
        (index, Some(self.frames[index].1 - into))
    }
}

#[derive(Default)]
struct Entry {
    playing: Option<Playing>,
    /// Its size, if it's known yet. `Some(None)` means it can't be read.
    natural: Option<Option<(u32, u32)>>,
    on_gpu: Option<OnGpu>,
    /// What's being read for it now.
    reading: Option<(u32, u32)>,
    /// What it should be read to fit next, if that's changed while it was being read.
    wanted: Option<(u32, u32)>,
    failed: bool,
    last_used: u64,
    /// Whether it's drawn, rather than made of pixels, so that it's drawn again at each size it's shown at.
    scalable: bool,
    /// Whether it only says what shape it is, not how big.
    shape_only: bool,
    /// What it was last drawn at, and what that's kept as, which most of the time it's drawn at again.
    drawn: Option<((u32, u32), Text)>,
    /// For a canvas, the version of its pixels the GPU has.
    canvas_version: u64,
}

/// What to draw for an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shown {
    pub(crate) id: u32,
}

/// A glyph made of gradients, kept as a picture, as what it is at this size in this color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ColorGlyphKey {
    pub(crate) font: u16,
    pub(crate) glyph: u16,
    pub(crate) size_bits: u32,
    pub(crate) text_color: [u8; 4],
}

/// Where a kept glyph's picture is, and where it goes from the pen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ColorGlyph {
    pub(crate) id: u32,
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    last_used: u64,
}

/// Every image in use.
pub struct Images {
    entries: HashMap<Text, Entry>,
    color_glyphs: HashMap<ColorGlyphKey, ColorGlyph>,
    /// The shape drawn last at each place, by where it is and how big, so that a shape that changes, as one does that's
    /// moving, only keeps the picture it is now.
    shape_places: HashMap<[i32; 4], ColorGlyphKey>,
    shared: Arc<Shared>,
    next_id: u32,
    frame: u64,
    gpu_bytes: usize,
    /// How many images are playing. Looking through them all each time the pointer moves would mostly
    /// be for nothing, as there usually aren't any.
    playing: usize,
    uploads: Vec<ImageUpload>,
    dropped: Vec<u32>,
    /// Whether to read right away rather than on another thread, as there's a picture to restore.
    read_now: bool,
    /// Whether animated images play, or stay on what they show first.
    animations: bool,
    /// What time animated images are shown at.
    now: Option<Instant>,
    /// Which change to any canvas was last looked at, so that a window only looks through its canvases when one's changed.
    canvas_generation: std::cell::Cell<u64>,
}

impl Default for Images {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            color_glyphs: HashMap::new(),
            shape_places: HashMap::new(),
            shared: Arc::default(),
            next_id: 1,
            frame: 0,
            gpu_bytes: 0,
            playing: 0,
            uploads: Vec::new(),
            dropped: Vec::new(),
            read_now: false,
            animations: false,
            now: None,
            canvas_generation: std::cell::Cell::new(0),
        }
    }
}

impl Images {
    /// Whether animated images play, or stay on what they show first. They start off, as something
    /// looking at the result wants it to stay the same, and this applies to what's read from now on.
    pub fn set_animations(&mut self, on: bool) {
        self.animations = on;
    }

    /// Moves what's playing along to this time, and says whether that changes what's on screen.
    pub fn play_to(&mut self, now: Instant) -> bool {
        self.now = Some(now);
        if self.playing == 0 {
            return false;
        }
        let frame = self.frame;
        self.entries.values().any(|entry| {
            entry.last_used == frame
                && entry
                    .playing
                    .as_ref()
                    .is_some_and(|playing| playing.at(now).0 != playing.shown)
        })
    }

    /// How long until what's on screen and playing needs to change, if anything is.
    pub fn next_frame_in(&self) -> Option<Duration> {
        if self.playing == 0 {
            return None;
        }
        let now = Instant::now();
        let frame = self.frame;
        self.entries
            .values()
            .filter(|entry| entry.last_used == frame)
            .filter_map(|entry| entry.playing.as_ref()?.at(now).1)
            .min()
            .map(Duration::from_millis)
    }

    /// Asks to be called when an image has finished reading. `waker` runs on another thread.
    pub fn set_waker(&mut self, waker: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut slot) = self.shared.waker.lock() {
            *slot = Some(Arc::new(waker));
        }
    }

    /// An image's size, in its own pixels. `None` if it can't be read. This can read the start of a
    /// file, but only once, and not if the element it's for has a size of its own.
    pub fn natural_size(&mut self, source: &Text) -> Option<(u32, u32)> {
        if source.starts_with("canvas:") {
            return crate::canvas::parse_source(source).map(|(_, w, h)| (w, h)).filter(|(w, h)| *w > 0 && *h > 0);
        }
        if let Some(entry) = self.entries.get(source) {
            if let Some(natural) = entry.natural {
                return natural;
            }
        }
        let found = read_size(source);
        let entry = self.entries.entry(source.clone()).or_default();
        entry.natural = Some(found.map(|found| found.size));
        entry.scalable = found.is_some_and(|found| found.scalable);
        entry.shape_only = found.is_some_and(|found| found.shape_only);
        found.map(|found| found.size)
    }

    /// Whether an image only says what shape it is, not how big, as an SVG with only a `viewBox` does, whose size is then
    /// only how big it's shown. Its natural size is still that shape. This is only known once that's been asked.
    pub fn shape_only(&self, source: &Text) -> bool {
        self.entries
            .get(source)
            .is_some_and(|entry| entry.shape_only)
    }

    /// What to draw for a picture that's drawn, rather than made of pixels, as big as fits `target` keeping its shape,
    /// which is kept apart from it at each size.
    fn request_drawn(
        &mut self,
        source: &Text,
        natural: (u32, u32),
        target: (u32, u32),
    ) -> Option<Shown> {
        let (width, height) = (natural.0.max(1) as f32, natural.1.max(1) as f32);
        let scale = (target.0 as f32 / width).min(target.1 as f32 / height);
        let size = (
            ((width * scale).round() as u32).clamp(1, 4096),
            ((height * scale).round() as u32).clamp(1, 4096),
        );
        let entry = self.entries.get_mut(source)?;
        entry.last_used = self.frame;
        let key = match &entry.drawn {
            Some((drawn, key)) if *drawn == size => key.clone(),
            _ => {
                let key = Text::from(format!("{source}{DRAWN_AT}{}x{}", size.0, size.1));
                entry.drawn = Some((size, key.clone()));
                key
            }
        };
        let sized = self.entries.entry(key.clone()).or_default();
        sized.natural = Some(Some(size));
        self.request(&key, size)
    }

    /// Whether something's finished reading, and is ready to show.
    pub fn arrived(&self) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        self.shared
            .queue
            .lock()
            .is_ok_and(|queue| !queue.finished.is_empty())
    }

    /// Waits for everything being read to finish, up to `timeout`, and says whether it did.
    pub fn wait(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let Ok(mut queue) = self.shared.queue.lock() else {
            return false;
        };
        while queue.worker_running {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            let Ok((next, _)) = self.shared.changed.wait_timeout(queue, left) else {
                return false;
            };
            queue = next;
        }
        true
    }

    /// Takes in what's finished reading.
    pub(crate) fn begin_frame(&mut self) {
        self.frame += 1;
        let finished = match self.shared.queue.lock() {
            Ok(mut queue) if !queue.finished.is_empty() => std::mem::take(&mut queue.finished),
            _ => return,
        };
        for done in finished {
            self.accept(done);
        }
    }

    fn accept(&mut self, done: Finished) {
        let Some(entry) = self.entries.get_mut(&done.source) else {
            return;
        };
        entry.reading = None;
        if matches!(done.read, Loaded::Failed) {
            entry.failed = true;
            return;
        }
        if let Some(old) = entry.on_gpu.take() {
            self.gpu_bytes -= old.width as usize * old.height as usize * 4;
            self.dropped.push(old.id);
        }
        if let Some(old) = entry.playing.take() {
            self.playing -= 1;
            // The first frame was just taken care of, as it's what's on the GPU.
            self.gpu_bytes -= old.bytes_per_frame * (old.frames.len() - 1);
            self.dropped
                .extend(old.frames.iter().skip(1).map(|frame| frame.0));
        }
        let mut new_id = || {
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1).max(1);
            id
        };
        match done.read {
            Loaded::Picture(picture) => {
                let id = new_id();
                entry.on_gpu = Some(OnGpu {
                    id,
                    width: picture.width,
                    height: picture.height,
                    target: done.target,
                });
                self.gpu_bytes += picture.pixels.len();
                self.uploads.push(ImageUpload {
                    id,
                    width: picture.width,
                    height: picture.height,
                    pixels: picture.pixels,
                    changes: None,
                });
            }
            Loaded::Animation(animation) => {
                let (width, height) = (animation.width, animation.height);
                let bytes_per_frame = width as usize * height as usize * 4;
                let mut frames = Vec::with_capacity(animation.frames.len());
                let mut end = 0u64;
                for frame in animation.frames {
                    let id = new_id();
                    end += u64::from(frame.delay);
                    frames.push((id, end));
                    self.gpu_bytes += bytes_per_frame;
                    self.uploads.push(ImageUpload {
                        id,
                        width,
                        height,
                        pixels: frame.pixels,
                        changes: None,
                    });
                }
                entry.on_gpu = Some(OnGpu {
                    id: frames[0].0,
                    width,
                    height,
                    target: done.target,
                });
                self.playing += 1;
                entry.playing = Some(Playing {
                    frames,
                    started: Instant::now(),
                    repeats: animation.repeats,
                    shown: 0,
                    bytes_per_frame,
                });
            }
            Loaded::Failed => {}
        }
        if let Some(next) = entry.wanted.take() {
            if next != done.target {
                self.start_reading(done.source, next);
            }
        }
    }

    fn start_reading(&mut self, source: Text, target: (u32, u32)) {
        if let Some(entry) = self.entries.get_mut(&source) {
            entry.reading = Some(target);
        }
        if self.read_now {
            let read = read_image(&source, target, self.animations);
            self.accept(Finished {
                source,
                target,
                read,
            });
            return;
        }
        let Ok(mut queue) = self.shared.queue.lock() else {
            return;
        };
        queue.jobs.push_back(Job {
            source,
            target,
            animations: self.animations,
        });
        if !queue.worker_running {
            queue.worker_running = true;
            let shared = self.shared.clone();
            let started = std::thread::Builder::new()
                .name("rsui images".into())
                .stack_size(WORKER_STACK)
                .spawn(move || work(&shared));
            if started.is_err() {
                queue.worker_running = false;
            }
        }
    }

    /// What to draw for an image that wants to be read to fit `target`, in physical pixels. It carries on
    /// showing what it has, if anything, until something better's been read.
    pub(crate) fn request(&mut self, source: &Text, target: (u32, u32)) -> Option<Shown> {
        if source.starts_with("canvas:") {
            return self.request_canvas(source);
        }
        let natural = self.natural_size(source)?;
        if self.entries.get(source).is_some_and(|entry| entry.scalable) {
            return self.request_drawn(source, natural, target);
        }
        let wanted = fitted(natural, target);
        let frame = self.frame;
        let entry = self.entries.get_mut(source)?;
        entry.last_used = frame;
        let good = entry
            .on_gpu
            .is_some_and(|gpu| good_for((gpu.width, gpu.height), wanted));
        if !good && !entry.failed {
            match entry.reading {
                None => {
                    self.start_reading(source.clone(), target);
                }
                Some(reading) => {
                    let coming = fitted(natural, reading);
                    if !good_for(coming, wanted) {
                        // Still growing while the last size is being read, so this size would be too small
                        // by the time it had been read too, and it would be read over and over. This much
                        // extra lasts a while, and it's within `SPARE`.
                        let growing = coming.0 < wanted.0 || coming.1 < wanted.1;
                        entry.wanted = Some(if growing {
                            (
                                target.0.saturating_mul(5) / 4,
                                target.1.saturating_mul(5) / 4,
                            )
                        } else {
                            target
                        });
                    }
                }
            }
        }
        let now = self.now;
        let entry = self.entries.get_mut(source)?;
        if let Some(playing) = &mut entry.playing {
            let index = playing.at(now.unwrap_or(playing.started)).0;
            playing.shown = index;
            return Some(Shown {
                id: playing.frames[index].0,
            });
        }
        entry.on_gpu.map(|gpu| Shown { id: gpu.id })
    }

    /// What to draw for a canvas: its pixels as the GPU has them, with what's changed since they were last given it on its way.
    #[cold]
    #[inline(never)]
    fn request_canvas(&mut self, source: &Text) -> Option<Shown> {
        let (canvas, width, height) = crate::canvas::parse_source(source)?;
        if width == 0 || height == 0 {
            return None;
        }
        let frame = self.frame;
        if !self.entries.contains_key(source) {
            // A canvas at its last size isn't shown again once it's been made another size.
            let prefix = format!("canvas:{canvas}:");
            let old: Vec<Text> = self.entries.keys().filter(|key| key.starts_with(&prefix)).cloned().collect();
            for key in old {
                self.let_go(&key);
                self.entries.remove(&key);
            }
        }
        let next_id = self.next_id;
        let entry = self.entries.entry(source.clone()).or_default();
        entry.last_used = frame;
        let whole = entry.on_gpu.is_none();
        let Some((area, pixels, version)) = crate::canvas::take_upload(canvas, whole, entry.canvas_version) else {
            return entry.on_gpu.map(|gpu| Shown { id: gpu.id });
        };
        entry.canvas_version = version;
        if whole {
            if area.width != width || area.height != height {
                return None;
            }
            self.next_id = self.next_id.wrapping_add(1).max(1);
            entry.on_gpu = Some(OnGpu { id: next_id, width, height, target: (width, height) });
            self.gpu_bytes += pixels.len();
            self.uploads.push(ImageUpload { id: next_id, width, height, pixels, changes: Some((0, 0)) });
            return Some(Shown { id: next_id });
        }
        let id = entry.on_gpu?.id;
        if area.width > 0 && area.height > 0 {
            self.uploads.push(ImageUpload { id, width: area.width, height: area.height, pixels, changes: Some((area.x, area.y)) });
        }
        Some(Shown { id })
    }

    /// Whether a canvas that's shown has changed since it was last given to the GPU, for a window to know to draw again.
    pub fn canvases_changed(&self) -> bool {
        let generation = crate::canvas::generation();
        if generation == self.canvas_generation.get() {
            return false;
        }
        self.canvas_generation.set(generation);
        self.entries.iter().any(|(source, entry)| {
            source.starts_with("canvas:")
                && crate::canvas::parse_source(source)
                    .and_then(|(id, _, _)| crate::canvas::version(id))
                    .is_some_and(|version| version != entry.canvas_version)
        })
    }

    /// The picture of a glyph made of gradients, drawn now if it hasn't been. `None` if it can't be.
    pub(crate) fn color_glyph(
        &mut self,
        key: ColorGlyphKey,
        draw: impl FnOnce() -> Option<ColorBitmap>,
    ) -> Option<ColorGlyph> {
        let frame = self.frame;
        if let Some(kept) = self.color_glyphs.get_mut(&key) {
            kept.last_used = frame;
            return Some(*kept);
        }
        let bitmap = draw()?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.gpu_bytes += bitmap.pixels.len();
        let kept = ColorGlyph {
            id,
            left: bitmap.left,
            top: bitmap.top,
            width: bitmap.width,
            height: bitmap.height,
            last_used: frame,
        };
        self.uploads.push(ImageUpload {
            id,
            width: bitmap.width,
            height: bitmap.height,
            pixels: bitmap.pixels,
            changes: None,
        });
        self.color_glyphs.insert(key, kept);
        Some(kept)
    }

    /// Like [`color_glyph`](Self::color_glyph), for a shape drawn at `place`: a picture of it that was there before, and
    /// isn't anywhere else now, is let go of as soon as there's a new one.
    pub(crate) fn shape_glyph(
        &mut self,
        key: ColorGlyphKey,
        place: [i32; 4],
        draw: impl FnOnce() -> Option<ColorBitmap>,
    ) -> Option<ColorGlyph> {
        let made = self.color_glyph(key, draw);
        if let Some(before) = self.shape_places.insert(place, key) {
            if before != key {
                let frame = self.frame;
                let unseen = self
                    .color_glyphs
                    .get(&before)
                    .is_some_and(|kept| kept.last_used < frame);
                if unseen {
                    if let Some(kept) = self.color_glyphs.remove(&before) {
                        self.gpu_bytes -= kept.width as usize * kept.height as usize * 4;
                        self.dropped.push(kept.id);
                    }
                }
            }
        }
        // What's kept for places stays as small as what's on screen, give or take.
        if self.shape_places.len() > 4 * self.color_glyphs.len() + 64 {
            let glyphs = &self.color_glyphs;
            self.shape_places.retain(|_, key| glyphs.contains_key(key));
        }
        made
    }

    /// Lets go of glyphs that weren't on screen this frame, oldest first, until there's room.
    fn let_go_of_color_glyphs(&mut self, until: usize) {
        if self.gpu_bytes <= until {
            return;
        }
        let frame = self.frame;
        let mut oldest: Vec<(u64, ColorGlyphKey)> = self
            .color_glyphs
            .iter()
            .filter(|(_, kept)| kept.last_used < frame)
            .map(|(key, kept)| (kept.last_used, *key))
            .collect();
        oldest.sort_by_key(|(used, _)| *used);
        for (_, key) in oldest {
            if self.gpu_bytes <= until {
                break;
            }
            if let Some(kept) = self.color_glyphs.remove(&key) {
                self.gpu_bytes -= kept.width as usize * kept.height as usize * 4;
                self.dropped.push(kept.id);
            }
        }
    }

    /// Hands over what the GPU needs, and lets go of what's over budget and not on screen.
    pub(crate) fn end_frame(
        &mut self,
        uploads: &mut Vec<ImageUpload>,
        dropped: &mut Vec<u32>,
    ) -> usize {
        self.read_now = false;
        self.let_go_of_color_glyphs(BUDGET);
        if self.gpu_bytes > BUDGET {
            let mut oldest: Vec<(u64, Text)> = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.on_gpu.is_some() && entry.last_used < self.frame)
                .map(|(source, entry)| (entry.last_used, source.clone()))
                .collect();
            oldest.sort_by_key(|(used, _)| *used);
            for (_, source) in oldest {
                if self.gpu_bytes <= BUDGET {
                    break;
                }
                self.let_go(&source);
            }
        }
        // Something that's let go of before it reached the GPU never needs to get there.
        if !self.dropped.is_empty() {
            let dropped_now = &self.dropped;
            self.uploads
                .retain(|upload| !dropped_now.contains(&upload.id));
            // Earlier paints may have exported uploads without a draw yet.
            uploads.retain(|upload| !dropped_now.contains(&upload.id));
        }
        let appended_at = uploads.len();
        uploads.append(&mut self.uploads);
        dropped.append(&mut self.dropped);
        appended_at
    }

    /// Deliver idle retirements without starting a frame or consuming arrivals.
    pub(crate) fn drain_dropped(&mut self, uploads: &mut Vec<ImageUpload>, dropped: &mut Vec<u32>) {
        if self.dropped.is_empty() {
            return;
        }
        self.uploads
            .retain(|upload| !self.dropped.contains(&upload.id));
        uploads.retain(|upload| !self.dropped.contains(&upload.id));
        dropped.append(&mut self.dropped);
    }

    fn let_go(&mut self, source: &Text) {
        if let Some(entry) = self.entries.get_mut(source) {
            if let Some(gpu) = entry.on_gpu.take() {
                self.gpu_bytes -= gpu.width as usize * gpu.height as usize * 4;
                self.dropped.push(gpu.id);
            }
            if let Some(playing) = entry.playing.take() {
                self.playing -= 1;
                self.gpu_bytes -= playing.bytes_per_frame * (playing.frames.len() - 1);
                self.dropped
                    .extend(playing.frames.iter().skip(1).map(|frame| frame.0));
            }
        }
    }

    /// Lets go of everything not on screen right now, for when things have gone quiet.
    pub fn let_go_of_unused(&mut self) {
        self.let_go_of_color_glyphs(0);
        let unused: Vec<Text> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.on_gpu.is_some() && entry.last_used < self.frame)
            .map(|(source, _)| source.clone())
            .collect();
        for source in &unused {
            self.let_go(source);
        }
        // What was drawn at a size of its own is kept only while it's shown, as there's one for each size.
        self.entries.retain(|source, entry| {
            !((source.contains(DRAWN_AT) || source.starts_with("rsui-image:")) && entry.on_gpu.is_none() && entry.reading.is_none())
        });
    }

    /// The GPU's been let go of, and everything on it is gone. What's on screen is read again right
    /// away, the next time it's drawn, so that the picture comes back whole.
    pub fn gpu_replaced(&mut self) {
        self.color_glyphs.clear();
        // With nothing to put back, as when a window first opens, nothing has to hold up the first frame.
        // Several invalidation boundaries can precede the next paint. Keep the
        // immediate recovery obligation until end_frame has replayed the images.
        self.read_now |= self.entries.values().any(|entry| entry.on_gpu.is_some());
        for entry in self.entries.values_mut() {
            entry.on_gpu = None;
            entry.playing = None;
            entry.failed = false;
        }
        self.gpu_bytes = 0;
        self.playing = 0;
        self.uploads.clear();
        self.dropped.clear();
    }

    /// How much GPU memory images are using.
    pub fn gpu_bytes(&self) -> usize {
        self.gpu_bytes
    }
}

#[test]
fn a_memory_image_can_be_read_only_while_its_owner_keeps_it() {
    let image = MemoryImage::new(br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="5"><rect width="4" height="5" fill="red"/></svg>"#.to_vec());
    let source = image.source().to_owned();
    assert_eq!(image_size(&source), Some((4, 5)));
    assert!(matches!(read_image(&source, (4, 5), false), Loaded::Picture(_)));
    drop(image);
    assert_eq!(image_size(&source), None);
    assert!(matches!(read_image(&source, (4, 5), false), Loaded::Failed));
}

#[cfg(test)]
mod export_boundary_test {
    use super::*;
    #[test]
    fn exported_tail_starts_after_retired_pending_uploads() {
        let mut images = Images::default();
        images.dropped.push(7);
        let upload = |id| ImageUpload {
            id,
            width: 1,
            height: 1,
            pixels: vec![255; 4],
            changes: None,
        };
        let mut pending = vec![upload(7), upload(8)];
        images.uploads.push(upload(9));
        let mut dropped = Vec::new();
        let start = images.end_frame(&mut pending, &mut dropped);
        assert_eq!(start, 1);
        assert_eq!(
            pending[start..].iter().map(|u| u.id).collect::<Vec<_>>(),
            [9]
        );
    }
}

#[cfg(test)]
mod idle_retirement_tests {
    use super::*;

    #[test]
    fn idle_and_frame_export_cancel_retired_uploads_without_consuming_arrivals() {
        for exported_retirement in [false, true] {
            let mut images = Images {
                frame: 2,
                ..Default::default()
            };
            for (name, last_used) in [("gone", 1), ("visible", 2), ("pending", 2)] {
                images.entries.insert(
                    Text::from(name),
                    Entry {
                        last_used,
                        reading: Some((1, 1)),
                        ..Default::default()
                    },
                );
            }
            let arrive = |images: &mut Images, name: &str| {
                images.accept(Finished {
                    source: Text::from(name),
                    target: (1, 1),
                    read: Loaded::Picture(Picture {
                        width: 1,
                        height: 1,
                        pixels: vec![255; 4],
                    }),
                })
            };
            arrive(&mut images, "gone");
            arrive(&mut images, "visible");
            let mut uploads = Vec::new();
            let mut dropped = Vec::new();
            images.end_frame(&mut uploads, &mut dropped);
            assert_eq!(uploads.len(), 2);
            let visible = images.entries[&Text::from("visible")]
                .on_gpu
                .as_ref()
                .unwrap()
                .id;
            images.let_go_of_unused();
            if exported_retirement {
                images.end_frame(&mut uploads, &mut dropped);
            }
            images.drain_dropped(&mut uploads, &mut dropped);
            assert_eq!(
                uploads.len(),
                1,
                "undrawn retired upload must not resurrect"
            );
            assert_eq!(uploads[0].id, visible);
            assert_eq!(dropped.len(), 1);
            assert!(images.entries[&Text::from("pending")].reading.is_some());
            arrive(&mut images, "pending");
            images.end_frame(&mut uploads, &mut dropped);
            assert_eq!(uploads.len(), 2);
            assert_eq!(images.gpu_bytes(), 8);
            arrive(&mut images, "unknown");
            assert_eq!(images.gpu_bytes(), 8);
        }
    }
}
