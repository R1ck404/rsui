import { useState } from 'react';
import { promises as fs } from 'fs';
import { dialogs } from 'rsui/desktop';
import { buildCommands, commandKeys, GLOBAL_SHORTCUT_ID, shortcutConflict } from '../commands';
import type { Context } from '../commands';
import { basename } from 'path';
import { settingsStore, useSettings } from '../settings';
import { exportSettings, importSettings, parseSettings, readField, settingPaths, SETTINGS_GROUPS } from '../settings-transfer';
import { css, SCHEMES, withAlpha } from '../themes';
import type { Scheme } from '../themes';
import { Icon } from './Icon';
import { keyLabel, SettingsButton, SettingsCard } from './controls';

type SettingPath = (typeof settingPaths)[number];
type Transfer = { kind: 'export' } | { kind: 'import'; name: string; file: ReturnType<typeof parseSettings> };
const filters = [{ name: 'JSON settings', extensions: ['json'] }];

export function SettingsTransferPage({ context, scheme }: { context: Context; scheme: Scheme }) {
    const settings = useSettings();
    const [transfer, setTransfer] = useState<Transfer | null>(null);
    const [selected, setSelected] = useState<Set<SettingPath>>(new Set());
    const [expanded, setExpanded] = useState<Set<string>>(new Set());
    const [busy, setBusy] = useState(false);
    const [status, setStatus] = useState<{ text: string; error?: boolean } | null>(null);

    const start = (next: Transfer) => {
        setStatus(null);
        const onlyGroup = next.kind === 'import' && SETTINGS_GROUPS.find((group) => next.file.paths.every((path) => group.fields.some(([key]) => key === path)));
        setExpanded(new Set(onlyGroup ? [onlyGroup.label] : []));
        setSelected(new Set(next.kind === 'export' ? settingPaths : next.file.paths));
        setTransfer(next);
    };
    const open = async () => {
        setBusy(true);
        setStatus(null);
        try {
            const path = await dialogs.open({ title: 'Import Terminal settings', filters });
            if (path) start({ kind: 'import', name: basename(path), file: parseSettings(await fs.readFile(path, 'utf8')) });
        } catch (error) {
            setStatus({ text: error instanceof Error ? error.message : String(error), error: true });
        } finally {
            setBusy(false);
        }
    };
    const apply = async () => {
        if (!transfer || !selected.size) return;
        setBusy(true);
        setStatus(null);
        try {
            const settings = settingsStore.get();
            if (transfer.kind === 'export') {
                const path = await dialogs.save({ title: 'Export Terminal settings', filters, start: 'terminal-settings.json' });
                if (!path) return;
                await fs.writeFile(path, exportSettings(settings, selected));
            } else {
                const patch = importSettings(settings, transfer.file.settings, selected);
                if (patch.keymap || patch.quakeKey) {
                    const next = { ...settings, ...patch };
                    const commands = buildCommands(context);
                    const globalConflict = shortcutConflict(commands, next, GLOBAL_SHORTCUT_ID, [next.quakeKey]);
                    if (globalConflict) throw new Error(`The global dropdown shortcut is already used by ${globalConflict}.`);
                    for (const command of commands) {
                        if (!Object.hasOwn(next.keymap, command.id)) continue;
                        const conflict = shortcutConflict(commands, next, command.id, commandKeys(command, next.keymap));
                        if (conflict) throw new Error(`${command.title} shares a shortcut with ${conflict}.`);
                    }
                }
                context.change(patch);
            }
            setStatus({ text: `${selected.size} ${selected.size === 1 ? 'setting' : 'settings'} ${transfer.kind === 'export' ? 'exported' : 'imported'}.` });
            setTransfer(null);
        } catch (error) {
            setStatus({ text: error instanceof Error ? error.message : String(error), error: true });
        } finally {
            setBusy(false);
        }
    };

    const available = transfer?.kind === 'import' ? transfer.file.paths : settingPaths;
    const toggle = (paths: readonly SettingPath[], on: boolean) => setSelected((old) => {
        const next = new Set(old);
        for (const path of paths) {
            if (on) next.add(path);
            else next.delete(path);
        }
        return next;
    });
    const sub = withAlpha(scheme.fg, 0.6);
    const line = withAlpha(scheme.fg, 0.09);
    const importPreview = (path: SettingPath) => {
        if (transfer?.kind !== 'import') return;
        const current = readField({ ...settings }, path);
        const incoming = readField(transfer.file.settings, path);
        const format = (value: unknown) => {
            if (path === 'scheme') return SCHEMES.find((scheme) => scheme.id === value)?.name ?? String(value);
            if (path === 'profile') return context.profiles.find((profile) => profile.id === value)?.name ?? String(value);
            if (path === 'quakeKey' && typeof value === 'string') return keyLabel(value);
            if (typeof value === 'boolean') return value ? 'On' : 'Off';
            if (value === '') return 'Empty';
            if (value === null) return 'None';
            if (Array.isArray(value)) return value.length ? value.join(', ') : 'None';
            if (typeof value === 'object') {
                const count = Object.keys(value).length;
                if (path === 'studio') return 'Saved design';
                return `${count} ${path === 'keymap' ? (count === 1 ? 'customized command' : 'customized commands') : (count === 1 ? 'entry' : 'entries')}`;
            }
            return String(value);
        };
        const value = format(incoming);
        if (JSON.stringify(current) === JSON.stringify(incoming)) return `${value} (unchanged)`;
        return typeof incoming === 'object' && incoming !== null && !Array.isArray(incoming)
            ? `Incoming ${value}` : `${format(current)} → ${value}`;
    };

    return (
        <>
            {status ? <p role="status" className="mb-4 text-[13px]" style={{ color: status.error ? css(scheme.ansi[3]) : sub }}>{status.text}</p> : null}
            {!transfer ? (
                <div className="flex flex-wrap gap-2">
                    <SettingsButton scheme={scheme} disabled={busy} onClick={() => void open()}>Import settings</SettingsButton>
                    <SettingsButton scheme={scheme} disabled={busy} onClick={() => start({ kind: 'export' })}>Export settings</SettingsButton>
                </div>
            ) : (
                <>
                    <h2 className="mb-2 text-[16px] font-semibold">{transfer.kind === 'export' ? 'Choose settings to export' : 'Choose settings to import'}</h2>
                    <p className="mb-4 text-[13px] [overflow-wrap:anywhere]" style={{ color: sub }}>
                        {transfer.kind === 'import' ? `From ${transfer.name}. Expand a section to review its values.` : 'Select a section or expand it to choose individual settings.'}
                    </p>
                    <div className="mb-3 flex flex-wrap items-center gap-2">
                        <SettingsButton scheme={scheme} disabled={busy || selected.size === available.length} onClick={() => setSelected(new Set(available))}>Select all</SettingsButton>
                        <SettingsButton scheme={scheme} disabled={busy || !selected.size} onClick={() => setSelected(new Set())}>Clear all</SettingsButton>
                        <span className="ml-auto text-[12px] tabular-nums" style={{ color: sub }}>{selected.size} of {available.length} selected</span>
                    </div>
                    <SettingsCard scheme={scheme}>
                        {SETTINGS_GROUPS.map(({ label, fields }) => {
                            const shown = fields.filter(([path]) => available.includes(path));
                            if (!shown.length) return null;
                            const paths = shown.map(([path]) => path);
                            const count = paths.filter((path) => selected.has(path)).length;
                            const open = expanded.has(label);
                            return (
                                <div key={label}>
                                    <div className="flex items-center gap-1 px-2">
                                        <button type="button" aria-label={`${open ? 'Collapse' : 'Expand'} ${label}`} aria-expanded={open}
                                            className="flex size-8 shrink-0 items-center justify-center rounded-md hover:bg-white/10" style={{ color: sub }}
                                            onClick={() => setExpanded((old) => {
                                                const next = new Set(old);
                                                if (next.has(label)) next.delete(label);
                                                else next.add(label);
                                                return next;
                                            })}>
                                            <span className={open ? '' : '-rotate-90'}><Icon name="down" size={12} /></span>
                                        </button>
                                        <Selection scheme={scheme} checked={count === paths.length ? true : count > 0 ? 'mixed' : false} disabled={busy}
                                            label={label} onClick={() => toggle(paths, count !== paths.length)} />
                                        <span className="shrink-0 pr-2 text-[11px] tabular-nums" style={{ color: sub }}>{count}/{paths.length}</span>
                                    </div>
                                    {open ? (
                                        <div className="mb-2 ml-12 mr-4 pl-3" style={{ borderLeft: `1px solid ${line}` }}>
                                            {shown.map(([path, name]) => (
                                                <Selection key={path} scheme={scheme} checked={selected.has(path)} disabled={busy}
                                                    label={name} detail={importPreview(path)} onClick={() => toggle([path], !selected.has(path))} />
                                            ))}
                                        </div>
                                    ) : null}
                                </div>
                            );
                        })}
                    </SettingsCard>
                    {selected.has('studio') ? <p className="mt-3 text-[12px]" style={{ color: sub }}>Studio designs reference media by file path.</p> : null}
                    <div className="mt-4 flex flex-wrap justify-end gap-2">
                        <SettingsButton scheme={scheme} disabled={busy} onClick={() => { setTransfer(null); setStatus(null); }}>Cancel</SettingsButton>
                        <SettingsButton scheme={scheme} primary disabled={busy || !selected.size} onClick={() => void apply()}>
                            {transfer.kind === 'export' ? 'Save JSON file' : 'Apply selected settings'}
                        </SettingsButton>
                    </div>
                </>
            )}
        </>
    );
}

function Selection({ scheme, checked, label, detail, disabled, onClick }: { scheme: Scheme; checked: boolean | 'mixed'; label: string; detail?: string; disabled: boolean; onClick: () => void }) {
    return (
        <button type="button" role="checkbox" aria-checked={checked} disabled={disabled}
            className="flex min-h-8 w-full flex-1 items-center gap-2.5 rounded-md px-1 py-1 text-left text-[13px] hover:bg-white/[0.06]"
            onClick={onClick}>
            <span aria-hidden="true" className="flex size-4 shrink-0 items-center justify-center rounded" style={{
                background: checked ? css(scheme.fg) : 'transparent', color: css(scheme.bg), border: `1px solid ${withAlpha(scheme.fg, 0.38)}`,
            }}>
                {checked === true ? <Icon name="check" size={11} /> : checked === 'mixed' ? <span className="h-px w-2 bg-current" /> : null}
            </span>
            <span className="min-w-0">
                <span className="block">{label}</span>
                {detail ? <span className="block text-[12px] [overflow-wrap:anywhere]" style={{ color: withAlpha(scheme.fg, 0.62) }}>{detail}</span> : null}
            </span>
        </button>
    );
}
