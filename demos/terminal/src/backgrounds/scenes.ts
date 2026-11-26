// The backgrounds that are drawn, not chosen from a file: each is a scene that draws itself on a canvas for a time, in two colors.
// Nothing in them follows the pointer: they're behind what's being worked on, and only move by themselves.
//
// The few that are a color for every pixel (light, fibers, warped glass) are drawn by `shader`, a small grid of pixels worked out
// one by one and stretched over the panel; the rest are dots, lines and characters drawn as such.

import type { Design } from '../studio/effects';
import { aurora, crtWarp, darkVeil, ghostFibers, gradientAscii, grainGradient, lightPillar, microSlats, patternWaves, silk } from './fields';

export type BackgroundKind =
    | 'none' | 'custom' | 'dither' | 'stars' | 'creature' | 'ascii' | 'halftone' | 'blobs' | 'rain'
    | 'horizon' | 'pattern' | 'slats' | 'fibers' | 'crt' | 'pillar' | 'aurora' | 'silk' | 'veil' | 'grain';

export const BACKGROUNDS: { id: BackgroundKind; name: string; hint: string }[] = [
    { id: 'none', name: 'None', hint: 'Just the color' },
    { id: 'dither', name: 'Dither', hint: 'A dotted glow that breathes from the corner' },
    { id: 'stars', name: 'Stars', hint: 'Dots that drift and twinkle' },
    { id: 'creature', name: 'Creature', hint: 'A whale shark of dots swims by' },
    { id: 'ascii', name: 'ASCII', hint: 'A field of characters that ripples' },
    { id: 'halftone', name: 'Halftone', hint: 'Round dots that swell as a wave passes' },
    { id: 'blobs', name: 'Blobs', hint: 'Soft colors that wander' },
    { id: 'rain', name: 'Rain', hint: 'Falling characters' },
    { id: 'horizon', name: 'Gradient ASCII', hint: 'A glow over a horizon, in characters' },
    { id: 'pattern', name: 'Pattern waves', hint: 'Lit dots on a swelling surface' },
    { id: 'slats', name: 'Slats', hint: 'Slats over a sea in perspective' },
    { id: 'fibers', name: 'Fibers', hint: 'Ribbons of glowing dots that twist' },
    { id: 'crt', name: 'CRT', hint: 'A phosphor plasma behind scanlines' },
    { id: 'pillar', name: 'Light pillar', hint: 'A twisting column of light' },
    { id: 'aurora', name: 'Aurora', hint: 'Curtains of light from the top' },
    { id: 'silk', name: 'Silk', hint: 'Folds catching a moving light' },
    { id: 'veil', name: 'Veil', hint: 'A dark haze that folds slowly' },
    { id: 'grain', name: 'Grain gradient', hint: 'A drifting gradient with grain' },
];

/** How a drawn background is set up when it's remixed in the studio: its two colors, its speed and how much of it there is. */
export interface SceneParams {
    color: number;
    color2: number;
    speed: number;
    density: number;
}

/**
 * What the studio made: what it's drawn from, a picture or video (`file`) or a drawn background (`source` and its `scene`), and
 * the effect it's drawn with.
 */
export interface StudioDesign {
    file: string;
    design: Design;
    source?: BackgroundKind | 'file';
    scene?: SceneParams;
}

export interface BackgroundOptions {
    kind: BackgroundKind;
    /** The color it's drawn in, as 0xRRGGBB. */
    color: number;
    /** A second color, for what has two. */
    color2: number;
    /** How strong it is, from 0 to 1. */
    intensity: number;
    /** How fast, 1 is as meant. */
    speed: number;
    /** How much of it there is, 1 is as meant. */
    density: number;
    animate: boolean;
    /** The studio's picture or video and what's done to it, for `custom`, with the scheme's colors it can use. */
    custom?: StudioDesign & { fg: number; accent: number };
}

export const rgba = (c: number, a: number) => `rgba(${(c >> 16) & 255},${(c >> 8) & 255},${c & 255},${Math.max(0, Math.min(1, a)).toFixed(3)})`;

