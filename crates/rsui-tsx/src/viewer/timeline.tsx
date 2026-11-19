// The Performance tab: where an app's time went, as a browser's profiler shows it. Across the top, every frame kept, as bars as
// tall as they took, coloured by whether they fitted in a frame's time; dragging across them picks what to look at. Under that,
// a flame chart of what was picked: each frame's update, the handlers and timers that ran, each component's render, layout,
// paint and the GPU, as blocks inside what they were inside, and, while JavaScript's being recorded, each function it was in.
// The wheel zooms where the pointer is, and dragging moves along. Under that, what was picked, added up: by what took the most
// time itself, as a treemap, and the frame chosen, with why each component in it rendered.
import { useState, useMemo, useEffect, useRef } from 'rsui';
import { Spans, Span, Placed, NS, bottomUp, byComponent, sampleBlocks, formatNs, Row } from './spans';
import { Theme, Frame, UI, MONO, BUDGET, ms } from './theme';
import { Icon, IconButton, Action, Across, Down, Toolbar, Empty, Heading, Cell } from './ui';

const FRAME_NS = BUDGET * NS;
/// How tall a block in the flame chart is, with the line under it.
const LANE = 19;
/// How many bars the overview of every frame has at most, each the slowest frame in its part of the time.
const OVERVIEW_BARS = 240;
/// How much is shown to begin with, following what's newest.
const FOLLOW_NS = 3000 * NS;

/// The colour of each kind of work, as a browser's profiler has them.
export function categoryColor(category: string, theme: Theme): string {
    switch (category) {
        case 'script':
            return theme.script;
        case 'js':
            return '#e8a33d';
        case 'style':
            return '#c87fd7';
        case 'layout':
            return theme.layout;
        case 'paint':
            return theme.paint;
        case 'gpu':
            return theme.gpu;
        case 'gc':
            return '#d9705a';
        case 'input':
            return '#5fb6c9';
        case 'frame':
            return theme.strong;
        case 'idle':
            return theme.line;
        default:
            return theme.dim;
    }
}

function frameColor(took: number, theme: Theme): string {
    return took > FRAME_NS ? theme.error : took > FRAME_NS / 2 ? theme.warn : theme.ok;
}

/// Where across an element a pointer event was, from 0 to 1, and how wide the element is, from what it was given.
function across(event: any): { at: number; width: number } {
    const box = event?.__chain?.[event.__depth];
    const width = box?.[8] ?? 0;
    const offset = event?.offsetX ?? 0;
    return { at: width > 0 ? Math.max(0, Math.min(1, offset / width)) : 0.5, width };
}

type View = { from: number; to: number } | null;

/// A block, or a gap, along a lane, as wide as the time it's for.
type Piece = { start: number; end: number; title?: string; tip?: string; color?: string; id?: string; merged?: number; first?: string; least?: boolean };

/// Lays blocks out along a lane between `from` and `to`, leaving out what's too small to see and running together what's
/// too small to see apart, so a lane of thousands is a few hundred things to draw.
function lay(blocks: Piece[], from: number, to: number, width: number, keepTiny = false): Piece[] {
    const least = (to - from) / Math.max(200, width);
    const out: Piece[] = [];
    let cursor = from;
    for (const block of blocks) {
        const start = Math.max(block.start, cursor);
        const end = Math.min(block.end, to);
        if (end <= start) {
            continue;
        }
        const last = out[out.length - 1];
        if (end - start < least) {
            // Too small to see: it goes in with what's just before it, if that's small too and close by.
            if (last && last.merged !== undefined && start - last.end < least) {
                last.end = end;
                last.merged += 1;
                last.title = `${last.first} ×${last.merged}`;
                last.tip = `${last.merged} blocks too small to tell apart here, the first ${last.first}: zoom in to see each`;
                last.id = undefined;
                cursor = end;
                continue;
            }
            if (end - start < least / 4 && !keepTiny) {
                continue;
            }
            out.push({ ...block, start, end, merged: 1, first: block.title, least: keepTiny });
            cursor = end;
            continue;
        }
        out.push({ ...block, start, end });
        cursor = end;
    }
    return out;
}

/// One lane of the flame chart: blocks as wide as their time, each a button that says what it is.
function Lane({ pieces, from, to, width, chosen, choose, theme }: { pieces: Piece[]; from: number; to: number; width: number; chosen: string | null; choose: (id: string | null) => void; theme: Theme }) {
    const children: any[] = [];
    let cursor = from;
    const span = Math.max(1, to - from);
    pieces.forEach((piece, index) => {
        if (piece.start > cursor) {
            children.push(<group key={`gap-${index}`} flex={piece.start - cursor} minWidth={0} />);
        }
        const wide = ((piece.end - piece.start) / span) * width;
        const picked = piece.id !== undefined && piece.id === chosen;
        children.push(
            <button
                key={`block-${index}`}
                name={piece.title ?? 'block'}
                tooltip={piece.tip ?? piece.title}
                onClick={() => choose(piece.id ?? null)}
                flex={piece.end - piece.start}
                minWidth={piece.least ? 2 : 0}
                height={LANE - 1}
                direction="row"
                align="center"
                clip
                background={piece.color ?? theme.dim}
                hover={theme.accent}
                borderWidth={picked ? 1 : 0}
                borderColor={theme.text}
                radius={wide > 6 ? 2 : 0}
            >
                {wide > 40 && piece.title ? (
                    // Out of the way of the layout, so a title never makes its block wider than its time.
                    <text position="absolute" left={4} right={2} top={2} height={LANE - 4} fontSize={11} fontFamily={UI} color="#111" noWrap ellipsis>
                        {piece.title}
                    </text>
                ) : null}
            </button>,
        );
        cursor = piece.end;
    });
    if (cursor < to) {
        children.push(<group key="gap-end" flex={to - cursor} minWidth={0} />);
    }
    return (
        <row height={LANE} align="start" minWidth={0}>
            {children}
        </row>
    );
}

