// The card the page sits on: a view for every tab that's awake, in every space, only the open one shown, and the start page
// while the open tab hasn't gone anywhere yet.

import { type Command, KEPT_KEYS, SHORTCUTS } from '../keys';
import * as S from '../session';
import type { Browser } from '../useBrowser';
import { engineOf } from '../url';
import { Page } from './Page';
import { StartPage } from './StartPage';

export function Pages(props: { browser: Browser; hidden: boolean; fullScreen: boolean; onSearch: () => void; onCommand: (command: Command) => void }) {
    const { browser, hidden } = props;
    const { session, space, active, awake, live } = browser;
    return (
        <>
            {session.spaces.flatMap((home) =>
                S.tabsOf(home)
                    .filter((tab) => tab.url !== '' && awake.has(tab.id))
                    .map((tab) => (
                        <Page
                            // A space kept apart has cookies of its own, which a view is made with.
                            key={tab.id + (home.apart ? ':' + home.id : '')}
                            tab={tab}
                            shown={!hidden && home.id === space.id && tab.id === space.active}
                            rounded={!props.fullScreen}
                            profile={home.apart ? 'space-' + home.id : 'default'}
                            state={live[tab.id]}
                            keys={KEPT_KEYS}
                            view={(view) => browser.keepView(tab.id, view)}
                            onState={(state) => browser.heard(tab, state)}
                            onNewWindow={(url) => browser.open(url)}
                            onKey={(key) => {
                                const command = SHORTCUTS[key.name];
                                if (command !== undefined) props.onCommand(command);
                            }}
                            onClose={() => browser.close(tab)}
                            onRetry={(url) => browser.viewOf(tab.id)?.go(url)}
                        />
                    )),
            )}
            {!hidden && (active === null || active.url === '') ? (
                <StartPage
                    engine={engineOf(browser.settings.engine)}
                    favorites={session.favorites}
                    history={browser.history}
                    onSearch={props.onSearch}
                    onOpen={browser.go}
                />
            ) : null}
        </>
    );
}
