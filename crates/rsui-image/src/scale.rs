//! Shrinks an image as its rows arrive, so that the whole of it never has to be in memory.

use std::sync::Arc;

use crate::color::ColorTransform;

/// Takes rows of red, green, blue and alpha, and builds a smaller image from them. Each new pixel is
/// the average of exactly what it covers in the old one.
pub(crate) struct Scaler {
    source: (u32, u32),
    target: (u32, u32),
    pixels: Vec<u8>,
    /// How far down the new image the next old row starts.
    across_row: Vec<f32>,
    /// What's been added up so far for the next new row.
    building: Vec<f32>,
    next_source_row: u32,
    next_target_row: u32,
    /// What to do to each row's colors, if they're not what a screen shows already.
    transform: Option<Arc<ColorTransform>>,
    changed_row: Vec<u8>,
}

impl Scaler {
    pub(crate) fn new(source: (u32, u32), target: (u32, u32)) -> Self {
        let target = (target.0.min(source.0).max(1), target.1.min(source.1).max(1));
        let scales = target != source;
        let width = target.0 as usize * 4;
        Self {
            source,
            target,
            pixels: vec![0; width * target.1 as usize],
            across_row: if scales { vec![0.0; width] } else { Vec::new() },
            building: if scales { vec![0.0; width] } else { Vec::new() },
            next_source_row: 0,
            next_target_row: 0,
            transform: None,
            changed_row: Vec::new(),
        }
    }

    /// Has each row's colors changed like this as it arrives. This has to happen before any shrinking,
    /// which has to be done on what'll be shown.
    pub(crate) fn set_transform(&mut self, transform: Option<ColorTransform>) {
        self.transform = transform.map(Arc::new);
    }

    pub(crate) fn set_shared_transform(&mut self, transform: Option<Arc<ColorTransform>>) {
        self.transform = transform;
    }

    pub(crate) fn size(&self) -> (u32, u32) {
        self.target
    }

    /// Takes the next row, whose colors aren't multiplied by its alpha yet.
    pub(crate) fn push(&mut self, row: &[u8]) {
        if let Some(transform) = self.transform.clone() {
            let mut changed = std::mem::take(&mut self.changed_row);
            changed.clear();
            changed.extend_from_slice(row);
            transform.apply_row(&mut changed);
            self.push_row(&changed);
            self.changed_row = changed;
        } else {
            self.push_row(row);
        }
    }

    fn push_row(&mut self, row: &[u8]) {
        if self.next_source_row >= self.source.1 {
            return;
        }
        let row_number = self.next_source_row;
        self.next_source_row += 1;
        if self.target == self.source {
            let start = row_number as usize * self.target.0 as usize * 4;
            let width = self.target.0 as usize * 4;
            for (to, from) in self.pixels[start..start + width]
                .chunks_exact_mut(4)
                .zip(row.chunks_exact(4))
            {
                let alpha = u32::from(from[3]);
                if alpha == 255 {
                    to.copy_from_slice(from);
                } else {
                    for part in 0..3 {
                        to[part] = ((u32::from(from[part]) * alpha + 127) / 255) as u8;
                    }
                    to[3] = from[3];
                }
            }
            return;
        }
        self.shrink_across(row);
        // Where this row falls in the new image, and how much of it goes to each new row.
        let per_row = self.target.1 as f32 / self.source.1 as f32;
        let start = row_number as f32 * per_row;
        let end = (row_number + 1) as f32 * per_row;
        let boundary = (self.next_target_row + 1) as f32;
        if end < boundary && row_number + 1 < self.source.1 {
            for (sum, value) in self.building.iter_mut().zip(&self.across_row) {
                *sum += value * per_row;
            }
            return;
        }
        // It finishes a new row, and whatever's left starts the one after.
        let first = (boundary - start).clamp(0.0, per_row);
        let rest = per_row - first;
        let width = self.target.0 as usize * 4;
        let at = self.next_target_row as usize * width;
        if at + width <= self.pixels.len() {
            for (index, (sum, value)) in self.building.iter_mut().zip(&self.across_row).enumerate()
            {
                let total = *sum + value * first;
                self.pixels[at + index] = (total + 0.5).clamp(0.0, 255.0) as u8;
                *sum = value * rest;
            }
        }
        self.next_target_row += 1;
    }

    /// Squeezes a row across, into `across_row`, multiplying colors by alpha on the way.
    fn shrink_across(&mut self, row: &[u8]) {
        self.across_row.fill(0.0);
        let per_pixel = self.target.0 as f32 / self.source.0 as f32;
        let last = self.target.0 as usize - 1;
        for (x, pixel) in row.chunks_exact(4).enumerate() {
            let alpha = f32::from(pixel[3]);
            let color = [
                f32::from(pixel[0]) * alpha / 255.0,
                f32::from(pixel[1]) * alpha / 255.0,
                f32::from(pixel[2]) * alpha / 255.0,
                alpha,
            ];
            let start = x as f32 * per_pixel;
            let end = start + per_pixel;
            let cell = (start as usize).min(last);
            let boundary = (cell + 1) as f32;
            if end <= boundary || cell == last {
                for (part, value) in color.iter().enumerate() {
                    self.across_row[cell * 4 + part] += value * per_pixel;
                }
            } else {
                let first = boundary - start;
                let second = end - boundary;
                for (part, value) in color.iter().enumerate() {
                    self.across_row[cell * 4 + part] += value * first;
                    self.across_row[(cell + 1) * 4 + part] += value * second;
                }
            }
        }
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.pixels
    }
}

/// Turns or flips an image, by what a camera's orientation number says, and returns its new size.
pub(crate) fn orient(pixels: &mut Vec<u8>, size: (u32, u32), orientation: u8) -> (u32, u32) {
    if orientation <= 1 || orientation > 8 {
        return size;
    }
    let (width, height) = (size.0 as usize, size.1 as usize);
    let swaps = orientation >= 5;
    let (new_width, new_height) = if swaps {
        (height, width)
    } else {
        (width, height)
    };
    let mut turned = vec![0u8; pixels.len()];
    for y in 0..height {
        for x in 0..width {
            let (to_x, to_y) = match orientation {
                2 => (width - 1 - x, y),
                3 => (width - 1 - x, height - 1 - y),
                4 => (x, height - 1 - y),
                5 => (y, x),
                6 => (height - 1 - y, x),
                7 => (height - 1 - y, width - 1 - x),
                _ => (y, width - 1 - x),
            };
            let from = (y * width + x) * 4;
            let to = (to_y * new_width + to_x) * 4;
            turned[to..to + 4].copy_from_slice(&pixels[from..from + 4]);
        }
    }
    *pixels = turned;
    (new_width as u32, new_height as u32)
}