/// The times along the top of the chart, at round numbers.
function Ruler({ from, to, origin, theme }: { from: number; to: number; origin: number; theme: Theme }) {
    const span = Math.max(1, to - from);
    const steps = [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000].map((step) => step * NS);
    const step = steps.find((one) => span / one <= 8) ?? 10_000 * NS;
    const ticks: number[] = [];
    // At round times from the oldest kept, which is what the times say.
    for (let at = origin + Math.ceil((from - origin) / step) * step; at <= to; at += step) {
        ticks.push(at);
    }
    // Each tick is the start of the stretch up to the next, with its line and time at its left.
    const children: any[] = [<group key="gap-start" flex={Math.max(0, (ticks[0] ?? to) - from)} minWidth={0} />];
    ticks.forEach((tick, index) => {
        children.push(
            <row key={`tick-${index}`} flex={Math.max(1, Math.min(to, ticks[index + 1] ?? to) - tick)} minWidth={0} height={18}>
                <group width={1} height={18} background={theme.line} />
                <text flex={1} minWidth={0} fontSize={10} fontFamily={UI} color={theme.dim} paddingLeft={3} noWrap ellipsis>
                    {formatNs(tick - origin)}
                </text>
            </row>,
        );
    });
    return (
        <column background={theme.bar}>
            <row height={18} minWidth={0}>
                {children}
            </row>
            <Across theme={theme} />
        </column>
    );
}

/// Every frame kept, as bars as tall as the slowest in each part of the time, with what's being looked at marked. Dragging
/// across it picks what to look at, and clicking moves there.
function Overview({ spans, view, setView, theme }: { spans: Spans; view: { from: number; to: number }; setView: (view: View) => void; theme: Theme }) {
    const [first, last] = spans.extent();
    const span = Math.max(1, last - first);
    const bars = useMemo(() => {
        const slowest = new Array(OVERVIEW_BARS).fill(0);
        for (const frame of spans.frames) {
            const at = Math.floor(((frame.start - first) / span) * OVERVIEW_BARS);
            if (at >= 0 && at < OVERVIEW_BARS) {
                slowest[at] = Math.max(slowest[at], frame.end - frame.start);
            }
        }
        return slowest;
    }, [spans.version, first, last]);
    const press = useRef<{ at: number; moved: boolean } | null>(null);
    const timeAt = (event: any) => first + across(event).at * span;
    const height = 44;
    const tallest = Math.max(FRAME_NS * 2, ...bars);
    const lo = Math.max(0, (view.from - first) / span);
    const hi = Math.min(1, (view.to - first) / span);
    return (
        <column
            name="Every frame kept"
            background={theme.page}
            onPointerDown={(event: any) => {
                press.current = { at: timeAt(event), moved: false };
            }}
            onPointerMove={(event: any) => {
                if (press.current && (event.buttons ?? 1) !== 0) {
                    const now = timeAt(event);
                    if (Math.abs(now - press.current.at) > span / 200) {
                        press.current.moved = true;
                        setView({ from: Math.min(press.current.at, now), to: Math.max(press.current.at, now) });
                    }
                }
            }}
            onPointerUp={(event: any) => {
                const pressed = press.current;
                press.current = null;
                if (pressed && !pressed.moved) {
                    // A click moves what's looked at there, as wide as it was.
                    const wide = view.to - view.from;
                    const middle = timeAt(event);
                    setView({ from: middle - wide / 2, to: middle + wide / 2 });
                }
            }}
        >
            <row height={height} align="end" paddingX={0} minWidth={0}>
                {lo > 0 ? <group flex={lo} minWidth={0} height={height} /> : null}
                <group flex={Math.max(0.002, hi - lo)} minWidth={2} height={height} background={theme.selected} opacity={0.55} />
                {hi < 1 ? <group flex={1 - hi} minWidth={0} height={height} /> : null}
            </row>
            <row height={height} align="end" minWidth={0} position="absolute" left={0} right={0} top={0}>
                {bars.map((took, index) => (
                    <group key={`bar-${index}`} flex={1} minWidth={0} height={took > 0 ? Math.max(2, (took / tallest) * (height - 4)) : 0} background={frameColor(took, theme)} opacity={0.9} />
                ))}
            </row>
            <group position="absolute" left={0} right={0} top={height - (FRAME_NS / tallest) * (height - 4)} height={1} background={theme.error} opacity={0.35} />
            <Across theme={theme} />
        </column>
    );
}

