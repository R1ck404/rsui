//! Pictures from SVG files, as an `<img>` or a CSS background shows them, drawn the way shapes are.
//!
//! What's read is what most SVG files are made of: paths and the other shapes, in groups, with their fills, strokes,
//! transforms and opacity, said as attributes, in `style` or in a `<style>` by class, id or kind of element. A gradient is
//! drawn as the color it's mostly made of. Text, masks, clipping, patterns and filters aren't drawn.

use std::collections::HashMap;

use rsui_image::Picture;
use rsui_text::Rasterizer;

use crate::paint::Color;
use crate::shape::draw_layers;

/// The most a picture's drawn at, either way, as a shape is.
const LARGEST: u32 = 4096;

/// Whether these bytes are an SVG file, rather than a picture made of pixels.
pub(crate) fn is_svg(bytes: &[u8]) -> bool {
    let start = &bytes[..bytes.len().min(64 * 1024)];
    let first = start
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map_or(&[][..], |at| &start[at..]);
    let first = first
        .strip_prefix(b"\xEF\xBB\xBF".as_slice())
        .unwrap_or(first);
    first.first() == Some(&b'<') && start.windows(4).any(|four| four == b"<svg")
}

/// How big an SVG file says it is, in CSS pixels, and whether it says, rather than only having a `viewBox` that says what shape it
/// is, which a browser makes as wide as there's room for.
pub(crate) fn svg_size(bytes: &[u8]) -> Option<((u32, u32), bool)> {
    let text = String::from_utf8_lossy(bytes);
    let root = Tokens::new(&text).find_map(|token| match token {
        Token::Open(tag) if tag.name == "svg" => Some(tag),
        _ => None,
    })?;
    let (width, height) = root_size(&root);
    let said = |name: &str| {
        root.get(name)
            .and_then(absolute_length)
            .is_some_and(|size| size > 0.0)
    };
    let own = said("width") || said("height") || root.get("viewBox").and_then(view_box).is_none();
    Some((
        (
            width.round().clamp(1.0, LARGEST as f32) as u32,
            height.round().clamp(1.0, LARGEST as f32) as u32,
        ),
        own,
    ))
}

/// Draws an SVG file to fit `size`, in pixels, keeping its shape unless it says not to.
pub(crate) fn draw_svg(bytes: &[u8], size: (u32, u32)) -> Option<Picture> {
    let text = String::from_utf8_lossy(bytes);
    let document = read(&text)?;
    let (width, height) = (size.0.clamp(1, LARGEST), size.1.clamp(1, LARGEST));
    let mut rasterizer = Rasterizer::default();
    let bitmap = draw_layers(
        &document.layers,
        document.view,
        document.stretch,
        (width as f32, height as f32),
        Color::BLACK,
        &mut rasterizer,
    )?;
    Some(Picture {
        width: bitmap.width,
        height: bitmap.height,
        pixels: bitmap.pixels,
    })
}

/// What an SVG file draws, as a shape's layers.
struct Document {
    layers: String,
    view: Option<[f32; 4]>,
    stretch: bool,
}

/// How big the `<svg>` at the root says it is, by its width and height, or the shape of its `viewBox`, or as big as a browser
/// makes one that says nothing.
fn root_size(root: &Tag) -> (f32, f32) {
    let said = |name: &str| {
        root.get(name)
            .and_then(absolute_length)
            .filter(|size| *size > 0.0)
    };
    let view = root.get("viewBox").and_then(view_box);
    match (said("width"), said("height"), view) {
        (Some(width), Some(height), _) => (width, height),
        (Some(width), None, Some(view)) => (width, width * view[3] / view[2]),
        (None, Some(height), Some(view)) => (height * view[2] / view[3], height),
        (None, None, Some(view)) => (view[2], view[3]),
        (width, height, None) => (width.unwrap_or(300.0), height.unwrap_or(150.0)),
    }
}

fn view_box(text: &str) -> Option<[f32; 4]> {
    let mut parts = text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(str::parse::<f32>);
    let mut next = || parts.next()?.ok();
    let view = [next()?, next()?, next()?, next()?];
    (view[2] > 0.0 && view[3] > 0.0).then_some(view)
}

/// A length that doesn't depend on anything around it, in pixels.
fn absolute_length(text: &str) -> Option<f32> {
    let text = text.trim();
    for (unit, pixels) in [
        ("px", 1.0),
        ("pt", 4.0 / 3.0),
        ("pc", 16.0),
        ("in", 96.0),
        ("cm", 96.0 / 2.54),
        ("mm", 96.0 / 25.4),
    ] {
        if let Some(number) = text.strip_suffix(unit) {
            return number
                .trim()
                .parse::<f32>()
                .ok()
                .map(|number| number * pixels);
        }
    }
    text.parse().ok()
}

