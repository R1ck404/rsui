import { SCHEMES, css, tokens, withAlpha } from '../themes';
import type { Scheme } from '../themes';
import { fitCrop, timecode } from '../term/recording-edits';
import { KEYSTROKE_CATEGORIES } from '../term/recording-keys';
import type { KeystrokeAnimation, KeystrokeCategory, KeystrokeDuration, KeystrokeLayout, KeystrokeLimit, KeystrokePosition } from '../term/recording-keys';
import type { RecordingEdits } from '../term/tab-session';
import { BACKGROUNDS } from './Background';
import { ColorEditor } from './ColorEditor';
import { Field, Switch } from './controls';
import { Icon } from './Icon';
import type { IconName } from './Icon';

export const editorButton = 'shrink-0 rounded-md px-2.5 py-1.5 text-[12px] font-medium hover:bg-current/8 focus-visible:outline-2 disabled:opacity-40';
const TOOLS = [
    { id: 'trim', label: 'Trim', icon: 'split' },
    { id: 'crop', label: 'Crop', icon: 'width' },
    { id: 'appearance', label: 'Appearance', icon: 'palette' },
    { id: 'keystrokes', label: 'Keystrokes', icon: 'keyboard' },
] as const satisfies ReadonlyArray<{ id: string; label: string; icon: IconName }>;

export type RecordingTool = typeof TOOLS[number]['id'];

export function RecordingCategories({ scheme, tool, onTool, busy }: {
    scheme: Scheme;
    tool: RecordingTool;
    onTool: (tool: RecordingTool) => void;
    busy: boolean;
}) {
    const t = tokens(scheme);
    const accent = css(scheme.ansi[4]);
    return (
        <nav className="px-1 pb-3" aria-label="Editing categories">
            <h2 className="px-1.5 pb-3 pt-1 text-[15px] font-semibold tracking-tight">Edit recording</h2>
            <div className="flex flex-col gap-0.5">
                {TOOLS.map(({ id, label, icon }) => (
                    <button key={id} aria-pressed={tool === id} disabled={busy}
                        className="flex h-8 items-center gap-2 rounded-md px-2.5 text-left text-[12px] font-medium focus-visible:outline-2"
                        style={{ color: tool === id ? accent : t.text, background: tool === id ? withAlpha(scheme.ansi[4], 0.13) : 'transparent' }}
                        onClick={() => onTool(id)}>
                        <Icon name={icon} size={15} />
                        {label}
                    </button>
                ))}
            </div>
        </nav>
    );
}

