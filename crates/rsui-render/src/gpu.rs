//! Puts scenes on screen with Direct3D 11.
//!
//! Every choice here comes from measuring what a window costs:
//! Direct3D 11 rather than 12, a flip swap chain with the fewest buffers it allows, and
//! driver threading turned off.

use std::collections::HashMap;
use std::ffi::c_void;
use std::fmt;

use rsui_text::Upload;
use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG, HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL,
    D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0,
    D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::{
    IDCompositionDesktopDevice, IDCompositionTarget, IDCompositionVisual, IDCompositionVisual2,
};
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::ProcessStatus::K32EmptyWorkingSet;
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::core::{HRESULT, Interface, s, w};

use crate::images::ImageUpload;
use crate::paint::Color;
use crate::scene::{BACKDROP_IMAGE, Batch, Filter, Layer, PixelRect, Quad, RoundClip, Scene};
use crate::transform::Matrix;
use crate::area::backdrop_areas;
pub(crate) use crate::area::{
    NO_AREA, NO_ROUND_CLIPS, RoundSlots, area_of, intersect, is_empty, layer_quad, placed_area, round_clips_of, union,
};

/// What showing a frame says when it can't be seen, `DXGI_STATUS_OCCLUDED`, which is a success: nothing waited for the screen.
const OCCLUDED: HRESULT = HRESULT(0x087A_0001);

#[cfg(feature = "render-diagnostics")]
#[path = "gpu_probe.rs"]
mod gpu_probe;

const VERTEX_SHADER: &[u8] = include_bytes!("quad_vs.dxbc");
const PIXEL_SHADER: &[u8] = include_bytes!("quad_ps.dxbc");
const IMAGE_SHADER: &[u8] = include_bytes!("image_ps.dxbc");
const BACKDROP_SHADER: &[u8] = include_bytes!("backdrop_ps.dxbc");

struct BackdropTarget {
    size: (u32, u32),
    texture: ID3D11Texture2D,
    picture: ID3D11ShaderResourceView,
}

/// How many quads the buffer starts with room for. Most UIs need far less.
const FIRST_QUAD_CAPACITY: u32 = 2048;
const FORMAT: DXGI_FORMAT = DXGI_FORMAT_B8G8R8A8_UNORM;

/// Why a frame couldn't be drawn.
#[derive(Debug, Clone)]
pub enum RenderError {
    /// The GPU was reset or removed, as happens when a driver updates. Everything on it
    /// is gone, so make a new [`Renderer`] and a new `Painter`, and draw again.
    DeviceLost,
    /// Direct3D reported something else.
    Graphics(windows::core::Error),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceLost => f.write_str("the GPU was reset or removed"),
            Self::Graphics(error) => write!(f, "Direct3D failed: {error}"),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<windows::core::Error> for RenderError {
    fn from(error: windows::core::Error) -> Self {
        if error.code() == DXGI_ERROR_DEVICE_REMOVED || error.code() == DXGI_ERROR_DEVICE_RESET {
            Self::DeviceLost
        } else {
            Self::Graphics(error)
        }
    }
}

enum Target {
    Window(IDXGISwapChain1),
    /// For drawing without a window.
    Offscreen {
        texture: ID3D11Texture2D,
        /// A second buffer to swap with, to behave as a window's buffers do.
        spare: Option<(ID3D11Texture2D, ID3D11RenderTargetView)>,
    },
}

struct FrameCapture {
    region: PixelRect,
    staging: ID3D11Texture2D,
    copied: bool,
}

/// What puts a window's swap chain over the window, through DirectComposition, rather than the window having one of its own.
///
/// A swap chain of the window's own can't have alpha, and once one has shown a frame, the system keeps showing its last frame
/// for as long as the window lasts, even after it's let go of, under anything shown this way. So a window that's given a
/// backdrop after its first frame would have that frame in front of the backdrop. Every window's swap chain is shown this way
/// instead: opaque, as a window's own is, until it has a backdrop, and then with alpha, by swapping the swap chain for another
/// in the same visual. Measured against a window's own swap chain, a window without a backdrop takes the same time to start
/// and to draw a frame, the same CPU while idle, and the same memory and GPU memory. What's added is `dcomp.dll` being loaded
/// and DirectComposition's device made, beside the GPU's device (see [`start_gpu_early`]), and about a millisecond of CPU on
/// a thread-pool thread once, a few seconds after.
struct Composition {
    device: IDCompositionDesktopDevice,
    target: IDCompositionTarget,
    /// What shows the swap chain, which the target has as its root.
    visual: IDCompositionVisual2,
    /// Whether the visual shows the swap chain that's drawn to now. A new one is only shown once it's had a frame, so that the
    /// window isn't empty for a moment in between.
    showing: bool,
}

impl Composition {
    /// Has the visual show `chain`, which has just been given a frame, if it doesn't already.
    fn show(&mut self, chain: &IDXGISwapChain1) -> Result<(), RenderError> {
        if !self.showing {
            // SAFETY: the chain, the visual and the device are all valid.
            unsafe {
                self.visual.SetContent(chain)?;
                self.device.Commit()?;
            }
            self.showing = true;
        }
        Ok(())
    }
}

impl Drop for Composition {
    fn drop(&mut self) {
        // The window stops showing the swap chain now, rather than when the system gets round to it.
        // SAFETY: both are valid until they're dropped, after this.
        unsafe {
            let _ = self.target.SetRoot(None::<&IDCompositionVisual>);
            let _ = self.device.Commit();
        }
    }
}

struct AtlasTexture {
    size: u32,
    texture: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
}

/// How many bytes are told to the shaders for each draw: the viewport, the atlas's size and how sharp edges should be, a
/// transform, a filter, and the round clips of [`round_clips_of`].
const FRAME_CONSTANTS_SIZE: u32 = 256;

/// Draws scenes to a window, or to memory for tests and screenshots.
pub struct Renderer {
    #[cfg(feature = "render-diagnostics")]
    full_layer_targets: bool,
    #[cfg(feature = "retained-frame")]
    frame_token: u64,
    #[cfg(feature = "mutation-pilot")]
    app_binding: u64,
    #[cfg(all(any(test, feature = "render-diagnostics"), feature = "mutation-pilot"))]
    fail_present: bool,
    #[cfg(feature = "mutation-pilot")]
    owned_stamp: Option<(u64, u64)>,
    #[cfg(feature = "mutation-pilot")]
    owned_pending: bool,
    #[cfg(feature = "render-diagnostics")]
    work: std::cell::Cell<crate::diagnostics::DrawWork>,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    target: Target,
    frame_capture: Option<FrameCapture>,
    // At most one staging surface. Submission-only renderers never allocate one.
    staging: std::cell::RefCell<Option<((u32, u32), ID3D11Texture2D)>>,
    /// The window this draws to, if it's one.
    window: HWND,
    /// What shows a window's swap chain. There's none if DirectComposition couldn't be had, and the swap chain is then the
    /// window's own, which is opaque.
    composition: Option<Composition>,
    /// Whether the window's swap chain has alpha, for the system's backdrop to show where nothing's drawn.
    see_through: bool,
    view: Option<ID3D11RenderTargetView>,
    size: (u32, u32),
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    input_layout: ID3D11InputLayout,
    blend: ID3D11BlendState,
    rasterizer: ID3D11RasterizerState,
    frame_constants: ID3D11Buffer,
    /// How glyphs' coverage is made lighter or darker for their color, as the scene being drawn says. See [`Scene::text_look`].
    text_look: [f32; 2],
    quads: ID3D11Buffer,
    quad_capacity: u32,
    atlas: Option<AtlasTexture>,
    sync_interval: u32,
    /// Whether the system said the last frame shown couldn't be seen, as when the window's behind something that has the
    /// screen to itself, which also means showing it didn't wait for the screen.
    occluded: bool,
    /// How many frames have been shown on the current buffers. The second buffer doesn't exist
    /// until there have been two.
    frames_since_compact: u32,
    /// For clearing part of a frame. This comes with Direct3D 11.1, and without it every frame
    /// is drawn in full.
    context1: Option<ID3D11DeviceContext1>,
    /// What was drawn last, to tell what's changed. Let go of when things go quiet.
    last_quads: Vec<Quad>,
    last_batches: Vec<Batch>,
    last_clear: Color,
    last_generation: u32,
    /// Whether the above says what's in the last frame.
    known: bool,
    /// What changed in the last frame, which the back buffer doesn't have yet if it's one frame
    /// further behind, as a window's is.
    previous_change: PixelRect,
    /// What this frame changes, to tell the system, unless it's everything.
    dirty: Option<PixelRect>,
    /// Whether this frame is the same as the last, so that nothing needs doing.
    nothing_to_show: bool,
    always_redraw: bool,
    /// What goes to the GPU when only part of a frame is drawn.
    partial_quads: Vec<Quad>,
    partial_batches: Vec<Batch>,
    /// What draws images, made the first time there's one to draw.
    image_pipeline: Option<(ID3D11PixelShader, ID3D11SamplerState)>,
    last_transforms: Vec<Matrix>,
    last_filters: Vec<Filter>,
    last_round_clips: Vec<RoundClip>,
    /// Images on the GPU, each only there as long as this holds it.
    images: HashMap<u32, ID3D11ShaderResourceView>,
    /// What the last frame faded as a whole, which a change to draws everything again.
    last_layers: Vec<Layer>,
    /// What something faded as a whole is drawn on before it's put down, as big as what's drawn to, made when it's first needed.
    layer_target: Option<((u32, u32), ID3D11RenderTargetView, ID3D11ShaderResourceView)>,
    layer_sampler: Option<ID3D11SamplerState>,
    backdrop_shader: Option<ID3D11PixelShader>,
    backdrop_targets: [Option<BackdropTarget>; 2],
}

/// Puts round clips where the shaders read them: each one's edges, its top corners, its bottom corners (bottom right first, as
/// they go round), then whether it keeps what's drawn inside (1) or outside (-1) or isn't there (0), and whether it's measured
/// on the screen.
fn write_round_slots(values: &mut [f32], rounds: RoundSlots) {
    for (at, round) in rounds.into_iter().enumerate() {
        let Some((clip, on_screen)) = round else {
            continue;
        };
        let [top_left, top_right, bottom_right, bottom_left] = clip.corners;
        let parts = [
            (0, clip.edges),
            (2, [top_left[0], top_left[1], top_right[0], top_right[1]]),
            (4, [bottom_right[0], bottom_right[1], bottom_left[0], bottom_left[1]]),
            (6, [if clip.outside { -1.0 } else { 1.0 }, if on_screen { 1.0 } else { 0.0 }, 0.0, 0.0]),
        ];
        for (register, part) in parts {
            let start = (register + at) * 4;
            values[start..start + 4].copy_from_slice(&part);
        }
    }
}

/// [`changed_area`] for frames that have something turned or resized in them, whose quads are where their batch's transform
/// puts them rather than where they say, and for frames whose filters (fading is one) changed, which only change the colors of
/// their own batch's quads. When the batches are the same runs with the same clips, transforms and filters, what's changed is
/// each quad that differs, where it was and where it is, and every quad of a batch whose transform or filter changed, both
/// ways. That's what a spinner turning or fading at every frame changes, where otherwise the whole window would be drawn again
/// for it.
fn changed_area_placed(
    old: (&[Quad], &[Batch], &[Matrix], &[Filter]),
    new: (&[Quad], &[Batch], &[Matrix], &[Filter]),
    whole: PixelRect,
) -> PixelRect {
    let (
        (old_quads, old_batches, old_matrices, old_filters),
        (new_quads, new_batches, new_matrices, new_filters),
    ) = (old, new);
    if old_batches.len() != new_batches.len() || old_quads.len() != new_quads.len() {
        return whole;
    }
    let matrix = |list: &[Matrix], at: u16| match at {
        0 => Some(crate::transform::IDENTITY),
        at => list.get(usize::from(at) - 1).copied(),
    };
    let filter = |list: &[Filter], at: u16| match at {
        0 => Some(None),
        at => list.get(usize::from(at) - 1).copied().map(Some),
    };
    let mut area = NO_AREA;
    for (a, b) in old_batches.iter().zip(new_batches) {
        if a.clip != b.clip
            || a.round != b.round
            || a.first != b.first
            || a.count != b.count
            || a.transform != b.transform
            || a.filter != b.filter
        {
            return whole;
        }
        let (Some(from), Some(to), Some(was), Some(is)) = (
            matrix(old_matrices, a.transform),
            matrix(new_matrices, b.transform),
            filter(old_filters, a.filter),
            filter(new_filters, b.filter),
        ) else {
            return whole;
        };
        let (first, end) = (a.first as usize, (a.first + a.count) as usize);
        let (Some(before), Some(after)) = (old_quads.get(first..end), new_quads.get(first..end))
        else {
            return whole;
        };
        // Another picture in the same place is different without its quad changing, as an animated image's frames are.
        let all = from != to || was != is || a.image != b.image;
        for (p, q) in before.iter().zip(after) {
            if all || p != q {
                area = union(area, intersect(placed_area(p, from), a.clip));
                area = union(area, intersect(placed_area(q, to), b.clip));
            }
        }
    }
    area
}

/// The area that differs between two frames, which is empty if they're the same. Comparing them
/// costs about a nanosecond per quad, and saves redrawing nearly everything.
fn changed_area<'a>(
    old: (&'a [Quad], &[Batch]),
    new: (&'a [Quad], &[Batch]),
    whole: PixelRect,
) -> PixelRect {
    let ((old_quads, old_batches), (new_quads, new_batches)) = (old, new);
    if old_batches.len() != new_batches.len() {
        return whole;
    }
    let mut area = NO_AREA;
    for (a, b) in old_batches.iter().zip(new_batches) {
        // Something clipped differently can look different without its quad changing.
        if a.clip != b.clip || a.round != b.round {
            return whole;
        }
        // Another picture in the same place is different without its quad changing, but only there.
        // That's each frame of an animated image.
        if a.image != b.image {
            let run = |quads: &'a [Quad], batch: &Batch| -> &'a [Quad] {
                let (first, count) = (batch.first as usize, batch.count as usize);
                &quads[first..first + count]
            };
            for quad in run(old_quads, a).iter().chain(run(new_quads, b)) {
                area = union(area, intersect(area_of(quad), a.clip));
            }
        }
    }
    if old_quads.len() == new_quads.len() {
        for (a, b) in old_quads.iter().zip(new_quads) {
            if a != b {
                area = union(area, union(area_of(a), area_of(b)));
            }
        }
        let same_runs = old_batches
            .iter()
            .zip(new_batches)
            .all(|(a, b)| a.first == b.first && a.count == b.count);
        return if same_runs { area } else { whole };
    }
    // Things added or removed move everything after them along the list, without moving them on
    // screen, so compare from both ends.
    let prefix = old_quads
        .iter()
        .zip(new_quads)
        .take_while(|(a, b)| a == b)
        .count();
    let most = old_quads.len().min(new_quads.len()) - prefix;
    let suffix = old_quads
        .iter()
        .rev()
        .zip(new_quads.iter().rev())
        .take(most)
        .take_while(|(a, b)| a == b)
        .count();
    for quad in old_quads[prefix..old_quads.len() - suffix]
        .iter()
        .chain(&new_quads[prefix..new_quads.len() - suffix])
    {
        area = union(area, area_of(quad));
    }
    // Whatever didn't change must also still be in the same run, and so under the same clip.
    let same_place = |a: usize, b: usize| {
        (a == b && a <= prefix)
            || (old_quads.len() - a == new_quads.len() - b && old_quads.len() - a <= suffix)
    };
    let same_runs = old_batches.iter().zip(new_batches).all(|(a, b)| {
        same_place(a.first as usize, b.first as usize)
            && same_place((a.first + a.count) as usize, (b.first + b.count) as usize)
    });
    if same_runs { area } else { whole }
}

// SAFETY: a renderer's objects can be used from any one thread at a time, which is what the
// single-threaded device promises, and owning a `Renderer` means nobody else is using them. This
// lets the slow part of starting happen on another thread.
unsafe impl Send for Renderer {}

impl Drop for Renderer {
    fn drop(&mut self) {
        // A window can only have one swap chain at a time, and this one isn't gone until
        // everything holding on to it has been dealt with.
        // SAFETY: nothing is using the context, as this owns it.
        unsafe {
            self.context.ClearState();
            self.context.Flush();
        }
    }
}

/// A limit on how much of this process can be in RAM at once, for as long as this lasts.
///
/// Starting up a GPU touches about 30 MB of a graphics driver for a moment, and hardly any of it again. Without a limit,
/// that all stays in RAM until something says otherwise, and it's most of what a program ever uses at its peak. With one,
/// the oldest of it goes back as more is touched. What's gone back normally stays in memory, ready to be used again
/// without reading from disk, so this costs some CPU rather than waiting. See `PERFORMANCE.md` in `rsui-core`.
///
/// Only one should exist at a time, as ending one ends any limit.
pub struct WorkingSetLimit {
    was: Option<(usize, usize)>,
}

const QUOTA_LIMITS_HARDWS_MIN_ENABLE: u32 = 0x1;
const QUOTA_LIMITS_HARDWS_MIN_DISABLE: u32 = 0x2;
const QUOTA_LIMITS_HARDWS_MAX_ENABLE: u32 = 0x4;
const QUOTA_LIMITS_HARDWS_MAX_DISABLE: u32 = 0x8;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn K32GetProcessMemoryInfo(
        process: *mut std::ffi::c_void,
        counters: *mut ProcessMemoryCounters,
        size: u32,
    ) -> i32;
    fn GetProcessWorkingSetSizeEx(
        process: *mut std::ffi::c_void,
        minimum: *mut usize,
        maximum: *mut usize,
        flags: *mut u32,
    ) -> i32;
    fn SetProcessWorkingSetSizeEx(
        process: *mut std::ffi::c_void,
        minimum: usize,
        maximum: usize,
        flags: u32,
    ) -> i32;
}

#[repr(C)]
#[derive(Default)]
struct ProcessMemoryCounters {
    size: u32,
    page_faults: u32,
    peak_working_set: usize,
    working_set: usize,
    other: [usize; 6],
}

impl WorkingSetLimit {
    /// Limits this process to what it has in RAM now and `room` more, or to `at_least`, whichever is more. If the system
    /// won't have it, there's no limit.
    pub fn with_room(room: usize, at_least: usize) -> Self {
        // SAFETY: these only read and change this process's limits, and the out parameters are valid.
        unsafe {
            let process = GetCurrentProcess().0;
            let mut counters = ProcessMemoryCounters {
                size: size_of::<ProcessMemoryCounters>() as u32,
                ..Default::default()
            };
            let (mut minimum, mut maximum, mut flags) = (0, 0, 0);
            if K32GetProcessMemoryInfo(process, &raw mut counters, counters.size) == 0
                || GetProcessWorkingSetSizeEx(
                    process,
                    &raw mut minimum,
                    &raw mut maximum,
                    &raw mut flags,
                ) == 0
            {
                return Self { was: None };
            }
            // Respect a limit owned by the host process or another starting window.
            if flags & (QUOTA_LIMITS_HARDWS_MIN_ENABLE | QUOTA_LIMITS_HARDWS_MAX_ENABLE) != 0 {
                return Self { was: None };
            }
            let limit = counters.working_set.saturating_add(room).max(at_least);
            let set = SetProcessWorkingSetSizeEx(
                process,
                minimum,
                limit.max(minimum + 1024 * 1024),
                QUOTA_LIMITS_HARDWS_MAX_ENABLE | QUOTA_LIMITS_HARDWS_MIN_DISABLE,
            );
            Self {
                was: (set != 0).then_some((minimum, maximum)),
            }
        }
    }
}

impl Drop for WorkingSetLimit {
    fn drop(&mut self) {
        if let Some((minimum, maximum)) = self.was {
            // SAFETY: this puts back what `with_room` read, for this process.
            unsafe {
                let _ = SetProcessWorkingSetSizeEx(
                    GetCurrentProcess().0,
                    minimum,
                    maximum,
                    QUOTA_LIMITS_HARDWS_MAX_DISABLE | QUOTA_LIMITS_HARDWS_MIN_DISABLE,
                );
            }
        }
    }
}

/// Gives back as much of this process's memory as the system will take, until it's next used.
pub fn give_back_memory() {
    // SAFETY: this only applies to the current process, and has no preconditions.
    unsafe {
        let _ = K32EmptyWorkingSet(GetCurrentProcess());
    }
}

/// `D3D11CreateDevice`, with what isn't used here as plain pointers.
type CreateDevice = unsafe extern "system" fn(
    adapter: *mut c_void,
    driver: D3D_DRIVER_TYPE,
    software: HMODULE,
    flags: D3D11_CREATE_DEVICE_FLAG,
    levels: *const D3D_FEATURE_LEVEL,
    level_count: u32,
    version: u32,
    device: *mut *mut c_void,
    level: *mut D3D_FEATURE_LEVEL,
    context: *mut *mut c_void,
) -> HRESULT;

fn create_device(
    driver: D3D_DRIVER_TYPE,
) -> Result<(ID3D11Device, ID3D11DeviceContext), RenderError> {
    let levels = [
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
    ];
    let (mut device, mut context) = (std::ptr::null_mut(), std::ptr::null_mut());
    // SAFETY: the library stays loaded for good, and the function has this signature. The out parameters
    // are valid, the feature levels outlive the call, and a device and context that were made are owned
    // by what's made from them here.
    unsafe {
        // Direct3D is found at this point, and not as the program starts, for this to happen on the thread
        // that's making a renderer. Otherwise it would all be loaded before the window could be opened,
        // which is what making it on another thread is there to avoid.
        let library = LoadLibraryW(w!("d3d11.dll"))?;
        let create = GetProcAddress(library, s!("D3D11CreateDevice"))
            .ok_or_else(windows::core::Error::from_thread)?;
        let create =
            std::mem::transmute::<unsafe extern "system" fn() -> isize, CreateDevice>(create);
        create(
            std::ptr::null_mut(),
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_SINGLETHREADED
                | D3D11_CREATE_DEVICE_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS,
            levels.as_ptr(),
            levels.len() as u32,
            D3D11_SDK_VERSION,
            &raw mut device,
            std::ptr::null_mut(),
            &raw mut context,
        )
        .ok()?;
        Ok((
            ID3D11Device::from_raw(device),
            ID3D11DeviceContext::from_raw(context),
        ))
    }
}

/// A device and its context, made on one thread for another to use, which the single-threaded device allows as long as only
/// one thread uses them at a time.
struct MadeDevice(Result<(ID3D11Device, ID3D11DeviceContext), RenderError>);

// SAFETY: what's in it is only ever used by whichever thread takes it, one at a time.
unsafe impl Send for MadeDevice {}

/// The GPU device that's being made ahead of the first window, if one is.
static EARLY_DEVICE: std::sync::Mutex<Option<std::thread::JoinHandle<MadeDevice>>> =
    std::sync::Mutex::new(None);

/// Starts making what draws to a window on the system's GPU, on another thread, for the first [`Renderer::for_window`] to
/// use. Loading a graphics driver takes a while, often longer than everything else a program does as it starts, and this
/// lets it happen at the same time, rather than after the window's open. A program that will soon open a window calls
/// this as early as it can.
pub fn start_gpu_early() {
    let Ok(mut early) = EARLY_DEVICE.lock() else {
        return;
    };
    if early.is_none() {
        *early = std::thread::Builder::new()
            .name("rsui gpu".to_owned())
            .spawn(|| MadeDevice(create_device(D3D_DRIVER_TYPE_HARDWARE)))
            .ok();
        // What shows a window's swap chain takes most of a millisecond to make, and doesn't need the device, so it's made
        // beside it rather than after it.
        if let Ok(mut composition) = EARLY_COMPOSITION.lock() {
            *composition = std::thread::Builder::new()
                .name("rsui composition".to_owned())
                .spawn(|| MadeComposition(new_composition_device()))
                .ok();
        }
    }
}

/// DirectComposition's device, made on one thread for another to use.
struct MadeComposition(Option<IDCompositionDesktopDevice>);

// SAFETY: DirectComposition's objects can be used from any thread, and this one is only used by whichever takes it.
unsafe impl Send for MadeComposition {}

/// DirectComposition's device that's being made ahead of the first window, if one is.
static EARLY_COMPOSITION: std::sync::Mutex<
    Option<std::thread::JoinHandle<MadeComposition>>,
> = std::sync::Mutex::new(None);

/// A DirectComposition device: the one [`start_gpu_early`] made, if it did and it worked, or a new one.
fn composition_device() -> Option<IDCompositionDesktopDevice> {
    let early = EARLY_COMPOSITION
        .lock()
        .ok()
        .and_then(|mut early| early.take());
    if let Some(Ok(MadeComposition(Some(made)))) = early.map(std::thread::JoinHandle::join) {
        return Some(made);
    }
    new_composition_device()
}

/// A new DirectComposition device. It isn't given the GPU device, which only drawing into DirectComposition's own surfaces
/// would need, and a swap chain from any device can be shown with it.
fn new_composition_device() -> Option<IDCompositionDesktopDevice> {
    let create = create_composition_device()?;
    // SAFETY: what's made is only used if the call succeeded, and it's of the interface that was asked for.
    unsafe {
        let mut made = std::ptr::null_mut();
        create(
            std::ptr::null_mut(),
            &IDCompositionDesktopDevice::IID,
            &mut made,
        )
        .ok()
        .ok()?;
        Some(IDCompositionDesktopDevice::from_raw(made))
    }
}

/// A device on the system's GPU: the one [`start_gpu_early`] made, if it did and it worked, or a new one.
fn hardware_device() -> Result<(ID3D11Device, ID3D11DeviceContext), RenderError> {
    let early = EARLY_DEVICE.lock().ok().and_then(|mut early| early.take());
    if let Some(Ok(MadeDevice(Ok(made)))) = early.map(std::thread::JoinHandle::join) {
        return Ok(made);
    }
    create_device(D3D_DRIVER_TYPE_HARDWARE)
}

impl Renderer {
    /// Draws to a window, on the system's GPU. `size` is in physical pixels.
    pub fn for_window(window: HWND, size: (u32, u32)) -> Result<Self, RenderError> {
        Self::for_window_seen_through(window, size, false)
    }

