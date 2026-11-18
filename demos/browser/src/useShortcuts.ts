// The browser's shortcuts while its own window has the keyboard. Inside a page, its `<webview>` hands them over instead
// (`keys` and `onKey`), so the same `run` does them either way.

import { useEffect } from 'react';
import { type Command, keyOf, SHORTCUTS } from './keys';

/** What still works while something's over the page: what opens something else. Typing in it isn't taken for shortcuts. */
const OVER_OVERLAYS = new Set<Command>(['settings', 'newTab', 'address']);

export function useShortcuts(run: (command: Command) => void, overlayOpen: boolean) {
    // Listened for again each render, so what's run is always this render's `run`.
    useEffect(() => {
        const heard = (event: any) => {
            const key = keyOf(event);
            const command = key === null ? undefined : SHORTCUTS[key];
            if (command !== undefined && (!overlayOpen || OVER_OVERLAYS.has(command))) {
                event.preventDefault();
                run(command);
            }
        };
        document.addEventListener('keydown', heard);
        return () => document.removeEventListener('keydown', heard);
    });
}
