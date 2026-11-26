// What the studio does to a picture. A frame is first sampled into a grid of cells, each with its brightness and color, after the
// adjustments (brightness, contrast, gamma, saturation, inverting); then an effect draws that grid. Sampling is a box filter over
// the pixels each cell covers, so it's cheap enough to do for every frame of a video, and the effect only draws what it needs.

export type EffectId =
    | 'raw' | 'ascii' | 'braille' | 'blocks' | 'dither' | 'halftone' | 'dots' | 'pixel' | 'lines' | 'edges' | 'posterize' | 'matrix';

export const EFFECTS: { id: EffectId; name: string; hint: string }[] = [
    { id: 'raw', name: 'Raw', hint: 'The picture as it is' },
    { id: 'ascii', name: 'ASCII', hint: 'Characters by brightness' },
    { id: 'braille', name: 'Braille', hint: 'Fine dots, eight to a cell' },
    { id: 'blocks', name: 'Blocks', hint: 'Half blocks, two colors a cell' },
    { id: 'dither', name: 'Dither', hint: 'One bit, ordered or diffused' },
    { id: 'halftone', name: 'Halftone', hint: 'Dots that grow with dark' },
    { id: 'dots', name: 'LED', hint: 'A grid of glowing dots' },
    { id: 'pixel', name: 'Pixel', hint: 'Big flat squares' },
    { id: 'lines', name: 'Engrave', hint: 'Lines that thicken in shadow' },
    { id: 'edges', name: 'Edges', hint: 'Outlines drawn in characters' },
    { id: 'posterize', name: 'Poster', hint: 'A few flat tones' },
    { id: 'matrix', name: 'Rain', hint: 'Falling glyphs lit by the picture' },
];

export type ColorMode = 'source' | 'mono' | 'duotone' | 'scheme';
export type Fade = 'none' | 'left' | 'right' | 'top' | 'bottom' | 'radial' | 'corner';
export type Charset = 'classic' | 'dense' | 'blocks' | 'binary' | 'kana' | 'custom';

export const CHARSETS: Record<Exclude<Charset, 'custom'>, string> = {
    classic: ' .:-=+*#%@',
    dense: " .'`^\",:;Il!i><~+_-?][}{1)(|\\/tfjrxnuvczXYUJCLQ0OZmwqpdbkhao*#MW&8%B@$",
    blocks: ' ░▒▓█',
    binary: ' 01',
    kana: ' ･ｰｧｨｩｪｫｬｭｮｯｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄ',
};

export interface Design {
    effect: EffectId;
    /** A cell's size in pixels, which sets how fine the effect is. */
    cell: number;
    brightness: number;
    contrast: number;
    gamma: number;
    saturation: number;
    invert: boolean;
    /** Where one bit becomes the other, for dither, braille and edges. */
    threshold: number;
    charset: Charset;
    customChars: string;
    /** Ordered (Bayer) or diffused (Floyd-Steinberg), for dither. */
    diffuse: boolean;
    round: boolean;
    /** Space between cells, as a share of one. */
    gap: number;
    levels: number;
    color: ColorMode;
    mono: number;
    low: number;
    high: number;
    /** How strong it is over the panel. */
    opacity: number;
    fade: Fade;
    fit: 'cover' | 'contain';
    /** Played faster or slower than it was recorded. */
    speed: number;
}

export const DEFAULT_DESIGN: Design = {
    effect: 'ascii',
    cell: 10,
    brightness: 0,
    contrast: 1.1,
    gamma: 1,
    saturation: 1,
    invert: false,
    threshold: 0.5,
    charset: 'classic',
    customChars: ' .oO@',
    diffuse: false,
    round: true,
    gap: 0.15,
    levels: 4,
    color: 'scheme',
    mono: 0x82aaff,
    low: 0x1b2a4a,
    high: 0xc3a6ff,
    opacity: 0.35,
    fade: 'radial',
    fit: 'cover',
    speed: 1,
};

/** The part of what's drawn a canvas shows, in its own pixels, as a scene's view is. */
export interface View {
    x: number;
    y: number;
    w: number;
    h: number;
}

/** A frame's pixels, red, green, blue and alpha in each four bytes. */
export interface Frame {
    w: number;
    h: number;
    data: Uint8ClampedArray;
}

/** A grid of cells, each with how bright it is (0 to 1) and its color. */
export interface Cells {
    cols: number;
    rows: number;
    lum: Float32Array;
    r: Uint8Array;
    g: Uint8Array;
    b: Uint8Array;
}

