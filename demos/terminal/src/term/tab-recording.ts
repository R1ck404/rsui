// Reader for older PNG-frame recordings.
import * as fs from 'fs';

interface Frame {
    at: number;
    offset: number;
    bytes: number;
    background: number;
}

export interface TabFilm {
    file: string;
    duration: number;
    frames: Frame[];
}

const MAGIC = 0x52535431; // RST1
const LAST_BYTES = 8;
const MAX_CHUNK = 32 * 1024 * 1024;

function readChunk(fd: number, offset: number, count: number): Buffer {
    const bytes = Buffer.alloc(count);
    let at = 0;
    while (at < count) {
        const got = fs.readSync(fd, bytes, at, count - at, offset + at);
        if (!got) throw new Error('The tab recording is incomplete');
        at += got;
    }
    return bytes;
}

export function loadTabFilm(file: string): TabFilm | null {
    let fd: number | undefined;
    try {
        const size = fs.statSync(file).size;
        if (size < LAST_BYTES) return null;
        fd = fs.openSync(file, 'r');
        const tail = readChunk(fd, size - LAST_BYTES, LAST_BYTES);
        const view = new DataView(tail.buffer, tail.byteOffset, tail.byteLength);
        const length = view.getUint32(0);
        const start = size - LAST_BYTES - length;
        if (view.getUint32(4) !== MAGIC || length > MAX_CHUNK || start < 0) return null;
        const data = JSON.parse(readChunk(fd, start, length).toString('utf8'));
        const frames: Frame[] = data?.frames;
        const duration: number = data?.duration;
        if (!Number.isSafeInteger(duration) || duration < 0 || !Array.isArray(frames) || !frames.length) return null;
        let end = 0;
        let time = 0;
        for (const frame of frames) {
            if (!frame || !Number.isSafeInteger(frame.at) || frame.at < time || frame.at > duration ||
                !Number.isSafeInteger(frame.offset) || frame.offset !== end || !Number.isSafeInteger(frame.bytes) ||
                !Number.isInteger(frame.background) || frame.background < 0 || frame.background > 0xffffff ||
                frame.bytes <= 0 || frame.bytes > MAX_CHUNK || frame.offset + frame.bytes > start) return null;
            end = frame.offset + frame.bytes;
            time = frame.at;
        }
        return { file, duration, frames };
    } catch {
        return null;
    } finally {
        if (fd !== undefined) fs.closeSync(fd);
    }
}

export function tabFrameAt(film: TabFilm, time: number) {
    let first = 0;
    let last = film.frames.length;
    while (first < last) {
        const middle = (first + last) >>> 1;
        if (film.frames[middle].at <= time) first = middle + 1;
        else last = middle;
    }
    return Math.max(0, first - 1);
}

export function readTabFrame(film: TabFilm, index: number): string {
    const frame = film.frames[index];
    const fd = fs.openSync(film.file, 'r');
    try {
        return 'data:image/png;base64,' + readChunk(fd, frame.offset, frame.bytes).toString('base64');
    } finally {
        fs.closeSync(fd);
    }
}
