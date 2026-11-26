// Settings, as a tab: the sections in a list, and the chosen one's settings in
// groups, each row a label with what it does under it and its control at the right. Everything takes effect as it's touched.

import { useState } from 'react';
import { dialogs } from 'rsui/desktop';
import type { Context } from '../commands';
import { FONTS } from '../settings';
import type { Layout, Settings } from '../settings';
import { compileTrigger } from '../term/triggers';
import type { TriggerRule } from '../term/triggers';
import { SCHEMES, SCHEME_COLORS, css, mix, resolveScheme, withAlpha, tokens } from '../themes';
import type { Scheme, SchemeColor } from '../themes';
import { BACKGROUNDS } from './Background';
import { ColorEditor } from './ColorEditor';
import { Icon } from './Icon';
import type { IconName } from './Icon';
import { PromptPage } from './PromptPage';
import { Field, Segmented, SettingsCard as Group, Slider, Switch, keyLabel, useFieldLabel } from './controls';
import type { FieldProps } from './controls';
import { KeymapPage } from './KeymapPage';
import { SettingsTransferPage } from './SettingsTransferPage';

export type PageId = 'general' | 'appearance' | 'background' | 'text' | 'prompt' | 'behavior' | 'strip' | 'shells' | 'keyboard' | 'transfer';

export const SETTINGS_PAGES: { id: PageId; label: string; icon: IconName; blurb: string }[] = [
    { id: 'general', label: 'General', icon: 'gear', blurb: 'How the window is laid out, and what it does when it starts.' },
    { id: 'appearance', label: 'Appearance', icon: 'palette', blurb: 'Colors, the window’s material and its shape.' },
    { id: 'background', label: 'Background', icon: 'image', blurb: 'The background behind terminals and tool pages.' },
    { id: 'text', label: 'Text & cursor', icon: 'terminal', blurb: 'The terminal’s font and how its cursor looks.' },
    { id: 'prompt', label: 'Prompt', icon: 'command', blurb: 'Choose what PowerShell shows before each command.' },
    { id: 'behavior', label: 'Behavior', icon: 'check', blurb: 'Selecting, pasting, links, commands and the bell.' },
    { id: 'strip', label: 'Strip', icon: 'width', blurb: 'The strip of columns, and the dropdown terminal.' },
    { id: 'shells', label: 'Shells', icon: 'command', blurb: 'What a new terminal runs, and what opens files.' },
    { id: 'keyboard', label: 'Keyboard', icon: 'hash', blurb: 'Click a shortcut to change it, or add another to a command.' },
    { id: 'transfer', label: 'Import & export', icon: 'folder', blurb: 'Save settings to a JSON file or apply settings from one.' },
];

interface PageProps {
    settings: Settings;
    scheme: Scheme;
    context: Context;
    change: (patch: Partial<Settings>) => void;
}

/** The sections of settings, as a list to pick from: in the window's sidebar, or at the side of the settings tab. */
export function SettingsNav(props: { page: PageId; scheme: Scheme; onPage: (page: PageId) => void }) {
    const t = tokens(props.scheme);
    return (
        <nav className="flex flex-col gap-px" style={{ '--settings-nav-hover': t.hover }}>
            {SETTINGS_PAGES.map((p) => {
                const on = p.id === props.page;
                return (
                    <button
                        key={p.id}
                        aria-label={p.label}
                        aria-current={on ? 'page' : undefined}
                        className="flex h-[30px] items-center gap-2.5 rounded-md pl-2 text-left transition-colors duration-150 hover:bg-[var(--settings-nav-hover)]"
                        style={{ background: on ? t.chosen : undefined, color: on ? t.text : withAlpha(props.scheme.fg, 0.72) }}
                        onClick={() => props.onPage(p.id)}
                    >
                        <span style={{ color: on ? t.text : t.sub }}>
                            <Icon name={p.icon} size={15} />
                        </span>
                        <span className="text-[13px]">{p.label}</span>
                    </button>
                );
            })}
        </nav>
    );
}

/** One section of settings, scrolling, with its title and what it's for. */
export function SettingsBody(props: PageProps & { page: PageId }) {
    const { scheme, page } = props;
    const t = tokens(scheme);
    const current = SETTINGS_PAGES.find((p) => p.id === page) ?? SETTINGS_PAGES[0];
    const solid = props.settings.material === 'none';
    return (
        <div className="size-full overflow-y-auto" style={{
            '--settings-card': withAlpha(mix(scheme.bg, scheme.fg, 0.045), solid ? 1 : 0.72),
            '--settings-card-selected': withAlpha(mix(scheme.bg, scheme.fg, 0.11), solid ? 1 : 0.8),
            '--settings-card-blur': solid ? 'none' : 'blur(12px)',
        }}>
            <div className="mx-auto max-w-[680px] px-8 pb-16 pt-9">
                <div className="text-[22px] font-semibold tracking-tight">{current.label}</div>
                <div className="mb-7 mt-1 text-[13px]" style={{ color: t.sub }}>
                    {current.blurb}
                </div>
                {page === 'general' ? <General {...props} /> : null}
                {page === 'appearance' ? <Appearance {...props} /> : null}
                {page === 'background' ? <BackgroundPage {...props} /> : null}
                {page === 'text' ? <TextPage {...props} /> : null}
                {page === 'prompt' ? <PromptPage {...props} /> : null}
                {page === 'behavior' ? <Behavior {...props} /> : null}
                {page === 'strip' ? <StripPage {...props} /> : null}
                {page === 'shells' ? <Shells {...props} /> : null}
                {page === 'keyboard' ? <KeymapPage context={props.context} scheme={scheme} /> : null}
                {page === 'transfer' ? <SettingsTransferPage context={props.context} scheme={scheme} /> : null}
            </div>
        </div>
    );
}

