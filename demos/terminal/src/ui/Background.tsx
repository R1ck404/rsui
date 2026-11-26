// What's drawn behind terminals and tool pages: nothing, one of the scenes in backgrounds/scenes.ts, or what was made in the studio, on a
// canvas of its own. Each is drawn at a modest frame rate, and stops while the window can't be seen.

import { useEffect, useMemo, useRef } from 'react';
import { FPS, QUALITY, sceneFor } from '../backgrounds/scenes';
import type { BackgroundKind, BackgroundOptions, Scene, StudioDesign, View } from '../backgrounds/scenes';
import { DEFAULT_SCENE, drawDesign, mediaSource, sceneSource } from '../backgrounds/source';
import type { FrameSource } from '../backgrounds/source';
import { load } from '../studio/media';
import type { Media } from '../studio/media';

export { BACKGROUNDS } from '../backgrounds/scenes';
export type { BackgroundKind, BackgroundOptions } from '../backgrounds/scenes';

/** Studio backgrounds with the same design share the source and its cached frames. */
const shared = new Map<string, { scene: Scene; users: number }>();

function sharedCustom(o: BackgroundOptions): { scene: Scene | null; release: () => void } {
    const key = JSON.stringify([o.custom ?? null, o.animate]);
    let entry = shared.get(key);
    if (!entry) {
        const scene = custom(o);
        if (!scene) return { scene: null, release: () => {} };
        entry = { scene, users: 0 };
        shared.set(key, entry);
    }
    entry.users++;
    const held = entry;
    return {
        scene: held.scene,
        release: () => {
            held.users--;
            if (held.users <= 0 && shared.get(key) === held) shared.delete(key);
        },
    };
}

/** What the studio made: its picture, the frame of its video, or a drawn background, with the effect drawn from it. */
function custom(o: BackgroundOptions): Scene | null {
    const c = o.custom;
    if (!c) return null;
    const palette = { fg: c.fg, accent: c.accent };
    if (c.source && c.source !== 'file') {
        const source = sceneSource(c.source, { ...DEFAULT_SCENE, ...(c.scene ?? {}) }, o.animate);
        return { fps: source.fps, draw: (ctx, W, H, t, view?: View) => drawDesign(ctx, source, c.design, W, H, palette, t, view) };
    }
    let source: FrameSource | null = null;
    const scene: Scene = {
        ready: load(c.file).then((loaded: Media) => {
            source = mediaSource(loaded, c.design.speed);
            scene.fps = source.fps;
        }),
        fps: 0,
        draw(ctx, W, H, t, view?: View) {
            if (source) drawDesign(ctx, source, c.design, W, H, palette, t, view);
        },
    };
    return scene;
}

/** What the settings say the background is, in the scheme's colors or a pair chosen by name. */
export function backgroundOptions(
    settings: { background: BackgroundKind; backgroundColor: string; backgroundIntensity: number; backgroundSpeed: number; backgroundDensity: number; animations: boolean; studio?: StudioDesign | null },
    scheme: { fg: number; ansi: number[] },
): BackgroundOptions {
    const pairs: Record<string, [number, number]> = {
        blue: [0x3b6cff, 0xbcd2ff],
        purple: [0x8b5cf6, 0xf0abfc],
        green: [0x22c55e, 0xbbf7d0],
        warm: [0xf97316, 0xfde68a],
        mono: [scheme.fg, scheme.fg],
    };
    let [color, color2] = pairs[settings.backgroundColor] ?? [scheme.ansi[4], scheme.ansi[12] ?? scheme.fg];
    if (settings.background === 'dither' && settings.backgroundColor === 'scheme') {
        // The scheme's text, cooled a little toward its blue: a quiet grey that reads as light, not as a color.
        const k = 0.5;
        const channel = (shift: number) => Math.round(((scheme.fg >> shift) & 255) * (1 - k) + ((scheme.ansi[4] >> shift) & 255) * k);
        color = (channel(16) << 16) | (channel(8) << 8) | channel(0);
        color2 = color;
    }
    return {
        kind: settings.background,
        color,
        color2,
        intensity: settings.backgroundIntensity,
        speed: settings.backgroundSpeed,
        density: settings.backgroundDensity,
        animate: settings.animations,
        custom: settings.studio ? { ...settings.studio, fg: scheme.fg, accent: scheme.ansi[4] } : undefined,
    };
}