export function RecordingInspector({ scheme, colors, tool, edits, onChange, start, end, aspect, busy, onMark, onRevert, onHide,
    hasKeys, hasMatchingKeys, categories, position, hold, presentation, canStepBack, canStepForward, onStepKey }: {
    scheme: Scheme;
    colors: Scheme;
    tool: RecordingTool;
    edits: RecordingEdits;
    onChange: (edits: RecordingEdits) => void;
    start: number;
    end: number;
    aspect: number;
    busy: boolean;
    onMark: (edge: 'from' | 'to') => void;
    onRevert?: () => void;
    onHide: () => void;
} & KeystrokeControlsProps) {
    const t = tokens(scheme);
    return (
        <aside className="flex w-[254px] max-w-[44%] shrink-0 flex-col overflow-hidden" aria-label="Editing controls"
            aria-disabled={busy || undefined} style={{ background: t.panel }}>
            <div className="flex h-10 shrink-0 items-center justify-between px-3">
                <h2 className="text-[13px] font-semibold">{TOOLS.find(({ id }) => id === tool)?.label}</h2>
                <button className={editorButton + ' flex size-7 items-center justify-center px-0'} aria-label="Hide editing controls"
                    title="Hide editing controls" disabled={busy} onClick={onHide}><Icon name="close" size={13} /></button>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3">
                <div className="flex flex-col gap-3">
                    {tool === 'trim' ? (
                        <>
                            <div className="flex flex-col gap-2 text-[12px]">
                                <div className="flex items-center justify-between gap-1"><span style={{ color: t.sub }}>Start</span><span className="font-mono">{timecode(start)}</span></div>
                                <div className="flex items-center justify-between gap-1"><span style={{ color: t.sub }}>End</span><span className="font-mono">{timecode(end)}</span></div>
                                <div className="flex items-center justify-between gap-1"><span style={{ color: t.sub }}>Duration</span><span className="font-mono">{timecode(end - start)}</span></div>
                            </div>
                            <div className="flex flex-wrap gap-1">
                                <button className={editorButton} style={{ background: t.chosen }} disabled={busy} onClick={() => onMark('from')}>Set start here</button>
                                <button className={editorButton} style={{ background: t.chosen }} disabled={busy} onClick={() => onMark('to')}>Set end here</button>
                            </div>
                            <button className={editorButton + ' self-start'} style={{ color: t.sub }} disabled={busy || (edits.from === undefined && edits.to === undefined)}
                                onClick={() => onChange({ from: undefined, to: undefined })}>Clear trim</button>
                        </>
                    ) : tool === 'crop' ? (
                        <>
                            <div className="text-[12px] leading-[18px]" style={{ color: t.sub }}>Drag the edges to crop. Drag inside the frame to move it.</div>
                            <div className="flex flex-wrap gap-1.5" role="group" aria-label="Crop aspect ratio">
                                {[[16, 9], [1, 1], [9, 16]].map(([w, h]) => (
                                    <button key={w} className={editorButton} style={{ background: t.chosen }}
                                        disabled={busy || Math.min(w / h / aspect, aspect * h / w) < 0.1}
                                        onClick={() => onChange({ crop: fitCrop(w / h, aspect) })}>{w}:{h}</button>
                                ))}
                            </div>
                            <button className={editorButton + ' self-start'} style={{ color: t.sub }} disabled={busy || edits.crop === undefined}
                                onClick={() => onChange({ crop: undefined })}>Reset crop</button>
                        </>
                    ) : tool === 'keystrokes' ? (
                        <KeystrokeControls scheme={scheme} on={edits.showKeystrokes === true} onChange={onChange} busy={busy}
                            hasKeys={hasKeys} hasMatchingKeys={hasMatchingKeys} categories={categories} position={position} hold={hold}
                            presentation={presentation} canStepBack={canStepBack} canStepForward={canStepForward} onStepKey={onStepKey} />
                    ) : (
                        <>
                            <RecordingChoice label="Theme" value={edits.scheme ?? ''} options={RECORDED_THEMES} disabled={busy}
                                onChange={(scheme) => onChange({ scheme: scheme || undefined, fg: undefined, bg: undefined, cursor: undefined })} scheme={scheme} />
                            <RecordingChoice label="Background" value={edits.background ?? ''} options={RECORDED_BACKGROUNDS} disabled={busy}
                                onChange={(background) => onChange({ background: background || undefined })} scheme={scheme} />
                            <div className="flex flex-col gap-3 border-t pt-3" style={{ borderColor: t.border }}>
                                {(['fg', 'bg', 'cursor'] as const).map((key) => (
                                    <ColorEditor compact disabled={busy} key={key + colors[key]} label={key === 'fg' ? 'Text' : key === 'bg' ? 'Base color' : 'Cursor'}
                                        value={colors[key]} onChange={(value) => onChange({ [key]: value })} scheme={scheme} />
                                ))}
                            </div>
                            <button className={editorButton + ' self-start'} style={{ color: t.sub }}
                                disabled={busy || [edits.scheme, edits.background, edits.fg, edits.bg, edits.cursor].every((value) => value === undefined)}
                                onClick={() => onChange({ scheme: undefined, background: undefined, fg: undefined, bg: undefined, cursor: undefined })}>
                                Reset appearance
                            </button>
                        </>
                    )}
                </div>
                {onRevert ? (
                    <div className="mt-4 border-t pt-2" style={{ borderColor: t.border }}>
                        <button className={editorButton} style={{ color: t.sub }} disabled={busy} onClick={onRevert}>Revert unsaved edits</button>
                    </div>
                ) : null}
            </div>
        </aside>
    );
}

type KeystrokeControlsProps = {
    hasKeys: boolean;
    hasMatchingKeys: boolean;
    categories: readonly KeystrokeCategory[];
    position: KeystrokePosition;
    hold: KeystrokeDuration;
    presentation: { layout: KeystrokeLayout; limit: KeystrokeLimit; animation: KeystrokeAnimation };
    canStepBack: boolean;
    canStepForward: boolean;
    onStepKey: (direction: -1 | 1) => void;
};

