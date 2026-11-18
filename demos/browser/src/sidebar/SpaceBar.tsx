// The bottom of the sidebar: settings, a dot for each space, the open one bigger and in full color, and a new space.

import * as I from '../icons';
import { spaceMenu } from '../menus';
import type { Browser } from '../useBrowser';
import { IconButton } from '../ui/IconButton';

export function SpaceBar(props: { browser: Browser; onSettings: () => void }) {
    const { browser } = props;
    const { spaces } = browser.session;
    return (
        <div className="flex h-8 shrink-0 items-center gap-1 pt-1">
            <IconButton icon={I.SettingsIcon} label="Settings (Ctrl+,)" onClick={props.onSettings} />
            <div className="flex min-w-0 flex-1 items-center justify-center gap-1">
                {spaces.map((space) => {
                    const open = space.id === browser.space.id;
                    return (
                        <button
                            key={space.id}
                            title={space.name}
                            aria-label={space.name}
                            aria-pressed={open}
                            onClick={() => browser.showSpace(space.id)}
                            onContextMenu={() => spaceMenu(browser, space, props.onSettings)}
                            className="flex size-7 items-center justify-center rounded-full transition-colors duration-(--b-speed) hover:bg-(--b-hover)"
                        >
                            <span
                                className={'rounded-full transition-all duration-(--b-speed) ' + (open ? 'size-2.5' : 'size-[7px] opacity-45')}
                                style={{ background: space.color }}
                            />
                        </button>
                    );
                })}
            </div>
            <IconButton icon={I.PlusIcon} label="New space" onClick={browser.addSpace} />
        </div>
    );
}
