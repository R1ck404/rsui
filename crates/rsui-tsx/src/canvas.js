// A page's `<canvas>`: `CanvasRenderingContext2D`, `Path2D`, `ImageData`, gradients and patterns, `OffscreenCanvas`, `ImageBitmap` and
// `DOMMatrix`, as a browser has them. It's only loaded the first time a page has a canvas or asks for one of these, so nothing else pays
// for it.
//
// Nothing's drawn here. Each call writes what it says into one list of numbers that every canvas shares, which goes to Rust in one go
// once what's running has finished (see `rsui::render::canvas`), or as soon as something has to be read back, as `getImageData()`
// does. What a getter says is kept here as it was set, so asking costs nothing.

'use strict';

const rsui = require('rsui');
const host = rsui.__host().canvas;

// What each number in the list says, as `rsui::render::canvas::ops` has them.
const SELECT = 1, SAVE = 2, RESTORE = 3, TRANSFORM = 4, RESET = 5, ALPHA = 6, COMPOSITE = 7, FILL_COLOR = 8, FILL_GRADIENT = 9,
    FILL_PATTERN = 10, STROKE_COLOR = 11, STROKE_GRADIENT = 12, STROKE_PATTERN = 13, LINE_WIDTH = 14, LINE_CAP = 15, LINE_JOIN = 16,
    MITER = 17, DASH = 18, DASH_OFFSET = 19, SHADOW_COLOR = 20, SHADOW_BLUR = 21, SHADOW_X = 22, SHADOW_Y = 23, SMOOTHING = 24,
    FONT = 25, ALIGN = 26, BASELINE = 27, DIRECTION = 28, LETTER_SPACING = 29, FILTER = 30, BEGIN = 31, MOVE = 32, LINE = 33,
    QUAD = 34, BEZIER = 35, ARC = 36, ARC_TO = 37, ELLIPSE = 38, RECT = 39, ROUND_RECT = 40, CLOSE = 41, FILL = 42, STROKE = 43,
    CLIP = 44, FILL_PATH = 45, STROKE_PATH = 46, CLIP_PATH = 47, FILL_RECT = 48, STROKE_RECT = 49, CLEAR_RECT = 50, FILL_TEXT = 51,
    STROKE_TEXT = 52, DRAW_IMAGE = 53, LINEAR = 54, RADIAL = 55, CONIC = 56, STOP = 57, PATTERN = 58, PATTERN_TRANSFORM = 59,
    PATH_DEFINE = 60, FREE = 61, IMAGE_SOURCE = 62, ADD_PATH = 65;

// The list, which is used again for each lot.
let list = new Float64Array(1 << 15);
let count = 0;
let strings = [];
/// Which canvas what's in the list now draws on.
let selected = 0;
let flushing = false;

/// Hands what's in the list to Rust.
function flush() {
    flushing = false;
    if (count === 0) {
        return;
    }
    const used = count;
    const said = strings;
    count = 0;
    strings = [];
    selected = 0;
    host.canvasRun(list, used, said);
}

/// Makes room for `need` more numbers, starting a new lot if there isn't any.
function room(need) {
    if (count + need > list.length) {
        flush();
        if (need > list.length) {
            list = new Float64Array(Math.max(need, list.length * 2));
        }
    }
    if (!flushing) {
        flushing = true;
        queueMicrotask(flush);
    }
}

/// Makes room, and says it's for this canvas.
function on(surface, need) {
    room(need + 2);
    if (selected !== surface) {
        list[count++] = SELECT;
        list[count++] = surface;
        selected = surface;
    }
}

function said(text) {
    strings.push(text);
    return strings.length - 1;
}

// Rust's numbers for what's made here, by kind, which are let go of when nothing here can reach them.
let nextId = 1;
const freed = typeof FinalizationRegistry === 'function'
    ? new FinalizationRegistry(([kind, id]) => {
        if (kind === 0) {
            host.canvasRelease(id);
        } else if (kind === 4) {
            host.canvasReleaseImage(id);
        } else {
            room(3);
            list[count++] = FREE;
            list[count++] = kind;
            list[count++] = id;
        }
    })
    : null;

function domException(name, message) {
    const error = new Error(message);
    error.name = name;
    error.code = { IndexSizeError: 1, NotSupportedError: 9, InvalidStateError: 11, SyntaxError: 12, SecurityError: 18 }[name] ?? 0;
    if (typeof globalThis.DOMException === 'function') {
        try {
            return new globalThis.DOMException(message, name);
        } catch {
            return error;
        }
    }
    return error;
}

const notSaid = new Set();
function notHere(what) {
    if (!notSaid.has(what)) {
        notSaid.add(what);
        console.warn(`rsui/canvas: ${what} isn't here yet, so it does nothing.`);
    }
}

// Colors, as CSS says them, packed as red, green, blue and alpha from the top byte down, with what a getter gives back for each.

const NAMED = 'aliceblue:f0f8ff,antiquewhite:faebd7,aqua:00ffff,aquamarine:7fffd4,azure:f0ffff,beige:f5f5dc,bisque:ffe4c4,black:000000,blanchedalmond:ffebcd,blue:0000ff,blueviolet:8a2be2,brown:a52a2a,burlywood:deb887,cadetblue:5f9ea0,chartreuse:7fff00,chocolate:d2691e,coral:ff7f50,cornflowerblue:6495ed,cornsilk:fff8dc,crimson:dc143c,cyan:00ffff,darkblue:00008b,darkcyan:008b8b,darkgoldenrod:b8860b,darkgray:a9a9a9,darkgreen:006400,darkgrey:a9a9a9,darkkhaki:bdb76b,darkmagenta:8b008b,darkolivegreen:556b2f,darkorange:ff8c00,darkorchid:9932cc,darkred:8b0000,darksalmon:e9967a,darkseagreen:8fbc8f,darkslateblue:483d8b,darkslategray:2f4f4f,darkslategrey:2f4f4f,darkturquoise:00ced1,darkviolet:9400d3,deeppink:ff1493,deepskyblue:00bfff,dimgray:696969,dimgrey:696969,dodgerblue:1e90ff,firebrick:b22222,floralwhite:fffaf0,forestgreen:228b22,fuchsia:ff00ff,gainsboro:dcdcdc,ghostwhite:f8f8ff,gold:ffd700,goldenrod:daa520,gray:808080,green:008000,greenyellow:adff2f,grey:808080,honeydew:f0fff0,hotpink:ff69b4,indianred:cd5c5c,indigo:4b0082,ivory:fffff0,khaki:f0e68c,lavender:e6e6fa,lavenderblush:fff0f5,lawngreen:7cfc00,lemonchiffon:fffacd,lightblue:add8e6,lightcoral:f08080,lightcyan:e0ffff,lightgoldenrodyellow:fafad2,lightgray:d3d3d3,lightgreen:90ee90,lightgrey:d3d3d3,lightpink:ffb6c1,lightsalmon:ffa07a,lightseagreen:20b2aa,lightskyblue:87cefa,lightslategray:778899,lightslategrey:778899,lightsteelblue:b0c4de,lightyellow:ffffe0,lime:00ff00,limegreen:32cd32,linen:faf0e6,magenta:ff00ff,maroon:800000,mediumaquamarine:66cdaa,mediumblue:0000cd,mediumorchid:ba55d3,mediumpurple:9370db,mediumseagreen:3cb371,mediumslateblue:7b68ee,mediumspringgreen:00fa9a,mediumturquoise:48d1cc,mediumvioletred:c71585,midnightblue:191970,mintcream:f5fffa,mistyrose:ffe4e1,moccasin:ffe4b5,navajowhite:ffdead,navy:000080,oldlace:fdf5e6,olive:808000,olivedrab:6b8e23,orange:ffa500,orangered:ff4500,orchid:da70d6,palegoldenrod:eee8aa,palegreen:98fb98,paleturquoise:afeeee,palevioletred:db7093,papayawhip:ffefd5,peachpuff:ffdab9,peru:cd853f,pink:ffc0cb,plum:dda0dd,powderblue:b0e0e6,purple:800080,rebeccapurple:663399,red:ff0000,rosybrown:bc8f8f,royalblue:4169e1,saddlebrown:8b4513,salmon:fa8072,sandybrown:f4a460,seagreen:2e8b57,seashell:fff5ee,sienna:a0522d,silver:c0c0c0,skyblue:87ceeb,slateblue:6a5acd,slategray:708090,slategrey:708090,snow:fffafa,springgreen:00ff7f,steelblue:4682b4,tan:d2b48c,teal:008080,thistle:d8bfd8,tomato:ff6347,turquoise:40e0d0,violet:ee82ee,wheat:f5deb3,white:ffffff,whitesmoke:f5f5f5,yellow:ffff00,yellowgreen:9acd32';
let named = null;
/// What each color written as text came to, as `[packed, text]`, or `null` for what isn't one.
const colors = new Map();

function color(value) {
    const text = String(value);
    let found = colors.get(text);
    if (found === undefined) {
        found = readColor(text);
        if (colors.size > 4096) {
            colors.clear();
        }
        colors.set(text, found);
    }
    return found;
}

function packColor(r, g, b, a) {
    const clamp = (v) => Math.max(0, Math.min(255, Math.round(v)));
    const alpha = Math.max(0, Math.min(1, a));
    const packed = ((clamp(r) << 24) | (clamp(g) << 16) | (clamp(b) << 8) | Math.round(alpha * 255)) >>> 0;
    const hex = (v) => clamp(v).toString(16).padStart(2, '0');
    // As a browser gives a canvas's color back: `#rrggbb` when it's opaque, `rgba()` otherwise.
    const text = alpha >= 1 ? `#${hex(r)}${hex(g)}${hex(b)}` : `rgba(${clamp(r)}, ${clamp(g)}, ${clamp(b)}, ${alphaText(alpha)})`;
    return [packed, text];
}

function alphaText(alpha) {
    if (alpha === 0) {
        return '0';
    }
    // The shortest that reads back as the same byte, as Chromium writes it.
    for (let digits = 1; digits < 6; digits += 1) {
        const text = String(Number(alpha.toFixed(digits)));
        if (Math.round(Number(text) * 255) === Math.round(alpha * 255)) {
            return text;
        }
    }
    return String(alpha);
}

