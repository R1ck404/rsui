// Keeping track of invoices, written the way an agent would: with nothing but `rsui-tsx --agent` to run it and see what it does.
// It has a list to filter and search, an invoice to edit with lines that add up, a settings window, a question before anything's
// deleted, and a file to export to. It saves as you go.
//
// Run it with `cargo run --release -p rsui-tsx -- crates/rsui-tsx/examples/invoices.tsx`.

import {
    focus,
    Select,
    Slider,
    mainWindow,
    Tabs,
    TextInput,
    useColorScheme,
    useEffect,
    useMemo,
    useReducer,
    useRef,
    useState,
    Window,
    createContext,
    useContext,
} from 'rsui';
import { dialogs } from 'rsui/desktop';
import { existsSync, mkdirSync, promises as fs, readFileSync, renameSync, writeFileSync } from 'fs';
import * as path from 'path';

export const title = 'Invoices';
export const size = [980, 640];
export const minSize = [720, 460];

type Status = 'draft' | 'sent' | 'paid';
interface Line {
    id: number;
    what: string;
    /** As typed, as what's being typed isn't a number yet. */
    quantity: string;
    price: string;
}
interface Invoice {
    id: number;
    number: string;
    client: string;
    issued: string;
    due: string;
    status: Status;
    /** Its own, as what's already been sent or paid doesn't change along with the settings. */
    tax: number;
    lines: Line[];
}
interface Settings {
    tax: number;
    currency: number;
}
interface Data {
    invoices: Invoice[];
    settings: Settings;
    nextId: number;
}

const CURRENCIES = ['EUR', 'USD', 'GBP'];
const SYMBOLS = ['€', '$', '£'];
const FILTERS = ['All', 'Draft', 'Sent', 'Overdue', 'Paid'];

const LIGHT = {
    page: '#f6f7f9', panel: '#ffffff', ink: '#1c2024', muted: '#60676f', line: '#e1e4e8', hover: '#eef1f4', chosen: '#e3edfd',
    primary: '#1f6feb', onPrimary: '#ffffff', danger: '#c62828', overdue: '#b3261e', paid: '#1a7f37', draft: '#6e7781', sent: '#1f6feb',
};
const DARK = {
    page: '#0f1216', panel: '#171b21', ink: '#e6e9ed', muted: '#98a1ab', line: '#2a3038', hover: '#21262d', chosen: '#1c2c44',
    primary: '#2f81f7', onPrimary: '#ffffff', danger: '#f0655d', overdue: '#f0655d', paid: '#3fb950', draft: '#98a1ab', sent: '#58a6ff',
};
const Theme = createContext(LIGHT);

const dayOf = (date: Date) => date.toISOString().slice(0, 10);
const daysFrom = (day: string, days: number) => dayOf(new Date(Date.parse(day) + days * 86_400_000));
const today = () => dayOf(new Date());

const number = (text: string) => {
    const value = Number(text.trim().replace(',', '.'));
    return text.trim() !== '' && Number.isFinite(value) ? value : null;
};
const isDay = (text: string) => /^\d{4}-\d{2}-\d{2}$/.test(text) && !Number.isNaN(Date.parse(text));
// Each of these is whole cents, so that what's shown always adds up, and rounding happens once for each amount that's shown.
const cents = (value: number) => Math.round(value * 100) / 100;
const lineTotal = (line: Line) => cents((number(line.quantity) ?? 0) * (number(line.price) ?? 0));
const subtotalOf = (invoice: Invoice) => cents(invoice.lines.reduce((sum, line) => sum + lineTotal(line), 0));
const taxOf = (invoice: Invoice) => cents(subtotalOf(invoice) * invoice.tax / 100);
const totalOf = (invoice: Invoice) => cents(subtotalOf(invoice) + taxOf(invoice));
const isOverdue = (invoice: Invoice) => invoice.status === 'sent' && invoice.due < today();
const statusOf = (invoice: Invoice) => (isOverdue(invoice) ? 'overdue' : invoice.status);

