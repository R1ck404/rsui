//! Areas of the window, which round clips a batch is cut off by, and the quad that puts down what's faded as a whole:
//! what every renderer works out the same way, whichever system draws it.
// The CPU renderer draws every frame whole, so it has no use yet for what tells which parts changed.
#![cfg_attr(not(windows), allow(dead_code))]

use crate::scene::{Batch, Layer, PixelRect, Quad, RoundClip};
use crate::transform::Matrix;

/// Whether two areas have anything in common.
pub(crate) fn is_empty(area: PixelRect) -> bool {
    area.left >= area.right || area.top >= area.bottom
}

pub(crate) fn union(a: PixelRect, b: PixelRect) -> PixelRect {
    if is_empty(a) {
        return b;
    }
    if is_empty(b) {
        return a;
    }
    PixelRect {
        left: a.left.min(b.left),
        top: a.top.min(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    }
}

pub(crate) fn intersect(a: PixelRect, b: PixelRect) -> PixelRect {
    PixelRect {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.min(b.right),
        bottom: a.bottom.min(b.bottom),
    }
}

pub(crate) const NO_AREA: PixelRect = PixelRect {
    left: 0,
    top: 0,
    right: 0,
    bottom: 0,
};

/// The whole pixels a quad touches.
pub(crate) fn area_of(quad: &Quad) -> PixelRect {
    let [x, y, width, height] = quad.rect;
    PixelRect {
        left: x.floor() as i32,
        top: y.floor() as i32,
        right: (x + width).ceil() as i32,
        bottom: (y + height).ceil() as i32,
    }
}

/// The whole pixels a quad touches once it's been through `matrix`, as the vertex shader puts its corners, with a pixel more
/// all round for the edges that are smoothed where it's turned.
pub(crate) fn placed_area(quad: &Quad, matrix: Matrix) -> PixelRect {
    if matrix == crate::transform::IDENTITY {
        return area_of(quad);
    }
    let [x, y, width, height] = quad.rect;
    let corners = [(x, y), (x + width, y), (x, y + height), (x + width, y + height)]
        .map(|(x, y)| crate::transform::apply_matrix(matrix, x, y));
    let (mut left, mut top, mut right, mut bottom) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in corners {
        left = left.min(x);
        top = top.min(y);
        right = right.max(x);
        bottom = bottom.max(y);
    }
    // Something squashed to nothing, or that's gone to infinity, could be anywhere.
    if !(left.is_finite() && top.is_finite() && right.is_finite() && bottom.is_finite()) {
        const FAR: i32 = 1 << 24;
        return PixelRect {
            left: -FAR,
            top: -FAR,
            right: FAR,
            bottom: FAR,
        };
    }
    let far = (1u32 << 24) as f32;
    PixelRect {
        left: (left.max(-far).floor() as i32) - 1,
        top: (top.max(-far).floor() as i32) - 1,
        right: (right.min(far).ceil() as i32) + 1,
        bottom: (bottom.min(far).ceil() as i32) + 1,
    }
}

/// The round clips a batch is cut off by, as the shaders take them, filled from the first: the one it's in, then the one that's
/// in. A third around those is rare, and only the rectangle around it cuts off what's drawn. Each says whether it's measured on
/// the screen, as one made outside what turns or resizes the batch is. One made inside another transform than the batch's is
/// left out too, as only the rectangle around it can be cut to.
pub(crate) type RoundSlots = [Option<(RoundClip, bool)>; 2];

/// No round clips at all.
pub(crate) const NO_ROUND_CLIPS: RoundSlots = [None, None];

/// The round clips `batch` is cut off by: see [`RoundSlots`].
pub(crate) fn round_clips_of(batch: &Batch, clips: &[RoundClip]) -> RoundSlots {
    let get = |at: u16| usize::from(at).checked_sub(1).and_then(|at| clips.get(at)).copied();
    let inner = get(batch.round);
    let outer = inner.and_then(|inner| get(inner.outer));
    let mut slots = NO_ROUND_CLIPS;
    let mut free = slots.iter_mut();
    for clip in [inner, outer].into_iter().flatten() {
        let on_screen = match clip.transform {
            at if at == batch.transform => false,
            0 => true,
            _ => continue,
        };
        if let Some(slot) = free.next() {
            *slot = Some((clip, on_screen));
        }
    }
    slots
}

/// The quad that puts a layer down: all of what's drawn to, reading all of the layer, at its opacity.
pub(crate) fn layer_quad(layer: &Layer, size: (u32, u32), origin: (i32, i32)) -> Quad {
    let alpha = (layer.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    Quad {
        rect: [origin.0 as f32, origin.1 as f32, size.0 as f32, size.1 as f32],
        glyph: [0, 0, u16::MAX, u16::MAX],
        color: [alpha; 4],
        radius: 0.0,
    }
}

