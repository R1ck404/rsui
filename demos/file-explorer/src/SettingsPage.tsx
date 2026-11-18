// The settings screen. Every setting in `SECTIONS` shows here, with its own control, and changes apply as they're made.

import { useEffect, useState } from 'react';
import { clipboard, dialogs } from 'rsui/desktop';
import { promises as fs } from 'fs';
import * as I from './icons';
import { keyOf, showKey } from './keys';
import { ACCENTS, COMMANDS, DEFAULT_KEYS, DEFAULTS, FONTS, SECTIONS, type Command, type Field, type Settings } from './settings';
import { nameOf } from './files';
import { Kbd, Segmented, Stepper, Toggle } from './ui';

const ICONS: Record<string, any> = {
    appearance: I.PaletteIcon,
    icons: I.ImageIcon,
    layout: I.LayoutIcon,
    columns: I.TableIcon,
    files: I.FolderIcon,
    behavior: I.SlidersIcon,
    search: I.SearchIcon,
    preview: I.PreviewIcon,
    performance: I.GaugeIcon,
    keyboard: I.KeyboardIcon,
    pinned: I.StarIcon,
    backup: I.HistoryIcon,
};

type Change = <K extends keyof Settings>(key: K, value: Settings[K]) => void;

function Control(props: { field: Field; settings: Settings; change: Change }) {
    const { field, settings, change } = props;
    const value = settings[field.key] as any;
    switch (field.kind) {
        case 'toggle':
            return <Toggle on={value} label={field.label} onChange={(on) => change(field.key, on as any)} />;
        case 'choice':
            return <Segmented value={value} options={field.options} onChange={(next) => change(field.key, next as any)} />;
        case 'number':
            return <Stepper value={value} min={field.min} max={field.max} step={field.step} unit={field.unit} onChange={(next) => change(field.key, next as any)} />;
        case 'color':
            return (
                <div className="flex flex-wrap items-center gap-1.5">
                    {ACCENTS.map(([color, name]) => (
                        <button
                            key={color}
                            title={name}
                            aria-label={name}
                            onClick={() => change('accent', color)}
                            className={'h-6 w-6 rounded-full ' + (value === color ? 'ring-2 ring-(--fx-text) ring-offset-2 ring-offset-(--fx-card)' : '')}
                            style={{ background: color }}
                        />
                    ))}
                    <button
                        className="ml-1 rounded-(--fx-radius) bg-(--fx-input) px-2.5 py-1 text-[0.92em] text-(--fx-muted) hover:text-(--fx-text)"
                        onClick={async () => {
                            const picked = await dialogs.pickColor({ initial: value });
                            if (picked) {
                                change('accent', picked);
                            }
                        }}
                    >
                        Custom…
                    </button>
                    <span className="font-mono text-[0.85em] text-(--fx-faint)">{value}</span>
                </div>
            );
        case 'font':
            return (
                <div className="flex flex-wrap items-center gap-1.5">
                    <Segmented value={FONTS.includes(value) ? value : ''} options={FONTS.map((font) => [font, font.replace(' Text', '')] as [string, string])} onChange={(next) => change('fontFamily', next)} />
                    <button
                        className="rounded-(--fx-radius) bg-(--fx-input) px-2.5 py-1 text-[0.92em] text-(--fx-muted) hover:text-(--fx-text)"
                        onClick={async () => {
                            const picked = await dialogs.pickFont({ initial: { family: value, size: settings.fontSize } });
                            if (picked) {
                                change('fontFamily', picked.family);
                            }
                        }}
                    >
                        {FONTS.includes(value) ? 'Other…' : value}
                    </button>
                </div>
            );
        case 'text':
            return (
                <div className="flex w-80 items-center gap-1.5">
                    <input
                        key={String(value)}
                        defaultValue={value}
                        placeholder={field.placeholder}
                        className="h-8 min-w-0 flex-1 rounded-(--fx-radius) border border-(--fx-line) bg-(--fx-input) px-2.5 text-(--fx-text) outline-none focus:border-(--fx-accent)"
                        onKeyDown={(event: any) => {
                            event.stopPropagation();
                            if (event.key === 'Enter') {
                                change(field.key, event.currentTarget.value as any);
                            }
                        }}
                        onBlur={(event: any) => change(field.key, event.currentTarget.value as any)}
                    />
                    {field.browse ? (
                        <button
                            className="h-8 rounded-(--fx-radius) bg-(--fx-input) px-2.5 text-[0.92em] text-(--fx-muted) hover:text-(--fx-text)"
                            onClick={async () => {
                                const picked = await dialogs.open({ folders: true, start: value || undefined });
                                if (picked) {
                                    change(field.key, picked as any);
                                }
                            }}
                        >
                            Browse…
                        </button>
                    ) : null}
                </div>
            );
    }
}

