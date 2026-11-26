// The strip itself: every column laid out in one row, slid so the open one is in the middle, or all of them shrunk into a grid
// in the overview. Each column is placed with a transform, so sliding and shrinking are the same motion, and the shells in them
// carry on drawing while they move.

import { useEffect, useRef, useState } from 'react';
import { hostname } from 'os';
import { TerminalPane, useSession } from '../term/TerminalPane';
import type { PaneActions } from '../term/TerminalPane';
import type { Session } from '../term/Session';
import { Icon } from '../ui/Icon';
import { Background, backgroundOptions } from '../ui/Background';
import type { PanelPlace } from '../ui/TabbedPanel';
import type { Column, Workspace } from '../workspace';
import type { Settings } from '../settings';
import { css, mix, withAlpha } from '../themes';
import type { Scheme } from '../themes';
import { columnWidth, overviewPlaces, stripPlaces } from './layout';
import { paneRects, paneStyle, stackedPanes } from './panes';
import type { PaneRect } from './panes';
import { PageBody, pageLabel } from '../ui/Pages';
import { withShortcut } from '../ui/controls';
import { RoutineSteps } from '../ui/Routines';

const HEADER = 32;
const LOCAL_HOST = hostname() || 'Local';

export function Strip(props: {
    workspace: Workspace;
    settings: Settings;
    scheme: Scheme;
    actions: PaneActions;
    shortcut: (id: string) => string | undefined;
    onSize: (cols: number, rows: number) => void;
    onClosePane: (session: Session) => void;
    /** The workspace's version, which says it changed in place. */
    version?: number;
}) {
    const { workspace, settings, scheme } = props;
    void props.version;
    const box = useRef<HTMLDivElement>(null);
    const [size, setSize] = useState({ w: 1200, h: 700 });
    useEffect(() => {
        const element = box.current;
        if (!element) return;
        const measure = () => {
            const r = element.getBoundingClientRect();
            if (r.width > 0 && r.height > 0) setSize((was) => (Math.abs(was.w - r.width) < 0.5 && Math.abs(was.h - r.height) < 0.5 ? was : { w: r.width, h: r.height }));
        };
        measure();
        const observer = new ResizeObserver(measure);
        observer.observe(element);
        return () => observer.disconnect();
    }, []);

    const widths = workspace.columns.map((c) => columnWidth(c.width, size.w, settings.peek));
    const places = workspace.overview ? overviewPlaces(widths, size.w, size.h) : stripPlaces(widths, workspace.focus, size.w, size.h - 8);
    const move = settings.animations ? 'transform 420ms cubic-bezier(0.22, 1, 0.36, 1), opacity 260ms ease' : 'none';

    return (
        <div ref={box} className="relative size-full overflow-hidden" onWheel={(e: any) => onStripWheel(e, workspace)}>
            {workspace.columns.map((column, i) => {
                const place = places[i];
                if (!place) return null;
                const focused = i === workspace.focus;
                const dim = settings.dimOthers && !focused && !workspace.overview;
                return (
                    <div
                        key={column.id}
                        className="absolute left-0 top-0"
                        style={{
                            width: place.w,
                            height: place.h,
                            transform: `translate(${place.x}px, ${place.y}px) scale(${place.scale})`,
                            transformOrigin: 'top left',
                            transition: move,
                            opacity: dim ? 0.62 : 1,
                        }}
                    >
                        <ColumnCard
                            column={column}
                            index={i}
                            focused={focused}
                            workspace={workspace}
                            settings={settings}
                            scheme={scheme}
                            actions={props.actions}
                            shortcut={props.shortcut}
                            onClosePane={props.onClosePane}
                            version={props.version}
                        />
                        {workspace.overview ? (
                            <div
                                key={`overview-${column.id}`}
                                className="group absolute inset-0 cursor-pointer rounded-[inherit]"
                                style={{ borderRadius: settings.radius, boxShadow: focused ? `0 0 0 ${2 / place.scale}px ${withAlpha(scheme.fg, 0.55)}` : 'none' }}
                                onClick={() => workspace.focusColumn(i)}
                            >
                                <div
                                    className="absolute left-4 top-3 rounded-full px-3 py-1 font-semibold"
                                    style={{ fontSize: 13 / place.scale, background: withAlpha(scheme.bg, 0.8), color: css(scheme.fg) }}
                                >
                                    {i + 1}
                                </div>
                            </div>
                        ) : null}
                    </div>
                );
            })}
            {workspace.columns.length === 0 ? <Empty scheme={scheme} workspace={workspace} /> : null}
        </div>
    );
}

