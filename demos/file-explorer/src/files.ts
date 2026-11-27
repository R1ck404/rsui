// Reading folders, sorting and formatting what's in them, searching, and copying and moving files.

import { files, type FolderEntry } from 'rsui/desktop';
import { promises as fs } from 'fs';
import type { Settings, SortKey } from './settings';

/** Where "This PC" is: the list of drives, rather than a folder. */
export const COMPUTER = '';

export type Kind =
    | 'folder' | 'image' | 'design' | 'video' | 'audio' | 'archive' | 'code' | 'web' | 'data' | 'script' | 'markdown' | 'text' | 'config'
    | 'doc' | 'sheet' | 'slides' | 'pdf' | 'ebook' | 'model' | 'database' | 'app' | 'link' | 'disk' | 'font' | 'lock' | 'mail'
    | 'calendar' | 'contact' | 'file';

export interface Entry {
    name: string;
    path: string;
    /** In lower case, with no dot, or empty. */
    ext: string;
    kind: Kind;
    folder: boolean;
    hidden: boolean;
    system: boolean;
    readonly: boolean;
    link: boolean;
    size: number;
    modified: number;
    created: number;
    /** For search results: the folder it's in, relative to where the search started. */
    within?: string;
    /** Where it is in its folder in Explorer's order of names, which `files.list` gives them in. */
    rank: number;
}

const KINDS: Record<string, Kind> = {};
const add = (kind: Kind, list: string) => list.split(' ').forEach((ext) => (KINDS[ext] = kind));
add('image', 'png jpg jpeg jfif gif webp bmp ico tif tiff heic heif avif raw cr2 cr3 nef arw dng jxl');
add('design', 'psd ai fig sketch xd svg indd afdesign afphoto kra xcf eps');
add('video', 'mp4 mkv mov avi webm wmv m4v flv mpg mpeg 3gp');
add('audio', 'mp3 wav flac ogg m4a aac wma opus aiff aif mid midi');
add('archive', 'zip rar 7z tar gz tgz bz2 xz zst cab');
add('code', 'js mjs cjs ts tsx jsx rs py pyw c h cpp cc hpp cs java go rb php swift kt kts lua dart scala zig hs ex exs erl clj r vue svelte');
add('web', 'html htm xhtml css scss sass less');
add('data', 'json jsonc json5 xml yaml yml toml sql proto graphql gql');
add('script', 'sh bash zsh fish ps1 psm1 bat cmd');
add('markdown', 'md markdown mdx rst');
add('text', 'txt log nfo');
add('config', 'ini cfg conf env editorconfig gitignore gitattributes properties reg');
add('doc', 'doc docx odt rtf pages wpd');
add('sheet', 'xls xlsx xlsm ods numbers csv tsv');
add('slides', 'ppt pptx odp key');
add('pdf', 'pdf');
add('ebook', 'epub mobi azw azw3 djvu');
add('model', 'obj fbx stl blend gltf glb 3ds dae 3mf');
add('database', 'db sqlite sqlite3 mdb accdb');
add('app', 'exe msi appx msix com scr apk');
add('link', 'lnk url webloc');
add('disk', 'iso img vhd vhdx dmg');
add('font', 'ttf otf woff woff2 fon');
add('lock', 'key pem pfx p12 gpg kdbx crt cer');
add('mail', 'eml msg mbox');
add('calendar', 'ics');
add('contact', 'vcf');

const LABELS: Record<Kind, string> = {
    folder: 'Folder', image: 'Picture', design: 'Design', video: 'Video', audio: 'Audio', archive: 'Archive', code: 'Code', web: 'Web page',
    data: 'Data', script: 'Script', markdown: 'Markdown', text: 'Text', config: 'Settings file', doc: 'Document', sheet: 'Spreadsheet',
    slides: 'Presentation', pdf: 'PDF', ebook: 'Book', model: '3D model', database: 'Database', app: 'Application', link: 'Shortcut',
    disk: 'Disk image', font: 'Font', lock: 'Key', mail: 'Email', calendar: 'Calendar', contact: 'Contact', file: 'File',
};