    /// Like [`for_window`](Self::for_window), for a window that's seen through where nothing's drawn, as one with a
    /// backdrop is, if `see_through` says so. See [`set_see_through`](Self::set_see_through).
    pub fn for_window_seen_through(
        window: HWND,
        size: (u32, u32),
        see_through: bool,
    ) -> Result<Self, RenderError> {
        let (device, context) = hardware_device()?;
        let size = (size.0.max(1), size.1.max(1));
        // One frame in the queue, rather than three, so what's on screen is what's happening
        // now. Measured to make no difference to memory.
        if let Ok(latency) = device.cast::<IDXGIDevice1>() {
            // SAFETY: this only sets a number on a valid device.
            unsafe {
                let _ = latency.SetMaximumFrameLatency(1);
            }
        }
        let (chain, composition) = match composition_chain(&device, size, see_through)
            .and_then(|chain| Ok((show_through_composition(window)?, chain)))
        {
            Ok((composition, chain)) => (chain, Some(composition)),
            // Without DirectComposition, the window has a swap chain of its own, which is opaque, as it'd be without a
            // backdrop.
            Err(_) => (window_chain(&device, window, size)?, None),
        };
        let view = Some(window_view(&device, &chain)?);
        let mut renderer = Self::finish(device, context, Target::Window(chain), view, size)?;
        renderer.window = window;
        renderer.see_through = see_through && composition.is_some();
        renderer.composition = composition;
        Ok(renderer)
    }

    /// Whether this draws to a window that's seen through where nothing's drawn.
    pub fn is_see_through(&self) -> bool {
        self.see_through
    }

    /// Has the window this draws to seen through where nothing's drawn over it, or not, for the system's backdrop to show
    /// there. What's drawn with the clear color [`Color::TRANSPARENT`] is then see-through, and anything drawn over it is
    /// as opaque as its color says.
    ///
    /// The swap chain DirectComposition shows is swapped for one with alpha, or one without, and the old one stays on
    /// screen until the new one's had a frame. The device and everything on it stay, and only the frame after this is drawn
    /// in full. It does nothing to an offscreen renderer, whose pixels have alpha anyway, or to a window that has a swap
    /// chain of its own, as DirectComposition couldn't be had, which stays opaque.
    ///
    /// If this fails, the renderer can't be drawn with, and it says the device was lost, so that a new one is made.
    pub fn set_see_through(&mut self, on: bool) -> Result<(), RenderError> {
        if !matches!(self.target, Target::Window(_))
            || self.composition.is_none()
            || on == self.see_through
        {
            return Ok(());
        }
        self.switch_see_through(on)
            .map_err(|_| RenderError::DeviceLost)
    }

    fn switch_see_through(&mut self, on: bool) -> Result<(), RenderError> {
        self.view = None;
        self.known = false;
        self.frames_since_compact = 0;
        // SAFETY: nothing refers to the old buffers once the view's gone and the state's cleared.
        unsafe {
            self.context.ClearState();
            self.context.Flush();
        }
        let (Target::Window(chain), Some(composition)) = (&mut self.target, &mut self.composition)
        else {
            return Ok(());
        };
        // The visual holds on to the old one, and shows it, until the new one's first frame.
        *chain = composition_chain(&self.device, self.size, on)?;
        composition.showing = false;
        self.see_through = on;
        self.view = Some(window_view(&self.device, chain)?);
        Ok(())
    }

    /// Draws to memory on a software GPU, with no window. Read the result with
    /// [`read_pixels`](Self::read_pixels). This works on any machine, and gives the same
    /// pixels on each, which is what tests need.
    pub fn offscreen(size: (u32, u32)) -> Result<Self, RenderError> {
        let (device, context) = create_device(D3D_DRIVER_TYPE_WARP)?;
        let size = (size.0.max(1), size.1.max(1));
        let (target, view) = offscreen_target(&device, size, false)?;
        Self::finish(device, context, target, Some(view), size)
    }

