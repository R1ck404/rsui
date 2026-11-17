// The parts of a tab's panes that aren't the panes: where each one sits, the handles between them, the outline of where a tab
// dragged over them would go, and the menu of layouts. How they're laid out is src/layout.ts.

import { memo, useEffect, useRef, useState } from 'react';
import { showKey } from './keys';
import { HANDLE, place, PRESETS, presetLayout, type Handle as HandleBox, type Layout, type Preset, type Rect } from './layout';
import { leftButton } from './reorder';
import type { Settings } from './settings';

/** How panels move to new places and sizes: see `--fx-move` and `--fx-ease` in theme.ts. */
const MOVES = 'duration-(--fx-move) ease-(--fx-ease) ';

/**
 * Where a pane sits. When `animate` is on it glides to a new place, and a pane that's `fresh`, just split off another, fades and
 * grows in rather than appearing all at once.
 */
export const Slot = memo(function Slot(props: { rect: Rect; animate: boolean; fresh: boolean; children: any }) {
    const { rect, animate, fresh, children } = props;
    const [shown, setShown] = useState(!fresh);
    useEffect(() => {
        if (shown) {
            return;
        }
        // Drawn once as it starts, so there's something for it to move from.
        const timer = setTimeout(() => setShown(true), 16);
        return () => clearTimeout(timer);
    }, []);
    return (
        <div
            className={
                'absolute ' +
                (animate || !shown ? 'transition-[left,top,opacity,transform] ' + MOVES : 'transition-[opacity,transform] ' + MOVES) +
                (shown ? 'opacity-100 [transform:none]' : 'opacity-0 [transform:scale(0.97)]')
            }
            style={{ left: rect.x, top: rect.y, width: rect.w, height: rect.h }}
        >
            {children}
        </div>
    );
});

/**
 * A handle between two panes, along a row or down a column of them. Dragging it moves the edge between them, as far as each can
 * go; a double-click makes the panes it's between, and the rest of that row or column, the same size again.
 */
export const Handle = memo(function Handle(props: {
    handle: HandleBox;
    animate: boolean;
    onStart: (handle: HandleBox) => void;
    onMove: (handle: HandleBox, by: number) => void;
    onDone: () => void;
    onReset: (handle: HandleBox) => void;
}) {
    const { handle, animate, onStart, onMove, onDone, onReset } = props;
    const from = useRef<number | null>(null);
    const [held, setHeld] = useState(false);
    const row = handle.dir === 'row';
    const r = handle.rect;
    const end = () => {
        if (from.current === null) {
            return;
        }
        from.current = null;
        setHeld(false);
        onDone();
    };
    return (
        <div
            className={
                'group absolute z-10 flex items-center justify-center ' +
                (row ? 'cursor-col-resize ' : 'cursor-row-resize ') +
                (animate ? 'transition-[left,top,width,height] ' + MOVES : '')
            }
            style={{ left: r.x, top: r.y, width: r.w, height: r.h }}
            onPointerDown={(event: any) => {
                if (!leftButton(event)) {
                    return;
                }
                from.current = row ? event.clientX : event.clientY;
                setHeld(true);
                onStart(handle);
            }}
            onPointerMove={(event: any) => {
                if (from.current === null) {
                    return;
                }
                // Let go somewhere it didn't hear, as outside the window: it's done.
                if (typeof event.buttons === 'number' && (event.buttons & 1) === 0) {
                    end();
                    return;
                }
                onMove(handle, (row ? event.clientX : event.clientY) - from.current);
            }}
            onPointerUp={end}
            onDoubleClick={() => onReset(handle)}
        >
            <span
                className={
                    'rounded-full transition-opacity duration-(--fx-speed) ' +
                    (row ? 'h-10 w-[3px] ' : 'h-[3px] w-10 ') +
                    (held ? 'bg-(--fx-accent) opacity-100' : 'bg-(--fx-faint) opacity-0 group-hover:opacity-60')
                }
            />
        </div>
    );
});

/**
 * What's shown over the panes while a tab's being dragged, which stay where they are. Lifted, they're outlined faintly, as somewhere it
 * can go. Over them, a see-through sheet in the accent covers where it'd land, with its folder's name, and glides from place to place
 * as the pointer moves; where there isn't room, it says so instead.
 */
