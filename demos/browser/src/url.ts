// What's typed into the command bar, as somewhere to go: an address, something that looks like one, or a search.

export interface Engine {
    id: string;
    name: string;
    /** Where a search goes, with `%s` for what's searched for. */
    search: string;
}

export const ENGINES: Engine[] = [
    { id: 'duckduckgo', name: 'DuckDuckGo', search: 'https://duckduckgo.com/?q=%s' },
    { id: 'google', name: 'Google', search: 'https://www.google.com/search?q=%s' },
    { id: 'bing', name: 'Bing', search: 'https://www.bing.com/search?q=%s' },
    { id: 'brave', name: 'Brave Search', search: 'https://search.brave.com/search?q=%s' },
    { id: 'ecosia', name: 'Ecosia', search: 'https://www.ecosia.org/search?q=%s' },
    { id: 'kagi', name: 'Kagi', search: 'https://kagi.com/search?q=%s' },
];

export function engineOf(id: string): Engine {
    return ENGINES.find((engine) => engine.id === id) ?? ENGINES[0];
}

const SCHEME = /^[a-z][a-z0-9+.-]*:/i;
/** Something with a dot and no spaces, ending in letters, as `example.com/path` is, or `localhost`, or an address by number. */
const HOSTLIKE = /^(localhost|(\d{1,3}\.){3}\d{1,3}|\[[0-9a-f:]+\]|[^\s/?#]+\.[a-z]{2,}|[^\s/?#]+\.xn--[a-z0-9-]+)(:\d+)?([/?#]\S*)?$/i;

/** Where what's typed goes: as it is if it has a scheme, with `https://` if it looks like an address, or a search otherwise. */
export function resolve(typed: string, engine: Engine): string {
    const text = typed.trim();
    if (text === '') {
        return '';
    }
    if (/^(https?|file|about|data|view-source|edge):/i.test(text)) {
        return text;
    }
    if (!/\s/.test(text) && HOSTLIKE.test(text)) {
        return (/^localhost|^(\d{1,3}\.){3}\d{1,3}/i.test(text) ? 'http://' : 'https://') + text;
    }
    if (SCHEME.test(text) && !/\s/.test(text) && !/^[a-z]+:\d/i.test(text)) {
        return text;
    }
    return searchUrl(text, engine);
}

export function searchUrl(text: string, engine: Engine): string {
    return engine.search.replace('%s', encodeURIComponent(text.trim()).replace(/%20/g, '+'));
}

/** Whether `typed` would be searched for rather than gone to. */
export function isSearch(typed: string, engine: Engine): boolean {
    return resolve(typed, engine) === searchUrl(typed, engine);
}

/** An address's host, without `www.`: what a tab without a title, and the sidebar's address, show. */
export function hostOf(url: string): string {
    const match = /^[a-z][a-z0-9+.-]*:\/\/(?:[^@/?#]*@)?([^/?#:]+|\[[^\]]+\])/i.exec(url);
    if (match === null) {
        return url.replace(/^about:/, '');
    }
    return match[1].replace(/^www\./i, '').toLowerCase();
}

/** An address as it's shown when it's not being edited: without its scheme, `www.` or a lone slash at the end. */
export function shortUrl(url: string): string {
    return url.replace(/^https?:\/\//i, '').replace(/^www\./i, '').replace(/\/$/, '');
}

export function isSecure(url: string): boolean {
    return /^https:/i.test(url);
}

/** What a search address searched for, if it's one of the engines', for showing the words rather than the address. */
export function searchedFor(url: string): string | null {
    for (const engine of ENGINES) {
        const [base] = engine.search.split('?');
        if (url.startsWith(base + '?')) {
            const query = /[?&]q=([^&#]*)/.exec(url);
            if (query !== null) {
                try {
                    return decodeURIComponent(query[1].replace(/\+/g, ' '));
                } catch {
                    return null;
                }
            }
        }
    }
    return null;
}

/** What a tab is called: its page's title, or until it has one, what was searched for or the address's host. */
export function titleOf(url: string, title: string): string {
    return title.trim() !== '' ? title.trim() : url === '' ? 'New Tab' : searchedFor(url) ?? hostOf(url);
}
