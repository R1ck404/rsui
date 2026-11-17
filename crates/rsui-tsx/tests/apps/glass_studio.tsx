// Glass studio: one window that's seen through, with the controls for what's behind it, and the window options around it.
//
//   - Backdrop: Solid, Clear (the desktop shows straight through), Mica, Mica Alt or Acrylic (the blur one), switched live.
//   - Tint, light or dark text, and an accent color for how it looks over whatever the backdrop is.
//   - Title bar: the system's, or one of this app's own (no frame, drag from the bar, its own minimize / maximize / close, and
//     grips to resize from), with a title you type.
//   - Always on top.
//   - Widgets: Add one to the desktop (a clock, a calendar or a note, in the layer you pick) and Remove it again, one at a time or
//     all together. Each is a frameless, transparent window of its own, left off the taskbar and on every virtual desktop.
//
// Mica, Mica Alt and Acrylic are Windows 11 (22H2 and later); on Windows 10 they leave the window as it was, and Clear and the
// widgets still work. Widgets in the `desktop` layer sit behind the icons, so the pointer can't reach them: remove those from the
// list here. `bottom` is beneath every other window but can be dragged and closed; Win+D hides it with the rest.
//
// It changes the real desktop, so it's for trying by hand: closing this window takes all the widgets with it. Run it with
// `cargo run --release -p rsui-tsx -- crates/rsui-tsx/examples/glass_studio.tsx`.

import { Line, mainWindow, Polyline, Rect, Slider, Svg, Switch, TextInput, useColorScheme, useCurrentWindow, useEffect, useState, useWindowState, Window } from 'rsui';
import { system } from 'rsui/desktop';

export const title = 'Glass studio';
export const size = [860, 600];
export const minSize = [700, 520];

type Backdrop = 'solid' | 'clear' | 'mica' | 'tabbed' | 'acrylic';
type Layer = 'desktop' | 'bottom' | 'top';
type Kind = 'clock' | 'calendar' | 'note';
interface Widget {
    id: number;
    kind: Kind;
    layer: Layer;
}

const BACKDROPS: { id: Backdrop; label: string }[] = [
    { id: 'solid', label: 'Solid' },
    { id: 'clear', label: 'Clear' },
    { id: 'mica', label: 'Mica' },
    { id: 'tabbed', label: 'Mica Alt' },
    { id: 'acrylic', label: 'Acrylic (blur)' },
];
const ACCENTS = ['#4c9aff', '#3ccf91', '#ff6fa5', '#ffb454'];
const KINDS: { id: Kind; label: string; size: [number, number] }[] = [
    { id: 'clock', label: 'Clock', size: [300, 150] },
    { id: 'calendar', label: 'Calendar', size: [268, 264] },
    { id: 'note', label: 'Note', size: [250, 210] },
];
const LAYERS: { id: Layer; label: string }[] = [
    { id: 'desktop', label: 'Desktop' },
    { id: 'bottom', label: 'Bottom' },
    { id: 'top', label: 'On top' },
];

const DARK = { ink: '#f3f5f8', muted: '#f3f5f8a8', base: '#101218', card: '#ffffff12', chip: '#ffffff1c', line: '#ffffff24' };
const LIGHT = { ink: '#16181d', muted: '#16181d9e', base: '#f8f9fb', card: '#ffffff8c', chip: '#00000012', line: '#0000001f' };
type Theme = typeof DARK;

const two = (n: number) => String(n).padStart(2, '0');

function useNow() {
    const [now, setNow] = useState(() => new Date());
    useEffect(() => {
        const timer = setInterval(() => setNow(new Date()), 1000);
        return () => clearInterval(timer);
    }, []);
    return now;
}

function Choice({ id, chosen, accent, theme, onClick, children }: { id: string; chosen: boolean; accent: string; theme: Theme; onClick: () => void; children: any }) {
    return (
        <button key={id} id={id} paddingX={11} paddingY={6} radius={7} background={chosen ? accent : theme.chip} color={chosen ? '#0b1220' : theme.ink} fontWeight={chosen ? 'bold' : undefined} onClick={onClick}>
            {children}
        </button>
    );
}

function Section({ theme, heading, children }: { theme: Theme; heading: string; children?: any }) {
    return (
        <column padding={14} gap={9} radius={12} background={theme.card} borderColor={theme.line} borderWidth={1}>
            <text fontWeight="bold" fontSize={13} color={theme.muted}>{heading}</text>
            {children}
        </column>
    );
}

// ---- widgets: each is a frameless, transparent window that only shows what's drawn in it ----