    /// Like [`offscreen`](Self::offscreen), on the system's GPU, as a window draws, for tests that compare the two.
    #[doc(hidden)]
    pub fn offscreen_on_hardware(size: (u32, u32)) -> Result<Self, RenderError> {
        let (device, context) = create_device(D3D_DRIVER_TYPE_HARDWARE)?;
        let size = (size.0.max(1), size.1.max(1));
        let (target, view) = offscreen_target(&device, size, false)?;
        Self::finish(device, context, target, Some(view), size)
    }

    /// Like [`offscreen`](Self::offscreen), swapping between two buffers as a window does, which
    /// is what drawing only part of a frame has to get right.
    pub fn offscreen_flipping(size: (u32, u32)) -> Result<Self, RenderError> {
        let (device, context) = create_device(D3D_DRIVER_TYPE_WARP)?;
        let size = (size.0.max(1), size.1.max(1));
        let (target, view) = offscreen_target(&device, size, true)?;
        Self::finish(device, context, target, Some(view), size)
    }

    fn finish(
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        target: Target,
        view: Option<ID3D11RenderTargetView>,
        size: (u32, u32),
    ) -> Result<Self, RenderError> {
        let (mut vertex_shader, mut pixel_shader, mut input_layout) = (None, None, None);
        let (mut blend, mut rasterizer, mut frame_constants) = (None, None, None);
        // SAFETY: each description is valid for the call, and the shaders are the bytes
        // this crate compiled. Outputs are only used if the call succeeded.
        unsafe {
            device.CreateVertexShader(VERTEX_SHADER, None, Some(&mut vertex_shader))?;
            device.CreatePixelShader(PIXEL_SHADER, None, Some(&mut pixel_shader))?;
            let element = |name, format, offset| D3D11_INPUT_ELEMENT_DESC {
                SemanticName: name,
                SemanticIndex: 0,
                Format: format,
                InputSlot: 0,
                AlignedByteOffset: offset,
                InputSlotClass: D3D11_INPUT_PER_INSTANCE_DATA,
                InstanceDataStepRate: 1,
            };
            device.CreateInputLayout(
                &[
                    element(s!("RECT"), DXGI_FORMAT_R32G32B32A32_FLOAT, 0),
                    element(s!("GLYPH"), DXGI_FORMAT_R16G16B16A16_UINT, 16),
                    element(s!("COLOR"), DXGI_FORMAT_R8G8B8A8_UNORM, 24),
                    element(s!("RADIUS"), DXGI_FORMAT_R32_FLOAT, 28),
                ],
                VERTEX_SHADER,
                Some(&mut input_layout),
            )?;

            let mut blend_description = D3D11_BLEND_DESC::default();
            blend_description.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
                BlendEnable: true.into(),
                // Each part of what's behind is covered as much as the shader says that part is, which is the same for each part
                // of everything but text drawn as ClearType draws it.
                SrcBlend: D3D11_BLEND_ONE,
                DestBlend: D3D11_BLEND_INV_SRC1_COLOR,
                BlendOp: D3D11_BLEND_OP_ADD,
                SrcBlendAlpha: D3D11_BLEND_ONE,
                DestBlendAlpha: D3D11_BLEND_INV_SRC1_ALPHA,
                BlendOpAlpha: D3D11_BLEND_OP_ADD,
                RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
            };
            device.CreateBlendState(&blend_description, Some(&mut blend))?;

            device.CreateRasterizerState(
                &D3D11_RASTERIZER_DESC {
                    FillMode: D3D11_FILL_SOLID,
                    CullMode: D3D11_CULL_NONE,
                    ScissorEnable: true.into(),
                    ..Default::default()
                },
                Some(&mut rasterizer),
            )?;

            device.CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: FRAME_CONSTANTS_SIZE,
                    Usage: D3D11_USAGE_DYNAMIC,
                    BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                    CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut frame_constants),
            )?;
        }
        let quads = create_quad_buffer(&device, FIRST_QUAD_CAPACITY)?;
        let context1 = context.cast::<ID3D11DeviceContext1>().ok();
        Ok(Self {
            device,
            context,
            target,
            frame_capture: None,
            staging: Default::default(),
            window: HWND::default(),
            composition: None,
            see_through: false,
            view,
            size,
            vertex_shader: vertex_shader.expect("the call succeeded"),
            pixel_shader: pixel_shader.expect("the call succeeded"),
            input_layout: input_layout.expect("the call succeeded"),
            blend: blend.expect("the call succeeded"),
            rasterizer: rasterizer.expect("the call succeeded"),
            frame_constants: frame_constants.expect("the call succeeded"),
            text_look: [0.0, 0.0],
            quads,
            quad_capacity: FIRST_QUAD_CAPACITY,
            atlas: None,
            sync_interval: 1,
            occluded: false,
            frames_since_compact: 0,
            context1,
            last_quads: Vec::new(),
            last_batches: Vec::new(),
            last_clear: Color::TRANSPARENT,
            #[cfg(feature = "retained-frame")]
            frame_token: 0,
            last_generation: 0,
            #[cfg(feature = "mutation-pilot")]
            owned_stamp: None,
            #[cfg(feature = "mutation-pilot")]
            app_binding: 0,
            #[cfg(feature = "mutation-pilot")]
            owned_pending: false,
            #[cfg(all(any(test, feature = "render-diagnostics"), feature = "mutation-pilot"))]
            fail_present: false,
            known: false,
            previous_change: NO_AREA,
            dirty: None,
            nothing_to_show: false,
            always_redraw: false,
            partial_quads: Vec::new(),
            partial_batches: Vec::new(),
            image_pipeline: None,
            last_transforms: Vec::new(),
            last_filters: Vec::new(),
            last_round_clips: Vec::new(),
            last_layers: Vec::new(),
            layer_target: None,
            layer_sampler: None,
            backdrop_shader: None,
            backdrop_targets: [None, None],
            #[cfg(feature = "render-diagnostics")]
            full_layer_targets: false,
            images: HashMap::new(),
            #[cfg(feature = "render-diagnostics")]
            work: Default::default(),
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Draws a scene, and shows it if this is drawing to a window, waiting for the screen
    /// if need be. What's in the scene is used up, so paint another one for the next frame. Only
    /// what's different from the last frame is drawn, and nothing at all if that's nothing.
    pub fn render(&mut self, scene: &mut Scene, clear: Color) -> Result<(), RenderError> {
        self.draw(scene, clear)?;
        self.present()
    }

    /// Shows what's been drawn, if this is drawing to a window. Normally [`render`](Self::render)
    /// does this. It's separate for measuring the two apart.
    pub fn present(&mut self) -> Result<(), RenderError> {
        #[cfg(feature = "mutation-pilot")]
        if self.owned_stamp.is_some() && !self.owned_pending {
            return Ok(());
        }
        #[cfg(all(any(test, feature = "render-diagnostics"), feature = "mutation-pilot"))]
        let result = if std::mem::take(&mut self.fail_present) {
            Err(RenderError::DeviceLost)
        } else {
            self.try_present()
        };
        #[cfg(not(all(any(test, feature = "render-diagnostics"), feature = "mutation-pilot")))]
        let result = self.try_present();
        #[cfg(feature = "mutation-pilot")]
        {
            self.owned_pending = false;
        }
        if result.is_err() {
            self.known = false;
        }
        result
    }

    fn try_present(&mut self) -> Result<(), RenderError> {
        if self.nothing_to_show {
            return Ok(());
        }
        self.frames_since_compact = self.frames_since_compact.saturating_add(1);
        match &mut self.target {
            Target::Window(chain) => {
                if let Some(capture) = &mut self.frame_capture {
                    let region = capture.region;
                    let source = D3D11_BOX {
                        left: region.left as u32,
                        top: region.top as u32,
                        front: 0,
                        right: region.right as u32,
                        bottom: region.bottom as u32,
                        back: 1,
                    };
                    // SAFETY: the capture region is inside the swap chain and matches the staging texture.
                    unsafe {
                        let back: ID3D11Texture2D = chain.GetBuffer(0)?;
                        self.context.CopySubresourceRegion(&capture.staging, 0, 0, 0, 0, &back, 0, Some(&source));
                    }
                    capture.copied = true;
                }
                let mut rect = self.dirty.map(|area| RECT {
                    left: area.left,
                    top: area.top,
                    right: area.right,
                    bottom: area.bottom,
                });
                // Saying what changed lets the system recomposite only that, which saves as
                // much again as drawing only that does.
                let parameters = DXGI_PRESENT_PARAMETERS {
                    DirtyRectsCount: u32::from(rect.is_some()),
                    pDirtyRects: rect
                        .as_mut()
                        .map_or(std::ptr::null_mut(), std::ptr::from_mut),
                    pScrollRect: std::ptr::null_mut(),
                    pScrollOffset: std::ptr::null_mut(),
                };
                // SAFETY: nothing holds the back buffer, as `draw` lets go of it, and `rect`
                // outlives the call.
                let shown =
                    unsafe { chain.Present1(self.sync_interval, DXGI_PRESENT(0), &parameters) };
                shown.ok()?;
                self.occluded = shown == OCCLUDED;
                if let Some(composition) = &mut self.composition {
                    composition.show(chain)?;
                }
            }
            Target::Offscreen {
                texture,
                spare: Some((other, other_view)),
                ..
            } => {
                std::mem::swap(texture, other);
                if let Some(view) = &mut self.view {
                    std::mem::swap(view, other_view);
                }
            }
            Target::Offscreen { .. } => {}
        }
        Ok(())
    }

    /// Runs a window draw and reads the requested physical pixels in the swap chain's BGRA order.
    #[doc(hidden)]
    pub fn capture_window_region(
        &mut self,
        region: PixelRect,
        draw: impl FnOnce(&mut Self) -> Result<(), RenderError>,
    ) -> Result<Vec<u8>, RenderError> {
        if !matches!(self.target, Target::Window(_))
            || self.frame_capture.is_some()
            || is_empty(region)
            || region.left < 0
            || region.top < 0
            || region.right as u32 > self.size.0
            || region.bottom as u32 > self.size.1
        {
            return Err(windows::core::Error::from(E_INVALIDARG).into());
        }
        let width = (region.right - region.left) as u32;
        let height = (region.bottom - region.top) as u32;
        let staging = self.staging_texture((width, height))?;
        self.frame_capture = Some(FrameCapture {
            region,
            staging,
            copied: false,
        });
        // Even if the scene is unchanged, the next swap chain buffer may have older pixels.
        self.known = false;
        let drawn = draw(self);
        let capture = self.frame_capture.take().expect("the capture was set above");
        drawn?;
        if !capture.copied {
            return Err(windows::core::Error::from(E_FAIL).into());
        }
        self.read_staging(&capture.staging, width as usize, height as usize, true)
    }

    /// Whether the last frame was the same as the one before, so wasn't shown, which also means nothing
    /// waited for the screen.
    pub fn showed_nothing(&self) -> bool {
        self.nothing_to_show
    }

    /// Whether the system said the last frame shown can't be seen, so showing it didn't wait for the screen either.
    pub fn was_occluded(&self) -> bool {
        self.occluded
    }

    /// Draws every frame in full, even if nothing changed, which is for measuring.
    pub fn set_always_redraw(&mut self, always: bool) {
        self.always_redraw = always;
    }

    /// How many frames behind the buffer being drawn to is, once there have been that many.
    fn history(&self) -> u32 {
        match &self.target {
            Target::Window(_) | Target::Offscreen { spare: Some(_), .. } => 2,
            Target::Offscreen { spare: None, .. } => 1,
        }
    }

    /// How many screen refreshes to wait for each time something's shown. Normally 1. Zero
    /// doesn't wait, which is for measuring how fast this can go.
    pub fn set_sync_interval(&mut self, interval: u32) {
        self.sync_interval = interval;
    }

    /// Draws a scene without showing it. Normally [`render`](Self::render) does this.
    pub fn draw(&mut self, scene: &mut Scene, clear: Color) -> Result<(), RenderError> {
        #[cfg(feature = "retained-frame")]
        {
            // Unique across renderers as well as submissions: moving an App to another
            // renderer or drawing unrelated commands can never reuse its old picture.
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            self.frame_token = NEXT
                .fetch_update(
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                    |value| value.checked_add(1),
                )
                .expect("frame token exhausted");
        }
        #[cfg(feature = "mutation-pilot")]
        {
            self.app_binding = 0;
        }
        #[cfg(feature = "mutation-pilot")]
        if self.owned_stamp.take().is_some() {
            self.known = false;
        }

        #[cfg(feature = "render-diagnostics")]
        self.work.set(crate::diagnostics::DrawWork {
            serial: self.work.get().serial.saturating_add(1),
            input: crate::diagnostics::Commands::of(scene),
            ..Default::default()
        });
        let result = self.try_draw::<false>(scene, clear, None, NO_AREA);
        if result.is_err() {
            #[cfg(feature = "render-diagnostics")]
            self.observe(|work| {
                work.outcome = crate::diagnostics::DrawOutcome::Error;
                work.presentation = crate::diagnostics::Presentation::None;
            });
            // Resources or constants may already have changed, even if painting failed.
            // Retrying must not treat the old command list as the current picture.
            self.known = false;
        }
        result
    }

    /// Token for the most recent ordinary draw. Only valid while its renderer
    /// still knows the framebuffer contents; this retains no extra commands.
    #[cfg(feature = "retained-frame")]
    #[doc(hidden)]
    pub fn frame_token(&self) -> u64 {
        self.frame_token
    }

    /// Accept an unchanged App frame without rebuilding or copying its commands.
    /// Resize, compaction, failed draws/presents and explicit redraw requests all
    /// require the ordinary path. There is no new presentation to queue.
    #[cfg(feature = "retained-frame")]
    #[doc(hidden)]
    pub fn reuse_frame(&mut self, token: u64) -> bool {
        if token == 0 || token != self.frame_token || !self.known || self.always_redraw {
            return false;
        }
        self.nothing_to_show = true;
        #[cfg(feature = "render-diagnostics")]
        self.work.set(crate::diagnostics::DrawWork {
            serial: self.work.get().serial.saturating_add(1),
            outcome: crate::diagnostics::DrawOutcome::Unchanged,
            ..Default::default()
        });
        true
    }

    fn try_draw<const RETAIN: bool>(
        &mut self,
        scene: &mut Scene,
        clear: Color,
        owned: Option<(PixelRect, &[(u32, u32)])>,
        pending_damage: PixelRect,
    ) -> Result<(), RenderError> {
        let text_changed = self.text_look != scene.text_look;
        self.text_look = scene.text_look;
        self.prepare_atlas(scene.atlas_size)?;
        self.upload_glyphs(&scene.uploads);
        scene.uploads.clear();
        let images_changed = self.upload_images(scene)?;

        let whole = PixelRect {
            left: 0,
            top: 0,
            right: self.size.0 as i32,
            bottom: self.size.1 as i32,
        };
        let has_backdrop = scene.batches.iter().any(|batch| batch.image == BACKDROP_IMAGE);
        let backdrop_affected = has_backdrop || self.last_batches.iter().any(|batch| batch.image == BACKDROP_IMAGE);
        // What's different from the last frame.
        // What's turned or resized isn't where its quads say, so what's changed is worked out from where their transforms put
        // them, and what's filtered or faded differently from its own quads (`changed_area_placed`).
        let placed = (owned.is_none()
            && !(scene.transforms.is_empty()
                && self.last_transforms.is_empty()
                && scene.filters.is_empty()
                && self.last_filters.is_empty()))
        .then(|| {
            changed_area_placed(
                (
                    &self.last_quads,
                    &self.last_batches,
                    &self.last_transforms,
                    &self.last_filters,
                ),
                (&scene.quads, &scene.batches, &scene.transforms, &scene.filters),
                whole,
            )
        });
        // Whether that couldn't be worked out, or came to everything.
        let transformed = placed == Some(whole);
        // Keep original batch indices for plain, non-nested opacity groups.
        // Their composition can be clipped to damage just like ordinary draws.
        // Structural layer changes and transformed/filtered cases stay on the
        // full-window path until their contributor correspondence is proven.
        let partial_layers = owned.is_none()
            && !scene.layers.is_empty()
            && scene.transforms.is_empty() && self.last_transforms.is_empty()
            && scene.filters.is_empty() && self.last_filters.is_empty()
            && scene.layers.len() == self.last_layers.len()
            && scene.layers.iter().zip(&self.last_layers)
                .all(|(a, b)| a.first == b.first && a.end == b.end)
            && scene.layers.iter().all(|layer| layer.first < layer.end && layer.end as usize <= scene.batches.len())
            && scene.layers.windows(2).all(|pair| pair[0].end <= pair[1].first);
        let layer_damage = if partial_layers {
            scene.layers.iter().zip(&self.last_layers).fold(NO_AREA, |damage, (now, before)| {
                if now == before { damage } else { union(damage, union(now.bounds, before.bounds)) }
            })
        } else { NO_AREA };
        #[cfg(feature = "render-diagnostics")]
        self.observe(|work| {
            work.full_reasons = crate::diagnostics::FullReasons {
                unknown_history: !self.known,
                always_redraw: self.always_redraw,
                no_partial_context: self.context1.is_none(),
                clear_changed: self.last_clear != clear,
                atlas_generation_changed: self.last_generation != scene.atlas_generation,
                text_changed,
                referenced_image_changed: images_changed,
                transformed,
            }
        });
        let change = if !self.known
            || self.always_redraw
            || self.context1.is_none()
            || self.last_clear != clear
            || self.last_generation != scene.atlas_generation
            // Resource-dependent pixels can change without any command changing. Redraw
            // in full: old/new clips and transforms make quad bounds alone unsafe here.
            || text_changed
            || images_changed
            || transformed
            // What's cut off at round corners that moved can look different without its quads or batches changing.
            || scene.round_clips != self.last_round_clips
            || (!partial_layers && scene.layers != self.last_layers)
        {
            whole
        } else if let Some((change, _)) = owned {
            change
        } else if let Some(change) = placed {
            intersect(union(change, pending_damage), whole)
        } else {
            intersect(
                union(
                    changed_area(
                        (&self.last_quads, &self.last_batches),
                        (&scene.quads, &scene.batches),
                        whole,
                    ),
                    pending_damage,
                ),
                whole,
            )
        };
        let change = intersect(union(change, layer_damage), whole);
        // A change outside a blur box can still change what it samples.
        let change = if backdrop_affected && !is_empty(change) { whole } else { change };
        #[cfg(feature = "render-diagnostics")]
        self.observe(|work| work.change = change);
        self.nothing_to_show = is_empty(change) && self.known && !is_empty(whole);
        self.last_clear = clear;
        self.last_generation = scene.atlas_generation;
        self.known = true;
        if self.nothing_to_show {
            #[cfg(feature = "render-diagnostics")]
            self.observe(|work| work.outcome = crate::diagnostics::DrawOutcome::Unchanged);
            if !RETAIN {
                self.keep_for_next_frame(scene);
            }
            return Ok(());
        }
        // What this buffer needs, given what it last showed.
        let region = if self.history() == 2 {
            union(change, self.previous_change)
        } else {
            change
        };
        // What's faded as a whole is drawn whole, as it's put down all at once.
        let (change, region) = if region != whole
            && !partial_layers
            && scene
                .layers
                .iter()
                .any(|layer| !is_empty(intersect(layer.bounds, region)))
        {
            (whole, whole)
        } else {
            (change, region)
        };
        // A contributor plan is exact for one fixed region, never a larger union.
        // Previous-buffer damage can extend outside the local plan. In that
        // case use ordinary ordered contributor selection for this draw only.
        let owned = owned.filter(|(exact, _)| *exact == region);
        #[cfg(feature = "render-diagnostics")]
        self.observe(|work| work.draw_region = region);
        self.previous_change = change;
        self.dirty = (self.frames_since_compact > 0 && change != whole).then_some(change);
        // For a small change, only what touches it goes to the GPU, which saves running the
        // vertex shader for everything else, only for all of it to be cut off.
        self.partial_quads.clear();
        self.partial_batches.clear();
        if region != whole && !partial_layers {
            if let Some((_, indices)) = owned {
                for &(q, b) in indices {
                    let batch = scene.batches[b as usize];
                    let first = self.partial_quads.len() as u32;
                    self.partial_quads.push(scene.quads[q as usize]);
                    if let Some(last) = self.partial_batches.last_mut().filter(|last| {
                        last.clip == intersect(batch.clip, region)
                            && last.image == batch.image
                            && last.transform == batch.transform
                            && last.filter == batch.filter
                            && last.round == batch.round
                    }) {
                        last.count += 1;
                    } else {
                        self.partial_batches.push(Batch {
                            first,
                            count: 1,
                            clip: intersect(batch.clip, region),
                            ..batch
                        });
                    }
                }
            } else {
                for batch in &scene.batches {
                    let clip = intersect(batch.clip, region);
                    if is_empty(clip) {
                        continue;
                    }
                    let first = self.partial_quads.len() as u32;
                    let run =
                        &scene.quads[batch.first as usize..(batch.first + batch.count) as usize];
                    #[cfg(feature = "render-diagnostics")]
                    self.observe(|work| {
                        work.partial_quads_tested =
                            work.partial_quads_tested.saturating_add(run.len() as u64)
                    });
                    // What's turned is where its transform puts it. One whose transform isn't there is drawn whole.
                    let matrix = match batch.transform {
                        0 => Some(crate::transform::IDENTITY),
                        at => scene.transforms.get(usize::from(at) - 1).copied(),
                    };
                    self.partial_quads.extend(run.iter().filter(|quad| {
                        matrix.is_none_or(|matrix| !is_empty(intersect(placed_area(quad, matrix), clip)))
                    }));
                    let count = self.partial_quads.len() as u32 - first;
                    if count > 0 {
                        self.partial_batches.push(Batch {
                            first,
                            count,
                            clip,
                            image: batch.image,
                            transform: batch.transform,
                            filter: batch.filter,
                            round: batch.round,
                        });
                    }
                }
            }
            let quads = std::mem::take(&mut self.partial_quads);
            let uploaded = self.upload_quads(&quads);
            self.partial_quads = quads;
            uploaded?;
        } else {
            if !scene.layers.is_empty() {
                self.make_image_pipeline()?;
                self.make_layer_target(self.needed_layer_size(&scene.layers))?;
            }
            let layer_size = self.layer_target.as_ref().map_or(self.size, |(size, _, _)| *size);
            let layered: Vec<Quad> = scene.layers.iter().map(|layer| layer_quad(layer, layer_size, self.layer_origin(layer))).collect();
            self.upload_quads_and(&scene.quads, &layered)?;
        }

        let view = self
            .view
            .clone()
            .expect("a view exists outside of a resize");
        let [r, g, b, a] = clear.premultiplied().map(|part| f32::from(part) / 255.0);
        if !scene.transforms.is_empty() {
            self.make_image_pipeline()?;
        }
        if has_backdrop {
            self.make_image_pipeline()?;
            self.prepare_backdrop(0, self.size)?;
            if let Some((size, _, _)) = &self.layer_target {
                self.prepare_backdrop(1, *size)?;
            }
        }
        // SAFETY: everything bound here stays alive for the draw calls, as `self` owns it.
        unsafe {
            self.write_frame_constants()?;
            let context = &self.context;
            match &self.context1 {
                Some(context1) if region != whole => context1.ClearView(
                    &view,
                    &[r, g, b, a],
                    Some(&[RECT {
                        left: region.left,
                        top: region.top,
                        right: region.right,
                        bottom: region.bottom,
                    }]),
                ),
                _ => context.ClearRenderTargetView(&view, &[r, g, b, a]),
            }
            context.OMSetRenderTargets(Some(&[Some(view.clone())]), None);
            context.OMSetBlendState(&self.blend, None, u32::MAX);
            context.RSSetState(&self.rasterizer);
            context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: self.size.0 as f32,
                Height: self.size.1 as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            context.IASetInputLayout(&self.input_layout);
            let stride = size_of::<Quad>() as u32;
            let offset = 0u32;
            let buffer = Some(self.quads.clone());
            context.IASetVertexBuffers(0, 1, Some(&raw const buffer), Some(&stride), Some(&offset));
            context.VSSetShader(&self.vertex_shader, None);
            context.VSSetConstantBuffers(0, Some(&[Some(self.frame_constants.clone())]));
            context.PSSetConstantBuffers(0, Some(&[Some(self.frame_constants.clone())]));
            context.PSSetShader(&self.pixel_shader, None);
            let atlas = self.atlas.as_ref().map(|atlas| atlas.view.clone());
            context.PSSetShaderResources(0, Some(&[atlas]));
            let batches = if region != whole && !partial_layers {
                &self.partial_batches
            } else {
                &scene.batches
            };
            // Most frames never leave the glyph shader, so changing shaders only happens around images.
            let mut drawing_image = false;
            let mut transform = 0;
            let mut filter = 0;
            let mut round = 0;
            if !scene.transforms.is_empty() {
                // Reading glyphs smoothly takes this, and it's only set for images otherwise.
                if let Some((_, sampler)) = &self.image_pipeline {
                    context.PSSetSamplers(0, Some(&[Some(sampler.clone())]));
                }
            }
            // What's faded as a whole: the next to start, and the one being drawn, by itself, if any.
            let layers: &[Layer] = if region == whole || partial_layers { &scene.layers } else { &[] };
            let mut next_layer = 0;
            let mut in_layer: Option<usize> = None;
            for (index, batch) in batches.iter().enumerate() {
                let index = index as u32;
                if let Some(at) = in_layer.filter(|&at| layers[at].end <= index) {
                    in_layer = None;
                    self.put_down_layer(&layers[at], scene.quads.len() + at, &view, scene.atlas_size, region)?;
                    (transform, filter, round, drawing_image) = (u16::MAX, u16::MAX, u16::MAX, true);
                }
                if in_layer.is_none() && layers.get(next_layer).is_some_and(|layer| layer.first == index) {
                    self.start_layer(&layers[next_layer]);
                    in_layer = Some(next_layer);
                    next_layer += 1;
                }
                let clip = intersect(batch.clip, region);
                if is_empty(clip) {
                    continue;
                }
                if batch.transform != transform || batch.filter != filter || batch.round != round {
                    transform = batch.transform;
                    filter = batch.filter;
                    round = batch.round;
                    self.write_constants(
                        transform
                            .checked_sub(1)
                            .and_then(|at| scene.transforms.get(usize::from(at)))
                            .copied(),
                        filter
                            .checked_sub(1)
                            .and_then(|at| scene.filters.get(usize::from(at))),
                        round_clips_of(batch, &scene.round_clips),
                        scene.atlas_size,
                    )?;
                }
                if batch.image == BACKDROP_IMAGE {
                    let matrix = batch.transform.checked_sub(1).and_then(|at| scene.transforms.get(usize::from(at))).copied();
                    let origin = in_layer.map_or((0, 0), |at| self.layer_origin(&layers[at]));
                    let (slot, source) = if in_layer.is_some() {
                        (1, &self.layer_target.as_ref().expect("drawing on a layer").1)
                    } else {
                        (0, &view)
                    };
                    if let Some(quad) = scene.quads.get(batch.first as usize) {
                        self.draw_backdrop(slot, source, origin, clip, quad, matrix, batch.first)?;
                        // This was a different pixel shader from either of the normal drawing shaders.
                        context.PSSetShader(&self.pixel_shader, None);
                        drawing_image = false;
                    }
                    continue;
                } else if batch.image != 0 {
                    let (Some((shader, sampler)), Some(picture)) =
                        (&self.image_pipeline, self.images.get(&batch.image))
                    else {
                        continue;
                    };
                    if !drawing_image {
                        context.PSSetShader(shader, None);
                        context.PSSetSamplers(0, Some(&[Some(sampler.clone())]));
                        drawing_image = true;
                    }
                    context.PSSetShaderResources(1, Some(&[Some(picture.clone())]));
                } else if drawing_image {
                    context.PSSetShader(&self.pixel_shader, None);
                    drawing_image = false;
                }
                let (dx, dy) = in_layer.map_or((0, 0), |at| self.layer_origin(&layers[at]));
                context.RSSetScissorRects(Some(&[RECT {
                    left: clip.left - dx,
                    top: clip.top - dy,
                    right: clip.right - dx,
                    bottom: clip.bottom - dy,
                }]));
                context.DrawInstanced(4, batch.count, 0, batch.first);
                #[cfg(feature = "render-diagnostics")]
                self.observe(|work| {
                    work.draw_calls = work.draw_calls.saturating_add(1);
                    work.submitted_quads =
                        work.submitted_quads.saturating_add(u64::from(batch.count));
                });
            }
            if let Some(at) = in_layer {
                self.put_down_layer(&layers[at], scene.quads.len() + at, &view, scene.atlas_size, region)?;
            }
            // Nothing may hold the back buffer when it's presented or resized.
            let context = &self.context;
            context.OMSetRenderTargets(None, None);
        }
        #[cfg(feature = "render-diagnostics")]
        self.observe(|work| {
            work.outcome = if region == whole {
                crate::diagnostics::DrawOutcome::Full
            } else {
                crate::diagnostics::DrawOutcome::Partial
            };
            work.presentation = self.dirty.map_or(
                crate::diagnostics::Presentation::Full,
                crate::diagnostics::Presentation::Dirty,
            );
        });
        if !RETAIN {
            self.keep_for_next_frame(scene);
        }
        Ok(())
    }

    #[cfg(all(feature = "render-diagnostics", feature = "mutation-pilot"))]
    #[doc(hidden)]
    pub fn fail_next_present_for_diagnostics(&mut self) {
        self.fail_present = true;
    }

    /// Bind the two independently owned objects without pointer identity. Any change
    /// on either side requires App resource replay before submission.
    #[cfg(feature = "mutation-pilot")]
    #[doc(hidden)]
    pub fn bind_automatic_app(&mut self, binding: &mut u64) -> bool {
        if *binding != 0 && *binding == self.app_binding {
            return true;
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let next = NEXT
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |v| v.checked_add(1),
            )
            .expect("binding exhausted");
        self.app_binding = next;
        *binding = next;
        self.known = false;
        false
    }

    /// Private-protocol pilot entry. Authoritative commands remain in Painter.
    /// Small WRITE_DISCARD uploads contain the ordered contributors for proven damage.
    #[cfg(feature = "mutation-pilot")]
    #[doc(hidden)]
    pub fn draw_owned_pilot(&mut self, painter: &mut crate::Painter) -> Result<(), RenderError> {
        #[cfg(feature = "retained-frame")]
        {
            self.frame_token = 0;
        }
        if self.owned_stamp.is_none() {
            // Crossing from the public renderer discards its unrelated history.
            // Do not retain a third full command capacity alongside Painter's pair.
            self.last_quads = Vec::new();
            self.last_batches = Vec::new();
            self.last_transforms = Vec::new();
            self.last_filters = Vec::new();
            self.last_round_clips = Vec::new();
        }
        let stamp = (painter.pilot.epoch, painter.pilot.revision);
        let same_owner = self
            .owned_stamp
            .is_some_and(|(epoch, _)| epoch == stamp.0 && epoch != 0);
        let mut pending_damage = NO_AREA;
        let comparing = painter.pilot.pending.is_some();
        if let Some((base, region)) = painter.pilot.pending {
            if self
                .owned_stamp
                .is_none_or(|last| last.0 != base.0 || base.0 == 0)
            {
                self.known = false;
            } else if self.owned_stamp != Some(base) {
                // The snapshot includes unsubmitted local patches. Their entire
                // region must be repainted even when old/new comparison is empty.
                pending_damage = region;
            }
            self.keep_for_next_frame(&mut painter.pilot.history);
        } else if !same_owner {
            self.known = false;
        }
        if self.owned_pending {
            // A draw without Present has not advanced the target's buffer history.
            self.known = false;
        }
        let plan = (!comparing && same_owner).then(|| {
            (
                if self.owned_stamp == Some(stamp) {
                    NO_AREA
                } else {
                    painter.pilot.region
                },
                painter.pilot.contributors.as_slice(),
            )
        });
        let scene = &mut painter.scene;
        #[cfg(feature = "render-diagnostics")]
        self.work.set(crate::diagnostics::DrawWork {
            serial: self.work.get().serial.saturating_add(1),
            input: crate::diagnostics::Commands::of(scene),
            ..Default::default()
        });
        let clear = scene.window_color.unwrap_or(if self.see_through {
            Color::TRANSPARENT
        } else {
            Color::hex(0xF6F8FA)
        });
        let result = self.try_draw::<true>(scene, clear, plan, pending_damage);
        if comparing {
            self.keep_for_next_frame(&mut painter.pilot.history);
        }
        if result.is_ok() {
            painter.pilot.pending = None;
            // Keep at most one spare command capacity. Bound slack after shrink;
            // never allocate a history on a local-only run.
            fn spare<T>(v: &mut Vec<T>, current: usize) {
                v.clear();
                let target = current.saturating_mul(2).max(64);
                if v.capacity() > target.saturating_mul(2) {
                    v.shrink_to(target);
                }
            }
            spare(&mut painter.pilot.history.quads, scene.quads.len());
            spare(&mut painter.pilot.history.batches, scene.batches.len());
            spare(
                &mut painter.pilot.history.transforms,
                scene.transforms.len(),
            );
            spare(&mut painter.pilot.history.filters, scene.filters.len());
            self.owned_stamp = Some(stamp);
            self.owned_pending = !self.nothing_to_show;
        } else {
            self.known = false;
            self.owned_pending = false;
            #[cfg(feature = "render-diagnostics")]
            self.observe(|work| {
                work.outcome = crate::diagnostics::DrawOutcome::Error;
                work.presentation = crate::diagnostics::Presentation::None;
            });
        }
        result
    }

    /// Keeps this frame's list to compare the next one with. Swapping saves copying it, and leaves
    /// the old list's memory in the scene for the next frame to fill.
    fn keep_for_next_frame(&mut self, scene: &mut Scene) {
        std::mem::swap(&mut self.last_quads, &mut scene.quads);
        std::mem::swap(&mut self.last_batches, &mut scene.batches);
        std::mem::swap(&mut self.last_transforms, &mut scene.transforms);
        std::mem::swap(&mut self.last_filters, &mut scene.filters);
        std::mem::swap(&mut self.last_round_clips, &mut scene.round_clips);
        std::mem::swap(&mut self.last_layers, &mut scene.layers);
    }

    /// Gives back most of the GPU memory a window uses, for when it isn't showing. Call
    /// [`resize`](Self::resize) and draw again to bring it back.
    pub fn hide(&mut self) -> Result<(), RenderError> {
        self.resize((1, 1))?;
        if let Target::Window(chain) = &self.target {
            // The old buffers stay in use until something newer has been presented.
            // SAFETY: the swap chain and its new buffers are valid after the resize.
            unsafe {
                chain.Present(0, DXGI_PRESENT(0)).ok()?;
            }
        }
        self.trim();
        Ok(())
    }

    /// Draws a scene without showing it, and returns the pixels, as blue, green, red and an unused
    /// byte for each, in rows from the top. For a window that's seen through, that byte is how opaque
    /// the pixel is, and the colors are multiplied by it. This is for keeping the picture on screen some other
    /// way, once the GPU is being let go of. Only works on a window.
    pub fn draw_to_memory(
        &mut self,
        scene: &mut Scene,
        clear: Color,
    ) -> Result<Vec<u8>, RenderError> {
        self.known = false;
        self.frames_since_compact = 0;
        self.draw(scene, clear)?;
        let Target::Window(chain) = &self.target else {
            return Ok(Vec::new());
        };
        let staging = self.staging_texture(self.size)?;
        // SAFETY: the swap chain and staging texture have the renderer size and BGRA format.
        unsafe {
            let back: ID3D11Texture2D = chain.GetBuffer(0)?;
            self.context.CopyResource(&staging, &back);
        }
        self.read_staging(&staging, self.size.0 as usize, self.size.1 as usize, true)
    }

    /// Lets go of the spare back buffer, for when things have gone quiet, and says whether it
    /// did. A window only needs a second buffer while it's changing, and the system makes that
    /// again when it's next needed, in about a third of a millisecond. If this returns `true`,
    /// draw again, as what's on screen stays there only until the next frame replaces it.
    pub fn compact(&mut self) -> Result<bool, RenderError> {
        if self.frames_since_compact < 2 || !matches!(self.target, Target::Window(_)) {
            return Ok(false);
        }
        self.frames_since_compact = 0;
        self.make_buffers(self.size)?;
        Ok(true)
    }

    /// Changes the size of what's drawn to, in physical pixels. Call this when the window
    /// changes size.
    pub fn resize(&mut self, size: (u32, u32)) -> Result<(), RenderError> {
        let size = (size.0.max(1), size.1.max(1));
        if size == self.size {
            return Ok(());
        }
        self.frames_since_compact = 0;
        self.make_buffers(size)
    }

    fn make_buffers(&mut self, size: (u32, u32)) -> Result<(), RenderError> {
        self.view = None;
        self.layer_target = None;
        self.backdrop_targets = [None, None];
        self.known = false;
        self.frames_since_compact = 0;
        // SAFETY: nothing refers to the old buffers, as the view was just released and
        // `render` unbinds it after every frame.
        unsafe {
            self.context.ClearState();
            self.context.Flush();
        }
        match &self.target {
            Target::Window(chain) => {
                // SAFETY: see above.
                unsafe {
                    chain.ResizeBuffers(
                        0,
                        size.0,
                        size.1,
                        DXGI_FORMAT_UNKNOWN,
                        DXGI_SWAP_CHAIN_FLAG(0),
                    )?;
                }
                self.view = Some(window_view(&self.device, chain)?);
            }
            Target::Offscreen { spare, .. } => {
                let flipping = spare.is_some();
                let (target, view) = offscreen_target(&self.device, size, flipping)?;
                self.target = target;
                self.view = Some(view);
            }
        }
        self.size = size;
        Ok(())
    }

    /// Gives back memory that isn't in use. Call this once nothing has changed for a while,
    /// not during animation, since it blocks for about a frame and the next frame runs a
    /// little slower.
    pub fn trim(&mut self) {
        self.trim_storage();
        // SAFETY: these calls have no preconditions beyond valid objects.
        unsafe {
            self.context.Flush();
            if let Ok(device) = self.device.cast::<IDXGIDevice3>() {
                device.Trim();
            }
        }
        give_back_memory();
    }

    /// Release retired images without drawing or presenting. A later draw can
    /// upload a replacement with the same ID through the ordinary upload path.
    pub fn release_images(&mut self, dropped: &mut Vec<u32>) {
        let mut removed = false;
        for id in dropped.drain(..) {
            if self.images.remove(&id).is_some() {
                removed = true;
                if self
                    .last_batches
                    .iter()
                    .any(|batch| batch.image == id && batch.count != 0)
                {
                    self.known = false;
                }
            }
        }
        if removed {
            // SAFETY: slot 1 holds the image; the context owns its own reference.
            // Every subsequent draw binds the resources it uses again.
            unsafe {
                self.context.PSSetShaderResources(1, Some(&[None]));
            }
        }
    }

    /// Release idle CPU history and oversized quad storage without OS working-set
    /// trimming. Live images and the font atlas stay ready for the next frame.
    pub fn trim_storage(&mut self) {
        self.staging.get_mut().take();
        // This temporary surface is reconstructible and never contains the
        // authoritative window image. The context is unbound below as well.
        self.layer_target = None;
        self.backdrop_targets = [None, None];
        let needed = (self.last_quads.len().min(u32::MAX as usize) as u32)
            .max(FIRST_QUAD_CAPACITY)
            .checked_next_power_of_two()
            .unwrap_or(self.quad_capacity);
        if self.quad_capacity / 4 > needed {
            // Allocation failure leaves the current buffer intact and usable.
            if let Ok(buffer) = create_quad_buffer(&self.device, needed) {
                self.quads = buffer;
                self.quad_capacity = needed;
            }
        }
        // SAFETY: no draw is in progress; every draw installs its full pipeline.
        // Unbind old buffers as the context owns references independently of us.
        unsafe {
            self.context.ClearState();
        }
        self.known = false;
        self.last_quads = Vec::new();
        self.last_batches = Vec::new();
        self.last_transforms = Vec::new();
        self.last_filters = Vec::new();
        self.last_round_clips = Vec::new();
        self.partial_quads = Vec::new();
        self.partial_batches = Vec::new();
        let target = self.images.len().saturating_mul(2).max(16);
        if self.images.capacity() > target.saturating_mul(2) {
            self.images.shrink_to(target);
        }
    }

    /// The pixels drawn so far, as red, green, blue and alpha for each, in rows from the
    /// top. Only offscreen renderers have this.
    pub fn read_pixels(&self) -> Result<Vec<u8>, RenderError> {
        self.read_offscreen(false)
    }

    /// Offscreen pixels in the texture's native blue, green, red, alpha order.
    #[doc(hidden)]
    pub fn read_pixels_bgra(&self) -> Result<Vec<u8>, RenderError> {
        self.read_offscreen(true)
    }

    fn read_offscreen(&self, bgra: bool) -> Result<Vec<u8>, RenderError> {
        let region = PixelRect {
            left: 0,
            top: 0,
            right: self.size.0 as i32,
            bottom: self.size.1 as i32,
        };
        self.read_offscreen_region(region, bgra)
    }

    /// Reads just the requested physical pixels from the most recently shown buffer.
    #[doc(hidden)]
    pub fn read_offscreen_region(
        &self,
        region: PixelRect,
        bgra: bool,
    ) -> Result<Vec<u8>, RenderError> {
        let Some(staging) = self.copy_offscreen_region(region)? else {
            return Ok(Vec::new());
        };
        self.read_staging(
            &staging,
            (region.right - region.left) as usize,
            (region.bottom - region.top) as usize,
            bgra,
        )
    }

    /// Blocks until the frame is complete. Only four bytes cross the GPU readback boundary.
    #[doc(hidden)]
    pub fn wait_for_frame(&self) -> Result<(), RenderError> {
        let Some(staging) = self.copy_offscreen_region(PixelRect {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        })?
        else {
            return Ok(());
        };
        // SAFETY: this staging texture supports CPU reads; Map synchronizes the preceding copy.
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            self.context.Unmap(&staging, 0);
        }
        Ok(())
    }

    fn copy_offscreen_region(
        &self,
        region: PixelRect,
    ) -> Result<Option<ID3D11Texture2D>, RenderError> {
        let Target::Offscreen { texture, spare } = &self.target else {
            return Ok(None);
        };
        if is_empty(region)
            || region.left < 0
            || region.top < 0
            || region.right as u32 > self.size.0
            || region.bottom as u32 > self.size.1
        {
            return Err(windows::core::Error::from(E_INVALIDARG).into());
        }
        let staging = self.staging_texture((
            (region.right - region.left) as u32,
            (region.bottom - region.top) as u32,
        ))?;
        let source = D3D11_BOX {
            left: region.left as u32,
            top: region.top as u32,
            front: 0,
            right: region.right as u32,
            bottom: region.bottom as u32,
            back: 1,
        };
        // After swapping, the spare buffer is the one that was just shown.
        let texture = spare.as_ref().map_or(texture, |(shown, _)| shown);
        // SAFETY: the validated box fits the source and matches the staging texture dimensions and format.
        unsafe {
            self.context
                .CopySubresourceRegion(&staging, 0, 0, 0, 0, texture, 0, Some(&source))
        };
        Ok(Some(staging))
    }

    fn staging_texture(&self, size: (u32, u32)) -> Result<ID3D11Texture2D, RenderError> {
        let mut cached = self.staging.borrow_mut();
        if let Some((_, texture)) = cached.as_ref().filter(|(was, _)| *was == size) {
            return Ok(texture.clone());
        }
        let (width, height) = size;
        let mut staging = None;
        // SAFETY: the texture description is valid and the output slot is writable.
        unsafe {
            self.device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: width,
                    Height: height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: FORMAT,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_STAGING,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut staging),
            )?;
        }
        let staging = staging.expect("the call succeeded");
        *cached = Some((size, staging.clone()));
        Ok(staging)
    }

    fn read_staging(
        &self,
        readback: &ID3D11Texture2D,
        width: usize,
        height: usize,
        bgra: bool,
    ) -> Result<Vec<u8>, RenderError> {
        let mut pixels = vec![0u8; width * height * 4];
        // SAFETY: the mapped memory is at least `RowPitch` by `height` bytes until `Unmap`,
        // and the copy stays within both it and `pixels`.
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(readback, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            for (row, target) in pixels.chunks_exact_mut(width * 4).enumerate() {
                let source = std::slice::from_raw_parts(
                    mapped
                        .pData
                        .cast::<u8>()
                        .add(row * mapped.RowPitch as usize),
                    width * 4,
                );
                if bgra {
                    target.copy_from_slice(source);
                } else {
                    for (to, from) in target.chunks_exact_mut(4).zip(source.chunks_exact(4)) {
                        to.copy_from_slice(&[from[2], from[1], from[0], from[3]]);
                    }
                }
            }
            self.context.Unmap(readback, 0);
        }
        Ok(pixels)
    }

    /// Makes sure the atlas texture is the size the scene needs, keeping what's in it if
    /// it only grew. Glyphs stay where they are when an atlas grows.
    fn prepare_atlas(&mut self, size: u32) -> Result<(), RenderError> {
        let size = size.max(1);
        if self.atlas.as_ref().is_some_and(|atlas| atlas.size == size) {
            return Ok(());
        }
        let mut texture = None;
        let mut view = None;
        // SAFETY: the descriptions are valid, and a copy stays inside the new texture,
        // since it only happens when that's the larger of the two.
        unsafe {
            self.device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: size,
                    Height: size,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_R8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut texture),
            )?;
            #[cfg(feature = "render-diagnostics")]
            self.observe(|work| work.texture_creations = work.texture_creations.saturating_add(1));
            let texture: ID3D11Texture2D = texture.expect("the call succeeded");
            self.device
                .CreateShaderResourceView(&texture, None, Some(&mut view))?;
            #[cfg(feature = "render-diagnostics")]
            self.observe(|work| work.view_creations = work.view_creations.saturating_add(1));
            if let Some(old) = &self.atlas {
                if old.size < size {
                    self.context
                        .CopySubresourceRegion(&texture, 0, 0, 0, 0, &old.texture, 0, None);
                    #[cfg(feature = "render-diagnostics")]
                    self.observe(|work| {
                        work.atlas_copy
                            .add(u64::from(old.size).saturating_mul(u64::from(old.size)))
                    });
                }
            }
            self.atlas = Some(AtlasTexture {
                size,
                texture,
                view: view.expect("the call succeeded"),
            });
        }
        Ok(())
    }

    /// Puts new images on the GPU and lets go of old ones. Reports successful changes
    /// referenced by both command lists, before those lists are swapped. Changed IDs
    /// and removed/added commands already contribute their bounds in changed_area.
    fn upload_images(&mut self, scene: &mut Scene) -> Result<bool, RenderError> {
        let mut changed = false;
        for id in scene.dropped_images.drain(..) {
            if self.images.remove(&id).is_some() {
                changed = changed || self.image_referenced(&scene.batches, id);
            }
        }
        if scene.image_uploads.is_empty() {
            return Ok(changed);
        }
        self.make_image_pipeline()?;
        for upload in scene.image_uploads.drain(..) {
            let id = upload.id;
            if self.upload_image(upload)? {
                changed = changed || self.image_referenced(&scene.batches, id);
            }
        }
        Ok(changed)
    }

    fn image_referenced(&self, batches: &[Batch], id: u32) -> bool {
        // Only a resource present in both command lists can change pixels without a
        // corresponding command change. New image IDs keep their ordinary local damage.
        // Only scan on actual resource mutations, with no allocation or cached-image scan.
        id != 0
            && self
                .last_batches
                .iter()
                .any(|batch| batch.image == id && batch.count != 0)
            && batches
                .iter()
                .any(|batch| batch.image == id && batch.count != 0)
    }

    /// Makes what images are drawn with, the first time it's needed.
    /// Makes what's faded as a whole drawn on first, if it isn't there, as big as what's drawn to.
    fn layer_origin(&self, layer: &Layer) -> (i32, i32) {
        #[cfg(feature = "render-diagnostics")]
        if self.full_layer_targets { return (0, 0); }
        (layer.bounds.left.clamp(0, self.size.0 as i32), layer.bounds.top.clamp(0, self.size.1 as i32))
    }

    fn needed_layer_size(&self, layers: &[Layer]) -> (u32, u32) {
        #[cfg(feature = "render-diagnostics")]
        if self.full_layer_targets { return self.size; }
        layers.iter().fold((1, 1), |(width, height), layer| {
            let (x, y) = self.layer_origin(layer);
            (width.max((layer.bounds.right.min(self.size.0 as i32) - x).max(1) as u32),
             height.max((layer.bounds.bottom.min(self.size.1 as i32) - y).max(1) as u32))
        })
    }

    /// Independent full-window opacity-surface reference for pixel diagnostics.
    #[cfg(feature = "render-diagnostics")]
    #[doc(hidden)]
    pub fn set_full_layer_targets_for_diagnostics(&mut self, full: bool) {
        self.full_layer_targets = full;
        self.layer_target = None;
        self.known = false;
    }

    fn prepare_backdrop(&mut self, slot: usize, size: (u32, u32)) -> Result<(), RenderError> {
        if self.backdrop_shader.is_none() {
            // SAFETY: this is the shader built from `quad.hlsl` for this device.
            unsafe { self.device.CreatePixelShader(BACKDROP_SHADER, None, Some(&mut self.backdrop_shader))?; }
        }
        if self.backdrop_targets[slot].as_ref().is_some_and(|target| target.size == size) {
            return Ok(());
        }
        let mut texture = None;
        let mut picture = None;
        // SAFETY: each description is valid and the texture remains alive through its view.
        unsafe {
            self.device.CreateTexture2D(&D3D11_TEXTURE2D_DESC {
                Width: size.0, Height: size.1, MipLevels: 1, ArraySize: 1,
                Format: FORMAT, SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT, BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                ..Default::default()
            }, None, Some(&mut texture))?;
            self.device.CreateShaderResourceView(texture.as_ref().expect("created a texture"), None, Some(&mut picture))?;
        }
        self.backdrop_targets[slot] = Some(BackdropTarget {
            size, texture: texture.expect("created a texture"), picture: picture.expect("created a view"),
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_backdrop(
        &self, slot: usize, source: &ID3D11RenderTargetView, origin: (i32, i32),
        clip: PixelRect, quad: &Quad, matrix: Option<Matrix>, index: u32,
    ) -> Result<(), RenderError> {
        let target = self.backdrop_targets[slot].as_ref().expect("prepared the backdrop target");
        let shader = self.backdrop_shader.as_ref().expect("prepared the backdrop shader");
        let (_, sampler) = self.image_pipeline.as_ref().expect("prepared the image sampler");
        let (visible, sample) = backdrop_areas(quad, clip, matrix, origin, target.size);
        if is_empty(visible) { return Ok(()); }
        let copy = D3D11_BOX {
            left: sample.left as u32, top: sample.top as u32, front: 0,
            right: sample.right as u32, bottom: sample.bottom as u32, back: 1,
        };
        let context = &self.context;
        // SAFETY: source and snapshot have the same dimensions and format. The input and output bindings are disjoint.
        unsafe {
            let drawn = source.GetResource()?;
            context.OMSetRenderTargets(None, None);
            context.PSSetShaderResources(1, Some(&[None]));
            context.CopySubresourceRegion(&target.texture, 0, copy.left, copy.top, 0, &drawn, 0, Some(&copy));
            context.OMSetRenderTargets(Some(&[Some(source.clone())]), None);
            context.PSSetShader(shader, None);
            context.PSSetSamplers(0, Some(&[Some(sampler.clone())]));
            context.PSSetShaderResources(1, Some(&[Some(target.picture.clone())]));
            context.RSSetScissorRects(Some(&[RECT { left: visible.left, top: visible.top, right: visible.right, bottom: visible.bottom }]));
            context.DrawInstanced(4, 1, 0, index);
            context.PSSetShaderResources(1, Some(&[None]));
        }
        Ok(())
    }

    fn make_layer_target(&mut self, needed: (u32, u32)) -> Result<(), RenderError> {
        if self.layer_sampler.is_none() {
            let mut sampler = None;
            // SAFETY: a complete valid sampler description; the device owns its state.
            unsafe { self.device.CreateSamplerState(&D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_POINT,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MaxLOD: f32::MAX,
                ..Default::default()
            }, Some(&mut sampler))?; }
            self.layer_sampler = sampler;
            #[cfg(feature = "render-diagnostics")]
            self.observe(|work| work.sampler_creations = work.sampler_creations.saturating_add(1));
        }
        if self.layer_target.as_ref().is_some_and(|(size, _, _)| size.0 >= needed.0 && size.1 >= needed.1) {
            return Ok(());
        }
        self.layer_target = None;
        let bucket = |wanted: u32, limit: u32| wanted.max(1).checked_next_power_of_two().unwrap_or(limit).min(limit);
        let size = (bucket(needed.0, self.size.0), bucket(needed.1, self.size.1));
        let (mut texture, mut view, mut picture) = (None, None, None);
        // SAFETY: the description is valid for the calls.
        unsafe {
            self.device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: size.0,
                    Height: size.1,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: FORMAT,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                    ..Default::default()
                },
                None,
                Some(&mut texture),
            )?;
            let texture: &ID3D11Texture2D = texture.as_ref().expect("the call succeeded");
            self.device.CreateRenderTargetView(texture, None, Some(&mut view))?;
            self.device.CreateShaderResourceView(texture, None, Some(&mut picture))?;
        }
        self.layer_target = Some((
            size,
            view.expect("the call succeeded"),
            picture.expect("the call succeeded"),
        ));
        Ok(())
    }

    /// Starts drawing what's faded as a whole, on a clear layer of its own.
    ///
    /// # Safety
    ///
    /// Only uses `self`'s own objects, inside a frame being drawn.
    unsafe fn start_layer(&self, layer: &Layer) {
        let Some((_, view, _)) = &self.layer_target else {
            return;
        };
        let context = &self.context;
        let (dx, dy) = self.layer_origin(layer);
        // SAFETY: as the function says.
        unsafe {
            // The layer's picture can't be read while it's drawn on.
            context.PSSetShaderResources(1, Some(&[None]));
            match &self.context1 {
                Some(context1) => context1.ClearView(
                    view,
                    &[0.0, 0.0, 0.0, 0.0],
                    Some(&[RECT {
                        left: layer.bounds.left.max(0) - dx,
                        top: layer.bounds.top.max(0) - dy,
                        right: layer.bounds.right.min(self.size.0 as i32) - dx,
                        bottom: layer.bounds.bottom.min(self.size.1 as i32) - dy,
                    }]),
                ),
                None => context.ClearRenderTargetView(view, &[0.0, 0.0, 0.0, 0.0]),
            }
            context.OMSetRenderTargets(Some(&[Some(view.clone())]), None);
            // Keep shader coordinates in window pixels. The viewport origin
            // maps those pixels into this layer's smaller temporary surface.
            context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: -(dx as f32), TopLeftY: -(dy as f32),
                Width: self.size.0 as f32, Height: self.size.1 as f32,
                MinDepth: 0.0, MaxDepth: 1.0,
            }]));
        }
    }

    /// Puts what's been drawn on the layer down on what's drawn to, at the layer's opacity, with the quad at `quad`.
    ///
    /// # Safety
    ///
    /// Only uses `self`'s own objects, inside a frame being drawn, with the image pipeline made.
    unsafe fn put_down_layer(
        &self,
        layer: &Layer,
        quad: usize,
        view: &ID3D11RenderTargetView,
        atlas_size: u32,
        region: PixelRect,
    ) -> Result<(), RenderError> {
        let (Some((_, _, picture)), Some((shader, smooth)), Some(sampler)) = (&self.layer_target, &self.image_pipeline, &self.layer_sampler) else {
            return Ok(());
        };
        let context = &self.context;
        // SAFETY: as the function says.
        unsafe {
            context.OMSetRenderTargets(Some(&[Some(view.clone())]), None);
            context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0, TopLeftY: 0.0,
                Width: self.size.0 as f32, Height: self.size.1 as f32,
                MinDepth: 0.0, MaxDepth: 1.0,
            }]));
            self.write_constants(None, None, NO_ROUND_CLIPS, atlas_size)?;
            context.PSSetShader(shader, None);
            context.PSSetSamplers(0, Some(&[Some(sampler.clone())]));
            context.PSSetShaderResources(1, Some(&[Some(picture.clone())]));
            let visible = intersect(layer.bounds, region);
            if !is_empty(visible) {
                context.RSSetScissorRects(Some(&[RECT {
                    left: visible.left, top: visible.top,
                    right: visible.right, bottom: visible.bottom,
                }]));
                context.DrawInstanced(4, 1, 0, quad as u32);
            }
            context.PSSetShaderResources(1, Some(&[None]));
            // Opacity composites are an aligned one-to-one texel copy. Restore
            // ordinary filtering before the next image/transformed-text batch.
            context.PSSetSamplers(0, Some(&[Some(smooth.clone())]));
        }
        Ok(())
    }

    fn make_image_pipeline(&mut self) -> Result<(), RenderError> {
        if self.image_pipeline.is_none() {
            let (mut shader, mut sampler) = (None, None);
            // SAFETY: the shader is the bytes this crate compiled, and the description is valid.
            unsafe {
                self.device
                    .CreatePixelShader(IMAGE_SHADER, None, Some(&mut shader))?;
                #[cfg(feature = "render-diagnostics")]
                self.observe(|work| {
                    work.shader_creations = work.shader_creations.saturating_add(1)
                });
                self.device.CreateSamplerState(
                    &D3D11_SAMPLER_DESC {
                        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                        MaxLOD: f32::MAX,
                        ..Default::default()
                    },
                    Some(&mut sampler),
                )?;
                #[cfg(feature = "render-diagnostics")]
                self.observe(|work| {
                    work.sampler_creations = work.sampler_creations.saturating_add(1)
                });
            }
            self.image_pipeline = Some((
                shader.expect("the call succeeded"),
                sampler.expect("the call succeeded"),
            ));
        }
        Ok(())
    }

    fn upload_image(&mut self, upload: ImageUpload) -> Result<bool, RenderError> {
        if let Some((x, y)) = upload.changes
            && let Some(view) = self.images.get(&upload.id)
        {
            return Ok(self.update_image(view, &upload, x, y));
        }
        let (mut texture, mut view) = (None, None);
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: upload.pixels.as_ptr().cast(),
            SysMemPitch: upload.width * 4,
            SysMemSlicePitch: 0,
        };
        // SAFETY: `pixels` holds `width` by `height` pixels of 4 bytes, and outlives the call.
        let made = unsafe {
            self.device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: upload.width,
                    Height: upload.height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    // What changes, as a canvas does, is changed in place where it changed.
                    Usage: if upload.changes.is_some() { D3D11_USAGE_DEFAULT } else { D3D11_USAGE_IMMUTABLE },
                    BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                    ..Default::default()
                },
                Some(&data),
                Some(&mut texture),
            )
        };
        // An image too big for this GPU just doesn't show, as text and everything else still can.
        if made.is_err() {
            return Ok(false);
        }
        #[cfg(feature = "render-diagnostics")]
        self.observe(|work| {
            work.texture_creations = work.texture_creations.saturating_add(1);
            work.image_upload.add(
                u64::from(upload.width)
                    .saturating_mul(u64::from(upload.height))
                    .saturating_mul(4),
            );
        });
        let texture: ID3D11Texture2D = texture.expect("the call succeeded");
        // SAFETY: the texture was just made with this binding.
        unsafe {
            self.device
                .CreateShaderResourceView(&texture, None, Some(&mut view))?;
        }
        #[cfg(feature = "render-diagnostics")]
        self.observe(|work| work.view_creations = work.view_creations.saturating_add(1));
        self.images
            .insert(upload.id, view.expect("the call succeeded"));
        Ok(true)
    }

    /// Puts the pixels of part of an image that changes where they go in it.
    #[cold]
    #[inline(never)]
    fn update_image(&self, view: &ID3D11ShaderResourceView, upload: &ImageUpload, x: u32, y: u32) -> bool {
        if upload.width == 0 || upload.height == 0 || upload.pixels.len() < upload.width as usize * upload.height as usize * 4 {
            return false;
        }
        // SAFETY: the view is of a texture this made, the box is inside it as the canvas said, and the pixels are that big.
        unsafe {
            let Ok(resource) = view.GetResource() else {
                return false;
            };
            let region = D3D11_BOX { left: x, top: y, front: 0, right: x + upload.width, bottom: y + upload.height, back: 1 };
            self.context.UpdateSubresource(&resource, 0, Some(&region), upload.pixels.as_ptr().cast(), upload.width * 4, 0);
        }
        true
    }

    fn upload_glyphs(&self, uploads: &[Upload]) {
        let Some(atlas) = &self.atlas else {
            return;
        };
        for upload in uploads {
            let area = D3D11_BOX {
                left: upload.x,
                top: upload.y,
                front: 0,
                right: upload.x + upload.width,
                bottom: upload.y + upload.height,
                back: 1,
            };
            // SAFETY: `area` lies within the texture, as the atlas placed it there, and
            // `coverage` holds `width` by `height` bytes.
            unsafe {
                self.context.UpdateSubresource(
                    &atlas.texture,
                    0,
                    Some(&area),
                    upload.coverage.as_ptr().cast(),
                    upload.width,
                    0,
                );
                #[cfg(feature = "render-diagnostics")]
                self.observe(|work| {
                    work.glyph_upload
                        .add(u64::from(upload.width).saturating_mul(u64::from(upload.height)))
                });
            }
        }
    }

    fn upload_quads(&mut self, quads: &[Quad]) -> Result<(), RenderError> {
        self.upload_quads_and(quads, &[])
    }

    /// Those, then `more` after them, as what puts down what's faded as a whole is.
    fn upload_quads_and(&mut self, quads: &[Quad], more: &[Quad]) -> Result<(), RenderError> {
        let needed = u32::try_from(quads.len() + more.len()).unwrap_or(u32::MAX);
        if needed > self.quad_capacity {
            let capacity = needed.next_power_of_two();
            self.quads = create_quad_buffer(&self.device, capacity)?;
            #[cfg(feature = "render-diagnostics")]
            self.observe(|work| work.buffer_creations = work.buffer_creations.saturating_add(1));
            self.quad_capacity = capacity;
        }
        if quads.is_empty() && more.is_empty() {
            return Ok(());
        }
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: the buffer holds at least `quads.len() + more.len()` quads, as just ensured.
        unsafe {
            self.context.Map(
                &self.quads,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut mapped),
            )?;
            std::ptr::copy_nonoverlapping(quads.as_ptr(), mapped.pData.cast::<Quad>(), quads.len());
            std::ptr::copy_nonoverlapping(more.as_ptr(), mapped.pData.cast::<Quad>().add(quads.len()), more.len());
            self.context.Unmap(&self.quads, 0);
            #[cfg(feature = "render-diagnostics")]
            self.observe(|work| {
                work.quad_upload
                    .add(crate::diagnostics::bytes::<Quad>(quads.len()))
            });
        }
        Ok(())
    }

    /// # Safety
    ///
    /// Only uses `self`'s own objects, and is unsafe only because the calls it makes are.
    unsafe fn write_frame_constants(&self) -> Result<(), RenderError> {
        self.write_constants(None, None, NO_ROUND_CLIPS, 0)
    }

    /// Says how what's about to be drawn is turned or resized, if it is, what its colors go through, if anything, and which
    /// round corners it's cut off at.
    fn write_constants(
        &self,
        matrix: Option<Matrix>,
        filter: Option<&Filter>,
        rounds: RoundSlots,
        atlas_size: u32,
    ) -> Result<(), RenderError> {
        let rows = filter.map_or(
            [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            |filter| filter.rows,
        );
        let [a, b, c, d, e, f] = matrix.unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let sharpness = matrix.map_or(1.0, |matrix| {
            let [a, b, c, d, ..] = matrix;
            (a * d - b * c).abs().sqrt()
        });
        let mut values = [0f32; 64];
        values[..32].copy_from_slice(&[
            self.size.0 as f32,
            self.size.1 as f32,
            atlas_size as f32,
            sharpness,
            a,
            b,
            c,
            d,
            e,
            f,
            if matrix.is_some() { 1.0 } else { 0.0 },
            0.0,
            self.text_look[0],
            self.text_look[1],
            0.0,
            0.0,
            rows[0],
            rows[1],
            rows[2],
            rows[3],
            rows[4],
            rows[5],
            rows[6],
            rows[7],
            rows[8],
            rows[9],
            rows[10],
            rows[11],
            if filter.is_some() { 1.0 } else { 0.0 },
            filter.map_or(1.0, |filter| filter.opacity),
            0.0,
            0.0,
        ]);
        write_round_slots(&mut values[32..], rounds);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: the buffer is as big as what's written.
        unsafe {
            self.context.Map(
                &self.frame_constants,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut mapped),
            )?;
            std::ptr::copy_nonoverlapping(
                values.as_ptr(),
                mapped.pData.cast::<f32>(),
                values.len(),
            );
            self.context.Unmap(&self.frame_constants, 0);
            #[cfg(feature = "render-diagnostics")]
            self.observe(|work| work.constant_upload.add(u64::from(FRAME_CONSTANTS_SIZE)));
        }
        Ok(())
    }
}