/** The part of a frame a canvas of this size shows, covering it or fitting inside it. */
function crop(frame: Frame, W: number, H: number, fit: 'cover' | 'contain') {
    const fa = frame.w / frame.h;
    const ca = W / H;
    if (fit === 'cover') {
        if (fa > ca) {
            const w = frame.h * ca;
            return { sx: (frame.w - w) / 2, sy: 0, sw: w, sh: frame.h, dx: 0, dy: 0, dw: W, dh: H };
        }
        const h = frame.w / ca;
        return { sx: 0, sy: (frame.h - h) / 2, sw: frame.w, sh: h, dx: 0, dy: 0, dw: W, dh: H };
    }
    if (fa > ca) {
        const dh = W / fa;
        return { sx: 0, sy: 0, sw: frame.w, sh: frame.h, dx: 0, dy: (H - dh) / 2, dw: W, dh };
    }
    const dw = H * fa;
    return { sx: 0, sy: 0, sw: frame.w, sh: frame.h, dx: (W - dw) / 2, dy: 0, dw, dh: H };
}

/** Averages the pixels under each cell of a `cols` by `rows` grid laid over a `W` by `H` canvas, and adjusts them. */
export function sample(frame: Frame, cols: number, rows: number, W: number, H: number, d: Design): Cells {
    const n = cols * rows;
    const cells: Cells = { cols, rows, lum: new Float32Array(n), r: new Uint8Array(n), g: new Uint8Array(n), b: new Uint8Array(n) };
    const c = crop(frame, W, H, d.fit);
    const data = frame.data;
    const fw = frame.w;
    for (let y = 0; y < rows; y++) {
        for (let x = 0; x < cols; x++) {
            // The cell's place on the canvas, and the pixels of the frame under it.
            const px0 = (x / cols) * W;
            const py0 = (y / rows) * H;
            const px1 = ((x + 1) / cols) * W;
            const py1 = ((y + 1) / rows) * H;
            const i = y * cols + x;
            if (px1 <= c.dx || px0 >= c.dx + c.dw || py1 <= c.dy || py0 >= c.dy + c.dh) {
                cells.lum[i] = d.invert ? 1 : 0;
                continue;
            }
            const sx0 = Math.max(0, Math.floor(c.sx + ((px0 - c.dx) / c.dw) * c.sw));
            const sy0 = Math.max(0, Math.floor(c.sy + ((py0 - c.dy) / c.dh) * c.sh));
            const sx1 = Math.min(fw, Math.max(sx0 + 1, Math.ceil(c.sx + ((px1 - c.dx) / c.dw) * c.sw)));
            const sy1 = Math.min(frame.h, Math.max(sy0 + 1, Math.ceil(c.sy + ((py1 - c.dy) / c.dh) * c.sh)));
            // At most a few samples each way, so a big picture costs no more than a small one.
            const stepX = Math.max(1, Math.floor((sx1 - sx0) / 3));
            const stepY = Math.max(1, Math.floor((sy1 - sy0) / 3));
            let r = 0;
            let g = 0;
            let b = 0;
            let count = 0;
            for (let sy = sy0; sy < sy1; sy += stepY) {
                for (let sx = sx0; sx < sx1; sx += stepX) {
                    const k = (sy * fw + sx) * 4;
                    // As over black: a frame with see-through parts, as a drawn background's own picture is, is as dark there
                    // as it's faint. Most are opaque, which this leaves as they are.
                    const a = data[k + 3];
                    if (a === 255) {
                        r += data[k];
                        g += data[k + 1];
                        b += data[k + 2];
                    } else if (a > 0) {
                        const f = a / 255;
                        r += data[k] * f;
                        g += data[k + 1] * f;
                        b += data[k + 2] * f;
                    }
                    count++;
                }
            }
            r /= count;
            g /= count;
            b /= count;
            // Saturation pulls each channel toward the grey of the same brightness.
            const grey = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            r = grey + (r - grey) * d.saturation;
            g = grey + (g - grey) * d.saturation;
            b = grey + (b - grey) * d.saturation;
            let l = grey / 255;
            l = (l - 0.5) * d.contrast + 0.5 + d.brightness;
            l = Math.max(0, Math.min(1, l));
            if (d.gamma !== 1) l = Math.pow(l, 1 / d.gamma);
            if (d.invert) l = 1 - l;
            cells.lum[i] = l;
            cells.r[i] = Math.max(0, Math.min(255, r));
            cells.g[i] = Math.max(0, Math.min(255, g));
            cells.b[i] = Math.max(0, Math.min(255, b));
        }
    }
    return cells;
}

/** How much of the effect shows at a point, as the fade says: so the text over it stays easy to read. */
export function fadeAt(fade: Fade, u: number, v: number): number {
    switch (fade) {
        case 'left':
            return Math.min(1, Math.max(0, (u - 0.15) * 1.6));
        case 'right':
            return Math.min(1, Math.max(0, (0.85 - u) * 1.6));
        case 'top':
            return Math.min(1, Math.max(0, (v - 0.1) * 1.5));
        case 'bottom':
            return Math.min(1, Math.max(0, (0.9 - v) * 1.5));
        case 'radial':
            return Math.min(1, Math.max(0, (Math.hypot(u - 0.5, v - 0.5) - 0.15) * 2.2));
        case 'corner':
            return Math.min(1, Math.max(0, 1 - Math.hypot(u, v) * 1.1));
        default:
            return 1;
    }
}