function FieldRow(props: { field: Field; settings: Settings; change: Change }) {
    const { field, settings, change } = props;
    const changed = settings[field.key] !== DEFAULTS[field.key];
    return (
        <div className="flex items-center gap-6 border-b border-(--fx-line) py-3 last:border-b-0">
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="flex items-center gap-2 text-(--fx-text)">
                    {field.label}
                    {changed ? (
                        <button
                            title="Back to the default"
                            className="text-[0.8em] text-(--fx-accent) hover:underline"
                            onClick={() => change(field.key, DEFAULTS[field.key] as any)}
                        >
                            Reset
                        </button>
                    ) : null}
                </span>
                {field.hint ? <span className="text-[0.88em] text-(--fx-faint)">{field.hint}</span> : null}
            </div>
            <div className="shrink-0">
                <Control field={field} settings={settings} change={change} />
            </div>
        </div>
    );
}

function Keyboard(props: { settings: Settings; change: Change }) {
    const { settings, change } = props;
    const [recording, setRecording] = useState<Command | null>(null);
    useEffect(() => {
        if (!recording) {
            return;
        }
        const heard = (event: any) => {
            event.preventDefault();
            event.stopPropagation();
            if (event.key === 'Escape') {
                setRecording(null);
                return;
            }
            const shortcut = keyOf(event);
            if (shortcut) {
                change('keys', { ...settings.keys, [recording]: shortcut });
                setRecording(null);
            }
        };
        document.addEventListener('keydown', heard, { capture: true });
        return () => document.removeEventListener('keydown', heard, { capture: true });
    }, [recording, settings.keys]);

    const uses = new Map<string, number>();
    for (const shortcut of Object.values(settings.keys)) {
        uses.set(shortcut, (uses.get(shortcut) ?? 0) + 1);
    }
    return (
        <div className="flex flex-col">
            {(Object.keys(COMMANDS) as Command[]).map((command) => {
                const shortcut = settings.keys[command];
                const clash = shortcut !== '' && (uses.get(shortcut) ?? 0) > 1;
                return (
                    <div key={command} className="flex items-center gap-4 border-b border-(--fx-line) py-2.5 last:border-b-0">
                        <span className="min-w-0 flex-1 text-(--fx-text)">{COMMANDS[command]}</span>
                        {clash ? <span className="text-[0.85em] text-red-500">Used twice</span> : null}
                        {recording === command ? (
                            <span className="animate-pulse text-[0.92em] text-(--fx-accent)">Press the keys… (Esc to cancel)</span>
                        ) : shortcut ? (
                            <Kbd>{showKey(shortcut)}</Kbd>
                        ) : (
                            <span className="text-[0.92em] text-(--fx-faint)">None</span>
                        )}
                        <button className="rounded-(--fx-radius) bg-(--fx-input) px-2.5 py-1 text-[0.9em] text-(--fx-muted) hover:text-(--fx-text)" onClick={() => setRecording(command)}>
                            Change
                        </button>
                        <button
                            className="rounded-(--fx-radius) px-2 py-1 text-[0.9em] text-(--fx-faint) hover:text-(--fx-text)"
                            onClick={() => change('keys', { ...settings.keys, [command]: shortcut === DEFAULT_KEYS[command] ? '' : DEFAULT_KEYS[command] })}
                        >
                            {shortcut === DEFAULT_KEYS[command] ? 'Clear' : 'Default'}
                        </button>
                    </div>
                );
            })}
        </div>
    );
}

