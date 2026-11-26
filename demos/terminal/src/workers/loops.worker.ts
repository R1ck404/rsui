// Works out the frames of looping backgrounds, and the cells of the studio's remixes of them, away from the thread that draws:
// see backgrounds/loops.ts. Asked for a loop from a frame on, it sends each frame as it has it, one at a time, taking turns between
// the loops it's been asked for, so that a frame asked for later doesn't wait for a whole other loop.

import { sceneFor } from '../backgrounds/scenes';
import type { BackgroundOptions, Scene } from '../backgrounds/scenes';
import { cellPixels, cellsOf } from '../studio/effects';
import { compactPixels } from '../backgrounds/loops';
import type { RemixSpec } from '../backgrounds/loops';

interface Job {
    job: number;
    W: number;
    H: number;
    remix: RemixSpec | null;
    scene: Scene;
    missing: number[];
    next: number;
}

const here = globalThis as any;
const queue: Job[] = [];
let waiting = false;

here.onmessage = (event: { data: any }) => {
    const m = event.data;
    if (m.type === 'stop') {
        const at = queue.findIndex((job) => job.job === m.job);
        if (at >= 0) queue.splice(at, 1);
        return;
    }
    if (m.type !== 'start') return;
    const o = m.o as BackgroundOptions;
    const scene = sceneFor(o, m.W, m.H);
    if (!scene?.compute || !scene.loop) return;
    queue.push({ job: m.job, W: m.W, H: m.H, remix: m.remix ?? null, scene, missing: m.missing, next: 0 });
    soon();
};

/** One frame next, after what's been sent has been heard. */
function soon() {
    if (waiting || queue.length === 0) return;
    waiting = true;
    setTimeout(step, 0);
}

function step() {
    waiting = false;
    const job = queue.shift();
    if (!job) return;
    const index = job.missing[job.next++];
    const frame = job.scene.compute!(index, job.W, job.H);
    if (job.remix) {
        const cells = cellsOf(frame, job.remix.design, job.W, job.H);
        const pixels = cellPixels(cells, job.remix.design, job.W, job.H, job.remix.palette);
        here.postMessage({ job: job.job, index, remix: { data: pixels ? compactPixels(pixels) : cells } });
    } else {
        here.postMessage({ job: job.job, index, w: frame.w, h: frame.h, data: frame.data });
    }
    // Taking turns: a loop that isn't done goes to the back.
    if (job.next < job.missing.length) queue.push(job);
    else here.postMessage({ job: job.job, done: true });
    soon();
}