// ---- Groups and rows

function Row(props: Omit<FieldProps, 'density'>) {
    return <Field {...props} density="card" />;
}

/** A card to pick, with what it is under it. */
function Choice(props: { on: boolean; scheme: Scheme; onClick: () => void; children: any }) {
    const { scheme } = props;
    return (
        <button
            className="settings-card flex flex-col gap-2 rounded-xl p-2.5 text-left transition-colors duration-150"
            data-selected={props.on ? '' : undefined}
            style={{
                boxShadow: `inset 0 0 0 ${props.on ? 1.5 : 1}px ${withAlpha(scheme.fg, props.on ? 0.4 : 0.05)}`,
            }}
            onClick={props.onClick}
        >
            {props.children}
        </button>
    );
}

function TextInput(props: { value: string; placeholder?: string; scheme: Scheme; onChange: (value: string) => void }) {
    const accessibility = useFieldLabel();
    return (
        <input
            {...accessibility}
            value={props.value}
            placeholder={props.placeholder}
            className="h-8 w-[240px] max-w-full rounded-lg px-3 text-[12.5px] focus-visible:outline-2"
            style={{ background: withAlpha(props.scheme.fg, 0.07), color: css(props.scheme.fg), fontFamily: 'Cascadia Mono, Consolas, monospace' }}
            onChange={(e: any) => props.onChange(typeof e === 'string' ? e : e.target.value)}
        />
    );
}

const percent = (v: number) => `${Math.round(v * 100)}%`;
const px = (v: number) => `${v} px`;

// ---- Pages

const LAYOUTS: { id: Layout; name: string; hint: string }[] = [
    { id: 'normal', name: 'Normal', hint: 'One at a time' },
    { id: 'tabs', name: 'Tabs', hint: 'Along the top' },
    { id: 'sidebar', name: 'Sidebar', hint: 'Listed on the left' },
    { id: 'strip', name: 'Strip', hint: 'Columns that slide' },
    { id: 'minimal', name: 'Minimal', hint: 'Just the text' },
];

function General({ settings, scheme, change }: PageProps) {
    return (
        <>
            <div className="mb-2 px-1 text-[12.5px] font-medium" style={{ color: withAlpha(scheme.fg, 0.45) }}>
                Layout
            </div>
            <div className="mb-7 grid grid-cols-5 gap-2">
                {LAYOUTS.map((layout) => (
                    <Choice key={layout.id} on={layout.id === settings.layout} scheme={scheme} onClick={() => change({ layout: layout.id })}>
                        <LayoutThumb id={layout.id} scheme={scheme} />
                        <div className="px-0.5">
                            <div className="text-[12.5px] font-medium">{layout.name}</div>
                            <div className="text-[11px]" style={{ color: withAlpha(scheme.fg, 0.48) }}>
                                {layout.hint}
                            </div>
                        </div>
                    </Choice>
                ))}
            </div>
            <Group title="Window" scheme={scheme}>
                <Row label="Interface font" hint="The terminal’s font everywhere makes it all one piece." scheme={scheme}>
                    <Segmented scheme={scheme} value={settings.uiFont} options={[{ value: 'system', label: 'System' }, { value: 'mono', label: 'Mono' }]} onChange={(uiFont) => change({ uiFont })} />
                </Row>
                {settings.layout === 'sidebar' ? (
                    <Row label="Sidebar width" scheme={scheme}>
                        <Slider scheme={scheme} value={settings.sidebarWidth} min={180} max={360} step={4} onChange={(sidebarWidth) => change({ sidebarWidth })} format={px} />
                    </Row>
                ) : null}
                <Row label="Animations" hint="Sliding between columns, the overview, the cursor and the background." scheme={scheme}>
                    <Switch scheme={scheme} on={settings.animations} onChange={(animations) => change({ animations })} />
                </Row>
            </Group>
            <Group title="Starting" scheme={scheme}>
                <Row label="Reopen where I left off" hint="The terminals, their shells and folders come back next time." scheme={scheme}>
                    <Switch scheme={scheme} on={settings.restoreSession} onChange={(restoreSession) => change({ restoreSession })} />
                </Row>
                {settings.restoreSession ? <Row label="Restore terminal output" hint="Save recent output as local plain text. Shells start fresh." scheme={scheme}>
                    <Segmented scheme={scheme} value={String(settings.restoreLines)}
                        options={[{ value: '0', label: 'Off' }, { value: '250', label: '250 lines' }, { value: '1000', label: '1,000 lines' }]}
                        onChange={(value) => change({ restoreLines: Number(value) })} />
                </Row> : null}
            </Group>
        </>
    );
}