function WidgetCard({ layer, onRemove, children }: { layer: Layer; onRemove: () => void; children?: any }) {
    return (
        <column flex={1} padding={6}>
            <column flex={1} padding={14} radius={18} background="#0e121c94" borderColor="#ffffff29" borderWidth={1} dragsWindow>
                {children}
                {layer !== 'desktop' ? (
                    <row justify="end">
                        <button key="x" id="widget-x" paddingX={8} paddingY={2} radius={6} background="#ffffff1a" hover="#ff5a5ab2" color="#ffffff" onClick={onRemove}>
                            Remove
                        </button>
                    </row>
                ) : null}
            </column>
        </column>
    );
}

function ClockWidget() {
    const now = useNow();
    return (
        <column flex={1} align="center" justify="center" gap={2}>
            <text fontSize={44} fontWeight="bold" color="#ffffff">{`${two(now.getHours())}:${two(now.getMinutes())}:${two(now.getSeconds())}`}</text>
            <text fontSize={14} color="#ffffffcc">{now.toDateString()}</text>
        </column>
    );
}

function CalendarWidget({ accent }: { accent: string }) {
    const now = useNow();
    const year = now.getFullYear();
    const month = now.getMonth();
    const lead = (new Date(year, month, 1).getDay() + 6) % 7; // Monday first.
    const days = new Date(year, month + 1, 0).getDate();
    const cells: (number | null)[] = [];
    for (let i = 0; i < lead; i++) cells.push(null);
    for (let d = 1; d <= days; d++) cells.push(d);
    while (cells.length % 7 !== 0) cells.push(null);
    const weeks: (number | null)[][] = [];
    for (let i = 0; i < cells.length; i += 7) weeks.push(cells.slice(i, i + 7));
    const names = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
    return (
        <column flex={1} gap={3} align="center">
            <text fontWeight="bold" fontSize={15} color="#ffffff">{`${names[month]} ${year}`}</text>
            <row gap={2}>
                {['M', 'T', 'W', 'T', 'F', 'S', 'S'].map((d, i) => (
                    <column key={i} width={30} align="center"><text fontSize={12} color="#ffffff99">{d}</text></column>
                ))}
            </row>
            {weeks.map((week, w) => (
                <row key={w} gap={2}>
                    {week.map((d, i) => (
                        <column key={i} width={30} paddingY={2} radius={8} align="center" background={d === now.getDate() ? accent : undefined}>
                            <text fontSize={13} color={d === now.getDate() ? '#0b1220' : '#ffffff'} fontWeight={d === now.getDate() ? 'bold' : undefined}>{d === null ? ' ' : String(d)}</text>
                        </column>
                    ))}
                </row>
            ))}
        </column>
    );
}

function NoteWidget() {
    const [text, setText] = useState('Click here and type.');
    return (
        <column flex={1} gap={6}>
            <text fontWeight="bold" fontSize={13} color="#ffffffb2">Note</text>
            <TextInput key="note-text" value={text} onChange={setText} rows={4} />
        </column>
    );
}

/// Where a new widget goes: stepped down and across from the top left, so they don't land on each other.
function Placed({ index, children }: { index: number; children?: any }) {
    const here = useCurrentWindow();
    useEffect(() => {
        here.moveTo(90 + (index % 6) * 46, 110 + (index % 6) * 46);
        here.showWithoutActivation();
    }, []);
    return children;
}

function WidgetWindow({ widget, index, accent, onRemove }: { widget: Widget; index: number; accent: string; onRemove: () => void }) {
    const spec = KINDS.find((k) => k.id === widget.kind)!;
    return (
        <Window title={`${spec.label} widget`} size={spec.size} owned={false} frameless transparent layer={widget.layer} skipTaskbar allDesktops onClose={onRemove}>
            <Placed index={index}>
                <WidgetCard layer={widget.layer} onRemove={onRemove}>
                    {widget.kind === 'clock' ? <ClockWidget /> : widget.kind === 'calendar' ? <CalendarWidget accent={accent} /> : <NoteWidget />}
                </WidgetCard>
            </Placed>
        </Window>
    );
}

// ---- the window's own title bar, for when the system's is turned off ----

/// The three buttons are drawn as shapes, not font glyphs, so they look the same on every machine.
function Glyph({ kind, color }: { kind: 'min' | 'max' | 'restore' | 'close'; color: string }) {
    return (
        <Svg viewBox="0 0 10 10" width={10} height={10} fill="none" stroke={color} strokeWidth={1}>
            {kind === 'min' ? <Line x1={0} y1={5} x2={10} y2={5} /> : null}
            {kind === 'max' ? <Rect x={0.5} y={0.5} width={9} height={9} /> : null}
            {kind === 'restore' ? <Rect x={0.5} y={2.5} width={7} height={7} /> : null}
            {kind === 'restore' ? <Polyline points="2.5,2.5 2.5,0.5 9.5,0.5 9.5,7.5 7.5,7.5" /> : null}
            {kind === 'close' ? <Line x1={0} y1={0} x2={10} y2={10} /> : null}
            {kind === 'close' ? <Line x1={10} y1={0} x2={0} y2={10} /> : null}
        </Svg>
    );
}

