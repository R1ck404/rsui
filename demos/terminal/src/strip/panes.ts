export type PaneSide = 'left' | 'right' | 'top' | 'bottom';

/** Leaves index the column's sessions, so the same layout can be restored with fresh shells. */
export type PaneLayout = number | { axis: 'x' | 'y'; children: PaneLayout[] };

export interface PaneRect {
    x: number;
    y: number;
    w: number;
    h: number;
}

const fullPane: PaneRect = { x: 0, y: 0, w: 1, h: 1 };

export function stackedPanes(count: number): PaneLayout {
    return count === 1 ? 0 : { axis: 'y', children: Array.from({ length: count }, (_, i) => i) };
}

export function paneRects(layout: PaneLayout, rect = fullPane): { index: number; rect: PaneRect }[] {
    if (typeof layout === 'number') return [{ index: layout, rect }];
    const { axis, children } = layout;
    const w = axis === 'x' ? rect.w / children.length : rect.w;
    const h = axis === 'y' ? rect.h / children.length : rect.h;
    return children.flatMap((child, i) => paneRects(child, {
        x: rect.x + (axis === 'x' ? i * w : 0),
        y: rect.y + (axis === 'y' ? i * h : 0),
        w,
        h,
    }));
}

export function splitPaneLayout(layout: PaneLayout, target: number | null, incoming: PaneLayout, side: PaneSide): PaneLayout {
    if (target === null || layout === target) {
        const axis = side === 'left' || side === 'right' ? 'x' : 'y';
        const children = side === 'left' || side === 'top' ? [incoming, layout] : [layout, incoming];
        return { axis, children };
    }
    if (typeof layout === 'number') return layout;
    return { ...layout, children: layout.children.map((child) => splitPaneLayout(child, target, incoming, side)) };
}

export function offsetPanes(layout: PaneLayout, offset: number): PaneLayout {
    if (typeof layout === 'number') return layout + offset;
    return { ...layout, children: layout.children.map((child) => offsetPanes(child, offset)) };
}

export function removePane(layout: PaneLayout, index: number): PaneLayout | null {
    if (typeof layout === 'number') return layout === index ? null : layout > index ? layout - 1 : layout;
    const children = layout.children.map((child) => removePane(child, index)).filter((child) => child !== null);
    return children.length === 0 ? null : children.length === 1 ? children[0] : { ...layout, children };
}

export function restorePaneLayout(value: unknown, count: number): PaneLayout | undefined {
    const seen = new Set<number>();
    const valid = (node: unknown, depth: number): node is PaneLayout => {
        if (typeof node === 'number') {
            if (!Number.isInteger(node) || node < 0 || node >= count || seen.has(node)) return false;
            seen.add(node);
            return true;
        }
        if (!node || typeof node !== 'object' || depth >= count) return false;
        const { axis, children } = node as Partial<Exclude<PaneLayout, number>>;
        return (axis === 'x' || axis === 'y') && Array.isArray(children) && children.length >= 2 &&
            children.length <= count && children.every((child) => valid(child, depth + 1));
    };
    return valid(value, 0) && seen.size === count ? value : undefined;
}

export function paneDropAt(rect: PaneRect, x: number, y: number): PaneSide | undefined {
    if (rect.w <= 0 || rect.h <= 0 || x < rect.x || x >= rect.x + rect.w || y < rect.y || y >= rect.y + rect.h) return;
    const fx = (x - rect.x) / rect.w;
    const fy = (y - rect.y) / rect.h;
    const across = Math.min(fx, 1 - fx);
    const down = Math.min(fy, 1 - fy);
    if (across <= down) return fx < 0.5 ? 'left' : 'right';
    return fy < 0.5 ? 'top' : 'bottom';
}

export function paneHalf(rect: PaneRect, side: PaneSide): PaneRect {
    if (side === 'left' || side === 'right') {
        const w = rect.w / 2;
        return { ...rect, w, x: rect.x + (side === 'right' ? w : 0) };
    }
    const h = rect.h / 2;
    return { ...rect, h, y: rect.y + (side === 'bottom' ? h : 0) };
}

export function paneStyle({ x, y, w, h }: PaneRect) {
    return { left: `${x * 100}%`, top: `${y * 100}%`, width: `${w * 100}%`, height: `${h * 100}%` };
}
