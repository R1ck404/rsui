// Where each column goes. In the strip they're side by side in one row, the open one in the middle of the window with the
// edges of its neighbors showing at the sides. In the overview they're all shrunk into a grid that fits the window.

import type { ColumnWidth } from '../settings';

export interface Place {
    x: number;
    y: number;
    /** The size the column is laid out at, which a scale then shrinks. */
    w: number;
    h: number;
    scale: number;
}

export const GAP = 10;

export function columnWidth(width: ColumnWidth, viewport: number, peek: number): number {
    switch (width) {
        case 'full':
            return Math.max(240, viewport - 2 * GAP);
        case 'wide':
            return Math.max(240, viewport - 2 * (peek + GAP));
        case 'twothirds':
            return Math.max(240, Math.round((viewport - 3 * GAP) * (2 / 3)));
        case 'half':
            return Math.max(240, Math.round((viewport - 3 * GAP) / 2));
        case 'third':
            return Math.max(200, Math.round((viewport - 4 * GAP) / 3));
    }
}

/** The strip: columns in a row, scrolled so the focused one's in the middle, without scrolling past either end. */
export function stripPlaces(widths: number[], focus: number, viewport: number, height: number): Place[] {
    const places: Place[] = [];
    let x = GAP;
    for (const w of widths) {
        places.push({ x, y: 0, w, h: height, scale: 1 });
        x += w + GAP;
    }
    const total = x;
    if (places.length === 0) return places;
    let scroll: number;
    if (total <= viewport) {
        scroll = -(viewport - total) / 2;
    } else {
        const f = places[Math.max(0, Math.min(focus, places.length - 1))];
        scroll = f.x + f.w / 2 - viewport / 2;
        scroll = Math.max(0, Math.min(total - viewport, scroll));
    }
    return places.map((p) => ({ ...p, x: Math.round(p.x - scroll) }));
}

/** The overview: every column at one scale, in as many rows as lets them be the biggest. */
export function overviewPlaces(widths: number[], viewport: number, height: number, margin = 28): Place[] {
    const n = widths.length;
    if (n === 0) return [];
    const cardW = Math.max(...widths);
    const gap = 22;
    let best = { rows: 1, cols: n, scale: 0 };
    for (let rows = 1; rows <= n; rows++) {
        const cols = Math.ceil(n / rows);
        const scale = Math.min(
            (viewport - 2 * margin - (cols - 1) * gap) / (cols * cardW),
            (height - 2 * margin - (rows - 1) * gap) / (rows * height),
            1,
        );
        if (scale > best.scale) best = { rows, cols, scale };
    }
    const { rows, cols, scale } = best;
    const gridW = cols * cardW * scale + (cols - 1) * gap;
    const gridH = rows * height * scale + (rows - 1) * gap;
    const left = (viewport - gridW) / 2;
    const top = (height - gridH) / 2;
    return widths.map((w, i) => {
        const row = Math.floor(i / cols);
        const col = i % cols;
        // A narrower column keeps its width, centered in its cell, and a last row that isn't full is centered too.
        const inset = ((cardW - w) * scale) / 2;
        const inRow = row === rows - 1 ? n - row * cols : cols;
        const shift = ((cols - inRow) * (cardW * scale + gap)) / 2;
        return { x: Math.round(left + shift + col * (cardW * scale + gap) + inset), y: Math.round(top + row * (height * scale + gap)), w, h: height, scale };
    });
}
