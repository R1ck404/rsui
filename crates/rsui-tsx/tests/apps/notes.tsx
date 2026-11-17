// A notes app: tabs, a big editor, a live preview of what's written in Markdown, a pane that can be resized by dragging, and a
// menu where the pointer is. Notes are files in a folder, written a moment after the last change.
//
// Run it with `cargo run --release -p rsui-tsx -- crates/rsui-tsx/examples/notes.tsx`.

import {
    Dialog,
    focus,
    TextInput,
    useCallback,
    mainWindow,
    useEffect,
    useMemo,
    useRef,
    useState,
} from 'rsui';
import { clipboard, dialogs, shell } from 'rsui/desktop';
import { existsSync, mkdirSync, promises as fs, readdirSync, readFileSync, writeFileSync } from 'fs';
import * as path from 'path';

export const title = 'Notes';
export const size = [1100, 700];

const INK = '#1f2328';
const MUTED = '#656d76';
const LINE = '#d8dee4';
const PAGE = '#ffffff';
const SIDE = '#f6f8fa';
const CODE = '#eaeef2';
const CHOSEN = '#ddf4ff';
const BLUE = '#0969da';
const ICONS = 'Segoe Fluent Icons, Segoe MDL2 Assets';

function folder(): string {
    return process.env.RSUI_NOTES_FOLDER ?? path.join(process.env.APPDATA ?? '.', 'rsui-notes');
}

interface Note {
    name: string;
    text: string;
}

function loadNotes(): Note[] {
    const where = folder();
    if (!existsSync(where)) {
        mkdirSync(where, { recursive: true });
    }
    const names = readdirSync(where).filter((name) => name.endsWith('.md'));
    if (names.length === 0) {
        return [{ name: 'Welcome', text: '# Welcome\n\nThis is **a note**, in *one* paragraph with `some inline code` and a [link](https://example.com) that wraps along with the rest of it as the pane gets narrower. It saves itself.\n\n- Try the list\n- Drag the line between the panes\n- Right click a note\n\n```\nsome code\n```\n' }];
    }
    return names.map((name) => ({ name: name.slice(0, -3), text: readFileSync(path.join(where, name), 'utf8') }));
}

const fileOf = (name: string) => path.join(folder(), name + '.md');

// What's written as Markdown, as far as a preview needs it. It's all one paragraph, however many looks it has.
function Inline({ text, ...look }: { text: string; [look: string]: unknown }) {
    const pieces = text.split(/(\*\*[^*]+\*\*|\*[^*]+\*|`[^`]+`|\[[^\]]+\]\([^)]+\))/).filter((piece) => piece !== '');
    return (
        <text selectable {...look}>
            {pieces.map((piece, index) => {
                if (piece.startsWith('**')) {
                    return <span key={index} fontWeight="bold">{piece.slice(2, -2)}</span>;
                }
                if (piece.startsWith('*')) {
                    return <span key={index} fontStyle="italic">{piece.slice(1, -1)}</span>;
                }
                if (piece.startsWith('`')) {
                    return <span key={index} fontFamily="Consolas" background={CODE} radius={3} paddingX={2}>{piece.slice(1, -1)}</span>;
                }
                const link = /^\[([^\]]+)\]\(([^)]+)\)$/.exec(piece);
                if (link !== null) {
                    return <link key={index} color={BLUE} underline onClick={() => shell.open(link[2])}>{link[1]}</link>;
                }
                return <span key={index}>{piece}</span>;
            })}
        </text>
    );
}

