//! Opt-in logical work and owned-storage observations, not timings or residency.
//!
//! Counts and byte arithmetic saturate. Getters do not reset anything. Serial zero
//! means no attempt; replacing a Painter/Renderer starts a new epoch. Storage
//! excludes decoder, font/shaping, allocator backing and driver memory. Census
//! walks retained collections only on request; scalar getters only copy records.

use crate::scene::Filter;
use crate::transform::Matrix;
use crate::{Batch, PixelRect, Quad, Scene};

/// Number of operations/descriptors and their logical payload bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Payload {
    pub count: u64,
    pub bytes: u64,
}
impl Payload {
    pub(crate) fn add(&mut self, bytes: u64) {
        self.count = self.count.saturating_add(1);
        self.bytes = self.bytes.saturating_add(bytes);
    }
}

/// Command element counts, saved before Renderer swaps command vectors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Commands {
    pub quads: u64,
    pub batches: u64,
    pub transforms: u64,
    pub filters: u64,
}
impl Commands {
    pub(crate) fn of(scene: &Scene) -> Self {
        Self {
            quads: scene.quads.len() as u64,
            batches: scene.batches.len() as u64,
            transforms: scene.transforms.len() as u64,
            filters: scene.filters.len() as u64,
        }
    }
}

/// Existing atlas accessors; no CPU texture mirror is included.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Atlas {
    pub generation: u32,
    pub size: u32,
    pub entries: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScenePath {
    #[default]
    None,
    FullTraversal,
    CaretReuse,
    #[cfg(feature = "mutation-pilot")]
    OwnedPalette,
}

/// Latest successful scene. One reset per paint, accumulating both retry passes.
/// Failed optional caret lookup leaves this record unchanged. Exports are new
/// payloads entering Scene, not rasterizations or GPU submissions. Pending queues
/// include prior unsubmitted payloads; their descriptor lengths are scanned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SceneWork {
    pub serial: u64,
    pub path: ScenePath,
    pub paint_passes: u64,
    /// Actual enter callbacks, including clipped nodes; not unique/mounted nodes.
    pub node_enters: u64,
    /// Passing callbacks supplying ancestors of lifted content.
    pub ancestor_passes: u64,
    /// Paragraph callbacks, separate from ordinary enters.
    pub paragraph_callbacks: u64,
    /// Actual rich-layout rebuilds and validated reuse of the existing scratch.
    pub paragraph_layouts: u64,
    pub paragraph_reuses: u64,
    /// Replayed short/plain text drawing commands, including translated runs.
    pub text_run_reuses: u64,
    /// Complete short runs replayed at a potentially different whole-pixel origin.
    pub translated_text_run_reuses: u64,
    /// Sum of materialized list lengths, including discarded retry output.
    pub quads_built: u64,
    pub batches_built: u64,
    pub output: Commands,
    /// Bytes cloned from the four kept lists before optional caret removal.
    pub caret_copy_bytes: u64,
    pub glyph_exports: Payload,
    pub image_exports: Payload,
    pub pending_glyphs: Payload,
    pub pending_images: Payload,
    pub pending_drops: u64,
    pub atlas_before: Atlas,
    pub atlas_after: Atlas,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawOutcome {
    #[default]
    Incomplete,
    Unchanged,
    Full,
    Partial,
    Error,
}

/// Independent flags for existing full-fallback operands. Geometric damage can
/// also cover the whole target without any of these flags being true.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FullReasons {
    pub unknown_history: bool,
    pub always_redraw: bool,
    pub no_partial_context: bool,
    pub clear_changed: bool,
    pub atlas_generation_changed: bool,
    pub text_changed: bool,
    pub referenced_image_changed: bool,
    /// Existing combined filter/transform fallback, evaluated only once.
    pub transformed: bool,
}

/// Presentation requested by a successful draw, not proof present() was called.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Presentation {
    #[default]
    None,
    Full,
    Dirty(PixelRect),
}

/// Last draw attempt, reset before any resource work. Error retains completed
/// operations; Unchanged may include resource work. Successful payloads count
/// handoff to D3D, not GPU execution. Creations exclude constructor/resize work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawWork {
    pub serial: u64,
    pub outcome: DrawOutcome,
    /// Evaluated at the damage decision; default if an earlier resource call failed.
    pub full_reasons: FullReasons,
    pub input: Commands,
    /// Logical change, independent of two-buffer history. Empty before decision.
    pub change: PixelRect,
    /// Work region including previous_change for flipping; empty on Unchanged.
    pub draw_region: PixelRect,
    pub presentation: Presentation,
    /// Predicate evaluations in eligible partial batches (not all input quads).
    pub partial_quads_tested: u64,
    /// Instances in actual DrawInstanced calls; missing images are excluded.
    pub submitted_quads: u64,
    pub draw_calls: u64,
    pub quad_upload: Payload,
    pub glyph_upload: Payload,
    /// Successful RGBA texture creation payload, even if a later view call fails.
    pub image_upload: Payload,
    pub constant_upload: Payload,
    /// R8 GPU-to-GPU copies, separate from CPU upload bytes.
    pub atlas_copy: Payload,
    pub texture_creations: u64,
    pub buffer_creations: u64,
    pub view_creations: u64,
    pub shader_creations: u64,
    pub sampler_creations: u64,
}