function readColor(text) {
    const value = text.trim().toLowerCase();
    if (value.startsWith('#')) {
        const digits = value.slice(1);
        if (!/^[0-9a-f]+$/.test(digits)) {
            return null;
        }
        const expand = digits.length <= 4 ? digits.replace(/./g, (c) => c + c) : digits;
        if (expand.length !== 6 && expand.length !== 8) {
            return null;
        }
        const n = (i) => parseInt(expand.slice(i, i + 2), 16);
        return packColor(n(0), n(2), n(4), expand.length === 8 ? n(6) / 255 : 1);
    }
    if (value === 'transparent') {
        return packColor(0, 0, 0, 0);
    }
    if (value === 'currentcolor') {
        return packColor(0, 0, 0, 1);
    }
    const open = value.indexOf('(');
    if (open < 0) {
        if (named === null) {
            named = new Map(NAMED.split(',').map((pair) => pair.split(':')));
        }
        const hex = named.get(value);
        return hex === undefined ? null : readColor('#' + hex);
    }
    if (!value.endsWith(')')) {
        return null;
    }
    const name = value.slice(0, open).trim();
    const parts = value.slice(open + 1, -1).trim().split(/\s*[,/]\s*|\s+/).filter((part) => part !== '');
    const num = (part, scale = 1) => {
        if (part === 'none') {
            return 0;
        }
        if (part.endsWith('%')) {
            const n = Number(part.slice(0, -1));
            return Number.isFinite(n) ? (n / 100) * scale : NaN;
        }
        return Number(part);
    };
    const alpha = (part) => (part === undefined ? 1 : num(part, 1));
    const angle = (part) => {
        const n = parseFloat(part);
        if (part.endsWith('turn')) return n * 360;
        if (part.endsWith('grad')) return n * 0.9;
        if (part.endsWith('rad')) return (n * 180) / Math.PI;
        return n;
    };
    const ok = (...values) => values.every((v) => Number.isFinite(v));
    if ((name === 'rgb' || name === 'rgba') && (parts.length === 3 || parts.length === 4)) {
        const [r, g, b] = parts.slice(0, 3).map((part) => num(part, 255));
        const a = alpha(parts[3]);
        return ok(r, g, b, a) ? packColor(r, g, b, a) : null;
    }
    if ((name === 'hsl' || name === 'hsla') && (parts.length === 3 || parts.length === 4)) {
        const h = angle(parts[0]);
        const s = num(parts[1].endsWith('%') ? parts[1] : parts[1] + '%', 1);
        const l = num(parts[2].endsWith('%') ? parts[2] : parts[2] + '%', 1);
        const a = alpha(parts[3]);
        if (!ok(h, s, l, a)) return null;
        const [r, g, b] = hslToRgb(h, s, l);
        return packColor(r * 255, g * 255, b * 255, a);
    }
    if (name === 'hwb' && (parts.length === 3 || parts.length === 4)) {
        const h = angle(parts[0]);
        let w = num(parts[1], 1);
        let bl = num(parts[2], 1);
        const a = alpha(parts[3]);
        if (!ok(h, w, bl, a)) return null;
        if (w + bl >= 1) {
            const gray = w / (w + bl);
            return packColor(gray * 255, gray * 255, gray * 255, a);
        }
        const [r, g, b] = hslToRgb(h, 1, 0.5).map((c) => c * (1 - w - bl) + w);
        return packColor(r * 255, g * 255, b * 255, a);
    }
    if ((name === 'oklab' || name === 'oklch' || name === 'lab' || name === 'lch') && (parts.length === 3 || parts.length === 4)) {
        const ok_ = name.startsWith('ok');
        let l = num(parts[0], ok_ ? 1 : 100);
        let a;
        let b;
        if (name.endsWith('ch')) {
            const c = num(parts[1], ok_ ? 0.4 : 150);
            const h = (angle(parts[2]) * Math.PI) / 180;
            a = c * Math.cos(h);
            b = c * Math.sin(h);
        } else {
            a = num(parts[1], ok_ ? 0.4 : 125);
            b = num(parts[2], ok_ ? 0.4 : 125);
        }
        const al = alpha(parts[3]);
        if (!ok(l, a, b, al)) return null;
        const rgb = ok_ ? oklabToSrgb(l, a, b) : labToSrgb(l, a, b);
        return packColor(rgb[0] * 255, rgb[1] * 255, rgb[2] * 255, al);
    }
    if (name === 'color' && parts.length >= 4) {
        const space = parts[0];
        const values = parts.slice(1, 4).map((part) => num(part, 1));
        const a = alpha(parts[4]);
        if (!ok(...values, a)) return null;
        if (space === 'srgb') {
            return packColor(values[0] * 255, values[1] * 255, values[2] * 255, a);
        }
        if (space === 'srgb-linear') {
            const encode = (c) => (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055);
            return packColor(encode(values[0]) * 255, encode(values[1]) * 255, encode(values[2]) * 255, a);
        }
        return null;
    }
    return null;
}

function hslToRgb(h, s, l) {
    const hue = (((h % 360) + 360) % 360) / 360;
    const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
    const p = 2 * l - q;
    const part = (t) => {
        const u = t < 0 ? t + 1 : t > 1 ? t - 1 : t;
        if (u < 1 / 6) return p + (q - p) * 6 * u;
        if (u < 1 / 2) return q;
        if (u < 2 / 3) return p + (q - p) * (2 / 3 - u) * 6;
        return p;
    };
    return s === 0 ? [l, l, l] : [part(hue + 1 / 3), part(hue), part(hue - 1 / 3)];
}

function linearToSrgb(c) {
    const v = Math.max(0, Math.min(1, c));
    return v <= 0.0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - 0.055;
}

function oklabToSrgb(l, a, b) {
    const l_ = (l + 0.3963377774 * a + 0.2158037573 * b) ** 3;
    const m_ = (l - 0.1055613458 * a - 0.0638541728 * b) ** 3;
    const s_ = (l - 0.0894841775 * a - 1.291485548 * b) ** 3;
    return [
        linearToSrgb(4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_),
        linearToSrgb(-1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_),
        linearToSrgb(-0.0041960863 * l_ - 0.7034186147 * m_ + 1.707614701 * s_),
    ];
}

function labToSrgb(l, a, b) {
    // CIE Lab with a D50 white, to XYZ, adapted to D65, to linear sRGB.
    const fy = (l + 16) / 116;
    const fx = fy + a / 500;
    const fz = fy - b / 200;
    const k = 24389 / 27;
    const e = 216 / 24389;
    const x = (fx ** 3 > e ? fx ** 3 : (116 * fx - 16) / k) * 0.3457 / 0.3585;
    const y = l > k * e ? fy ** 3 : l / k;
    const z = (fz ** 3 > e ? fz ** 3 : (116 * fz - 16) / k) * (1 - 0.3457 - 0.3585) / 0.3585;
    const x65 = 0.9554734527 * x - 0.0230985368 * y + 0.0632593086 * z;
    const y65 = -0.0283697093 * x + 1.0099954580 * y + 0.0210413989 * z;
    const z65 = 0.0123140016 * x - 0.0205076964 * y + 1.3303659366 * z;
    return [
        linearToSrgb(3.2409699419 * x65 - 1.5373831776 * y65 - 0.4986107603 * z65),
        linearToSrgb(-0.9692436363 * x65 + 1.8759675015 * y65 + 0.0415550574 * z65),
        linearToSrgb(0.0556300797 * x65 - 0.2039769589 * y65 + 1.0569715142 * z65),
    ];
}

// `font`, as CSS's shorthand says it.

const FONT_SIZES = { 'xx-small': 9, 'x-small': 10, small: 13, medium: 16, large: 18, 'x-large': 24, 'xx-large': 32, 'xxx-large': 48 };
const fonts = new Map();

/// What a `font` string comes to, as `{ size, weight, italic, family, text, variant }`, or `null` if it isn't one.
function readFont(value, emSize) {
    const key = value + '\u0000' + emSize;
    let found = fonts.get(key);
    if (found === undefined) {
        found = parseFont(String(value), emSize);
        if (fonts.size > 1024) {
            fonts.clear();
        }
        fonts.set(key, found);
    }
    return found;
}

function parseFont(value, emSize) {
    const text = value.trim();
    // The size and what follows it: the first thing that's a length or a size's name.
    const match = /^(.*?)(?:^|\s)((?:[0-9]*\.?[0-9]+(?:e[+-]?[0-9]+)?)(?:px|pt|pc|in|cm|mm|q|em|rem|ex|ch|%|vw|vh)|xx-small|x-small|small|medium|large|x-large|xx-large|xxx-large|larger|smaller)(?:\s*\/\s*(?:[^\s]+))?\s+(.+)$/i.exec(text);
    if (match === null) {
        return null;
    }
    const before = match[1].trim().toLowerCase().split(/\s+/).filter((word) => word !== '');
    const sizeText = match[2].toLowerCase();
    const family = match[3].trim();
    if (family === '' || /^(inherit|initial|unset|revert)$/i.test(family)) {
        return null;
    }
    let italic = false;
    let weight = 400;
    let variant = 'normal';
    let stretch = 'normal';
    let seenWeight = false;
    for (const word of before) {
        if (word === 'normal') {
            continue;
        }
        if (word === 'italic' || word === 'oblique') {
            italic = true;
        } else if (word === 'small-caps') {
            variant = 'small-caps';
        } else if (word === 'bold' || word === 'bolder') {
            weight = 700;
            seenWeight = true;
        } else if (word === 'lighter') {
            weight = 100;
            seenWeight = true;
        } else if (/^[0-9]+$/.test(word) && !seenWeight && Number(word) >= 1 && Number(word) <= 1000) {
            weight = Number(word);
            seenWeight = true;
        } else if (/^(ultra-|extra-|semi-)?(condensed|expanded)$/.test(word)) {
            stretch = word;
        } else if (/^-?[0-9.]+deg$/.test(word)) {
            // An oblique's angle.
        } else {
            return null;
        }
    }
    let size;
    if (FONT_SIZES[sizeText] !== undefined) {
        size = FONT_SIZES[sizeText];
    } else if (sizeText === 'larger') {
        size = emSize * 1.2;
    } else if (sizeText === 'smaller') {
        size = emSize / 1.2;
    } else {
        const number = parseFloat(sizeText);
        const unit = sizeText.replace(/^[0-9.e+-]+/, '');
        const per = { px: 1, pt: 4 / 3, pc: 16, in: 96, cm: 96 / 2.54, mm: 96 / 25.4, q: 96 / 101.6, em: emSize, rem: 16, ex: emSize / 2, ch: emSize / 2, '%': emSize / 100, vw: (globalThis.innerWidth ?? 800) / 100, vh: (globalThis.innerHeight ?? 600) / 100 }[unit];
        size = number * per;
    }
    if (!Number.isFinite(size) || size < 0) {
        return null;
    }
    const sizePx = Math.round(size * 1000) / 1000;
    const weightText = weight === 400 ? '' : weight === 700 ? 'bold ' : `${weight} `;
    const out = `${italic ? 'italic ' : ''}${weightText}${variant === 'small-caps' ? 'small-caps ' : ''}${stretch !== 'normal' ? stretch + ' ' : ''}${sizePx}px ${family}`;
    // Whether the size is the same whatever the canvas's own font size and the window are.
    const fixed = FONT_SIZES[sizeText] !== undefined || /(px|pt|pc|in|cm|mm|q)$/.test(sizeText);
    return { size: sizePx, weight, italic, family, text: out, variant, stretch, fixed };
}

// What `save()` keeps, and what the getters say.
class DrawingState {
    constructor() {
        this.fillStyle = '#000000';
        this.fillKind = FILL_COLOR;
        this.fillValue = 0x000000ff;
        this.strokeStyle = '#000000';
        this.strokeKind = STROKE_COLOR;
        this.strokeValue = 0x000000ff;
        this.lineWidth = 1;
        this.lineCap = 'butt';
        this.lineJoin = 'miter';
        this.miterLimit = 10;
        this.lineDash = [];
        this.lineDashOffset = 0;
        this.globalAlpha = 1;
        this.globalCompositeOperation = 'source-over';
        this.shadowBlur = 0;
        this.shadowColor = 'rgba(0, 0, 0, 0)';
        this.shadowOffsetX = 0;
        this.shadowOffsetY = 0;
        this.font = '10px sans-serif';
        this.fontParts = null;
        /// The text `font` was last set to, as it was given.
        this.fontGiven = null;
        this.textAlign = 'start';
        this.textBaseline = 'alphabetic';
        this.direction = 'inherit';
        this.letterSpacing = '0px';
        this.letterSpacingPx = 0;
        this.wordSpacing = '0px';
        this.fontKerning = 'auto';
        this.fontStretch = 'normal';
        this.fontVariantCaps = 'normal';
        this.textRendering = 'auto';
        this.imageSmoothingEnabled = true;
        this.imageSmoothingQuality = 'low';
        this.filter = 'none';
        this.a = 1;
        this.b = 0;
        this.c = 0;
        this.d = 1;
        this.e = 0;
        this.f = 0;
    }
}

