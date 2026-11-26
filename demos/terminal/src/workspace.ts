// The strip: the columns, which has the keyboard, and what's done to them. A column holds one shell, or a few stacked one over
// another. It's a store like a session is, and what it keeps of a run (which shells, where, how wide) is saved for the next.

import { launchFor, profileById } from './launch';
import { newId, Session } from './term/Session';
import type { SessionOptions } from './term/Session';
import { resolveScheme } from './themes';
import type { Scheme } from './themes';
import { settingsStore, storageGet, storageSet } from './settings';
import type { ColumnWidth } from './settings';
import { defaultTheme } from './vt/terminal';
import type { Command, Theme } from './vt/terminal';
import type { RoutineRunner } from './routines';
import { offsetPanes, paneRects, removePane, restorePaneLayout, splitPaneLayout, stackedPanes } from './strip/panes';
import type { PaneLayout, PaneSide } from './strip/panes';

const settings = () => settingsStore.get();
const MAX_PANES = 4;

/** What a tab is: shells, or one of the app's own pages. */
export type TabKind = 'terminal' | 'browser' | 'settings' | 'studio' | 'recordings' | 'routines';

/** What a page tab is showing: the section of settings, the recording chosen, where the browser is. */
export interface TabView {
    page?: string;
    recording?: string | null;
    /** Where a browser was sent; `at` is where it is now. */
    url?: string;
    at?: string;
    title?: string;
    icon?: string;
}

export interface Column {
    id: number;
    kind: TabKind;
    view: TabView;
    routine?: { name: string; runner: RoutineRunner; close: () => void };
    /** A terminal tab's shells; empty for a page. */
    sessions: Session[];
    /** Omitted for the original, evenly stacked layout. */
    paneLayout?: PaneLayout;
    /** Which shell has the keyboard. */
    active: number;
    zoomed?: boolean;
    width: ColumnWidth;
}

/**
 * A workspace: a set of tabs of its own, for one thing being worked on, with a name and a color to tell it by. New shells in it
 * start in its folder, if it has one. Its shells carry on running while another workspace is open.
 */
export interface Space {
    id: number;
    name: string;
    /** Which of the scheme's colors marks it: an index into its ANSI colors. */
    color: number;
    folder: string;
    columns: Column[];
    focus: number;
}

/** The colors a workspace can be marked with, as ANSI color numbers. */
export const SPACE_COLORS = [4, 5, 2, 3, 6, 1];

export interface Toast {
    id: number;
    text: string;
    detail?: string;
    tone: 'info' | 'ok' | 'bad';
    until: number;
}

export interface Hooks {
    onBell: (session: Session) => void;
    onFinished: (session: Session, command: Command) => void;
    onClipboard: (text: string) => void;
    onNotify: (session: Session, title: string, body: string) => void;
    onProgress: (session: Session) => void;
}

type SavedShell = { profile: string; cwd: string; transcript?: string[]; bookmarks?: { text: string; output: string }[] };
type SavedColumn = SavedShell & { kind?: TabKind; view?: TabView; width: ColumnWidth; stack: SavedShell[]; active?: number; paneLayout?: PaneLayout };

interface Saved {
    /** Before there were workspaces, the one set of tabs. */
    columns?: SavedColumn[];
    focus?: number;
    spaces?: { name: string; color: number; folder: string; columns: SavedColumn[]; focus: number }[];
    space?: number;
}

export function themeOf(scheme: Scheme): Theme {
    return { fg: scheme.fg, bg: scheme.bg, cursor: scheme.cursor, ansi: scheme.ansi };
}

export class Workspace {
    spaces: Space[] = [newSpace('Main', SPACE_COLORS[0])];
    /** Which workspace is open. */
    spaceIndex = 0;
    overview = false;
    toasts: Toast[] = [];
    version = 0;
    /** The size a new shell starts at, until its column measures itself. */
    startSize = { cols: 100, rows: 30 };
    hooks: Hooks;

    #listeners = new Set<() => void>();
    #pending = false;
    #saveTimer: ReturnType<typeof setTimeout> | null = null;
    #transcriptDirty = false;

    constructor(hooks: Hooks) {
        this.hooks = hooks;
    }

    subscribe = (listener: () => void) => {
        this.#listeners.add(listener);
        return () => this.#listeners.delete(listener);
    };
    getVersion = () => this.version;

