// A file explorer: fast on folders of any size, made to look and work like a modern browser, and configurable down to the last
// detail.
//
//   rsui-tsx app.tsx
//
// The window is a frame with panels on it, as Arc's is: tabs along the top (or down the sidebar), the sidebar on the frame, and each
// tab one folder or several, split side by side and above one another in any arrangement, with the preview beside them.
// Everything between them can be dragged to resize it, and the sidebar dragged narrower than it goes collapses to a rail of
// icons. How it all looks is in Customize (Ctrl+K), with every other setting a click further.
//
// Folders are read in one pass on another thread (rsui's `files.list`), only the rows on screen are drawn, and each row renders
// again only when something about it changed: see src/Pane.tsx and src/List.tsx. How a tab's panes are laid out is
// src/layout.ts.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { mainWindow, useColorScheme } from 'rsui';
import { PageWindow } from 'rsui/web';
import './app.css';

import { COMPUTER, nameOf, normalize, readDrives, type Drive } from './src/files';
import { Customize, Splitter, Tabs, WindowButtons, type TabView } from './src/Chrome';
import * as I from './src/icons';
import { keyOf } from './src/keys';
import {
    beside, dirOf, dragged, evened, findSplit, fits, heir, insert, MAX_PANES, neighbor, pane, paneIds, place, PRESETS, presetLayout, presetOf,
    remove, sideAt, smallest, splitId, tidy, withSizes, type Handle as HandleBox, type Layout, type Preset, type Rect, type Side,
} from './src/layout';
import type { Look } from './src/List';
import { Pane, useStable, WINDOW_COMMANDS, type PaneReport } from './src/Pane';
import { DockLayer, Handle, LayoutMenu, Notice, Slot } from './src/Panes';
import { Preview } from './src/Preview';
import {
    COMMANDS, loadSession, loadSettings, saveSession, saveSettings, type Command, type PaneState, type Session, type Settings, type TabState,
} from './src/settings';
import { SettingsPage } from './src/SettingsPage';
import { driveIcon, HOME, Sidebar, type SidebarOrder } from './src/Sidebar';
import { applyVariables, colorsOf, cssVariables, fontStack, gridTile, radiusOf, rowHeight } from './src/theme';
import { IconButton } from './src/ui';

export const title = 'Files';
export const size = [1240, 780];
// It draws its own title bar: the tabs, with the window's buttons beside them.
export const frameless = true;

/** No order of its own for the sidebar yet: as it comes. */
const NO_ORDER: SidebarOrder = {};

/** The sidebar collapsed: a rail just wide enough for its icons, each in the middle of what's lit behind it, which is as wide as
 * the button that hides the sidebar, above it: 6.5px of the frame (`pl-2`, at the 13px the page's rem is), then 8px (`px-2.5`)
 * either side of a 16px icon. */
const RAIL = 39;
const SIDEBAR_MIN = 170;
const SIDEBAR_MAX = 420;
/** Dragged narrower than this, the sidebar collapses; collapsed, dragged wider than the other, it opens again. The gap between them
 * keeps it from flickering between the two with the pointer on the edge. */
const COLLAPSE_BELOW = 112;
const EXPAND_ABOVE = 136;
/** A window narrower than this has the sidebar collapsed, whatever it's set to, so the panes keep their room. */
const RAIL_WINDOW = 760;
/** How long panels are moving to new places, after a split or the like, before they go back to following what changes at once. */
const MOVING_FOR = 420;

let made = 0;
const uid = () => Date.now().toString(36) + (made++).toString(36);
const paneAt = (path: string): PaneState => ({ id: uid(), stack: [path], at: 0 });
function tabAt(path: string): TabState {
    const first = paneAt(path);
    return { id: uid(), panes: [first], layout: pane(first.id), focus: first.id };
}
const folderOf = (pane: PaneState | undefined) => (pane ? pane.stack[pane.at] ?? HOME : HOME);
const focusOf = (tab: TabState) => (tab.panes.some((one) => one.id === tab.focus) ? tab.focus : tab.panes[0].id);

function startFolder(settings: Settings, session: Session): string {
    switch (settings.startAt) {
        case 'computer':
            return COMPUTER;
        case 'last':
            return session.lastFolder ?? HOME;
        case 'custom':
            return settings.startPath ? normalize(settings.startPath) : HOME;
        default:
            return HOME;
    }
}

/**
 * A tab as it was saved, with new ids so nothing's mixed up with what an earlier run made, or null if there's nothing usable in it.
 * Tabs saved before there were layouts (two panes at most, `split` a share of the width, `focus` an index) become a row.
 */
