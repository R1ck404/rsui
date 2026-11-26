// What can be changed, what it starts as, and keeping it between runs.

import { useSyncExternalStore } from 'react';
import type { BackgroundKind, StudioDesign } from './backgrounds/scenes';
import { DEFAULT_PROMPT } from './prompt';
import type { PromptSettings } from './prompt';
import type { SchemeOverrides } from './themes';
import { isKeymap } from './keymap';
import type { Keymap } from './keymap';
import { isTriggerList } from './term/triggers';
import type { TriggerRule } from './term/triggers';
import type { Terminal } from './vt/terminal';

export type { BackgroundKind } from './backgrounds/scenes';

export type ColumnWidth = 'full' | 'wide' | 'twothirds' | 'half' | 'third';
export type CursorShape = 'program' | 'block' | 'bar' | 'underline';
export type BellMode = 'off' | 'flash' | 'sound' | 'both';
export type Material = 'mica' | 'acrylic' | 'tabbed' | 'none';
/** How the shells are laid out: one at a time, with tabs along the top or down a sidebar, as a strip of columns, or as little as possible. */
export type Layout = 'normal' | 'tabs' | 'sidebar' | 'strip' | 'minimal';

export interface Settings {
    layout: Layout;
    /** The window's own text: the system's, or the terminal's font everywhere. */
    uiFont: 'system' | 'mono';
    showStatusBar: boolean;
    sidebarWidth: number;
    /** Room between the text and the edge of its card. */
    textPadding: number;
    // The background
    background: BackgroundKind;
    /** Its colors: from the scheme (`scheme`), or two of the scheme's colors by name. */
    backgroundColor: 'scheme' | 'blue' | 'purple' | 'green' | 'warm' | 'mono';
    backgroundIntensity: number;
    backgroundSpeed: number;
    backgroundDensity: number;
    /** What the studio made: a picture or video, and what's done to it. Shown when `background` is `custom`. */
    studio: StudioDesign | null;
    // Looks
    scheme: string;
    schemeOverrides: Record<string, SchemeOverrides>;
    fontFamily: string;
    fontSize: number;
    /** How tall a line is, as a share of the font's height. */
    lineHeight: number;
    /** Whether `=>` and `!=` join into one sign, which Cascadia Code does and Cascadia Mono doesn't. */
    ligatures: boolean;
    material: Material;
    /** How much of the frame's tint shows, from 0 for all material to 1 for solid. */
    frameOpacity: number;
    /** How see-through the columns are, over the frame's material. */
    columnOpacity: number;
    radius: number;
    padding: number;
    // The strip
    columnWidth: ColumnWidth;
    /** How much of the next column shows beside the open one, in pixels. */
    peek: number;
    animations: boolean;
    dimOthers: boolean;
    // The cursor
    cursorShape: CursorShape;
    cursorBlink: boolean;
    /** The cursor glides to where it goes instead of jumping. */
    smoothCursor: boolean;
    /** It leaves a short trail as it glides. */
    cursorTrail: boolean;
    // Behavior
    scrollback: number;
    copyOnSelect: boolean;
    warnOnMultilinePaste: boolean;
    pasteAtPromptOnly: boolean;
    graphics: Terminal['graphics'];
    kittyKeyboard: boolean;
    kittyKeysFirst: boolean;
    triggers: TriggerRule[];
    attentionOn: 'notifications' | 'notificationsAndBell' | 'off';
    paneStatus: boolean;
    replaySeconds: number;
    replaySpeed: '1' | '2' | '4' | '8';
    restoreLines: number;
    discoverSsh: boolean;
    discoverWsl: boolean;
    bell: BellMode;
    /** A command that takes this many seconds to finish tells you, if you're looking at something else. 0 is never. */
    notifyAfter: number;
    clickLinks: 'click' | 'ctrl' | 'off';
    linkBrowser: 'system' | 'builtIn';
    showMarks: boolean;
    stickyCommand: boolean;
    scrollMinimap: boolean;
    allowClipboardWrite: boolean;
    profile: string;
    prompt: PromptSettings;
    /** A command line to open a column with, which `profile` can name as `custom`. */
    customCommand: string;
    editor: string;
    keymap: Keymap;
    // Quake
    quakeKey: string;
    quakeHeight: number;
    quakeEnabled: boolean;
    // Restoring
    restoreSession: boolean;
    closeToTray: boolean;
}

