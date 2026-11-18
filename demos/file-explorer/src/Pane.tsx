// One side of a tab: a folder, with its history, what's selected, the search, and everything done to files, drawn as a panel of
// its own. A tab has one of these, or several beside and above each other. Only the one that has the keyboard (`active`) hears it.
// It's as big as layout makes it, and only knows how wide that is in the steps that change what it shows: see `useRoom`.

import { useEffect, useMemo, useRef, useState } from 'react';
import { clipboard, dialogs, drag, menus, shell, type MenuItem } from 'rsui/desktop';
import { promises as fs, watch } from 'fs';
import { exec } from 'child_process';

import {
    COMPUTER, formatSize, freeName, join, matcher, nameOf, parentOf, readFolder, search, sortEntries, transfer, type Drive, type Entry,
} from './files';
import { keyOf } from './keys';
import { COLUMN_WIDTH, ColumnHeaders, FileList, type Handlers, type Look } from './List';
import { COMMANDS, type Command, type PaneState, type Session, type Settings, type SortKey, type View } from './settings';
import { driveIcon } from './Sidebar';
import { PaneBar } from './Toolbar';
import { UsageBar } from './ui';

interface Listing {
    folder: string;
    entries: Entry[];
    error: string | null;
    took: number;
}

/** What the window itself does with these, whichever side has the keyboard: see `App`. */
export const WINDOW_COMMANDS = new Set<Command>([
    'newTab', 'closeTab', 'nextTab', 'previousTab', 'split', 'splitDown', 'closePane', 'otherPane', 'paneLeft', 'paneRight', 'paneUp', 'paneDown',
    'evenPanes', 'collapseSidebar', 'customize', 'settings', 'toggleSidebar', 'togglePreview',
]);

/** What the panel says about itself, for the tab it's in, the preview and the window's title. */
export interface PaneReport {
    state: PaneState;
    entry: Entry | null;
    count: number;
}

export const BAR_HEIGHT = 46;
export const STATUS_HEIGHT = 28;
export const HEADER_HEIGHT = 30;

/** What a pane's showing that it carries on with when it's made again somewhere else, as it is when it's split the other way. */
interface Kept {
    history: { stack: string[]; at: number };
    query: string;
    recursive: boolean;
    selected: Set<string>;
    cursor: string | null;
    anchor: string | null;
}

/** Each pane's, by its id, as it last drew. */
const kept = new Map<string, Kept>();

/** Lets go of what's kept for panes that aren't in `live`, which are gone. */
export function forgetPanes(live: Set<string>) {
    for (const id of [...kept.keys()]) {
        if (!live.has(id)) {
            kept.delete(id);
        }
    }
}

/** What a pane shows, for how wide layout's made it: the toolbar's narrower forms, and which columns fit. */
interface Room {
    narrow: boolean;
    tiny: boolean;
    cols: Look['cols'];
}

/** The columns that fit `width`: in a narrow panel, as one side of a split is, the least useful go first, so names keep room. */
function columnsFor(width: number, look: Look): Look['cols'] {
    const cols = { ...look.cols };
    const room = (key: keyof typeof cols) => (cols[key] ? COLUMN_WIDTH[key] + 12 : 0);
    const used = () => (Object.keys(cols) as (keyof typeof cols)[]).reduce((sum, key) => sum + room(key), 0);
    for (const key of ['attributes', 'created', 'type', 'modified', 'size'] as const) {
        if (width - used() - look.iconSize - 70 >= 180) {
            break;
        }
        cols[key] = false;
    }
    return cols;
}

function roomFor(width: number, look: Look): Room {
    return { narrow: width < 620, tiny: width < 440, cols: columnsFor(width, look) };
}

/**
 * What `element` has room for, from how wide layout makes it, which is told after it's laid out. Only a change in what that comes
 * to draws the pane again, not every pixel of a window being resized. Until it's been laid out, it's as wide as there's room for.
 */
function useRoom(element: { current: any }, look: Look): Room {
    const width = useRef<number | null>(null);
    const [room, setRoom] = useState<Room>(() => roomFor(Infinity, look));
    const latest = useRef({ look, room });
    latest.current = { look, room };
    const update = () => {
        const next = roomFor(width.current ?? Infinity, latest.current.look);
        if (JSON.stringify(next) !== JSON.stringify(latest.current.room)) {
            latest.current.room = next;
            setRoom(next);
        }
    };
    useEffect(() => {
        const target = element.current;
        if (!target || typeof ResizeObserver !== 'function') {
            return;
        }
        const observer = new ResizeObserver((entries: any[]) => {
            const entry = entries[entries.length - 1];
            width.current = entry?.contentRect?.width ?? null;
            update();
        });
        observer.observe(target);
        return () => observer.disconnect();
    }, []);
    // The columns asked for, or the size of the icons, can change what fits too.
    useEffect(update, [look]);
    return room;
}

