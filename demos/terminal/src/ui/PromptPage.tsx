import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { DEFAULT_PROMPT, PROMPT_SEGMENTS, previewPrompt, promptSegmentOrder } from '../prompt';
import type { PromptSettings, SegmentId } from '../prompt';
import { settingsStore, usePromptSettings } from '../settings';
import type { Settings } from '../settings';
import { Row } from '../term/Row';
import { lookFor } from '../term/TerminalPane';
import { css, withAlpha } from '../themes';
import type { Scheme } from '../themes';
import { Terminal } from '../vt/terminal';
import { ColorEditor } from './ColorEditor';
import { Icon } from './Icon';
import { Field, Section, Segmented, Switch } from './controls';
import { useReorder } from './reorder';

interface Props {
    settings: Settings;
    scheme: Scheme;
    change: (patch: Partial<Settings>) => void;
}

export function PromptPage({ settings, scheme, change }: Props) {
    const config = usePromptSettings();
    const [customize, setCustomize] = useState(false);
    const [editing, setEditing] = useState<SegmentId | null>(null);
    const update = useCallback((patch: Partial<PromptSettings>) => change({ prompt: { ...settingsStore.get().prompt, ...patch } }), [change]);
    const button = { background: withAlpha(scheme.fg, 0.09) };

    return (
        <>
            <PromptPreview settings={settings} config={config} scheme={scheme} />
            <Field label="Style" scheme={scheme}>
                <Segmented scheme={scheme} value={config.style} options={[
                    { value: 'shell', label: 'Shell default' },
                    { value: 'minimal', label: 'Minimal' },
                    { value: 'powerline', label: 'Powerline' },
                ]} onChange={(style) => update({ style })} />
            </Field>
            <p className="mb-6 mt-2 text-[12px]" style={{ color: withAlpha(scheme.fg, 0.62) }}>
                Prompt styles apply when you open a new PowerShell tab.
            </p>
            {config.style !== 'shell' ? (
                <>
                    <button className="mb-4 flex items-center gap-2 rounded-md px-3 py-1.5 text-[12px]" style={button}
                        aria-expanded={customize} onClick={() => setCustomize(!customize)}>
                        <Icon name={customize ? 'up' : 'down'} size={12} />
                        Customize prompt
                    </button>
                    {customize ? (
                        <>
                            <Section title="Layout" scheme={scheme}>
                                {config.style === 'powerline' ? (
                                    <Field label="Segment edges" scheme={scheme}>
                                        <Segmented scheme={scheme} value={config.edge} options={[
                                            { value: 'arrow', label: 'Arrow' }, { value: 'round', label: 'Round' }, { value: 'flat', label: 'Flat' },
                                        ]} onChange={(edge) => update({ edge })} />
                                    </Field>
                                ) : null}
                                {config.order.includes('folder') ? (
                                    <Field label="Folder display" scheme={scheme}>
                                        <Segmented scheme={scheme} value={config.folder} options={[
                                            { value: 'name', label: 'Name' }, { value: 'home', label: 'With ~' }, { value: 'full', label: 'Full path' },
                                        ]} onChange={(folder) => update({ folder })} />
                                    </Field>
                                ) : null}
                                <Field label="Type on a new line" scheme={scheme}>
                                    <Switch label="Type on a new line" scheme={scheme} on={config.newLine} onChange={(newLine) => update({ newLine })} />
                                </Field>
                            </Section>
                            <PromptSegments scheme={scheme} style={config.style} order={config.order}
                                segmentOrder={config.segmentOrder} colorsBySegment={config.colors}
                                editing={editing} setEditing={setEditing} update={update} />
                        </>
                    ) : null}
                </>
            ) : null}
        </>
    );
}

interface SegmentProps {
    scheme: Scheme;
    style: PromptSettings['style'];
    order: SegmentId[];
    segmentOrder: PromptSettings['segmentOrder'];
    colorsBySegment: PromptSettings['colors'];
    editing: SegmentId | null;
    setEditing: (id: SegmentId | null) => void;
    update: (patch: Partial<PromptSettings>) => void;
}

