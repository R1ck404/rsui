// Pictures of what's in files, as Explorer makes them, kept by path and time so each is asked for once. The oldest go first once
// there are more than the setting allows.

import { useEffect, useState } from 'react';
import { files } from 'rsui/desktop';
import type { Entry } from './files';

const kept = new Map<string, string | null>();
const asking = new Map<string, Promise<string | null>>();

function keyOf(entry: Entry, size: number): string {
    return entry.path + ':' + entry.modified + ':' + size;
}

function remember(key: string, url: string | null, most: number) {
    kept.delete(key);
    kept.set(key, url);
    while (kept.size > most) {
        kept.delete(kept.keys().next().value as string);
    }
}

export function thumbnail(entry: Entry, size: number, most: number): Promise<string | null> {
    const key = keyOf(entry, size);
    if (kept.has(key)) {
        return Promise.resolve(kept.get(key) ?? null);
    }
    let pending = asking.get(key);
    if (!pending) {
        pending = files
            .thumbnail(entry.path, { size })
            .then((found) => (found ? found.url : null), () => null)
            .then((url) => {
                remember(key, url, most);
                asking.delete(key);
                return url;
            });
        asking.set(key, pending);
    }
    return pending;
}

/** What Explorer would show of it, once it's been made, or `null` until then and for what has none. */
export function useThumbnail(entry: Entry | null, size: number, enabled: boolean, most: number, delay = 0): string | null {
    const key = entry ? keyOf(entry, size) : '';
    const [url, setUrl] = useState<string | null>(() => (entry && enabled ? kept.get(key) ?? null : null));
    useEffect(() => {
        if (!entry || !enabled) {
            setUrl(null);
            return;
        }
        if (kept.has(key)) {
            setUrl(kept.get(key) ?? null);
            return;
        }
        setUrl(null);
        let live = true;
        // A moment's wait, so holding an arrow key down or flinging through a folder doesn't ask for every one it passes.
        const later = setTimeout(() => thumbnail(entry, size, most).then((found) => live && setUrl(found)), delay);
        return () => {
            live = false;
            clearTimeout(later);
        };
    }, [key, enabled]);
    return url;
}

export const thumbnailCount = (): number => kept.size;

/** Kinds that have a picture of what's in them. */
export const HAS_THUMBNAIL = new Set(['image', 'design', 'video', 'pdf', 'doc', 'sheet', 'slides']);