function Pinned(props: { pinned: string[]; folder: string; onPinned: (pinned: string[]) => void }) {
    const { pinned, folder, onPinned } = props;
    const move = (from: number, to: number) => {
        if (to < 0 || to >= pinned.length) {
            return;
        }
        const next = [...pinned];
        next.splice(to, 0, next.splice(from, 1)[0]);
        onPinned(next);
    };
    return (
        <div className="flex flex-col gap-3">
            {pinned.length === 0 ? <span className="text-(--fx-faint)">Nothing pinned yet. Pin a folder with its menu or Ctrl+D.</span> : null}
            {pinned.map((path, at) => (
                <div key={path} className="flex items-center gap-3 rounded-(--fx-radius) bg-(--fx-input) px-3 py-2">
                    <span className="flex shrink-0 text-(--fx-accent)"><I.StarIcon size={16} /></span>
                    <div className="flex min-w-0 flex-1 flex-col">
                        <span className="truncate text-(--fx-text)">{nameOf(path)}</span>
                        <span className="truncate text-[0.85em] text-(--fx-faint)">{path}</span>
                    </div>
                    <button className="px-1.5 text-(--fx-muted) hover:text-(--fx-text)" onClick={() => move(at, at - 1)}>↑</button>
                    <button className="px-1.5 text-(--fx-muted) hover:text-(--fx-text)" onClick={() => move(at, at + 1)}>↓</button>
                    <button className="text-[0.9em] text-(--fx-muted) hover:text-red-500" onClick={() => onPinned(pinned.filter((one) => one !== path))}>
                        Remove
                    </button>
                </div>
            ))}
            {folder && !pinned.includes(folder) ? (
                <button
                    className="self-start rounded-(--fx-radius) bg-(--fx-accent) px-3 py-1.5 text-(--fx-accent-text)"
                    onClick={() => onPinned([...pinned, folder])}
                >
                    Pin {nameOf(folder)}
                </button>
            ) : null}
        </div>
    );
}