export const DockLayer = memo(function DockLayer(props: {
    lifting: boolean;
    landing: Rect | null;
    refused: { rect: Rect; why: string } | null;
    what: { icon: any; title: string } | null;
    width: number;
    height: number;
}) {
    const { lifting, landing, refused, what, width, height } = props;
    // The sheet stays where it last was as it fades, rather than jumping to nowhere first, and starts from there when it's back.
    const last = useRef<Rect | null>(landing);
    if (landing) {
        last.current = landing;
    }
    const sheet = landing ?? last.current;
    const Icon = what?.icon;
    return (
        <>
            <div
                className={
                    'pointer-events-none absolute inset-0 z-20 rounded-(--fx-radius-lg) border-[1.5px] border-dashed border-(--fx-accent) transition-opacity duration-[180ms] ' +
                    (lifting && !landing && !refused ? 'opacity-50' : 'opacity-0')
                }
                style={{ width, height }}
            />
            {sheet ? (
                <div
                    className={
                        'pointer-events-none absolute z-30 flex flex-col items-center justify-center gap-1.5 overflow-hidden rounded-(--fx-radius-lg) border-[1.5px] border-(--fx-accent) bg-(--fx-accent-soft) text-(--fx-accent) transition-[left,top,width,height,opacity] duration-[180ms] ease-(--fx-ease) ' +
                        (landing ? 'opacity-100' : 'opacity-0')
                    }
                    style={{ left: sheet.x, top: sheet.y, width: sheet.w, height: sheet.h }}
                >
                    {Icon ? <Icon size={22} strokeWidth={1.6} /> : null}
                    {what ? <span className="max-w-[85%] truncate text-[0.95em] font-medium text-(--fx-text)">{what.title}</span> : null}
                </div>
            ) : null}
            {refused ? (
                <span
                    className="pointer-events-none absolute z-30 rounded-full bg-(--fx-pop) px-3.5 py-1.5 text-[0.9em] whitespace-nowrap shadow-(--fx-pop-shadow) [transform:translate(-50%,-50%)]"
                    style={{ left: refused.rect.x + refused.rect.w / 2, top: refused.rect.y + refused.rect.h / 2 }}
                >
                    {refused.why}
                </span>
            ) : null}
        </>
    );
});

/** A layout drawn small, as its panes' shapes. */
function Mini(props: { layout: Layout; width: number; height: number; focus?: string }) {
    const { panes } = place(props.layout, { x: 0, y: 0, w: props.width, h: props.height }, 2);
    return (
        <span className="relative block shrink-0" style={{ width: props.width, height: props.height }}>
            {[...panes].map(([id, r]) => (
                <span
                    key={id}
                    className={'absolute rounded-[3px] ' + (id === props.focus ? 'bg-(--fx-accent)' : 'bg-current opacity-45')}
                    style={{ left: r.x, top: r.y, width: r.w, height: r.h }}
                />
            ))}
        </span>
    );
}

const PRESET_IDS = ['1', '2', '3', '4'];

/**
 * The layouts a tab can have, drawn as what they look like, and splitting the pane that has the keyboard, closing it, or making them
 * all the same size. What there isn't room for in the window is greyed out.
 */
