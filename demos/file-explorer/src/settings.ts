// Every setting the explorer has, what it starts as, and how the settings page shows it. The page is drawn from `SECTIONS`,
// so a setting added here shows up there with no other change.

import { storage } from 'rsui/desktop';
import type { Layout } from './layout';

export type Theme = 'system' | 'light' | 'dark';
export type View = 'details' | 'list' | 'grid';
export type SortKey = 'name' | 'size' | 'modified' | 'created' | 'type';

export const COMMANDS = {
    back: 'Go back',
    forward: 'Go forward',
    up: 'Go up a folder',
    refresh: 'Refresh',
    focusAddress: 'Edit the address',
    focusSearch: 'Search',
    newFolder: 'New folder',
    rename: 'Rename',
    delete: 'Delete',
    copy: 'Copy',
    cut: 'Cut',
    paste: 'Paste',
    selectAll: 'Select all',
    copyPath: 'Copy path',
    toggleHidden: 'Show or hide hidden files',
    toggleSidebar: 'Show or hide the sidebar',
    togglePreview: 'Show or hide the preview',
    viewDetails: 'Details view',
    viewList: 'List view',
    viewGrid: 'Grid view',
    openTerminal: 'Open a terminal here',
    properties: 'Properties',
    settings: 'Open settings',
    pin: 'Pin or unpin this folder',
    newTab: 'New tab',
    closeTab: 'Close tab',
    nextTab: 'Next tab',
    previousTab: 'Previous tab',
    split: 'Split to the right',
    splitDown: 'Split downward',
    closePane: 'Close the pane',
    otherPane: 'Go to the next pane',
    paneLeft: 'Go to the pane on the left',
    paneRight: 'Go to the pane on the right',
    paneUp: 'Go to the pane above',
    paneDown: 'Go to the pane below',
    evenPanes: 'Make the panes the same size',
    collapseSidebar: 'Collapse or expand the sidebar',
    customize: 'Customize',
} as const;
export type Command = keyof typeof COMMANDS;

export const DEFAULT_KEYS: Record<Command, string> = {
    back: 'Alt+ArrowLeft',
    forward: 'Alt+ArrowRight',
    up: 'Alt+ArrowUp',
    refresh: 'F5',
    focusAddress: 'Ctrl+L',
    focusSearch: 'Ctrl+F',
    newFolder: 'Ctrl+Shift+N',
    rename: 'F2',
    delete: 'Delete',
    copy: 'Ctrl+C',
    cut: 'Ctrl+X',
    paste: 'Ctrl+V',
    selectAll: 'Ctrl+A',
    copyPath: 'Ctrl+Shift+C',
    toggleHidden: 'Ctrl+H',
    toggleSidebar: 'Ctrl+B',
    togglePreview: 'Ctrl+P',
    viewDetails: 'Ctrl+1',
    viewList: 'Ctrl+2',
    viewGrid: 'Ctrl+3',
    openTerminal: 'Ctrl+`',
    properties: 'Alt+Enter',
    settings: 'Ctrl+,',
    pin: 'Ctrl+D',
    newTab: 'Ctrl+T',
    closeTab: 'Ctrl+W',
    nextTab: 'Ctrl+Tab',
    previousTab: 'Ctrl+Shift+Tab',
    split: 'Ctrl+\\',
    splitDown: 'Ctrl+Shift+\\',
    closePane: 'Ctrl+Shift+W',
    otherPane: 'Ctrl+J',
    paneLeft: 'Ctrl+Alt+ArrowLeft',
    paneRight: 'Ctrl+Alt+ArrowRight',
    paneUp: 'Ctrl+Alt+ArrowUp',
    paneDown: 'Ctrl+Alt+ArrowDown',
    evenPanes: 'Ctrl+Shift+=',
    collapseSidebar: 'Ctrl+Shift+B',
    customize: 'Ctrl+K',
};

