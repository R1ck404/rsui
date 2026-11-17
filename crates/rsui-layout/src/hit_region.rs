//! A bounded certificate for an unchanged raw hit-test decision trace.
//! Geometry and element metadata must remain unchanged while it is reused.

use crate::Rect;

/// An open rectangle in query coordinates certified to give the same raw hit.
/// Rounded corners, pointer ownership, and text refinement are not cached.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitRegion {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl HitRegion {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let (x, y) = (f64::from(x), f64::from(y));
        x > self.left && x < self.right && y > self.top && y < self.bottom
    }

    #[cfg(test)]
    pub(crate) fn bounds(&self) -> [f64; 4] {
        [self.left, self.top, self.right, self.bottom]
    }
}

pub(crate) trait HitProbe {
    #[inline]
    fn transform(&mut self, _: Option<[f32; 6]>, _: Rect, _: f32, _: f32, _: usize) {}

    #[inline]
    fn contains(&mut self, rect: Rect, x: f32, y: f32, _: usize) -> bool {
        rect.contains(x, y)
    }
}

pub(crate) struct NoProbe;
impl HitProbe for NoProbe {}

pub(crate) struct RegionProbe {
    region: HitRegion,
    query: [f64; 2],
    max_abs: f64,
    span: f64,
    depth: usize,
    supported: bool,
}

impl RegionProbe {
    pub(crate) fn new(x: f32, y: f32, bounds: Rect) -> Self {
        let right = bounds.x + bounds.width;
        let bottom = bounds.y + bounds.height;
        let mut probe = Self {
            region: HitRegion {
                left: f64::from(bounds.x),
                top: f64::from(bounds.y),
                right: f64::from(right),
                bottom: f64::from(bottom),
            },
            query: [f64::from(x), f64::from(y)],
            max_abs: 0.0,
            span: f64::from(bounds.width.abs().max(bounds.height.abs())),
            depth: 0,
            supported: bounds.width > 0.0 && bounds.height > 0.0,
        };
        probe.operands(&[
            x,
            y,
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            right,
            bottom,
        ]);
        probe
    }

    fn operands(&mut self, values: &[f32]) {
        for &value in values {
            self.supported &= value.is_finite();
            self.max_abs = self.max_abs.max(f64::from(value).abs());
        }
    }

    fn at_depth(&mut self, depth: usize) {
        self.depth = self.depth.max(depth);
        self.supported &= depth <= 64;
    }

    pub(crate) fn finish(mut self) -> Option<HitRegion> {
        if !self.supported {
            return None;
        }
        // The error argument below requires finite intermediate arithmetic for
        // every later query, including a large translation that is cancelled
        // farther down the path. This very conservative range limit leaves
        // ample headroom for <=2*(64+2) bounded operations and their roundoff.
        if self.max_abs + self.span > f64::from(f32::MAX) / 1024.0 {
            return None;
        }
        // Each axis traverses at most about 2*(depth+2) f32 add/sub operations:
        // a translation and local-origin subtraction at each ancestor. Exact
        // identity multiplications add no error. Along the old trace, every
        // operand is bounded by M=max_abs. A new viewport query differs by at
        // most D=span before accumulated rounding. Writing its local delta as
        // delta_j=delta_(j-1)+epsilon'_j-epsilon_j bounds its error by
        // O(n*EPSILON*(4*M+D)), including cancellation/subnormal roundoff.
        // With depth <=64, n*EPSILON is tiny; the factor16 below conservatively
        // dominates both traces' error and the small geometric-series factor.
        // Rect edge sums are fixed f32 values, not recomputed from the query.
        // Requiring strict interior after this inset preserves every observed
        // predicate, hence the complete deterministic raw decision trace.
        let margin = 16.0
            * (self.depth as f64 + 1.0)
            * f64::from(f32::EPSILON)
            * (self.max_abs + self.span + 1.0);
        self.region.left += margin;
        self.region.top += margin;
        self.region.right -= margin;
        self.region.bottom -= margin;
        self.region
            .contains(self.query[0] as f32, self.query[1] as f32)
            .then_some(self.region)
    }
}

impl HitProbe for RegionProbe {
    fn transform(&mut self, matrix: Option<[f32; 6]>, rect: Rect, x: f32, y: f32, depth: usize) {
        self.at_depth(depth);
        self.operands(&[x, y, rect.x, rect.y, rect.width, rect.height]);
        if let Some(matrix @ [a, b, c, d, _, _]) = matrix {
            self.operands(&matrix);
            self.supported &= a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0;
        }
    }

    fn contains(&mut self, rect: Rect, x: f32, y: f32, depth: usize) -> bool {
        let answer = rect.contains(x, y);
        self.at_depth(depth);
        let right = rect.x + rect.width;
        let bottom = rect.y + rect.height;
        self.operands(&[x, y, rect.x, rect.y, rect.width, rect.height, right, bottom]);
        if !self.supported {
            return answer;
        }
        // A finite empty rectangle rejects every query without a point-dependent
        // branch. Keep the actual predicate's result, including signed zero.
        if right <= rect.x || bottom <= rect.y {
            return answer;
        }
        let dx = f64::from(x) - self.query[0];
        let dy = f64::from(y) - self.query[1];
        let left = f64::from(rect.x) - dx;
        let top = f64::from(rect.y) - dy;
        let right = f64::from(right) - dx;
        let bottom = f64::from(bottom) - dy;
        if answer {
            self.region.left = self.region.left.max(left);
            self.region.top = self.region.top.max(top);
            self.region.right = self.region.right.min(right);
            self.region.bottom = self.region.bottom.min(bottom);
        } else {
            // One false comparison suffices. Prefer the largest separation so
            // nearby queries have the widest chance of retaining that failure.
            let candidates = [
                (left - self.query[0], 0),
                (self.query[0] - right, 1),
                (top - self.query[1], 2),
                (self.query[1] - bottom, 3),
            ];
            let Some(&(_, side)) = candidates
                .iter()
                .filter(|(distance, _)| *distance >= 0.0)
                .max_by(|a, b| a.0.total_cmp(&b.0))
            else {
                self.supported = false;
                return answer;
            };
            match side {
                0 => self.region.right = self.region.right.min(left),
                1 => self.region.left = self.region.left.max(right),
                2 => self.region.bottom = self.region.bottom.min(top),
                _ => self.region.top = self.region.top.max(bottom),
            }
        }
        answer
    }
}
