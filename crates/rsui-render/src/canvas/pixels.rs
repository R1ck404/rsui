//! Laying rows of pixels on the canvas four at a time, with SSE2, which every x86-64 processor has. Each gives exactly what
//! laying them one at a time does (see `paint::blend_row` and `Drawing::blit`), with the same rounding: the parts of a pixel are
//! worked on as 16-bit numbers, where nothing these multiply comes to can overflow.

use super::paint::{div255, scale32};

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// How much of a pixel is covered, from 0 to 1, as a whole number out of 256.
#[inline(always)]
fn cover256(k: f32) -> u32 {
    // From 0 to 1, so this is a plain conversion, without the checks `as` adds.
    let scaled = k.max(0.0).min(1.0) * 256.0 + 0.5;
    // SAFETY: `max` leaves no NaN, so `scaled` is between 0.5 and 256.5, which an `i32` holds.
    unsafe { scaled.to_int_unchecked::<i32>() as u32 }
}

/// One pixel of `source-over`: `s` through `c` out of 256 of it, on `d`.
#[inline(always)]
fn over(d: u32, s: u32, c: u32) -> u32 {
    let s = if c >= 256 { s } else { scale32(s, c) };
    let sa = s >> 24;
    s + scale32(d, 256 - (sa + (sa >> 7)))
}

#[inline(always)]
fn get(out: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([out[i * 4], out[i * 4 + 1], out[i * 4 + 2], out[i * 4 + 3]])
}

#[inline(always)]
fn put(out: &mut [u8], i: usize, p: u32) {
    out[i * 4..i * 4 + 4].copy_from_slice(&p.to_le_bytes());
}

/// How much of a pixel is covered, from 0 to 255, as a whole number out of 256, which is what `cover256` makes of it over 255.
#[inline(always)]
fn byte256(k: u8) -> u32 {
    u32::from(k) + u32::from(k >> 7)
}

/// Lays `color`, packed and multiplied by alpha, on a row of the canvas through how much of each pixel it covers, as
/// `source-over` does.
pub fn blend_solid(out: &mut [u8], cover: &[f32], color: u32) {
    let n = cover.len().min(out.len() / 4);
    // SAFETY: each is asked for four coverages below `n`.
    #[cfg(target_arch = "x86_64")]
    let four = |i: usize| unsafe { covers(cover.as_ptr().add(i)) };
    #[cfg(not(target_arch = "x86_64"))]
    let four = |_: usize| ();
    solid_by(out, n, color, four, |j| cover256(cover[j]));
}

/// The same, with how much of each pixel is covered from 0 to 255, as text's coverage is.
pub fn blend_solid8(out: &mut [u8], cover: &[u8], color: u32) {
    let n = cover.len().min(out.len() / 4);
    // SAFETY: each is asked for four coverages below `n`.
    #[cfg(target_arch = "x86_64")]
    let four = |i: usize| unsafe {
        let zero = _mm_setzero_si128();
        let bytes = u32::from_le_bytes([cover[i], cover[i + 1], cover[i + 2], cover[i + 3]]);
        let c = _mm_unpacklo_epi16(_mm_unpacklo_epi8(_mm_cvtsi32_si128(bytes as i32), zero), zero);
        _mm_add_epi32(c, _mm_srli_epi32(c, 7))
    };
    #[cfg(not(target_arch = "x86_64"))]
    let four = |_: usize| ();
    solid_by(out, n, color, four, |j| byte256(cover[j]));
}

#[cfg(target_arch = "x86_64")]
type Four = __m128i;
#[cfg(not(target_arch = "x86_64"))]
type Four = ();

/// `blend_solid`, with each four pixels' coverage from `four` and each one's from `one`, out of 256.
#[inline(always)]
fn solid_by(out: &mut [u8], n: usize, color: u32, four: impl Fn(usize) -> Four, one: impl Fn(usize) -> u32) {
    let mut i = 0;
    #[cfg(not(target_arch = "x86_64"))]
    let _ = four;
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: SSE2 is always there on x86-64, and every load and store is of four pixels below `n`.
        unsafe {
            let zero = _mm_setzero_si128();
            let s16 = _mm_unpacklo_epi8(_mm_set1_epi32(color as i32), zero);
            let whole = _mm_set1_epi32(256);
            let full_inv = inverse(s16);
            let opaque = color >> 24 == 255;
            let solid = _mm_set1_epi32(color as i32);
            while i + 4 <= n {
                let c = four(i);
                let at = out.as_mut_ptr().add(i * 4) as *mut __m128i;
                let all = _mm_movemask_epi8(_mm_cmpeq_epi32(c, whole));
                if all == 0xFFFF {
                    if opaque {
                        _mm_storeu_si128(at, solid);
                    } else {
                        let d = _mm_loadu_si128(at);
                        let lo = _mm_add_epi16(s16, scale(_mm_unpacklo_epi8(d, zero), full_inv));
                        let hi = _mm_add_epi16(s16, scale(_mm_unpackhi_epi8(d, zero), full_inv));
                        _mm_storeu_si128(at, _mm_packus_epi16(lo, hi));
                    }
                } else if _mm_movemask_epi8(_mm_cmpeq_epi32(c, zero)) != 0xFFFF {
                    let (flo, fhi) = spread(c);
                    let d = _mm_loadu_si128(at);
                    _mm_storeu_si128(at, over4(d, scale(s16, flo), scale(s16, fhi)));
                }
                i += 4;
            }
        }
    }
    for j in i..n {
        let c = one(j);
        if c != 0 {
            let p = over(get(out, j), color, c);
            put(out, j, p);
        }
    }
}

