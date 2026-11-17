// Shortcuts as text, like `Ctrl+Shift+N`, from a key press.

const NAMES: Record<string, string> = { ' ': 'Space', Esc: 'Escape', Del: 'Delete', Left: 'ArrowLeft', Right: 'ArrowRight', Up: 'ArrowUp', Down: 'ArrowDown' };
/** What a symbol is typed with shift on a US keyboard, which is the key a shortcut means. */
const UNSHIFTED: Record<string, string> = {
    '!': '1', '@': '2', '#': '3', '$': '4', '%': '5', '^': '6', '&': '7', '*': '8', '(': '9', ')': '0',
    '_': '-', '+': '=', '|': '\\', '{': '[', '}': ']', ':': ';', '"': "'", '<': ',', '>': '.', '?': '/', '~': '`',
};
const MODIFIERS = new Set(['Control', 'Shift', 'Alt', 'Meta', 'OS']);

/** The shortcut a key press is, or `null` for a modifier on its own. */
export function keyOf(event: { key: string; ctrlKey: boolean; shiftKey: boolean; altKey: boolean; metaKey: boolean }): string | null {
    if (MODIFIERS.has(event.key)) {
        return null;
    }
    let key = NAMES[event.key] ?? event.key;
    if (key.length === 1) {
        key = key.toUpperCase();
    }
    // With shift held, a symbol key says its shifted symbol, which isn't what was meant for a shortcut like Ctrl+Shift+1.
    if (event.shiftKey && UNSHIFTED[key] !== undefined) key = UNSHIFTED[key];
    const parts: string[] = [];
    if (event.ctrlKey) parts.push('Ctrl');
    if (event.altKey) parts.push('Alt');
    if (event.shiftKey) parts.push('Shift');
    if (event.metaKey) parts.push('Win');
    parts.push(key);
    return parts.join('+');
}

/** How a shortcut is shown: `ArrowLeft` as `←` and so on. */
export function showKey(shortcut: string): string {
    return shortcut
        .replace('ArrowLeft', '←')
        .replace('ArrowRight', '→')
        .replace('ArrowUp', '↑')
        .replace('ArrowDown', '↓');
}
