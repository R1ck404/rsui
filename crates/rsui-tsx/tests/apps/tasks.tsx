// A small app to keep track of what's to be done, the way a real one would be written. It saves as you go.
//
// Run it with `cargo run --release -p rsui-tsx -- crates/rsui-tsx/examples/tasks.tsx`.

import {
    Checkbox,
    Dialog,
    focus,
    MenuButton,
    Select,
    TextInput,
    useCallback,
    useEffect,
    useMemo,
    useReducer,
    useRef,
    useState,
} from 'rsui';
import * as fs from 'fs';
import * as path from 'path';

export const title = 'Tasks';

const INK = '#1f2328';
const MUTED = '#656d76';
const BLUE = '#1f6feb';
const LINE = '#d8dee4';
const PAGE = '#f6f8fa';
const CARD = '#ffffff';
const CHOSEN = '#ddf4ff';
const RED = '#cf222e';

interface Task {
    id: number;
    text: string;
    done: boolean;
    project: string;
    notes: string;
    added: number;
}

interface Data {
    tasks: Task[];
    projects: string[];
    nextId: number;
}

const FIRST_TIME: Data = {
    tasks: [
        { id: 1, text: 'Try adding a task', done: false, project: 'Inbox', notes: '', added: 1 },
        { id: 2, text: 'Tick this one off', done: false, project: 'Inbox', notes: 'Notes go here.', added: 2 },
        { id: 3, text: 'Plan the week', done: false, project: 'Home', notes: '', added: 3 },
    ],
    projects: ['Inbox', 'Home', 'Work'],
    nextId: 4,
};

// Where it all goes. Something asked for by name wins, which is how the tests keep theirs apart.
function dataFile(): string {
    const named = process.env.RSUI_TASKS_FILE;
    if (named) {
        return named;
    }
    const home = process.env.APPDATA ?? process.env.HOME ?? '.';
    return path.join(home, 'rsui-tasks', 'tasks.json');
}

function load(): Data {
    try {
        const read = JSON.parse(fs.readFileSync(dataFile(), 'utf8'));
        if (Array.isArray(read.tasks) && Array.isArray(read.projects)) {
            return read;
        }
    } catch (error) {
        // Not there yet, or not readable, and either way there's nothing to carry on from.
    }
    return FIRST_TIME;
}

// Written to another file first, so that stopping halfway can't lose what's there.
function save(data: Data) {
    const file = dataFile();
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file + '.new', JSON.stringify(data));
    fs.renameSync(file + '.new', file);
}

type Action =
    | { kind: 'add'; text: string; project: string }
    | { kind: 'toggle'; id: number }
    | { kind: 'edit'; id: number; text?: string; notes?: string }
    | { kind: 'remove'; id: number }
    | { kind: 'restore'; task: Task }
    | { kind: 'clearDone'; project: string | null }
    | { kind: 'addProject'; name: string }
    | { kind: 'removeProject'; name: string };

function reduce(data: Data, action: Action): Data {
    switch (action.kind) {
        case 'add': {
            const task = {
                id: data.nextId,
                text: action.text,
                done: false,
                project: action.project,
                notes: '',
                added: data.nextId,
            };
            return { ...data, tasks: [...data.tasks, task], nextId: data.nextId + 1 };
        }
        case 'toggle':
            return {
                ...data,
                tasks: data.tasks.map((task) => (task.id === action.id ? { ...task, done: !task.done } : task)),
            };
        case 'edit':
            return {
                ...data,
                tasks: data.tasks.map((task) =>
                    task.id === action.id
                        ? { ...task, text: action.text ?? task.text, notes: action.notes ?? task.notes }
                        : task,
                ),
            };
        case 'remove':
            return { ...data, tasks: data.tasks.filter((task) => task.id !== action.id) };
        case 'restore':
            return { ...data, tasks: [...data.tasks, action.task] };
        case 'clearDone':
            return {
                ...data,
                tasks: data.tasks.filter(
                    (task) => !task.done || (action.project !== null && task.project !== action.project),
                ),
            };
        case 'addProject':
            return data.projects.includes(action.name)
                ? data
                : { ...data, projects: [...data.projects, action.name] };
        case 'removeProject':
            return {
                ...data,
                projects: data.projects.filter((name) => name !== action.name),
                tasks: data.tasks.map((task) => (task.project === action.name ? { ...task, project: 'Inbox' } : task)),
            };
    }
}

const SORTS = ['Oldest first', 'Newest first', 'By name', 'Not done first'];

function sorted(tasks: Task[], sort: number): Task[] {
    const copy = [...tasks];
    switch (sort) {
        case 1:
            return copy.sort((a, b) => b.added - a.added);
        case 2:
            return copy.sort((a, b) => (a.text < b.text ? -1 : a.text > b.text ? 1 : 0));
        case 3:
            return copy.sort((a, b) => Number(a.done) - Number(b.done) || a.added - b.added);
        default:
            return copy.sort((a, b) => a.added - b.added);
    }
}