type Chosen = { kind: 'span'; span: Placed } | { kind: 'sample'; label: number; start: number; end: number; depth: number } | null;

/// The flame chart of what's between `from` and `to`: the app's own work, by how deep it was, then JavaScript as it was
/// sampled.
function FlameChart({ spans, from, to, setView, chosen, choose, theme }: { spans: Spans; from: number; to: number; setView: (view: View) => void; chosen: Chosen; choose: (chosen: Chosen) => void; theme: Theme }) {
    const width = useRef(1000);
    const drag = useRef<{ x: number; from: number; to: number } | null>(null);
    const placed = useMemo(() => spans.placed(from, to), [spans.version, from, to]);
    const blocks = useMemo(() => sampleBlocks(spans, from, to), [spans.version, from, to]);
    const frames = useMemo(() => spans.framesIn(from, to), [spans.version, from, to]);
    const chosenId = chosen?.kind === 'span' ? `s${chosen.span.start}-${chosen.span.kind}-${chosen.span.depth}` : chosen?.kind === 'sample' ? `j${chosen.start}-${chosen.depth}` : null;

    const lanes = useMemo(() => {
        const byDepth: Piece[][] = [];
        for (const span of placed) {
            const kind = spans.kind(span);
            const title = spans.title(span);
            const pieces = (byDepth[span.depth] ??= []);
            pieces.push({
                start: span.start,
                end: span.end,
                title,
                tip: `${title}\n${formatNs(span.end - span.start)}, ${formatNs(span.self)} itself\n${kind.about}`,
                color: kind.name === 'Render' && span.arg & 4 ? theme.warn : categoryColor(kind.category, theme),
                id: `s${span.start}-${span.kind}-${span.depth}`,
            });
        }
        const js: Piece[][] = [];
        for (const block of blocks) {
            const where = spans.where(block.label);
            (js[block.depth] ??= []).push({
                start: block.start,
                end: block.end,
                title: where.name,
                tip: `${where.name}\n${where.at}\n${formatNs(block.end - block.start)}`,
                color: block.depth % 2 ? '#f0b85a' : '#e8a33d',
                id: `j${block.start}-${block.depth}`,
            });
        }
        return { app: byDepth, js };
    }, [placed, blocks, theme]);

    const pick = (id: string | null) => {
        if (id === null) {
            choose(null);
        } else if (id.startsWith('s')) {
            const span = placed.find((one) => `s${one.start}-${one.kind}-${one.depth}` === id);
            choose(span ? { kind: 'span', span } : null);
        } else {
            const block = blocks.find((one) => `j${one.start}-${one.depth}` === id);
            choose(block ? { kind: 'sample', ...block } : null);
        }
    };

    const zoom = (event: any) => {
        const { at, width: wide } = across(event);
        if (wide > 0) {
            width.current = wide;
        }
        const pivot = from + at * (to - from);
        const factor = (event.deltaY ?? 0) > 0 ? 1.25 : 0.8;
        const span = Math.max(20_000, (to - from) * factor);
        setView({ from: pivot - (pivot - from) * (span / (to - from)), to: pivot + (to - pivot) * (span / (to - from)) });
        event.preventDefault?.();
    };

    const laneOf = (pieces: Piece[], key: string) => (
        <Lane key={key} pieces={lay(pieces, from, to, width.current)} from={from} to={to} width={width.current} chosen={chosenId} choose={pick} theme={theme} />
    );

    return (
        <column
            flex={1}
            minWidth={0}
            onWheel={zoom}
            onPointerMove={(event: any) => {
                const { width: wide } = across(event);
                if (wide > 0) {
                    width.current = wide;
                }
                if (drag.current && (event.buttons ?? 0) !== 0) {
                    const moved = ((event.clientX ?? 0) - drag.current.x) / width.current;
                    const span = drag.current.to - drag.current.from;
                    setView({ from: drag.current.from - moved * span, to: drag.current.to - moved * span });
                }
            }}
            onPointerDown={(event: any) => {
                drag.current = { x: event.clientX ?? 0, from, to };
            }}
            onPointerUp={() => {
                drag.current = null;
            }}
        >
            <Ruler from={from} to={to} origin={spans.extent()[0]} theme={theme} />
            <scroll key="flame" flex={1}>
                <column minWidth={0} paddingTop={2}>
                    <row height={16} align="center" paddingX={6} gap={6}>
                        <text fontSize={10} fontFamily={UI} fontWeight="bold" color={theme.dim}>FRAMES</text>
                    </row>
                    <Lane
                        pieces={lay(
                            frames.map((frame) => ({
                                start: frame.start,
                                end: frame.end,
                                title: `${formatNs(frame.end - frame.start)}`,
                                tip: `Frame ${frame.arg}: ${formatNs(frame.end - frame.start)}${frame.end - frame.start > FRAME_NS ? `, over the ${BUDGET.toFixed(1)} ms there is` : ''}`,
                                color: frameColor(frame.end - frame.start, theme),
                                id: undefined,
                            })),
                            from,
                            to,
                            width.current,
                            true,
                        )}
                        from={from}
                        to={to}
                        width={width.current}
                        chosen={null}
                        choose={() => {}}
                        theme={theme}
                    />
                    <row height={16} align="center" paddingX={6} paddingTop={4}>
                        <text fontSize={10} fontFamily={UI} fontWeight="bold" color={theme.dim}>MAIN THREAD</text>
                    </row>
                    {lanes.app.length === 0 ? <text fontSize={11} fontFamily={UI} color={theme.dim} paddingX={6} paddingY={4}>Nothing ran here.</text> : null}
                    {lanes.app.map((pieces, depth) => laneOf(pieces ?? [], `app-${depth}`))}
                    {lanes.js.length > 0 || spans.recording ? (
                        <row height={16} align="center" paddingX={6} paddingTop={4}>
                            <text fontSize={10} fontFamily={UI} fontWeight="bold" color={theme.dim}>JAVASCRIPT (SAMPLED EVERY MS)</text>
                        </row>
                    ) : null}
                    {lanes.js.map((pieces, depth) => laneOf(pieces ?? [], `js-${depth}`))}
                </column>
            </scroll>
        </column>
    );
}