const COMPOSITES = ['source-over', 'source-in', 'source-out', 'source-atop', 'destination-over', 'destination-in', 'destination-out', 'destination-atop', 'lighter', 'copy', 'xor', 'multiply', 'screen', 'overlay', 'darken', 'lighten', 'color-dodge', 'color-burn', 'hard-light', 'soft-light', 'difference', 'exclusion', 'hue', 'saturation', 'color', 'luminosity'];
const COMPOSITE_INDEX = new Map(COMPOSITES.map((name, index) => [name, index]));
const CAPS = { butt: 0, round: 1, square: 2 };
const JOINS = { miter: 0, round: 1, bevel: 2 };
const ALIGNS = { start: 0, end: 1, left: 2, right: 3, center: 4 };
const BASELINES = { alphabetic: 0, top: 1, hanging: 2, middle: 3, ideographic: 4, bottom: 5 };

const finite = (...values) => values.every((v) => Number.isFinite(v));

/// What's behind a canvas: its number in Rust, and its size.
class Surface {
    constructor(width, height, opaque = false) {
        this.width = width;
        this.height = height;
        this.id = host.canvasCreate(width, height, opaque);
        this.opaque = opaque;
        freed?.register(this, [0, this.id]);
    }

    resize(width, height) {
        flush();
        this.width = width;
        this.height = height;
        host.canvasResize(this.id, width, height);
    }
}

// What a page's code draws on a canvas with.
class CanvasRenderingContext2D {
    constructor(canvas, surface, attributes) {
        Object.defineProperty(this, '_canvas', { value: canvas });
        Object.defineProperty(this, '_surface', { value: surface, writable: true });
        Object.defineProperty(this, '_id', { value: surface.id, writable: true });
        Object.defineProperty(this, '_s', { value: new DrawingState(), writable: true });
        Object.defineProperty(this, '_saved', { value: [], writable: true });
        Object.defineProperty(this, '_attributes', { value: attributes });
    }

    get canvas() {
        return this._canvas;
    }

    getContextAttributes() {
        return { ...this._attributes };
    }

    /// Back to how it was when made, as a canvas that's been given a size is.
    _forget() {
        this._s = new DrawingState();
        this._saved = [];
    }

    _op(code, need) {
        if (selected !== this._id || count + need + 1 > list.length) {
            on(this._id, need + 1);
        }
        list[count++] = code;
    }

    // The state.

    save() {
        // What was kept goes on the list as it is, and drawing goes on with a new state that reads what it doesn't set from
        // it. Nothing in a state is changed in place (`lineDash` is replaced, never added to), so that's all a copy needs.
        // Saves many deep are made one again, so that reading never goes far.
        const s = this._s;
        this._saved.push(s);
        if (this._saved.length % 16 === 0) {
            const flat = {};
            for (const key in s) {
                flat[key] = s[key];
            }
            this._s = flat;
        } else {
            this._s = Object.create(s);
        }
        this._op(SAVE, 0);
    }

    restore() {
        const back = this._saved.pop();
        if (back !== undefined) {
            this._s = back;
            this._op(RESTORE, 0);
        }
    }

    reset() {
        this._forget();
        this._op(RESET, 0);
    }

    isContextLost() {
        return false;
    }

    get globalAlpha() {
        return this._s.globalAlpha;
    }
    set globalAlpha(value) {
        const v = Number(value);
        if (!Number.isFinite(v) || v < 0 || v > 1 || v === this._s.globalAlpha) {
            return;
        }
        this._s.globalAlpha = v;
        this._op(ALPHA, 1);
        list[count++] = v;
    }

    get globalCompositeOperation() {
        return this._s.globalCompositeOperation;
    }
    set globalCompositeOperation(value) {
        const index = COMPOSITE_INDEX.get(String(value));
        if (index === undefined || this._s.globalCompositeOperation === String(value)) {
            return;
        }
        this._s.globalCompositeOperation = String(value);
        this._op(COMPOSITE, 1);
        list[count++] = index;
    }

    get fillStyle() {
        return this._s.fillStyle;
    }
    set fillStyle(value) {
        this._style(value, true);
    }

    get strokeStyle() {
        return this._s.strokeStyle;
    }
    set strokeStyle(value) {
        this._style(value, false);
    }

    _style(value, fill) {
        const s = this._s;
        // The same color again, as charts set it for each thing they draw.
        if (typeof value === 'string' && value === (fill ? s.fillStyle : s.strokeStyle)) {
            return;
        }
        let kind;
        let number;
        let shown;
        if (value instanceof CanvasGradient) {
            kind = fill ? FILL_GRADIENT : STROKE_GRADIENT;
            number = value._id;
            shown = value;
        } else if (value instanceof CanvasPattern) {
            kind = fill ? FILL_PATTERN : STROKE_PATTERN;
            number = value._id;
            shown = value;
        } else {
            const found = color(value);
            if (found === null) {
                return;
            }
            kind = fill ? FILL_COLOR : STROKE_COLOR;
            number = found[0];
            shown = found[1];
        }
        if (fill) {
            s.fillStyle = shown;
            if (s.fillKind === kind && s.fillValue === number) {
                return;
            }
            s.fillKind = kind;
            s.fillValue = number;
        } else {
            s.strokeStyle = shown;
            if (s.strokeKind === kind && s.strokeValue === number) {
                return;
            }
            s.strokeKind = kind;
            s.strokeValue = number;
        }
        this._op(kind, 1);
        list[count++] = number;
    }

    get lineWidth() {
        return this._s.lineWidth;
    }
    set lineWidth(value) {
        if (value === this._s.lineWidth) {
            return;
        }
        const v = Number(value);
        if (!Number.isFinite(v) || v <= 0 || v === this._s.lineWidth) {
            return;
        }
        this._s.lineWidth = v;
        this._op(LINE_WIDTH, 1);
        list[count++] = v;
    }

    get lineCap() {
        return this._s.lineCap;
    }
    set lineCap(value) {
        const v = CAPS[value];
        if (v === undefined || typeof value !== 'string' || value === this._s.lineCap) {
            return;
        }
        this._s.lineCap = value;
        this._op(LINE_CAP, 1);
        list[count++] = v;
    }

    get lineJoin() {
        return this._s.lineJoin;
    }
    set lineJoin(value) {
        const v = JOINS[value];
        if (v === undefined || typeof value !== 'string' || value === this._s.lineJoin) {
            return;
        }
        this._s.lineJoin = value;
        this._op(LINE_JOIN, 1);
        list[count++] = v;
    }

    get miterLimit() {
        return this._s.miterLimit;
    }
    set miterLimit(value) {
        const v = Number(value);
        if (!Number.isFinite(v) || v <= 0 || v === this._s.miterLimit) {
            return;
        }
        this._s.miterLimit = v;
        this._op(MITER, 1);
        list[count++] = v;
    }

    getLineDash() {
        return this._s.lineDash.slice();
    }
    setLineDash(segments) {
        const values = Array.from(segments ?? [], Number);
        if (values.some((v) => !Number.isFinite(v) || v < 0)) {
            return;
        }
        const dash = values.length % 2 === 1 ? values.concat(values) : values;
        this._s.lineDash = dash;
        this._op(DASH, 1 + dash.length);
        list[count++] = dash.length;
        for (const v of dash) {
            list[count++] = v;
        }
    }

    get lineDashOffset() {
        return this._s.lineDashOffset;
    }
    set lineDashOffset(value) {
        const v = Number(value);
        if (!Number.isFinite(v) || v === this._s.lineDashOffset) {
            return;
        }
        this._s.lineDashOffset = v;
        this._op(DASH_OFFSET, 1);
        list[count++] = v;
    }

    get shadowBlur() {
        return this._s.shadowBlur;
    }
    set shadowBlur(value) {
        this._number('shadowBlur', SHADOW_BLUR, value, 0);
    }
    get shadowOffsetX() {
        return this._s.shadowOffsetX;
    }
    set shadowOffsetX(value) {
        this._number('shadowOffsetX', SHADOW_X, value, -Infinity);
    }
    get shadowOffsetY() {
        return this._s.shadowOffsetY;
    }
    set shadowOffsetY(value) {
        this._number('shadowOffsetY', SHADOW_Y, value, -Infinity);
    }

    _number(name, code, value, least) {
        const v = Number(value);
        if (!Number.isFinite(v) || v < least || v === this._s[name]) {
            return;
        }
        this._s[name] = v;
        this._op(code, 1);
        list[count++] = v;
    }

    get shadowColor() {
        return this._s.shadowColor;
    }
    set shadowColor(value) {
        const found = color(value);
        if (found === null || found[1] === this._s.shadowColor) {
            return;
        }
        this._s.shadowColor = found[1];
        this._op(SHADOW_COLOR, 1);
        list[count++] = found[0];
    }

    get imageSmoothingEnabled() {
        return this._s.imageSmoothingEnabled;
    }
    set imageSmoothingEnabled(value) {
        const v = Boolean(value);
        if (v === this._s.imageSmoothingEnabled) {
            return;
        }
        this._s.imageSmoothingEnabled = v;
        this._op(SMOOTHING, 1);
        list[count++] = v ? 1 : 0;
    }
    get imageSmoothingQuality() {
        return this._s.imageSmoothingQuality;
    }
    set imageSmoothingQuality(value) {
        if (value === 'low' || value === 'medium' || value === 'high') {
            this._s.imageSmoothingQuality = value;
        }
    }

    get filter() {
        return this._s.filter;
    }
    set filter(value) {
        const text = String(value);
        if (text === this._s.filter) {
            return;
        }
        this._s.filter = text;
        const index = text.trim() === 'none' || text.trim() === '' ? -1 : said(text);
        this._op(FILTER, 1);
        list[count++] = index;
    }

    // Text.

    get font() {
        return this._s.font;
    }
    set font(value) {
        const s = this._s;
        // The same text as last time, with a size that can't have changed since, as charts set it for each label.
        if (value === s.fontGiven && s.fontParts !== null && s.fontParts.fixed) {
            return;
        }
        const parts = readFont(String(value), this._emSize());
        if (parts === null) {
            return;
        }
        s.fontGiven = typeof value === 'string' ? value : null;
        if (parts === s.fontParts) {
            return;
        }
        this._s.font = parts.text;
        this._s.fontParts = parts;
        this._op(FONT, 4);
        list[count++] = parts.size;
        list[count++] = parts.weight;
        list[count++] = parts.italic ? 1 : 0;
        list[count++] = said(parts.family);
    }

    _emSize() {
        const canvas = this._canvas;
        const size = canvas?.__live?.look?.fontSize;
        return typeof size === 'number' ? size : canvas instanceof OffscreenCanvas || canvas?.__live === undefined ? 10 : 16;
    }

    _fontParts() {
        return this._s.fontParts ?? (this._s.fontParts = readFont('10px sans-serif', 10));
    }

    get textAlign() {
        return this._s.textAlign;
    }
    set textAlign(value) {
        const v = ALIGNS[value];
        if (v === undefined || typeof value !== 'string' || value === this._s.textAlign) {
            return;
        }
        this._s.textAlign = value;
        this._op(ALIGN, 1);
        list[count++] = v;
    }

    get textBaseline() {
        return this._s.textBaseline;
    }
    set textBaseline(value) {
        const v = BASELINES[value];
        if (v === undefined || typeof value !== 'string' || value === this._s.textBaseline) {
            return;
        }
        this._s.textBaseline = value;
        this._op(BASELINE, 1);
        list[count++] = v;
    }

    get direction() {
        return this._s.direction;
    }
    set direction(value) {
        if (value !== 'ltr' && value !== 'rtl' && value !== 'inherit') {
            return;
        }
        this._s.direction = value;
        this._op(DIRECTION, 1);
        list[count++] = this._rtl() ? 1 : 0;
    }

    _rtl() {
        if (this._s.direction === 'rtl') {
            return true;
        }
        if (this._s.direction === 'ltr') {
            return false;
        }
        return this._canvas?.__live?.look?.textDirection === 'rtl';
    }

