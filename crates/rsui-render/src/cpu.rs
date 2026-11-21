//! Puts scenes in memory with the CPU, on systems where rsui has no GPU renderer of its own yet, and for tests everywhere
//! that need the same pixels on every machine.
//!
//! It draws exactly what the Direct3D renderer's shaders draw (`quad.hlsl`), pixel by pixel, with the same arithmetic:
//! each quad covers the pixels whose middles are inside it, its color there is worked out as the pixel shader works it
//! out, and it goes over what's behind with the same blending, one byte to a channel. What's faded as a whole is drawn on
//! a layer of its own and put down after, as there. So a test written against one gives the same answer with the other,
//! to within a little rounding where the GPU's arithmetic differs from the CPU's.
//!
//! Most of a frame is rectangles of one color and glyphs that line up with whole pixels, and those don't go through the
//! whole of the shader's arithmetic. Everything is drawn every frame that's different from the last, and nothing at all
//! for one that's the same.
//!
//! On Linux and Android, a renderer made for a window draws with Vulkan instead (`vulkan.rs`), through this same
//! interface, and only what tells one frame from the next is kept here.

use std::collections::HashMap;
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::ffi::c_void;
use std::fmt;

use rsui_text::Upload;

use crate::area::{NO_ROUND_CLIPS, RoundSlots, intersect, is_empty, layer_quad, round_clips_of};
use crate::images::ImageUpload;
use crate::paint::Color;
use crate::scene::{Batch, Filter, Layer, PixelRect, Quad, RoundClip, Scene};
use crate::transform::{IDENTITY, Matrix, apply_matrix, invert_matrix};

/// What a shadow inside something is marked with, in place of a glyph's width in the atlas, as the shaders have it.
const INSET_MARK: u16 = 65534;