/// A length, where a percentage is of `whole`.
fn length(text: &str, whole: f32) -> f32 {
    let text = text.trim();
    if let Some(percent) = text.strip_suffix('%') {
        return percent
            .trim()
            .parse::<f32>()
            .map_or(0.0, |percent| percent / 100.0 * whole);
    }
    absolute_length(text).unwrap_or(0.0)
}

fn read(text: &str) -> Option<Document> {
    let sheet = Sheet::gather(text);
    let mut tokens = Tokens::new(text);
    let root = tokens.by_ref().find_map(|token| match token {
        Token::Open(tag) if tag.name == "svg" => Some(tag),
        _ => None,
    })?;
    let (width, height) = root_size(&root);
    // What it says it shows, or what's as big as it is from the top left, as it'd be on a page.
    let view = root
        .get("viewBox")
        .and_then(view_box)
        .or(Some([0.0, 0.0, width, height]));
    let stretch = root
        .get("preserveAspectRatio")
        .is_some_and(|how| how.trim().starts_with("none"));
    let whole = view.map_or((width, height), |view| (view[2], view[3]));
    let mut walker = Walker {
        sheet: &sheet,
        whole,
        layers: String::new(),
        states: Vec::new(),
        skipping: 0,
    };
    let start = walker.state_of(&root, &State::default());
    walker.states.push(start);
    if root.empty {
        return Some(Document {
            layers: String::new(),
            view,
            stretch,
        });
    }
    for token in tokens {
        match token {
            Token::Open(tag) => walker.open(&tag),
            Token::Close(_) => {
                if walker.skipping > 0 {
                    walker.skipping -= 1;
                } else if walker.states.len() > 1 {
                    walker.states.pop();
                } else {
                    break;
                }
            }
            Token::Text(_) => {}
        }
    }
    Some(Document {
        layers: walker.layers,
        view,
        stretch,
    })
}

/// What's said about how something's drawn, which what's inside it goes on with.
#[derive(Debug, Clone)]
struct State {
    fill: Option<Color>,
    stroke: Option<Color>,
    stroke_width: f32,
    cap: &'static str,
    join: &'static str,
    dashes: String,
    dash_offset: f32,
    fill_opacity: f32,
    stroke_opacity: f32,
    /// What everything around it multiplies how opaque it is by, which isn't quite what a browser does, as that draws a
    /// group by itself first, but is the same unless what's in a group overlaps.
    opacity: f32,
    color: Color,
    transform: String,
    hidden: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            fill: Some(Color::BLACK),
            stroke: None,
            stroke_width: 1.0,
            cap: "butt",
            join: "miter",
            dashes: String::new(),
            dash_offset: 0.0,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
            opacity: 1.0,
            color: Color::BLACK,
            transform: String::new(),
            hidden: false,
        }
    }
}

/// Kinds of element that draw nothing, and nothing in them does either, unless something else points to it.
const NOT_DRAWN: &[&str] = &[
    "defs",
    "clipPath",
    "mask",
    "pattern",
    "symbol",
    "marker",
    "linearGradient",
    "radialGradient",
    "style",
    "title",
    "desc",
    "metadata",
    "filter",
    "text",
    "foreignObject",
    "script",
    "image",
];

struct Walker<'a> {
    sheet: &'a Sheet,
    /// How big the picture is in its own numbers, which a percentage is of.
    whole: (f32, f32),
    layers: String,
    states: Vec<State>,
    /// How many elements deep it is in something that's not drawn.
    skipping: usize,
}

