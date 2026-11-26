// The newer backgrounds, in the spirit of the ones React Bits makes with WebGL shaders, worked out again for a plain 2D canvas: an
// ASCII glow over a horizon, a lit dot pattern on a silk-like swell, slats on a sea seen in perspective, ribbons of light dots, a
// CRT's quantized plasma, a twisting pillar of light, an aurora, silk, a dark veil and a grainy gradient.
//
// Two ways of drawing. The ones made of marks (characters, dots, slats) work out a value per mark and draw it, marks that look the
// same batched into one path. The ones that are a color for every pixel go through `shader`: a small grid of pixels, a few times
// coarser than the panel, worked out one by one and stretched over it with smoothing, which suits soft light.

import type { BackgroundOptions, Scene } from './scenes';
import { frameAt, loopStore } from './loops';

// ---- Small helpers

const clamp = (x: number, lo = 0, hi = 1) => (x < lo ? lo : x > hi ? hi : x);
const mix = (a: number, b: number, k: number) => a + (b - a) * k;
const smoothstep = (a: number, b: number, x: number) => {
    const k = clamp((x - a) / (b - a));
    return k * k * (3 - 2 * k);
};
const rgbOf = (c: number): [number, number, number] => [((c >> 16) & 255) / 255, ((c >> 8) & 255) / 255, (c & 255) / 255];
/** Lengths, as Math.hypot gives, but much cheaper: hypot takes care over overflow that these never come near. */
const len2 = (x: number, y: number) => Math.sqrt(x * x + y * y);
const len3 = (x: number, y: number, z: number) => Math.sqrt(x * x + y * y + z * z);
const css = (r: number, g: number, b: number, a: number) =>
    `rgba(${Math.round(clamp(r) * 255)},${Math.round(clamp(g) * 255)},${Math.round(clamp(b) * 255)},${clamp(a).toFixed(3)})`;

/** A number from 0 to 1 that looks random, the same each time for the same whole numbers. */
function hash3(x: number, y: number, z: number): number {
    let h = Math.imul(x | 0, 374761393) + Math.imul(y | 0, 668265263) + Math.imul(z | 0, 1274126177);
    h = Math.imul(h ^ (h >>> 13), 1103515245);
    h ^= h >>> 16;
    return (h >>> 0) / 4294967296;
}

const fade = (t: number) => t * t * t * (t * (t * 6 - 15) + 10);

/** Smooth value noise in 2D, from 0 to 1. */
function noise2(x: number, y: number): number {
    const xi = Math.floor(x);
    const yi = Math.floor(y);
    const fx = fade(x - xi);
    const fy = fade(y - yi);
    const a = hash3(xi, yi, 0);
    const b = hash3(xi + 1, yi, 0);
    const c = hash3(xi, yi + 1, 0);
    const d = hash3(xi + 1, yi + 1, 0);
    return mix(mix(a, b, fx), mix(c, d, fx), fy);
}

/** A few octaves of noise2, from about 0 to 1. */
function fbm(x: number, y: number, octaves = 3): number {
    let sum = 0;
    let amp = 0.5;
    let norm = 0;
    for (let i = 0; i < octaves; i++) {
        sum += noise2(x, y) * amp;
        norm += amp;
        x = x * 2.03 + 17.1;
        y = y * 2.03 - 9.7;
        amp *= 0.5;
    }
    return sum / norm;
}

/** How long the shader scenes take to come round to where they started, in seconds: each is a seamless loop this long. */
export const LOOP = 12;

/**
 * A scene that's a color for every pixel. `pixel` is given a point (`u` from 0 to the panel's aspect, `v` from 0 at the top to 1),
 * where the loop is (`phase`, from 0 to 2π, as the scene goes round it once every `LOOP` seconds), and its place on the grid (`x`,
 * `y`), and writes light into `out` as red, green and blue that add to what's under it. `frame` runs once a frame first, for
 * what's the same across a row or a column, or what's cheaper worked out once a point. It's worked out on a grid `res` times
 * coarser than the panel and stretched over it.
 *
 * Everything that moves in one goes round the loop a whole number of times, and noise drifts in a circle rather than a line, so
 * the last frame runs on into the first. That makes the frames worth keeping, and worth working out ahead, away from drawing: the
 * loop's `fps * LOOP` frames are worked out on a worker (see `loops.ts`), from the one being shown on, and kept, shared by every
 * canvas that shows the same scene. Drawing only ever shows a frame that's ready, so it costs next to nothing from the first.
 * A loop of a panel's size keeps from 3 to 13 MB of frames. Another size starts again.
 */