    get letterSpacing() {
        return this._s.letterSpacing;
    }
    set letterSpacing(value) {
        const text = String(value).trim();
        const match = /^(-?[0-9.]+)(px|em|rem|pt)?$/.exec(text);
        if (match === null) {
            return;
        }
        const n = Number(match[1]);
        const unit = match[2] ?? (n === 0 ? 'px' : null);
        if (unit === null || !Number.isFinite(n)) {
            return;
        }
        const px = n * { px: 1, em: this._fontParts().size, rem: 16, pt: 4 / 3 }[unit];
        this._s.letterSpacing = `${n}${unit}`;
        this._s.letterSpacingPx = px;
        this._op(LETTER_SPACING, 1);
        list[count++] = px;
    }

    get wordSpacing() {
        return this._s.wordSpacing;
    }
    set wordSpacing(value) {
        if (/^-?[0-9.]+(px|em|rem)$|^0$/.test(String(value).trim())) {
            this._s.wordSpacing = String(value).trim();
        }
    }
    get fontKerning() {
        return this._s.fontKerning;
    }
    set fontKerning(value) {
        if (value === 'auto' || value === 'normal' || value === 'none') this._s.fontKerning = value;
    }
    get fontStretch() {
        return this._s.fontStretch;
    }
    set fontStretch(value) {
        if (/^(ultra-|extra-|semi-)?(condensed|expanded)$|^normal$/.test(String(value))) this._s.fontStretch = String(value);
    }
    get fontVariantCaps() {
        return this._s.fontVariantCaps;
    }
    set fontVariantCaps(value) {
        if (/^(normal|small-caps|all-small-caps|petite-caps|all-petite-caps|unicase|titling-caps)$/.test(String(value))) this._s.fontVariantCaps = String(value);
    }
    get textRendering() {
        return this._s.textRendering;
    }
    set textRendering(value) {
        if (/^(auto|optimizeSpeed|optimizeLegibility|geometricPrecision)$/.test(String(value))) this._s.textRendering = String(value);
    }

    fillText(text, x, y, maxWidth) {
        this._text(FILL_TEXT, text, x, y, maxWidth);
    }

    strokeText(text, x, y, maxWidth) {
        this._text(STROKE_TEXT, text, x, y, maxWidth);
    }

    _text(code, text, x, y, maxWidth) {
        const nx = +x;
        const ny = +y;
        const most = maxWidth === undefined ? Infinity : +maxWidth;
        if (nx - nx + (ny - ny) !== 0 || most !== most) {
            return;
        }
        this._op(code, 4);
        list[count++] = said(String(text));
        list[count++] = nx;
        list[count++] = ny;
        list[count++] = most;
    }

    measureText(text) {
        const parts = this._fontParts();
        const s = this._s;
        const found = host.canvasMeasure(parts.size, parts.weight, parts.italic ? 1 : 0, parts.family, s.letterSpacingPx, this._rtl(), ALIGNS[s.textAlign], BASELINES[s.textBaseline], String(text));
        return new TextMetrics(found);
    }

    // Transforms.

    _transformed() {
        const s = this._s;
        this._op(TRANSFORM, 6);
        list[count++] = s.a;
        list[count++] = s.b;
        list[count++] = s.c;
        list[count++] = s.d;
        list[count++] = s.e;
        list[count++] = s.f;
    }

    _multiply(a, b, c, d, e, f) {
        const s = this._s;
        const sa = s.a;
        const sb = s.b;
        const sc = s.c;
        const sd = s.d;
        const se = s.e;
        const sf = s.f;
        s.a = sa * a + sc * b;
        s.b = sb * a + sd * b;
        s.c = sa * c + sc * d;
        s.d = sb * c + sd * d;
        s.e = sa * e + sc * f + se;
        s.f = sb * e + sd * f + sf;
        this._transformed();
    }

    translate(x, y) {
        const nx = +x;
        const ny = +y;
        if (nx - nx + (ny - ny) === 0) {
            const s = this._s;
            s.e += s.a * nx + s.c * ny;
            s.f += s.b * nx + s.d * ny;
            this._transformed();
        }
    }

    scale(x, y) {
        const nx = +x;
        const ny = +y;
        if (nx - nx + (ny - ny) === 0) {
            this._multiply(nx, 0, 0, ny, 0, 0);
        }
    }

    rotate(angle) {
        const a = +angle;
        if (a - a === 0) {
            const cos = Math.cos(a);
            const sin = Math.sin(a);
            this._multiply(cos, sin, -sin, cos, 0, 0);
        }
    }

    transform(a, b, c, d, e, f) {
        const values = [a, b, c, d, e, f].map(Number);
        if (finite(...values)) {
            this._multiply(...values);
        }
    }

    setTransform(a, b, c, d, e, f) {
        let values;
        if (typeof a === 'object' && a !== null) {
            const m = a;
            values = [m.a ?? m.m11 ?? 1, m.b ?? m.m12 ?? 0, m.c ?? m.m21 ?? 0, m.d ?? m.m22 ?? 1, m.e ?? m.m41 ?? 0, m.f ?? m.m42 ?? 0].map(Number);
        } else if (a === undefined) {
            values = [1, 0, 0, 1, 0, 0];
        } else {
            values = [a, b, c, d, e, f].map(Number);
        }
        if (!finite(...values)) {
            return;
        }
        const s = this._s;
        [s.a, s.b, s.c, s.d, s.e, s.f] = values;
        this._transformed();
    }

    resetTransform() {
        this.setTransform(1, 0, 0, 1, 0, 0);
    }

    getTransform() {
        const s = this._s;
        return new DOMMatrix([s.a, s.b, s.c, s.d, s.e, s.f]);
    }

    // Paths.

    beginPath() {
        if (selected !== this._id || count + 1 > list.length) {
            on(this._id, 1);
        }
        list[count++] = BEGIN;
    }

    closePath() {
        this._op(CLOSE, 0);
    }

    moveTo(x, y) {
        const nx = +x;
        const ny = +y;
        // Anything that isn't a finite number makes this not a number.
        if (nx - nx + (ny - ny) !== 0) {
            return;
        }
        if (selected !== this._id || count + 3 > list.length) {
            on(this._id, 3);
        }
        list[count++] = MOVE;
        list[count++] = nx;
        list[count++] = ny;
    }

    lineTo(x, y) {
        const nx = +x;
        const ny = +y;
        if (nx - nx + (ny - ny) !== 0) {
            return;
        }
        if (selected !== this._id || count + 3 > list.length) {
            on(this._id, 3);
        }
        list[count++] = LINE;
        list[count++] = nx;
        list[count++] = ny;
    }

    quadraticCurveTo(cx, cy, x, y) {
        this._numbers(QUAD, [cx, cy, x, y]);
    }

    bezierCurveTo(c1x, c1y, c2x, c2y, x, y) {
        this._numbers(BEZIER, [c1x, c1y, c2x, c2y, x, y]);
    }

    _numbers(code, values) {
        for (let i = 0; i < values.length; i += 1) {
            const v = +values[i];
            if (v - v !== 0) {
                return;
            }
            values[i] = v;
        }
        this._op(code, values.length);
        for (let i = 0; i < values.length; i += 1) {
            list[count++] = values[i];
        }
    }

    arc(x, y, radius, startAngle, endAngle, anticlockwise = false) {
        const r = +radius;
        if (r < 0) {
            throw domException('IndexSizeError', `Failed to execute 'arc' on 'CanvasRenderingContext2D': The radius provided (${r}) is negative.`);
        }
        const nx = +x;
        const ny = +y;
        const a0 = +startAngle;
        const a1 = +endAngle;
        if (nx - nx + (ny - ny) + (r - r) + (a0 - a0) + (a1 - a1) !== 0) {
            return;
        }
        if (selected !== this._id || count + 7 > list.length) {
            on(this._id, 7);
        }
        list[count++] = ARC;
        list[count++] = nx;
        list[count++] = ny;
        list[count++] = r;
        list[count++] = a0;
        list[count++] = a1;
        list[count++] = anticlockwise ? 1 : 0;
    }

    arcTo(x1, y1, x2, y2, radius) {
        const r = Number(radius);
        if (r < 0) {
            throw domException('IndexSizeError', `Failed to execute 'arcTo' on 'CanvasRenderingContext2D': The radius provided (${r}) is negative.`);
        }
        this._numbers(ARC_TO, [x1, y1, x2, y2, r]);
    }

    ellipse(x, y, radiusX, radiusY, rotation, startAngle, endAngle, anticlockwise = false) {
        const values = [x, y, radiusX, radiusY, rotation, startAngle, endAngle].map(Number);
        if (values[2] < 0 || values[3] < 0) {
            throw domException('IndexSizeError', `Failed to execute 'ellipse' on 'CanvasRenderingContext2D': The ${values[2] < 0 ? 'major' : 'minor'}-axis radius provided is negative.`);
        }
        if (!finite(...values)) {
            return;
        }
        this._op(ELLIPSE, 8);
        for (const v of values) {
            list[count++] = v;
        }
        list[count++] = anticlockwise ? 1 : 0;
    }

    rect(x, y, w, h) {
        this._numbers(RECT, [x, y, w, h]);
    }

    roundRect(x, y, w, h, radii = 0) {
        const corners = roundRectRadii(radii, 'roundRect', 'CanvasRenderingContext2D');
        const values = [x, y, w, h].map(Number);
        if (!finite(...values) || corners === null) {
            return;
        }
        this._op(ROUND_RECT, 12);
        for (const v of values) {
            list[count++] = v;
        }
        for (const v of corners) {
            list[count++] = v;
        }
    }

    _rule(rule) {
        return rule === 'evenodd' ? 1 : 0;
    }

    fill(pathOrRule, rule) {
        if (pathOrRule === undefined) {
            if (selected !== this._id || count + 2 > list.length) {
                on(this._id, 2);
            }
            list[count++] = FILL;
            list[count++] = 0;
            return;
        }
        if (pathOrRule instanceof Path2D) {
            const id = pathOrRule._sync();
            this._op(FILL_PATH, 2);
            list[count++] = id;
            list[count++] = this._rule(rule);
        } else {
            this._op(FILL, 1);
            list[count++] = this._rule(pathOrRule);
        }
    }

    stroke(path) {
        if (path === undefined) {
            if (selected !== this._id || count + 1 > list.length) {
                on(this._id, 1);
            }
            list[count++] = STROKE;
            return;
        }
        if (path instanceof Path2D) {
            const id = path._sync();
            this._op(STROKE_PATH, 1);
            list[count++] = id;
        } else {
            this._op(STROKE, 0);
        }
    }

    clip(pathOrRule, rule) {
        if (pathOrRule instanceof Path2D) {
            const id = pathOrRule._sync();
            this._op(CLIP_PATH, 2);
            list[count++] = id;
            list[count++] = this._rule(rule);
        } else {
            this._op(CLIP, 1);
            list[count++] = this._rule(pathOrRule);
        }
    }

    isPointInPath(pathOrX, xOrY, yOrRule, rule) {
        let path = 0;
        let x;
        let y;
        let even;
        if (pathOrX instanceof Path2D) {
            path = pathOrX._sync();
            [x, y, even] = [Number(xOrY), Number(yOrRule), rule === 'evenodd'];
        } else {
            [x, y, even] = [Number(pathOrX), Number(xOrY), yOrRule === 'evenodd'];
        }
        if (!finite(x, y)) {
            return false;
        }
        flush();
        return host.canvasPointIn(this._surface.id, path, x, y, even, false);
    }