impl Walker<'_> {
    fn open(&mut self, tag: &Tag) {
        if self.skipping > 0 || NOT_DRAWN.contains(&tag.name) {
            if !tag.empty {
                self.skipping += 1;
            }
            return;
        }
        let Some(around) = self.states.last() else {
            return;
        };
        let state = self.state_of(tag, around);
        let hidden = self
            .said(tag, "display")
            .is_some_and(|display| display.trim() == "none");
        if hidden {
            if !tag.empty {
                self.skipping += 1;
            }
            return;
        }
        if let Some(d) = self.path_of(tag) {
            if !state.hidden {
                self.add_layer(&d, &state);
            }
        }
        if !tag.empty {
            self.states.push(state);
        }
    }

    /// What an element says about something, as CSS, a `style` or an attribute does, which is the order they count in.
    fn said<'t>(&'t self, tag: &'t Tag, name: &str) -> Option<&'t str> {
        if let Some(style) = tag.get("style") {
            if let Some(value) = declarations(style)
                .rev()
                .find(|(said, _)| *said == name)
                .map(|(_, value)| value)
            {
                return Some(value);
            }
        }
        if let Some(value) = self.sheet.said(tag, name) {
            return Some(value);
        }
        tag.get(name)
    }

    fn state_of(&self, tag: &Tag, around: &State) -> State {
        let mut state = around.clone();
        if let Some(color) = self
            .said(tag, "color")
            .and_then(|text| self.sheet.color(text, around.color))
        {
            state.color = color;
        }
        let paint = |text: &str, current: Color| {
            let text = text.trim();
            if text == "none" {
                None
            } else {
                self.sheet.color(text, current)
            }
        };
        if let Some(text) = self.said(tag, "fill") {
            state.fill = paint(text, state.color);
        }
        if let Some(text) = self.said(tag, "stroke") {
            state.stroke = paint(text, state.color);
        }
        let number = |name: &str| {
            self.said(tag, name)
                .map(|text| length(text, self.whole.0.hypot(self.whole.1) / 2f32.sqrt()))
        };
        if let Some(width) = number("stroke-width") {
            state.stroke_width = width.max(0.0);
        }
        if let Some(offset) = number("stroke-dashoffset") {
            state.dash_offset = offset;
        }
        let fraction = |name: &str| {
            self.said(tag, name).and_then(|text| {
                let text = text.trim();
                match text.strip_suffix('%') {
                    Some(percent) => percent
                        .trim()
                        .parse::<f32>()
                        .ok()
                        .map(|percent| percent / 100.0),
                    None => text.parse::<f32>().ok(),
                }
                .map(|value| value.clamp(0.0, 1.0))
            })
        };
        if let Some(opacity) = fraction("fill-opacity") {
            state.fill_opacity = opacity;
        }
        if let Some(opacity) = fraction("stroke-opacity") {
            state.stroke_opacity = opacity;
        }
        if let Some(opacity) = fraction("opacity") {
            state.opacity *= opacity;
        }
        if let Some(cap) = self.said(tag, "stroke-linecap") {
            state.cap = match cap.trim() {
                "round" => "round",
                "square" => "square",
                _ => "butt",
            };
        }
        if let Some(join) = self.said(tag, "stroke-linejoin") {
            state.join = match join.trim() {
                "round" => "round",
                "bevel" => "bevel",
                _ => "miter",
            };
        }
        if let Some(dashes) = self.said(tag, "stroke-dasharray") {
            state.dashes = if dashes.trim() == "none" {
                String::new()
            } else {
                dashes
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .filter(|part| !part.is_empty())
                    .map(|part| length(part, self.whole.0).to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            };
        }
        if let Some(visibility) = self.said(tag, "visibility") {
            state.hidden = matches!(visibility.trim(), "hidden" | "collapse");
        }
        let mut own = tag.get("transform").map(svg_transform).unwrap_or_default();
        // A `<svg>` inside another is moved to where it says, as a group.
        if tag.name == "svg" && !self.states.is_empty() {
            let (x, y) = (
                tag.get("x").map_or(0.0, |x| length(x, self.whole.0)),
                tag.get("y").map_or(0.0, |y| length(y, self.whole.1)),
            );
            if x != 0.0 || y != 0.0 {
                own = format!("translate({x} {y}) {own}");
            }
        }
        if !own.trim().is_empty() {
            state.transform = if state.transform.is_empty() {
                own
            } else {
                format!("{} {own}", state.transform)
            };
        }
        state
    }

    /// The path an element draws, as SVG's `d` writes it, if it's a shape.
    fn path_of(&self, tag: &Tag) -> Option<String> {
        let (across, down) = self.whole;
        let at = |name: &str, whole: f32| tag.get(name).map_or(0.0, |text| length(text, whole));
        Some(match tag.name {
            "path" => tag.get("d")?.to_owned(),
            "line" => format!(
                "M{} {}L{} {}",
                at("x1", across),
                at("y1", down),
                at("x2", across),
                at("y2", down)
            ),
            "polyline" | "polygon" => {
                let points = tag.get("points")?.trim();
                if points.is_empty() {
                    return None;
                }
                format!("M{points}{}", if tag.name == "polygon" { "Z" } else { "" })
            }
            "rect" => {
                let (x, y, width, height) = (
                    at("x", across),
                    at("y", down),
                    at("width", across),
                    at("height", down),
                );
                if width <= 0.0 || height <= 0.0 {
                    return None;
                }
                let rx = tag.get("rx").map(|text| length(text, across));
                let ry = tag.get("ry").map(|text| length(text, down));
                let (rx, ry) = match (rx, ry) {
                    (Some(rx), Some(ry)) => (rx, ry),
                    (Some(r), None) | (None, Some(r)) => (r, r),
                    (None, None) => (0.0, 0.0),
                };
                let (rx, ry) = (rx.clamp(0.0, width / 2.0), ry.clamp(0.0, height / 2.0));
                if rx <= 0.0 || ry <= 0.0 {
                    format!("M{x} {y}h{width}v{height}h{}Z", -width)
                } else {
                    let arc = format!("a{rx} {ry} 0 0 1");
                    let (along, between) = (width - 2.0 * rx, height - 2.0 * ry);
                    format!(
                        "M{} {y}h{along}{arc} {rx} {ry}v{between}{arc} {} {ry}h{}{arc} {} {}v{}{arc} {rx} {}Z",
                        x + rx,
                        -rx,
                        -along,
                        -rx,
                        -ry,
                        -between,
                        -ry
                    )
                }
            }
            "circle" | "ellipse" => {
                let (cx, cy) = (at("cx", across), at("cy", down));
                let (rx, ry) = if tag.name == "circle" {
                    let r = at("r", across.hypot(down) / 2f32.sqrt());
                    (r, r)
                } else {
                    (at("rx", across), at("ry", down))
                };
                if rx <= 0.0 || ry <= 0.0 {
                    return None;
                }
                format!(
                    "M{} {cy}a{rx} {ry} 0 1 1 {} 0a{rx} {ry} 0 1 1 {} 0Z",
                    cx + rx,
                    -2.0 * rx,
                    2.0 * rx
                )
            }
            _ => return None,
        })
    }

    fn add_layer(&mut self, d: &str, state: &State) {
        let paint = |color: Option<Color>, opacity: f32| match color {
            Some(color) => {
                let alpha = (f32::from(color.a) * opacity * state.opacity)
                    .round()
                    .clamp(0.0, 255.0) as u8;
                if alpha == 0 {
                    String::new()
                } else {
                    format!("#{:02x}{:02x}{:02x}{alpha:02x}", color.r, color.g, color.b)
                }
            }
            None => String::new(),
        };
        let fill = paint(state.fill, state.fill_opacity);
        let stroke = if state.stroke_width > 0.0 {
            paint(state.stroke, state.stroke_opacity)
        } else {
            String::new()
        };
        if fill.is_empty() && stroke.is_empty() {
            return;
        }
        let flat = |text: &str| text.replace(['\t', '\n', '\r'], " ");
        if !self.layers.is_empty() {
            self.layers.push('\n');
        }
        self.layers.push_str(&format!(
            "{fill}\t{stroke}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            state.stroke_width,
            state.cap,
            flat(d),
            flat(&state.transform),
            state.dashes,
            state.dash_offset,
            state.join
        ));
    }
}

