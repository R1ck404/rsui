//! Backgrounds that go from one color to another, as CSS's `linear-gradient` does. One is drawn into a picture of its own size the
//! first time it's needed, and kept, as backgrounds rarely change.

use rsui_text::ColorBitmap;

/// A color stop: premultiplied red, green, blue and alpha, from 0 to 1, and where it is along the gradient, if it says.
struct Stop {
    color: [f32; 4],
    at: Option<Where>,
}

#[derive(Clone, Copy)]
enum Where {
    /// A share of the gradient's length.
    Share(f32),
    /// Pixels from its start.
    Pixels(f32),
}

/// Draws a background's gradients as `background_gradient` says them, into a picture this many physical pixels across and down,
/// with corners this round. `scale` is how many physical pixels a logical one is. `None` if none can be read.
///
/// There's one to a line, from the one on top, as CSS lists them, each as `kind|stop|stop...`, where a stop is
/// `#rrggbbaa`, then where it is, as `20%` or `20px`, if it says. The kind is an angle in degrees for a linear one, as
/// `"90|#ff0000ff 0%|#0000ffff"`, or `radial`, then `circle` or `ellipse`, how big it is, and where its middle is, as
/// `radial ellipse farthest-corner 50% 50%` or `radial circle 40px 0% 10px`. A kind that starts with `*` repeats.
pub(crate) fn draw_gradient(
    spec: &str,
    width: u32,
    height: u32,
    radius: f32,
    scale: f32,
) -> Option<ColorBitmap> {
    draw_gradient_tiled(spec, width, height, radius, scale, None, (0.0, 0.0))
}

