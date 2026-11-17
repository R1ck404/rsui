// A small file browser: long folders, the keyboard, things loading in the background, and pictures and text to look at.
//
// Run it with `cargo run --release -p rsui-tsx -- crates/rsui-tsx/examples/files.tsx`.
// `RSUI_FILES_START` says where to start, which is your home folder otherwise.

import { focus, Select, TextInput, useCallback, useEffect, useMemo, useRef, useState } from 'rsui';
import { promises as fs } from 'fs';
import * as path from 'path';

export const title = 'Files';

const INK = '#1f2328';
const MUTED = '#656d76';
const LINE = '#d8dee4';
const PAGE = '#f6f8fa';
const CARD = '#ffffff';
const CHOSEN = '#ddf4ff';
const RED = '#cf222e';
const ROW = 28;
const IMAGES = ['.png', '.jpg', '.jpeg', '.webp'];
const TEXTS = ['.txt', '.md', '.json', '.js', '.ts', '.tsx', '.rs', '.toml', '.py', '.css', '.html', '.log', '.yml', '.yaml', '.ini', '.xml', '.csv'];
const MOST_TEXT = 200 * 1024;

interface Entry {
    name: string;
    folder: boolean;
    size: number;
    modified: number;
}

function startFolder(): string {
    return process.env.RSUI_FILES_START ?? process.env.USERPROFILE ?? process.env.HOME ?? process.cwd();
}

function kindOf(entry: Entry): string {
    if (entry.folder) {
        return 'Folder';
    }
    const extension = path.extname(entry.name).toLowerCase();
    if (IMAGES.includes(extension)) {
        return 'Picture';
    }
    if (TEXTS.includes(extension)) {
        return 'Text';
    }
    return extension === '' ? 'File' : extension.slice(1).toUpperCase();
}

function sizeOf(bytes: number): string {
    if (bytes < 1024) {
        return bytes + ' B';
    }
    const units = ['KB', 'MB', 'GB', 'TB'];
    let size = bytes;
    let unit = -1;
    do {
        size /= 1024;
        unit += 1;
    } while (size >= 1024 && unit < units.length - 1);
    return size.toFixed(size < 10 ? 1 : 0) + ' ' + units[unit];
}

function dateOf(time: number): string {
    return new Date(time).toLocaleString(undefined, {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
        hour: 'numeric',
        minute: '2-digit',
        hour12: false,
    });
}

// Reads a folder on other threads, so that a slow one or a large one doesn't hold anything up. A file that goes away while
// this happens, or can't be read, just isn't listed.
async function readFolder(folder: string): Promise<Entry[]> {
    const names = await fs.readdir(folder);
    const found = await Promise.all(
        names.map(async (name: string) => {
            try {
                const stat = await fs.stat(path.join(folder, name));
                return { name, folder: stat.isDirectory(), size: stat.size, modified: stat.mtimeMs };
            } catch (error) {
                return null;
            }
        }),
    );
    return found.filter((entry) => entry !== null) as Entry[];
}

const SORTS = ['By name', 'Newest first', 'Largest first'];

function sortEntries(entries: Entry[], sort: number): Entry[] {
    const by = [
        (a: Entry, b: Entry) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()),
        (a: Entry, b: Entry) => b.modified - a.modified,
        (a: Entry, b: Entry) => b.size - a.size,
    ][sort];
    return [...entries].sort((a, b) => Number(b.folder) - Number(a.folder) || by(a, b));
}

// Windows comes with a font of these, and which one depends on its version.
const ICONS = 'Segoe Fluent Icons, Segoe MDL2 Assets';
const FOLDER = '\uE8B7';
const DOCUMENT = '\uE8A5';
const PICTURE = '\uEB9F';
const BACK = '\uE72B';
const UP = '\uE74A';

function Icon({ glyph, color }: { glyph: string; color?: string }) {
    return <text fontFamily={ICONS} fontSize={14} color={color}>{glyph}</text>;
}