function TitleBar({ text, theme, maximized }: { text: string; theme: Theme; maximized: boolean }) {
    const cell = { width: 46, height: 34, align: 'center', justify: 'center' } as const;
    const toggle = () => (maximized ? mainWindow.restore() : mainWindow.maximize());
    return (
        <row align="center" dragsWindow onDoubleClick={toggle}>
            <text flex={1} paddingX={14} fontSize={13} fontWeight="bold" color={theme.ink}>{text}</text>
            <button key="min" id="min" {...cell} background="#00000000" hover={theme.chip} onClick={() => mainWindow.minimize()}><Glyph kind="min" color={theme.ink} /></button>
            <button key="max" id="max" {...cell} background="#00000000" hover={theme.chip} onClick={toggle}><Glyph kind={maximized ? 'restore' : 'max'} color={theme.ink} /></button>
            <button key="close" id="close" {...cell} background="#00000000" hover="#e5484d" onClick={() => mainWindow.close()}><Glyph kind="close" color={theme.ink} /></button>
        </row>
    );
}

/// A color with an opacity. The native layer takes `#rrggbbaa`, and ignores `rgb()` and `rgba()` strings, as it does names.
const withAlpha = (hex: string, alpha: number) => hex.slice(0, 7) + Math.round(alpha * 255).toString(16).padStart(2, '0');