/// A bar split by where the time went, with what each part came to.
function Breakdown({ rows, total, theme }: { rows: [string, number][]; total: number; theme: Theme }) {
    const shown = rows.filter(([, took]) => took > 0);
    return (
        <column gap={8}>
            <row height={10} radius={3} background={theme.bar} minWidth={0}>
                {shown.map(([category, took]) => (
                    <group key={`share-${category}`} flex={took} minWidth={0} height={10} background={categoryColor(category, theme)} />
                ))}
                {total > 0 && shown.length === 0 ? <group flex={1} /> : null}
            </row>
            <row gap={16}>
                {shown.map(([category, took]) => (
                    <row key={`legend-${category}`} gap={6} align="center">
                        <group width={10} height={10} radius={2} background={categoryColor(category, theme)} />
                        <text fontSize={12} fontFamily={UI} color={theme.text}>{`${category === 'js' ? 'javascript' : category} ${formatNs(took)}`}</text>
                        <text fontSize={12} fontFamily={UI} color={theme.dim}>{total > 0 ? `${Math.round((took / total) * 100)}%` : ''}</text>
                    </row>
                ))}
            </row>
        </column>
    );
}

/// What the range comes to: how many frames, how many were slow, and where the time went.
function Summary({ spans, from, to, frames, theme }: { spans: Spans; from: number; to: number; frames: Frame[]; theme: Theme }) {
    const worked = useMemo(() => {
        const by = new Map<string, number>();
        for (const span of spans.placed(from, to)) {
            const category = spans.kind(span).category;
            const self = Math.max(0, Math.min(span.self, Math.min(span.end, to) - Math.max(span.start, from)));
            by.set(category, (by.get(category) ?? 0) + self);
        }
        const frames = spans.framesIn(from, to);
        const slow = frames.filter((frame) => frame.end - frame.start > FRAME_NS).length;
        const worst = frames.reduce((most, frame) => Math.max(most, frame.end - frame.start), 0);
        return { by: [...by.entries()].sort((a, b) => b[1] - a[1]), frames: frames.length, slow, worst };
    }, [spans.version, from, to]);
    const busy = worked.by.filter(([category]) => category !== 'idle' && category !== 'frame').reduce((sum, [, took]) => sum + took, 0);
    return (
        <column padding={14} gap={14}>
            <row gap={22}>
                <Stat label="Range" value={formatNs(to - from)} theme={theme} />
                <Stat label="Frames" value={String(worked.frames)} theme={theme} />
                <Stat label="Over 16.7 ms" value={String(worked.slow)} theme={theme} color={worked.slow > 0 ? theme.error : theme.ok} />
                <Stat label="Slowest" value={worked.frames ? formatNs(worked.worst) : '–'} theme={theme} color={worked.worst > FRAME_NS ? theme.error : theme.text} />
                <Stat label="Busy" value={formatNs(busy)} theme={theme} />
            </row>
            <Breakdown rows={worked.by.filter(([category]) => category !== 'frame')} total={busy} theme={theme} />
            {frames.some((frame) => frame.handled.length > 0) ? (
                <column gap={4}>
                    <text fontSize={11} fontFamily={UI} fontWeight="bold" color={theme.dim}>WHAT CAME IN</text>
                    {frames.flatMap((frame) => frame.handled).slice(-12).map((handled, index) => (
                        <text key={`handled-${index}`} fontSize={12} fontFamily={UI} color={theme.text} selectable>
                            {`${handled.action} on #${handled.key} ran ${handled.note ? handled.note + ' ' : ''}for ${ms(handled.took)}${handled.rejected ? ', and was turned down' : ''}`}
                        </text>
                    ))}
                </column>
            ) : null}
            <text fontSize={12} fontFamily={UI} color={theme.dim}>
                {spans.recording
                    ? 'Recording JavaScript: every millisecond, where it is shows under the main thread.'
                    : spans.samples.length > 0
                      ? `JavaScript was sampled ${spans.sampled(from, to).length} times in this range: see Bottom-up and Treemap for where.`
                      : 'Press Record to sample where JavaScript is every millisecond, to see which functions the script time went to.'}
            </text>
        </column>
    );
}

