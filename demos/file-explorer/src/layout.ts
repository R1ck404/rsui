// How a tab's panes are laid out: a tree of splits, each a row or a column of what's in it, down to the panes. Splitting a pane
// beside one in a row of them puts the new one in that row, rather than splitting it into a row of its own, so three side by side
// are one row of three and each handle between them moves only the two it's between. Taking a pane out gives its room to the one
// before it (or after it, for the first), and a split left with one thing in it is that thing again, so the tree never has more in
// it than what's shown.
//
// Everything here makes a new tree rather than changing the one it's given, and none of it knows about React.

export type Dir = 'row' | 'column';
export type Side = 'left' | 'right' | 'top' | 'bottom';

export type Layout =
    | { kind: 'pane'; id: string }
    | { kind: 'split'; id: string; dir: Dir; children: Layout[]; sizes: number[] };

export interface Rect {
    x: number;
    y: number;
    w: number;
    h: number;
}

/** A handle between two things in a split: dragging it moves the edge between `index` and the one after. */
export interface Handle {
    split: string;
    index: number;
    dir: Dir;
    /** Where it is, as `place` works it out. The handles that are shown are where layout puts them, in the gaps. */
    rect?: Rect;
    /** The pixels the split's sizes are shares of, which is how far a share moves for a pixel of drag, as it's measured when a
     * drag starts. */
    room: number;
}

/** The smallest a pane can be, so that its toolbar still fits. */
export const MIN_WIDTH = 280;
export const MIN_HEIGHT = 160;
/** No more than this many panes in a tab, which is more than there's room for on most screens anyway. */
export const MAX_PANES = 8;
/** The thinnest a handle is, so that it can be caught however small the gap between panels is. */
export const HANDLE = 6;

let made = 0;
export const splitId = () => 's' + Date.now().toString(36) + (made++).toString(36);

export const pane = (id: string): Layout => ({ kind: 'pane', id });

/** The panes in it, as they're read: left to right, then top to bottom. */
export function paneIds(layout: Layout): string[] {
    return layout.kind === 'pane' ? [layout.id] : layout.children.flatMap(paneIds);
}

export const dirOf = (side: Side): Dir => (side === 'left' || side === 'right' ? 'row' : 'column');
const before = (side: Side) => side === 'left' || side === 'top';

/** Shares that add up to one, each at least a little, from whatever they were. */
function shares(sizes: number[], count: number): number[] {
    const given = Array.from({ length: count }, (_, at) => {
        const size = sizes[at];
        return typeof size === 'number' && Number.isFinite(size) && size > 0 ? size : 1 / count;
    });
    const total = given.reduce((sum, size) => sum + size, 0);
    return given.map((size) => size / total);
}

/**
 * The same tree, tidied: splits of one thing are that thing, a row in a row is one row (with its shares of its share), shares add up
 * to one, and a pane that's in it twice is only in it the first time. What isn't a layout at all is null.
 */
export function tidy(layout: any, seen = new Set<string>()): Layout | null {
    if (!layout || typeof layout !== 'object') {
        return null;
    }
    if (layout.kind === 'pane') {
        if (typeof layout.id !== 'string' || seen.has(layout.id)) {
            return null;
        }
        seen.add(layout.id);
        return { kind: 'pane', id: layout.id };
    }
    if (layout.kind !== 'split' || !Array.isArray(layout.children) || (layout.dir !== 'row' && layout.dir !== 'column')) {
        return null;
    }
    const given = shares(Array.isArray(layout.sizes) ? layout.sizes : [], layout.children.length);
    const children: Layout[] = [];
    const sizes: number[] = [];
    layout.children.forEach((child: any, at: number) => {
        const kept = tidy(child, seen);
        if (kept === null) {
            return;
        }
        if (kept.kind === 'split' && kept.dir === layout.dir) {
            kept.children.forEach((inner, i) => {
                children.push(inner);
                sizes.push(given[at] * kept.sizes[i]);
            });
        } else {
            children.push(kept);
            sizes.push(given[at]);
        }
    });
    if (children.length === 0) {
        return null;
    }
    if (children.length === 1) {
        return children[0];
    }
    return { kind: 'split', id: typeof layout.id === 'string' ? layout.id : splitId(), dir: layout.dir, children, sizes: shares(sizes, children.length) };
}