fn create_quad_buffer(device: &ID3D11Device, capacity: u32) -> Result<ID3D11Buffer, RenderError> {
    let mut buffer = None;
    // SAFETY: the description is valid for the call.
    unsafe {
        device.CreateBuffer(
            &D3D11_BUFFER_DESC {
                ByteWidth: capacity.saturating_mul(size_of::<Quad>() as u32),
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                ..Default::default()
            },
            None,
            Some(&mut buffer),
        )?;
    }
    Ok(buffer.expect("the call succeeded"))
}

fn factory_of(device: &ID3D11Device) -> Result<IDXGIFactory2, RenderError> {
    // SAFETY: these only ask a valid device what made it.
    unsafe {
        let dxgi_device: IDXGIDevice = device.cast()?;
        Ok(dxgi_device.GetAdapter()?.GetParent()?)
    }
}

/// The swap chain a window has of its own, which is opaque. This is only for when DirectComposition can't be had: once one
/// of these has shown a frame, the window shows its last frame for as long as it lasts, under anything else.
fn window_chain(
    device: &ID3D11Device,
    window: HWND,
    size: (u32, u32),
) -> Result<IDXGISwapChain1, RenderError> {
    let factory = factory_of(device)?;
    // SAFETY: `window` is the caller's window, and the description is valid for the call.
    unsafe {
        let chain = factory.CreateSwapChainForHwnd(
            device,
            window,
            &DXGI_SWAP_CHAIN_DESC1 {
                Width: size.0,
                Height: size.1,
                Format: FORMAT,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                // Unlike discarding, this keeps what's in each buffer, which is what lets a
                // frame redraw only what changed.
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                Scaling: DXGI_SCALING_NONE,
                ..Default::default()
            },
            None,
            None,
        )?;
        factory.MakeWindowAssociation(window, DXGI_MWA_NO_ALT_ENTER)?;
        Ok(chain)
    }
}

