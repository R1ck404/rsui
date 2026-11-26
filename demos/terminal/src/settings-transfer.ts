import { BACKGROUNDS } from './backgrounds/scenes';
import { isKeymap, isNativeShortcut } from './keymap';
import { SEGMENT_IDS } from './prompt';
import type { PromptSettings } from './prompt';
import type { Settings } from './settings';
import { DEFAULT_DESIGN, EFFECTS, CHARSETS } from './studio/effects';
import { SCHEMES, SCHEME_COLORS } from './themes';
import { isTriggerList } from './term/triggers';

type Rule = (value: unknown) => boolean;
type SettingPath = Exclude<keyof Settings, 'prompt'> | `prompt.${keyof PromptSettings}`;
type PortableSettings = Partial<Omit<Settings, 'prompt'>> & { prompt?: Partial<PromptSettings> };
type Field = readonly [path: SettingPath, label: string, accepts: Rule];

const record = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const bool: Rule = (value) => typeof value === 'boolean';
const string: Rule = (value) => typeof value === 'string';
const range = (min: number, max: number): Rule => (value) => typeof value === 'number' && value >= min - 1e-9 && value <= max + 1e-9;
const oneOf = (...options: unknown[]): Rule => (value) => options.includes(value);
const rgb: Rule = (value) => typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 0xffffff;
const entries = (key: Rule, item: Rule): Rule => (value) => record(value) && Object.entries(value).every(([name, val]) => key(name) && item(val));
const shape = (rules: Record<string, Rule>, required = false): Rule => (value) => record(value)
    && Object.entries(value).every(([key, item]) => Object.hasOwn(rules, key) && rules[key](item))
    && (!required || Object.keys(rules).every((key) => Object.hasOwn(value, key)));

const background = oneOf('custom', ...BACKGROUNDS.map(({ id }) => id));
const scheme = oneOf(...SCHEMES.map(({ id }) => id));
const colors = entries(scheme, shape({
    ...Object.fromEntries(Object.keys(SCHEME_COLORS).map((key) => [key, rgb])),
    ansi: entries((key) => /^(?:\d|1[0-5])$/.test(String(key)), rgb),
}));
const promptSegments = oneOf(...SEGMENT_IDS);
const promptOrder: Rule = (value) => Array.isArray(value) && value.every(promptSegments) && new Set(value).size === value.length;
const promptColors = entries(promptSegments, shape({ fg: rgb, bg: rgb }));

const design = shape({
    effect: oneOf(...EFFECTS.map(({ id }) => id)),
    cell: range(1, 32), brightness: range(-0.5, 0.5), contrast: range(0.3, 3), gamma: range(0.3, 3), saturation: range(0, 2), invert: bool,
    threshold: range(0, 1), charset: oneOf(...Object.keys(CHARSETS), 'custom'), customChars: string, diffuse: bool, round: bool,
    gap: range(0, 0.8), levels: range(2, 8), color: oneOf('source', 'mono', 'duotone', 'scheme'), mono: rgb, low: rgb, high: rgb,
    opacity: range(0, 1), fade: oneOf('none', 'left', 'right', 'top', 'bottom', 'radial', 'corner'), fit: oneOf('cover', 'contain'), speed: range(0.25, 3),
} satisfies Record<keyof typeof DEFAULT_DESIGN, Rule>, true);
const studio: Rule = (value) => value === null || (record(value) && string(value.file) && design(value.design)
    && (value.source === undefined || value.source === 'file' || background(value.source))
    && (value.scene === undefined || shape({ color: rgb, color2: rgb, speed: range(0.1, 3), density: range(0.3, 2.5) }, true)(value.scene)));