/// A transform as SVG writes it, as CSS does: `rotate()` around a point is that point moved to, turned, and moved back.
fn svg_transform(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('(') {
        let Some(close) = rest[open..].find(')').map(|at| at + open) else {
            break;
        };
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let inside = &rest[open + 1..close];
        let numbers: Vec<&str> = inside
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|part| !part.is_empty())
            .collect();
        if name == "rotate" && numbers.len() == 3 {
            out.push_str(&format!(
                " translate({} {}) rotate({}) translate(-{} -{})",
                numbers[1], numbers[2], numbers[0], numbers[1], numbers[2]
            ));
            out = out.replace("--", "");
        } else {
            out.push_str(&format!(" {name}({})", numbers.join(" ")));
        }
        rest = &rest[close + 1..];
    }
    out.trim().to_owned()
}

/// What's in a `style`, or a rule: each name and what it says, in order.
fn declarations(text: &str) -> impl DoubleEndedIterator<Item = (&str, &str)> {
    text.split(';').filter_map(|one| {
        let (name, value) = one.split_once(':')?;
        let value = value.trim();
        let value = value.strip_suffix("!important").unwrap_or(value).trim();
        Some((name.trim(), value))
    })
}

/// One simple selector: of a kind of element, with an id and classes, any of which can be left out.
#[derive(Debug, Default)]
struct Selector {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
}