function Button({ primary = false, children, ...rest }) {
    const paint = primary
        ? { background: BLUE, hover: '#388bfd', pressed: '#1558c0', color: '#ffffff' }
        : { background: LINE, hover: '#c9d1d9', pressed: '#afb8c1', color: INK };
    return <button paddingX={12} paddingY={6} radius={6} {...paint} {...rest}>{children}</button>;
}

function ProjectRow({ name, count, chosen, onChoose }) {
    return (
        <item
            key={'project-' + name}
            direction="row"
            justify="space_between"
            paddingX={10}
            paddingY={6}
            radius={6}
            background={chosen ? CHOSEN : undefined}
            hover={chosen ? undefined : '#eaeef2'}
            onClick={onChoose}
        >
            <text>{name}</text>
            <text color={MUTED}>{count}</text>
        </item>
    );
}

function Sidebar({ data, project, setProject, dispatch }) {
    const [adding, setAdding] = useState('');
    const open = (name: string | null) => data.tasks.filter((task) => !task.done && (name === null || task.project === name)).length;
    const add = () => {
        const name = adding.trim();
        if (name !== '') {
            dispatch({ kind: 'addProject', name });
            setProject(name);
            setAdding('');
        }
    };
    return (
        <group width={200} padding={12} gap={4} background={CARD}>
            <text color={MUTED} fontWeight="bold" paddingX={10} paddingY={4}>Projects</text>
            <ProjectRow name="Everything" count={open(null)} chosen={project === null} onChoose={() => setProject(null)} />
            {data.projects.map((name: string) => (
                <ProjectRow key={name} name={name} count={open(name)} chosen={project === name} onChoose={() => setProject(name)} />
            ))}
            <group flex={1} />
            <TextInput
                key="new-project"
                name="New project"
                placeholder="New project"
                value={adding}
                onChange={setAdding}
                onKeyPress={(key: string) => key === 'enter' && add()}
            />
        </group>
    );
}

// What's given to this stays the same unless something about this row changes, as `dispatch`, `setChosen` and `onRemove`
// never do. So a row that hasn't changed doesn't render again, however many of them there are. Making functions here in
// the parent, as `onClick={() => ...}`, would be new ones each time, and so all of them would.
function TaskRow({ task, chosen, setChosen, dispatch, onRemove, showProject }) {
    return (
        <item
            key={'task-' + task.id}
            direction="row"
            align="center"
            gap={10}
            paddingX={12}
            paddingY={8}
            radius={6}
            background={chosen ? CHOSEN : CARD}
            hover={chosen ? undefined : '#f3f4f6'}
            onClick={() => setChosen(task.id)}
        >
            <Checkbox
                id={'done-' + task.id}
                label={task.text}
                showLabel={false}
                checked={task.done}
                onChange={() => dispatch({ kind: 'toggle', id: task.id })}
            />
            <text flex={1} ellipsis color={task.done ? MUTED : INK}>{task.text}</text>
            {showProject ? <text color={MUTED}>{task.project}</text> : null}
            <button key={'remove-' + task.id} name={'Remove ' + task.text} paddingX={8} radius={4} color={RED} hover="#ffebe9" onClick={() => onRemove(task)}>
                Remove
            </button>
        </item>
    );
}

// What's being typed is kept here, so that each key only renders this, and not the whole list.
function AddTask({ project, dispatch }) {
    const [draft, setDraft] = useState('');
    const add = () => {
        const text = draft.trim();
        if (text !== '') {
            dispatch({ kind: 'add', text, project: project ?? 'Inbox' });
            setDraft('');
        }
    };
    return (
        <row gap={8} onKeyPress={(key: string) => key === 'enter' && add()}>
            <TextInput key="draft" name="New task" placeholder="What needs doing?" flex={1} value={draft} onChange={setDraft} />
            <Button primary key="add" onClick={add}>Add</Button>
        </row>
    );
}

function Details({ task, dispatch }) {
    return (
        <group width={260} padding={16} gap={10} background={CARD}>
            <text color={MUTED} fontWeight="bold">Details</text>
            {task === undefined ? (
                <text color={MUTED}>Choose a task to see more about it.</text>
            ) : (
                <group gap={10}>
                    <TextInput
                        key={'title-' + task.id}
                        name="What's to be done"
                        value={task.text}
                        onChange={(text: string) => dispatch({ kind: 'edit', id: task.id, text })}
                    />
                    <TextInput
                        key={'notes-' + task.id}
                        name="Notes"
                        placeholder="Notes"
                        rows={6}
                        value={task.notes}
                        onChange={(notes: string) => dispatch({ kind: 'edit', id: task.id, notes })}
                    />
                    <text color={MUTED}>In {task.project}</text>
                </group>
            )}
        </group>
    );
}

