// A calm, minimal browser. rsui draws all of it, the frame, the sidebar, the command bar, the start page, settings and the
// tour, and each page is a `<webview>`, the system's browser view, where the page's card is.
//
// The window is a soft frame, tinted with the open space's color, with the sidebar on it and the page as a white card, as
// Arc's is. What the browser holds and does is in `src/useBrowser.ts`; this is the window that shows it.

import './app.css';
import { useState } from 'react';
import { PageWindow } from 'rsui/web';
import { CommandBar } from './src/command/CommandBar';
import type { Suggestion } from './src/command/suggest';
import type { Command } from './src/keys';
import { Onboarding } from './src/onboarding/Onboarding';
import { Pages } from './src/page/Pages';
import * as S from './src/session';
import { SettingsSheet } from './src/settings/SettingsSheet';
import { Dock } from './src/sidebar/Dock';
import { Peek } from './src/sidebar/Peek';
import { Sidebar } from './src/sidebar/Sidebar';
import { TitleBar } from './src/TitleBar';
import { useBrowser } from './src/useBrowser';
import { useShortcuts } from './src/useShortcuts';
import { useWindowFollowsPage, useWindowLook } from './src/useWindowLook';
import { engineOf, titleOf } from './src/url';

export const title = 'Browser';
export const size = [1280, 820];
// It draws its own title bar: the strip above the page, with the window's buttons in its corner.
export const frameless = true;

export default function Root() {
    return (
        <PageWindow>
            <Browser />
        </PageWindow>
    );
}

/** What's over the page: the command bar, to open a new tab or change where the open one goes, settings, or the tour. */
type Overlay = { kind: 'none' } | { kind: 'command'; replace: boolean } | { kind: 'settings' } | { kind: 'tour' };