export function Pane(props: {
    initial: PaneState;
    settings: Settings;
    /** What's remembered of each folder's view, by its lowercased path, and what's pinned: only what a pane uses of the
     * session, so that what else is remembered, as which pane has the keyboard, doesn't render every pane. */
    views: Session['views'];
    pinned: Session['pinned'];
    drives: Drive[];
    look: Look;
    active: boolean;
    keysOff: boolean;
    change: <K extends keyof Settings>(key: K, value: Settings[K]) => void;
    updateSession: (update: (was: Session) => Session) => void;
    onReport: (report: PaneReport) => void;
    onActivate: (id: string) => void;
    onOpenTab: (path: string) => void;
    /** Opens a folder in a new pane beside this one, to its right or below it. */
    onOpenSplit: (path: string, side: 'right' | 'bottom') => void;
    onRefreshDrives: () => void;
    /** Closes this pane, when it's one of several. */
    onClose?: (id: string) => void;
    /** Ringed softly in the accent, as the one of several panes that has the keyboard. */
    marked?: boolean;
}) {
    const { initial, settings, views, pinned, drives, look, active, keysOff, change, updateSession } = props;
    // Made again somewhere else, it carries on with what it was showing.
    const [was] = useState(() => kept.get(initial.id));
    const [history, setHistory] = useState(() => was?.history ?? { stack: initial.stack, at: initial.at });
    const folder = history.stack[history.at];
    const [listing, setListing] = useState<Listing>({ folder: '\0', entries: [], error: null, took: 0 });
    const [reload, setReload] = useState(0);
    const [query, setQuery] = useState(was?.query ?? '');
    const [recursive, setRecursive] = useState(was?.recursive ?? settings.searchRecursive);
    const [found, setFound] = useState<{ entries: Entry[]; searching: boolean; folders: number } | null>(null);
    const [selected, setSelected] = useState<Set<string>>(() => was?.selected ?? new Set());
    const [cursor, setCursor] = useState<string | null>(was?.cursor ?? null);
    const [anchor, setAnchor] = useState<string | null>(was?.anchor ?? null);
    const [renaming, setRenaming] = useState<string | null>(null);
    const [reveal, setReveal] = useState<{ path: string | null; at: number }>({ path: was?.cursor ?? null, at: 0 });
    kept.set(initial.id, { history, query, recursive, selected, cursor, anchor });
    const [editAddressAt, setEditAddressAt] = useState(0);
    const [focusSearchAt, setFocusSearchAt] = useState(0);
    const [notice, setNotice] = useState<string | null>(null);
    const [now, setNow] = useState(Date.now);
    const clip = useRef<{ paths: string[]; cut: boolean } | null>(null);
    const wantSelect = useRef<string | null>(null);
    const typed = useRef({ text: '', at: 0 });

    // The view and sort, for this folder if it has its own.
    const own = settings.rememberViewPerFolder ? views[folder.toLowerCase()] : undefined;
    const view: View = own?.view ?? settings.view;
    const sortKey: SortKey = own?.sortKey ?? settings.sortKey;
    const sortDescending = own?.sortDescending ?? settings.sortDescending;
    const setViewing = (next: Partial<{ view: View; sortKey: SortKey; sortDescending: boolean }>) => {
        if (settings.rememberViewPerFolder) {
            updateSession((was) => ({
                ...was,
                views: { ...was.views, [folder.toLowerCase()]: { view, sortKey, sortDescending, ...next } },
            }));
        } else {
            for (const [key, value] of Object.entries(next)) {
                change(key as keyof Settings, value as any);
            }
        }
    };

    // Reading the folder

    useEffect(() => {
        let live = true;
        if (folder === COMPUTER) {
            setListing({ folder, entries: [], error: null, took: 0 });
            return;
        }
        const started = performance.now();
        readFolder(folder, settings.dotfilesHidden).then(
            (entries) => live && setListing({ folder, entries, error: null, took: performance.now() - started }),
            (error) => live && setListing({ folder, entries: [], error: String(error?.message ?? error), took: 0 }),
        );
        setNow(Date.now());
        return () => {
            live = false;
        };
    }, [folder, reload, settings.dotfilesHidden]);

    useEffect(() => {
        const clock = setInterval(() => setNow(Date.now()), 60_000);
        return () => clearInterval(clock);
    }, []);

    // Watching it, so what changes outside shows up.
    useEffect(() => {
        if (!settings.liveRefresh || folder === COMPUTER) {
            return;
        }
        let waiting: any = null;
        let watcher: any = null;
        try {
            watcher = watch(folder, () => {
                clearTimeout(waiting);
                waiting = setTimeout(() => setReload((n) => n + 1), 150);
            });
        } catch {
            return;
        }
        return () => {
            clearTimeout(waiting);
            watcher?.close();
        };
    }, [folder, settings.liveRefresh]);

    // Searching below this folder.
    const recursiveSearch = recursive && query.trim() !== '' && folder !== COMPUTER;
    useEffect(() => {
        if (!recursiveSearch) {
            setFound(null);
            return;
        }
        let cancelled = false;
        const matches = matcher(query.trim(), settings.searchMatch, settings.searchCase);
        const later = setTimeout(() => {
            let all: Entry[] = [];
            setFound({ entries: [], searching: true, folders: 0 });
            let shown = 0;
            search(
                folder,
                matches!,
                { depth: settings.searchDepth, most: settings.searchMaxResults, hidden: settings.searchHidden || settings.showHidden, dotfilesHidden: settings.dotfilesHidden },
                (batch) => {
                    all = all.concat(batch);
                    // Shown a few times a second at most, so a big search doesn't spend its time drawing.
                    const at = performance.now();
                    if (at - shown > 120) {
                        shown = at;
                        setFound({ entries: all, searching: true, folders: 0 });
                    }
                },
                () => cancelled,
            ).then((done) => !cancelled && setFound({ entries: all, searching: false, folders: done.folders }));
        }, settings.searchDelay);
        return () => {
            cancelled = true;
            clearTimeout(later);
        };
    }, [recursiveSearch, query, folder, settings.searchMatch, settings.searchCase, settings.searchDepth, settings.searchMaxResults, settings.searchHidden]);

    // What's shown, in order

    const sortTook = useRef(0);
    const items = useMemo(() => {
        const started = performance.now();
        if (folder === COMPUTER) {
            return drives.map((drive) => ({
                name: (drive.label || 'Local Disk') + ' (' + drive.path.slice(0, 2) + ')',
                path: drive.path,
                ext: '',
                kind: 'folder' as const,
                folder: true,
                hidden: false,
                system: false,
                readonly: false,
                link: false,
                size: drive.total - drive.free,
                modified: 0,
                created: 0,
                within: drive.total > 0 ? formatSize(drive.free, settings.sizeUnits) + ' free' : '',
                rank: 0,
            }));
        }
        const source = found ? found.entries : listing.folder === folder ? listing.entries : [];
        const matches = found ? null : matcher(query.trim(), settings.searchMatch, settings.searchCase);
        const shown = source.filter(
            (entry) =>
                (settings.showHidden || !entry.hidden || (entry.system && settings.showSystem)) &&
                (settings.showSystem || !(entry.system && entry.hidden)) &&
                (matches === null || matches(entry.name)),
        );
        const sorted = sortEntries(shown, settings, sortKey, sortDescending, !found);
        sortTook.current = performance.now() - started;
        return sorted;
    }, [
        listing, found, folder, drives, query, sortKey, sortDescending, settings.showHidden, settings.showSystem, settings.foldersFirst,
        settings.naturalSort, settings.caseSensitiveSort, settings.searchMatch, settings.searchCase, settings.sizeUnits,
    ]);
    const byPath = useMemo(() => new Map(items.map((item, at) => [item.path, at])), [items]);

    // When a folder's read, keep what was selected, or choose what was asked for, or the first thing.
    useEffect(() => {
        if (listing.folder !== folder && folder !== COMPUTER) {
            return;
        }
        const want = wantSelect.current;
        wantSelect.current = null;
        if (want && byPath.has(want)) {
            setSelected(new Set([want]));
            setCursor(want);
            setAnchor(want);
            setReveal({ path: want, at: Date.now() });
            return;
        }
        const kept = new Set([...selected].filter((path) => byPath.has(path)));
        if (kept.size !== selected.size) {
            setSelected(kept);
        }
        if (cursor === null || !byPath.has(cursor)) {
            const first = settings.selectFirstOnOpen && items.length > 0 ? items[0].path : null;
            setCursor(first);
            setAnchor(first);
            if (first && kept.size === 0) {
                setSelected(new Set([first]));
            }
        }
    }, [items]);

    // Moving around

    const go = (path: string, select?: string) => {
        setQuery('');
        setFound(null);
        setRenaming(null);
        setSelected(new Set());
        setCursor(null);
        wantSelect.current = select ?? null;
        setHistory((was) => {
            if (was.stack[was.at] === path) {
                setReload((n) => n + 1);
                return was;
            }
            const stack = was.stack.slice(0, was.at + 1).concat(path).slice(-200);
            return { stack, at: stack.length - 1 };
        });
    };
    const step = (by: number) => {
        const at = history.at + by;
        if (at < 0 || at >= history.stack.length) {
            return;
        }
        setQuery('');
        setFound(null);
        wantSelect.current = by < 0 ? history.stack[history.at] : null;
        setHistory({ ...history, at });
    };
    const up = () => {
        if (folder !== COMPUTER) {
            go(parentOf(folder), folder);
        }
    };

    // Doing things to files

    const say = (text: string) => {
        setNotice(text);
        setTimeout(() => setNotice((was) => (was === text ? null : was)), 4000);
    };
    const chosen = (): string[] => (selected.size > 0 ? [...selected] : cursor ? [cursor] : []);
    const entryAt = (path: string | null) => (path === null ? undefined : items[byPath.get(path) ?? -1]);
    const failed = (what: string) => (error: any) => say(`Couldn't ${what}: ${error?.message ?? error}`);

    const open = (path: string) => {
        const entry = entryAt(path);
        if (!entry) {
            return;
        }
        if (entry.folder) {
            go(entry.path);
        } else {
            shell.open(entry.path).catch(failed('open ' + entry.name));
        }
    };
    const remove = async () => {
        const paths = chosen();
        if (paths.length === 0 || folder === COMPUTER) {
            return;
        }
        if (settings.confirmDelete) {
            const what = paths.length === 1 ? `"${nameOf(paths[0])}"` : `these ${paths.length} items`;
            const answer = await dialogs.message({
                title: 'Delete',
                message: settings.useRecycleBin ? `Move ${what} to the Recycle Bin?` : `Delete ${what} for good? This can't be undone.`,
                kind: 'warning',
                buttons: 'yes-no',
            });
            if (answer !== 'yes') {
                return;
            }
        }
        const index = Math.min(...paths.map((path) => byPath.get(path) ?? 0));
        wantSelect.current = items.filter((item) => !paths.includes(item.path))[Math.max(0, index - 1)]?.path ?? null;
        try {
            if (settings.useRecycleBin) {
                await shell.trash(paths);
            } else {
                for (const path of paths) {
                    await fs.rm(path, { recursive: true, force: true });
                }
            }
            say(`Deleted ${paths.length === 1 ? nameOf(paths[0]) : paths.length + ' items'}`);
        } catch (error) {
            failed('delete')(error);
        }
        setReload((n) => n + 1);
    };
    const newFolder = async () => {
        if (folder === COMPUTER) {
            return;
        }
        const name = freeName(new Set(listing.entries.map((entry) => entry.name)), 'New folder');
        try {
            await fs.mkdir(join(folder, name));
            wantSelect.current = join(folder, name);
            setRenaming(join(folder, name));
            setReload((n) => n + 1);
        } catch (error) {
            failed('make a folder')(error);
        }
    };
    const rename = async (path: string, name: string | null) => {
        setRenaming(null);
        const entry = entryAt(path);
        if (!entry || name === null || name.trim() === '' || name === entry.name) {
            return;
        }
        if (/[\\/:*?"<>|]/.test(name)) {
            say('A name can\'t have any of \\ / : * ? " < > |');
            return;
        }
        const to = join(parentOf(path), name.trim());
        try {
            await fs.rename(path, to);
            wantSelect.current = to;
            setReload((n) => n + 1);
        } catch (error) {
            failed('rename ' + entry.name)(error);
        }
    };
    const copy = (cut: boolean) => {
        const paths = chosen();
        if (paths.length === 0 || folder === COMPUTER) {
            return;
        }
        clip.current = { paths, cut };
        clipboard.write({ files: paths, text: paths.join('\r\n') }).catch(() => {});
        say(`${cut ? 'Cut' : 'Copied'} ${paths.length === 1 ? nameOf(paths[0]) : paths.length + ' items'}`);
    };
    const paste = async (into = folder) => {
        if (into === COMPUTER) {
            return;
        }
        let paths: string[] = [];
        try {
            paths = (await clipboard.read()).files ?? [];
        } catch {
            paths = clip.current?.paths ?? [];
        }
        if (paths.length === 0) {
            return;
        }
        const cut = clip.current?.cut === true && clip.current.paths.join('\n') === paths.join('\n');
        try {
            const made = await transfer(paths, into, cut);
            if (cut) {
                clip.current = null;
            }
            wantSelect.current = made[0] ?? null;
            say(`${cut ? 'Moved' : 'Copied'} ${made.length === 1 ? nameOf(made[0]) : made.length + ' items'}`);
        } catch (error) {
            failed(cut ? 'move' : 'copy')(error);
        }
        setReload((n) => n + 1);
    };
    const dropped = async (paths: string[], move: boolean) => {
        if (folder === COMPUTER || paths.length === 0) {
            return;
        }
        try {
            const made = await transfer(paths, folder, move);
            wantSelect.current = made[0] ?? null;
            say(`${move ? 'Moved' : 'Copied'} ${made.length} item${made.length === 1 ? '' : 's'} here`);
        } catch (error) {
            failed('copy')(error);
        }
        setReload((n) => n + 1);
    };
    const togglePin = (path = folder) => {
        if (path === COMPUTER) {
            return;
        }
        updateSession((was) => ({
            ...was,
            pinned: was.pinned.includes(path) ? was.pinned.filter((one) => one !== path) : [...was.pinned, path],
        }));
    };
    const terminal = (where = folder) => {
        if (where === COMPUTER) {
            return;
        }
        exec(settings.terminalCommand.replace(/\{path\}/g, where), { cwd: where }, (error) => error && say('The terminal command failed: ' + error.message));
    };

    // Picking things with the pointer and the keyboard

    const pickTo = (path: string, mods: { ctrl: boolean; shift: boolean }) => {
        if (mods.shift && anchor !== null && byPath.has(anchor)) {
            const [from, to] = [byPath.get(anchor)!, byPath.get(path)!].sort((a, b) => a - b);
            const range = items.slice(from, to + 1).map((item) => item.path);
            setSelected(mods.ctrl ? new Set([...selected, ...range]) : new Set(range));
        } else if (mods.ctrl) {
            const next = new Set(selected);
            if (next.has(path)) {
                next.delete(path);
            } else {
                next.add(path);
            }
            setSelected(next);
            setAnchor(path);
        } else {
            setSelected(new Set([path]));
            setAnchor(path);
        }
        setCursor(path);
        setReveal({ path, at: Date.now() });
    };

    const moveCursor = (by: number | 'start' | 'end', mods: { ctrl: boolean; shift: boolean }) => {
        if (items.length === 0) {
            return;
        }
        const at = cursor === null ? -1 : byPath.get(cursor) ?? -1;
        let next = by === 'start' ? 0 : by === 'end' ? items.length - 1 : at + by;
        if (typeof by === 'number') {
            if (settings.wrapNavigation && Math.abs(by) === 1) {
                next = (next + items.length) % items.length;
            }
            next = Math.max(0, Math.min(items.length - 1, next));
        }
        const path = items[next].path;
        if (mods.ctrl && !mods.shift) {
            setCursor(path);
            setReveal({ path, at: Date.now() });
        } else {
            pickTo(path, { ctrl: false, shift: mods.shift });
        }
    };

    const run = (command: Command) => {
        switch (command) {
            case 'back': return step(-1);
            case 'forward': return step(1);
            case 'up': return up();
            case 'refresh': props.onRefreshDrives(); return setReload((n) => n + 1);
            case 'focusAddress': return setEditAddressAt(Date.now());
            case 'focusSearch': return setFocusSearchAt(Date.now());
            case 'newFolder': return void newFolder();
            case 'rename': return cursor && folder !== COMPUTER ? setRenaming(cursor) : undefined;
            case 'delete': return void remove();
            case 'copy': return copy(false);
            case 'cut': return copy(true);
            case 'paste': return void paste();
            case 'selectAll': return setSelected(new Set(items.map((item) => item.path)));
            case 'copyPath': {
                const paths = chosen();
                clipboard.write(paths.length ? paths.join('\r\n') : folder).catch(() => {});
                return say('Copied the path');
            }
            case 'toggleHidden': return change('showHidden', !settings.showHidden);
            case 'viewDetails': return setViewing({ view: 'details' });
            case 'viewList': return setViewing({ view: 'list' });
            case 'viewGrid': return setViewing({ view: 'grid' });
            case 'openTerminal': return terminal();
            case 'properties': {
                const path = chosen()[0] ?? folder;
                return path === COMPUTER ? undefined : void shell.properties(path).catch(() => {});
            }
            case 'pin': return togglePin(entryAt(cursor)?.folder ? cursor! : folder);
        }
    };

    // The keyboard is heard on the document, by the side that has it, with whatever this render knows.
    // How many tiles fit across, and rows on a screen, as the list last said, for the keyboard.
    const shape = useRef({ perRow: 1, perPage: 10 });
    const latest = useRef<any>(null);
    latest.current = { run, moveCursor, open, settings, items, cursor, selected, byPath, renaming, view, pickTo, chosen, active, keysOff, shape: shape.current };
    useEffect(() => {
        const heard = (event: any) => {
            const now = latest.current;
            if (!now.active || now.keysOff || now.renaming) {
                return;
            }
            const inField = event.target?.tagName === 'INPUT' || event.target?.tagName === 'TEXTAREA';
            const shortcut = keyOf(event);
            if (!shortcut) {
                return;
            }
            const command = (Object.keys(COMMANDS) as Command[]).find((one) => now.settings.keys[one] === shortcut);
            if (command && WINDOW_COMMANDS.has(command)) {
                return;
            }
            if (inField && (!command || !/^(Ctrl|Alt|F\d)/.test(shortcut) || /^Ctrl\+[ACVXZ]$/.test(shortcut))) {
                return;
            }
            if (command) {
                event.preventDefault();
                now.run(command);
                return;
            }
            if (inField) {
                return;
            }
            const mods = { ctrl: event.ctrlKey, shift: event.shiftKey };
            const { perRow, perPage } = now.shape;
            const across = now.view === 'grid' ? perRow : 1;
            switch (event.key) {
                case 'ArrowDown': event.preventDefault(); return now.moveCursor(across, mods);
                case 'ArrowUp': event.preventDefault(); return now.moveCursor(-across, mods);
                case 'ArrowRight': if (now.view === 'grid') { event.preventDefault(); return now.moveCursor(1, mods); } return;
                case 'ArrowLeft': if (now.view === 'grid') { event.preventDefault(); return now.moveCursor(-1, mods); } return;
                case 'PageDown': event.preventDefault(); return now.moveCursor(perPage * across, mods);
                case 'PageUp': event.preventDefault(); return now.moveCursor(-perPage * across, mods);
                case 'Home': event.preventDefault(); return now.moveCursor('start', mods);
                case 'End': event.preventDefault(); return now.moveCursor('end', mods);
                case 'Enter': {
                    event.preventDefault();
                    const paths: string[] = now.chosen();
                    const files = paths.filter((path) => !now.items[now.byPath.get(path)]?.folder);
                    if (paths.length === 1 || files.length === 0) {
                        return paths[0] && now.open(paths[0]);
                    }
                    return files.forEach((path) => now.open(path));
                }
                case 'Backspace':
                    if (now.settings.backspaceUp) {
                        event.preventDefault();
                        now.run('up');
                    }
                    return;
                case ' ':
                    if (event.ctrlKey && now.cursor) {
                        event.preventDefault();
                        return now.pickTo(now.cursor, { ctrl: true, shift: false });
                    }
                    break;
            }
            // Typing a name goes to the first thing that starts with it.
            if (now.settings.typeahead && event.key.length === 1 && !event.ctrlKey && !event.altKey && !event.metaKey) {
                const at = performance.now();
                typed.current = { text: (at - typed.current.at < 900 ? typed.current.text : '') + event.key.toLowerCase(), at };
                const prefix = typed.current.text;
                const from = now.cursor === null ? 0 : now.byPath.get(now.cursor) ?? 0;
                const list: Entry[] = now.items;
                for (let i = 0; i < list.length; i += 1) {
                    const item = list[(from + (prefix.length === 1 ? 1 : 0) + i) % list.length];
                    if (item.name.toLowerCase().startsWith(prefix)) {
                        now.pickTo(item.path, { ctrl: false, shift: false });
                        break;
                    }
                }
            }
        };
        document.addEventListener('keydown', heard);
        return () => document.removeEventListener('keydown', heard);
    }, []);

    // Menus

    const contextMenu = (path: string | null) => {
        if (path !== null && !selected.has(path)) {
            pickTo(path, { ctrl: false, shift: false });
        }
        const keys = settings.keys;
        const entry = entryAt(path);
        const many = path !== null && selected.has(path) ? selected.size : 1;
        const menu: MenuItem[] = entry
            ? [
                  { label: 'Open', onClick: () => open(entry.path) },
                  ...(entry.folder
                      ? [
                            { label: 'Open in new tab', onClick: () => props.onOpenTab(entry.path) },
                            { label: 'Open beside', onClick: () => props.onOpenSplit(entry.path, 'right') },
                            { label: 'Open below', onClick: () => props.onOpenSplit(entry.path, 'bottom') },
                            { label: 'Open in terminal', onClick: () => terminal(entry.path) },
                        ]
                      : [{ label: 'Open with…', onClick: () => void shell.openWith(entry.path).catch(() => {}) }]),
                  { separator: true },
                  { label: 'Cut', shortcut: keys.cut, onClick: () => run('cut'), disabled: folder === COMPUTER },
                  { label: 'Copy', shortcut: keys.copy, onClick: () => run('copy'), disabled: folder === COMPUTER },
                  ...(entry.folder ? [{ label: 'Paste into folder', onClick: () => void paste(entry.path) }] : []),
                  { label: many > 1 ? `Copy ${many} paths` : 'Copy path', shortcut: keys.copyPath, onClick: () => run('copyPath') },
                  { separator: true },
                  { label: 'Rename', shortcut: keys.rename, onClick: () => setRenaming(entry.path), disabled: many > 1 || folder === COMPUTER },
                  { label: settings.useRecycleBin ? 'Move to Recycle Bin' : 'Delete for good', shortcut: keys.delete, onClick: () => void remove(), disabled: folder === COMPUTER },
                  { separator: true },
                  ...(entry.folder
                      ? [{ label: pinned.includes(entry.path) ? 'Unpin' : 'Pin', onClick: () => togglePin(entry.path) }]
                      : []),
                  { label: 'Show in Explorer', onClick: () => void shell.reveal(entry.path).catch(() => {}) },
                  { label: 'Properties', shortcut: keys.properties, onClick: () => void shell.properties(entry.path).catch(() => {}) },
              ]
            : [
                  { label: 'New folder', shortcut: keys.newFolder, onClick: () => void newFolder(), disabled: folder === COMPUTER },
                  { label: 'Paste', shortcut: keys.paste, onClick: () => void paste(), disabled: folder === COMPUTER },
                  { separator: true },
                  {
                      label: 'View',
                      items: (['details', 'list', 'grid'] as View[]).map((one) => ({ label: one[0].toUpperCase() + one.slice(1), checked: view === one, onClick: () => setViewing({ view: one }) })),
                  },
                  {
                      label: 'Sort by',
                      items: [
                          ...(['name', 'modified', 'created', 'size', 'type'] as SortKey[]).map((one) => ({
                              label: one[0].toUpperCase() + one.slice(1),
                              checked: sortKey === one,
                              onClick: () => setViewing({ sortKey: one }),
                          })),
                          { separator: true },
                          { label: 'Descending', checked: sortDescending, onClick: () => setViewing({ sortDescending: !sortDescending }) },
                          { label: 'Folders first', checked: settings.foldersFirst, onClick: () => change('foldersFirst', !settings.foldersFirst) },
                      ],
                  },
                  { label: 'Hidden items', checked: settings.showHidden, shortcut: keys.toggleHidden, onClick: () => run('toggleHidden') },
                  { label: 'Refresh', shortcut: keys.refresh, onClick: () => run('refresh') },
                  { separator: true },
                  { label: pinned.includes(folder) ? 'Unpin this folder' : 'Pin this folder', onClick: () => togglePin(), disabled: folder === COMPUTER },
                  { label: 'Open in terminal', shortcut: keys.openTerminal, onClick: () => terminal(), disabled: folder === COMPUTER },
                  { label: 'Properties', onClick: () => run('properties'), disabled: folder === COMPUTER },
              ];
        menus.popup(menu);
    };

    // What rows call.
    const handlers: Handlers = {
        onPick: (path, mods) => pickTo(path, mods),
        onActivate: (path, mods) => {
            pickTo(path, mods);
            if (settings.openWith === 'single' && !mods.ctrl && !mods.shift) {
                open(path);
            }
        },
        onOpen: (path) => open(path),
        onContext: (path) => contextMenu(path),
        onDragOut: (path) => {
            const paths: string[] = selected.has(path) ? [...selected] : [path];
            drag.startFiles(paths).then((done: string) => done === 'move' && setReload((n) => n + 1), () => {});
        },
        onRename: (path, name) => void rename(path, name),
        // Clicking where there's nothing lets go of what's selected; with nothing selected it changes nothing.
        onEmptyClick: () => {
            setSelected((was) => (was.size === 0 ? was : new Set()));
            setAnchor(null);
        },
        onEmptyDouble: () => settings.doubleClickEmptyUp && run('up'),
    };

    // What the bar is given.
    const on = {
        shape: (rows: number, columns: number) => {
            shape.current.perRow = Math.max(1, columns);
            shape.current.perPage = Math.max(1, Math.floor(rows) - 1);
        },
        back: () => step(-1),
        forward: () => step(1),
        up: () => up(),
        refresh: () => run('refresh'),
        view: (next: View) => setViewing({ view: next }),
        hidden: () => change('showHidden', !settings.showHidden),
        newFolder: () => void newFolder(),
        sort: (key: SortKey) =>
            setViewing(key === sortKey ? { sortDescending: !sortDescending } : { sortKey: key, sortDescending: key !== 'name' && key !== 'type' }),
        activate: () => props.onActivate(initial.id),
        close: () => props.onClose?.(initial.id),
        dragOver: (event: any) => {
            if (folder !== COMPUTER) {
                event.preventDefault();
            }
        },
        drop: (event: any) => {
            event.preventDefault();
            const paths = (event.dataTransfer?.files ?? []).map((file: any) => file.path).filter(Boolean);
            void dropped(paths, event.shiftKey);
        },
    };

    const section = useRef<any>(null);
    const room = useRoom(section, look);

    // The columns that fit, as `useRoom` has them from how wide it's laid out.
    const cols = room.cols;
    const narrowed = (Object.keys(cols) as (keyof typeof cols)[]).some((key) => cols[key] !== look.cols[key]);
    // The side of a split that doesn't have the keyboard shows what's selected softly, as a window that isn't in front does.
    const soft = !active && look.selection === 'filled';
    const fitted: Look = narrowed || soft ? { ...look, cols, selection: soft ? 'subtle' : look.selection } : look;

    // What it says about itself, for its tab, the preview and the window's title.
    const focusEntry = entryAt(cursor) ?? null;
    useEffect(() => {
        props.onReport({ state: { id: initial.id, stack: history.stack, at: history.at }, entry: selected.size > 1 ? null : focusEntry, count: selected.size });
    }, [history, focusEntry, selected]);

    // Drawing

    let selectedSize = 0;
    for (const path of selected) {
        const entry = items[byPath.get(path) ?? -1];
        if (entry && !entry.folder) {
            selectedSize += entry.size;
        }
    }
    const drive = drives.find((one) => folder.toLowerCase().startsWith(one.path.toLowerCase()));
    const emptyText =
        listing.error && listing.folder === folder
            ? `Can't open this folder: ${listing.error}`
            : query
              ? found?.searching
                  ? 'Searching…'
                  : 'Nothing matches "' + query + '"'
              : listing.folder !== folder
                ? ''
                : 'This folder is empty';
    const headers = view === 'details' && settings.showColumnHeaders && folder !== COMPUTER;

    return (
        <section
            ref={section}
            className={
                'flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-(--fx-radius-lg) bg-(--fx-card) ' +
                (props.marked ? 'shadow-[0_0_0_1.5px_var(--fx-accent-soft),var(--fx-card-shadow)] ' : 'shadow-(--fx-card-shadow) ') +
                (active ? '' : 'opacity-[0.985]')
            }
            onPointerDownCapture={on.activate}
            onDragOver={on.dragOver}
            onDrop={on.drop}
        >
            {settings.showToolbar ? (
                <PaneBar
                    settings={settings}
                    folder={folder}
                    canBack={history.at > 0}
                    canForward={history.at < history.stack.length - 1}
                    view={view}
                    query={query}
                    recursive={recursive}
                    searching={found?.searching === true}
                    editAddressAt={editAddressAt}
                    focusSearchAt={focusSearchAt}
                    narrow={room.narrow}
                    tiny={room.tiny}
                    onClose={props.onClose ? on.close : undefined}
                    onBack={on.back}
                    onForward={on.forward}
                    onRefresh={on.refresh}
                    onGo={go}
                    onQuery={setQuery}
                    onRecursive={setRecursive}
                    onView={on.view}
                    onHidden={on.hidden}
                    onNewFolder={on.newFolder}
                />
            ) : null}
            {headers ? <ColumnHeaders look={fitted} sortKey={sortKey} descending={sortDescending} onSort={on.sort} /> : null}
            {folder === COMPUTER ? (
                <Drives drives={drives} settings={settings} onOpen={go} />
            ) : (
                <FileList
                    items={items}
                    view={view}
                    look={fitted}
                    selected={selected}
                    cursor={cursor}
                    renaming={renaming}
                    reveal={reveal}
                    now={now}
                    empty={emptyText}
                    handlers={handlers}
                    folder={folder}
                    onShape={on.shape}
                />
            )}
            {settings.showStatusBar ? (
                <footer className="flex h-[28px] shrink-0 items-center gap-3 px-4 text-[0.8em] text-(--fx-faint)">
                    <span>
                        {items.length.toLocaleString()} {items.length === 1 ? 'item' : 'items'}
                        {found ? (found.searching ? ' found so far' : ` found in ${found.folders.toLocaleString()} folders`) : ''}
                    </span>
                    {selected.size > 0 ? (
                        <span>
                            {selected.size.toLocaleString()} selected{selectedSize > 0 ? ' · ' + formatSize(selectedSize, settings.sizeUnits) : ''}
                        </span>
                    ) : null}
                    {notice ? <span className="truncate text-(--fx-accent)">{notice}</span> : null}
                    <span className="ml-auto flex items-center gap-3">
                        {settings.showPerf && folder !== COMPUTER ? (
                            <span className="font-mono">
                                read {listing.took.toFixed(1)} ms · sort {sortTook.current.toFixed(1)} ms
                            </span>
                        ) : null}
                        {drive && drive.total > 0 ? <span>{formatSize(drive.free, settings.sizeUnits)} free</span> : null}
                    </span>
                </footer>
            ) : null}
        </section>
    );
}

/** This PC: each drive as a card with how full it is. */
function Drives(props: { drives: Drive[]; settings: Settings; onOpen: (path: string) => void }) {
    const { drives, settings, onOpen } = props;
    return (
        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
            <div className="mb-3 text-[0.8em] font-medium text-(--fx-faint)">Devices and drives</div>
            <div className="flex flex-wrap gap-3">
                {drives.map((drive) => {
                    const Icon = driveIcon(drive.kind);
                    const used = drive.total > 0 ? 1 - drive.free / drive.total : 0;
                    return (
                        <button
                            key={drive.path}
                            onDoubleClick={() => onOpen(drive.path)}
                            onClick={() => settings.openWith === 'single' && onOpen(drive.path)}
                            className="flex w-72 items-center gap-3 rounded-(--fx-radius-lg) bg-(--fx-input) p-3.5 text-left transition-colors duration-(--fx-speed) hover:bg-(--fx-press)"
                        >
                            <span className="flex shrink-0 text-(--fx-accent)"><Icon size={30} strokeWidth={1.4} /></span>
                            <span className="flex min-w-0 flex-1 flex-col gap-1.5">
                                <span className="truncate font-medium">
                                    {drive.label || 'Local Disk'} ({drive.path.slice(0, 2)})
                                </span>
                                {drive.total > 0 ? (
                                    <>
                                        <UsageBar used={used} accent={settings.accent} height={4} />
                                        <span className="text-[0.85em] text-(--fx-muted)">
                                            {formatSize(drive.free, settings.sizeUnits)} free of {formatSize(drive.total, settings.sizeUnits)}
                                        </span>
                                    </>
                                ) : (
                                    <span className="text-[0.85em] text-(--fx-faint)">{drive.kind === 'optical' ? 'No disc' : 'Not available'}</span>
                                )}
                            </span>
                        </button>
                    );
                })}
            </div>
        </div>
    );
}
