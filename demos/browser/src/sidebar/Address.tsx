// The open page's address at the top of the sidebar, which opens the command bar to change it. What's loading runs along its
// bottom.

import * as I from '../icons';
import { hostOf, isSecure, searchedFor, shortUrl } from '../url';

export function Address(props: { url: string; loading: boolean; showFullUrl: boolean; onClick: () => void }) {
    const { url, loading, showFullUrl } = props;
    const searched = searchedFor(url);
    const shown = url === '' ? 'Search or enter address' : searched ?? (showFullUrl ? shortUrl(url) : hostOf(url));
    const Icon = url === '' || searched !== null ? I.SearchIcon : isSecure(url) ? I.LockIcon : I.GlobeIcon;
    return (
        <button
            onClick={props.onClick}
            title={url}
            className="relative flex h-9 shrink-0 items-center gap-2 overflow-hidden rounded-(--b-radius) bg-(--b-hover) px-3 text-left transition-colors duration-(--b-speed) hover:bg-(--b-press)"
        >
            <span className="flex shrink-0 text-(--b-faint)">
                <Icon size={14} strokeWidth={1.8} />
            </span>
            <span className={'min-w-0 flex-1 truncate ' + (url === '' ? 'text-(--b-faint)' : 'text-(--b-text)')}>{shown}</span>
            <span
                className={
                    'absolute bottom-0 left-0 h-0.5 bg-(--b-accent) transition-all duration-(--b-move) ' + (loading ? 'w-[72%] opacity-100' : 'w-0 opacity-0')
                }
            />
        </button>
    );
}