export const SETTINGS_GROUPS: readonly { label: string; fields: readonly Field[] }[] = [
    { label: 'General', fields: [
        ['layout', 'Window layout', oneOf('normal', 'tabs', 'sidebar', 'strip', 'minimal')],
        ['uiFont', 'Interface font', oneOf('system', 'mono')],
        ['showStatusBar', 'Show status bar', bool],
        ['sidebarWidth', 'Sidebar width', range(180, 360)],
        ['animations', 'Animations', bool],
        ['restoreSession', 'Reopen where I left off', bool],
        ['restoreLines', 'Restore terminal output', oneOf(0, 250, 1000)],
        ['closeToTray', 'Close to tray', bool],
    ] },
    { label: 'Appearance', fields: [
        ['scheme', 'Color scheme', scheme],
        ['schemeOverrides', 'Custom scheme colors', colors],
        ['material', 'Window material', oneOf('mica', 'acrylic', 'tabbed', 'none')],
        ['frameOpacity', 'Frame opacity', range(0, 1)],
        ['columnOpacity', 'Panel opacity', range(0.3, 1)],
        ['radius', 'Corner radius', range(0, 24)],
        ['textPadding', 'Text padding', range(0, 40)],
        ['padding', 'Window padding', range(0, 100)],
    ] },
    { label: 'Background', fields: [
        ['background', 'Background type', background],
        ['backgroundColor', 'Background colors', oneOf('scheme', 'blue', 'purple', 'green', 'warm', 'mono')],
        ['backgroundIntensity', 'Intensity', range(0.1, 1)],
        ['backgroundSpeed', 'Speed', range(0.1, 3)],
        ['backgroundDensity', 'Density', range(0.3, 2.5)],
        ['studio', 'Studio design', studio],
    ] },
    { label: 'Text & cursor', fields: [
        ['fontFamily', 'Font family', string],
        ['fontSize', 'Font size', range(7, 40)],
        ['lineHeight', 'Line height', range(1, 1.6)],
        ['ligatures', 'Ligatures', bool],
        ['cursorShape', 'Cursor shape', oneOf('program', 'block', 'bar', 'underline')],
        ['cursorBlink', 'Cursor blink', bool],
        ['smoothCursor', 'Gliding cursor', bool],
        ['cursorTrail', 'Cursor trail', bool],
    ] },
    { label: 'Prompt', fields: [
        ['prompt.style', 'Style', oneOf('shell', 'minimal', 'powerline')],
        ['prompt.order', 'Enabled segments and order', promptOrder],
        ['prompt.segmentOrder', 'Segment positions in settings', promptOrder],
        ['prompt.colors', 'Segment colors', promptColors],
        ['prompt.folder', 'Folder display', oneOf('name', 'home', 'full')],
        ['prompt.edge', 'Segment edges', oneOf('arrow', 'round', 'flat')],
        ['prompt.newLine', 'Type on a new line', bool],
    ] },
    { label: 'Behavior', fields: [
        ['scrollback', 'Scrollback', range(1, 100000)],
        ['copyOnSelect', 'Copy on select', bool],
        ['warnOnMultilinePaste', 'Ask before pasting several lines', bool],
        ['pasteAtPromptOnly', 'Limit paste warnings to shell prompts', bool],
        ['graphics', 'Inline graphics', oneOf('all', 'kitty', 'sixel', 'iterm', 'off')],
        ['kittyKeyboard', 'Kitty keyboard protocol', bool],
        ['kittyKeysFirst', 'Prefer terminal application shortcuts', bool],
        ['triggers', 'Output triggers', isTriggerList],
        ['attentionOn', 'Mark panes that need attention', oneOf('notifications', 'notificationsAndBell', 'off')],
        ['paneStatus', 'Pane status bar', bool],
        ['replaySeconds', 'Instant replay history', oneOf(0, 30, 60, 300)],
        ['replaySpeed', 'Default playback speed', oneOf('1', '2', '4', '8')],
        ['clickLinks', 'Link click gesture', oneOf('click', 'ctrl', 'off')],
        ['linkBrowser', 'Open web links in', oneOf('system', 'builtIn')],
        ['allowClipboardWrite', 'Let programs copy', bool],
        ['showMarks', 'Mark each command', bool],
        ['stickyCommand', 'Keep the command at the top', bool],
        ['scrollMinimap', 'Commands on the scrollbar', bool],
        ['notifyAfter', 'Tell me when a command finishes', range(0, 60)],
        ['bell', 'Bell', oneOf('off', 'flash', 'sound', 'both')],
    ] },
    { label: 'Strip', fields: [
        ['columnWidth', 'New column width', oneOf('full', 'wide', 'twothirds', 'half', 'third')],
        ['peek', 'Neighbors showing', range(0, 140)],
        ['dimOthers', 'Dim the others', bool],
        ['quakeEnabled', 'Enable global dropdown shortcut', bool],
        ['quakeHeight', 'Dropdown height', range(0.3, 1)],
    ] },
    { label: 'Shells', fields: [
        ['profile', 'Default shell', string],
        ['discoverSsh', 'Discover SSH aliases', bool],
        ['discoverWsl', 'Discover WSL distributions', bool],
        ['customCommand', 'Custom shell command', string],
        ['editor', 'Open files with', string],
    ] },
    { label: 'Keyboard', fields: [
        ['keymap', 'Application shortcuts', isKeymap],
        ['quakeKey', 'Global dropdown shortcut', isNativeShortcut],
    ] },
];

