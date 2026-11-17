// The window around the panels: its tabs, its own minimize, maximize and close buttons (it has no title bar of its own), the
// handles that resize the panels, and the popover that customizes how it looks.

import { memo, useMemo, useRef, useState } from 'react';
import { mainWindow, useWindowState } from 'rsui';
import * as I from './icons';
import { ACCENTS, type Settings } from './settings';
import { leftButton } from './reorder';
import { IconButton, Segmented, Stepper, Toggle } from './ui';

type IconType = (props: { size?: number; strokeWidth?: number; className?: string }) => any;

/** What a tab shows: its folder's name and icon, and how many panes it has. */
export interface TabView {
    id: string;
    title: string;
    icon: IconType;
    /** How many panes it's split into. */
    panes: number;
}

const Tab = memo(function Tab(props: {
    tab: TabView;
    active: boolean;
    vertical: boolean;
    rail: boolean;
    index: number;
    lifted: { dx: number; dy: number; out: boolean } | null;
    holder: (id: string, element: any) => void;
    onSelect: (index: number) => void;
    onClose: (index: number) => void;
    onGrab: (index: number, x: number, y: number) => void;
    onDrag: (x: number, y: number) => void;
    onDrop: () => void;
}) {
    const { tab, active, vertical, rail, index, lifted, holder, onSelect, onClose, onGrab, onDrag, onDrop } = props;
    const Icon = tab.icon;
    return (
        <div
            ref={(element: any) => holder(tab.id, element)}
            role="tab"
            aria-selected={active}
            title={tab.title}
            onClick={() => onSelect(index)}
            onAuxClick={(event: any) => event.button === 1 && onClose(index)}
            onPointerDown={(event: any) => leftButton(event) && onGrab(index, event.clientX, event.clientY)}
            onPointerMove={(event: any) => (typeof event.buttons !== 'number' || (event.buttons & 1) !== 0 ? onDrag(event.clientX, event.clientY) : onDrop())}
            onPointerUp={onDrop}
            className={
                'group relative flex min-w-0 items-center gap-2 overflow-hidden rounded-(--fx-radius-lg) pr-1 pl-2.5 ' +
                (lifted ? 'z-50 shadow-(--fx-pop-shadow) ' + (lifted.out ? 'opacity-90 ' : '') : 'transition-colors duration-(--fx-speed) ') +
                (vertical ? 'h-8 w-full ' : 'h-[30px] max-w-[220px] min-w-[44px] flex-1 basis-0 ') +
                (active || lifted ? 'bg-(--fx-tab) text-(--fx-text) shadow-(--fx-tab-shadow)' : 'text-(--fx-muted) hover:bg-(--fx-tab-hover) hover:text-(--fx-text)')
            }
            style={lifted ? { transform: `translate(${lifted.dx}px, ${lifted.dy}px)` } : undefined}
        >
            <span className={'flex shrink-0 ' + (active ? 'text-(--fx-accent)' : '')}><Icon size={15} strokeWidth={1.6} /></span>
            <span className={'min-w-0 flex-1 truncate text-[0.95em] transition-opacity duration-(--fx-speed) ' + (rail ? 'opacity-0' : '')}>{tab.title}</span>
            {tab.panes > 1 && !rail ? <span className="flex shrink-0 text-(--fx-faint)" title={tab.panes + ' panes'}><I.LayoutIcon size={12} strokeWidth={1.7} /></span> : null}
            {rail ? null : <button
                title="Close tab"
                aria-label="Close tab"
                onPointerDown={(event: any) => event.stopPropagation()}
                onClick={(event: any) => {
                    event.stopPropagation();
                    onClose(index);
                }}
                className={
                    'flex h-5 w-5 shrink-0 items-center justify-center rounded-[5px] text-(--fx-faint) hover:bg-(--fx-hover) hover:text-(--fx-text) ' +
                    (active ? 'opacity-100' : 'opacity-0 group-hover:opacity-100')
                }
            >
                <I.XIcon size={11} strokeWidth={2} />
            </button>}
        </div>
    );
});

