/** Missing commands inherit their defaults. An empty list explicitly unbinds a command. */
export type Keymap = Record<string, string[]>;

const MODIFIERS = ['ctrl', 'alt', 'shift', 'meta'];
const FUNCTION_KEY = /^f(?:[1-9]|1[0-9]|2[0-4])$/;
const NAMED_KEY = /^(?:space|plus|enter|tab|escape|backspace|delete|insert|home|end|pageup|pagedown|left|right|up|down)$/;
const KEY_NAMES = new Map([['arrowleft', 'left'], ['arrowright', 'right'], ['arrowup', 'up'], ['arrowdown', 'down'], [' ', 'space'], ['+', 'plus'], ['esc', 'escape']]);

export function chord(e: { key: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey?: boolean }): string {
    const key = e.key.toLowerCase();
    return (e.ctrlKey ? 'ctrl+' : '') + (e.altKey ? 'alt+' : '') + (e.shiftKey ? 'shift+' : '') + (e.metaKey ? 'meta+' : '') + (KEY_NAMES.get(key) ?? key);
}

export function isShortcut(value: unknown): value is string {
    if (typeof value !== 'string' || value !== value.toLowerCase()) return false;
    const parts = value.split('+');
    const key = parts.pop()!;
    const canonical = MODIFIERS.filter((modifier) => parts.includes(modifier));
    if (parts.join('+') !== canonical.join('+')) return false;
    if (FUNCTION_KEY.test(key)) return true;
    if (parts.some((part) => part !== 'shift')) {
        const code = key.codePointAt(0) ?? 0;
        const printable = [...key].length === 1 && key.trim() === key && code >= 0x20 && (code < 0x7f || code > 0x9f);
        return NAMED_KEY.test(key) || printable;
    }
    return parts.length === 1 && /^(?:pageup|pagedown|home|end|insert|delete|left|right|up|down)$/.test(key);
}

/** The subset the native shortcut APIs can register by name. */
export function isNativeShortcut(value: unknown): value is string {
    if (!isShortcut(value)) return false;
    const key = value.split('+').at(-1)!;
    return FUNCTION_KEY.test(key) || NAMED_KEY.test(key) || /^[a-z0-9]$/.test(key) || (key.length === 1 && ";=,./`[]\\'-".includes(key));
}

export function isKeymap(value: unknown): value is Keymap {
    return typeof value === 'object' && value !== null && !Array.isArray(value)
        && Object.entries(value).every(([id, keys]) => /^[a-z][\w:-]*$/i.test(id) && !Object.hasOwn(Object.prototype, id)
            && Array.isArray(keys) && keys.every(isShortcut) && new Set(keys).size === keys.length);
}