let wheelLock = 0;

/** A wheel over the frame between columns, or sideways anywhere, steps from column to column. */
function onStripWheel(e: any, workspace: Workspace) {
    const sideways = Math.abs(e.deltaX) > Math.abs(e.deltaY) && Math.abs(e.deltaX) > 4;
    if (!sideways && !e.altKey) return;
    const now = Date.now();
    if (now - wheelLock < 260) return;
    wheelLock = now;
    const delta = sideways ? e.deltaX : e.deltaY;
    workspace.moveFocus(delta > 0 ? 1 : -1);
}

function Empty(props: { scheme: Scheme; workspace: Workspace }) {
    return (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-3" style={{ color: withAlpha(props.scheme.fg, 0.6) }}>
            <div className="text-[15px]">No terminals open</div>
            <button
                className="rounded-lg px-4 py-2 text-[13px] font-medium"
                style={{ background: css(props.scheme.ansi[4]), color: css(props.scheme.bg) }}
                onClick={() => props.workspace.addColumn()}
            >
                Open one
            </button>
        </div>
    );
}

/** A column's card: in the strip with a header over each shell, or `plain`, as one shell fills the window in the other layouts, with
 * a header only over shells that are stacked. */
export function ColumnCard(props: {
    column: Column;
    index: number;
    plain?: boolean;
    /** The active tab and this panel already have a shared background. */
    joined?: PanelPlace;
    focused: boolean;
    workspace: Workspace;
    settings: Settings;
    scheme: Scheme;
    actions: PaneActions;
    shortcut: (id: string) => string | undefined;
    onClosePane: (session: Session) => void;
    /** A page's own list is in the window's sidebar, not at the side of its panel. */
    navOutside?: boolean;
    /** The workspace's version: a page's view changes in place. */
    version?: number;
}) {
    const { column, focused, workspace, settings, scheme, joined } = props;
    const frame = scheme.frame;
    const page = column.kind !== 'terminal';
    const studio = column.kind === 'studio';
    const showBackground = !studio && !joined && settings.background !== 'none' && (props.plain || focused);
    const radius = `${settings.radius}px`;
    const singlePane = column.routine || column.zoomed;
    const panes = page || column.sessions.length === 0 ? null : paneRects(singlePane ? column.active : column.paneLayout ?? stackedPanes(column.sessions.length)).map(({ index, rect }) => (
        <PaneBox
            key={column.sessions[index].id}
            session={column.sessions[index]}
            columnFocused={focused}
            isActive={focused && index === column.active}
            rect={rect}
            stacked={!column.routine && column.sessions.length > 1}
            plain={props.plain === true}
            column={column}
            workspace={workspace}
            settings={settings}
            scheme={scheme}
            actions={props.actions}
            shortcut={props.shortcut}
            onClosePane={props.onClosePane}
        />
    ));
    return (
        <div
            className={(joined ? '' : 'column-in ') + 'relative flex size-full flex-col overflow-hidden'}
            style={{
                borderRadius: joined ? `${joined.squareLeft ? '0' : radius} ${joined.squareRight ? '0' : radius} ${radius} ${radius}` : radius,
                background: joined ? undefined : studio ? css(scheme.bg) : withAlpha(scheme.bg, settings.columnOpacity),
                border: props.plain || column.kind === 'recordings' ? 'none' : `1px solid ${withAlpha(scheme.fg, focused ? 0.16 : 0.06)}`,
                boxShadow: props.plain
                    ? 'none'
                    : focused
                      ? `0 18px 48px ${withAlpha(mix(frame, 0x000000, 0.6), 0.55)}`
                      : `0 8px 24px ${withAlpha(0x000000, 0.25)}`,
            }}
        >
            {showBackground ? <Background options={backgroundOptions(settings, scheme)} /> : null}
            {page && !props.plain ? <PageHeader column={column} index={props.index} workspace={workspace} scheme={scheme} /> : null}
            {page ? (
                <div className="relative min-h-0 flex-1">
                    <PageBody column={column} workspace={workspace} settings={settings} scheme={scheme} withNav={!props.navOutside} focused={focused} version={props.version ?? workspace.version} />
                </div>
            ) : null}
            {column.routine ? (
                <div className="relative flex min-h-0 flex-1">
                    <RoutineSteps runner={column.routine.runner} scheme={scheme} activeId={column.sessions[column.active]?.id}
                        sessionIds={column.sessions.map(({ id }) => id)} tabWidth={joined?.tabWidth}
                        close={() => workspace.closeColumn(workspace.columns.indexOf(column))} />
                    <div className="relative min-w-0 flex-1">
                        {panes ?? <div className="px-4 py-3 text-[12px]" style={{ color: withAlpha(scheme.fg, 0.6) }}>No open panes</div>}
                    </div>
                </div>
            ) : panes}
        </div>
    );
}