/** `node` put beside the pane `target`, on its `side`, taking half its room. */
export function insert(layout: Layout, target: string, node: Layout, side: Side): Layout {
    const dir = dirOf(side);
    const walk = (at: Layout): Layout => {
        if (at.kind === 'pane') {
            if (at.id !== target) {
                return at;
            }
            return { kind: 'split', id: splitId(), dir, children: before(side) ? [node, at] : [at, node], sizes: [0.5, 0.5] };
        }
        const index = at.children.findIndex((child) => child.kind === 'pane' && child.id === target);
        if (index >= 0 && at.dir === dir) {
            // Beside one in a row of them, it goes in the row, in half of that one's place.
            const children = at.children.slice();
            const sizes = at.sizes.slice();
            const half = sizes[index] / 2;
            sizes[index] = half;
            const place = before(side) ? index : index + 1;
            children.splice(place, 0, node);
            sizes.splice(place, 0, half);
            return { ...at, children, sizes };
        }
        return { ...at, children: at.children.map(walk) };
    };
    return tidy(walk(layout)) ?? layout;
}

/** `node` put beside the whole of `layout`, on its `side`, as a tab dropped on the edge of all the panes is: a share as big as one of
 * what's along that way already, so a third pane in a row is a third of it. */
export function beside(layout: Layout, node: Layout, side: Side): Layout {
    const dir = dirOf(side);
    const along = layout.kind === 'split' && layout.dir === dir ? layout.children.length : 1;
    const share = 1 / (along + 1);
    const children = before(side) ? [node, layout] : [layout, node];
    const sizes = before(side) ? [share, 1 - share] : [1 - share, share];
    return tidy({ kind: 'split', id: splitId(), dir, children, sizes }) ?? layout;
}

/** The tree without the pane `id`, with its room given to the one before it, or after it for the first, or null if it was all. */
export function remove(layout: Layout, id: string): Layout | null {
    const walk = (at: Layout): Layout | null => {
        if (at.kind === 'pane') {
            return at.id === id ? null : at;
        }
        const children: Layout[] = [];
        const sizes: number[] = [];
        // Room of one taken out with nothing before it, for the next.
        let spare = 0;
        at.children.forEach((child, index) => {
            const kept = walk(child);
            if (kept !== null) {
                children.push(kept);
                sizes.push(at.sizes[index] + spare);
                spare = 0;
            } else if (sizes.length > 0) {
                sizes[sizes.length - 1] += at.sizes[index];
            } else {
                spare += at.sizes[index];
            }
        });
        if (children.length === 0) {
            return null;
        }
        return { ...at, children, sizes };
    };
    const kept = walk(layout);
    return kept === null ? null : tidy(kept);
}

/** The pane that's given the room of `id` when it's taken out: the one beside it that grows, or the nearest one of those. */
export function heir(layout: Layout, id: string): string | null {
    const find = (at: Layout): string | null | undefined => {
        if (at.kind === 'pane') {
            return undefined;
        }
        const index = at.children.findIndex((child) => child.kind === 'pane' && child.id === id);
        if (index >= 0) {
            const next = index > 0 ? at.children[index - 1] : at.children[index + 1];
            if (!next) {
                return null;
            }
            const ids = paneIds(next);
            // The nearest of what's beside it: the last of what's before it, or the first of what's after.
            return index > 0 ? ids[ids.length - 1] : ids[0];
        }
        for (const child of at.children) {
            const found = find(child);
            if (found !== undefined) {
                return found;
            }
        }
        return undefined;
    };
    return find(layout) ?? null;
}

/** How small `layout` can be along `dir`, with `gap` between what's in it. */
export function smallest(layout: Layout, dir: Dir, gap: number): number {
    if (layout.kind === 'pane') {
        return dir === 'row' ? MIN_WIDTH : MIN_HEIGHT;
    }
    const each = layout.children.map((child) => smallest(child, dir, gap));
    return layout.dir === dir ? each.reduce((sum, size) => sum + size, 0) + gap * (layout.children.length - 1) : Math.max(...each);
}

/** Splitting `room` into `sizes`, rounded so that they and the gaps between them add up to it exactly. */
function lengths(room: number, sizes: number[], gap: number): number[] {
    const free = Math.max(0, room - gap * (sizes.length - 1));
    let used = 0;
    let share = 0;
    return sizes.map((size) => {
        share += size;
        const end = Math.round(free * share);
        const length = end - used;
        used = end;
        return length;
    });
}