export function kindLabel(entry: Entry): string {
    if (entry.kind === 'file' && entry.ext !== '') {
        return entry.ext.toUpperCase() + ' file';
    }
    return LABELS[entry.kind];
}

/** Windows paths start at a drive and are split by `\`. Everywhere else there's one tree, from `/`. */
export const WINDOWS = process.platform === 'win32';
export const SEP = WINDOWS ? '\\' : '/';

/** Whether `path` is where a tree starts: a drive, as `C:\`, or `/`. */
function isRoot(path: string): boolean {
    return WINDOWS ? /^[A-Za-z]:\\?$/.test(path) : path === '/';
}

/** `path` without the separators it ends in, unless it's a root. */
function trimmed(path: string): string {
    if (isRoot(path)) {
        return path;
    }
    return WINDOWS ? path.replace(/\\+$/, '') : path.replace(/\/+$/, '');
}

export function join(folder: string, name: string): string {
    return folder.endsWith(SEP) ? folder + name : folder + SEP + name;
}

/** Whether `path` is inside `folder`, comparing names as the system does: in any case on Windows. */
export function isInside(path: string, folder: string): boolean {
    const [inner, outer] = WINDOWS ? [path.toLowerCase(), folder.toLowerCase()] : [path, folder];
    return inner.startsWith(outer.endsWith(SEP) ? outer : outer + SEP);
}

/** Whether two paths are the same, as the system compares them. */
export function samePath(one: string, other: string): boolean {
    return WINDOWS ? one.toLowerCase() === other.toLowerCase() : one === other;
}

/** The folder a path is in, or `COMPUTER` for a drive's root. */
export function parentOf(path: string): string {
    if (path === COMPUTER || isRoot(path)) {
        return COMPUTER;
    }
    const clean = trimmed(path);
    const cut = clean.lastIndexOf(SEP);
    if (cut < 0) {
        return COMPUTER;
    }
    if (!WINDOWS && cut === 0) {
        return '/';
    }
    const parent = clean.slice(0, cut);
    return /^[A-Za-z]:$/.test(parent) ? parent + '\\' : parent;
}

/** What a drive is called: its label and letter on Windows, as `Local Disk (C:)`, and its label or folder elsewhere. */
export function driveName(drive: { label: string; path: string }): string {
    if (WINDOWS) {
        return (drive.label || 'Local Disk') + ' (' + drive.path.slice(0, 2) + ')';
    }
    return drive.label || (drive.path === '/' ? 'File System' : nameOf(drive.path));
}

export function nameOf(path: string): string {
    if (path === COMPUTER) {
        return WINDOWS ? 'This PC' : 'Computer';
    }
    if (isRoot(path)) {
        return WINDOWS ? path.slice(0, 2).toUpperCase() : '/';
    }
    const clean = trimmed(path);
    return clean.slice(clean.lastIndexOf(SEP) + 1);
}

/** Each folder from the drive down to `path`, for breadcrumbs. */
export function crumbsOf(path: string): { name: string; path: string }[] {
    const crumbs = [{ name: nameOf(COMPUTER), path: COMPUTER }];
    if (path === COMPUTER) {
        return crumbs;
    }
    if (!WINDOWS) {
        crumbs.push({ name: '/', path: '/' });
        let at = '/';
        for (const part of trimmed(path).split('/').filter((part) => part !== '')) {
            at = join(at, part);
            crumbs.push({ name: part, path: at });
        }
        return crumbs;
    }
    const parts = trimmed(path).split('\\');
    let at = '';
    for (const part of parts) {
        at = at === '' ? part + '\\' : join(at, part);
        crumbs.push({ name: at.length === 3 ? part.toUpperCase() : part, path: at });
    }
    return crumbs;
}