function PaneBox(props: {
    session: Session;
    columnFocused: boolean;
    isActive: boolean;
    rect: PaneRect;
    stacked: boolean;
    plain: boolean;
    column: Column;
    workspace: Workspace;
    settings: Settings;
    scheme: Scheme;
    actions: PaneActions;
    shortcut: (id: string) => string | undefined;
    onClosePane: (session: Session) => void;
}) {
    const { session, isActive, workspace, settings, scheme, column } = props;
    const showHeader = !props.plain || props.stacked || !!column.routine;
    useSession(session);
    useEffect(() => {
        session.focus(isActive);
        return () => session.focus(false);
    }, [isActive]);
    const title = session.displayName;
    const running = session.commands.length > 0 && session.commands[session.commands.length - 1].endLine === null && session.commands[session.commands.length - 1].outputLine !== null;
    const dot = session.attention ? scheme.ansi[5] : session.exitCode !== null ? scheme.ansi[1] : running ? scheme.ansi[3] : scheme.ansi[2];
    const broadcast = props.actions.broadcast && workspace.columns.includes(column) && session.exitCode === null;
    const flash = session.bellUntil > Date.now() && (settings.bell === 'flash' || settings.bell === 'both');
    const divider = `1px solid ${withAlpha(scheme.fg, 0.1)}`;
    return (
        <div
            className="absolute flex flex-col overflow-hidden"
            style={{
                ...paneStyle(props.rect),
                borderTop: props.rect.y > 0 ? divider : undefined,
                borderLeft: props.rect.x > 0 ? divider : undefined,
            }}
        >
            {showHeader ? (
            <div
                className="flex shrink-0 select-none items-center gap-2 px-3"
                style={{ height: HEADER, color: withAlpha(scheme.fg, isActive ? 0.85 : 0.5) }}
                onPointerDown={() => workspace.activate(session)}
            >
                <span className="size-2 shrink-0 rounded-full" title={session.attention ? `Needs attention: ${session.attention}` : undefined}
                    style={{ background: css(dot), boxShadow: running || session.attention ? `0 0 8px ${css(dot)}` : 'none' }} />
                <span className="min-w-0 truncate text-[12px] font-medium">{title}</span>
                {session.cwd !== '' && session.cwd !== title ? (
                    <span className="min-w-0 truncate text-[11.5px]" style={{ color: withAlpha(scheme.fg, 0.38) }}>
                        {shorten(session.cwd)}
                    </span>
                ) : null}
                <div className="flex-1" />
                {session.attention ? <span className="truncate text-[11px]" title={session.attention} style={{ color: css(scheme.ansi[5]) }}>Needs attention</span> : null}
                {session.recording ? <span className="rec-dot size-2 rounded-full" style={{ background: '#ff4d5e' }} title="Recording" /> : null}
                <HeaderButton scheme={scheme} label="Command history and bookmarks" onClick={() => { workspace.activate(session); session.historyOpen = !session.historyOpen; session.changed(); }}>
                    <Icon name="command" size={14} />
                </HeaderButton>
                {props.stacked ? <HeaderButton scheme={scheme} label={column.zoomed ? 'Unzoom pane' : 'Zoom pane'} onClick={() => { workspace.activate(session); workspace.togglePaneZoom(); }}>
                    <Icon name={column.zoomed ? 'split' : 'grid'} size={14} />
                </HeaderButton> : null}
                {props.plain ? null : (
                    <HeaderButton scheme={scheme} label={`Width: ${column.width} (click to change)`} onClick={() => workspace.cycleWidth(column)}>
                        <Icon name="width" size={14} />
                    </HeaderButton>
                )}
                {column.routine ? null : <HeaderButton scheme={scheme} label={withShortcut('Split down', props.shortcut('split'))} onClick={() => { workspace.activate(session); workspace.split(); }}>
                    <Icon name="split" size={14} />
                </HeaderButton>}
                <HeaderButton scheme={scheme} label={column.routine ? 'Close pane' : withShortcut('Close', props.shortcut('close'))} onClick={() => props.onClosePane(session)}>
                    <Icon name="close" size={13} />
                </HeaderButton>
            </div>
            ) : null}
            <div
                className="relative min-h-0 flex-1"
                style={{
                    opacity: props.columnFocused || !settings.dimOthers ? 1 : 0.9,
                    padding: `${showHeader ? 4 : Math.round(settings.textPadding * 0.75)}px 2px ${Math.round(settings.textPadding / 2)}px ${Math.max(0, settings.textPadding - (settings.showMarks ? 12 : 4))}px`,
                }}
            >
                <TerminalPane
                    session={session}
                    settings={settings}
                    scheme={scheme}
                    active={isActive}
                    actions={{ ...props.actions, focusMe: () => workspace.activate(session) }}
                />
                {flash ? <div key={session.term.bells} className="bell-flash pointer-events-none absolute inset-0" style={{ background: withAlpha(scheme.fg, 0.1) }} /> : null}
                {broadcast ? <div className="pointer-events-none absolute bottom-1 right-4 rounded px-1.5 text-[10px] leading-[15px]"
                    style={{ color: css(scheme.ansi[3]), background: css(scheme.bg) }}>Broadcast input</div> : null}
                {!showHeader && session.attention ? <button className="absolute right-4 top-0 rounded px-2 text-[11px]" title={session.attention}
                    style={{ color: css(scheme.ansi[5]), background: css(scheme.bg) }} onClick={() => session.acknowledge()}>Needs attention</button> : null}
            </div>
            {settings.paneStatus ? <PaneStatus session={session} scheme={scheme} /> : null}
        </div>
    );
}