/// Nonnegative area using widened subtraction, including extreme coordinates.
pub fn area(rect: PixelRect) -> u64 {
    (i64::from(rect.right) - i64::from(rect.left)).max(0) as u64
        * (i64::from(rect.bottom) - i64::from(rect.top)).max(0) as u64
}

/// Owned element storage in bytes, excluding vector header and allocator overhead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListStorage {
    pub len_bytes: u64,
    pub capacity_bytes: u64,
}
impl ListStorage {
    pub(crate) fn of<T>(list: &Vec<T>) -> Self {
        Self {
            len_bytes: bytes::<T>(list.len()),
            capacity_bytes: bytes::<T>(list.capacity()),
        }
    }
    pub(crate) fn add(&mut self, other: Self) {
        self.len_bytes = self.len_bytes.saturating_add(other.len_bytes);
        self.capacity_bytes = self.capacity_bytes.saturating_add(other.capacity_bytes);
    }
}
pub(crate) fn bytes<T>(count: usize) -> u64 {
    (count as u64).saturating_mul(size_of::<T>() as u64)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandStorage {
    pub quads: ListStorage,
    pub batches: ListStorage,
    pub transforms: ListStorage,
    pub filters: ListStorage,
}
impl CommandStorage {
    pub(crate) fn of(q: &Vec<Quad>, b: &Vec<Batch>, t: &Vec<Matrix>, f: &Vec<Filter>) -> Self {
        Self {
            quads: ListStorage::of(q),
            batches: ListStorage::of(b),
            transforms: ListStorage::of(t),
            filters: ListStorage::of(f),
        }
    }
    pub(crate) fn len_bytes(self) -> u64 {
        self.quads
            .len_bytes
            .saturating_add(self.batches.len_bytes)
            .saturating_add(self.transforms.len_bytes)
            .saturating_add(self.filters.len_bytes)
    }
}

/// Explicit census of Painter-owned lists. Map capacities are entries, never
/// guessed bucket bytes. Text-run keys' backing strings, atlas internal shelves/
/// maps/queues and Images internal collections are not observed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PainterStorage {
    pub comparison_spare: CommandStorage,
    pub scene: CommandStorage,
    pub kept_caret: CommandStorage,
    pub glyph_descriptors: ListStorage,
    pub glyph_payload: ListStorage,
    pub image_descriptors: ListStorage,
    pub image_payload: ListStorage,
    pub dropped_images: ListStorage,
    pub live_text_quads: ListStorage,
    pub spare_text_quads: ListStorage,
    pub spare_text_descriptors: ListStorage,
    /// Exact retained key capacity; no duplicate paragraph layout is kept.
    pub paragraph_key_capacity_bytes: u64,
    pub text_entries: u64,
    pub text_map_capacity: u64,
    pub scratch_glyphs: ListStorage,
    pub scratch_lines: ListStorage,
    pub clips: ListStorage,
    pub fades: ListStorage,
    pub turning: ListStorage,
    pub atlas: Atlas,
    /// Existing Images::gpu_bytes estimate, not actual GPU residency.
    pub image_payload_estimate: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RendererStorage {
    /// Logical RGBA opacity-intermediate payload, excluding driver padding.
    pub layer_target_capacity_bytes: u64,
    pub history: CommandStorage,
    pub partial_quads: ListStorage,
    pub partial_batches: ListStorage,
    pub quad_buffer_capacity_bytes: u64,
    /// Logical R8 texture area, excluding driver padding.
    pub atlas_capacity_bytes: u64,
    pub image_entries: u64,
    pub image_map_capacity: u64,
}

impl Default for DrawWork {
    fn default() -> Self {
        Self {
            serial: Default::default(),
            outcome: Default::default(),
            full_reasons: Default::default(),
            input: Default::default(),
            change: PixelRect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            draw_region: PixelRect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            presentation: Default::default(),
            partial_quads_tested: Default::default(),
            submitted_quads: Default::default(),
            draw_calls: Default::default(),
            quad_upload: Default::default(),
            glyph_upload: Default::default(),
            image_upload: Default::default(),
            constant_upload: Default::default(),
            atlas_copy: Default::default(),
            texture_creations: Default::default(),
            buffer_creations: Default::default(),
            view_creations: Default::default(),
            shader_creations: Default::default(),
            sampler_creations: Default::default(),
        }
    }
}