export const DEFAULTS = {
    // Appearance
    theme: 'system' as Theme,
    accent: '#3b82f6',
    palette: 'zinc' as 'zinc' | 'slate' | 'stone' | 'neutral' | 'gray',
    fontFamily: 'Segoe UI Variable Text',
    fontSize: 13,
    density: 'cozy' as 'compact' | 'cozy' | 'comfortable',
    radius: 'medium' as 'none' | 'small' | 'medium' | 'large',
    iconSize: 18,
    iconStroke: 1.5,
    iconStyle: 'blode' as 'blode' | 'system',
    tintFolders: true,
    colorFileIcons: true,
    gridThumbnails: true,
    zebra: false,
    rowLines: false,
    selection: 'filled' as 'filled' | 'subtle' | 'outline',
    animations: true,
    backdrop: 'none' as 'none' | 'mica' | 'acrylic',
    /** How much of the window's frame the desktop shows through, with a material, from 0 to 100. */
    translucency: 60,
    /** How much of the accent the frame around the panels takes, from 0 to 100, as Arc tints its window. */
    frameTint: 10,
    /** How far apart the panels are, and from the frame's edges. */
    panelGap: 8,
    panelShadow: true,
    // Layout
    tabsAt: 'top' as 'top' | 'side',
    restoreTabs: true,
    view: 'details' as View,
    gridSize: 'medium' as 'small' | 'medium' | 'large' | 'huge',
    showSidebar: true,
    sidebarWidth: 220,
    /** The sidebar as a narrow rail of icons, as it becomes when it's dragged smaller than it can be. */
    sidebarCollapsed: false,
    showQuickAccess: true,
    showPinned: true,
    showDrives: true,
    showDriveUsage: true,
    showToolbar: true,
    toolbarLabels: false,
    addressMode: 'breadcrumbs' as 'breadcrumbs' | 'path',
    showStatusBar: true,
    showPreview: false,
    previewWidth: 300,
    showColumnHeaders: true,
    colSize: true,
    colType: true,
    colModified: true,
    colCreated: false,
    colAttributes: false,
    // Files and folders
    showHidden: false,
    showSystem: false,
    dotfilesHidden: true,
    dimHidden: true,
    showExtensions: true,
    foldersFirst: true,
    sortKey: 'name' as SortKey,
    sortDescending: false,
    naturalSort: true,
    caseSensitiveSort: false,
    rememberViewPerFolder: false,
    dateFormat: 'relative' as 'relative' | 'short' | 'long' | 'iso',
    clock24: true,
    sizeUnits: 'binary' as 'binary' | 'decimal' | 'bytes',
    // Behavior
    openWith: 'double' as 'double' | 'single',
    startAt: 'home' as 'home' | 'last' | 'custom' | 'computer',
    startPath: '',
    confirmDelete: true,
    useRecycleBin: true,
    liveRefresh: true,
    typeahead: true,
    backspaceUp: true,
    wrapNavigation: false,
    renameSelectsBase: true,
    selectFirstOnOpen: true,
    doubleClickEmptyUp: false,
    terminalCommand: 'wt -d "{path}"',
    // Search
    searchRecursive: false,
    searchDepth: 6,
    searchMaxResults: 2000,
    searchCase: false,
    searchMatch: 'contains' as 'contains' | 'prefix' | 'fuzzy' | 'glob' | 'regex',
    searchHidden: false,
    searchDelay: 120,
    // Preview
    previewImages: true,
    previewText: true,
    previewMaxKB: 64,
    previewMono: true,
    previewWrap: false,
    previewMeta: true,
    // Performance
    overscan: 8,
    thumbnailCache: 400,
    systemIconCache: true,
    showPerf: false,
    // Keyboard
    keys: DEFAULT_KEYS as Record<Command, string>,
};

export type Settings = typeof DEFAULTS;
export type SettingKey = Exclude<keyof Settings, 'keys'>;

const store = storage.open('file-explorer');

export function loadSettings(): Settings {
    const saved = (store.get('settings') ?? {}) as Partial<Settings>;
    return { ...DEFAULTS, ...saved, keys: { ...DEFAULT_KEYS, ...(saved.keys ?? {}) } };
}

