// The window around the shells, in each of its layouts:
//
// - normal: one shell at a time on a panel, under a quiet bar with its name and folder
// - tabs: the same, with each shell a tab that joins the panel, as a folder's tab does
// - sidebar: every shell listed down the left with what it's doing, as a list of threads
// - strip: shells side by side as columns that slide, with the neighbors' edges showing
// - minimal: the panel and nothing else; a bar fades in when the pointer is at the top
//
// Everything here takes its colors from `tokens`, so every layout is the same few greys: the frame, the panel a step lighter,
// text in three strengths, and the faint fills for hover and what's chosen.

import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { mainWindow, useWindowState } from 'rsui';
import { ColumnCard, Strip, StripDots } from '../strip/Strip';
import { paneDropAt, paneHalf, paneRects, paneStyle, stackedPanes } from '../strip/panes';
import type { PaneRect, PaneSide } from '../strip/panes';
import { PaneResizePausedContext } from '../term/TerminalPane';
import type { PaneActions } from '../term/TerminalPane';
import type { Session } from '../term/Session';
import type { Workspace } from '../workspace';
import type { Settings } from '../settings';
import { css, tokens, withAlpha } from '../themes';
import type { Scheme } from '../themes';
import { Icon } from './Icon';
import type { IconName } from './Icon';
import { IconButton, Kbd, withShortcut } from './controls';
import type { IconButtonProps } from './controls';
import { TabbedPanel } from './TabbedPanel';
import type { PanelPlace, PanelTab } from './TabbedPanel';
import { useReorder } from './reorder';
import type { ReorderOutside, TabDragProps } from './reorder';
import { Background, backgroundOptions } from './Background';
import { PageNav, hasNav, pageLabel } from './Pages';
import { RecordingSidebar, RecordingSidebarContext, createRecordingSidebar } from './RecordingLayout';
import { SpaceButton } from './Spaces';
import type { Anchor } from './Spaces';
import type { Column } from '../workspace';

export interface ChromeHandlers {
    shortcut: (id: string) => string | undefined;
    onNew: () => void;
    onProfiles: () => void;
    onSearch: () => void;
    onPalette: () => void;
    onSettings: () => void;
    onQuake: () => void;
    onStudio: () => void;
    /** Starts or stops recording the terminal tab. */
    onRecord: () => void;
    onRecordings: () => void;
    beforeClose: () => Promise<void>;
    onBrowser: () => void;
    onRoutines: () => void;
    /** The menu of everything else: the other pages, the overview, the palette. */
    onMore: () => void;
    /** Shows the list of workspaces. */
    onSpaces: (anchor: Anchor) => void;
}

interface Props {
    workspace: Workspace;
    settings: Settings;
    scheme: Scheme;
    actions: PaneActions;
    handlers: ChromeHandlers;
    broadcast: boolean;
    tabRecording: 'idle' | 'starting' | 'recording' | 'paused' | 'saving';
    /** The workspace's version: it changes in place, so this is what says it did. */
    version: number;
    onClosePane: (session: Session) => void;
}

/** The few colors the window is made of. */
const noSubscribe = () => () => {};
const zero = () => 0;
function useSessionVersion(session: Session | undefined) {
    useSyncExternalStore(session ? session.subscribe : noSubscribe, session ? session.getVersion : zero);
}

/** Draws again when any of these shells changes, as a row of tabs showing all their names must. */
function useSessionsVersion(sessions: Session[]) {
    const key = sessions.map((s) => s.id).join(',');
    const subscribe = useMemo(() => (listener: () => void) => {
        const stops = sessions.map((s) => s.subscribe(listener));
        return () => stops.forEach((stop) => stop());
    }, [key]);
    const total = () => sessions.reduce((sum, s) => sum + s.version, 0);
    useSyncExternalStore(subscribe, total);
}

export function Chrome(props: Props) {
    const { workspace, settings } = props;
    void props.version;
    const layout = settings.layout;
    if (layout === 'sidebar') return <SidebarLayout {...props} />;
    if (layout === 'minimal') return <MinimalLayout {...props} />;
    const showTabs = layout === 'tabs' && !workspace.overview;
    return (
        <div className="flex size-full flex-col">
            {showTabs ? null : <TopBar {...props} />}
            <div className="relative min-h-0 flex-1">
                {showTabs ? (
                    <TabsLayout {...props} />
                ) : layout === 'strip' || workspace.overview ? (
                    <Strip
                        workspace={workspace} settings={settings} scheme={props.scheme} actions={props.actions} onSize={() => {}}
                        onClosePane={props.onClosePane} version={props.version} shortcut={props.handlers.shortcut}
                    />
                ) : (
                    <Content {...props} inset="0 8px 8px 8px" />
                )}
            </div>
        </div>
    );
}

interface TabDrop {
    column: Column;
    sessionId: number | null;
    side: PaneSide;
    rect: PaneRect;
}