impl Selector {
    fn read(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty()
            || text.contains(|c: char| {
                c.is_whitespace() || matches!(c, '>' | '+' | '~' | ':' | '[' | '*')
            })
        {
            return None;
        }
        let mut selector = Self::default();
        let mut rest = text;
        let name_end = rest.find(['.', '#']).unwrap_or(rest.len());
        if name_end > 0 {
            selector.tag = Some(rest[..name_end].to_owned());
        }
        rest = &rest[name_end..];
        while let Some(mark) = rest.chars().next() {
            let end = rest[1..].find(['.', '#']).map_or(rest.len(), |at| at + 1);
            let name = rest[1..end].to_owned();
            if mark == '.' {
                selector.classes.push(name);
            } else {
                selector.id = Some(name);
            }
            rest = &rest[end..];
        }
        Some(selector)
    }

    fn matches(&self, tag: &Tag) -> bool {
        if self.tag.as_deref().is_some_and(|name| name != tag.name) {
            return false;
        }
        if self
            .id
            .as_deref()
            .is_some_and(|id| tag.get("id") != Some(id))
        {
            return false;
        }
        let classes = tag.get("class").unwrap_or("");
        self.classes
            .iter()
            .all(|class| classes.split_whitespace().any(|own| own == class))
    }
}

/// What a file's `<style>`s say, and the colors of its gradients.
#[derive(Default)]
struct Sheet {
    rules: Vec<(Selector, Vec<(String, String)>)>,
    gradients: HashMap<String, Color>,
}

impl Sheet {
    fn gather(text: &str) -> Self {
        let mut sheet = Self::default();
        let mut in_style = false;
        // Each gradient's stops, and which it takes them from if it has none of its own.
        let mut stops: HashMap<String, Vec<Color>> = HashMap::new();
        let mut borrowed: Vec<(String, String)> = Vec::new();
        let mut gradient: Option<String> = None;
        let mut style_text = String::new();
        for token in Tokens::new(text) {
            match token {
                Token::Open(tag) => match tag.name {
                    "style" => in_style = !tag.empty,
                    "linearGradient" | "radialGradient" => {
                        let id = tag.get("id").unwrap_or("").to_owned();
                        if let Some(from) = tag.get("href").or_else(|| tag.get("xlink:href")) {
                            borrowed
                                .push((id.clone(), from.trim().trim_start_matches('#').to_owned()));
                        }
                        stops.entry(id.clone()).or_default();
                        gradient = (!tag.empty).then_some(id);
                    }
                    "stop" => {
                        let Some(id) = &gradient else {
                            continue;
                        };
                        let mut color = tag.get("stop-color").map(str::to_owned);
                        let mut opacity = tag.get("stop-opacity").map(str::to_owned);
                        if let Some(style) = tag.get("style") {
                            for (name, value) in declarations(style) {
                                match name {
                                    "stop-color" => color = Some(value.to_owned()),
                                    "stop-opacity" => opacity = Some(value.to_owned()),
                                    _ => {}
                                }
                            }
                        }
                        let mut color = color
                            .and_then(|text| parse_color(&text, Color::BLACK))
                            .unwrap_or(Color::BLACK);
                        if let Some(opacity) =
                            opacity.and_then(|text| text.trim().parse::<f32>().ok())
                        {
                            color.a = (f32::from(color.a) * opacity.clamp(0.0, 1.0)).round() as u8;
                        }
                        stops.entry(id.clone()).or_default().push(color);
                    }
                    _ => {}
                },
                Token::Close(name) => match name {
                    "style" => in_style = false,
                    "linearGradient" | "radialGradient" => gradient = None,
                    _ => {}
                },
                Token::Text(inside) if in_style => style_text.push_str(inside),
                Token::Text(_) => {}
            }
        }
        for (id, from) in borrowed {
            if stops.get(&id).is_some_and(Vec::is_empty) {
                let theirs = stops.get(&from).cloned().unwrap_or_default();
                stops.insert(id, theirs);
            }
        }
        for (id, colors) in stops {
            if let Some(color) = average(&colors) {
                sheet.gradients.insert(id, color);
            }
        }
        sheet.read_rules(&style_text);
        sheet
    }

