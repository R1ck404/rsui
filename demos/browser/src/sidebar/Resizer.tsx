// The gap beside the sidebar, which resizes it as it's dragged and puts it back to its usual width when it's double-clicked.

import { useRef } from 'react';
import { DEFAULT_SETTINGS } from '../model';

const SIDEBAR_MIN = 200;
const SIDEBAR_MAX = 420;

export function Resizer(props: {
    width: number;
    /** Whether the sidebar's on the right, so dragging left makes it wider. */
    right: boolean;
    /** Each move, with the width it's dragged to. */
    onMove: (width: number) => void;
    /** Let go, with where it ended up. */
    onDone: (width: number) => void;
}) {
    const from = useRef<{ x: number; width: number; now: number } | null>(null);
    return (
        <div
            className="w-2 shrink-0 cursor-col-resize"
            title="Drag to resize the sidebar"
            onPointerDown={(event: any) => {
                event.currentTarget.setPointerCapture?.(event.pointerId);
                from.current = { x: event.clientX, width: props.width, now: props.width };
            }}
            onPointerMove={(event: any) => {
                const drag = from.current;
                if (drag === null) return;
                const by = (event.clientX - drag.x) * (props.right ? -1 : 1);
                drag.now = Math.max(SIDEBAR_MIN, Math.min(SIDEBAR_MAX, drag.width + by));
                props.onMove(drag.now);
            }}
            onPointerUp={() => {
                const drag = from.current;
                from.current = null;
                if (drag !== null) props.onDone(drag.now);
            }}
            onDoubleClick={() => props.onDone(DEFAULT_SETTINGS.sidebarWidth)}
        />
    );
}