const BAYER8 = (() => {
    const m = [0, 32, 8, 40, 2, 34, 10, 42, 48, 16, 56, 24, 50, 18, 58, 26, 12, 44, 4, 36, 14, 46, 6, 38, 60, 28, 52, 20, 62, 30, 54, 22, 3, 35, 11, 43, 1, 33, 9, 41, 51, 19, 59, 27, 49, 17, 57, 25, 15, 47, 7, 39, 13, 45, 5, 37, 63, 31, 55, 23, 61, 29, 53, 21];
    return m.map((v) => (v + 0.5) / 64);
})();

export interface Palette {
    /** The scheme's colors, for the `scheme` color mode: its text and its accent. */
    fg: number;
    accent: number;
}

/** A cell's color, as the design's color mode says, at a brightness `l`. */
function colorOf(d: Design, cells: Cells, i: number, l: number, p: Palette): [number, number, number] {
    switch (d.color) {
        case 'source':
            return [cells.r[i], cells.g[i], cells.b[i]];
        case 'mono':
            return [(d.mono >> 16) & 255, (d.mono >> 8) & 255, d.mono & 255];
        case 'duotone': {
            const mix = (shift: number) => Math.round(((d.low >> shift) & 255) * (1 - l) + ((d.high >> shift) & 255) * l);
            return [mix(16), mix(8), mix(0)];
        }
        default: {
            const mix = (shift: number) => Math.round(((p.accent >> shift) & 255) * (1 - l) + ((p.fg >> shift) & 255) * l);
            return [mix(16), mix(8), mix(0)];
        }
    }
}

/**
 * What an effect draws, gathered by color and laid on at the end: cells whose colors are within a 32nd of each other share one
 * path and one fill, and characters one `fillStyle`. A grid of cells drawn one fill each is ten thousand fills a frame; this is
 * a few hundred at most, and the difference can't be seen.
 */
class Inks {
    shapes = new Map<number, Path2D>();
    texts = new Map<number, { text: string; x: number; y: number }[]>();

    /** The key for a color and an alpha: each part to 5 bits, alpha to 32 steps. `-1` for nothing to draw. */
    static key(c: [number, number, number], a: number): number {
        const alpha = Math.round(Math.max(0, Math.min(1, a)) * 31);
        if (alpha === 0) return -1;
        return ((alpha * 32 + (c[0] >> 3)) * 32 + (c[1] >> 3)) * 32 + (c[2] >> 3);
    }

    static style(key: number): string {
        const b = key & 31;
        const g = (key >> 5) & 31;
        const r = (key >> 10) & 31;
        const alpha = key >> 15;
        // The middle of each step.
        return `rgba(${r * 8 + 4},${g * 8 + 4},${b * 8 + 4},${(alpha / 31).toFixed(3)})`;
    }

    /** The path to add a shape of this color to, or null if it wouldn't show. */
    shape(c: [number, number, number], a: number): Path2D | null {
        const key = Inks.key(c, a);
        if (key < 0) return null;
        let path = this.shapes.get(key);
        if (!path) {
            path = new Path2D();
            this.shapes.set(key, path);
        }
        return path;
    }

    rect(c: [number, number, number], a: number, x: number, y: number, w: number, h: number) {
        this.shape(c, a)?.rect(x, y, w, h);
    }

    circle(c: [number, number, number], a: number, x: number, y: number, r: number) {
        const path = this.shape(c, a);
        if (!path) return;
        path.moveTo(x + r, y);
        path.arc(x, y, r, 0, Math.PI * 2);
    }

    text(c: [number, number, number], a: number, text: string, x: number, y: number) {
        const key = Inks.key(c, a);
        if (key < 0) return;
        let list = this.texts.get(key);
        if (!list) {
            list = [];
            this.texts.set(key, list);
        }
        list.push({ text, x, y });
    }

    draw(ctx: CanvasRenderingContext2D) {
        for (const [key, path] of this.shapes) {
            ctx.fillStyle = Inks.style(key);
            ctx.fill(path);
        }
        for (const [key, list] of this.texts) {
            ctx.fillStyle = Inks.style(key);
            for (const t of list) ctx.fillText(t.text, t.x, t.y);
        }
    }
}

export function charsOf(d: Design): string {
    const set = d.charset === 'custom' ? d.customChars || ' #' : CHARSETS[d.charset];
    return set.length > 1 ? set : ' ' + set;
}

/** The grid an effect wants for a canvas: how many cells across and down, and how finely each is sampled. */
export function gridFor(d: Design, W: number, H: number): { cols: number; rows: number; sub: [number, number] } {
    const cw = Math.max(2, d.cell);
    // Characters are taller than they're wide.
    const textual = d.effect === 'ascii' || d.effect === 'edges' || d.effect === 'matrix' || d.effect === 'braille' || d.effect === 'blocks';
    const ch = textual ? cw * 1.8 : cw;
    const cols = Math.max(1, Math.round(W / cw));
    const rows = Math.max(1, Math.round(H / ch));
    const sub: [number, number] = d.effect === 'braille' ? [2, 4] : d.effect === 'blocks' ? [1, 2] : [1, 1];
    return { cols, rows, sub };
}