/// Lays `color` on every pixel of a row, all of each covered, as `source-over` does.
pub fn blend_span(out: &mut [u8], color: u32) {
    let n = out.len() / 4;
    if color == 0 {
        return;
    }
    if color >> 24 == 255 {
        for px in out.chunks_exact_mut(4) {
            px.copy_from_slice(&color.to_le_bytes());
        }
        return;
    }
    let mut i = 0;
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: SSE2 is always there on x86-64, and every load and store is of four pixels below `n`.
        unsafe {
            let zero = _mm_setzero_si128();
            let s16 = _mm_unpacklo_epi8(_mm_set1_epi32(color as i32), zero);
            let keep = inverse(s16);
            while i + 4 <= n {
                let at = out.as_mut_ptr().add(i * 4) as *mut __m128i;
                let d = _mm_loadu_si128(at);
                let lo = _mm_add_epi16(s16, scale(_mm_unpacklo_epi8(d, zero), keep));
                let hi = _mm_add_epi16(s16, scale(_mm_unpackhi_epi8(d, zero), keep));
                _mm_storeu_si128(at, _mm_packus_epi16(lo, hi));
                i += 4;
            }
        }
    }
    for j in i..n {
        let p = over(get(out, j), color, 256);
        put(out, j, p);
    }
}

/// Lays `color` on the part of a rectangle from `[x0, y0]` to `[x1, y1]` that's in `reach`, the whole pixels it touches that
/// can be drawn on, of a canvas `width` across, as `source-over` does, with the same coverage as a row at a time from its
/// edges: each pixel's part of the rectangle across, times its row's part down. Says which rows it drew on.
pub fn rect_solid(pixels: &mut [u8], width: usize, [x0, y0, x1, y1]: [f32; 4], reach: [i32; 4], color: u32) -> Option<(i32, i32)> {
    let [rx0, ry0, rx1, ry1] = reach;
    let n = (rx1 - rx0) as usize;
    let across = |i: usize| {
        let px = rx0 + i as i32;
        ((px + 1) as f32).min(x1) - (px as f32).max(x0)
    };
    // The columns it covers all of are the ones between its edges.
    let mut lo = 0;
    while lo < n && across(lo) < 1.0 {
        lo += 1;
    }
    let mut hi = n;
    while hi > lo && across(hi - 1) < 1.0 {
        hi -= 1;
    }
    let mut rows: Option<(i32, i32)> = None;
    for py in ry0..ry1 {
        let part = ((py + 1) as f32).min(y1) - (py as f32).max(y0);
        if part <= 0.0 {
            continue;
        }
        rows = Some(rows.map_or((py, py + 1), |(a, _)| (a, py + 1)));
        let start = (py as usize * width + rx0 as usize) * 4;
        let row = &mut pixels[start..start + n * 4];
        for i in (0..lo).chain(hi..n) {
            let c = cover256(if part >= 1.0 { across(i) } else { across(i) * part });
            if c != 0 {
                let p = over(get(row, i), color, c);
                put(row, i, p);
            }
        }
        if lo < hi {
            let c = if part >= 1.0 { 256 } else { cover256(part) };
            if c != 0 {
                blend_span(&mut row[lo * 4..hi * 4], if c >= 256 { color } else { scale32(color, c) });
            }
        }
    }
    rows
}