export function Background(props: { options: BackgroundOptions; outline?: string; time?: number; paused?: boolean }) {
    const outline = useMemo(() => props.outline ? new Path2D(props.outline) : null, [props.outline]);
    const clip = useRef(outline);
    clip.current = outline;
    const paintedOutline = useRef(outline);
    const canvas = useRef<HTMLCanvasElement>(null);
    const box = useRef<HTMLDivElement>(null);
    const latest = useRef(props.options);
    latest.current = props.options;
    const paused = useRef(props.paused);
    paused.current = props.paused;
    const wasPaused = useRef(props.paused);
    const time = useRef(props.time);
    time.current = props.time;
    const controlled = props.time !== undefined;
    const o = props.options;
    const repaint = useRef<(() => void) | null>(null);
    const resize = useRef<(() => boolean) | null>(null);

    useEffect(() => {
        const element = canvas.current;
        const holder = box.current;
        if (!element || !holder || o.kind === 'none') return;
        const ctx = element.getContext('2d');
        if (!ctx) return;
        const studio = o.kind === 'custom' ? sharedCustom(o) : null;
        let scene = studio?.scene ?? null;
        let size = { w: 0, h: 0 };
        const quality = QUALITY[o.kind];
        const start = Date.now();
        let timer: ReturnType<typeof setTimeout> | null = null;
        let resizeTimer: ReturnType<typeof setTimeout> | null = null;
        let stopped = false;
        const canDraw = () => !paused.current && (controlled || !document.hidden);
        const frame = () => {
            if (stopped || !scene) return;
            if (timer !== null) clearTimeout(timer);
            timer = null;
            if (!canDraw() || resizeTimer !== null) return;
            const now = latest.current;
            const t = (controlled ? now.animate ? time.current ?? 0 : 0 : (Date.now() - start) / 1000) * now.speed;
            ctx.setTransform(quality, 0, 0, quality, 0, 0);
            ctx.clearRect(0, 0, size.w, size.h);
            const shape = clip.current;
            if (shape) {
                ctx.save();
                ctx.clip(shape);
            }
            const began = performance.now();
            scene.draw(ctx, size.w, size.h, t);
            if (shape) ctx.restore();
            paintedOutline.current = shape;
            const fps = scene.fps ?? FPS[o.kind];
            if (controlled || !now.animate || !fps) return;
            // A frame that's slow (a big window, a busy machine) is followed by a longer wait, so that a background never holds
            // the thread for more than about a sixth of the time, whatever it costs: what's being typed gets the rest.
            const cost = performance.now() - began;
            timer = setTimeout(frame, Math.min(1000, Math.max(1000 / fps, cost * 5)));
        };
        const dimensions = () => {
            const r = holder.getBoundingClientRect();
            return { w: Math.max(1, Math.round(r.width)), h: Math.max(1, Math.round(r.height)) };
        };
        const fit = () => {
            if (!canDraw()) return false;
            const { w, h } = dimensions();
            if (w === size.w && h === size.h) return false;
            size = { w, h };
            element.width = Math.max(1, Math.round(w * quality));
            element.height = Math.max(1, Math.round(h * quality));
            if (!studio) scene = sceneFor(latest.current, w, h);
            frame();
            return true;
        };
        const scheduleFit = () => {
            if (!canDraw()) return;
            const { w, h } = dimensions();
            if (resizeTimer !== null) clearTimeout(resizeTimer);
            resizeTimer = null;
            if (w === size.w && h === size.h) {
                // Wake the animation if a pending resize returned to the original size.
                if (timer === null && latest.current.animate) frame();
                return;
            }
            // While the window is being dragged, let CSS stretch the last frame across the panel.
            resizeTimer = setTimeout(() => {
                resizeTimer = null;
                if (!fit()) frame();
            }, 80);
        };
        fit();
        resize.current = fit;
        repaint.current = () => {
            const { w, h } = dimensions();
            if (w !== size.w || h !== size.h) scheduleFit();
            else frame();
        };
        const observer = new ResizeObserver(scheduleFit);
        observer.observe(holder);
        const visibilityChanged = () => {
            if (!fit()) frame();
        };
        document.addEventListener('visibilitychange', visibilityChanged);
        // A scene that loads draws again when it has, and starts moving then if it moves.
        studio?.scene?.ready?.then(frame, () => {});
        return () => {
            stopped = true;
            if (timer !== null) clearTimeout(timer);
            if (resizeTimer !== null) clearTimeout(resizeTimer);
            observer.disconnect();
            document.removeEventListener('visibilitychange', visibilityChanged);
            resize.current = null;
            repaint.current = null;
            studio?.release();
        };
    }, [controlled, o.kind, o.density, o.animate, o.color, o.color2, o.kind === 'dither' ? o.intensity : 0, o.custom ? JSON.stringify(o.custom) : '']);

    useEffect(() => {
        if (outline !== paintedOutline.current || (controlled && o.animate)) repaint.current?.();
    }, [outline, props.time]);

    useEffect(() => {
        const resume = wasPaused.current && !props.paused;
        wasPaused.current = props.paused;
        if (resume && !resize.current?.()) repaint.current?.();
    }, [props.paused]);

    if (o.kind === 'none') return null;
    return (
        <div ref={box} className="pointer-events-none absolute inset-0 overflow-hidden">
            <canvas ref={canvas} style={{ width: '100%', height: '100%', display: 'block' }} />
        </div>
    );
}