/// A swap chain for DirectComposition to show over a window, which is opaque, or has premultiplied alpha if it's to be
/// `see_through`. It's otherwise as a window's own is: two buffers, flipped in turn, each keeping what's in it.
fn composition_chain(
    device: &ID3D11Device,
    size: (u32, u32),
    see_through: bool,
) -> Result<IDXGISwapChain1, RenderError> {
    let factory = factory_of(device)?;
    // SAFETY: the description is valid for the call.
    unsafe {
        Ok(factory.CreateSwapChainForComposition(
            device,
            &DXGI_SWAP_CHAIN_DESC1 {
                Width: size.0,
                Height: size.1,
                Format: FORMAT,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                // Unlike discarding, this keeps what's in each buffer, which is what lets a
                // frame redraw only what changed.
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                // A swap chain for composition has to say how it's scaled, though it's always shown at its own size.
                Scaling: DXGI_SCALING_STRETCH,
                // Opaque, the system copies it rather than blending it with what's under it.
                AlphaMode: if see_through {
                    DXGI_ALPHA_MODE_PREMULTIPLIED
                } else {
                    DXGI_ALPHA_MODE_IGNORE
                },
                ..Default::default()
            },
            None,
        )?)
    }
}

/// DirectComposition's way in, `DCompositionCreateDevice2`, which is looked up the first time it's needed.
type CreateCompositionDevice = unsafe extern "system" fn(
    *mut c_void,
    *const windows::core::GUID,
    *mut *mut c_void,
) -> HRESULT;

