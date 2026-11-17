//! Turns colors from whatever color space a file says it's in, into the one screens show.
//!
//! Most pictures are in that space already, and saying so costs nothing here, as a profile that wouldn't
//! change anything is recognized and dropped.

use std::sync::OnceLock;

/// What kind of colors a picture has, which a profile has to match to be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileKind {
    Rgb,
    Gray,
    Cmyk,
}

/// From the space profiles work in, with its D50 white, to linear light for a screen.
const FROM_D50: [[f32; 3]; 3] = [
    [3.133_856, -1.616_867, -0.490_615],
    [-0.978_768, 1.916_142, 0.033_454],
    [0.071_945, -0.228_991, 1.405_243],
];
const D50: [f32; 3] = [0.964_2, 1.0, 0.824_9];
/// How much a profile can be out by and still count as what screens already show, out of 255.
const SAME_AS_SCREEN: i32 = 1;
/// The most numbers a profile's table can hold, which is far more than any real one.
const MOST_TABLE_ENTRIES: usize = 1 << 24;

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let part = bytes.get(at..at + 2)?;
    Some(u16::from_be_bytes([part[0], part[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let part = bytes.get(at..at + 4)?;
    Some(u32::from_be_bytes([part[0], part[1], part[2], part[3]]))
}

/// A number with 16 bits after its point.
fn read_fixed(bytes: &[u8], at: usize) -> Option<f32> {
    Some(read_u32(bytes, at)? as i32 as f32 / 65536.0)
}

/// How a stored value turns into light.
#[derive(Debug, Clone)]
enum Curve {
    Power(f32),
    Table(Vec<f32>),
    /// One of the forms made of a power and a straight part, with its numbers.
    Parametric(u16, [f32; 7]),
}

impl Curve {
    fn eval(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Self::Power(gamma) => x.powf(*gamma),
            Self::Table(table) => interpolate(table, x),
            Self::Parametric(kind, p) => {
                let [g, a, b, c, d, e, f] = *p;
                let power = |x: f32| (a * x + b).max(0.0).powf(g);
                match kind {
                    0 => x.powf(g),
                    1 => {
                        if a != 0.0 && x >= -b / a {
                            power(x)
                        } else {
                            0.0
                        }
                    }
                    2 => {
                        if a != 0.0 && x >= -b / a {
                            power(x) + c
                        } else {
                            c
                        }
                    }
                    3 => {
                        if x >= d {
                            power(x)
                        } else {
                            c * x
                        }
                    }
                    _ => {
                        if x >= d {
                            power(x) + e
                        } else {
                            c * x + f
                        }
                    }
                }
            }
        }
    }

    /// Reads a curve, and says how many bytes it takes up.
    fn read(bytes: &[u8]) -> Option<(Self, usize)> {
        match bytes.get(..4)? {
            b"curv" => {
                let count = read_u32(bytes, 8)? as usize;
                let size = 12usize.checked_add(count.checked_mul(2)?)?;
                let curve = match count {
                    0 => Self::Power(1.0),
                    1 => Self::Power(f32::from(read_u16(bytes, 12)?) / 256.0),
                    _ => Self::Table(
                        (0..count)
                            .map(|at| Some(f32::from(read_u16(bytes, 12 + at * 2)?) / 65535.0))
                            .collect::<Option<Vec<f32>>>()?,
                    ),
                };
                Some((curve, size))
            }
            b"para" => {
                let kind = read_u16(bytes, 8)?;
                let count = *[1usize, 3, 4, 5, 7].get(usize::from(kind))?;
                let mut p = [0.0; 7];
                for (index, value) in p.iter_mut().enumerate().take(count) {
                    *value = read_fixed(bytes, 12 + index * 4)?;
                }
                if count == 1 {
                    p[1] = 1.0;
                }
                Some((Self::Parametric(kind, p), 12 + count * 4))
            }
            _ => None,
        }
    }
}

fn interpolate(table: &[f32], x: f32) -> f32 {
    let last = table.len() - 1;
    let place = x.clamp(0.0, 1.0) * last as f32;
    let low = (place as usize).min(last);
    let high = (low + 1).min(last);
    let part = place - low as f32;
    table[low] + (table[high] - table[low]) * part
}

fn find_tag<'a>(profile: &'a [u8], wanted: &[u8; 4]) -> Option<&'a [u8]> {
    let count = read_u32(profile, 128)? as usize;
    for index in 0..count.min(1024) {
        let entry = 132 + index * 12;
        if profile.get(entry..entry + 4)? == wanted {
            let start = read_u32(profile, entry + 4)? as usize;
            let size = read_u32(profile, entry + 8)? as usize;
            return profile.get(start..start.checked_add(size)?);
        }
    }
    None
}