export const DEFAULTS: Settings = {
    layout: 'normal',
    uiFont: 'system',
    showStatusBar: false,
    sidebarWidth: 216,
    textPadding: 16,
    background: 'dither',
    backgroundColor: 'scheme',
    backgroundIntensity: 0.7,
    backgroundSpeed: 1,
    backgroundDensity: 1,
    studio: null,
    scheme: 'graphite',
    schemeOverrides: {},
    fontFamily: 'Cascadia Code',
    fontSize: 13.5,
    lineHeight: 1.25,
    ligatures: true,
    material: 'none',
    frameOpacity: 1,
    columnOpacity: 0.97,
    radius: 10,
    padding: 14,
    columnWidth: 'wide',
    peek: 44,
    animations: true,
    dimOthers: true,
    cursorShape: 'program',
    cursorBlink: true,
    smoothCursor: true,
    cursorTrail: false,
    scrollback: 10000,
    copyOnSelect: false,
    warnOnMultilinePaste: true,
    pasteAtPromptOnly: true,
    graphics: 'all',
    kittyKeyboard: true,
    kittyKeysFirst: false,
    triggers: [],
    attentionOn: 'notificationsAndBell',
    paneStatus: false,
    replaySeconds: 60,
    replaySpeed: '1',
    restoreLines: 0,
    discoverSsh: true,
    discoverWsl: true,
    bell: 'flash',
    notifyAfter: 10,
    clickLinks: 'ctrl',
    linkBrowser: 'system',
    showMarks: true,
    stickyCommand: true,
    scrollMinimap: true,
    allowClipboardWrite: true,
    profile: 'powershell',
    prompt: DEFAULT_PROMPT,
    customCommand: '',
    editor: 'code -g {file}:{line}:{col}',
    keymap: {},
    quakeKey: 'ctrl+`',
    quakeHeight: 0.62,
    quakeEnabled: true,
    restoreSession: true,
    closeToTray: false,
};

export const FONTS = ['Cascadia Code', 'Cascadia Mono', 'Consolas', 'Lucida Console', 'Courier New'];

export function storageGet(key: string): string | null {
    try {
        return localStorage.getItem(key);
    } catch {
        return null;
    }
}

export function storageSet(key: string, value: string) {
    try {
        localStorage.setItem(key, value);
    } catch {
        // A place that can't keep things just forgets them.
    }
}

/** A value that components can read and change, and hear about. */
export function createStore<T extends object>(initial: T, onChange?: (value: T) => void) {
    let state = initial;
    const listeners = new Set<() => void>();
    return {
        get: () => state,
        set(patch: Partial<T>) {
            state = { ...state, ...patch };
            onChange?.(state);
            for (const listener of [...listeners]) listener();
        },
        subscribe(listener: () => void) {
            listeners.add(listener);
            return () => listeners.delete(listener);
        },
    };
}

function load(): Settings {
    const text = storageGet('terminal.settings');
    if (text === null) return DEFAULTS;
    try {
        const stored = JSON.parse(text) as Partial<Settings> | null;
        return {
            ...DEFAULTS,
            ...stored,
            prompt: { ...DEFAULT_PROMPT, ...stored?.prompt },
            keymap: isKeymap(stored?.keymap) ? stored.keymap : DEFAULTS.keymap,
            triggers: isTriggerList(stored?.triggers) ? stored.triggers : DEFAULTS.triggers,
        };
    } catch {
        return DEFAULTS;
    }
}

export const settingsStore = createStore<Settings>(load(), (value) => storageSet('terminal.settings', JSON.stringify(value)));

export function useSettings(): Settings {
    return useSyncExternalStore(settingsStore.subscribe, settingsStore.get);
}

const windowKeys = (Object.keys(DEFAULTS) as (keyof Settings)[]).filter((key) => key !== 'prompt');
let lastSettings = settingsStore.get();
let windowSettings = lastSettings;

function getWindowSettings(): Settings {
    const next = settingsStore.get();
    if (next !== lastSettings) {
        lastSettings = next;
        if (windowKeys.some((key) => !Object.is(windowSettings[key], next[key]))) windowSettings = next;
    }
    return windowSettings;
}

/** Prompt edits are used by the editor and by the next shell launch. The window doesn't need to redraw for them. */
export function useWindowSettings(): Settings {
    return useSyncExternalStore(settingsStore.subscribe, getWindowSettings);
}

const getPromptSettings = () => settingsStore.get().prompt;

export function usePromptSettings(): PromptSettings {
    return useSyncExternalStore(settingsStore.subscribe, getPromptSettings);
}

export const changeSettings = (patch: Partial<Settings>) => settingsStore.set(patch);