/// A gradient over a box this big, as [`draw_gradient`] draws it, in tiles of `tile` pixels from its top left, over and over, as
/// CSS's `background-size` with `background-repeat: repeat` has it: Mantine's striped progress bar is a 20 pixel square of stripes.
pub(crate) fn draw_gradient_tiled(
    spec: &str,
    width: u32,
    height: u32,
    radius: f32,
    scale: f32,
    tile: Option<(f32, f32)>,
    shift: (f32, f32),
) -> Option<ColorBitmap> {
    if width == 0 || height == 0 {
        return None;
    }
    let (w, h) = (width as f32, height as f32);
    let (tile_w, tile_h) = tile.filter(|(across, down)| *across >= 1.0 && *down >= 1.0).unwrap_or((w, h));
    let layers: Vec<Layer> = spec
        .lines()
        .filter_map(|line| Layer::read(line, (tile_w, tile_h), scale))
        .collect();
    if layers.is_empty() {
        return None;
    }
    let half = (w / 2.0, h / 2.0);
    let radius = radius.min(w.min(h) / 2.0).max(0.0);
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height {
        for x in 0..width {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            // Tiles start where `background-position` puts the first, and go on from there both ways.
            let (tx, ty) = ((px - shift.0).rem_euclid(tile_w), (py - shift.1).rem_euclid(tile_h));
            // From the bottom, each over what's under it.
            let mut color = [0f32; 4];
            for layer in layers.iter().rev() {
                let over = layer.color_at(tx, ty);
                let leave = 1.0 - over[3];
                color = std::array::from_fn(|channel| over[channel] + color[channel] * leave);
            }
            let cover = coverage((px - half.0).abs(), (py - half.1).abs(), half, radius);
            let at = (y as usize * width as usize + x as usize) * 4;
            for (channel, value) in color.iter().enumerate() {
                pixels[at + channel] = (value * cover * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Some(ColorBitmap {
        left: 0,
        top: 0,
        width,
        height,
        pixels,
    })
}

/// The color of a background's gradients at a place, in pixels from the top left of a box this big, premultiplied, from 0 to 1.
pub(crate) fn color_at_place(
    spec: &str,
    (w, h): (f32, f32),
    scale: f32,
    x: f32,
    y: f32,
) -> Option<[f32; 4]> {
    let layers: Vec<Layer> = spec
        .lines()
        .filter_map(|line| Layer::read(line, (w, h), scale))
        .collect();
    if layers.is_empty() {
        return None;
    }
    let mut color = [0f32; 4];
    for layer in layers.iter().rev() {
        let over = layer.color_at(x, y);
        let leave = 1.0 - over[3];
        color = std::array::from_fn(|channel| over[channel] + color[channel] * leave);
    }
    Some(color)
}

/// What a gradient's colors go along.
enum Shape {
    /// Across, at an angle, through the middle: the angle's sine and cosine, how long the line is, in pixels, and the middle.
    Linear {
        sin: f32,
        cos: f32,
        length: f32,
        middle: (f32, f32),
    },
    /// Out from a middle, to an ellipse this wide and tall, in pixels.
    Radial {
        middle: (f32, f32),
        reach: (f32, f32),
    },
}

/// One of a background's gradients.
struct Layer {
    shape: Shape,
    stops: Vec<Stop>,
    places: Vec<f32>,
    repeating: bool,
}

impl Layer {
    fn read(line: &str, (w, h): (f32, f32), scale: f32) -> Option<Self> {
        let mut parts = line.split('|');
        let kind = parts.next()?.trim();
        let stops: Vec<Stop> = parts.filter_map(read_stop).collect();
        if stops.is_empty() {
            return None;
        }
        let (repeating, kind) = match kind.strip_prefix('*') {
            Some(rest) => (true, rest.trim()),
            None => (false, kind),
        };
        let (shape, length) = if let Some(radial) = kind.strip_prefix("radial") {
            let words: Vec<&str> = radial.split_whitespace().collect();
            if words.len() < 3 {
                return None;
            }
            let count = words.len();
            let circle = words[0] == "circle";
            let place = |word: &str, whole: f32| -> Option<f32> {
                match word.strip_suffix('%') {
                    Some(percent) => percent
                        .parse::<f32>()
                        .ok()
                        .map(|percent| percent / 100.0 * whole),
                    None => word
                        .trim_end_matches("px")
                        .parse::<f32>()
                        .ok()
                        .map(|pixels| pixels * scale),
                }
            };
            let middle = (place(words[count - 2], w)?, place(words[count - 1], h)?);
            let near = (
                middle.0.abs().min((w - middle.0).abs()),
                middle.1.abs().min((h - middle.1).abs()),
            );
            let far = (
                middle.0.abs().max((w - middle.0).abs()),
                middle.1.abs().max((h - middle.1).abs()),
            );
            let reach = match &words[1..count - 2] {
                ["closest-side"] if circle => (near.0.min(near.1), near.0.min(near.1)),
                ["closest-side"] => near,
                ["farthest-side"] if circle => (far.0.max(far.1), far.0.max(far.1)),
                ["farthest-side"] => far,
                ["closest-corner"] if circle => (near.0.hypot(near.1), near.0.hypot(near.1)),
                // An ellipse through a corner is as it would be to the sides, made bigger to get there.
                ["closest-corner"] => (
                    near.0 * std::f32::consts::SQRT_2,
                    near.1 * std::f32::consts::SQRT_2,
                ),
                [one] if circle && *one != "farthest-corner" => {
                    let size = place(one, w)?;
                    (size, size)
                }
                [across, down] => (place(across, w)?, place(down, h)?),
                _ if circle => (far.0.hypot(far.1), far.0.hypot(far.1)),
                _ => (
                    far.0 * std::f32::consts::SQRT_2,
                    far.1 * std::f32::consts::SQRT_2,
                ),
            };
            (Shape::Radial { middle, reach }, reach.0)
        } else {
            let angle: f32 = kind
                .strip_prefix("linear")
                .unwrap_or(kind)
                .trim()
                .parse()
                .ok()?;
            // The line the colors go along goes through the middle, as long as it takes for its ends to reach the corners.
            let (sin, cos) = angle.to_radians().sin_cos();
            let length = (w * sin).abs() + (h * cos).abs();
            (
                Shape::Linear {
                    sin,
                    cos,
                    length,
                    middle: (w / 2.0, h / 2.0),
                },
                length,
            )
        };
        let places = place_stops(&stops, length / scale.max(0.01));
        Some(Self {
            shape,
            stops,
            places,
            repeating,
        })
    }

    /// Its color at a place in its picture, in pixels from the top left.
    fn color_at(&self, x: f32, y: f32) -> [f32; 4] {
        let mut along = match self.shape {
            Shape::Linear {
                sin,
                cos,
                length,
                middle,
            } => {
                if length > 0.0 {
                    ((x - middle.0) * sin - (y - middle.1) * cos) / length + 0.5
                } else {
                    0.0
                }
            }
            Shape::Radial { middle, reach } => {
                if reach.0 <= 0.0 || reach.1 <= 0.0 {
                    // With no size at all, it's all its last color, as CSS has it.
                    return self.stops[self.stops.len() - 1].color;
                }
                ((x - middle.0) / reach.0).hypot((y - middle.1) / reach.1)
            }
        };
        if self.repeating {
            let (first, last) = (self.places[0], self.places[self.places.len() - 1]);
            let period = last - first;
            if period > 1e-6 {
                along = first + (along - first).rem_euclid(period);
            }
        }
        color_at(&self.stops, &self.places, along)
    }
}

fn read_stop(text: &str) -> Option<Stop> {
    let mut words = text.split_whitespace();
    let hex = words.next()?.strip_prefix('#')?;
    let byte = |at: usize| {
        u8::from_str_radix(hex.get(at..at + 2)?, 16)
            .ok()
            .map(|value| f32::from(value) / 255.0)
    };
    let (red, green, blue) = (byte(0)?, byte(2)?, byte(4)?);
    let alpha = if hex.len() >= 8 { byte(6)? } else { 1.0 };
    let at = words.next().and_then(|place| {
        if let Some(share) = place.strip_suffix('%') {
            share
                .parse::<f32>()
                .ok()
                .map(|share| Where::Share(share / 100.0))
        } else {
            place
                .trim_end_matches("px")
                .parse::<f32>()
                .ok()
                .map(Where::Pixels)
        }
    });
    Some(Stop {
        color: [red * alpha, green * alpha, blue * alpha, alpha],
        at,
    })
}

/// Where each stop is, as a share of the gradient's length, which is `length` logical pixels. One that doesn't say is halfway between
/// the ones around it that do, and the ends are at the ends; none is before one that comes before it, as CSS has it.
fn place_stops(stops: &[Stop], length: f32) -> Vec<f32> {
    let mut places: Vec<Option<f32>> = stops
        .iter()
        .map(|stop| {
            stop.at.map(|at| match at {
                Where::Share(share) => share,
                Where::Pixels(pixels) if length > 0.0 => pixels / length,
                Where::Pixels(_) => 0.0,
            })
        })
        .collect();
    let last = places.len() - 1;
    places[0].get_or_insert(0.0);
    places[last].get_or_insert(1.0);
    let mut most = f32::MIN;
    for place in places.iter_mut().flatten() {
        most = most.max(*place);
        *place = most;
    }
    let mut out: Vec<f32> = places
        .iter()
        .map(|place| place.unwrap_or(f32::NAN))
        .collect();
    let mut at = 0;
    while at < out.len() {
        if out[at].is_nan() {
            let start = at - 1;
            let mut end = at;
            while out[end].is_nan() {
                end += 1;
            }
            let (from, to) = (out[start], out[end]);
            for (step, index) in (start + 1..end).enumerate() {
                out[index] = from + (to - from) * (step + 1) as f32 / (end - start) as f32;
            }
            at = end;
        }
        at += 1;
    }
    out
}

fn color_at(stops: &[Stop], places: &[f32], along: f32) -> [f32; 4] {
    if along <= places[0] {
        return stops[0].color;
    }
    for index in 1..stops.len() {
        if along <= places[index] {
            let span = places[index] - places[index - 1];
            let share = if span > 0.0 {
                (along - places[index - 1]) / span
            } else {
                1.0
            };
            let (from, to) = (stops[index - 1].color, stops[index].color);
            return std::array::from_fn(|channel| {
                from[channel] + (to[channel] - from[channel]) * share
            });
        }
    }
    stops[stops.len() - 1].color
}

/// How much of a pixel this far from the middle, across and down, is inside a box with corners this round.
fn coverage(x: f32, y: f32, half: (f32, f32), radius: f32) -> f32 {
    let (inner_x, inner_y) = (half.0 - radius, half.1 - radius);
    let (out_x, out_y) = ((x - inner_x).max(0.0), (y - inner_y).max(0.0));
    let distance = (out_x * out_x + out_y * out_y).sqrt() - radius;
    let edge = if radius > 0.0 {
        distance
    } else {
        (x - half.0).max(y - half.1)
    };
    (0.5 - edge).clamp(0.0, 1.0)
}

/// Draws a border that's two lines, or two colors, as `pattern` says, `width` physical pixels wide, around a box this many physical
/// pixels across and down. Where two sides meet, they meet on the line from the corner in, as a browser has it.
pub(crate) fn draw_styled_border(
    across: u32,
    down: u32,
    width: f32,
    color: crate::paint::Color,
    pattern: crate::paint::BorderPattern,
) -> Option<ColorBitmap> {
    use crate::paint::BorderPattern;
    if across == 0 || down == 0 || width <= 0.0 {
        return None;
    }
    let premultiplied = |color: crate::paint::Color| {
        let alpha = f32::from(color.a) / 255.0;
        [
            f32::from(color.r) / 255.0 * alpha,
            f32::from(color.g) / 255.0 * alpha,
            f32::from(color.b) / 255.0 * alpha,
            alpha,
        ]
    };
    let light = premultiplied(color);
    let dark = premultiplied(darker(color));
    // A double border's lines are each a third of it, to the nearest pixel, as Blink has them.
    let third = ((width + 1.0) / 3.0).floor().max(1.0);
    let (w, h) = (across as f32, down as f32);
    // Each pixel is looked at in sixteen places, which smooths where the sides meet.
    const STEPS: u32 = 4;
    let mut pixels = vec![0u8; across as usize * down as usize * 4];
    for y in 0..down {
        for x in 0..across {
            // Nothing that's inside the border needs looking at closely.
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            if cx.min(w - cx).min(cy).min(h - cy) > width + 1.0 {
                continue;
            }
            let mut sum = [0f32; 4];
            for sub_y in 0..STEPS {
                for sub_x in 0..STEPS {
                    let sx = x as f32 + (sub_x as f32 + 0.5) / STEPS as f32;
                    let sy = y as f32 + (sub_y as f32 + 0.5) / STEPS as f32;
                    let (from_left, from_right, from_top, from_bottom) = (sx, w - sx, sy, h - sy);
                    let depth = from_left.min(from_right).min(from_top).min(from_bottom);
                    if depth >= width {
                        continue;
                    }
                    let top_or_left = from_top.min(from_left) <= from_bottom.min(from_right);
                    let outer_half = depth < width / 2.0;
                    let shade = match pattern {
                        BorderPattern::Double => {
                            if depth < third || depth >= width - third {
                                Some(light)
                            } else {
                                None
                            }
                        }
                        BorderPattern::Inset => Some(if top_or_left { dark } else { light }),
                        BorderPattern::Outset => Some(if top_or_left { light } else { dark }),
                        BorderPattern::Groove => Some(if top_or_left == outer_half {
                            dark
                        } else {
                            light
                        }),
                        BorderPattern::Ridge => Some(if top_or_left == outer_half {
                            light
                        } else {
                            dark
                        }),
                        _ => Some(light),
                    };
                    if let Some(shade) = shade {
                        for channel in 0..4 {
                            sum[channel] += shade[channel];
                        }
                    }
                }
            }
            let count = (STEPS * STEPS) as f32;
            let at = (y as usize * across as usize + x as usize) * 4;
            for channel in 0..4 {
                pixels[at + channel] =
                    (sum[channel] / count * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Some(ColorBitmap {
        left: 0,
        top: 0,
        width: across,
        height: down,
        pixels,
    })
}

/// A wavy line under text, as Blink draws one, this many physical pixels long and this thick, from the top of where a straight line
/// would be: 9 pixels from one crest to the next and a little over 2 high for a line 1 pixel thick, longer and higher for a thicker
/// one, starting a little way into a trough.
pub(crate) fn draw_wave(
    across: u32,
    thickness: f32,
    color: crate::paint::Color,
) -> Option<ColorBitmap> {
    let length = 5.0 + 4.0 * thickness;
    let reach = 0.3 + thickness;
    let middle = thickness / 2.0 + 2.0 * thickness;
    let down = (middle + reach + thickness).ceil() as u32 + 1;
    let turn = std::f32::consts::TAU / length;
    let alpha = f32::from(color.a) / 255.0;
    let paint = [
        f32::from(color.r) / 255.0 * alpha,
        f32::from(color.g) / 255.0 * alpha,
        f32::from(color.b) / 255.0 * alpha,
        alpha,
    ];
    const STEPS: u32 = 4;
    let mut pixels = vec![0u8; across as usize * down as usize * 4];
    for x in 0..across {
        for sub_x in 0..STEPS {
            let sx = x as f32 + (sub_x as f32 + 0.5) / STEPS as f32;
            let along = sx - 1.5;
            let y = middle + reach * (along * turn).cos();
            let slope = -reach * turn * (along * turn).sin();
            let half = thickness * (1.0 + slope * slope).sqrt() / 2.0;
            for row in 0..down {
                let mut inside = 0;
                for sub_y in 0..STEPS {
                    let sy = row as f32 + (sub_y as f32 + 0.5) / STEPS as f32;
                    if (sy - y).abs() <= half {
                        inside += 1;
                    }
                }
                if inside > 0 {
                    let share = inside as f32 / (STEPS * STEPS) as f32;
                    let at = (row as usize * across as usize + x as usize) * 4;
                    for channel in 0..4 {
                        let value =
                            f32::from(pixels[at + channel]) + paint[channel] * share * 255.0;
                        pixels[at + channel] = value.round().min(255.0) as u8;
                    }
                }
            }
        }
    }
    Some(ColorBitmap {
        left: 0,
        top: 0,
        width: across,
        height: down,
        pixels,
    })
}

/// The shape of a box whose corners aren't all one circle, or that's cut to a shape, in physical pixels from its top left.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BoxShape {
    /// How far across and down each corner's curve goes, from the top left going around.
    corners: [[f32; 2]; 4],
    clip: Option<Clip>,
}

/// A shape something's cut to, as CSS's `clip-path` says.
#[derive(Debug, Clone, PartialEq)]
enum Clip {
    Ellipse {
        middle: (f32, f32),
        reach: (f32, f32),
    },
    Inset {
        edges: [f32; 4],
        round: f32,
    },
    Polygon(Vec<(f32, f32)>),
}

impl BoxShape {
    /// What an element's `corner_radii` and `clip_path` say about its shape, at this place, if anything does. `None` if its corners
    /// are all one circle and it isn't cut to a shape, which is drawn more simply.
    pub(crate) fn of(element: &rsui_core::Element, area: [f32; 4], scale: f32) -> Option<Self> {
        use rsui_core::PropValue;
        let (w, h) = (area[2].max(0.0), area[3].max(0.0));
        let text = |name: &str| match element.props.get(name) {
            Some(PropValue::Str(text)) => Some(text.as_str()),
            _ => None,
        };
        let size = |word: &str, whole: f32| -> f32 {
            match word.strip_suffix('%') {
                Some(share) => share
                    .parse::<f32>()
                    .map_or(0.0, |share| share / 100.0 * whole),
                None => word
                    .trim_end_matches("px")
                    .parse::<f32>()
                    .map_or(0.0, |pixels| pixels * scale),
            }
        };
        let mut corners = match text("corner_radii") {
            Some(said) => {
                let parts: Vec<f32> = said
                    .split_whitespace()
                    .enumerate()
                    .map(|(at, word)| size(word, if at % 2 == 0 { w } else { h }))
                    .collect();
                if parts.len() < 8 {
                    return None;
                }
                [
                    [parts[0], parts[1]],
                    [parts[2], parts[3]],
                    [parts[4], parts[5]],
                    [parts[6], parts[7]],
                ]
            }
            None => {
                let radius = match element
                    .props
                    .get("radius")
                    .or_else(|| element.props.get("corner_radius"))
                {
                    Some(PropValue::Float(radius)) => *radius as f32 * scale,
                    Some(PropValue::Int(radius)) => *radius as f32 * scale,
                    _ => 0.0,
                };
                [[radius; 2]; 4]
            }
        };
        // Corners that would overlap are all made smaller by as much as it takes for them not to, as CSS has it.
        let mut times = 1f32;
        for (one, two, along, whole) in [(0, 1, 0, w), (3, 2, 0, w), (0, 3, 1, h), (1, 2, 1, h)] {
            let sum = corners[one][along] + corners[two][along];
            if sum > whole && sum > 0.0 {
                times = times.min(whole / sum);
            }
        }
        for corner in &mut corners {
            corner[0] *= times;
            corner[1] *= times;
        }
        let clip = text("clip_path").and_then(|said| Clip::read(said, w, h, scale));
        let same = corners
            .iter()
            .all(|corner| corner[0] == corners[0][0] && corner[1] == corners[0][0]);
        if same && clip.is_none() {
            return None;
        }
        Some(Self { corners, clip })
    }

    /// How far across and down each corner's curve goes, in physical pixels, from the top left going around.
    pub(crate) fn corners(&self) -> [[f32; 2]; 4] {
        self.corners
    }

    pub(crate) fn hash_into(&self, hasher: &mut impl std::hash::Hasher) {
        use std::hash::Hash;
        for corner in &self.corners {
            corner[0].to_bits().hash(hasher);
            corner[1].to_bits().hash(hasher);
        }
        format!("{:?}", self.clip).hash(hasher);
    }

    /// Whether a place, in pixels from the top left, is inside a box this big with these corners, made smaller by `inset` all
    /// around, and inside what it's cut to.
    fn holds(&self, x: f32, y: f32, (w, h): (f32, f32), inset: f32) -> bool {
        self.holds_inset(x, y, (w, h), [inset; 4])
    }

    /// The inner border edge has each radius reduced by the adjacent side's width, independently on the two axes.
    fn holds_inset(&self, x: f32, y: f32, (w, h): (f32, f32), inset: [f32; 4]) -> bool {
        let [top, right, bottom, left] = inset;
        let (right, bottom) = (w - right, h - bottom);
        if x < left || y < top || x > right || y > bottom {
            return false;
        }
        let inside_corner = |cx: f32, cy: f32, rx: f32, ry: f32| {
            let (dx, dy) = ((x - cx) / rx.max(1e-3), (y - cy) / ry.max(1e-3));
            dx * dx + dy * dy <= 1.0
        };
        let curve = |at: usize, horizontal: usize, vertical: usize| {
            [
                (self.corners[at][0] - inset[horizontal]).max(0.0),
                (self.corners[at][1] - inset[vertical]).max(0.0),
            ]
        };
        let [tl, tr, br, bl] = [curve(0, 3, 0), curve(1, 1, 0), curve(2, 1, 2), curve(3, 3, 2)];
        let inside = if x < left + tl[0] && y < top + tl[1] {
            inside_corner(left + tl[0], top + tl[1], tl[0], tl[1])
        } else if x > right - tr[0] && y < top + tr[1] {
            inside_corner(right - tr[0], top + tr[1], tr[0], tr[1])
        } else if x > right - br[0] && y > bottom - br[1] {
            inside_corner(right - br[0], bottom - br[1], br[0], br[1])
        } else if x < left + bl[0] && y > bottom - bl[1] {
            inside_corner(left + bl[0], bottom - bl[1], bl[0], bl[1])
        } else {
            true
        };
        inside
            && self
                .clip
                .as_ref()
                .is_none_or(|clip| clip.holds(x, y, (w, h)))
    }
}

impl Clip {
    /// Reads `circle()`, `ellipse()`, `inset()` and `polygon()` for a box this many pixels across and down.
    fn read(text: &str, w: f32, h: f32, scale: f32) -> Option<Self> {
        let text = text.trim();
        let open = text.find('(')?;
        let name = text[..open].trim();
        let inside = text[open + 1..text.rfind(')')?].trim();
        let size = |word: &str, whole: f32| -> Option<f32> {
            match word.strip_suffix('%') {
                Some(share) => share.parse::<f32>().ok().map(|share| share / 100.0 * whole),
                None => word
                    .trim_end_matches("px")
                    .parse::<f32>()
                    .ok()
                    .map(|pixels| pixels * scale),
            }
        };
        let place = |words: &[&str]| -> (f32, f32) {
            let one = |word: &str, whole: f32| match word {
                "left" | "top" => 0.0,
                "center" => whole / 2.0,
                "right" | "bottom" => whole,
                word => size(word, whole).unwrap_or(whole / 2.0),
            };
            match words {
                [x, y, ..] => (one(x, w), one(y, h)),
                [x] => (one(x, w), h / 2.0),
                [] => (w / 2.0, h / 2.0),
            }
        };
        match name {
            "circle" | "ellipse" => {
                let (shape, at) = match inside.split_once(" at ") {
                    Some((shape, at)) => (shape.trim(), at.trim()),
                    None if inside.starts_with("at ") => ("", &inside[3..]),
                    None => (inside, ""),
                };
                let middle = place(&at.split_whitespace().collect::<Vec<_>>());
                let near = (middle.0.min(w - middle.0), middle.1.min(h - middle.1));
                let far = (middle.0.max(w - middle.0), middle.1.max(h - middle.1));
                let radius = |word: &str, whole: f32, near: f32, far: f32| match word {
                    "" | "closest-side" => near,
                    "farthest-side" => far,
                    word => size(word, whole).unwrap_or(near),
                };
                let words: Vec<&str> = shape.split_whitespace().collect();
                let reach = if name == "circle" {
                    // A percentage of a circle is of a diagonal over the square root of two, as CSS has it.
                    let diagonal = w.hypot(h) / std::f32::consts::SQRT_2;
                    let r = radius(
                        words.first().copied().unwrap_or(""),
                        diagonal,
                        near.0.min(near.1),
                        far.0.max(far.1),
                    );
                    (r, r)
                } else {
                    (
                        radius(words.first().copied().unwrap_or(""), w, near.0, far.0),
                        radius(words.get(1).copied().unwrap_or(""), h, near.1, far.1),
                    )
                };
                Some(Self::Ellipse { middle, reach })
            }
            "inset" => {
                let (edges, round) = match inside.split_once(" round ") {
                    Some((edges, round)) => (
                        edges,
                        size(round.split_whitespace().next()?, w.min(h)).unwrap_or(0.0),
                    ),
                    None => (inside, 0.0),
                };
                let words: Vec<&str> = edges.split_whitespace().collect();
                let [t, r, b, l] = match words.as_slice() {
                    [a] => [*a; 4],
                    [a, b] => [*a, *b, *a, *b],
                    [a, b, c] => [*a, *b, *c, *b],
                    [a, b, c, d, ..] => [*a, *b, *c, *d],
                    [] => return None,
                };
                Some(Self::Inset {
                    edges: [size(t, h)?, size(r, w)?, size(b, h)?, size(l, w)?],
                    round,
                })
            }
            "polygon" => {
                let points: Vec<(f32, f32)> = inside
                    .split(',')
                    .filter_map(|point| {
                        let mut words = point
                            .split_whitespace()
                            .filter(|word| *word != "nonzero" && *word != "evenodd");
                        Some((size(words.next()?, w)?, size(words.next()?, h)?))
                    })
                    .collect();
                (points.len() >= 3).then_some(Self::Polygon(points))
            }
            _ => None,
        }
    }

    fn holds(&self, x: f32, y: f32, (w, h): (f32, f32)) -> bool {
        match self {
            Self::Ellipse { middle, reach } => {
                let (dx, dy) = (
                    (x - middle.0) / reach.0.max(1e-3),
                    (y - middle.1) / reach.1.max(1e-3),
                );
                dx * dx + dy * dy <= 1.0
            }
            Self::Inset {
                edges: [t, r, b, l],
                round,
            } => {
                let shape = BoxShape {
                    corners: [[*round; 2]; 4],
                    clip: None,
                };
                x >= *l
                    && y >= *t
                    && x <= w - r
                    && y <= h - b
                    && shape.holds(x - l, y - t, (w - l - r, h - t - b), 0.0)
            }
            Self::Polygon(points) => {
                // Inside if a line from it to the right crosses the edges an odd number of times.
                let mut inside = false;
                let mut last = points[points.len() - 1];
                for &point in points {
                    if (point.1 > y) != (last.1 > y)
                        && x < (last.0 - point.0) * (y - point.1) / (last.1 - point.1) + point.0
                    {
                        inside = !inside;
                    }
                    last = point;
                }
                inside
            }
        }
    }
}

/// The widths and colors of a shaped box's top, right, bottom and left borders, in physical pixels.
#[derive(Clone, Copy)]
pub(crate) struct ShapeBorder {
    pub(crate) widths: [f32; 4],
    pub(crate) colors: [Option<crate::paint::Color>; 4],
}

/// Draws a box as `shape` says it is, with its background and border, this many physical pixels across and down.
pub(crate) fn draw_shaped_box(
    across: u32,
    down: u32,
    shape: &BoxShape,
    background: Option<crate::paint::Color>,
    border: Option<ShapeBorder>,
) -> Option<ColorBitmap> {
    if across == 0 || down == 0 {
        return None;
    }
    let premultiplied = |color: crate::paint::Color| {
        let alpha = f32::from(color.a) / 255.0;
        [
            f32::from(color.r) / 255.0 * alpha,
            f32::from(color.g) / 255.0 * alpha,
            f32::from(color.b) / 255.0 * alpha,
            alpha,
        ]
    };
    let fill = background.map(premultiplied);
    let widths = border.map_or([0.0; 4], |border| border.widths);
    let lines = border.map_or([None; 4], |border| border.colors.map(|color| color.map(premultiplied)));
    let size = (across as f32, down as f32);
    const STEPS: u32 = 4;
    let mut pixels = vec![0u8; across as usize * down as usize * 4];
    for y in 0..down {
        for x in 0..across {
            let mut sum = [0f32; 4];
            for sub_y in 0..STEPS {
                for sub_x in 0..STEPS {
                    let sx = x as f32 + (sub_x as f32 + 0.5) / STEPS as f32;
                    let sy = y as f32 + (sub_y as f32 + 0.5) / STEPS as f32;
                    if !shape.holds(sx, sy, size, 0.0) {
                        continue;
                    }
                    let mut color = fill.unwrap_or([0.0; 4]);
                    if border.is_some() && !shape.holds_inset(sx, sy, size, widths) {
                        // Adjacent sides meet along the line between their outer and inner corners. Dividing by each
                        // side's width keeps that join correct for unequal widths and leaves missing sides absent.
                        let distances = [sy, size.0 - sx, size.1 - sy, sx];
                        let mut nearest = f32::INFINITY;
                        let mut side = 0;
                        for at in 0..4 {
                            if widths[at] > 0.0 && distances[at] / widths[at] < nearest {
                                nearest = distances[at] / widths[at];
                                side = at;
                            }
                        }
                        if let Some(over) = lines[side] {
                            color = std::array::from_fn(|at| over[at] + color[at] * (1.0 - over[3]));
                        }
                    }
                    for at in 0..4 {
                        sum[at] += color[at];
                    }
                }
            }
            let count = (STEPS * STEPS) as f32;
            let at = (y as usize * across as usize + x as usize) * 4;
            for channel in 0..4 {
                pixels[at + channel] =
                    (sum[channel] / count * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Some(ColorBitmap {
        left: 0,
        top: 0,
        width: across,
        height: down,
        pixels,
    })
}

/// The darker color Blink uses for the shaded sides of a border, which takes away a third of the brightest part from each.
pub(crate) fn darker(color: crate::paint::Color) -> crate::paint::Color {
    let (r, g, b) = (
        f32::from(color.r) / 255.0,
        f32::from(color.g) / 255.0,
        f32::from(color.b) / 255.0,
    );
    if color.r == 0 && color.g == 0 && color.b == 0 {
        return crate::paint::Color::rgba(0x54, 0x54, 0x54, color.a);
    }
    let most = r.max(g).max(b);
    let times = if most == 0.0 {
        0.0
    } else {
        ((most - 0.33) / most).max(0.0)
    };
    let part = |value: f32| (value * times * 255.99) as u8;
    crate::paint::Color::rgba(part(r), part(g), part(b), color.a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(bitmap: &ColorBitmap, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * bitmap.width + x) * 4) as usize;
        [
            bitmap.pixels[at],
            bitmap.pixels[at + 1],
            bitmap.pixels[at + 2],
            bitmap.pixels[at + 3],
        ]
    }

    #[test]
    fn a_gradient_goes_from_its_first_color_to_its_last_along_its_angle() {
        let across = draw_gradient("90|#ff0000ff|#0000ffff", 100, 10, 0.0, 1.0).unwrap();
        assert_eq!(pixel(&across, 0, 5), [254, 0, 1, 255]);
        assert_eq!(pixel(&across, 99, 5), [1, 0, 254, 255]);
        let down = draw_gradient("180|#ffffffff|#000000ff", 10, 100, 0.0, 1.0).unwrap();
        assert!(pixel(&down, 5, 0)[0] > 250 && pixel(&down, 5, 99)[0] < 5);
        assert_eq!(pixel(&down, 5, 50)[0], 126);
    }

    #[test]
    fn tiles_start_where_they_are_put() {
        // Mantine's striped, animated Progress moves its stripes with `background-position`.
        let spec = "90|#ff0000ff 50%|#0000ffff 50%";
        let plain = draw_gradient_tiled(spec, 40, 4, 0.0, 1.0, Some((10.0, 4.0)), (0.0, 0.0)).unwrap();
        let moved = draw_gradient_tiled(spec, 40, 4, 0.0, 1.0, Some((10.0, 4.0)), (3.0, 0.0)).unwrap();
        for x in 3..40 {
            assert_eq!(pixel(&moved, x, 2), pixel(&plain, x - 3, 2), "at {x}");
        }
        // What's before where the first tile starts is the end of the one before it.
        assert_eq!(pixel(&moved, 0, 2), pixel(&plain, 7, 2));
    }

    #[test]
    fn stops_go_where_they_say_and_ones_that_dont_go_between() {
        let stops: Vec<Stop> = ["#000000ff", "#000000ff 50%", "#000000ff", "#000000ff 20px"]
            .iter()
            .filter_map(|text| read_stop(text))
            .collect();
        let places = place_stops(&stops, 100.0);
        // A stop can't be before one that comes before it, so the last is where the one before it is.
        assert_eq!(places, [0.0, 0.5, 0.5, 0.5]);
    }

    #[test]
    fn a_radial_one_goes_out_from_its_middle() {
        let round = draw_gradient(
            "radial circle closest-side 50% 50%|#ff0000ff|#0000ffff",
            40,
            40,
            0.0,
            1.0,
        )
        .unwrap();
        assert!(pixel(&round, 20, 20)[0] > 240);
        assert!(pixel(&round, 0, 20)[2] > 240, "it ends at the side");
        assert!(
            pixel(&round, 0, 0)[2] > 250,
            "and is its last color past that"
        );
    }

    #[test]
    fn a_repeating_one_starts_again_at_its_end() {
        let stripes = draw_gradient(
            "*90|#000000ff 0px|#000000ff 5px|#ffffffff 5px|#ffffffff 10px",
            40,
            4,
            0.0,
            1.0,
        )
        .unwrap();
        assert_eq!(pixel(&stripes, 2, 2)[0], 0);
        assert_eq!(pixel(&stripes, 7, 2)[0], 255);
        assert_eq!(pixel(&stripes, 12, 2)[0], 0);
        assert_eq!(pixel(&stripes, 37, 2)[0], 255);
    }

    #[test]
    fn the_first_is_on_top() {
        let both = draw_gradient(
            "90|#ff000080|#ff000080\n90|#0000ffff|#0000ffff",
            4,
            4,
            0.0,
            1.0,
        )
        .unwrap();
        assert_eq!(pixel(&both, 2, 2), [128, 0, 127, 255]);
    }

    #[test]
    fn round_corners_leave_the_corners_out() {
        let round = draw_gradient("180|#ff0000ff|#ff0000ff", 40, 40, 10.0, 1.0).unwrap();
        assert_eq!(pixel(&round, 0, 0)[3], 0);
        assert_eq!(pixel(&round, 20, 20)[3], 255);
    }
}
