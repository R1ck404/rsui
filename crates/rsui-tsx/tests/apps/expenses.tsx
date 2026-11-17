// An expense tracker: a form, charts made of shapes, a sortable table of however many rows, files of comma-separated
// values going in and out, and colors that follow the system. It saves as it goes.
//
// Run it with `cargo run --release -p rsui-tsx -- crates/rsui-tsx/examples/expenses.tsx`.

import {
    Circle,
    createContext,
    G,
    Path,
    Polyline,
    Rect,
    Select,
    Svg,
    TextInput,
    useCallback,
    useColorScheme,
    useContext,
    useEffect,
    useMemo,
    useRef,
    useState,
} from 'rsui';
import { dialogs } from 'rsui/desktop';
import { existsSync, mkdirSync, promises as fs, readFileSync, renameSync, writeFileSync } from 'fs';
import * as path from 'path';

export const title = 'Expenses';
export const size = [1120, 720];

interface Expense {
    id: number;
    date: string;
    description: string;
    category: string;
    amount: number;
}

const CATEGORIES = ['Food', 'Home', 'Travel', 'Fun', 'Other'];
const CHART = ['#2f81f7', '#3fb950', '#d29922', '#db61a2', '#8b949e'];
const LIGHT = { page: '#ffffff', side: '#f6f8fa', line: '#d8dee4', ink: '#1f2328', muted: '#656d76', hover: '#eaeef2', chosen: '#ddf4ff', bad: '#cf222e', accent: '#0969da' };
const DARK = { page: '#0d1117', side: '#161b22', line: '#30363d', ink: '#e6edf3', muted: '#8b949e', hover: '#21262d', chosen: '#1f3a5f', bad: '#f85149', accent: '#2f81f7' };
const Theme = createContext(LIGHT);
const ROW = 34;

const file = () => process.env.RSUI_EXPENSES_FILE ?? path.join(process.env.APPDATA ?? '.', 'rsui-expenses', 'expenses.json');

function firstTime(): Expense[] {
    const today = new Date();
    const made: Expense[] = [];
    const words = [
        ['Food', 'Groceries', 62.4],
        ['Home', 'Rent', 780],
        ['Travel', 'Train tickets', 34.5],
        ['Fun', 'Concert', 45],
        ['Food', 'Lunch with friends', 18.75],
        ['Other', 'Haircut', 22],
    ] as const;
    for (let month = 0; month < 6; month += 1) {
        words.forEach(([category, description, amount], index) => {
            const date = new Date(today.getFullYear(), today.getMonth() - month, 3 + index * 4);
            made.push({ id: made.length + 1, date: dayOf(date), description, category, amount: Math.round(amount * (1 + ((month * 7 + index * 3) % 5) / 20) * 100) / 100 });
        });
    }
    return made;
}

const dayOf = (date: Date) => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
const money = (amount: number) => amount.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const monthName = (month: string) => new Date(month + '-01T12:00:00').toLocaleDateString('en-US', { month: 'short' });

function load(): Expense[] {
    if (!existsSync(file())) {
        return firstTime();
    }
    try {
        return JSON.parse(readFileSync(file(), 'utf8'));
    } catch {
        return [];
    }
}

// Comma-separated values, where a value can have commas, line breaks and quotes in it if it's in quotes.
function readCsv(text: string): string[][] {
    const rows: string[][] = [];
    let row: string[] = [];
    let value = '';
    let quoted = false;
    for (let at = 0; at < text.length; at += 1) {
        const c = text[at];
        if (quoted) {
            if (c === '"' && text[at + 1] === '"') {
                value += '"';
                at += 1;
            } else if (c === '"') {
                quoted = false;
            } else {
                value += c;
            }
        } else if (c === '"') {
            quoted = true;
        } else if (c === ',') {
            row.push(value);
            value = '';
        } else if (c === '\n') {
            row.push(value);
            rows.push(row);
            row = [];
            value = '';
        } else if (c !== '\r') {
            value += c;
        }
    }
    if (value !== '' || row.length > 0) {
        row.push(value);
        rows.push(row);
    }
    return rows;
}