/** A cheap smooth noise from sines: enough for a slow field to look alive. */
export function wave(x: number, y: number, t: number): number {
    return (
        Math.sin(x * 0.9 + t * 0.7) * 0.35 +
        Math.sin(y * 1.1 - t * 0.5) * 0.25 +
        Math.sin((x + y) * 0.6 + t * 0.9) * 0.25 +
        Math.sin(Math.sqrt((x - 3) * (x - 3) + (y - 2) * (y - 2)) * 1.3 - t * 1.1) * 0.15
    );
}

/** The part of a scene a canvas shows, in the scene's own pixels. */
export interface View {
    x: number;
    y: number;
    w: number;
    h: number;
}

export interface Scene {
    /** For a scene that has to load something first: it's drawn again once it has. */
    ready?: Promise<unknown>;
    /** Frames a second, if it isn't the kind's own. */
    fps?: number;
    /** For a scene that's a seamless loop: `count` frames, `fps` a second, the same each time round. */
    loop?: { fps: number; count: number };
    /**
     * For a scene that's worked out as a small picture and stretched over the canvas: that picture, for the scene `w` by `h` at
     * `t` seconds, with its colors not multiplied by alpha, over nothing. What the studio draws from it reads it as it is,
     * without it being drawn and read back.
     */
    pixels?(t: number, w: number, h: number): ImageData | null;
    /** For a scene that loops: frame `index` of it, worked out now, for the worker that works them out ahead (see loops.ts). */
    compute?(index: number, w: number, h: number): { w: number; h: number; data: Uint8ClampedArray };
    /**
     * Draws the scene `w` by `h` at `t` seconds. Nothing in it follows the pointer: it's behind what's being worked on. `view`
     * is the part of it the canvas shows, when that isn't all of it, for what can leave the rest out.
     */
    draw(ctx: CanvasRenderingContext2D, w: number, h: number, t: number, view?: View): void;
}

function stars(o: BackgroundOptions, w: number, h: number): Scene {
    const n = Math.round((w * h) / 9000 * o.density);
    const list = Array.from({ length: n }, () => ({
        x: Math.random() * w, y: Math.random() * h, r: Math.random() < 0.85 ? 1 : 1.6,
        vx: (Math.random() - 0.5) * 4, vy: (Math.random() - 0.5) * 3, phase: Math.random() * 6.28, rate: 0.5 + Math.random() * 1.5,
    }));
    return {
        draw(ctx, W, H, t) {
            for (const s of list) {
                const x = (((s.x + s.vx * t) % W) + W) % W;
                const y = (((s.y + s.vy * t) % H) + H) % H;
                const a = (0.25 + 0.75 * (0.5 + 0.5 * Math.sin(s.phase + t * s.rate))) * o.intensity;
                ctx.fillStyle = rgba(o.color, a);
                ctx.fillRect(x, y, s.r * 1.5, s.r * 1.5);
            }
        },
    };
}

/** A whale shark made of dots, swimming a slow loop. Its spine is where its head was a moment ago, and a moment before that, so the
 * body follows the head round each turn. */
