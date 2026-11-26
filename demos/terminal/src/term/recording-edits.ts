export const VIDEO_FPS = 24;
export const VIDEO_FRAME_MS = 1000 / VIDEO_FPS;

export type CropEdge = 'left' | 'top' | 'right' | 'bottom';
export type Crop = Record<CropEdge, number>;

export const FULL_FRAME: Crop = { left: 0, top: 0, right: 1, bottom: 1 };
const MIN_CROP = 0.1;

export const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

export function normalizeCrop(crop: Crop | undefined): Crop {
    if (!crop || !Number.isFinite(crop.left) || !Number.isFinite(crop.top) ||
        !Number.isFinite(crop.right) || !Number.isFinite(crop.bottom)) return FULL_FRAME;
    const left = clamp(crop.left, 0, 1 - MIN_CROP);
    const top = clamp(crop.top, 0, 1 - MIN_CROP);
    return { left, top, right: clamp(crop.right, left + MIN_CROP, 1), bottom: clamp(crop.bottom, top + MIN_CROP, 1) };
}

export function cropEdge(crop: Crop, edge: CropEdge, position: number): Crop {
    const limits = {
        left: [0, crop.right - MIN_CROP],
        top: [0, crop.bottom - MIN_CROP],
        right: [crop.left + MIN_CROP, 1],
        bottom: [crop.top + MIN_CROP, 1],
    };
    const [min, max] = limits[edge];
    return { ...crop, [edge]: clamp(position, min, max) };
}

export function fitCrop(aspect: number, sourceAspect: number): Crop {
    const width = Math.min(1, aspect / sourceAspect);
    const height = Math.min(1, sourceAspect / aspect);
    return { left: (1 - width) / 2, right: (1 + width) / 2, top: (1 - height) / 2, bottom: (1 + height) / 2 };
}

export function moveCrop(crop: Crop, x: number, y: number): Crop {
    const dx = clamp(x, -crop.left, 1 - crop.right);
    const dy = clamp(y, -crop.top, 1 - crop.bottom);
    return { left: crop.left + dx, top: crop.top + dy, right: crop.right + dx, bottom: crop.bottom + dy };
}

export function trimTime(edge: 'from' | 'to', at: number, start: number, end: number, duration: number): number {
    const minimum = Math.min(Math.round(VIDEO_FRAME_MS), duration);
    return edge === 'from'
        ? clamp(Math.round(at), 0, Math.max(0, end - minimum))
        : clamp(Math.round(at), Math.min(duration, start + minimum), duration);
}

export function timecode(ms: number) {
    const time = Math.round(ms);
    const minutes = Math.floor(time / 60000);
    const seconds = String(Math.floor(time / 1000) % 60).padStart(2, '0');
    return `${minutes}:${seconds}.${String(time % 1000).padStart(3, '0')}`;
}