    fn read_rules(&mut self, text: &str) {
        // Comments go first, as they can have anything in them.
        let mut plain = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("/*") {
            plain.push_str(&rest[..start]);
            rest = rest[start + 2..]
                .find("*/")
                .map_or("", |end| &rest[start + 2 + end + 2..]);
        }
        plain.push_str(rest);
        for rule in plain.split('}') {
            let Some((selectors, body)) = rule.split_once('{') else {
                continue;
            };
            if selectors.trim_start().starts_with('@') {
                continue;
            }
            let said: Vec<(String, String)> = declarations(body)
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect();
            for selector in selectors.split(',').filter_map(Selector::read) {
                self.rules.push((selector, said.clone()));
            }
        }
    }

    /// What the rules that apply to an element say about something, where the last one that says counts, or one by id over
    /// one that isn't, as CSS has it.
    fn said<'a>(&'a self, tag: &Tag, name: &str) -> Option<&'a str> {
        let mut found: Option<(usize, &str)> = None;
        for (selector, said) in &self.rules {
            if !selector.matches(tag) {
                continue;
            }
            let Some((_, value)) = said.iter().rev().find(|(said, _)| said == name) else {
                continue;
            };
            let weight = usize::from(selector.id.is_some()) * 100
                + selector.classes.len() * 10
                + usize::from(selector.tag.is_some());
            if found.is_none_or(|(most, _)| weight >= most) {
                found = Some((weight, value));
            }
        }
        found.map(|(_, value)| value)
    }

    /// A color as it's written for a fill or a stroke, with a gradient as the color it mostly is.
    fn color(&self, text: &str, current: Color) -> Option<Color> {
        let text = text.trim();
        if let Some(inside) = text.strip_prefix("url(") {
            let (reference, after) = inside.split_once(')').unwrap_or((inside, ""));
            let id = reference
                .trim()
                .trim_matches(['"', '\''])
                .trim_start_matches('#');
            return self.gradients.get(id).copied().or_else(|| {
                let after = after.trim();
                if after.is_empty() || after == "none" {
                    None
                } else {
                    parse_color(after, current)
                }
            });
        }
        parse_color(text, current)
    }
}

/// The color a gradient of these mostly is.
fn average(colors: &[Color]) -> Option<Color> {
    if colors.is_empty() {
        return None;
    }
    let mut sums = [0f32; 4];
    for color in colors {
        let alpha = f32::from(color.a) / 255.0;
        sums[0] += f32::from(color.r) * alpha;
        sums[1] += f32::from(color.g) * alpha;
        sums[2] += f32::from(color.b) * alpha;
        sums[3] += alpha;
    }
    let count = colors.len() as f32;
    let part = |sum: f32| {
        if sums[3] > 0.0 {
            (sum / sums[3]).round().clamp(0.0, 255.0) as u8
        } else {
            0
        }
    };
    Some(Color::rgba(
        part(sums[0]),
        part(sums[1]),
        part(sums[2]),
        (sums[3] / count * 255.0).round() as u8,
    ))
}

/// A color as CSS writes it.
fn parse_color(text: &str, current: Color) -> Option<Color> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    if lower == "currentcolor" {
        return Some(current);
    }
    if lower == "transparent" {
        return Some(Color::TRANSPARENT);
    }
    if lower.starts_with('#') {
        return Color::from_text(&lower);
    }
    if let Some(open) = lower.find('(') {
        let name = lower[..open].trim();
        let inside = lower[open + 1..].trim_end_matches(')');
        let parts: Vec<&str> = inside
            .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
            .filter(|part| !part.is_empty())
            .collect();
        let alpha = parts.get(3).map_or(Some(1.0), |text| {
            let text = text.trim();
            match text.strip_suffix('%') {
                Some(percent) => percent.parse::<f32>().ok().map(|percent| percent / 100.0),
                None => text.parse::<f32>().ok(),
            }
        })?;
        let alpha = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        let channel = |text: &str| -> Option<u8> {
            let value = match text.strip_suffix('%') {
                Some(percent) => percent.parse::<f32>().ok()? * 2.55,
                None => text.parse::<f32>().ok()?,
            };
            Some(value.round().clamp(0.0, 255.0) as u8)
        };
        return match name {
            "rgb" | "rgba" => Some(Color::rgba(
                channel(parts.first()?)?,
                channel(parts.get(1)?)?,
                channel(parts.get(2)?)?,
                alpha,
            )),
            "hsl" | "hsla" => {
                let hue = parts.first()?.trim_end_matches("deg").parse::<f32>().ok()?;
                let fraction = |text: &str| {
                    text.trim_end_matches('%')
                        .parse::<f32>()
                        .ok()
                        .map(|value| value / 100.0)
                };
                let (s, l) = (fraction(parts.get(1)?)?, fraction(parts.get(2)?)?);
                let [r, g, b] = hsl(hue, s, l);
                Some(Color::rgba(r, g, b, alpha))
            }
            _ => None,
        };
    }
    NAMED
        .binary_search_by(|(name, _)| (*name).cmp(lower.as_str()))
        .ok()
        .map(|at| Color::hex(NAMED[at].1))
}