export function saveSettings(settings: Settings): void {
    // Only what differs from the defaults is kept, so a new default reaches people who never changed that setting.
    const changed: Record<string, unknown> = {};
    for (const key of Object.keys(settings) as (keyof Settings)[]) {
        if (key === 'keys') {
            const keys: Record<string, string> = {};
            for (const command of Object.keys(settings.keys) as Command[]) {
                if (settings.keys[command] !== DEFAULT_KEYS[command]) {
                    keys[command] = settings.keys[command];
                }
            }
            if (Object.keys(keys).length > 0) {
                changed.keys = keys;
            }
        } else if (settings[key] !== DEFAULTS[key]) {
            changed[key] = settings[key];
        }
    }
    store.set('settings', changed);
}

/** What the app remembers between runs that isn't a setting. */
export interface Session {
    lastFolder?: string;
    pinned: string[];
    views: Record<string, { view: View; sortKey: SortKey; sortDescending: boolean }>;
    tabs?: TabState[];
    activeTab?: number;
    /** The order the sidebar's sections, places and drives are dragged into. */
    sidebarOrder?: { sections?: string[]; places?: string[]; drives?: string[] };
}

/** One side of a tab: a folder and how it got there. */
export interface PaneState {
    id: string;
    stack: string[];
    at: number;
}

/**
 * A tab: its panes, laid out by `layout` (see src/layout.ts), and `focus`, the id of the one that has the keyboard. Tabs saved before
 * there were layouts have only `split`, a share of the width for the first of two side by side, and `focus` as an index.
 */
export interface TabState {
    id: string;
    panes: PaneState[];
    layout: Layout;
    focus: string;
}

export function loadSession(): Session {
    const saved = (store.get('session') ?? {}) as Partial<Session>;
    return { pinned: saved.pinned ?? [], views: saved.views ?? {}, lastFolder: saved.lastFolder, tabs: saved.tabs, activeTab: saved.activeTab, sidebarOrder: saved.sidebarOrder };
}

export function saveSession(session: Session): void {
    store.set('session', session);
}

// How the settings page shows each setting.

export type Field =
    | { key: SettingKey; label: string; hint?: string; kind: 'toggle' }
    | { key: SettingKey; label: string; hint?: string; kind: 'choice'; options: [string | number, string][] }
    | { key: SettingKey; label: string; hint?: string; kind: 'number'; min: number; max: number; step: number; unit?: string }
    | { key: SettingKey; label: string; hint?: string; kind: 'text'; placeholder?: string; browse?: boolean }
    | { key: SettingKey; label: string; hint?: string; kind: 'color' }
    | { key: SettingKey; label: string; hint?: string; kind: 'font' };

export interface Section {
    id: string;
    title: string;
    blurb: string;
    fields: Field[];
}

export const ACCENTS: [string, string][] = [
    ['#3b82f6', 'Blue'], ['#6366f1', 'Indigo'], ['#8b5cf6', 'Purple'], ['#ec4899', 'Pink'], ['#ef4444', 'Red'], ['#f97316', 'Orange'],
    ['#eab308', 'Yellow'], ['#22c55e', 'Green'], ['#14b8a6', 'Teal'], ['#06b6d4', 'Cyan'], ['#64748b', 'Slate'],
];
export const FONTS = ['Segoe UI Variable Text', 'Segoe UI', 'Bahnschrift', 'Calibri', 'Cascadia Mono', 'Consolas'];