function Stat({ label, value, theme, color }: { label: string; value: string; theme: Theme; color?: string }) {
    return (
        <column gap={2}>
            <text fontSize={11} fontFamily={UI} color={theme.dim}>{label}</text>
            <text fontSize={18} fontFamily={UI} color={color ?? theme.text}>{value}</text>
        </column>
    );
}

/// Everything in the range, by the time it took itself, as a browser's bottom-up view has it.
function BottomUp({ rows, theme, openSource }: { rows: Row[]; theme: Theme; openSource?: (file: string, line: number) => void }) {
    const [sorted, setSorted] = useState(1);
    const ordered = useMemo(() => {
        const key = (row: Row) => [0, row.self, row.total, row.count][sorted];
        return sorted === 0 ? [...rows].sort((a, b) => a.name.localeCompare(b.name)) : [...rows].sort((a, b) => key(b) - key(a));
    }, [rows, sorted]);
    const most = Math.max(1, ...rows.map((row) => row.self));
    return (
        <column flex={1}>
            <Heading columns={['Activity', 'Self time', 'Total time', 'Count', 'Where']} widths={[0, 150, 100, 70, 260]} theme={theme} sorted={sorted} onSort={setSorted} />
            <scroll key="bottom-up" flex={1}>
                {ordered.length === 0 ? <Empty text="Nothing ran in what's picked." theme={theme} /> : null}
                {ordered.slice(0, 300).map((row) => (
                    <column key={`row-${row.key}`}>
                        <row height={22} align="center" hover={theme.hover}>
                            <row flex={1} minWidth={0} align="center" gap={6} paddingX={8}>
                                <group width={9} height={9} radius={2} background={categoryColor(row.category, theme)} />
                                <text flex={1} minWidth={0} fontSize={12} fontFamily={row.category === 'js' ? MONO : UI} color={row.category === 'js' ? theme.fn : theme.text} noWrap ellipsis selectable>{row.name}</text>
                            </row>
                            <row width={150} align="center" gap={6} paddingX={8}>
                                <text width={64} fontSize={12} fontFamily={UI} color={theme.text} noWrap>{formatNs(row.self)}</text>
                                <group width={Math.max(1, (row.self / most) * 64)} height={8} radius={2} background={categoryColor(row.category, theme)} />
                            </row>
                            <Cell text={formatNs(row.total)} width={100} theme={theme} />
                            <Cell text={String(row.count)} width={70} theme={theme} color={theme.dim} />
                            {row.at && openSource ? (
                                <button name={`Open ${row.at}`} onClick={() => {
                                    const colon = row.at.lastIndexOf(':');
                                    openSource(row.at.slice(0, colon), Number(row.at.slice(colon + 1)));
                                }} width={260} height={22} direction="row" align="center" paddingX={8} hover={theme.raised}>
                                    <text flex={1} minWidth={0} fontSize={12} fontFamily={MONO} color={theme.accent} noWrap ellipsis>{shortPlace(row.at)}</text>
                                </button>
                            ) : (
                                <Cell text={row.at} width={260} theme={theme} color={theme.dim} />
                            )}
                        </row>
                        <Across theme={theme} />
                    </column>
                ))}
            </scroll>
        </column>
    );
}

/// `C:\long\path\to\app\src\Row.tsx:12` as `src/Row.tsx:12`.
export function shortPlace(at: string): string {
    const parts = at.replace(/\\/g, '/').split('/');
    return parts.slice(-2).join('/');
}

type Tile = { name: string; weight: number; color: string; tip: string };

/// A treemap made by halving: the tiles are split into two lots of about the same weight, side by side, and each lot again
/// the other way, so each tile's area is its share.
function Tiles({ tiles, across: sideways, theme, depth = 0 }: { tiles: Tile[]; across: boolean; theme: Theme; depth?: number }) {
    if (tiles.length === 0) {
        return null;
    }
    if (tiles.length === 1) {
        const tile = tiles[0];
        return (
            <column flex={1} minWidth={0} minHeight={0} padding={1}>
                <column flex={1} minWidth={0} minHeight={0} background={tile.color} radius={3} padding={4} tooltip={tile.tip} name={tile.name}>
                    <text fontSize={11} fontFamily={UI} color="#111" noWrap ellipsis>{tile.name}</text>
                    <text fontSize={10} fontFamily={UI} color="#333" noWrap ellipsis>{tile.tip.split('\n')[1] ?? ''}</text>
                </column>
            </column>
        );
    }
    const total = tiles.reduce((sum, tile) => sum + tile.weight, 0);
    let half = 0;
    let split = 0;
    while (split < tiles.length - 1 && half + tiles[split].weight <= total / 2) {
        half += tiles[split].weight;
        split += 1;
    }
    split = Math.max(1, split);
    half = tiles.slice(0, split).reduce((sum, tile) => sum + tile.weight, 0);
    const first = tiles.slice(0, split);
    const rest = tiles.slice(split);
    const Holder = sideways ? 'row' : 'column';
    return (
        <Holder flex={1} minWidth={0} minHeight={0}>
            <column flex={half} minWidth={0} minHeight={0}>
                <Tiles tiles={first} across={!sideways} theme={theme} depth={depth + 1} />
            </column>
            <column flex={total - half} minWidth={0} minHeight={0}>
                <Tiles tiles={rest} across={!sideways} theme={theme} depth={depth + 1} />
            </column>
        </Holder>
    );
}