fn hsl(hue: f32, saturation: f32, lightness: f32) -> [u8; 3] {
    let (s, l) = (saturation.clamp(0.0, 1.0), lightness.clamp(0.0, 1.0));
    let k = |n: f32| (n + hue.rem_euclid(360.0) / 30.0) % 12.0;
    let a = s * l.min(1.0 - l);
    let part = |n: f32| {
        let k = k(n);
        let value = l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0);
        (value * 255.0).round().clamp(0.0, 255.0) as u8
    };
    [part(0.0), part(8.0), part(4.0)]
}

/// An element's start, with its attributes.
#[derive(Debug)]
struct Tag<'a> {
    name: &'a str,
    attributes: Vec<(&'a str, String)>,
    /// Whether it ends where it starts, as `<path />` does.
    empty: bool,
}

impl Tag<'_> {
    fn get(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(said, _)| *said == name)
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug)]
enum Token<'a> {
    Open(Tag<'a>),
    Close(&'a str),
    Text(&'a str),
}

/// The tags and text in an XML file, leaving out comments and what's not part of what it shows.
struct Tokens<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Tokens<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, at: 0 }
    }
}

/// An element's name, without what it's in front of, as `svg:path` is a path.
fn local(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        loop {
            let text: &'a str = self.text;
            let rest = &text[self.at..];
            if rest.is_empty() {
                return None;
            }
            if !rest.starts_with('<') {
                let end = rest.find('<').unwrap_or(rest.len());
                self.at += end;
                return Some(Token::Text(&rest[..end]));
            }
            if let Some(inside) = rest.strip_prefix("<![CDATA[") {
                let end = inside.find("]]>").unwrap_or(inside.len());
                self.at += 9 + end + 3.min(inside.len() - end);
                return Some(Token::Text(&inside[..end]));
            }
            let skipped = [("<!--", "-->"), ("<?", "?>"), ("<!", ">")]
                .iter()
                .find(|(start, _)| rest.starts_with(start))
                .map(|(_, end)| rest.find(end).map_or(rest.len(), |at| at + end.len()));
            if let Some(length) = skipped {
                self.at += length;
                continue;
            }
            if let Some(inside) = rest.strip_prefix("</") {
                let end = inside.find('>').unwrap_or(inside.len());
                self.at += 2 + end + 1.min(inside.len() - end);
                return Some(Token::Close(local(inside[..end].trim())));
            }
            return Some(self.open_tag());
        }
    }
}

impl<'a> Tokens<'a> {
    fn open_tag(&mut self) -> Token<'a> {
        let text: &'a str = self.text;
        let bytes = text.as_bytes();
        let mut at = self.at + 1;
        let name_start = at;
        while at < bytes.len()
            && !bytes[at].is_ascii_whitespace()
            && bytes[at] != b'/'
            && bytes[at] != b'>'
        {
            at += 1;
        }
        let name = local(&text[name_start..at]);
        let mut attributes = Vec::new();
        let mut empty = false;
        loop {
            while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                at += 1;
            }
            if at >= bytes.len() {
                break;
            }
            if bytes[at] == b'>' {
                at += 1;
                break;
            }
            if bytes[at] == b'/' {
                at += 1;
                if bytes.get(at) == Some(&b'>') {
                    empty = true;
                    at += 1;
                    break;
                }
                continue;
            }
            let start = at;
            while at < bytes.len()
                && !bytes[at].is_ascii_whitespace()
                && !matches!(bytes[at], b'=' | b'>' | b'/')
            {
                at += 1;
            }
            let attribute = &text[start..at];
            while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                at += 1;
            }
            let mut value = "";
            if bytes.get(at) == Some(&b'=') {
                at += 1;
                while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                    at += 1;
                }
                match bytes.get(at) {
                    Some(&quote @ (b'"' | b'\'')) => {
                        let end = text[at + 1..]
                            .find(char::from(quote))
                            .map_or(text.len(), |end| at + 1 + end);
                        value = &text[at + 1..end];
                        at = (end + 1).min(text.len());
                    }
                    _ => {
                        let start = at;
                        while at < bytes.len()
                            && !bytes[at].is_ascii_whitespace()
                            && bytes[at] != b'>'
                        {
                            at += 1;
                        }
                        value = &text[start..at];
                    }
                }
            }
            if attribute.is_empty() {
                at += 1;
                continue;
            }
            attributes.push((attribute, unescape(value)));
        }
        self.at = at.min(text.len());
        Token::Open(Tag {
            name,
            attributes,
            empty,
        })
    }
}