fn create_composition_device() -> Option<CreateCompositionDevice> {
    static FOUND: std::sync::OnceLock<Option<CreateCompositionDevice>> =
        std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        // SAFETY: the library stays loaded for good, and the function has this signature.
        unsafe {
            let library = LoadLibraryW(w!("dcomp.dll")).ok()?;
            let found = GetProcAddress(library, s!("DCompositionCreateDevice2"))?;
            Some(std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                CreateCompositionDevice,
            >(found))
        }
    })
}

/// What puts a swap chain over a window, through DirectComposition, under any windows inside it, as the window's own would
/// be. It shows nothing until it's given a swap chain that's had a frame, with [`Composition::show`].
fn show_through_composition(window: HWND) -> Result<Composition, RenderError> {
    let composition = composition_device().ok_or_else(|| {
        RenderError::Graphics(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_NOTIMPL,
        ))
    })?;
    // SAFETY: the device is valid, and `window` is the caller's.
    unsafe {
        let target = composition.CreateTargetForHwnd(window, false)?;
        let visual = composition.CreateVisual()?;
        target.SetRoot(&visual)?;
        Ok(Composition {
            device: composition,
            target,
            visual,
            showing: false,
        })
    }
}

fn window_view(
    device: &ID3D11Device,
    chain: &IDXGISwapChain1,
) -> Result<ID3D11RenderTargetView, RenderError> {
    let mut view = None;
    // SAFETY: buffer 0 is always valid for a flip swap chain, and is the one drawn to.
    unsafe {
        let back_buffer: ID3D11Texture2D = chain.GetBuffer(0)?;
        device.CreateRenderTargetView(&back_buffer, None, Some(&mut view))?;
    }
    Ok(view.expect("the call succeeded"))
}