/// Why a frame couldn't be drawn.
#[derive(Debug, Clone)]
pub enum RenderError {
    /// What was drawn on is gone, which can't happen to memory, but is here for code written for a GPU's being reset.
    DeviceLost,
    /// Something else, as the system said it.
    Graphics(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceLost => f.write_str("what was drawn on is gone"),
            Self::Graphics(error) => write!(f, "drawing failed: {error}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// A limit on how much of this process can be in RAM at once. Only Windows has such a thing, so elsewhere this does
/// nothing: see the Direct3D renderer's.
pub struct WorkingSetLimit {
    _private: (),
}

impl WorkingSetLimit {
    /// Nothing, here.
    pub fn with_room(_room: usize, _at_least: usize) -> Self {
        Self { _private: () }
    }
}

/// Gives back as much of this process's memory as the system will take. The memory this renderer uses is the process's
/// own, which the allocator gives back itself, so there's nothing more to do here.
pub fn give_back_memory() {}

/// Starts what draws to a window ahead of time. Memory needs no starting, so this does nothing.
pub fn start_gpu_early() {}

/// An image, as its pixels: red, green, blue and alpha, with colors multiplied by alpha.
struct Image {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

/// What the last frame drew, to tell whether the next is the same.
#[derive(Default)]
struct Last {
    quads: Vec<Quad>,
    batches: Vec<Batch>,
    transforms: Vec<Matrix>,
    filters: Vec<Filter>,
    round_clips: Vec<RoundClip>,
    layers: Vec<Layer>,
    clear: Option<Color>,
    generation: u32,
    text_look: [f32; 2],
}

/// Draws scenes to memory.
pub struct Renderer {
    size: (u32, u32),
    /// What's been drawn: red, green, blue and alpha for each pixel, in rows from the top, colors multiplied by alpha.
    pixels: Vec<[u8; 4]>,
    /// How much of each pixel of the atlas a glyph covers, `atlas_size` on each side.
    atlas: Vec<u8>,
    atlas_size: u32,
    images: HashMap<u32, Image>,
    /// What's faded as a whole is drawn on first, and how big that is.
    layer: Vec<[u8; 4]>,
    layer_size: (u32, u32),
    text_look: [f32; 2],
    last: Last,
    /// Whether `last` says what's in `pixels`.
    known: bool,
    nothing_to_show: bool,
    always_redraw: bool,
    see_through: bool,
    #[cfg(feature = "retained-frame")]
    frame_token: u64,
    /// What draws instead, on a GPU, for a window.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    vulkan: Option<crate::vulkan::Vulkan>,
}

impl Renderer {
    /// Draws to memory. Read the result with [`read_pixels`](Self::read_pixels). This works on any machine, and gives
    /// the same pixels on each, which is what tests need.
    pub fn offscreen(size: (u32, u32)) -> Result<Self, RenderError> {
        let size = (size.0.max(1), size.1.max(1));
        let mut renderer = Self::empty(size);
        renderer.pixels = vec![[0; 4]; size.0 as usize * size.1 as usize];
        Ok(renderer)
    }

    /// A renderer with nothing drawn on yet.
    fn empty(size: (u32, u32)) -> Self {
        Self {
            size,
            pixels: Vec::new(),
            atlas: Vec::new(),
            atlas_size: 0,
            images: HashMap::new(),
            layer: Vec::new(),
            layer_size: (0, 0),
            text_look: [0.0, 0.0],
            last: Last::default(),
            known: false,
            nothing_to_show: false,
            always_redraw: false,
            see_through: false,
            #[cfg(feature = "retained-frame")]
            frame_token: 0,
            #[cfg(any(target_os = "linux", target_os = "android"))]
            vulkan: None,
        }
    }

    /// Draws to a Wayland surface with Vulkan. This fails when there's no Vulkan, no GPU that can draw on the surface, or
    /// only a GPU that's really the CPU, which is slower than [`offscreen`](Self::offscreen) drawing by itself; then the
    /// caller draws with that instead. `RSUI_VULKAN=1` takes a software GPU anyway, and `RSUI_VULKAN=0` never tries.
    ///
    /// The caller asks for the surface's frame callbacks, as Wayland has it, and what's drawn can't be read back with
    /// [`read_pixels`](Self::read_pixels).
    ///
    /// # Safety
    ///
    /// `display` is a libwayland `wl_display` and `surface` a `wl_surface` of it, and both outlive the renderer.
    #[cfg(target_os = "linux")]
    pub unsafe fn for_wayland(display: *mut c_void, surface: *mut c_void, size: (u32, u32)) -> Result<Self, RenderError> {
        Self::for_window(crate::vulkan::Window::Wayland { display, surface }, size)
    }

    /// Draws to an Android window (`ANativeWindow`) with Vulkan, as [`for_wayland`](Self::for_wayland) does.
    ///
    /// # Safety
    ///
    /// `window` is an `ANativeWindow` that outlives the renderer.
    #[cfg(target_os = "android")]
    pub unsafe fn for_android(window: *mut c_void, size: (u32, u32)) -> Result<Self, RenderError> {
        Self::for_window(crate::vulkan::Window::Android { window }, size)
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn for_window(window: crate::vulkan::Window, size: (u32, u32)) -> Result<Self, RenderError> {
        let wanted = std::env::var("RSUI_VULKAN").unwrap_or_default();
        if wanted == "0" {
            return Err(RenderError::Graphics("RSUI_VULKAN=0 says not to use Vulkan".to_owned()));
        }
        let size = (size.0.max(1), size.1.max(1));
        let vulkan = crate::vulkan::Vulkan::new(Some(window), size, wanted == "1")?;
        Ok(Self { vulkan: Some(vulkan), ..Self::empty(size) })
    }

    /// Draws to memory with Vulkan, on whatever GPU there is, software ones included, for checking what Vulkan draws
    /// against what [`offscreen`](Self::offscreen) does.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[doc(hidden)]
    pub fn offscreen_vulkan(size: (u32, u32)) -> Result<Self, RenderError> {
        let size = (size.0.max(1), size.1.max(1));
        let vulkan = crate::vulkan::Vulkan::new(None, size, true)?;
        Ok(Self { vulkan: Some(vulkan), ..Self::empty(size) })
    }

    /// How many errors Vulkan's validation layers (`RSUI_VULKAN_VALIDATE=1`) have reported in this process.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[doc(hidden)]
    pub fn vulkan_validation_errors() -> u32 {
        crate::vulkan::validation_errors()
    }

    /// Whether this draws on a window itself, with the GPU, rather than into memory for the caller to show.
    pub fn draws_to_window(&self) -> bool {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if let Some(vulkan) = &self.vulkan {
            return vulkan.draws_to_window();
        }
        false
    }

    /// The same as [`offscreen`](Self::offscreen): there's no other hardware to draw on here.
    #[doc(hidden)]
    pub fn offscreen_on_hardware(size: (u32, u32)) -> Result<Self, RenderError> {
        Self::offscreen(size)
    }

    /// The same as [`offscreen`](Self::offscreen). A GPU's window swaps between two buffers, which drawing only part of
    /// a frame has to get right; this draws the whole of each frame that changes into one.
    pub fn offscreen_flipping(size: (u32, u32)) -> Result<Self, RenderError> {
        Self::offscreen(size)
    }

    pub fn is_see_through(&self) -> bool {
        self.see_through
    }

    /// Memory has alpha anyway, so this only remembers it. A window drawn with Vulkan is seen through from its next frame.
    pub fn set_see_through(&mut self, on: bool) -> Result<(), RenderError> {
        self.see_through = on;
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if let Some(vulkan) = &mut self.vulkan {
            vulkan.set_see_through(on);
            self.known = false;
        }
        Ok(())
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Draws a scene. What's in the scene is used up, so paint another one for the next frame.
    pub fn render(&mut self, scene: &mut Scene, clear: Color) -> Result<(), RenderError> {
        self.draw(scene, clear)?;
        self.present()
    }

    /// Shows what's been drawn, which in memory is nothing to do.
    pub fn present(&mut self) -> Result<(), RenderError> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if let Some(vulkan) = &mut self.vulkan {
            let shown = vulkan.present();
            if shown.is_err() {
                self.known = false;
            }
            // A window that had nothing to draw on showed nothing, which the caller mustn't wait to see.
            if !shown? && vulkan.draws_to_window() {
                self.nothing_to_show = true;
            }
        }
        Ok(())
    }

    /// Whether the last frame was the same as the one before, so nothing was drawn.
    pub fn showed_nothing(&self) -> bool {
        self.nothing_to_show
    }

    pub fn was_occluded(&self) -> bool {
        false
    }

    /// Draws every frame in full, even if nothing changed, which is for measuring.
    pub fn set_always_redraw(&mut self, always: bool) {
        self.always_redraw = always;
    }

    /// Nothing waits for a screen here.
    pub fn set_sync_interval(&mut self, _interval: u32) {}

    /// Token for the most recent ordinary draw, as the Direct3D renderer has it.
    #[cfg(feature = "retained-frame")]
    #[doc(hidden)]
    pub fn frame_token(&self) -> u64 {
        self.frame_token
    }

    /// Accept an unchanged frame without drawing it.
    #[cfg(feature = "retained-frame")]
    #[doc(hidden)]
    pub fn reuse_frame(&mut self, token: u64) -> bool {
        if token == 0 || token != self.frame_token || !self.known || self.always_redraw {
            return false;
        }
        self.nothing_to_show = true;
        true
    }

    /// Draws a scene without showing it. Normally [`render`](Self::render) does this.
    pub fn draw(&mut self, scene: &mut Scene, clear: Color) -> Result<(), RenderError> {
        #[cfg(feature = "retained-frame")]
        {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            self.frame_token = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if self.vulkan.is_some() {
            return self.draw_with_vulkan(scene, clear, false);
        }
        self.prepare_atlas(scene.atlas_size);
        self.upload_glyphs(&scene.uploads);
        scene.uploads.clear();
        let images_changed = self.upload_images(scene);
        let same = self.same_as_last(scene, clear, images_changed);
        self.text_look = scene.text_look;
        self.nothing_to_show = same;
        if !same {
            self.draw_all(scene, clear);
        }
        self.keep_for_next_frame(scene, clear);
        Ok(())
    }

    /// [`draw`](Self::draw), with Vulkan, on the window or, if `to_memory` says so, into memory.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn draw_with_vulkan(&mut self, scene: &mut Scene, clear: Color, to_memory: bool) -> Result<(), RenderError> {
        let vulkan = self.vulkan.as_mut().expect("drawing with Vulkan");
        let images_changed = vulkan.take_uploads(scene, &self.last.batches);
        if vulkan.needs_whole_frame() || to_memory {
            self.known = false;
        }
        let same = self.same_as_last(scene, clear, images_changed);
        self.text_look = scene.text_look;
        self.nothing_to_show = same;
        if !same {
            let vulkan = self.vulkan.as_mut().expect("drawing with Vulkan");
            if let Err(error) = vulkan.draw(scene, clear, self.text_look, to_memory) {
                self.known = false;
                return Err(error);
            }
        }
        self.keep_for_next_frame(scene, clear);
        // What's in memory isn't what's on the window, so the window's next frame is drawn whatever it is.
        if to_memory && self.draws_to_window() {
            self.known = false;
        }
        Ok(())
    }

    /// Whether a scene is the same as the last frame, so there's nothing to draw.
    fn same_as_last(&self, scene: &Scene, clear: Color, images_changed: bool) -> bool {
        self.known
            && !self.always_redraw
            && !images_changed
            && self.last.clear == Some(clear)
            && self.last.generation == scene.atlas_generation
            && self.last.text_look == scene.text_look
            && self.last.quads == scene.quads
            && self.last.batches == scene.batches
            && self.last.transforms == scene.transforms
            && self.last.filters == scene.filters
            && self.last.round_clips == scene.round_clips
            && self.last.layers == scene.layers
    }

    fn keep_for_next_frame(&mut self, scene: &mut Scene, clear: Color) {
        std::mem::swap(&mut self.last.quads, &mut scene.quads);
        std::mem::swap(&mut self.last.batches, &mut scene.batches);
        std::mem::swap(&mut self.last.transforms, &mut scene.transforms);
        std::mem::swap(&mut self.last.filters, &mut scene.filters);
        std::mem::swap(&mut self.last.round_clips, &mut scene.round_clips);
        std::mem::swap(&mut self.last.layers, &mut scene.layers);
        self.last.clear = Some(clear);
        self.last.generation = scene.atlas_generation;
        self.last.text_look = scene.text_look;
        self.known = true;
    }

    fn draw_all(&mut self, scene: &Scene, clear: Color) {
        let fill = clear.premultiplied();
        self.pixels.fill(fill);
        let whole = PixelRect { left: 0, top: 0, right: self.size.0 as i32, bottom: self.size.1 as i32 };
        let mut next_layer = 0;
        let mut in_layer: Option<usize> = None;
        for (index, batch) in scene.batches.iter().enumerate() {
            let index = index as u32;
            if let Some(at) = in_layer.filter(|&at| scene.layers[at].end <= index) {
                in_layer = None;
                self.put_down_layer(&scene.layers[at]);
            }
            if in_layer.is_none() && scene.layers.get(next_layer).is_some_and(|layer| layer.first == index) {
                self.start_layer(&scene.layers[next_layer]);
                in_layer = Some(next_layer);
                next_layer += 1;
            }
            let clip = intersect(batch.clip, whole);
            if is_empty(clip) {
                continue;
            }
            let matrix = batch.transform.checked_sub(1).and_then(|at| scene.transforms.get(usize::from(at))).copied();
            let filter = batch.filter.checked_sub(1).and_then(|at| scene.filters.get(usize::from(at))).copied();
            let rounds = round_clips_of(batch, &scene.round_clips);
            let look = Look::new(matrix, filter, rounds, self.atlas_size, self.text_look);
            let quads = scene.quads.get(batch.first as usize..(batch.first + batch.count) as usize).unwrap_or_default();
            let origin = in_layer.map_or((0, 0), |at| layer_origin(&scene.layers[at], self.size));
            let image = if batch.image != 0 {
                match self.images.get(&batch.image) {
                    Some(image) => Some(image),
                    None => continue,
                }
            } else {
                None
            };
            let (target, width) = if in_layer.is_some() {
                (&mut self.layer, self.layer_size.0)
            } else {
                (&mut self.pixels, self.size.0)
            };
            let mut canvas = Canvas { pixels: target, width, origin };
            for quad in quads {
                draw_quad(&mut canvas, quad, &look, clip, image, &self.atlas);
            }
        }
        if let Some(at) = in_layer {
            self.put_down_layer(&scene.layers[at]);
        }
    }

    /// Starts drawing what's faded as a whole, on a clear layer of its own.
    fn start_layer(&mut self, layer: &Layer) {
        let (x, y) = layer_origin(layer, self.size);
        let width = (layer.bounds.right.min(self.size.0 as i32) - x).max(1) as u32;
        let height = (layer.bounds.bottom.min(self.size.1 as i32) - y).max(1) as u32;
        if self.layer_size.0 < width || self.layer_size.1 < height {
            self.layer_size = (self.layer_size.0.max(width), self.layer_size.1.max(height));
            self.layer = vec![[0; 4]; self.layer_size.0 as usize * self.layer_size.1 as usize];
        } else {
            self.layer.fill([0; 4]);
        }
    }

    /// Puts what's been drawn on the layer down on what's drawn to, at the layer's opacity, as the quad the Direct3D
    /// renderer draws for it does: a copy of each pixel, through the layer's opacity, and blended as anything else is.
    fn put_down_layer(&mut self, layer: &Layer) {
        let origin = layer_origin(layer, self.size);
        let quad = layer_quad(layer, self.layer_size, origin);
        let alpha = f32::from(quad.color[3]) / 255.0;
        let visible = intersect(
            layer.bounds,
            PixelRect {
                left: origin.0,
                top: origin.1,
                right: (origin.0 + self.layer_size.0 as i32).min(self.size.0 as i32),
                bottom: (origin.1 + self.layer_size.1 as i32).min(self.size.1 as i32),
            },
        );
        if is_empty(visible) {
            return;
        }
        for y in visible.top..visible.bottom {
            for x in visible.left..visible.right {
                let from = self.layer[(y - origin.1) as usize * self.layer_size.0 as usize + (x - origin.0) as usize];
                if from[3] == 0 && from[0] == 0 && from[1] == 0 && from[2] == 0 {
                    continue;
                }
                let color = from.map(|part| f32::from(part) / 255.0 * alpha);
                let at = y as usize * self.size.0 as usize + x as usize;
                self.pixels[at] = blend(self.pixels[at], color, [color[3]; 4]);
            }
        }
    }

    /// Draws a scene, and returns the pixels, as blue, green, red and how opaque each is, in rows from the top, with
    /// colors multiplied by that. This is for keeping the picture some other way.
    pub fn draw_to_memory(&mut self, scene: &mut Scene, clear: Color) -> Result<Vec<u8>, RenderError> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if self.vulkan.is_some() {
            self.draw_with_vulkan(scene, clear, true)?;
            let mut out = self.vulkan.as_ref().expect("drawing with Vulkan").read_pixels()?;
            for pixel in out.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            return Ok(out);
        }
        self.known = false;
        self.draw(scene, clear)?;
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for [r, g, b, a] in &self.pixels {
            out.extend_from_slice(&[*b, *g, *r, *a]);
        }
        Ok(out)
    }

    /// Gives back most of what this holds, for when it isn't showing.
    pub fn hide(&mut self) -> Result<(), RenderError> {
        // A window drawn with Vulkan keeps its swapchain, which is what the compositor shows; the rest is let go of.
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if self.vulkan.is_some() {
            self.trim();
            return Ok(());
        }
        self.resize((1, 1))?;
        self.trim();
        Ok(())
    }

    /// Memory has no spare buffer to let go of.
    pub fn compact(&mut self) -> Result<bool, RenderError> {
        Ok(false)
    }

    /// Changes the size of what's drawn to, in physical pixels.
    pub fn resize(&mut self, size: (u32, u32)) -> Result<(), RenderError> {
        let size = (size.0.max(1), size.1.max(1));
        if size == self.size {
            return Ok(());
        }
        self.size = size;
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if let Some(vulkan) = &mut self.vulkan {
            vulkan.resize(size);
            self.known = false;
            return Ok(());
        }
        self.pixels = vec![[0; 4]; size.0 as usize * size.1 as usize];
        self.layer = Vec::new();
        self.layer_size = (0, 0);
        self.known = false;
        Ok(())
    }

    /// Gives back memory that isn't in use.
    pub fn trim(&mut self) {
        self.trim_storage();
        give_back_memory();
    }

    /// Lets go of images that are gone.
    pub fn release_images(&mut self, dropped: &mut Vec<u32>) {
        for id in dropped.drain(..) {
            #[cfg(any(target_os = "linux", target_os = "android"))]
            let had = match &mut self.vulkan {
                Some(vulkan) => vulkan.drop_picture(id),
                None => self.images.remove(&id).is_some(),
            };
            #[cfg(not(any(target_os = "linux", target_os = "android")))]
            let had = self.images.remove(&id).is_some();
            if had && self.last.batches.iter().any(|batch| batch.image == id && batch.count != 0) {
                self.known = false;
            }
        }
    }

    /// Lets go of what the last frame was, and the layer, which are made again when they're next needed.
    pub fn trim_storage(&mut self) {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if let Some(vulkan) = &mut self.vulkan {
            vulkan.trim();
        }
        self.layer = Vec::new();
        self.layer_size = (0, 0);
        self.last = Last::default();
        self.known = false;
    }

    /// The pixels drawn so far, as red, green, blue and alpha for each, in rows from the top.
    pub fn read_pixels(&self) -> Result<Vec<u8>, RenderError> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if let Some(vulkan) = &self.vulkan {
            return vulkan.read_pixels();
        }
        Ok(self.pixels.iter().flatten().copied().collect())
    }

    /// Makes sure the atlas is the size the scene needs, keeping what's in it if it only grew.
    fn prepare_atlas(&mut self, size: u32) {
        let size = size.max(1);
        if size == self.atlas_size {
            return;
        }
        let mut atlas = vec![0u8; size as usize * size as usize];
        if self.atlas_size < size {
            let old = self.atlas_size as usize;
            for row in 0..old {
                atlas[row * size as usize..row * size as usize + old].copy_from_slice(&self.atlas[row * old..row * old + old]);
            }
        }
        self.atlas = atlas;
        self.atlas_size = size;
    }

    fn upload_glyphs(&mut self, uploads: &[Upload]) {
        let size = self.atlas_size as usize;
        for upload in uploads {
            let (x, y, width) = (upload.x as usize, upload.y as usize, upload.width as usize);
            for row in 0..upload.height as usize {
                let (Some(from), true) = (upload.coverage.get(row * width..row * width + width), y + row < size && x + width <= size)
                else {
                    continue;
                };
                let at = (y + row) * size + x;
                self.atlas[at..at + width].copy_from_slice(from);
            }
        }
    }

    /// Takes new images and lets go of old ones. Says whether one that was drawn last time and is drawn again changed.
    fn upload_images(&mut self, scene: &mut Scene) -> bool {
        let mut changed = false;
        let referenced = |batches: &[Batch], id: u32| batches.iter().any(|batch| batch.image == id && batch.count != 0);
        for id in scene.dropped_images.drain(..) {
            if self.images.remove(&id).is_some() {
                changed |= referenced(&self.last.batches, id) && referenced(&scene.batches, id);
            }
        }
        for upload in scene.image_uploads.drain(..) {
            let ImageUpload { id, width, height, pixels, changes } = upload;
            // An image with no pixels, or fewer than it says, is turned down, as a GPU turns down such a texture, and
            // whatever had this number stays as it was.
            if width == 0 || height == 0 || pixels.len() < width as usize * height as usize * 4 {
                continue;
            }
            if let (Some((x, y)), Some(image)) = (changes, self.images.get_mut(&id)) {
                for row in 0..height {
                    let (to_y, to_x) = ((y + row) as usize, x as usize);
                    if to_y >= image.height as usize || to_x + width as usize > image.width as usize {
                        continue;
                    }
                    let from = row as usize * width as usize * 4;
                    let to = (to_y * image.width as usize + to_x) * 4;
                    image.pixels[to..to + width as usize * 4].copy_from_slice(&pixels[from..from + width as usize * 4]);
                }
            } else {
                self.images.insert(id, Image { width, height, pixels });
            }
            // What was drawn without it, as one that hadn't come yet, changes as much as one that's replaced.
            changed |= referenced(&self.last.batches, id) && referenced(&scene.batches, id);
        }
        changed
    }
}

/// Where a layer's top left is, on what's drawn to.
fn layer_origin(layer: &Layer, size: (u32, u32)) -> (i32, i32) {
    (layer.bounds.left.clamp(0, size.0 as i32), layer.bounds.top.clamp(0, size.1 as i32))
}

/// What a batch is drawn with, as the shaders' constants say it.
struct Look {
    matrix: Matrix,
    inverse: Matrix,
    transformed: bool,
    sharpness: f32,
    atlas_size: f32,
    gamma: f32,
    contrast: f32,
    filter: Option<Filter>,
    rounds: RoundSlots,
}

impl Look {
    fn new(matrix: Option<Matrix>, filter: Option<Filter>, rounds: RoundSlots, atlas_size: u32, text_look: [f32; 2]) -> Self {
        let sharpness = matrix.map_or(1.0, |[a, b, c, d, ..]| (a * d - b * c).abs().sqrt());
        let inverse = matrix.and_then(invert_matrix).unwrap_or(IDENTITY);
        Self {
            matrix: matrix.unwrap_or(IDENTITY),
            inverse,
            transformed: matrix.is_some(),
            sharpness,
            atlas_size: atlas_size as f32,
            gamma: text_look[0],
            contrast: text_look[1],
            filter,
            rounds,
        }
    }

    fn plain(&self) -> bool {
        !self.transformed && self.filter.is_none() && self.rounds == NO_ROUND_CLIPS
    }
}

/// Where what's drawn goes: whole rows of `width` pixels, whose top left is at `origin` on the window.
struct Canvas<'a> {
    pixels: &'a mut [[u8; 4]],
    width: u32,
    origin: (i32, i32),
}

impl Canvas<'_> {
    #[inline]
    fn at(&mut self, x: i32, y: i32) -> Option<&mut [u8; 4]> {
        let (x, y) = (x - self.origin.0, y - self.origin.1);
        if x < 0 || y < 0 || x as u32 >= self.width {
            return None;
        }
        self.pixels.get_mut(y as usize * self.width as usize + x as usize)
    }
}

/// What's behind, with `color` over it, covering each of its channels as much as `cover` says, as the blend state does:
/// `color + behind * (1 - cover)`.
#[inline]
fn blend(behind: [u8; 4], color: [f32; 4], cover: [f32; 4]) -> [u8; 4] {
    let mut out = [0u8; 4];
    for part in 0..4 {
        let value = color[part] + f32::from(behind[part]) / 255.0 * (1.0 - cover[part]);
        out[part] = unorm(value);
    }
    out
}

/// A value from 0 to 1 as a byte, as a GPU stores it.
#[inline]
fn unorm(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round_ties_even() as u8
}

#[inline]
fn saturate(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

/// Draws one quad over the pixels it covers inside `clip`.
fn draw_quad(canvas: &mut Canvas, quad: &Quad, look: &Look, clip: PixelRect, image: Option<&Image>, atlas: &[u8]) {
    let [x, y, width, height] = quad.rect;
    if !(width > 0.0 && height > 0.0) || !x.is_finite() || !y.is_finite() {
        return;
    }
    // The pixels whose middles might be inside it.
    let (left, top, right, bottom) = if look.transformed {
        let corners = [(x, y), (x + width, y), (x, y + height), (x + width, y + height)]
            .map(|(px, py)| apply_matrix(look.matrix, px, py));
        let (mut l, mut t, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (px, py) in corners {
            l = l.min(px);
            t = t.min(py);
            r = r.max(px);
            b = b.max(py);
        }
        ((l - 0.5).ceil(), (t - 0.5).ceil(), (r - 0.5).ceil(), (b - 0.5).ceil())
    } else {
        ((x - 0.5).ceil(), (y - 0.5).ceil(), (x + width - 0.5).ceil(), (y + height - 0.5).ceil())
    };
    let clamp = |value: f32, low: i32, high: i32| (value.max(low as f32).min(high as f32)) as i32;
    let left = clamp(left, clip.left, clip.right);
    let right = clamp(right, clip.left, clip.right);
    let top = clamp(top, clip.top, clip.bottom);
    let bottom = clamp(bottom, clip.top, clip.bottom);
    if left >= right || top >= bottom {
        return;
    }
    let color = quad.color.map(|part| f32::from(part) / 255.0);
    let glyph = quad.glyph;
    let inset = glyph[2] == INSET_MARK;
    let plain_rect = image.is_none() && glyph == [0, 0, 0, 0] && quad.radius == 0.0;
    // A rectangle of one color, which is most of every frame: the same color over every pixel it covers.
    if plain_rect && look.plain() {
        if color[3] == 0.0 && color[0] == 0.0 && color[1] == 0.0 && color[2] == 0.0 {
            return;
        }
        let opaque = quad.color[3] == 255;
        for py in top..bottom {
            for px in left..right {
                if let Some(pixel) = canvas.at(px, py) {
                    *pixel = if opaque { quad.color } else { blend(*pixel, color, [color[3]; 4]) };
                }
            }
        }
        return;
    }
    let shape_w = if inset { -f32::from(glyph[1]) / 16.0 } else { (f32::from(glyph[0]) - f32::from(glyph[1])) / 16.0 };
    let shape = [width / 2.0, height / 2.0, quad.radius, shape_w];
    let inset_at = if inset { [f32::from(glyph[0]) / 16.0, 1.0] } else { [0.0, 0.0] };
    let inset_uv = [f32::from(glyph[3] >> 8) - 128.0, f32::from(glyph[3] & 255) - 128.0];
    // How bright the color is, for glyphs drawn as a browser draws them, worked out once for the quad.
    let gray_curve = (look.gamma > 0.0 && image.is_none() && glyph[2] > 0 && !inset).then(|| {
        let rgb = unpremultiplied(color);
        let brightness = ((rgb[0] * 255.0).round_ties_even() * 54.0
            + (rgb[1] * 255.0).round_ties_even() * 183.0
            + (rgb[2] * 255.0).round_ties_even() * 19.0)
            / 256.0;
        CoverageCurve::new(brightness.floor(), look)
    });
    let lcd = image.is_none()
        && glyph[2] > 0
        && glyph[3] > 0
        && !inset
        && quad.radius < 0.0
        && !look.transformed
        && look.filter.is_none();
    let lcd_curves = (lcd && look.gamma > 0.0).then(|| {
        let rgb = unpremultiplied(color);
        rgb.map(|part| CoverageCurve::new((part * 255.0).round_ties_even(), look))
    });
    let atlas_size = look.atlas_size.max(1.0) as usize;
    for py in top..bottom {
        for px in left..right {
            let screen = (px as f32 + 0.5, py as f32 + 0.5);
            let before = if look.transformed { apply_matrix(look.inverse, screen.0, screen.1) } else { screen };
            if look.transformed
                && !(before.0 >= x && before.0 < x + width && before.1 >= y && before.1 < y + height)
            {
                continue;
            }
            let local = [before.0 - x - width / 2.0, before.1 - y - height / 2.0];
            let corner = [(before.0 - x) / width, (before.1 - y) / height];
            let uv = if inset {
                inset_uv
            } else {
                [
                    if glyph[2] > 0 { f32::from(glyph[0]) + corner[0] * f32::from(glyph[2]) } else { -1.0 },
                    if glyph[3] > 0 { f32::from(glyph[1]) + corner[1] * f32::from(glyph[3]) } else { -1.0 },
                ]
            };
            let at = [before.0, before.1, screen.0, screen.1];
            let rounds = round_clips_cover(&look.rounds, at, look.sharpness);
            if rounds <= 0.0 {
                continue;
            }
            let Some(pixel) = canvas.at(px, py) else { continue };
            if lcd {
                let (ax, ay) = (uv[0].floor() as isize, uv[1].floor() as isize);
                let load = |x: isize| -> f32 {
                    if x < 0 || ay < 0 || x as usize >= atlas_size || ay as usize >= atlas_size {
                        0.0
                    } else {
                        f32::from(atlas[ay as usize * atlas_size + x as usize]) / 255.0
                    }
                };
                let mut cover = [load(ax - 1), load(ax), load(ax + 1)];
                if let Some(curves) = &lcd_curves {
                    for part in 0..3 {
                        cover[part] = curves[part].apply(cover[part], look);
                    }
                }
                let cover = cover.map(|part| part * rounds);
                let out_color = [
                    color[0] * cover[0],
                    color[1] * cover[1],
                    color[2] * cover[2],
                    color[3] * cover[0].max(cover[1]).max(cover[2]),
                ];
                let out_cover = [cover[0] * color[3], cover[1] * color[3], cover[2] * color[3], out_color[3]];
                *pixel = blend(*pixel, out_color, out_cover);
                continue;
            }
            let shaded = match image {
                Some(image) => image_color(color, uv, local, shape, look, image),
                None => color_of(
                    &Shading { color, local, shape, uv, inset: inset_at },
                    look,
                    atlas,
                    atlas_size,
                    gray_curve.as_ref(),
                ),
            };
            let shaded = shaded.map(|part| part * rounds);
            let drawn = filter_color(shaded, look.filter.as_ref());
            if drawn == [0.0; 4] {
                continue;
            }
            *pixel = blend(*pixel, drawn, [drawn[3]; 4]);
        }
    }
}

/// What the pixel shader is given at a pixel.
struct Shading {
    color: [f32; 4],
    local: [f32; 2],
    shape: [f32; 4],
    uv: [f32; 2],
    inset: [f32; 2],
}

fn unpremultiplied(color: [f32; 4]) -> [f32; 3] {
    if color[3] > 0.0 {
        [color[0] / color[3], color[1] / color[3], color[2] / color[3]]
    } else {
        [color[0], color[1], color[2]]
    }
}

fn length(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

/// How far a point is outside a rounded rectangle of this half size and radius, from its middle: negative inside.
fn rounded_distance(local: [f32; 2], half: [f32; 2], radius: f32) -> f32 {
    let inside = [local[0].abs() - half[0] + radius, local[1].abs() - half[1] + radius];
    length(inside[0].max(0.0), inside[1].max(0.0)) + inside[0].max(inside[1]).min(0.0) - radius
}

fn rounded_cover(local: [f32; 2], half: [f32; 2], radius: f32, sharpness: f32) -> f32 {
    saturate(0.5 - rounded_distance(local, half, radius) * sharpness)
}

/// The error function, as the shaders work it out.
fn erf(x: f32) -> f32 {
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let y = 1.0
        - (((((1.061_405_4 * t - 1.453_152) * t) + 1.421_413_8) * t - 0.284_496_74) * t + 0.254_829_6) * t * (-x * x).exp();
    if x < 0.0 { -y } else { y }
}

/// Skia's mask gamma, as the shaders have it, for one brightness, which a quad's glyphs all share: worked out for each
/// of the 256 coverages a glyph's pixel can have, once.
struct CoverageCurve {
    table: [f32; 256],
}

impl CoverageCurve {
    fn new(brightness: f32, look: &Look) -> Self {
        let mut table = [0f32; 256];
        for (at, value) in table.iter_mut().enumerate() {
            *value = browser_curve(at as f32 / 255.0, brightness, look.gamma, look.contrast);
        }
        Self { table }
    }

    fn apply(&self, coverage: f32, look: &Look) -> f32 {
        let at = coverage * 255.0;
        if at.fract() == 0.0 && (0.0..=255.0).contains(&at) {
            self.table[at as usize]
        } else {
            // Read smoothly from a transformed atlas, which isn't one of the 256.
            let _ = look;
            self.table[(at.round_ties_even().clamp(0.0, 255.0)) as usize]
        }
    }
}

fn gamma_of(value: f32, gamma: f32) -> f32 {
    if value > 0.0 { value.powf(gamma) } else { 0.0 }
}

fn browser_curve(coverage: f32, brightness: f32, gamma: f32, contrast: f32) -> f32 {
    let step = (brightness / 32.0).floor();
    let src = (step * 32.0 + step * 4.0 + (step / 2.0).floor()) / 255.0;
    let dst = 1.0 - src;
    let lin_src = gamma_of(src, gamma);
    let lin_dst = gamma_of(dst, gamma);
    let a = coverage + (1.0 - coverage) * contrast * lin_dst * coverage;
    if (src - dst).abs() < 1.0 / 256.0 {
        return (a * 255.0).round_ties_even() / 255.0;
    }
    let lin_out = lin_src * a + (1.0 - a) * lin_dst;
    let out_value = if lin_out > 0.0 { lin_out.powf(1.0 / gamma) } else { 0.0 };
    (255.0 * (out_value - dst) / (src - dst)).round_ties_even() / 255.0
}

/// Reads the atlas smoothly, as a sampler does, at a place in pixels.
fn sample_atlas(atlas: &[u8], size: usize, u: f32, v: f32) -> f32 {
    let (tx, ty) = (u - 0.5, v - 0.5);
    let (x0, y0) = (tx.floor(), ty.floor());
    let (fx, fy) = (tx - x0, ty - y0);
    let get = |x: f32, y: f32| -> f32 {
        let x = (x as isize).clamp(0, size as isize - 1) as usize;
        let y = (y as isize).clamp(0, size as isize - 1) as usize;
        f32::from(atlas.get(y * size + x).copied().unwrap_or(0)) / 255.0
    };
    let top = get(x0, y0) * (1.0 - fx) + get(x0 + 1.0, y0) * fx;
    let low = get(x0, y0 + 1.0) * (1.0 - fx) + get(x0 + 1.0, y0 + 1.0) * fx;
    top * (1.0 - fy) + low * fy
}

/// Reads an image smoothly, as a sampler does, at a place from 0 to 1 across and down.
fn sample_image(image: &Image, u: f32, v: f32) -> [f32; 4] {
    let (width, height) = (image.width.max(1) as isize, image.height.max(1) as isize);
    let (tx, ty) = (u * width as f32 - 0.5, v * height as f32 - 0.5);
    let (x0, y0) = (tx.floor(), ty.floor());
    let (fx, fy) = (tx - x0, ty - y0);
    let get = |x: f32, y: f32| -> [f32; 4] {
        let x = (x as isize).clamp(0, width - 1) as usize;
        let y = (y as isize).clamp(0, height - 1) as usize;
        let at = (y * image.width as usize + x) * 4;
        match image.pixels.get(at..at + 4) {
            Some(pixel) => [pixel[0], pixel[1], pixel[2], pixel[3]].map(|part| f32::from(part) / 255.0),
            None => [0.0; 4],
        }
    };
    let (a, b, c, d) = (get(x0, y0), get(x0 + 1.0, y0), get(x0, y0 + 1.0), get(x0 + 1.0, y0 + 1.0));
    let mut out = [0f32; 4];
    for part in 0..4 {
        let top = a[part] * (1.0 - fx) + b[part] * fx;
        let low = c[part] * (1.0 - fx) + d[part] * fx;
        out[part] = top * (1.0 - fy) + low * fy;
    }
    out
}

fn image_color(color: [f32; 4], uv: [f32; 2], local: [f32; 2], shape: [f32; 4], look: &Look, image: &Image) -> [f32; 4] {
    let sample = sample_image(image, uv[0] / 65535.0, uv[1] / 65535.0);
    let mut out = [color[0] * sample[0], color[1] * sample[1], color[2] * sample[2], color[3] * sample[3]];
    if shape[2] > 0.0 || look.transformed {
        let cover = rounded_cover(local, [shape[0], shape[1]], shape[2], look.sharpness);
        out = out.map(|part| part * cover);
    }
    out
}

fn color_of(input: &Shading, look: &Look, atlas: &[u8], atlas_size: usize, curve: Option<&CoverageCurve>) -> [f32; 4] {
    let color = input.color;
    let scaled = |by: f32| color.map(|part| part * by);
    if input.inset[1] > 0.0 {
        // A shadow inside something.
        let reach = -input.shape[3];
        let spread = input.inset[0];
        let local = [input.local[0] - input.uv[0], input.local[1] - input.uv[1]];
        let half = [(input.shape[0] - spread).max(0.0), (input.shape[1] - spread).max(0.0)];
        let radius = (input.shape[2] - spread).max(0.0);
        let distance = rounded_distance(local, half, radius);
        let hole = if reach > 0.0 {
            0.5 * (1.0 - erf(distance / (reach / 3.0 * std::f32::consts::SQRT_2)))
        } else {
            saturate(0.5 - distance * look.sharpness)
        };
        let cover = rounded_cover(input.local, [input.shape[0], input.shape[1]], input.shape[2], look.sharpness);
        return scaled(cover * (1.0 - hole));
    }
    if input.uv[0] >= 0.0 {
        let mut coverage = if look.transformed {
            sample_atlas(atlas, atlas_size, input.uv[0], input.uv[1])
        } else {
            let (x, y) = (input.uv[0].floor() as isize, input.uv[1].floor() as isize);
            if x < 0 || y < 0 || x as usize >= atlas_size || y as usize >= atlas_size {
                0.0
            } else {
                f32::from(atlas[y as usize * atlas_size + x as usize]) / 255.0
            }
        };
        if let Some(curve) = curve {
            coverage = curve.apply(coverage, look);
        }
        return scaled(coverage);
    }
    if input.shape[3] < 0.0 {
        // A soft shadow.
        let reach = -input.shape[3];
        let box_half = [input.shape[0] - reach, input.shape[1] - reach];
        let spread = reach / 3.0 * std::f32::consts::SQRT_2;
        if input.shape[2] <= 0.0 {
            let near = [(input.local[0] + box_half[0]) / spread, (input.local[1] + box_half[1]) / spread];
            let far = [(input.local[0] - box_half[0]) / spread, (input.local[1] - box_half[1]) / spread];
            let each = [0.5 * (erf(near[0]) - erf(far[0])), 0.5 * (erf(near[1]) - erf(far[1]))];
            return scaled(each[0] * each[1]);
        }
        let distance = rounded_distance(input.local, box_half, input.shape[2]);
        return scaled(0.5 * (1.0 - erf(distance / spread)));
    }
    if input.shape[2] > 0.0 || input.shape[3] > 0.0 || look.transformed {
        let distance = rounded_distance(input.local, [input.shape[0], input.shape[1]], input.shape[2]);
        let mut coverage = saturate(0.5 - distance * look.sharpness);
        if input.shape[3] > 0.0 {
            coverage *= saturate(0.5 + (distance + input.shape[3]) * look.sharpness);
        }
        return scaled(coverage);
    }
    color
}

/// How much of a pixel at `at` is inside a box with round corners, as the shaders work it out.
fn round_clip_cover(at: [f32; 2], clip: &RoundClip, sharp: f32) -> f32 {
    let [left, top, right, bottom] = clip.edges;
    let middle = [(left + right) / 2.0, (top + bottom) / 2.0];
    let half = [(right - left) / 2.0, (bottom - top) / 2.0];
    let local = [at[0] - middle[0], at[1] - middle[1]];
    let [top_left, top_right, bottom_right, bottom_left] = clip.corners;
    let curve = if local[1] < 0.0 {
        if local[0] < 0.0 { top_left } else { top_right }
    } else if local[0] < 0.0 {
        bottom_left
    } else {
        bottom_right
    };
    let away = [local[0].abs() - half[0] + curve[0], local[1].abs() - half[1] + curve[1]];
    let distance = if away[0] > 0.0 && away[1] > 0.0 && curve[0] > 0.0 && curve[1] > 0.0 {
        let along = length(away[0] / curve[0], away[1] / curve[1]);
        along * (along - 1.0) / length(away[0] / (curve[0] * curve[0]), away[1] / (curve[1] * curve[1]))
    } else {
        let out_by = [local[0].abs() - half[0], local[1].abs() - half[1]];
        out_by[0].max(out_by[1])
    };
    saturate(0.5 - distance * sharp)
}

fn round_clips_cover(rounds: &RoundSlots, at: [f32; 4], sharpness: f32) -> f32 {
    let mut cover = 1.0;
    for (clip, on_screen) in rounds.iter().flatten() {
        let (point, sharp) = if *on_screen { ([at[2], at[3]], 1.0) } else { ([at[0], at[1]], sharpness) };
        let inside = round_clip_cover(point, clip, sharp);
        cover *= if clip.outside { 1.0 - inside } else { inside };
    }
    cover
}

fn filter_color(color: [f32; 4], filter: Option<&Filter>) -> [f32; 4] {
    let Some(filter) = filter else { return color };
    if color[3] <= 0.0 {
        return color;
    }
    let plain = [color[0] / color[3], color[1] / color[3], color[2] / color[3], 1.0];
    let row = |at: usize| {
        let r = &filter.rows[at * 4..at * 4 + 4];
        saturate(r[0] * plain[0] + r[1] * plain[1] + r[2] * plain[2] + r[3] * plain[3])
    };
    let alpha = color[3] * filter.opacity;
    [row(0) * alpha, row(1) * alpha, row(2) * alpha, alpha]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene_of(quads: Vec<Quad>, size: (u32, u32)) -> Scene {
        let mut scene = Scene::default();
        let count = quads.len() as u32;
        scene.quads = quads;
        scene.batches = vec![Batch {
            first: 0,
            count,
            clip: PixelRect { left: 0, top: 0, right: size.0 as i32, bottom: size.1 as i32 },
            image: 0,
            transform: 0,
            filter: 0,
            round: 0,
        }];
        scene
    }

    fn rect(rect: [f32; 4], color: [u8; 4], radius: f32) -> Quad {
        Quad { rect, glyph: [0; 4], color, radius }
    }

    #[test]
    fn a_rectangle_covers_the_pixels_whose_middles_are_in_it() {
        let mut renderer = Renderer::offscreen((8, 4)).unwrap();
        let mut scene = scene_of(vec![rect([1.4, 0.0, 3.2, 2.0], [255, 0, 0, 255], 0.0)], (8, 4));
        renderer.render(&mut scene, Color::WHITE).unwrap();
        let pixels = renderer.read_pixels().unwrap();
        let red = |x: usize, y: usize| pixels[(y * 8 + x) * 4..(y * 8 + x) * 4 + 4] == [255, 0, 0, 255];
        // Middles at 1.5 .. 4.5 are inside 1.4 .. 4.6.
        assert!(!red(0, 0) && red(1, 0) && red(4, 0) && !red(5, 0));
        assert!(red(1, 1) && !red(1, 2));
    }

    #[test]
    fn see_through_colors_go_over_what_is_behind() {
        let mut renderer = Renderer::offscreen((2, 1)).unwrap();
        let mut scene = scene_of(vec![rect([0.0, 0.0, 2.0, 1.0], [0, 0, 128, 128], 0.0)], (2, 1));
        renderer.render(&mut scene, Color::WHITE).unwrap();
        let pixels = renderer.read_pixels().unwrap();
        // 128/255 blue over white: red and green are what's left of white, and blue is full.
        assert_eq!(&pixels[..4], &[127, 127, 255, 255]);
    }

    #[test]
    fn round_corners_leave_the_corner_pixel_out() {
        let mut renderer = Renderer::offscreen((20, 20)).unwrap();
        let mut scene = scene_of(vec![rect([0.0, 0.0, 20.0, 20.0], [0, 0, 0, 255], 8.0)], (20, 20));
        renderer.render(&mut scene, Color::WHITE).unwrap();
        let pixels = renderer.read_pixels().unwrap();
        assert_eq!(&pixels[..4], &[255, 255, 255, 255], "the corner shows what's behind");
        let middle = (10 * 20 + 10) * 4;
        assert_eq!(&pixels[middle..middle + 4], &[0, 0, 0, 255]);
    }

    #[test]
    fn the_same_scene_twice_draws_nothing_the_second_time() {
        let mut renderer = Renderer::offscreen((4, 4)).unwrap();
        let quads = vec![rect([0.0, 0.0, 2.0, 2.0], [0, 255, 0, 255], 0.0)];
        renderer.render(&mut scene_of(quads.clone(), (4, 4)), Color::WHITE).unwrap();
        assert!(!renderer.showed_nothing());
        renderer.render(&mut scene_of(quads, (4, 4)), Color::WHITE).unwrap();
        assert!(renderer.showed_nothing());
    }
}
