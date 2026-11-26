// Box drawing and block characters, drawn as rectangles instead of from the font, so lines meet from cell to cell and row to row
// whatever the line height, as they do in a TUI's frames.

/** What a character is made of: lines out from the middle to each side (0 none, 1 light, 2 heavy, 3 double), or blocks. */
export type BoxShape =
    | { kind: 'lines'; up: number; down: number; left: number; right: number }
    | { kind: 'block'; x0: number; y0: number; x1: number; y1: number; alpha: number };

const L = (up: number, down: number, left: number, right: number): BoxShape => ({ kind: 'lines', up, down, left, right });
const B = (x0: number, y0: number, x1: number, y1: number, alpha = 1): BoxShape => ({ kind: 'block', x0, y0, x1, y1, alpha });

const TABLE = new Map<number, BoxShape>([
    [0x2500, L(0, 0, 1, 1)], [0x2501, L(0, 0, 2, 2)], [0x2502, L(1, 1, 0, 0)], [0x2503, L(2, 2, 0, 0)],
    [0x2504, L(0, 0, 1, 1)], [0x2505, L(0, 0, 2, 2)], [0x2506, L(1, 1, 0, 0)], [0x2507, L(2, 2, 0, 0)],
    [0x2508, L(0, 0, 1, 1)], [0x2509, L(0, 0, 2, 2)], [0x250a, L(1, 1, 0, 0)], [0x250b, L(2, 2, 0, 0)],
    [0x250c, L(0, 1, 0, 1)], [0x250d, L(0, 1, 0, 2)], [0x250e, L(0, 2, 0, 1)], [0x250f, L(0, 2, 0, 2)],
    [0x2510, L(0, 1, 1, 0)], [0x2511, L(0, 1, 2, 0)], [0x2512, L(0, 2, 1, 0)], [0x2513, L(0, 2, 2, 0)],
    [0x2514, L(1, 0, 0, 1)], [0x2515, L(1, 0, 0, 2)], [0x2516, L(2, 0, 0, 1)], [0x2517, L(2, 0, 0, 2)],
    [0x2518, L(1, 0, 1, 0)], [0x2519, L(1, 0, 2, 0)], [0x251a, L(2, 0, 1, 0)], [0x251b, L(2, 0, 2, 0)],
    [0x251c, L(1, 1, 0, 1)], [0x251d, L(1, 1, 0, 2)], [0x2520, L(2, 2, 0, 1)], [0x2523, L(2, 2, 0, 2)],
    [0x2524, L(1, 1, 1, 0)], [0x2525, L(1, 1, 2, 0)], [0x2528, L(2, 2, 1, 0)], [0x252b, L(2, 2, 2, 0)],
    [0x252c, L(0, 1, 1, 1)], [0x252f, L(0, 1, 2, 2)], [0x2530, L(0, 2, 1, 1)], [0x2533, L(0, 2, 2, 2)],
    [0x2534, L(1, 0, 1, 1)], [0x2537, L(1, 0, 2, 2)], [0x2538, L(2, 0, 1, 1)], [0x253b, L(2, 0, 2, 2)],
    [0x253c, L(1, 1, 1, 1)], [0x253f, L(1, 1, 2, 2)], [0x2542, L(2, 2, 1, 1)], [0x254b, L(2, 2, 2, 2)],
    [0x254c, L(0, 0, 1, 1)], [0x254d, L(0, 0, 2, 2)], [0x254e, L(1, 1, 0, 0)], [0x254f, L(2, 2, 0, 0)],
    [0x2550, L(0, 0, 3, 3)], [0x2551, L(3, 3, 0, 0)], [0x2552, L(0, 1, 0, 3)], [0x2553, L(0, 3, 0, 1)],
    [0x2554, L(0, 3, 0, 3)], [0x2555, L(0, 1, 3, 0)], [0x2556, L(0, 3, 1, 0)], [0x2557, L(0, 3, 3, 0)],
    [0x2558, L(1, 0, 0, 3)], [0x2559, L(3, 0, 0, 1)], [0x255a, L(3, 0, 0, 3)], [0x255b, L(1, 0, 3, 0)],
    [0x255c, L(3, 0, 1, 0)], [0x255d, L(3, 0, 3, 0)], [0x255e, L(1, 1, 0, 3)], [0x255f, L(3, 3, 0, 1)],
    [0x2560, L(3, 3, 0, 3)], [0x2561, L(1, 1, 3, 0)], [0x2562, L(3, 3, 1, 0)], [0x2563, L(3, 3, 3, 0)],
    [0x2564, L(0, 1, 3, 3)], [0x2565, L(0, 3, 1, 1)], [0x2566, L(0, 3, 3, 3)], [0x2567, L(1, 0, 3, 3)],
    [0x2568, L(3, 0, 1, 1)], [0x2569, L(3, 0, 3, 3)], [0x256a, L(1, 1, 3, 3)], [0x256b, L(3, 3, 1, 1)],
    [0x256c, L(3, 3, 3, 3)],
    // Rounded corners are drawn square, which is how they meet the lines beside them anyway.
    [0x256d, L(0, 1, 0, 1)], [0x256e, L(0, 1, 1, 0)], [0x256f, L(1, 0, 1, 0)], [0x2570, L(1, 0, 0, 1)],
    [0x2574, L(0, 0, 1, 0)], [0x2575, L(1, 0, 0, 0)], [0x2576, L(0, 0, 0, 1)], [0x2577, L(0, 1, 0, 0)],
    [0x2578, L(0, 0, 2, 0)], [0x2579, L(2, 0, 0, 0)], [0x257a, L(0, 0, 0, 2)], [0x257b, L(0, 2, 0, 0)],
    [0x257c, L(0, 0, 1, 2)], [0x257d, L(1, 2, 0, 0)], [0x257e, L(0, 0, 2, 1)], [0x257f, L(2, 1, 0, 0)],
    // Blocks.
    [0x2580, B(0, 0, 1, 0.5)], [0x2588, B(0, 0, 1, 1)], [0x258c, B(0, 0, 0.5, 1)], [0x2590, B(0.5, 0, 1, 1)],
    [0x2591, B(0, 0, 1, 1, 0.25)], [0x2592, B(0, 0, 1, 1, 0.5)], [0x2593, B(0, 0, 1, 1, 0.75)],
    [0x2594, B(0, 0, 1, 1 / 8)], [0x2595, B(7 / 8, 0, 1, 1)],
]);
for (let n = 1; n <= 7; n++) {
    TABLE.set(0x2580 + n, B(0, 1 - n / 8, 1, 1));
    TABLE.set(0x2590 - n, B(0, 0, n / 8, 1));
}