function restoredTab(saved: any): TabState | null {
    if (!saved || !Array.isArray(saved.panes)) {
        return null;
    }
    const ids = new Map<string, string>();
    const panes: PaneState[] = [];
    for (const one of saved.panes) {
        if (!one || !Array.isArray(one.stack) || panes.length >= MAX_PANES) {
            continue;
        }
        const whole = one.stack.filter((path: any) => typeof path === 'string' && path !== '');
        const stack = whole.slice(-50);
        if (stack.length === 0) {
            continue;
        }
        const id = uid();
        if (typeof one.id === 'string') {
            ids.set(one.id, id);
        }
        const at = Math.min(Math.max(0, (Number(one.at) || 0) - (whole.length - stack.length)), stack.length - 1);
        panes.push({ id, stack, at });
    }
    if (panes.length === 0) {
        return null;
    }
    const renamed = (node: any): any => {
        if (node?.kind === 'pane') {
            return ids.has(node.id) ? { kind: 'pane', id: ids.get(node.id) } : null;
        }
        if (node?.kind === 'split' && Array.isArray(node.children)) {
            return { ...node, id: splitId(), children: node.children.map(renamed) };
        }
        return null;
    };
    const all = panes.map((one) => one.id);
    let layout: Layout | null = saved.layout ? tidy(renamed(saved.layout)) : null;
    if (layout === null || paneIds(layout).length !== all.length) {
        const first = typeof saved.split === 'number' && all.length === 2 ? Math.min(0.8, Math.max(0.2, saved.split)) : 1 / all.length;
        layout =
            all.length === 1
                ? pane(all[0])
                : { kind: 'split', id: splitId(), dir: 'row', children: all.map(pane), sizes: all.length === 2 ? [first, 1 - first] : all.map(() => 1 / all.length) };
    }
    const focus =
        typeof saved.focus === 'string' && ids.has(saved.focus)
            ? ids.get(saved.focus)!
            : typeof saved.focus === 'number'
              ? all[Math.min(Math.max(0, saved.focus), all.length - 1)]
              : all[0];
    return { id: uid(), panes, layout, focus };
}

/** The tabs it opens with: those from last time, if it keeps them, or one where the settings say to start. */
function firstTabs(settings: Settings, session: Session): { tabs: TabState[]; active: number } {
    const env = process.env.FILE_EXPLORER_START;
    if (env) {
        return { tabs: [tabAt(normalize(env))], active: 0 };
    }
    if (settings.restoreTabs) {
        const tabs = (session.tabs ?? []).map(restoredTab).filter((tab): tab is TabState => tab !== null);
        if (tabs.length > 0) {
            return { tabs, active: Math.min(Math.max(0, session.activeTab ?? 0), tabs.length - 1) };
        }
    }
    return { tabs: [tabAt(startFolder(settings, session))], active: 0 };
}

// The window is a page, so what app.css says about `html` and `body` (the font and its size from the settings, the colors) goes on
// it, and text is laid out as a browser lays it out.
export default function Root() {
    return (
        <PageWindow>
            <App />
        </PageWindow>
    );
}

type Overlay = 'none' | 'customize' | 'settings' | 'layout';

/** What's being dragged and where it's got to, which is only kept once it's let go. */
interface Dragging {
    sidebar?: number;
    rail?: boolean;
    preview?: number;
    split?: { id: string; sizes: number[] };
}

/**
 * A tab being dragged (`id`, and where it is among the tabs), and when it's over the panes, the pane it's over (`*` for all of them,
 * near their outer edge), the side of it, and whether it can go there. It goes there with its panes and all, and is closed. The open
 * tab is already there, so it can't.
 */
interface Dock {
    index: number;
    id: string;
    over: boolean;
    target: string | null;
    side: Side;
    ok: boolean;
    why: string;
    rect: Rect | null;
}

/** The place a dragged tab's held open among the panes. */
const DOCK_ID = '~dock';
/** How near the edge of all the panes the pointer is, for a tab to go beside all of them rather than the one it's over. */
const DOCK_EDGE = 28;