function shader(
    o: BackgroundOptions,
    res: number,
    fps: number,
    pixel: (u: number, v: number, phase: number, out: Float32Array, x: number, y: number) => void,
    frame?: (aspect: number, phase: number, w: number, h: number) => void,
): Scene {
    const count = Math.round(LOOP * fps);
    const out = new Float32Array(3);
    const sizeOf = (W: number, H: number) => ({ w: Math.max(2, Math.ceil(W / res)), h: Math.max(2, Math.ceil(H / res)) });
    /** One frame of the loop, worked out now: what the worker does, and drawing does where there's no worker. */
    const compute = (index: number, W: number, H: number) => {
        const { w, h } = sizeOf(W, H);
        const data = new Uint8ClampedArray(w * h * 4);
        const aspect = W / H;
        const phase = (2 * Math.PI * index) / count;
        frame?.(aspect, phase, w, h);
        const strength = o.intensity;
        for (let y = 0; y < h; y++) {
            const v = (y + 0.5) / h;
            for (let x = 0; x < w; x++) {
                out[0] = out[1] = out[2] = 0;
                pixel(((x + 0.5) / w) * aspect, v, phase, out, x, y);
                // Light that adds is drawn as its brightest channel's alpha over the color it has at full strength.
                const r = out[0] * strength;
                const g = out[1] * strength;
                const b = out[2] * strength;
                const a = r > g ? (r > b ? r : b) : g > b ? g : b;
                if (a < 0.004) continue;
                const k = (y * w + x) * 4;
                const am = a > 1 ? 1 : a;
                data[k] = (r / a) * 255;
                data[k + 1] = (g / a) * 255;
                data[k + 2] = (b / a) * 255;
                data[k + 3] = am * 255;
            }
        }
        return { w, h, data };
    };
    let canvas: HTMLCanvasElement | null = null;
    let ctx: CanvasRenderingContext2D | null = null;
    let shownImage: ImageData | null = null;
    const loop = { fps, count };
    /** The frame for `t` if it's ready, or the nearest one before it that is. */
    const ready = (t: number, W: number, H: number): ImageData | null => {
        const store = loopStore(o, W, H, count);
        const index = ((Math.floor(t * fps) % count) + count) % count;
        return frameAt(store, index, () => compute(index, W, H), o.animate);
    };
    return {
        loop,
        compute,
        pixels: ready,
        draw(target, W, H, t) {
            const image = ready(t, W, H);
            if (!image) return;
            const { w, h } = sizeOf(W, H);
            if (!canvas || !ctx || canvas.width !== w || canvas.height !== h) {
                canvas = document.createElement('canvas');
                canvas.width = w;
                canvas.height = h;
                ctx = canvas.getContext('2d')!;
                shownImage = null;
            }
            if (image !== shownImage) {
                ctx.putImageData(image, 0, 0);
                shownImage = image;
            }
            target.save();
            target.imageSmoothingEnabled = true;
            target.drawImage(canvas, 0, 0, w, h, 0, 0, W, H);
            target.restore();
        },
    };
}

/** Marks drawn in a few batches: each mark's level is rounded to one of `steps`, and each step is one path, filled once. */
class Batches {
    paths: Path2D[];
    constructor(public steps: number) {
        this.paths = Array.from({ length: steps }, () => new Path2D());
    }
    at(level: number): Path2D {
        return this.paths[Math.min(this.steps - 1, Math.max(0, Math.round(level * (this.steps - 1))))];
    }
    fill(ctx: CanvasRenderingContext2D, style: (level: number) => string) {
        this.paths.forEach((path, i) => {
            if (i === 0) return;
            ctx.fillStyle = style(i / (this.steps - 1));
            ctx.fill(path);
        });
    }
}

// ---- Gradient ASCII: a glow rising over a curved horizon, drawn in characters by how bright it is.

export function gradientAscii(o: BackgroundOptions): Scene {
    const ramp = ' =+xX0S8#@';
    const c1 = rgbOf(o.color);
    const c2 = rgbOf(o.color2);
    return {
        draw(ctx, W, H, t) {
            const ch = Math.round(15 / Math.sqrt(o.density));
            const cw = Math.round(ch * 0.62);
            ctx.font = `${Math.round(ch * 0.92)}px "Cascadia Mono", Consolas, monospace`;
            ctx.textBaseline = 'top';
            const aspect = W / H;
            const span = 0.5 * Math.max(aspect, 1);
            const cols = Math.ceil(W / cw);
            const rows = Math.ceil(H / ch);
            const tick = Math.floor(t * 9);
            const batches: { text: string; x: number; y: number }[][] = Array.from({ length: 6 }, () => []);
            for (let row = 0; row < rows; row++) {
                for (let col = 0; col < cols; col++) {
                    let px = ((col + 0.5) / cols - 0.5) * aspect;
                    let py = 0.5 - (row + 0.5) / rows;
                    // A slow warp, so the glow's edge flows.
                    px += (noise2(px * 1.6, py * 1.6 + 0.3 * t * 0.5) - 0.5) * 0.09;
                    py += (noise2(px * 1.6 + 4.3, py * 1.6 - 0.24 * t * 0.5) - 0.5) * 0.09;
                    const arch = -0.17 - 0.2 * px * px + 0.02 * Math.sin(0.6 * t);
                    const edge = py - arch;
                    // Bright at the horizon, fading up into the sky, dark under it.
                    const rim = smoothstep(-0.08, 0.01, edge) * Math.exp(-Math.max(edge, 0) * 7);
                    const sunX = 0.08 * Math.sin(0.31 * t) * span;
                    const sun = Math.exp(-((px - sunX) ** 2 / 0.05 + (py - arch - 0.05) ** 2 / 0.02)) * smoothstep(-0.05, 0.02, edge);
                    const value = clamp(rim * 0.9 + sun * 0.6);
                    if (value < 0.08) continue;
                    const wobble = (hash3(col, row, tick) - 0.5) * 0.12;
                    const tone = clamp(Math.pow(value, 0.75) + wobble);
                    const index = Math.round(tone * (ramp.length - 1));
                    if (index === 0) continue;
                    if (hash3(col, row, 7) > 0.85 * mix(1, 0.3 + 1.1 * tone, 0.55)) continue;
                    batches[Math.min(5, Math.floor(tone * 6))].push({ text: ramp[index], x: col * cw, y: row * ch });
                }
            }
            batches.forEach((list, i) => {
                const k = (i + 0.5) / 6;
                ctx.fillStyle = css(mix(c1[0], c2[0], k), mix(c1[1], c2[1], k), mix(c1[2], c2[2], k), (0.25 + 0.75 * k) * o.intensity);
                for (const g of list) ctx.fillText(g.text, g.x, g.y);
            });
        },
    };
}