function useTabDock(workspace: Workspace) {
    const content = useRef<HTMLDivElement>(null);
    const [preview, setPreview] = useState<TabDrop | null>(null);
    const shown = useRef<TabDrop | null>(null);

    const hit = (id: string | number, x: number, y: number): TabDrop | null => {
        const column = workspace.current;
        const box = content.current?.getBoundingClientRect();
        if (typeof id !== 'number' || !column || !box || box.width <= 0 || box.height <= 0 ||
            !workspace.canDockTab(id, column)) return null;
        const px = (x - box.left) / box.width;
        const py = (y - box.top) / box.height;
        // A zoomed stack takes up the entire panel. Dock next to the stack and unzoom.
        const panes = paneRects(column.zoomed ? 0 : column.paneLayout ?? stackedPanes(column.sessions.length));
        for (const { index, rect } of panes) {
            const side = paneDropAt(rect, px, py);
            if (side) return { column, sessionId: column.zoomed ? null : column.sessions[index].id, side, rect: paneHalf(rect, side) };
        }
        return null;
    };

    const show = (next: TabDrop | null) => {
        const old = shown.current;
        if (next === old || (old && next && old.column === next.column && old.sessionId === next.sessionId && old.side === next.side &&
            old.rect.x === next.rect.x && old.rect.y === next.rect.y && old.rect.w === next.rect.w && old.rect.h === next.rect.h)) return;
        shown.current = next;
        setPreview(next);
    };

    const outside: ReorderOutside = {
        over: (id, x, y) => show(hit(id, x, y)),
        drop: (id, x, y) => {
            const target = hit(id, x, y);
            if (typeof id === 'number' && target) workspace.dockTab(id, target.column, target.sessionId, target.side);
        },
        clear: () => show(null),
    };
    return { content, preview, outside };
}

/** The open shell's panel, filling the room there is. */
function Content(props: Props & { inset: string; joined?: PanelPlace; navOutside?: boolean; dock?: ReturnType<typeof useTabDock> }) {
    const { workspace, scheme } = props;
    void props.version;
    const t = tokens(scheme);
    const column = workspace.current;
    if (!column) {
        return (
            <div className="flex size-full flex-col items-center justify-center gap-4">
                <div className="text-[14px]" style={{ color: t.sub }}>
                    No terminals open
                </div>
                <button
                    className="flex h-9 items-center gap-2 rounded-lg px-4 text-[13px] font-medium"
                    style={{ background: t.chosen, color: t.text }}
                    onClick={props.handlers.onNew}
                >
                    <Icon name="plus" size={15} />
                    New terminal
                    {props.handlers.shortcut('new') ? <Kbd scheme={scheme}>{props.handlers.shortcut('new')!}</Kbd> : null}
                </button>
            </div>
        );
    }
    return (
        <div ref={props.dock?.content} className="relative size-full" style={{ padding: props.inset }}>
            <ColumnCard
                key={column.id}
                column={column}
                index={workspace.focus}
                plain
                joined={props.joined}
                focused
                workspace={workspace}
                settings={props.settings}
                scheme={props.scheme}
                actions={props.actions}
                onClosePane={props.onClosePane}
                shortcut={props.handlers.shortcut}
                navOutside={props.navOutside}
                version={props.version}
            />
            {props.dock?.preview?.column === column ? (
                <div aria-hidden="true" data-tab-drop-preview={props.dock.preview.side} className="pointer-events-none absolute z-10"
                    style={paneStyle(props.dock.preview.rect)}>
                    <div className="absolute inset-1 rounded-lg"
                        style={{ background: withAlpha(scheme.ansi[4], 0.24), border: `2px solid ${withAlpha(scheme.ansi[4], 0.85)}` }} />
                </div>
            ) : null}
        </div>
    );
}

// ---- Small parts

export function WindowButtons(props: { scheme: Scheme; beforeClose?: () => void | Promise<void> }) {
    const { maximized } = useWindowState();
    const t = tokens(props.scheme);
    const base = 'flex w-[46px] items-center justify-center transition-colors duration-150';
    return (
        <div className="flex h-full shrink-0 items-stretch [app-region:no-drag]">
            <button aria-label="Minimize" className={base + ' hover:bg-white/[0.06]'} style={{ color: t.sub }} onClick={() => mainWindow.minimize()}>
                <span className="h-px w-[10px] bg-current" />
            </button>
            <button
                aria-label={maximized ? 'Restore' : 'Maximize'}
                className={base + ' hover:bg-white/[0.06]'}
                style={{ color: t.sub }}
                onClick={() => (maximized ? mainWindow.restore() : mainWindow.maximize())}
            >
                <span className="size-[10px] rounded-[2px] border border-current" />
            </button>
            <button aria-label="Close" className={base + ' hover:bg-[#c42b1c] hover:text-white'} style={{ color: t.sub }}
                onClick={async () => { await props.beforeClose?.(); mainWindow.close(); }}>
                <Icon name="close" size={13} strokeWidth={1.5} />
            </button>
        </div>
    );
}

