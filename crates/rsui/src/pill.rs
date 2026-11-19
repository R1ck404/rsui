//! The small pill in the corner of a window, in a program that's being worked on: how long the last frame took, the
//! last few as bars, and how many frames a second there are while something's moving. Clicking it opens the inspector,
//! and its × puts it away until the program starts again.
//!
//! It's only drawn when the app draws anyway, so it never costs a frame of its own: what it shows is the frame before.

use std::collections::VecDeque;

use rsui_render::{Color, Overlay, OverlayBox, OverlayText};

/// How many frames the bars show.
const BARS: usize = 24;
/// How many frames are kept, for counting how many there were in the last second.
const KEPT: usize = 240;
/// What a frame has to fit in, at 60 a second, in nanoseconds.
const BUDGET: u64 = 16_666_667;

const BACKGROUND: Color = Color::rgba(0x16, 0x1B, 0x22, 0xE0);
const BACKGROUND_HOVER: Color = Color::rgba(0x22, 0x2A, 0x34, 0xF0);
const EDGE: Color = Color::rgba(0xFF, 0xFF, 0xFF, 0x1F);
const TEXT: Color = Color::hex(0xE6EDF3);
const DIM: Color = Color::hex(0x8B949E);
const GOOD: Color = Color::hex(0x3FB950);
const TIGHT: Color = Color::hex(0xD29922);
const OVER: Color = Color::hex(0xF85149);

/// Where on the pill something is, in logical pixels from its left.
const WIDTH: f32 = 176.0;
const HEIGHT: f32 = 22.0;
const MARGIN: f32 = 8.0;
const CLOSE: f32 = 18.0;

/// What a click on the pill was on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PillHit {
    Open,
    Close,
}

#[derive(Default)]
pub(crate) struct Pill {
    /// Whether this window has one, as only a window does, in a program being worked on.
    pub(crate) wanted: bool,
    dismissed: bool,
    /// Each frame: when it started, when it was handed to the GPU, and when the screen took it, in nanoseconds.
    frames: VecDeque<(u64, u64, u64)>,
    /// Where it is, in physical pixels, as it was last drawn.
    rect: [f32; 4],
    scale: f32,
    pub(crate) hovered: bool,
    /// Whether a press started on it, so the release that goes with it is its too.
    pub(crate) pressed: bool,
}

impl Pill {
    pub(crate) fn showing(&self) -> bool {
        self.wanted && !self.dismissed
    }

    /// Keeps a frame that was drawn: from `began` it worked, at `gpu` it started handing it to the GPU, by `handed` it
    /// had, and at `end` the screen took it.
    pub(crate) fn frame(&mut self, began: u64, _gpu: u64, handed: u64, end: u64) {
        if !self.wanted {
            return;
        }
        if self.frames.len() >= KEPT {
            self.frames.pop_front();
        }
        self.frames.push_back((began, handed, end));
    }

    pub(crate) fn dismiss(&mut self) {
        self.dismissed = true;
        self.hovered = false;
    }

    /// What's at `x, y`, in physical pixels, if it's on the pill.
    pub(crate) fn hit(&self, x: f32, y: f32) -> Option<PillHit> {
        if !self.showing() || self.rect[2] == 0.0 {
            return None;
        }
        let [left, top, width, height] = self.rect;
        if x < left || y < top || x >= left + width || y >= top + height {
            return None;
        }
        Some(if x >= left + width - CLOSE * self.scale {
            PillHit::Close
        } else {
            PillHit::Open
        })
    }

    /// Draws it into `overlay`, in the bottom right corner of a window `size` big.
    pub(crate) fn draw(&mut self, overlay: &mut Overlay, scale: f32, size: (u32, u32)) {
        if !self.showing() {
            return;
        }
        let s = scale;
        self.scale = s;
        let (width, height) = (WIDTH * s, HEIGHT * s);
        let left = size.0 as f32 - width - MARGIN * s;
        let top = size.1 as f32 - height - MARGIN * s;
        if left < 0.0 || top < 0.0 {
            self.rect = [0.0; 4];
            return;
        }
        self.rect = [left, top, width, height];
        let radius = height / 2.0;
        overlay.boxes.push(OverlayBox {
            rect: self.rect,
            color: if self.hovered { BACKGROUND_HOVER } else { BACKGROUND },
            outline: 0.0,
            radius,
        });
        overlay.boxes.push(OverlayBox {
            rect: self.rect,
            color: EDGE,
            outline: s.max(1.0),
            radius,
        });

        let last = self.frames.back().copied();
        let took = last.map(|(began, handed, _)| handed.saturating_sub(began));
        let mut x = left + 9.0 * s;
        let middle = top + height / 2.0;
        // A dot in the color of the last frame: in time, close, or over.
        let dot = 7.0 * s;
        overlay.boxes.push(OverlayBox {
            rect: [x, middle - dot / 2.0, dot, dot],
            color: took.map_or(DIM, color_of),
            outline: 0.0,
            radius: dot / 2.0,
        });
        x += dot + 3.0 * s;
        overlay.texts.push(OverlayText {
            text: took.map_or_else(|| "–".to_owned(), |took| format!("{:.1} ms", took as f64 / 1e6)),
            left: x,
            top: top + 2.0 * s,
            size: 11.0,
            widest: 60.0,
            color: TEXT,
            background: Color::TRANSPARENT,
        });
        x += 52.0 * s;

        // The last few frames, as bars the height of their time, up to two frames' worth.
        let (bar, gap, tall) = (2.0 * s, 1.0 * s, 12.0 * s);
        let bottom = middle + tall / 2.0;
        let shown: Vec<&(u64, u64, u64)> = self.frames.iter().rev().take(BARS).collect();
        for (at, (began, handed, _)) in shown.iter().rev().enumerate() {
            let took = handed.saturating_sub(*began);
            let high = ((took as f32 / (2 * BUDGET) as f32) * tall).clamp(s, tall);
            overlay.boxes.push(OverlayBox {
                rect: [x + at as f32 * (bar + gap), bottom - high, bar, high],
                color: color_of(took),
                outline: 0.0,
                radius: 0.0,
            });
        }
        x += BARS as f32 * (bar + gap) + 5.0 * s;

        // Frames a second, only while there are enough to count: an app that's drawn once since it was clicked isn't
        // running at 1 a second.
        let fps = last.map_or(0, |(_, _, end)| {
            self.frames
                .iter()
                .rev()
                .take_while(|(_, _, at)| end.saturating_sub(*at) < 1_000_000_000)
                .count()
        });
        overlay.texts.push(OverlayText {
            text: if fps >= 5 { format!("{fps} fps") } else { "idle".to_owned() },
            left: x,
            top: top + 2.0 * s,
            size: 11.0,
            widest: 50.0,
            color: DIM,
            background: Color::TRANSPARENT,
        });

        overlay.texts.push(OverlayText {
            text: "×".to_owned(),
            left: left + width - CLOSE * s + 2.0 * s,
            top: top + 1.5 * s,
            size: 12.0,
            widest: 20.0,
            color: if self.hovered { TEXT } else { DIM },
            background: Color::TRANSPARENT,
        });
        if self.hovered {
            overlay.texts.push(OverlayText {
                text: "Open the inspector  (F12)".to_owned(),
                left: left + width - 160.0 * s,
                top: top - 26.0 * s,
                size: 11.0,
                widest: 200.0,
                color: TEXT,
                background: BACKGROUND,
            });
        }
    }
}

fn color_of(took: u64) -> Color {
    if took > BUDGET {
        OVER
    } else if took > BUDGET / 2 {
        TIGHT
    } else {
        GOOD
    }
}