/** x to the 2.2, for x from 0 to 1 in 256 steps. */
const GAMMA = Float32Array.from({ length: 256 }, (_, i) => Math.pow(i / 255, 2.2));

// ---- Pattern waves: a grid of dots on a swelling, silk-like surface, lit from one side, so the folds show as light and shadow.

export function patternWaves(o: BackgroundOptions): Scene {
    const c1 = rgbOf(o.color);
    const c2 = rgbOf(o.color2);
    const dir = (20 * Math.PI) / 180;
    const hx = Math.cos(dir);
    const hy = Math.sin(dir);
    const la = dir + Math.PI;
    const L = [0.78 * Math.cos(la), 0.78 * Math.sin(la), 0.62];
    const Hn = (() => {
        const v = [L[0], L[1], L[2] + 1];
        const n = len3(v[0], v[1], v[2]);
        return [v[0] / n, v[1] / n, v[2] / n];
    })();
    let heights = new Float32Array(0);
    return {
        draw(ctx, W, H, t) {
            const pitch = Math.max(8, Math.round(12 / Math.sqrt(o.density)));
            const cols = Math.ceil(W / pitch) + 2;
            const rows = Math.ceil(H / pitch) + 2;
            if (heights.length !== cols * rows) heights = new Float32Array(cols * rows);
            const unit = 520;
            const tt = t * 0.35;
            for (let y = 0; y < rows; y++) {
                for (let x = 0; x < cols; x++) {
                    const px = ((x - 1) * pitch - W / 2) / unit;
                    const py = ((y - 1) * pitch - H / 2) / unit;
                    const u = px * hx + py * hy;
                    const v = -px * hy + py * hx;
                    const bend = 1.7 * (noise2(0.85 * v * 3 + 0.15 * tt, 0.3 * u * 3 - 0.1 * tt) * 2 - 1) + 0.4 * Math.sin(1.6 * v + 0.2 * tt);
                    const phase = 5.5 * u + bend - 0.45 * tt * 3;
                    const swell = 0.6 + 0.4 * (noise2(0.55 * u * 3 + 3 - 0.12 * tt, 0.45 * v * 3 + 0.08 * tt) * 2 - 1);
                    const fold = Math.sin(phase) + 0.32 * Math.sin(2 * phase + 1.3);
                    const ripple = 0.16 * Math.sin(13.75 * u + 1.9 * bend - 0.9 * tt * 3 + 2.1);
                    heights[y * cols + x] = (fold + ripple) * swell;
                }
            }
            const steps = 10;
            const batches = new Batches(steps);
            // The slope across a cell, in the surface's own units, made steeper or flatter by how deep the folds are.
            const scale = (unit / (2 * pitch)) * 0.38;
            for (let y = 1; y < rows - 1; y++) {
                for (let x = 1; x < cols - 1; x++) {
                    const h = heights[y * cols + x];
                    const gx = (heights[y * cols + x + 1] - heights[y * cols + x - 1]) * scale;
                    const gy = (heights[(y + 1) * cols + x] - heights[(y - 1) * cols + x]) * scale;
                    const nl = len3(gx, gy, 1);
                    const n0 = -gx / nl;
                    const n1 = -gy / nl;
                    const n2 = 1 / nl;
                    const diffuse = Math.max(0, n0 * L[0] + n1 * L[1] + n2 * L[2]);
                    // The highlight, to the 160th power, by squaring: x^128 times x^32.
                    const h1 = Math.max(0, n0 * Hn[0] + n1 * Hn[1] + n2 * Hn[2]);
                    const h2 = h1 * h1;
                    const h8 = h2 * h2 * h2 * h2;
                    const h32 = h8 * h8 * h8 * h8;
                    const spec = h32 * h32 * h32 * h32 * h32 * 0.9;
                    const hollow = 0.7 + 0.3 * clamp(0.5 * h + 0.5);
                    let tone = clamp(diffuse * hollow * 0.78 + spec);
                    tone = clamp((tone - 0.42) * 1.25 + 0.42);
                    const level = GAMMA[Math.round(tone * 255)];
                    const cx = (x - 1) * pitch;
                    const cy = (y - 1) * pitch;
                    // Fading out toward the edges.
                    const ex = Math.abs((cx / W) * 2 - 1);
                    const ey = Math.abs((cy / H) * 2 - 1);
                    const edge = 1 - smoothstep(0.5, 1.18, Math.max(ex, ey));
                    const shown = (0.22 + 0.78 * level) * edge;
                    if (shown < 0.1) continue;
                    const r = 0.5 * pitch * 0.95 * Math.max(Math.sqrt(level), 0.14);
                    const path = batches.at(shown);
                    // A dot a pixel or two across looks the same square, and costs a third of a round one.
                    if (r < 1.6) {
                        path.rect(cx - r, cy - r, r * 2, r * 2);
                    } else {
                        path.moveTo(cx + r, cy);
                        path.arc(cx, cy, r, 0, Math.PI * 2);
                    }
                }
            }
            batches.fill(ctx, (k) => css(mix(c1[0], c2[0], k), mix(c1[1], c2[1], k), mix(c1[2], c2[2], k), k * 0.8 * o.intensity));
        },
    };
}