/** The icons at the right of a bar: few, and the rest are in the palette. */
function Actions(props: Props) {
    const { scheme, handlers, workspace } = props;
    return (
        <div className="flex items-center gap-0.5 pr-1">
            <IconButton keepFocus noDrag icon="plus" iconSize={18} label={withShortcut('New terminal', handlers.shortcut('new')) + ' · right-click to choose a shell'} scheme={scheme} onClick={handlers.onNew} onContextMenu={handlers.onProfiles} />
            <IconButton keepFocus noDrag icon="search" label={withShortcut('Find', handlers.shortcut('find'))} scheme={scheme} onClick={handlers.onSearch} />
            <RecordButton {...props} />
            <IconButton keepFocus noDrag icon="dots" iconSize={14} label="More" scheme={scheme} pressed={workspace.overview} onClick={handlers.onMore} />
        </div>
    );
}

/** The window recorder is available across terminal and tool tabs. */
function RecordButton(props: Props) {
    const { scheme, handlers, tabRecording } = props;
    const on = tabRecording === 'recording' || tabRecording === 'paused';
    const pending = tabRecording === 'starting' || tabRecording === 'saving';
    const label = pending ? (tabRecording === 'starting' ? 'Starting tab recording' : 'Saving tab recording')
        : withShortcut(tabRecording === 'paused' ? 'Stop paused tab recording' : on ? 'Stop recording' : 'Record terminal tab', handlers.shortcut('record')) + (on ? '' : ' · right-click for recordings');
    const hint = ' · Saves typed keys, including hidden input';
    const [hover, setHover] = useState(false);
    const t = tokens(scheme);
    return (
        <button
            aria-label={label}
            title={label + hint}
            disabled={pending}
            className="flex size-8 items-center justify-center rounded-lg transition-colors duration-150 [app-region:no-drag]"
            style={{ color: on ? css(scheme.ansi[1]) : hover ? t.text : t.sub, background: on ? withAlpha(scheme.ansi[1], 0.14) : hover ? t.hover : 'transparent' }}
            onPointerEnter={() => setHover(true)}
            onPointerLeave={() => setHover(false)}
            onPointerDown={(e: any) => e.preventDefault()}
            onClick={handlers.onRecord}
            onContextMenu={handlers.onRecordings}
        >
            <span className={tabRecording === 'recording' ? 'rec-dot' : ''}>
                <Icon name={on ? 'stop' : 'record'} size={on ? 16 : 14} />
            </span>
        </button>
    );
}

function titleOf(session: Session | undefined): { title: string; sub: string } {
    if (!session) return { title: 'Terminal', sub: 'No shell' };
    const title = session.displayName;
    const sub = session.cwd ? shortPath(session.cwd) : session.profileName;
    return { title, sub: sub === title ? 'This PC' : sub };
}

// ---- Bars

function TopBar(props: Props) {
    const { workspace, scheme } = props;
    void props.version;
    const session = workspace.currentSession;
    useSessionVersion(session);
    return (
        <div className="flex h-12 shrink-0 items-center gap-2 pl-2.5 [app-region:drag]">
            <SpaceButton workspace={workspace} scheme={scheme} version={props.version} onOpen={props.handlers.onSpaces} />
            <div className="flex flex-1 items-center justify-center">
                {workspace.columns.length > 1 ? <StripDots workspace={workspace} scheme={scheme} version={props.version} /> : null}
            </div>
            <Actions {...props} />
            <WindowButtons scheme={scheme} beforeClose={props.handlers.beforeClose} />
        </div>
    );
}

/** Tabs along the top, the open one joined to the panel as one surface. */
function TabsLayout(props: Props) {
    const { workspace, scheme, handlers } = props;
    void props.version;
    useSessionsVersion(workspace.columns.flatMap((c) => (c.kind === 'terminal' ? [c.sessions[c.active]] : [])));
    const dock = useTabDock(workspace);
    return (
        <TabbedPanel
            {...tabbedLook(props)}
            scheme={scheme}
            tabs={workspace.columns.map((column, i) => tabFor(column, i === workspace.focus, () => workspace.focusColumn(i), () => workspace.closeColumn(i), scheme))}
            onReorder={(from, to) => workspace.reorderColumn(from, to)}
            tabDock={dock.outside}
            lead={<SpaceButton workspace={workspace} scheme={scheme} version={props.version} onOpen={handlers.onSpaces} />}
            leadWidth={Math.min(190, 64 + workspace.space.name.length * 7)}
            after={<SmallButton icon="plus" label={withShortcut('New terminal', handlers.shortcut('new'))} scheme={scheme} onClick={handlers.onNew} onContextMenu={handlers.onProfiles} />}
            right={
                <>
                    <div className="flex items-center gap-0.5 pr-1">
                        <SmallButton icon="search" label={withShortcut('Find', handlers.shortcut('find'))} scheme={scheme} onClick={handlers.onSearch} />
                        <SmallButton icon="grid" label={withShortcut('Overview', handlers.shortcut('overview'))} scheme={scheme} onClick={() => workspace.toggleOverview()} />
                        <RecordButton {...props} />
                        <SmallButton icon="studio" label="Background studio" scheme={scheme} onClick={handlers.onStudio} />
                        <SmallButton icon="gear" label={withShortcut('Settings', handlers.shortcut('settings'))} scheme={scheme} onClick={handlers.onSettings} />
                    </div>
                    <WindowButtons scheme={scheme} beforeClose={props.handlers.beforeClose} />
                </>
            }
        >
            <Content {...props} inset="0" dock={dock} />
        </TabbedPanel>
    );
}

