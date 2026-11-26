import { crc32, deflateSync, inflateSync } from 'zlib';

const MAX_PIXELS = 4 * 1024 * 1024;
export const MAX_IMAGE_DATA = 24 * 1024 * 1024;
export interface Picture { data: string; w: number; h: number; pixelAspect?: number }

function validSize(w: number, h: number) {
    return Number.isInteger(w) && Number.isInteger(h) && w > 0 && h > 0 && w <= 4096 && h <= 4096 && w * h <= MAX_PIXELS;
}

function png(w: number, h: number, bytes: Uint8Array, channels: number): Picture {
    const stride = w * channels;
    const raster = Buffer.alloc((stride + 1) * h);
    for (let y = 0; y < h; y++) raster.set(bytes.subarray(y * stride, (y + 1) * stride), y * (stride + 1) + 1);
    const chunk = (type: string, data: Uint8Array) => {
        const out = Buffer.alloc(12 + data.length);
        const view = new DataView(out.buffer, out.byteOffset, out.byteLength);
        view.setUint32(0, data.length);
        out.write(type, 4);
        out.set(data, 8);
        view.setUint32(out.length - 4, crc32(out.subarray(4, -4)));
        return out;
    };
    const header = Buffer.alloc(13);
    const view = new DataView(header.buffer, header.byteOffset, header.byteLength);
    view.setUint32(0, w);
    view.setUint32(4, h);
    header[8] = 8;
    header[9] = channels === 4 ? 6 : 2;
    const data = Buffer.concat([
        Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', header),
        chunk('IDAT', deflateSync(raster, { level: 1 })), chunk('IEND', new Uint8Array()),
    ]).toString('base64');
    return { data, w, h };
}

export function kittyPicture(data: string, options: Record<string, string>): Picture | null {
    if (data.length > MAX_IMAGE_DATA || !/^[a-zA-Z0-9+/]*={0,2}$/.test(data)) return null;
    try {
        let bytes = Buffer.from(data, 'base64');
        if (options.o === 'z') {
            const rawSize = Number(options.s) * Number(options.v) * (options.f === '24' ? 3 : 4);
            const limit = options.f === '100' ? MAX_PIXELS * 4 : rawSize;
            if (!Number.isInteger(limit) || limit < 1 || limit > MAX_PIXELS * 4) return null;
            bytes = inflateSync(bytes, { maxOutputLength: limit });
        }
        else if (options.o) return null;
        const format = options.f ?? '32';
        if (format === '100') {
            if (bytes.length < 33 || bytes.toString('hex', 0, 8) !== '89504e470d0a1a0a' || bytes.toString('ascii', 12, 16) !== 'IHDR') return null;
            const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
            const w = view.getUint32(16), h = view.getUint32(20);
            return validSize(w, h) ? { data: bytes.toString('base64'), w, h } : null;
        }
        const channels = format === '24' ? 3 : format === '32' ? 4 : 0;
        const w = Number(options.s), h = Number(options.v);
        return channels && validSize(w, h) && bytes.length === w * h * channels ? png(w, h, bytes, channels) : null;
    } catch {
        return null;
    }
}

const DEFAULT_COLORS = [0x000000, 0x3333cc, 0xcc2121, 0x33cc33, 0xcc33cc, 0x33cccc, 0xcccc33, 0x878787,
    0x424242, 0x545499, 0x994242, 0x549954, 0x995499, 0x549999, 0x999954, 0xcccccc];

function sixelColor(mode: number, a: number, b: number, c: number) {
    const byte = (n: number) => Math.round(Math.min(1, Math.max(0, n)) * 255);
    if (mode === 2) return (byte(a / 100) << 16) | (byte(b / 100) << 8) | byte(c / 100);
    if (mode !== 1) return 0;
    // DEC places blue, red, and green at 0, 120, and 240 degrees.
    const hue = ((a + 240) % 360) / 60;
    const light = b / 100;
    const chroma = (1 - Math.abs(2 * light - 1)) * c / 100;
    const secondary = chroma * (1 - Math.abs(hue % 2 - 1));
    const [r, g, blue] = [[chroma, secondary, 0], [secondary, chroma, 0], [0, chroma, secondary],
        [0, secondary, chroma], [secondary, 0, chroma], [chroma, 0, secondary]][Math.floor(hue)];
    const base = light - chroma / 2;
    return (byte(r + base) << 16) | (byte(g + base) << 8) | byte(blue + base);
}

export function sixelPicture(data: string, transparent: boolean, background: number, defaultAspect = 1): Picture | null {
    if (data.length > 4 * 1024 * 1024) return null;
    // Measure first so decoding never allocates based on the protocol's theoretical maximum alone.
    let w = 0, h = 0;
    let aspect = defaultAspect;
    let pixels: Uint8Array | null = null;
    for (let pass = 0; pass < 2; pass++) {
        let x = 0, y = 0, color = 0, repeat = 1;
        let paintCount = 0;
        const palette = [...DEFAULT_COLORS];
        const input = /([!#"])([\d;]*)|[\x3f-\x7e$-]/g;
        for (const token of data.matchAll(input)) {
            const op = token[0][0];
            if (op === '!' || op === '#' || op === '"') {
                const values = token[2].split(';').map(Number);
                if (op === '!') repeat = values[0] || 1;
                else if (op === '#') {
                    color = Math.min(255, values[0]);
                    if (values.length === 5) palette[color] = sixelColor(values[1], values[2], values[3], values[4]);
                } else if (!pixels) {
                    w = Math.max(w, values[2] || 0);
                    h = Math.max(h, values[3] || 0);
                    if (values[0] > 0 && values[1] > 0) aspect = Math.max(0.25, Math.min(4, values[0] / values[1]));
                }
            } else if (op === '$') x = 0;
            else if (op === '-') { x = 0; y += 6; }
            else {
                const bits = op.charCodeAt(0) - 63;
                const end = x + repeat;
                if (end > 4096 || y >= 4096) return null;
                if (!pixels) {
                    w = Math.max(w, end);
                    if (bits) h = Math.max(h, y + 32 - Math.clz32(bits));
                    if (bits && (paintCount += repeat * 6) > MAX_PIXELS * 8) return null;
                } else if (bits) {
                    const ink = palette[color] ?? 0;
                    for (let bit = 0; bit < 6 && y + bit < h; bit++) if (bits & (1 << bit)) {
                        for (let col = x; col < end; col++) {
                            const at = ((y + bit) * w + col) * 4;
                            pixels[at] = ink >>> 16;
                            pixels[at + 1] = ink >>> 8;
                            pixels[at + 2] = ink;
                            pixels[at + 3] = 255;
                        }
                    }
                }
                x = end;
                repeat = 1;
            }
        }
        if (!pixels) {
            if (!validSize(w, h)) return null;
            pixels = new Uint8Array(w * h * 4);
            if (!transparent) for (let i = 0; i < pixels.length; i += 4) {
                pixels[i] = background >>> 16;
                pixels[i + 1] = background >>> 8;
                pixels[i + 2] = background;
                pixels[i + 3] = 255;
            }
        }
    }
    return { ...png(w, h, pixels!, 4), pixelAspect: aspect };
}