/// A narrow rectangle in a color through a clip that's soft at its edges, as a chart's bars are drawn: the same as each of its
/// rows through [`blend_masked`], with each pixel's part of the rectangle across times its row's part down, but with what each
/// column lays on worked out once for the rows it covers all of, where the clip lets all of each pixel through. `clip` is the
/// mask, `bounds` across from `bounds[0], bounds[1]`, and `full` where it's all 255. Says which rows it drew on. It's for at
/// most [`NARROW`] columns.
#[allow(clippy::too_many_arguments)]
pub fn rect_masked(
    pixels: &mut [u8],
    width: usize,
    [x0, y0, x1, y1]: [f32; 4],
    reach: [i32; 4],
    color: u32,
    clip: &[u8],
    bounds: [i32; 4],
    full: [i32; 4],
) -> Option<(i32, i32)> {
    let [rx0, ry0, rx1, ry1] = reach;
    let n = (rx1 - rx0) as usize;
    debug_assert!(n <= NARROW);
    let n = n.min(NARROW);
    let mut across = [0.0f32; NARROW];
    // What each column lays on in a row it covers all of, as `blend_masked` works it out: the color through its coverage, and
    // how much of what's under it stays; or nothing.
    let mut lay = [(0u32, 0u32, false); NARROW];
    let s = [color & 255, (color >> 8) & 255, (color >> 16) & 255, color >> 24];
    let laid = |k: f32| {
        let c = (k * 255.0 + 0.5) as u32;
        let s8 = if c >= 255 { s } else { [div255(s[0] * c), div255(s[1] * c), div255(s[2] * c), div255(s[3] * c)] };
        (s8[0] | s8[1] << 8 | s8[2] << 16 | s8[3] << 24, 255 - s8[3])
    };
    for i in 0..n {
        let px = rx0 + i as i32;
        across[i] = ((px + 1) as f32).min(x1) - (px as f32).max(x0);
        if across[i] > 0.0 {
            let (s8, keep) = laid(across[i]);
            lay[i] = (s8, keep, true);
        }
    }
    let put = |d: &mut [u8], (s8, keep): (u32, u32)| {
        if keep == 0 {
            d.copy_from_slice(&s8.to_le_bytes());
            return;
        }
        for c in 0..4 {
            d[c] = (((s8 >> (8 * c)) & 255) + div255(u32::from(d[c]) * keep)).min(255) as u8;
        }
    };
    let clip_width = (bounds[2] - bounds[0]) as usize;
    // The columns where the clip lets all through.
    let (full_lo, full_hi) = ((full[0] - rx0).clamp(0, n as i32) as usize, (full[2] - rx0).clamp(0, n as i32) as usize);
    // The same four pixels at a time, for rows it covers all of where the clip lets all through all along: each column's color
    // and what stays of what's under it, with a column that's past the rectangle or not covered laying nothing and keeping all
    // of what's there, which is the same pixel again.
    let groups = n.div_ceil(4);
    #[cfg(target_arch = "x86_64")]
    let fours = {
        let mut fours = [(0u128, 0u128, 0u128); NARROW / 4];
        for (g, four) in fours.iter_mut().enumerate().take(groups) {
            let (mut s, mut keep) = ([0u32; 4], [255u16; 8 * 2]);
            for lane in 0..4 {
                let i = g * 4 + lane;
                if i < n && lay[i].2 {
                    s[lane] = lay[i].0;
                    keep[lane * 4..lane * 4 + 4].fill(lay[i].1 as u16);
                }
            }
            let s = u128::from(s[0]) | u128::from(s[1]) << 32 | u128::from(s[2]) << 64 | u128::from(s[3]) << 96;
            let pack = |k: &[u16]| k.iter().enumerate().fold(0u128, |a, (j, v)| a | u128::from(*v) << (16 * j));
            *four = (s, pack(&keep[..8]), pack(&keep[8..]));
        }
        fours
    };
    let mut rows: Option<(i32, i32)> = None;
    for py in ry0..ry1 {
        let part = ((py + 1) as f32).min(y1) - (py as f32).max(y0);
        if part <= 0.0 {
            continue;
        }
        rows = Some(rows.map_or((py, py + 1), |(a, _)| (a, py + 1)));
        let start = (py as usize * width + rx0 as usize) * 4;
        let all_down = py >= full[1] && py < full[3];
        #[cfg(target_arch = "x86_64")]
        if part >= 1.0 && all_down && full_lo == 0 && full_hi == n && start + groups * 16 <= pixels.len() {
            // SAFETY: SSE2 is always there on x86-64, and the stores are of the `groups * 16` bytes from `start`, which the
            // canvas has; those past the rectangle are put back as they were.
            unsafe {
                let zero = _mm_setzero_si128();
                for (g, (s, keep_lo, keep_hi)) in fours.iter().enumerate().take(groups) {
                    let at = pixels.as_mut_ptr().add(start + g * 16) as *mut __m128i;
                    let d = _mm_loadu_si128(at);
                    let s = _mm_loadu_si128((s as *const u128).cast());
                    let lo = div255x(_mm_mullo_epi16(_mm_unpacklo_epi8(d, zero), _mm_loadu_si128((keep_lo as *const u128).cast())));
                    let hi = div255x(_mm_mullo_epi16(_mm_unpackhi_epi8(d, zero), _mm_loadu_si128((keep_hi as *const u128).cast())));
                    _mm_storeu_si128(at, _mm_adds_epu8(s, _mm_packus_epi16(lo, hi)));
                }
            }
            continue;
        }
        let row = &mut pixels[start..start + n * 4];
        let mask = &clip[(py - bounds[1]) as usize * clip_width + (rx0 - bounds[0]) as usize..][..n];
        for i in 0..n {
            let d = &mut row[i * 4..i * 4 + 4];
            if part >= 1.0 && all_down && i >= full_lo && i < full_hi {
                let (s8, keep, any) = lay[i];
                if any {
                    put(d, (s8, keep));
                }
                continue;
            }
            let cover = if part >= 1.0 { across[i] } else { across[i] * part };
            let k = cover * (f32::from(mask[i]) / 255.0);
            if k <= 0.0 {
                continue;
            }
            put(d, laid(k));
        }
    }
    rows
}

/// The most columns [`rect_masked`] is for.
pub const NARROW: usize = 32;

/// The same, with a color of its own for each pixel, from `source`.
pub fn blend_shaded(out: &mut [u8], cover: &[f32], source: &[u32]) {
    let n = cover.len().min(out.len() / 4).min(source.len());
    let mut i = 0;
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: as in `blend_solid`, and `source` has `n` pixels too.
        unsafe {
            let zero = _mm_setzero_si128();
            let whole = _mm_set1_epi32(256);
            while i + 4 <= n {
                let c = covers(cover.as_ptr().add(i));
                let s = _mm_loadu_si128(source.as_ptr().add(i) as *const __m128i);
                let at = out.as_mut_ptr().add(i * 4) as *mut __m128i;
                if _mm_movemask_epi8(_mm_cmpeq_epi32(c, whole)) == 0xFFFF {
                    let d = _mm_loadu_si128(at);
                    _mm_storeu_si128(at, over4(d, _mm_unpacklo_epi8(s, zero), _mm_unpackhi_epi8(s, zero)));
                } else if _mm_movemask_epi8(_mm_cmpeq_epi32(c, zero)) != 0xFFFF {
                    let (flo, fhi) = spread(c);
                    let d = _mm_loadu_si128(at);
                    _mm_storeu_si128(at, over4(d, scale(_mm_unpacklo_epi8(s, zero), flo), scale(_mm_unpackhi_epi8(s, zero), fhi)));
                }
                i += 4;
            }
        }
    }
    for j in i..n {
        let c = cover256(cover[j]);
        if c != 0 {
            let p = over(get(out, j), source[j], c);
            put(out, j, p);
        }
    }
}

