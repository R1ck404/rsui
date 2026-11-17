// Picking something up from a list and putting it somewhere else in it: it follows the pointer, the others make room as it passes
// them, and the click that ends a drag doesn't count as one. The tabs, the sidebar's places, drives and favorites, and its sections
// are all ordered this way.

import { useMemo, useRef, useState } from 'react';

/** How far something's pulled before it's being dragged rather than clicked. */
const DRAG_AFTER = 5;

/** Which way the list goes: along a row, down a column, or both, as a grid of tiles does. */
export type Axis = 'x' | 'y' | 'both';

interface Slot {
    x: number;
    y: number;
    w: number;
    h: number;
}

export interface Reorder {
    /** How far the one being dragged is from its place, for its `transform`, or null for the rest. */
    lifted: (id: string) => { dx: number; dy: number } | null;
    /** What each one is given: its element, and the pointer going down on it, moving and coming up. */
    ref: (id: string) => (element: any) => void;
    down: (index: number, event: any) => void;
    move: (event: any) => void;
    up: () => void;
    /** Whether a click is the end of a drag, which isn't a click. Asking says so once. */
    wasDrag: () => boolean;
}

/** Reordering `ids` (in the order they're shown) along `axis`, with `onMove` told each time one moves to another place. */
export function useReorder(ids: string[], axis: Axis, onMove: (from: number, to: number) => void): Reorder {
    const elements = useRef(new Map<string, any>());
    const held = useRef<{ id: string; start: number; at: number; x: number; y: number; slots: Slot[]; moved: boolean } | null>(null);
    const dropped = useRef(false);
    const [lifted, setLifted] = useState<{ id: string; dx: number; dy: number } | null>(null);
    const latest = useRef({ ids, onMove });
    latest.current = { ids, onMove };
    const refs = useRef(new Map<string, (element: any) => void>());

    const made = useMemo(
        () => ({
            ref: (id: string) => {
                let ref = refs.current.get(id);
                if (ref === undefined) {
                    ref = (element: any) => {
                        if (element) {
                            elements.current.set(id, element);
                        } else {
                            elements.current.delete(id);
                        }
                    };
                    refs.current.set(id, ref);
                }
                return ref;
            },
            down: (index: number, event: any) => {
                // Only the main button picks something up: the other opens its menu, and the middle one does what it does.
                if (!leftButton(event)) {
                    held.current = null;
                    return;
                }
                const now = latest.current;
                // Where each is now, which are the places the one picked up can go.
                const slots = now.ids.map((id) => {
                    const box = elements.current.get(id)?.getBoundingClientRect?.();
                    return box ? { x: box.left, y: box.top, w: box.width, h: box.height } : { x: 0, y: 0, w: 0, h: 0 };
                });
                held.current = { id: now.ids[index], start: index, at: index, x: event.clientX, y: event.clientY, slots, moved: false };
            },
            move: (event: any) => {
                const grip = held.current;
                if (grip === null) {
                    return;
                }
                // What's moved with another button held, or none, isn't being dragged by the main one.
                if (typeof event.buttons === 'number' && (event.buttons & 1) === 0) {
                    held.current = null;
                    setLifted(null);
                    return;
                }
                const [x, y] = [event.clientX, event.clientY];
                const [dx, dy] = [x - grip.x, y - grip.y];
                if (!grip.moved && Math.abs(dx) + Math.abs(dy) < DRAG_AFTER) {
                    return;
                }
                grip.moved = true;
                // The place under the pointer: the one it's in, for a grid, or the one it's along from, for a row or a column.
                let to = grip.at;
                grip.slots.forEach((slot, at) => {
                    if (slot.w <= 0 || slot.h <= 0) {
                        return;
                    }
                    const acrossX = x >= slot.x && x < slot.x + slot.w;
                    const acrossY = y >= slot.y && y < slot.y + slot.h;
                    if (axis === 'both' ? acrossX && acrossY : axis === 'x' ? acrossX : acrossY) {
                        to = at;
                    }
                });
                if (to !== grip.at) {
                    latest.current.onMove(grip.at, to);
                    grip.at = to;
                }
                // It follows the pointer from wherever its place is now. Along a row or a column, places can be different sizes:
                // moving on, it ends where the one it's taken the place of ended; moving back, it starts where that started.
                const [home, here] = [grip.slots[grip.start], grip.slots[grip.at]];
                const later = grip.at > grip.start;
                const shiftX = axis === 'x' && later ? here.x + here.w - home.w - home.x : here.x - home.x;
                const shiftY = axis === 'y' && later ? here.y + here.h - home.h - home.y : here.y - home.y;
                setLifted({ id: grip.id, dx: dx - shiftX, dy: dy - shiftY });
            },
            up: () => {
                dropped.current = held.current?.moved === true;
                held.current = null;
                setLifted(null);
            },
            wasDrag: () => {
                const was = dropped.current;
                dropped.current = false;
                return was;
            },
        }),
        [axis],
    );
    return {
        ...made,
        lifted: (id: string) => (lifted !== null && lifted.id === id ? { dx: lifted.dx, dy: lifted.dy } : null),
    };
}

/** Whether a press is of the main button alone, the left one, which is what picks something up. */
export function leftButton(event: any): boolean {
    if (typeof event.button === 'number' && event.button !== 0) {
        return false;
    }
    return typeof event.buttons !== 'number' || event.buttons === 0 || (event.buttons & 1) !== 0;
}

/** `items` in the order `order` keeps, by `key`, with what it doesn't know of after, as they came. */
export function ordered<T>(items: T[], order: string[] | undefined, key: (item: T) => string): T[] {
    if (!order || order.length === 0) {
        return items;
    }
    const rank = new Map(order.map((id, at) => [id, at]));
    return items
        .map((item, at) => ({ item, at, rank: rank.get(key(item)) ?? order.length + at }))
        .sort((a, b) => a.rank - b.rank)
        .map((one) => one.item);
}

/** `ids` with the one at `from` moved to `to`. */
export function moved(ids: string[], from: number, to: number): string[] {
    const next = ids.slice();
    const [one] = next.splice(from, 1);
    next.splice(to, 0, one);
    return next;
}