/** What a tabbed panel needs to look like the panel it joins: its color, its corners and its background. */
function tabbedLook(props: Props, pauseBackground?: boolean) {
    const { settings, scheme, workspace } = props;
    const studio = workspace.current?.kind === 'studio';
    return {
        panelColor: studio ? css(scheme.bg) : withAlpha(scheme.bg, settings.columnOpacity),
        radius: settings.radius,
        panelBackground: studio || settings.background === 'none' ? undefined : (outline: string) => <Background options={backgroundOptions(settings, scheme)} outline={outline} paused={pauseBackground} />,
    };
}

function tabFor(column: Column, active: boolean, onPick: () => void, onClose: (() => void) | undefined, scheme: Scheme): PanelTab {
    if (column.routine) return { key: column.id, title: column.routine.name, icon: 'split', active, status: column.routine.runner.tabStatus, onPick, onClose };
    const session = column.sessions[column.active];
    if (column.kind !== 'terminal' || !session) {
        const label = pageLabel(column);
        return { key: column.id, title: label.title, icon: label.icon, active, status: null, onPick, onClose };
    }
    const status = statusOf(session, scheme);
    const failed = session.exitCode !== null || (session.lastFinished()?.exit ?? 0) !== 0;
    return { key: column.id, title: session.displayName, active, status: column.sessions.some((pane) => pane.attention) ? 'attention' : status.busy ? 'busy' : failed ? 'failed' : null, onPick, onClose };
}

// ---- Sidebar

const SIDEBAR_SLIDE_MS = 220;
const SIDEBAR_SLIDE = `transform ${SIDEBAR_SLIDE_MS}ms ease-out`;
const SIDEBAR_BUTTON_SPACE = 36;
const SIDEBAR_BUTTON_FADE = 'opacity 140ms ease-out, transform 140ms ease-out';