function dataFile() {
    return process.env.RSUI_INVOICES_FILE ?? path.join(process.env.APPDATA ?? '.', 'rsui-invoices', 'invoices.json');
}

function firstData(): Data {
    const day = today();
    const invoice = (id: number, client: string, issued: number, status: Status, lines: [string, string, string][]): Invoice => ({
        id,
        number: 'INV-' + String(1000 + id),
        client,
        issued: daysFrom(day, issued),
        due: daysFrom(day, issued + 30),
        status,
        tax: 21,
        lines: lines.map(([what, quantity, price], at) => ({ id: at + 1, what, quantity, price })),
    });
    return {
        invoices: [
            invoice(1, 'Northwind Traders', -60, 'paid', [['Design work', '12', '85'], ['Hosting', '1', '40']]),
            invoice(2, 'Blue Yonder', -45, 'sent', [['Consulting', '8', '120']]),
            invoice(3, 'Contoso Ltd', -5, 'sent', [['Development', '30', '95'], ['Support', '4', '60']]),
            invoice(4, 'Fabrikam', 0, 'draft', [['Workshop', '2', '900']]),
        ],
        settings: { tax: 21, currency: 0 },
        nextId: 5,
    };
}

function load(): Data {
    try {
        const data: Data = JSON.parse(readFileSync(dataFile(), 'utf8'));
        // What was saved before invoices had a tax of their own had the one in the settings.
        for (const invoice of data.invoices) {
            invoice.tax ??= data.settings.tax;
        }
        return data;
    } catch {
        return firstData();
    }
}

// Written to another file first, so that stopping halfway can't lose what's there.
function save(data: Data) {
    const file = dataFile();
    mkdirSync(path.dirname(file), { recursive: true });
    writeFileSync(file + '.new', JSON.stringify(data));
    renameSync(file + '.new', file);
}

type Action =
    | { kind: 'add'; invoice: Invoice }
    | { kind: 'change'; id: number; change: Partial<Invoice> }
    | { kind: 'remove'; id: number }
    | { kind: 'settings'; change: Partial<Settings> };

function reducer(data: Data, action: Action): Data {
    switch (action.kind) {
        case 'add':
            return { ...data, invoices: [...data.invoices, action.invoice], nextId: data.nextId + 1 };
        case 'change':
            return {
                ...data,
                invoices: data.invoices.map((one) => (one.id === action.id ? { ...one, ...action.change } : one)),
            };
        case 'remove':
            return { ...data, invoices: data.invoices.filter((one) => one.id !== action.id) };
        case 'settings':
            return { ...data, settings: { ...data.settings, ...action.change } };
    }
}

function Action({ id, onClick, children, primary, danger, disabled, tooltip }: {
    id: string;
    onClick: () => void;
    children: any;
    primary?: boolean;
    danger?: boolean;
    disabled?: boolean;
    tooltip?: string;
}) {
    const theme = useContext(Theme);
    return (
        <button
            key={id}
            paddingX={12}
            paddingY={6}
            radius={6}
            disabled={disabled}
            tooltip={tooltip}
            background={primary ? theme.primary : theme.panel}
            hover={primary ? undefined : theme.hover}
            borderColor={primary ? theme.primary : theme.line}
            borderWidth={1}
            color={primary ? theme.onPrimary : danger ? theme.danger : theme.ink}
            onClick={onClick}
        >
            {children}
        </button>
    );
}

function Badge({ invoice }: { invoice: Invoice }) {
    const theme = useContext(Theme);
    const status = statusOf(invoice);
    return (
        <text fontSize={11} fontWeight="bold" color={theme[status]}>
            {status.toUpperCase()}
        </text>
    );
}