/**
 * Draws a design on a canvas `W` by `H`. `frame` is the picture (a video's current frame), `t` the time in seconds, for what moves.
 * The effect's own background is left clear, so the panel's color shows under it.
 */
export function render(ctx: CanvasRenderingContext2D, frame: Frame, d: Design, W: number, H: number, p: Palette, t = 0, view?: View) {
    if (d.effect === 'raw') {
        renderRaw(ctx, frame, d, W, H);
        return;
    }
    drawGrid(ctx, cellsOf(frame, d, W, H), d, W, H, p, t, view);
}

/** The cells an effect draws from, for a canvas `W` by `H`: what's worth keeping, for a picture that comes round again. */
export function cellsOf(frame: Frame, d: Design, W: number, H: number): Cells {
    const grid = gridFor(d, W, H);
    return sample(frame, grid.cols * grid.sub[0], grid.rows * grid.sub[1], W, H, d);
}

/** Draws a remix from sampled cells or from their finished pixels. */
export function drawGrid(ctx: CanvasRenderingContext2D, cells: Cells | Uint8ClampedArray, d: Design, W: number, H: number, p: Palette, t = 0, view?: View) {
    const grid = gridFor(d, W, H);
    const cw = W / grid.cols;
    const ch = H / grid.rows;
    const alpha = d.opacity;
    const at = (x: number, y: number) => fadeAt(d.fade, (x + 0.5) / grid.cols, (y + 0.5) / grid.rows) * alpha;
    if (cells instanceof Uint8ClampedArray || asImage(d)) {
        const pixels = cells instanceof Uint8ClampedArray ? cells : pixelsOf(d, cells, grid, at, p);
        drawCellImage(ctx, d, grid, cw, ch, W, H, pixels);
        return;
    }
    const { cols, rows } = cells;
    // The cells the view shows, in whole cells, with one more each way for what reaches over (halftone's offset rows). Dither
    // that's diffused carries each cell's error to the next, so it's always worked out from the first.
    const shown =
        view && !(d.effect === 'dither' && d.diffuse)
            ? {
                  x0: Math.max(0, Math.floor(view.x / cw) - 1),
                  y0: Math.max(0, Math.floor(view.y / ch) - 1),
                  x1: Math.min(grid.cols, Math.ceil((view.x + view.w) / cw) + 1),
                  y1: Math.min(grid.rows, Math.ceil((view.y + view.h) / ch) + 1),
              }
            : { x0: 0, y0: 0, x1: grid.cols, y1: grid.rows };
    const inks = new Inks();
    drawCells(ctx, inks, d, cells, grid, rows, cols, cw, ch, at, p, t, shown);
    inks.draw(ctx);
}

/** The small canvas the cells are drawn on a pixel each, and the masks that leave each cell's shape, by what they're for. */
const gridScratch: { canvas: HTMLCanvasElement | null; ctx: CanvasRenderingContext2D | null; pixels: Uint8ClampedArray | null } =
    { canvas: null, ctx: null, pixels: null };
const masks = new Map<string, HTMLCanvasElement>();

/** Whether an effect draws the same shape in every cell, only in its own color and strength, so it can be drawn as a picture. */
function asImage(d: Design): boolean {
    const e = d.effect;
    return e === 'dots' || e === 'pixel' || e === 'posterize' || e === 'dither' || e === 'blocks';
}

/**
 * The effects that draw the same shape in every cell, only in its own color and strength (LED dots, pixels, poster, dither and
 * half blocks), drawn as a picture: each cell a pixel of a small one, stretched over the canvas a cell to a pixel, then with
 * what's between the shapes taken out by a mask made once for the size. Three pictures drawn, however many cells; drawn as
 * shapes, it's one each, which is ten thousand a frame on a big panel. `given` is the cells' pixels if they've been made
 * already (on the worker, see backgrounds/loops.ts). Says whether it could.
 */
function drawCellImage(
    ctx: CanvasRenderingContext2D,
    d: Design,
    grid: { cols: number; rows: number },
    cw: number,
    ch: number,
    W: number,
    H: number,
    pixels: Uint8ClampedArray,
) {
    const gw = grid.cols;
    const gh = d.effect === 'blocks' ? grid.rows * 2 : grid.rows;
    if (!gridScratch.canvas || !gridScratch.ctx || gridScratch.canvas.width !== gw || gridScratch.canvas.height !== gh) {
        gridScratch.canvas = document.createElement('canvas');
        gridScratch.canvas.width = gw;
        gridScratch.canvas.height = gh;
        gridScratch.ctx = gridScratch.canvas.getContext('2d')!;
        gridScratch.pixels = null;
    }
    if (pixels !== gridScratch.pixels) {
        gridScratch.ctx.putImageData(new ImageData(pixels, gw, gh), 0, 0);
        gridScratch.pixels = pixels;
    }
    layOn(ctx, gridScratch.canvas, d, grid, gw, gh, cw, ch, W, H);
}

