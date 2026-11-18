// What the window looks like and says: the theme's and the space's colors on the page, the frame's material, the page's title
// on the taskbar, and full screen while a page has something full screen.

import { useEffect } from 'react';
import { mainWindow, useColorScheme } from 'rsui';
import type { Settings } from './model';
import { apply, variables } from './theme';

/** Puts the colors on the page, and says whether they're dark. */
export function useWindowLook(settings: Settings, color: string): boolean {
    const system = useColorScheme();
    const dark = settings.theme === 'system' ? system === 'dark' : settings.theme === 'dark';
    apply(variables(settings, dark, color));
    useEffect(() => {
        try {
            mainWindow.setBackdrop(settings.material);
        } catch {
            // Windows 10 has no materials, and the frame is solid there anyway.
        }
    }, [settings.material]);
    return dark;
}

export function useWindowFollowsPage(title: string, fullScreen: boolean) {
    useEffect(() => {
        mainWindow.setTitle(title === '' ? 'Browser' : title + ' – Browser');
    }, [title]);
    useEffect(() => {
        mainWindow.setFullscreen(fullScreen);
    }, [fullScreen]);
}