function creature(o: BackgroundOptions, w: number, h: number): Scene {
    const field = stars({ ...o, density: o.density * 0.35, intensity: o.intensity * 0.6 }, w, h);
    const SEGMENTS = 64;
    const length = Math.min(w * 0.5, h * 0.75);
    const headAt = (t: number) => ({
        x: w * 0.5 + Math.cos(t * 0.11) * w * 0.32 + Math.sin(t * 0.23) * w * 0.05,
        y: h * 0.55 + Math.sin(t * 0.17) * h * 0.2,
    });
    // The half-width of the body from the head (0) to the tail (1): a broad flat head, the pectoral fins, a thin stalk, the tail.
    const width = (s: number) => {
        let base = s < 0.12 ? 0.06 + (s / 0.12) * 0.03 : 0.09 * Math.pow(1 - (s - 0.12) / 0.88, 1.4) + 0.008;
        if (s > 0.2 && s < 0.38) base += Math.sin(((s - 0.2) / 0.18) * Math.PI) * 0.09;
        if (s > 0.86) base += Math.sin(((s - 0.86) / 0.14) * Math.PI) * 0.07;
        return base * length;
    };
    return {
        draw(ctx, W, H, t) {
            field.draw(ctx, W, H, t);
            const a = headAt(t);
            const b = headAt(t - 0.2);
            const speed = Math.max(15, Math.hypot(a.x - b.x, a.y - b.y) / 0.2);
            const gapTime = length / SEGMENTS / speed;
            const spine = [];
            for (let i = 0; i <= SEGMENTS; i++) spine.push(headAt(t - i * gapTime));
            const dot = Math.max(1.4, (length / SEGMENTS) * 0.38);
            for (let i = 0; i < SEGMENTS; i++) {
                const p = spine[i];
                const q = spine[i + 1];
                const dx = p.x - q.x;
                const dy = p.y - q.y;
                const len = Math.hypot(dx, dy) || 1;
                const nx = -dy / len;
                const ny = dx / len;
                const s = i / SEGMENTS;
                const half = width(s);
                const across = Math.max(1, Math.round(half / (length / SEGMENTS)));
                // A glow that runs down the body, as if it breathed.
                const glow = 0.6 + 0.4 * Math.sin(t * 2.4 - s * 10);
                for (let j = -across; j <= across; j++) {
                    const off = (j / across) * half;
                    const centre = 1 - Math.abs(j / across);
                    const alpha = (0.32 + 0.68 * centre * glow) * o.intensity * (1 - s * 0.3);
                    ctx.fillStyle = rgba(centre > 0.6 ? o.color2 : o.color, alpha);
                    ctx.beginPath();
                    ctx.arc(p.x + nx * off, p.y + ny * off, dot * (0.55 + 0.45 * centre), 0, Math.PI * 2);
                    ctx.fill();
                }
            }
        },
    };
}

function ascii(o: BackgroundOptions, w: number, h: number): Scene {
    const ramp = ' .·:-=+*#%@';
    const cell = Math.round(16 / Math.sqrt(o.density));
    return {
        draw(ctx, W, H, t) {
            ctx.font = `${Math.round(cell * 0.82)}px "Cascadia Mono", Consolas, monospace`;
            ctx.textBaseline = 'top';
            const cols = Math.ceil(W / cell);
            const rows = Math.ceil(H / cell);
            for (let y = 0; y < rows; y++) {
                for (let x = 0; x < cols; x++) {
                    const v = 0.5 + 0.5 * wave(x * 0.09, y * 0.12, t * 0.6);
                    if (v < 0.42) continue;
                    const index = Math.min(ramp.length - 1, Math.floor(((v - 0.42) / 0.58) * ramp.length));
                    const ch = ramp[index];
                    if (ch === ' ') continue;
                    ctx.fillStyle = rgba(v > 0.85 ? o.color2 : o.color, (0.18 + v * 0.55) * o.intensity);
                    ctx.fillText(ch, x * cell, y * cell);
                }
            }
            void w;
            void h;
        },
    };
}

function halftone(o: BackgroundOptions): Scene {
    return {
        draw(ctx, W, H, t) {
            const gap = Math.round(20 / Math.sqrt(o.density));
            // A slow swell that wanders, as a hand would.
            const fx = W * (0.5 + 0.35 * Math.sin(t * 0.21));
            const fy = H * (0.5 + 0.3 * Math.cos(t * 0.17));
            for (let y = gap / 2; y < H; y += gap) {
                for (let x = gap / 2; x < W; x += gap) {
                    const v = Math.max(0.5 + 0.5 * wave(x / 140, y / 140, t * 0.5), 1 - Math.sqrt((x - fx) * (x - fx) + (y - fy) * (y - fy)) / 220);
                    const r = v * gap * 0.26;
                    if (r < 0.6) continue;
                    ctx.fillStyle = rgba(v > 0.85 ? o.color2 : o.color, (0.06 + v * 0.32) * o.intensity);
                    ctx.beginPath();
                    ctx.arc(x, y, r, 0, Math.PI * 2);
                    ctx.fill();
                }
            }
        },
    };
}