/** How far a tab's pulled before it's being dragged rather than clicked. */
const DRAG_AFTER = 5;

/**
 * The tabs, along the top or down the sidebar. A tab can be picked up and moved, following the pointer either way, and the others
 * make room as it passes them, along the row or down the column, which is the order it's dropped in. Over the panes it's being
 * docked instead, beside one of them: `onDock` is told where it is all along, and says when it's over them, and `onDockDrop` is told
 * when it's let go there.
 */
export const Tabs = memo(function Tabs(props: {
    tabs: TabView[];
    active: number;
    vertical: boolean;
    rail?: boolean;
    onSelect: (index: number) => void;
    onClose: (index: number) => void;
    onNew: () => void;
    onMove: (from: number, to: number) => void;
    /** Told where a tab being dragged is, and null when it's let go: whether it's over the panes, to be docked there. */
    onDock?: (at: { index: number; x: number; y: number } | null) => boolean;
    onDockDrop?: (index: number) => void;
}) {
    const { tabs, active, vertical, onSelect, onClose, onNew, onMove } = props;
    const rail = props.rail === true;
    const elements = useRef(new Map<string, any>());
    const held = useRef<{
        id: string;
        start: number;
        at: number;
        x: number;
        y: number;
        slots: { x: number; y: number; w: number; h: number }[];
        moved: boolean;
        out: boolean;
    } | null>(null);
    const dropped = useRef(false);
    // While a tab's held, the window's told where the pointer goes too, so that it isn't lost when what's under it is something else.
    const following = useRef<(() => void) | null>(null);
    const [lifted, setLifted] = useState<{ id: string; dx: number; dy: number; out: boolean } | null>(null);
    const latest = useRef({ tabs, onMove, onSelect, vertical, onDock: props.onDock, onDockDrop: props.onDockDrop });
    latest.current = { tabs, onMove, onSelect, vertical, onDock: props.onDock, onDockDrop: props.onDockDrop };

    const on = useMemo(
        () => ({
            holder: (id: string, element: any) => {
                if (element) {
                    elements.current.set(id, element);
                } else {
                    elements.current.delete(id);
                }
            },
            grab: (index: number, x: number, y: number) => {
                const now = latest.current;
                // Where each tab is now, which are the places the one picked up can go.
                const slots = now.tabs.map((one) => {
                    const box = elements.current.get(one.id)?.getBoundingClientRect?.();
                    return box ? { x: box.left, y: box.top, w: box.width, h: box.height } : { x: 0, y: 0, w: 0, h: 0 };
                });
                held.current = { id: now.tabs[index].id, start: index, at: index, x, y, slots, moved: false, out: false };
                following.current?.();
                const moved = (event: any) => {
                    if (typeof event.buttons === 'number' && (event.buttons & 1) === 0) {
                        on.drop();
                    } else {
                        on.drag(event.clientX, event.clientY);
                    }
                };
                const up = () => on.drop();
                const target: any = window;
                target.addEventListener('pointermove', moved);
                target.addEventListener('pointerup', up);
                // Escape puts it back where it was picked up, and so does the window losing the pointer, as to another window.
                const key = (event: any) => {
                    if (event.key === 'Escape' && held.current !== null) {
                        event.preventDefault?.();
                        event.stopPropagation?.();
                        on.cancel();
                    }
                };
                const away = () => on.cancel();
                target.addEventListener('keydown', key, true);
                target.addEventListener('blur', away);
                following.current = () => {
                    target.removeEventListener('pointermove', moved);
                    target.removeEventListener('pointerup', up);
                    target.removeEventListener('keydown', key, true);
                    target.removeEventListener('blur', away);
                    following.current = null;
                };
            },
            drag: (x: number, y: number) => {
                const grip = held.current;
                if (grip === null) {
                    return;
                }
                const [dx, dy] = [x - grip.x, y - grip.y];
                if (!grip.moved && Math.abs(dx) + Math.abs(dy) < DRAG_AFTER) {
                    return;
                }
                grip.moved = true;
                const now = latest.current;
                // Over the panes, it's being docked there, which the window shows, rather than moved among the tabs.
                const out = now.onDock?.({ index: grip.at, x, y }) === true;
                grip.out = out;
                if (out) {
                    const [home, here] = [grip.slots[grip.start], grip.slots[grip.at]];
                    setLifted({ id: grip.id, dx: dx - (here.x - home.x), dy: dy - (here.y - home.y), out: true });
                    return;
                }
                // The place under the pointer, along the way the tabs go.
                const along = now.vertical ? y : x;
                let to = grip.at;
                grip.slots.forEach((slot, at) => {
                    const [from, size] = now.vertical ? [slot.y, slot.h] : [slot.x, slot.w];
                    if (size > 0 && along >= from && along < from + size) {
                        to = at;
                    }
                });
                if (to !== grip.at) {
                    now.onMove(grip.at, to);
                    grip.at = to;
                }
                // It follows the pointer from wherever its place is now.
                const [home, here] = [grip.slots[grip.start], grip.slots[grip.at]];
                setLifted({ id: grip.id, dx: dx - (here.x - home.x), dy: dy - (here.y - home.y), out: false });
            },
            // Let go of without doing anything: back where it was, and the click that ends it isn't one.
            cancel: () => {
                const grip = held.current;
                following.current?.();
                if (grip === null) {
                    return;
                }
                held.current = null;
                dropped.current = grip.moved;
                setLifted(null);
                if (grip.moved) {
                    if (grip.at !== grip.start) {
                        latest.current.onMove(grip.at, grip.start);
                    }
                    latest.current.onDock?.(null);
                }
            },
            drop: () => {
                const grip = held.current;
                following.current?.();
                // Heard on the tab and on the window both: only the first counts.
                if (grip === null) {
                    return;
                }
                dropped.current = grip.moved === true;
                held.current = null;
                setLifted(null);
                if (grip?.moved) {
                    if (grip.out) {
                        latest.current.onDockDrop?.(grip.at);
                    }
                    latest.current.onDock?.(null);
                }
            },
            select: (index: number) => {
                // The click that ends a drag isn't one.
                if (dropped.current) {
                    dropped.current = false;
                    return;
                }
                latest.current.onSelect(index);
            },
        }),
        [],
    );

    return (
        <div role="tablist" className={vertical ? 'flex flex-col gap-0.5' : 'flex min-w-0 flex-1 items-center gap-1'}>
            {tabs.map((tab, index) => (
                <Tab
                    key={tab.id}
                    tab={tab}
                    index={index}
                    active={index === active}
                    vertical={vertical}
                    rail={rail}
                    lifted={lifted !== null && lifted.id === tab.id ? lifted : null}
                    holder={on.holder}
                    onSelect={on.select}
                    onClose={onClose}
                    onGrab={on.grab}
                    onDrag={on.drag}
                    onDrop={on.drop}
                />
            ))}
            <button
                title="New tab"
                aria-label="New tab"
                onClick={onNew}
                className={
                    'flex shrink-0 items-center gap-2 rounded-(--fx-radius-lg) text-(--fx-faint) transition-colors duration-(--fx-speed) hover:bg-(--fx-tab-hover) hover:text-(--fx-text) ' +
                    (vertical ? 'h-8 w-full px-2.5' : 'h-[30px] w-[30px] justify-center')
                }
            >
                <I.PlusIcon size={15} strokeWidth={1.8} />
                {vertical ? <span className={'truncate text-[0.95em] transition-opacity duration-(--fx-speed) ' + (rail ? 'opacity-0' : '')}>New tab</span> : null}
            </button>
        </div>
    );
});