export const LayoutMenu = memo(function LayoutMenu(props: {
    settings: Settings;
    layout: Layout;
    focus: string;
    current: Preset | null;
    count: number;
    canPreset: (preset: Preset) => boolean;
    canSplit: { right: boolean; bottom: boolean };
    onPreset: (preset: Preset) => void;
    onSplit: (side: 'right' | 'bottom') => void;
    onEven: () => void;
    onClosePane: () => void;
    onClose: () => void;
}) {
    const { settings, layout, focus, current, count, canPreset, canSplit } = props;
    const keys = settings.keys;
    const action = (label: string, shortcut: string, enabled: boolean, onClick: () => void, picture?: any) => (
        <button
            disabled={!enabled}
            onClick={() => {
                onClick();
                props.onClose();
            }}
            className={
                'flex h-[32px] items-center gap-2.5 rounded-(--fx-radius) px-2 text-left transition-colors duration-(--fx-speed) ' +
                (enabled ? 'hover:bg-(--fx-hover)' : 'opacity-40')
            }
        >
            <span className="flex w-[22px] shrink-0 justify-center text-(--fx-muted)">{picture}</span>
            <span className="min-w-0 flex-1 truncate">{label}</span>
            {shortcut ? <span className="shrink-0 text-[0.85em] text-(--fx-faint)">{showKey(shortcut)}</span> : null}
        </button>
    );
    const box = (dir: 'row' | 'column'): Layout => ({
        kind: 'split',
        id: 'x',
        dir,
        children: [{ kind: 'pane', id: 'a' }, { kind: 'pane', id: 'b' }],
        sizes: [0.5, 0.5],
    });
    return (
        <>
            <div className="absolute inset-0 z-30" onClick={props.onClose} onContextMenu={props.onClose} />
            <div className="absolute top-[46px] right-[150px] z-40 flex w-[300px] flex-col gap-1 rounded-(--fx-radius-lg) bg-(--fx-pop) p-3 text-[0.95em] shadow-(--fx-pop-shadow)">
                <div className="flex items-baseline justify-between px-1 pb-1">
                    <span className="font-medium">Layout</span>
                    <span className="text-[0.85em] text-(--fx-faint)">{count === 1 ? 'One pane' : count + ' panes'}</span>
                </div>
                <div className="grid grid-cols-4 gap-1.5 pb-1">
                    {PRESETS.map((preset) => {
                        const enabled = canPreset(preset.id);
                        const chosen = preset.id === current;
                        return (
                            <button
                                key={preset.id}
                                title={preset.label + (enabled ? '' : ': the window is too small')}
                                aria-label={preset.label}
                                disabled={!enabled}
                                onClick={() => {
                                    props.onPreset(preset.id);
                                    props.onClose();
                                }}
                                className={
                                    'flex h-[50px] items-center justify-center rounded-(--fx-radius) transition-colors duration-(--fx-speed) ' +
                                    (chosen
                                        ? 'bg-(--fx-accent-soft) text-(--fx-accent)'
                                        : enabled
                                          ? 'bg-(--fx-input) text-(--fx-muted) hover:bg-(--fx-press) hover:text-(--fx-text)'
                                          : 'bg-(--fx-input) text-(--fx-faint) opacity-40')
                                }
                            >
                                <Mini layout={presetLayout(preset.id, PRESET_IDS)} width={44} height={30} />
                            </button>
                        );
                    })}
                    {current === null ? (
                        // A layout of its own, made by splitting panes, isn't one of these: it's shown as it is, last.
                        <span className="flex h-[50px] items-center justify-center rounded-(--fx-radius) bg-(--fx-accent-soft) text-(--fx-accent)" title="This tab's own layout">
                            <Mini layout={layout} width={44} height={30} focus={focus} />
                        </span>
                    ) : null}
                </div>
                <div className="h-px bg-(--fx-line)" />
                <div className="flex flex-col pt-1">
                    {action('Split right', keys.split, canSplit.right, () => props.onSplit('right'), <Mini layout={box('row')} width={18} height={13} />)}
                    {action('Split down', keys.splitDown, canSplit.bottom, () => props.onSplit('bottom'), <Mini layout={box('column')} width={18} height={13} />)}
                    {action('Same sizes', keys.evenPanes, count > 1, props.onEven, <span className="text-[1.1em] leading-none">=</span>)}
                    {action('Close this pane', keys.closePane, count > 1, props.onClosePane, <span className="text-[1.1em] leading-none">×</span>)}
                </div>
                <p className="px-1 pt-1 text-[0.85em] leading-snug text-(--fx-faint)">
                    Drag a tab onto a pane to open it beside it. Drag between panes to resize them; double-click to even them out.
                </p>
            </div>
        </>
    );
});

/** A short note at the bottom of the window, as when there's no room to split a pane, which goes by itself. */
export const Notice = memo(function Notice(props: { text: string | null }) {
    const last = useRef('');
    if (props.text) {
        last.current = props.text;
    }
    return (
        <div
            className={
                'pointer-events-none absolute bottom-6 left-1/2 z-50 rounded-full bg-(--fx-pop) px-4 py-2 text-[0.9em] shadow-(--fx-pop-shadow) transition-[opacity,transform] duration-(--fx-move) ease-(--fx-ease) ' +
                (props.text ? 'opacity-100 [transform:translateX(-50%)]' : 'opacity-0 [transform:translate(-50%,8px)]')
            }
        >
            {last.current}
        </div>
    );
});

export { HANDLE };