    isPointInStroke(pathOrX, xOrY, y) {
        let path = 0;
        let px;
        let py;
        if (pathOrX instanceof Path2D) {
            path = pathOrX._sync();
            [px, py] = [Number(xOrY), Number(y)];
        } else {
            [px, py] = [Number(pathOrX), Number(xOrY)];
        }
        if (!finite(px, py)) {
            return false;
        }
        flush();
        return host.canvasPointIn(this._surface.id, path, px, py, false, true);
    }

    drawFocusIfNeeded() {}

    scrollPathIntoView() {}

    // Rectangles.

    fillRect(x, y, w, h) {
        this._rect(FILL_RECT, x, y, w, h);
    }

    strokeRect(x, y, w, h) {
        this._rect(STROKE_RECT, x, y, w, h);
    }

    clearRect(x, y, w, h) {
        this._rect(CLEAR_RECT, x, y, w, h);
    }

    _rect(code, x, y, w, h) {
        const nx = +x;
        const ny = +y;
        const nw = +w;
        const nh = +h;
        if (nx - nx + (ny - ny) + (nw - nw) + (nh - nh) !== 0 || (code !== STROKE_RECT && (nw === 0 || nh === 0))) {
            return;
        }
        this._op(code, 4);
        list[count++] = nx;
        list[count++] = ny;
        list[count++] = nw;
        list[count++] = nh;
    }

    // Pictures.

    drawImage(image, ...rest) {
        const source = imageSource(image);
        if (source === null) {
            return;
        }
        const [kind, id, width, height] = source;
        let values;
        if (rest.length === 2) {
            values = [0, 0, width, height, rest[0], rest[1], width, height];
        } else if (rest.length === 4) {
            values = [0, 0, width, height, rest[0], rest[1], rest[2], rest[3]];
        } else if (rest.length === 8) {
            values = rest;
        } else {
            throw new TypeError(`Failed to execute 'drawImage' on 'CanvasRenderingContext2D': Valid arities are: [3, 5, 9], but ${rest.length + 1} arguments provided.`);
        }
        if (width === 0 || height === 0) {
            return;
        }
        this._op(DRAW_IMAGE, 10);
        list[count++] = kind;
        list[count++] = id;
        for (const v of values) {
            list[count++] = Number(v);
        }
    }

    createImageData(width, height, settings) {
        if (width instanceof ImageData) {
            return new ImageData(width.width, width.height);
        }
        const w = Math.abs(Math.trunc(Number(width)));
        const h = Math.abs(Math.trunc(Number(height)));
        if (w === 0 || h === 0 || !finite(w, h)) {
            throw domException('IndexSizeError', `Failed to execute 'createImageData' on 'CanvasRenderingContext2D': The source ${w === 0 ? 'width' : 'height'} is 0.`);
        }
        return new ImageData(w, h, settings);
    }

    getImageData(sx, sy, sw, sh, settings) {
        let x = Math.trunc(Number(sx));
        let y = Math.trunc(Number(sy));
        let w = Math.trunc(Number(sw));
        let h = Math.trunc(Number(sh));
        if (!finite(x, y, w, h)) {
            throw new TypeError("Failed to execute 'getImageData' on 'CanvasRenderingContext2D': The provided double value is non-finite.");
        }
        if (w === 0 || h === 0) {
            throw domException('IndexSizeError', `Failed to execute 'getImageData' on 'CanvasRenderingContext2D': The source ${w === 0 ? 'width' : 'height'} is 0.`);
        }
        if (w < 0) {
            x += w;
            w = -w;
        }
        if (h < 0) {
            y += h;
            h = -h;
        }
        flush();
        const data = new Uint8ClampedArray(w * h * 4);
        host.canvasRead(this._surface.id, x, y, w, h, data);
        return new ImageData(data, w, h, settings);
    }

    putImageData(imagedata, dx, dy, dirtyX = 0, dirtyY = 0, dirtyWidth, dirtyHeight) {
        if (!(imagedata instanceof ImageData)) {
            throw new TypeError("Failed to execute 'putImageData' on 'CanvasRenderingContext2D': parameter 1 is not of type 'ImageData'.");
        }
        const values = [dx, dy, dirtyX, dirtyY, dirtyWidth ?? imagedata.width, dirtyHeight ?? imagedata.height].map(Number);
        if (!finite(...values)) {
            return;
        }
        flush();
        host.canvasWrite(this._surface.id, imagedata.data, imagedata.width, imagedata.height, ...values.map(Math.trunc));
    }

    createLinearGradient(x0, y0, x1, y1) {
        const values = [x0, y0, x1, y1].map(Number);
        if (!finite(...values)) {
            throw new TypeError("Failed to execute 'createLinearGradient' on 'CanvasRenderingContext2D': The provided double value is non-finite.");
        }
        return new CanvasGradient(LINEAR, values);
    }

    createRadialGradient(x0, y0, r0, x1, y1, r1) {
        const values = [x0, y0, r0, x1, y1, r1].map(Number);
        if (!finite(...values)) {
            throw new TypeError("Failed to execute 'createRadialGradient' on 'CanvasRenderingContext2D': The provided double value is non-finite.");
        }
        if (values[2] < 0 || values[5] < 0) {
            throw domException('IndexSizeError', `Failed to execute 'createRadialGradient' on 'CanvasRenderingContext2D': The ${values[2] < 0 ? 'r0' : 'r1'} provided is less than 0.`);
        }
        return new CanvasGradient(RADIAL, values);
    }

    createConicGradient(startAngle, x, y) {
        const values = [startAngle, x, y].map(Number);
        if (!finite(...values)) {
            throw new TypeError("Failed to execute 'createConicGradient' on 'CanvasRenderingContext2D': The provided double value is non-finite.");
        }
        return new CanvasGradient(CONIC, values);
    }

    createPattern(image, repetition) {
        const source = imageSource(image);
        if (source === null) {
            return null;
        }
        const repeat = { repeat: 0, '': 0, null: 0, 'repeat-x': 1, 'repeat-y': 2, 'no-repeat': 3 }[repetition === null || repetition === undefined ? '' : String(repetition)];
        if (repeat === undefined) {
            throw domException('SyntaxError', `Failed to execute 'createPattern' on 'CanvasRenderingContext2D': The provided type ('${repetition}') is not one of 'repeat', 'no-repeat', 'repeat-x', or 'repeat-y'.`);
        }
        if (source[2] === 0 || source[3] === 0) {
            return null;
        }
        return new CanvasPattern(source, repeat, this._surface.id);
    }
}

/// `roundRect()`'s radii, as each corner's across and down, from the top left round, or `null` for what draws nothing.
function roundRectRadii(radii, method, kind) {
    const all = Array.isArray(radii) || (radii !== null && typeof radii === 'object' && typeof radii[Symbol.iterator] === 'function' && typeof radii !== 'string') ? Array.from(radii) : [radii];
    if (all.length < 1 || all.length > 4) {
        throw new RangeError(`Failed to execute '${method}' on '${kind}': ${all.length} radii provided. Between one and four radii are necessary.`);
    }
    const pairs = [];
    for (const one of all) {
        let x;
        let y;
        if (typeof one === 'object' && one !== null) {
            x = Number(one.x ?? 0);
            y = Number(one.y ?? 0);
        } else {
            x = Number(one);
            y = x;
        }
        if (!finite(x, y)) {
            return null;
        }
        if (x < 0 || y < 0) {
            throw new RangeError(`Failed to execute '${method}' on '${kind}': Radius value ${x < 0 ? x : y} is negative.`);
        }
        pairs.push([x, y]);
    }
    const [a, b, c, d] = pairs;
    const corners = all.length === 1 ? [a, a, a, a] : all.length === 2 ? [a, b, a, b] : all.length === 3 ? [a, b, c, b] : [a, b, c, d];
    return corners.flat();
}

/// What `measureText()` gives.
class TextMetrics {
    constructor(found) {
        [this.width, this.actualBoundingBoxLeft, this.actualBoundingBoxRight, this.actualBoundingBoxAscent, this.actualBoundingBoxDescent,
            this.fontBoundingBoxAscent, this.fontBoundingBoxDescent, this.emHeightAscent, this.emHeightDescent, this.hangingBaseline,
            this.alphabeticBaseline, this.ideographicBaseline] = found;
    }
}

class CanvasGradient {
    constructor(kind, values) {
        Object.defineProperty(this, '_id', { value: nextId++ });
        room(2 + values.length);
        list[count++] = kind;
        list[count++] = this._id;
        for (const v of values) {
            list[count++] = v;
        }
        freed?.register(this, [1, this._id]);
    }

    addColorStop(offset, value) {
        const at = Number(offset);
        if (!(at >= 0 && at <= 1)) {
            throw domException('IndexSizeError', `Failed to execute 'addColorStop' on 'CanvasGradient': The provided value (${offset}) is outside the range (0.0, 1.0).`);
        }
        const found = color(value);
        if (found === null) {
            throw domException('SyntaxError', `Failed to execute 'addColorStop' on 'CanvasGradient': The value provided ('${value}') could not be parsed as a color.`);
        }
        room(4);
        list[count++] = STOP;
        list[count++] = this._id;
        list[count++] = at;
        list[count++] = found[0];
    }
}

class CanvasPattern {
    constructor(source, repeat, surface) {
        Object.defineProperty(this, '_id', { value: nextId++ });
        on(surface, 5);
        list[count++] = PATTERN;
        list[count++] = this._id;
        list[count++] = source[0];
        list[count++] = source[1];
        list[count++] = repeat;
        freed?.register(this, [2, this._id]);
    }

    setTransform(matrix = {}) {
        const m = matrix;
        const values = [m.a ?? m.m11 ?? 1, m.b ?? m.m12 ?? 0, m.c ?? m.m21 ?? 0, m.d ?? m.m22 ?? 1, m.e ?? m.m41 ?? 0, m.f ?? m.m42 ?? 0].map(Number);
        if (!finite(...values)) {
            return;
        }
        room(8);
        list[count++] = PATTERN_TRANSFORM;
        list[count++] = this._id;
        for (const v of values) {
            list[count++] = v;
        }
    }
}

/// A path to draw again, as many times as it's wanted, through whatever the transform is then.
class Path2D {
    constructor(from) {
        Object.defineProperty(this, '_ops', { value: [], writable: true });
        Object.defineProperty(this, '_id', { value: 0, writable: true });
        Object.defineProperty(this, '_sent', { value: -1, writable: true });
        if (from instanceof Path2D) {
            this._ops = from._ops.slice();
        } else if (typeof from === 'string') {
            svgPath(from, this);
        }
    }

    /// Its number in Rust, having sent what it has now if that's changed.
    _sync() {
        if (this._id === 0) {
            this._id = nextId++;
            freed?.register(this, [3, this._id]);
        }
        if (this._sent !== this._ops.length) {
            const ops = this._ops;
            room(3 + ops.length);
            list[count++] = PATH_DEFINE;
            list[count++] = this._id;
            list[count++] = ops.length;
            for (let i = 0; i < ops.length; i += 1) {
                list[count++] = ops[i];
            }
            this._sent = ops.length;
        }
        return this._id;
    }

    _push(code, values) {
        for (const v of values) {
            if (!Number.isFinite(Number(v))) {
                return;
            }
        }
        this._ops.push(code, ...values.map(Number));
    }

    addPath(path, transform) {
        if (!(path instanceof Path2D)) {
            throw new TypeError("Failed to execute 'addPath' on 'Path2D': parameter 1 is not of type 'Path2D'.");
        }
        const m = transform ?? {};
        const values = [m.a ?? m.m11 ?? 1, m.b ?? m.m12 ?? 0, m.c ?? m.m21 ?? 0, m.d ?? m.m22 ?? 1, m.e ?? m.m41 ?? 0, m.f ?? m.m42 ?? 0].map(Number);
        if (!finite(...values)) {
            return;
        }
        this._ops.push(ADD_PATH, path._ops.length, ...values, ...path._ops);
    }

