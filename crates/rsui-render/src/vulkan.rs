//! Draws scenes with Vulkan, on Linux and Android: the same quads, through the same arithmetic (`quad.glsl`, a port of
//! `quad.hlsl` by hand), as the Direct3D renderer, into a window's swapchain or into memory.
//!
//! The library is loaded when the first renderer is made, so nothing about Vulkan is linked into the program, and a machine
//! without it just draws with the CPU. Only a real GPU is used for a window unless `RSUI_VULKAN=1` says a software one will
//! do: a CPU drawing through Vulkan is slower than the CPU renderer drawing by itself.
//!
//! It works one frame at a time. Each frame waits for the last to finish on the GPU before it starts, which costs nothing in
//! a window, as a window only draws again once the compositor has shown the last frame, and it means nothing the GPU reads
//! is ever written while it's reading it. What's faded as a whole is drawn on an image of its own, then put down with the
//! quad the Direct3D renderer puts it down with. Every frame that's different from the last is drawn whole; the renderer
//! that owns this works out which are.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicU32, Ordering};

use ash::vk;
use rsui_text::Upload;

use crate::area::{NO_ROUND_CLIPS, RoundSlots, intersect, is_empty, layer_quad, round_clips_of};
use crate::gpu::RenderError;
use crate::images::ImageUpload;
use crate::paint::Color;
use crate::scene::{Filter, Layer, PixelRect, Quad, Scene};
use crate::transform::Matrix;

/// The shaders, compiled from `quad.glsl` by glslang: see the commands at its top.
const VERTEX_SHADER: &[u8] = include_bytes!("quad_vert.spv");
const QUAD_SHADER: &[u8] = include_bytes!("quad_frag.spv");
const IMAGE_SHADER: &[u8] = include_bytes!("image_frag.spv");

/// How big the shaders' constants are, which is also how far apart each batch's are in the buffer they're read from: no
/// GPU asks for more than this between them.
const CONSTANTS_SIZE: u64 = 256;

/// How many quads there's room for at first.
const FIRST_QUAD_CAPACITY: u64 = 2048;

/// A staging buffer bigger than this, which only a big image needs, is let go of once the frame that used it is done.
const STAGING_KEPT: u64 = 1 << 20;

const _: () = assert!(size_of::<Quad>() == 32, "the vertex input reads quads 32 bytes apart");

/// How many messages the validation layers have reported as errors, for tests to check there were none.
static VALIDATION_ERRORS: AtomicU32 = AtomicU32::new(0);

/// How many errors the validation layers (`RSUI_VULKAN_VALIDATE=1`) have reported, in this process.
pub(crate) fn validation_errors() -> u32 {
    VALIDATION_ERRORS.load(Ordering::Relaxed)
}

impl From<vk::Result> for RenderError {
    fn from(result: vk::Result) -> Self {
        match result {
            vk::Result::ERROR_DEVICE_LOST | vk::Result::ERROR_SURFACE_LOST_KHR => Self::DeviceLost,
            other => Self::Graphics(format!("Vulkan: {other}")),
        }
    }
}

fn failed(what: &str) -> RenderError {
    RenderError::Graphics(format!("Vulkan: {what}"))
}

/// What a window gives to be drawn on. The pointers are the system's own, and must outlive the renderer.
#[derive(Clone, Copy)]
pub(crate) enum Window {
    /// A `wl_display` and a `wl_surface` of libwayland's.
    #[cfg(target_os = "linux")]
    Wayland { display: *mut c_void, surface: *mut c_void },
    /// An `ANativeWindow`.
    #[cfg(target_os = "android")]
    Android { window: *mut c_void },
}

/// A buffer the CPU writes or reads, mapped for as long as it lives.
struct Buffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    mapped: *mut u8,
    size: u64,
}

/// An image, what it's read through, and how it's laid out now, as the commands recorded so far leave it.
struct Texture {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    width: u32,
    height: u32,
    layout: vk::ImageLayout,
}

/// An image a scene shows, and the descriptor set that reads it.
struct Picture {
    texture: Texture,
    pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
}

/// An image drawn on: a layer, or what's drawn into memory. Its texture's layout isn't kept track of, as every frame
/// starts drawing on it from nothing, and says what it was left as when it's read.
struct Canvas {
    texture: Texture,
    framebuffer: vk::Framebuffer,
}

/// Drawing into memory: the image, and the buffer it's copied into at the end of each frame for the CPU to read.
struct MemoryTarget {
    canvas: Canvas,
    readback: Buffer,
    drawn: bool,
}

struct Swapchain {
    handle: vk::SwapchainKHR,
    extent: vk::Extent2D,
    images: Vec<vk::Image>,
    views: Vec<vk::ImageView>,
    framebuffers: Vec<vk::Framebuffer>,
    /// Signalled when each image has been drawn, for presenting it to wait on.
    done: Vec<vk::Semaphore>,
}

/// What the atlas needs before the next frame, in order.
enum AtlasWork {
    /// To be this big, keeping what's in it if it grew.
    Size(u32),
    Glyphs(Upload),
}

/// What the GPU may still be using, let go of once the frame in flight is done.
enum Retired {
    Texture(Texture),
    Buffer(Buffer),
    Picture(Picture),
    Canvas(Canvas),
}

/// The Vulkan renderer's own state. The renderer that owns it decides when a frame needs drawing.
pub(crate) struct Vulkan {
    _entry: ash::Entry,
    instance: ash::Instance,
    debug: Option<(ash::ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    surfaces: Option<ash::khr::surface::Instance>,
    surface: vk::SurfaceKHR,
    physical: vk::PhysicalDevice,
    memory_types: vk::PhysicalDeviceMemoryProperties,
    largest_image: u32,
    device: ash::Device,
    queue: vk::Queue,
    swapchains: Option<ash::khr::swapchain::Device>,
    swapchain: Option<Swapchain>,
    /// Whether the swapchain has to be made again before the next frame: the window changed size, or said it had.
    stale: bool,
    /// The swapchain image drawn this frame, waiting to be presented.
    acquired: Option<u32>,
    format: vk::Format,
    color_space: vk::ColorSpaceKHR,
    see_through: bool,
    size: (u32, u32),
    /// A render pass that clears what it draws on first, and one that keeps what's there.
    clearing: vk::RenderPass,
    keeping: vk::RenderPass,
    set_layouts: [vk::DescriptorSetLayout; 2],
    layout: vk::PipelineLayout,
    quad_pipeline: vk::Pipeline,
    image_pipeline: vk::Pipeline,
    smooth: vk::Sampler,
    exact: vk::Sampler,
    pool: vk::DescriptorPool,
    /// The constants and the atlas, which every draw reads.
    frame_set: vk::DescriptorSet,
    frame_set_stale: bool,
    /// What's faded as a whole, as it's put down.
    layer_set: vk::DescriptorSet,
    picture_pools: Vec<vk::DescriptorPool>,
    commands: vk::CommandPool,
    command: vk::CommandBuffer,
    fence: vk::Fence,
    /// Signalled when the swapchain image to draw on is ready.
    ready: vk::Semaphore,
    in_flight: bool,
    instances: Option<Buffer>,
    constants: Option<Buffer>,
    staging: Option<Buffer>,
    atlas: Option<Texture>,
    atlas_wanted: u32,
    atlas_work: Vec<AtlasWork>,
    pictures: HashMap<u32, Picture>,
    picture_work: Vec<ImageUpload>,
    layer: Option<Canvas>,
    memory: Option<MemoryTarget>,
    retired: Vec<Retired>,
}

// SAFETY: everything here is this renderer's own, and Vulkan only asks that each object is used by one thread at a time,
// which `&mut self` sees to. The mapped pointers point into memory it owns.
unsafe impl Send for Vulkan {}

unsafe extern "system" fn report(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _types: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user: *mut c_void,
) -> vk::Bool32 {
    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        VALIDATION_ERRORS.fetch_add(1, Ordering::Relaxed);
    }
    // SAFETY: the layers give a message that ends in a zero, for as long as this runs.
    let message = unsafe { data.as_ref().filter(|data| !data.p_message.is_null()).map(|data| CStr::from_ptr(data.p_message)) };
    if let Some(message) = message {
        let kind = if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) { "error" } else { "warning" };
        eprintln!("rsui vulkan {kind}: {}", message.to_string_lossy());
    }
    vk::FALSE
}

/// What each layout an image can be in was last used by, as a stage and what it wrote, and what's about to use it, as a
/// stage and what it reads or writes, for the barriers between them.
fn layout_use(layout: vk::ImageLayout) -> (vk::PipelineStageFlags, vk::AccessFlags, vk::AccessFlags) {
    match layout {
        vk::ImageLayout::TRANSFER_DST_OPTIMAL => {
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::TRANSFER_WRITE, vk::AccessFlags::TRANSFER_WRITE)
        }
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL => {
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::empty(), vk::AccessFlags::TRANSFER_READ)
        }
        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL => {
            (vk::PipelineStageFlags::FRAGMENT_SHADER, vk::AccessFlags::empty(), vk::AccessFlags::SHADER_READ)
        }
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL => (
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
        ),
        vk::ImageLayout::PRESENT_SRC_KHR => {
            (vk::PipelineStageFlags::BOTTOM_OF_PIPE, vk::AccessFlags::empty(), vk::AccessFlags::empty())
        }
        _ => (vk::PipelineStageFlags::TOP_OF_PIPE, vk::AccessFlags::empty(), vk::AccessFlags::empty()),
    }
}

const COLOR: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

const COLOR_LAYERS: vk::ImageSubresourceLayers =
    vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 };