function Appearance({ settings, scheme, change }: PageProps) {
    const solid = settings.material === 'none';
    return (
        <>
            <div className="mb-2 px-1 text-[12.5px] font-medium" style={{ color: withAlpha(scheme.fg, 0.45) }}>
                Color scheme
            </div>
            <div className="mb-7 grid grid-cols-4 gap-2">
                {SCHEMES.map(({ id }) => resolveScheme(id, settings.schemeOverrides[id])).map((s) => (
                    <button
                        key={s.id}
                        aria-pressed={s.id === settings.scheme}
                        className="overflow-hidden rounded-xl text-left transition-transform duration-150"
                        style={{ border: `2px solid ${s.id === settings.scheme ? withAlpha(scheme.fg, 0.7) : 'transparent'}` }}
                        onClick={() => change({ scheme: s.id })}
                    >
                        <div className="flex h-[54px] flex-col justify-center gap-1.5 px-3" style={{ background: css(s.bg) }}>
                            <div className="h-[3px] w-12 rounded-full" style={{ background: withAlpha(s.fg, 0.7) }} />
                            <div className="flex gap-1">
                                {[1, 2, 3, 4, 5, 6].map((k) => (
                                    <span key={k} className="h-[3px] w-3 rounded-full" style={{ background: css(s.ansi[k]) }} />
                                ))}
                            </div>
                        </div>
                        <div className="truncate px-3 py-2 text-[12px] font-medium" style={{ background: css(s.frame), color: css(s.fg) }}>
                            {s.name}{settings.schemeOverrides[s.id] ? ' *' : ''}
                        </div>
                    </button>
                ))}
            </div>
            <SchemeEditor key={settings.scheme} settings={settings} scheme={scheme} change={change} />
            <Group title="Window" scheme={scheme}>
                <Row label="Material" hint="Sets the window backdrop and the surface of Settings cards." scheme={scheme}>
                    <Segmented
                        scheme={scheme}
                        value={settings.material}
                        options={[{ value: 'none', label: 'Solid' }, { value: 'mica', label: 'Mica' }, { value: 'acrylic', label: 'Acrylic' }, { value: 'tabbed', label: 'Tabbed' }]}
                        onChange={(material) => change({ material, frameOpacity: material === 'none' ? 1 : Math.min(settings.frameOpacity, 0.55) })}
                    />
                </Row>
                {solid ? null : (
                    <Row label="Frame tint" hint="How much of the scheme’s color lies over the material." scheme={scheme}>
                        <Slider scheme={scheme} value={settings.frameOpacity} min={0} max={1} step={0.05} onChange={(frameOpacity) => change({ frameOpacity })} format={percent} />
                    </Row>
                )}
                <Row label="Panel opacity" hint="Lower lets the window backdrop show behind the text." scheme={scheme}>
                    <Slider scheme={scheme} value={settings.columnOpacity} min={0.3} max={1} step={0.02} onChange={(columnOpacity) => change({ columnOpacity })} format={percent} />
                </Row>
                <Row label="Corner radius" scheme={scheme}>
                    <Slider scheme={scheme} value={settings.radius} min={0} max={24} step={1} onChange={(radius) => change({ radius })} format={px} />
                </Row>
                <Row label="Room around the text" scheme={scheme}>
                    <Slider scheme={scheme} value={settings.textPadding} min={0} max={40} step={2} onChange={(textPadding) => change({ textPadding })} format={px} />
                </Row>
            </Group>
        </>
    );
}

const ANSI_NAMES = ['Black', 'Red', 'Green', 'Yellow', 'Blue', 'Magenta', 'Cyan', 'White'];

