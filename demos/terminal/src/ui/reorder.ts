import { useEffect, useRef } from 'react';
import type { PointerEvent } from 'react';

type RowElement = HTMLDivElement & {
    style: { transform: string };
    toggleAttribute(name: string, on: boolean): void;
};

interface Slot {
    row: RowElement;
    start: number;
    size: number;
}

interface Drag {
    slots: Slot[];
    from: number;
    to: number;
    start: number;
    crossStart: number;
    crossMin: number;
    crossMax: number;
    order: string;
    pointer: number;
    handle: HTMLElement;
    moved: boolean;
}

type ItemKey = string | number;

export interface ReorderOutside {
    over: (id: ItemKey, x: number, y: number) => void;
    drop: (id: ItemKey, x: number, y: number) => void;
    clear: () => void;
}

export function useReorder(ids: ItemKey[], axis: 'X' | 'Y', onMove: (from: number, to: number) => void,
    onDragging?: (dragging: boolean) => void, outside?: ReorderOutside) {
    const rows = useRef(new Map<ItemKey, RowElement>());
    const held = useRef<Drag | null>(null);
    const order = JSON.stringify(ids);
    const position = (bounds: ReturnType<HTMLElement['getBoundingClientRect']>) => axis === 'X' ? bounds.left : bounds.top;
    const point = (event: PointerEvent<HTMLElement>) => axis === 'X' ? event.clientX : event.clientY;
    const crossPoint = (event: PointerEvent<HTMLElement>) => axis === 'X' ? event.clientY : event.clientX;
    const shift = (row: RowElement, offset: number, cross = 0) => {
        const transform = cross !== 0
            ? `translate(${axis === 'X' ? offset : cross}px, ${axis === 'X' ? cross : offset}px)`
            : offset === 0 ? '' : `translate${axis}(${offset}px)`;
        if (row.style.transform !== transform) row.style.transform = transform;
    };

    const finish = (commit: boolean) => {
        const drag = held.current;
        if (!drag) return;
        held.current = null;
        const reordered = commit && drag.from !== drag.to;
        if (drag.moved) {
            drag.slots[drag.from].row.toggleAttribute('data-dragging', false);
            for (const { row } of drag.slots) {
                if (reordered) row.toggleAttribute('data-dropped', true);
                shift(row, 0);
            }
            onDragging?.(false);
            outside?.clear();
        }
        if (drag.handle.hasPointerCapture(drag.pointer)) drag.handle.releasePointerCapture(drag.pointer);
        if (reordered) {
            onMove(drag.from, drag.to);
            requestAnimationFrame(() => {
                for (const { row } of drag.slots) row.toggleAttribute('data-dropped', false);
            });
        }
    };

    useEffect(() => {
        if (ids.length < 2) return;
        const cancel = () => finish(false);
        const escape = (event: KeyboardEvent) => {
            if (event.key === 'Escape' && held.current) {
                event.preventDefault();
                event.stopPropagation();
                cancel();
            }
        };
        window.addEventListener('keydown', escape, true);
        window.addEventListener('blur', cancel);
        window.addEventListener('pointercancel', cancel);
        return () => {
            window.removeEventListener('keydown', escape, true);
            window.removeEventListener('blur', cancel);
            window.removeEventListener('pointercancel', cancel);
            cancel();
        };
    }, [order, axis]);

    const bind = (id: ItemKey, element: HTMLDivElement | null) => {
        if (element) rows.current.set(id, element as RowElement);
        else rows.current.delete(id);
    };
    const start = (from: number, event: PointerEvent<HTMLElement>) => {
        if (event.button !== 0 || ids.length < 2) return;
        if (held.current) {
            if (held.current.handle.hasPointerCapture(held.current.pointer)) return;
            finish(false);
        }
        const slots: Slot[] = [];
        for (const id of ids) {
            const row = rows.current.get(id);
            const bounds = row?.parentElement?.getBoundingClientRect();
            if (!row || !bounds) return;
            const size = axis === 'X' ? bounds.width : bounds.height;
            if (size <= 0) return;
            slots.push({ row, start: position(bounds), size });
        }
        const home = event.currentTarget.getBoundingClientRect();
        event.currentTarget.setPointerCapture(event.pointerId);
        held.current = {
            slots, from, to: from, start: point(event), crossStart: crossPoint(event),
            crossMin: axis === 'X' ? home.top : home.left, crossMax: axis === 'X' ? home.bottom : home.right,
            order, pointer: event.pointerId, handle: event.currentTarget, moved: false,
        };
    };

    const track = (drag: Drag, event: PointerEvent<HTMLElement>) => {
        const { slots, from } = drag;
        const home = slots[from];
        const first = slots[0];
        const last = slots[slots.length - 1];
        const bounds = first.row.parentElement?.getBoundingClientRect();
        const scroll = bounds ? position(bounds) - first.start : 0;
        const delta = point(event) - drag.start - scroll;
        const cross = crossPoint(event);
        if (!drag.moved) {
            if (Math.abs(delta) < 4 && (!outside || Math.abs(cross - drag.crossStart) < 4)) return false;
            drag.moved = true;
            home.row.toggleAttribute('data-dragging', true);
            onDragging?.(true);
        }

        const offList = !!outside && (cross < drag.crossMin || cross >= drag.crossMax);
        let to = offList ? from : 0;
        if (!offList) {
            const center = home.start + home.size / 2 + delta;
            for (let i = 0; i < slots.length - 1; i++) {
                const before = slots[i];
                const after = slots[i + 1];
                const boundary = (before.start + before.size / 2 + after.start + after.size / 2) / 2;
                if (center < boundary) break;
                to = i + 1;
            }
        }
        const gap = slots[1].start - first.start - first.size;
        if (drag.to !== to) {
            for (let i = 0; i < slots.length; i++) {
                if (i === from) continue;
                const direction = i >= to && i < from ? 1 : i <= to && i > from ? -1 : 0;
                shift(slots[i].row, direction * (home.size + gap));
            }
            drag.to = to;
        }
        const bounded = Math.max(first.start - home.start, Math.min(last.start + last.size - home.start - home.size, delta));
        shift(home.row, offList ? delta : bounded + (delta - bounded) * 0.2, offList ? cross - drag.crossStart : 0);
        if (offList) outside.over(ids[from], event.clientX, event.clientY);
        else outside?.clear();
        return offList;
    };

    const move = (event: PointerEvent<HTMLElement>) => {
        const drag = held.current;
        if (!drag || event.pointerId !== drag.pointer) return;
        if ((event.buttons & 1) === 0 || drag.order !== order) finish(false);
        else track(drag, event);
    };
    const drop = (event: PointerEvent<HTMLElement>) => {
        const drag = held.current;
        if (!drag || event.pointerId !== drag.pointer || event.button !== 0) return false;
        if (drag.order !== order) {
            finish(false);
            return false;
        }
        const offList = track(drag, event);
        finish(!offList);
        if (offList) outside?.drop(ids[drag.from], event.clientX, event.clientY);
        if (drag.moved) return false;
        const { left, right, top, bottom } = event.currentTarget.getBoundingClientRect();
        return event.clientX >= left && event.clientX < right && event.clientY >= top && event.clientY < bottom;
    };

    // A tab with its own navigation can hide the list on selection. Give the drag first use of the press.
    const tab = (index: number, onPick: () => void, onMiddle = onPick) => ({
        ref: ids.length > 1 ? (element: HTMLDivElement | null) => bind(ids[index], element) : undefined,
        onPointerDown: (event: PointerEvent<HTMLDivElement>) => {
            if (event.button === 0 && ids.length > 1) start(index, event);
            else if (event.button === 1) onMiddle();
            else onPick();
        },
        onPointerMove: move,
        onLostPointerCapture: (event: PointerEvent<HTMLDivElement>) => {
            if (held.current?.pointer === event.pointerId) finish(false);
        },
        onPointerUp: (event: PointerEvent<HTMLDivElement>) => {
            if (drop(event)) onPick();
        },
    });

    return { bind, start, move, drop, tab };
}

export type TabDragProps = ReturnType<ReturnType<typeof useReorder>['tab']>;