/** The pixels drawn for a set of cells, or `null` for an effect that draws the cells directly. */
export function cellPixels(cells: Cells, d: Design, W: number, H: number, p: Palette): Uint8ClampedArray | null {
    if (!asImage(d)) return null;
    const grid = gridFor(d, W, H);
    const at = (x: number, y: number) => fadeAt(d.fade, (x + 0.5) / grid.cols, (y + 0.5) / grid.rows) * d.opacity;
    return pixelsOf(d, cells, grid, at, p);
}

/** Each cell's color and strength as a pixel, not multiplied by alpha; half blocks a pixel for each half. */
function pixelsOf(d: Design, cells: Cells, grid: { cols: number; rows: number }, at: (x: number, y: number) => number, p: Palette): Uint8ClampedArray {
    const e = d.effect;
    const blocks = e === 'blocks';
    const gw = grid.cols;
    const gh = blocks ? grid.rows * 2 : grid.rows;
    const data = new Uint8ClampedArray(gw * gh * 4);
    const put = (px: number, py: number, c: [number, number, number], a: number) => {
        const k = (py * gw + px) * 4;
        data[k] = c[0];
        data[k + 1] = c[1];
        data[k + 2] = c[2];
        data[k + 3] = Math.max(0, Math.min(1, a)) * 255;
    };
    const cols = cells.cols;
    if (blocks) {
        // Each cell is two pixels, its top and bottom halves.
        for (let y = 0; y < grid.rows; y++) {
            for (let x = 0; x < grid.cols; x++) {
                const a = at(x, y);
                const top = y * 2 * cols + x;
                const bottom = (y * 2 + 1) * cols + x;
                put(x, y * 2, colorOf(d, cells, top, cells.lum[top], p), a * cells.lum[top] * 1.2);
                put(x, y * 2 + 1, colorOf(d, cells, bottom, cells.lum[bottom], p), a * cells.lum[bottom] * 1.2);
            }
        }
    } else if (e === 'dither') {
        // As `drawCells` has it: on or off, by a threshold that's ordered or carries each cell's error on.
        const lum = cells.lum.slice();
        const rows = cells.rows;
        for (let y = 0; y < rows; y++) {
            for (let x = 0; x < cols; x++) {
                const i = y * cols + x;
                const l = lum[i];
                let on: boolean;
                if (d.diffuse) {
                    on = l > d.threshold;
                    const err = l - (on ? 1 : 0);
                    if (x + 1 < cols) lum[i + 1] += (err * 7) / 16;
                    if (y + 1 < rows) {
                        if (x > 0) lum[i + cols - 1] += (err * 3) / 16;
                        lum[i + cols] += (err * 5) / 16;
                        if (x + 1 < cols) lum[i + cols + 1] += err / 16;
                    }
                } else {
                    on = l + (d.threshold - 0.5) * -1 > BAYER8[(y % 8) * 8 + (x % 8)];
                }
                put(x, y, colorOf(d, cells, i, cells.lum[i], p), on ? at(x, y) : 0);
            }
        }
    } else {
        for (let y = 0; y < grid.rows; y++) {
            for (let x = 0; x < grid.cols; x++) {
                const i = y * cols + x;
                let l = cells.lum[i];
                if (e === 'posterize') l = Math.round(l * (Math.max(2, d.levels) - 1)) / (Math.max(2, d.levels) - 1);
                const a = e === 'dots' ? at(x, y) * (0.08 + l * 0.92) : at(x, y) * (d.color === 'source' && e === 'pixel' ? 1 : l);
                put(x, y, colorOf(d, cells, i, l, p), a);
            }
        }
    }
    return data;
}

/** The cells' pixels stretched over the canvas a cell to a pixel, and what's between their shapes taken out. */
function layOn(ctx: CanvasRenderingContext2D, small: HTMLCanvasElement, d: Design, grid: { cols: number; rows: number }, gw: number, gh: number, cw: number, ch: number, W: number, H: number) {
    ctx.save();
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(small, 0, 0, gw, gh, 0, 0, W, H);
    const mask = maskFor(d, grid, cw, ch, W, H);
    if (mask) {
        ctx.globalCompositeOperation = 'destination-out';
        ctx.drawImage(mask, 0, 0);
    }
    ctx.restore();
}

/**
 * What's between each cell's shape, for `drawAsImage` to take out: the whole canvas, but for a hole the shape of every cell's
 * dot or square, worked out once for a size and kept. `null` for effects that fill their cells.
 */