/**
 * Where each pane would go in `rect`, and the handles between them. The panes that are shown are laid out by the page, as flex
 * boxes with these sizes as their shares (see src/Splits.tsx); this is for what isn't there to be laid out, as where a tab dropped
 * on them would land, and the small pictures of layouts in the menu.
 */
export function place(layout: Layout, rect: Rect, gap: number): { panes: Map<string, Rect>; handles: Handle[] } {
    const panes = new Map<string, Rect>();
    const handles: Handle[] = [];
    const walk = (at: Layout, box: Rect) => {
        if (at.kind === 'pane') {
            panes.set(at.id, box);
            return;
        }
        const row = at.dir === 'row';
        const each = lengths(row ? box.w : box.h, at.sizes, gap);
        const room = Math.max(1, (row ? box.w : box.h) - gap * (at.children.length - 1));
        let offset = row ? box.x : box.y;
        at.children.forEach((child, index) => {
            const length = each[index];
            walk(child, row ? { x: offset, y: box.y, w: length, h: box.h } : { x: box.x, y: offset, w: box.w, h: length });
            offset += length;
            if (index < at.children.length - 1) {
                // The handle fills the gap, and is never thinner than can be caught.
                const thick = Math.max(gap, HANDLE);
                const from = offset + gap / 2 - thick / 2;
                handles.push({
                    split: at.id,
                    index,
                    dir: at.dir,
                    rect: row ? { x: from, y: box.y, w: thick, h: box.h } : { x: box.x, y: from, w: box.w, h: thick },
                    room,
                });
                offset += gap;
            }
        });
    };
    walk(layout, rect);
    return { panes, handles };
}

/** The split `id` in the tree, or null. */
export function findSplit(layout: Layout, id: string): Extract<Layout, { kind: 'split' }> | null {
    if (layout.kind === 'pane') {
        return null;
    }
    if (layout.id === id) {
        return layout;
    }
    for (const child of layout.children) {
        const found = findSplit(child, id);
        if (found) {
            return found;
        }
    }
    return null;
}

/** The split `id` with `sizes` instead. */
export function withSizes(layout: Layout, id: string, sizes: number[]): Layout {
    if (layout.kind === 'pane') {
        return layout;
    }
    if (layout.id === id) {
        return { ...layout, sizes: shares(sizes, layout.children.length) };
    }
    return { ...layout, children: layout.children.map((child) => withSizes(child, id, sizes)) };
}

/**
 * The sizes of the split `id` with the edge after `index` moved by `by` pixels from where it was in `start`, as far as each side of
 * it can go and still be as big as what's in it needs. With no room for that, as in a small window, each side keeps at least a
 * share of what the two of them have.
 */
export function dragged(layout: Layout, id: string, index: number, start: number[], by: number, room: number, gap: number): number[] {
    const split = findSplit(layout, id);
    if (!split || index < 0 || index + 1 >= split.children.length) {
        return start;
    }
    const pair = start[index] + start[index + 1];
    const least = (child: Layout) => Math.min(smallest(child, split.dir, gap) / room, pair * 0.35);
    const [lo, hi] = [least(split.children[index]), pair - least(split.children[index + 1])];
    const first = Math.min(hi, Math.max(lo, start[index] + by / room));
    const sizes = start.slice();
    sizes[index] = first;
    sizes[index + 1] = pair - first;
    return sizes;
}

/** Whether a pane at `rect` has room for another beside it on `side`, each still at least as big as a pane can be. */
export function fits(rect: Rect | undefined, side: Side, gap: number, node: Layout = pane(''), width = MIN_WIDTH, height = MIN_HEIGHT): boolean {
    if (!rect) {
        return false;
    }
    const dir = dirOf(side);
    const need = dir === 'row' ? width + gap + smallest(node, 'row', gap) : height + gap + smallest(node, 'column', gap);
    const across = dir === 'row' ? smallest(node, 'column', gap) <= rect.h : smallest(node, 'row', gap) <= rect.w;
    return (dir === 'row' ? rect.w : rect.h) >= need && across;
}