function SidebarLayout(props: Props) {
    const { workspace, settings, scheme, handlers } = props;
    void props.version;
    const t = tokens(scheme);
    const [sidebar, setSidebar] = useState<'open' | 'opening' | 'preparing' | 'sliding' | 'closed'>('open');
    const [recordingNav] = useState(createRecordingSidebar);
    const opening = sidebar === 'opening';
    const hidden = sidebar !== 'open' && !opening;
    const sliding = sidebar === 'sliding';
    const closing = sidebar === 'preparing' || sliding;
    useEffect(() => {
        if (!settings.animations && (closing || opening)) {
            setSidebar(opening ? 'open' : 'closed');
            return;
        }
        if (sidebar === 'preparing') {
            let frame = requestAnimationFrame(() => {
                frame = requestAnimationFrame(() => setSidebar('sliding'));
            });
            return () => cancelAnimationFrame(frame);
        }
        if (!sliding && !opening) return;
        const timer = setTimeout(() => setSidebar(opening ? 'open' : 'closed'), SIDEBAR_SLIDE_MS + 60);
        return () => clearTimeout(timer);
    }, [sidebar, settings.animations]);
    const sidebarAway = hidden && sidebar !== 'preparing';
    // Give the expanded panel a frame to lay out at the old edge before sliding it into place.
    const panelOffset = sidebar === 'preparing' ? settings.sidebarWidth - 6 : 0;
    const transformTransition = sliding && settings.animations ? SIDEBAR_SLIDE : 'none';
    const showSidebar = hidden || opening
        ? <div className="flex items-center" inert={sidebar !== 'closed' || undefined} style={{
            opacity: sidebar === 'preparing' || opening ? 0 : 1,
            transform: sidebar === 'preparing' || opening ? 'translateX(-5px)' : 'none',
            transition: settings.animations && (sliding || opening) ? SIDEBAR_BUTTON_FADE : 'none',
        }}>
            <SmallButton icon="sidebar" label="Show the sidebar" scheme={scheme}
                onClick={() => setSidebar(settings.animations ? 'opening' : 'open')} />
        </div>
        : null;
    const windowButtons = (
        <div className="h-full" style={{
            transform: panelOffset ? `translateX(${-panelOffset}px)` : 'none', transition: transformTransition,
        }}>
            <WindowButtons scheme={scheme} beforeClose={props.handlers.beforeClose} />
        </div>
    );
    const column = workspace.current;
    const session = workspace.currentSession;
    useSessionVersion(session);
    // A page with a list of its own shows it here, the window's list sliding away to the left for it. The last such page stays
    // drawn while it slides out again.
    const paged = column !== undefined && hasNav(column.kind) && !workspace.overview ? column : undefined;
    const sidebarWidth = paged?.kind === 'recordings' ? Math.min(settings.sidebarWidth, window.innerWidth * 0.3) : settings.sidebarWidth;
    const sidebarRemaining = (panelWidth: number) => {
        return sidebarWidth > 0 ? Math.max(0, Math.min(1, 1 - (window.innerWidth - panelWidth) / sidebarWidth)) : 0;
    };
    const sidebarTabs = workspace.columns.map((tab, index) => ({ tab, index })).filter(({ tab }) => tab.kind !== 'settings');
    const tabIds = paged || hidden ? [] : sidebarTabs.map(({ tab }) => tab.id);
    const dock = useTabDock(workspace);
    const reorder = useReorder(tabIds, 'Y', (from, to) => workspace.reorderColumn(sidebarTabs[from].index, sidebarTabs[to].index), undefined, dock.outside);
    const lastPaged = useRef<Column | undefined>(undefined);
    if (paged) lastPaged.current = paged;
    const shownPage = hidden ? undefined : paged ??
        (lastPaged.current && workspace.columns.includes(lastPaged.current) ? lastPaged.current : undefined);
    const slide = settings.animations ? 'transform 320ms cubic-bezier(0.22, 1, 0.36, 1), opacity 220ms ease' : 'none';
    const body = (
        <div className="relative flex size-full overflow-hidden">
            <div className="relative shrink-0 overflow-hidden" inert={hidden || opening || undefined} style={{
                position: closing ? 'absolute' : 'relative', top: closing ? 0 : undefined, bottom: closing ? 0 : undefined,
                width: sidebar === 'closed' ? 0 : opening ? sidebarWidth : settings.sidebarWidth,
                maxWidth: paged?.kind === 'recordings' ? '30%' : undefined,
                transition: settings.animations && opening ? `width ${SIDEBAR_SLIDE_MS}ms ease-out` : 'none',
                zIndex: hidden ? 1 : undefined, pointerEvents: hidden ? 'none' : undefined,
                visibility: sidebar === 'closed' ? 'hidden' : undefined,
            }} onTransitionEnd={(event) => {
                if (opening && event.target === event.currentTarget && event.propertyName === 'width') setSidebar('open');
            }}>
                <div className="absolute inset-y-0 right-0" style={{
                    width: settings.sidebarWidth, maxWidth: paged?.kind === 'recordings' ? '30vw' : undefined,
                    transform: sidebarAway ? 'translateX(-100%)' : 'none',
                    transition: settings.animations && sliding ? SIDEBAR_SLIDE : 'none',
                }}>
                    <div
                        className="absolute bottom-0 left-0 top-0 flex flex-col"
                        style={{ width: settings.sidebarWidth, transform: paged ? 'translateX(-32%)' : 'none', opacity: paged ? 0 : 1, transition: slide, pointerEvents: paged ? 'none' : 'auto' }}
                    >
                        <div className="flex h-11 shrink-0 items-center gap-1 px-2 [app-region:drag]">
                            <SmallButton icon="sidebar" label="Hide the sidebar" scheme={scheme}
                                onClick={() => setSidebar(settings.animations ? 'preparing' : 'closed')} />
                            <div className="flex min-w-0 flex-1">
                                <SpaceButton workspace={workspace} scheme={scheme} version={props.version} onOpen={handlers.onSpaces} />
                            </div>
                            <SmallButton icon="search" label={withShortcut('Find', handlers.shortcut('find'))} scheme={scheme} onClick={handlers.onSearch} />
                        </div>
                        <div className="flex shrink-0 flex-col gap-px px-2">
                            <NavItem icon="plus" label="New terminal" keys={handlers.shortcut('new')} scheme={scheme} onClick={handlers.onNew} />
                            <NavItem icon="command" label="Commands" keys={handlers.shortcut('palette')} scheme={scheme} onClick={handlers.onPalette} />
                            <NavItem icon="split" label="Routines" keys={handlers.shortcut('routines')} scheme={scheme} onClick={handlers.onRoutines} />
                            <NavItem icon="grid" label="Overview" keys={handlers.shortcut('overview')} scheme={scheme} onClick={() => workspace.toggleOverview()} />
                        </div>
                        <div className="mt-5 flex h-7 items-center justify-between pl-3.5 pr-2">
                            <span className="text-[12px]" style={{ color: t.faint }}>
                                Tabs
                            </span>
                            <SmallButton icon="plus" label="New tab · right-click for more" scheme={scheme} onClick={handlers.onNew} onContextMenu={handlers.onProfiles} />
                        </div>
                        <div className="min-h-0 flex-1 overflow-y-auto px-2">
                            <div className="flex flex-col gap-0.5 pb-0.5" data-reorderable={tabIds.length > 1 ? '' : undefined}>
                                {sidebarTabs.map(({ tab: c, index: i }, row) => {
                                    const item = {
                                        active: i === workspace.focus && !workspace.overview,
                                        scheme,
                                        drag: reorder.tab(row, () => workspace.focusColumn(i)),
                                        onClose: () => workspace.closeColumn(i),
                                    };
                                    return (
                                        <div key={c.id}>
                                            {c.routine ? <PageItem {...item} title={c.routine.name} icon="split" status={c.routine.runner.tabStatus} />
                                                : c.kind === 'terminal' ? <SidebarItem {...item} session={c.sessions[c.active]} stacked={c.sessions.length} />
                                                : <PageItem {...item} {...pageLabel(c)} />}
                                        </div>
                                    );
                                })}
                            </div>
                        </div>
                        <div className="grid shrink-0 grid-cols-4 justify-items-center gap-y-0.5 px-2 py-1.5">
                            <SmallButton icon="browser" label="New browser tab" scheme={scheme} onClick={handlers.onBrowser} />
                            <SmallButton icon="studio" label="Background studio" scheme={scheme} onClick={handlers.onStudio} />
                            <SmallButton icon="video" label="Recordings" scheme={scheme} onClick={handlers.onRecordings} />
                            <SmallButton icon="gear" label={withShortcut('Settings', handlers.shortcut('settings'))} scheme={scheme} onClick={handlers.onSettings} />
                        </div>
                    </div>
                    {shownPage ? (
                        <div
                            className="absolute bottom-0 left-0 top-0 flex flex-col"
                            inert={!paged || undefined}
                            style={{ width: shownPage.kind === 'recordings' ? '100%' : settings.sidebarWidth,
                                transform: paged ? 'none' : 'translateX(100%)', transition: slide, pointerEvents: paged ? 'auto' : 'none' }}
                        >
                            <div className="flex h-11 shrink-0 items-center px-2 [app-region:drag]">
                                <BackButton scheme={scheme} onClick={() => workspace.back()} />
                            </div>
                            {shownPage.kind === 'recordings' ? (
                                <RecordingSidebar sidebar={recordingNav} />
                            ) : (
                                <>
                                    <div className="px-3.5 pb-2 pt-1 text-[15px] font-semibold tracking-tight" style={{ color: t.text }}>
                                        {pageLabel(shownPage).title}
                                    </div>
                                    <div className="min-h-0 flex-1 overflow-y-auto px-2">
                                        <PageNav column={shownPage} workspace={workspace} scheme={scheme} version={props.version} />
                                    </div>
                                </>
                            )}
                        </div>
                    ) : null}
                </div>
            </div>
            <div className="relative min-w-0 flex-1" style={{
                transform: panelOffset ? `translateX(${panelOffset}px)` : 'none', transition: transformTransition,
            }} onTransitionEnd={(event) => {
                if (sliding && event.target === event.currentTarget && event.propertyName === 'transform') setSidebar('closed');
            }}>
                {workspace.overview ? (
                    <div className="flex size-full flex-col">
                        <div className="flex h-11 shrink-0 items-center justify-between [app-region:drag]">
                            <div className="flex items-center pl-2">{showSidebar}</div>
                            {windowButtons}
                        </div>
                        <div className="relative min-h-0 flex-1">
                            <Strip
                                workspace={workspace} settings={settings} scheme={scheme} actions={props.actions} onSize={() => {}}
                                onClosePane={props.onClosePane} version={props.version} shortcut={handlers.shortcut}
                            />
                        </div>
                    </div>
                ) : (
                    <TabbedPanel
                        {...tabbedLook(props, closing || opening)}
                        scheme={scheme}
                        inset={{ left: opening ? (width) => Math.round(6 * sidebarRemaining(width)) : hidden ? 6 : 0, right: 6, bottom: 6 }}
                        tabs={column ? [tabFor(column, true, () => {}, () => workspace.closeColumn(workspace.focus), scheme)] : []}
                        lead={showSidebar}
                        leadWidth={opening ? (width) => Math.round(SIDEBAR_BUTTON_SPACE * sidebarRemaining(width)) : SIDEBAR_BUTTON_SPACE}
                        after={<SmallButton icon="plus" label="New tab · right-click for more" scheme={scheme} onClick={handlers.onNew} onContextMenu={handlers.onProfiles} />}
                        right={windowButtons}
                    >
                        <Content {...props} inset="0" navOutside={!hidden} dock={dock} />
                    </TabbedPanel>
                )}
            </div>
        </div>
    );
    return (
        <RecordingSidebarContext.Provider value={recordingNav}>
            <PaneResizePausedContext.Provider value={opening || closing}>{body}</PaneResizePausedContext.Provider>
        </RecordingSidebarContext.Provider>
    );
}

