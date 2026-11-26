import { useEffect, useRef } from 'react';
import type { PointerEvent } from 'react';

type Point = { x: number; y: number };
type Pointer = PointerEvent<HTMLElement>;
const pointOf = (event: Pointer): Point => ({ x: event.clientX, y: event.clientY });

/** Coalesce preview work during a drag and apply the exact release position. */
export function useRecordingDrag<T>(apply: (intent: T, point: Point) => void) {
    const active = useRef<{ pointer: number; intent: T; point: Point } | null>(null);
    const pending = useRef<ReturnType<typeof setTimeout> | null>(null);
    const latest = useRef(apply);
    latest.current = apply;

    const clear = () => {
        if (pending.current !== null) clearTimeout(pending.current);
        pending.current = null;
        active.current = null;
    };
    useEffect(() => clear, []);

    const begin = (event: Pointer, intent: T) => {
        if (event.button !== 0 || active.current) return;
        event.stopPropagation();
        event.currentTarget.setPointerCapture(event.pointerId);
        const point = pointOf(event);
        active.current = { pointer: event.pointerId, intent, point };
        latest.current(intent, point);
    };

    return {
        begin,
        events: {
            onPointerMove(event: Pointer) {
                const drag = active.current;
                if (drag?.pointer !== event.pointerId) return;
                drag.point = pointOf(event);
                if (pending.current !== null) return;
                pending.current = setTimeout(() => {
                    pending.current = null;
                    latest.current(drag.intent, drag.point);
                }, 16);
            },
            onPointerUp(event: Pointer) {
                const drag = active.current;
                if (drag?.pointer !== event.pointerId) return;
                clear();
                latest.current(drag.intent, pointOf(event));
            },
            onPointerCancel(event: Pointer) {
                if (active.current?.pointer === event.pointerId) clear();
            },
            onLostPointerCapture(event: Pointer) {
                if (active.current?.pointer === event.pointerId) clear();
            },
        },
    };
}