const csvValue = (value: string) => (/[",\n]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value);
const writeCsv = (all: Expense[]) => ['date,description,category,amount', ...all.map((one) => [one.date, csvValue(one.description), csvValue(one.category), one.amount.toFixed(2)].join(','))].join('\n') + '\n';

function expensesFromCsv(text: string, firstId: number): Expense[] {
    const rows = readCsv(text);
    const made: Expense[] = [];
    for (const row of rows.slice(1)) {
        const amount = Number(row[3]);
        if (row.length >= 4 && /^\d{4}-\d{2}-\d{2}$/.test(row[0]) && Number.isFinite(amount)) {
            made.push({ id: firstId + made.length, date: row[0], description: row[1], category: CATEGORIES.includes(row[2]) ? row[2] : 'Other', amount });
        }
    }
    return made;
}

function Icon({ d, color, size = 16 }: { d: string; color?: string; size?: number }) {
    return (
        <Svg viewBox="0 0 24 24" width={size} height={size} color={color} fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
            <Path d={d} />
        </Svg>
    );
}

const TRASH = 'M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13';
const UP = 'M6 14l6-6 6 6';
const DOWN = 'M6 10l6 6 6-6';

// A slice of a ring, from one share of the way around to another, starting from the top.
function slicePath(from: number, to: number, outer: number, inner: number, middle: number): string {
    const at = (share: number, radius: number) => {
        const angle = (share - 0.25) * 2 * Math.PI;
        return `${middle + radius * Math.cos(angle)} ${middle + radius * Math.sin(angle)}`;
    };
    // The whole ring can't be one arc, as both its ends would be in one place.
    const end = Math.min(to, from + 0.99999);
    const large = end - from > 0.5 ? 1 : 0;
    return `M${at(from, outer)}A${outer} ${outer} 0 ${large} 1 ${at(end, outer)}L${at(end, inner)}A${inner} ${inner} 0 ${large} 0 ${at(from, inner)}Z`;
}

function Donut({ shares, total, chosen, onChoose }: { shares: { category: string; amount: number }[]; total: number; chosen: string | null; onChoose: (category: string) => void }) {
    const theme = useContext(Theme);
    let from = 0;
    return (
        <row gap={16} align="center">
            <Svg key="donut" name="Spending by category" viewBox="0 0 120 120" width={120} height={120}>
                {total === 0 ? <Circle cx={60} cy={60} r={50} fill="none" stroke={theme.line} strokeWidth={16} /> : null}
                {shares.map((share, index) => {
                    const to = from + share.amount / total;
                    const d = slicePath(from, to, 58, chosen === share.category ? 34 : 42, 60);
                    from = to;
                    return <Path key={share.category} d={d} fill={CHART[CATEGORIES.indexOf(share.category)] ?? CHART[index]} />;
                })}
            </Svg>
            <column gap={4}>
                {shares.map((share) => (
                    <row key={share.category} gap={8} align="center" padding={3} radius={4} hover={theme.hover} background={chosen === share.category ? theme.chosen : undefined} onClick={() => onChoose(share.category)}>
                        <Svg viewBox="0 0 10 10" width={10} height={10}>
                            <Rect width={10} height={10} rx={2} fill={CHART[CATEGORIES.indexOf(share.category)]} />
                        </Svg>
                        <text key={'legend-' + share.category} width={60}>{share.category}</text>
                        <text color={theme.muted} width={80} textAlign="right">{money(share.amount)}</text>
                    </row>
                ))}
            </column>
        </row>
    );
}

function Months({ months }: { months: { month: string; amount: number }[] }) {
    const theme = useContext(Theme);
    const most = Math.max(1, ...months.map((one) => one.amount));
    return (
        <row gap={8} height={140} align="end" flex={1}>
            {months.map((one) => (
                <column key={one.month} flex={1} align="stretch" justify="end" height="100%" gap={4} tooltip={`${one.month}: ${money(one.amount)}`}>
                    <group key={'bar-' + one.month} height={Math.max(2, (one.amount / most) * 100)} radius={3} background={theme.accent} />
                    <text color={theme.muted} fontSize={11} textAlign="center">{monthName(one.month)}</text>
                </column>
            ))}
        </row>
    );
}

function Trend({ points }: { points: [number, number][] }) {
    const theme = useContext(Theme);
    if (points.length < 2) {
        return null;
    }
    return (
        <Svg key="trend" name="Spending over time" viewBox="0 0 400 100" stretch height={100} alignSelf="stretch">
            <G stroke={theme.line} strokeWidth={1} fill="none">
                <Path d="M0 25H400M0 50H400M0 75H400" />
            </G>
            <Polyline points={points} fill="none" stroke={theme.accent} strokeWidth={2} strokeLinecap="round" />
        </Svg>
    );
}

function Header({ label, column, width, flex, sort, onSort, right }: { label: string; column: keyof Expense; width?: number; flex?: number; sort: { by: keyof Expense; up: boolean }; onSort: (by: keyof Expense) => void; right?: boolean }) {
    const theme = useContext(Theme);
    return (
        <row key={'sort-' + column} width={width} flex={flex} gap={4} align="center" justify={right ? 'end' : 'start'} hover={theme.hover} radius={4} padding={2} onClick={() => onSort(column)}>
            <text fontWeight="bold" color={theme.muted}>{label}</text>
            {sort.by === column ? <Icon d={sort.up ? UP : DOWN} size={12} color={theme.muted} /> : null}
        </row>
    );
}

export default function App() {
    const scheme = useColorScheme();
    const theme = scheme === 'dark' ? DARK : LIGHT;
    const [expenses, setExpenses] = useState<Expense[]>(load);
    const unsaved = useRef<Expense[] | null>(null);
    const [description, setDescription] = useState('');
    const [amount, setAmount] = useState('');
    const [date, setDate] = useState(() => dayOf(new Date()));
    const [category, setCategory] = useState(0);
    const [problem, setProblem] = useState('');
    const [message, setMessage] = useState('');
    const [search, setSearch] = useState('');
    const [chosen, setChosen] = useState<string | null>(null);
    const [sort, setSort] = useState<{ by: keyof Expense; up: boolean }>({ by: 'date', up: false });
    const nextId = useRef(expenses.reduce((most, one) => Math.max(most, one.id), 0) + 1);

    // Writing waits until things have settled, and a new file only replaces the old one when it's whole.
    useEffect(() => {
        unsaved.current = expenses;
        const timer = setTimeout(async () => {
            unsaved.current = null;
            const where = file();
            await fs.mkdir(path.dirname(where), { recursive: true });
            await fs.writeFile(where + '.new', JSON.stringify(expenses));
            await fs.rename(where + '.new', where);
        }, 300);
        return () => clearTimeout(timer);
    }, [expenses]);

    // Closing the window doesn't wait for that, so anything that hasn't been written by then is written as it goes.
    useEffect(() => () => {
        if (unsaved.current !== null) {
            const where = file();
            mkdirSync(path.dirname(where), { recursive: true });
            writeFileSync(where + '.new', JSON.stringify(unsaved.current));
            renameSync(where + '.new', where);
        }
    }, []);

    const shown = useMemo(() => {
        const words = search.toLowerCase();
        const found = expenses.filter((one) => (chosen === null || one.category === chosen) && (words === '' || one.description.toLowerCase().includes(words)));
        const direction = sort.up ? 1 : -1;
        return found.sort((a, b) => {
            const [x, y] = [a[sort.by], b[sort.by]];
            const order = typeof x === 'number' ? x - (y as number) : String(x).localeCompare(String(y));
            return order * direction || a.id - b.id;
        });
    }, [expenses, search, chosen, sort]);

    const { total, shares, months, points } = useMemo(() => {
        const byCategory = new Map<string, number>();
        const byMonth = new Map<string, number>();
        let sum = 0;
        for (const one of expenses) {
            sum += one.amount;
            byCategory.set(one.category, (byCategory.get(one.category) ?? 0) + one.amount);
            const month = one.date.slice(0, 7);
            byMonth.set(month, (byMonth.get(month) ?? 0) + one.amount);
        }
        const ordered = [...byMonth.entries()].sort(([a], [b]) => a.localeCompare(b)).slice(-6);
        const most = Math.max(1, ...ordered.map(([, value]) => value));
        const drawn: [number, number][] = ordered.map(([, value], index) => [(index / Math.max(1, ordered.length - 1)) * 396 + 2, 96 - (value / most) * 92]);
        return {
            total: sum,
            shares: CATEGORIES.filter((name) => byCategory.has(name)).map((name) => ({ category: name, amount: byCategory.get(name) as number })),
            months: ordered.map(([month, value]) => ({ month, amount: value })),
            points: drawn,
        };
    }, [expenses]);

    const add = () => {
        const value = Number(amount.replace(',', '.'));
        if (description.trim() === '') {
            setProblem('Say what it was for.');
        } else if (!Number.isFinite(value) || value <= 0) {
            setProblem('The amount has to be a number above zero.');
        } else if (!/^\d{4}-\d{2}-\d{2}$/.test(date) || Number.isNaN(new Date(date).getTime())) {
            setProblem('The date has to be like 2026-11-07.');
        } else {
            const made = { id: nextId.current, date, description: description.trim(), category: CATEGORIES[category], amount: Math.round(value * 100) / 100 };
            nextId.current += 1;
            setExpenses((all) => [...all, made]);
            setDescription('');
            setAmount('');
            setProblem('');
        }
    };

    const remove = (id: number) => setExpenses((all) => all.filter((one) => one.id !== id));

    const bringIn = async (paths: string[]) => {
        for (const where of paths.filter((one) => /\.csv$/i.test(one))) {
            const text = await fs.readFile(where, 'utf8');
            const made = expensesFromCsv(text, nextId.current);
            nextId.current += made.length;
            setExpenses((all) => [...all, ...made]);
            setMessage(`Added ${made.length} from ${path.basename(where)}`);
        }
    };

    const exportAll = async () => {
        const where = await dialogs.save({ title: 'Export', filters: [{ name: 'Comma-separated values', extensions: ['csv'] }] });
        if (where !== null) {
            await fs.writeFile(where, writeCsv(expenses));
            setMessage('Exported to ' + where);
        }
    };

    const onSort = (by: keyof Expense) => setSort((old) => ({ by, up: old.by === by ? !old.up : by !== 'amount' && by !== 'date' }));

    const renderRow = useCallback((index: number) => {
        const one = shown[index];
        return (
            <row key={one.id} height={ROW} gap={12} align="center" paddingX={12} hover={theme.hover} borderColor={theme.line}>
                <text key={'date-' + one.id} width={90} color={theme.muted}>{one.date}</text>
                <text key={'what-' + one.id} flex={1} ellipsis>{one.description}</text>
                <text width={70} color={theme.muted}>{one.category}</text>
                <text key={'amount-' + one.id} width={80} textAlign="right">{money(one.amount)}</text>
                <group key={'remove-' + one.id} name={'Remove ' + one.description} padding={4} radius={4} hover={theme.hover} onClick={() => remove(one.id)}>
                    <Icon d={TRASH} size={14} color={theme.muted} />
                </group>
            </row>
        );
    }, [shown, theme]);

    return (
        <Theme.Provider value={theme}>
            <window key="root" direction="row" background={theme.page} color={theme.ink} onDropFiles={bringIn}>
                <column width={260} padding={16} gap={10} background={theme.side}>
                    <text fontSize={18} fontWeight="bold">Add an expense</text>
                    <TextInput key="description" placeholder="What for" value={description} onChange={setDescription} />
                    <TextInput key="amount" placeholder="How much" value={amount} onChange={setAmount} />
                    <TextInput key="date" placeholder="When" value={date} onChange={setDate} />
                    <Select id="category" options={CATEGORIES} value={category} onChange={setCategory} />
                    <button key="add" padding={8} radius={6} background={theme.accent} color="#ffffff" hover="#1f6feb" onClick={add}>Add it</button>
                    {problem === '' ? null : <text key="problem" color={theme.bad}>{problem}</text>}
                    <group flex={1} />
                    {message === '' ? null : <text key="message" color={theme.muted} fontSize={12}>{message}</text>}
                    <text color={theme.muted} fontSize={12}>Drop a .csv file anywhere to add what's in it.</text>
                    <button key="export" padding={6} radius={6} borderColor={theme.line} borderWidth={1} color={theme.ink} hover={theme.hover} onClick={exportAll}>Export</button>
                </column>
                <column flex={1} padding={16} gap={14}>
                    <row gap={24} align="end">
                        <column gap={2}>
                            <text color={theme.muted}>Altogether</text>
                            <text key="total" fontSize={28} fontWeight="bold">{money(total)}</text>
                        </column>
                        <text color={theme.muted}>
                            across <span key="count" fontWeight="bold">{expenses.length}</span> expenses, with <span fontWeight="bold">{shown.length}</span> showing
                        </text>
                    </row>
                    <row gap={24} align="center">
                        <Donut shares={shares} total={total} chosen={chosen} onChoose={(name) => setChosen((old) => (old === name ? null : name))} />
                        <Months months={months} />
                    </row>
                    <Trend points={points} />
                    <TextInput key="search" placeholder="Find" value={search} onChange={setSearch} />
                    <row gap={12} paddingX={12}>
                        <Header label="Date" column="date" width={90} sort={sort} onSort={onSort} />
                        <Header label="What for" column="description" flex={1} sort={sort} onSort={onSort} />
                        <Header label="Kind" column="category" width={70} sort={sort} onSort={onSort} />
                        <Header label="Amount" column="amount" width={80} sort={sort} onSort={onSort} right />
                        <group width={22} />
                    </row>
                    <list key="rows" flex={1} count={shown.length} visible={12} rowHeight={ROW} renderRow={renderRow} />
                </column>
            </window>
        </Theme.Provider>
    );
}