fn read_xyz(tag: &[u8]) -> Option<[f32; 3]> {
    if tag.get(..4)? != b"XYZ " {
        return None;
    }
    Some([
        read_fixed(tag, 8)?,
        read_fixed(tag, 12)?,
        read_fixed(tag, 16)?,
    ])
}

/// The steps from a picture's colors to the space profiles work in, by way of a table.
#[derive(Debug, Clone)]
pub(crate) struct LutPipeline {
    inputs: usize,
    input_curves: Vec<Curve>,
    grid: Vec<usize>,
    /// By the first input, then the next, with `outputs` numbers at each point.
    table: Vec<f32>,
    /// What comes after the table, for the newer kind of profile, as curves, a matrix and curves.
    middle_curves: Vec<Curve>,
    matrix: Option<[f32; 12]>,
    output_curves: Vec<Curve>,
    /// How to turn what comes out into the space profiles work in.
    lab: bool,
    /// Whether it's the older way of storing Lab, whose top value isn't quite the top.
    legacy_16_bit_lab: bool,
}

impl LutPipeline {
    fn read(tag: &[u8], inputs: usize, lab: bool) -> Option<Self> {
        match tag.get(..4)? {
            b"mft1" | b"mft2" => Self::read_old(tag, inputs, lab),
            b"mAB " => Self::read_new(tag, inputs, lab),
            _ => None,
        }
    }

    fn read_old(tag: &[u8], inputs: usize, lab: bool) -> Option<Self> {
        let wide = tag.get(..4)? == b"mft2";
        if usize::from(*tag.get(8)?) != inputs || *tag.get(9)? != 3 {
            return None;
        }
        let points = usize::from(*tag.get(10)?);
        if points < 2 {
            return None;
        }
        let (input_entries, output_entries, mut at) = if wide {
            (
                usize::from(read_u16(tag, 48)?),
                usize::from(read_u16(tag, 50)?),
                52,
            )
        } else {
            (256, 256, 48)
        };
        if input_entries < 2 || output_entries < 2 {
            return None;
        }
        let read_value = |at: usize| -> Option<f32> {
            if wide {
                Some(f32::from(read_u16(tag, at)?) / 65535.0)
            } else {
                Some(f32::from(*tag.get(at)?) / 255.0)
            }
        };
        let size = if wide { 2 } else { 1 };
        let mut input_curves = Vec::with_capacity(inputs);
        for _ in 0..inputs {
            let values = (0..input_entries)
                .map(|index| read_value(at + index * size))
                .collect::<Option<Vec<f32>>>()?;
            input_curves.push(Curve::Table(values));
            at += input_entries * size;
        }
        let total = (0..inputs).try_fold(3usize, |total, _| total.checked_mul(points))?;
        if total > MOST_TABLE_ENTRIES {
            return None;
        }
        let table = (0..total)
            .map(|index| read_value(at + index * size))
            .collect::<Option<Vec<f32>>>()?;
        at += total * size;
        let mut output_curves = Vec::with_capacity(3);
        for _ in 0..3 {
            let values = (0..output_entries)
                .map(|index| read_value(at + index * size))
                .collect::<Option<Vec<f32>>>()?;
            output_curves.push(Curve::Table(values));
            at += output_entries * size;
        }
        Some(Self {
            inputs,
            input_curves,
            grid: vec![points; inputs],
            table,
            middle_curves: Vec::new(),
            matrix: None,
            output_curves,
            lab,
            legacy_16_bit_lab: wide,
        })
    }