function SchemeEditor({ settings, scheme, change }: Pick<PageProps, 'settings' | 'scheme' | 'change'>) {
    const [open, setOpen] = useState(false);
    const [selected, select] = useState<SchemeColor | number>('bg');
    const overrides = settings.schemeOverrides[scheme.id];
    const ansiName = (i: number) => (i >= 8 ? 'Bright ' : '') + ANSI_NAMES[i % 8];
    const commit = (value: number) => {
        const next = typeof selected === 'number'
            ? { ...overrides, ansi: { ...overrides?.ansi, [selected]: value } }
            : { ...overrides, [selected]: value };
        change({ schemeOverrides: { ...settings.schemeOverrides, [scheme.id]: next } });
    };
    const reset = () => {
        const next = { ...settings.schemeOverrides };
        delete next[scheme.id];
        change({ schemeOverrides: next });
    };
    const buttons = (keys: (SchemeColor | number)[]) => keys.map((key) => {
        const label = typeof key === 'number' ? ansiName(key) : SCHEME_COLORS[key];
        const value = typeof key === 'number' ? scheme.ansi[key] : scheme[key];
        return (
            <button key={key} aria-label={label} aria-pressed={selected === key} title={label}
                className="flex items-center gap-1.5 rounded-md px-2 py-1 text-[11.5px]"
                style={{ background: withAlpha(scheme.fg, selected === key ? 0.16 : 0.06) }}
                onClick={() => select(key)}>
                <span className="size-3.5 shrink-0 rounded-sm" style={{ background: css(value), border: `1px solid ${withAlpha(scheme.fg, 0.25)}` }} />
                {label}
            </button>
        );
    });
    const label = typeof selected === 'number' ? ansiName(selected) : SCHEME_COLORS[selected];
    const value = typeof selected === 'number' ? scheme.ansi[selected] : scheme[selected];
    const button = { background: withAlpha(scheme.fg, 0.09) };
    return (
        <div className="mb-7">
            <button className="flex items-center gap-2 rounded-md px-3 py-1.5 text-[12px]" style={button}
                aria-expanded={open} onClick={() => setOpen(!open)}>
                <Icon name={open ? 'up' : 'down'} size={12} />
                Customize colors
            </button>
            {open ? (
                <div className="settings-card mt-3 flex flex-col gap-3 rounded-lg p-4">
                    <div className="flex flex-wrap gap-1">{buttons(Object.keys(SCHEME_COLORS) as SchemeColor[])}</div>
                    <div className="flex flex-wrap gap-1">{buttons(scheme.ansi.map((_, i) => i))}</div>
                    <div className="flex flex-wrap items-center justify-between gap-3">
                        <ColorEditor key={`${selected}:${value}`} label={label} value={value} scheme={scheme} onChange={commit} />
                        {overrides ? <button className="rounded-md px-3 py-1.5 text-[12px]" style={button} onClick={reset}>Reset scheme colors</button> : null}
                    </div>
                </div>
            ) : null}
        </div>
    );
}

/** What a studio design is drawn from, by name. */
function studioName(studio: NonNullable<Settings['studio']>): string {
    if (studio.source && studio.source !== 'file') return (BACKGROUNDS.find((b) => b.id === studio.source)?.name ?? 'Background') + ', remixed';
    return studio.file.split(/[\\/]/).pop() ?? '';
}

function BackgroundPage({ settings, scheme, change, context }: PageProps) {
    return (
        <>
            <button
                className="settings-card mb-5 flex w-full items-center gap-4 rounded-xl p-4 text-left transition-colors duration-150"
                data-selected={settings.background === 'custom' ? '' : undefined}
                style={{ boxShadow: `inset 0 0 0 ${settings.background === 'custom' ? 1.5 : 1}px ${withAlpha(scheme.fg, settings.background === 'custom' ? 0.4 : 0.05)}` }}
                onClick={() => context.openTool('studio')}
            >
                <span className="flex size-10 shrink-0 items-center justify-center rounded-xl" style={{ background: withAlpha(scheme.fg, 0.08) }}>
                    <Icon name="studio" size={18} />
                </span>
                <span className="min-w-0 flex-1">
                    <span className="block text-[13.5px] font-medium">{settings.studio ? 'Your own' : 'Make your own'}</span>
                    <span className="block truncate text-[12px]" style={{ color: withAlpha(scheme.fg, 0.5) }}>
                        {settings.studio ? studioName(settings.studio) + ' · open the studio' : 'A picture, a video or a background, as ASCII, braille, halftone and more'}
                    </span>
                </span>
                {settings.studio && settings.background !== 'custom' ? (
                    <span
                        className="rounded-lg px-3 py-1.5 text-[12px]"
                        style={{ background: withAlpha(scheme.fg, 0.1) }}
                        onClick={(e: any) => {
                            e.stopPropagation();
                            change({ background: 'custom' });
                        }}
                    >
                        Use
                    </span>
                ) : null}
            </button>
            <div className="mb-7 grid grid-cols-4 gap-2">
                {BACKGROUNDS.map((b) => (
                    <Choice key={b.id} on={b.id === settings.background} scheme={scheme} onClick={() => change({ background: b.id })}>
                        <div className="px-1 py-0.5">
                            <div className="text-[13px] font-medium">{b.name}</div>
                            <div className="mt-0.5 text-[11.5px] leading-[15px]" style={{ color: withAlpha(scheme.fg, 0.48) }}>
                                {b.hint}
                            </div>
                        </div>
                    </Choice>
                ))}
            </div>
            {settings.background === 'none' ? null : (
                <Group title="How it looks" scheme={scheme}>
                    <Row label="Colors" scheme={scheme}>
                        <Segmented
                            scheme={scheme}
                            value={settings.backgroundColor}
                            options={[
                                { value: 'scheme', label: 'Scheme' }, { value: 'blue', label: 'Blue' }, { value: 'purple', label: 'Violet' },
                                { value: 'green', label: 'Green' }, { value: 'warm', label: 'Warm' }, { value: 'mono', label: 'Mono' },
                            ]}
                            onChange={(backgroundColor) => change({ backgroundColor })}
                        />
                    </Row>
                    <Row label="Strength" scheme={scheme}>
                        <Slider scheme={scheme} value={settings.backgroundIntensity} min={0.1} max={1} step={0.05} onChange={(backgroundIntensity) => change({ backgroundIntensity })} format={percent} />
                    </Row>
                    <Row label="Speed" scheme={scheme}>
                        <Slider scheme={scheme} value={settings.backgroundSpeed} min={0.1} max={3} step={0.1} onChange={(v) => change({ backgroundSpeed: Math.round(v * 10) / 10 })} format={(v) => `${v.toFixed(1)}×`} />
                    </Row>
                    <Row label="Amount" scheme={scheme}>
                        <Slider scheme={scheme} value={settings.backgroundDensity} min={0.3} max={2.5} step={0.1} onChange={(v) => change({ backgroundDensity: Math.round(v * 10) / 10 })} format={(v) => `${v.toFixed(1)}×`} />
                    </Row>
                </Group>
            )}
        </>
    );
}