function Backup(props: { settings: Settings; onReplace: (settings: Settings) => void }) {
    const { settings, onReplace } = props;
    const [said, setSaid] = useState('');
    const button = 'self-start rounded-(--fx-radius) bg-(--fx-input) px-3 py-1.5 text-(--fx-text) hover:bg-(--fx-hover)';
    const read = (text: string) => {
        const parsed = JSON.parse(text);
        onReplace({ ...DEFAULTS, ...parsed, keys: { ...DEFAULT_KEYS, ...(parsed.keys ?? {}) } });
    };
    return (
        <div className="flex flex-col gap-3">
            <p className="text-(--fx-muted)">Settings are saved as you change them. Take them to another computer, or start over.</p>
            <div className="flex flex-wrap gap-2">
                <button
                    className={button}
                    onClick={async () => {
                        const where = await dialogs.save({ title: 'Save settings', start: 'file-explorer-settings.json', filters: [{ name: 'JSON', extensions: ['json'] }] });
                        if (where) {
                            await fs.writeFile(where, JSON.stringify(settings, null, 2));
                            setSaid('Saved to ' + where);
                        }
                    }}
                >
                    Export to a file…
                </button>
                <button
                    className={button}
                    onClick={async () => {
                        const where = await dialogs.open({ title: 'Load settings', filters: [{ name: 'JSON', extensions: ['json'] }] });
                        if (where) {
                            try {
                                read(await fs.readFile(where, 'utf8'));
                                setSaid('Loaded ' + where);
                            } catch (error) {
                                setSaid('That file isn\'t settings: ' + String(error));
                            }
                        }
                    }}
                >
                    Import from a file…
                </button>
                <button
                    className={button}
                    onClick={async () => {
                        await clipboard.write(JSON.stringify(settings, null, 2));
                        setSaid('Copied to the clipboard');
                    }}
                >
                    Copy as JSON
                </button>
                <button
                    className={button}
                    onClick={async () => {
                        try {
                            read((await clipboard.read()).text ?? '');
                            setSaid('Pasted from the clipboard');
                        } catch {
                            setSaid('The clipboard doesn\'t hold settings');
                        }
                    }}
                >
                    Paste from JSON
                </button>
            </div>
            <div className="mt-4 flex flex-col gap-2 border-t border-(--fx-line) pt-4">
                <span className="text-(--fx-text)">Start over</span>
                <span className="text-[0.9em] text-(--fx-faint)">Every setting and shortcut goes back to how it came. Pinned folders stay.</span>
                <button
                    className="self-start rounded-(--fx-radius) bg-red-500 px-3 py-1.5 text-white hover:bg-red-600"
                    onClick={async () => {
                        const answer = await dialogs.message({ title: 'Reset settings', message: 'Reset every setting to its default?', kind: 'warning', buttons: 'yes-no' });
                        if (answer === 'yes') {
                            onReplace({ ...DEFAULTS, keys: { ...DEFAULT_KEYS } });
                            setSaid('Everything is back to its default');
                        }
                    }}
                >
                    Reset all settings
                </button>
            </div>
            {said ? <span className="text-[0.9em] text-(--fx-accent)">{said}</span> : null}
        </div>
    );
}

const EXTRA = [
    { id: 'keyboard', title: 'Keyboard', blurb: 'Every shortcut, and what it does.' },
    { id: 'pinned', title: 'Pinned folders', blurb: 'What the sidebar keeps at hand, in order.' },
    { id: 'backup', title: 'Backup & reset', blurb: 'Export, import, or start over.' },
];