// ---- Slats: rounded slats over a sea seen in perspective; each is lit, and stretched, by the waves passing under it.

const SLAT_TURN = [0, -0.95, 0.78, 1.62];
const SLAT_LEN = [2.6, 1.7, 1.12, 0.76];
const SLAT_SHIFT = [0, 2.3, 4.1, 1.1];
const SLAT_AMP = [1, 0.75, 0.5, 0.3];

export function microSlats(o: BackgroundOptions): Scene {
    const c1 = rgbOf(o.color);
    const c2 = rgbOf(o.color2);
    const dir = (250 * Math.PI) / 180;
    const steep = 0.45 * 0.55;
    const waves = SLAT_TURN.map((turn, i) => {
        const k = (2 * Math.PI) / SLAT_LEN[i];
        return { cos: Math.cos(dir + turn), sin: Math.sin(dir + turn), k, omega: 0.35 * Math.sqrt(9.81 * k), shift: SLAT_SHIFT[i], amp: SLAT_AMP[i] };
    });
    const total = waves.reduce((sum, w) => sum + w.amp * (1 + steep), 0);
    return {
        draw(ctx, W, H, t) {
            const size = 1 / Math.sqrt(o.density);
            const sw = 9 * size;
            const sh = 22 * size;
            const gap = 3 * size;
            const px = sw + gap;
            const py = sh + gap;
            const cols = Math.ceil(W / px);
            const rows = Math.ceil(H / py);
            const ox = (W - cols * px) / 2;
            const oy = (H - rows * py) / 2;
            const tt = t * 0.6;
            const horizon = mix(8, 0.5, 0.55);
            const scale = 1.5;
            const steps = 9;
            const plain = new Batches(steps);
            const lit = new Batches(steps);
            for (let row = 0; row < rows; row++) {
                const cy = oy + row * py + py / 2;
                const v = clamp(cy / H);
                const depth = (1 + horizon) / (v + horizon);
                const haze = mix(1, 1 - 0.55, Math.pow(1 - v, 1.4));
                for (let col = 0; col < cols; col++) {
                    const cx = ox + col * px + px / 2;
                    const wx = (((cx - W / 2) / H) * depth) / scale;
                    const wy = ((depth - 1) * horizon * 2.2) / scale;
                    const meander = 0.55 * Math.sin(0.9 * (wx * 0.23 + wy * 0.41) + 0.13 * tt) + 0.3 * Math.sin(1.4 * (-wx * 0.37 + wy * 0.19) - 0.09 * tt);
                    let height = 0;
                    for (let i = 0; i < 4; i++) {
                        const w = waves[i];
                        const th = w.k * (wx * w.cos + wy * w.sin) - w.omega * tt + w.shift + meander * (0.6 + 0.3 * i);
                        height += w.amp * (Math.cos(th) + steep * Math.cos(2 * th));
                    }
                    const level = clamp(0.5 + (0.5 * height) / (0.85 * total));
                    const crest = smoothstep(0.6, 1, level);
                    const glint = crest * crest * 0.7;
                    const light = clamp((smoothstep(0.3, 0.95, level) + 0.8 * glint) * haze);
                    if (light < 0.05) continue;
                    const len = sh * clamp(mix(0.55, 1, level), 0.08, 1);
                    const x = cx - sw / 2;
                    const y = cy - len / 2;
                    (glint > 0.25 ? lit : plain).at(light).roundRect(x, y, sw, len, Math.min(sw, len) * 0.375);
                }
            }
            plain.fill(ctx, (k) => css(c1[0], c1[1], c1[2], k * 0.7 * o.intensity));
            lit.fill(ctx, (k) => css(mix(c1[0], c2[0], 0.7), mix(c1[1], c2[1], 0.7), mix(c1[2], c2[2], 0.7), k * o.intensity));
        },
    };
}

// ---- Fibers: three ribbons that wave and twist through space, each a sheet of threads of glowing dots flowing along it.

const RIBBONS = [
    { y0: 0, z0: 0, slope: 0, phase: 0 },
    { y0: 1.9, z0: -4.6, slope: -0.08, phase: 2.4 },
    { y0: -1.05, z0: 2.0, slope: 0.12, phase: 4.1 },
];

