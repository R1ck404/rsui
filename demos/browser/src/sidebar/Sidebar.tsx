// The sidebar, on the window's frame, as Arc's is: the page's address, the favorites as tiles, the space's pinned tabs and its
// tabs of today, and the spaces along the bottom.

import * as I from '../icons';
import { favoriteMenu, tabMenu } from '../menus';
import type { Tab } from '../model';
import type { Browser } from '../useBrowser';
import { Address } from './Address';
import { Favorites } from './Favorites';
import { SpaceBar } from './SpaceBar';
import { TabRow } from './TabRow';

export function Sidebar(props: {
    browser: Browser;
    onAddress: () => void;
    onNewTab: () => void;
    onSettings: () => void;
    /** A tab was picked, as a sidebar shown over the page then goes. */
    onPicked: () => void;
}) {
    const { browser } = props;
    const { space, active, awake } = browser;
    const row = (tab: Tab, pinned: boolean) => (
        <TabRow
            key={tab.id}
            tab={tab}
            pinned={pinned}
            open={tab.id === active?.id}
            live={awake.has(tab.id)}
            onPick={() => {
                props.onPicked();
                browser.select(tab);
            }}
            onClose={() => browser.close(tab)}
            onMenu={() => tabMenu(browser, tab)}
        />
    );
    return (
        <div className="flex h-full min-h-0 flex-col gap-2 px-2 pb-2">
            <Address url={active?.url ?? ''} loading={browser.activeState?.loading === true} showFullUrl={browser.settings.showFullUrl} onClick={props.onAddress} />
            <Favorites
                favorites={browser.session.favorites}
                openUrl={active?.url ?? ''}
                onOpen={browser.openFavorite}
                onMenu={(favorite) => favoriteMenu(browser, favorite)}
            />
            <div className="flex items-center gap-2 px-2 pt-2 text-[11.5px] font-medium text-(--b-muted)">
                <span className="size-2 shrink-0 rounded-full" style={{ background: space.color }} />
                <span className="min-w-0 flex-1 truncate">{space.name}</span>
                {space.apart ? (
                    <span title="Its own cookies and sign-ins" className="flex text-(--b-faint)">
                        <I.ShieldIcon size={13} strokeWidth={1.7} />
                    </span>
                ) : null}
            </div>
            <div className="-mx-2 flex min-h-0 flex-1 flex-col overflow-y-auto px-2">
                {space.pinned.map((tab) => row(tab, true))}
                {space.pinned.length > 0 ? <div className="mx-2 my-2 h-px shrink-0 bg-(--b-line)" /> : null}
                <div className="flex items-center justify-between px-2 pt-0.5 pb-1">
                    <span className="text-[11px] text-(--b-faint)">Today</span>
                    {space.tabs.length > 0 ? (
                        <button onClick={browser.clearToday} className="rounded-[5px] px-1.5 text-[11px] text-(--b-faint) hover:bg-(--b-hover) hover:text-(--b-muted)">
                            Clear
                        </button>
                    ) : null}
                </div>
                <button
                    onClick={props.onNewTab}
                    className="flex h-8 shrink-0 items-center gap-2.5 rounded-(--b-radius) px-2 text-left text-(--b-muted) transition-colors duration-(--b-speed) hover:bg-(--b-hover) hover:text-(--b-text)"
                >
                    <I.PlusIcon size={16} strokeWidth={1.7} />
                    <span>New Tab</span>
                </button>
                {space.tabs.map((tab) => row(tab, false))}
            </div>
            <SpaceBar browser={browser} onSettings={props.onSettings} />
        </div>
    );
}