/** Minimize, maximize and close, drawn as quietly as the rest, since the window has no title bar of its own. */
export const WindowButtons = memo(function WindowButtons() {
    const state = useWindowState();
    const button = 'flex h-full w-[46px] items-center justify-center text-(--fx-muted) transition-colors duration-(--fx-speed) ';
    return (
        <div className="flex h-full shrink-0 items-stretch">
            <button title="Minimize" aria-label="Minimize" className={button + 'hover:bg-(--fx-hover) hover:text-(--fx-text)'} onClick={() => mainWindow.minimize()}>
                <span className="h-px w-[10px] bg-current" />
            </button>
            <button
                title={state.maximized ? 'Restore' : 'Maximize'}
                aria-label={state.maximized ? 'Restore' : 'Maximize'}
                className={button + 'hover:bg-(--fx-hover) hover:text-(--fx-text)'}
                onClick={() => (state.maximized ? mainWindow.restore() : mainWindow.maximize())}
            >
                {state.maximized ? (
                    <span className="relative h-[10px] w-[10px]">
                        <span className="absolute top-0 right-0 h-[8px] w-[8px] rounded-[2px] border border-current" />
                        <span className="absolute bottom-0 left-0 h-[8px] w-[8px] rounded-[2px] border border-current bg-(--fx-frame)" />
                    </span>
                ) : (
                    <span className="h-[10px] w-[10px] rounded-[2px] border border-current" />
                )}
            </button>
            <button title="Close" aria-label="Close" className={button + 'hover:bg-[#e81123] hover:text-white'} onClick={() => mainWindow.close()}>
                <I.XIcon size={13} strokeWidth={1.6} />
            </button>
        </div>
    );
});