export function ghostFibers(o: BackgroundOptions): Scene {
    const c1 = rgbOf(o.color);
    const c2 = rgbOf(o.color2);
    const threads = Math.max(6, Math.round(16 * o.density));
    const dots = 120;
    const seeds = Array.from({ length: RIBBONS.length * threads }, (_, i) => hash3(i, 3, 9));
    // The camera's turn: a little yaw and pitch, and a roll.
    const yaw = 0.85 * 0.65;
    const pitch = 0.12;
    const roll = (18 * Math.PI) / 180;
    const rotate = (p: [number, number, number]): [number, number, number] => {
        let [x, y, z] = p;
        let c = Math.cos(yaw);
        let s = Math.sin(yaw);
        [x, z] = [x * c + z * s, -x * s + z * c];
        c = Math.cos(pitch);
        s = Math.sin(pitch);
        [y, z] = [y * c - z * s, y * s + z * c];
        c = Math.cos(roll);
        s = Math.sin(roll);
        [x, y] = [x * c - y * s, x * s + y * c];
        return [x, y, z];
    };
    const focal = 1 / (2 * Math.tan(0.28));
    const curve = (r: (typeof RIBBONS)[number], s: number, t: number): [number, number, number] => {
        const x = 16 * s;
        const y = 0.52 * Math.sin(0.43 * x + 0.31 * t + r.phase) + 0.26 * Math.sin(0.79 * x - 0.23 * t + 1.3 + 0.7 * r.phase) + r.y0 + r.slope * x;
        const z = 0.65 * Math.sin(0.33 * x + 0.19 * t + 0.7 + 1.3 * r.phase) + r.z0;
        return [x, y, z];
    };
    return {
        draw(ctx, W, H, t) {
            const tt = t * 0.5;
            const flow = 12 * 0.4 * tt;
            const twistRoll = tt * 0.15;
            const steps = 8;
            const near = new Batches(steps);
            const far = new Batches(steps);
            ctx.save();
            ctx.globalCompositeOperation = 'lighter';
            RIBBONS.forEach((r, ri) => {
                for (let fi = 0; fi < threads; fi++) {
                    const seed = seeds[ri * threads + fi];
                    const across = (fi / (threads - 1)) * 2 - 1;
                    const layer = (fi % 3) - 1;
                    for (let k = 0; k < dots; k++) {
                        let s = ((k + flow + seed * dots) / dots) % 1;
                        s = s - 0.5;
                        const c = curve(r, s, tt);
                        const c2p = curve(r, s + 0.002, tt);
                        let tx = c2p[0] - c[0];
                        let ty = c2p[1] - c[1];
                        let tz = c2p[2] - c[2];
                        const tl = len3(tx, ty, tz) || 1;
                        tx /= tl;
                        ty /= tl;
                        tz /= tl;
                        // A frame along the curve: n0 is "up" made square to it, b0 the third way.
                        let nx = -ty * tx;
                        let ny = 1 - ty * ty;
                        let nz = -ty * tz;
                        const nl = len3(nx, ny, nz) || 1;
                        nx /= nl;
                        ny /= nl;
                        nz /= nl;
                        const bx = ty * nz - tz * ny;
                        const by = tz * nx - tx * nz;
                        const bz = tx * ny - ty * nx;
                        const theta = 0.483 * 1.2 * c[0] + twistRoll + 0.6 * r.phase + layer * 0.7 * 0.42 + 0.5 * Math.sin(0.26 * c[0] + 0.27 * tt + r.phase);
                        const ct = Math.cos(theta);
                        const st = Math.sin(theta);
                        const half = 0.62 * (0.78 + 0.22 * Math.cos(Math.PI * s));
                        const off = across * half;
                        const lift = layer * 0.7 * 0.16 * half;
                        const p = rotate([
                            c[0] + (ct * nx + st * bx) * off + (-st * nx + ct * bx) * lift,
                            c[1] + (ct * ny + st * by) * off + (-st * ny + ct * by) * lift,
                            c[2] + (ct * nz + st * bz) * off + (-st * nz + ct * bz) * lift,
                        ]);
                        const depth = 7.5 - p[2];
                        if (depth < 0.5) continue;
                        const sx = W / 2 + (p[0] / depth) * focal * H;
                        const sy = H / 2 - (p[1] / depth) * focal * H;
                        if (sx < -4 || sx > W + 4 || sy < -4 || sy > H + 4) continue;
                        const end = 1 - smoothstep(0.36, 0.5, Math.abs(s));
                        const twinkle = Math.pow(Math.max(Math.sin(tt * (1.2 + 2.6 * seed) * 3 + 61 * seed + k), 0), 36) * 0.6;
                        const level = clamp((7.5 / depth) * 0.75 * end * (1 + 2.5 * twinkle));
                        if (level < 0.05) continue;
                        const radius = clamp((1.1 * 7.5) / depth, 0.7, 2.2);
                        const path = (depth < 7.5 ? near : far).at(level);
                        path.moveTo(sx + radius, sy);
                        path.arc(sx, sy, radius, 0, Math.PI * 2);
                    }
                }
            });
            far.fill(ctx, (k) => css(c1[0], c1[1], c1[2], k * 0.55 * o.intensity));
            near.fill(ctx, (k) => css(mix(c1[0], c2[0], 0.6), mix(c1[1], c2[1], 0.6), mix(c1[2], c2[2], 0.6), k * 0.65 * o.intensity));
            ctx.restore();
        },
    };
}

// ---- CRT: a slow plasma on a coarse grid, held to a few levels, that fades as phosphor does, glowing behind scanlines.