    closePath() {
        this._ops.push(CLOSE);
    }
    moveTo(x, y) {
        this._push(MOVE, [x, y]);
    }
    lineTo(x, y) {
        this._push(LINE, [x, y]);
    }
    quadraticCurveTo(cx, cy, x, y) {
        this._push(QUAD, [cx, cy, x, y]);
    }
    bezierCurveTo(c1x, c1y, c2x, c2y, x, y) {
        this._push(BEZIER, [c1x, c1y, c2x, c2y, x, y]);
    }
    arc(x, y, radius, startAngle, endAngle, anticlockwise = false) {
        if (Number(radius) < 0) {
            throw domException('IndexSizeError', `Failed to execute 'arc' on 'Path2D': The radius provided (${radius}) is negative.`);
        }
        this._push(ARC, [x, y, radius, startAngle, endAngle, anticlockwise ? 1 : 0]);
    }
    arcTo(x1, y1, x2, y2, radius) {
        if (Number(radius) < 0) {
            throw domException('IndexSizeError', `Failed to execute 'arcTo' on 'Path2D': The radius provided (${radius}) is negative.`);
        }
        this._push(ARC_TO, [x1, y1, x2, y2, radius]);
    }
    ellipse(x, y, radiusX, radiusY, rotation, startAngle, endAngle, anticlockwise = false) {
        if (Number(radiusX) < 0 || Number(radiusY) < 0) {
            throw domException('IndexSizeError', "Failed to execute 'ellipse' on 'Path2D': The radius provided is negative.");
        }
        this._push(ELLIPSE, [x, y, radiusX, radiusY, rotation, startAngle, endAngle, anticlockwise ? 1 : 0]);
    }
    rect(x, y, w, h) {
        this._push(RECT, [x, y, w, h]);
    }
    roundRect(x, y, w, h, radii = 0) {
        const corners = roundRectRadii(radii, 'roundRect', 'Path2D');
        if (corners !== null) {
            this._push(ROUND_RECT, [x, y, w, h, ...corners]);
        }
    }
}

/// Reads SVG path data, as `new Path2D('M0 0 L10 10')` is given, into a path's steps, as far as it reads, as a browser does.
function svgPath(d, path) {
    const tokens = String(d).match(/[a-df-z]|[-+]?(?:[0-9]*\.[0-9]+|[0-9]+\.?)(?:e[-+]?[0-9]+)?/gi) ?? [];
    let at = 0;
    let command = '';
    let x = 0;
    let y = 0;
    let startX = 0;
    let startY = 0;
    let lastControl = null;
    let lastQuad = null;
    const isNumber = () => at < tokens.length && !/^[a-z]$/i.test(tokens[at]);
    const n = () => Number(tokens[at++]);
    const flag = () => {
        // Flags can be written with nothing between them, as `a1 1 0 00 10 10`.
        const token = tokens[at];
        if (token.length > 1 && (token[0] === '0' || token[0] === '1')) {
            tokens[at] = token.slice(1);
            return token[0] === '1';
        }
        at += 1;
        return token === '1';
    };
    while (at < tokens.length) {
        if (/^[a-z]$/i.test(tokens[at])) {
            command = tokens[at++];
        } else if (command === '') {
            return;
        }
        const relative = command === command.toLowerCase();
        const ox = relative ? x : 0;
        const oy = relative ? y : 0;
        switch (command.toLowerCase()) {
            case 'm': {
                if (!isNumber()) return;
                x = ox + n();
                y = oy + n();
                path.moveTo(x, y);
                startX = x;
                startY = y;
                command = relative ? 'l' : 'L';
                lastControl = lastQuad = null;
                break;
            }
            case 'l':
                if (!isNumber()) return;
                x = ox + n();
                y = oy + n();
                path.lineTo(x, y);
                lastControl = lastQuad = null;
                break;
            case 'h':
                if (!isNumber()) return;
                x = ox + n();
                path.lineTo(x, y);
                lastControl = lastQuad = null;
                break;
            case 'v':
                if (!isNumber()) return;
                y = oy + n();
                path.lineTo(x, y);
                lastControl = lastQuad = null;
                break;
            case 'c': {
                if (!isNumber()) return;
                const v = [n(), n(), n(), n(), n(), n()];
                path.bezierCurveTo(ox + v[0], oy + v[1], ox + v[2], oy + v[3], ox + v[4], oy + v[5]);
                lastControl = [ox + v[2], oy + v[3]];
                lastQuad = null;
                x = ox + v[4];
                y = oy + v[5];
                break;
            }
            case 's': {
                if (!isNumber()) return;
                const v = [n(), n(), n(), n()];
                const c1 = lastControl === null ? [x, y] : [2 * x - lastControl[0], 2 * y - lastControl[1]];
                path.bezierCurveTo(c1[0], c1[1], ox + v[0], oy + v[1], ox + v[2], oy + v[3]);
                lastControl = [ox + v[0], oy + v[1]];
                lastQuad = null;
                x = ox + v[2];
                y = oy + v[3];
                break;
            }
            case 'q': {
                if (!isNumber()) return;
                const v = [n(), n(), n(), n()];
                path.quadraticCurveTo(ox + v[0], oy + v[1], ox + v[2], oy + v[3]);
                lastQuad = [ox + v[0], oy + v[1]];
                lastControl = null;
                x = ox + v[2];
                y = oy + v[3];
                break;
            }
            case 't': {
                if (!isNumber()) return;
                const v = [n(), n()];
                const c = lastQuad === null ? [x, y] : [2 * x - lastQuad[0], 2 * y - lastQuad[1]];
                path.quadraticCurveTo(c[0], c[1], ox + v[0], oy + v[1]);
                lastQuad = c;
                lastControl = null;
                x = ox + v[0];
                y = oy + v[1];
                break;
            }
            case 'a': {
                if (!isNumber()) return;
                const rx = Math.abs(n());
                const ry = Math.abs(n());
                const rotation = (n() * Math.PI) / 180;
                const large = flag();
                const sweep = flag();
                const tx = ox + n();
                const ty = oy + n();
                svgArc(path, x, y, rx, ry, rotation, large, sweep, tx, ty);
                x = tx;
                y = ty;
                lastControl = lastQuad = null;
                break;
            }
            case 'z':
                path.closePath();
                x = startX;
                y = startY;
                lastControl = lastQuad = null;
                // A `z` takes nothing, so what follows it starts again.
                command = '';
                break;
            default:
                return;
        }
    }
}