function Crumbs({ folder, go }) {
    const parts = useMemo(() => {
        const pieces = [];
        let at = folder;
        for (;;) {
            pieces.unshift(at);
            const up = path.dirname(at);
            if (up === at) {
                return pieces;
            }
            at = up;
        }
    }, [folder]);
    return (
        <row gap={2} align="center">
            {parts.map((part: string, index: number) => (
                <button
                    key={'crumb-' + index}
                    paddingX={6}
                    paddingY={3}
                    radius={4}
                    hover="#eaeef2"
                    fontWeight={index === parts.length - 1 ? 'bold' : undefined}
                    onClick={() => go(part)}
                >
                    {path.basename(part) || part}
                </button>
            ))}
        </row>
    );
}

function Row({ entry, index, chosen, choose, open }) {
    return (
        <item
            key={'row-' + index}
            name={entry.name}
            direction="row"
            align="center"
            gap={10}
            paddingX={10}
            height={ROW}
            background={chosen ? CHOSEN : undefined}
            hover={chosen ? undefined : '#f3f4f6'}
            onClick={() => choose(index)}
            onDoubleClick={() => open(entry)}
        >
            <Icon glyph={entry.folder ? FOLDER : kindOf(entry) === 'Picture' ? PICTURE : DOCUMENT} color={entry.folder ? '#0969da' : MUTED} />
            <text flex={1} ellipsis color={entry.folder ? '#0969da' : INK}>{entry.name}</text>
            <text color={MUTED} width={70} ellipsis>{kindOf(entry)}</text>
            <text color={MUTED} width={70}>{entry.folder ? '' : sizeOf(entry.size)}</text>
            <text color={MUTED} width={150}>{dateOf(entry.modified)}</text>
        </item>
    );
}

// Shows what can be shown of what's chosen, once it's been read.
function Preview({ folder, entry }) {
    const [text, setText] = useState<string | null>(null);
    const [problem, setProblem] = useState<string | null>(null);
    const kind = entry === undefined ? null : kindOf(entry);
    const full = entry === undefined ? '' : path.join(folder, entry.name);
    useEffect(() => {
        setText(null);
        setProblem(null);
        if (kind !== 'Text' || entry.size > MOST_TEXT) {
            return;
        }
        let wanted = true;
        fs.readFile(full, 'utf8').then(
            (read: string) => wanted && setText(read),
            (error: Error) => wanted && setProblem(error.message),
        );
        // By the time this finishes, something else may have been chosen.
        return () => {
            wanted = false;
        };
    }, [full]);
    if (entry === undefined) {
        return <text color={MUTED}>Choose something to look at it.</text>;
    }
    return (
        <group flex={1} gap={8}>
            <text fontWeight="bold" ellipsis>{entry.name}</text>
            <text color={MUTED}>{kind}{entry.folder ? '' : ', ' + sizeOf(entry.size)}</text>
            <text color={MUTED}>{dateOf(entry.modified)}</text>
            {problem !== null ? <text color={RED}>{problem}</text> : null}
            {kind === 'Picture' ? <image key="picture" src={full} name={entry.name} flex={1} radius={6} /> : null}
            {kind === 'Text' && entry.size > MOST_TEXT ? <text color={MUTED}>Too long to show here.</text> : null}
            {text !== null ? <TextInput key="text" name="What's in it" rows={20} flex={1} value={text} inert /> : null}
        </group>
    );
}