function maskFor(d: Design, grid: { cols: number; rows: number }, cw: number, ch: number, W: number, H: number): HTMLCanvasElement | null {
    const e = d.effect;
    if (e === 'posterize' || e === 'blocks' || (e === 'pixel' && d.gap === 0)) return null;
    const key = `${W}x${H}:${grid.cols}x${grid.rows}:${e}:${d.gap}:${d.round}`;
    let mask = masks.get(key);
    if (mask) return mask;
    mask = document.createElement('canvas');
    mask.width = Math.ceil(W);
    mask.height = Math.ceil(H);
    const m = mask.getContext('2d')!;
    m.fillStyle = '#000';
    m.fillRect(0, 0, mask.width, mask.height);
    m.globalCompositeOperation = 'destination-out';
    m.beginPath();
    for (let y = 0; y < grid.rows; y++) {
        for (let x = 0; x < grid.cols; x++) {
            if (e === 'dots') {
                const r = (cw / 2) * (1 - d.gap);
                if (r < 0.4) continue;
                const cx = x * cw + cw / 2;
                const cy = y * ch + ch / 2;
                if (d.round) {
                    m.moveTo(cx + r, cy);
                    m.arc(cx, cy, r, 0, Math.PI * 2);
                } else {
                    m.rect(cx - r, cy - r, r * 2, r * 2);
                }
            } else if (e === 'dither') {
                const size = cw * (1 - d.gap * 0.5);
                if (d.round) {
                    m.moveTo(x * cw + cw / 2 + size / 2, y * ch + ch / 2);
                    m.arc(x * cw + cw / 2, y * ch + ch / 2, size / 2, 0, Math.PI * 2);
                } else {
                    m.rect(x * cw, y * ch, size, size);
                }
            } else {
                const g = cw * d.gap;
                m.rect(x * cw + g / 2, y * ch + g / 2, cw - g + 0.5, ch - g + 0.5);
            }
        }
    }
    m.fill();
    // Only the last few sizes are kept: each is a canvas's worth of pixels.
    if (masks.size >= 3) masks.delete(masks.keys().next().value!);
    masks.set(key, mask);
    return mask;
}