function KeystrokeControls({ scheme, on, onChange, busy, hasKeys, hasMatchingKeys, categories, position, hold, presentation,
    canStepBack, canStepForward, onStepKey }: KeystrokeControlsProps & {
    scheme: Scheme;
    on: boolean;
    onChange: (patch: RecordingEdits) => void;
    busy: boolean;
}) {
    const t = tokens(scheme);
    return (
        hasKeys ? (
            <>
                <Field label="Show keystrokes" hint="Show key presses in the preview and MP4." scheme={scheme}>
                    <Switch scheme={scheme} on={on} disabled={busy}
                        onChange={(show) => onChange({ showKeystrokes: show || undefined })} />
                </Field>
                {on ? (
                    <>
                        <div className="flex flex-wrap gap-1">
                            <button className={editorButton} style={{ background: t.chosen }} disabled={busy || !canStepBack}
                                onClick={() => onStepKey(-1)}>Previous key</button>
                            <button className={editorButton} style={{ background: t.chosen }} disabled={busy || !canStepForward}
                                onClick={() => onStepKey(1)}>Next key</button>
                        </div>
                        <RecordingChoice label="When keys overlap" value={presentation.layout} options={KEY_LAYOUTS} scheme={scheme}
                            disabled={busy} onChange={(keystrokeLayout) => onChange({ keystrokeLayout })} />
                        {presentation.layout !== 'latest' ? (
                            <RecordingChoice label="Maximum visible presses" value={presentation.limit} options={KEY_LIMITS} scheme={scheme}
                                disabled={busy} onChange={(keystrokeLimit) => onChange({ keystrokeLimit })} />
                        ) : null}
                        <RecordingChoice label="Animation" value={presentation.animation} options={KEY_ANIMATIONS} scheme={scheme}
                            disabled={busy} onChange={(keystrokeAnimation) => onChange({ keystrokeAnimation })} />
                        <RecordingChoice label="Time on screen" value={hold} options={KEY_DURATIONS} scheme={scheme}
                            disabled={busy} onChange={(keystrokeDuration) => onChange({ keystrokeDuration })} />
                        <RecordingChoice label="Overlay position" value={position} options={KEY_POSITIONS} scheme={scheme}
                            disabled={busy} onChange={(keystrokePosition) => onChange({ keystrokePosition })} />
                        <div className="flex flex-col gap-1.5" role="group" aria-label="Keys to show">
                            <div className="flex flex-wrap items-center justify-between gap-1">
                                <span className="text-[12px] font-medium">Keys to show</span>
                                <button className={editorButton} style={{ background: t.chosen }}
                                    disabled={busy || categories.length === KEYSTROKE_CATEGORIES.length}
                                    onClick={() => onChange({ keystrokeCategories: undefined })}>Show all</button>
                            </div>
                            {KEYSTROKE_CATEGORIES.map((category) => {
                                const { label, hint } = KEY_LABELS[category];
                                return (
                                    <Field key={category} label={label} hint={hint} scheme={scheme}>
                                        <Switch scheme={scheme} on={categories.includes(category)} disabled={busy}
                                            onChange={(on) => {
                                                const next = KEYSTROKE_CATEGORIES.filter((key) => key === category ? on : categories.includes(key));
                                                onChange({ keystrokeCategories: next.length === KEYSTROKE_CATEGORIES.length ? undefined : next });
                                            }} />
                                    </Field>
                                );
                            })}
                        </div>
                        {!hasMatchingKeys ? <p className="text-[12px]" style={{ color: t.sub }}>No recorded keys match these filters.</p> : null}
                    </>
                ) : null}
            </>
        ) : (
            <p className="text-[12px] leading-[18px]" style={{ color: t.sub }}>
                No keystrokes in this recording.
            </p>
        )
    );
}

const KEY_LABELS = {
    shortcuts: { label: 'Shortcuts', hint: 'Ctrl+C, Shift+Tab' },
    special: { label: 'Special keys', hint: 'Enter, Esc, arrows, F1–F12' },
    typed: { label: 'Typed characters', hint: 'Letters, numbers, and symbols if captured' },
} satisfies Record<KeystrokeCategory, { label: string; hint: string }>;
const KEY_POSITIONS: { id: KeystrokePosition; name: string }[] = [
    { id: 'bottom-center', name: 'Bottom center' }, { id: 'bottom-left', name: 'Bottom left' },
    { id: 'bottom-right', name: 'Bottom right' }, { id: 'top-center', name: 'Top center' },
];
const KEY_DURATIONS: { id: KeystrokeDuration; name: string }[] = [
    { id: '500', name: '0.5 seconds' }, { id: '1000', name: '1 second' }, { id: '2000', name: '2 seconds' },
];
const KEY_LAYOUTS: { id: KeystrokeLayout; name: string }[] = [
    { id: 'latest', name: 'Replace with latest' }, { id: 'row', name: 'Keep in a row' }, { id: 'stack', name: 'Stack vertically' },
];
const KEY_LIMITS: { id: KeystrokeLimit; name: string }[] = [
    { id: '3', name: '3 presses' }, { id: '6', name: '6 presses' }, { id: '12', name: '12 presses' },
];
const KEY_ANIMATIONS: { id: KeystrokeAnimation; name: string }[] = [
    { id: 'none', name: 'None' }, { id: 'fade', name: 'Fade' }, { id: 'pop', name: 'Pop in' },
];
const AS_RECORDED = { id: '' as const, name: 'As recorded' };
const RECORDED_THEMES = [AS_RECORDED, ...SCHEMES.map(({ id, name }) => ({ id, name }))];
const RECORDED_BACKGROUNDS = [AS_RECORDED, ...BACKGROUNDS];

function RecordingChoice<T extends string>({ label, value, options, onChange, scheme, disabled }: {
    label: string;
    value: T;
    options: { id: T; name: string }[];
    onChange: (value: T) => void;
    scheme: Scheme;
    disabled: boolean;
}) {
    const t = tokens(scheme);
    return (
        <div className="flex flex-col gap-1 text-[12px]">
            <span style={{ color: t.sub }}>{label}</span>
            <select aria-label={label} disabled={disabled} value={value}
                className="h-8 w-full rounded-md px-2 text-[12px] focus-visible:outline-2"
                style={{ background: t.chosen, color: t.text }}
                onChange={(event) => {
                    const selected = options.find(({ id }) => id === event.currentTarget.value);
                    if (selected) onChange(selected.id);
                }}>
                {options.map(({ id, name }) => <option key={id} value={id}>{name}</option>)}
            </select>
        </div>
    );
}
