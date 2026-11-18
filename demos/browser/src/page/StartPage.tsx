// What a new tab shows before it goes anywhere: a greeting, a search field that opens the command bar, the favorites and
// where the browser's been lately. It's all rsui's own, so it's there at once.

import * as I from '../icons';
import type { Favorite, Visit } from '../model';
import { Kbd } from '../ui/controls';
import { SiteIcon } from '../ui/SiteIcon';
import { type Engine, hostOf, shortUrl, titleOf } from '../url';

function greeting(hour: number): string {
    return hour < 5 ? 'Good night' : hour < 12 ? 'Good morning' : hour < 18 ? 'Good afternoon' : 'Good evening';
}

export function StartPage(props: {
    engine: Engine;
    favorites: Favorite[];
    history: Visit[];
    onSearch: () => void;
    onOpen: (url: string) => void;
}) {
    const { engine, favorites, history } = props;
    const recent = history.slice(0, 5);
    return (
        <div className="flex h-full w-full flex-col items-center overflow-y-auto bg-(--b-card) px-8 animate-[fade-in_var(--b-move)_var(--b-ease)]">
            <div className="flex w-full max-w-[560px] flex-col items-stretch pt-[16vh] pb-12">
                <h1 className="text-center text-[28px] font-semibold tracking-[-0.02em] text-(--b-text)">{greeting(new Date().getHours())}</h1>
                <p className="mt-1.5 text-center text-[14px] text-(--b-muted)">Where to?</p>
                <button
                    onClick={props.onSearch}
                    className="mt-8 flex h-12 items-center gap-3 rounded-(--b-radius-lg) bg-(--b-hover) px-4 text-left text-[14px] text-(--b-faint) transition-colors duration-(--b-speed) hover:bg-(--b-press)"
                >
                    <I.SearchIcon size={17} strokeWidth={1.8} />
                    <span className="flex-1">Search {engine.name} or enter address</span>
                    <Kbd>Ctrl</Kbd>
                    <Kbd>L</Kbd>
                </button>
                {favorites.length > 0 ? (
                    <div className="mt-10 grid grid-cols-[repeat(auto-fill,minmax(88px,1fr))] gap-2">
                        {favorites.map((favorite) => (
                            <button
                                key={favorite.id}
                                onClick={() => props.onOpen(favorite.url)}
                                className="flex flex-col items-center gap-2 rounded-(--b-radius-lg) px-2 py-3 transition-colors duration-(--b-speed) hover:bg-(--b-hover)"
                            >
                                <span className="flex h-12 w-12 items-center justify-center rounded-(--b-radius-lg) bg-(--b-hover)">
                                    <SiteIcon url={favorite.url} icon={favorite.icon} size={22} />
                                </span>
                                <span className="w-full truncate text-center text-[12px] text-(--b-muted)">{favorite.title || hostOf(favorite.url)}</span>
                            </button>
                        ))}
                    </div>
                ) : null}
                {recent.length > 0 ? (
                    <div className="mt-10 flex flex-col">
                        <span className="px-3 pb-1.5 text-[11.5px] font-medium text-(--b-faint)">Recently</span>
                        {recent.map((visit) => (
                            <button
                                key={visit.url}
                                onClick={() => props.onOpen(visit.url)}
                                className="flex h-9 items-center gap-3 rounded-(--b-radius) px-3 text-left transition-colors duration-(--b-speed) hover:bg-(--b-hover)"
                            >
                                <SiteIcon url={visit.url} icon="" size={15} />
                                <span className="min-w-0 flex-1 truncate text-(--b-text)">{titleOf(visit.url, visit.title)}</span>
                                <span className="max-w-[45%] shrink-0 truncate text-[12px] text-(--b-faint)">{shortUrl(visit.url)}</span>
                            </button>
                        ))}
                    </div>
                ) : null}
            </div>
        </div>
    );
}
