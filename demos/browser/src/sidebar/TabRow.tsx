// A tab in the sidebar. The open one is a lifted pill; one that's asleep is quieter. A pinned tab's button lets its page go
// rather than closing it.

import * as I from '../icons';
import type { Tab } from '../model';
import { SiteIcon } from '../ui/SiteIcon';
import { titleOf } from '../url';

export function TabRow(props: {
    tab: Tab;
    pinned: boolean;
    /** Whether it's the tab that's showing. */
    open: boolean;
    /** Whether its page is kept, rather than asleep. */
    live: boolean;
    onPick: () => void;
    onClose: () => void;
    onMenu: () => void;
}) {
    const { tab, pinned, open, live } = props;
    const title = titleOf(tab.url, tab.title);
    const quiet = !live && !open;
    const close = pinned ? 'Put to sleep' : 'Close tab';
    return (
        <div
            role="tab"
            aria-selected={open}
            title={title}
            onClick={props.onPick}
            onAuxClick={(event: any) => event.button === 1 && props.onClose()}
            onContextMenu={props.onMenu}
            className={
                'group flex h-8 shrink-0 items-center gap-2.5 rounded-(--b-radius) pr-1 pl-2 text-(--b-text) transition-colors duration-(--b-speed) ' +
                (open ? 'bg-(--b-tab) shadow-(--b-tab-shadow)' : 'hover:bg-(--b-hover)')
            }
        >
            <span className={'flex shrink-0 transition-opacity duration-(--b-speed) ' + (quiet ? 'opacity-60' : '')}>
                <SiteIcon url={tab.url} icon={tab.icon} size={16} />
            </span>
            <span className={'min-w-0 flex-1 truncate ' + (quiet ? 'text-(--b-muted)' : '')}>{title}</span>
            <button
                title={close}
                aria-label={close}
                onClick={(event: any) => {
                    event.stopPropagation();
                    props.onClose();
                }}
                className={
                    'flex size-6 shrink-0 items-center justify-center rounded-[6px] text-(--b-muted) hover:bg-(--b-press) hover:text-(--b-text) ' +
                    (open ? 'opacity-100' : 'opacity-0 group-hover:opacity-100')
                }
            >
                {pinned ? <span className="h-0.5 w-[9px] rounded-full bg-current" /> : <I.CloseIcon size={12} strokeWidth={1.8} />}
            </button>
        </div>
    );
}