function Field({ label, children, problem }: { label: string; children: any; problem?: string | null }) {
    const theme = useContext(Theme);
    return (
        <column gap={4}>
            <text fontSize={12} color={theme.muted}>{label}</text>
            {children}
            {problem ? <text fontSize={12} color={theme.danger}>{problem}</text> : null}
        </column>
    );
}

function Summary({ data, money }: { data: Data; money: (value: number) => string }) {
    const theme = useContext(Theme);
    const sum = (test: (invoice: Invoice) => boolean) =>
        data.invoices.filter(test).reduce((all, invoice) => all + totalOf(invoice), 0);
    const parts: [string, number, string][] = [
        ['Outstanding', sum((one) => one.status === 'sent'), theme.ink],
        ['Overdue', sum(isOverdue), theme.overdue],
        ['Paid', sum((one) => one.status === 'paid'), theme.paid],
    ];
    return (
        <row gap={24} paddingX={16} paddingY={12} background={theme.panel} borderColor={theme.line} borderWidth={1} radius={8}>
            {parts.map(([label, value, color]) => (
                <column key={label} gap={2} name={label + ' total'}>
                    <text fontSize={12} color={theme.muted}>{label}</text>
                    <text key={'total-' + label} fontSize={18} fontWeight="bold" color={color}>{money(value)}</text>
                </column>
            ))}
        </row>
    );
}

function Row({ invoice, chosen, money, total, onChoose }: {
    invoice: Invoice;
    chosen: boolean;
    money: (value: number) => string;
    total: number;
    onChoose: () => void;
}) {
    const theme = useContext(Theme);
    return (
        <item
            key={'invoice-' + invoice.id}
            name={invoice.number + ' ' + invoice.client}
            paddingX={12}
            paddingY={10}
            gap={4}
            radius={6}
            background={chosen ? theme.chosen : undefined}
            hover={chosen ? undefined : theme.hover}
            onClick={onChoose}
        >
            <row justify="space_between" gap={8}>
                <text fontWeight="bold" ellipsis flex={1}>{invoice.client || 'No client yet'}</text>
                <text fontWeight="bold">{money(total)}</text>
            </row>
            <row justify="space_between" gap={8}>
                <text fontSize={12} color={theme.muted}>{invoice.number + ' · due ' + invoice.due}</text>
                <Badge invoice={invoice} />
            </row>
        </item>
    );
}

function LineRow({ line, money, onChange, onRemove }: {
    line: Line;
    money: (value: number) => string;
    onChange: (change: Partial<Line>) => void;
    onRemove: () => void;
}) {
    const theme = useContext(Theme);
    const bad = number(line.quantity) === null || number(line.price) === null;
    return (
        <row key={'line-' + line.id} gap={8} align="center">
            <TextInput key={'what-' + line.id} name="What it's for" flex={1} value={line.what} placeholder="What it's for" onChange={(what: string) => onChange({ what })} />
            <TextInput key={'quantity-' + line.id} name="Quantity" width={70} value={line.quantity} onChange={(quantity: string) => onChange({ quantity })} />
            <TextInput key={'price-' + line.id} name="Price" width={90} value={line.price} onChange={(price: string) => onChange({ price })} />
            <text key={'line-total-' + line.id} width={100} textAlign="right" color={bad ? theme.danger : theme.ink}>
                {bad ? 'Not a number' : money(lineTotal(line))}
            </text>
            <Action id={'remove-line-' + line.id} onClick={onRemove} tooltip="Take this line off">×</Action>
        </row>
    );
}