function TextPage({ settings, scheme, change }: PageProps) {
    return (
        <>
            <Group title="Font" scheme={scheme}>
                <Row label="Family" scheme={scheme}>
                    <div className="settings-fonts flex max-w-[330px] flex-wrap justify-end gap-1">
                        {FONTS.map((font) => (
                            <button type="button" aria-pressed={settings.fontFamily === font}
                                key={font}
                                className="h-7 rounded-md px-2.5 text-[12px]"
                                style={{ background: settings.fontFamily === font ? withAlpha(scheme.fg, 0.16) : withAlpha(scheme.fg, 0.06), fontFamily: font, color: css(scheme.fg) }}
                                onClick={() => change({ fontFamily: font, ligatures: font === 'Cascadia Code' })}
                            >
                                {font}
                            </button>
                        ))}
                    </div>
                </Row>
                <Row label="Custom font" hint="Choose an installed monospaced family." scheme={scheme}>
                    <button aria-label="Choose font" className="h-8 rounded-md px-2.5 text-[12px]"
                        style={{ background: withAlpha(scheme.fg, 0.09) }}
                        onClick={async () => {
                            const picked = await dialogs.pickFont({ initial: { family: settings.fontFamily, size: settings.fontSize * 0.75 } });
                            if (picked) change({ fontFamily: picked.family, ligatures: picked.family === 'Cascadia Code' });
                        }}>
                        {FONTS.includes(settings.fontFamily) ? 'Choose font…' : settings.fontFamily}
                    </button>
                </Row>
                <Row label="Size" scheme={scheme}>
                    <Slider scheme={scheme} value={settings.fontSize} min={7} max={40} step={0.5} onChange={(fontSize) => change({ fontSize })} format={px} />
                </Row>
                <Row label="Line height" hint="1.0 joins lines of text closely; more gives them air." scheme={scheme}>
                    <Slider scheme={scheme} value={settings.lineHeight} min={1} max={1.6} step={0.05} onChange={(v) => change({ lineHeight: Math.round(v * 100) / 100 })} format={(v) => v.toFixed(2)} />
                </Row>
                <Row label="Ligatures" hint="Cascadia Code joins => and != into one sign; Cascadia Mono doesn’t." scheme={scheme}>
                    <Switch
                        scheme={scheme}
                        on={settings.ligatures}
                        onChange={(ligatures) =>
                            change({
                                ligatures,
                                fontFamily: ligatures && settings.fontFamily === 'Cascadia Mono' ? 'Cascadia Code' : !ligatures && settings.fontFamily === 'Cascadia Code' ? 'Cascadia Mono' : settings.fontFamily,
                            })
                        }
                    />
                </Row>
            </Group>
            <Group title="Cursor" scheme={scheme}>
                <Row label="Shape" hint="As asked follows what the program wants." scheme={scheme}>
                    <Segmented
                        scheme={scheme}
                        value={settings.cursorShape}
                        options={[{ value: 'program', label: 'As asked' }, { value: 'block', label: 'Block' }, { value: 'bar', label: 'Bar' }, { value: 'underline', label: 'Line' }]}
                        onChange={(cursorShape) => change({ cursorShape })}
                    />
                </Row>
                <Row label="Blink" scheme={scheme}>
                    <Switch scheme={scheme} on={settings.cursorBlink} onChange={(cursorBlink) => change({ cursorBlink })} />
                </Row>
                <Row label="Glide" hint="The cursor slides to where it goes instead of jumping." scheme={scheme}>
                    <Switch scheme={scheme} on={settings.smoothCursor} onChange={(smoothCursor) => change({ smoothCursor })} />
                </Row>
                <Row label="Trail" hint="As it glides, it leaves a short fading trail." scheme={scheme}>
                    <Switch scheme={scheme} on={settings.cursorTrail} onChange={(cursorTrail) => change({ cursorTrail })} />
                </Row>
            </Group>
        </>
    );
}