/// Records a barrier that takes an image from one layout to another, after what used it before and before what uses it next.
///
/// # Safety
///
/// `command` is recording, and `image` is alive and in `from`.
unsafe fn image_barrier(
    device: &ash::Device,
    command: vk::CommandBuffer,
    image: vk::Image,
    from: vk::ImageLayout,
    to: vk::ImageLayout,
) {
    let (src_stage, src_access, _) = layout_use(from);
    let (dst_stage, _, dst_access) = layout_use(to);
    let barrier = vk::ImageMemoryBarrier::default()
        .src_access_mask(src_access)
        .dst_access_mask(dst_access)
        .old_layout(from)
        .new_layout(to)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(COLOR);
    // SAFETY: as the function says.
    unsafe { device.cmd_pipeline_barrier(command, src_stage, dst_stage, vk::DependencyFlags::empty(), &[], &[], &[barrier]) };
}

/// [`image_barrier`] for a texture, which then remembers its new layout.
///
/// # Safety
///
/// `command` is recording, and `texture` is alive.
unsafe fn transition(device: &ash::Device, command: vk::CommandBuffer, texture: &mut Texture, to: vk::ImageLayout) {
    // SAFETY: as the function says; the texture knows its layout.
    unsafe { image_barrier(device, command, texture.image, texture.layout, to) };
    texture.layout = to;
}

/// Puts round clips where the shaders read them, as the Direct3D renderer does: each one's edges, its top corners, its
/// bottom corners (bottom right first, as they go round), then whether it keeps what's drawn inside (1) or outside (-1) or
/// isn't there (0), and whether it's measured on the screen.
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