function Detail({ invoice, data, money, dispatch, onDelete, onDuplicate, onExport }: {
    invoice: Invoice;
    data: Data;
    money: (value: number) => string;
    dispatch: (action: Action) => void;
    onDelete: () => void;
    onDuplicate: () => void;
    onExport: () => void;
}) {
    const theme = useContext(Theme);
    const change = (update: Partial<Invoice>) => dispatch({ kind: 'change', id: invoice.id, change: update });
    const changeLine = (id: number, update: Partial<Line>) =>
        change({ lines: invoice.lines.map((line) => (line.id === id ? { ...line, ...update } : line)) });
    const subtotal = subtotalOf(invoice);
    const nextLine = invoice.lines.reduce((most, line) => Math.max(most, line.id), 0) + 1;
    return (
        <scroll key="detail" flex={1} gap={16} padding={16} background={theme.panel} borderColor={theme.line} borderWidth={1} radius={8}>
            <row justify="space_between" align="center" gap={8}>
                <text key="invoice-number" fontSize={20} fontWeight="bold">{invoice.number}</text>
                <Badge invoice={invoice} />
            </row>
            <Field label="Client" problem={invoice.client.trim() === '' ? 'An invoice needs a client' : null}>
                <TextInput key="client" name="Client" value={invoice.client} onChange={(client: string) => change({ client })} />
            </Field>
            <row gap={12}>
                <column flex={1}>
                    <Field label="Issued" problem={isDay(invoice.issued) ? null : 'Use the form 2026-12-31'}>
                        <TextInput key="issued" name="Issued" value={invoice.issued} onChange={(issued: string) => change({ issued })} />
                    </Field>
                </column>
                <column flex={1}>
                    <Field label="Due" problem={isDay(invoice.due) ? (invoice.due < invoice.issued ? "It's due before it was issued" : null) : 'Use the form 2026-12-31'}>
                        <TextInput key="due" name="Due" value={invoice.due} onChange={(due: string) => change({ due })} />
                    </Field>
                </column>
            </row>
            <column gap={8}>
                <text fontWeight="bold">Lines</text>
                {invoice.lines.map((line) => (
                    <LineRow
                        key={line.id}
                        line={line}
                        money={money}
                        onChange={(update) => changeLine(line.id, update)}
                        onRemove={() => change({ lines: invoice.lines.filter((other) => other.id !== line.id) })}
                    />
                ))}
                {invoice.lines.length === 0 ? <text color={theme.muted}>Nothing on this one yet.</text> : null}
                <row>
                    <Action id="add-line" onClick={() => change({ lines: [...invoice.lines, { id: nextLine, what: '', quantity: '1', price: '0' }] })}>Add a line</Action>
                </row>
            </column>
            <column gap={4} align="end">
                <text key="subtotal">{'Subtotal ' + money(subtotal)}</text>
                {invoice.status === 'draft' && invoice.tax !== data.settings.tax ? (
                    <Action id="use-tax" onClick={() => change({ tax: data.settings.tax })}>{'Use ' + data.settings.tax + '% from the settings'}</Action>
                ) : null}
                <text key="tax" color={theme.muted}>{'Tax at ' + invoice.tax + '% ' + money(taxOf(invoice))}</text>
                <text key="total" fontSize={18} fontWeight="bold">{'Total ' + money(totalOf(invoice))}</text>
            </column>
            <row gap={8}>
                {invoice.status === 'draft' ? <Action id="send" primary onClick={() => change({ status: 'sent' })}>Mark as sent</Action> : null}
                {invoice.status === 'sent' ? <Action id="pay" primary onClick={() => change({ status: 'paid' })}>Mark as paid</Action> : null}
                {invoice.status === 'paid' ? <Action id="unpay" onClick={() => change({ status: 'sent' })}>Not paid after all</Action> : null}
                <Action id="duplicate" onClick={onDuplicate}>Duplicate</Action>
                <Action id="export" onClick={onExport}>Export</Action>
                <Action id="delete" danger onClick={onDelete}>Delete</Action>
            </row>
        </scroll>
    );
}