    changed() {
        const current = this.columns[this.focus];
        if (current && current.kind === 'terminal') this.#lastTerminal = current.id;
        this.paneChanged();
        this.scheduleSave();
    }

    paneChanged() {
        if (this.#pending) return;
        this.#pending = true;
        setTimeout(() => {
            this.#pending = false;
            this.version++;
            for (const listener of [...this.#listeners]) listener();
        }, 0);
    }

    get space(): Space {
        return this.spaces[this.spaceIndex];
    }

    /** The open workspace's tabs. */
    get columns(): Column[] {
        return this.space.columns;
    }

    get focus(): number {
        return this.space.focus;
    }

    set focus(value: number) {
        this.space.focus = value;
    }

    /** Every tab of every workspace. */
    get allColumns(): Column[] {
        return this.spaces.flatMap((space) => space.columns);
    }

    // ---- Workspaces

    /** Opens a workspace, by its place in the list. */
    switchSpace(index: number) {
        if (index < 0 || index >= this.spaces.length || index === this.spaceIndex) return;
        this.spaceIndex = index;
        this.overview = false;
        if (this.columns.length === 0) this.addColumn(undefined, this.space.folder || undefined, 'end');
        this.changed();
    }

    moveSpace(by: number) {
        const n = this.spaces.length;
        if (n > 1) this.switchSpace((this.spaceIndex + by + n) % n);
    }

    /** A new workspace, with a terminal in it, opened. */
    addSpace(name?: string, folder = ''): Space {
        const used = new Set(this.spaces.map((s) => s.color));
        const color = SPACE_COLORS.find((c) => !used.has(c)) ?? SPACE_COLORS[this.spaces.length % SPACE_COLORS.length];
        const space = newSpace(name ?? nextName(this.spaces), color, folder);
        this.spaces.push(space);
        this.spaceIndex = this.spaces.length - 1;
        this.overview = false;
        this.addColumn(undefined, folder || undefined, 'end');
        this.changed();
        return space;
    }

    editSpace(space: Space, patch: Partial<Pick<Space, 'name' | 'color' | 'folder'>>) {
        Object.assign(space, patch);
        this.changed();
    }

    /** Closes a workspace and every shell in it. The last one is never closed. */
    closeSpace(index = this.spaceIndex) {
        if (this.spaces.length <= 1) return;
        const [space] = this.spaces.splice(index, 1);
        for (const column of space.columns) {
            column.routine?.close();
            for (const session of column.sessions) session.dispose();
        }
        if (this.spaceIndex >= index) this.spaceIndex = Math.max(0, this.spaceIndex - 1);
        if (this.columns.length === 0) this.addColumn(undefined, this.space.folder || undefined, 'end');
        this.changed();
    }

    reorderSpace(from: number, to: number) {
        if (from === to || to < 0 || to >= this.spaces.length) return;
        const open = this.space;
        const [space] = this.spaces.splice(from, 1);
        this.spaces.splice(to, 0, space);
        this.spaceIndex = this.spaces.indexOf(open);
        this.changed();
    }

    /** Moves the open tab to another workspace, and goes with it. */
    moveColumnToSpace(target: Space) {
        const column = this.current;
        if (!column || target === this.space) return;
        const from = this.space;
        from.columns.splice(from.focus, 1);
        from.focus = Math.max(0, Math.min(from.focus, from.columns.length - 1));
        target.columns.push(column);
        target.focus = target.columns.length - 1;
        this.spaceIndex = this.spaces.indexOf(target);
        if (from.columns.length === 0) {
            const session = this.spawn(undefined, from.folder || undefined);
            from.columns.push({ id: newId(), kind: 'terminal', view: {}, sessions: [session], active: 0, width: settings().columnWidth });
        }
        this.changed();
    }

    /** Which workspace a shell is in. */
    spaceOf(session: Session): Space | undefined {
        return this.spaces.find((space) => space.columns.some((column) => column.sessions.includes(session)));
    }

    get current(): Column | undefined {
        return this.columns[this.focus];
    }

    get currentSession(): Session | undefined {
        const column = this.current;
        return column && column.kind === 'terminal' ? column.sessions[column.active] : undefined;
    }

    get lastTerminalSession(): Session | undefined {
        const column = this.columns.find((column) => column.id === this.#lastTerminal && column.kind === 'terminal');
        return column?.sessions[column.active];
    }

    // ---- Starting shells

    spawn(profileId?: string, cwd?: string, optionsPatch?: Pick<SessionOptions, 'transcript' | 'savedBookmarks' | 'label'>): Session {
        const settings = settingsStore.get();
        const profile = profileById(settings, profileId ?? settings.profile);
        const launch = launchFor(profile, settings.prompt, cwd);
        const options: SessionOptions = {
            id: newId(),
            profileId: profile.id,
            profileName: profile.name,
            command: launch.command,
            cwd: cwd && /^(?:[A-Za-z]:[\\/]|\\\\)/.test(cwd) && profile.source !== 'ssh' ? cwd : undefined,
            initialCwd: profile.source === 'wsl' && cwd?.startsWith('/') ? cwd : undefined,
            env: launch.env,
            cols: this.startSize.cols,
            rows: this.startSize.rows,
            scrollback: settings.scrollback,
            theme: themeOf(resolveScheme(settings.scheme, settings.schemeOverrides[settings.scheme])),
            settings: () => settingsStore.get(),
            ...optionsPatch,
            host: profile.host,
            onOutput: () => {
                if (settingsStore.get().restoreSession && settingsStore.get().restoreLines > 0 && !this.#transcriptDirty) {
                    this.#transcriptDirty = true;
                    this.scheduleSave();
                }
            },
            onAttention: () => this.paneChanged(),
            onBell: this.hooks.onBell,
            onFinished: this.hooks.onFinished,
            onClipboard: this.hooks.onClipboard,
            onNotify: this.hooks.onNotify,
            onProgress: this.hooks.onProgress,
            onExit: () => {
                this.changed();
            },
        };
        return new Session(options);
    }

    // ---- Columns

    addColumn(profileId?: string, cwd?: string, where: 'after' | 'end' | 'before' = 'after'): Column {
        const settings = settingsStore.get();
        const session = this.spawn(profileId, cwd ?? this.currentSession?.cwd ?? (this.space.folder || undefined));
        const column: Column = { id: newId(), kind: 'terminal', view: {}, sessions: [session], active: 0, width: settings.columnWidth };
        const at = where === 'end' || this.columns.length === 0 ? this.columns.length : where === 'after' ? this.focus + 1 : this.focus;
        this.columns.splice(at, 0, column);
        this.focus = at;
        this.overview = false;
        this.changed();
        return column;
    }

    openRoutine(name: string, runner: RoutineRunner): Column {
        const unsubscribe = runner.subscribe(() => this.paneChanged());
        const routine = { name, runner, close: () => { unsubscribe(); runner.stop(); } };
        const column: Column = { id: newId(), kind: 'terminal', routine, view: {}, sessions: [], active: 0, width: settings().columnWidth };
        this.columns.splice(this.focus + 1, 0, column);
        this.focus = this.columns.indexOf(column);
        this.overview = false;
        this.changed();
        return column;
    }

    focusTab(column: Column) {
        const space = this.spaces.find(({ columns }) => columns.includes(column));
        if (!space) return;
        this.switchSpace(this.spaces.indexOf(space));
        this.focusColumn(space.columns.indexOf(column));
    }

    closeSession(session: Session) {
        const space = this.spaceOf(session);
        if (!space) return;
        const index = space.columns.findIndex((column) => column.sessions.includes(session));
        const column = space.columns[index];
        const at = column.sessions.indexOf(session);
        column.sessions.splice(at, 1);
        session.dispose();
        if (column.sessions.length === 0 && !column.routine) {
            space.columns.splice(index, 1);
            if (space.focus > index || space.focus >= space.columns.length) space.focus = Math.max(0, space.focus - 1);
        } else {
            if (column.paneLayout !== undefined) {
                column.paneLayout = removePane(column.paneLayout, at) ?? undefined;
            }
            column.active = Math.max(0, Math.min(column.active - Number(at < column.active), column.sessions.length - 1));
            if (column.sessions.length <= 1) {
                column.paneLayout = undefined;
                column.zoomed = false;
            }
        }
        this.changed();
    }

    /** A shell that ended is started again in its place, in the folder it was in. */
    restart(session: Session) {
        for (const column of this.allColumns) {
            const at = column.sessions.indexOf(session);
            if (at < 0) continue;
            if (column.routine) return;
            const fresh = this.spawn(session.profileId, session.cwd);
            column.sessions[at] = fresh;
            session.dispose();
            this.changed();
            return;
        }
    }

    closeColumn(index = this.focus) {
        const column = this.columns[index];
        if (!column) return;
        column.routine?.close();
        for (const session of column.sessions) session.dispose();
        this.columns.splice(index, 1);
        if (this.focus > index || this.focus >= this.columns.length) this.focus = Math.max(0, this.focus - 1);
        this.changed();
    }

    /**
     * Opens one of the app's pages as a tab: the one that's open already, if there is one, or a new one after the open tab.
     * A browser always gets a tab of its own.
     */
    openTool(kind: Exclude<TabKind, 'terminal'>, view: TabView = {}) {
        if (kind !== 'browser') {
            const at = this.columns.findIndex((column) => column.kind === kind);
            if (at >= 0) {
                Object.assign(this.columns[at].view, view);
                this.focusColumn(at);
                this.changed();
                return;
            }
        }
        const column: Column = { id: newId(), kind, view: { ...view }, sessions: [], active: 0, width: settings().columnWidth };
        const at = this.columns.length === 0 ? 0 : this.focus + 1;
        this.columns.splice(at, 0, column);
        this.focus = at;
        this.overview = false;
        this.changed();
    }

    /** Changes what a page tab shows. */
    setView(column: Column, patch: TabView) {
        Object.assign(column.view, patch);
        this.changed();
    }

    /** Back from a page to the terminal that was open before it, or to the empty home view. */
    back() {
        const at = this.columns.findIndex((column) => column.id === this.#lastTerminal && column.kind === 'terminal');
        const fallback = this.columns.findIndex((column) => column.kind === 'terminal');
        if (at >= 0) this.focusColumn(at);
        else if (fallback >= 0) this.focusColumn(fallback);
        else if (this.current || this.overview) {
            this.focus = -1;
            this.overview = false;
            this.changed();
        }
    }

    #lastTerminal = 0;

    focusColumn(index: number) {
        if (this.columns.length === 0) return;
        const next = Math.max(0, Math.min(this.columns.length - 1, index));
        if (next === this.focus && !this.overview) return;
        this.focus = next;
        this.overview = false;
        this.changed();
    }

    moveFocus(by: number) {
        this.focusColumn(this.focus + by);
    }

    moveColumn(by: -1 | 1) {
        this.reorderColumn(this.focus, this.focus + by);
    }

    reorderColumn(from: number, to: number) {
        const column = this.columns[from];
        if (!column || from === to || to < 0 || to >= this.columns.length) return;
        const current = this.current;
        this.columns.splice(from, 1);
        this.columns.splice(to, 0, column);
        if (current) this.focus = this.columns.indexOf(current);
        this.changed();
    }

    setWidth(width: ColumnWidth, column = this.current) {
        if (!column) return;
        column.width = width;
        this.changed();
    }

    cycleWidth(column = this.current) {
        if (!column) return;
        const order: ColumnWidth[] = ['third', 'half', 'twothirds', 'wide', 'full'];
        column.width = order[(order.indexOf(column.width) + 1) % order.length];
        this.changed();
    }

    // ---- Stacks

    canDockTab(sourceId: number, target: Column): boolean {
        const source = this.columns.find((column) => column.id === sourceId);
        return !this.overview && target === this.current && source !== undefined && source !== target &&
            source.kind === 'terminal' && target.kind === 'terminal' && !source.routine && !target.routine &&
            source.sessions.length > 0 && target.sessions.length > 0 &&
            source.sessions.length + target.sessions.length <= MAX_PANES;
    }

    /** Moves the live shells from another tab into a split of the current tab. */
    dockTab(sourceId: number, target: Column, targetSessionId: number | null, side: PaneSide) {
        if (!this.canDockTab(sourceId, target)) return;
        const sourceIndex = this.columns.findIndex((column) => column.id === sourceId);
        const source = this.columns[sourceIndex];
        const at = targetSessionId === null ? null : target.sessions.findIndex((session) => session.id === targetSessionId);
        if (at === -1 || (at === null && !target.zoomed)) return;
        const count = target.sessions.length;
        const incoming = offsetPanes(source.paneLayout ?? stackedPanes(source.sessions.length), count);
        target.paneLayout = splitPaneLayout(target.paneLayout ?? stackedPanes(count), at, incoming, side);
        target.sessions.push(...source.sessions);
        target.active = count + source.active;
        target.zoomed = false;
        this.columns.splice(sourceIndex, 1);
        this.focus = this.columns.indexOf(target);
        this.changed();
    }

    split(profileId?: string) {
        const column = this.current;
        if (!column || column.kind !== 'terminal' || column.routine || column.sessions.length >= MAX_PANES) return;
        const session = this.spawn(profileId, this.currentSession?.cwd);
        if (column.paneLayout !== undefined) {
            const next = column.sessions.length;
            column.paneLayout = splitPaneLayout(column.paneLayout, column.active, next, 'bottom');
            column.sessions.push(session);
            column.active = next;
        } else {
            column.sessions.splice(column.active + 1, 0, session);
            column.active++;
        }
        this.changed();
    }

    focusPane(by: number) {
        const column = this.current;
        if (!column) return;
        const order = column.paneLayout === undefined ? column.sessions.map((_, i) => i) : paneRects(column.paneLayout).map(({ index }) => index);
        const at = order.indexOf(column.active);
        const next = order[Math.max(0, Math.min(order.length - 1, at + by))];
        if (next === undefined) return;
        if (next !== column.active) {
            column.active = next;
            this.changed();
        }
    }

    activate(session: Session) {
        const space = this.spaceOf(session);
        if (!space) return;
        if (space !== this.space) {
            this.spaceIndex = this.spaces.indexOf(space);
            this.changed();
        }
        const index = this.columns.findIndex((column) => column.sessions.includes(session));
        if (index < 0) return;
        const column = this.columns[index];
        const at = column.sessions.indexOf(session);
        if (this.focus !== index || column.active !== at || this.overview) {
            this.focus = index;
            column.active = at;
            this.overview = false;
            this.changed();
        }
    }

    toggleOverview(on = !this.overview) {
        this.overview = on;
        this.changed();
    }

    // ---- Messages

    toast(text: string, tone: Toast['tone'] = 'info', detail?: string, ms = 3200) {
        const toast: Toast = { id: newId(), text, tone, detail, until: Date.now() + ms };
        this.toasts = [...this.toasts.filter((t) => t.until > Date.now()), toast];
        this.changed();
        setTimeout(() => {
            this.toasts = this.toasts.filter((t) => t.id !== toast.id);
            this.changed();
        }, ms + 50);
    }

    // ---- Keeping and restoring

    togglePaneZoom() {
        const column = this.current;
        if (!column || column.sessions.length < 2) return;
        column.zoomed = !column.zoomed;
        this.changed();
    }

    scheduleSave() {
        if (this.#saveTimer !== null) return;
        this.#saveTimer = setTimeout(() => this.save(), this.#transcriptDirty ? 2500 : 100);
    }

    savedLayoutUsesDiscoveredProfiles() {
        const layout = storageGet('terminal.layout');
        return layout !== null && /"profile"\s*:\s*"(?:ssh|wsl):/.test(layout);
    }

    save() {
        if (this.#saveTimer !== null) clearTimeout(this.#saveTimer);
        this.#saveTimer = null;
        this.#transcriptDirty = false;
        const lines = settings().restoreSession ? settings().restoreLines : 0;
        let remaining = 1024 * 1024;
        const saveShell = (session?: Session): SavedShell => {
            if (!session) return { profile: '', cwd: '' };
            const transcript = lines && remaining > 0 ? session.term.transcript(lines, Math.min(65536, remaining)) : undefined;
            if (transcript) remaining -= transcript.reduce((size, line) => size + line.length + 1, 0);
            const bookmarks: SavedShell['bookmarks'] = [];
            if (lines) for (const { command, output } of session.bookmarks.values()) {
                const text = command.text.slice(0, 8192);
                if (!text || remaining <= text.length) continue;
                const kept = output.slice(0, remaining - text.length);
                bookmarks.push({ text, output: kept });
                remaining -= text.length + kept.length;
            }
            return { profile: session.profileId, cwd: session.cwd, transcript, bookmarks: bookmarks.length ? bookmarks : undefined };
        };
        const saved: Saved = {
            spaces: this.spaces.map((space) => {
                const columns = space.columns.filter((column) => !column.routine);
                const before = space.columns.slice(0, space.focus).filter((column) => !column.routine).length;
                return {
                    name: space.name,
                    color: space.color,
                    folder: space.folder,
                    columns: columns.map((column) => saveColumn(column, saveShell)),
                    focus: Math.max(0, Math.min(before, columns.length - 1)),
                };
            }),
            space: this.spaceIndex,
        };
        storageSet('terminal.layout', JSON.stringify(saved));
    }

    restore(): boolean {
        const text = storageGet('terminal.layout');
        if (text === null) return false;
        let saved: Saved;
        try {
            saved = JSON.parse(text) as Saved;
        } catch {
            return false;
        }
        const spaces = Array.isArray(saved.spaces)
            ? saved.spaces
            : Array.isArray(saved.columns)
              ? [{ name: 'Main', color: SPACE_COLORS[0], folder: '', columns: saved.columns, focus: saved.focus ?? 0 }]
              : [];
        if (spaces.length === 0) return false;
        this.spaces = [];
        for (const item of spaces.slice(0, 12)) {
            const space = newSpace(item.name || 'Workspace', item.color ?? SPACE_COLORS[0], item.folder ?? '');
            space.columns = (item.columns ?? []).slice(0, 12).map((c) => this.#restoreColumn(c));
            space.focus = Math.max(0, Math.min(space.columns.length - 1, item.focus ?? 0));
            this.spaces.push(space);
        }
        this.spaceIndex = Math.max(0, Math.min(this.spaces.length - 1, saved.space ?? 0));
        if (this.columns.length === 0) this.addColumn(undefined, this.space.folder || undefined, 'end');
        this.changed();
        return true;
    }

    #restoreColumn(item: SavedColumn): Column {
        if (item.kind && item.kind !== 'terminal') {
            return { id: newId(), kind: item.kind, view: { ...(item.view ?? {}) }, sessions: [], active: 0, width: item.width };
        }
        const restore = (shell: SavedShell) => {
            const limit = settings().restoreLines;
            const savedBookmarks = limit && Array.isArray(shell.bookmarks) ? shell.bookmarks.slice(0, 32)
                .filter((entry) => typeof entry?.text === 'string' && typeof entry.output === 'string')
                .map(({ text, output }) => ({ text: text.slice(0, 8192), output: output.slice(0, 16384) })) : undefined;
            return this.spawn(shell.profile, shell.cwd, { transcript: restoredLines(shell.transcript, limit), savedBookmarks });
        };
        const session = restore(item);
        const column: Column = { id: newId(), kind: 'terminal', view: {}, sessions: [session], active: 0, width: item.width };
        for (const more of item.stack ?? []) column.sessions.push(restore(more));
        column.paneLayout = restorePaneLayout(item.paneLayout, column.sessions.length);
        column.active = Math.min(column.sessions.length - 1, Math.max(0, item.active ?? 0));
        return column;
    }

    setTheme(scheme: Scheme) {
        const theme = themeOf(scheme);
        for (const column of this.allColumns) {
            for (const session of column.sessions) {
                session.term.theme = theme;
                session.changed();
            }
        }
    }
}

function newSpace(name: string, color: number, folder = ''): Space {
    return { id: newId(), name, color, folder, columns: [], focus: 0 };
}

function nextName(spaces: Space[]): string {
    for (let n = spaces.length + 1; ; n++) {
        const name = `Workspace ${n}`;
        if (!spaces.some((s) => s.name === name)) return name;
    }
}

function restoredLines(value: unknown, max: number): string[] | undefined {
    if (!max || !Array.isArray(value)) return undefined;
    return value.slice(-max).filter((line): line is string => typeof line === 'string').map((line) => line.slice(0, 4096));
}

function saveColumn(column: Column, saveShell: (session?: Session) => SavedShell): SavedColumn {
    return {
        kind: column.kind,
        view: column.kind === 'browser' ? { url: column.view.at ?? column.view.url } : column.kind === 'settings' ? { page: column.view.page } : {},
        ...saveShell(column.sessions[0]),
        width: column.width,
        stack: column.sessions.slice(1).map(saveShell),
        active: column.active,
        paneLayout: column.paneLayout,
    };
}

export { defaultTheme };
