// How big a cell is for a font: the width one character takes, measured over a hundred of them so the fraction stays, and the
// height a line of it needs. Rows are laid out in whole cells from these, so they have to be what the text really takes.

export interface Metrics {
    /** A cell's width in pixels, which is usually a fraction. */
    cw: number;
    /** A row's height. */
    ch: number;
    /** The font's own height of a line, which the row's is that times the line height. */
    natural: number;
    /** Where text goes down from the top of a row to sit in the middle of it. */
    offset: number;
}

const cache = new Map<string, Metrics>();
let context: CanvasRenderingContext2D | null = null;

export function measureFont(family: string, size: number, lineHeight: number): Metrics {
    const key = `${family}|${size}|${lineHeight}`;
    const kept = cache.get(key);
    if (kept) return kept;
    let cw = size * 0.6;
    let natural = Math.round(size * 1.2);
    try {
        context ??= document.createElement('canvas').getContext('2d');
        if (context) {
            context.font = `${size}px ${JSON.stringify(family)}`;
            const m = context.measureText('M'.repeat(100));
            if (m.width > 0) cw = m.width / 100;
            const height = (m.fontBoundingBoxAscent ?? 0) + (m.fontBoundingBoxDescent ?? 0);
            if (height > 0) natural = height;
        }
    } catch {
        // Without a canvas the size is a guess that's close for monospaced fonts.
    }
    const ch = Math.max(natural, Math.round(natural * lineHeight));
    const metrics = { cw, ch, natural, offset: Math.floor((ch - natural) / 2) };
    cache.set(key, metrics);
    return metrics;
}
