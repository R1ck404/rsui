// What can be done to the spaces, tabs and favorites, as plain functions from one session to the next, so the browser's state
// changes in one place and each change is easy to follow.

import type { WebViewState } from 'rsui/webview';
import { type Favorite, newId, newSpace, type Session, type Space, SPACE_COLORS, type Tab } from './model';
import { hostOf, titleOf } from './url';

/** How many closed tabs can be opened again. */
const CLOSED_LIMIT = 50;

/** Every tab of a space, pinned first. */
export function tabsOf(space: Space): Tab[] {
    return [...space.pinned, ...space.tabs];
}

export function openSpace(session: Session): Space {
    return session.spaces.find((space) => space.id === session.space) ?? session.spaces[0];
}

/** The open tab of a space, if it has one. */
export function activeTab(space: Space): Tab | null {
    return tabsOf(space).find((tab) => tab.id === space.active) ?? null;
}

export function isPinned(space: Space, id: string): boolean {
    return space.pinned.some((tab) => tab.id === id);
}

/** `session` with the tab `id` changed by `change`, wherever it is. */
export function withTab(session: Session, id: string, change: (tab: Tab) => Tab): Session {
    let changed = false;
    const map = (tabs: Tab[]) =>
        tabs.map((tab) => {
            if (tab.id !== id) return tab;
            const next = change(tab);
            changed ||= next !== tab;
            return next;
        });
    const spaces = session.spaces.map((space) => {
        const before = changed;
        const pinned = map(space.pinned);
        const tabs = map(space.tabs);
        return changed !== before ? { ...space, pinned, tabs } : space;
    });
    return changed ? { ...session, spaces } : session;
}

export function withSpace(session: Session, id: string, change: (space: Space) => Space): Session {
    return { ...session, spaces: session.spaces.map((space) => (space.id === id ? change(space) : space)) };
}

/** Opens `tab` in a space, just after its open tab, or at the top of today's, and makes it the open one. */
export function insertTab(session: Session, spaceId: string, tab: Tab): Session {
    return withSpace(session, spaceId, (space) => {
        const at = space.tabs.findIndex((other) => other.id === space.active);
        const tabs = space.tabs.slice();
        tabs.splice(at >= 0 ? at + 1 : 0, 0, tab);
        return { ...space, tabs, active: tab.id };
    });
}

export function selectTab(session: Session, spaceId: string, tab: Tab | null): Session {
    const next = { ...withSpace(session, spaceId, (space) => ({ ...space, active: tab?.id ?? null })), space: spaceId };
    return tab ? withTab(next, tab.id, (one) => ({ ...one, seen: Date.now() })) : next;
}

/** The tab that's opened when the open one at `index` of `tabs` closes: the one below it, or above it at the end. */
function heirOf(tabs: Tab[], index: number): Tab | null {
    return tabs[index + 1] ?? tabs[index - 1] ?? null;
}

/**
 * Closes one of today's tabs, keeping it to be opened again, and opens the next one if it was open. Gives what's open after,
 * which is `undefined` when it wasn't the open one.
 */
export function closeTab(session: Session, spaceId: string, tab: Tab): { session: Session; heir: Tab | null | undefined } {
    const space = session.spaces.find((one) => one.id === spaceId);
    if (space === undefined) return { session, heir: undefined };
    const at = space.tabs.findIndex((one) => one.id === tab.id);
    const heir = space.active === tab.id ? heirOf(space.tabs, at) : undefined;
    const next = withSpace(session, spaceId, (one) => ({
        ...one,
        tabs: one.tabs.filter((other) => other.id !== tab.id),
        active: heir === undefined ? one.active : heir?.id ?? null,
    }));
    return { session: remember(next, spaceId, [tab]), heir };
}

/** Closes all of a space's tabs of today, keeping its pinned ones. */
export function clearToday(session: Session, spaceId: string): Session {
    const space = session.spaces.find((one) => one.id === spaceId);
    if (space === undefined) return session;
    const next = withSpace(session, spaceId, (one) => ({ ...one, tabs: [], active: isPinned(one, one.active ?? '') ? one.active : null }));
    return remember(next, spaceId, space.tabs);
}

/** Keeps closed tabs, newest last, to be opened again. One that never went anywhere isn't worth it. */
function remember(session: Session, spaceId: string, tabs: Tab[]): Session {
    const kept = tabs.filter((tab) => tab.url !== '').map((tab) => ({ space: spaceId, tab }));
    return kept.length === 0 ? session : { ...session, closed: [...session.closed, ...kept].slice(-CLOSED_LIMIT) };
}