/// Lays a row on the canvas through a clip that covers only part of some pixels, as `source-over` does there: how much of each
/// pixel is covered times how much of it the clip lets through, `clip`, from 0 to 255, then the color through that out of 255,
/// `s + d * (255 - sa) / 255`, rounded as Skia does. `source` is each pixel's color, or `None` for `color` everywhere. `clip`
/// is `None` where the clip lets all of each pixel through, which is the same as 255s and needn't be read.
pub fn blend_masked(out: &mut [u8], cover: &[f32], clip: Option<&[u8]>, color: u32, source: Option<&[u32]>) {
    let n = cover.len().min(out.len() / 4).min(clip.map_or(usize::MAX, <[u8]>::len)).min(source.map_or(usize::MAX, <[u32]>::len));
    let mut i = 0;
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: SSE2 is always there on x86-64, and every load and store is of four pixels below `n`.
        unsafe {
            let zero = _mm_setzero_si128();
            let all255 = _mm_set1_epi16(255);
            let solid = _mm_set1_epi32(color as i32);
            while i + 4 <= n {
                // As `Clip::at` has it, a byte over 255, times the coverage, then out of 255, rounded. 255 over 255 is 1, so
                // without a clip it's the coverage.
                let k = match clip {
                    Some(clip) => {
                        let m = u32::from_le_bytes([clip[i], clip[i + 1], clip[i + 2], clip[i + 3]]);
                        let m = _mm_cvtepi32_ps(_mm_unpacklo_epi16(_mm_unpacklo_epi8(_mm_cvtsi32_si128(m as i32), zero), zero));
                        _mm_mul_ps(_mm_loadu_ps(cover.as_ptr().add(i)), _mm_div_ps(m, _mm_set1_ps(255.0)))
                    }
                    None => _mm_loadu_ps(cover.as_ptr().add(i)),
                };
                let k = _mm_and_ps(k, _mm_cmpgt_ps(k, _mm_setzero_ps()));
                let c = _mm_cvttps_epi32(_mm_add_ps(_mm_mul_ps(k, _mm_set1_ps(255.0)), _mm_set1_ps(0.5)));
                if _mm_movemask_epi8(_mm_cmpeq_epi32(c, zero)) == 0xFFFF {
                    i += 4;
                    continue;
                }
                let s = match source {
                    Some(source) => _mm_loadu_si128(source.as_ptr().add(i) as *const __m128i),
                    None => solid,
                };
                let (flo, fhi) = spread(c);
                let slo = div255x(_mm_mullo_epi16(_mm_unpacklo_epi8(s, zero), flo));
                let shi = div255x(_mm_mullo_epi16(_mm_unpackhi_epi8(s, zero), fhi));
                let at = out.as_mut_ptr().add(i * 4) as *mut __m128i;
                let d = _mm_loadu_si128(at);
                let dlo = div255x(_mm_mullo_epi16(_mm_unpacklo_epi8(d, zero), _mm_sub_epi16(all255, alphas(slo))));
                let dhi = div255x(_mm_mullo_epi16(_mm_unpackhi_epi8(d, zero), _mm_sub_epi16(all255, alphas(shi))));
                _mm_storeu_si128(at, _mm_adds_epu8(_mm_packus_epi16(slo, shi), _mm_packus_epi16(dlo, dhi)));
                i += 4;
            }
        }
    }
    for j in i..n {
        let k = clip.map_or(cover[j], |clip| cover[j] * (f32::from(clip[j]) / 255.0));
        if k <= 0.0 {
            continue;
        }
        let s = source.map_or(color, |source| source[j]);
        let s = [s & 255, (s >> 8) & 255, (s >> 16) & 255, s >> 24];
        let c = (k * 255.0 + 0.5) as u32;
        let s8 = if c >= 255 { s } else { [div255(s[0] * c), div255(s[1] * c), div255(s[2] * c), div255(s[3] * c)] };
        let d = &mut out[j * 4..j * 4 + 4];
        if s8[3] == 255 {
            d.copy_from_slice(&[s8[0] as u8, s8[1] as u8, s8[2] as u8, 255]);
            continue;
        }
        let keep = 255 - s8[3];
        for c in 0..4 {
            d[c] = (s8[c] + div255(u32::from(d[c]) * keep)).min(255) as u8;
        }
    }
}