function Behavior({ settings, scheme, change }: PageProps) {
    return (
        <>
            <Group title="Text" scheme={scheme}>
                <Row label="Scrollback" hint="How many lines are kept to scroll back to." scheme={scheme}>
                    <Segmented
                        scheme={scheme}
                        value={String(settings.scrollback)}
                        options={[{ value: '1000', label: '1,000' }, { value: '10000', label: '10,000' }, { value: '100000', label: '100,000' }]}
                        onChange={(v) => change({ scrollback: Number(v) })}
                    />
                </Row>
                <Row label="Copy on select" scheme={scheme}>
                    <Switch scheme={scheme} on={settings.copyOnSelect} onChange={(copyOnSelect) => change({ copyOnSelect })} />
                </Row>
                <Row label="Ask before pasting several lines" hint="Each line break would run what’s before it." scheme={scheme}>
                    <Switch scheme={scheme} on={settings.warnOnMultilinePaste} onChange={(warnOnMultilinePaste) => change({ warnOnMultilinePaste })} />
                </Row>
                {settings.warnOnMultilinePaste ? <Row label="Limit paste warnings to shell prompts" hint="Shells without prompt marks still ask." scheme={scheme}>
                    <Switch scheme={scheme} on={settings.pasteAtPromptOnly} onChange={(pasteAtPromptOnly) => change({ pasteAtPromptOnly })} />
                </Row> : null}
                <Row label="Link click gesture" scheme={scheme}>
                    <Segmented
                        scheme={scheme}
                        value={settings.clickLinks}
                        options={[{ value: 'ctrl', label: 'Ctrl+click' }, { value: 'click', label: 'Click' }, { value: 'off', label: 'Never' }]}
                        onChange={(clickLinks) => change({ clickLinks })}
                    />
                </Row>
                <Row label="Open web links in" scheme={scheme}>
                    <Segmented scheme={scheme} value={settings.linkBrowser}
                        options={[{ value: 'system', label: 'Default browser' }, { value: 'builtIn', label: 'Built-in browser' }]}
                        onChange={(linkBrowser) => change({ linkBrowser })} />
                </Row>
                <Row label="Let programs copy" hint="A program can put text on the clipboard." scheme={scheme}>
                    <Switch scheme={scheme} on={settings.allowClipboardWrite} onChange={(allowClipboardWrite) => change({ allowClipboardWrite })} />
                </Row>
            </Group>
            <Group title="Commands" scheme={scheme}>
                <Row label="Mark each command" hint="A soft band on the line it was typed on, red when it failed." scheme={scheme}>
                    <Switch scheme={scheme} on={settings.showMarks} onChange={(showMarks) => change({ showMarks })} />
                </Row>
                <Row label="Keep the command at the top" hint="While you’re scrolled up in its output." scheme={scheme}>
                    <Switch scheme={scheme} on={settings.stickyCommand} onChange={(stickyCommand) => change({ stickyCommand })} />
                </Row>
                <Row label="Commands on the scrollbar" scheme={scheme}>
                    <Switch scheme={scheme} on={settings.scrollMinimap} onChange={(scrollMinimap) => change({ scrollMinimap })} />
                </Row>
                <Row label="Tell me when one finishes" hint="After this long, if you’re looking at something else." scheme={scheme}>
                    <Slider scheme={scheme} value={settings.notifyAfter} min={0} max={60} step={5} onChange={(notifyAfter) => change({ notifyAfter })} format={(v) => (v === 0 ? 'Never' : `${v} s`)} />
                </Row>
            </Group>
            <Group title="Terminal protocols" scheme={scheme}>
                <Row label="Inline graphics" scheme={scheme}>
                    <Segmented scheme={scheme} value={settings.graphics}
                        options={[{ value: 'all', label: 'All' }, { value: 'kitty', label: 'Kitty' }, { value: 'sixel', label: 'Sixel' }, { value: 'iterm', label: 'iTerm2' }, { value: 'off', label: 'Off' }]}
                        onChange={(graphics) => change({ graphics })} />
                </Row>
                <Row label="Kitty keyboard protocol" hint="Applications can request more precise key reports." scheme={scheme}>
                    <Switch scheme={scheme} on={settings.kittyKeyboard} onChange={(kittyKeyboard) => change({ kittyKeyboard })} />
                </Row>
                {settings.kittyKeyboard ? <Row label="Prefer terminal application shortcuts" hint="When an application enables Kitty keyboard mode, it gets matching keys first. The command palette stays available." scheme={scheme}>
                    <Switch scheme={scheme} on={settings.kittyKeysFirst} onChange={(kittyKeysFirst) => change({ kittyKeysFirst })} />
                </Row> : null}
            </Group>
            <Group title="Recordings and replay" scheme={scheme}>
                <Row label="Instant replay history" hint="Keep recent output in memory for each open pane." scheme={scheme}>
                    <Segmented scheme={scheme} value={String(settings.replaySeconds)}
                        options={[{ value: '0', label: 'Off' }, { value: '30', label: '30 s' }, { value: '60', label: '1 min' }, { value: '300', label: '5 min' }]}
                        onChange={(value) => change({ replaySeconds: Number(value) })} />
                </Row>
                <Row label="Default playback speed" scheme={scheme}>
                    <Segmented scheme={scheme} value={settings.replaySpeed}
                        options={[{ value: '1', label: '1×' }, { value: '2', label: '2×' }, { value: '4', label: '4×' }, { value: '8', label: '8×' }]}
                        onChange={(replaySpeed) => change({ replaySpeed })} />
                </Row>
                <Row label="Pane status bar" hint="Show the host, local Git branch, and command time." scheme={scheme}>
                    <Switch scheme={scheme} on={settings.paneStatus} onChange={(paneStatus) => change({ paneStatus })} />
                </Row>
            </Group>
            <Group title="Bell" scheme={scheme}>
                <Row label="When a program rings it" scheme={scheme}>
                    <Segmented
                        scheme={scheme}
                        value={settings.bell}
                        options={[{ value: 'off', label: 'Nothing' }, { value: 'flash', label: 'Flash' }, { value: 'sound', label: 'Sound' }, { value: 'both', label: 'Both' }]}
                        onChange={(bell) => change({ bell })}
                    />
                </Row>
                <Row label="Mark panes that need attention" scheme={scheme}>
                    <Segmented scheme={scheme} value={settings.attentionOn}
                        options={[{ value: 'off', label: 'Off' }, { value: 'notifications', label: 'Notifications' }, { value: 'notificationsAndBell', label: 'Notifications and bell' }]}
                        onChange={(attentionOn) => change({ attentionOn })} />
                </Row>
            </Group>
            <TriggerEditor rules={settings.triggers} scheme={scheme} setRules={(triggers) => change({ triggers })} />
        </>
    );
}