export default function App() {
    const scheme = useColorScheme();
    const state = useWindowState();
    const [backdrop, setBackdrop] = useState<Backdrop>('mica');
    const [dark, setDark] = useState(scheme === 'dark');
    const [shade, setShade] = useState(0.2);
    const [wash, setWash] = useState(0.3);
    const [accent, setAccent] = useState(ACCENTS[0]);
    const [custom, setCustom] = useState(false);
    const [onTop, setOnTop] = useState(false);
    const [winTitle, setWinTitle] = useState('Glass studio');
    const [widgets, setWidgets] = useState<Widget[]>([]);
    const [nextId, setNextId] = useState(1);
    const [kind, setKind] = useState<Kind>('clock');
    const [layer, setLayer] = useState<Layer>('bottom');
    const theme = dark ? DARK : LIGHT;

    // What's behind the window. Clear is just "transparent": nothing of the system's, so the desktop shows where nothing's drawn.
    useEffect(() => {
        mainWindow.setTransparent(backdrop === 'clear');
        mainWindow.setBackdrop(backdrop === 'mica' || backdrop === 'tabbed' || backdrop === 'acrylic' ? backdrop : 'none');
    }, [backdrop]);
    useEffect(() => mainWindow.setFrameless(custom), [custom]);
    useEffect(() => mainWindow.setAlwaysOnTop(onTop), [onTop]);
    useEffect(() => mainWindow.setTitle(winTitle || 'Glass studio'), [winTitle]);

    const add = () => {
        setWidgets([...widgets, { id: nextId, kind, layer }]);
        setNextId(nextId + 1);
    };
    const remove = (id: number) => setWidgets((all) => all.filter((w) => w.id !== id));
    const label = (w: Widget) => `${KINDS.find((k) => k.id === w.kind)!.label} #${w.id}`;
    // Two layers over the backdrop: a dark (or light) shade, and a wash of the accent color, which shows even on a dark Mica.
    const veil = backdrop === 'solid' ? theme.base : withAlpha(theme.base, shade);
    const tint = withAlpha(accent, wash);
    const pct = (n: number) => `${Math.round(n * 100)}%`;
    const grip = (edge: 'right' | 'bottom' | 'bottom-right') => (custom ? { onPointerDown: () => mainWindow.startResize(edge) } : {});

    return (
        <window>
            <column flex={1} background={veil} color={theme.ink}>
              <column flex={1} background={tint}>
                {custom ? <TitleBar text={winTitle} theme={theme} maximized={state.maximized} /> : null}
                <row flex={1}>
                    <column flex={1} padding={16} gap={12}>
                        <Section theme={theme} heading="BACKGROUND">
                            <row gap={6}>
                                {BACKDROPS.map((b) => (
                                    <Choice key={b.id} id={`backdrop-${b.id}`} chosen={backdrop === b.id} accent={accent} theme={theme} onClick={() => setBackdrop(b.id)}>{b.label}</Choice>
                                ))}
                            </row>
                            <text color={theme.muted}>{`Shade (dark/light veil): ${pct(shade)}`}</text>
                            <Slider id="shade" value={shade} min={0} max={0.95} step={0.05} width={300} onChange={setShade} />
                            <text color={theme.muted}>{`Tint (accent color wash): ${pct(wash)}`}</text>
                            <Slider id="wash" value={wash} min={0} max={0.8} step={0.05} width={300} onChange={setWash} />
                        </Section>
                        <Section theme={theme} heading="LOOK">
                            <row gap={14} align="center">
                                <Switch id="dark" label="Dark" checked={dark} onChange={setDark} />
                                <text color={theme.muted}>Accent</text>
                                <row gap={6}>
                                    {ACCENTS.map((c) => (
                                        <button key={c} id={`accent-${c}`} width={26} height={26} radius={13} background={c} borderWidth={accent === c ? 3 : 0} borderColor={theme.ink} onClick={() => setAccent(c)} />
                                    ))}
                                </row>
                                <button key="sys-accent" id="sys-accent" paddingX={9} paddingY={5} radius={7} background={theme.chip} color={theme.ink} onClick={() => { const c = system.accentColor(); if (c) setAccent(c.slice(0, 7)); }}>
                                    Windows accent
                                </button>
                            </row>
                        </Section>
                        <Section theme={theme} heading="WINDOW">
                            <Switch id="custom-bar" label="Custom title bar (no system frame)" checked={custom} onChange={setCustom} />
                            <Switch id="on-top" label="Always on top" checked={onTop} onChange={setOnTop} />
                            <row gap={8} align="center">
                                <text color={theme.muted}>Title</text>
                                <TextInput key="title-input" flex={1} value={winTitle} onChange={setWinTitle} placeholder="Window title" />
                            </row>
                        </Section>
                    </column>
                    <column width={330} padding={16} paddingLeft={0} gap={12}>
                        <Section theme={theme} heading="DESKTOP WIDGETS">
                            <row gap={6}>
                                {KINDS.map((k) => (
                                    <Choice key={k.id} id={`kind-${k.id}`} chosen={kind === k.id} accent={accent} theme={theme} onClick={() => setKind(k.id)}>{k.label}</Choice>
                                ))}
                            </row>
                            <row gap={6}>
                                {LAYERS.map((l) => (
                                    <Choice key={l.id} id={`layer-${l.id}`} chosen={layer === l.id} accent={accent} theme={theme} onClick={() => setLayer(l.id)}>{l.label}</Choice>
                                ))}
                            </row>
                            <row gap={6}>
                                <button key="add" id="add-widget" paddingX={12} paddingY={7} radius={7} background={accent} color="#0b1220" fontWeight="bold" onClick={add}>Add to desktop</button>
                                <button key="clear" id="remove-all" paddingX={12} paddingY={7} radius={7} background={theme.chip} color={theme.ink} disabled={widgets.length === 0} onClick={() => setWidgets([])}>Remove all</button>
                            </row>
                            <text color={theme.muted} fontSize={12}>
                                {layer === 'desktop' ? 'Behind the icons, like wallpaper: it can\'t be clicked, so remove it from the list.' : layer === 'bottom' ? 'Under every window, but you can drag it and remove it.' : 'Above every window.'}
                            </text>
                        </Section>
                        <Section theme={theme} heading={`ON THE DESKTOP (${widgets.length})`}>
                            {widgets.length === 0 ? <text color={theme.muted}>Nothing yet.</text> : null}
                            {widgets.map((w) => (
                                <row key={w.id} gap={8} align="center">
                                    <text flex={1}>{`${label(w)}, ${w.layer}`}</text>
                                    <button key={`rm-${w.id}`} id={`remove-${w.id}`} paddingX={10} paddingY={4} radius={6} background={theme.chip} hover="#e5484db2" color={theme.ink} onClick={() => remove(w.id)}>Remove</button>
                                </row>
                            ))}
                        </Section>
                    </column>
                    {custom ? <column width={6} {...grip('right')} /> : null}
                </row>
                {custom ? (
                    <row height={8}>
                        <column flex={1} {...grip('bottom')} />
                        <column width={14} {...grip('bottom-right')} />
                    </row>
                ) : null}
              </column>
            </column>
            {widgets.map((w, i) => (
                <WidgetWindow key={w.id} widget={w} index={i} accent={accent} onRemove={() => remove(w.id)} />
            ))}
        </window>
    );
}
