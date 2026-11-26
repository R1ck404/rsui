//! Blurring, for shadows and `filter: blur()`, as three boxes one after another, which is what the filter spec has a Gaussian
//! blur as, and near what Skia does.

/// How far each of the three boxes reaches to the left and right, for this much spread.
pub fn boxes(sigma: f32) -> [(usize, usize); 3] {
    if !(sigma > 0.0) {
        return [(0, 0); 3];
    }
    let d = (sigma * 3.0 * (2.0 * std::f32::consts::PI).sqrt() / 4.0 + 0.5).floor() as usize;
    if d <= 1 {
        return [(0, 0); 3];
    }
    if d % 2 == 1 {
        let r = (d - 1) / 2;
        [(r, r); 3]
    } else {
        [(d / 2, d / 2 - 1), (d / 2 - 1, d / 2), (d / 2, d / 2)]
    }
}

/// How far a blur this spread reaches past what's blurred, each way.
pub fn reach(sigma: f32) -> usize {
    boxes(sigma).iter().map(|(left, right)| left.max(right)).sum::<usize>() + 1
}

/// Blurs `channels` numbers for each of `width` by `height` pixels, with nothing past the edges.
pub fn blur(values: &mut [f32], width: usize, height: usize, channels: usize, sigma_x: f32, sigma_y: f32) {
    blur_edges(values, width, height, channels, sigma_x, sigma_y, false);
}

/// Blurs pixels from a backdrop, extending the window's edge pixels past the window.
#[cfg(not(windows))]
pub(crate) fn backdrop(values: &mut [f32], width: usize, height: usize, sigma: f32) {
    blur_edges(values, width, height, 4, sigma, sigma, true);
}

fn blur_edges(values: &mut [f32], width: usize, height: usize, channels: usize, sigma_x: f32, sigma_y: f32, extend: bool) {
    let mut line = Vec::new();
    let mut out = Vec::new();
    for (left, right) in boxes(sigma_x) {
        if left + right == 0 {
            continue;
        }
        for y in 0..height {
            for c in 0..channels {
                line.clear();
                line.extend((0..width).map(|x| values[(y * width + x) * channels + c]));
                box_line(&line, left, right, &mut out, extend);
                for x in 0..width {
                    values[(y * width + x) * channels + c] = out[x];
                }
            }
        }
    }
    for (left, right) in boxes(sigma_y) {
        if left + right == 0 {
            continue;
        }
        for x in 0..width {
            for c in 0..channels {
                line.clear();
                line.extend((0..height).map(|y| values[(y * width + x) * channels + c]));
                box_line(&line, left, right, &mut out, extend);
                for y in 0..height {
                    values[(y * width + x) * channels + c] = out[y];
                }
            }
        }
    }
}

/// The average of each value with `left` before it and `right` after it, counting what's past the ends as nothing.
fn box_line(line: &[f32], left: usize, right: usize, out: &mut Vec<f32>, extend: bool) {
    let n = line.len();
    out.clear();
    out.resize(n, 0.0);
    let size = (left + right + 1) as f32;
    let mut sum = 0.0f32;
    // The window for position 0 is [-left, right].
    for value in line.iter().take(right.min(n.saturating_sub(1)) + 1) {
        sum += value;
    }
    let (first, last) = if extend && n > 0 { (line[0], line[n - 1]) } else { (0.0, 0.0) };
    sum += first * left as f32 + last * (right + 1).saturating_sub(n) as f32;
    for (x, value) in out.iter_mut().enumerate() {
        *value = sum / size;
        let add = x + right + 1;
        sum += line.get(add).copied().unwrap_or(last);
        sum -= x.checked_sub(left).and_then(|at| line.get(at)).copied().unwrap_or(first);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blurring_keeps_what_there_is() {
        let (w, h) = (40, 40);
        let mut values = vec![0.0; w * h];
        for y in 15..25 {
            for x in 15..25 {
                values[y * w + x] = 1.0;
            }
        }
        blur(&mut values, w, h, 1, 3.0, 3.0);
        let total: f32 = values.iter().sum();
        assert!((total - 100.0).abs() < 0.01, "{total}");
        assert!(values[20 * w + 20] < 1.0 && values[20 * w + 12] > 0.0);
    }

    #[test]
    fn even_boxes_are_offset_both_ways() {
        let b = boxes(2.0);
        assert_eq!(b[2].0, b[2].1);
    }
}
