// The right-click menus of tabs, favorites and spaces. They're the system's own menus, so they show over pages too.

import { menus } from 'rsui/desktop';
import type { Favorite, Space, Tab } from './model';
import * as S from './session';
import type { Browser } from './useBrowser';

export function tabMenu(browser: Browser, tab: Tab) {
    const { session, space, awake, active } = browser;
    const pinned = S.isPinned(space, tab.id);
    const goes = tab.url !== '';
    const others = session.spaces.filter((one) => one.id !== space.id);
    menus.popup([
        { label: pinned ? 'Unpin' : 'Pin', shortcut: 'ctrl+d', onClick: () => browser.togglePin(tab) },
        {
            label: S.favoriteFor(session, tab.url) ? 'Remove from favorites' : 'Add to favorites',
            disabled: !goes,
            onClick: () => browser.toggleFavorite(tab),
        },
        { separator: true },
        { label: 'Copy link', disabled: !goes, onClick: () => browser.copyUrl(tab.url) },
        { label: 'Duplicate', disabled: !goes, onClick: () => browser.open(tab.url, tab.title) },
        ...(others.length > 0 ? [{ label: 'Move to', items: others.map((one) => ({ label: one.name, onClick: () => browser.moveTab(tab, one.id) })) }] : []),
        { separator: true },
        { label: 'Put to sleep', disabled: !awake.has(tab.id) || tab.id === active?.id, onClick: () => browser.sleep([tab.id]) },
        { label: pinned ? 'Close page' : 'Close', shortcut: 'ctrl+w', onClick: () => browser.close(tab) },
    ]);
}

export function favoriteMenu(browser: Browser, favorite: Favorite) {
    menus.popup([
        { label: 'Open in new tab', onClick: () => browser.open(favorite.url, favorite.title) },
        { label: 'Copy link', onClick: () => browser.copyUrl(favorite.url) },
        { separator: true },
        { label: 'Remove from favorites', onClick: () => browser.removeFavorite(favorite.id) },
    ]);
}

export function spaceMenu(browser: Browser, space: Space, onEdit: () => void) {
    menus.popup([
        { label: 'Edit spaces…', onClick: onEdit },
        { separator: true },
        { label: 'Delete ' + space.name, disabled: browser.session.spaces.length <= 1, onClick: () => browser.removeSpace(space.id) },
    ]);
}
