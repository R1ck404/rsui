// What the command bar suggests for what's typed: the address or search it makes first, then open tabs (switched to rather
// than opened again), favorites and where the browser's been, best match first.

import type { Favorite, Tab, Visit } from '../model';
import { type Engine, hostOf, isSearch, resolve, searchUrl, shortUrl, titleOf } from '../url';

export interface Suggestion {
    kind: 'go' | 'search' | 'tab' | 'favorite' | 'history';
    title: string;
    url: string;
    icon: string;
    /** The open tab it is, for switching to rather than opening again. */
    tab?: Tab;
}

const LIMIT = 7;

/** Where the browser's been most lately, for when nothing's been typed. */
function recent(history: Visit[]): Suggestion[] {
    return history.slice(0, LIMIT).map((visit) => ({ kind: 'history', title: titleOf(visit.url, visit.title), url: visit.url, icon: '' }));
}

/** How well `text` matches a place, or 0 for not at all: every word has to be in its title or address. */
function score(words: string[], title: string, url: string): number {
    const lowTitle = title.toLowerCase();
    const lowUrl = shortUrl(url).toLowerCase();
    let total = 0;
    for (const word of words) {
        const inUrl = lowUrl.indexOf(word);
        const inTitle = lowTitle.indexOf(word);
        if (inUrl < 0 && inTitle < 0) {
            return 0;
        }
        // At the start of the host, or of a word in the title, counts most.
        total += inUrl === 0 ? 6 : inTitle === 0 || (inTitle > 0 && /\s/.test(lowTitle[inTitle - 1])) ? 4 : 2;
    }
    return total;
}

/** Where suggestions come from. */
export interface Places {
    engine: Engine;
    tabs: Tab[];
    favorites: Favorite[];
    history: Visit[];
}

/**
 * What to suggest for `text`. While it's still `current`, the address of the page being changed, that address comes first and
 * the rest is where the browser's been lately, as nothing new has been typed yet.
 */
export function suggest(text: string, current: string, places: Places): Suggestion[] {
    const { engine, tabs, favorites, history } = places;
    if (current !== '' && text === current) {
        return [{ kind: 'go', title: shortUrl(current), url: current, icon: '' }, ...recent(history).filter((one) => one.url !== current)];
    }
    const typed = text.trim();
    if (typed === '') {
        return recent(history);
    }
    const words = typed.toLowerCase().split(/\s+/);
    const found: (Suggestion & { score: number })[] = [];
    const seen = new Set<string>();
    const add = (suggestion: Suggestion, points: number) => {
        if (points > 0 && !seen.has(suggestion.url)) {
            seen.add(suggestion.url);
            found.push({ ...suggestion, score: points });
        }
    };
    for (const tab of tabs) {
        if (tab.url !== '') {
            add({ kind: 'tab', title: titleOf(tab.url, tab.title), url: tab.url, icon: tab.icon, tab }, score(words, tab.title, tab.url) * 1.6);
        }
    }
    for (const favorite of favorites) {
        add({ kind: 'favorite', title: favorite.title || hostOf(favorite.url), url: favorite.url, icon: favorite.icon }, score(words, favorite.title, favorite.url) * 1.4);
    }
    const now = Date.now();
    for (const visit of history) {
        const points = score(words, visit.title, visit.url);
        if (points > 0) {
            const days = (now - visit.last) / 86_400_000;
            add({ kind: 'history', title: titleOf(visit.url, visit.title), url: visit.url, icon: '' }, points * (1 + Math.log2(1 + visit.visits) / 3) / (1 + days / 14));
        }
    }
    found.sort((a, b) => b.score - a.score);
    const first: Suggestion = isSearch(typed, engine)
        ? { kind: 'search', title: typed, url: searchUrl(typed, engine), icon: '' }
        : { kind: 'go', title: shortUrl(resolve(typed, engine)), url: resolve(typed, engine), icon: '' };
    return [first, ...found.filter((one) => one.url !== first.url).slice(0, LIMIT - 1)];
}