/** The pane next to `from` on `side`: the one that's most beside it, nearest first. */
export function neighbor(rects: Map<string, Rect>, from: string, side: Side): string | null {
    const here = rects.get(from);
    if (!here) {
        return null;
    }
    let best: { id: string; score: number } | null = null;
    for (const [id, rect] of rects) {
        if (id === from) {
            continue;
        }
        const row = side === 'left' || side === 'right';
        const gapTo =
            side === 'left' ? here.x - (rect.x + rect.w) : side === 'right' ? rect.x - (here.x + here.w) : side === 'top' ? here.y - (rect.y + rect.h) : rect.y - (here.y + here.h);
        if (gapTo < -1) {
            continue;
        }
        const overlap = row
            ? Math.min(here.y + here.h, rect.y + rect.h) - Math.max(here.y, rect.y)
            : Math.min(here.x + here.w, rect.x + rect.w) - Math.max(here.x, rect.x);
        if (overlap <= 0) {
            continue;
        }
        const score = gapTo * 1000 - overlap;
        if (best === null || score < best.score) {
            best = { id, score };
        }
    }
    return best?.id ?? null;
}

/** Which side of `rect` the point is nearest, as a share of its size, so the middle of a tall pane still has a left and a right. */
export function sideAt(rect: Rect, x: number, y: number): Side {
    const fx = (x - rect.x) / Math.max(1, rect.w);
    const fy = (y - rect.y) / Math.max(1, rect.h);
    const distances: [Side, number][] = [
        ['left', fx],
        ['right', 1 - fx],
        ['top', fy],
        ['bottom', 1 - fy],
    ];
    distances.sort((a, b) => a[1] - b[1]);
    return distances[0][0];
}

/** The half of `rect` on `side`, which is where something dropped there goes. */
export function halfOf(rect: Rect, side: Side, gap: number): Rect {
    const w = (rect.w - gap) / 2;
    const h = (rect.h - gap) / 2;
    switch (side) {
        case 'left':
            return { x: rect.x, y: rect.y, w, h: rect.h };
        case 'right':
            return { x: rect.x + rect.w - w, y: rect.y, w, h: rect.h };
        case 'top':
            return { x: rect.x, y: rect.y, w: rect.w, h };
        default:
            return { x: rect.x, y: rect.y + rect.h - h, w: rect.w, h };
    }
}

// Ready-made layouts, as a row of choices to pick from: each says how many panes it has and how they're laid out, with `#` for
// the next pane.

export type Preset = 'single' | 'columns' | 'rows' | 'three' | 'main-side' | 'main-bottom' | 'grid';

export const PRESETS: { id: Preset; label: string; count: number; shape: any }[] = [
    { id: 'single', label: 'One pane', count: 1, shape: '#' },
    { id: 'columns', label: 'Side by side', count: 2, shape: { dir: 'row', children: ['#', '#'] } },
    { id: 'rows', label: 'One above the other', count: 2, shape: { dir: 'column', children: ['#', '#'] } },
    { id: 'three', label: 'Three columns', count: 3, shape: { dir: 'row', children: ['#', '#', '#'] } },
    { id: 'main-side', label: 'One beside two', count: 3, shape: { dir: 'row', children: ['#', { dir: 'column', children: ['#', '#'] }] } },
    { id: 'main-bottom', label: 'One above two', count: 3, shape: { dir: 'column', children: ['#', { dir: 'row', children: ['#', '#'] }] } },
    { id: 'grid', label: 'Four in a grid', count: 4, shape: { dir: 'row', children: [{ dir: 'column', children: ['#', '#'] }, { dir: 'column', children: ['#', '#'] }] } },
];

/** A preset's layout with the panes `ids` in it, in order. */
export function presetLayout(preset: Preset, ids: string[]): Layout {
    const shape = PRESETS.find((one) => one.id === preset)?.shape ?? '#';
    let next = 0;
    const build = (at: any): Layout => {
        if (at === '#') {
            return pane(ids[next++]);
        }
        const children = at.children.map(build);
        return { kind: 'split', id: splitId(), dir: at.dir, children, sizes: children.map(() => 1 / children.length) };
    };
    return build(shape);
}

/** Which preset a layout is, if it's one, with its shares even or not. */
export function presetOf(layout: Layout): Preset | null {
    const shapeOf = (at: Layout): any => (at.kind === 'pane' ? '#' : { dir: at.dir, children: at.children.map(shapeOf) });
    const text = JSON.stringify(shapeOf(layout));
    return PRESETS.find((one) => JSON.stringify(one.shape) === text)?.id ?? null;
}

/** Every split's shares made even again. */
export function evened(layout: Layout): Layout {
    if (layout.kind === 'pane') {
        return layout;
    }
    return { ...layout, children: layout.children.map(evened), sizes: layout.children.map(() => 1 / layout.children.length) };
}