    fn read_new(tag: &[u8], inputs: usize, lab: bool) -> Option<Self> {
        if usize::from(*tag.get(8)?) != inputs || *tag.get(9)? != 3 {
            return None;
        }
        let offset = |at: usize| -> Option<usize> { Some(read_u32(tag, at)? as usize) };
        let (b, matrix_at, m, clut, a) = (
            offset(12)?,
            offset(16)?,
            offset(20)?,
            offset(24)?,
            offset(28)?,
        );
        let curves = |mut at: usize, count: usize| -> Option<Vec<Curve>> {
            let mut list = Vec::with_capacity(count);
            for _ in 0..count {
                let (curve, size) = Curve::read(tag.get(at..)?)?;
                list.push(curve);
                // Each starts on a multiple of four.
                at += size.next_multiple_of(4);
            }
            Some(list)
        };
        if b == 0 || clut == 0 || a == 0 {
            return None;
        }
        let input_curves = curves(a, inputs)?;
        let mut grid = Vec::with_capacity(inputs);
        for index in 0..inputs {
            let points = usize::from(*tag.get(clut + index)?);
            if points < 2 {
                return None;
            }
            grid.push(points);
        }
        let precision = *tag.get(clut + 16)?;
        let total = grid
            .iter()
            .try_fold(3usize, |total, points| total.checked_mul(*points))?;
        if total > MOST_TABLE_ENTRIES {
            return None;
        }
        let start = clut + 20;
        let table = (0..total)
            .map(|index| match precision {
                1 => Some(f32::from(*tag.get(start + index)?) / 255.0),
                2 => Some(f32::from(read_u16(tag, start + index * 2)?) / 65535.0),
                _ => None,
            })
            .collect::<Option<Vec<f32>>>()?;
        let middle_curves = if m != 0 { curves(m, 3)? } else { Vec::new() };
        let matrix = if matrix_at != 0 {
            let mut values = [0.0; 12];
            for (index, value) in values.iter_mut().enumerate() {
                *value = read_fixed(tag, matrix_at + index * 4)?;
            }
            Some(values)
        } else {
            None
        };
        Some(Self {
            inputs,
            input_curves,
            grid,
            table,
            middle_curves,
            matrix,
            output_curves: curves(b, 3)?,
            lab,
            legacy_16_bit_lab: false,
        })
    }

    /// The three values at a point in the table, given which point it is along each input.
    #[inline]
    fn at(&self, offset: usize) -> [f32; 3] {
        [
            self.table[offset],
            self.table[offset + 1],
            self.table[offset + 2],
        ]
    }

    /// Looks up the last three inputs, between the points around them, going by the shortest way from
    /// one corner of the cube they're in to the other, which is what keeps grays gray.
    fn look_up_3(&self, base: usize, values: &[f32], grid: &[usize], steps: &[usize]) -> [f32; 3] {
        let locate = |value: f32, points: usize| {
            let place = value.clamp(0.0, 1.0) * (points - 1) as f32;
            let low = (place as usize).min(points - 2);
            (low, place - low as f32)
        };
        let (x, rx) = locate(values[0], grid[0]);
        let (y, ry) = locate(values[1], grid[1]);
        let (z, rz) = locate(values[2], grid[2]);
        let origin = base + x * steps[0] + y * steps[1] + z * steps[2];
        let c000 = self.at(origin);
        let c111 = self.at(origin + steps[0] + steps[1] + steps[2]);
        let (sx, sy, sz) = (steps[0], steps[1], steps[2]);
        let order: [(usize, f32); 3] = if rx >= ry {
            if ry >= rz {
                [(sx, rx), (sy, ry), (sz, rz)]
            } else if rx >= rz {
                [(sx, rx), (sz, rz), (sy, ry)]
            } else {
                [(sz, rz), (sx, rx), (sy, ry)]
            }
        } else if rx >= rz {
            [(sy, ry), (sx, rx), (sz, rz)]
        } else if ry >= rz {
            [(sy, ry), (sz, rz), (sx, rx)]
        } else {
            [(sz, rz), (sy, ry), (sx, rx)]
        };
        let first = self.at(origin + order[0].0);
        let second = self.at(origin + order[0].0 + order[1].0);
        let mut out = [0.0; 3];
        for part in 0..3 {
            out[part] = c000[part]
                + (first[part] - c000[part]) * order[0].1
                + (second[part] - first[part]) * order[1].1
                + (c111[part] - second[part]) * order[2].1;
        }
        out
    }