function PaneStatus({ session, scheme }: { session: Session; scheme: Scheme }) {
    const current = session.commands.at(-1);
    const command = current?.startedAt != null ? current : session.lastFinished();
    const started = command?.startedAt ?? null;
    const running = started !== null && command?.endedAt === null;
    const [now, setNow] = useState(Date.now);
    useEffect(() => {
        if (!running) return;
        setNow(Date.now());
        const id = setInterval(() => setNow(Date.now()), 1000);
        return () => clearInterval(id);
    }, [running, started]);
    const seconds = started === null ? null : Math.max(0, Math.floor(((command?.endedAt ?? now) - started) / 1000));
    return (
        <div className="flex h-6 shrink-0 items-center gap-3 overflow-hidden px-3 text-[11px]"
            style={{ color: withAlpha(scheme.fg, 0.68), borderTop: `1px solid ${withAlpha(scheme.fg, 0.12)}` }}>
            <span className="min-w-0 truncate" title={session.term.host || LOCAL_HOST}>{session.term.host || LOCAL_HOST}</span>
            {session.gitBranch ? <span className="min-w-0 truncate" title={session.gitBranch}>Git {session.gitBranch}</span> : null}
            {seconds !== null ? <span className="ml-auto shrink-0 tabular-nums" style={{ color: running ? css(scheme.ansi[3]) : undefined }}>
                {running ? 'Running ' : command?.exit ? `Exit ${command.exit} · ` : 'Finished in '}{seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`}
            </span> : null}
        </div>
    );
}

/** A page's name over it in the strip, as a shell has. */
function PageHeader(props: { column: Column; index: number; workspace: Workspace; scheme: Scheme }) {
    const { column, workspace, scheme } = props;
    const label = pageLabel(column);
    return (
        <div className="flex shrink-0 select-none items-center gap-2 px-3" style={{ height: HEADER, color: withAlpha(scheme.fg, 0.7), borderBottom: `1px solid ${withAlpha(scheme.fg, 0.07)}` }}>
            <Icon name={label.icon} size={13} />
            <span className="min-w-0 flex-1 truncate text-[12px] font-medium">{label.title}</span>
            <HeaderButton scheme={scheme} label={`Width: ${column.width} (click to change)`} onClick={() => workspace.cycleWidth(column)}>
                <Icon name="width" size={14} />
            </HeaderButton>
            <HeaderButton scheme={scheme} label="Close" onClick={() => workspace.closeColumn(workspace.columns.indexOf(column))}>
                <Icon name="close" size={13} />
            </HeaderButton>
        </div>
    );
}

function shorten(cwd: string): string {
    const clean = cwd.replace(/\\/g, '/');
    const parts = clean.split('/').filter(Boolean);
    if (parts.length <= 3) return clean;
    return '…/' + parts.slice(-2).join('/');
}

function HeaderButton(props: { scheme: Scheme; label: string; onClick: () => void; children: any }) {
    return (
        <button
            aria-label={props.label}
            title={props.label}
            className="flex size-6 items-center justify-center rounded-md transition-colors hover:bg-white/10"
            style={{ color: 'inherit' }}
            onPointerDown={(e: any) => e.stopPropagation()}
            onClick={(e: any) => {
                e.stopPropagation();
                props.onClick();
            }}
        >
            {props.children}
        </button>
    );
}

/** The dots along the bottom: one for each column, the open one long, which can be clicked to go there. */
export function StripDots(props: { workspace: Workspace; scheme: Scheme; version?: number }) {
    const { workspace, scheme } = props;
    void props.version;
    return (
        <div className="flex items-center gap-1.5">
            {workspace.columns.map((column, i) => {
                const session = column.sessions[column.active];
                if (column.routine || !session) {
                    const title = column.routine?.name ?? pageLabel(column).title;
                    const status = column.routine?.runner.tabStatus;
                    const color = status ? scheme.ansi[status === 'failed' ? 1 : 3] : scheme.fg;
                    return (
                        <button
                            key={column.id}
                            aria-label={`Column ${i + 1}`}
                            title={`${i + 1}: ${title}`}
                            className="h-[5px] rounded-full transition-all duration-300"
                            style={{ width: i === workspace.focus ? 18 : 5, background: withAlpha(color, status ? 0.9 : i === workspace.focus ? 0.7 : 0.22) }}
                            onClick={() => workspace.focusColumn(i)}
                        />
                    );
                }
                const busy = session.commands.length > 0 && session.commands[session.commands.length - 1].endLine === null && session.commands[session.commands.length - 1].outputLine !== null;
                const failed = session.exitCode !== null;
                const attention = column.sessions.some((pane) => pane.attention);
                const color = attention ? scheme.ansi[5] : failed ? scheme.ansi[1] : busy ? scheme.ansi[3] : i === workspace.focus ? scheme.ansi[4] : scheme.fg;
                return (
                    <button
                        key={column.id}
                        aria-label={`Column ${i + 1}`}
                        title={`${i + 1}: ${session.displayName}${attention ? ' · Needs attention' : ''}`}
                        className="h-[5px] rounded-full transition-all duration-300"
                        style={{ width: i === workspace.focus ? 18 : 5, background: withAlpha(attention || busy || failed ? color : scheme.fg, i === workspace.focus ? 0.7 : attention || busy || failed ? 0.9 : 0.22) }}
                        onClick={() => workspace.focusColumn(i)}
                    />
                );
            })}
        </div>
    );
}
