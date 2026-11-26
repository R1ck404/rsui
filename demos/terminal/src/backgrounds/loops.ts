// The frames of the backgrounds that loop (see `shader` in fields.ts), and of the studio's remixes of them, worked out ahead on a
// worker of their own and kept, shared by every canvas that shows the same one. Drawing then only shows what's ready: the work of
// a frame (a few milliseconds of maths for each of thousands of pixels) never lands in the frame the window draws, so a background
// costs next to nothing from its first moment, not only once it's been round its loop.
//
// The worker is asked for a loop from the frame being shown on, and sends each frame as it has it; until a frame comes, drawing
// shows the nearest one before it that has. With no worker (one that can't start, or fails), frames are worked out as they're
// needed, as before.

import type { BackgroundOptions } from './scenes';
import type { Cells, Design, Palette } from '../studio/effects';
import { deflateSync, inflateSync } from 'zlib';

/** A frame of a loop as it's sent: its pixels, not multiplied by alpha. */
export interface LoopPixels {
    w: number;
    h: number;
    data: Uint8ClampedArray;
}

/** The representation the remix actually draws. */
export interface LoopCells {
    data: Cells | Uint8ClampedArray | { compressed: Uint8Array; length: number };
}

/** The worker can store exact rendered pixels in a smaller form. */
export function compactPixels(pixels: Uint8ClampedArray): LoopCells['data'] {
    if (pixels.byteLength < 8192) return pixels;
    const bytes = deflateSync(pixels, { level: 1 });
    if (bytes.byteLength >= pixels.byteLength * 0.8) return pixels;
    return { compressed: new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength), length: pixels.byteLength };
}