export function crtWarp(o: BackgroundOptions): Scene {
    const c1 = rgbOf(o.color);
    let grid: HTMLCanvasElement | null = null;
    let gctx: CanvasRenderingContext2D | null = null;
    let image: ImageData | null = null;
    let values = new Float32Array(0);
    let last = -1;
    return {
        draw(ctx, W, H, t) {
            const cols = Math.round(80 * Math.sqrt(o.density));
            const rows = Math.max(4, Math.round(cols * (H / W)));
            if (!grid || !gctx || !image || grid.width !== cols || grid.height !== rows) {
                grid = document.createElement('canvas');
                grid.width = cols;
                grid.height = rows;
                gctx = grid.getContext('2d')!;
                image = gctx.createImageData(cols, rows);
                values = new Float32Array(cols * rows);
                last = -1;
            }
            const dt = last < 0 ? 1 : clamp(t - last, 0, 1);
            last = t;
            const decay = Math.pow(0.4, dt * 6);
            const tt = t * 0.5;
            const F = 2.5 / 2.2;
            const A = 0.3 / 0.28;
            const levels = 3;
            const data = image.data;
            const [r, g, b] = c1;
            for (let y = 0; y < rows; y++) {
                for (let x = 0; x < cols; x++) {
                    const ux = ((x + 0.5) / cols - 0.5) * F + 0.5;
                    const uy = ((y + 0.5) / rows - 0.5) * F + 0.5;
                    const cx = ux + 0.5 * Math.sin(tt / 2 + 0.4);
                    const cy = uy + 0.5 * Math.cos(tt / 4 + 0.7);
                    const f =
                        0.7 * Math.sin(0.5 * ux + tt / 5 + 1.1) +
                        3.0 * Math.sin(1.6 * uy + tt / 5 + 2.3) +
                        Math.sin(10 * (uy * Math.sin(tt / 2 + 0.6) + ux * Math.cos(tt / 5)) + tt / 2) +
                        0.4 * Math.sin(Math.sqrt(100 * cx * cx + 100 * cy * cy + 1) + tt) +
                        0.9 * Math.sin(Math.sqrt(75 * cx * cx + 25 * cy * cy + 1) + tt) -
                        1.4 * Math.sin(Math.sqrt(256 * cx * cx + 25 * cy * cy + 1) + tt) +
                        0.3 * Math.sin(0.5 * uy + ux + Math.sin(tt));
                    const s = 0.5 + 0.499 * Math.sin(f * A);
                    const level = Math.floor(s * levels) / (levels - 1);
                    const i = y * cols + x;
                    const value = Math.max(level, values[i] * decay - 0.004);
                    values[i] = value;
                    const a = value * value;
                    const k = i * 4;
                    data[k] = r * 255;
                    data[k + 1] = g * 255;
                    data[k + 2] = b * 255;
                    data[k + 3] = a * 255;
                }
            }
            gctx.putImageData(image, 0, 0);
            const flicker = 0.96 + 0.04 * Math.sin(t * 47);
            ctx.save();
            // The glow: the grid stretched soft, wide and faint.
            ctx.imageSmoothingEnabled = true;
            ctx.globalAlpha = 0.22 * o.intensity * flicker;
            ctx.drawImage(grid, 0, 0, cols, rows, -W * 0.01, -H * 0.01, W * 1.02, H * 1.02);
            // The cells themselves, crisp.
            ctx.imageSmoothingEnabled = false;
            ctx.globalAlpha = 0.16 * o.intensity * flicker;
            ctx.drawImage(grid, 0, 0, cols, rows, 0, 0, W, H);
            // Scanlines: every third row of pixels taken out.
            ctx.globalAlpha = 0.55;
            ctx.globalCompositeOperation = 'destination-out';
            ctx.fillStyle = '#000';
            ctx.beginPath();
            for (let y = 0; y < H; y += 3) ctx.rect(0, y, W, 1);
            ctx.fill();
            ctx.restore();
        },
    };
}

// ---- Light pillar: a column of light that twists and leans, its strands flowing upward, one color at its foot and another above.

export function lightPillar(o: BackgroundOptions): Scene {
    const top = rgbOf(o.color);
    const bottom = rgbOf(o.color2);
    const tilt = (25 * Math.PI) / 180;
    const ct = Math.cos(tilt);
    const st = Math.sin(tilt);
    let center = 0;
    return shader(
        o,
        9,
        15,
        (u, v, p, out) => {
            const X0 = (u - center) * 2;
            const Y0 = (0.5 - v) * 2;
            const X = X0 * ct - Y0 * st * 0.35;
            const Y = X0 * st * 0.35 + Y0 * ct;
            const sway = 0.22 * Math.sin(Y * 1.7 + p) + 0.1 * Math.sin(Y * 4.3 - 2 * p);
            const wx = X + sway;
            const width = 0.3 * (1 + 0.15 * Math.sin(Y * 2.6 + p));
            const d = wx / width;
            const core = Math.exp(-d * d);
            const halo = 0.4 / (1 + (d * d) / 6);
            // Strands: bright threads in the column that twist round as they rise.
            const strands = 0.5 + 0.5 * Math.sin(d * 5.5 + Math.sin(Y * 3 - 2 * p) * 2 + Y * 2 - 3 * p);
            const fadeY = smoothstep(1.25, 0.35, Math.abs(Y));
            const light = (core * (0.55 + 0.45 * strands) + halo) * fadeY;
            const e = 1 - Math.exp(-light * 1.6);
            const k = clamp((Y + 1) / 2);
            out[0] = e * mix(bottom[0], top[0], k);
            out[1] = e * mix(bottom[1], top[1], k);
            out[2] = e * mix(bottom[2], top[2], k);
        },
        (aspect) => {
            center = aspect * 0.62;
        },
    );
}

// ---- Aurora: curtains of light hanging from the top, their hems waving, colored along their length.

