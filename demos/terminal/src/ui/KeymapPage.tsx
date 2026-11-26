import { useEffect, useRef, useState } from 'react';
import { buildCommands, commandKeys, GLOBAL_SHORTCUT_ID, shortcutConflict } from '../commands';
import type { Command, Context } from '../commands';
import { chord, isNativeShortcut, isShortcut } from '../keymap';
import { DEFAULTS } from '../settings';
import type { Settings } from '../settings';
import { css, withAlpha } from '../themes';
import type { Scheme } from '../themes';
import { Icon } from './Icon';
import { Kbd, keyLabel, SettingsButton, SettingsCard } from './controls';

const GLOBAL = GLOBAL_SHORTCUT_ID;
const dropdown: Command = { id: GLOBAL, title: 'Toggle the dropdown terminal from any app', group: 'Global', run: (context) => context.toggleQuake() };

export function KeymapPage({ context, scheme }: { context: Context; scheme: Scheme }) {
    const { settings, change, setCapturingShortcut } = context;
    const [query, setQuery] = useState('');
    const [draft, setDraft] = useState<{ id: string; key: string; replace?: string } | null>(null);
    const [message, setMessage] = useState<{ id: string; text: string } | null>(null);
    const [beforeReset, setBeforeReset] = useState<Pick<Settings, 'keymap' | 'quakeKey'> | null>(null);
    const input = useRef<HTMLInputElement>(null);
    const editing = draft?.id;
    const replacing = draft?.replace;
    useEffect(() => {
        if (editing) input.current?.focus();
    }, [editing, replacing]);
    useEffect(() => () => setCapturingShortcut(false), [setCapturingShortcut]);

    const commands = buildCommands(context);
    const keysFor = (command: Command) => command.id === GLOBAL ? [settings.quakeKey] : commandKeys(command, settings.keymap);
    const customized = Object.keys(settings.keymap).length > 0 || settings.quakeKey !== DEFAULTS.quakeKey;
    const groups = new Map<string, Command[]>();
    const search = query.trim().toLowerCase();
    for (const command of [dropdown, ...commands]) {
        if (search && ![command.title, command.group, ...keysFor(command).map(keyLabel)].some((text) => text.toLowerCase().includes(search))) continue;
        const group = groups.get(command.group) ?? [];
        group.push(command);
        groups.set(command.group, group);
    }

    const closeEditor = () => {
        setCapturingShortcut(false);
        setDraft(null);
        setMessage(null);
    };
    const save = (command: Command, keys: string[], reset = false) => {
        const { id } = command;
        const conflict = shortcutConflict(commands, settings, id, keys.filter((key) => !keysFor(command).includes(key)));
        if (conflict) {
            setMessage({ id, text: `Already used by ${conflict}. Choose another shortcut or remove that binding first.` });
            return;
        }
        if (id === GLOBAL) change({ quakeKey: keys[0] });
        else {
            const keymap = { ...settings.keymap };
            if (reset) delete keymap[id];
            else keymap[id] = keys;
            change({ keymap });
        }
        setBeforeReset(null);
        closeEditor();
    };
    const edit = (id: string, replace?: string) => {
        setMessage(null);
        setDraft({ id, key: '', replace });
    };

    const sub = withAlpha(scheme.fg, 0.56);
    return (
        <>
            <div className="mb-5 flex items-center gap-2">
                <input
                    aria-label="Filter commands and shortcuts"
                    placeholder="Filter commands and shortcuts"
                    value={query}
                    className="h-8 min-w-0 flex-1 rounded-md px-3 text-[13px]"
                    style={{ background: withAlpha(scheme.fg, 0.07), color: css(scheme.fg) }}
                    onChange={(e: any) => { closeEditor(); setQuery(typeof e === 'string' ? e : e.target.value); }}
                />
                <SettingsButton scheme={scheme} disabled={!customized} onClick={() => {
                    setBeforeReset({ keymap: settings.keymap, quakeKey: settings.quakeKey });
                    change({ keymap: {}, quakeKey: DEFAULTS.quakeKey });
                    closeEditor();
                }}>Reset all shortcuts</SettingsButton>
            </div>
            {beforeReset ? (
                <div className="mb-4 flex items-center gap-3 text-[13px]">
                    <span role="status" style={{ color: sub }}>Default shortcuts restored.</span>
                    <SettingsButton scheme={scheme} onClick={() => { change(beforeReset); setBeforeReset(null); closeEditor(); }}>Undo reset</SettingsButton>
                </div>
            ) : null}
            {groups.size === 0 ? <p className="text-[13px]" style={{ color: sub }}>No commands match.</p> : null}
            {[...groups].map(([group, list]) => (
                <div key={group} style={{ contentVisibility: 'auto', containIntrinsicHeight: `${46 + list.length * 44}px` }}>
                    <SettingsCard scheme={scheme} title={group}>
                        {list.map((command) => {
                            const { id, title } = command;
                            const global = id === GLOBAL;
                            const keys = keysFor(command);
                            const changed = global ? settings.quakeKey !== DEFAULTS.quakeKey : Object.hasOwn(settings.keymap, id);
                            const active = draft?.id === id;
                            const candidate = active ? draft.key : '';
                            const conflict = candidate ? shortcutConflict(commands, settings, id, [candidate]) : undefined;
                            const problem = keys.includes(candidate) ? 'This shortcut is already assigned here.'
                                : conflict ? `Already used by ${conflict}. Choose another shortcut or remove that binding first.`
                                : message?.id === id ? message.text : undefined;
                            const saveDraft = () => {
                                if (!active || !candidate || problem) return;
                                const next = global ? [candidate] : draft.replace
                                    ? keys.map((key) => key === draft.replace ? candidate : key)
                                    : [...keys, candidate];
                                save(command, next);
                            };
                            return (
                                <div key={id} className="px-3 py-2">
                                    <div className="flex flex-wrap items-center justify-between gap-2">
                                        <div className="min-w-0 flex-1 text-[13px]">{title}
                                            {global && !settings.quakeEnabled ? <div className="text-[11px]" style={{ color: sub }}>Disabled in Strip settings</div> : null}
                                        </div>
                                        <div className="flex flex-wrap items-center gap-1.5">
                                            {keys.length === 0 ? <span className="text-[11px]" style={{ color: sub }}>Unassigned</span> : null}
                                            {keys.map((key) => (
                                                <div key={key} className="flex items-center gap-0.5">
                                                    {global ? <Kbd scheme={scheme}>{keyLabel(key)}</Kbd> : (
                                                        <button type="button" className="rounded p-0.5 hover:bg-white/10" title="Change shortcut"
                                                            aria-label={`Change ${keyLabel(key)} for ${title}`} onClick={() => edit(id, key)}>
                                                            <Kbd scheme={scheme}>{keyLabel(key)}</Kbd>
                                                        </button>
                                                    )}
                                                    {global ? null : (
                                                        <button type="button" className="rounded p-1 hover:bg-white/10" style={{ color: sub }}
                                                            aria-label={`Remove ${keyLabel(key)} from ${title}`} onClick={() => save(command, keys.filter((entry) => entry !== key))}>
                                                            <Icon name="close" size={11} />
                                                        </button>
                                                    )}
                                                </div>
                                            ))}
                                            {changed ? <SettingsButton scheme={scheme} label={`Reset shortcut for ${title}`} onClick={() => save(command, global ? [DEFAULTS.quakeKey] : command.keys ?? [], true)}>Reset</SettingsButton> : null}
                                            {active ? null : <SettingsButton scheme={scheme} label={`${global ? 'Change' : 'Add'} shortcut for ${title}`} onClick={() => edit(id)}>{global ? 'Change' : 'Add'}</SettingsButton>}
                                        </div>
                                    </div>
                                    {active ? (
                                        <div className="mt-2 flex flex-wrap items-center gap-2">
                                            <input
                                                ref={input}
                                                readOnly
                                                aria-label={`Press a shortcut for ${title}`}
                                                placeholder="Press a shortcut"
                                                value={draft.key ? keyLabel(draft.key) : ''}
                                                className="h-8 min-w-0 flex-1 rounded-md px-2.5 text-[13px]"
                                                style={{ background: withAlpha(scheme.fg, 0.08), color: css(scheme.fg) }}
                                                onFocus={() => setCapturingShortcut(true)}
                                                onBlur={() => setCapturingShortcut(false)}
                                                onKeyDown={(e: KeyboardEvent) => {
                                                    const key = chord(e);
                                                    if (key === 'tab' || key === 'shift+tab') return;
                                                    e.preventDefault();
                                                    if (key === 'escape') return closeEditor();
                                                    if (e.repeat || e.isComposing || ['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) return;
                                                    if (key === 'enter') return saveDraft();
                                                    if (!(global ? isNativeShortcut(key) : isShortcut(key))) {
                                                        setDraft({ ...draft, key: '' });
                                                        setMessage({ id, text: 'Use a supported key combination, such as Ctrl+Shift+K or F8.' });
                                                        return;
                                                    }
                                                    setMessage(null);
                                                    setDraft({ ...draft, key });
                                                }}
                                            />
                                            <SettingsButton scheme={scheme} primary disabled={!candidate || !!problem} onClick={saveDraft}>Save</SettingsButton>
                                            <SettingsButton scheme={scheme} onClick={closeEditor}>Cancel</SettingsButton>
                                        </div>
                                    ) : null}
                                    {problem ? <p role="status" className="mt-2 text-[12px]" style={{ color: css(scheme.ansi[3]) }}>{problem}</p> : null}
                                </div>
                            );
                        })}
                    </SettingsCard>
                </div>
            ))}
        </>
    );
}