export default function App() {
    const [data, dispatch] = useReducer(reduce, null, load);
    const [project, setProject] = useState<string | null>(null);
    const [search, setSearch] = useState('');
    const [sort, setSort] = useState(0);
    const [chosen, setChosen] = useState<number | null>(null);
    const [confirming, setConfirming] = useState(false);
    const [removed, setRemoved] = useState<Task | null>(null);
    const [problem, setProblem] = useState<string | null>(null);
    const firstRun = useRef(true);

    // Saves a moment after the last change, so that typing doesn't write the file on every key. Closing the window
    // doesn't wait for that, so anything that hasn't been written by then is written as it goes.
    const unsaved = useRef<Data | null>(null);
    useEffect(() => {
        if (firstRun.current) {
            firstRun.current = false;
            return;
        }
        unsaved.current = data;
        const timer = setTimeout(() => {
            unsaved.current = null;
            try {
                save(data);
                setProblem(null);
            } catch (error) {
                setProblem('Couldn\'t save: ' + error.message);
            }
        }, 300);
        return () => clearTimeout(timer);
    }, [data]);
    useEffect(() => () => {
        if (unsaved.current !== null) {
            save(unsaved.current);
        }
    }, []);

    // The chance to undo goes away by itself.
    useEffect(() => {
        if (removed === null) {
            return;
        }
        const timer = setTimeout(() => setRemoved(null), 6000);
        return () => clearTimeout(timer);
    }, [removed]);

    const shown = useMemo(() => {
        const wanted = search.trim().toLowerCase();
        const matching = data.tasks.filter(
            (task: Task) =>
                (project === null || task.project === project) &&
                (wanted === '' || task.text.toLowerCase().includes(wanted) || task.notes.toLowerCase().includes(wanted)),
        );
        return sorted(matching, sort);
    }, [data.tasks, project, search, sort]);

    const remove = useCallback((task: Task) => {
        dispatch({ kind: 'remove', id: task.id });
        setRemoved(task);
        setChosen((was: number | null) => (was === task.id ? null : was));
    }, []);
    const inView = data.tasks.filter((task: Task) => project === null || task.project === project);
    const left = inView.filter((task: Task) => !task.done).length;
    const finished = inView.length - left;
    const canRemoveProject = project !== null && project !== 'Inbox';

    return (
        <window
            direction="row"
            background={PAGE}
            color={INK}
            onKeyPress={(key: string) => {
                if (key === 'ctrl+n') {
                    focus('draft');
                } else if (key === 'ctrl+f') {
                    focus('search');
                }
            }}
        >
            <Sidebar data={data} project={project} setProject={setProject} dispatch={dispatch} />
            <group flex={1} padding={16} gap={12}>
                <row gap={8} align="center">
                    <text flex={1} fontSize={22} fontWeight="bold">{project ?? 'Everything'}</text>
                    <TextInput key="search" name="Search" placeholder="Search" width={180} value={search} onChange={setSearch} />
                    <Select id="sort" options={SORTS} value={sort} onChange={setSort} />
                    {canRemoveProject ? (
                        <MenuButton
                            id="project-menu"
                            label="More"
                            items={[{
                                label: 'Remove this project',
                                onSelect: () => {
                                    dispatch({ kind: 'removeProject', name: project });
                                    setProject(null);
                                },
                            }]}
                        />
                    ) : null}
                </row>
                <AddTask project={project} dispatch={dispatch} />
                <scroll key="tasks" flex={1} gap={4}>
                    {shown.map((task: Task) => (
                        <TaskRow
                            key={task.id}
                            task={task}
                            chosen={chosen === task.id}
                            setChosen={setChosen}
                            dispatch={dispatch}
                            onRemove={remove}
                            showProject={project === null}
                        />
                    ))}
                    {shown.length === 0 ? (
                        <text key="empty" color={MUTED} padding={16}>
                            {search === '' ? 'Nothing here yet.' : 'Nothing matches that.'}
                        </text>
                    ) : null}
                </scroll>
                <row gap={8} align="center">
                    <text key="count" flex={1} color={MUTED}>{left} left, {finished} done</text>
                    {problem !== null ? <text key="problem" color={RED}>{problem}</text> : null}
                    {removed !== null ? (
                        <row gap={8} align="center">
                            <text color={MUTED}>Removed "{removed.text}"</text>
                            <Button
                                key="undo"
                                onClick={() => {
                                    dispatch({ kind: 'restore', task: removed });
                                    setRemoved(null);
                                }}
                            >
                                Undo
                            </Button>
                        </row>
                    ) : null}
                    {finished > 0 ? <Button key="clear" onClick={() => setConfirming(true)}>Clear done</Button> : null}
                </row>
            </group>
            <Details task={data.tasks.find((task: Task) => task.id === chosen)} dispatch={dispatch} />
            <Dialog id="confirm" name="Clear what's done" open={confirming} onClose={() => setConfirming(false)}>
                <group gap={12} align="start">
                    <text>Remove {finished} finished {finished === 1 ? 'task' : 'tasks'}? This can't be undone.</text>
                    <row gap={8}>
                        <Button
                            primary
                            key="confirm-clear"
                            onClick={() => {
                                dispatch({ kind: 'clearDone', project });
                                setConfirming(false);
                            }}
                        >
                            Remove them
                        </Button>
                        <Button key="confirm-cancel" onClick={() => setConfirming(false)}>Keep them</Button>
                    </row>
                </group>
            </Dialog>
        </window>
    );
}