function Browser() {
    const browser = useBrowser();
    const { settings, session, space, active, activeState } = browser;
    const [overlay, setOverlay] = useState<Overlay>(() => (settings.onboarded ? { kind: 'none' } : { kind: 'tour' }));
    // The sidebar shown over the page while it's hidden and the pointer's at the window's edge.
    const [peek, setPeek] = useState(false);
    const [dragWidth, setDragWidth] = useState<number | null>(null);

    const dark = useWindowLook(settings, space.color);
    const pageTitle = active ? titleOf(active.url, activeState?.title ?? active.title) : '';
    const fullScreen = activeState?.fullScreen === true;
    useWindowFollowsPage(pageTitle, fullScreen);

    const touring = overlay.kind === 'tour';
    const right = settings.sidebarSide === 'right';
    const showSidebar = !settings.sidebarHidden && !fullScreen && !touring;
    const sidebarWidth = dragWidth ?? settings.sidebarWidth;

    const close = () => {
        setOverlay({ kind: 'none' });
        browser.activeView()?.focus();
    };
    const openCommand = (replace: boolean) => {
        // A page that has the keyboard gives it back, for what's typed to go in the command bar.
        browser.activeView()?.blur();
        setOverlay({ kind: 'command', replace });
    };
    const newTab = () => (settings.newTab === 'page' ? browser.open('') : openCommand(false));
    const choose = (suggestion: Suggestion, inNewTab: boolean) => {
        setOverlay({ kind: 'none' });
        if (suggestion.tab) browser.select(suggestion.tab);
        else if (inNewTab || active === null) browser.open(suggestion.url);
        else browser.go(suggestion.url);
    };

    const run = (command: Command) => {
        const tabs = S.tabsOf(space);
        const at = tabs.findIndex((tab) => tab.id === active?.id);
        switch (command) {
            case 'newTab':
                return newTab();
            case 'address':
                return openCommand(true);
            case 'closeTab':
                return active ? browser.close(active) : undefined;
            case 'reopenTab':
                return browser.reopen();
            case 'nextTab':
            case 'previousTab': {
                const by = command === 'nextTab' ? 1 : -1;
                return tabs.length > 0 ? browser.select(tabs[(at + by + tabs.length) % tabs.length]) : undefined;
            }
            case 'reload':
                return browser.activeView()?.reload();
            case 'back':
                return browser.activeView()?.back();
            case 'forward':
                return browser.activeView()?.forward();
            case 'sidebar':
                setPeek(false);
                return browser.change({ sidebarHidden: !settings.sidebarHidden });
            case 'pin':
                return active ? browser.togglePin(active) : undefined;
            case 'copyUrl':
                return active && active.url !== '' ? browser.copyUrl(active.url) : undefined;
            case 'settings':
                browser.activeView()?.blur();
                return setOverlay({ kind: 'settings' });
            case 'nextSpace':
                return browser.switchSpace(1);
            case 'previousSpace':
                return browser.switchSpace(-1);
            case 'devTools':
                return browser.activeView()?.devTools();
            default: {
                // Ctrl+1 to 8 are those tabs, and Ctrl+9 the last, as in other browsers.
                const n = Number(command.slice(3));
                const tab = n === 9 ? tabs[tabs.length - 1] : tabs[n - 1];
                return tab ? browser.select(tab) : undefined;
            }
        }
    };
    useShortcuts(run, overlay.kind !== 'none');

    const sidebar = (
        <Sidebar
            browser={browser}
            onAddress={() => openCommand(true)}
            onNewTab={newTab}
            onSettings={() => run('settings')}
            onPicked={() => setPeek(false)}
        />
    );

    return (
        <div className={'relative flex h-screen flex-col overflow-hidden bg-(--b-frame) text-(--b-text) ' + (dark ? 'dark' : '')}>
            {fullScreen ? null : (
                <TitleBar
                    sidebarWidth={sidebarWidth}
                    sidebarShown={showSidebar}
                    sidebarHidden={settings.sidebarHidden}
                    dragging={dragWidth !== null}
                    right={right}
                    title={touring ? '' : pageTitle}
                    navigation={!touring}
                    canGoBack={activeState?.canGoBack === true}
                    canGoForward={activeState?.canGoForward === true}
                    loading={activeState?.loading === true}
                    canReload={active !== null && active.url !== ''}
                    onSidebar={() => run('sidebar')}
                    onBack={() => run('back')}
                    onForward={() => run('forward')}
                    onReload={() => run('reload')}
                    onStop={() => browser.activeView()?.stop()}
                />
            )}
            <div className={'relative flex min-h-0 flex-1 ' + (right ? 'flex-row-reverse' : '')}>
                {fullScreen ? null : (
                    <Dock
                        shown={showSidebar}
                        width={sidebarWidth}
                        right={right}
                        dragging={dragWidth !== null}
                        onResize={setDragWidth}
                        onResized={(width) => {
                            setDragWidth(null);
                            browser.change({ sidebarWidth: width });
                        }}
                        onEdge={() => !touring && setPeek(true)}
                    >
                        {sidebar}
                    </Dock>
                )}
                <main
                    className={
                        'relative flex min-w-0 flex-1 overflow-hidden bg-(--b-card) ' +
                        (fullScreen ? '' : 'mb-2 rounded-(--b-radius-lg) shadow-(--b-card-shadow) ' + (right ? 'ml-2' : 'mr-2'))
                    }
                >
                    <Pages browser={browser} hidden={touring} fullScreen={fullScreen} onSearch={() => openCommand(true)} onCommand={run} />
                    {touring ? (
                        <Onboarding
                            settings={settings}
                            color={space.color}
                            onSettings={browser.change}
                            onColor={(color) => browser.changeSpace(space.id, { color })}
                            onDone={(favorites) => {
                                browser.addFavorites(favorites);
                                browser.change({ onboarded: true });
                                setOverlay({ kind: 'command', replace: false });
                            }}
                        />
                    ) : null}
                </main>
                {showSidebar || fullScreen || touring ? null : (
                    <Peek open={peek} width={sidebarWidth} right={right} onLeave={() => setPeek(false)}>
                        {sidebar}
                    </Peek>
                )}
            </div>
            {overlay.kind === 'command' ? (
                <CommandBar
                    initial={overlay.replace ? active?.url ?? '' : ''}
                    replace={overlay.replace}
                    engine={engineOf(settings.engine)}
                    tabs={S.tabsOf(space)}
                    favorites={session.favorites}
                    history={browser.history}
                    onChoose={choose}
                    onClose={close}
                />
            ) : overlay.kind === 'settings' ? (
                <SettingsSheet browser={browser} onTour={() => setOverlay({ kind: 'tour' })} onClose={close} />
            ) : null}
        </div>
    );
}