/** Back out of a page's list to the window's own, and to the shell that was open. */
function BackButton(props: { scheme: Scheme; onClick: () => void }) {
    const t = tokens(props.scheme);
    const [hover, setHover] = useState(false);
    return (
        <button
            className="flex h-7 items-center gap-1 rounded-md pl-1 pr-2.5 text-[13px] transition-colors duration-150 [app-region:no-drag]"
            style={{ color: hover ? t.text : t.sub, background: hover ? t.hover : 'transparent' }}
            onPointerEnter={() => setHover(true)}
            onPointerLeave={() => setHover(false)}
            onPointerDown={(e: any) => e.preventDefault()}
            onClick={props.onClick}
        >
            <Icon name="left" size={15} />
            Back
        </button>
    );
}

interface SidebarTabProps {
    active: boolean;
    scheme: Scheme;
    drag: TabDragProps;
    onClose: () => void;
}

/** A page in the sidebar's list of tabs: its icon and name. */
function PageItem(props: SidebarTabProps & { title: string; icon: IconName; status?: 'busy' | 'failed' | null }) {
    const { scheme } = props;
    const t = tokens(scheme);
    const [hover, setHover] = useState(false);
    return (
        <div
            className="reorder-item sidebar-tab relative flex h-8 items-center gap-2 rounded-lg px-2.5"
            style={{ background: props.active ? t.chosen : hover ? t.hover : 'transparent' }}
            onPointerEnter={() => setHover(true)}
            onPointerLeave={() => setHover(false)}
            {...props.drag}
        >
            <span className="flex shrink-0" style={{ color: t.sub }}>
                <Icon name={props.icon} size={14} />
            </span>
            <span className="min-w-0 flex-1 truncate text-[13px]" style={{ color: props.active ? t.text : withAlpha(scheme.fg, 0.72) }}>
                {props.title}
            </span>
            {hover ? (
                <button
                    aria-label="Close"
                    className="flex size-[18px] shrink-0 items-center justify-center rounded-md hover:bg-white/10"
                    style={{ color: t.sub }}
                    onPointerDown={(e: any) => e.stopPropagation()}
                    onClick={props.onClose}
                >
                    <Icon name="close" size={10} strokeWidth={1.8} />
                </button>
            ) : props.status ? <span className="size-1.5 shrink-0 rounded-full" style={{ background: css(scheme.ansi[props.status === 'failed' ? 1 : 3]) }} /> : null}
        </div>
    );
}