export function boxShape(cp: number): BoxShape | null {
    if (cp < 0x2500 || cp > 0x259f) return null;
    return TABLE.get(cp) ?? null;
}

export interface Rect {
    x: number;
    y: number;
    w: number;
    h: number;
    alpha: number;
}

/** The rectangles that draw a shape in a cell at `left`, `cw` wide and `ch` tall. */
export function boxRects(shape: BoxShape, left: number, cw: number, ch: number): Rect[] {
    if (shape.kind === 'block') {
        const x0 = Math.round(left + shape.x0 * cw);
        const x1 = Math.round(left + shape.x1 * cw);
        const y0 = Math.round(shape.y0 * ch);
        const y1 = Math.round(shape.y1 * ch);
        return [{ x: x0, y: y0, w: Math.max(1, x1 - x0), h: Math.max(1, y1 - y0), alpha: shape.alpha }];
    }
    const light = Math.max(1, Math.round(ch / 18));
    const heavy = light * 2;
    const x0 = Math.round(left);
    const x1 = Math.round(left + cw);
    const cx = Math.round(left + cw / 2);
    const cy = Math.round(ch / 2);
    const rects: Rect[] = [];
    const thick = (w: number) => (w === 2 ? heavy : light);
    const { up, down, left: l, right: r } = shape;
    // How far a line reaches past the middle, so a corner's two halves meet without a notch.
    const reachH = Math.max(thick(up), thick(down), up === 3 || down === 3 ? light * 3 : 0) / 2;
    const reachV = Math.max(thick(l), thick(r), l === 3 || r === 3 ? light * 3 : 0) / 2;
    const horizontal = (from: number, to: number, weight: number) => {
        if (weight === 3) {
            rects.push({ x: from, y: cy - light - Math.floor(light / 2), w: to - from, h: light, alpha: 1 });
            rects.push({ x: from, y: cy + light - Math.floor(light / 2), w: to - from, h: light, alpha: 1 });
        } else {
            const t = thick(weight);
            rects.push({ x: from, y: cy - Math.floor(t / 2), w: to - from, h: t, alpha: 1 });
        }
    };
    const vertical = (from: number, to: number, weight: number) => {
        if (weight === 3) {
            rects.push({ x: cx - light - Math.floor(light / 2), y: from, w: light, h: to - from, alpha: 1 });
            rects.push({ x: cx + light - Math.floor(light / 2), y: from, w: light, h: to - from, alpha: 1 });
        } else {
            const t = thick(weight);
            rects.push({ x: cx - Math.floor(t / 2), y: from, w: t, h: to - from, alpha: 1 });
        }
    };
    if (l) horizontal(x0, Math.ceil(cx + (r ? 0 : reachH)), l);
    if (r) horizontal(Math.floor(cx - (l ? 0 : reachH)), x1, r);
    if (up) vertical(0, Math.ceil(cy + (down ? 0 : reachV)), up);
    if (down) vertical(Math.floor(cy - (up ? 0 : reachV)), ch, down);
    return rects.filter((rect) => rect.w > 0 && rect.h > 0);
}