export default function App() {
    const [folder, setFolder] = useState(startFolder);
    const [entries, setEntries] = useState<Entry[] | null>(null);
    const [problem, setProblem] = useState<string | null>(null);
    const [filter, setFilter] = useState('');
    const [sort, setSort] = useState(0);
    const [chosen, setChosen] = useState(0);
    const [back, setBack] = useState<string[]>([]);
    const reading = useRef(0);

    useEffect(() => {
        // Only the last folder asked for counts, however long the others take.
        const mine = ++reading.current;
        setEntries(null);
        setProblem(null);
        setChosen(0);
        readFolder(folder).then(
            (found: Entry[]) => mine === reading.current && setEntries(found),
            (error: Error) => {
                if (mine === reading.current) {
                    setEntries([]);
                    setProblem(error.message);
                }
            },
        );
    }, [folder]);

    const shown = useMemo(() => {
        const wanted = filter.trim().toLowerCase();
        const matching = (entries ?? []).filter((entry) => wanted === '' || entry.name.toLowerCase().includes(wanted));
        return sortEntries(matching, sort);
    }, [entries, filter, sort]);

    const go = useCallback((next: string) => {
        setFolder((was: string) => {
            if (was !== next) {
                setBack((old: string[]) => [...old, was]);
            }
            return next;
        });
        setFilter('');
    }, []);
    const open = useCallback((entry: Entry) => {
        if (entry.folder) {
            setFolder((was: string) => {
                setBack((old: string[]) => [...old, was]);
                return path.join(was, entry.name);
            });
            setFilter('');
        }
    }, []);
    const goBack = () => {
        if (back.length > 0) {
            setFolder(back[back.length - 1]);
            setBack(back.slice(0, -1));
            setFilter('');
        }
    };
    const up = () => go(path.dirname(folder));

    const onKey = (key: string) => {
        const last = shown.length - 1;
        const move: Record<string, number> = { down: chosen + 1, up: chosen - 1, pagedown: chosen + 20, pageup: chosen - 20, home: 0, end: last };
        if (key in move) {
            setChosen(Math.max(0, Math.min(last, move[key])));
            return true;
        }
        if (key === 'enter' && shown[chosen] !== undefined) {
            open(shown[chosen]);
            return true;
        }
        if (key === 'backspace' || key === 'alt+left') {
            goBack();
            return true;
        }
        if (key === 'alt+up') {
            up();
            return true;
        }
        if (key === 'ctrl+f') {
            focus('filter');
            return true;
        }
        return false;
    };

    // `list` asks for only what's in view, whatever the size of the folder.
    const rowFor = useCallback(
        (index: number) => (shown[index] === undefined ? null : (
            <Row key={shown[index].name} entry={shown[index]} index={index} chosen={index === chosen} choose={setChosen} open={open} />
        )),
        [shown, chosen, open],
    );

    return (
        <window key="root" background={PAGE} color={INK} onKeyPress={onKey}>
            <row gap={8} align="center" padding={10} background={CARD}>
                <button key="back" name="Back" direction="row" gap={6} align="center" paddingX={10} paddingY={5} radius={6} background={LINE} opacity={back.length > 0 ? undefined : 0.5} onClick={goBack}>
                    <Icon glyph={BACK} />
                    <text>Back</text>
                </button>
                <button key="up" name="Up" direction="row" gap={6} align="center" paddingX={10} paddingY={5} radius={6} background={LINE} onClick={up}>
                    <Icon glyph={UP} />
                    <text>Up</text>
                </button>
                <group flex={1}>
                    <Crumbs folder={folder} go={go} />
                </group>
                <TextInput key="filter" name="Filter" placeholder="Filter" width={180} value={filter} onChange={(text: string) => { setFilter(text); setChosen(0); }} />
                <Select id="sort" options={SORTS} value={sort} onChange={(index: number) => { setSort(index); setChosen(0); }} />
            </row>
            <row flex={1}>
                <group flex={1} background={CARD}>
                    {problem !== null ? <text key="problem" color={RED} padding={12}>{problem}</text> : null}
                    {entries === null ? <text key="loading" color={MUTED} padding={12}>Reading this folder.</text> : null}
                    {entries !== null && shown.length === 0 && problem === null ? (
                        <text key="empty" color={MUTED} padding={12}>{filter === '' ? 'Nothing here.' : 'Nothing matches that.'}</text>
                    ) : null}
                    <list
                        key="rows"
                        flex={1}
                        count={shown.length}
                        visible={40}
                        rowHeight={ROW}
                        reveal={chosen}
                        renderRow={rowFor}
                    />
                </group>
                <group width={320} padding={14} background={PAGE}>
                    <Preview folder={folder} entry={shown[chosen]} />
                </group>
            </row>
            <row paddingX={12} paddingY={6} background={CARD} justify="space_between">
                <text key="count" color={MUTED}>{shown.length} of {entries?.length ?? 0} items</text>
                <text color={MUTED}>Arrows move, enter or double click opens, backspace goes back</text>
            </row>
        </window>
    );
}