/** Opens the last closed tab again, at the top of its space's, or the open space's if its own has gone. */
export function reopenLast(session: Session): { session: Session; tab: Tab } | null {
    const last = session.closed[session.closed.length - 1];
    if (last === undefined) return null;
    const into = session.spaces.some((space) => space.id === last.space) ? last.space : session.space;
    const next = withSpace(session, into, (space) => ({ ...space, tabs: [last.tab, ...space.tabs], active: last.tab.id }));
    return { session: { ...next, space: into, closed: session.closed.slice(0, -1) }, tab: last.tab };
}

/** Moves a tab between a space's pinned tabs and today's. */
export function togglePinned(session: Session, spaceId: string, tab: Tab): Session {
    return withSpace(session, spaceId, (space) =>
        isPinned(space, tab.id)
            ? { ...space, pinned: space.pinned.filter((other) => other.id !== tab.id), tabs: [tab, ...space.tabs] }
            : { ...space, tabs: space.tabs.filter((other) => other.id !== tab.id), pinned: [...space.pinned, tab] },
    );
}

export function favoriteFor(session: Session, url: string): Favorite | undefined {
    return session.favorites.find((favorite) => hostOf(favorite.url) === hostOf(url));
}

/** Adds the tab's site to the favorites, or takes it away if it's there. */
export function toggleFavorite(session: Session, tab: Tab): Session {
    const there = favoriteFor(session, tab.url);
    return there
        ? { ...session, favorites: session.favorites.filter((one) => one !== there) }
        : { ...session, favorites: [...session.favorites, { id: newId(), url: tab.url, title: titleOf(tab.url, tab.title), icon: tab.icon }] };
}

/** Adds favorites, leaving out sites that are favorites already. */
export function addFavorites(session: Session, favorites: Favorite[]): Session {
    const fresh = favorites.filter((one) => favoriteFor(session, one.url) === undefined);
    return fresh.length === 0 ? session : { ...session, favorites: [...session.favorites, ...fresh] };
}

export function removeFavorite(session: Session, id: string): Session {
    return { ...session, favorites: session.favorites.filter((one) => one.id !== id) };
}

/** Moves a tab to the top of another space's tabs of today. */
export function moveTab(session: Session, from: string, to: string, tab: Tab): Session {
    const out = withSpace(session, from, (space) => ({
        ...space,
        pinned: space.pinned.filter((other) => other.id !== tab.id),
        tabs: space.tabs.filter((other) => other.id !== tab.id),
        active: space.active === tab.id ? null : space.active,
    }));
    return withSpace(out, to, (space) => ({ ...space, tabs: [tab, ...space.tabs] }));
}

/** A new space, in a color no other space has while there is one, which is opened. */
export function addSpace(session: Session): Session {
    const used = new Set(session.spaces.map((space) => space.color));
    const color = (SPACE_COLORS.find((one) => !used.has(one.color)) ?? SPACE_COLORS[session.spaces.length % SPACE_COLORS.length]).color;
    const made = newSpace('Space ' + (session.spaces.length + 1), color);
    return { ...session, spaces: [...session.spaces, made], space: made.id };
}

/** Deletes a space with its tabs, unless it's the only one. */
export function removeSpace(session: Session, id: string): Session {
    const spaces = session.spaces.filter((space) => space.id !== id);
    if (spaces.length === 0) return session;
    return { ...session, spaces, space: session.space === id ? spaces[0].id : session.space };
}

/** The space `by` places along from the open one, going round. */
export function nextSpace(session: Session, by: number): string {
    const at = session.spaces.findIndex((space) => space.id === session.space);
    return session.spaces[(at + by + session.spaces.length) % session.spaces.length].id;
}

/** A tab with what its page says now: where it is, its title and its icon, keeping what it had while that's still right. */
export function withPageState(tab: Tab, state: WebViewState): Tab {
    const samePage = tab.url === state.url;
    const title = state.title !== '' ? state.title : samePage ? tab.title : '';
    const icon = state.icon !== '' ? state.icon : samePage || hostOf(tab.url) === hostOf(state.url) ? tab.icon : '';
    if (samePage && tab.title === title && tab.icon === icon) return tab;
    return { ...tab, url: state.url, title, icon };
}

/** Favorites of a site without an icon get the one its page has just shown. */
export function withFavoriteIcon(session: Session, url: string, icon: string): Session {
    const host = hostOf(url);
    if (icon === '' || !session.favorites.some((one) => hostOf(one.url) === host && one.icon === '')) return session;
    return { ...session, favorites: session.favorites.map((one) => (hostOf(one.url) === host && one.icon === '' ? { ...one, icon } : one)) };
}