/** The open shell as a small rounded tab at the top of the panel. */
function SessionPill(props: { session: Session | undefined; scheme: Scheme }) {
    useSessionVersion(props.session);
    const t = tokens(props.scheme);
    const { title } = titleOf(props.session);
    return (
        <div className="flex h-7 w-[220px] min-w-0 items-center gap-2 rounded-lg px-2.5 [app-region:no-drag]" style={{ background: withAlpha(props.scheme.fg, 0.07), color: t.text }}>
            <span style={{ color: t.sub }}>
                <Icon name="terminal" size={14} strokeWidth={1.5} />
            </span>
            <span className="min-w-0 flex-1 truncate text-[13px]">{title}</span>
        </div>
    );
}

/** A small quiet icon button, as the sidebar's headings have. */
function SmallButton(props: Pick<IconButtonProps, 'icon' | 'label' | 'scheme' | 'onClick' | 'onContextMenu'>) {
    return <IconButton {...props} size={28} keepFocus noDrag />;
}

function NavItem(props: { icon: IconName; label: string; keys?: string; scheme: Scheme; onClick: () => void }) {
    const t = tokens(props.scheme);
    const [hover, setHover] = useState(false);
    return (
        <button type="button"
            className="flex h-7 items-center gap-2.5 rounded-md pl-2 pr-1.5 text-left transition-colors duration-150"
            style={{ background: hover ? t.hover : 'transparent', color: withAlpha(props.scheme.fg, 0.82) }}
            onPointerEnter={() => setHover(true)}
            onPointerLeave={() => setHover(false)}
            onPointerDown={(event) => event.preventDefault()}
            onClick={props.onClick}
        >
            <span style={{ color: t.sub }}><Icon name={props.icon} size={15} /></span>
            <span className="flex-1 truncate whitespace-nowrap text-[13px]">{props.label}</span>
            {hover && props.keys ? <Kbd scheme={props.scheme}>{props.keys}</Kbd> : null}
        </button>
    );
}