const PromptSegments = memo(function PromptSegments(props: SegmentProps) {
    const { scheme, style, order, segmentOrder, colorsBySegment, editing, setEditing, update } = props;
    const ids = useMemo(() => promptSegmentOrder({ order, segmentOrder }), [order, segmentOrder]);
    const move = (from: number, to: number) => {
        if (to < 0 || to >= ids.length) return;
        const next = [...ids];
        const [id] = next.splice(from, 1);
        next.splice(to, 0, id);
        update({ order: next.filter((segment) => order.includes(segment)), segmentOrder: next });
    };
    const reorder = useReorder(ids, 'Y', move);
    const button = { background: withAlpha(scheme.fg, 0.09) };

    return (
        <>
            <Section title="Segments" scheme={scheme}>
                <div className="flex flex-col gap-1">
                    {ids.map((id, i) => {
                        const enabled = order.includes(id);
                        const part = PROMPT_SEGMENTS[id];
                        const colors = colorsBySegment[id];
                        const fg = colors?.fg ?? (style === 'minimal' ? scheme.fg : part.fg);
                        const bg = colors?.bg ?? part.bg;
                        const setColor = (key: 'fg' | 'bg', value: number) =>
                            update({ colors: { ...colorsBySegment, [id]: { ...colors, [key]: value } } });
                        return (
                            <div key={id}>
                                <div ref={(element) => { reorder.bind(id, element); }}
                                    className="settings-card reorder-item relative rounded-lg px-2 py-1.5">
                                    <div className="flex min-h-8 flex-wrap items-center gap-x-2 gap-y-1">
                                        <button
                                            type="button"
                                            aria-label={'Reorder ' + part.label}
                                            title="Drag to reorder or use the up and down arrow keys"
                                            className="prompt-segment-grip flex size-8 shrink-0 touch-none items-center justify-center rounded-md focus-visible:outline-2"
                                            style={{ color: withAlpha(scheme.fg, 0.56) }}
                                            onPointerDown={(event) => reorder.start(i, event)}
                                            onPointerMove={reorder.move}
                                            onPointerUp={reorder.drop}
                                            onKeyDown={(event) => {
                                                if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
                                                    event.preventDefault();
                                                    move(i, i + (event.key === 'ArrowUp' ? -1 : 1));
                                                }
                                            }}
                                        >
                                            <Icon name="grip" size={16} />
                                        </button>
                                        <Switch label={part.label} scheme={scheme} on={enabled} onChange={(on) =>
                                            update({ order: ids.filter((item) => item === id ? on : order.includes(item)), segmentOrder: ids })} />
                                        <span className="min-w-0 flex-1 text-[12px]" style={{ opacity: enabled ? 1 : 0.65 }}>{part.label}</span>
                                        {enabled ? (
                                            <button aria-label={part.label + ' colors'} aria-expanded={editing === id}
                                                className="flex items-center gap-1.5 rounded-md px-2 py-1 text-[11px]" style={button}
                                                onClick={() => setEditing(editing === id ? null : id)}>
                                                <span className="size-2.5 rounded-sm" style={{ background: css(style === 'powerline' ? bg : fg) }} />
                                                Colors
                                            </button>
                                        ) : null}
                                    </div>
                                    {enabled && editing === id ? (
                                        <div className="flex flex-wrap gap-3 py-2">
                                            <ColorEditor key={fg} scheme={scheme} label="Text" value={fg} onChange={(value) => setColor('fg', value)} />
                                            {style === 'powerline' ? (
                                                <ColorEditor key={'bg' + bg} scheme={scheme} label="Fill" value={bg} onChange={(value) => setColor('bg', value)} />
                                            ) : null}
                                        </div>
                                    ) : null}
                                </div>
                            </div>
                        );
                    })}
                </div>
            </Section>
            <button className="rounded-md px-3 py-1.5 text-[12px]" style={button} onClick={() => {
                setEditing(null);
                update({ ...DEFAULT_PROMPT, style });
            }}>Reset prompt customization</button>
        </>
    );
});

function PromptPreview({ settings, config, scheme }: { settings: Settings; config: PromptSettings; scheme: Scheme }) {
    const ref = useRef<HTMLDivElement>(null);
    const [width, setWidth] = useState(480);
    useEffect(() => {
        const el = ref.current;
        if (!el) return;
        const measure = () => setWidth(el.getBoundingClientRect().width);
        const observer = new ResizeObserver(measure);
        measure();
        observer.observe(el);
        return () => observer.disconnect();
    }, []);
    const look = lookFor({ ...settings, showMarks: false }, scheme);
    const cols = Math.max(1, Math.floor((width - look.gutter) / look.cw));
    const term = new Terminal(cols, Math.ceil(120 / cols) + 2, 0);
    term.write(previewPrompt(config));
    if (term.cursorPendingWrap) term.write('\r\n');
    const shape = settings.cursorShape;
    return (
        <div className="mb-4 rounded-lg px-4 py-5" style={{ background: css(scheme.bg), border: `1px solid ${withAlpha(scheme.fg, 0.15)}` }}>
            <div className="relative overflow-hidden" ref={ref} style={{ height: look.ch * (term.cursorY + 1) }}>
                {Array.from({ length: term.cursorY + 1 }, (_, y) => (
                    <Row key={y} cols={cols} look={look} line={term.lineAt(y)} ver={term.lineAt(y).ver} styles={term.styles}
                        paletteMap={term.palette} ansi={scheme.ansi} palette={0} sel="" found="" link="" mark="" note="" />
                ))}
                <div className="absolute" aria-hidden="true" style={{
                    left: look.gutter + term.cursorX * look.cw,
                    top: term.cursorY * look.ch + (shape === 'underline' ? look.ch - 2 : 0),
                    width: shape === 'bar' ? 2 : look.cw,
                    height: shape === 'underline' ? 2 : look.ch,
                    background: css(scheme.cursor),
                }} />
            </div>
        </div>
    );
}
