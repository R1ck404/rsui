// The browser's state, what's kept of it, and everything that changes it: settings, spaces and tabs, history, how each tab's
// page is, which pages are awake, and the views showing them.

import { useEffect, useRef, useState } from 'react';
import { clipboard } from 'rsui/desktop';
import type { WebViewHandle, WebViewState } from 'rsui/webview';
import {
    type Favorite, forgetHistory, loadHistory, loadSession, loadSettings, newTab, retitled, saveHistory, saveSession, saveSettings,
    type Session, type Settings, type Space, type Tab, visited, type Visit,
} from './model';
import * as S from './session';
import { hostOf } from './url';

export type Browser = ReturnType<typeof useBrowser>;

export function useBrowser() {
    const [settings, setSettings] = useState<Settings>(loadSettings);
    const [session, setSession] = useState<Session>(loadSession);
    const [history, setHistory] = useState<Visit[]>(loadHistory);
    // How each tab's page is, as its view last said.
    const [live, setLive] = useState<Record<string, WebViewState>>({});
    // Tabs whose pages are open: the open one, and those looked at lately. The rest are asleep, and have no view.
    const [awake, setAwake] = useState<Set<string>>(() => {
        const active = S.openSpace(session).active;
        return new Set(active ? [active] : []);
    });
    const views = useRef(new Map<string, WebViewHandle>());

    const update = (change: (session: Session) => Session) =>
        setSession((before) => {
            const next = change(before);
            if (next !== before) saveSession(next);
            return next;
        });
    const remember = (change: (history: Visit[]) => Visit[]) =>
        setHistory((before) => {
            const next = change(before);
            if (next !== before) saveHistory(next);
            return next;
        });
    const change = (changes: Partial<Settings>) =>
        setSettings((before) => {
            const next = { ...before, ...changes };
            saveSettings(next);
            return next;
        });
    const wake = (id: string) => setAwake((before) => (before.has(id) ? before : new Set([...before, id])));
    const sleep = (ids: Iterable<string>) => {
        const gone = new Set(ids);
        setAwake((before) => {
            const next = new Set([...before].filter((id) => !gone.has(id)));
            return next.size === before.size ? before : next;
        });
    };

    const space = S.openSpace(session);
    const active = S.activeTab(space);
    const view = (id: string | undefined) => (id === undefined ? null : views.current.get(id) ?? null);
    const activeView = () => view(active?.id);

    // Tabs that haven't been looked at for a while let their pages go.
    useEffect(() => {
        if (settings.sleepAfter <= 0) return undefined;
        const timer = setInterval(() => {
            const open = new Set(session.spaces.map((one) => one.active));
            const limit = Date.now() - settings.sleepAfter * 60_000;
            sleep(session.spaces.flatMap(S.tabsOf).filter((tab) => !open.has(tab.id) && tab.seen < limit).map((tab) => tab.id));
        }, 60_000);
        return () => clearInterval(timer);
    }, [settings.sleepAfter, session]);

    const select = (tab: Tab | null, spaceId: string = space.id) => {
        update((before) => S.selectTab(before, spaceId, tab));
        if (tab !== null && tab.url !== '') wake(tab.id);
    };

    /** Opens `url` in a new tab after the open one, and goes to it. An empty one shows the start page. */
    const open = (url: string, title = '') => {
        const tab = newTab(url, title);
        update((before) => S.insertTab(before, space.id, tab));
        if (url !== '') wake(tab.id);
    };

    /** Sends the open tab somewhere else, or opens a tab there if none is open. */
    const go = (url: string) => {
        if (active === null) return open(url);
        update((before) => S.withTab(before, active.id, (tab) => ({ ...tab, url, seen: Date.now() })));
        wake(active.id);
        view(active.id)?.go(url);
    };

    /** Closes a tab. A pinned tab stays, and only lets its page go. */
    const close = (tab: Tab) => {
        if (S.isPinned(space, tab.id)) {
            sleep([tab.id]);
            if (space.active === tab.id) select(null);
            return;
        }
        const closed = S.closeTab(session, space.id, tab);
        update(() => closed.session);
        sleep([tab.id]);
        if (closed.heir) wake(closed.heir.id);
    };

    const reopen = () => {
        const reopened = S.reopenLast(session);
        if (reopened === null) return;
        update(() => reopened.session);
        wake(reopened.tab.id);
    };

    const openFavorite = (favorite: Favorite) => {
        const there = S.tabsOf(space).find((tab) => tab.url !== '' && hostOf(tab.url) === hostOf(favorite.url));
        if (there) select(there);
        else open(favorite.url, favorite.title);
    };

    const removeSpace = (id: string) => {
        const gone = session.spaces.find((one) => one.id === id);
        update((before) => S.removeSpace(before, id));
        if (gone) sleep(S.tabsOf(gone).map((tab) => tab.id));
    };

    /** What a tab's view says about its page, which is kept, and remembered in the history once the page has loaded. */
    const heard = (tab: Tab, state: WebViewState) => {
        setLive((before) => ({ ...before, [tab.id]: state }));
        if (state.url === '') return;
        update((before) => S.withFavoriteIcon(S.withTab(before, tab.id, (one) => S.withPageState(one, state)), state.url, state.icon));
        if (settings.keepHistory && !state.loading && state.error === null && /^https?:/i.test(state.url)) {
            remember((before) => (before[0]?.url === state.url ? retitled(before, state.url, state.title) : visited(before, state.url, state.title)));
        }
    };

    return {
        settings,
        session,
        history,
        live,
        awake,
        space,
        active,
        activeState: active ? live[active.id] : undefined,
        change,
        activeView,
        /** Keeps the view of a tab's page, or forgets it. */
        keepView: (id: string, handle: WebViewHandle | null) => (handle ? views.current.set(id, handle) : views.current.delete(id)),
        viewOf: view,
        select,
        open,
        go,
        close,
        reopen,
        heard,
        openFavorite,
        removeSpace,
        sleep,
        togglePin: (tab: Tab) => update((before) => S.togglePinned(before, space.id, tab)),
        toggleFavorite: (tab: Tab) => update((before) => S.toggleFavorite(before, tab)),
        addFavorites: (favorites: Favorite[]) => update((before) => S.addFavorites(before, favorites)),
        removeFavorite: (id: string) => update((before) => S.removeFavorite(before, id)),
        // Spaces kept apart have cookies of their own, so a moved tab's page opens again in the one it's gone to.
        moveTab: (tab: Tab, to: string) => {
            update((before) => S.moveTab(before, space.id, to, tab));
            sleep([tab.id]);
        },
        clearToday: () => {
            update((before) => S.clearToday(before, space.id));
            sleep(space.tabs.map((tab) => tab.id));
        },
        showSpace: (id: string) => update((before) => ({ ...before, space: id })),
        switchSpace: (by: number) => update((before) => ({ ...before, space: S.nextSpace(before, by) })),
        addSpace: () => update(S.addSpace),
        changeSpace: (id: string, changes: Partial<Space>) => update((before) => S.withSpace(before, id, (one) => ({ ...one, ...changes }))),
        copyUrl: (url: string) => void clipboard.write(url),
        clearHistory: () => {
            forgetHistory();
            setHistory([]);
        },
    };
}