function TriggerEditor({ rules, scheme, setRules }: { rules: TriggerRule[]; scheme: Scheme; setRules: (rules: TriggerRule[]) => void }) {
    const changeRule = (index: number, patch: Partial<TriggerRule>) => setRules(rules.map((rule, i) => i === index ? { ...rule, ...patch } : rule));
    return (
        <Group title="Output triggers" scheme={scheme}>
            {rules.map((rule, index) => (
                <div key={index} className="flex flex-col gap-2 px-4 py-3">
                    <div className="flex items-center gap-2">
                        <div className="min-w-0 flex-1"><TextInput value={rule.pattern} placeholder="Regular expression" scheme={scheme} onChange={(pattern) => changeRule(index, { pattern: pattern.slice(0, 256) })} /></div>
                        <button aria-label="Remove trigger" className="rounded px-2 py-1 hover:bg-white/10" onClick={() => setRules(rules.filter((_, i) => i !== index))}>Remove</button>
                    </div>
                    {rule.pattern && !compileTrigger(rule) ? <div className="text-[11px]" style={{ color: css(scheme.ansi[1]) }}>Invalid regular expression</div> : null}
                    <div className="flex flex-wrap items-center gap-x-3 gap-y-2 text-[11px]">
                        <Segmented scheme={scheme} value={rule.action} options={[{ value: 'highlight', label: 'Highlight' }, { value: 'notify', label: 'Notify' }, { value: 'attention', label: 'Needs attention' }]}
                            onChange={(action) => changeRule(index, { action })} />
                        <button aria-pressed={rule.ignoreCase} className="rounded px-2 py-1" style={{ background: rule.ignoreCase ? withAlpha(scheme.fg, 0.13) : undefined }}
                            onClick={() => changeRule(index, { ignoreCase: !rule.ignoreCase })}>Ignore case</button>
                        <button aria-pressed={rule.partial} className="rounded px-2 py-1" style={{ background: rule.partial ? withAlpha(scheme.fg, 0.13) : undefined }}
                            onClick={() => changeRule(index, { partial: !rule.partial })}>Match as text arrives</button>
                    </div>
                </div>
            ))}
            <Row label="Match terminal output" hint="Use a regular expression, such as ^ERROR or Continue\\?." scheme={scheme}>
                <button disabled={rules.length >= 24} className="rounded px-3 py-1.5 hover:bg-white/10" style={{ background: withAlpha(scheme.fg, 0.07) }}
                    onClick={() => setRules([...rules, { pattern: '', action: 'highlight', ignoreCase: false, partial: false }])}>Add trigger</button>
            </Row>
        </Group>
    );
}

function StripPage({ settings, scheme, change }: PageProps) {
    return (
        <>
            <Group title="Columns" scheme={scheme}>
                <Row label="New columns are" scheme={scheme}>
                    <Segmented
                        scheme={scheme}
                        value={settings.columnWidth}
                        options={[{ value: 'third', label: '⅓' }, { value: 'half', label: '½' }, { value: 'twothirds', label: '⅔' }, { value: 'wide', label: 'Wide' }, { value: 'full', label: 'Full' }]}
                        onChange={(columnWidth) => change({ columnWidth })}
                    />
                </Row>
                <Row label="Neighbors showing" hint="How much of the next column shows beside a wide one." scheme={scheme}>
                    <Slider scheme={scheme} value={settings.peek} min={0} max={140} step={4} onChange={(peek) => change({ peek })} format={px} />
                </Row>
                <Row label="Dim the others" scheme={scheme}>
                    <Switch scheme={scheme} on={settings.dimOthers} onChange={(dimOthers) => change({ dimOthers })} />
                </Row>
            </Group>
            <Group title="Dropdown terminal" scheme={scheme}>
                <Row label="Slide down from the top" hint={`${keyLabel(settings.quakeKey)} shows and hides it, from any program.`} scheme={scheme}>
                    <Switch scheme={scheme} on={settings.quakeEnabled} onChange={(quakeEnabled) => change({ quakeEnabled })} />
                </Row>
                <Row label="Height" scheme={scheme}>
                    <Slider scheme={scheme} value={settings.quakeHeight} min={0.3} max={1} step={0.05} onChange={(quakeHeight) => change({ quakeHeight })} format={percent} />
                </Row>
            </Group>
        </>
    );
}