const TILE_COLORS = ['#7aa6e8', '#9b84e4', '#62b36a', '#e3b341', '#5fb6c9', '#c87fd7', '#e88a6a', '#8cc4a0', '#d6a2c8', '#a8b86a'];

/// What took the most time in the range, as a treemap: by component, or by function JavaScript was in.
function Treemap({ spans, from, to, rows, theme }: { spans: Spans; from: number; to: number; rows: Row[]; theme: Theme }) {
    const [by, setBy] = useState<'components' | 'functions'>('components');
    const tiles = useMemo(() => {
        if (by === 'components') {
            return byComponent(spans, from, to)
                .filter((row) => row.self > 0)
                .slice(0, 60)
                .map((row, index) => ({
                    name: row.name,
                    weight: row.self,
                    color: row.wasted > row.renders / 2 ? '#e3b341' : TILE_COLORS[index % TILE_COLORS.length],
                    tip: `${row.name}\n${formatNs(row.self)} over ${row.renders} renders${row.wasted ? `, ${row.wasted} changing nothing` : ''}`,
                }));
        }
        return rows
            .filter((row) => row.category === 'js' && row.self > 0)
            .slice(0, 60)
            .map((row, index) => ({ name: row.name, weight: row.self, color: TILE_COLORS[index % TILE_COLORS.length], tip: `${row.name}\n${formatNs(row.self)} itself, at ${shortPlace(row.at)}` }));
    }, [spans.version, from, to, by, rows]);
    return (
        <column flex={1}>
            <Toolbar theme={theme}>
                <Action label="By component" onClick={() => setBy('components')} theme={theme} strong={by === 'components'} />
                <Action label="By function" onClick={() => setBy('functions')} theme={theme} strong={by === 'functions'} />
                <text fontSize={12} fontFamily={UI} color={theme.dim} paddingX={8}>
                    {by === 'components' ? 'Each component by the time it took itself. Yellow ones mostly rendered for nothing.' : 'Each function by the time JavaScript spent in it itself, while recording.'}
                </text>
            </Toolbar>
            {tiles.length === 0 ? (
                <Empty text={by === 'components' ? 'No component rendered in what’s picked.' : 'Nothing was recorded in what’s picked. Press Record, use the app, and pick that time.'} theme={theme} />
            ) : (
                <column flex={1} padding={6} minHeight={0}>
                    <Tiles tiles={tiles} across theme={theme} />
                </column>
            )}
        </column>
    );
}

/// What's known about the block chosen in the chart.
function Details({ spans, chosen, theme, openSource }: { spans: Spans; chosen: Chosen; theme: Theme; openSource?: (file: string, line: number) => void }) {
    if (!chosen) {
        return <Empty text="Click a block in the chart to see what it was. The wheel zooms, dragging moves along, and dragging across the frames at the top picks a range." theme={theme} />;
    }
    if (chosen.kind === 'sample') {
        const where = spans.where(chosen.label);
        return (
            <column padding={14} gap={8}>
                <text fontSize={14} fontFamily={MONO} color={theme.fn} selectable>{where.name}</text>
                <text fontSize={12} fontFamily={UI} color={theme.dim} selectable>{where.at}</text>
                <text fontSize={12} fontFamily={UI} color={theme.text}>{`JavaScript was sampled in it for ${formatNs(chosen.end - chosen.start)} here.`}</text>
                {openSource && where.file ? <row><Action label="Show in Sources" onClick={() => openSource(where.file, where.line)} theme={theme} /></row> : null}
            </column>
        );
    }
    const span = chosen.span;
    const kind = spans.kind(span);
    const causes = ['it was new', 'what was around it gave it different props', 'its state changed', 'it was moving'];
    return (
        <column padding={14} gap={8}>
            <row gap={8} align="center">
                <group width={12} height={12} radius={3} background={categoryColor(kind.category, theme)} />
                <text fontSize={14} fontFamily={UI} fontWeight="bold" color={theme.text} selectable>{spans.title(span)}</text>
            </row>
            <text fontSize={12} fontFamily={UI} color={theme.dim}>{kind.about}</text>
            <row gap={22}>
                <Stat label="Took" value={formatNs(span.end - span.start)} theme={theme} color={span.end - span.start > FRAME_NS ? theme.error : theme.text} />
                <Stat label="Itself" value={formatNs(span.self)} theme={theme} />
                <Stat label="Started" value={`${formatNs(span.start - spans.extent()[0])} in`} theme={theme} />
            </row>
            {kind.name === 'Render' ? (
                <text fontSize={12} fontFamily={UI} color={span.arg & 4 ? theme.warn : theme.text}>
                    {`It rendered because ${causes[span.arg & 3] ?? 'something changed'}${span.arg & 4 ? ', and what it gave was the same as before, so the render could have been left out.' : '.'}`}
                </text>
            ) : null}
        </column>
    );
}

