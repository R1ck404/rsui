// What the studio draws from: a picture, a video, or one of the backgrounds that are drawn (stars, light pillar, CRT…), so any of
// them can be remixed: turned into ASCII, dithered, halftoned. A drawn background that loops is worked out ahead on a worker (see
// loops.ts); one that doesn't is drawn small on a canvas of its own, black behind it, and read back as a frame, which the effect
// then samples as it would a video's.

import { cellPixels, cellsOf, drawGrid, fadeOut, render } from '../studio/effects';
import type { Cells, Design, Frame, Palette, View } from '../studio/effects';
import { compactPixels, frameAt, readCells, remixStore } from './loops';
import type { LoopCells, RemixSpec } from './loops';
import type { Media } from '../studio/media';
import { FPS, sceneFor } from './scenes';
import type { BackgroundKind, BackgroundOptions, Scene, SceneParams } from './scenes';

export const DEFAULT_SCENE: SceneParams = { color: 0x82aaff, color2: 0xc3a6ff, speed: 1, density: 1 };

export interface FrameSource {
    /** How many frames a second it changes at, 0 for a still. */
    fps: number;
    /** What it shows `t` seconds in, for a canvas `W` by `H`. */
    frameAt(t: number, W: number, H: number): Frame;
    /** Draws it on the canvas as it is, sharper than its frame, for the `raw` effect. */
    direct?(ctx: CanvasRenderingContext2D, W: number, H: number, t: number): void;
    /**
     * For a drawn background that loops: the cells a design samples from its frame for `t`, worked out ahead on the worker and
     * kept (see loops.ts), or the nearest ready before them; `null` until the first are. `undefined` for one that doesn't loop.
     */
    loopCells?(t: number, W: number, H: number, remix: RemixSpec): ReturnType<typeof readCells> | null | undefined;
}

export function mediaSource(media: Media, speed: number): FrameSource {
    const moving = media.count > 1;
    return {
        fps: moving ? media.fps * Math.max(0.1, speed) : 0,
        frameAt: (t) => media.frameAt(moving ? Math.floor(t * media.fps * speed) : 0),
    };
}

/** The options a drawn background is drawn with in the studio: at full strength, the effect's own strength does the rest. */
export function sceneOptions(kind: BackgroundKind, p: SceneParams): BackgroundOptions {
    return { kind, color: p.color, color2: p.color2, intensity: 1, speed: p.speed, density: p.density, animate: true };
}

/** A drawn background as a source, read back at most `longest` pixels along its longer side. */
export function sceneSource(kind: BackgroundKind, p: SceneParams, animate = true): FrameSource {
    const longest = 480;
    const o = { ...sceneOptions(kind, p), animate };
    let canvas: HTMLCanvasElement | null = null;
    let ctx: CanvasRenderingContext2D | null = null;
    let scene: Scene | null = null;
    let sized = '';
    // The full-size scene `direct` draws, made for the size it's drawn at.
    let full: Scene | null = null;
    let fullSized = '';
    let shown: LoopCells | null = null;
    let ready: ReturnType<typeof readCells> | null = null;
    const ensure = (W: number, H: number) => {
        if (sized !== `${W}x${H}`) {
            scene = sceneFor(o, W, H);
            sized = `${W}x${H}`;
        }
    };
    return {
        fps: FPS[kind] || 12,
        loopCells(t, W, H, remix) {
            ensure(W, H);
            const loop = scene?.loop;
            const compute = scene?.compute;
            if (!loop || !compute) return undefined;
            const index = ((Math.floor(t * p.speed * loop.fps) % loop.count) + loop.count) % loop.count;
            let fresh: Cells | Uint8ClampedArray | undefined;
            const frame = frameAt(remixStore(o, W, H, loop.count, remix), index, () => {
                const cells = cellsOf(compute(index, W, H), remix.design, W, H);
                const pixels = cellPixels(cells, remix.design, W, H, remix.palette);
                fresh = pixels ?? cells;
                return { data: pixels ? compactPixels(pixels) : cells };
            }, animate);
            if (!frame) return null;
            if (frame !== shown) {
                shown = frame;
                ready = fresh ?? readCells(frame);
            }
            return ready;
        },
        frameAt(t, W, H) {
            ensure(W, H);
            // A scene that's a small picture already: that, as it is, with no canvas drawn and nothing read back.
            const own = scene?.pixels?.(t * p.speed, W, H);
            if (own) return { w: own.width, h: own.height, data: own.data };
            const k = Math.min(1, longest / Math.max(W, H));
            const w = Math.max(1, Math.round(W * k));
            const h = Math.max(1, Math.round(H * k));
            if (!canvas || canvas.width !== w || canvas.height !== h) {
                canvas = document.createElement('canvas');
                canvas.width = w;
                canvas.height = h;
                ctx = canvas.getContext('2d');
            }
            if (!ctx) return { w: 1, h: 1, data: new Uint8ClampedArray(4) };
            ctx.setTransform(1, 0, 0, 1, 0, 0);
            ctx.globalAlpha = 1;
            ctx.fillStyle = '#000';
            ctx.fillRect(0, 0, w, h);
            ctx.setTransform(k, 0, 0, k, 0, 0);
            scene?.draw(ctx, W, H, t * p.speed);
            const image = ctx.getImageData(0, 0, w, h);
            return { w, h, data: image.data };
        },
        direct(target, W, H, t) {
            if (fullSized !== `${W}x${H}`) {
                full = sceneFor(o, W, H);
                fullSized = `${W}x${H}`;
            }
            full?.draw(target, W, H, t * p.speed);
        },
    };
}

/**
 * Draws a design from a source: the effect over its frame, or, for `raw` from a drawn background, the background itself. A drawn
 * background that loops has its cells (and for effects drawn as a picture, their pixels) worked out ahead on a worker and kept,
 * so drawing them is all that's left: no frame drawn, read back or sampled while the window draws. `view` is the part of it the
 * canvas shows, when that isn't all.
 */
export function drawDesign(ctx: CanvasRenderingContext2D, source: FrameSource, d: Design, W: number, H: number, palette: Palette, t: number, view?: View) {
    if (d.effect === 'raw' && source.direct) {
        ctx.save();
        ctx.globalAlpha = d.opacity;
        source.direct(ctx, W, H, t);
        ctx.restore();
        fadeOut(ctx, d.fade, W, H);
        return;
    }
    const looped = d.effect === 'raw' ? undefined : source.loopCells?.(t, W, H, { design: d, palette });
    if (looped === undefined) {
        render(ctx, source.frameAt(t, W, H), d, W, H, palette, t, view);
        return;
    }
    if (looped) drawGrid(ctx, looped, d, W, H, palette, t, view);
}