/// Lays a row of a picture's pixels on the canvas pixel for pixel, as `source-over` does, with `alpha8` out of 255 of each, as
/// `drawImage()` of a picture that isn't moved off whole pixels or resized does: `s + d * (255 - sa) / 255`, rounded as Skia does.
pub fn blit_row(dest: &mut [u8], source: &[u8], alpha8: u32) {
    let n = dest.len().min(source.len()) / 4;
    let mut i = 0;
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: SSE2 is always there on x86-64, and every load and store is of four pixels below `n`.
        unsafe {
            let zero = _mm_setzero_si128();
            let k = _mm_set1_epi16(alpha8 as i16);
            let all255 = _mm_set1_epi16(255);
            while i + 4 <= n {
                let s = _mm_loadu_si128(source.as_ptr().add(i * 4) as *const __m128i);
                // Nothing to lay on: every pixel's alpha is 0.
                if _mm_movemask_epi8(_mm_cmpeq_epi32(_mm_srli_epi32(s, 24), zero)) == 0xFFFF {
                    i += 4;
                    continue;
                }
                let at = dest.as_mut_ptr().add(i * 4) as *mut __m128i;
                let (mut slo, mut shi) = (_mm_unpacklo_epi8(s, zero), _mm_unpackhi_epi8(s, zero));
                if alpha8 < 255 {
                    slo = div255x(_mm_mullo_epi16(slo, k));
                    shi = div255x(_mm_mullo_epi16(shi, k));
                }
                let d = _mm_loadu_si128(at);
                let keep_lo = _mm_sub_epi16(all255, alphas(slo));
                let keep_hi = _mm_sub_epi16(all255, alphas(shi));
                let dlo = div255x(_mm_mullo_epi16(_mm_unpacklo_epi8(d, zero), keep_lo));
                let dhi = div255x(_mm_mullo_epi16(_mm_unpackhi_epi8(d, zero), keep_hi));
                // Each part is at most 255 once it's added up, as `min(255)` has it.
                let s8 = _mm_packus_epi16(slo, shi);
                _mm_storeu_si128(at, _mm_adds_epu8(s8, _mm_packus_epi16(dlo, dhi)));
                i += 4;
            }
        }
    }
    for j in i..n {
        let (d, s) = (&mut dest[j * 4..j * 4 + 4], &source[j * 4..j * 4 + 4]);
        let sa = u32::from(s[3]);
        if sa == 0 {
            continue;
        }
        if sa == 255 && alpha8 == 255 {
            d.copy_from_slice(s);
            continue;
        }
        let s8 = if alpha8 == 255 {
            [u32::from(s[0]), u32::from(s[1]), u32::from(s[2]), sa]
        } else {
            [div255(u32::from(s[0]) * alpha8), div255(u32::from(s[1]) * alpha8), div255(u32::from(s[2]) * alpha8), div255(sa * alpha8)]
        };
        let keep = 255 - s8[3];
        for c in 0..4 {
            d[c] = (s8[c] + div255(u32::from(d[c]) * keep)).min(255) as u8;
        }
    }
}

/// A row of a picture sampled between pixels, as `paint::image_row32` does it: `at` is where each sample is across, in steps of
/// 1/65536 of a pixel, going on by `step`, and `wy` how far down from `top` to `bottom` it is, out of 256. A sample's two pixels
/// across are clamped to `first..=last`. Across first, then down, each rounded, as `lerp32` does. Says how many it did, which is
/// all of `out`.
///
/// # Safety
///
/// `top` and `bottom` must have at least `last + 1` pixels, and `first <= last`.
#[cfg(target_arch = "x86_64")]
pub unsafe fn bilinear_run(top: &[u32], bottom: &[u32], (first, last): (i64, i64), at: &mut i64, step: i64, wy: u32, out: &mut [u32]) -> usize {
    use super::paint::lerp32;
    // SAFETY: SSE2 is always there on x86-64. Pixels `c` and `c + 1` are read only when `first <= c < last`.
    unsafe {
        let zero = _mm_setzero_si128();
        let half = _mm_set1_epi16(0x80);
        let down = _mm_unpacklo_epi64(_mm_set1_epi16((256 - wy) as i16), _mm_set1_epi16(wy as i16));
        let (t, b) = (top.as_ptr(), bottom.as_ptr());
        for px in out.iter_mut() {
            let x0 = *at >> 16;
            let wx = ((*at >> 8) & 255) as i16;
            *at += step;
            if x0 < first || x0 >= last {
                let c0 = x0.clamp(first, last) as usize;
                let c1 = (x0 + 1).clamp(first, last) as usize;
                let up = lerp32(top[c0], top[c1], wx as u32);
                let low = lerp32(bottom[c0], bottom[c1], wx as u32);
                *px = lerp32(up, low, wy);
                continue;
            }
            let c = x0 as usize;
            // Both pixels across, in the top row and the bottom one, as 16-bit parts.
            let above = _mm_unpacklo_epi8(_mm_loadl_epi64(t.add(c) as *const __m128i), zero);
            let below = _mm_unpacklo_epi8(_mm_loadl_epi64(b.add(c) as *const __m128i), zero);
            let across = _mm_unpacklo_epi64(_mm_set1_epi16(256 - wx), _mm_set1_epi16(wx));
            let (above, below) = (_mm_mullo_epi16(above, across), _mm_mullo_epi16(below, across));
            // Each row's two added, rounded: the top's in the low half and the bottom's in the high one.
            let sum = _mm_add_epi16(_mm_unpacklo_epi64(above, below), _mm_unpackhi_epi64(above, below));
            let rows = _mm_mullo_epi16(_mm_srli_epi16(_mm_add_epi16(sum, half), 8), down);
            let mixed = _mm_srli_epi16(_mm_add_epi16(_mm_add_epi16(rows, _mm_srli_si128(rows, 8)), half), 8);
            *px = _mm_cvtsi128_si32(_mm_packus_epi16(mixed, mixed)) as u32;
        }
        out.len()
    }
}