    fn look_up(&self, values: &[f32]) -> [f32; 3] {
        let mut steps = [3usize; 4];
        for index in (0..self.inputs.saturating_sub(1)).rev() {
            steps[index] = steps[index + 1] * self.grid[index + 1];
        }
        match self.inputs {
            1 => {
                let points = self.grid[0];
                let place = values[0].clamp(0.0, 1.0) * (points - 1) as f32;
                let low = (place as usize).min(points - 2);
                let part = place - low as f32;
                let (a, b) = (self.at(low * 3), self.at((low + 1) * 3));
                [
                    a[0] + (b[0] - a[0]) * part,
                    a[1] + (b[1] - a[1]) * part,
                    a[2] + (b[2] - a[2]) * part,
                ]
            }
            3 => self.look_up_3(0, values, &self.grid, &steps),
            _ => {
                // One input by itself, and the other three as usual.
                let points = self.grid[0];
                let place = values[0].clamp(0.0, 1.0) * (points - 1) as f32;
                let low = (place as usize).min(points - 2);
                let part = place - low as f32;
                let a = self.look_up_3(low * steps[0], &values[1..], &self.grid[1..], &steps[1..]);
                let b = self.look_up_3(
                    (low + 1) * steps[0],
                    &values[1..],
                    &self.grid[1..],
                    &steps[1..],
                );
                [
                    a[0] + (b[0] - a[0]) * part,
                    a[1] + (b[1] - a[1]) * part,
                    a[2] + (b[2] - a[2]) * part,
                ]
            }
        }
    }

    /// From a picture's colors to the space profiles work in.
    fn eval(&self, input: &[f32]) -> [f32; 3] {
        let mut values = [0.0; 4];
        for (index, curve) in self.input_curves.iter().enumerate() {
            values[index] = curve.eval(input[index]);
        }
        let mut out = self.look_up(&values[..self.inputs]);
        if !self.middle_curves.is_empty() {
            for (value, curve) in out.iter_mut().zip(&self.middle_curves) {
                *value = curve.eval(*value);
            }
        }
        if let Some(m) = &self.matrix {
            let [a, b, c] = out;
            out = [
                m[0] * a + m[1] * b + m[2] * c + m[9],
                m[3] * a + m[4] * b + m[5] * c + m[10],
                m[6] * a + m[7] * b + m[8] * c + m[11],
            ];
        }
        for (value, curve) in out.iter_mut().zip(&self.output_curves) {
            *value = curve.eval(*value);
        }
        if self.lab {
            let scale = if self.legacy_16_bit_lab {
                65535.0 / 65280.0
            } else {
                1.0
            };
            let l = out[0] * scale * 100.0;
            let a = out[1] * scale * 255.0 - 128.0;
            let b = out[2] * scale * 255.0 - 128.0;
            lab_to_xyz(l, a, b)
        } else {
            // In XYZ, 1 + 32767/32768 is the top of the range.
            [out[0] * 1.999_97, out[1] * 1.999_97, out[2] * 1.999_97]
        }
    }
}