function blobs(o: BackgroundOptions): Scene {
    const seeds = [0, 1.7, 3.1, 4.6].map((p, i) => ({ p, c: i % 2 === 0 ? o.color : o.color2, r: 0.45 + i * 0.08 }));
    return {
        draw(ctx, W, H, t) {
            for (const s of seeds) {
                const x = W * (0.5 + 0.38 * Math.sin(t * 0.13 + s.p));
                const y = H * (0.5 + 0.34 * Math.cos(t * 0.11 + s.p * 1.3));
                const r = Math.max(W, H) * s.r;
                const g = ctx.createRadialGradient(x, y, 0, x, y, r);
                g.addColorStop(0, rgba(s.c, 0.55 * o.intensity));
                g.addColorStop(1, rgba(s.c, 0));
                ctx.fillStyle = g;
                ctx.fillRect(0, 0, W, H);
            }
        },
    };
}

function rain(o: BackgroundOptions, w: number): Scene {
    const cell = 16;
    const glyphs = 'アイウエオカキクケコサシスセソ0123456789ﾊﾋﾌﾍﾎ:.=*+-<>';
    const columns = Array.from({ length: Math.ceil(w / cell) }, () => ({
        y: Math.random() * -40, speed: 4 + Math.random() * 8, length: 6 + Math.floor(Math.random() * 18), on: Math.random() < 0.55 * o.density,
    }));
    let last = 0;
    return {
        draw(ctx, W, H, t) {
            const dt = Math.min(0.1, t - last);
            last = t;
            ctx.font = `${cell - 2}px "Cascadia Mono", Consolas, monospace`;
            ctx.textBaseline = 'top';
            columns.forEach((c, i) => {
                if (!c.on) return;
                c.y += c.speed * dt;
                if ((c.y - c.length) * cell > H) {
                    c.y = Math.random() * -20;
                    c.speed = 4 + Math.random() * 8;
                }
                for (let k = 0; k < c.length; k++) {
                    const row = Math.floor(c.y) - k;
                    if (row < 0 || row * cell > H) continue;
                    const ch = glyphs[(row * 7 + i * 13 + Math.floor(t * 3) * (k === 0 ? 1 : 0)) % glyphs.length];
                    ctx.fillStyle = rgba(k === 0 ? o.color2 : o.color, (k === 0 ? 0.95 : 0.7 * (1 - k / c.length)) * o.intensity);
                    ctx.fillText(ch, i * cell, row * cell);
                }
            });
            void W;
        },
    };
}

/** Ordered dithering's threshold for a cell: a 4×4 Bayer matrix, so the dots thin out in an even weave, not at random. */
const BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5].map((v) => (v + 0.5) / 16);

/**
 * A glow of tiny square dots in the top left corner, thinning out across the panel, as print halftone does. It breathes, and slow
 * waves run through it, so its edge crawls as the dots there come and go.
 */