/// An SVG arc, from its ends to its middle and angles, as the SVG spec works that out.
function svgArc(path, x1, y1, rx, ry, phi, large, sweep, x2, y2) {
    if (rx === 0 || ry === 0 || (x1 === x2 && y1 === y2)) {
        path.lineTo(x2, y2);
        return;
    }
    const cos = Math.cos(phi);
    const sin = Math.sin(phi);
    const dx = (x1 - x2) / 2;
    const dy = (y1 - y2) / 2;
    const x1p = cos * dx + sin * dy;
    const y1p = -sin * dx + cos * dy;
    const lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if (lambda > 1) {
        rx *= Math.sqrt(lambda);
        ry *= Math.sqrt(lambda);
    }
    const sign = large === sweep ? -1 : 1;
    const numerator = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    const root = sign * Math.sqrt(Math.max(0, numerator / (rx * rx * y1p * y1p + ry * ry * x1p * x1p)));
    const cxp = (root * rx * y1p) / ry;
    const cyp = (-root * ry * x1p) / rx;
    const cx = cos * cxp - sin * cyp + (x1 + x2) / 2;
    const cy = sin * cxp + cos * cyp + (y1 + y2) / 2;
    const angle = (ux, uy, vx, vy) => {
        const a = Math.atan2(ux * vy - uy * vx, ux * vx + uy * vy);
        return a;
    };
    const start = angle(1, 0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let delta = angle((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry);
    if (!sweep && delta > 0) delta -= 2 * Math.PI;
    if (sweep && delta < 0) delta += 2 * Math.PI;
    path.ellipse(cx, cy, rx, ry, phi, start, start + delta, !sweep);
}

/// Pixels as `getImageData()` gives them and `putImageData()` takes them.
class ImageData {
    constructor(dataOrWidth, widthOrHeight, heightOrSettings, settings) {
        let data;
        let width;
        let height;
        if (typeof dataOrWidth === 'object' && dataOrWidth !== null && ArrayBuffer.isView(dataOrWidth)) {
            data = dataOrWidth;
            width = Math.trunc(Number(widthOrHeight));
            if (!(data instanceof Uint8ClampedArray)) {
                throw new TypeError("Failed to construct 'ImageData': parameter 1 is not of type 'Uint8ClampedArray'.");
            }
            if (data.length === 0 || data.length % 4 !== 0) {
                throw domException('InvalidStateError', "Failed to construct 'ImageData': The input data length is not a multiple of 4.");
            }
            if (!(width > 0) || (data.length / 4) % width !== 0) {
                throw domException('IndexSizeError', "Failed to construct 'ImageData': The input data length is not a multiple of (4 * width).");
            }
            height = typeof heightOrSettings === 'number' ? Math.trunc(heightOrSettings) : data.length / 4 / width;
            if (height * width * 4 !== data.length) {
                throw domException('IndexSizeError', "Failed to construct 'ImageData': The input data length is not equal to (4 * width * height).");
            }
        } else {
            width = Math.trunc(Number(dataOrWidth));
            height = Math.trunc(Number(widthOrHeight));
            if (!(width > 0) || !(height > 0)) {
                throw domException('IndexSizeError', `Failed to construct 'ImageData': The source ${width > 0 ? 'height' : 'width'} is zero or not a number.`);
            }
            data = new Uint8ClampedArray(width * height * 4);
        }
        Object.defineProperty(this, 'width', { value: width, enumerable: true });
        Object.defineProperty(this, 'height', { value: height, enumerable: true });
        Object.defineProperty(this, 'data', { value: data, enumerable: true });
        Object.defineProperty(this, 'colorSpace', { value: 'srgb', enumerable: true });
    }
}

/// A picture kept as it was, from `createImageBitmap()` or `transferToImageBitmap()`.
class ImageBitmap {
    constructor(id, width, height) {
        Object.defineProperty(this, '_id', { value: id, writable: true });
        Object.defineProperty(this, '_width', { value: width, writable: true });
        Object.defineProperty(this, '_height', { value: height, writable: true });
        freed?.register(this, [4, id], this);
    }
    get width() {
        return this._width;
    }
    get height() {
        return this._height;
    }
    close() {
        if (this._id !== 0) {
            host.canvasReleaseImage(this._id);
            freed?.unregister(this);
            this._id = 0;
            this._width = 0;
            this._height = 0;
        }
    }
}

/// Which picture something given to `drawImage()` is, as its kind (0 for a picture, 1 for a canvas), its number and its size, or
/// `null` for what isn't one of those.
const pictureIds = new WeakMap();

function imageSource(image) {
    if (image === null || image === undefined) {
        throw new TypeError("Failed to execute 'drawImage' on 'CanvasRenderingContext2D': The provided value is not of type '(CSSImageValue or HTMLCanvasElement or HTMLImageElement or HTMLVideoElement or ImageBitmap or OffscreenCanvas or SVGImageElement or VideoFrame)'.");
    }
    if (image instanceof ImageBitmap) {
        if (image._id === 0) {
            throw domException('InvalidStateError', "Failed to execute 'drawImage' on 'CanvasRenderingContext2D': The image source is detached.");
        }
        return [0, image._id, image._width, image._height];
    }
    const canvas = canvasStateOf(image, false);
    if (canvas !== null) {
        if (canvas.surface.width === 0 || canvas.surface.height === 0) {
            throw domException('InvalidStateError', "Failed to execute 'drawImage' on 'CanvasRenderingContext2D': The image argument is a canvas element with a width or height of 0.");
        }
        return [1, canvas.surface.id, canvas.surface.width, canvas.surface.height];
    }
    const src = image.currentSrc || image.src || image.__live?.given?.src || image.getAttribute?.('src');
    const tag = String(image.tagName ?? image.localName ?? '').toLowerCase();
    if (typeof src === 'string' && src !== '' && (tag === 'img' || tag === 'image' || image instanceof PageImage)) {
        let known = pictureIds.get(image);
        if (known === undefined || known.src !== src) {
            known = { src, id: nextId++, size: null };
            pictureIds.set(image, known);
            const size = host.canvasLoadImage(known.id, src);
            known.size = size ?? [0, 0];
        }
        return [0, known.id, known.size[0], known.size[1]];
    }
    if (tag === 'video') {
        notHere('Drawing a <video> on a canvas');
        return null;
    }
    if (tag === 'img') {
        return null;
    }
    throw new TypeError("Failed to execute 'drawImage' on 'CanvasRenderingContext2D': The provided value is not of type '(CSSImageValue or HTMLCanvasElement or HTMLImageElement or HTMLVideoElement or ImageBitmap or OffscreenCanvas or SVGImageElement or VideoFrame)'.");
}

// Canvases: a page's `<canvas>`, one `document.createElement('canvas')` made, and `OffscreenCanvas`, each with what's behind it.

/// What a canvas element has: its surface and its context. What code gives it through `style` is the page element's own, as
/// any element's is: see `styleFor` in `web.js`.
class CanvasState {
    constructor(width, height, redraw) {
        this.surface = new Surface(width, height);
        this.context = null;
        this.kind = null;
        this.redraw = redraw;
        /// The size the page's own `width` and `height` last said, so that only a change to them makes it that size again.
        this.givenWidth = undefined;
        this.givenHeight = undefined;
        this.refs = new Map();
    }

    get width() {
        return this.surface.width;
    }

    get height() {
        return this.surface.height;
    }

    /// What it's shown as: an image that's this canvas.
    source() {
        return `canvas:${this.surface.id}:${this.surface.width}x${this.surface.height}`;
    }

    setSize(width, height) {
        this.surface.resize(width, height);
        this.context?._forget();
        this.redraw?.();
    }

    /// What the page's own `width` and `height` say now, which makes it that size if they've changed.
    given(width, height) {
        const w = attributeSize(width, 300);
        const h = attributeSize(height, 150);
        if (w !== this.givenWidth || h !== this.givenHeight) {
            const first = this.givenWidth === undefined;
            this.givenWidth = w;
            this.givenHeight = h;
            if (first && w === this.surface.width && h === this.surface.height) {
                return;
            }
            this.surface.resize(w, h);
            this.context?._forget();
        }
    }

    getContext(element, type, attributes) {
        const kind = String(type);
        if (kind !== '2d') {
            if (kind === 'webgl' || kind === 'webgl2' || kind === 'experimental-webgl' || kind === 'webgpu' || kind === 'bitmaprenderer') {
                notHere(`A canvas's '${kind}' context`);
            }
            return null;
        }
        if (this.context === null) {
            const opaque = attributes?.alpha === false;
            if (opaque) {
                this.surface = new Surface(this.surface.width, this.surface.height, true);
                this.redraw?.();
            }
            this.context = new CanvasRenderingContext2D(element, this.surface, {
                alpha: !opaque,
                colorSpace: 'srgb',
                desynchronized: attributes?.desynchronized === true,
                willReadFrequently: attributes?.willReadFrequently === true,
            });
        }
        return this.context;
    }
}

/// How big a canvas's `width` or `height` says it is, as a browser reads the attribute: a whole number that isn't negative, or what it
/// is when it's not said.
function attributeSize(value, otherwise) {
    if (value === undefined || value === null || value === '') {
        return otherwise;
    }
    const n = typeof value === 'number' ? Math.trunc(value) : parseInt(String(value).trim(), 10);
    return Number.isFinite(n) && n >= 0 ? n : otherwise;
}

/// The canvas behind an element that's a canvas, made the first time one's asked about if `make` says.
function canvasStateOf(element, make = true) {
    if (element === null || typeof element !== 'object') {
        return null;
    }
    if (element instanceof OffscreenCanvas) {
        return element._state;
    }
    const kept = element.__canvas ?? element.__live?.given?.__canvas;
    if (kept !== undefined) {
        return kept;
    }
    const tag = String(element.localName ?? element.tagName ?? '').toLowerCase();
    if (!make || tag !== 'canvas') {
        return null;
    }
    // One `document.createElement()` made, which isn't drawn anywhere, or a `<canvas>` there's no page component for.
    // An element made by `rsui/dom` keeps its attributes as a list of names and values (see `attrsOf` in dom.js), which is a new list
    // when one's added, so it's read each time.
    const said = (name, otherwise) => {
        const attributes = element.__attributes ?? element.__attrs;
        const value = attributes instanceof Map ? attributes.get(name) : Array.isArray(attributes) ? element.getAttribute(name) ?? undefined : undefined;
        return attributeSize(value, otherwise);
    };
    const state = new CanvasState(said('width', 300), said('height', 150), null);
    Object.defineProperty(element, '__canvas', { value: state, configurable: true });
    // `setAttribute('width', ...)` makes it that size, as setting `width` does.
    const changed = element.__attributesChanged;
    element.__attributesChanged = (name) => {
        changed?.(name);
        if (name === 'width' || name === 'height') {
            state.setSize(said('width', 300), said('height', 150));
        }
    };
    return state;
}

/// What a canvas element has, on `HTMLCanvasElement.prototype`, which every canvas has.
const canvasElementProps = {
    width: {
        get() {
            return canvasStateOf(this).width;
        },
        set(value) {
            const state = canvasStateOf(this);
            state.setSize(attributeSize(value, 0), state.height);
        },
        configurable: true,
    },
    height: {
        get() {
            return canvasStateOf(this).height;
        },
        set(value) {
            const state = canvasStateOf(this);
            state.setSize(state.width, attributeSize(value, 0));
        },
        configurable: true,
    },
    getContext: {
        value(type, attributes) {
            return canvasStateOf(this).getContext(this, type, attributes);
        },
        configurable: true,
        writable: true,
    },
    toDataURL: {
        value(type, quality) {
            const state = canvasStateOf(this);
            flush();
            return host.canvasDataUrl(state.surface.id, String(type ?? 'image/png'), quality);
        },
        configurable: true,
        writable: true,
    },
    toBlob: {
        value(callback, type, quality) {
            if (typeof callback !== 'function') {
                throw new TypeError("Failed to execute 'toBlob' on 'HTMLCanvasElement': The callback provided as parameter 1 is not a function.");
            }
            const state = canvasStateOf(this);
            flush();
            const bytes = host.canvasEncode(state.surface.id, String(type ?? 'image/png'), quality);
            setTimeout(() => callback(bytes === null ? null : new Blob([bytes], { type: 'image/png' })), 0);
        },
        configurable: true,
        writable: true,
    },
    transferControlToOffscreen: {
        value() {
            notHere("A canvas's transferControlToOffscreen()");
            throw domException('InvalidStateError', "Failed to execute 'transferControlToOffscreen' on 'HTMLCanvasElement': Cannot transfer control from a canvas for more than one time.");
        },
        configurable: true,
        writable: true,
    },
    captureStream: {
        value() {
            notHere("A canvas's captureStream()");
            return null;
        },
        configurable: true,
        writable: true,
    },
};

/// Gives a page element what a canvas has, through its prototype.
function attach(element, state) {
    if (element === null || typeof element !== 'object' || element.__canvasAttached === state) {
        return;
    }
    Object.defineProperty(element, '__canvas', { value: state, configurable: true });
    Object.defineProperty(element, '__canvasAttached', { value: state, configurable: true });
    for (const name of Object.keys(canvasElementProps)) {
        // What every element was given that a canvas has its own of.
        if (Object.prototype.hasOwnProperty.call(element, name)) {
            delete element[name];
        }
    }
    if (!('getContext' in element)) {
        Object.defineProperties(element, canvasElementProps);
    }
}

/// A canvas that isn't on the page, as a worker would have.
class OffscreenCanvas {
    constructor(width, height) {
        const w = attributeSize(width, 0);
        const h = attributeSize(height, 0);
        Object.defineProperty(this, '_state', { value: new CanvasState(w, h, null) });
        this.oncontextlost = null;
        this.oncontextrestored = null;
    }
    get width() {
        return this._state.width;
    }
    set width(value) {
        this._state.setSize(attributeSize(value, 0), this._state.height);
    }
    get height() {
        return this._state.height;
    }
    set height(value) {
        this._state.setSize(this._state.width, attributeSize(value, 0));
    }
    getContext(type, attributes) {
        return this._state.getContext(this, type, attributes);
    }
    convertToBlob(options) {
        flush();
        const bytes = host.canvasEncode(this._state.surface.id, String(options?.type ?? 'image/png'), options?.quality);
        if (bytes === null) {
            return Promise.reject(domException('IndexSizeError', "Failed to execute 'convertToBlob' on 'OffscreenCanvas': The canvas has no pixels."));
        }
        return Promise.resolve(new Blob([bytes], { type: 'image/png' }));
    }
    transferToImageBitmap() {
        flush();
        const id = nextId++;
        const size = host.canvasSnapshot(this._state.surface.id, id) ?? [0, 0];
        // What it had goes with the picture, as a browser has it.
        this._state.setSize(this._state.width, this._state.height);
        return new ImageBitmap(id, size[0], size[1]);
    }
    addEventListener() {}
    removeEventListener() {}
}

/// `createImageBitmap()`: a copy of a picture, a canvas, `ImageData` or another bitmap, or part of one, as it is now.
function createImageBitmap(source, sx, sy, sw, sh, options) {
    try {
        let crop = null;
        if (typeof sx === 'number') {
            crop = [sx, sy, sw, sh].map(Number);
            if (!finite(...crop) || crop[2] === 0 || crop[3] === 0) {
                return Promise.reject(domException('IndexSizeError', "Failed to execute 'createImageBitmap' on 'Window': The crop rect width is 0."));
            }
        } else {
            options = sx;
        }
        let width;
        let height;
        let draw;
        if (source instanceof ImageData) {
            width = source.width;
            height = source.height;
            draw = (ctx) => ctx.putImageData(source, crop ? -crop[0] : 0, crop ? -crop[1] : 0);
        } else if (source instanceof Blob) {
            return source.arrayBuffer().then((buffer) => {
                const bytes = new Uint8Array(buffer);
                let text = '';
                for (let i = 0; i < bytes.length; i += 1) {
                    text += String.fromCharCode(bytes[i]);
                }
                const image = new PageImage();
                image.src = `data:${source.type || 'image/png'};base64,${btoa(text)}`;
                return createImageBitmap(image, ...(crop ?? []), ...(options === undefined ? [] : [options]));
            });
        } else {
            const found = imageSource(source);
            if (found === null) {
                return Promise.reject(domException('InvalidStateError', "Failed to execute 'createImageBitmap' on 'Window': The source image could not be decoded."));
            }
            width = found[2];
            height = found[3];
            draw = (ctx) => (crop ? ctx.drawImage(source, crop[0], crop[1], crop[2], crop[3], 0, 0, Math.abs(crop[2]), Math.abs(crop[3])) : ctx.drawImage(source, 0, 0));
        }
        const outWidth = Math.round(options?.resizeWidth ?? (crop ? Math.abs(crop[2]) : width));
        const outHeight = Math.round(options?.resizeHeight ?? (crop ? Math.abs(crop[3]) : height));
        const scratch = new OffscreenCanvas(outWidth, outHeight);
        const ctx = scratch.getContext('2d');
        if (options?.resizeWidth !== undefined || options?.resizeHeight !== undefined) {
            ctx.scale(outWidth / (crop ? Math.abs(crop[2]) : width), outHeight / (crop ? Math.abs(crop[3]) : height));
        }
        if (options?.imageOrientation === 'flipY') {
            ctx.translate(0, outHeight);
            ctx.scale(1, -1);
        }
        draw(ctx);
        return Promise.resolve(scratch.transferToImageBitmap());
    } catch (error) {
        return Promise.reject(error);
    }
}

/// `new Image()`: a picture that reads its `src`, says `load` or `error` once what's running now has finished, and can be drawn.
class PageImage extends (globalThis.HTMLImageElement ?? Object) {
    constructor(width, height) {
        super();
        Object.defineProperty(this, '_src', { value: '', writable: true });
        Object.defineProperty(this, '_natural', { value: null, writable: true });
        Object.defineProperty(this, '_listeners', { value: new Map() });
        this.tagName = 'IMG';
        this.nodeName = 'IMG';
        this.localName = 'img';
        this.nodeType = 1;
        this.onload = null;
        this.onerror = null;
        this.crossOrigin = null;
        this.decoding = 'auto';
        this.loading = 'eager';
        this.alt = '';
        this._width = width === undefined ? undefined : Number(width);
        this._height = height === undefined ? undefined : Number(height);
    }
    get src() {
        return this._src;
    }
    set src(value) {
        this._src = String(value);
        this._natural = null;
        const src = this._src;
        setTimeout(() => {
            if (this._src !== src) {
                return;
            }
            const size = src === '' ? undefined : rsui.__host().imageSize(src);
            this._natural = size ?? [0, 0];
            const type = size === undefined ? 'error' : 'load';
            const event = { type, target: this, currentTarget: this, bubbles: false, preventDefault() {}, stopPropagation() {} };
            this[`on${type}`]?.call(this, event);
            for (const listener of this._listeners.get(type) ?? []) {
                typeof listener === 'function' ? listener.call(this, event) : listener.handleEvent?.(event);
            }
        }, 0);
    }
    get currentSrc() {
        return this._src;
    }
    get complete() {
        return this._src === '' || this._natural !== null;
    }
    get naturalWidth() {
        return this._natural?.[0] ?? 0;
    }
    get naturalHeight() {
        return this._natural?.[1] ?? 0;
    }
    get width() {
        return this._width ?? this.naturalWidth;
    }
    set width(value) {
        this._width = Number(value);
    }
    get height() {
        return this._height ?? this.naturalHeight;
    }
    set height(value) {
        this._height = Number(value);
    }
    decode() {
        return new Promise((resolve, reject) => {
            const check = () => {
                if (this._natural === null) {
                    setTimeout(check, 0);
                } else if (this._natural[0] === 0) {
                    reject(domException('EncodingError', 'The source image cannot be decoded.'));
                } else {
                    resolve();
                }
            };
            check();
        });
    }
    addEventListener(type, listener) {
        const list = this._listeners.get(type) ?? [];
        list.push(listener);
        this._listeners.set(type, list);
    }
    removeEventListener(type, listener) {
        const list = this._listeners.get(type) ?? [];
        const at = list.indexOf(listener);
        if (at >= 0) {
            list.splice(at, 1);
        }
    }
    setAttribute(name, value) {
        if (String(name).toLowerCase() === 'src') {
            this.src = value;
        } else {
            this[name] = value;
        }
    }
    getAttribute(name) {
        return String(name).toLowerCase() === 'src' ? this._src : this[name] ?? null;
    }
}

/// `DOMMatrix`, for what `getTransform()` gives and what's given to `setTransform()`, as far as two dimensions go.
class DOMMatrix {
    constructor(init) {
        let values = [1, 0, 0, 1, 0, 0];
        if (Array.isArray(init) || ArrayBuffer.isView(init)) {
            if (init.length === 6) {
                values = Array.from(init, Number);
            } else if (init.length === 16) {
                values = [init[0], init[1], init[4], init[5], init[12], init[13]].map(Number);
            } else {
                throw new TypeError("Failed to construct 'DOMMatrix': The sequence must contain 6 elements for a 2D matrix or 16 elements for a 3D matrix.");
            }
        } else if (typeof init === 'string' && init.trim() !== '' && init.trim() !== 'none') {
            values = parseTransformList(init);
        } else if (init !== null && typeof init === 'object') {
            values = [init.a ?? init.m11 ?? 1, init.b ?? init.m12 ?? 0, init.c ?? init.m21 ?? 0, init.d ?? init.m22 ?? 1, init.e ?? init.m41 ?? 0, init.f ?? init.m42 ?? 0].map(Number);
        }
        [this.a, this.b, this.c, this.d, this.e, this.f] = values;
    }
    static fromMatrix(other) {
        return new DOMMatrix(other);
    }
    static fromFloat32Array(array) {
        return new DOMMatrix(array);
    }
    static fromFloat64Array(array) {
        return new DOMMatrix(array);
    }
    get m11() { return this.a; }
    set m11(v) { this.a = v; }
    get m12() { return this.b; }
    set m12(v) { this.b = v; }
    get m21() { return this.c; }
    set m21(v) { this.c = v; }
    get m22() { return this.d; }
    set m22(v) { this.d = v; }
    get m41() { return this.e; }
    set m41(v) { this.e = v; }
    get m42() { return this.f; }
    set m42(v) { this.f = v; }
    get m13() { return 0; }
    get m14() { return 0; }
    get m23() { return 0; }
    get m24() { return 0; }
    get m31() { return 0; }
    get m32() { return 0; }
    get m33() { return 1; }
    get m34() { return 0; }
    get m43() { return 0; }
    get m44() { return 1; }
    get is2D() {
        return true;
    }
    get isIdentity() {
        return this.a === 1 && this.b === 0 && this.c === 0 && this.d === 1 && this.e === 0 && this.f === 0;
    }
    multiplySelf(other) {
        const o = new DOMMatrix(other);
        const [a, b, c, d, e, f] = [this.a, this.b, this.c, this.d, this.e, this.f];
        this.a = a * o.a + c * o.b;
        this.b = b * o.a + d * o.b;
        this.c = a * o.c + c * o.d;
        this.d = b * o.c + d * o.d;
        this.e = a * o.e + c * o.f + e;
        this.f = b * o.e + d * o.f + f;
        return this;
    }
    preMultiplySelf(other) {
        const o = new DOMMatrix(other);
        const copy = new DOMMatrix(this);
        Object.assign(this, o.multiplySelf(copy));
        return this;
    }
    multiply(other) {
        return new DOMMatrix(this).multiplySelf(other);
    }
    translateSelf(x = 0, y = 0) {
        return this.multiplySelf([1, 0, 0, 1, x, y]);
    }
    translate(x, y) {
        return new DOMMatrix(this).translateSelf(x, y);
    }
    scaleSelf(x = 1, y = x, _z, ox = 0, oy = 0) {
        this.translateSelf(ox, oy);
        this.multiplySelf([x, 0, 0, y, 0, 0]);
        return this.translateSelf(-ox, -oy);
    }
    scale(x, y, z, ox, oy) {
        return new DOMMatrix(this).scaleSelf(x, y, z, ox, oy);
    }
    rotateSelf(x = 0, y, z) {
        const degrees = z ?? (y === undefined ? x : 0);
        const r = (degrees * Math.PI) / 180;
        return this.multiplySelf([Math.cos(r), Math.sin(r), -Math.sin(r), Math.cos(r), 0, 0]);
    }
    rotate(x, y, z) {
        return new DOMMatrix(this).rotateSelf(x, y, z);
    }
    skewXSelf(sx = 0) {
        return this.multiplySelf([1, 0, Math.tan((sx * Math.PI) / 180), 1, 0, 0]);
    }
    skewX(sx) {
        return new DOMMatrix(this).skewXSelf(sx);
    }
    skewYSelf(sy = 0) {
        return this.multiplySelf([1, Math.tan((sy * Math.PI) / 180), 0, 1, 0, 0]);
    }
    skewY(sy) {
        return new DOMMatrix(this).skewYSelf(sy);
    }
    invertSelf() {
        const det = this.a * this.d - this.b * this.c;
        if (det === 0 || !Number.isFinite(det)) {
            [this.a, this.b, this.c, this.d, this.e, this.f] = [NaN, NaN, NaN, NaN, NaN, NaN];
            return this;
        }
        const [a, b, c, d, e, f] = [this.a, this.b, this.c, this.d, this.e, this.f];
        this.a = d / det;
        this.b = -b / det;
        this.c = -c / det;
        this.d = a / det;
        this.e = (c * f - d * e) / det;
        this.f = (b * e - a * f) / det;
        return this;
    }
    inverse() {
        return new DOMMatrix(this).invertSelf();
    }
    transformPoint(point = {}) {
        const x = point.x ?? 0;
        const y = point.y ?? 0;
        return { x: this.a * x + this.c * y + this.e, y: this.b * x + this.d * y + this.f, z: point.z ?? 0, w: point.w ?? 1 };
    }
    toFloat32Array() {
        return new Float32Array([this.a, this.b, 0, 0, this.c, this.d, 0, 0, 0, 0, 1, 0, this.e, this.f, 0, 1]);
    }
    toFloat64Array() {
        return new Float64Array([this.a, this.b, 0, 0, this.c, this.d, 0, 0, 0, 0, 1, 0, this.e, this.f, 0, 1]);
    }
    toJSON() {
        return { a: this.a, b: this.b, c: this.c, d: this.d, e: this.e, f: this.f, is2D: true, isIdentity: this.isIdentity };
    }
    toString() {
        return `matrix(${this.a}, ${this.b}, ${this.c}, ${this.d}, ${this.e}, ${this.f})`;
    }
}

/// What a CSS transform list comes to, as far as it goes in two dimensions.
function parseTransformList(text) {
    const m = new DOMMatrix();
    for (const [, name, args] of String(text).matchAll(/([a-zA-Z0-9]+)\(([^)]*)\)/g)) {
        const values = args.split(/[\s,]+/).filter((v) => v !== '').map((v) => parseFloat(v));
        const degrees = (v, raw) => (/rad$/.test(raw) ? (v * 180) / Math.PI : /turn$/.test(raw) ? v * 360 : v);
        const raw = args.trim();
        switch (name) {
            case 'matrix':
                m.multiplySelf(values);
                break;
            case 'translate':
                m.translateSelf(values[0], values[1] ?? 0);
                break;
            case 'translateX':
                m.translateSelf(values[0], 0);
                break;
            case 'translateY':
                m.translateSelf(0, values[0]);
                break;
            case 'scale':
                m.scaleSelf(values[0], values[1] ?? values[0]);
                break;
            case 'rotate':
                m.rotateSelf(degrees(values[0], raw));
                break;
            case 'skewX':
                m.skewXSelf(degrees(values[0], raw));
                break;
            case 'skewY':
                m.skewYSelf(degrees(values[0], raw));
                break;
            default:
                throw new SyntaxError(`Failed to construct 'DOMMatrix': Failed to parse '${text}'.`);
        }
    }
    return [m.a, m.b, m.c, m.d, m.e, m.f];
}

module.exports = {
    CanvasRenderingContext2D,
    OffscreenCanvasRenderingContext2D: CanvasRenderingContext2D,
    CanvasGradient,
    CanvasPattern,
    Path2D,
    ImageData,
    ImageBitmap,
    TextMetrics,
    OffscreenCanvas,
    DOMMatrix,
    PageImage,
    createImageBitmap,
    CanvasState,
    canvasElementProps,
    canvasStateOf,
    attach,
    attributeSize,
    flush,
};