fn lab_to_xyz(l: f32, a: f32, b: f32) -> [f32; 3] {
    let y = (l + 16.0) / 116.0;
    let x = a / 500.0 + y;
    let z = y - b / 200.0;
    let curve = |t: f32| {
        if t > 6.0 / 29.0 {
            t * t * t
        } else {
            3.0 * (6.0f32 / 29.0).powi(2) * (t - 4.0 / 29.0)
        }
    };
    [curve(x) * D50[0], curve(y) * D50[1], curve(z) * D50[2]]
}

/// From linear light to what's stored for a screen, by how bright it is, in 4096 steps.
fn to_screen_table() -> &'static [u8; 4096] {
    static TABLE: OnceLock<Box<[u8; 4096]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = Box::new([0u8; 4096]);
        for (index, value) in table.iter_mut().enumerate() {
            let linear = index as f32 / 4095.0;
            let stored = if linear <= 0.003_130_8 {
                12.92 * linear
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            };
            *value = (stored * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
        }
        table
    })
}

#[inline]
fn to_screen(linear: f32) -> u8 {
    to_screen_table()[(linear.clamp(0.0, 1.0) * 4095.0 + 0.5) as usize]
}

#[inline]
fn apply_matrix(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn multiply(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (row, line) in out.iter_mut().enumerate() {
        for (column, value) in line.iter_mut().enumerate() {
            *value = (0..3).map(|at| a[row][at] * b[at][column]).sum();
        }
    }
    out
}

/// How to turn a picture's colors into what a screen shows.
#[derive(Debug, Clone)]
pub(crate) enum ColorTransform {
    /// Each value through a curve, then a matrix.
    Matrix {
        curves: Box<[[f32; 256]; 3]>,
        matrix: [[f32; 3]; 3],
    },
    /// By way of a table, for profiles made from measurements, such as for printing.
    Lut(Box<LutPipeline>),
}

fn sampled(curve: &Curve) -> [f32; 256] {
    let mut table = [0.0; 256];
    for (index, value) in table.iter_mut().enumerate() {
        *value = curve.eval(index as f32 / 255.0);
    }
    table
}

impl ColorTransform {
    /// Reads a profile, for a picture of this kind. `None` if it can't be used or, as is most common,
    /// it wouldn't change anything.
    pub(crate) fn from_icc(profile: &[u8], kind: ProfileKind) -> Option<Self> {
        if profile.get(36..40)? != b"acsp" {
            return None;
        }
        let space = profile.get(16..20)?;
        let fits = match kind {
            ProfileKind::Rgb => space == b"RGB ",
            ProfileKind::Gray => space == b"GRAY",
            ProfileKind::Cmyk => space == b"CMYK",
        };
        if !fits {
            return None;
        }
        let lab = profile.get(20..24)? == b"Lab ";
        // A table is what's been measured, and if there is one, it's what's meant to be used.
        if let Some(tag) = find_tag(profile, b"A2B0").or_else(|| find_tag(profile, b"A2B1")) {
            let inputs = match kind {
                ProfileKind::Rgb => 3,
                ProfileKind::Gray => 1,
                ProfileKind::Cmyk => 4,
            };
            if let Some(pipeline) = LutPipeline::read(tag, inputs, lab) {
                return Some(Self::Lut(Box::new(pipeline)))
                    .filter(|transform| kind == ProfileKind::Cmyk || !transform.changes_nothing());
            }
        }
        match kind {
            ProfileKind::Rgb => {
                let curve_of = |name: &[u8; 4]| Some(Curve::read(find_tag(profile, name)?)?.0);
                let curves = [curve_of(b"rTRC")?, curve_of(b"gTRC")?, curve_of(b"bTRC")?];
                let (r, g, b) = (
                    read_xyz(find_tag(profile, b"rXYZ")?)?,
                    read_xyz(find_tag(profile, b"gXYZ")?)?,
                    read_xyz(find_tag(profile, b"bXYZ")?)?,
                );
                let to_d50 = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
                Self::from_curves_and_matrix(&curves, multiply(&FROM_D50, &to_d50))
            }
            ProfileKind::Gray => {
                let curve = Curve::read(find_tag(profile, b"kTRC")?)?.0;
                let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
                Self::from_curves_and_matrix(&[curve.clone(), curve.clone(), curve], identity)
            }
            ProfileKind::Cmyk => None,
        }
    }

    /// From a PNG's older way of saying what it is, as a gamma and where its primaries and white are.
    pub(crate) fn from_gamma_and_primaries(
        gamma: Option<f32>,
        primaries: Option<[f32; 8]>,
    ) -> Option<Self> {
        // A gamma of about a half and nothing else is how most PNGs say that they're ordinary.
        let power = gamma.filter(|gamma| *gamma > 0.0).map_or(2.2, |g| 1.0 / g);
        if primaries.is_none() && (power - 2.2).abs() < 0.11 {
            return None;
        }
        let curve = Curve::Power(power);
        let matrix = match primaries {
            Some(primaries) => primaries_to_screen(primaries)?,
            None => [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        Self::from_curves_and_matrix(&[curve.clone(), curve.clone(), curve], matrix)
    }

    fn from_curves_and_matrix(curves: &[Curve; 3], matrix: [[f32; 3]; 3]) -> Option<Self> {
        let transform = Self::Matrix {
            curves: Box::new([
                sampled(&curves[0]),
                sampled(&curves[1]),
                sampled(&curves[2]),
            ]),
            matrix,
        };
        (!transform.changes_nothing()).then_some(transform)
    }

    fn changes_nothing(&self) -> bool {
        const STEP: usize = 51;
        for r in (0..=255).step_by(STEP) {
            for g in (0..=255).step_by(STEP) {
                for b in (0..=255).step_by(STEP) {
                    let out = self.rgb([r as u8, g as u8, b as u8]);
                    let close = |a: u8, b: usize| (i32::from(a) - b as i32).abs() <= SAME_AS_SCREEN;
                    if !close(out[0], r) || !close(out[1], g) || !close(out[2], b) {
                        return false;
                    }
                }
            }
        }
        true
    }

    #[inline]
    fn rgb(&self, input: [u8; 3]) -> [u8; 3] {
        match self {
            Self::Matrix { curves, matrix } => {
                let linear = [
                    curves[0][usize::from(input[0])],
                    curves[1][usize::from(input[1])],
                    curves[2][usize::from(input[2])],
                ];
                let out = apply_matrix(matrix, linear);
                [to_screen(out[0]), to_screen(out[1]), to_screen(out[2])]
            }
            Self::Lut(pipeline) => {
                let values = if pipeline.inputs == 1 {
                    [f32::from(input[0]) / 255.0, 0.0, 0.0]
                } else {
                    [
                        f32::from(input[0]) / 255.0,
                        f32::from(input[1]) / 255.0,
                        f32::from(input[2]) / 255.0,
                    ]
                };
                let out = apply_matrix(&FROM_D50, pipeline.eval(&values));
                [to_screen(out[0]), to_screen(out[1]), to_screen(out[2])]
            }
        }
    }

    /// Changes a row of red, green, blue and alpha, leaving alpha as it is.
    pub(crate) fn apply_row(&self, row: &mut [u8]) {
        let mut last = ([0u8; 3], self.rgb([0; 3]));
        for pixel in row.chunks_exact_mut(4) {
            let input = [pixel[0], pixel[1], pixel[2]];
            // Pictures often have runs of one color, and working one out isn't always cheap.
            if input != last.0 {
                last = (input, self.rgb(input));
            }
            pixel[..3].copy_from_slice(&last.1);
        }
    }

    /// From amounts of ink, where 255 means all of it, to what a screen shows.
    pub(crate) fn ink_to_screen(&self, ink: [u8; 4]) -> [u8; 3] {
        match self {
            Self::Lut(pipeline) if pipeline.inputs == 4 => {
                let values = [
                    f32::from(ink[0]) / 255.0,
                    f32::from(ink[1]) / 255.0,
                    f32::from(ink[2]) / 255.0,
                    f32::from(ink[3]) / 255.0,
                ];
                let out = apply_matrix(&FROM_D50, pipeline.eval(&values));
                [to_screen(out[0]), to_screen(out[1]), to_screen(out[2])]
            }
            _ => {
                let part = |value: u8| {
                    let left = u32::from(255 - value) * u32::from(255 - ink[3]);
                    ((left + 127) / 255) as u8
                };
                [part(ink[0]), part(ink[1]), part(ink[2])]
            }
        }
    }
}

/// From a color space given by where its primaries and white are, to linear light for a screen.
fn primaries_to_screen(p: [f32; 8]) -> Option<[[f32; 3]; 3]> {
    let [
        white_x,
        white_y,
        red_x,
        red_y,
        green_x,
        green_y,
        blue_x,
        blue_y,
    ] = p;
    if red_y <= 0.0 || green_y <= 0.0 || blue_y <= 0.0 || white_y <= 0.0 {
        return None;
    }
    let column = |x: f32, y: f32| [x / y, 1.0, (1.0 - x - y) / y];
    let (r, g, b) = (
        column(red_x, red_y),
        column(green_x, green_y),
        column(blue_x, blue_y),
    );
    let white = column(white_x, white_y);
    let raw = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
    let scale = apply_matrix(&invert(&raw)?, white);
    let to_xyz = [
        [
            raw[0][0] * scale[0],
            raw[0][1] * scale[1],
            raw[0][2] * scale[2],
        ],
        [
            raw[1][0] * scale[0],
            raw[1][1] * scale[1],
            raw[1][2] * scale[2],
        ],
        [
            raw[2][0] * scale[0],
            raw[2][1] * scale[1],
            raw[2][2] * scale[2],
        ],
    ];
    // Its white has to become the white the rest of this works with.
    const BRADFORD: [[f32; 3]; 3] = [
        [0.895_1, 0.266_4, -0.161_4],
        [-0.750_2, 1.713_5, 0.036_7],
        [0.038_9, -0.068_5, 1.029_6],
    ];
    let from_cone = apply_matrix(&BRADFORD, white);
    let to_cone = apply_matrix(&BRADFORD, D50);
    let scale = [
        [to_cone[0] / from_cone[0], 0.0, 0.0],
        [0.0, to_cone[1] / from_cone[1], 0.0],
        [0.0, 0.0, to_cone[2] / from_cone[2]],
    ];
    let adapt = multiply(&invert(&BRADFORD)?, &multiply(&scale, &BRADFORD));
    Some(multiply(&FROM_D50, &multiply(&adapt, &to_xyz)))
}

fn invert(m: &[[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let cofactor = |a: usize, b: usize, c: usize, d: usize| m[a][b] * m[c][d];
    let c00 = cofactor(1, 1, 2, 2) - cofactor(1, 2, 2, 1);
    let c01 = cofactor(1, 2, 2, 0) - cofactor(1, 0, 2, 2);
    let c02 = cofactor(1, 0, 2, 1) - cofactor(1, 1, 2, 0);
    let det = m[0][0] * c00 + m[0][1] * c01 + m[0][2] * c02;
    if det.abs() < 1e-9 {
        return None;
    }
    Some([
        [
            c00 / det,
            (cofactor(0, 2, 2, 1) - cofactor(0, 1, 2, 2)) / det,
            (cofactor(0, 1, 1, 2) - cofactor(0, 2, 1, 1)) / det,
        ],
        [
            c01 / det,
            (cofactor(0, 0, 2, 2) - cofactor(0, 2, 2, 0)) / det,
            (cofactor(0, 2, 1, 0) - cofactor(0, 0, 1, 2)) / det,
        ],
        [
            c02 / det,
            (cofactor(0, 1, 2, 0) - cofactor(0, 0, 2, 1)) / det,
            (cofactor(0, 0, 1, 1) - cofactor(0, 1, 1, 0)) / det,
        ],
    ])
}