function dither(o: BackgroundOptions): Scene {
    const STEP = 4;
    const SIZE = 2;
    // The glow and its ripple are smooth over many pixels, so the level they make is worked out every NODE pixels, and the canvas
    // reads between: what's worked out for each of the tens of thousands of dots, one at a time, is what made it slow.
    const NODE = 32;
    // How far out anything can show: the ripple is never more than 0.16, and the lowest threshold is 0.03.
    const REACH = 1.15;
    let field = new Float32Array(0);
    let view = field;
    return {
        draw(ctx, W, H, t) {
            const breath = 1 + 0.07 * Math.sin(t * 0.35);
            const rx = W * (0.55 + 0.25 * o.density) * breath;
            const ry = H * (0.4 + 0.2 * o.density) * (2 - breath);
            const cols = Math.ceil((rx * REACH) / NODE) + 1;
            const rows = Math.ceil((ry * REACH) / NODE) + 1;
            if (view.length !== cols * rows) {
                if (field.length < cols * rows) field = new Float32Array(cols * rows);
                view = field.subarray(0, cols * rows);
            }
            for (let j = 0, k = 0; j < rows; j++) {
                const y = j * NODE;
                const uy = y / ry;
                const yy = uy * uy;
                for (let i = 0; i < cols; i++, k++) {
                    const x = i * NODE;
                    const ux = x / rx;
                    // How much shows there: 1 less the distance from the corner, and the ripple, the way thresholds are set.
                    const g = (1 - Math.sqrt(ux * ux + yy) + 0.16 * wave(x / 90, y / 90, t * 0.8)) * 1.6;
                    view[k] = g < 0 ? 0 : g > 2 ? 2 : g;
                }
            }
            ctx.fillStyle = rgba(o.color, 0.16 * o.intensity);
            // Dots where the level is over their place in a 4 by 4 Bayer matrix, so they thin out in an even weave; every other
            // row is shifted half a step, which weaves them diagonally.
            if (typeof ctx.fillDither === 'function') {
                ctx.fillDither(view, cols, rows, NODE, STEP, SIZE);
                return;
            }
            ctx.beginPath();
            for (let y = 0, row = 0; y <= (rows - 1) * NODE; y += STEP, row++) {
                const j = Math.min(rows - 2, Math.floor(y / NODE));
                const fy = y / NODE - j;
                for (let x = row % 2 === 0 ? 0 : STEP / 2, col = 0; x <= (cols - 1) * NODE; x += STEP, col++) {
                    const i = Math.min(cols - 2, Math.floor(x / NODE));
                    const fx = x / NODE - i;
                    const a = view[j * cols + i] * (1 - fx) + view[j * cols + i + 1] * fx;
                    const b = view[(j + 1) * cols + i] * (1 - fx) + view[(j + 1) * cols + i + 1] * fx;
                    if (a + (b - a) * fy > BAYER[(row % 4) * 4 + (col % 4)]) ctx.rect(x, y, SIZE, SIZE);
                }
            }
            ctx.fill();
        },
    };
}

/** The scene for a kind that's drawn, `w` by `h`, or null for none and for the studio's own, which isn't one of these. */
export function sceneFor(o: BackgroundOptions, w: number, h: number): Scene | null {
    switch (o.kind) {
        case 'dither': return dither(o);
        case 'stars': return stars(o, w, h);
        case 'creature': return creature(o, w, h);
        case 'ascii': return ascii(o, w, h);
        case 'halftone': return halftone(o);
        case 'blobs': return blobs(o);
        case 'rain': return rain(o, w);
        case 'horizon': return gradientAscii(o);
        case 'pattern': return patternWaves(o);
        case 'slats': return microSlats(o);
        case 'fibers': return ghostFibers(o);
        case 'crt': return crtWarp(o);
        case 'pillar': return lightPillar(o);
        case 'aurora': return aurora(o);
        case 'silk': return silk(o);
        case 'veil': return darkVeil(o);
        case 'grain': return grainGradient(o);
        default: return null;
    }
}

/** How often each is drawn, a second: the slow ones need few frames to look smooth. */
export const FPS: Record<BackgroundKind, number> = {
    none: 0, custom: 0, dither: 12, stars: 24, creature: 30, ascii: 12, halftone: 20, blobs: 15, rain: 20,
    horizon: 12, pattern: 10, slats: 20, fibers: 20, crt: 15, pillar: 15, aurora: 15, silk: 15, veil: 12, grain: 12,
};
/** What fraction of the window's pixels each is drawn at; the soft ones look the same at less. */
export const QUALITY: Record<BackgroundKind, number> = {
    none: 1, custom: 1, dither: 1, stars: 1, creature: 1, ascii: 1, halftone: 1, blobs: 0.25, rain: 1,
    horizon: 1, pattern: 1, slats: 1, fibers: 1, crt: 1, pillar: 1 / 9, aurora: 1 / 8, silk: 1 / 8, veil: 1 / 14, grain: 1 / 14,
};