export const SECTIONS: Section[] = [
    {
        id: 'appearance',
        title: 'Appearance',
        blurb: 'Colors, type and how things are drawn.',
        fields: [
            { key: 'theme', label: 'Theme', kind: 'choice', options: [['system', 'System'], ['light', 'Light'], ['dark', 'Dark']] },
            { key: 'accent', label: 'Accent color', hint: 'Used for the selection, focus and folder icons.', kind: 'color' },
            { key: 'palette', label: 'Neutral palette', kind: 'choice', options: [['zinc', 'Zinc'], ['slate', 'Slate'], ['stone', 'Stone'], ['neutral', 'Neutral'], ['gray', 'Gray']] },
            { key: 'fontFamily', label: 'Font', kind: 'font' },
            { key: 'fontSize', label: 'Text size', kind: 'number', min: 11, max: 18, step: 1, unit: 'px' },
            { key: 'density', label: 'Density', hint: 'How tall rows and controls are.', kind: 'choice', options: [['compact', 'Compact'], ['cozy', 'Cozy'], ['comfortable', 'Comfortable']] },
            { key: 'radius', label: 'Corners', kind: 'choice', options: [['none', 'Square'], ['small', 'Small'], ['medium', 'Medium'], ['large', 'Round']] },
            { key: 'selection', label: 'Selection style', kind: 'choice', options: [['filled', 'Filled'], ['subtle', 'Subtle'], ['outline', 'Outline']] },
            { key: 'zebra', label: 'Striped rows', kind: 'toggle' },
            { key: 'rowLines', label: 'Lines between rows', kind: 'toggle' },
            { key: 'animations', label: 'Animations', hint: 'Hover fades and panel transitions.', kind: 'toggle' },
            { key: 'backdrop', label: 'Window material', hint: 'Windows 11 only. The frame around the panels shows the desktop through it.', kind: 'choice', options: [['none', 'Solid'], ['mica', 'Mica'], ['acrylic', 'Acrylic']] },
            { key: 'translucency', label: 'See-through frame', hint: 'How much of the material shows, when there is one.', kind: 'number', min: 0, max: 100, step: 10, unit: '%' },
            { key: 'frameTint', label: 'Frame tint', hint: 'How much of the accent color the frame takes.', kind: 'number', min: 0, max: 60, step: 5, unit: '%' },
            { key: 'panelGap', label: 'Space around panels', kind: 'number', min: 0, max: 16, step: 2, unit: 'px' },
            { key: 'panelShadow', label: 'Panel shadows', kind: 'toggle' },
        ],
    },
    {
        id: 'icons',
        title: 'Icons',
        blurb: 'Blode icons, or the ones Windows draws.',
        fields: [
            { key: 'iconStyle', label: 'Icon set', hint: 'System icons are read from Windows and cached.', kind: 'choice', options: [['blode', 'Blode'], ['system', 'Windows']] },
            { key: 'iconSize', label: 'Icon size', kind: 'number', min: 14, max: 28, step: 2, unit: 'px' },
            { key: 'iconStroke', label: 'Line weight', kind: 'choice', options: [[1.25, 'Thin'], [1.5, 'Regular'], [1.75, 'Medium'], [2, 'Bold']] },
            { key: 'tintFolders', label: 'Folders in the accent color', kind: 'toggle' },
            { key: 'colorFileIcons', label: 'Color files by type', kind: 'toggle' },
            { key: 'gridThumbnails', label: 'Pictures in grid view', hint: 'Shows photos, videos and documents as Explorer does.', kind: 'toggle' },
        ],
    },
    {
        id: 'layout',
        title: 'Layout',
        blurb: 'Which panels show and how big they are.',
        fields: [
            { key: 'view', label: 'Default view', kind: 'choice', options: [['details', 'Details'], ['list', 'List'], ['grid', 'Grid']] },
            { key: 'gridSize', label: 'Grid tiles', kind: 'choice', options: [['small', 'Small'], ['medium', 'Medium'], ['large', 'Large'], ['huge', 'Huge']] },
            { key: 'tabsAt', label: 'Tabs', hint: 'Along the top, or down the sidebar as Arc has them.', kind: 'choice', options: [['top', 'Top'], ['side', 'Sidebar']] },
            { key: 'restoreTabs', label: 'Reopen tabs', hint: 'The tabs that were open last time, and their split views.', kind: 'toggle' },
            { key: 'showToolbar', label: 'Toolbar', kind: 'toggle' },
            { key: 'addressMode', label: 'Address bar', kind: 'choice', options: [['breadcrumbs', 'Breadcrumbs'], ['path', 'Plain path']] },
            { key: 'showSidebar', label: 'Sidebar', kind: 'toggle' },
            { key: 'sidebarCollapsed', label: 'Collapsed sidebar', hint: 'Only icons. Dragging the sidebar narrower than it goes does this too.', kind: 'toggle' },
            { key: 'sidebarWidth', label: 'Sidebar width', kind: 'number', min: 170, max: 420, step: 10, unit: 'px' },
            { key: 'showQuickAccess', label: 'Quick access in the sidebar', kind: 'toggle' },
            { key: 'showPinned', label: 'Pinned folders in the sidebar', kind: 'toggle' },
            { key: 'showDrives', label: 'Drives in the sidebar', kind: 'toggle' },
            { key: 'showDriveUsage', label: 'Drive usage bars', kind: 'toggle' },
            { key: 'showPreview', label: 'Preview pane', kind: 'toggle' },
            { key: 'previewWidth', label: 'Preview width', kind: 'number', min: 220, max: 560, step: 20, unit: 'px' },
            { key: 'showStatusBar', label: 'Status bar', kind: 'toggle' },
        ],
    },
    {
        id: 'columns',
        title: 'Columns',
        blurb: 'What the details view shows for each item.',
        fields: [
            { key: 'showColumnHeaders', label: 'Column headers', kind: 'toggle' },
            { key: 'colSize', label: 'Size', kind: 'toggle' },
            { key: 'colType', label: 'Type', kind: 'toggle' },
            { key: 'colModified', label: 'Date modified', kind: 'toggle' },
            { key: 'colCreated', label: 'Date created', kind: 'toggle' },
            { key: 'colAttributes', label: 'Attributes', kind: 'toggle' },
        ],
    },
    {
        id: 'files',
        title: 'Files & folders',
        blurb: 'What shows, in what order, and how it reads.',
        fields: [
            { key: 'showHidden', label: 'Show hidden items', kind: 'toggle' },
            { key: 'showSystem', label: 'Show protected system files', kind: 'toggle' },
            { key: 'dotfilesHidden', label: 'Treat names starting with a dot as hidden', hint: 'As macOS and Linux do, for .git, .cargo and the like.', kind: 'toggle' },
            { key: 'dimHidden', label: 'Fade hidden items', kind: 'toggle' },
            { key: 'showExtensions', label: 'Show file extensions', kind: 'toggle' },
            { key: 'foldersFirst', label: 'Folders before files', kind: 'toggle' },
            { key: 'sortKey', label: 'Sort by', kind: 'choice', options: [['name', 'Name'], ['size', 'Size'], ['modified', 'Modified'], ['created', 'Created'], ['type', 'Type']] },
            { key: 'sortDescending', label: 'Sort descending', kind: 'toggle' },
            { key: 'naturalSort', label: 'Natural number order', hint: 'file2 comes before file10.', kind: 'toggle' },
            { key: 'caseSensitiveSort', label: 'Case-sensitive sorting', kind: 'toggle' },
            { key: 'rememberViewPerFolder', label: 'Remember view and sort per folder', kind: 'toggle' },
            { key: 'dateFormat', label: 'Dates', kind: 'choice', options: [['relative', '2 hours ago'], ['short', '17/11/2026'], ['long', '17 Nov 2026, 14:05'], ['iso', '2026-11-17 14:05']] },
            { key: 'clock24', label: '24-hour clock', kind: 'toggle' },
            { key: 'sizeUnits', label: 'Sizes', kind: 'choice', options: [['binary', '1.5 MiB'], ['decimal', '1.6 MB'], ['bytes', '1,572,864 B']] },
        ],
    },
    {
        id: 'behavior',
        title: 'Behavior',
        blurb: 'Opening, deleting and moving around.',
        fields: [
            { key: 'openWith', label: 'Open items with', kind: 'choice', options: [['double', 'Double click'], ['single', 'Single click']] },
            { key: 'startAt', label: 'Start in', kind: 'choice', options: [['home', 'Home folder'], ['computer', 'This PC'], ['last', 'Last folder'], ['custom', 'A folder I choose']] },
            { key: 'startPath', label: 'Start folder', hint: 'Used when "Start in" is set to a folder you choose.', kind: 'text', placeholder: process.platform === 'win32' ? 'C:\\Users\\you\\Projects' : '/home/you/Projects', browse: true },
            { key: 'confirmDelete', label: 'Ask before deleting', kind: 'toggle' },
            { key: 'useRecycleBin', label: 'Delete to the Recycle Bin', hint: 'Off deletes for good.', kind: 'toggle' },
            { key: 'liveRefresh', label: 'Update as files change', hint: 'Watches the open folder.', kind: 'toggle' },
            { key: 'typeahead', label: 'Type to jump to a name', kind: 'toggle' },
            { key: 'backspaceUp', label: 'Backspace goes up a folder', kind: 'toggle' },
            { key: 'wrapNavigation', label: 'Arrow keys wrap around', kind: 'toggle' },
            { key: 'renameSelectsBase', label: 'Rename selects the name without its extension', kind: 'toggle' },
            { key: 'selectFirstOnOpen', label: 'Select the first item when a folder opens', kind: 'toggle' },
            { key: 'doubleClickEmptyUp', label: 'Double click empty space to go up', kind: 'toggle' },
            { key: 'terminalCommand', label: 'Terminal command', hint: '{path} is the open folder.', kind: 'text', placeholder: 'wt -d "{path}"' },
        ],
    },
    {
        id: 'search',
        title: 'Search',
        blurb: 'How the search box matches.',
        fields: [
            { key: 'searchRecursive', label: 'Search subfolders', kind: 'toggle' },
            { key: 'searchDepth', label: 'How deep to search', kind: 'number', min: 1, max: 20, step: 1, unit: 'levels' },
            { key: 'searchMaxResults', label: 'Most results', kind: 'number', min: 100, max: 20000, step: 100 },
            { key: 'searchMatch', label: 'Match', kind: 'choice', options: [['contains', 'Contains'], ['prefix', 'Starts with'], ['fuzzy', 'Fuzzy'], ['glob', 'Wildcards'], ['regex', 'Regex']] },
            { key: 'searchCase', label: 'Match case', kind: 'toggle' },
            { key: 'searchHidden', label: 'Include hidden items', kind: 'toggle' },
            { key: 'searchDelay', label: 'Wait before searching subfolders', kind: 'number', min: 0, max: 600, step: 20, unit: 'ms' },
        ],
    },
    {
        id: 'preview',
        title: 'Preview',
        blurb: 'What the preview pane shows.',
        fields: [
            { key: 'previewImages', label: 'Show pictures', kind: 'toggle' },
            { key: 'previewText', label: 'Show text files', kind: 'toggle' },
            { key: 'previewMaxKB', label: 'Most text to read', kind: 'number', min: 8, max: 1024, step: 8, unit: 'KB' },
            { key: 'previewMono', label: 'Monospace text', kind: 'toggle' },
            { key: 'previewWrap', label: 'Wrap long lines', kind: 'toggle' },
            { key: 'previewMeta', label: 'Show details', kind: 'toggle' },
        ],
    },
    {
        id: 'performance',
        title: 'Performance',
        blurb: 'Trade memory for speed, and see what things cost.',
        fields: [
            { key: 'overscan', label: 'Rows drawn past the edge', hint: 'More is smoother when flinging, fewer is lighter.', kind: 'number', min: 0, max: 40, step: 2, unit: 'rows' },
            { key: 'thumbnailCache', label: 'Thumbnails kept', kind: 'number', min: 50, max: 4000, step: 50 },
            { key: 'systemIconCache', label: 'Share Windows icons by file type', hint: 'Off reads each file\'s own icon.', kind: 'toggle' },
            { key: 'showPerf', label: 'Show timings in the status bar', kind: 'toggle' },
        ],
    },
];