fn offscreen_target(
    device: &ID3D11Device,
    size: (u32, u32),
    flipping: bool,
) -> Result<(Target, ID3D11RenderTargetView), RenderError> {
    let description = D3D11_TEXTURE2D_DESC {
        Width: size.0,
        Height: size.1,
        MipLevels: 1,
        ArraySize: 1,
        Format: FORMAT,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        ..Default::default()
    };
    let (mut texture, mut view) = (None, None);
    // SAFETY: the descriptions are valid for the calls.
    unsafe {
        device.CreateTexture2D(&description, None, Some(&mut texture))?;
        let texture: &ID3D11Texture2D = texture.as_ref().expect("the call succeeded");
        device.CreateRenderTargetView(texture, None, Some(&mut view))?;
    }
    let spare = if flipping {
        let (mut other, mut other_view) = (None, None);
        // SAFETY: as above.
        unsafe {
            device.CreateTexture2D(&description, None, Some(&mut other))?;
            let other: &ID3D11Texture2D = other.as_ref().expect("the call succeeded");
            device.CreateRenderTargetView(other, None, Some(&mut other_view))?;
        }
        Some((
            other.expect("the call succeeded"),
            other_view.expect("the call succeeded"),
        ))
    } else {
        None
    };
    Ok((
        Target::Offscreen {
            texture: texture.expect("the call succeeded"),
            spare,
        },
        view.expect("the call succeeded"),
    ))
}

#[cfg(feature = "render-diagnostics")]
impl Renderer {
    fn observe(&self, update: impl FnOnce(&mut crate::diagnostics::DrawWork)) {
        let mut work = self.work.get();
        update(&mut work);
        self.work.set(work);
    }

    /// Copies the last attempt, including completed work on errors. Does not reset.
    pub fn draw_work(&self) -> crate::diagnostics::DrawWork {
        self.work.get()
    }