export function SettingsPage(props: {
    settings: Settings;
    change: Change;
    replace: (settings: Settings) => void;
    pinned: string[];
    folder: string;
    onPinned: (pinned: string[]) => void;
    onClose: () => void;
}) {
    const { settings, change, replace, pinned, folder, onPinned, onClose } = props;
    const [section, setSection] = useState('appearance');
    const [query, setQuery] = useState('');

    useEffect(() => {
        const heard = (event: any) => {
            if (event.key === 'Escape') {
                onClose();
            }
        };
        document.addEventListener('keydown', heard);
        return () => document.removeEventListener('keydown', heard);
    }, []);

    const wanted = query.trim().toLowerCase();
    const matching = wanted
        ? SECTIONS.map((one) => ({ ...one, fields: one.fields.filter((field) => (field.label + ' ' + (field.hint ?? '')).toLowerCase().includes(wanted)) })).filter((one) => one.fields.length > 0)
        : [];
    const current = SECTIONS.find((one) => one.id === section);
    const extra = EXTRA.find((one) => one.id === section);
    const changedCount = SECTIONS.reduce((sum, one) => sum + one.fields.filter((field) => settings[field.key] !== DEFAULTS[field.key]).length, 0);
    const totalCount = SECTIONS.reduce((sum, one) => sum + one.fields.length, 0) + Object.keys(COMMANDS).length;

    return (
        <div className="flex min-h-0 min-w-0 flex-1 gap-(--fx-gap)">
            <nav className="flex w-56 shrink-0 flex-col gap-0.5 overflow-y-auto pb-3 pl-2 [scrollbar-width:none]">
                <div className="flex items-center gap-2 px-2 pt-1 pb-3">
                    <span className="flex text-(--fx-accent)"><I.SettingsIcon size={18} /></span>
                    <span className="text-[1.1em] font-semibold text-(--fx-text)">Settings</span>
                </div>
                <div className="mb-2 flex h-8 items-center gap-1.5 rounded-(--fx-radius-lg) bg-(--fx-tab-hover) px-2.5 focus-within:bg-(--fx-tab)">
                    <I.SearchIcon size={14} className="shrink-0 text-(--fx-faint)" />
                    <input
                        value={query}
                        placeholder={`Search ${totalCount} settings`}
                        className="min-w-0 flex-1 bg-transparent text-(--fx-text) outline-none placeholder:text-(--fx-faint)"
                        onChange={(event: any) => setQuery(event.target.value)}
                        onKeyDown={(event: any) => event.stopPropagation()}
                    />
                </div>
                {[...SECTIONS, ...EXTRA].map((one) => {
                    const Icon = ICONS[one.id] ?? I.SettingsIcon;
                    const active = !wanted && one.id === section;
                    return (
                        <button
                            key={one.id}
                            onClick={() => {
                                setQuery('');
                                setSection(one.id);
                            }}
                            className={
                                'flex items-center gap-2.5 rounded-(--fx-radius-lg) px-2.5 py-[7px] text-left transition-colors duration-(--fx-speed) ' +
                                (active ? 'bg-(--fx-tab) text-(--fx-text) shadow-(--fx-tab-shadow)' : 'text-(--fx-muted) hover:bg-(--fx-tab-hover) hover:text-(--fx-text)')
                            }
                        >
                            <span className={'flex ' + (active ? 'text-(--fx-accent)' : '')}><Icon size={16} strokeWidth={1.6} /></span>
                            {one.title}
                        </button>
                    );
                })}
                <div className="mt-auto px-2.5 pt-4 text-[0.82em] text-(--fx-faint)">{changedCount} changed from the defaults</div>
            </nav>
            <main className="flex min-w-0 flex-1 flex-col overflow-hidden rounded-(--fx-radius-lg) bg-(--fx-card) shadow-(--fx-card-shadow)">
                <div className="flex shrink-0 items-center justify-between px-8 pt-6 pb-3">
                    <div className="flex flex-col">
                        <span className="text-[1.35em] font-semibold text-(--fx-text)">{wanted ? `Results for "${query}"` : (current ?? extra)?.title}</span>
                        <span className="text-(--fx-muted)">{wanted ? `${matching.reduce((sum, one) => sum + one.fields.length, 0)} settings` : (current ?? extra)?.blurb}</span>
                    </div>
                    <button className="flex items-center gap-1.5 rounded-(--fx-radius) bg-(--fx-input) px-3 py-1.5 text-(--fx-text) transition-colors duration-(--fx-speed) hover:bg-(--fx-press)" onClick={onClose}>
                        <I.XIcon size={14} strokeWidth={2} /> Done
                    </button>
                </div>
                <div className="min-h-0 flex-1 overflow-y-auto px-8 py-4 [scrollbar-width:thin]">
                    <div className="max-w-3xl">
                        {wanted ? (
                            matching.map((one) => (
                                <div key={one.id} className="mb-6">
                                    <div className="pb-1 text-[0.8em] font-medium text-(--fx-faint)">{one.title}</div>
                                    {one.fields.map((field) => (
                                        <FieldRow key={field.key} field={field} settings={settings} change={change} />
                                    ))}
                                </div>
                            ))
                        ) : current ? (
                            current.fields.map((field) => <FieldRow key={field.key} field={field} settings={settings} change={change} />)
                        ) : section === 'keyboard' ? (
                            <Keyboard settings={settings} change={change} />
                        ) : section === 'pinned' ? (
                            <Pinned pinned={pinned} folder={folder} onPinned={onPinned} />
                        ) : (
                            <Backup settings={settings} onReplace={replace} />
                        )}
                    </div>
                </div>
            </main>
        </div>
    );
}
