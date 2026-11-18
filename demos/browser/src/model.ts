// What the browser keeps: its settings, its spaces with their tabs, the favorites, and where it's been. Each is one key in the
// app's own store, saved as it changes.

import { storage } from 'rsui/desktop';

/** A page open in a space, or one kept in its pinned section. */
export interface Tab {
    id: string;
    url: string;
    title: string;
    /** The page's icon as a `data:` PNG, or '' for its letter. */
    icon: string;
    /** When it was last looked at, for putting tabs that haven't been to sleep. */
    seen: number;
}

/** One of the sites along the top of the sidebar, the same in every space. */
export interface Favorite {
    id: string;
    url: string;
    title: string;
    icon: string;
}

/** A set of tabs with a color of its own, and cookies of its own if it's kept apart. */
export interface Space {
    id: string;
    name: string;
    /** The space's accent, which tints the window while it's open. */
    color: string;
    pinned: Tab[];
    tabs: Tab[];
    /** The tab that's open, among the pinned or the others, or null for the start page. */
    active: string | null;
    /** Whether its pages have cookies and storage of their own, apart from the other spaces'. */
    apart: boolean;
}

export interface Session {
    spaces: Space[];
    space: string;
    favorites: Favorite[];
    /** Tabs closed lately, newest last, with where they were, for Ctrl+Shift+T. */
    closed: { space: string; tab: Tab }[];
}

export interface Visit {
    url: string;
    title: string;
    visits: number;
    last: number;
}

export type Theme = 'light' | 'dark' | 'system';

export interface Settings {
    /** Whether the first-run tour has been finished. */
    onboarded: boolean;
    theme: Theme;
    engine: string;
    sidebarSide: 'left' | 'right';
    sidebarWidth: number;
    sidebarHidden: boolean;
    /** Windows 11's material behind the frame, which shows the desktop through it, blurred. */
    material: 'mica' | 'acrylic' | 'none';
    /** How much of the space's color the frame takes, from 0 to 100. */
    tint: number;
    corners: 'soft' | 'round';
    animations: boolean;
    /** Minutes a tab that isn't looked at stays awake, or 0 for always. Asleep, it's only its address, and costs nothing. */
    sleepAfter: number;
    /** Whether Ctrl+T opens the command bar, or a tab with the start page. */
    newTab: 'command' | 'page';
    showFullUrl: boolean;
    /** Whether going somewhere is remembered for suggestions. */
    keepHistory: boolean;
}

export const DEFAULT_SETTINGS: Settings = {
    onboarded: false,
    theme: 'light',
    engine: 'duckduckgo',
    sidebarSide: 'left',
    sidebarWidth: 248,
    sidebarHidden: false,
    material: 'mica',
    tint: 45,
    corners: 'soft',
    animations: true,
    sleepAfter: 30,
    newTab: 'command',
    showFullUrl: false,
    keepHistory: true,
};

/** The colors a space can be, as Arc's are: soft enough that the frame tinted with them stays calm. */
export const SPACE_COLORS = [
    { name: 'Sky', color: '#4f8df5' },
    { name: 'Iris', color: '#7c6cf2' },
    { name: 'Rose', color: '#ec5f86' },
    { name: 'Coral', color: '#f2795a' },
    { name: 'Amber', color: '#e8a33c' },
    { name: 'Sage', color: '#5aa880' },
    { name: 'Teal', color: '#3aa6b0' },
    { name: 'Graphite', color: '#7a7f8c' },
];

export function newId(): string {
    return Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
}

export function newTab(url = '', title = ''): Tab {
    return { id: newId(), url, title, icon: '', seen: Date.now() };
}

export function newSpace(name: string, color: string, apart = false): Space {
    return { id: newId(), name, color, pinned: [], tabs: [], active: null, apart };
}

const store = storage.open('browser');

function read<T extends object>(key: string, fallback: T): T {
    try {
        const kept = store.get(key);
        return kept !== null && typeof kept === 'object' ? ({ ...fallback, ...kept } as T) : fallback;
    } catch {
        return fallback;
    }
}

export function loadSettings(): Settings {
    return read('settings', DEFAULT_SETTINGS);
}

export function saveSettings(settings: Settings): void {
    store.set('settings', settings);
}

export function freshSession(): Session {
    const space = newSpace('Personal', SPACE_COLORS[0].color);
    return { spaces: [space], space: space.id, favorites: [], closed: [] };
}

export function loadSession(): Session {
    const session = read('session', freshSession());
    if (!Array.isArray(session.spaces) || session.spaces.length === 0) {
        return freshSession();
    }
    if (!session.spaces.some((space) => space.id === session.space)) {
        session.space = session.spaces[0].id;
    }
    return session;
}

export function saveSession(session: Session): void {
    // Only the last few closed tabs are worth keeping between runs.
    store.set('session', { ...session, closed: session.closed.slice(-20) });
}

const HISTORY_LIMIT = 3000;

export function loadHistory(): Visit[] {
    const kept = store.get('history');
    return Array.isArray(kept) ? kept : [];
}

/** Where it's been, with `url` visited now, the most recent first. */
export function visited(history: Visit[], url: string, title: string): Visit[] {
    const at = history.findIndex((visit) => visit.url === url);
    const before = at >= 0 ? history[at] : null;
    const visit = { url, title: title || before?.title || '', visits: (before?.visits ?? 0) + 1, last: Date.now() };
    const rest = at >= 0 ? [...history.slice(0, at), ...history.slice(at + 1)] : history;
    return [visit, ...rest].slice(0, HISTORY_LIMIT);
}

/** The same, with a title it's since been given. */
export function retitled(history: Visit[], url: string, title: string): Visit[] {
    const at = history.findIndex((visit) => visit.url === url);
    if (at < 0 || history[at].title === title || title === '') {
        return history;
    }
    const next = history.slice();
    next[at] = { ...next[at], title };
    return next;
}

export function saveHistory(history: Visit[]): void {
    store.set('history', history);
}

export function forgetHistory(): void {
    store.delete('history');
}