/** What `render` draws of each effect, gathered in `inks`. */
function drawCells(
    ctx: CanvasRenderingContext2D,
    inks: Inks,
    d: Design,
    cells: Cells,
    grid: { cols: number; rows: number; sub: [number, number] },
    rows: number,
    cols: number,
    cw: number,
    ch: number,
    at: (x: number, y: number) => number,
    p: Palette,
    t: number,
    shown: { x0: number; y0: number; x1: number; y1: number },
) {
    switch (d.effect) {
        case 'ascii':
        case 'matrix': {
            const chars = d.effect === 'matrix' ? CHARSETS.kana : charsOf(d);
            ctx.font = `${Math.round(ch * 0.82)}px "Cascadia Mono", Consolas, monospace`;
            ctx.textBaseline = 'top';
            for (let y = shown.y0; y < Math.min(rows, shown.y1); y++) {
                for (let x = shown.x0; x < Math.min(cols, shown.x1); x++) {
                    const i = y * cols + x;
                    let l = cells.lum[i];
                    if (d.effect === 'matrix') {
                        // A bright head falls down each column; the picture decides how strongly it lights what it passes.
                        const speed = 4 + ((x * 7919) % 5);
                        const head = ((t * speed + (x * 13) % rows) % (rows + 12)) - 6;
                        const trail = Math.max(0, 1 - Math.abs(y - head) / 9) * (y <= head ? 1 : 0.15);
                        l = l * (0.25 + trail);
                    }
                    const k = Math.min(chars.length - 1, Math.floor(l * chars.length));
                    const glyph = d.effect === 'matrix' ? chars[1 + ((x * 31 + y * 17 + Math.floor(t * 2)) % (chars.length - 1))] : chars[k];
                    if (glyph === ' ' || l < 0.04) continue;
                    const a = at(x, y) * (d.effect === 'matrix' ? Math.min(1, l * 1.6) : 0.55 + l * 0.45);
                    if (a < 0.02) continue;
                    inks.text(colorOf(d, cells, i, l, p), a, glyph, x * cw, y * ch);
                }
            }
            return;
        }
        case 'braille': {
            ctx.font = `${Math.round(ch * 0.9)}px "Segoe UI Symbol", "Cascadia Mono", monospace`;
            ctx.textBaseline = 'top';
            // Dots in a braille cell, by their place in the 2 by 4 grid: the bit each sets.
            const bits = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
            for (let y = shown.y0; y < shown.y1; y++) {
                for (let x = shown.x0; x < shown.x1; x++) {
                    let code = 0;
                    let sum = 0;
                    for (let sy = 0; sy < 4; sy++) {
                        for (let sx = 0; sx < 2; sx++) {
                            const i = (y * 4 + sy) * cols + x * 2 + sx;
                            const l = cells.lum[i];
                            sum += l;
                            if (l > d.threshold) code |= bits[sy][sx];
                        }
                    }
                    if (code === 0) continue;
                    const i = y * 4 * cols + x * 2;
                    inks.text(colorOf(d, cells, i, sum / 8, p), at(x, y), String.fromCharCode(0x2800 + code), x * cw, y * ch);
                }
            }
            return;
        }
        case 'blocks': {
            // Each cell is two pixels, top and bottom, as an upper half block and the cell's background.
            for (let y = shown.y0; y < shown.y1; y++) {
                for (let x = shown.x0; x < shown.x1; x++) {
                    const top = (y * 2) * cols + x;
                    const bottom = (y * 2 + 1) * cols + x;
                    const a = at(x, y);
                    inks.rect(colorOf(d, cells, top, cells.lum[top], p), a * cells.lum[top] * 1.2, x * cw, y * ch, cw + 0.5, ch / 2 + 0.5);
                    inks.rect(colorOf(d, cells, bottom, cells.lum[bottom], p), a * cells.lum[bottom] * 1.2, x * cw, y * ch + ch / 2, cw + 0.5, ch / 2 + 0.5);
                }
            }
            return;
        }
        case 'dither': {
            const lum = cells.lum.slice();
            const size = cw * (1 - d.gap * 0.5);
            for (let y = shown.y0; y < Math.min(rows, shown.y1); y++) {
                for (let x = shown.x0; x < Math.min(cols, shown.x1); x++) {
                    const i = y * cols + x;
                    const l = lum[i];
                    let on: boolean;
                    if (d.diffuse) {
                        on = l > d.threshold;
                        // Floyd-Steinberg: what's lost here goes on to the cells not drawn yet.
                        const err = l - (on ? 1 : 0);
                        if (x + 1 < cols) lum[i + 1] += (err * 7) / 16;
                        if (y + 1 < rows) {
                            if (x > 0) lum[i + cols - 1] += (err * 3) / 16;
                            lum[i + cols] += (err * 5) / 16;
                            if (x + 1 < cols) lum[i + cols + 1] += err / 16;
                        }
                    } else {
                        on = l + (d.threshold - 0.5) * -1 > BAYER8[(y % 8) * 8 + (x % 8)];
                    }
                    if (!on) continue;
                    const color = colorOf(d, cells, i, cells.lum[i], p);
                    if (d.round) inks.circle(color, at(x, y), x * cw + cw / 2, y * ch + ch / 2, size / 2);
                    else inks.rect(color, at(x, y), x * cw, y * ch, size, size);
                }
            }
            return;
        }
        case 'halftone':
        case 'dots': {
            for (let y = shown.y0; y < Math.min(rows, shown.y1); y++) {
                for (let x = shown.x0; x < Math.min(cols, shown.x1); x++) {
                    const i = y * cols + x;
                    const l = cells.lum[i];
                    const r = d.effect === 'halftone' ? (cw / 2) * Math.sqrt(l) * (1 - d.gap * 0.3) : (cw / 2) * (1 - d.gap);
                    const a = d.effect === 'halftone' ? at(x, y) : at(x, y) * (0.08 + l * 0.92);
                    if (r < 0.4 || a < 0.02) continue;
                    const color = colorOf(d, cells, i, l, p);
                    const cx = x * cw + cw / 2 + (d.effect === 'halftone' && y % 2 === 1 ? cw / 2 : 0);
                    if (d.round) inks.circle(color, a, cx, y * ch + ch / 2, r);
                    else inks.rect(color, a, cx - r, y * ch + ch / 2 - r, r * 2, r * 2);
                }
            }
            return;
        }
        case 'pixel':
        case 'posterize': {
            const g = d.effect === 'pixel' ? cw * d.gap : 0;
            for (let y = shown.y0; y < Math.min(rows, shown.y1); y++) {
                for (let x = shown.x0; x < Math.min(cols, shown.x1); x++) {
                    const i = y * cols + x;
                    let l = cells.lum[i];
                    if (d.effect === 'posterize') l = Math.round(l * (Math.max(2, d.levels) - 1)) / (Math.max(2, d.levels) - 1);
                    const a = at(x, y) * (d.color === 'source' && d.effect === 'pixel' ? 1 : l);
                    if (a < 0.02) continue;
                    inks.rect(colorOf(d, cells, i, l, p), a, x * cw + g / 2, y * ch + g / 2, cw - g + 0.5, ch - g + 0.5);
                }
            }
            return;
        }
        case 'lines': {
            for (let y = shown.y0; y < Math.min(rows, shown.y1); y++) {
                for (let x = shown.x0; x < Math.min(cols, shown.x1); x++) {
                    const i = y * cols + x;
                    const dark = 1 - cells.lum[i];
                    const th = ch * dark * (1 - d.gap * 0.5);
                    if (th < 0.3) continue;
                    inks.rect(colorOf(d, cells, i, cells.lum[i], p), at(x, y), x * cw, y * ch + (ch - th) / 2, cw + 0.5, th);
                }
            }
            return;
        }
        case 'edges': {
            ctx.font = `${Math.round(ch * 0.82)}px "Cascadia Mono", Consolas, monospace`;
            ctx.textBaseline = 'top';
            const L = (x: number, y: number) => cells.lum[Math.max(0, Math.min(rows - 1, y)) * cols + Math.max(0, Math.min(cols - 1, x))];
            for (let y = shown.y0; y < Math.min(rows, shown.y1); y++) {
                for (let x = shown.x0; x < Math.min(cols, shown.x1); x++) {
                    const gx = L(x + 1, y - 1) + 2 * L(x + 1, y) + L(x + 1, y + 1) - L(x - 1, y - 1) - 2 * L(x - 1, y) - L(x - 1, y + 1);
                    const gy = L(x - 1, y + 1) + 2 * L(x, y + 1) + L(x + 1, y + 1) - L(x - 1, y - 1) - 2 * L(x, y - 1) - L(x + 1, y - 1);
                    const m = Math.hypot(gx, gy);
                    if (m < d.threshold * 1.2) continue;
                    // The edge runs across the gradient.
                    const angle = ((Math.atan2(gy, gx) * 180) / Math.PI + 180) % 180;
                    const glyph = angle < 22.5 || angle >= 157.5 ? '|' : angle < 67.5 ? '/' : angle < 112.5 ? '-' : '\\';
                    const i = y * cols + x;
                    inks.text(colorOf(d, cells, i, Math.min(1, m / 2), p), at(x, y) * Math.min(1, m / 1.5), glyph, x * cw, y * ch);
                }
            }
            return;
        }
    }
}

