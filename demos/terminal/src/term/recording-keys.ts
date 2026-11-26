import type { KeyInfo } from '../vt/keys';

export type RecordedKeystroke = { t: number; keys: string[] };
export const KEYSTROKE_CATEGORIES = ['shortcuts', 'special', 'typed'] as const;
export type KeystrokeCategory = typeof KEYSTROKE_CATEGORIES[number];
export const KEYSTROKE_POSITIONS = ['bottom-center', 'bottom-left', 'bottom-right', 'top-center'] as const;
export type KeystrokePosition = typeof KEYSTROKE_POSITIONS[number];
export const KEYSTROKE_DURATIONS = ['500', '1000', '2000'] as const;
export type KeystrokeDuration = typeof KEYSTROKE_DURATIONS[number];
export const KEYSTROKE_LAYOUTS = ['row', 'latest', 'stack'] as const;
export type KeystrokeLayout = typeof KEYSTROKE_LAYOUTS[number];
export const KEYSTROKE_LIMITS = ['3', '6', '12'] as const;
export type KeystrokeLimit = typeof KEYSTROKE_LIMITS[number];
export const KEYSTROKE_ANIMATIONS = ['none', 'fade', 'pop'] as const;
export type KeystrokeAnimation = typeof KEYSTROKE_ANIMATIONS[number];

export function keystrokeOption<T extends string>(options: readonly [T, ...T[]], value?: T): T {
    return value !== undefined && options.includes(value) ? value : options[0];
}

const label: Readonly<Record<string, string>> = {
    Enter: 'Enter', Tab: 'Tab', Backspace: 'Backspace', Escape: 'Esc', Delete: 'Delete', Insert: 'Insert',
    Home: 'Home', End: 'End', PageUp: 'Page Up', PageDown: 'Page Down',
    ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→',
};
const specialKeys = new Set(Object.values(label));
const functionKey = /^F(?:[1-9]|[12]\d|3[0-5])$/;

export function recordingKey(event: KeyInfo): string[] | null {
    const { key } = event;
    const altGraph = event.getModifierState?.('AltGraph');
    const modified = !altGraph && (event.ctrlKey || event.altKey || event.metaKey);
    const printable = Array.from(key).length === 1 && !/[\x00-\x1f\x7f-\x9f]/.test(key);
    let name = Object.hasOwn(label, key) ? label[key] : undefined;
    if (printable) {
        name = key === ' ' ? 'Space' : modified ? key.toUpperCase() : key;
    } else if (functionKey.test(key)) {
        name = key;
    }
    if (!name) return null;
    const keys = [];
    if (!altGraph && event.ctrlKey) keys.push('Ctrl');
    if (!altGraph && event.altKey) keys.push('Alt');
    if (event.shiftKey && (!printable || modified)) keys.push('Shift');
    if (event.metaKey) keys.push('Win');
    return [...keys, name];
}

export function keystrokeCategory({ keys }: RecordedKeystroke): KeystrokeCategory {
    if (keys.length > 1) return 'shortcuts';
    return specialKeys.has(keys[0]) || functionKey.test(keys[0]) ? 'special' : 'typed';
}

export function filterKeystrokes(events: readonly RecordedKeystroke[], categories: readonly KeystrokeCategory[]) {
    return KEYSTROKE_CATEGORIES.every((category) => categories.includes(category))
        ? events : events.filter((event) => categories.includes(keystrokeCategory(event)));
}

/** The first key strictly after the playhead. */
export function nextKeyIndex(events: readonly RecordedKeystroke[], time: number) {
    let low = 0, high = events.length;
    while (low < high) {
        const middle = (low + high) >>> 1;
        if (events[middle].t <= time) low = middle + 1;
        else high = middle;
    }
    return low;
}

export function visibleKeystrokes(events: readonly RecordedKeystroke[], time: number, from = 0, hold = 1000, limit = 3) {
    const end = nextKeyIndex(events, time);
    return events.slice(Math.max(0, end - limit), end).filter(({ t }) => t >= from && t > time - hold);
}

/** Sample the overlay at the recording's time so playback and export agree. */
export function keystrokeMotion(animation: KeystrokeAnimation, age: number, hold: number) {
    if (animation === 'none') return undefined;
    const progress = Math.min(1, Math.max(0, age / 125));
    const entrance = 1 - (1 - progress) ** 3;
    const exit = Math.min(1, Math.max(0, (hold - age) / 125));
    return {
        opacity: Math.min(0.35 + 0.65 * entrance, exit),
        transform: animation === 'pop' ? `scale(${(0.94 + 0.06 * entrance).toFixed(3)})` : undefined,
    };
}
