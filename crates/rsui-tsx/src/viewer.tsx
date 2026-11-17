// rsui's inspector, laid out as a browser's is. `rsui-tsx --view file.rsuitrace` shows a trace that was saved, with the file in
// RSUI_TRACE: each frame as what came in, what rendered and why, and where the time went, with the components that took the
// most, and every change of state. `rsui-tsx --view` looks at an app while it runs, over the pipe each app run with rsui-tsx
// listens on, as said in the files in RSUI_LIVE: then there's its elements, what it says and a console for it, its code with
// breakpoints and logpoints, and what came in to it, to start it again from any of that and stop in what it did.
import { useState, useMemo, useEffect, useRef, useCallback, useColorScheme, TextInput, Svg, Path, Circle, focus } from 'rsui';
import { readFileSync, readdirSync, unlinkSync } from 'fs';
import { connect } from 'net';

export const title = 'rsui inspector';
export const size = [1280, 800];
export const minSize = [720, 420];

type Handled = { action: string; key: string; took: number; note: string | null; rejected: boolean };
type Render = { name: string | null; cause: string; took: number; own: number; changed: boolean; depth: number; why: string | null };
type Frame = { number: number; at: number; script: number; layout: number; paint: number; gpu: number; present?: number; handled: Handled[]; renders: Render[] };
type Change = { number: number; component: string; said: string };
type Trace = { app: string; file: string | null; frames: Frame[]; history: Change[]; summary: string };
type Running = { pid: number; pipe: string; app: string; file: string; started: number; place: string };
type Heard = { number: number; line: string };
type Logpoint = { at: string; expression: string; hits: number };
type Breakpoint = { at: string; hits: number };
type Local = { name: string; value: string };
type StoppedFrame = { at: string; function: string; debugged: boolean; locals: Local[] };
type Stopped = { reason: string; at: string; function: string; source: { line: number; text: string }[]; frames: StoppedFrame[] };
type Input = { number: number; at: number; kind: string; said: string };
type El = { path: number[]; tag: string; key: string | null; name: string; props: Record<string, any>; actions: string[]; rect: number[] | null; children: El[]; more: number };
type Made = { component: string | null; around: string[]; why: string | null; hooks: string[]; rect: number[] | null; inspecting: boolean };
type Answer = { error?: string; said?: string; [more: string]: any };
type Ask = (request: object) => Promise<Answer>;

const BUDGET = 16.667;
/// The most frames the inspector keeps of an app it's looking at, as the app does.
const MOST_FRAMES = 4000;
/// The most lines of what's been said it keeps, and how many of a long list are shown.
const MOST_HEARD = 1000;
const MOST_LISTED = 300;
/// How often it asks an app what's new, and what's in its window while that's shown.
const ASK_EVERY = 250;
const TREE_EVERY = 1000;
const UI = 'Segoe UI';
const MONO = 'Consolas';
const ROW = 20;
/// How wide a pane has to be for what's in it, below which it scrolls across, as it does docked along an app's right.
const PANEL_LEAST = 820;

function load(): Trace | string | null {
    const file = process.env.RSUI_TRACE;
    if (!file) {
        return process.env.RSUI_LIVE ? null : 'There is no trace to show. Open one with rsui-tsx --view file.rsuitrace.';
    }
    try {
        return JSON.parse(readFileSync(file, 'utf8'));
    } catch (error) {
        return `${file} couldn't be read: ${error}`;
    }
}

const TRACE = load();

/// The colors of a browser's inspector, dark and light.
function colors(dark: boolean) {
    return dark
        ? {
              page: '#202124', bar: '#292a2d', raised: '#35363a', line: '#3c4043', strong: '#5f6368', text: '#e8eaed', dim: '#9aa0a6',
              hover: '#2f3134', selected: '#33445c', chosen: '#3c4043', accent: '#8ab4f8', onAccent: '#202124', field: '#202124',
              tag: '#5db0d7', attr: '#9bbbdc', value: '#f29766', str: '#f29766', num: '#9980ff', kw: '#c58af9', comment: '#9aa0a6', fn: '#78d9ec',
              error: '#f28b82', errorBack: '#4e3534', warn: '#fdd663', warnBack: '#413c26', ok: '#81c995',
              paused: '#fdd663', pausedBack: '#3d3a28', exec: '#2c4268', breakpoint: '#5186ec', logpoint: '#e37400',
              script: '#e3b341', layout: '#a487f2', paint: '#6fc26d', gpu: '#6a9fe6',
              margin: '#7b5a3a', border: '#8a7a3a', padding: '#5d6e3a', content: '#3a5e78', box: '#e8eaed',
          }
        : {
              page: '#ffffff', bar: '#f1f3f4', raised: '#ffffff', line: '#d6d9dc', strong: '#9aa0a6', text: '#1f1f1f', dim: '#5f6368',
              hover: '#f1f4f8', selected: '#cfe8fc', chosen: '#e8eaed', accent: '#1a73e8', onAccent: '#ffffff', field: '#ffffff',
              tag: '#881280', attr: '#994500', value: '#1a1aa6', str: '#c41a16', num: '#1c00cf', kw: '#aa0d91', comment: '#236e25', fn: '#0b6e99',
              error: '#d93025', errorBack: '#fce8e6', warn: '#9b5600', warnBack: '#fef7e0', ok: '#188038',
              paused: '#7a5a00', pausedBack: '#fef7e0', exec: '#dbe8fd', breakpoint: '#1a73e8', logpoint: '#e37400',
              script: '#f2c14e', layout: '#9b84e4', paint: '#62b36a', gpu: '#7aa6e8',
              margin: '#f9cc9d', border: '#fddd9b', padding: '#c3d08b', content: '#8cb6c0', box: '#1f1f1f',
          };
}
type Theme = ReturnType<typeof colors>;
const DARK = colors(true);
const LIGHT = colors(false);

const PARTS: [keyof Frame, string, keyof Theme][] = [
    ['script', 'Scripting', 'script'],
    ['layout', 'Layout', 'layout'],
    ['paint', 'Painting', 'paint'],
    ['gpu', 'GPU', 'gpu'],
];

function ms(value: number): string {
    if (value > 0 && value < 0.1) {
        return '<0.1 ms';
    }
    return `${value.toFixed(1)} ms`;
}

function total(frame: Frame): number {
    return frame.script + frame.layout + frame.paint + frame.gpu;
}

function plural(count: number, what: string): string {
    return `${count} ${what}${count === 1 ? '' : 's'}`;
}

/// One line of JSON each way with an app, over its pipe, each answer coming in the order it was asked.
class Link {
    #socket: any;
    #waiting: ((answer: Answer) => void)[] = [];
    #given = '';
    gone = false;

    constructor(pipe: string, onGone: (why: string) => void) {
        this.#socket = connect(pipe);
        this.#socket.setEncoding?.('utf8');
        this.#socket.on('data', (chunk: unknown) => {
            this.#given += String(chunk);
            let end;
            while ((end = this.#given.indexOf('\n')) >= 0) {
                const line = this.#given.slice(0, end);
                this.#given = this.#given.slice(end + 1);
                let answer: Answer;
                try {
                    answer = JSON.parse(line);
                } catch {
                    answer = { error: `the app said something that isn't JSON: ${line.slice(0, 80)}` };
                }
                this.#waiting.shift()?.(answer);
            }
        });
        let why = 'The app has stopped.';
        this.#socket.on('error', (error: { code?: string; message?: string }) => {
            why = error.code === 'ENOENT' ? 'The app is no longer running.' : `The app can't be reached: ${error.message ?? error}`;
        });
        this.#socket.on('close', () => {
            this.gone = true;
            for (const waiting of this.#waiting.splice(0)) {
                waiting({ error: why });
            }
            onGone(why);
        });
    }

    ask(request: object): Promise<Answer> {
        if (this.gone) {
            return Promise.resolve({ error: 'The app has stopped.' });
        }
        return new Promise((done) => {
            this.#waiting.push(done);
            this.#socket.write(JSON.stringify(request) + '\n');
        });
    }

    close() {
        this.gone = true;
        this.#socket.destroy();
    }
}

/// The apps that say they can be looked at, newest first.
function runningApps(): Running[] {
    const folder = process.env.RSUI_LIVE;
    if (!folder) {
        return [];
    }
    let names: string[] = [];
    try {
        names = readdirSync(folder);
    } catch {
        return [];
    }
    const found: Running[] = [];
    for (const name of names) {
        if (!name.endsWith('.json')) {
            continue;
        }
        const place = `${folder}\\${name}`;
        try {
            found.push({ ...JSON.parse(readFileSync(place, 'utf8')), place });
        } catch {
            // It's being written, or it's not one of these.
        }
    }
    const wanted = Number(process.env.RSUI_LIVE_PID);
    return found.filter((one) => !wanted || one.pid === wanted).sort((a, b) => b.started - a.started);
}

// Pieces everything's made of.

const ICONS: Record<string, string> = {
    resume: 'M8 5v14l11-7z',
    pause: 'M6 5h4v14H6zM14 5h4v14h-4z',
    right: 'M9 6l6 6-6 6z',
    down: 'M6 9l6 6 6-6z',
    close: 'M6 6l12 12M18 6L6 18',
    clear: 'M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18zM6 6l12 12',
    save: 'M12 3v12M7 10l5 5 5-5M5 20h14',
    eye: 'M2 12c3-6 17-6 20 0c-3 6-17 6-20 0zM12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6z',
    freeze: 'M12 2v20M4 7l16 10M20 7L4 17',
    file: 'M6 2h8l4 4v16H6zM14 2v4h4',
    folder: 'M3 6h6l2 2h10v11H3z',
    back: 'M15 6l-6 6 6 6',
    replay: 'M4 12a8 8 0 1 0 3-6.2M4 4v5h5',
    stop: 'M7 7h10v10H7z',
    dockRight: 'M3 5h18v14H3zM15 5v14',
    dockBottom: 'M3 5h18v14H3zM3 14h18',
    undock: 'M9 9h11v10H9zM4 15V5h11',
};

/// An icon, drawn with lines, or filled for those that are shapes.
function Icon({ name, color, size = 16 }: { name: string; color: string; size?: number }) {
    const filled = name === 'resume' || name === 'pause' || name === 'right' || name === 'down' || name === 'stop';
    if (name === 'over' || name === 'into' || name === 'out') {
        return (
            <Svg width={size} height={size} viewBox="0 0 24 24" color={color} fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
                {name === 'over' ? <Path d="M4 13a8 8 0 0 1 15-3.5M20 4v6h-6" /> : null}
                {name === 'into' ? <Path d="M12 3v10M8 9l4 4 4-4" /> : null}
                {name === 'out' ? <Path d="M12 14V4M8 8l4-4 4 4" /> : null}
                <Circle cx={12} cy={19} r={2} fill="currentColor" />
            </Svg>
        );
    }
    return (
        <Svg width={size} height={size} viewBox="0 0 24 24" color={color} fill={filled ? 'currentColor' : 'none'} stroke={filled ? 'none' : 'currentColor'} strokeWidth={2} strokeLinecap="round">
            <Path d={ICONS[name] ?? ''} />
        </Svg>
    );
}