    /// Explicit owned-storage census; GPU capacities are logical payload estimates.
    pub fn retained_storage(&self) -> crate::diagnostics::RendererStorage {
        use crate::diagnostics::{CommandStorage, ListStorage, RendererStorage, bytes};
        RendererStorage {
            history: CommandStorage::of(
                &self.last_quads,
                &self.last_batches,
                &self.last_transforms,
                &self.last_filters,
            ),
            partial_quads: ListStorage::of(&self.partial_quads),
            partial_batches: ListStorage::of(&self.partial_batches),
            quad_buffer_capacity_bytes: bytes::<Quad>(self.quad_capacity as usize),
            atlas_capacity_bytes: self.atlas.as_ref().map_or(0, |atlas| {
                u64::from(atlas.size).saturating_mul(u64::from(atlas.size))
            }),
            image_entries: self.images.len() as u64,
            image_map_capacity: self.images.capacity() as u64,
            layer_target_capacity_bytes: self.layer_target.as_ref().map_or(0, |(size, _, _)| u64::from(size.0) * u64::from(size.1) * 4),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHOLE: PixelRect = PixelRect {
        left: 0,
        top: 0,
        right: 500,
        bottom: 500,
    };

    fn quad(x: f32, y: f32) -> Quad {
        Quad {
            rect: [x, y, 10.0, 10.0],
            glyph: [0; 4],
            color: [255; 4],
            radius: 0.0,
        }
    }

    fn batch(first: u32, count: u32, clip: PixelRect, image: u32) -> Batch {
        Batch {
            first,
            count,
            clip,
            image,
            transform: 0,
            filter: 0,
            round: 0,
        }
    }

    #[test]
    fn another_frame_of_an_image_only_changes_where_that_image_is() {
        let quads = [quad(0.0, 0.0), quad(100.0, 100.0), quad(300.0, 300.0)];
        let old = [
            batch(0, 1, WHOLE, 0),
            batch(1, 1, WHOLE, 7),
            batch(2, 1, WHOLE, 0),
        ];
        let new = [
            batch(0, 1, WHOLE, 0),
            batch(1, 1, WHOLE, 8),
            batch(2, 1, WHOLE, 0),
        ];
        assert_eq!(
            changed_area((&quads, &old), (&quads, &new), WHOLE),
            PixelRect {
                left: 100,
                top: 100,
                right: 110,
                bottom: 110,
            }
        );
    }

    #[test]
    fn an_image_only_changes_what_its_clip_shows() {
        let quads = [quad(100.0, 100.0)];
        let clip = PixelRect {
            left: 105,
            top: 0,
            right: 500,
            bottom: 500,
        };
        let old = [batch(0, 1, clip, 7)];
        let new = [batch(0, 1, clip, 8)];
        assert_eq!(
            changed_area((&quads, &old), (&quads, &new), WHOLE),
            PixelRect {
                left: 105,
                top: 100,
                right: 110,
                bottom: 110,
            }
        );
    }

    #[test]
    fn a_changed_clip_still_means_everything() {
        let quads = [quad(100.0, 100.0)];
        let clip = PixelRect {
            left: 105,
            top: 0,
            right: 500,
            bottom: 500,
        };
        let old = [batch(0, 1, WHOLE, 7)];
        let new = [batch(0, 1, clip, 7)];
        assert_eq!(changed_area((&quads, &old), (&quads, &new), WHOLE), WHOLE);
    }

    #[test]
    fn new_image_ids_keep_their_damage_local() {
        let size = (40, 20);
        let area = PixelRect {
            left: 0,
            top: 0,
            right: 10,
            bottom: 10,
        };
        for flipping in [false, true] {
            let make = || {
                if flipping {
                    Renderer::offscreen_flipping(size)
                } else {
                    Renderer::offscreen(size)
                }
                .unwrap()
            };
            let scene = |id, old: Option<u32>, color: Option<[u8; 4]>| Scene {
                quads: vec![Quad {
                    glyph: [0, 0, 65535, 65535],
                    ..quad(0.0, 0.0)
                }],
                batches: vec![batch(0, 1, WHOLE, id)],
                dropped_images: old.into_iter().collect(),
                image_uploads: color
                    .into_iter()
                    .map(|color| ImageUpload {
                        id,
                        width: 1,
                        height: 1,
                        pixels: color.to_vec(),
                        changes: None,
                    })
                    .collect(),
                ..Scene::default()
            };
            let mut candidate = make();
            let mut reference = make();
            reference.set_always_redraw(true);
            for (id, old, color) in [
                (7, None, [255, 0, 0, 255]),
                (8, Some(7), [0, 0, 255, 255]),
                (9, Some(8), [0, 255, 0, 255]),
            ] {
                for renderer in [&mut candidate, &mut reference] {
                    renderer
                        .render(&mut scene(id, old, Some(color)), Color::WHITE)
                        .unwrap();
                }
                assert_eq!(
                    candidate.read_pixels().unwrap(),
                    reference.read_pixels().unwrap()
                );
                if old.is_some() {
                    assert_eq!(candidate.previous_change, area, "new ID keeps local damage");
                }
                candidate
                    .render(&mut scene(id, None, None), Color::WHITE)
                    .unwrap();
                assert!(candidate.showed_nothing());
                assert_eq!(
                    candidate.read_pixels().unwrap(),
                    reference.read_pixels().unwrap()
                );
            }
        }
    }

    #[cfg(feature = "mutation-pilot")]
    #[test]
    fn owned_submission_replays_resource_and_buffer_epochs_without_command_history() {
        for flipping in [false, true] {
            let make = || {
                if flipping {
                    Renderer::offscreen_flipping((40, 20))
                } else {
                    Renderer::offscreen((40, 20))
                }
                .unwrap()
            };
            let mut gpu = make();
            let mut reference = make();
            reference.set_always_redraw(true);
            let mut painter = crate::Painter::new();
            for step in 0..8 {
                let mut scene = Scene {
                    window_color: Some(Color::WHITE),
                    quads: vec![Quad {
                        glyph: [0, 0, 65535, 65535],
                        ..quad(0.0, 0.0)
                    }],
                    batches: vec![batch(0, 1, WHOLE, 7)],
                    atlas_generation: step / 2,
                    text_look: [step as f32 / 10.0, 0.0],
                    ..Scene::default()
                };
                if step == 3 {
                    scene.dropped_images.push(7);
                } else {
                    scene.image_uploads.push(ImageUpload {
                        id: 7,
                        width: 1,
                        height: 1,
                        pixels: vec![step as u8 * 30, 0, 255, 255],
                        changes: None,
                    });
                }
                if step == 6 {
                    gpu = make();
                    reference = make();
                    reference.set_always_redraw(true);
                }
                *painter.last_scene() = scene.clone();
                gpu.draw_owned_pilot(&mut painter).unwrap();
                gpu.present().unwrap();
                reference.render(&mut scene, Color::WHITE).unwrap();
                assert_eq!(
                    gpu.read_pixels().unwrap(),
                    reference.read_pixels().unwrap(),
                    "step {step}"
                );
                assert_eq!(
                    painter.last_scene().quads.len(),
                    1,
                    "authoritative commands retained"
                );
                assert_eq!(
                    gpu.last_quads.capacity(),
                    0,
                    "no previous-command allocation"
                );
            }
        }
    }

    #[test]
    fn idle_image_release_invalidates_referenced_history_and_allows_same_id_reupload() {
        for flipping in [false, true] {
            let make = || {
                if flipping {
                    Renderer::offscreen_flipping((20, 20))
                } else {
                    Renderer::offscreen((20, 20))
                }
                .unwrap()
            };
            let scene = |color: Option<[u8; 4]>| Scene {
                quads: vec![Quad {
                    glyph: [0, 0, 65535, 65535],
                    ..quad(0.0, 0.0)
                }],
                batches: vec![batch(0, 1, WHOLE, 7)],
                image_uploads: color
                    .into_iter()
                    .map(|color| ImageUpload {
                        id: 7,
                        width: 1,
                        height: 1,
                        pixels: color.to_vec(),
                        changes: None,
                    })
                    .collect(),
                ..Scene::default()
            };
            let mut candidate = make();
            let mut reference = make();
            reference.set_always_redraw(true);
            candidate
                .render(&mut scene(Some([255, 0, 0, 255])), Color::WHITE)
                .unwrap();
            let before = candidate.read_pixels().unwrap();
            let mut dropped = vec![7, 7, 99];
            candidate.release_images(&mut dropped);
            assert!(dropped.is_empty());
            assert!(candidate.images.is_empty());
            assert!(!candidate.known);
            assert_eq!(
                candidate.read_pixels().unwrap(),
                before,
                "release must not draw"
            );
            // Unchanged commands with a now-missing resource must erase old pixels.
            for renderer in [&mut candidate, &mut reference] {
                renderer.render(&mut scene(None), Color::WHITE).unwrap();
            }
            assert_ne!(candidate.read_pixels().unwrap(), before);
            assert_eq!(
                candidate.read_pixels().unwrap(),
                reference.read_pixels().unwrap()
            );
            for renderer in [&mut candidate, &mut reference] {
                renderer
                    .render(&mut scene(Some([0, 0, 255, 255])), Color::WHITE)
                    .unwrap();
            }
            assert_eq!(
                candidate.read_pixels().unwrap(),
                reference.read_pixels().unwrap()
            );
            candidate.render(&mut scene(None), Color::WHITE).unwrap();
            assert!(candidate.showed_nothing());
        }
    }

    #[cfg(feature = "mutation-pilot")]
    #[test]
    fn owned_failed_submission_is_not_acknowledged() {
        for full_repaint in [false, true] {
            for fail_upload in [false, true] {
                for flipping in [false, true] {
                    let mut gpu = if flipping {
                        Renderer::offscreen_flipping((20, 20))
                    } else {
                        Renderer::offscreen((20, 20))
                    }
                    .unwrap();
                    let mut painter = crate::Painter::new();
                    painter.scene.quads.push(quad(0.0, 0.0));
                    painter.scene.batches.push(batch(0, 1, WHOLE, 0));
                    painter.pilot.epoch = 1;
                    painter.pilot.valid = true;
                    painter.pilot.region = area_of(&painter.scene.quads[0]);
                    painter.pilot.contributors.push((0, 0));
                    gpu.draw_owned_pilot(&mut painter).unwrap();
                    gpu.present().unwrap();
                    let before = gpu.owned_stamp;
                    let mut unmappable = None;
                    // SAFETY: valid description, owned output; deliberately not CPU-mappable.
                    unsafe {
                        gpu.device
                            .CreateBuffer(
                                &D3D11_BUFFER_DESC {
                                    ByteWidth: FRAME_CONSTANTS_SIZE,
                                    Usage: D3D11_USAGE_DEFAULT,
                                    BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                                    ..Default::default()
                                },
                                None,
                                Some(&mut unmappable),
                            )
                            .unwrap();
                    }
                    let working = if fail_upload {
                        std::mem::replace(&mut gpu.quads, unmappable.unwrap())
                    } else {
                        std::mem::replace(&mut gpu.frame_constants, unmappable.unwrap())
                    };
                    if full_repaint {
                        painter.pilot.pending = Some(((1, 0), painter.pilot.region));
                        painter.pilot.history.quads = std::mem::take(&mut painter.scene.quads);
                        painter.pilot.history.batches = std::mem::take(&mut painter.scene.batches);
                        painter.scene.quads.push(quad(0.0, 0.0));
                        painter.scene.batches.push(batch(0, 1, WHOLE, 0));
                    }
                    painter.pilot.revision += 1;
                    painter.scene.quads[0].color = [255, 0, 0, 255];
                    assert!(gpu.draw_owned_pilot(&mut painter).is_err());
                    assert_eq!(gpu.owned_stamp, before);
                    assert_eq!(painter.pilot.pending.is_some(), full_repaint);
                    assert!(!gpu.known);
                    if fail_upload {
                        gpu.quads = working;
                    } else {
                        gpu.frame_constants = working;
                    }
                    gpu.draw_owned_pilot(&mut painter).unwrap();
                    assert_eq!(gpu.owned_stamp, Some((1, 1)));
                    gpu.present().unwrap();
                    let mut reference = Renderer::offscreen((20, 20)).unwrap();
                    reference
                        .render(&mut painter.scene.clone(), Color::hex(0xF6F8FA))
                        .unwrap();
                    assert_eq!(gpu.read_pixels().unwrap(), reference.read_pixels().unwrap());
                    painter.pilot.revision += 1;
                    painter.scene.quads[0].color = [0, 255, 0, 255];
                    gpu.draw_owned_pilot(&mut painter).unwrap();
                    gpu.fail_present = true;
                    assert!(gpu.present().is_err());
                    assert!(!gpu.known);
                    assert!(!gpu.owned_pending);
                    gpu.draw_owned_pilot(&mut painter).unwrap();
                    #[cfg(feature = "render-diagnostics")]
                    assert_eq!(
                        gpu.draw_work().outcome,
                        crate::diagnostics::DrawOutcome::Full
                    );
                    gpu.present().unwrap();
                    reference
                        .render(&mut painter.scene.clone(), Color::hex(0xF6F8FA))
                        .unwrap();
                    assert_eq!(gpu.read_pixels().unwrap(), reference.read_pixels().unwrap());
                }
            }
        }
    }

    #[test]
    fn a_failed_draw_after_upload_does_not_commit_resource_history() {
        for flipping in [false, true] {
            let make = || {
                if flipping {
                    Renderer::offscreen_flipping((20, 20))
                } else {
                    Renderer::offscreen((20, 20))
                }
                .unwrap()
            };
            let scene = |color: Option<[u8; 4]>, moved| Scene {
                quads: vec![Quad {
                    glyph: [0, 0, 65535, 65535],
                    ..quad(if moved { 5.0 } else { 0.0 }, 0.0)
                }],
                batches: vec![batch(0, 1, WHOLE, 7)],
                image_uploads: color
                    .into_iter()
                    .map(|color| ImageUpload {
                        id: 7,
                        width: 1,
                        height: 1,
                        pixels: color.to_vec(),
                        changes: None,
                    })
                    .collect(),
                ..Scene::default()
            };
            let mut candidate = make();
            let mut reference = make();
            reference.set_always_redraw(true);
            for renderer in [&mut candidate, &mut reference] {
                renderer
                    .render(&mut scene(Some([255, 0, 0, 255]), false), Color::WHITE)
                    .unwrap();
            }
            let before = reference.read_pixels().unwrap();
            candidate
                .render(&mut scene(None, false), Color::WHITE)
                .unwrap();
            assert!(candidate.showed_nothing());

            // Inject a Map failure after the image upload: DEFAULT buffers cannot be
            // mapped with WRITE_DISCARD. This avoids relying on an actual device loss.
            let mut unmappable = None;
            // SAFETY: this valid buffer description needs no initial data; the output is owned.
            unsafe {
                candidate
                    .device
                    .CreateBuffer(
                        &D3D11_BUFFER_DESC {
                            ByteWidth: FRAME_CONSTANTS_SIZE,
                            Usage: D3D11_USAGE_DEFAULT,
                            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                            ..Default::default()
                        },
                        None,
                        Some(&mut unmappable),
                    )
                    .unwrap();
            }
            let working = std::mem::replace(&mut candidate.frame_constants, unmappable.unwrap());
            let mut failed = scene(Some([0, 0, 255, 255]), true);
            assert!(candidate.draw(&mut failed, Color::WHITE).is_err());
            assert!(
                failed.image_uploads.is_empty(),
                "the upload already happened"
            );
            assert!(
                !candidate.known,
                "an incomplete draw must invalidate history"
            );
            #[cfg(feature = "render-diagnostics")]
            {
                use crate::diagnostics::{DrawOutcome, Payload, Presentation};
                let work = candidate.draw_work();
                assert_eq!(work.outcome, DrawOutcome::Error);
                assert_eq!(work.image_upload, Payload { count: 1, bytes: 4 });
                assert_eq!(work.quad_upload.bytes, size_of::<Quad>() as u64);
                assert_eq!(work.constant_upload, Payload::default());
                assert_eq!(work.draw_calls, 0);
                assert_eq!(work.presentation, Presentation::None);
            }
            candidate.frame_constants = working;

            reference
                .render(&mut scene(Some([0, 0, 255, 255]), false), Color::WHITE)
                .unwrap();
            let expected = reference.read_pixels().unwrap();
            assert!(expected != before, "the replacement is visible");
            // Recover with the original geometry and no upload: the resource mutation
            // survives the failure, but cannot be found by comparing command lists.
            candidate
                .render(&mut scene(None, false), Color::WHITE)
                .unwrap();
            assert!(!candidate.showed_nothing());
            assert_eq!(candidate.read_pixels().unwrap(), expected);
            #[cfg(feature = "render-diagnostics")]
            {
                use crate::diagnostics::{DrawOutcome, Payload};
                let work = candidate.draw_work();
                assert_eq!(work.outcome, DrawOutcome::Full);
                assert!(work.full_reasons.unknown_history);
                assert_eq!(work.image_upload, Payload::default());
                assert_eq!(
                    work.constant_upload,
                    Payload {
                        count: 1,
                        bytes: u64::from(FRAME_CONSTANTS_SIZE)
                    }
                );
                assert_eq!(work.draw_calls, 1);
            }
            for (moved, unchanged) in [(false, true), (true, false), (true, true)] {
                candidate
                    .render(&mut scene(None, moved), Color::WHITE)
                    .unwrap();
                reference
                    .render(&mut scene(None, moved), Color::WHITE)
                    .unwrap();
                assert_eq!(candidate.showed_nothing(), unchanged);
                assert_eq!(
                    candidate.read_pixels().unwrap(),
                    reference.read_pixels().unwrap()
                );
            }
        }
    }
}