/** Small canvases the raw effect reuses: the adjusted picture, and the fade's mask. */
const scratch: { picture: HTMLCanvasElement | null; mask: HTMLCanvasElement | null; maskKey: string } = { picture: null, mask: null, maskKey: '' };

function canvas(w: number, h: number, kept: HTMLCanvasElement | null): HTMLCanvasElement {
    const c = kept ?? document.createElement('canvas');
    if (c.width !== w) c.width = w;
    if (c.height !== h) c.height = h;
    return c;
}

/**
 * The picture itself: its pixels adjusted once (brightness, contrast, gamma, saturation, inverting), drawn scaled as the fit says,
 * at the design's strength, then faded by a small mask stretched over it. A few draws, however big the canvas.
 */
function renderRaw(ctx: CanvasRenderingContext2D, frame: Frame, d: Design, W: number, H: number) {
    const picture = (scratch.picture = canvas(frame.w, frame.h, scratch.picture));
    const pctx = picture.getContext('2d')!;
    const image = pctx.createImageData(frame.w, frame.h);
    const src = frame.data;
    const out = image.data;
    for (let k = 0; k < src.length; k += 4) {
        let r = src[k];
        let g = src[k + 1];
        let b = src[k + 2];
        const grey = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        r = grey + (r - grey) * d.saturation;
        g = grey + (g - grey) * d.saturation;
        b = grey + (b - grey) * d.saturation;
        const adjust = (v: number) => {
            let x = v / 255;
            x = (x - 0.5) * d.contrast + 0.5 + d.brightness;
            x = Math.max(0, Math.min(1, x));
            if (d.gamma !== 1) x = Math.pow(x, 1 / d.gamma);
            if (d.invert) x = 1 - x;
            return x * 255;
        };
        out[k] = adjust(r);
        out[k + 1] = adjust(g);
        out[k + 2] = adjust(b);
        out[k + 3] = 255;
    }
    pctx.putImageData(image, 0, 0);
    const c = crop(frame, W, H, d.fit);
    ctx.save();
    ctx.globalAlpha = d.opacity;
    ctx.imageSmoothingEnabled = true;
    ctx.drawImage(picture, c.sx, c.sy, c.sw, c.sh, c.dx, c.dy, c.dw, c.dh);
    ctx.restore();
    fadeOut(ctx, d.fade, W, H);
}

/** Fades what's on a canvas as `fade` says, by a small mask stretched over it. */
export function fadeOut(ctx: CanvasRenderingContext2D, fade: Fade, W: number, H: number) {
    if (fade === 'none') return;
    if (scratch.maskKey !== fade || !scratch.mask) {
        const mw = 64;
        const mh = 40;
        const mask = (scratch.mask = canvas(mw, mh, scratch.mask));
        const mctx = mask.getContext('2d')!;
        const m = mctx.createImageData(mw, mh);
        for (let y = 0; y < mh; y++) {
            for (let x = 0; x < mw; x++) {
                const k = (y * mw + x) * 4;
                m.data[k] = m.data[k + 1] = m.data[k + 2] = 255;
                m.data[k + 3] = Math.round(fadeAt(fade, (x + 0.5) / mw, (y + 0.5) / mh) * 255);
            }
        }
        mctx.putImageData(m, 0, 0);
        scratch.maskKey = fade;
    }
    ctx.save();
    ctx.globalCompositeOperation = 'destination-in';
    ctx.imageSmoothingEnabled = true;
    ctx.drawImage(scratch.mask!, 0, 0, W, H);
    ctx.restore();
}