/// A button that's only an icon, named for what it does, which is what shows when the pointer's on it.
function IconButton({ name, icon, onClick, theme, color, disabled, on }: { name: string; icon: string; onClick: () => void; theme: Theme; color?: string; disabled?: boolean; on?: boolean }) {
    return (
        <button name={name} tooltip={name} onClick={onClick} disabled={disabled} width={26} height={24} radius={4} align="center" justify="center" background={on ? theme.chosen : 'transparent'} hover={theme.raised}>
            <Icon name={icon} color={color ?? (on ? theme.accent : theme.dim)} />
        </button>
    );
}

/// A small button with words on it, as a browser's inspector has in its toolbars.
function Action({ label, onClick, theme, disabled, strong }: { label: string; onClick: () => void; theme: Theme; disabled?: boolean; strong?: boolean }) {
    return (
        <button name={label} onClick={onClick} disabled={disabled} direction="row" paddingX={10} height={24} radius={4} align="center" justify="center" background={strong ? theme.accent : 'transparent'} borderWidth={1} borderColor={strong ? theme.accent : theme.line} hover={strong ? theme.accent : theme.raised}>
            <text fontSize={12} fontFamily={UI} color={strong ? theme.onAccent : theme.accent} noWrap>{label}</text>
        </button>
    );
}

function Across({ theme }: { theme: Theme }) {
    return <group height={1} background={theme.line} />;
}

function Down({ theme }: { theme: Theme }) {
    return <group width={1} background={theme.line} />;
}

/// A line between two panes, which can be dragged to make the one it's for wider or narrower.
function Splitter({ width, setWidth, least, most, theme, fromRight }: { width: number; setWidth: (width: number) => void; least: number; most: number; theme: Theme; fromRight?: boolean }) {
    return (
        <group
            width={5}
            align="center"
            hover={theme.line}
            onDrag={(phase: string, x: number, y: number, dx: number) => {
                if (phase === 'move') {
                    setWidth(Math.max(least, Math.min(most, width + (fromRight ? -dx : dx))));
                }
            }}
        >
            <group width={1} flex={1} background={theme.line} />
        </group>
    );
}

/// A row of tabs across the top of what they're for, the chosen one underlined.
function TabStrip({ tabs, value, onChange, theme, right }: { tabs: string[]; value: string; onChange: (tab: string) => void; theme: Theme; right?: any }) {
    return (
        <column background={theme.bar}>
            <row height={29} align="center" paddingX={4}>
                <scroll key="tabs" axis="horizontal" flex={1} shrink={1} minWidth={180} height={29}>
                    <row height={29}>
                        {tabs.map((tab) => (
                            <button key={`tab-${tab}`} name={tab} onClick={() => onChange(tab)} direction="column" height={29} paddingX={10} justify="center" hover={theme.hover}>
                                <group flex={1} />
                                <text fontSize={12} fontFamily={UI} color={tab === value ? theme.text : theme.dim} noWrap>{tab}</text>
                                <group flex={1} />
                                <group height={2} width="100%" radius={1} background={tab === value ? theme.accent : 'transparent'} />
                            </button>
                        ))}
                    </row>
                </scroll>
                {right ?? null}
            </row>
            <Across theme={theme} />
        </column>
    );
}

/// A strip of tools at the top of a pane.
function Toolbar({ theme, children }: { theme: Theme; children?: any }) {
    return (
        <column background={theme.bar}>
            <row height={28} align="center" gap={2} paddingX={4}>
                {children}
            </row>
            <Across theme={theme} />
        </column>
    );
}

/// A part of a sidebar, which can be folded away.
function Section({ title, open, onToggle, theme, children, right }: { title: string; open: boolean; onToggle: () => void; theme: Theme; children?: any; right?: any }) {
    return (
        <column>
            <row height={24} align="center" background={theme.bar}>
                <button direction="row" justify="start" name={title} onClick={onToggle} flex={1} height={24} align="center" gap={4} paddingX={6} hover={theme.hover}>
                    <Icon name={open ? 'down' : 'right'} color={theme.dim} size={12} />
                    <text fontSize={12} fontFamily={UI} fontWeight="bold" color={theme.text}>{title}</text>
                </button>
                {right ?? null}
            </row>
            <Across theme={theme} />
            {open ? <column paddingY={2}>{children}</column> : null}
        </column>
    );
}

/// What a pane says when it has nothing to show.
function Empty({ text, theme }: { text: string; theme: Theme }) {
    return (
        <column flex={1} align="center" justify="center" padding={24}>
            <text fontSize={12} fontFamily={UI} color={theme.dim} textAlign="center" maxWidth={420}>{text}</text>
        </column>
    );
}

function Field({ name, value, onChange, placeholder, theme, width, flex, onKeyPress, mono }: { name: string; value: string; onChange: (value: string) => void; placeholder?: string; theme: Theme; width?: number; flex?: number; onKeyPress?: (key: string) => unknown; mono?: boolean }) {
    return (
        <TextInput
            key={`field-${name}`}
            name={name}
            value={value}
            onChange={onChange}
            placeholder={placeholder}
            width={width}
            flex={flex}
            onKeyPress={onKeyPress}
            fontSize={12}
            fontFamily={mono ? MONO : UI}
            paddingX={6}
            paddingY={3}
            radius={3}
            background={theme.field}
            color={theme.text}
            borderColor={theme.line}
        />
    );
}

/// A table's heading, its columns as wide as `widths` says, with 0 for the one that takes what's left.
function Heading({ columns, widths, theme, sorted, onSort }: { columns: string[]; widths: number[]; theme: Theme; sorted?: number; onSort?: (column: number) => void }) {
    return (
        <column background={theme.bar}>
            <row height={22} align="center">
                {columns.map((column, index) => (
                    <button direction="row" justify="start" key={`heading-${column}`} name={`Sort by ${column}`} onClick={() => onSort?.(index)} width={widths[index] || undefined} flex={widths[index] ? undefined : 1} height={22} paddingX={8} align="center" hover={onSort ? theme.hover : undefined}>
                        <text fontSize={12} fontFamily={UI} color={theme.text} noWrap ellipsis>{`${column}${sorted === index ? ' ▾' : ''}`}</text>
                    </button>
                ))}
            </row>
            <Across theme={theme} />
        </column>
    );
}

function Cell({ text, width, theme, color, mono, bold }: { text: string; width: number; theme: Theme; color?: string; mono?: boolean; bold?: boolean }) {
    return (
        <text width={width || undefined} flex={width ? undefined : 1} paddingX={8} fontSize={12} fontFamily={mono ? MONO : UI} fontWeight={bold ? 'bold' : 'normal'} color={color ?? theme.text} noWrap ellipsis selectable>
            {text}
        </text>
    );
}

// Elements.

/// What a tag's written as in JSX, from what the tree calls its role.
function tagOf(element: El): string {
    switch (element.tag) {
        case 'text_input':
            return 'input';
        case 'list_item':
            return 'item';
        case 'menu_item':
            return 'menuitem';
        case 'group':
            return element.props.direction === 'row' ? 'row' : element.props.direction === 'column' ? 'column' : 'group';
        default:
            return element.tag;
    }
}

/// A prop's name as JSX writes it.
function camel(name: string): string {
    return name.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase());
}

function shownValue(value: any): string {
    return typeof value === 'string' ? JSON.stringify(value) : `{${String(value)}}`;
}

const SAID_INLINE = 4;

/// An element's opening tag, with its key and its first few props, as a browser's inspector writes one.
function OpeningTag({ element, theme, closed }: { element: El; theme: Theme; closed: string | null }) {
    const tag = tagOf(element);
    const props = Object.entries(element.props).filter(([name]) => name !== 'direction' && !name.startsWith('scroll_') && name !== 'first_visible');
    return (
        <text fontSize={12} fontFamily={MONO} noWrap>
            <span color={theme.tag}>{`<${tag}`}</span>
            {element.key && !/^\d+$/.test(element.key) ? (
                <span>
                    <span color={theme.attr}>{' key'}</span>
                    <span color={theme.dim}>=</span>
                    <span color={theme.value}>{JSON.stringify(element.key)}</span>
                </span>
            ) : null}
            {props.slice(0, SAID_INLINE).map(([name, value]) => (
                <span key={`prop-${name}`}>
                    <span color={theme.attr}>{` ${camel(name)}`}</span>
                    <span color={theme.dim}>=</span>
                    <span color={theme.value}>{shownValue(value)}</span>
                </span>
            ))}
            {props.length > SAID_INLINE ? <span color={theme.dim}>{' …'}</span> : null}
            <span color={theme.tag}>{'>'}</span>
            {closed !== null ? <span color={theme.text}>{closed}</span> : null}
            {closed !== null ? <span color={theme.tag}>{`</${tag}>`}</span> : null}
        </text>
    );
}

/// One element's row in the tree.
function TreeRow({ element, id, depth, has, opened, chosen, flip, choose, point, theme }: { element: El; id: string; depth: number; has: boolean; opened: boolean; chosen: boolean; flip: (id: string) => void; choose: (id: string) => void; point: (element: El | null) => void; theme: Theme }) {
    // Something with only a name in it is written on one line, as text is.
    const closed = !has ? element.name : opened ? null : '…';
    return (
        <row height={ROW} align="center" background={chosen ? theme.selected : 'transparent'} hover={chosen ? theme.selected : theme.hover} onMouseEnter={() => point(element)}>
            <group width={depth * 14 + 4} />
            {has ? (
                <button name={`${opened ? 'Close' : 'Open'} ${id}`} width={14} height={ROW} align="center" justify="center" onClick={() => flip(id)}>
                    <Icon name={opened ? 'down' : 'right'} color={theme.dim} size={10} />
                </button>
            ) : (
                <group width={14} />
            )}
            <button direction="row" justify="start" name={`Element ${id}`} flex={1} height={ROW} align="center" onClick={() => choose(id)} onDoubleClick={() => has && flip(id)}>
                <OpeningTag element={element} theme={theme} closed={closed} />
            </button>
        </row>
    );
}

/// The elements in the app's window as a tree, opened as far as it's been opened, with each row's own element filled in
/// in the app while the pointer's on it.
function ElementTree({ root, open, flip, chosen, choose, point, theme }: { root: El; open: Set<string>; flip: (id: string) => void; chosen: string; choose: (id: string) => void; point: (element: El | null) => void; theme: Theme }) {
    const rows: any[] = [];
    const walk = (element: El, depth: number) => {
        const id = element.path.join('.');
        const has = element.children.length > 0;
        const opened = has && open.has(id);
        rows.push(<TreeRow key={`el-${id}`} element={element} id={id} depth={depth} has={has} opened={opened} chosen={chosen === id} flip={flip} choose={choose} point={point} theme={theme} />);
        if (opened) {
            for (const child of element.children) {
                walk(child, depth + 1);
            }
            if (element.more > 0) {
                rows.push(
                    <row key={`more-${id}`} height={ROW} align="center">
                        <group width={(depth + 1) * 14 + 18} />
                        <text fontSize={12} fontFamily={UI} color={theme.dim}>{`${element.more} more aren't shown`}</text>
                    </row>,
                );
            }
            rows.push(
                <row key={`close-${id}`} height={ROW} align="center" hover={theme.hover}>
                    <group width={depth * 14 + 18} />
                    <text fontSize={12} fontFamily={MONO} color={theme.tag}>{`</${tagOf(element)}>`}</text>
                </row>,
            );
        }
    };
    walk(root, 0);
    return (
        <scroll key="element-tree" flex={1} axis="both" paddingY={4} onMouseLeave={() => point(null)}>
            {rows}
        </scroll>
    );
}