export function normalize(path: string): string {
    let clean = path.trim().replace(/^"|"$/g, '');
    if (!WINDOWS) {
        clean = clean.replace(/\/{2,}/g, '/');
        return clean === '' ? '/' : trimmed(clean);
    }
    clean = clean.replace(/\//g, '\\');
    if (/^[A-Za-z]:$/.test(clean)) {
        clean += '\\';
    }
    return trimmed(clean);
}

function entryOf(folder: string, found: FolderEntry): Entry {
    const name: string = found.name;
    const dot = name.lastIndexOf('.');
    const ext = !found.folder && dot > 0 ? name.slice(dot + 1).toLowerCase() : '';
    return {
        name,
        path: join(folder, name),
        ext,
        kind: found.folder ? 'folder' : KINDS[ext] ?? 'file',
        folder: found.folder,
        hidden: found.hidden,
        system: found.system,
        readonly: found.readonly,
        link: found.link,
        size: found.size,
        modified: found.modified,
        created: found.created,
        rank: 0,
    };
}

/** Everything in a folder, with sizes, times and attributes, read in one pass on another thread. */
export async function readFolder(folder: string, dotfilesHidden = false): Promise<Entry[]> {
    const listed = await files.list(folder);
    const entries = new Array<Entry>(listed.length);
    for (let at = 0; at < listed.length; at += 1) {
        entries[at] = entryOf(folder, listed[at]);
        entries[at].rank = at;
        if (dotfilesHidden && entries[at].name.startsWith('.')) {
            entries[at].hidden = true;
        }
    }
    return entries;
}

export interface Drive {
    path: string;
    label: string;
    kind: string;
    total: number;
    free: number;
}

export const readDrives = (): Promise<Drive[]> => files.drives();

// Ordering

/**
 * Puts entries in order. Each name is turned into a key once (lower case unless case matters, and with each run of digits
 * padded so that file2 comes before file10), and keys are compared as plain strings. That's about 30 times faster than
 * comparing names with `Intl.Collator` each time, which matters on a folder of 25,000 things. It changes nothing it's given,
 * so the engine gives what it gave again while what it's given is the same, and it only runs when that changes.
 */
const ZEROS = '0000000000000000';

export function sortEntries(
    entries: Entry[],
    settings: Pick<Settings, 'naturalSort' | 'caseSensitiveSort' | 'foldersFirst'>,
    key: SortKey,
    descending: boolean,
    /** Whether they all come from one folder, so `rank` already says their order by name. */
    oneFolder = true,
): Entry[] {
    const direction = descending ? -1 : 1;
    if (oneFolder && settings.naturalSort && !settings.caseSensitiveSort) {
        // The usual case, which needs no keys at all: Rust has put them in this order already.
        const byRank = (a: Entry, b: Entry) => a.rank - b.rank;
        const by: (a: Entry, b: Entry) => number =
            key === 'size' ? (a, b) => a.size - b.size || byRank(a, b)
            : key === 'modified' ? (a, b) => a.modified - b.modified || byRank(a, b)
            : key === 'created' ? (a, b) => a.created - b.created || byRank(a, b)
            : key === 'type' ? (a, b) => (a.ext < b.ext ? -1 : a.ext > b.ext ? 1 : 0) || byRank(a, b)
            : byRank;
        return entries.slice().sort((a, b) => {
            if (settings.foldersFirst && a.folder !== b.folder) {
                return a.folder ? -1 : 1;
            }
            return by(a, b) * direction;
        });
    }
    const nameKey = (name: string) => {
        const folded = settings.caseSensitiveSort ? name : name.toLowerCase();
        return settings.naturalSort ? folded.replace(/\d+/g, (digits) => (digits.length >= 16 ? digits : ZEROS.slice(digits.length) + digits)) : folded;
    };
    const keyed = entries.map((entry) => ({ entry, name: nameKey(entry.name), ext: entry.ext }));
    const byName = (a: { name: string }, b: { name: string }) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
    const by: (a: (typeof keyed)[number], b: (typeof keyed)[number]) => number =
        key === 'size' ? (a, b) => a.entry.size - b.entry.size || byName(a, b)
        : key === 'modified' ? (a, b) => a.entry.modified - b.entry.modified || byName(a, b)
        : key === 'created' ? (a, b) => a.entry.created - b.entry.created || byName(a, b)
        : key === 'type' ? (a, b) => (a.ext < b.ext ? -1 : a.ext > b.ext ? 1 : 0) || byName(a, b)
        : byName;
    keyed.sort((a, b) => {
        if (settings.foldersFirst && a.entry.folder !== b.entry.folder) {
            return a.entry.folder ? -1 : 1;
        }
        return by(a, b) * direction;
    });
    return keyed.map((one) => one.entry);
}

// What things read as

const BINARY = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
const DECIMAL = ['B', 'kB', 'MB', 'GB', 'TB'];
const grouped = new Intl.NumberFormat();

export function formatSize(bytes: number, units: Settings['sizeUnits']): string {
    if (units === 'bytes') {
        return grouped.format(bytes) + ' B';
    }
    const step = units === 'binary' ? 1024 : 1000;
    const names = units === 'binary' ? BINARY : DECIMAL;
    let size = bytes;
    let unit = 0;
    while (size >= step && unit < names.length - 1) {
        size /= step;
        unit += 1;
    }
    return (unit === 0 ? String(size) : size.toFixed(size < 10 ? 1 : 0)) + ' ' + names[unit];
}

const pad = (number: number) => String(number).padStart(2, '0');

export function formatDate(time: number, format: Settings['dateFormat'], clock24: boolean, now: number): string {
    if (!time) {
        return '';
    }
    const date = new Date(time);
    const clock = clock24
        ? pad(date.getHours()) + ':' + pad(date.getMinutes())
        : ((date.getHours() + 11) % 12) + 1 + ':' + pad(date.getMinutes()) + (date.getHours() < 12 ? ' AM' : ' PM');
    if (format === 'iso') {
        return date.getFullYear() + '-' + pad(date.getMonth() + 1) + '-' + pad(date.getDate()) + ' ' + clock;
    }
    if (format === 'short') {
        return pad(date.getDate()) + '/' + pad(date.getMonth() + 1) + '/' + date.getFullYear();
    }
    const month = MONTHS[date.getMonth()];
    if (format === 'long') {
        return date.getDate() + ' ' + month + ' ' + date.getFullYear() + ', ' + clock;
    }
    const seconds = Math.round((now - time) / 1000);
    if (seconds < 0) {
        return date.getDate() + ' ' + month + ' ' + date.getFullYear();
    }
    if (seconds < 60) {
        return 'Just now';
    }
    if (seconds < 3600) {
        const minutes = Math.floor(seconds / 60);
        return minutes + (minutes === 1 ? ' minute ago' : ' minutes ago');
    }
    if (seconds < 86400) {
        const hours = Math.floor(seconds / 3600);
        return hours + (hours === 1 ? ' hour ago' : ' hours ago');
    }
    const days = Math.floor(seconds / 86400);
    if (days === 1) {
        return 'Yesterday, ' + clock;
    }
    if (days < 7) {
        return DAYS[date.getDay()] + ', ' + clock;
    }
    return date.getDate() + ' ' + month + (date.getFullYear() === new Date(now).getFullYear() ? '' : ' ' + date.getFullYear());
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];

export function attributesOf(entry: Entry): string {
    return (entry.readonly ? 'R' : '') + (entry.hidden ? 'H' : '') + (entry.system ? 'S' : '') + (entry.link ? 'L' : '');
}

export function displayName(entry: Entry, showExtensions: boolean): string {
    if (showExtensions || entry.folder || entry.ext === '') {
        return entry.name;
    }
    return entry.name.slice(0, entry.name.length - entry.ext.length - 1);
}

// Matching

export type Matcher = (name: string) => boolean;

export function matcher(query: string, mode: Settings['searchMatch'], matchCase: boolean): Matcher | null {
    if (query === '') {
        return null;
    }
    const fold = (text: string) => (matchCase ? text : text.toLowerCase());
    const wanted = fold(query);
    switch (mode) {
        case 'prefix':
            return (name) => fold(name).startsWith(wanted);
        case 'fuzzy':
            return (name) => {
                const have = fold(name);
                let at = 0;
                for (let i = 0; i < have.length && at < wanted.length; i += 1) {
                    if (have[i] === wanted[at]) {
                        at += 1;
                    }
                }
                return at === wanted.length;
            };
        case 'glob': {
            const source = query.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.');
            const pattern = new RegExp('^' + source + '$', matchCase ? '' : 'i');
            return (name) => pattern.test(name);
        }
        case 'regex':
            try {
                const pattern = new RegExp(query, matchCase ? '' : 'i');
                return (name) => pattern.test(name);
            } catch {
                return () => false;
            }
        default:
            return (name) => fold(name).includes(wanted);
    }
}

/**
 * Looks through `root` and what's under it, a folder at a time and breadth first, so near results come first. Results come in
 * batches through `found`. It stops when `cancelled()` says so, at `most` results, or `depth` folders down.
 */
export async function search(
    root: string,
    matches: Matcher,
    options: { depth: number; most: number; hidden: boolean; dotfilesHidden: boolean },
    found: (batch: Entry[]) => void,
    cancelled: () => boolean,
): Promise<{ folders: number; results: number }> {
    let level = [root];
    let folders = 0;
    let results = 0;
    for (let depth = 0; depth < options.depth && level.length > 0; depth += 1) {
        const next: string[] = [];
        for (const folder of level) {
            if (cancelled() || results >= options.most) {
                return { folders, results };
            }
            let listed: Entry[];
            try {
                listed = await readFolder(folder, options.dotfilesHidden);
            } catch {
                continue;
            }
            folders += 1;
            const batch: Entry[] = [];
            const within = folder === root ? '' : folder.slice(root.length).replace(WINDOWS ? /^\\/ : /^\//, '');
            for (const entry of listed) {
                if (!options.hidden && (entry.hidden || entry.system)) {
                    continue;
                }
                if (entry.folder && !entry.link) {
                    next.push(entry.path);
                }
                if (matches(entry.name) && results < options.most) {
                    entry.within = within;
                    batch.push(entry);
                    results += 1;
                }
            }
            if (batch.length > 0) {
                found(batch);
            }
        }
        level = next;
    }
    return { folders, results };
}

// Changing files

/** A name like `name` that isn't taken in `folder`: `name (2)`, `name (3)` and so on. */
export function freeName(taken: Set<string>, name: string): string {
    const lower = new Set([...taken].map((one) => one.toLowerCase()));
    if (!lower.has(name.toLowerCase())) {
        return name;
    }
    const dot = name.lastIndexOf('.');
    const base = dot > 0 ? name.slice(0, dot) : name;
    const ext = dot > 0 ? name.slice(dot) : '';
    for (let n = 2; ; n += 1) {
        const tried = `${base} (${n})${ext}`;
        if (!lower.has(tried.toLowerCase())) {
            return tried;
        }
    }
}

async function copyInto(from: string, to: string): Promise<void> {
    const stat = await fs.stat(from);
    if (!stat.isDirectory()) {
        await fs.copyFile(from, to);
        return;
    }
    await fs.mkdir(to, { recursive: true });
    for (const name of await fs.readdir(from)) {
        await copyInto(join(from, name), join(to, name));
    }
}

/** Copies or moves `paths` into `folder`, giving each a free name. It's the new paths. */
export async function transfer(paths: string[], folder: string, move: boolean): Promise<string[]> {
    const taken = new Set((await fs.readdir(folder)).map(String));
    const made: string[] = [];
    for (const from of paths) {
        const sameFolder = samePath(parentOf(from), folder);
        if (move && sameFolder) {
            continue;
        }
        const name = freeName(taken, nameOf(from));
        taken.add(name);
        const to = join(folder, name);
        if (samePath(folder, from) || isInside(folder, from)) {
            throw new Error(`can't put ${nameOf(from)} inside itself`);
        }
        if (move) {
            try {
                await fs.rename(from, to);
            } catch {
                await copyInto(from, to);
                await fs.rm(from, { recursive: true, force: true });
            }
        } else {
            await copyInto(from, to);
        }
        made.push(to);
    }
    return made;
}