function Shells({ settings, scheme, change, context }: PageProps) {
    return (
        <>
            <Group title="Discovered profiles" scheme={scheme}>
                <Row label="SSH aliases" hint="Create profiles from Host entries in ~/.ssh/config." scheme={scheme}>
                    <Switch scheme={scheme} on={settings.discoverSsh} onChange={(discoverSsh) => { change({ discoverSsh }); context.refreshProfiles(); }} />
                </Row>
                <Row label="WSL distributions" scheme={scheme}>
                    <Switch scheme={scheme} on={settings.discoverWsl} onChange={(discoverWsl) => { change({ discoverWsl }); context.refreshProfiles(); }} />
                </Row>
                <Row label="Scan again" scheme={scheme}>
                    <button className="rounded px-3 py-1.5 hover:bg-white/10" onClick={context.refreshProfiles}>Refresh profiles</button>
                </Row>
            </Group>
            <Group title="New terminals open" scheme={scheme}>
                {context.profiles.map((profile) => (
                    <button key={profile.id} className="flex h-12 w-full items-center gap-3 px-4 text-left" onClick={() => change({ profile: profile.id })}>
                        <span style={{ color: withAlpha(scheme.fg, 0.6) }}>
                            <Icon name="terminal" size={16} strokeWidth={1.6} />
                        </span>
                        <span className="flex-1 text-[13.5px] font-medium">{profile.name}</span>
                        {profile.id === settings.profile ? (
                            <span style={{ color: css(scheme.fg) }}>
                                <Icon name="check" size={16} strokeWidth={2} />
                            </span>
                        ) : null}
                    </button>
                ))}
            </Group>
            <Group title="Yours" scheme={scheme}>
                <Row label="A command line" hint="Shows up above as Custom: `wsl.exe -d Ubuntu`, `ssh me@server`." scheme={scheme}>
                    <TextInput value={settings.customCommand} placeholder="wsl.exe -d Ubuntu" scheme={scheme} onChange={(customCommand) => change({ customCommand })} />
                </Row>
                <Row label="Open files with" hint="For a file and line you Ctrl+click: {file}, {line}, {col}." scheme={scheme}>
                    <TextInput value={settings.editor} scheme={scheme} onChange={(editor) => change({ editor })} />
                </Row>
            </Group>
        </>
    );
}

/** A small drawing of a layout, for picking one. */
function LayoutThumb(props: { id: Layout; scheme: Scheme }) {
    const { scheme } = props;
    const frame = css(mix(scheme.frame, scheme.fg, 0.04));
    const panel = css(mix(scheme.bg, scheme.fg, 0.08));
    const line = withAlpha(scheme.fg, 0.3);
    const text = (
        <div className="flex flex-col gap-[3px] p-1.5">
            <div className="h-[2px] w-3/5 rounded" style={{ background: line }} />
            <div className="h-[2px] w-2/5 rounded" style={{ background: line }} />
        </div>
    );
    const box = 'flex h-[50px] w-full overflow-hidden rounded-md';
    switch (props.id) {
        case 'normal':
            return (
                <div className={box + ' flex-col gap-1 p-1'} style={{ background: frame }}>
                    <div className="h-1 w-1/3 rounded" style={{ background: line }} />
                    <div className="flex-1 rounded-[3px]" style={{ background: panel }}>{text}</div>
                </div>
            );
        case 'tabs':
            return (
                <div className={box + ' flex-col px-1 pb-1 pt-1.5'} style={{ background: frame }}>
                    <div className="flex gap-1">
                        <div className="h-2 w-5 rounded-t-[3px]" style={{ background: panel }} />
                        <div className="mt-0.5 h-1 w-4 rounded" style={{ background: line }} />
                    </div>
                    <div className="flex-1 rounded-[3px] rounded-tl-none" style={{ background: panel }}>{text}</div>
                </div>
            );
        case 'sidebar':
            return (
                <div className={box + ' gap-1 p-1'} style={{ background: frame }}>
                    <div className="flex w-1/4 flex-col gap-1 pt-1">
                        <div className="h-1 rounded" style={{ background: line }} />
                        <div className="h-1 rounded" style={{ background: line }} />
                        <div className="h-1 w-2/3 rounded" style={{ background: line }} />
                    </div>
                    <div className="flex-1 rounded-[3px]" style={{ background: panel }}>{text}</div>
                </div>
            );
        case 'strip':
            return (
                <div className={box + ' items-stretch gap-1 py-1'} style={{ background: frame }}>
                    <div className="w-1.5 rounded-r-[3px]" style={{ background: panel, opacity: 0.6 }} />
                    <div className="flex-1 rounded-[3px]" style={{ background: panel }}>{text}</div>
                    <div className="w-1.5 rounded-l-[3px]" style={{ background: panel, opacity: 0.6 }} />
                </div>
            );
        default:
            return (
                <div className={box + ' p-0.5'} style={{ background: frame }}>
                    <div className="flex-1 rounded-[3px]" style={{ background: panel }}>{text}</div>
                </div>
            );
    }
}