export function readCells(frame: LoopCells): Cells | Uint8ClampedArray {
    const data = frame.data;
    if (!('compressed' in data)) return data;
    const bytes = inflateSync(data.compressed, { maxOutputLength: data.length });
    return new Uint8ClampedArray(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}

/** A remix's design and colors, which the worker needs to make cells. */
export interface RemixSpec {
    design: Design;
    palette: Palette;
}

interface Store<T> {
    key: string;
    count: number;
    frames: (T | null)[];
    bytes: number;
    usedAt: number;
    /** Which job the worker's on for it, if it's been asked. */
    job: number;
    /** What it's for, to ask the worker with. */
    ask: { o: BackgroundOptions; W: number; H: number; remix: RemixSpec | null };
}

/** The stores kept, bounded by count and by bytes of inactive frames. */
const stores = new Map<string, Store<ImageData> | Store<LoopCells>>();
const MOST = 6;
const CACHE_BYTES = 8 * 1024 * 1024;
const RECENT_MS = 1_200;
const WORKER_IDLE_MS = 10_000;
let cachedBytes = 0;
const jobs = new Map<number, Store<ImageData> | Store<LoopCells>>();
let nextJob = 1;
/** The worker: not tried yet, running, or none (it couldn't start, or failed). */
let worker: Worker | null | undefined;
let retireTimer: ReturnType<typeof setTimeout> | undefined;

function retireWhenDone() {
    if (jobs.size || !worker || retireTimer !== undefined) return;
    retireTimer = setTimeout(() => {
        retireTimer = undefined;
        if (!jobs.size) {
            worker?.terminate();
            worker = undefined;
        }
    }, WORKER_IDLE_MS);
}

function cancelJob(store: Store<ImageData> | Store<LoopCells>) {
    if (store.job) {
        jobs.delete(store.job);
        worker?.postMessage({ type: 'stop', job: store.job });
        store.job = 0;
        retireWhenDone();
    }
}

function forget(store: Store<ImageData> | Store<LoopCells>) {
    cancelJob(store);
    stores.delete(store.key);
    cachedBytes -= store.bytes;
}

function trimCache(now: number) {
    if (cachedBytes <= CACHE_BYTES) return;
    const oldestFirst = [...stores.values()].sort((a, b) => a.usedAt - b.usedAt);
    for (const store of oldestFirst) {
        // Keep the full animation for any panel that's using it, including multiple visible panels.
        if (now - store.usedAt < RECENT_MS) continue;
        forget(store);
        if (cachedBytes <= CACHE_BYTES) return;
    }
}

function bytesOf(data: LoopCells['data']): number {
    if (data instanceof Uint8ClampedArray) return data.byteLength;
    if ('compressed' in data) return data.compressed.byteLength;
    return data.lum.byteLength + data.r.byteLength + data.g.byteLength + data.b.byteLength;
}

function remember<T extends ImageData | LoopCells>(store: Store<T>, index: number, frame: T) {
    const previous = store.frames[index];
    const change = bytesOf(frame.data) - (previous ? bytesOf(previous.data) : 0);
    store.frames[index] = frame;
    store.bytes += change;
    cachedBytes += change;
    trimCache(performance.now());
}

function workerOf(): Worker | null {
    if (worker !== undefined) return worker;
    try {
        const started = new Worker('./src/workers/loops.worker.ts');
        worker = started;
        started.onmessage = (event: { data: any }) => {
            if (worker === started) arrived(event.data);
        };
        started.onerror = () => {
            if (worker !== started) return;
            started.terminate();
            // Frames are worked out as they're needed from now on, as with no worker.
            worker = null;
            for (const store of jobs.values()) store.job = 0;
            jobs.clear();
            clearTimeout(retireTimer);
            retireTimer = undefined;
        };
    } catch {
        worker = null;
    }
    return worker;
}

function arrived(message: { job: number; done?: boolean; index: number; w?: number; h?: number; data?: Uint8ClampedArray; remix?: LoopCells }) {
    const store = jobs.get(message.job);
    if (!store) return;
    if (message.done) {
        jobs.delete(message.job);
        store.job = 0;
        retireWhenDone();
        return;
    }
    if (message.index < 0 || message.index >= store.count) return;
    if (message.remix) {
        remember(store as Store<LoopCells>, message.index, message.remix);
    } else if (message.data && message.w && message.h) {
        remember(store as Store<ImageData>, message.index, new ImageData(message.data, message.w, message.h));
    }
}

function storeOf<T extends ImageData | LoopCells>(key: string, count: number, ask: Store<T>['ask']): Store<T> {
    const now = performance.now();
    let store = stores.get(key) as Store<T> | undefined;
    if (store) {
        store.usedAt = now;
        trimCache(now);
        return store;
    }
    store = { key, count, frames: new Array(count).fill(null), bytes: 0, usedAt: now, job: 0, ask };
    stores.set(key, store as any);
    if (stores.size > MOST) {
        let oldest = store as Store<ImageData> | Store<LoopCells>;
        for (const other of stores.values()) if (other.usedAt <= oldest.usedAt && other !== store) oldest = other;
        forget(oldest);
    }
    return store;
}

const keys = new WeakMap<object, string>();

function keyOf(value: object): string {
    let key = keys.get(value);
    if (key === undefined) {
        key = JSON.stringify(value);
        keys.set(value, key);
    }
    return key;
}

/** The frames of a looping scene with these options, for a canvas `W` by `H`. */
export function loopStore(o: BackgroundOptions, W: number, H: number, count: number): Store<ImageData> {
    return storeOf<ImageData>(`${keyOf(o)}:${W}x${H}`, count, { o, W, H, remix: null });
}

/** The cells of a remix of a looping scene with these options, for a canvas `W` by `H`. */
export function remixStore(o: BackgroundOptions, W: number, H: number, count: number, remix: RemixSpec): Store<LoopCells> {
    return storeOf<LoopCells>(`${keyOf(o)}:${W}x${H}:${keyOf(remix.design)}:${keyOf(remix.palette)}`, count, { o, W, H, remix });
}

/**
 * Frame `index` of a store if it's ready, or else the nearest before it that is, having asked the worker for the loop if it
 * hasn't been. With no worker, `make` works it out now. `null` until a first frame comes.
 */
export function frameAt<T extends ImageData | LoopCells>(store: Store<T>, index: number, make: () => T | LoopPixels, precompute = true): T | null {
    const have = store.frames[index];
    if (jobs.size > 1) {
        for (const other of jobs.values()) {
            if (store.usedAt - other.usedAt >= RECENT_MS) cancelJob(other);
        }
    }
    if (have) return have;
    const w = precompute ? workerOf() : null;
    if (!w) {
        const made = make();
        const frame = ('w' in (made as LoopPixels) && 'h' in (made as LoopPixels)
            ? new ImageData((made as LoopPixels).data, (made as LoopPixels).w, (made as LoopPixels).h) : made) as T;
        remember(store, index, frame);
        return frame;
    }
    if (!store.job) {
        clearTimeout(retireTimer);
        retireTimer = undefined;
        store.job = nextJob++;
        jobs.set(store.job, store as any);
        // If a size comes back, fill only the frames that weren't ready when we left it.
        const missing = [];
        for (let offset = 0; offset < store.count; offset++) {
            const at = (index + offset) % store.count;
            if (!store.frames[at]) missing.push(at);
        }
        w.postMessage({ type: 'start', job: store.job, ...store.ask, missing });
    }
    for (let back = 1; back < store.count; back++) {
        const earlier = store.frames[(index - back + store.count) % store.count];
        if (earlier) return earlier;
    }
    return null;
}