/// The box a browser's inspector draws for an element: its margin, border and padding around what's inside, with how big
/// each is.
function BoxModel({ element, theme }: { element: El; theme: Theme }) {
    const prop = (names: string[]) => {
        for (const name of names) {
            const value = element.props[name];
            if (typeof value === 'number') {
                return value;
            }
        }
        return 0;
    };
    const side = (kind: string, which: string, across: string) => prop([`${kind}_${which}`, `${kind}${which[0].toUpperCase()}${which.slice(1)}`, `${kind}_${across}`, kind]);
    const padding = { top: side('padding', 'top', 'y'), right: side('padding', 'right', 'x'), bottom: side('padding', 'bottom', 'y'), left: side('padding', 'left', 'x') };
    const margin = { top: side('margin', 'top', 'y'), right: side('margin', 'right', 'x'), bottom: side('margin', 'bottom', 'y'), left: side('margin', 'left', 'x') };
    const border = prop(['border_width', 'borderWidth']);
    const [, , width, height] = element.rect ?? [0, 0, 0, 0];
    const inner = `${Math.round(width - padding.left - padding.right - border * 2)} × ${Math.round(height - padding.top - padding.bottom - border * 2)}`;
    const number = (value: number) => (value === 0 ? '-' : String(Math.round(value * 10) / 10));
    const label = (text: string) => <text fontSize={10} fontFamily={UI} color={theme.box}>{text}</text>;
    const ring = (name: string, color: string, sides: { top: number; right: number; bottom: number; left: number }, inside: any) => (
        <column background={color} borderWidth={1} borderColor={theme.strong} paddingX={6} paddingY={2} align="center" minWidth={120}>
            <row width="100%">
                {label(name)}
                <group flex={1} />
            </row>
            {label(number(sides.top))}
            <row align="center" gap={6}>
                {label(number(sides.left))}
                {inside}
                {label(number(sides.right))}
            </row>
            {label(number(sides.bottom))}
        </column>
    );
    return (
        <column align="center" padding={12}>
            {ring(
                'margin',
                theme.margin,
                margin,
                ring(
                    'border',
                    theme.border,
                    { top: border, right: border, bottom: border, left: border },
                    ring(
                        'padding',
                        theme.padding,
                        padding,
                        <column background={theme.content} borderWidth={1} borderColor={theme.strong} paddingX={12} paddingY={6} align="center">
                            {label(element.rect ? inner : 'not laid out')}
                        </column>,
                    ),
                ),
            )}
            {element.rect ? (
                <text fontSize={11} fontFamily={MONO} color={theme.dim} paddingTop={8} selectable>
                    {`at ${Math.round(element.rect[0])}, ${Math.round(element.rect[1])}, ${Math.round(width)} × ${Math.round(height)}`}
                </text>
            ) : null}
        </column>
    );
}

/// What's known about the element chosen: its props, its box, the component that made it, and what it does.
function ElementSidebar({ element, made, theme }: { element: El | null; made: Made | null; theme: Theme }) {
    const [tab, setTab] = useState('Props');
    if (!element) {
        return <Empty text="Choose an element to see its props, its box, and the component that made it." theme={theme} />;
    }
    const props = Object.entries(element.props);
    return (
        <column flex={1}>
            <TabStrip tabs={['Props', 'Layout', 'Component', 'Listeners']} value={tab} onChange={setTab} theme={theme} />
            {tab === 'Props' ? (
                <scroll key="props" flex={1} padding={8} gap={2}>
                    <text fontSize={12} fontFamily={MONO} color={theme.tag} paddingBottom={4}>{`${tagOf(element)} {`}</text>
                    {element.name ? (
                        <row key="prop-name" paddingLeft={16}>
                            <text fontSize={12} fontFamily={MONO} color={theme.attr}>name</text>
                            <text fontSize={12} fontFamily={MONO} color={theme.dim}>{': '}</text>
                            <text flex={1} fontSize={12} fontFamily={MONO} color={theme.str} selectable>{JSON.stringify(element.name)}</text>
                        </row>
                    ) : null}
                    {props.map(([name, value]) => (
                        <row key={`prop-${name}`} paddingLeft={16}>
                            <text fontSize={12} fontFamily={MONO} color={theme.attr}>{camel(name)}</text>
                            <text fontSize={12} fontFamily={MONO} color={theme.dim}>{': '}</text>
                            <text flex={1} fontSize={12} fontFamily={MONO} color={typeof value === 'string' ? theme.str : theme.num} selectable>
                                {typeof value === 'string' ? JSON.stringify(value) : String(value)}
                            </text>
                        </row>
                    ))}
                    <text fontSize={12} fontFamily={MONO} color={theme.tag}>{'}'}</text>
                    {props.length === 0 && !element.name ? <text fontSize={12} fontFamily={UI} color={theme.dim} paddingTop={6}>It has no props of its own.</text> : null}
                </scroll>
            ) : null}
            {tab === 'Layout' ? (
                <scroll key="layout" flex={1}>
                    <BoxModel element={element} theme={theme} />
                </scroll>
            ) : null}
            {tab === 'Component' ? (
                <scroll key="component" flex={1} padding={10} gap={8}>
                    {made === null ? <text fontSize={12} fontFamily={UI} color={theme.dim}>Finding out.</text> : null}
                    {made && made.component ? <text fontSize={14} fontFamily={MONO} color={theme.fn}>{`<${made.component}>`}</text> : null}
                    {made && made.around.length > 0 ? <text fontSize={12} fontFamily={UI} color={theme.dim} selectable>{`In ${made.around.join(' › ')}`}</text> : null}
                    {made && made.why ? (
                        <column gap={4}>
                            <text fontSize={11} fontFamily={UI} fontWeight="bold" color={theme.dim}>WHY IT RENDERED</text>
                            <text fontSize={12} fontFamily={UI} color={theme.text} selectable>{made.why}</text>
                        </column>
                    ) : null}
                    {made && made.hooks.length > 0 ? (
                        <column gap={2}>
                            <text fontSize={11} fontFamily={UI} fontWeight="bold" color={theme.dim}>HOOKS</text>
                            {made.hooks.map((hook, index) => (
                                <text key={`hook-${index}`} fontSize={12} fontFamily={MONO} color={theme.text} selectable>{hook}</text>
                            ))}
                        </column>
                    ) : null}
                    {made && !made.component && !made.why ? (
                        <text fontSize={12} fontFamily={UI} color={theme.dim}>What made it hasn't rendered since the app started keeping that. Use the app, and it shows here.</text>
                    ) : null}
                </scroll>
            ) : null}
            {tab === 'Listeners' ? (
                <scroll key="listeners" flex={1} padding={10} gap={4}>
                    {element.actions.length === 0 ? <text fontSize={12} fontFamily={UI} color={theme.dim}>Nothing here listens for anything.</text> : null}
                    {element.actions.map((action) => (
                        <row key={`action-${action}`} gap={6} align="center">
                            <Icon name="right" color={theme.dim} size={10} />
                            <text fontSize={12} fontFamily={MONO} color={theme.text}>{action}</text>
                        </row>
                    ))}
                </scroll>
            ) : null}
        </column>
    );
}

function findElement(root: El | null, id: string): El | null {
    if (!root) {
        return null;
    }
    if (root.path.join('.') === id) {
        return root;
    }
    for (const child of root.children) {
        const found = findElement(child, id);
        if (found) {
            return found;
        }
    }
    return null;
}