function Preview({ text }: { text: string }) {
    const blocks = useMemo(() => {
        const found = [];
        let code: string[] | null = null;
        for (const line of text.split('\n')) {
            if (line.startsWith('```')) {
                if (code !== null) {
                    found.push({ kind: 'code', text: code.join('\n') });
                    code = null;
                } else {
                    code = [];
                }
            } else if (code !== null) {
                code.push(line);
            } else if (line.startsWith('#')) {
                const level = line.indexOf(' ');
                found.push({ kind: 'heading', level, text: line.slice(level + 1) });
            } else if (line.startsWith('- ')) {
                found.push({ kind: 'item', text: line.slice(2) });
            } else if (line.trim() !== '') {
                found.push({ kind: 'paragraph', text: line });
            }
        }
        if (code !== null) {
            found.push({ kind: 'code', text: code.join('\n') });
        }
        return found;
    }, [text]);
    return (
        <scroll key="preview" flex={1} padding={18} gap={10} background={PAGE}>
            {blocks.map((block, index) => {
                switch (block.kind) {
                    case 'heading':
                        return <Inline key={index} text={block.text} fontWeight="bold" fontSize={28 - block.level * 4} />;
                    case 'item':
                        return (
                            <row key={index} gap={8}>
                                <text color={MUTED}>{'•'}</text>
                                <Inline text={block.text} flex={1} />
                            </row>
                        );
                    case 'code':
                        return <text key={index} fontFamily="Consolas" padding={10} radius={6} background={SIDE}>{block.text}</text>;
                    default:
                        return <Inline key={index} text={block.text} />;
                }
            })}
        </scroll>
    );
}

// The line between the panes, which moves them when it's dragged.
function Splitter({ onMove }: { onMove: (by: number) => void }) {
    return (
        <group
            key="splitter"
            width={6}
            hover={LINE}
            background={SIDE}
            onDrag={(phase: string, x: number, y: number, dx: number) => {
                if (phase === 'move') {
                    onMove(dx);
                }
            }}
        />
    );
}

function NoteRow({ note, chosen, choose, openMenu }) {
    return (
        <item
            key={'note-' + note.name}
            name={note.name}
            paddingX={10}
            paddingY={7}
            radius={6}
            gap={2}
            background={chosen ? CHOSEN : undefined}
            hover={chosen ? undefined : '#eaeef2'}
            onClick={() => choose(note.name)}
            onContextMenu={(x: number, y: number) => openMenu(note.name, x, y)}
        >
            <text ellipsis>{note.name}</text>
            <text ellipsis color={MUTED} fontSize={12}>{note.text.split('\n').find((line: string) => line.trim() !== '') ?? 'Empty'}</text>
        </item>
    );
}

function Tab({ name, chosen, choose, close }) {
    return (
        <row
            key={'tab-' + name}
            align="center"
            gap={6}
            paddingX={12}
            paddingY={6}
            background={chosen ? PAGE : SIDE}
            hover={chosen ? undefined : '#eaeef2'}
        >
            <button key={'tab-open-' + name} onClick={() => choose(name)} fontWeight={chosen ? 'bold' : undefined}>{name}</button>
            <button key={'tab-close-' + name} name={'Close ' + name} fontFamily={ICONS} fontSize={9} radius={3} paddingX={3} paddingY={3} hover={LINE} onClick={() => close(name)}>{''}</button>
        </row>
    );
}