/**
 * A handle between two panels, which resizes them as it's dragged: `onMove` is told how far the pointer's gone across since it
 * went down, and `onDone` when it's let go. It's the gap between them, with a line that shows while it's hovered or dragged.
 */
export const Splitter = memo(function Splitter(props: {
    gap: number;
    onStart: () => void;
    onMove: (by: number) => void;
    onDone: () => void;
    onReset?: () => void;
    disabled?: boolean;
}) {
    const { gap, onStart, onMove, onDone, onReset, disabled } = props;
    const from = useRef<number | null>(null);
    const [held, setHeld] = useState(false);
    return (
        <div
            className={'group relative z-10 flex shrink-0 items-center justify-center self-stretch ' + (disabled ? 'pointer-events-none' : 'cursor-col-resize')}
            style={{ width: Math.max(gap, 6), marginLeft: gap < 6 ? (gap - 6) / 2 : 0, marginRight: gap < 6 ? (gap - 6) / 2 : 0 }}
            onPointerDown={(event: any) => {
                if (disabled || !leftButton(event)) {
                    return;
                }
                from.current = event.clientX;
                setHeld(true);
                onStart();
            }}
            onPointerMove={(event: any) => {
                if (from.current !== null) {
                    onMove(event.clientX - from.current);
                }
            }}
            onPointerUp={() => {
                if (from.current === null) {
                    return;
                }
                from.current = null;
                setHeld(false);
                onDone();
            }}
            onDoubleClick={disabled ? undefined : onReset}
        >
            <span
                className={
                    'h-10 w-[3px] rounded-full transition-opacity duration-(--fx-speed) ' +
                    (held ? 'bg-(--fx-accent) opacity-100' : 'bg-(--fx-faint) opacity-0 group-hover:opacity-60')
                }
            />
        </div>
    );
});

const Row = memo(function Row(props: { label: string; children: any }) {
    return (
        <div className="flex min-h-9 items-center justify-between gap-4">
            <span className="text-(--fx-muted)">{props.label}</span>
            {props.children}
        </div>
    );
});

/**
 * How it looks, in one place, as Arc's theme picker is: the theme, the accent, the window's material and how much of the desktop
 * it shows, the frame's tint, density, corners, text size, and where the tabs go. Each changes as it's chosen. Everything else is
 * in the settings, a click away.
 */