const fields = SETTINGS_GROUPS.flatMap((group) => group.fields);
export const settingPaths = fields.map(([path]) => path);
const format = 'rsui-terminal-settings';
const version = 1;

function promptKey(path: SettingPath) {
    return path.startsWith('prompt.') ? path.slice(7) as keyof PromptSettings : undefined;
}

export function readField(source: Record<string, unknown>, path: SettingPath): unknown {
    const key = promptKey(path);
    if (!key) return Object.hasOwn(source, path) ? source[path] : undefined;
    const prompt = source.prompt;
    return record(prompt) && Object.hasOwn(prompt, key) ? prompt[key] : undefined;
}

function selectSettings(source: PortableSettings, selection: ReadonlySet<SettingPath>): PortableSettings {
    const flat: [string, unknown][] = [];
    const prompt: [string, unknown][] = [];
    for (const path of settingPaths) {
        if (!selection.has(path)) continue;
        const value = readField(source, path);
        if (value === undefined) continue;
        const key = promptKey(path);
        if (key) prompt.push([key, value]);
        else flat.push([path, value]);
    }
    if (prompt.length) flat.push(['prompt', Object.fromEntries(prompt)]);
    return Object.fromEntries(flat);
}

export function exportSettings(settings: Settings, selection: ReadonlySet<SettingPath>): string {
    return JSON.stringify({ format, version, settings: selectSettings(settings, selection) }, null, 2) + '\n';
}

export function parseSettings(text: string): { settings: PortableSettings; paths: SettingPath[] } {
    let data: unknown;
    try {
        data = JSON.parse(text.replace(/^\uFEFF/, ''));
    } catch {
        throw new Error('The file contains invalid JSON. Choose a settings export.');
    }
    if (!record(data) || data.format !== format || !record(data.settings)) throw new Error('Choose a Terminal settings export.');
    if (data.version !== version) throw new Error('This version of the settings file is unsupported.');
    const source = data.settings;
    if (Object.hasOwn(source, 'prompt') && !record(source.prompt)) throw new Error('Prompt settings must be an object.');
    const paths = fields.filter(([path, label, accepts]) => {
        const value = readField(source, path);
        if (value === undefined) return false;
        if (!accepts(value)) throw new Error(`The file contains an invalid value for ${label}.`);
        return true;
    }).map(([path]) => path);
    if (paths.length === 0) throw new Error('The file has no recognized settings.');
    return { settings: selectSettings(source, new Set(paths)), paths };
}

export function importSettings(current: Settings, source: PortableSettings, selection: ReadonlySet<SettingPath>): Partial<Settings> {
    const { prompt, ...patch } = selectSettings(source, selection);
    return prompt ? { ...patch, prompt: { ...current.prompt, ...prompt } } : patch;
}