/// Goes through a row of pixels from `source` to `dest`, copying four at a time those whose alpha is 255, which multiplying
/// colors by alpha or taking it out leaves as they are, or every pixel with its alpha made 255 if `opaque`, and handing each
/// of the rest to `one`.
pub fn each_unless_opaque(dest: &mut [u8], source: &[u8], opaque: bool, mut one: impl FnMut(&mut [u8], &[u8])) {
    let n = dest.len().min(source.len()) / 4;
    let mut i = 0;
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: SSE2 is always there on x86-64, and every load and store is of four pixels below `n`.
        unsafe {
            let alpha = _mm_set1_epi32(0xFF00_0000u32 as i32);
            while i + 4 <= n {
                let p = _mm_loadu_si128(source.as_ptr().add(i * 4) as *const __m128i);
                if opaque {
                    _mm_storeu_si128(dest.as_mut_ptr().add(i * 4) as *mut __m128i, _mm_or_si128(p, alpha));
                } else if _mm_movemask_epi8(_mm_cmpeq_epi32(_mm_and_si128(p, alpha), alpha)) == 0xFFFF {
                    _mm_storeu_si128(dest.as_mut_ptr().add(i * 4) as *mut __m128i, p);
                } else {
                    for j in i..i + 4 {
                        one(&mut dest[j * 4..j * 4 + 4], &source[j * 4..j * 4 + 4]);
                    }
                }
                i += 4;
            }
        }
    }
    for j in i..n {
        one(&mut dest[j * 4..j * 4 + 4], &source[j * 4..j * 4 + 4]);
    }
}

/// Four coverages from 0 to 1 as whole numbers out of 256, as `cover256` makes each.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn covers(k: *const f32) -> __m128i {
    // SAFETY: the caller has four numbers at `k`. `max` with 0 second gives 0 for NaN, as `f32::max` does.
    unsafe {
        let k = _mm_min_ps(_mm_max_ps(_mm_loadu_ps(k), _mm_setzero_ps()), _mm_set1_ps(1.0));
        _mm_cvttps_epi32(_mm_add_ps(_mm_mul_ps(k, _mm_set1_ps(256.0)), _mm_set1_ps(0.5)))
    }
}

/// Four numbers, one for each pixel, as each pixel's four parts have them: the first two pixels', then the last two's.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn spread(c: __m128i) -> (__m128i, __m128i) {
    // SAFETY: SSE2 only.
    unsafe {
        let p = _mm_packs_epi32(c, c);
        let q = _mm_unpacklo_epi16(p, p);
        (_mm_unpacklo_epi32(q, q), _mm_unpackhi_epi32(q, q))
    }
}

/// Each part times `f` out of 256, rounded, as `scale32` does.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn scale(x: __m128i, f: __m128i) -> __m128i {
    // SAFETY: SSE2 only. A part is at most 255 and `f` at most 256, so this stays below 65536.
    unsafe { _mm_srli_epi16(_mm_add_epi16(_mm_mullo_epi16(x, f), _mm_set1_epi16(0x80)), 8) }
}

/// Each pixel's alpha, in all four of its parts.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn alphas(x: __m128i) -> __m128i {
    // SAFETY: SSE2 only.
    unsafe { _mm_shufflehi_epi16(_mm_shufflelo_epi16(x, 0xFF), 0xFF) }
}

/// How much of what's there `source-over` keeps under a pixel with this alpha, out of 256.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn inverse(s: __m128i) -> __m128i {
    // SAFETY: SSE2 only.
    unsafe {
        let a = alphas(s);
        _mm_sub_epi16(_mm_set1_epi16(256), _mm_add_epi16(a, _mm_srli_epi16(a, 7)))
    }
}

/// `source-over` of four pixels, with what's laid on as 16-bit parts, the first two pixels' and the last two's.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn over4(d: __m128i, slo: __m128i, shi: __m128i) -> __m128i {
    // SAFETY: SSE2 only.
    unsafe {
        let zero = _mm_setzero_si128();
        let lo = _mm_add_epi16(slo, scale(_mm_unpacklo_epi8(d, zero), inverse(slo)));
        let hi = _mm_add_epi16(shi, scale(_mm_unpackhi_epi8(d, zero), inverse(shi)));
        _mm_packus_epi16(lo, hi)
    }
}