/// Text with XML's escapes for characters turned back into those characters.
fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(';').filter(|end| *end <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let found = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| {
                    name.strip_prefix('#')
                        .and_then(|decimal| decimal.parse().ok())
                })
                .and_then(char::from_u32),
        };
        match found {
            Some(character) => {
                out.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Every color CSS has a name for, in order, as red, green and blue.
const NAMED: &[(&str, u32)] = &[
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aqua", 0x00ffff),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("black", 0x000000),
    ("blanchedalmond", 0xffebcd),
    ("blue", 0x0000ff),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xa9a9a9),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkslategrey", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xadff2f),
    ("grey", 0x808080),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightgrey", 0xd3d3d3),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("lime", 0x00ff00),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("navy", 0x000080),
    ("oldlace", 0xfdf5e6),
    ("olive", 0x808000),
    ("olivedrab", 0x6b8e23),
    ("orange", 0xffa500),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xff0000),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("silver", 0xc0c0c0),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("teal", 0x008080),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
    ("yellowgreen", 0x9acd32),
];

#[cfg(test)]
mod tests {
    use super::*;

    const DOT: &str = "<svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'><circle cx='10' cy='10' r='6' fill='#3b82f6'/></svg>";

    #[test]
    fn an_svg_is_known_by_its_start() {
        assert!(is_svg(DOT.as_bytes()));
        assert!(is_svg(b"\xEF\xBB\xBF<?xml version=\"1.0\"?>\n<svg></svg>"));
        assert!(!is_svg(b"\x89PNG\r\n"));
    }

    #[test]
    fn its_size_is_what_it_says() {
        assert_eq!(svg_size(DOT.as_bytes()), Some(((20, 20), true)));
        assert_eq!(
            svg_size(b"<svg viewBox='0 0 24 12'/>"),
            Some(((24, 12), false))
        );
        assert_eq!(
            svg_size(b"<svg width='48' viewBox='0 0 24 12'/>"),
            Some(((48, 24), true))
        );
        assert_eq!(svg_size(b"<svg/>"), Some(((300, 150), true)));
    }

    #[test]
    fn shapes_become_layers() {
        let document = read(DOT).unwrap();
        assert_eq!(
            document.layers,
            "#3b82f6ff\t\t1\tbutt\tM16 10a6 6 0 1 1 -12 0a6 6 0 1 1 12 0Z\t\t\t0\tmiter"
        );
        let drawn = draw_svg(DOT.as_bytes(), (20, 20)).unwrap();
        assert_eq!((drawn.width, drawn.height), (20, 20));
        let middle = (10 * 20 + 10) * 4;
        assert_eq!(&drawn.pixels[middle..middle + 4], &[0x3b, 0x82, 0xf6, 255]);
        assert_eq!(&drawn.pixels[0..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn what_a_group_says_goes_on_to_what_is_in_it() {
        let document = read(
            "<svg viewBox='0 0 10 10'><style>.a{stroke:red}</style><g fill='none' stroke-width='2' transform='rotate(90 5 5)'>\
             <rect class='a' width='4' height='4' style='stroke-linejoin:round'/></g><defs><path d='M0 0L1 1'/></defs></svg>",
        )
        .unwrap();
        assert_eq!(
            document.layers,
            "\t#ff0000ff\t2\tbutt\tM0 0h4v4h-4Z\ttranslate(5 5) rotate(90) translate(-5 -5)\t\t0\tround"
        );
    }

    #[test]
    fn colors_read_as_css_writes_them() {
        assert_eq!(
            parse_color("tomato", Color::BLACK),
            Some(Color::hex(0xff6347))
        );
        assert_eq!(
            parse_color("rgb(10, 20, 30)", Color::BLACK),
            Some(Color::hex(0x0a141e))
        );
        assert_eq!(
            parse_color("rgba(0 0 0 / 50%)", Color::BLACK),
            Some(Color::rgba(0, 0, 0, 128))
        );
        assert_eq!(
            parse_color("hsl(0, 100%, 50%)", Color::BLACK),
            Some(Color::hex(0xff0000))
        );
        assert_eq!(
            parse_color("currentColor", Color::WHITE),
            Some(Color::WHITE)
        );
    }
}