function Elements({ ask, theme, gone }: { ask: Ask; theme: Theme; gone: boolean }) {
    const [root, setRoot] = useState<El | null>(null);
    const [cut, setCut] = useState(false);
    const [said, setSaid] = useState<string | null>(null);
    const [open, setOpen] = useState<Set<string>>(new Set(['', '0', '0.0']));
    const [chosen, setChosen] = useState('');
    const [made, setMade] = useState<Made | null>(null);
    const [side, setSide] = useState(360);
    useEffect(() => {
        let stopped = false;
        let timer: any = null;
        let last = '';
        const again = async () => {
            const answer = await ask({ ask: 'tree' });
            if (stopped) {
                return;
            }
            if (answer.error) {
                setSaid(answer.error);
            } else {
                // Only what's changed is shown again: the same tree keeps every row as it was.
                const now = JSON.stringify(answer.tree);
                if (now !== last) {
                    last = now;
                    setRoot(answer.tree);
                    setCut(answer.cut);
                }
                setSaid(null);
            }
            timer = setTimeout(again, TREE_EVERY);
        };
        if (!gone) {
            again();
        }
        return () => {
            stopped = true;
            clearTimeout(timer);
            ask({ ask: 'highlight' });
        };
    }, [gone]);
    const element = findElement(root, chosen);
    useEffect(() => {
        setMade(null);
        if (element && !gone) {
            ask({ ask: 'element', path: element.path }).then((answer) => setMade(answer.error ? null : (answer as any)));
        }
    }, [chosen]);
    const point = useCallback(
        (at: El | null) => {
            if (!gone) {
                ask(at ? { ask: 'highlight', path: at.path, name: tagOf(at) } : { ask: 'highlight' });
            }
        },
        [gone],
    );
    const flip = useCallback((id: string) => {
        setOpen((was: Set<string>) => {
            const next = new Set(was);
            if (next.has(id)) {
                next.delete(id);
            } else {
                next.add(id);
            }
            return next;
        });
    }, []);
    // Where the chosen element is, as tags from the window in.
    const crumbs: El[] = [];
    let at: El | null = root;
    const steps = chosen === '' ? [] : chosen.split('.');
    if (at) {
        crumbs.push(at);
        for (const step of steps) {
            at = at?.children[Number(step)] ?? null;
            if (at) {
                crumbs.push(at);
            }
        }
    }
    return (
        <row flex={1}>
            <column flex={1}>
                {root ? (
                    <ElementTree root={root} open={open} flip={flip} chosen={chosen} choose={setChosen} point={point} theme={theme} />
                ) : (
                    <Empty text={said ?? (gone ? 'The app has stopped.' : 'Asking the app what its window has in it.')} theme={theme} />
                )}
                {cut ? <text fontSize={11} fontFamily={UI} color={theme.dim} paddingX={8}>The window has more than can be shown at once, so only the first part is.</text> : null}
                <Across theme={theme} />
                <row height={24} align="center" paddingX={4} gap={2} background={theme.bar}>
                    {crumbs.map((crumb, index) => (
                        <button key={`crumb-${index}`} name={`Crumb ${crumb.path.join('.')}`} onClick={() => setChosen(crumb.path.join('.'))} direction="row" paddingX={6} height={20} radius={3} align="center" hover={theme.hover} background={index === crumbs.length - 1 ? theme.chosen : 'transparent'}>
                            <text fontSize={11} fontFamily={MONO} color={index === crumbs.length - 1 ? theme.tag : theme.dim} noWrap>
                                {`${tagOf(crumb)}${crumb.key && !/^\d+$/.test(crumb.key) ? `#${crumb.key}` : ''}`}
                            </text>
                        </button>
                    ))}
                </row>
            </column>
            <Splitter width={side} setWidth={setSide} least={240} most={640} theme={theme} fromRight />
            <column width={side}>
                <ElementSidebar element={element} made={made} theme={theme} />
            </column>
        </row>
    );
}

// Console.

type Worked = { after: number; input: string; output: string; threw: boolean };

/// What a line that's been said is, for how it's shown.
function kindOf(line: string): string {
    if (line.startsWith('logpoint ')) {
        return 'logpoint';
    }
    if (/^(breakpoint|paused|step) at /.test(line)) {
        return 'debugger';
    }
    if (/^(Uncaught|error|Error|TypeError|ReferenceError|RangeError|SyntaxError)\b/.test(line) || line.includes(' threw ')) {
        return 'error';
    }
    if (/^(warning|Warning|rsui\/web:)/.test(line)) {
        return 'warn';
    }
    return 'log';
}

function ConsoleLine({ text, kind, theme, prefix }: { text: string; kind: string; theme: Theme; prefix?: string }) {
    const back = kind === 'error' ? theme.errorBack : kind === 'warn' ? theme.warnBack : 'transparent';
    const color = kind === 'error' ? theme.error : kind === 'warn' ? theme.warn : kind === 'logpoint' ? theme.logpoint : kind === 'debugger' ? theme.accent : kind === 'input' ? theme.accent : theme.text;
    return (
        <column background={back}>
            <row paddingX={8} paddingY={3} gap={6}>
                <text width={12} fontSize={12} fontFamily={MONO} color={theme.dim}>{prefix ?? ''}</text>
                <text flex={1} fontSize={12} fontFamily={MONO} color={color} selectable>{text}</text>
            </row>
            <group height={1} background={kind === 'error' || kind === 'warn' ? back : theme.line} />
        </column>
    );
}

function Console({ heard, worked, run, clear, cleared, stopped, theme }: { heard: Heard[]; worked: Worked[]; run: (expression: string) => void; clear: () => void; cleared: number; stopped: Stopped | null; theme: Theme }) {
    const [typed, setTyped] = useState('');
    const [filter, setFilter] = useState('');
    const [history, setHistory] = useState<string[]>([]);
    const [back, setBack] = useState(-1);
    const shown = heard.filter((one) => one.number > cleared && (filter === '' || one.line.toLowerCase().includes(filter.toLowerCase()))).slice(-MOST_LISTED);
    const items: any[] = [];
    const evaluations = worked.filter((one) => one.after >= cleared);
    let next = 0;
    const evaluated = (upTo: number) => {
        while (next < evaluations.length && evaluations[next].after <= upTo) {
            const one = evaluations[next];
            items.push(<ConsoleLine key={`in-${next}`} text={one.input} kind="input" prefix="›" theme={theme} />);
            items.push(<ConsoleLine key={`out-${next}`} text={one.output} kind={one.threw ? 'error' : 'log'} prefix="‹" theme={theme} />);
            next += 1;
        }
    };
    for (const one of shown) {
        evaluated(one.number - 1);
        items.push(<ConsoleLine key={`heard-${one.number}`} text={one.line} kind={kindOf(one.line)} theme={theme} />);
    }
    evaluated(Number.MAX_SAFE_INTEGER);
    const submit = () => {
        const expression = typed.trim();
        if (expression === '') {
            return;
        }
        run(expression);
        setHistory([...history, expression].slice(-50));
        setBack(-1);
        setTyped('');
    };
    return (
        <column flex={1}>
            <Toolbar theme={theme}>
                <IconButton name="Clear the console" icon="clear" onClick={clear} theme={theme} />
                <Down theme={theme} />
                <group width={4} />
                <Field name="Filter" value={filter} onChange={setFilter} placeholder="Filter" width={220} theme={theme} />
                <group flex={1} />
                <text fontSize={12} fontFamily={UI} color={stopped ? theme.paused : theme.dim} paddingRight={8}>
                    {stopped ? `Working things out in ${stopped.frames[0]?.function || 'the function'} at ${stopped.at}` : 'Working things out at the top of the app'}
                </text>
            </Toolbar>
            <scroll key="console" flex={1}>
                {items.length === 0 ? <text fontSize={12} fontFamily={UI} color={theme.dim} padding={10}>What console.log and logpoints say shows here.</text> : null}
                {items}
                <row paddingX={8} paddingY={4} gap={6} align="center">
                    <text width={12} fontSize={13} fontFamily={MONO} color={theme.accent}>›</text>
                    <TextInput
                        key="console-prompt"
                        name="Console prompt"
                        flex={1}
                        value={typed}
                        onChange={setTyped}
                        onKeyPress={(key: string) => {
                            if (key === 'enter') {
                                submit();
                                return true;
                            }
                            if (key === 'up' && history.length > 0) {
                                const at = back < 0 ? history.length - 1 : Math.max(0, back - 1);
                                setBack(at);
                                setTyped(history[at]);
                                return true;
                            }
                            if (key === 'down' && back >= 0) {
                                const at = back + 1;
                                setBack(at >= history.length ? -1 : at);
                                setTyped(at >= history.length ? '' : history[at]);
                                return true;
                            }
                            return false;
                        }}
                        fontSize={12}
                        fontFamily={MONO}
                        paddingX={0}
                        paddingY={2}
                        borderWidth={0}
                        background="transparent"
                        color={theme.text}
                    />
                </row>
            </scroll>
        </column>
    );
}

// Sources.

const KEYWORDS = new Set(
    'const let var function return if else for while do switch case break continue new class extends import export from default async await try catch finally throw typeof instanceof in of this null undefined true false yield delete void super static get set as type interface'.split(' '),
);

/// A line of code in pieces, each with what it is, carrying on from a comment the line before left open.
function tokens(line: string, inComment: boolean): { pieces: [string, string][]; inComment: boolean } {
    const pieces: [string, string][] = [];
    let at = 0;
    const push = (kind: string, text: string) => {
        if (text !== '') {
            pieces.push([kind, text]);
        }
    };
    while (at < line.length) {
        if (inComment) {
            const end = line.indexOf('*/', at);
            const stop = end < 0 ? line.length : end + 2;
            push('comment', line.slice(at, stop));
            at = stop;
            inComment = end < 0;
            continue;
        }
        const rest = line.slice(at);
        if (rest.startsWith('//')) {
            push('comment', rest);
            break;
        }
        if (rest.startsWith('/*')) {
            inComment = true;
            continue;
        }
        const c = line[at];
        if (c === '"' || c === "'" || c === '`') {
            let end = at + 1;
            while (end < line.length && line[end] !== c) {
                end += line[end] === '\\' ? 2 : 1;
            }
            push('str', line.slice(at, end + 1));
            at = end + 1;
            continue;
        }
        if (/[0-9]/.test(c) && (at === 0 || !/[\w$]/.test(line[at - 1]))) {
            const match = /^[0-9][0-9_.xXa-fA-FeEn]*/.exec(rest);
            push('num', match ? match[0] : c);
            at += match ? match[0].length : 1;
            continue;
        }
        if (/[A-Za-z_$]/.test(c)) {
            const match = /^[\w$]+/.exec(rest);
            const word = match ? match[0] : c;
            const before = line.slice(0, at).trimEnd();
            const kind = KEYWORDS.has(word) ? 'kw' : before.endsWith('<') || before.endsWith('</') ? 'tag' : rest.slice(word.length).trimStart().startsWith('(') || /^[A-Z]/.test(word) ? 'fn' : 'plain';
            push(kind, word);
            at += word.length;
            continue;
        }
        const match = /^[^\w$"'`/0-9]+|^\//.exec(rest);
        const text = match ? match[0] : c;
        push('plain', text);
        at += text.length;
    }
    return { pieces, inComment };
}

/// The app's own source files, from its folder in, but not what's in packages or what's built.
function sourceFiles(folder: string): string[] {
    const found: string[] = [];
    const walk = (dir: string, from: string, depth: number) => {
        let entries: any[] = [];
        try {
            entries = readdirSync(dir, { withFileTypes: true });
        } catch {
            return;
        }
        for (const entry of entries) {
            const name = entry.name;
            if (name.startsWith('.') || name === 'node_modules' || name === 'target' || name === 'dist' || name === 'build') {
                continue;
            }
            if (entry.isDirectory()) {
                if (depth < 4) {
                    walk(`${dir}\\${name}`, `${from}${name}/`, depth + 1);
                }
            } else if (/\.(tsx|ts|jsx|js|mjs|cjs|css)$/.test(name)) {
                found.push(`${from}${name}`);
            }
            if (found.length >= 400) {
                return;
            }
        }
    };
    walk(folder, '', 0);
    return found.sort((a, b) => (a.includes('/') === b.includes('/') ? a.localeCompare(b) : a.includes('/') ? 1 : -1));
}

function placeOf(at: string): [string, number] {
    const cut = at.lastIndexOf(':');
    return [at.slice(0, cut), Number(at.slice(cut + 1))];
}

/// What each kind of piece of code is colored.
function colorOf(kind: string, theme: Theme): string {
    return kind === 'kw' ? theme.kw : kind === 'str' ? theme.str : kind === 'num' ? theme.num : kind === 'comment' ? theme.comment : kind === 'fn' ? theme.fn : kind === 'tag' ? theme.tag : theme.text;
}

/// A line of code, with its number, which is where a breakpoint goes.
function CodeLine({ number, pieces, marked, logs, here, gutter, toggle, menu, theme }: { number: number; pieces: [string, string][]; marked: boolean; logs: boolean; here: boolean; gutter: number; toggle: (line: number, marked: boolean) => void; menu: (line: number) => void; theme: Theme }) {
    return (
        <row height={18} align="center" background={here ? theme.exec : 'transparent'}>
            <button
                key={`src-line-${number}`}
                name={`Line ${number}`}
                tooltip={marked ? 'Take the breakpoint away' : 'Stop here. Right-click for a logpoint'}
                onClick={() => toggle(number, marked)}
                onContextMenu={() => menu(number)}
                direction="row"
                width={gutter}
                height={18}
                align="center"
                justify="end"
                paddingRight={8}
                hover={theme.hover}
            >
                <text fontSize={12} fontFamily={MONO} paddingX={4} radius={2} background={marked ? theme.breakpoint : logs ? theme.logpoint : 'transparent'} color={marked || logs ? '#ffffff' : theme.dim}>
                    {String(number)}
                </text>
            </button>
            <text fontSize={12} fontFamily={MONO} noWrap paddingLeft={4}>
                {pieces.length === 0 ? ' ' : null}
                {pieces.map(([kind, piece], at) => (
                    <span key={`piece-${at}`} color={colorOf(kind, theme)}>{piece}</span>
                ))}
            </text>
        </row>
    );
}

function CodeView({ file, folder, breakpoints, logpoints, stopped, frame, ask, onSaid, theme }: { file: string; folder: string; breakpoints: Breakpoint[]; logpoints: Logpoint[]; stopped: Stopped | null; frame: number; ask: Ask; onSaid: (said: string | null) => void; theme: Theme }) {
    const [editing, setEditing] = useState<number | null>(null);
    const [expression, setExpression] = useState('');
    const text = useMemo(() => {
        try {
            return readFileSync(`${folder}\\${file.replace(/\//g, '\\')}`, 'utf8') as string;
        } catch (error) {
            return `// ${file} couldn't be read: ${error}`;
        }
    }, [file, folder, stopped?.at]);
    const lines = useMemo(() => text.replace(/\r/g, '').split('\n'), [text]);
    const coloured = useMemo(() => {
        let inComment = false;
        return lines.map((line) => {
            const done = tokens(line, inComment);
            inComment = done.inComment;
            return done.pieces;
        });
    }, [lines]);
    const marks = new Set(breakpoints.map((one) => one.at));
    const logged = new Set(logpoints.map((one) => one.at));
    const chosenFrame = stopped?.frames[frame];
    const [stoppedFile, stoppedLine] = chosenFrame ? placeOf(chosenFrame.at) : ['', 0];
    const here = stoppedFile === file ? stoppedLine : 0;
    // The line it's stopped at is brought into view, with some of what's around it.
    useEffect(() => {
        if (here > 0) {
            focus(`src-line-${Math.max(1, here - 6)}`);
            focus(`src-line-${Math.min(lines.length, here + 8)}`);
        }
    }, [here, file, stopped?.at]);
    const toggle = useCallback(
        async (line: number, marked: boolean) => {
            const at = `${file}:${line}`;
            const answer = await ask(marked ? { ask: 'breakpoint', at, take: true } : { ask: 'breakpoint', at });
            onSaid(answer.error ?? null);
        },
        [file],
    );
    const menu = useCallback((line: number) => {
        setEditing((was: number | null) => (was === line ? null : line));
        setExpression('');
    }, []);
    const addLogpoint = async (line: number) => {
        const at = `${file}:${line}`;
        const answer = await ask({ ask: 'logpoint', at, expression: expression.trim() });
        onSaid(answer.error ?? null);
        if (!answer.error) {
            setEditing(null);
            setExpression('');
        }
    };
    const gutter = String(lines.length).length * 8 + 22;
    return (
        <scroll key={`code-${file}`} flex={1} axis="both" paddingY={4}>
            {lines.map((line, index) => {
                const number = index + 1;
                const at = `${file}:${number}`;
                return [
                    <CodeLine key={`line-${number}`} number={number} pieces={coloured[index]} marked={marks.has(at)} logs={logged.has(at)} here={number === here} gutter={gutter} toggle={toggle} menu={menu} theme={theme} />,
                    editing === number ? (
                            <row key={`logpoint-row-${number}`} paddingLeft={gutter} paddingY={4} paddingRight={8} gap={6} align="center" background={theme.bar}>
                                <text fontSize={12} fontFamily={UI} color={theme.logpoint}>Logpoint</text>
                                <TextInput
                                    key={`logpoint-${number}`}
                                    name="Logpoint expression"
                                    autoFocus
                                    flex={1}
                                    value={expression}
                                    onChange={setExpression}
                                    placeholder="What to say each time this line runs, as items.length"
                                    onKeyPress={(key: string) => {
                                        if (key === 'enter' && expression.trim() !== '') {
                                            addLogpoint(number);
                                            return true;
                                        }
                                        if (key === 'escape') {
                                            setEditing(null);
                                            return true;
                                        }
                                        return false;
                                    }}
                                    fontSize={12}
                                    fontFamily={MONO}
                                    paddingX={6}
                                    paddingY={3}
                                    radius={3}
                                    background={theme.field}
                                    color={theme.text}
                                    borderColor={theme.logpoint}
                                />
                            </row>
                        ) : null,
                ];
            })}
        </scroll>
    );
}

function Sources({ app, news, ask, frame, setFrame, wanted, theme, gone }: { app: Running; news: News; ask: Ask; frame: number; setFrame: (frame: number) => void; wanted: string | null; theme: Theme; gone: boolean }) {
    const folder = app.file.slice(0, app.file.lastIndexOf('\\'));
    const main = app.file.slice(folder.length + 1);
    const files = useMemo(() => sourceFiles(folder), [folder]);
    const [file, setFile] = useState(wanted ?? main);
    const [left, setLeft] = useState(220);
    const [right, setRight] = useState(320);
    const [said, setSaid] = useState<string | null>(null);
    const [open, setOpen] = useState<Record<string, boolean>>({ watch: true, breakpoints: true, scope: true, stack: true });
    const [watches, setWatches] = useState<string[]>([]);
    const [watching, setWatching] = useState('');
    const [watched, setWatched] = useState<Record<string, { value: string; threw: boolean }>>({});
    const stopped = gone ? null : news.paused;
    useEffect(() => {
        if (wanted) {
            setFile(wanted);
        }
    }, [wanted]);
    // What's watched is worked out again each time it stops, in the function chosen.
    useEffect(() => {
        if (!stopped) {
            return;
        }
        Promise.all(watches.map((one) => ask({ ask: 'evaluate', frame, expression: one }))).then((answers) => {
            const next: Record<string, { value: string; threw: boolean }> = {};
            answers.forEach((answer, index) => {
                next[watches[index]] = { value: String(answer.error ?? answer.value ?? ''), threw: Boolean(answer.error || answer.threw) };
            });
            setWatched(next);
        });
    }, [stopped?.at, frame, watches.join('\n')]);
    const toggle = (name: string) => setOpen({ ...open, [name]: !open[name] });
    const resume = async (how: string) => {
        const answer = await ask({ ask: 'resume', how });
        setSaid(answer.error ?? null);
    };
    const chosen = stopped?.frames[Math.min(frame, (stopped?.frames.length ?? 1) - 1)];
    const reason = stopped ? (stopped.reason === 'breakpoint' ? 'Paused on breakpoint' : stopped.reason === 'step' ? 'Paused after a step' : 'Paused') : null;
    const note = (text: string) => <text fontSize={12} fontFamily={UI} color={theme.dim} paddingX={12} paddingY={4}>{text}</text>;
    return (
        <row flex={1}>
            <column width={left}>
                <Toolbar theme={theme}>
                    <text fontSize={12} fontFamily={UI} color={theme.text} paddingX={6}>Files</text>
                </Toolbar>
                <scroll key="files" flex={1} paddingY={4}>
                    {files.map((one) => {
                        const depth = one.split('/').length - 1;
                        return (
                            <button direction="row" justify="start" key={`file-${one}`} name={`File ${one}`} onClick={() => setFile(one)} height={ROW} align="center" gap={6} paddingLeft={8 + depth * 12} paddingRight={8} background={one === file ? theme.selected : 'transparent'} hover={one === file ? theme.selected : theme.hover}>
                                <Icon name="file" color={one.endsWith('.css') ? theme.kw : theme.tag} size={14} />
                                <text fontSize={12} fontFamily={UI} color={theme.text} noWrap ellipsis>{one.slice(one.lastIndexOf('/') + 1)}</text>
                            </button>
                        );
                    })}
                    {files.length === 0 ? note('No source files were found next to the app.') : null}
                </scroll>
            </column>
            <Splitter width={left} setWidth={setLeft} least={140} most={420} theme={theme} />
            <column flex={1}>
                <column background={theme.bar}>
                    <row height={28} align="center" paddingX={4}>
                        <row height={28} paddingX={10} align="center" gap={6} background={theme.page}>
                            <Icon name="file" color={theme.tag} size={14} />
                            <text fontSize={12} fontFamily={UI} color={theme.text} noWrap>{file}</text>
                        </row>
                        <group flex={1} />
                        {said ? <text fontSize={12} fontFamily={UI} color={theme.error} paddingRight={8} selectable>{said}</text> : null}
                    </row>
                    <Across theme={theme} />
                </column>
                <CodeView file={file} folder={folder} breakpoints={news.breakpoints} logpoints={news.logpoints} stopped={stopped} frame={frame} ask={ask} onSaid={setSaid} theme={theme} />
            </column>
            <Splitter width={right} setWidth={setRight} least={220} most={560} theme={theme} fromRight />
            <column width={right}>
                <Toolbar theme={theme}>
                    {stopped ? (
                        <IconButton name="Resume" icon="resume" onClick={() => resume('continue')} theme={theme} color={theme.accent} />
                    ) : (
                        <IconButton name="Pause the app" icon="pause" onClick={() => ask({ ask: 'pause' }).then((answer) => setSaid(answer.error ?? null))} theme={theme} disabled={gone} />
                    )}
                    <IconButton name="Step over" icon="over" onClick={() => resume('over')} theme={theme} disabled={!stopped} color={stopped ? theme.accent : theme.strong} />
                    <IconButton name="Step into" icon="into" onClick={() => resume('into')} theme={theme} disabled={!stopped} color={stopped ? theme.accent : theme.strong} />
                    <IconButton name="Step out" icon="out" onClick={() => resume('out')} theme={theme} disabled={!stopped} color={stopped ? theme.accent : theme.strong} />
                </Toolbar>
                <scroll key="debugger" flex={1}>
                    {reason ? (
                        <column background={theme.pausedBack} paddingX={10} paddingY={8} gap={2}>
                            <text fontSize={12} fontFamily={UI} fontWeight="bold" color={theme.paused}>{reason}</text>
                            <text fontSize={12} fontFamily={UI} color={theme.text} selectable>{`${stopped?.function || 'a function with no name'}, ${stopped?.at}`}</text>
                        </column>
                    ) : null}
                    <Section title="Watch" open={open.watch} onToggle={() => toggle('watch')} theme={theme}>
                        {watches.map((one) => (
                            <row key={`watch-${one}`} height={ROW} align="center" paddingLeft={12} paddingRight={2} hover={theme.hover}>
                                <text fontSize={12} fontFamily={MONO} color={theme.attr} noWrap>{one}</text>
                                <text fontSize={12} fontFamily={MONO} color={theme.dim}>{': '}</text>
                                <text flex={1} fontSize={12} fontFamily={MONO} color={watched[one]?.threw ? theme.error : theme.text} noWrap ellipsis selectable>
                                    {stopped ? watched[one]?.value ?? '…' : '<not available>'}
                                </text>
                                <IconButton name={`Stop watching ${one}`} icon="close" onClick={() => setWatches(watches.filter((other) => other !== one))} theme={theme} />
                            </row>
                        ))}
                        <row paddingX={8} paddingY={4}>
                            <Field
                                name="Watch expression"
                                value={watching}
                                onChange={setWatching}
                                placeholder="Add an expression to watch"
                                flex={1}
                                mono
                                theme={theme}
                                onKeyPress={(key: string) => {
                                    if (key === 'enter' && watching.trim() !== '') {
                                        setWatches([...watches.filter((one) => one !== watching.trim()), watching.trim()]);
                                        setWatching('');
                                        return true;
                                    }
                                    return false;
                                }}
                            />
                        </row>
                    </Section>
                    <Section title="Breakpoints" open={open.breakpoints} onToggle={() => toggle('breakpoints')} theme={theme}>
                        {news.breakpoints.length === 0 && news.logpoints.length === 0 ? note("Click a line's number to stop there, or right-click it for a logpoint.") : null}
                        {news.breakpoints.map((one) => (
                            <row key={`bp-${one.at}`} height={ROW + 2} align="center" paddingLeft={12} gap={6} hover={theme.hover}>
                                <group width={10} height={10} radius={2} background={theme.breakpoint} />
                                <button direction="row" justify="start" name={`Show ${one.at}`} flex={1} height={ROW} align="center" onClick={() => setFile(placeOf(one.at)[0])}>
                                    <text fontSize={12} fontFamily={UI} color={theme.text} noWrap ellipsis>{one.at}</text>
                                </button>
                                <text fontSize={11} fontFamily={UI} color={theme.dim}>{one.hits === 0 ? '' : one.hits === 1 ? 'stopped once' : `stopped ${one.hits}×`}</text>
                                <IconButton name={`Take away the breakpoint at ${one.at}`} icon="close" onClick={() => ask({ ask: 'breakpoint', at: one.at, take: true })} theme={theme} />
                            </row>
                        ))}
                        {news.logpoints.map((one) => (
                            <row key={`lp-${one.at}-${one.expression}`} height={ROW + 2} align="center" paddingLeft={12} gap={6} hover={theme.hover}>
                                <group width={10} height={10} radius={2} background={theme.logpoint} />
                                <button direction="row" justify="start" name={`Show ${one.at}`} flex={1} height={ROW} align="center" onClick={() => setFile(placeOf(one.at)[0])}>
                                    <text fontSize={12} fontFamily={UI} color={theme.text} noWrap ellipsis>
                                        {`${one.at}  `}
                                        <span color={theme.logpoint}>{one.expression}</span>
                                    </text>
                                </button>
                                <text fontSize={11} fontFamily={UI} color={theme.dim}>{one.hits === 0 ? '' : `ran ${one.hits}×`}</text>
                                <IconButton name={`Take away the logpoint at ${one.at}`} icon="close" onClick={() => ask({ ask: 'logpoint', at: one.at })} theme={theme} />
                            </row>
                        ))}
                    </Section>
                    <Section title="Scope" open={open.scope} onToggle={() => toggle('scope')} theme={theme}>
                        {!chosen ? note('Not paused') : null}
                        {chosen && chosen.locals.length === 0 ? note(chosen.debugged ? 'Nothing in its variables here.' : "It's not one of the app's own files, so its variables aren't known.") : null}
                        {chosen?.locals.map((local) => (
                            <row key={`local-${local.name}`} paddingLeft={12} paddingRight={8} paddingY={1} hover={theme.hover}>
                                <text fontSize={12} fontFamily={MONO} color={local.name === 'this' ? theme.kw : theme.attr} noWrap>{local.name}</text>
                                <text fontSize={12} fontFamily={MONO} color={theme.dim}>{': '}</text>
                                <text flex={1} fontSize={12} fontFamily={MONO} color={/^-?[\d.]+$/.test(local.value) ? theme.num : /^['"]/.test(local.value) ? theme.str : theme.text} selectable>
                                    {local.value}
                                </text>
                            </row>
                        ))}
                    </Section>
                    <Section title="Call Stack" open={open.stack} onToggle={() => toggle('stack')} theme={theme}>
                        {!stopped ? note('Not paused') : null}
                        {stopped?.frames.map((one, index) => (
                            <button
                                key={`stack-${index}`}
                                name={`Call ${index}`}
                                direction="row" justify="start"
                                onClick={() => {
                                    setFrame(index);
                                    setFile(placeOf(one.at)[0]);
                                }}
                                height={ROW + 2}
                                align="center"
                                paddingLeft={6}
                                paddingRight={8}
                                gap={4}
                                background={index === frame ? theme.selected : 'transparent'}
                                hover={index === frame ? theme.selected : theme.hover}
                            >
                                <group width={12}>{index === frame ? <Icon name="right" color={theme.accent} size={12} /> : null}</group>
                                <text flex={1} fontSize={12} fontFamily={UI} color={one.debugged ? theme.text : theme.dim} noWrap ellipsis>{one.function || '(anonymous)'}</text>
                                <text fontSize={11} fontFamily={UI} color={theme.dim} noWrap>{one.at}</text>
                            </button>
                        ))}
                    </Section>
                </scroll>
            </column>
        </row>
    );
}

// Performance.

/// Each frame as a bar, its height how long it took and split into where the time went, with a line at the time there is
/// for one. The newest are on the right.
function FrameStrip({ frames, chosen, choose, theme }: { frames: Frame[]; chosen: number | undefined; choose: (number: number) => void; theme: Theme }) {
    const height = 80;
    // A frame much longer than the rest, as one the app was stopped in, doesn't flatten the rest: it's cut off.
    const tallest = Math.min(BUDGET * 6, Math.max(BUDGET * 2, ...frames.map(total)));
    const scale = (height - 6) / tallest;
    return (
        <column background={theme.page}>
            <row height={height} align="end" paddingX={8} gap={1}>
                {frames.map((frame) => (
                    <button key={`bar-${frame.number}`} name={`Frame ${frame.number}`} onClick={() => choose(frame.number)} width={6} height={height} direction="column" justify="end" background={frame.number === chosen ? theme.chosen : 'transparent'} hover={theme.hover}>
                        {PARTS.slice()
                            .reverse()
                            .map(([part, , color]) => {
                                const tall = Math.min(height, Math.max(0, (frame[part] as number) * scale));
                                return tall >= 0.5 ? <group key={`${part}`} width={6} height={tall} background={theme[color]} /> : null;
                            })}
                        {total(frame) * scale < 1 ? <group width={6} height={1} background={theme.strong} /> : null}
                    </button>
                ))}
                <group flex={1} />
            </row>
            <group position="absolute" left={0} right={0} top={height - BUDGET * scale} height={1} background={theme.error} opacity={0.6} />
            <Across theme={theme} />
        </column>
    );
}

function FrameDetail({ frame, theme }: { frame: Frame; theme: Theme }) {
    const took = total(frame);
    return (
        <column gap={12} padding={14}>
            <row gap={10} align="center">
                <text fontSize={14} fontFamily={UI} fontWeight="bold" color={theme.text}>{`Frame ${frame.number}`}</text>
                <text fontSize={12} fontFamily={UI} color={theme.dim}>{`at ${frame.at < 1000 ? `${Math.round(frame.at)} ms` : `${(frame.at / 1000).toFixed(2)} s`}`}</text>
                <text fontSize={12} fontFamily={UI} color={took > BUDGET ? theme.error : theme.ok}>{`${ms(took)}${took > BUDGET ? `, over the ${BUDGET.toFixed(1)} ms there is` : ''}`}</text>
            </row>
            <row height={10} radius={2} width={400} background={theme.bar}>
                {PARTS.map(([part, , color]) => {
                    const share = took > 0 ? (frame[part] as number) / took : 0;
                    return share > 0 ? <group key={`share-${part}`} width={Math.max(1, share * 400)} height={10} background={theme[color]} /> : null;
                })}
            </row>
            <row gap={18}>
                {PARTS.map(([part, label, color]) => (
                    <row key={`legend-${part}`} gap={6} align="center">
                        <group width={10} height={10} radius={2} background={theme[color]} />
                        <text fontSize={12} fontFamily={UI} color={theme.text}>{`${label} ${ms(frame[part] as number)}`}</text>
                    </row>
                ))}
            </row>
            {(frame.present ?? 0) >= 0.1 ? <text fontSize={12} fontFamily={UI} color={theme.dim}>{`Then ${ms(frame.present ?? 0)} waiting for the screen, which isn't counted.`}</text> : null}
            {frame.handled.length > 0 ? (
                <column gap={4}>
                    <text fontSize={11} fontFamily={UI} fontWeight="bold" color={theme.dim}>WHAT CAME IN</text>
                    {frame.handled.map((handled, index) => (
                        <text key={`handled-${index}`} fontSize={12} fontFamily={UI} color={theme.text} selectable>
                            {`${handled.action} on #${handled.key} ran ${handled.note ? handled.note + ' ' : ''}for ${ms(handled.took)}${handled.rejected ? ', and was turned down' : ''}`}
                        </text>
                    ))}
                </column>
            ) : null}
            <column>
                <Heading columns={['Component', 'Self time', 'Total time', 'Why']} widths={[220, 90, 90, 0]} theme={theme} />
                {frame.renders.length === 0 ? <text fontSize={12} fontFamily={UI} color={theme.dim} padding={8}>Nothing rendered.</text> : null}
                {frame.renders.map((render, index) => (
                    <column key={`render-${index}`}>
                        <row height={ROW + 2} align="center" hover={theme.hover}>
                            <row width={220} align="center" paddingLeft={8 + Math.min(render.depth, 8) * 10} gap={6}>
                                <group width={8} height={8} radius={4} background={render.changed ? theme.accent : theme.warn} />
                                <text flex={1} fontSize={12} fontFamily={MONO} color={theme.fn} noWrap ellipsis>{render.name ?? 'a component'}</text>
                            </row>
                            <Cell text={ms(render.own)} width={90} theme={theme} />
                            <Cell text={ms(render.took)} width={90} theme={theme} />
                            <Cell text={render.changed ? render.why ?? render.cause : `changed nothing. ${render.why ?? ''}`} width={0} theme={theme} color={render.changed ? theme.text : theme.warn} />
                        </row>
                        <Across theme={theme} />
                    </column>
                ))}
            </column>
        </column>
    );
}

function Performance({ frames, live, summary, theme }: { frames: Frame[]; live: boolean; summary: () => Promise<string>; theme: Theme }) {
    const busy = useMemo(() => frames.filter((frame) => frame.renders.length > 0 || frame.handled.length > 0), [frames]);
    const [chosen, setChosen] = useState<number | null>(live ? null : busy.length > 0 ? busy[busy.length - 1].number : null);
    const [text, setText] = useState<string | null>(null);
    const shown = chosen ?? busy[busy.length - 1]?.number;
    const frame = busy.find((one) => one.number === shown);
    const renders = frames.reduce((sum, one) => sum + one.renders.length, 0);
    const wasted = frames.reduce((sum, one) => sum + one.renders.filter((render) => !render.changed).length, 0);
    const over = frames.filter((one) => total(one) > BUDGET).length;
    return (
        <column flex={1}>
            <Toolbar theme={theme}>
                <text fontSize={12} fontFamily={UI} color={theme.dim} paddingX={6}>
                    {`${plural(frames.length, 'frame')} · ${plural(renders, 'render')} · ${wasted} changing nothing · ${over} over budget`}
                </text>
                <group flex={1} />
                {live && chosen !== null ? <Action label="Show the newest" onClick={() => setChosen(null)} theme={theme} /> : null}
                <Action label={text === null ? 'Summary' : 'Frames'} onClick={async () => setText(text === null ? await summary() : null)} theme={theme} />
            </Toolbar>
            {text !== null ? (
                <scroll key="summary" flex={1} padding={12}>
                    <text fontSize={12} fontFamily={MONO} color={theme.text} selectable>{text || 'Working it out.'}</text>
                </scroll>
            ) : (
                <column flex={1}>
                    <FrameStrip frames={busy.slice(-MOST_LISTED)} chosen={shown} choose={setChosen} theme={theme} />
                    <scroll key="frame" flex={1}>
                        {frame ? <FrameDetail frame={frame} theme={theme} /> : <Empty text={live ? 'Nothing has happened yet. Use the app, and each frame where something did shows here.' : 'Nothing happened in what was kept.'} theme={theme} />}
                    </scroll>
                </column>
            )}
        </column>
    );
}

/// Every component that rendered, with how long it took and how often it rendered for nothing, as a profiler shows it.
function Rendering({ frames, theme }: { frames: Frame[]; theme: Theme }) {
    const [sorted, setSorted] = useState(1);
    const rows = useMemo(() => {
        const by = new Map<string, { renders: number; wasted: number; own: number; took: number }>();
        for (const frame of frames) {
            for (const render of frame.renders) {
                const name = render.name ?? 'a component';
                const kept = by.get(name) ?? { renders: 0, wasted: 0, own: 0, took: 0 };
                kept.renders += 1;
                kept.wasted += render.changed ? 0 : 1;
                kept.own += render.own;
                kept.took += render.took;
                by.set(name, kept);
            }
        }
        return [...by.entries()];
    }, [frames]);
    const keyOf = (row: [string, { renders: number; wasted: number; own: number; took: number }]) => [0, row[1].own, row[1].took, row[1].renders, row[1].wasted][sorted];
    const ordered = sorted === 0 ? [...rows].sort((a, b) => a[0].localeCompare(b[0])) : [...rows].sort((a, b) => keyOf(b) - keyOf(a));
    const most = Math.max(0.001, ...rows.map((row) => row[1].own));
    return (
        <column flex={1}>
            <Heading columns={['Component', 'Self time', 'Total time', 'Renders', 'Changing nothing']} widths={[0, 160, 110, 90, 130]} theme={theme} sorted={sorted} onSort={setSorted} />
            <scroll key="components" flex={1}>
                {ordered.length === 0 ? <Empty text="Nothing has rendered yet." theme={theme} /> : null}
                {ordered.map(([name, kept]) => (
                    <column key={`component-${name}`}>
                        <row height={ROW + 2} align="center" hover={theme.hover}>
                            <Cell text={name} width={0} theme={theme} color={theme.fn} mono />
                            <row width={160} align="center" paddingX={8} gap={6}>
                                <text width={64} fontSize={12} fontFamily={UI} color={theme.text} noWrap>{ms(kept.own)}</text>
                                <group width={Math.max(1, (kept.own / most) * 70)} height={8} radius={2} background={theme.script} />
                            </row>
                            <Cell text={ms(kept.took)} width={110} theme={theme} />
                            <Cell text={String(kept.renders)} width={90} theme={theme} />
                            <Cell text={String(kept.wasted)} width={130} theme={theme} color={kept.wasted > 0 ? theme.warn : theme.dim} />
                        </row>
                        <Across theme={theme} />
                    </column>
                ))}
            </scroll>
        </column>
    );
}

/// Every change of state kept. Looking at an app, each can be gone back to, and what's after where things are now is greyed,
/// as it'll be gone through again going forward, or replaced by what changes next.
function History({ history, now, goTo, said, theme }: { history: Change[]; now: number | null; goTo: ((change: number) => void) | null; said: string | null; theme: Theme }) {
    const oldest = history[0]?.number ?? 1;
    const row = (number: number, component: string, change: string, ahead: boolean, key: string) => (
        <column key={key}>
            <row height={ROW + 6} align="center" hover={theme.hover} background={now === number ? theme.chosen : 'transparent'}>
                <Cell text={`${number}`} width={60} theme={theme} color={theme.dim} />
                <Cell text={component} width={180} theme={theme} color={ahead ? theme.dim : theme.fn} mono />
                <Cell text={change} width={0} theme={theme} color={ahead ? theme.dim : theme.text} mono />
                <row width={170} justify="end" paddingRight={8}>
                    {goTo ? now === number ? <text fontSize={12} fontFamily={UI} color={theme.ok} paddingRight={6}>now</text> : <Action label={ahead ? 'Go forward to here' : 'Go back to here'} onClick={() => goTo(number)} theme={theme} /> : null}
                </row>
            </row>
            <Across theme={theme} />
        </column>
    );
    return (
        <column flex={1}>
            <Toolbar theme={theme}>
                <text fontSize={12} fontFamily={UI} color={theme.dim} paddingX={6}>{plural(history.length, 'change') + ' of state'}</text>
                <group flex={1} />
                {said ? <text fontSize={12} fontFamily={UI} color={theme.text} paddingRight={8} selectable>{said}</text> : null}
            </Toolbar>
            <Heading columns={['#', 'Component', 'Change', '']} widths={[60, 180, 0, 170]} theme={theme} />
            <scroll key="history" flex={1}>
                {history.length === 0 ? <Empty text={goTo ? 'No state has changed yet. Use the app, and each change shows here.' : 'No state changed in what was kept.'} theme={theme} /> : null}
                {goTo && history.length > 0 ? row(oldest - 1, '', oldest > 1 ? 'Before the oldest change kept' : 'Before anything changed', false, 'change-start') : null}
                {history.map((change) => row(change.number, change.component, change.said, now !== null && change.number > now, `change-${change.number}`))}
            </scroll>
        </column>
    );
}

/// The inputs that are worth starting a replay at: those that do something, rather than the pointer only moving.
const DOING = ['down', 'up', 'buttondown', 'buttonup', 'key', 'text', 'composition', 'scroll', 'files', 'drag', 'access', 'close'];

/// What's come in to the app, each of which it can be started again from, replaying everything before it.
function Replay({ count, ask, onStarted, theme }: { count: number; ask: Ask; onStarted: (pid: number) => void; theme: Theme }) {
    const [inputs, setInputs] = useState<Input[]>([]);
    const [all, setAll] = useState(false);
    const [said, setSaid] = useState<string | null>(null);
    useEffect(() => {
        let gone = false;
        ask({ ask: 'inputs', after: inputs.length }).then((answer) => {
            if (!gone && !answer.error && answer.inputs.length > 0) {
                setInputs([...inputs, ...answer.inputs]);
            }
        });
        return () => {
            gone = true;
        };
    }, [count]);
    const again = async (to: number, pause: boolean) => {
        const answer = await ask({ ask: 'replay', to, pause });
        setSaid(answer.error ?? answer.said ?? null);
        if (answer.pid) {
            onStarted(answer.pid);
        }
    };
    const shown = (all ? inputs : inputs.filter((one) => DOING.includes(one.kind))).slice(-MOST_LISTED);
    return (
        <column flex={1}>
            <Toolbar theme={theme}>
                <IconButton name={all ? 'Only what does something' : 'Show moves too'} icon="eye" on={all} onClick={() => setAll(!all)} theme={theme} />
                <Down theme={theme} />
                <text flex={1} fontSize={12} fontFamily={UI} color={said ? theme.text : theme.dim} paddingX={6} noWrap ellipsis selectable>
                    {said ?? 'Starts the app again in a window of its own, giving it everything up to the input chosen at the times it came, with the same random numbers.'}
                </text>
            </Toolbar>
            <Heading columns={['#', 'Time', 'Input', '']} widths={[60, 80, 0, 330]} theme={theme} />
            <scroll key="inputs" flex={1}>
                {shown.length === 0 ? <Empty text="Nothing has come in yet. Use the app, and what you do shows here." theme={theme} /> : null}
                {shown.map((one) => (
                    <column key={`input-${one.number}`}>
                        <row height={ROW + 8} align="center" hover={theme.hover}>
                            <Cell text={`${one.number}`} width={60} theme={theme} color={theme.dim} />
                            <Cell text={`${(one.at / 1000).toFixed(2)} s`} width={80} theme={theme} color={theme.dim} />
                            <Cell text={one.said} width={0} theme={theme} />
                            <row width={330} justify="end" gap={6} paddingRight={8}>
                                <Action label="Replay to here" onClick={() => again(one.number, false)} theme={theme} />
                                <Action label="Replay and stop in it" onClick={() => again(one.number, true)} theme={theme} strong />
                            </row>
                        </row>
                        <Across theme={theme} />
                    </column>
                ))}
            </scroll>
        </column>
    );
}

/// A trace that was saved.
function Saved({ trace, theme }: { trace: Trace; theme: Theme }) {
    const [tab, setTab] = useState('Performance');
    return (
        <window background={theme.page} color={theme.text} fontFamily={UI}>
            <TabStrip
                tabs={['Performance', 'Rendering', 'State']}
                value={tab}
                onChange={setTab}
                theme={theme}
                right={<text fontSize={12} fontFamily={UI} color={theme.dim} paddingRight={10}>{trace.file ?? trace.app}</text>}
            />
            {tab === 'Performance' ? <Performance frames={trace.frames} live={false} summary={() => Promise.resolve(trace.summary)} theme={theme} /> : null}
            {tab === 'Rendering' ? <Rendering frames={trace.frames} theme={theme} /> : null}
            {tab === 'State' ? <History history={trace.history} now={null} goTo={null} said={null} theme={theme} /> : null}
        </window>
    );
}

type News = { frames: Frame[]; history: Change[]; now: number; heard: Heard[]; logpoints: Logpoint[]; showing: boolean; breakpoints: Breakpoint[]; paused: Stopped | null; inputs: number };
const NO_NEWS: News = { frames: [], history: [], now: 0, heard: [], logpoints: [], showing: false, breakpoints: [], paused: null, inputs: 0 };
const TABS = ['Elements', 'Console', 'Sources', 'Performance', 'Rendering', 'State', 'Replay'];

/// An app being looked at while it runs, until it stops, when what was kept stays to be looked at.
function Watching({ app, onLeave, onReplay, theme }: { app: Running; onLeave: () => void; onReplay: (pid: number) => void; theme: Theme }) {
    const [news, setNews] = useState<News>(NO_NEWS);
    const [gone, setGone] = useState<string | null>(null);
    // Whether what's shown has stopped being brought up to date, to read it, which doesn't stop the app.
    const [frozen, setFrozen] = useState(false);
    const [tab, setTab] = useState('Elements');
    const [said, setSaid] = useState<string | null>(null);
    const [frame, setFrame] = useState(0);
    const [wanted, setWanted] = useState<string | null>(null);
    const [worked, setWorked] = useState<Worked[]>([]);
    const [cleared, setCleared] = useState(0);
    // Which edge of the app's window this is kept along, if it's docked in it, as a browser's inspector can be.
    const [docked, setDocked] = useState<string | null>(null);
    const link = useRef<Link | null>(null);
    const frozenNow = useRef(false);
    frozenNow.current = frozen;
    // What's said about something done goes after a while, as a browser's inspector's notes do.
    useEffect(() => {
        if (said === null) {
            return undefined;
        }
        const timer = setTimeout(() => setSaid(null), 6000);
        return () => clearTimeout(timer);
    }, [said]);

    useEffect(() => {
        const made = new Link(app.pipe, (why) => {
            setGone(why);
            if (why.includes('no longer running')) {
                try {
                    unlinkSync(app.place);
                } catch {
                    // It's gone already.
                }
            }
        });
        link.current = made;
        let kept = NO_NEWS;
        let heardUpTo = 0;
        let timer: any = null;
        const askAgain = async () => {
            if (made.gone) {
                return;
            }
            if (!frozenNow.current) {
                const after = kept.frames[kept.frames.length - 1]?.number ?? 0;
                const answer = await made.ask({ ask: 'news', after, heard: heardUpTo });
                if (answer.error) {
                    if (!made.gone) {
                        setSaid(answer.error);
                    }
                } else {
                    // Frames are numbered again from 1 when the app starts keeping them again.
                    const fresh: Frame[] = answer.frames;
                    const again = answer.started || (fresh.length > 0 && fresh[0].number <= after);
                    const frames = fresh.length === 0 && !again ? kept.frames : [...(again ? [] : kept.frames), ...fresh].slice(-MOST_FRAMES);
                    const heard = answer.heard.length === 0 ? kept.heard : [...kept.heard, ...answer.heard].slice(-MOST_HEARD);
                    heardUpTo = answer.said;
                    const history: Change[] = answer.history;
                    const showing = answer.showing ?? kept.showing;
                    const inputs = answer.inputs ?? kept.inputs;
                    const stopped: Stopped | null = answer.paused ?? null;
                    const same =
                        frames === kept.frames &&
                        heard === kept.heard &&
                        answer.now === kept.now &&
                        showing === kept.showing &&
                        inputs === kept.inputs &&
                        history.length === kept.history.length &&
                        history[history.length - 1]?.number === kept.history[kept.history.length - 1]?.number &&
                        JSON.stringify(answer.logpoints) === JSON.stringify(kept.logpoints) &&
                        JSON.stringify(answer.breakpoints) === JSON.stringify(kept.breakpoints) &&
                        JSON.stringify(stopped) === JSON.stringify(kept.paused);
                    if (!same) {
                        // Where it's just stopped is shown, in its code, whatever was being looked at.
                        if (stopped && (!kept.paused || kept.paused.at !== stopped.at)) {
                            setTab('Sources');
                            setFrame(0);
                            setWanted(placeOf(stopped.at)[0]);
                        }
                        kept = { frames, history, now: answer.now, heard, logpoints: answer.logpoints, showing, breakpoints: answer.breakpoints ?? [], paused: stopped, inputs };
                        setNews(kept);
                    }
                }
            }
            timer = setTimeout(askAgain, ASK_EVERY);
        };
        askAgain();
        return () => {
            clearTimeout(timer);
            made.close();
        };
    }, [app.pipe]);

    const ask: Ask = useCallback((request: object) => link.current?.ask(request) ?? Promise.resolve({ error: 'The app has stopped.' }), []);
    const goTo = async (change: number) => {
        const answer = await ask({ ask: 'goto', change });
        setSaid(answer.error ?? answer.said ?? null);
    };
    const save = async () => {
        const answer = await ask({ ask: 'save' });
        setSaid(answer.error ?? answer.said ?? null);
    };
    const showInApp = async () => {
        const answer = await ask({ ask: 'show', on: !news.showing });
        if (answer.error) {
            setSaid(answer.error);
        }
    };
    const resume = async (how: string) => {
        const answer = await ask({ ask: 'resume', how });
        setSaid(answer.error ?? null);
    };
    const dock = async (side: string | null) => {
        const answer = await ask({ ask: 'dock', pid: process.pid, side, size: side === 'bottom' ? 320 : 520 });
        if (answer.error) {
            setSaid(answer.error);
        } else {
            setDocked(side);
        }
    };
    // When the app's gone, so is where it was docked, as the app gives it back as it closes.
    useEffect(() => {
        if (gone) {
            setDocked(null);
        }
    }, [gone]);
    const run = async (expression: string) => {
        const after = news.heard[news.heard.length - 1]?.number ?? 0;
        const stopped = news.paused;
        const answer = await ask(stopped ? { ask: 'evaluate', frame, expression } : { ask: 'evaluate', expression });
        const output = String(answer.error ?? answer.value ?? '');
        setWorked((was) => [...was, { after, input: expression, output, threw: Boolean(answer.error || answer.threw) }].slice(-200));
    };
    const stopped = gone ? null : news.paused;
    const status = gone ?? (stopped ? `paused, process ${app.pid}` : frozen ? 'not brought up to date' : `running, process ${app.pid}`);

    return (
        <window background={theme.page} color={theme.text} fontFamily={UI}>
            <TabStrip
                tabs={TABS}
                value={tab}
                onChange={setTab}
                theme={theme}
                right={
                    <row align="center" gap={2} paddingRight={6} shrink={1}>
                        <group width={8} height={8} radius={4} background={gone ? theme.strong : stopped ? theme.paused : theme.ok} />
                        <text fontSize={12} fontFamily={UI} color={theme.text} paddingLeft={4} noWrap ellipsis shrink={6} minWidth={0} maxWidth={140}>{app.app}</text>
                        <text fontSize={12} fontFamily={UI} color={theme.dim} paddingLeft={6} paddingRight={6} noWrap ellipsis shrink={12} minWidth={0} maxWidth={180}>{status}</text>
                        <Down theme={theme} />
                        {gone ? <Action label="Look at another" onClick={onLeave} theme={theme} /> : null}
                        {gone ? null : <IconButton name="Undock into a separate window" icon="undock" on={docked === null} onClick={() => dock(null)} theme={theme} />}
                        {gone ? null : <IconButton name="Dock to the bottom of the app" icon="dockBottom" on={docked === 'bottom'} onClick={() => dock('bottom')} theme={theme} />}
                        {gone ? null : <IconButton name="Dock to the right of the app" icon="dockRight" on={docked === 'right'} onClick={() => dock('right')} theme={theme} />}
                        {gone ? null : <Down theme={theme} />}
                        {gone ? null : <IconButton name={news.showing ? 'Hide in the app' : 'Show in the app'} icon="eye" on={news.showing} onClick={showInApp} theme={theme} />}
                        {gone ? null : <IconButton name={frozen ? 'Bring up to date' : 'Stop updating'} icon="freeze" on={frozen} onClick={() => setFrozen(!frozen)} theme={theme} />}
                        {gone ? null : <IconButton name="Save a trace" icon="save" onClick={save} theme={theme} />}
                    </row>
                }
            />
            {stopped ? (
                <column background={theme.pausedBack}>
                    <row height={30} align="center" paddingX={10} gap={8}>
                        <Icon name="pause" color={theme.paused} size={14} />
                        <text fontSize={12} fontFamily={UI} fontWeight="bold" color={theme.paused}>Paused in debugger</text>
                        <text flex={1} fontSize={12} fontFamily={UI} color={theme.text} noWrap ellipsis>{`${stopped.function || 'a function with no name'}, ${stopped.at}`}</text>
                        <IconButton name="Resume script" icon="resume" onClick={() => resume('continue')} theme={theme} color={theme.accent} />
                        <IconButton name="Step over the next call" icon="over" onClick={() => resume('over')} theme={theme} color={theme.accent} />
                    </row>
                    <Across theme={theme} />
                </column>
            ) : null}
            {said && tab !== 'State' ? (
                <column background={theme.bar}>
                    <row height={24} align="center" paddingX={10}>
                        <text flex={1} fontSize={12} fontFamily={UI} color={theme.text} noWrap ellipsis selectable>{said}</text>
                        <IconButton name="Dismiss" icon="close" onClick={() => setSaid(null)} theme={theme} />
                    </row>
                    <Across theme={theme} />
                </column>
            ) : null}
            <scroll key="panel" axis="horizontal" flex={1}>
            <column flex={1} minWidth={PANEL_LEAST} height="100%">
            {tab === 'Elements' ? <Elements ask={ask} theme={theme} gone={Boolean(gone)} /> : null}
            {tab === 'Console' ? <Console heard={news.heard} worked={worked} run={run} clear={() => setCleared(news.heard[news.heard.length - 1]?.number ?? 0)} cleared={cleared} stopped={stopped} theme={theme} /> : null}
            {tab === 'Sources' ? <Sources app={app} news={news} ask={ask} frame={frame} setFrame={setFrame} wanted={wanted} theme={theme} gone={Boolean(gone)} /> : null}
            {tab === 'Performance' ? (
                <Performance
                    frames={news.frames}
                    live
                    summary={async () => {
                        const answer = await ask({ ask: 'summary' });
                        return answer.error ?? answer.summary ?? '';
                    }}
                    theme={theme}
                />
            ) : null}
            {tab === 'Rendering' ? <Rendering frames={news.frames} theme={theme} /> : null}
            {tab === 'State' ? <History history={news.history} now={news.now} goTo={gone ? null : goTo} said={said} theme={theme} /> : null}
            {tab === 'Replay' ? <Replay count={news.inputs} ask={ask} onStarted={onReplay} theme={theme} /> : null}
            </column>
            </scroll>
        </window>
    );
}

/// The apps there are to look at, until one's chosen, as a browser lists what it can inspect. With only one, that's the one.
function Live({ theme }: { theme: Theme }) {
    const [apps, setApps] = useState<Running[]>(runningApps);
    const [chosen, setChosen] = useState<Running | null>(null);
    const [left, setLeft] = useState<number[]>([]);
    // A replay that's been started, which is looked at once it says it's there.
    const [awaited, setAwaited] = useState<number | null>(null);
    useEffect(() => {
        if (chosen) {
            return undefined;
        }
        const timer = setInterval(() => setApps(runningApps()), awaited ? 200 : 1000);
        return () => clearInterval(timer);
    }, [chosen, awaited]);
    const choosable = apps.filter((one) => !left.includes(one.pid));
    const only = choosable.length === 1 ? choosable[0] : null;
    const replayed = awaited ? apps.find((one) => one.pid === awaited) : undefined;
    // With only one, that's chosen, so it stays when it stops, for what was kept to be looked at. A replay that's been
    // started is chosen as soon as it's there.
    useEffect(() => {
        if (replayed) {
            setAwaited(null);
            setChosen(replayed);
        } else if (!chosen && only && !awaited) {
            setChosen(only);
        }
    }, [chosen, only?.pid, replayed?.pid]);
    if (chosen) {
        return (
            <Watching
                key={`app-${chosen.pid}`}
                app={chosen}
                onLeave={() => {
                    setLeft([...left, chosen.pid]);
                    setChosen(null);
                    setApps(runningApps());
                }}
                onReplay={(pid) => {
                    setAwaited(pid);
                    setChosen(null);
                }}
                theme={theme}
            />
        );
    }
    return (
        <window background={theme.page} color={theme.text} fontFamily={UI}>
            <TabStrip tabs={['Apps']} value="Apps" onChange={() => {}} theme={theme} />
            <column padding={24} gap={16} maxWidth={760}>
                <text fontSize={18} fontFamily={UI} color={theme.text}>{awaited ? `Starting the replay, process ${awaited}` : 'Apps running'}</text>
                <text fontSize={12} fontFamily={UI} color={theme.dim}>
                    {choosable.length === 0 ? 'Run one with rsui-tsx app.tsx and it shows up here. A packed app can’t be looked at.' : 'These are running, and can be inspected while they run.'}
                </text>
                <column borderWidth={1} borderColor={theme.line} radius={4}>
                    {choosable.map((one, index) => (
                        <column key={`app-${one.pid}`}>
                            {index > 0 ? <Across theme={theme} /> : null}
                            <row padding={12} gap={12} align="center" hover={theme.hover}>
                                <column flex={1} gap={2}>
                                    <text fontSize={13} fontFamily={UI} fontWeight="bold" color={theme.text}>{one.app}</text>
                                    <text fontSize={12} fontFamily={UI} color={theme.dim} noWrap ellipsis>{`${one.file} · process ${one.pid}`}</text>
                                </column>
                                <button name={one.app} onClick={() => setChosen(one)} direction="row" paddingX={8} height={24} align="center" hover={theme.hover} radius={4}>
                                    <text fontSize={12} fontFamily={UI} color={theme.accent}>inspect</text>
                                </button>
                            </row>
                        </column>
                    ))}
                    {choosable.length === 0 ? <text fontSize={12} fontFamily={UI} color={theme.dim} padding={12}>Nothing yet.</text> : null}
                </column>
            </column>
        </window>
    );
}

export default function Viewer() {
    const theme = useColorScheme() === 'dark' ? DARK : LIGHT;
    if (TRACE === null) {
        return <Live theme={theme} />;
    }
    if (typeof TRACE === 'string') {
        return (
            <window padding={24} background={theme.page}>
                <text color={theme.error} selectable>{TRACE}</text>
            </window>
        );
    }
    return <Saved trace={TRACE} theme={theme} />;
}