/// The shaders' constants for what's about to be drawn, laid out as `Frame` is in both `quad.hlsl` and `quad.glsl`.
fn constants_of(
    size: (u32, u32),
    atlas_size: u32,
    text_look: [f32; 2],
    matrix: Option<Matrix>,
    filter: Option<&Filter>,
    rounds: RoundSlots,
) -> [f32; 64] {
    let rows = filter.map_or([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0], |filter| filter.rows);
    let [a, b, c, d, e, f] = matrix.unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let sharpness = matrix.map_or(1.0, |[a, b, c, d, ..]| (a * d - b * c).abs().sqrt());
    let mut values = [0f32; 64];
    values[..32].copy_from_slice(&[
        size.0 as f32,
        size.1 as f32,
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
        text_look[0],
        text_look[1],
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
    values
}

/// What writes the constants for what's drawn next, and has it read from there.
type UseConstants<'a> = dyn FnMut(Option<Matrix>, Option<&Filter>, RoundSlots) + 'a;

/// Where a layer's top left is, on what's drawn to.
fn layer_origin(layer: &Layer, size: (u32, u32)) -> (i32, i32) {
    (layer.bounds.left.clamp(0, size.0 as i32), layer.bounds.top.clamp(0, size.1 as i32))
}

/// How big the image layers are drawn on has to be for all of these: a power of two each way, so that it's seldom made
/// again, but never bigger than the window.
fn layer_size_for(layers: &[Layer], size: (u32, u32)) -> (u32, u32) {
    let needed = layers.iter().fold((1, 1), |(width, height), layer| {
        let (x, y) = layer_origin(layer, size);
        (
            width.max((layer.bounds.right.min(size.0 as i32) - x).max(1) as u32),
            height.max((layer.bounds.bottom.min(size.1 as i32) - y).max(1) as u32),
        )
    });
    let bucket = |wanted: u32, limit: u32| wanted.max(1).checked_next_power_of_two().unwrap_or(limit).min(limit.max(1));
    (bucket(needed.0, size.0), bucket(needed.1, size.1))
}

/// A clip on the window, as a scissor on what's drawn on, whose top left is at `origin` on the window and which is `extent`
/// big, or nothing if none of it's on there.
fn scissor_of(clip: PixelRect, origin: (i32, i32), extent: (u32, u32)) -> Option<vk::Rect2D> {
    let left = (clip.left - origin.0).clamp(0, extent.0 as i32);
    let right = (clip.right - origin.0).clamp(0, extent.0 as i32);
    let top = (clip.top - origin.1).clamp(0, extent.1 as i32);
    let bottom = (clip.bottom - origin.1).clamp(0, extent.1 as i32);
    (left < right && top < bottom).then(|| vk::Rect2D {
        offset: vk::Offset2D { x: left, y: top },
        extent: vk::Extent2D { width: (right - left) as u32, height: (bottom - top) as u32 },
    })
}

/// What's made before the renderer itself, let go of again if what comes after fails.
struct Early {
    entry: ash::Entry,
    instance: ash::Instance,
    debug: Option<(ash::ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    surfaces: Option<ash::khr::surface::Instance>,
    surface: vk::SurfaceKHR,
}

impl Early {
    fn destroy(self) {
        // SAFETY: each was made here, and nothing made from them is left.
        unsafe {
            if let Some(surfaces) = &self.surfaces {
                surfaces.destroy_surface(self.surface, None);
            }
            if let Some((utils, messenger)) = &self.debug {
                utils.destroy_debug_utils_messenger(*messenger, None);
            }
            self.instance.destroy_instance(None);
        }
    }
}

/// Loads Vulkan and makes an instance with what drawing on `window` needs, and the validation layers if
/// `RSUI_VULKAN_VALIDATE=1` asks for them and they're installed.
fn make_instance(window: Option<Window>) -> Result<Early, RenderError> {
    // SAFETY: loading the system's Vulkan library runs nothing of ours, and it stays loaded while the entry lives.
    let entry = unsafe { ash::Entry::load() }.map_err(|error| failed(&format!("no Vulkan here ({error})")))?;
    // SAFETY: plain queries.
    let (available, layers) = unsafe {
        (entry.enumerate_instance_extension_properties(None)?, entry.enumerate_instance_layer_properties().unwrap_or_default())
    };
    let has = |name: &CStr| available.iter().any(|found| found.extension_name_as_c_str() == Ok(name));
    let mut extensions: Vec<*const c_char> = Vec::new();
    if let Some(window) = window {
        let surface_kind = match window {
            #[cfg(target_os = "linux")]
            Window::Wayland { .. } => ash::khr::wayland_surface::NAME,
            #[cfg(target_os = "android")]
            Window::Android { .. } => ash::khr::android_surface::NAME,
        };
        for name in [ash::khr::surface::NAME, surface_kind] {
            if !has(name) {
                return Err(failed(&format!("{} isn't there", name.to_string_lossy())));
            }
            extensions.push(name.as_ptr());
        }
    }
    const VALIDATION: &CStr = c"VK_LAYER_KHRONOS_validation";
    let validate = std::env::var_os("RSUI_VULKAN_VALIDATE").is_some_and(|value| value == "1")
        && layers.iter().any(|layer| layer.layer_name_as_c_str() == Ok(VALIDATION))
        && has(ash::ext::debug_utils::NAME);
    let layer_names = [VALIDATION.as_ptr()];
    if validate {
        extensions.push(ash::ext::debug_utils::NAME.as_ptr());
    }
    let app = vk::ApplicationInfo::default().application_name(c"rsui").engine_name(c"rsui").api_version(vk::API_VERSION_1_0);
    let info = vk::InstanceCreateInfo::default()
        .application_info(&app)
        .enabled_extension_names(&extensions)
        .enabled_layer_names(if validate { &layer_names[..] } else { &[] });
    // SAFETY: every name asked for was offered, and the description lives through the call.
    let instance = unsafe { entry.create_instance(&info, None)? };
    let debug = validate.then(|| {
        let utils = ash::ext::debug_utils::Instance::new(&entry, &instance);
        let info = vk::DebugUtilsMessengerCreateInfoEXT::default()
            .message_severity(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR)
            .message_type(
                vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                    | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                    | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
            )
            .pfn_user_callback(Some(report));
        // SAFETY: the extension was enabled, and `report` lives for good.
        unsafe { utils.create_debug_utils_messenger(&info, None) }.ok().map(|messenger| (utils, messenger))
    });
    let mut early = Early { entry, instance, debug: debug.flatten(), surfaces: None, surface: vk::SurfaceKHR::null() };
    if early.debug.is_some() {
        eprintln!("rsui vulkan: the validation layers are on");
    }
    if let Some(window) = window {
        let surfaces = ash::khr::surface::Instance::new(&early.entry, &early.instance);
        // SAFETY: the window's pointers are alive, as the caller promised, and the extension was enabled.
        let made = unsafe {
            match window {
                #[cfg(target_os = "linux")]
                Window::Wayland { display, surface } => ash::khr::wayland_surface::Instance::new(&early.entry, &early.instance)
                    .create_wayland_surface(&vk::WaylandSurfaceCreateInfoKHR::default().display(display).surface(surface), None),
                #[cfg(target_os = "android")]
                Window::Android { window } => ash::khr::android_surface::Instance::new(&early.entry, &early.instance)
                    .create_android_surface(&vk::AndroidSurfaceCreateInfoKHR::default().window(window), None),
            }
        };
        match made {
            Ok(surface) => {
                early.surface = surface;
                early.surfaces = Some(surfaces);
            }
            Err(error) => {
                early.destroy();
                return Err(error.into());
            }
        }
    }
    Ok(early)
}

/// The GPU to draw with, and its queue family that draws (and presents to the surface, if there is one). An integrated GPU
/// is the first choice, as it's the one a laptop's compositor is on and the one that wakes nothing else; a software one is
/// only taken if `software` allows it.
fn pick_device(early: &Early, software: bool) -> Result<(vk::PhysicalDevice, u32), RenderError> {
    // SAFETY: plain queries on a live instance.
    let devices = unsafe { early.instance.enumerate_physical_devices()? };
    let mut best: Option<(u32, vk::PhysicalDevice, u32)> = None;
    for physical in devices {
        // SAFETY: plain queries on a device the instance listed.
        let (properties, families, extensions) = unsafe {
            (
                early.instance.get_physical_device_properties(physical),
                early.instance.get_physical_device_queue_family_properties(physical),
                early.instance.enumerate_device_extension_properties(physical).unwrap_or_default(),
            )
        };
        let rank = match properties.device_type {
            vk::PhysicalDeviceType::INTEGRATED_GPU => 0,
            vk::PhysicalDeviceType::DISCRETE_GPU => 1,
            vk::PhysicalDeviceType::VIRTUAL_GPU => 2,
            vk::PhysicalDeviceType::CPU if software => 4,
            vk::PhysicalDeviceType::CPU => continue,
            _ => 3,
        };
        if early.surfaces.is_some()
            && !extensions.iter().any(|found| found.extension_name_as_c_str() == Ok(ash::khr::swapchain::NAME))
        {
            continue;
        }
        let family = families.iter().enumerate().position(|(at, family)| {
            family.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                && early.surfaces.as_ref().is_none_or(|surfaces| {
                    // SAFETY: a plain query on a live surface.
                    unsafe { surfaces.get_physical_device_surface_support(physical, at as u32, early.surface) }.unwrap_or(false)
                })
        });
        if let Some(family) = family
            && best.is_none_or(|(best_rank, ..)| rank < best_rank)
        {
            best = Some((rank, physical, family as u32));
        }
    }
    best.map(|(_, physical, family)| (physical, family)).ok_or_else(|| failed("no GPU here can draw on this"))
}

impl Vulkan {
    /// Draws on `window`, or into memory if there's none. `software` lets a GPU that's really the CPU be used.
    pub(crate) fn new(window: Option<Window>, size: (u32, u32), software: bool) -> Result<Self, RenderError> {
        let early = make_instance(window)?;
        let (physical, family) = match pick_device(&early, software) {
            Ok(found) => found,
            Err(error) => {
                early.destroy();
                return Err(error);
            }
        };
        let priorities = [1.0f32];
        let queues = [vk::DeviceQueueCreateInfo::default().queue_family_index(family).queue_priorities(&priorities)];
        let extensions = [ash::khr::swapchain::NAME.as_ptr()];
        let info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queues)
            .enabled_extension_names(if early.surfaces.is_some() { &extensions[..] } else { &[] });
        // SAFETY: the family and extension were checked, and the description lives through the call.
        let device = match unsafe { early.instance.create_device(physical, &info, None) } {
            Ok(device) => device,
            Err(error) => {
                early.destroy();
                return Err(error.into());
            }
        };
        // SAFETY: plain queries; the queue was asked for.
        let (queue, memory_types, properties) = unsafe {
            (
                device.get_device_queue(family, 0),
                early.instance.get_physical_device_memory_properties(physical),
                early.instance.get_physical_device_properties(physical),
            )
        };
        let swapchains = early.surfaces.is_some().then(|| ash::khr::swapchain::Device::new(&early.instance, &device));
        let Early { entry, instance, debug, surfaces, surface } = early;
        // Everything from here is let go of by `Drop` if making the rest fails, as null handles are nothing to destroy.
        let mut vulkan = Self {
            _entry: entry,
            instance,
            debug,
            surfaces,
            surface,
            physical,
            memory_types,
            largest_image: properties.limits.max_image_dimension2_d,
            device,
            queue,
            swapchains,
            swapchain: None,
            stale: true,
            acquired: None,
            format: vk::Format::B8G8R8A8_UNORM,
            color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
            see_through: false,
            size: (size.0.max(1), size.1.max(1)),
            clearing: vk::RenderPass::null(),
            keeping: vk::RenderPass::null(),
            set_layouts: [vk::DescriptorSetLayout::null(); 2],
            layout: vk::PipelineLayout::null(),
            quad_pipeline: vk::Pipeline::null(),
            image_pipeline: vk::Pipeline::null(),
            smooth: vk::Sampler::null(),
            exact: vk::Sampler::null(),
            pool: vk::DescriptorPool::null(),
            frame_set: vk::DescriptorSet::null(),
            frame_set_stale: true,
            layer_set: vk::DescriptorSet::null(),
            picture_pools: Vec::new(),
            commands: vk::CommandPool::null(),
            command: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            ready: vk::Semaphore::null(),
            in_flight: false,
            instances: None,
            constants: None,
            staging: None,
            atlas: None,
            atlas_wanted: 0,
            atlas_work: Vec::new(),
            pictures: HashMap::new(),
            picture_work: Vec::new(),
            layer: None,
            memory: None,
            retired: Vec::new(),
        };
        vulkan.choose_format()?;
        vulkan.make_pipelines(family)?;
        Ok(vulkan)
    }

    /// Picks what the swapchain's images are: blue, green, red and alpha a byte each, as Direct3D draws, if the surface
    /// offers it, and never a format that turns colors into sRGB on the way in, which would blend differently.
    fn choose_format(&mut self) -> Result<(), RenderError> {
        let Some(surfaces) = &self.surfaces else {
            return Ok(());
        };
        // SAFETY: a plain query on a live surface.
        let formats = unsafe { surfaces.get_physical_device_surface_formats(self.physical, self.surface)? };
        let srgb = vk::ColorSpaceKHR::SRGB_NONLINEAR;
        let chosen = [vk::Format::B8G8R8A8_UNORM, vk::Format::R8G8B8A8_UNORM]
            .into_iter()
            .find_map(|wanted| formats.iter().find(|found| found.format == wanted && found.color_space == srgb))
            .or_else(|| formats.iter().find(|found| found.format == vk::Format::UNDEFINED))
            .ok_or_else(|| failed("the surface takes no format this draws"))?;
        self.format = if chosen.format == vk::Format::UNDEFINED { vk::Format::B8G8R8A8_UNORM } else { chosen.format };
        self.color_space = chosen.color_space;
        Ok(())
    }

    /// Makes the render passes, the pipelines and what they read, and what each frame is recorded and waited on with.
    fn make_pipelines(&mut self, family: u32) -> Result<(), RenderError> {
        let device = &self.device;
        // SAFETY: each description is complete and lives through its call; what's made is stored at once, so `Drop` lets
        // go of it if something after fails.
        unsafe {
            let pass = |load: vk::AttachmentLoadOp, initial: vk::ImageLayout| {
                let attachments = [vk::AttachmentDescription {
                    format: self.format,
                    samples: vk::SampleCountFlags::TYPE_1,
                    load_op: load,
                    store_op: vk::AttachmentStoreOp::STORE,
                    stencil_load_op: vk::AttachmentLoadOp::DONT_CARE,
                    stencil_store_op: vk::AttachmentStoreOp::DONT_CARE,
                    initial_layout: initial,
                    final_layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    ..Default::default()
                }];
                let color = [vk::AttachmentReference { attachment: 0, layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL }];
                let subpasses = [vk::SubpassDescription::default()
                    .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                    .color_attachments(&color)];
                // What's drawn on may have been drawn on, read as a layer, or copied out before; it's drawn on after.
                let dependencies = [vk::SubpassDependency {
                    src_subpass: vk::SUBPASS_EXTERNAL,
                    dst_subpass: 0,
                    src_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                        | vk::PipelineStageFlags::FRAGMENT_SHADER
                        | vk::PipelineStageFlags::TRANSFER,
                    dst_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                    src_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                    dst_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                    dependency_flags: vk::DependencyFlags::empty(),
                }];
                device.create_render_pass(
                    &vk::RenderPassCreateInfo::default()
                        .attachments(&attachments)
                        .subpasses(&subpasses)
                        .dependencies(&dependencies),
                    None,
                )
            };
            self.clearing = pass(vk::AttachmentLoadOp::CLEAR, vk::ImageLayout::UNDEFINED)?;
            self.keeping = pass(vk::AttachmentLoadOp::LOAD, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)?;

            let frame_bindings = [
                vk::DescriptorSetLayoutBinding::default()
                    .binding(0)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT),
                vk::DescriptorSetLayoutBinding::default()
                    .binding(1)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            ];
            let picture_bindings = [vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)];
            self.set_layouts[0] = device
                .create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&frame_bindings), None)?;
            self.set_layouts[1] = device
                .create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&picture_bindings), None)?;
            self.layout =
                device.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&self.set_layouts), None)?;

            let sampler = |filter: vk::Filter, mipmap: vk::SamplerMipmapMode| {
                device.create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(filter)
                        .min_filter(filter)
                        .mipmap_mode(mipmap)
                        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .max_lod(vk::LOD_CLAMP_NONE),
                    None,
                )
            };
            self.smooth = sampler(vk::Filter::LINEAR, vk::SamplerMipmapMode::LINEAR)?;
            // A layer is put down one of its pixels to each of the window's, as the Direct3D renderer reads it.
            self.exact = sampler(vk::Filter::NEAREST, vk::SamplerMipmapMode::NEAREST)?;

            let sizes = [
                vk::DescriptorPoolSize { ty: vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC, descriptor_count: 1 },
                vk::DescriptorPoolSize { ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER, descriptor_count: 2 },
            ];
            self.pool =
                device.create_descriptor_pool(&vk::DescriptorPoolCreateInfo::default().max_sets(2).pool_sizes(&sizes), None)?;
            let sets = device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default().descriptor_pool(self.pool).set_layouts(&self.set_layouts),
            )?;
            (self.frame_set, self.layer_set) = (sets[0], sets[1]);

            let module = |bytes: &[u8]| -> Result<vk::ShaderModule, RenderError> {
                let words = ash::util::read_spv(&mut std::io::Cursor::new(bytes)).map_err(|_| failed("a shader isn't SPIR-V"))?;
                Ok(device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None)?)
            };
            let vertex = module(VERTEX_SHADER)?;
            let shaders = module(QUAD_SHADER).and_then(|quad| match module(IMAGE_SHADER) {
                Ok(image) => Ok((quad, image)),
                Err(error) => {
                    device.destroy_shader_module(quad, None);
                    Err(error)
                }
            });
            let made = shaders.and_then(|(quad, image)| {
                let made = self.make_pipeline_pair(vertex, quad, image);
                device.destroy_shader_module(quad, None);
                device.destroy_shader_module(image, None);
                made
            });
            device.destroy_shader_module(vertex, None);
            (self.quad_pipeline, self.image_pipeline) = made?;

            self.commands = device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
                    .queue_family_index(family),
                None,
            )?;
            self.command = device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(self.commands)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )?[0];
            self.fence = device.create_fence(&vk::FenceCreateInfo::default(), None)?;
            self.ready = device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?;
        }
        Ok(())
    }

    /// The two pipelines, which differ only in their pixel shader: one for rectangles and glyphs, one for images.
    fn make_pipeline_pair(
        &self,
        vertex: vk::ShaderModule,
        quad: vk::ShaderModule,
        image: vk::ShaderModule,
    ) -> Result<(vk::Pipeline, vk::Pipeline), RenderError> {
        // The quad's four numbers, then its glyph's, then its color, then its radius, as the Direct3D input layout reads it.
        let bindings = [vk::VertexInputBindingDescription { binding: 0, stride: 32, input_rate: vk::VertexInputRate::INSTANCE }];
        let attribute = |location, format, offset| vk::VertexInputAttributeDescription { location, binding: 0, format, offset };
        let attributes = [
            attribute(0, vk::Format::R32G32B32A32_SFLOAT, 0),
            attribute(1, vk::Format::R16G16B16A16_UINT, 16),
            attribute(2, vk::Format::R8G8B8A8_UNORM, 24),
            attribute(3, vk::Format::R32_SFLOAT, 28),
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&bindings)
            .vertex_attribute_descriptions(&attributes);
        let assembly = vk::PipelineInputAssemblyStateCreateInfo::default().topology(vk::PrimitiveTopology::TRIANGLE_STRIP);
        let viewport = vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(vk::SampleCountFlags::TYPE_1);
        // What's drawn goes over what's behind as much as it covers it, colors already multiplied by that. Direct3D blends
        // each part by its own cover, for ClearType's text, which isn't drawn here, so for everything else it's the same.
        let blend_attachments = [vk::PipelineColorBlendAttachmentState {
            blend_enable: vk::TRUE,
            src_color_blend_factor: vk::BlendFactor::ONE,
            dst_color_blend_factor: vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
            color_blend_op: vk::BlendOp::ADD,
            src_alpha_blend_factor: vk::BlendFactor::ONE,
            dst_alpha_blend_factor: vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
            alpha_blend_op: vk::BlendOp::ADD,
            color_write_mask: vk::ColorComponentFlags::RGBA,
        }];
        let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let stages = |pixel| {
            [
                vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::VERTEX).module(vertex).name(c"main"),
                vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::FRAGMENT).module(pixel).name(c"main"),
            ]
        };
        let (quad_stages, image_stages) = (stages(quad), stages(image));
        let info = vk::GraphicsPipelineCreateInfo::default()
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&assembly)
            .viewport_state(&viewport)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .color_blend_state(&blend)
            .dynamic_state(&dynamic)
            .layout(self.layout)
            .render_pass(self.clearing)
            .subpass(0);
        let infos = [info.stages(&quad_stages), info.stages(&image_stages)];
        // SAFETY: the descriptions are complete and live through the call.
        let made = unsafe { self.device.create_graphics_pipelines(vk::PipelineCache::null(), &infos, None) };
        match made {
            Ok(pipelines) => Ok((pipelines[0], pipelines[1])),
            Err((pipelines, error)) => {
                for pipeline in pipelines {
                    // SAFETY: what was made of them, if anything, is used by nothing.
                    unsafe { self.device.destroy_pipeline(pipeline, None) };
                }
                Err(error.into())
            }
        }
    }

    pub(crate) fn draws_to_window(&self) -> bool {
        self.surfaces.is_some()
    }

    /// Whether the next frame has to be drawn whatever it is, as the swapchain's to be made again.
    pub(crate) fn needs_whole_frame(&self) -> bool {
        self.surfaces.is_some() && self.stale
    }

    pub(crate) fn resize(&mut self, size: (u32, u32)) {
        let size = (size.0.max(1), size.1.max(1));
        if size == self.size {
            return;
        }
        self.size = size;
        self.stale = true;
        if let Some(memory) = self.memory.take() {
            self.retired.push(Retired::Canvas(memory.canvas));
            self.retired.push(Retired::Buffer(memory.readback));
        }
        if let Some(layer) = self.layer.take() {
            self.retired.push(Retired::Canvas(layer));
        }
    }

    pub(crate) fn set_see_through(&mut self, on: bool) {
        if on != self.see_through {
            self.see_through = on;
            self.stale = true;
        }
    }

    /// Lets go of what's only kept to save making it again: what layers are drawn on, the staging buffer, and what drawing
    /// into memory read back.
    pub(crate) fn trim(&mut self) {
        if let Some(layer) = self.layer.take() {
            self.retired.push(Retired::Canvas(layer));
        }
        if let Some(staging) = self.staging.take() {
            self.retired.push(Retired::Buffer(staging));
        }
        if self.surfaces.is_some()
            && let Some(memory) = self.memory.take()
        {
            self.retired.push(Retired::Canvas(memory.canvas));
            self.retired.push(Retired::Buffer(memory.readback));
        }
        // With nothing in flight, it can all go now.
        if !self.in_flight {
            self.let_go_of_retired();
        }
    }

    /// Takes what a scene has for the GPU to keep: glyphs for the atlas, how big that is, and images to show or let go of.
    /// They're put on the GPU with the next frame that's drawn. Says whether an image that was drawn last time (`last`)
    /// and is drawn again changed, which changes the frame even if nothing else does.
    pub(crate) fn take_uploads(&mut self, scene: &mut Scene, last: &[crate::scene::Batch]) -> bool {
        let size = scene.atlas_size.max(1);
        if size != self.atlas_wanted {
            self.atlas_wanted = size;
            self.atlas_work.push(AtlasWork::Size(size));
        }
        self.atlas_work.extend(scene.uploads.drain(..).map(AtlasWork::Glyphs));
        let referenced =
            |batches: &[crate::scene::Batch], id: u32| batches.iter().any(|batch| batch.image == id && batch.count != 0);
        let mut changed = false;
        for id in scene.dropped_images.drain(..) {
            if self.drop_picture(id) {
                changed |= referenced(last, id) && referenced(&scene.batches, id);
            }
        }
        for upload in scene.image_uploads.drain(..) {
            let known = self.pictures.contains_key(&upload.id) || self.picture_work.iter().any(|work| work.id == upload.id);
            changed |= known && referenced(last, upload.id) && referenced(&scene.batches, upload.id);
            self.picture_work.push(upload);
        }
        changed
    }

    /// Lets go of an image, and says whether there was one.
    pub(crate) fn drop_picture(&mut self, id: u32) -> bool {
        let waiting = self.picture_work.len();
        self.picture_work.retain(|work| work.id != id);
        let had = self.picture_work.len() != waiting;
        match self.pictures.remove(&id) {
            Some(picture) => {
                self.retired.push(Retired::Picture(picture));
                true
            }
            None => had,
        }
    }

    /// Waits for the frame in flight, if there is one, then lets go of what it was the last to use.
    fn finish_frame(&mut self) -> Result<(), RenderError> {
        if self.in_flight {
            // SAFETY: the fence was submitted with the frame.
            unsafe {
                self.device.wait_for_fences(&[self.fence], true, u64::MAX)?;
                self.device.reset_fences(&[self.fence])?;
            }
            self.in_flight = false;
        }
        self.let_go_of_retired();
        Ok(())
    }

    fn let_go_of_retired(&mut self) {
        for retired in std::mem::take(&mut self.retired) {
            match retired {
                Retired::Texture(texture) => self.destroy_texture(texture),
                Retired::Buffer(buffer) => self.destroy_buffer(buffer),
                Retired::Picture(picture) => {
                    // SAFETY: the set came from this pool, which lets sets go, and nothing in flight reads it.
                    unsafe {
                        let _ = self.device.free_descriptor_sets(picture.pool, &[picture.set]);
                    }
                    self.destroy_texture(picture.texture);
                }
                Retired::Canvas(canvas) => self.destroy_canvas(canvas),
            }
        }
    }

    fn destroy_texture(&self, texture: Texture) {
        // SAFETY: the texture is this renderer's, and nothing in flight uses it.
        unsafe {
            self.device.destroy_image_view(texture.view, None);
            self.device.destroy_image(texture.image, None);
            self.device.free_memory(texture.memory, None);
        }
    }

    fn destroy_buffer(&self, buffer: Buffer) {
        // SAFETY: the buffer is this renderer's, and nothing in flight uses it; freeing its memory unmaps it.
        unsafe {
            self.device.destroy_buffer(buffer.buffer, None);
            self.device.free_memory(buffer.memory, None);
        }
    }

    fn destroy_canvas(&self, canvas: Canvas) {
        // SAFETY: as for the texture.
        unsafe { self.device.destroy_framebuffer(canvas.framebuffer, None) };
        self.destroy_texture(canvas.texture);
    }

    fn memory_type(&self, allowed: u32, wanted: vk::MemoryPropertyFlags) -> Option<u32> {
        (0..self.memory_types.memory_type_count)
            .find(|&at| allowed & (1 << at) != 0 && self.memory_types.memory_types[at as usize].property_flags.contains(wanted))
    }

    /// A buffer the CPU can write (or read, as `cached` prefers), exactly `size` bytes, mapped.
    fn make_buffer(&self, size: u64, usage: vk::BufferUsageFlags, cached: bool) -> Result<Buffer, RenderError> {
        let size = size.max(4);
        // SAFETY: the descriptions are complete; each thing made is let go of if what follows fails.
        unsafe {
            let buffer = self.device.create_buffer(
                &vk::BufferCreateInfo::default().size(size).usage(usage).sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )?;
            let needs = self.device.get_buffer_memory_requirements(buffer);
            let visible = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
            let kind = cached
                .then(|| self.memory_type(needs.memory_type_bits, visible | vk::MemoryPropertyFlags::HOST_CACHED))
                .flatten()
                .or_else(|| self.memory_type(needs.memory_type_bits, visible));
            let Some(kind) = kind else {
                self.device.destroy_buffer(buffer, None);
                return Err(failed("no memory the CPU can reach"));
            };
            let memory = match self
                .device
                .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(needs.size).memory_type_index(kind), None)
            {
                Ok(memory) => memory,
                Err(error) => {
                    self.device.destroy_buffer(buffer, None);
                    return Err(error.into());
                }
            };
            let mapped = self
                .device
                .bind_buffer_memory(buffer, memory, 0)
                .and_then(|()| self.device.map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty()));
            match mapped {
                Ok(mapped) => Ok(Buffer { buffer, memory, mapped: mapped.cast(), size }),
                Err(error) => {
                    self.device.destroy_buffer(buffer, None);
                    self.device.free_memory(memory, None);
                    Err(error.into())
                }
            }
        }
    }

    /// An image on the GPU, `width` by `height`, with nothing in it yet.
    fn make_texture(
        &self,
        width: u32,
        height: u32,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
    ) -> Result<Texture, RenderError> {
        // SAFETY: the descriptions are complete; each thing made is let go of if what follows fails.
        unsafe {
            let image = self.device.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(format)
                    .extent(vk::Extent3D { width, height, depth: 1 })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(usage)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .initial_layout(vk::ImageLayout::UNDEFINED),
                None,
            )?;
            let needs = self.device.get_image_memory_requirements(image);
            let kind = self
                .memory_type(needs.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
                .or_else(|| self.memory_type(needs.memory_type_bits, vk::MemoryPropertyFlags::empty()));
            let memory = kind.ok_or_else(|| failed("no memory for an image")).and_then(|kind| {
                Ok(self.device.allocate_memory(
                    &vk::MemoryAllocateInfo::default().allocation_size(needs.size).memory_type_index(kind),
                    None,
                )?)
            });
            let memory = match memory {
                Ok(memory) => memory,
                Err(error) => {
                    self.device.destroy_image(image, None);
                    return Err(error);
                }
            };
            let view = self.device.bind_image_memory(image, memory, 0).and_then(|()| {
                self.device.create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(format)
                        .subresource_range(COLOR),
                    None,
                )
            });
            match view {
                Ok(view) => Ok(Texture { image, memory, view, width, height, layout: vk::ImageLayout::UNDEFINED }),
                Err(error) => {
                    self.device.destroy_image(image, None);
                    self.device.free_memory(memory, None);
                    Err(error.into())
                }
            }
        }
    }

    /// An image to draw on, the swapchain's format, `size` big, with a framebuffer.
    fn make_canvas(&self, size: (u32, u32), usage: vk::ImageUsageFlags) -> Result<Canvas, RenderError> {
        let texture = self.make_texture(size.0, size.1, self.format, usage | vk::ImageUsageFlags::COLOR_ATTACHMENT)?;
        let views = [texture.view];
        // SAFETY: the view is of an image made for this render pass's format.
        let made = unsafe {
            self.device.create_framebuffer(
                &vk::FramebufferCreateInfo::default()
                    .render_pass(self.clearing)
                    .attachments(&views)
                    .width(size.0)
                    .height(size.1)
                    .layers(1),
                None,
            )
        };
        match made {
            Ok(framebuffer) => Ok(Canvas { texture, framebuffer }),
            Err(error) => {
                self.destroy_texture(texture);
                Err(error.into())
            }
        }
    }

    /// Makes the swapchain again, at the window's size now, with the last one's images handed over to it.
    fn make_swapchain(&mut self) -> Result<(), RenderError> {
        let (Some(surfaces), Some(swapchains)) = (&self.surfaces, &self.swapchains) else {
            return Ok(());
        };
        // SAFETY: nothing may use the old swapchain's images while it's replaced.
        unsafe { self.device.device_wait_idle()? };
        // SAFETY: a plain query on a live surface.
        let capabilities = unsafe { surfaces.get_physical_device_surface_capabilities(self.physical, self.surface)? };
        // Android's driver says the size the window had a moment ago: a SurfaceView made shorter by the soft keyboard is still
        // its old height there, and a swapchain made that size is squashed into the window. Android takes any size and scales
        // it, so there the window's own size is used, as `ANativeWindow_getHeight` says it now.
        let ours = cfg!(target_os = "android") || capabilities.current_extent.width == u32::MAX;
        let extent = if ours {
            vk::Extent2D {
                width: self.size.0.clamp(capabilities.min_image_extent.width, capabilities.max_image_extent.width.max(1)),
                height: self.size.1.clamp(capabilities.min_image_extent.height, capabilities.max_image_extent.height.max(1)),
            }
        } else {
            capabilities.current_extent
        };
        let old = self.swapchain.take();
        if extent.width == 0 || extent.height == 0 {
            // A window with no size has nothing to draw on until it has one again.
            if let Some(old) = old {
                self.destroy_swapchain(old);
            }
            return Ok(());
        }
        // Two images, as the Direct3D renderer's flip chain has, if the driver will take that few.
        let mut count = capabilities.min_image_count.max(2);
        if capabilities.max_image_count > 0 {
            count = count.min(capabilities.max_image_count);
        }
        let alpha = capabilities.supported_composite_alpha;
        let composite = [
            (self.see_through, vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED),
            (true, vk::CompositeAlphaFlagsKHR::OPAQUE),
            (true, vk::CompositeAlphaFlagsKHR::INHERIT),
            (true, vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED),
            (true, vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED),
        ]
        .into_iter()
        .find_map(|(wanted, flag)| (wanted && alpha.contains(flag)).then_some(flag))
        .unwrap_or(vk::CompositeAlphaFlagsKHR::OPAQUE);
        // The window isn't turned in the shaders yet, so the system turns it, if it has to.
        let transform = if capabilities.supported_transforms.contains(vk::SurfaceTransformFlagsKHR::IDENTITY) {
            vk::SurfaceTransformFlagsKHR::IDENTITY
        } else {
            capabilities.current_transform
        };
        let info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(count)
            .image_format(self.format)
            .image_color_space(self.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(transform)
            .composite_alpha(composite)
            // FIFO, which every driver has, and which Mesa doesn't make four images of as it does MAILBOX.
            .present_mode(vk::PresentModeKHR::FIFO)
            .clipped(true)
            .old_swapchain(old.as_ref().map_or(vk::SwapchainKHR::null(), |old| old.handle));
        // SAFETY: the description is complete; the old swapchain is retired by this and destroyed after.
        let handle = unsafe { swapchains.create_swapchain(&info, None) };
        if let Some(old) = old {
            self.destroy_swapchain(old);
        }
        let handle = handle?;
        let mut chain =
            Swapchain { handle, extent, images: Vec::new(), views: Vec::new(), framebuffers: Vec::new(), done: Vec::new() };
        // SAFETY: the images are the swapchain's, and each view and framebuffer is made for them; on failure what was made
        // so far goes with the swapchain.
        let made = unsafe {
            (|| -> Result<(), RenderError> {
                chain.images = swapchains.get_swapchain_images(handle)?;
                for &image in &chain.images {
                    let view = self.device.create_image_view(
                        &vk::ImageViewCreateInfo::default()
                            .image(image)
                            .view_type(vk::ImageViewType::TYPE_2D)
                            .format(self.format)
                            .subresource_range(COLOR),
                        None,
                    )?;
                    chain.views.push(view);
                    let views = [view];
                    chain.framebuffers.push(
                        self.device.create_framebuffer(
                            &vk::FramebufferCreateInfo::default()
                                .render_pass(self.clearing)
                                .attachments(&views)
                                .width(extent.width)
                                .height(extent.height)
                                .layers(1),
                            None,
                        )?,
                    );
                    chain.done.push(self.device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?);
                }
                Ok(())
            })()
        };
        if let Err(error) = made {
            self.destroy_swapchain(chain);
            return Err(error);
        }
        self.swapchain = Some(chain);
        self.stale = false;
        Ok(())
    }

    fn destroy_swapchain(&self, chain: Swapchain) {
        // SAFETY: the device is idle, so nothing uses any of it.
        unsafe {
            for framebuffer in chain.framebuffers {
                self.device.destroy_framebuffer(framebuffer, None);
            }
            for view in chain.views {
                self.device.destroy_image_view(view, None);
            }
            for semaphore in chain.done {
                self.device.destroy_semaphore(semaphore, None);
            }
            if let Some(swapchains) = &self.swapchains {
                swapchains.destroy_swapchain(chain.handle, None);
            }
        }
    }

    /// Takes the next swapchain image to draw on, making the swapchain again first if it's out of date, or nothing if the
    /// window has no size to draw at.
    fn acquire(&mut self) -> Result<Option<u32>, RenderError> {
        if self.ready == vk::Semaphore::null() {
            return Err(RenderError::DeviceLost);
        }
        for _ in 0..2 {
            if self.stale || self.swapchain.is_none() {
                self.make_swapchain()?;
            }
            let (Some(chain), Some(swapchains)) = (&self.swapchain, &self.swapchains) else {
                return Ok(None);
            };
            // SAFETY: the semaphore isn't signalled: the frame that waited on it last is done.
            match unsafe { swapchains.acquire_next_image(chain.handle, u64::MAX, self.ready, vk::Fence::null()) } {
                Ok((index, suboptimal)) => {
                    // It can still be drawn on and shown; it's made again for the next frame.
                    self.stale |= suboptimal;
                    return Ok(Some(index));
                }
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => self.stale = true,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(None)
    }

    /// Makes sure there's a buffer at `slot` at least `size` bytes big, made again bigger if not, and says whether it was.
    fn make_room(
        &mut self,
        slot: fn(&mut Self) -> &mut Option<Buffer>,
        size: u64,
        usage: vk::BufferUsageFlags,
    ) -> Result<bool, RenderError> {
        if slot(self).as_ref().is_some_and(|buffer| buffer.size >= size) {
            return Ok(false);
        }
        // Twice what's needed, so that a scene growing a little at a time doesn't make it again each frame.
        let made = self.make_buffer(size.max(1).next_power_of_two(), usage, false)?;
        if let Some(old) = slot(self).replace(made) {
            self.retired.push(Retired::Buffer(old));
        }
        Ok(true)
    }

    /// Draws a scene: into the window's next swapchain image, presented by [`present`](Self::present), or into memory.
    pub(crate) fn draw(&mut self, scene: &Scene, clear: Color, text_look: [f32; 2], to_memory: bool) -> Result<(), RenderError> {
        self.finish_frame()?;
        let to_memory = to_memory || self.surfaces.is_none();
        if !to_memory {
            match self.acquire()? {
                Some(index) => self.acquired = Some(index),
                None => return Ok(()),
            }
        }
        let drawn = self.record_and_submit(scene, clear, text_look, to_memory);
        if drawn.is_err() {
            // Whatever was half done, start again: the swapchain, and the semaphore, which may be signalled with nothing
            // to wait on it.
            // SAFETY: waiting for everything, then replacing what nothing uses.
            unsafe {
                let _ = self.device.device_wait_idle();
                self.device.destroy_semaphore(self.ready, None);
                self.ready = self.device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None).unwrap_or_default();
            }
            self.in_flight = false;
            self.acquired = None;
            self.stale = true;
        }
        drawn
    }

    /// Shows what was drawn on the window, and says whether anything was. A swapchain that's gone out of date is made
    /// again for the next frame, which [`needs_whole_frame`](Self::needs_whole_frame) then says.
    pub(crate) fn present(&mut self) -> Result<bool, RenderError> {
        let (Some(index), Some(chain), Some(swapchains)) = (self.acquired.take(), &self.swapchain, &self.swapchains) else {
            return Ok(false);
        };
        let waits = [chain.done[index as usize]];
        let chains = [chain.handle];
        let indices = [index];
        let info = vk::PresentInfoKHR::default().wait_semaphores(&waits).swapchains(&chains).image_indices(&indices);
        // SAFETY: the image was acquired and drawn this frame, and its semaphore is signalled when it's done.
        match unsafe { swapchains.queue_present(self.queue, &info) } {
            Ok(suboptimal) => {
                self.stale |= suboptimal;
                Ok(true)
            }
            // Nothing was shown, so there's no frame for the window to wait for; the next one's drawn whole.
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.stale = true;
                Ok(false)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn record_and_submit(
        &mut self,
        scene: &Scene,
        clear: Color,
        text_look: [f32; 2],
        to_memory: bool,
    ) -> Result<(), RenderError> {
        let size = self.size;
        // Where it's drawn.
        if to_memory && self.memory.is_none() {
            let canvas = self.make_canvas(size, vk::ImageUsageFlags::TRANSFER_SRC)?;
            let readback =
                match self.make_buffer(u64::from(size.0) * u64::from(size.1) * 4, vk::BufferUsageFlags::TRANSFER_DST, true) {
                    Ok(buffer) => buffer,
                    Err(error) => {
                        self.destroy_canvas(canvas);
                        return Err(error);
                    }
                };
            self.memory = Some(MemoryTarget { canvas, readback, drawn: false });
        }
        let (framebuffer, target_image, extent) = match (to_memory, &self.memory, &self.swapchain, self.acquired) {
            (true, Some(memory), ..) => (memory.canvas.framebuffer, memory.canvas.texture.image, size),
            (false, _, Some(chain), Some(index)) => {
                (chain.framebuffers[index as usize], chain.images[index as usize], (chain.extent.width, chain.extent.height))
            }
            _ => return Err(failed("nothing to draw on")),
        };

        // The quads, then a quad for each layer, which puts it down.
        let layer_size = if scene.layers.is_empty() { (1, 1) } else { layer_size_for(&scene.layers, size) };
        if !scene.layers.is_empty()
            && self.layer.as_ref().is_none_or(|layer| layer.texture.width < layer_size.0 || layer.texture.height < layer_size.1)
        {
            let made = self.make_canvas(layer_size, vk::ImageUsageFlags::SAMPLED)?;
            if let Some(old) = self.layer.replace(made) {
                self.retired.push(Retired::Canvas(old));
            }
            let layer = self.layer.as_ref().expect("just made");
            let images = [vk::DescriptorImageInfo {
                sampler: self.exact,
                image_view: layer.texture.view,
                image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            }];
            let write = vk::WriteDescriptorSet::default()
                .dst_set(self.layer_set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&images);
            // SAFETY: nothing in flight reads the set, as the last frame is done.
            unsafe { self.device.update_descriptor_sets(&[write], &[]) };
        }
        let layer_size = self.layer.as_ref().map_or(layer_size, |layer| (layer.texture.width, layer.texture.height));
        let quad_count = scene.quads.len() as u64 + scene.layers.len() as u64;
        self.make_room(
            |this| &mut this.instances,
            (quad_count.max(FIRST_QUAD_CAPACITY)) * 32,
            vk::BufferUsageFlags::VERTEX_BUFFER,
        )?;
        // A slot for the frame, one for each batch whose constants change, and two for each layer, which puts it down
        // with its own and then goes back to what was being drawn.
        let slots = 1 + scene.batches.len() as u64 + 2 * scene.layers.len() as u64;
        self.frame_set_stale |=
            self.make_room(|this| &mut this.constants, slots * CONSTANTS_SIZE, vk::BufferUsageFlags::UNIFORM_BUFFER)?;
        {
            let instances = self.instances.as_ref().expect("made above");
            let layered: Vec<Quad> =
                scene.layers.iter().map(|layer| layer_quad(layer, layer_size, layer_origin(layer, size))).collect();
            // SAFETY: the buffer holds at least this many quads, and the GPU isn't reading it, as the last frame is done.
            unsafe {
                std::ptr::copy_nonoverlapping(scene.quads.as_ptr().cast::<u8>(), instances.mapped, scene.quads.len() * 32);
                std::ptr::copy_nonoverlapping(
                    layered.as_ptr().cast::<u8>(),
                    instances.mapped.add(scene.quads.len() * 32),
                    layered.len() * 32,
                );
            }
        }

        let command = self.command;
        // SAFETY: the command buffer isn't in flight, as the last frame is done.
        unsafe {
            self.device.reset_command_buffer(command, vk::CommandBufferResetFlags::empty())?;
            self.device.begin_command_buffer(
                command,
                &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
        }
        self.record_uploads(command)?;
        if self.frame_set_stale {
            let (Some(constants), Some(atlas)) = (&self.constants, &self.atlas) else {
                return Err(failed("no atlas"));
            };
            let buffers = [vk::DescriptorBufferInfo { buffer: constants.buffer, offset: 0, range: CONSTANTS_SIZE }];
            let images = [vk::DescriptorImageInfo {
                sampler: self.smooth,
                image_view: atlas.view,
                image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            }];
            let writes = [
                vk::WriteDescriptorSet::default()
                    .dst_set(self.frame_set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                    .buffer_info(&buffers),
                vk::WriteDescriptorSet::default()
                    .dst_set(self.frame_set)
                    .dst_binding(1)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(&images),
            ];
            // SAFETY: nothing in flight reads the set, and the command buffer being recorded hasn't bound it yet.
            unsafe { self.device.update_descriptor_sets(&writes, &[]) };
            self.frame_set_stale = false;
        }
        let atlas_size = self.atlas.as_ref().map_or(1, |atlas| atlas.width);
        // SAFETY: everything recorded is this renderer's own and stays alive until the frame's fence says it's done.
        unsafe {
            self.record_scene(command, scene, clear, text_look, atlas_size, framebuffer, extent, layer_size);
            if to_memory {
                let memory = self.memory.as_mut().expect("made above");
                image_barrier(
                    &self.device,
                    command,
                    target_image,
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                );
                let region = vk::BufferImageCopy {
                    buffer_offset: 0,
                    buffer_row_length: 0,
                    buffer_image_height: 0,
                    image_subresource: COLOR_LAYERS,
                    image_offset: vk::Offset3D::default(),
                    image_extent: vk::Extent3D { width: size.0, height: size.1, depth: 1 },
                };
                self.device.cmd_copy_image_to_buffer(
                    command,
                    target_image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    memory.readback.buffer,
                    &[region],
                );
                let readable = vk::BufferMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::HOST_READ)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .buffer(memory.readback.buffer)
                    .offset(0)
                    .size(vk::WHOLE_SIZE);
                self.device.cmd_pipeline_barrier(
                    command,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::HOST,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[readable],
                    &[],
                );
                memory.drawn = true;
            } else {
                image_barrier(
                    &self.device,
                    command,
                    target_image,
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    vk::ImageLayout::PRESENT_SRC_KHR,
                );
            }
            self.device.end_command_buffer(command)?;
            let commands = [command];
            let waits = [self.ready];
            let stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
            let done = match (to_memory, &self.swapchain, self.acquired) {
                (false, Some(chain), Some(index)) => Some(chain.done[index as usize]),
                _ => None,
            };
            let signals = done.map(|done| [done]);
            let mut submit = vk::SubmitInfo::default().command_buffers(&commands);
            if let Some(signals) = &signals {
                submit = submit.wait_semaphores(&waits).wait_dst_stage_mask(&stages).signal_semaphores(signals);
            }
            self.device.queue_submit(self.queue, &[submit], self.fence)?;
        }
        self.in_flight = true;
        // A big staging buffer, for a big image, isn't kept for the next.
        if self.staging.as_ref().is_some_and(|staging| staging.size > STAGING_KEPT)
            && let Some(staging) = self.staging.take()
        {
            self.retired.push(Retired::Buffer(staging));
        }
        Ok(())
    }

    /// Records what the atlas and images need before this frame: the atlas made bigger (or smaller), glyphs put in it, and
    /// images made, changed or replaced, all through one staging buffer.
    fn record_uploads(&mut self, command: vk::CommandBuffer) -> Result<(), RenderError> {
        let align = |bytes: usize| (bytes as u64 + 3) & !3;
        let staged: u64 = self
            .atlas_work
            .iter()
            .map(|work| match work {
                AtlasWork::Glyphs(upload) => align(upload.coverage.len()),
                AtlasWork::Size(_) => 0,
            })
            .chain(self.picture_work.iter().map(|work| align(work.pixels.len())))
            .sum();
        if staged > 0 {
            self.make_room(|this| &mut this.staging, staged, vk::BufferUsageFlags::TRANSFER_SRC)?;
        }
        let staging =
            self.staging.as_ref().map_or((vk::Buffer::null(), std::ptr::null_mut()), |staging| (staging.buffer, staging.mapped));
        let mut at = 0u64;
        // Copies the bytes into the staging buffer, and says where they went.
        let mut stage = |bytes: &[u8]| -> u64 {
            let offset = at;
            // SAFETY: the buffer was made big enough for everything staged this frame, and the GPU isn't reading it.
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), staging.1.add(offset as usize), bytes.len()) };
            at += align(bytes.len());
            offset
        };

        let work = std::mem::take(&mut self.atlas_work);
        if self.atlas.is_none() && !work.iter().any(|work| matches!(work, AtlasWork::Size(_))) {
            self.atlas_work.push(AtlasWork::Size(1));
        }
        for work in work.into_iter().chain(std::mem::take(&mut self.atlas_work)) {
            match work {
                AtlasWork::Size(size) => {
                    if self.atlas.as_ref().is_some_and(|atlas| atlas.width == size) {
                        continue;
                    }
                    let usage =
                        vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::TRANSFER_SRC;
                    let mut texture = self.make_texture(size, size, vk::Format::R8_UNORM, usage)?;
                    // SAFETY: both images are alive and recorded into this frame only.
                    unsafe {
                        transition(&self.device, command, &mut texture, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                        self.device.cmd_clear_color_image(
                            command,
                            texture.image,
                            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                            &vk::ClearColorValue { float32: [0.0; 4] },
                            &[COLOR],
                        );
                        if let Some(mut old) = self.atlas.take() {
                            if old.width < size {
                                // What's in the old atlas stays where it is, in the top left of the new.
                                transition(&self.device, command, &mut old, vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
                                transition(&self.device, command, &mut texture, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                                let region = vk::ImageCopy {
                                    src_subresource: COLOR_LAYERS,
                                    src_offset: vk::Offset3D::default(),
                                    dst_subresource: COLOR_LAYERS,
                                    dst_offset: vk::Offset3D::default(),
                                    extent: vk::Extent3D { width: old.width, height: old.height, depth: 1 },
                                };
                                self.device.cmd_copy_image(
                                    command,
                                    old.image,
                                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                                    texture.image,
                                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                                    &[region],
                                );
                            }
                            self.retired.push(Retired::Texture(old));
                        }
                        // Glyphs copied in after go where the clear and the copy were.
                        transition(&self.device, command, &mut texture, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                    }
                    self.atlas = Some(texture);
                    self.frame_set_stale = true;
                }
                AtlasWork::Glyphs(upload) => {
                    let Some(atlas) = self.atlas.as_mut() else { continue };
                    let height = upload.height.min(atlas.height.saturating_sub(upload.y));
                    if upload.width == 0
                        || height == 0
                        || upload.x + upload.width > atlas.width
                        || upload.coverage.len() < upload.width as usize * upload.height as usize
                    {
                        continue;
                    }
                    let offset = stage(&upload.coverage);
                    // SAFETY: the region is inside the atlas, and the staging buffer holds its bytes, a row of `width`
                    // after another.
                    unsafe {
                        if atlas.layout != vk::ImageLayout::TRANSFER_DST_OPTIMAL {
                            transition(&self.device, command, atlas, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                        }
                        let region = vk::BufferImageCopy {
                            buffer_offset: offset,
                            buffer_row_length: 0,
                            buffer_image_height: 0,
                            image_subresource: COLOR_LAYERS,
                            image_offset: vk::Offset3D { x: upload.x as i32, y: upload.y as i32, z: 0 },
                            image_extent: vk::Extent3D { width: upload.width, height, depth: 1 },
                        };
                        self.device.cmd_copy_buffer_to_image(
                            command,
                            staging.0,
                            atlas.image,
                            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                            &[region],
                        );
                    }
                }
            }
        }
        if let Some(atlas) = self.atlas.as_mut().filter(|atlas| atlas.layout != vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL) {
            // SAFETY: the atlas is alive and recorded into this frame.
            unsafe { transition(&self.device, command, atlas, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL) };
        }

        for upload in std::mem::take(&mut self.picture_work) {
            let (width, height) = (upload.width, upload.height);
            if width == 0 || height == 0 || upload.pixels.len() < width as usize * height as usize * 4 {
                continue;
            }
            // Part of an image that changes, as a canvas does, where it changed.
            if let (Some((x, y)), Some(picture)) = (upload.changes, self.pictures.get_mut(&upload.id)) {
                let texture = &mut picture.texture;
                let height = height.min(texture.height.saturating_sub(y));
                if height == 0 || x + width > texture.width {
                    continue;
                }
                let offset = stage(&upload.pixels);
                // SAFETY: the region is inside the image, and the staging buffer holds its pixels.
                unsafe {
                    transition(&self.device, command, texture, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                    let region = vk::BufferImageCopy {
                        buffer_offset: offset,
                        buffer_row_length: 0,
                        buffer_image_height: 0,
                        image_subresource: COLOR_LAYERS,
                        image_offset: vk::Offset3D { x: x as i32, y: y as i32, z: 0 },
                        image_extent: vk::Extent3D { width, height, depth: 1 },
                    };
                    self.device.cmd_copy_buffer_to_image(
                        command,
                        staging.0,
                        texture.image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &[region],
                    );
                    transition(&self.device, command, texture, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
                }
                continue;
            }
            // An image too big for this GPU just doesn't show, as text and everything else still can.
            if width > self.largest_image || height > self.largest_image {
                continue;
            }
            let Ok(mut texture) = self.make_texture(
                width,
                height,
                vk::Format::R8G8B8A8_UNORM,
                vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
            ) else {
                continue;
            };
            let (pool, set) = match self.picture_set(texture.view) {
                Ok(found) => found,
                Err(error) => {
                    self.destroy_texture(texture);
                    return Err(error);
                }
            };
            let offset = stage(&upload.pixels);
            // SAFETY: the image was just made this size, and the staging buffer holds its pixels.
            unsafe {
                transition(&self.device, command, &mut texture, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                let region = vk::BufferImageCopy {
                    buffer_offset: offset,
                    buffer_row_length: 0,
                    buffer_image_height: 0,
                    image_subresource: COLOR_LAYERS,
                    image_offset: vk::Offset3D::default(),
                    image_extent: vk::Extent3D { width, height, depth: 1 },
                };
                self.device.cmd_copy_buffer_to_image(
                    command,
                    staging.0,
                    texture.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
                transition(&self.device, command, &mut texture, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
            }
            if let Some(old) = self.pictures.insert(upload.id, Picture { texture, pool, set }) {
                self.retired.push(Retired::Picture(old));
            }
        }
        Ok(())
    }

    /// A descriptor set that reads an image through the smooth sampler, from a pool with room, made if none has.
    fn picture_set(&mut self, view: vk::ImageView) -> Result<(vk::DescriptorPool, vk::DescriptorSet), RenderError> {
        const SETS_IN_A_POOL: u32 = 64;
        let layouts = [self.set_layouts[1]];
        let mut found = None;
        for &pool in self.picture_pools.iter().rev() {
            // SAFETY: the pool and layout are this renderer's.
            if let Ok(sets) = unsafe {
                self.device.allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&layouts),
                )
            } {
                found = Some((pool, sets[0]));
                break;
            }
        }
        let (pool, set) = match found {
            Some(found) => found,
            None => {
                let sizes =
                    [vk::DescriptorPoolSize { ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER, descriptor_count: SETS_IN_A_POOL }];
                // SAFETY: the description is complete; the pool's kept, and let go of with the renderer.
                unsafe {
                    let pool = self.device.create_descriptor_pool(
                        &vk::DescriptorPoolCreateInfo::default()
                            .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
                            .max_sets(SETS_IN_A_POOL)
                            .pool_sizes(&sizes),
                        None,
                    )?;
                    self.picture_pools.push(pool);
                    let sets = self.device.allocate_descriptor_sets(
                        &vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&layouts),
                    )?;
                    (pool, sets[0])
                }
            }
        };
        let images = [vk::DescriptorImageInfo {
            sampler: self.smooth,
            image_view: view,
            image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        }];
        let write = vk::WriteDescriptorSet::default()
            .dst_set(set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(&images);
        // SAFETY: the set is new, so nothing reads it.
        unsafe { self.device.update_descriptor_sets(&[write], &[]) };
        Ok((pool, set))
    }

    /// Records drawing the scene's batches on what `framebuffer` is, as the CPU renderer's `draw_all` draws them: each
    /// batch's quads cut to its clip, with what's faded as a whole drawn on the layer first and put down after.
    ///
    /// # Safety
    ///
    /// `command` is recording; the buffers, sets and layer are ready for this scene.
    #[allow(clippy::too_many_arguments)]
    unsafe fn record_scene(
        &self,
        command: vk::CommandBuffer,
        scene: &Scene,
        clear: Color,
        text_look: [f32; 2],
        atlas_size: u32,
        framebuffer: vk::Framebuffer,
        extent: (u32, u32),
        layer_size: (u32, u32),
    ) {
        let device = &self.device;
        let size = self.size;
        let whole = PixelRect { left: 0, top: 0, right: size.0 as i32, bottom: size.1 as i32 };
        let constants = self.constants.as_ref().expect("made for the frame").mapped;
        let mut slots = 0u64;
        // Writes the constants for what's drawn next to a slot of their own, and reads them from there.
        let mut use_constants = |matrix: Option<Matrix>, filter: Option<&Filter>, rounds: RoundSlots| {
            let values = constants_of(size, atlas_size, text_look, matrix, filter, rounds);
            let offset = slots * CONSTANTS_SIZE;
            slots += 1;
            // SAFETY: there's a slot for every batch and layer, and the GPU isn't reading the buffer yet.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    values.as_ptr().cast::<u8>(),
                    constants.add(offset as usize),
                    CONSTANTS_SIZE as usize,
                );
                device.cmd_bind_descriptor_sets(
                    command,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.layout,
                    0,
                    &[self.frame_set],
                    &[offset as u32],
                );
            }
        };
        let viewport = |x: f32, y: f32| {
            let whole = vk::Viewport { x, y, width: size.0 as f32, height: size.1 as f32, min_depth: 0.0, max_depth: 1.0 };
            // SAFETY: as the function says.
            unsafe { device.cmd_set_viewport(command, 0, &[whole]) };
        };
        // Begins drawing on what `framebuffer` is, cleared to `color` first if there is one, or kept as it is.
        let begin = |framebuffer: vk::Framebuffer, extent: (u32, u32), color: Option<[f32; 4]>| {
            let clears = color.map(|color| [vk::ClearValue { color: vk::ClearColorValue { float32: color } }]);
            let info = vk::RenderPassBeginInfo::default()
                .render_pass(if color.is_some() { self.clearing } else { self.keeping })
                .framebuffer(framebuffer)
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D::default(),
                    extent: vk::Extent2D { width: extent.0, height: extent.1 },
                })
                .clear_values(clears.as_ref().map_or(&[], |clears| &clears[..]));
            // SAFETY: as the function says.
            unsafe { device.cmd_begin_render_pass(command, &info, vk::SubpassContents::INLINE) };
        };
        let clear_color = clear.premultiplied().map(|part| f32::from(part) / 255.0);
        // SAFETY: as the function says.
        unsafe {
            begin(framebuffer, extent, Some(clear_color));
            viewport(0.0, 0.0);
            device.cmd_bind_pipeline(command, vk::PipelineBindPoint::GRAPHICS, self.quad_pipeline);
            let instances = self.instances.as_ref().expect("made for the frame");
            device.cmd_bind_vertex_buffers(command, 0, &[instances.buffer], &[0]);
        }
        use_constants(None, None, NO_ROUND_CLIPS);
        let (mut transform, mut filter, mut round) = (0u16, 0u16, 0u16);
        let mut drawing_image = false;
        let mut next_layer = 0;
        let mut in_layer: Option<usize> = None;
        let layer = self.layer.as_ref();
        // Puts down the layer that's been drawn on, at its opacity, with its quad, after the scene's own.
        let put_down = |at: usize, use_constants: &mut UseConstants<'_>| {
            let Some(canvas) = layer else { return };
            let layer = &scene.layers[at];
            // SAFETY: as the function says; the layer was drawn on in the pass that ends here.
            unsafe {
                device.cmd_end_render_pass(command);
                image_barrier(
                    device,
                    command,
                    canvas.texture.image,
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                );
                begin(framebuffer, extent, None);
                viewport(0.0, 0.0);
            }
            use_constants(None, None, NO_ROUND_CLIPS);
            // SAFETY: as the function says.
            unsafe {
                device.cmd_bind_pipeline(command, vk::PipelineBindPoint::GRAPHICS, self.image_pipeline);
                device.cmd_bind_descriptor_sets(command, vk::PipelineBindPoint::GRAPHICS, self.layout, 1, &[self.layer_set], &[]);
                if let Some(scissor) = scissor_of(intersect(layer.bounds, whole), (0, 0), extent) {
                    device.cmd_set_scissor(command, 0, &[scissor]);
                    device.cmd_draw(command, 4, 1, 0, (scene.quads.len() + at) as u32);
                }
            }
        };
        for (index, batch) in scene.batches.iter().enumerate() {
            let index = index as u32;
            if let Some(at) = in_layer.filter(|&at| scene.layers[at].end <= index) {
                in_layer = None;
                put_down(at, &mut use_constants);
                // What was being drawn with has to be said again.
                (transform, filter, round, drawing_image) = (u16::MAX, u16::MAX, u16::MAX, true);
            }
            if in_layer.is_none()
                && let Some(canvas) = layer
                && scene.layers.get(next_layer).is_some_and(|layer| layer.first == index)
            {
                let (dx, dy) = layer_origin(&scene.layers[next_layer], size);
                // SAFETY: as the function says. The layer's pixels map to the window's from its top left, as the Direct3D
                // renderer's viewport does it, so the shaders still work in the window's pixels.
                unsafe {
                    device.cmd_end_render_pass(command);
                    begin(canvas.framebuffer, layer_size, Some([0.0; 4]));
                    viewport(-(dx as f32), -(dy as f32));
                }
                in_layer = Some(next_layer);
                next_layer += 1;
            }
            let clip = intersect(batch.clip, whole);
            if is_empty(clip) || batch.count == 0 {
                continue;
            }
            let (origin, target) = match in_layer {
                Some(at) => (layer_origin(&scene.layers[at], size), layer_size),
                None => ((0, 0), extent),
            };
            let Some(scissor) = scissor_of(clip, origin, target) else { continue };
            let picture = if batch.image != 0 {
                match self.pictures.get(&batch.image) {
                    Some(picture) => Some(picture),
                    None => continue,
                }
            } else {
                None
            };
            if batch.transform != transform || batch.filter != filter || batch.round != round {
                (transform, filter, round) = (batch.transform, batch.filter, batch.round);
                use_constants(
                    transform.checked_sub(1).and_then(|at| scene.transforms.get(usize::from(at))).copied(),
                    filter.checked_sub(1).and_then(|at| scene.filters.get(usize::from(at))),
                    round_clips_of(batch, &scene.round_clips),
                );
            }
            // SAFETY: as the function says.
            unsafe {
                match picture {
                    Some(picture) => {
                        if !drawing_image {
                            device.cmd_bind_pipeline(command, vk::PipelineBindPoint::GRAPHICS, self.image_pipeline);
                            drawing_image = true;
                        }
                        device.cmd_bind_descriptor_sets(
                            command,
                            vk::PipelineBindPoint::GRAPHICS,
                            self.layout,
                            1,
                            &[picture.set],
                            &[],
                        );
                    }
                    None if drawing_image => {
                        device.cmd_bind_pipeline(command, vk::PipelineBindPoint::GRAPHICS, self.quad_pipeline);
                        drawing_image = false;
                    }
                    None => {}
                }
                device.cmd_set_scissor(command, 0, &[scissor]);
                device.cmd_draw(command, 4, batch.count, 0, batch.first);
            }
        }
        if let Some(at) = in_layer {
            put_down(at, &mut use_constants);
        }
        // SAFETY: as the function says.
        unsafe { device.cmd_end_render_pass(command) };
    }

    /// What was last drawn into memory, as red, green, blue and alpha for each pixel, in rows from the top.
    pub(crate) fn read_pixels(&self) -> Result<Vec<u8>, RenderError> {
        let pixels = self.size.0 as usize * self.size.1 as usize * 4;
        let Some(memory) = self.memory.as_ref().filter(|memory| memory.drawn) else {
            if self.surfaces.is_some() {
                return Err(failed("what's drawn on a window can't be read back"));
            }
            return Ok(vec![0; pixels]);
        };
        if self.in_flight {
            // SAFETY: the fence was submitted with the frame; it's reset when the next starts.
            unsafe { self.device.wait_for_fences(&[self.fence], true, u64::MAX)? };
        }
        // SAFETY: the frame that copied into the buffer is done, and the buffer is as big as the picture.
        let read = unsafe { std::slice::from_raw_parts(memory.readback.mapped, pixels) };
        let mut out = read.to_vec();
        if self.format == vk::Format::B8G8R8A8_UNORM {
            for pixel in out.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }
        Ok(out)
    }
}

impl Drop for Vulkan {
    fn drop(&mut self) {
        // SAFETY: once the device is idle, nothing uses anything here, and each is let go of once; null handles are
        // nothing to destroy.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.in_flight = false;
            if let Some(chain) = self.swapchain.take() {
                self.destroy_swapchain(chain);
            }
            for buffer in [self.instances.take(), self.constants.take(), self.staging.take()].into_iter().flatten() {
                self.retired.push(Retired::Buffer(buffer));
            }
            if let Some(atlas) = self.atlas.take() {
                self.retired.push(Retired::Texture(atlas));
            }
            for (_, picture) in std::mem::take(&mut self.pictures) {
                self.retired.push(Retired::Picture(picture));
            }
            if let Some(layer) = self.layer.take() {
                self.retired.push(Retired::Canvas(layer));
            }
            if let Some(memory) = self.memory.take() {
                self.retired.push(Retired::Canvas(memory.canvas));
                self.retired.push(Retired::Buffer(memory.readback));
            }
            self.let_go_of_retired();
            let device = &self.device;
            device.destroy_semaphore(self.ready, None);
            device.destroy_fence(self.fence, None);
            device.destroy_command_pool(self.commands, None);
            for pool in self.picture_pools.drain(..) {
                device.destroy_descriptor_pool(pool, None);
            }
            device.destroy_descriptor_pool(self.pool, None);
            device.destroy_pipeline(self.quad_pipeline, None);
            device.destroy_pipeline(self.image_pipeline, None);
            device.destroy_pipeline_layout(self.layout, None);
            for layout in self.set_layouts {
                device.destroy_descriptor_set_layout(layout, None);
            }
            device.destroy_sampler(self.smooth, None);
            device.destroy_sampler(self.exact, None);
            device.destroy_render_pass(self.clearing, None);
            device.destroy_render_pass(self.keeping, None);
            device.destroy_device(None);
            if let Some(surfaces) = &self.surfaces {
                surfaces.destroy_surface(self.surface, None);
            }
            if let Some((utils, messenger)) = &self.debug {
                utils.destroy_debug_utils_messenger(*messenger, None);
            }
            self.instance.destroy_instance(None);
        }
    }
}