function csvOf(invoice: Invoice) {
    const quote = (text: string) => (/[",\n]/.test(text) ? '"' + text.replaceAll('"', '""') + '"' : text);
    const rows = [['What', 'Quantity', 'Price', 'Total']];
    for (const line of invoice.lines) {
        rows.push([line.what, line.quantity, line.price, lineTotal(line).toFixed(2)]);
    }
    rows.push(['Tax at ' + invoice.tax + '%', '', '', taxOf(invoice).toFixed(2)]);
    rows.push(['Total', '', '', totalOf(invoice).toFixed(2)]);
    return rows.map((row) => row.map(quote).join(',')).join('\n') + '\n';
}

export default function App() {
    const scheme = useColorScheme();
    const theme = scheme === 'dark' ? DARK : LIGHT;
    const [data, dispatch] = useReducer(reducer, null, load);
    const [filter, setFilter] = useState(0);
    const [search, setSearch] = useState('');
    // The newest, which is what's at the top of the list.
    const [chosenId, setChosenId] = useState<number | null>(() => {
        const newest = [...data.invoices].sort((a, b) => b.issued.localeCompare(a.issued))[0];
        return newest?.id ?? null;
    });
    const [showSettings, setShowSettings] = useState(false);
    const [deleting, setDeleting] = useState(false);
    const [message, setMessage] = useState('');

    // Saves a moment after the last change, so that typing doesn't write the file on every key. Closing the window doesn't wait
    // for that, so anything that hasn't been written by then is written as it goes.
    const first = useRef(true);
    const unsaved = useRef<Data | null>(null);
    useEffect(() => {
        if (first.current) {
            first.current = false;
            return;
        }
        unsaved.current = data;
        const timer = setTimeout(() => {
            unsaved.current = null;
            try {
                save(data);
            } catch (error) {
                setMessage("Couldn't save: " + error.message);
            }
        }, 300);
        return () => clearTimeout(timer);
    }, [data]);
    useEffect(() => () => {
        if (unsaved.current !== null) {
            save(unsaved.current);
        }
    }, []);

    const symbol = SYMBOLS[data.settings.currency];
    const money = (value: number) => symbol + value.toFixed(2).replace(/\B(?=(\d{3})+(?!\d))/g, ',');

    const shown = useMemo(() => {
        const words = search.trim().toLowerCase();
        const wanted = FILTERS[filter].toLowerCase();
        return data.invoices
            .filter((one) => filter === 0 || statusOf(one) === wanted)
            .filter((one) => words === '' || (one.client + ' ' + one.number).toLowerCase().includes(words))
            .sort((a, b) => b.issued.localeCompare(a.issued));
    }, [data, filter, search]);
    const chosen = data.invoices.find((one) => one.id === chosenId) ?? null;

    useEffect(() => {
        mainWindow.setTitle(chosen ? chosen.number + ' - Invoices' : 'Invoices');
    }, [chosen?.number]);

    const addInvoice = () => {
        const id = data.nextId;
        const day = today();
        dispatch({
            kind: 'add',
            invoice: {
                id,
                number: 'INV-' + String(1000 + id),
                client: '',
                issued: day,
                due: daysFrom(day, 30),
                status: 'draft',
                tax: data.settings.tax,
                lines: [{ id: 1, what: '', quantity: '1', price: '0' }],
            },
        });
        setFilter(0);
        setSearch('');
        setChosenId(id);
        focus('client');
    };
    const duplicate = () => {
        if (!chosen) return;
        const id = data.nextId;
        const day = today();
        dispatch({
            kind: 'add',
            invoice: { ...chosen, id, number: 'INV-' + String(1000 + id), issued: day, due: daysFrom(day, 30), status: 'draft', tax: data.settings.tax },
        });
        setChosenId(id);
    };
    const exportChosen = async () => {
        if (!chosen) return;
        const where = await dialogs.save({ title: 'Export ' + chosen.number, start: chosen.number + '.csv', filters: [{ name: 'CSV', extensions: ['csv'] }] });
        if (where === null) return;
        try {
            await fs.writeFile(where, csvOf(chosen));
            setMessage('Exported to ' + where);
        } catch (error) {
            setMessage("Couldn't export: " + error.message);
        }
    };
    const removeChosen = () => {
        if (chosen) {
            dispatch({ kind: 'remove', id: chosen.id });
            const others = shown.filter((one) => one.id !== chosen.id);
            setChosenId(others[0]?.id ?? null);
        }
        setDeleting(false);
    };
    const onKey = (key: string) => {
        if (key === 'ctrl+n') {
            addInvoice();
            return true;
        }
        if (key === 'ctrl+f') {
            focus('search');
            return true;
        }
        return false;
    };

    return (
        <Theme.Provider value={theme}>
            <window key="root" background={theme.page} color={theme.ink} padding={16} gap={12} onKeyPress={onKey}>
                <row justify="space_between" align="center" gap={12}>
                    <Summary data={data} money={money} />
                    <row gap={8}>
                        <Action id="settings" onClick={() => setShowSettings(!showSettings)}>Settings</Action>
                        <Action id="new" primary onClick={addInvoice} tooltip="Ctrl+N">New invoice</Action>
                    </row>
                </row>
                <row flex={1} gap={12}>
                    <column width={340} gap={8}>
                        <TextInput key="search" name="Search" placeholder="Search by client or number" value={search} onChange={setSearch} />
                        <Tabs id="filter" options={FILTERS} value={filter} onChange={setFilter} />
                        <scroll key="list" flex={1} gap={4}>
                            {shown.map((invoice) => (
                                <Row
                                    key={invoice.id}
                                    invoice={invoice}
                                    chosen={invoice.id === chosenId}
                                    money={money}
                                    total={totalOf(invoice)}
                                    onChoose={() => setChosenId(invoice.id)}
                                />
                            ))}
                            {shown.length === 0 ? <text key="none" color={theme.muted} padding={12}>Nothing matches that.</text> : null}
                        </scroll>
                    </column>
                    {chosen ? (
                        <Detail invoice={chosen} data={data} money={money} dispatch={dispatch} onDelete={() => setDeleting(true)} onDuplicate={duplicate} onExport={exportChosen} />
                    ) : (
                        <column flex={1} align="center" justify="center"><text color={theme.muted}>Choose an invoice, or make a new one.</text></column>
                    )}
                </row>
                {message ? <text key="message" color={theme.muted}>{message}</text> : null}

                {showSettings ? (
                    <Window title="Settings" size={[340, 240]} minSize={[300, 220]} onClose={() => setShowSettings(false)}>
                        <column flex={1} padding={16} gap={12} background={theme.panel} color={theme.ink}>
                            <text key="tax-now" fontWeight="bold">{'Tax on new invoices: ' + data.settings.tax + '%'}</text>
                            <Slider id="tax" name="Tax" value={data.settings.tax} min={0} max={30} step={1} onChange={(tax: number) => dispatch({ kind: 'settings', change: { tax } })} />
                            <text fontWeight="bold">Currency</text>
                            <Select id="currency" options={CURRENCIES} value={data.settings.currency} onChange={(currency: number) => dispatch({ kind: 'settings', change: { currency } })} />
                        </column>
                    </Window>
                ) : null}

                {deleting && chosen ? (
                    <Window title="Delete it?" size={[340, 130]} modal onClose={() => setDeleting(false)}>
                        <column flex={1} padding={16} gap={12} background={theme.panel} color={theme.ink}>
                            <text>{'Delete ' + chosen.number + ' for ' + (chosen.client || 'nobody yet') + '? This can\'t be undone.'}</text>
                            <row gap={8} justify="end">
                                <Action id="keep" onClick={() => setDeleting(false)}>Keep it</Action>
                                <Action id="yes-delete" primary onClick={removeChosen}>Delete</Action>
                            </row>
                        </column>
                    </Window>
                ) : null}
            </window>
        </Theme.Provider>
    );
}
