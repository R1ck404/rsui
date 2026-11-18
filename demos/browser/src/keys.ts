// The browser's shortcuts. They work while a page has the keyboard too: each is given to the page's view as one the app keeps,
// so the page never sees it.

export type Command =
    | 'newTab' | 'address' | 'closeTab' | 'reopenTab' | 'nextTab' | 'previousTab' | 'reload' | 'back' | 'forward'
    | 'sidebar' | 'pin' | 'copyUrl' | 'settings' | 'nextSpace' | 'previousSpace' | 'devTools'
    | 'tab1' | 'tab2' | 'tab3' | 'tab4' | 'tab5' | 'tab6' | 'tab7' | 'tab8' | 'tab9';

export const SHORTCUTS: Record<string, Command> = {
    'Ctrl+T': 'newTab',
    'Ctrl+L': 'address',
    F6: 'address',
    'Ctrl+W': 'closeTab',
    'Ctrl+Shift+T': 'reopenTab',
    'Ctrl+Tab': 'nextTab',
    'Ctrl+Shift+Tab': 'previousTab',
    'Ctrl+R': 'reload',
    F5: 'reload',
    'Alt+ArrowLeft': 'back',
    'Alt+ArrowRight': 'forward',
    'Ctrl+S': 'sidebar',
    'Ctrl+D': 'pin',
    'Ctrl+Shift+C': 'copyUrl',
    'Ctrl+,': 'settings',
    'Ctrl+Alt+ArrowRight': 'nextSpace',
    'Ctrl+Alt+ArrowLeft': 'previousSpace',
    'Ctrl+Shift+I': 'devTools',
    ...Object.fromEntries([1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => ['Ctrl+' + n, ('tab' + n) as Command])),
};

/** Every shortcut, for a page's view to keep for the app. Escape stays the page's: it closes its own menus and full screen. */
export const KEPT_KEYS = Object.keys(SHORTCUTS);

/** What's shown for each, in the settings and on the last page of the tour. */
export const SHOWN: { keys: string; does: string }[] = [
    { keys: 'Ctrl+T', does: 'New tab, or search' },
    { keys: 'Ctrl+L', does: 'Edit the address' },
    { keys: 'Ctrl+W', does: 'Close the tab' },
    { keys: 'Ctrl+Shift+T', does: 'Reopen a closed tab' },
    { keys: 'Ctrl+Tab', does: 'Next tab' },
    { keys: 'Ctrl+1 … 9', does: 'Go to a tab' },
    { keys: 'Ctrl+S', does: 'Show or hide the sidebar' },
    { keys: 'Ctrl+D', does: 'Pin or unpin the tab' },
    { keys: 'Ctrl+Alt+← →', does: 'Switch spaces' },
    { keys: 'Ctrl+Shift+C', does: 'Copy the address' },
    { keys: 'Alt+← →', does: 'Back and forward' },
    { keys: 'Ctrl+,', does: 'Settings' },
];

const NAMES: Record<string, string> = { ' ': 'Space', Esc: 'Escape', Left: 'ArrowLeft', Right: 'ArrowRight', Up: 'ArrowUp', Down: 'ArrowDown' };
const UNSHIFTED: Record<string, string> = { '!': '1', '@': '2', '#': '3', '$': '4', '%': '5', '^': '6', '&': '7', '*': '8', '(': '9', '<': ',' };
const MODIFIERS = new Set(['Control', 'Shift', 'Alt', 'Meta', 'OS']);

/** The shortcut a key press is, as `Ctrl+Shift+T`, or `null` for a modifier on its own. */
export function keyOf(event: { key: string; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }): string | null {
    if (MODIFIERS.has(event.key)) {
        return null;
    }
    let key = NAMES[event.key] ?? event.key;
    if (key.length === 1) {
        key = key.toUpperCase();
    }
    if (event.shiftKey && UNSHIFTED[key] !== undefined) {
        key = UNSHIFTED[key];
    }
    const parts: string[] = [];
    if (event.ctrlKey) parts.push('Ctrl');
    if (event.altKey) parts.push('Alt');
    if (event.shiftKey) parts.push('Shift');
    parts.push(key);
    return parts.join('+');
}