/// One frame's renders, as the app kept them: each component, how long it took and why it rendered.
function FrameRenders({ frame, theme }: { frame: Frame | undefined; theme: Theme }) {
    if (!frame) {
        return <Empty text="Pick a range with a frame where something rendered, and its renders show here, with why each happened." theme={theme} />;
    }
    return (
        <column flex={1}>
            <Heading columns={['Component', 'Self time', 'Total time', 'Why']} widths={[240, 90, 90, 0]} theme={theme} />
            <scroll key="renders" flex={1}>
                {frame.handled.map((handled, index) => (
                    <text key={`handled-${index}`} fontSize={12} fontFamily={UI} color={theme.text} paddingX={8} paddingY={4} selectable>
                        {`${handled.action} on #${handled.key} ran ${handled.note ? handled.note + ' ' : ''}for ${ms(handled.took)}${handled.rejected ? ', and was turned down' : ''}`}
                    </text>
                ))}
                {frame.renders.length === 0 ? <text fontSize={12} fontFamily={UI} color={theme.dim} padding={8}>Nothing rendered in this frame.</text> : null}
                {frame.renders.map((render, index) => (
                    <column key={`render-${index}`}>
                        <row height={22} align="center" hover={theme.hover}>
                            <row width={240} align="center" paddingLeft={8 + Math.min(render.depth, 8) * 10} gap={6}>
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
            </scroll>
        </column>
    );
}

const HEAT_MODES = ['Off', 'Render time', 'Renders', 'Wasted renders'];

/// Works out how hot each element is over the frames in a range, for the heatmap drawn over the app.
function heatCells(frames: Frame[], mode: string): [number[], number, string][] {
    const by = new Map<string, { path: number[]; value: number; name: string }>();
    for (const frame of frames) {
        for (const render of frame.renders) {
            const path = (render as any).path as number[] | null;
            if (!path) {
                continue;
            }
            const value = mode === 'Render time' ? render.own : mode === 'Renders' ? 1 : render.changed ? 0 : 1;
            if (value <= 0) {
                continue;
            }
            const key = path.join('/');
            const kept = by.get(key) ?? { path, value: 0, name: render.name ?? '' };
            kept.value += value;
            by.set(key, kept);
        }
    }
    const most = Math.max(0, ...[...by.values()].map((cell) => cell.value));
    if (most <= 0) {
        return [];
    }
    return [...by.values()]
        .sort((a, b) => b.value - a.value)
        .slice(0, 400)
        .map((cell) => [cell.path, cell.value / most, `${cell.name} ${mode === 'Render time' ? ms(cell.value) : `×${cell.value}`}`]);
}

/// The Performance tab.
export function Timeline({
    spans,
    frames,
    live,
    ask,
    summary,
    openSource,
    theme,
}: {
    spans: Spans;
    frames: Frame[];
    live: boolean;
    ask?: (request: object) => Promise<any>;
    summary?: () => Promise<string>;
    openSource?: (file: string, line: number) => void;
    theme: Theme;
}) {
    const [view, setView] = useState<View>(null);
    const [chosen, choose] = useState<Chosen>(null);
    const [below, setBelow] = useState('Summary');
    const [heat, setHeat] = useState('Off');
    const [said, setSaid] = useState<string | null>(null);
    const [text, setText] = useState('');
    const [first, last] = spans.extent();
    // Following what's newest, unless a range has been picked.
    // A trace that was saved is shown whole, to begin with.
    const shown = view ?? (live ? { from: Math.max(first, last - FOLLOW_NS), to: last } : { from: first, to: last });
    const from = Math.max(first - FOLLOW_NS, shown.from);
    const to = Math.max(from + 20_000, shown.to);
    const rows = useMemo(() => (below === 'Bottom-up' || below === 'Treemap' ? bottomUp(spans, from, to) : []), [spans.version, from, to, below]);
    // The frames the app kept renders for that were painted in the range.
    const kept = useMemo(() => frames.filter((frame) => (frame as any).painted >= from && (frame as any).painted <= to), [frames, from, to]);
    const busiest = useMemo(() => kept.reduce<Frame | undefined>((most, frame) => (!most || frame.renders.length > most.renders.length ? frame : most), undefined), [kept]);

    // The heatmap over the app follows the range.
    useEffect(() => {
        if (!ask) {
            return;
        }
        const timer = setTimeout(() => {
            ask({ ask: 'heat', cells: heat === 'Off' ? [] : heatCells(kept, heat) });
        }, 120);
        return () => clearTimeout(timer);
    }, [heat, kept, ask]);
    useEffect(
        () => () => {
            ask?.({ ask: 'heat', cells: [] });
        },
        [],
    );

    const record = async () => {
        const answer = await ask?.({ ask: 'record', on: !spans.recording });
        if (answer?.error) {
            setSaid(answer.error);
        }
    };

    if (spans.frames.length === 0 && spans.spans.length === 0) {
        return (
            <column flex={1}>
                <Toolbar theme={theme}>
                    {live ? <IconButton name={spans.recording ? 'Stop recording JavaScript' : 'Record JavaScript'} icon="record" on={spans.recording} color={spans.recording ? theme.error : undefined} onClick={record} theme={theme} /> : null}
                </Toolbar>
                <Empty text={live ? 'Nothing has been drawn yet. Use the app, and each frame shows here, with where its time went.' : 'This trace has no timings in it. Traces saved by an older rsui only have frames: see Rendering.'} theme={theme} />
            </column>
        );
    }

    return (
        <column flex={1}>
            <Toolbar theme={theme}>
                {live ? <IconButton name={spans.recording ? 'Stop recording JavaScript' : 'Record JavaScript'} icon="record" on={spans.recording} color={spans.recording ? theme.error : undefined} onClick={record} theme={theme} /> : null}
                {live ? (
                    <IconButton name={view === null ? 'Following what’s newest' : 'Follow what’s newest'} icon="live" on={view === null} onClick={() => setView(null)} theme={theme} />
                ) : null}
                <IconButton name="Zoom out" icon="zoomOut" onClick={() => {
                    const middle = (from + to) / 2;
                    const wide = Math.min(to - from, 15_000 * NS);
                    setView({ from: middle - wide, to: middle + wide });
                }} theme={theme} />
                <IconButton name="Show everything kept" icon="fit" onClick={() => setView({ from: first, to: last })} theme={theme} />
                <Down theme={theme} />
                <text fontSize={12} fontFamily={UI} color={theme.dim} paddingX={6} noWrap>
                    {`${formatNs(to - from)} shown, from ${formatNs(Math.max(0, from - first))}${spans.level < 2 ? ' · light detail until the inspector opened' : ''}`}
                </text>
                <group flex={1} />
                {live ? <text fontSize={12} fontFamily={UI} color={theme.dim} paddingX={4}>Heat in app</text> : null}
                {live
                    ? HEAT_MODES.map((mode) => <Action key={`heat-${mode}`} label={mode} onClick={() => setHeat(mode)} theme={theme} strong={heat === mode} />)
                    : null}
            </Toolbar>
            {said ? <text fontSize={12} fontFamily={UI} color={theme.error} paddingX={10} paddingY={4}>{said}</text> : null}
            <Overview spans={spans} view={{ from, to }} setView={setView} theme={theme} />
            <column flex={3} minHeight={140}>
                <FlameChart spans={spans} from={from} to={to} setView={setView} chosen={chosen} choose={(one) => {
                    choose(one);
                    setBelow('Details');
                }} theme={theme} />
            </column>
            <Across theme={theme} />
            <column flex={2} minHeight={120}>
                <row height={28} align="center" background={theme.bar} paddingX={4}>
                    {['Summary', 'Details', 'Bottom-up', 'Treemap', 'Renders'].map((tab) => (
                        <button key={`below-${tab}`} name={tab} onClick={() => setBelow(tab)} height={28} paddingX={10} direction="column" justify="center" hover={theme.hover}>
                            <group flex={1} />
                            <text fontSize={12} fontFamily={UI} color={tab === below ? theme.text : theme.dim} noWrap>{tab}</text>
                            <group flex={1} />
                            <group height={2} width="100%" background={tab === below ? theme.accent : 'transparent'} />
                        </button>
                    ))}
                    <group flex={1} />
                    {summary ? (
                        <Action
                            label="As text"
                            onClick={async () => {
                                setText(await summary());
                                setBelow('Text');
                            }}
                            theme={theme}
                        />
                    ) : null}
                </row>
                <Across theme={theme} />
                {below === 'Summary' ? <scroll key="summary" flex={1}><Summary spans={spans} from={from} to={to} frames={kept} theme={theme} /></scroll> : null}
                {below === 'Details' ? <scroll key="details" flex={1}><Details spans={spans} chosen={chosen} theme={theme} openSource={openSource} /></scroll> : null}
                {below === 'Bottom-up' ? <BottomUp rows={rows} theme={theme} openSource={openSource} /> : null}
                {below === 'Treemap' ? <Treemap spans={spans} from={from} to={to} rows={rows} theme={theme} /> : null}
                {below === 'Renders' ? <FrameRenders frame={busiest} theme={theme} /> : null}
                {below === 'Text' ? (
                    <scroll key="text" flex={1} padding={12}>
                        <text fontSize={12} fontFamily={MONO} color={theme.text} selectable>{text || 'Working it out.'}</text>
                    </scroll>
                ) : null}
            </column>
        </column>
    );
}