export function aurora(o: BackgroundOptions): Scene {
    const c1 = rgbOf(o.color);
    const c2 = rgbOf(o.color2);
    const layers = 2;
    // Per layer and column: where the hem is, and how bright the curtain is there (its rays and lobes); and per column, its tint.
    let hems = new Float32Array(0);
    let shine = new Float32Array(0);
    let tints = new Float32Array(0);
    let res = 0;
    return shader(
        o,
        8,
        15,
        (_u, v, _p, out, x) => {
            let glow = 0;
            for (let k = 0; k < layers; k++) {
                const soft = 0.12 * (1 + 0.9 * k);
                const rise = hems[k * res + x] - v;
                // Brightest at the hem, fading up toward the top, as a curtain is.
                const lit = smoothstep(-soft * 0.5, 0.03, rise) * Math.exp(-Math.max(rise, 0) * 4.5);
                glow += lit * shine[k * res + x];
            }
            const tint = tints[x];
            const e = clamp(glow) * 0.85;
            out[0] = e * mix(c1[0], c2[0], tint);
            out[1] = e * mix(c1[1], c2[1], tint);
            out[2] = e * mix(c1[2], c2[2], tint);
        },
        (a, p, w) => {
            res = w;
            if (hems.length !== layers * w) {
                hems = new Float32Array(layers * w);
                shine = new Float32Array(layers * w);
                tints = new Float32Array(w);
            }
            for (let x = 0; x < w; x++) {
                const u = ((x + 0.5) / w) * a;
                tints[x] = clamp(u / a + 0.08 * Math.sin(p));
                for (let k = 0; k < layers; k++) {
                    const seed = 13.7 * k;
                    // Each layer goes round its own circle, the second the other way.
                    const angle = k === 0 ? p : 2.1 - p;
                    const c = Math.cos(angle);
                    const s = Math.sin(angle);
                    const X = u - a / 2;
                    const warp = noise2(X * 0.9 + 0.1 * c + seed, 0.1 * s + seed) * 2 - 1;
                    const wave =
                        0.72 * (noise2(X * 1.5 + 0.35 * warp + 0.14 * c + seed, 0.14 * s - seed) * 2 - 1) +
                        0.28 * (noise2(X * 3.2 + 0.5 * warp + 0.2 * s - seed, 0.2 * c + 0.5 * seed) * 2 - 1);
                    hems[k * w + x] = 0.42 + 0.07 * k + 0.22 * wave * o.density;
                    // Rays: the curtain's folds, as fine streaks up and down it; and slow lobes along it.
                    const rays = 0.55 + 0.45 * noise2(u * 6 + k * 31.7 + 0.6 * c, 0.6 * s + k * 5);
                    const lobes = 0.75 + 0.25 * noise2(u * 3 + 0.4 * s + k * 13.7, 0.4 * c);
                    shine[k * w + x] = rays * lobes * (k === 0 ? 1 : 0.5);
                }
            }
        },
    );
}

// ---- Silk: folds of cloth catching a light that moves along them.

export function silk(o: BackgroundOptions): Scene {
    const base = rgbOf(o.color);
    const sheen = rgbOf(o.color2);
    const L = (() => {
        const v = [-0.45, -0.55, 0.85];
        const n = len3(v[0], v[1], v[2]);
        return [v[0] / n, v[1] / n, v[2] / n];
    })();
    const Hv = (() => {
        const v = [L[0], L[1], L[2] + 1];
        const n = len3(v[0], v[1], v[2]);
        return [v[0] / n, v[1] / n, v[2] / n];
    })();
    let grid = new Float32Array(0);
    let gw = 0;
    // How much a step of one grid pixel is in the pattern's units, across and down, so the slope is the same at any size.
    let sx = 1;
    let sy = 1;
    const pattern = (x: number, y: number, p: number) => {
        y += 0.03 * Math.sin(8 * x - p);
        return 0.6 + 0.4 * Math.sin(5 * (x + y + Math.cos(3 * x + 5 * y) + 0.03 * Math.sin(p)) + Math.sin(20 * (x + y) - 2 * p));
    };
    return shader(
        o,
        8,
        15,
        (_u, _v, _p, out, x, y) => {
            const i = y * gw + x;
            const p = grid[i];
            const nx = -(grid[i + 1] - p) * sx;
            const ny = -(grid[i + gw] - p) * sy;
            const nl = len3(nx, ny, 1);
            const ndl = (nx * L[0] + ny * L[1] + L[2]) / nl;
            const ndh = Math.max(0, (nx * Hv[0] + ny * Hv[1] + Hv[2]) / nl);
            const n = clamp((p - 0.2) / 0.8);
            const relief = 1 + (ndl - L[2]) * 1.8 * 0.5;
            const gloss = Math.pow(ndh, 48) * smoothstep(0.25, 0.85, n);
            const n2 = n * n;
            const shine = (0.85 * gloss + 0.4 * n2 * n2 * n) * 0.32 * (1 + 0.6 * L[2]);
            const shade = (0.08 + 0.92 * n2) * relief * 0.38;
            out[0] = base[0] * shade + sheen[0] * shine;
            out[1] = base[1] * shade + sheen[1] * shine;
            out[2] = base[2] * shade + sheen[2] * shine;
        },
        (aspect, p, w, h) => {
            const d = Math.sqrt(o.density);
            gw = w + 1;
            if (grid.length !== gw * (h + 1)) grid = new Float32Array(gw * (h + 1));
            for (let y = 0; y <= h; y++) {
                const v = ((y + 0.5) / h) * d;
                for (let x = 0; x <= w; x++) grid[y * gw + x] = pattern(((x + 0.5) / w) * aspect * d, v, p);
            }
            // The slope was made for steps of 0.004; a grid pixel is (aspect / w) by (1 / h) of the pattern, times the density.
            sx = (0.004 / ((aspect / w) * d)) * 26 * 0.4;
            sy = (0.004 / ((1 / h) * d)) * 26 * 0.4;
        },
    );
}