/// `x / 255` for each part, rounded, as `div255` does, for `x` up to 255 * 255.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn div255x(x: __m128i) -> __m128i {
    // SAFETY: SSE2 only. `x + 128` is at most 65153, and adding a 256th of it stays below 65536.
    unsafe {
        let t = _mm_add_epi16(x, _mm_set1_epi16(128));
        _mm_srli_epi16(_mm_add_epi16(t, _mm_srli_epi16(t, 8)), 8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn random(seed: &mut u32) -> u32 {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *seed >> 8
    }

    /// A pixel with its colors multiplied by its alpha, as every pixel on a canvas is.
    fn premultiplied(seed: &mut u32) -> u32 {
        let a = match random(seed) % 4 {
            0 => 0,
            1 => 255,
            _ => random(seed) % 256,
        };
        let part = |seed: &mut u32| if a == 0 { 0 } else { random(seed) % (a + 1) };
        part(seed) | part(seed) << 8 | part(seed) << 16 | a << 24
    }

    fn row(seed: &mut u32, n: usize) -> (Vec<u8>, Vec<f32>, Vec<u32>) {
        let canvas: Vec<u8> = (0..n).flat_map(|_| premultiplied(seed).to_le_bytes()).collect();
        let cover: Vec<f32> = (0..n)
            .map(|_| match random(seed) % 4 {
                0 => 0.0,
                1 => 1.0,
                _ => (random(seed) % 1000) as f32 / 999.0,
            })
            .collect();
        let source: Vec<u32> = (0..n).map(|_| premultiplied(seed)).collect();
        (canvas, cover, source)
    }

    #[test]
    fn four_at_a_time_is_the_same_as_one_at_a_time() {
        let mut seed = 7;
        for n in [0, 1, 3, 4, 5, 8, 17, 64] {
            for _ in 0..50 {
                let (canvas, cover, source) = row(&mut seed, n);
                let color = premultiplied(&mut seed);
                let mut expected = canvas.clone();
                super::super::paint::blend_row(&mut expected, &cover, |_| color);
                let mut got = canvas.clone();
                blend_solid(&mut got, &cover, color);
                assert_eq!(got, expected, "solid {color:08x}");
                let mut expected = canvas.clone();
                super::super::paint::blend_row(&mut expected, &cover, |i| source[i]);
                let mut got = canvas.clone();
                blend_shaded(&mut got, &cover, &source);
                assert_eq!(got, expected, "shaded");
                // Runs of whole coverage, as the inside of a shape has.
                let whole = vec![1.0; n];
                let mut expected = canvas.clone();
                super::super::paint::blend_row(&mut expected, &whole, |_| color);
                let mut got = canvas.clone();
                blend_solid(&mut got, &whole, color);
                assert_eq!(got, expected, "whole {color:08x}");
                // Coverage as bytes, as text has it, is the same as each over 255.
                let bytes: Vec<u8> = (0..n).map(|_| (random(&mut seed) % 256) as u8).collect();
                let floats: Vec<f32> = bytes.iter().map(|b| f32::from(*b) / 255.0).collect();
                let mut expected = canvas.clone();
                super::super::paint::blend_row(&mut expected, &floats, |_| color);
                let mut got = canvas.clone();
                blend_solid8(&mut got, &bytes, color);
                assert_eq!(got, expected, "bytes {color:08x}");
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn sampling_between_pixels_is_the_same_as_one_part_at_a_time() {
        use super::super::paint::lerp32;
        let mut seed = 5;
        for width in [2usize, 3, 32] {
            let top: Vec<u32> = (0..width).map(|_| premultiplied(&mut seed)).collect();
            let bottom: Vec<u32> = (0..width).map(|_| premultiplied(&mut seed)).collect();
            for _ in 0..40 {
                let wy = random(&mut seed) % 256;
                let step = (random(&mut seed) % 200_000) as i64 - 100_000;
                let start = (random(&mut seed) % (width as u32 * 65536 + 131072)) as i64 - 65536;
                let (first, last) = (0i64, width as i64 - 1);
                let mut got = vec![0u32; 50];
                let mut at = start;
                unsafe { bilinear_run(&top, &bottom, (first, last), &mut at, step, wy, &mut got) };
                let mut at = start;
                let expected: Vec<u32> = (0..50)
                    .map(|_| {
                        let (x0, wx) = (at >> 16, ((at >> 8) & 255) as u32);
                        at += step;
                        let (c0, c1) = (x0.clamp(first, last) as usize, (x0 + 1).clamp(first, last) as usize);
                        lerp32(lerp32(top[c0], top[c1], wx), lerp32(bottom[c0], bottom[c1], wx), wy)
                    })
                    .collect();
                assert_eq!(got, expected);
            }
        }
    }

    #[test]
    fn through_a_clip_four_at_a_time_is_the_same_as_one_at_a_time() {
        let mut seed = 13;
        for n in [1, 4, 6, 16, 37] {
            for _ in 0..50 {
                let (canvas, cover, source) = row(&mut seed, n);
                let clip: Vec<u8> = (0..n).map(|_| [0u8, 255, (random(&mut seed) % 256) as u8][(random(&mut seed) % 3) as usize]).collect();
                let color = premultiplied(&mut seed);
                for shaded in [false, true] {
                    let mut got = canvas.clone();
                    blend_masked(&mut got, &cover, Some(&clip), color, shaded.then_some(&source[..]));
                    // What `draw_direct` did a pixel at a time.
                    let mut expected = canvas.clone();
                    for i in 0..n {
                        let k = cover[i] * (f32::from(clip[i]) / 255.0);
                        if k <= 0.0 {
                            continue;
                        }
                        let p = if shaded { source[i] } else { color };
                        let s8 = [p & 255, (p >> 8) & 255, (p >> 16) & 255, p >> 24];
                        let c = (k * 255.0 + 0.5) as u32;
                        let s8 = if c >= 255 { s8 } else { s8.map(|v| div255(v * c)) };
                        let d = &mut expected[i * 4..i * 4 + 4];
                        if s8[3] == 255 {
                            d.copy_from_slice(&[s8[0] as u8, s8[1] as u8, s8[2] as u8, 255]);
                            continue;
                        }
                        for c in 0..4 {
                            d[c] = (s8[c] + div255(u32::from(d[c]) * (255 - s8[3]))).min(255) as u8;
                        }
                    }
                    assert_eq!(got, expected, "{shaded} {color:08x}");
                }
            }
        }
    }

    #[test]
    fn no_clip_is_the_same_as_one_that_lets_all_through() {
        let mut seed = 29;
        for n in [1, 4, 7, 16, 33] {
            for _ in 0..50 {
                let (canvas, cover, source) = row(&mut seed, n);
                let color = premultiplied(&mut seed);
                for shaded in [false, true] {
                    let (mut got, mut expected) = (canvas.clone(), canvas.clone());
                    blend_masked(&mut got, &cover, None, color, shaded.then_some(&source[..]));
                    blend_masked(&mut expected, &cover, Some(&vec![255u8; n]), color, shaded.then_some(&source[..]));
                    assert_eq!(got, expected);
                }
            }
        }
    }

    #[test]
    fn a_narrow_rectangle_through_a_clip_is_the_same_as_its_rows() {
        let mut seed = 41;
        let (w, h) = (40usize, 30usize);
        for _ in 0..300 {
            let canvas: Vec<u8> = (0..w * h).flat_map(|_| premultiplied(&mut seed).to_le_bytes()).collect();
            let color = premultiplied(&mut seed);
            // A clip over all of it, soft round its edges, 255 inside `full`.
            let full = [2 + (random(&mut seed) % 6) as i32, 1 + (random(&mut seed) % 5) as i32, 30 + (random(&mut seed) % 9) as i32, 22 + (random(&mut seed) % 7) as i32];
            let clip: Vec<u8> = (0..w * h)
                .map(|i| {
                    let (x, y) = ((i % w) as i32, (i / w) as i32);
                    if x >= full[0] && x < full[2] && y >= full[1] && y < full[3] { 255 } else { (random(&mut seed) % 256) as u8 }
                })
                .collect();
            let f = |seed: &mut u32, most: u32| (random(seed) % (most * 100)) as f32 / 100.0;
            let (x0, y0) = (f(&mut seed, 36), f(&mut seed, 26));
            let (x1, y1) = ((x0 + f(&mut seed, 6)).min(w as f32), (y0 + f(&mut seed, 20)).min(h as f32));
            let reach = [x0.floor() as i32, y0.floor() as i32, (x1.ceil() as i32).min(w as i32), (y1.ceil() as i32).min(h as i32)];
            if reach[2] <= reach[0] || reach[3] <= reach[1] {
                continue;
            }
            let mut got = canvas.clone();
            let rows = rect_masked(&mut got, w, [x0, y0, x1, y1], reach, color, &clip, [0, 0, w as i32, h as i32], full);
            // What `draw_direct` does a row at a time.
            let mut expected = canvas.clone();
            let mut drawn = None;
            let across: Vec<f32> = (reach[0]..reach[2]).map(|px| ((px + 1) as f32).min(x1) - (px as f32).max(x0)).collect();
            for py in reach[1]..reach[3] {
                let part = ((py + 1) as f32).min(y1) - (py as f32).max(y0);
                if part <= 0.0 {
                    continue;
                }
                drawn = Some(drawn.map_or((py, py + 1), |(a, _): (i32, i32)| (a, py + 1)));
                let cover: Vec<f32> = if part >= 1.0 { across.clone() } else { across.iter().map(|a| a * part).collect() };
                let at = py as usize * w + reach[0] as usize;
                let n = across.len();
                blend_masked(&mut expected[at * 4..(at + n) * 4], &cover, Some(&clip[at..at + n]), color, None);
            }
            assert_eq!(rows, drawn);
            assert_eq!(got, expected, "{x0} {y0} {x1} {y1} {color:08x}");
        }
    }

    #[test]
    fn a_rectangle_is_the_same_as_its_rows() {
        let mut seed = 3;
        for _ in 0..200 {
            let (w, h) = (24usize, 20usize);
            let canvas: Vec<u8> = (0..w * h).flat_map(|_| premultiplied(&mut seed).to_le_bytes()).collect();
            let color = premultiplied(&mut seed);
            let at = |seed: &mut u32, most: f32| (random(seed) % 1000) as f32 / 1000.0 * most;
            let (x0, y0) = (at(&mut seed, 20.0) - 2.0, at(&mut seed, 16.0) - 2.0);
            let (x1, y1) = (x0 + at(&mut seed, 12.0), y0 + at(&mut seed, 12.0));
            let reach = [
                (x0.floor() as i32).max(0),
                (y0.floor() as i32).max(0),
                (x1.ceil() as i32).min(w as i32),
                (y1.ceil() as i32).min(h as i32),
            ];
            if reach[2] <= reach[0] || reach[3] <= reach[1] {
                continue;
            }
            let mut got = canvas.clone();
            rect_solid(&mut got, w, [x0, y0, x1, y1], reach, color);
            let mut expected = canvas.clone();
            let across: Vec<f32> = (reach[0]..reach[2]).map(|px| ((px + 1) as f32).min(x1) - (px as f32).max(x0)).collect();
            for py in reach[1]..reach[3] {
                let part = ((py + 1) as f32).min(y1) - (py as f32).max(y0);
                if part <= 0.0 {
                    continue;
                }
                let row: Vec<f32> = if part >= 1.0 { across.clone() } else { across.iter().map(|a| a * part).collect() };
                let start = (py as usize * w + reach[0] as usize) * 4;
                super::super::paint::blend_row(&mut expected[start..start + row.len() * 4], &row, |_| color);
            }
            assert_eq!(got, expected, "{x0} {y0} {x1} {y1} {color:08x}");
        }
    }

    #[test]
    fn blitting_four_at_a_time_is_the_same_as_one_at_a_time() {
        let mut seed = 11;
        for n in [1, 4, 7, 16, 33] {
            for alpha8 in [255, 254, 128, 1, 0] {
                let (canvas, _, source) = row(&mut seed, n);
                let source: Vec<u8> = source.iter().flat_map(|p| p.to_le_bytes()).collect();
                let mut got = canvas.clone();
                blit_row(&mut got, &source, alpha8);
                let mut expected = canvas.clone();
                for (d, s) in expected.chunks_exact_mut(4).zip(source.chunks_exact(4)) {
                    let sa = u32::from(s[3]);
                    if sa == 0 {
                        continue;
                    }
                    let s8 = [s[0], s[1], s[2], s[3]].map(|v| if alpha8 == 255 { u32::from(v) } else { div255(u32::from(v) * alpha8) });
                    let keep = 255 - s8[3];
                    for c in 0..4 {
                        d[c] = (s8[c] + div255(u32::from(d[c]) * keep)).min(255) as u8;
                    }
                }
                assert_eq!(got, expected, "alpha {alpha8}");
            }
        }
    }
}
