// Turning a file into frames the studio can sample. A picture is decoded once and kept at most 720 pixels wide. A video is read
// with the system's own decoders through `video` in `rsui/desktop`: 320 pixels wide, 12 frames a second, its first 15 seconds,
// kept by the engine and copied out one frame at a time into the same buffer. Both are kept by path, so going back and forth
// between effects never reads a file twice.

import * as fs from 'fs';
import * as path from 'path';
import { video } from 'rsui/desktop';
import type { Frame } from './effects';

export interface Media {
    kind: 'image' | 'video';
    name: string;
    /** How many frames, and how many a second: one and none for a picture. */
    count: number;
    fps: number;
    /** Frame `i`. A video's frames share one buffer, so a frame is good until the next is asked for. */
    frameAt(i: number): Frame;
}

export const IMAGE_TYPES = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp'];
export const VIDEO_TYPES = ['mp4', 'mov', 'webm', 'mkv', 'avi', 'm4v', 'wmv'];

const cache = new Map<string, Promise<Media>>();

export function kindOf(file: string): 'image' | 'video' | null {
    const ext = path.extname(file).slice(1).toLowerCase();
    if (IMAGE_TYPES.includes(ext)) return 'image';
    if (VIDEO_TYPES.includes(ext)) return 'video';
    return null;
}

export function load(file: string): Promise<Media> {
    let found = cache.get(file);
    if (!found) {
        found = kindOf(file) === 'video' ? loadVideo(file) : loadImage(file);
        cache.set(file, found);
        // A failure isn't kept, so it can be tried again.
        found.catch(() => cache.delete(file));
    }
    return found;
}

const MIME: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp', bmp: 'image/bmp' };

async function loadImage(file: string): Promise<Media> {
    const ext = path.extname(file).slice(1).toLowerCase();
    const bytes = fs.readFileSync(file);
    const url = `data:${MIME[ext] ?? 'image/png'};base64,${bytes.toString('base64')}`;
    const image = new Image();
    await new Promise<void>((done, fail) => {
        image.onload = () => done();
        image.onerror = () => fail(new Error('That picture can’t be read'));
        image.src = url;
    });
    const scale = Math.min(1, 720 / Math.max(1, image.naturalWidth));
    const w = Math.max(1, Math.round(image.naturalWidth * scale));
    const h = Math.max(1, Math.round(image.naturalHeight * scale));
    const canvas = document.createElement('canvas');
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext('2d')!;
    ctx.drawImage(image, 0, 0, w, h);
    const frame: Frame = { w, h, data: ctx.getImageData(0, 0, w, h).data };
    return { kind: 'image', name: path.basename(file), count: 1, fps: 0, frameAt: () => frame };
}

async function loadVideo(file: string): Promise<Media> {
    const opened = await video.open(file, { width: 320, fps: 12, seconds: 15 });
    const frame: Frame = { w: opened.width, h: opened.height, data: new Uint8ClampedArray(opened.width * opened.height * 4) };
    let current = -1;
    return {
        kind: 'video',
        name: path.basename(file),
        count: opened.count,
        fps: opened.fps,
        frameAt(i: number) {
            const index = ((i % opened.count) + opened.count) % opened.count;
            if (index !== current) {
                opened.read(index, frame.data);
                current = index;
            }
            return frame;
        },
    };
}