function App() {
    const [settings, setSettings] = useState(loadSettings);
    const [session, setSession] = useState(loadSession);
    const [drives, setDrives] = useState<Drive[]>([]);
    const [{ tabs, active }, setTabState] = useState(() => firstTabs(settings, session));
    const [overlay, setOverlay] = useState<Overlay>('none');
    const [peek, setPeek] = useState<{ entry: PaneReport['entry']; count: number }>({ entry: null, count: 0 });
    const [dragging, setDragging] = useState<Dragging>({});
    const [dock, setDock] = useState<Dock | null>(null);
    const [windowSize, setWindowSize] = useState({ width: window.innerWidth, height: window.innerHeight });

    // Settings and what's remembered

    const change = useCallback(<K extends keyof Settings>(key: K, value: Settings[K]) => {
        setSettings((was) => {
            if (was[key] === value) {
                return was;
            }
            const next = { ...was, [key]: value };
            saveSettings(next);
            return next;
        });
    }, []);
    const replaceSettings = useCallback((next: Settings) => {
        saveSettings(next);
        setSettings(next);
    }, []);
    const updateSession = useCallback((update: (was: Session) => Session) => {
        setSession((was) => {
            const next = update(was);
            saveSession(next);
            return next;
        });
    }, []);

    const system = useColorScheme();
    const dark = settings.theme === 'system' ? system === 'dark' : settings.theme === 'dark';
    const colors = useMemo(() => colorsOf(settings, dark), [settings, dark]);
    applyVariables(cssVariables(settings, colors));
    useEffect(() => {
        const resized = () => setWindowSize({ width: window.innerWidth, height: window.innerHeight });
        window.addEventListener('resize', resized);
        return () => window.removeEventListener('resize', resized);
    }, []);
    useEffect(() => {
        try {
            mainWindow.setBackdrop(settings.backdrop);
        } catch {
            // Windows 10 has no backdrops.
        }
    }, [settings.backdrop]);
    const refreshDrives = useStable(() => readDrives().then(setDrives, () => {}));
    useEffect(() => {
        refreshDrives();
    }, []);

    // Panels gliding to new places and sizes, for a moment after something's split, closed, collapsed or shown, and not otherwise:
    // not while a window's resized or a handle dragged, which follow the pointer at once.
    const [moving, setMoving] = useState(false);
    const movingTimer = useRef<any>(null);
    const glide = useCallback(() => {
        setMoving(true);
        clearTimeout(movingTimer.current);
        movingTimer.current = setTimeout(() => setMoving(false), MOVING_FOR);
    }, []);
    useEffect(() => () => clearTimeout(movingTimer.current), []);

    // A note that goes by itself, as when there's no room to split a pane.
    const [notice, setNotice] = useState<string | null>(null);
    const noticeTimer = useRef<any>(null);
    const tell = useCallback((text: string) => {
        setNotice(text);
        clearTimeout(noticeTimer.current);
        noticeTimer.current = setTimeout(() => setNotice(null), 1800);
    }, []);

    const look: Look = useMemo(
        () => ({
            rowHeight: rowHeight(settings),
            iconSize: settings.iconSize,
            stroke: settings.iconStroke,
            iconStyle: settings.iconStyle,
            tintFolders: settings.tintFolders,
            colorFiles: settings.colorFileIcons,
            sharedIcons: settings.systemIconCache,
            showExtensions: settings.showExtensions,
            dimHidden: settings.dimHidden,
            zebra: settings.zebra,
            rowLines: settings.rowLines,
            selection: settings.selection,
            dateFormat: settings.dateFormat,
            clock24: settings.clock24,
            sizeUnits: settings.sizeUnits,
            cols: {
                size: settings.colSize,
                type: settings.colType,
                modified: settings.colModified,
                created: settings.colCreated,
                attributes: settings.colAttributes,
            },
            tile: gridTile(settings),
            overscan: settings.overscan,
            renameSelectsBase: settings.renameSelectsBase,
            colors,
            radius: radiusOf(settings),
            font: fontStack(settings),
            fontSize: settings.fontSize,
            hoverEffects: true,
            gridThumbnails: settings.gridThumbnails,
            thumbnailCache: settings.thumbnailCache,
        }),
        [settings, colors],
    );

    // Where everything goes

    const gap = settings.panelGap;
    const tab = tabs[active] ?? tabs[0];
    const focusId = focusOf(tab);
    const focused = tab.panes.find((one) => one.id === focusId)!;
    // Tabs down the side need a sidebar to be in; without one, they're along the top.
    const tabsOnSide = settings.tabsAt === 'side' && settings.showSidebar;
    const narrowWindow = windowSize.width < RAIL_WINDOW;
    const rail = dragging.rail ?? (settings.sidebarCollapsed || narrowWindow);
    const sidebarWidth = !settings.showSidebar ? 0 : dragging.sidebar ?? (rail ? RAIL : settings.sidebarWidth);
    const previewWidth = settings.showPreview ? dragging.preview ?? settings.previewWidth : 0;
    const strip = settings.tabsAt === 'top' || !tabsOnSide ? 44 : 40;
    const bodyHeight = Math.max(120, windowSize.height - strip - gap);
    // The handles beside the sidebar and the preview are the gaps there, and one's there when the sidebar's hidden too, as the gap.
    const panesWidth = Math.max(160, windowSize.width - sidebarWidth - gap - (settings.showPreview ? previewWidth + gap : 0) - gap);
    const view = tab;
    const viewFocus = focusOf(view);
    const layout = dragging.split ? withSizes(view.layout, dragging.split.id, dragging.split.sizes) : view.layout;
    // With a tab over the panes, where it'd land: worked out as if it were there, and shown over them, which stay as they are.
    const landing = useMemo(() => {
        if (dock === null || !dock.over || !dock.ok || dock.target === null) {
            return null;
        }
        const after = dock.target === '*' ? beside(layout, pane(DOCK_ID), dock.side) : insert(layout, dock.target, pane(DOCK_ID), dock.side);
        return place(after, { x: 0, y: 0, w: panesWidth, h: bodyHeight }, gap).panes.get(DOCK_ID) ?? null;
    }, [dock, layout, panesWidth, bodyHeight, gap]);
    const placed = useMemo(() => place(layout, { x: 0, y: 0, w: panesWidth, h: bodyHeight }, gap), [layout, panesWidth, bodyHeight, gap]);
    const animate = moving && dragging.split === undefined && dragging.sidebar === undefined;

    // Tabs, and the panes in each

    const setTabs = (update: (was: { tabs: TabState[]; active: number }) => { tabs: TabState[]; active: number }) => setTabState(update);
    const withTab = (update: (tab: TabState) => TabState) =>
        setTabs((was) => ({ ...was, tabs: was.tabs.map((one, at) => (at === was.active ? update(one) : one)) }));

    // What each pane was first given while it's shown, which it keeps to: what it's at now is its own, and only told here. One
    // that's shown again, after another tab, starts from where it was when it was last told.
    const firsts = useRef(new Map<string, PaneState>());
    const shownIds = new Set(tab.panes.map((one) => one.id));
    for (const id of [...firsts.current.keys()]) {
        if (!shownIds.has(id)) {
            firsts.current.delete(id);
        }
    }
    const initialOf = (state: PaneState) => {
        if (!firsts.current.has(state.id)) {
            firsts.current.set(state.id, state);
        }
        return firsts.current.get(state.id)!;
    };
    // Panes just split off another, which fade in. They're told so as they're first shown, and forgotten after.
    const fresh = useRef(new Set<string>());
    useEffect(() => {
        fresh.current.clear();
    });

    /** Where a pane beside `target` can go: on `side` if there's room, or the other way if there's room for that, or nowhere. */
    const roomFor = (target: string, side: Side, node: Layout = pane('')): Side | null => {
        const rect = placed.panes.get(target);
        if (fits(rect, side, gap, node)) {
            return side;
        }
        const other: Side = dirOf(side) === 'row' ? 'bottom' : 'right';
        return fits(rect, other, gap, node) ? other : null;
    };
    const presetFits = (preset: Preset) => {
        const count = PRESETS.find((one) => one.id === preset)?.count ?? 1;
        const shape = presetLayout(preset, Array.from({ length: count }, (_, at) => String(at)));
        return smallest(shape, 'row', gap) <= panesWidth && smallest(shape, 'column', gap) <= bodyHeight;
    };

    const act = {
        newTab: (path = folderOf(focused)) =>
            setTabs((was) => ({ tabs: [...was.tabs.slice(0, was.active + 1), tabAt(path), ...was.tabs.slice(was.active + 1)], active: was.active + 1 })),
        closeTab: (index: number) =>
            setTabs((was) => {
                if (was.tabs.length === 1) {
                    return { tabs: [tabAt(HOME)], active: 0 };
                }
                const tabs = was.tabs.filter((_, at) => at !== index);
                const active = index < was.active || was.active >= tabs.length ? Math.max(0, was.active - 1) : was.active;
                return { tabs, active };
            }),
        select: (index: number) => setTabs((was) => ({ ...was, active: Math.max(0, Math.min(was.tabs.length - 1, index)) })),
        // A tab dragged to another place, with the open one still open wherever it's gone.
        move: (from: number, to: number) =>
            setTabs((was) => {
                if (from === to || from < 0 || to < 0 || from >= was.tabs.length || to >= was.tabs.length) {
                    return was;
                }
                const open = was.tabs[was.active].id;
                const tabs = was.tabs.slice();
                const [moved] = tabs.splice(from, 1);
                tabs.splice(to, 0, moved);
                return { tabs, active: tabs.findIndex((one) => one.id === open) };
            }),
        cycle: (by: number) => setTabs((was) => ({ ...was, active: (was.active + by + was.tabs.length) % was.tabs.length })),
        /** A new pane beside `target` (the one with the keyboard), on `side`, or the other way if there's only room for that. */
        split: (side: Side, path?: string, target = focusId): boolean => {
            if (tab.panes.length >= MAX_PANES) {
                tell(`A tab can have ${MAX_PANES} panes at most`);
                return false;
            }
            const where = roomFor(target, side);
            if (where === null) {
                tell('There isn’t room to split this pane');
                return false;
            }
            const made = paneAt(path ?? folderOf(tab.panes.find((one) => one.id === target)));
            fresh.current.add(made.id);
            glide();
            withTab((one) => ({ ...one, panes: [...one.panes, made], layout: insert(one.layout, target, pane(made.id), where), focus: made.id }));
            return true;
        },
        closePane: (id = focusId) => {
            if (tab.panes.length <= 1) {
                return;
            }
            const next = remove(tab.layout, id);
            if (next === null) {
                return;
            }
            const takes = heir(tab.layout, id) ?? paneIds(next)[0];
            glide();
            withTab((one) => ({ ...one, panes: one.panes.filter((p) => p.id !== id), layout: next, focus: one.focus === id ? takes : one.focus }));
        },
        preset: (preset: Preset) => {
            if (!presetFits(preset)) {
                tell('The window is too small for that layout');
                return;
            }
            const count = PRESETS.find((one) => one.id === preset)?.count ?? 1;
            const order = paneIds(tab.layout);
            // Fewer panes keeps the one with the keyboard and those first in order; more opens new ones where it is.
            const keep =
                order.length <= count
                    ? order
                    : [focusId, ...order.filter((id) => id !== focusId)].slice(0, count).sort((a, b) => order.indexOf(a) - order.indexOf(b));
            const added = Array.from({ length: count - keep.length }, () => paneAt(folderOf(focused)));
            added.forEach((one) => fresh.current.add(one.id));
            const next = presetLayout(preset, [...keep, ...added.map((one) => one.id)]);
            glide();
            withTab((one) => ({
                ...one,
                panes: [...one.panes.filter((p) => keep.includes(p.id)), ...added],
                layout: next,
                focus: keep.includes(one.focus) ? one.focus : keep[0] ?? added[0].id,
            }));
        },
        even: () => {
            glide();
            withTab((one) => ({ ...one, layout: evened(one.layout) }));
        },
        focus: (id: string) => withTab((one) => (one.focus === id || !one.panes.some((p) => p.id === id) ? one : { ...one, focus: id })),
        nextPane: () => {
            const order = paneIds(tab.layout);
            act.focus(order[(order.indexOf(focusId) + 1) % order.length]);
        },
        paneToward: (side: Side) => {
            const next = neighbor(placed.panes, focusId, side);
            if (next) {
                act.focus(next);
            }
        },
        sidebar: () => {
            glide();
            change('showSidebar', !settings.showSidebar);
        },
        collapse: () => {
            glide();
            if (!settings.showSidebar) {
                change('showSidebar', true);
                return;
            }
            if (narrowWindow) {
                tell('The window is too narrow for the whole sidebar');
                return;
            }
            change('sidebarCollapsed', !settings.sidebarCollapsed);
        },
        preview: () => {
            glide();
            change('showPreview', !settings.showPreview);
        },
    };

    // A tab being dragged: where it'd go, from where the pointer is over the panes, as they are without it.
    const panesBox = useRef<any>(null);
    const dockAt = (at: { index: number; x: number; y: number }): Dock => {
        const from = tabs[at.index];
        const none: Dock = { index: at.index, id: from?.id ?? '', over: false, target: null, side: 'right', ok: false, why: '', rect: null };
        const box = panesBox.current?.getBoundingClientRect?.();
        // Over the panes only while they're shown: not behind the settings.
        if (!from || !box || latest.current?.overlay === 'settings') {
            return none;
        }
        const node = from.layout;
        const count = from.panes.length;
        const [x, y] = [at.x - box.left, at.y - box.top];
        const whole = { x: 0, y: 0, w: panesWidth, h: bodyHeight };
        if (x < 0 || y < 0 || x >= whole.w || y >= whole.h) {
            return none;
        }
        const into = tab;
        // The tab that's open is what's shown: it's already here, and can't go beside itself.
        if (from.id === into.id) {
            return { ...none, over: true, rect: whole, why: 'This tab is already open here' };
        }
        if (into.panes.length + count > MAX_PANES) {
            return { ...none, over: true, rect: whole, side: sideAt(whole, x, y), why: `A tab can have ${MAX_PANES} panes at most` };
        }
        const rects = place(into.layout, whole, gap).panes;
        // Near the outer edge of several panes, it goes beside all of them.
        const edge = Math.min(x, y, whole.w - x, whole.h - y) < DOCK_EDGE;
        if (edge && into.panes.length > 1) {
            const side = sideAt(whole, x, y);
            const ok = fits(whole, side, gap, node, smallest(into.layout, 'row', gap), smallest(into.layout, 'column', gap));
            return { ...none, over: true, target: '*', side, ok, why: 'There isn’t room for it there', rect: whole };
        }
        for (const [id, rect] of rects) {
            const half = gap / 2 + 1;
            if (x >= rect.x - half && x < rect.x + rect.w + half && y >= rect.y - half && y < rect.y + rect.h + half) {
                const side = sideAt(rect, x, y);
                return { ...none, over: true, target: id, side, ok: fits(rect, side, gap, node), why: 'There isn’t room for it there', rect };
            }
        }
        return { ...none, over: true };
    };

    const on = {
        report: useStable((report: PaneReport) => {
            setTabs((was) => {
                let changed = false;
                const tabs = was.tabs.map((one) => {
                    const at = one.panes.findIndex((p) => p.id === report.state.id);
                    if (at < 0) {
                        return one;
                    }
                    const was = one.panes[at];
                    if (was.at === report.state.at && was.stack.length === report.state.stack.length && folderOf(was) === folderOf(report.state)) {
                        return one;
                    }
                    changed = true;
                    const panes = one.panes.slice();
                    panes[at] = report.state;
                    return { ...one, panes };
                });
                return changed ? { ...was, tabs } : was;
            });
            if (report.state.id === focusId) {
                setPeek({ entry: report.entry, count: report.count });
            }
        }),
        activate: useStable((id: string) => act.focus(id)),
        openTab: useStable((path: string) => act.newTab(path)),
        // Opening a folder beside where it was clicked, or in a new tab when there's no room beside it.
        openSplit: useStable((path: string, side: 'right' | 'bottom') => {
            if (tab.panes.length >= MAX_PANES || roomFor(focusId, side) === null) {
                act.newTab(path);
                tell('No room beside it, so it’s opened in a new tab');
                return;
            }
            act.split(side, path);
        }),
        closePane: useStable((id: string) => act.closePane(id)),
        open: useStable((path: string) => {
            // The sidebar opens what it's clicked on in the pane that has the keyboard, which is told by its key changing.
            withTab((one) => {
                const target = focusOf(one);
                const panes = one.panes.slice();
                const at = panes.findIndex((p) => p.id === target);
                const was = panes[at];
                const stack = was.stack.slice(0, was.at + 1).concat(path);
                const made = { id: uid(), stack, at: stack.length - 1 };
                panes[at] = made;
                const swap = (node: Layout): Layout =>
                    node.kind === 'pane' ? (node.id === target ? pane(made.id) : node) : { ...node, children: node.children.map(swap) };
                return { ...one, panes, layout: swap(one.layout), focus: made.id };
            });
        }),
        unpin: useStable((path: string) => updateSession((was) => ({ ...was, pinned: was.pinned.filter((one) => one !== path) }))),
        pinned: useStable((pinned: string[]) => updateSession((was) => ({ ...was, pinned }))),
        order: useStable((sidebarOrder: SidebarOrder) => updateSession((was) => ({ ...was, sidebarOrder }))),
        select: useStable((index: number) => act.select(index)),
        close: useStable((index: number) => act.closeTab(index)),
        move: useStable((from: number, to: number) => act.move(from, to)),
        newTab: useStable(() => act.newTab()),
        sidebar: useStable(() => act.sidebar()),
        preview: useStable(() => act.preview()),
        layoutMenu: useStable(() => setOverlay((was) => (was === 'layout' ? 'none' : 'layout'))),
        customize: useStable(() => setOverlay((was) => (was === 'customize' ? 'none' : 'customize'))),
        closeOverlay: useStable(() => setOverlay('none')),
        settings: useStable(() => setOverlay('settings')),
        preset: useStable((preset: Preset) => act.preset(preset)),
        splitFocused: useStable((side: 'right' | 'bottom') => act.split(side)),
        even: useStable(() => act.even()),
        closeFocused: useStable(() => act.closePane()),
        // Where a tab being dragged is, which is shown over the panes as it moves; null when it's let go.
        dock: useStable((at: { index: number; x: number; y: number } | null): boolean => {
            const was = latest.current.dock as Dock | null;
            const next = at === null ? null : dockAt(at);
            const same =
                was !== null &&
                next !== null &&
                (['index', 'id', 'over', 'target', 'side', 'ok', 'why'] as const).every((key) => was[key] === next[key]) &&
                JSON.stringify(was.rect) === JSON.stringify(next.rect);
            if (!same && !(was === null && next === null)) {
                latest.current.dock = next;
                setDock(next);
            }
            return next?.over === true;
        }),
        dockDrop: useStable((index: number) => {
            const where = latest.current.dock as Dock | null;
            if (!where || !where.over || !where.ok || where.target === null || where.index !== index) {
                return;
            }
            const put = (layout: Layout, node: Layout) =>
                where.target === '*' ? beside(layout, node, where.side) : insert(layout, where.target!, node, where.side);
            // The tab it was worked out for, wherever it is now, and still not the open one.
            const from = tabs.find((one) => one.id === where.id);
            const into = tab;
            if (!from || from.id === into.id) {
                return;
            }
            from.panes.forEach((one) => fresh.current.add(one.id));
            glide();
            setTabs((was) => {
                const moving = was.tabs.find((one) => one.id === from.id);
                const host = was.tabs[was.active];
                if (!moving || !host || moving.id === host.id) {
                    return was;
                }
                // Only beside a pane that's still there, and with none of its panes in both: anything else is left as it was.
                const there = new Set(paneIds(host.layout));
                if ((where.target !== '*' && !there.has(where.target!)) || moving.panes.some((one) => there.has(one.id))) {
                    return was;
                }
                if (host.panes.length + moving.panes.length > MAX_PANES) {
                    return was;
                }
                const merged: TabState = {
                    ...host,
                    panes: [...host.panes, ...moving.panes],
                    layout: put(host.layout, moving.layout),
                    focus: focusOf(moving),
                };
                const tabs = was.tabs.filter((one) => one.id !== moving.id).map((one) => (one.id === host.id ? merged : one));
                return { tabs, active: tabs.findIndex((one) => one.id === merged.id) };
            });
        }),
    };

    // What's remembered of the tabs, and the window's title.
    useEffect(() => {
        const folder = folderOf(focused);
        document.title = (folder === COMPUTER ? 'This PC' : nameOf(folder)) + ' - Files';
        updateSession((was) => ({
            ...was,
            tabs: tabs.map((one) => ({ ...one, panes: one.panes.map((p) => ({ ...p, stack: p.stack.slice(-30), at: Math.min(p.at, 29) })) })),
            activeTab: active,
            lastFolder: folder,
        }));
    }, [tabs, active]);

    // The window's own shortcuts, whichever pane has the keyboard. The rest are each pane's: see `Pane`.
    const latest = useRef<any>(null);
    latest.current = { act, settings, overlay, on, active, dock };
    useEffect(() => {
        const heard = (event: any) => {
            const now = latest.current;
            const shortcut = keyOf(event);
            if (!shortcut) {
                return;
            }
            if (shortcut === 'Escape' && (now.overlay === 'customize' || now.overlay === 'layout')) {
                setOverlay('none');
                return;
            }
            const command = (Object.keys(COMMANDS) as Command[]).find((one) => now.settings.keys[one] === shortcut);
            if (!command || !WINDOW_COMMANDS.has(command) || (now.overlay === 'settings' && command !== 'settings')) {
                return;
            }
            event.preventDefault();
            switch (command) {
                case 'newTab': return now.act.newTab();
                case 'closeTab': return now.act.closeTab(now.active);
                case 'nextTab': return now.act.cycle(1);
                case 'previousTab': return now.act.cycle(-1);
                case 'split': return now.act.split('right');
                case 'splitDown': return now.act.split('bottom');
                case 'closePane': return now.act.closePane();
                case 'otherPane': return now.act.nextPane();
                case 'paneLeft': return now.act.paneToward('left');
                case 'paneRight': return now.act.paneToward('right');
                case 'paneUp': return now.act.paneToward('top');
                case 'paneDown': return now.act.paneToward('bottom');
                case 'evenPanes': return now.act.even();
                case 'collapseSidebar': return now.act.collapse();
                case 'customize': return now.on.customize();
                case 'settings': return setOverlay((was) => (was === 'settings' ? 'none' : 'settings'));
                case 'toggleSidebar': return now.act.sidebar();
                case 'togglePreview': return now.act.preview();
            }
        };
        document.addEventListener('keydown', heard);
        return () => document.removeEventListener('keydown', heard);
    }, []);

    // Resizing what's beside what, by the handles between them

    const base = useRef<{ width: number; rail: boolean; sizes: number[]; room: number }>({ width: 0, rail: false, sizes: [], room: 1 });
    const resize = {
        sidebarStart: useStable(() => {
            base.current = { ...base.current, width: sidebarWidth, rail };
        }),
        // It follows the pointer between its narrowest and widest. Pulled well past the narrowest it collapses to a rail, gliding
        // there, and pulled back out of the rail it opens again, gliding to where the pointer is.
        sidebarMove: useStable((by: number) => {
            if (narrowWindow) {
                return;
            }
            const raw = base.current.width + by;
            const wasRail = dragging.rail ?? base.current.rail;
            const nowRail = wasRail ? raw < EXPAND_ABOVE : raw < COLLAPSE_BELOW;
            if (nowRail !== wasRail) {
                glide();
            }
            setDragging({
                sidebar: nowRail ? RAIL : Math.round(Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, raw))),
                rail: nowRail,
            });
        }),
        sidebarDone: useStable(() => {
            if (dragging.rail !== undefined) {
                change('sidebarCollapsed', dragging.rail);
                if (!dragging.rail && dragging.sidebar !== undefined) {
                    change('sidebarWidth', dragging.sidebar);
                }
            }
            setDragging({});
        }),
        sidebarReset: useStable(() => act.collapse()),
        previewStart: useStable(() => {
            base.current = { ...base.current, width: previewWidth };
        }),
        previewMove: useStable((by: number) => setDragging({ preview: Math.round(Math.min(640, Math.max(220, base.current.width - by))) })),
        previewDone: useStable(() => {
            if (dragging.preview !== undefined) change('previewWidth', dragging.preview);
            setDragging({});
        }),
        handleStart: useStable((handle: HandleBox) => {
            const split = findSplit(tab.layout, handle.split);
            base.current = { ...base.current, sizes: split ? split.sizes.slice() : [], room: handle.room };
        }),
        handleMove: useStable((handle: HandleBox, by: number) => {
            const sizes = dragged(tab.layout, handle.split, handle.index, base.current.sizes, by, base.current.room, gap);
            setDragging({ split: { id: handle.split, sizes } });
        }),
        handleDone: useStable(() => {
            const done = dragging.split;
            if (done) withTab((one) => ({ ...one, layout: withSizes(one.layout, done.id, done.sizes) }));
            setDragging({});
        }),
        // A double-click evens out the row or column it's in.
        handleReset: useStable((handle: HandleBox) => {
            const split = findSplit(tab.layout, handle.split);
            if (!split) return;
            glide();
            withTab((one) => ({ ...one, layout: withSizes(one.layout, handle.split, split.children.map(() => 1)) }));
        }),
    };

    // Drawing

    const views: TabView[] = useMemo(
        () =>
            tabs.map((one) => {
                const folder = folderOf(one.panes.find((p) => p.id === focusOf(one)));
                const drive = drives.find((d) => d.path.toLowerCase() === folder.toLowerCase());
                return {
                    id: one.id,
                    title: folder === COMPUTER ? 'This PC' : drive ? (drive.label || 'Local Disk') + ' (' + drive.path.slice(0, 2) + ')' : nameOf(folder),
                    icon: folder === COMPUTER ? I.ComputerIcon : drive ? driveIcon(drive.kind) : I.FolderIcon,
                    panes: one.panes.length,
                };
            }),
        [tabs, drives],
    );
    const tabList = (
        <Tabs
            tabs={views}
            active={active}
            vertical={tabsOnSide}
            rail={tabsOnSide && rail}
            onSelect={on.select}
            onClose={on.close}
            onNew={on.newTab}
            onMove={on.move}
            onDock={on.dock}
            onDockDrop={on.dockDrop}
        />
    );
    const stroke = settings.iconStroke + 0.15;
    const several = tab.panes.length > 1;
    const focusRect = placed.panes.get(focusId);

    return (
        <div className={'relative flex h-screen flex-col overflow-hidden bg-(--fx-frame) text-(--fx-text) ' + (dark ? 'dark' : '')}>
            <div className="flex shrink-0 items-center [app-region:drag]" style={{ height: strip }}>
                <div className="flex h-full min-w-0 flex-1 items-center gap-1 pr-3 pl-2">
                    {/* As big as what's in the sidebar below it, and in line with it, collapsed or not. */}
                    <IconButton icon={I.SidebarIcon} label={settings.showSidebar ? 'Hide the sidebar' : 'Show the sidebar'} stroke={stroke} box={32} onClick={on.sidebar} />
                    {!tabsOnSide && overlay !== 'settings' ? <div className="ml-1 flex min-w-0 flex-1">{tabList}</div> : <div className="flex-1" />}
                </div>
                <div className="flex h-full shrink-0 items-center gap-0.5 pr-1.5">
                    {overlay === 'settings' ? null : (
                        <IconButton icon={I.LayoutIcon} label="Layout" stroke={stroke} active={overlay === 'layout' || several} onClick={on.layoutMenu} />
                    )}
                    <IconButton icon={I.PreviewIcon} label="Preview" stroke={stroke} active={settings.showPreview} onClick={on.preview} />
                    <IconButton icon={I.PaletteIcon} label="Customize" stroke={stroke} active={overlay === 'customize'} onClick={on.customize} />
                </div>
                <WindowButtons />
            </div>
            <div className="flex min-h-0 flex-1" style={{ paddingRight: gap, paddingBottom: gap, paddingLeft: overlay === 'settings' ? gap : 0 }}>
                {overlay === 'settings' ? (
                    <SettingsPage
                        settings={settings}
                        change={change}
                        replace={replaceSettings}
                        pinned={session.pinned}
                        folder={folderOf(focused)}
                        onPinned={on.pinned}
                        onClose={on.closeOverlay}
                    />
                ) : (
                    <>
                        <Sidebar
                            settings={settings}
                            folder={folderOf(focused)}
                            pinned={session.pinned}
                            drives={drives}
                            width={sidebarWidth}
                            rail={rail}
                            animate={moving}
                            order={session.sidebarOrder ?? NO_ORDER}
                            tabs={tabsOnSide ? tabList : undefined}
                            onOpen={on.open}
                            onUnpin={on.unpin}
                            onPinned={on.pinned}
                            onOrder={on.order}
                        />
                        <Splitter
                            gap={gap}
                            disabled={!settings.showSidebar || narrowWindow}
                            onStart={resize.sidebarStart}
                            onMove={resize.sidebarMove}
                            onDone={resize.sidebarDone}
                            onReset={resize.sidebarReset}
                        />
                        <div
                            ref={panesBox}
                            className={'relative shrink-0 self-start ' + (moving ? 'transition-[width] duration-(--fx-move) ease-(--fx-ease)' : '')}
                            style={{ width: panesWidth, height: bodyHeight }}
                        >
                            {view.panes.map((state) => {
                                const rect = placed.panes.get(state.id);
                                if (!rect) {
                                    return null;
                                }
                                return (
                                    <Slot key={state.id} rect={rect} animate={animate} fresh={fresh.current.has(state.id)}>
                                        <Pane
                                            initial={initialOf(state)}
                                            settings={settings}
                                            session={session}
                                            drives={drives}
                                            look={look}
                                            active={state.id === viewFocus}
                                            keysOff={overlay !== 'none'}
                                            width={rect.w}
                                            height={rect.h}
                                            animate={animate}
                                            marked={view.panes.length > 1 && state.id === viewFocus}
                                            change={change}
                                            updateSession={updateSession}
                                            onReport={on.report}
                                            onActivate={on.activate}
                                            onOpenTab={on.openTab}
                                            onOpenSplit={on.openSplit}
                                            onClose={view.panes.length > 1 ? on.closePane : undefined}
                                            onRefreshDrives={refreshDrives}
                                        />
                                    </Slot>
                                );
                            })}
                            {placed.handles.map((handle) => (
                                <Handle
                                    key={handle.split + ':' + handle.index}
                                    handle={handle}
                                    animate={animate}
                                    onStart={resize.handleStart}
                                    onMove={resize.handleMove}
                                    onDone={resize.handleDone}
                                    onReset={resize.handleReset}
                                />
                            ))}
                            {/* Only a tab that could go there shows they're somewhere it can go: not the one that's open. */}
                            <DockLayer
                                lifting={dock !== null && dock.id !== tab.id}
                                landing={landing}
                                refused={dock && dock.over && !dock.ok && dock.why && dock.rect ? { rect: dock.rect, why: dock.why } : null}
                                what={dock && views[dock.index] ? { icon: views[dock.index].icon, title: views[dock.index].title } : null}
                                width={panesWidth}
                                height={bodyHeight}
                            />
                        </div>
                        {settings.showPreview ? (
                            <>
                                <Splitter gap={gap} onStart={resize.previewStart} onMove={resize.previewMove} onDone={resize.previewDone} />
                                <Preview entry={peek.entry} count={peek.count} settings={settings} now={Date.now()} colors={colors} width={previewWidth} />
                            </>
                        ) : null}
                    </>
                )}
            </div>
            {overlay === 'customize' ? <Customize settings={settings} change={change} onClose={on.closeOverlay} onAllSettings={on.settings} /> : null}
            {overlay === 'layout' ? (
                <LayoutMenu
                    settings={settings}
                    layout={tab.layout}
                    focus={focusId}
                    current={presetOf(tab.layout)}
                    count={tab.panes.length}
                    canPreset={presetFits}
                    canSplit={{
                        right: tab.panes.length < MAX_PANES && fits(focusRect, 'right', gap),
                        bottom: tab.panes.length < MAX_PANES && fits(focusRect, 'bottom', gap),
                    }}
                    onPreset={on.preset}
                    onSplit={on.splitFocused}
                    onEven={on.even}
                    onClosePane={on.closeFocused}
                    onClose={on.closeOverlay}
                />
            ) : null}
            <Notice text={notice} />
        </div>
    );
}