// ---- Veil: a dark, slowly folding haze of one color, its brightest parts glowing.

export function darkVeil(o: BackgroundOptions): Scene {
    const c1 = rgbOf(o.color);
    const c2 = rgbOf(o.color2);
    return shader(o, 14, 12, (u, v, p, out) => {
        const c = Math.cos(p);
        const sn = Math.sin(p);
        const s = 1.6 * Math.sqrt(o.density);
        const qx = u * s + 0.05 * sn;
        const qy = v * s + 0.05 * c;
        const wx = fbm(qx + 0.06 * c, qy + 0.06 * sn, 2);
        const wy = fbm(qx + 5.2 - 0.05 * sn, qy + 1.3 + 0.05 * c, 2);
        const f = fbm(qx + 2.2 * wx + 0.04 * c, qy + 2.2 * wy - 0.04 * sn, 3);
        const energy = Math.pow(clamp((f - 0.25) * 1.6), 2.2) * 1.15;
        const light = energy + 0.35 * energy * energy;
        const e = (1 - Math.exp(-light * 1.4)) * 0.8;
        const hot = smoothstep(0.55, 0.95, e);
        out[0] = e * mix(c1[0], c2[0], hot);
        out[1] = e * mix(c1[1], c2[1], hot);
        out[2] = e * mix(c1[2], c2[2], hot);
    });
}

// ---- Grain gradient: a soft gradient across the panel whose bands drift and bend, with a fine grain over it.

export function grainGradient(o: BackgroundOptions): Scene {
    const c1 = rgbOf(o.color);
    const c2 = rgbOf(o.color2);
    const angle = (55 * Math.PI) / 180;
    const dx = Math.cos(angle);
    const dy = Math.sin(angle);
    let grain: HTMLCanvasElement | null = null;
    const field = shader(o, 14, 12, (u, v, p, out) => {
        const c = Math.cos(p);
        const s = Math.sin(p);
        const px = u - 0.5;
        const py = v - 0.5;
        const qx = px + 0.14 * c;
        const qy = py + 0.14 * s;
        const w1x = noise2(0.85 * qx * 2 + 0.04 * c, 0.85 * qy * 2 + 0.03 * s);
        const w1y = noise2(0.85 * qx * 2 + 4.1 - 0.03 * s, 0.85 * qy * 2 + 1.7 - 0.04 * c);
        const w2x = noise2(1.55 * qx * 2 + 1.8 * w1x + 1.3 + 0.02 * c, 1.55 * qy * 2 + 1.8 * w1y + 7.1);
        const w2y = noise2(1.55 * qx * 2 + 1.8 * w1x + 6.4, 1.55 * qy * 2 + 1.8 * w1y + 2.2 - 0.02 * s);
        const flow = 0.8 * o.density;
        const sx = px + ((w1x - 0.5) * 1.1 + (w2x - 0.5) * 0.55) * flow;
        const sy = py + ((w1y - 0.5) * 1.1 + (w2y - 0.5) * 0.55) * flow;
        const raw = clamp((sx * dx + sy * dy) / 1.1 + 0.5);
        // Dark at one end, through the first color, to the second at the other.
        const a = smoothstep(0.15, 0.6, raw);
        const b = smoothstep(0.55, 1, raw);
        const e = a * 0.32;
        out[0] = e * mix(c1[0], c2[0], b);
        out[1] = e * mix(c1[1], c2[1], b);
        out[2] = e * mix(c1[2], c2[2], b);
    });
    return {
        // The grain over it shimmers on its own, but the gradient under it is what's kept, and that loops.
        loop: field.loop,
        pixels: field.pixels,
        compute: field.compute,
        draw(ctx, W, H, t) {
            field.draw(ctx, W, H, t);
            if (!grain) {
                const size = 128;
                grain = document.createElement('canvas');
                grain.width = size;
                grain.height = size;
                const g = grain.getContext('2d')!;
                const image = g.createImageData(size, size);
                for (let i = 0; i < size * size; i++) {
                    const n = hash3(i, 5, 1);
                    image.data[i * 4] = image.data[i * 4 + 1] = image.data[i * 4 + 2] = n > 0.5 ? 255 : 0;
                    image.data[i * 4 + 3] = Math.abs(n - 0.5) * 2 * 255;
                }
                g.putImageData(image, 0, 0);
            }
            // The grain, only over what's drawn, at a new place each frame so it shimmers.
            ctx.save();
            ctx.globalCompositeOperation = 'source-atop';
            ctx.globalAlpha = 0.12;
            const ox = -Math.floor(hash3(Math.floor(t * 24), 1, 2) * 128);
            const oy = -Math.floor(hash3(Math.floor(t * 24), 2, 1) * 128);
            for (let y = oy; y < H; y += 128) for (let x = ox; x < W; x += 128) ctx.drawImage(grain, x, y);
            ctx.restore();
        },
    };
}