export const Customize = memo(function Customize(props: {
    settings: Settings;
    change: <K extends keyof Settings>(key: K, value: Settings[K]) => void;
    onClose: () => void;
    onAllSettings: () => void;
}) {
    const { settings: s, change, onClose, onAllSettings } = props;
    return (
        <>
            <div className="absolute inset-0 z-30" onClick={onClose} />
            <div
                className="absolute top-[46px] right-3 z-40 flex w-[340px] flex-col gap-1 rounded-(--fx-radius-lg) bg-(--fx-pop) p-4 text-[0.95em] shadow-(--fx-pop-shadow)"
                onKeyDown={(event: any) => event.key === 'Escape' && onClose()}
            >
                <div className="mb-1 flex items-center justify-between">
                    <span className="font-semibold text-(--fx-text)">Customize</span>
                    <IconButton icon={I.XIcon} label="Close" size={13} onClick={onClose} />
                </div>
                <div className="mb-2 flex flex-wrap gap-2">
                    {ACCENTS.map(([color, name]) => (
                        <button
                            key={color}
                            title={name}
                            aria-label={name}
                            onClick={() => change('accent', color)}
                            className={
                                'h-6 w-6 rounded-full transition-transform duration-(--fx-speed) hover:scale-110 ' +
                                (s.accent.toLowerCase() === color ? 'ring-2 ring-(--fx-text) ring-offset-2 ring-offset-(--fx-pop)' : '')
                            }
                            style={{ background: color }}
                        />
                    ))}
                </div>
                <Row label="Theme">
                    <Segmented small value={s.theme} options={[['system', 'Auto'], ['light', 'Light'], ['dark', 'Dark']]} onChange={(value) => change('theme', value)} />
                </Row>
                <Row label="Material">
                    <Segmented small value={s.backdrop} options={[['none', 'Solid'], ['mica', 'Mica'], ['acrylic', 'Acrylic']]} onChange={(value) => change('backdrop', value)} />
                </Row>
                {s.backdrop !== 'none' ? (
                    <Row label="See-through">
                        <Stepper value={s.translucency} min={0} max={100} step={10} unit="%" onChange={(value) => change('translucency', value)} />
                    </Row>
                ) : null}
                <Row label="Frame tint">
                    <Stepper value={s.frameTint} min={0} max={60} step={5} unit="%" onChange={(value) => change('frameTint', value)} />
                </Row>
                <Row label="Tabs">
                    <Segmented small value={s.tabsAt} options={[['top', 'Top'], ['side', 'Sidebar']]} onChange={(value) => change('tabsAt', value)} />
                </Row>
                <Row label="Density">
                    <Segmented small value={s.density} options={[['compact', 'Compact'], ['cozy', 'Cozy'], ['comfortable', 'Roomy']]} onChange={(value) => change('density', value)} />
                </Row>
                <Row label="Corners">
                    <Segmented small value={s.radius} options={[['none', 'None'], ['small', 'S'], ['medium', 'M'], ['large', 'L']]} onChange={(value) => change('radius', value)} />
                </Row>
                <Row label="Space">
                    <Stepper value={s.panelGap} min={0} max={16} step={2} unit="px" onChange={(value) => change('panelGap', value)} />
                </Row>
                <Row label="Text size">
                    <Stepper value={s.fontSize} min={11} max={18} step={1} unit="px" onChange={(value) => change('fontSize', value)} />
                </Row>
                <Row label="Shadows">
                    <Toggle label="Shadows" on={s.panelShadow} onChange={(on) => change('panelShadow', on)} />
                </Row>
                <button
                    onClick={onAllSettings}
                    className="mt-2 flex h-8 items-center justify-center gap-2 rounded-(--fx-radius) bg-(--fx-input) text-(--fx-muted) transition-colors duration-(--fx-speed) hover:bg-(--fx-press) hover:text-(--fx-text)"
                >
                    <I.SettingsIcon size={14} strokeWidth={1.7} />
                    All settings
                </button>
            </div>
        </>
    );
});