export default function App() {
    const [notes, setNotes] = useState(loadNotes);
    const [chosen, setChosen] = useState<string>(() => notes[0].name);
    const [tabs, setTabs] = useState<string[]>(() => [notes[0].name]);
    const [filter, setFilter] = useState('');
    const [editorWidth, setEditorWidth] = useState(420);
    const [menu, setMenu] = useState<{ name: string; x: number; y: number } | null>(null);
    const [renaming, setRenaming] = useState<string | null>(null);
    const [newName, setNewName] = useState('');
    const [message, setMessage] = useState('');
    const lastSaved = useRef(new Map(notes.map((note: Note) => [note.name, note.text])));

    // With every tab closed, there's nothing to show.
    const note = tabs.includes(chosen) ? notes.find((item: Note) => item.name === chosen) : undefined;

    useEffect(() => {
        mainWindow.setTitle(chosen + ' - Notes');
    }, [chosen]);

    // Writes what's changed, a moment after the last change.
    useEffect(() => {
        const timer = setTimeout(() => {
            for (const item of notes) {
                if (lastSaved.current.get(item.name) !== item.text) {
                    lastSaved.current.set(item.name, item.text);
                    fs.writeFile(fileOf(item.name), item.text).catch((error: Error) => setMessage(error.message));
                }
            }
        }, 400);
        return () => clearTimeout(timer);
    }, [notes]);

    // Closing the window doesn't wait for that, so anything that hasn't been written by then is written as it goes.
    const latestNotes = useRef(notes);
    latestNotes.current = notes;
    useEffect(() => () => {
        for (const item of latestNotes.current) {
            if (lastSaved.current.get(item.name) !== item.text) {
                writeFileSync(fileOf(item.name), item.text);
            }
        }
    }, []);

    const choose = useCallback((name: string) => {
        setChosen(name);
        setTabs((open: string[]) => (open.includes(name) ? open : [...open, name]));
    }, []);
    const closeTab = useCallback((name: string) => {
        setTabs((open: string[]) => {
            const left = open.filter((tab) => tab !== name);
            setChosen((was: string) => (was === name && left.length > 0 ? left[left.length - 1] : was));
            return left;
        });
    }, []);
    const openMenu = useCallback((name: string, x: number, y: number) => setMenu({ name, x, y }), []);
    const edit = useCallback((text: string) => {
        setNotes((all: Note[]) => all.map((item) => (item.name === chosen ? { ...item, text } : item)));
    }, [chosen]);

    const addNote = () => {
        let number = 1;
        while (notes.some((item: Note) => item.name === 'Note ' + number)) {
            number += 1;
        }
        const name = 'Note ' + number;
        setNotes([...notes, { name, text: '' }]);
        choose(name);
        focus('editor');
    };
    const removeNote = (name: string) => {
        const left = notes.filter((item: Note) => item.name !== name);
        const kept = left.length > 0 ? left : [{ name: 'Note 1', text: '' }];
        setNotes(kept);
        setTabs((open: string[]) => {
            const stillOpen = open.filter((tab) => tab !== name);
            return stillOpen.length > 0 ? stillOpen : [kept[0].name];
        });
        if (chosen === name) {
            setChosen(kept[0].name);
        }
        lastSaved.current.delete(name);
        fs.rm(fileOf(name), { force: true });
        setMessage('Removed ' + name);
    };
    const rename = () => {
        const to = newName.trim();
        if (renaming === null || to === '' || notes.some((item: Note) => item.name === to)) {
            setRenaming(null);
            return;
        }
        const from = renaming;
        setNotes(notes.map((item: Note) => (item.name === from ? { ...item, name: to } : item)));
        setTabs((open: string[]) => open.map((tab) => (tab === from ? to : tab)));
        if (chosen === from) {
            setChosen(to);
        }
        lastSaved.current.delete(from);
        fs.rm(fileOf(from), { force: true });
        setRenaming(null);
    };
    const exportNote = async () => {
        const where = await dialogs.save({ title: 'Save a copy', start: chosen + '.md', filters: [{ name: 'Markdown', extensions: ['md'] }] });
        if (where !== null && note !== undefined) {
            await fs.writeFile(where, note.text);
            setMessage('Saved a copy to ' + where);
        }
    };
    const bringIn = async (where: string) => {
        const text = await fs.readFile(where, 'utf8');
        const name = path.basename(where, path.extname(where));
        setNotes((all: Note[]) => (all.some((item) => item.name === name) ? all : [...all, { name, text }]));
        choose(name);
    };
    const importNote = async () => {
        const where = await dialogs.open({ title: 'Open a note', filters: [{ name: 'Markdown and text', extensions: ['md', 'txt'] }] });
        if (where !== null) {
            await bringIn(where);
        }
    };
    const onDropFiles = (paths: string[]) => {
        for (const where of paths.filter((one) => /\.(md|txt)$/i.test(one))) {
            bringIn(where);
        }
    };

    const onKey = (key: string) => {
        if (key === 'ctrl+n') {
            addNote();
        } else if (key === 'ctrl+f') {
            focus('filter');
        } else if (key === 'ctrl+w') {
            closeTab(chosen);
        } else {
            return false;
        }
        return true;
    };

    const wanted = filter.trim().toLowerCase();
    const listed = notes.filter((item: Note) => wanted === '' || item.name.toLowerCase().includes(wanted) || item.text.toLowerCase().includes(wanted));
    const words = note === undefined ? 0 : note.text.split(/\s+/).filter((word: string) => word !== '').length;

    return (
        <window key="root" direction="row" color={INK} background={PAGE} onKeyPress={onKey} onDropFiles={onDropFiles}>
            <group width={230} padding={10} gap={8} background={SIDE}>
                <row gap={6} align="center">
                    <TextInput key="filter" name="Find a note" placeholder="Find a note" flex={1} value={filter} onChange={setFilter} />
                    <button key="add" name="New note" fontFamily={ICONS} paddingX={8} paddingY={6} radius={6} hover={LINE} onClick={addNote}>{''}</button>
                </row>
                <scroll key="list" flex={1} gap={2}>
                    {listed.map((item: Note) => (
                        <NoteRow key={item.name} note={item} chosen={item.name === chosen} choose={choose} openMenu={openMenu} />
                    ))}
                    {listed.length === 0 ? <text key="none" color={MUTED} padding={10}>Nothing matches that.</text> : null}
                </scroll>
                <row gap={6}>
                    <button key="import" paddingX={8} paddingY={4} radius={6} hover={LINE} onClick={importNote}>Open</button>
                    <button key="export" paddingX={8} paddingY={4} radius={6} hover={LINE} onClick={exportNote}>Save a copy</button>
                </row>
            </group>
            <group flex={1}>
                <row key="tabs" background={SIDE} gap={1}>
                    {tabs.map((tab: string) => (
                        <Tab key={tab} name={tab} chosen={tab === chosen} choose={choose} close={closeTab} />
                    ))}
                </row>
                {note === undefined ? (
                    <text key="nothing" color={MUTED} padding={24}>Choose a note on the left, or make a new one.</text>
                ) : (
                    <row key="panes" flex={1}>
                        <group width={editorWidth} padding={10}>
                            <TextInput key="editor" name="What's in the note" rows={6} flex={1} value={note.text} onChange={edit} fontFamily="Consolas" />
                        </group>
                        <Splitter onMove={(by: number) => setEditorWidth((width: number) => Math.max(160, Math.min(900, width + by)))} />
                        <Preview text={note.text} />
                    </row>
                )}
                <row paddingX={12} paddingY={5} justify="space_between" background={SIDE}>
                    <text key="words" color={MUTED}>{words} {words === 1 ? 'word' : 'words'}</text>
                    <text key="message" color={MUTED} ellipsis>{message}</text>
                </row>
            </group>
            {menu !== null ? (
                <menu
                    key="note-menu"
                    overlay="at"
                    overlayX={menu.x}
                    overlayY={menu.y}
                    padding={4}
                    minWidth={160}
                    background="#ffffff"
                    borderColor="#d0d7de"
                    borderWidth={1}
                    radius={8}
                    shadowBlur={14}
                    shadowY={4}
                    shadowColor="#00000026"
                    onDismiss={() => setMenu(null)}
                >
                    <menuitem key="menu-rename" paddingX={10} paddingY={6} radius={4} hover="#eaeef2" onClick={() => { setRenaming(menu.name); setNewName(menu.name); setMenu(null); }}>Rename</menuitem>
                    <menuitem key="menu-copy" paddingX={10} paddingY={6} radius={4} hover="#eaeef2" onClick={() => { clipboard.write({ text: notes.find((item: Note) => item.name === menu.name)?.text ?? '' }); setMenu(null); }}>Copy the text</menuitem>
                    <menuitem key="menu-remove" paddingX={10} paddingY={6} radius={4} hover="#ffebe9" color="#cf222e" onClick={() => { removeNote(menu.name); setMenu(null); }}>Remove</menuitem>
                </menu>
            ) : null}
            <Dialog id="rename" name="Rename a note" open={renaming !== null} onClose={() => setRenaming(null)}>
                <group gap={12} onKeyPress={(key: string) => { if (key === 'enter') { rename(); return true; } return false; }}>
                    <text fontWeight="bold">What should it be called?</text>
                    <TextInput autoFocus key="rename-to" name="New name" width={280} value={newName} onChange={setNewName} />
                    <row gap={8}>
                        <button key="rename-do" paddingX={12} paddingY={6} radius={6} background="#1f6feb" color="#ffffff" onClick={rename}>Rename</button>
                        <button key="rename-cancel" paddingX={12} paddingY={6} radius={6} background={LINE} onClick={() => setRenaming(null)}>Cancel</button>
                    </row>
                </group>
            </Dialog>
        </window>
    );
}