function SidebarItem(props: SidebarTabProps & { session: Session; stacked: number }) {
    const { session, scheme } = props;
    useSessionVersion(session);
    const t = tokens(scheme);
    const status = statusOf(session, scheme);
    const failed = session.exitCode !== null || (session.lastFinished()?.exit ?? 0) !== 0;
    const [hover, setHover] = useState(false);
    const last = session.lastFinished();
    const detail = session.attention || (status.busy ? lastCommandText(session) || 'Running' : last && last.text ? last.text : session.cwd ? shortPath(session.cwd) : '');
    const dot = session.attention ? css(scheme.ansi[5]) : status.busy ? css(scheme.ansi[3]) : failed ? css(scheme.ansi[1]) : null;
    return (
        <div
            className="reorder-item sidebar-tab relative flex flex-col justify-center rounded-lg px-2.5 py-1.5"
            style={{ background: props.active ? t.chosen : hover ? t.hover : 'transparent' }}
            onPointerEnter={() => setHover(true)}
            onPointerLeave={() => setHover(false)}
            {...props.drag}
        >
            <div className="flex h-5 items-center gap-2">
                <span className="min-w-0 flex-1 truncate text-[13px]" style={{ color: props.active ? t.text : withAlpha(scheme.fg, 0.72) }}>
                    {session.displayName}
                    {props.stacked > 1 ? <span style={{ color: t.faint }}> +{props.stacked - 1}</span> : null}
                </span>
                {hover ? (
                    <button
                        aria-label="Close"
                        className="flex size-[18px] shrink-0 items-center justify-center rounded-md hover:bg-white/10"
                        style={{ color: t.sub }}
                        onPointerDown={(e: any) => e.stopPropagation()}
                        onClick={props.onClose}
                    >
                        <Icon name="close" size={10} strokeWidth={1.8} />
                    </button>
                ) : dot !== null ? (
                    <div className="size-[6px] shrink-0 rounded-full" style={{ background: dot }} />
                ) : null}
            </div>
            {detail ? (
                <div className="truncate text-[12px] leading-[17px]" style={{ color: t.faint }}>
                    {detail}
                </div>
            ) : null}
        </div>
    );
}

// ---- Minimal

function MinimalLayout(props: Props) {
    const { workspace, settings, scheme } = props;
    void props.version;
    const t = tokens(scheme);
    const session = workspace.currentSession;
    useSessionVersion(session);
    const [hover, setHover] = useState(false);
    return (
        <div className="relative size-full p-1.5">
            {workspace.overview ? (
                <Strip
                    workspace={workspace} settings={settings} scheme={scheme} actions={props.actions} onSize={() => {}}
                    onClosePane={props.onClosePane} version={props.version} shortcut={props.handlers.shortcut}
                />
            ) : (
                <Content {...props} inset="0" />
            )}
            <div
                className="absolute inset-x-1.5 top-1.5 z-10 h-11 [app-region:drag]"
                style={{ opacity: hover ? 1 : 0, transition: 'opacity 200ms ease' }}
                onPointerEnter={() => setHover(true)}
                onPointerLeave={() => setHover(false)}
            >
                <div
                    className="flex size-full items-center gap-2 pl-2"
                    style={{ borderRadius: `${settings.radius}px ${settings.radius}px 0 0`, background: t.panel, borderBottom: `1px solid ${t.border}` }}
                >
                    <SpaceButton workspace={workspace} scheme={scheme} version={props.version} onOpen={props.handlers.onSpaces} />
                    <div className="flex-1" />
                    <Actions {...props} />
                    <WindowButtons scheme={scheme} beforeClose={props.handlers.beforeClose} />
                </div>
            </div>
        </div>
    );
}

// ---- Bits

export function statusOf(session: Session, scheme: Scheme): { color: string; busy: boolean } {
    const list = session.commands;
    const last = list[list.length - 1];
    const busy = last !== undefined && last.endLine === null && last.outputLine !== null;
    if (session.exitCode !== null) return { color: css(scheme.ansi[1]), busy: false };
    if (busy) return { color: css(scheme.ansi[3]), busy: true };
    const done = session.lastFinished();
    if (done && done.exit !== 0) return { color: css(scheme.ansi[1]), busy: false };
    return { color: css(scheme.ansi[2]), busy: false };
}

function lastCommandText(session: Session): string {
    const list = session.commands;
    return list.length > 0 ? list[list.length - 1].text : '';
}

export function shortPath(cwd: string): string {
    const clean = cwd.replace(/\\/g, '/').replace(/^[A-Za-z]:\/Users\/[^/]+/, '~');
    const parts = clean.split('/').filter(Boolean);
    if (parts.length <= 3) return clean;
    return '…/' + parts.slice(-2).join('/');
}

function ago(time: number): string {
    const s = Math.max(0, (Date.now() - time) / 1000);
    if (s < 45) return 'now';
    if (s < 3600) return `${Math.round(s / 60)}m`;
    if (s < 86400) return `${Math.round(s / 3600)}h`;
    return `${Math.round(s / 86400)}d`;
}
