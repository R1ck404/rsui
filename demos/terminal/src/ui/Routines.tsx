import { createContext, useContext, useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { DEMO_ROUTINE, readRoutines, routineCommand } from '../routines';
import type { Routine, RoutineRunner, RoutineStep, StepRun } from '../routines';
import { css, withAlpha } from '../themes';
import type { Scheme } from '../themes';
import { Kbd, SettingsButton } from './controls';
import { Icon } from './Icon';
import { TAB_WIDTH } from './TabbedPanel';

type Profile = { id: string; name: string };

const button = 'shrink-0 rounded-md px-2 py-1.5 text-[12px] hover:bg-white/10 focus-visible:outline-2';
const mono = { fontFamily: 'Cascadia Mono, Consolas, monospace' };
const inputValue = (event: any): string => typeof event === 'string' ? event : event.target.value;

interface RoutinesState {
    routines: Routine[];
    draft: RoutineStep[];
    editor: Routine | null;
    profiles: Profile[];
    defaultProfile: string;
    shortcut: (id: string) => string | undefined;
    onRun: (routine: Routine) => void;
    onEdit: (routine: Routine | null) => void;
    onSave: (routine: Routine) => void;
    onDelete: (routine: Routine) => void;
}

export const RoutinesContext = createContext<RoutinesState | null>(null);

export function RoutinesPage({ scheme }: { scheme: Scheme }) {
    const props = useContext(RoutinesContext);
    if (!props) return null;
    const { editor } = props;
    return (
        <section aria-label="Routines" className="mx-auto flex size-full max-w-[760px] flex-col px-4 pt-5" style={{ color: css(scheme.fg) }}
            onKeyDown={(event) => {
                if (editor && event.key === 'Escape') {
                    event.stopPropagation();
                    props.onEdit(null);
                }
            }}>
            <div className="shrink-0 px-4 py-3">
                <h2 className="text-[22px] font-semibold">{editor ? editor.id ? 'Edit routine' : 'New routine' : 'Routines'}</h2>
            </div>
            {editor ? (
                <RoutineEditor key={editor.id} routine={editor} scheme={scheme} profiles={props.profiles} defaultProfile={props.defaultProfile}
                    onSave={props.onSave} onCancel={() => props.onEdit(null)} />
            ) : (
                <div className="min-h-0 overflow-y-auto px-4">
                    {props.routines.map((routine) => {
                        const demo = routine.id === DEMO_ROUTINE.id;
                        const shortcut = props.shortcut(`routine:${routine.id}`);
                        return (
                            <div key={routine.id} className="flex items-center gap-2 border-b border-white/10 py-3">
                                <div className="min-w-0 flex-1">
                                    <div className="truncate text-[14px] font-medium" title={routine.name}>{routine.name}</div>
                                    <div className="text-[12px]" style={{ color: withAlpha(scheme.fg, 0.7) }}>
                                        {demo ? 'Simulated Web, API, and Tests. API needs a retry on the first attempt.' : `${routine.steps.length} ${routine.steps.length === 1 ? 'step' : 'steps'}`}
                                    </div>
                                </div>
                                {shortcut ? <Kbd scheme={scheme}>{shortcut}</Kbd> : null}
                                {demo ? null : <button className={button} aria-label={`Edit ${routine.name}`} onClick={() => props.onEdit(routine)}>Edit</button>}
                                {demo ? null : <button className={button} aria-label={`Delete ${routine.name}`} onClick={() => props.onDelete(routine)}>Delete</button>}
                                <SettingsButton scheme={scheme} label={`Run ${routine.name}`} onClick={() => props.onRun(routine)}>Run</SettingsButton>
                            </div>
                        );
                    })}
                </div>
            )}
            {editor ? null : (
                <div className="flex shrink-0 items-center gap-2 px-4 py-3">
                    <div className="min-w-0 flex-1 text-[12px]" style={{ color: withAlpha(scheme.fg, 0.7) }}>Assign launch shortcuts in Settings → Keyboard.</div>
                    {props.draft.length > 0 ? <SettingsButton scheme={scheme} onClick={() => props.onEdit({ id: '', name: '', steps: props.draft })}>Save draft ({props.draft.length})</SettingsButton> : null}
                    <SettingsButton scheme={scheme} primary onClick={() => props.onEdit({ id: '', name: '', steps: [] })}>New routine</SettingsButton>
                </div>
            )}
        </section>
    );
}

function RoutineEditor(props: {
    routine: Routine;
    scheme: Scheme;
    profiles: Profile[];
    defaultProfile: string;
    onSave: (routine: Routine) => void;
    onCancel: () => void;
}) {
    const { routine, scheme, profiles } = props;
    const [name, setName] = useState(routine.name);
    const [steps, setSteps] = useState(() => routine.steps.map((step, key) => ({ ...step, phase: String(step.phase), key })));
    const nextKey = useRef(steps.length);
    const fieldRef = useRef<HTMLInputElement>(null);
    useEffect(() => { fieldRef.current?.focus(); }, []);
    const field = 'min-w-0 w-full rounded-md px-2 py-1.5 text-[12px] focus-visible:outline-2';
    const fill = { background: withAlpha(scheme.fg, 0.08), color: css(scheme.fg) };
    const muted = { color: withAlpha(scheme.fg, 0.7) };
    const candidate = {
        id: routine.id || 'draft', name: name.trim(),
        steps: steps.map(({ phase, name, command, profile, cwd, ready }) => ({ name: name.trim(), phase: Number(phase), command, profile, cwd: cwd.trim(), ready: ready.trim() ? ready : '' })),
    };
    const canSave = readRoutines(JSON.stringify([candidate])).length === 1;
    const addStep = () => {
        const key = nextKey.current++;
        const profile = profiles.find(({ id }) => id === props.defaultProfile)?.id ?? profiles[0]?.id ?? 'powershell';
        setSteps([...steps, { key, name: '', command: '', cwd: '', ready: '', profile, phase: String((Number(steps.at(-1)?.phase) || 0) + 1) }]);
    };

    return (
        <>
            <div className="min-h-0 overflow-y-auto px-4 py-3">
                <div className="mb-1 text-[12px]" style={muted}>Routine name</div>
                <input ref={fieldRef} aria-label="Routine name" value={name} className={field} style={fill} onChange={(event: any) => setName(inputValue(event))} />
                <div className="mb-1 mt-3 text-[12px]" style={muted}>Steps in the same phase start together. Later phases wait for every earlier step to succeed or print its ready text.</div>
                {steps.map((step, index) => {
                    const patch = (update: Partial<typeof step>) => setSteps(steps.map((item) => item.key === step.key ? { ...item, ...update } : item));
                    const currentProfile = profiles.findIndex(({ id }) => id === step.profile);
                    return (
                        <div key={step.key} className="flex flex-col gap-2 border-b border-white/10 py-3">
                            <div className="flex items-center gap-2">
                                <input aria-label={`Pane name ${index + 1}`} placeholder="Pane name" value={step.name} className={field} style={fill} onChange={(event: any) => patch({ name: inputValue(event) })} />
                                <span className="shrink-0 text-[12px]" style={muted}>Phase</span>
                                <input aria-label={`Phase ${index + 1}`} value={step.phase} className="w-12 rounded-md px-2 py-1.5 text-[12px] focus-visible:outline-2" style={fill} onChange={(event: any) => patch({ phase: inputValue(event) })} />
                                <button className={button} aria-label={`Remove step ${index + 1}`} onClick={() => setSteps(steps.filter((item) => item.key !== step.key))}>Remove</button>
                            </div>
                            <input aria-label={`Command ${index + 1}`} placeholder="Shell command" value={step.command} className={field} style={{ ...fill, ...mono }} onChange={(event: any) => patch({ command: inputValue(event) })} />
                            <div className="flex gap-2">
                                <button className={button} style={fill} title="Click to switch shell profiles" disabled={profiles.length < 2 && currentProfile >= 0}
                                    aria-label={`Shell profile ${index + 1} ${profiles[currentProfile]?.name ?? step.profile}`}
                                    onClick={() => { if (profiles.length) patch({ profile: profiles[(currentProfile + 1) % profiles.length].id }); }}>
                                    {profiles[currentProfile]?.name ?? step.profile}{profiles.length > 1 ? ' ↻' : ''}
                                </button>
                                <input aria-label={`Folder ${index + 1}`} placeholder="Folder (defaults to launch folder)" value={step.cwd} className={field} style={{ ...fill, ...mono }} onChange={(event: any) => patch({ cwd: inputValue(event) })} />
                            </div>
                            <input aria-label={`Ready text ${index + 1}`} placeholder="Ready text (optional, otherwise wait for exit code 0)" value={step.ready} className={field} style={{ ...fill, ...mono }} onChange={(event: any) => patch({ ready: inputValue(event) })} />
                        </div>
                    );
                })}
                <button className={`${button} mt-2`} onClick={addStep}>+ Add step</button>
            </div>
            <div className="flex shrink-0 items-center justify-end gap-2 border-t border-white/10 px-4 py-3">
                {canSave ? null : <div className="mr-auto text-[11px]" style={muted}>Enter a name, a command for each step, and positive phase numbers. Use absolute Windows paths for saved folders.</div>}
                <SettingsButton scheme={scheme} onClick={props.onCancel}>Cancel</SettingsButton>
                <SettingsButton scheme={scheme} primary disabled={!canSave} onClick={() => props.onSave({ ...candidate, id: routine.id })}>Save routine</SettingsButton>
            </div>
        </>
    );
}

const status: Record<StepRun['status'], [string, number | null]> = {
    waiting: ['Queued', null], starting: ['Starting', 3], running: ['Running', 3], ready: ['Ready', 2],
    passed: ['Passed', 2], failed: ['Failed', 1], stopped: ['Stopped', null],
};

export function RoutineSteps({ runner, scheme, activeId, sessionIds, tabWidth, close }: {
    runner: RoutineRunner; scheme: Scheme; activeId?: number; sessionIds: number[]; tabWidth?: number; close: () => void;
}) {
    useSyncExternalStore(runner.subscribe, runner.getVersion);
    const run = runner.run;
    if (!run) return null;
    const muted = withAlpha(scheme.fg, 0.63);
    const line = withAlpha(scheme.fg, 0.12);
    const accent = css(scheme.ansi[4]);
    const tone = (entry: StepRun) => {
        const index = status[entry.status][1];
        return index === null ? muted : css(scheme.ansi[index]);
    };
    return (
        <aside aria-label="Routine steps" className="relative flex shrink-0 flex-col border-r"
            style={{ width: tabWidth ?? TAB_WIDTH, maxWidth: tabWidth === undefined ? '42%' : undefined, borderColor: line }}>
            <div className="shrink-0 border-b px-3 pb-2.5 pt-3" style={{ borderColor: line }}>
                <div className="flex items-center gap-1.5" style={{ color: accent }}>
                    <Icon name="split" size={13} />
                    <h2 className="min-w-0 flex-1 truncate text-[12px] font-semibold" style={{ color: css(scheme.fg) }} title={run.routine.name}>{run.routine.name}</h2>
                    <button className="flex size-[18px] shrink-0 items-center justify-center rounded hover:bg-white/10 focus-visible:outline-2"
                        aria-label="Close routine tab" title="Close routine tab" style={{ color: muted }} onClick={close}><Icon name="close" size={11} /></button>
                </div>
                <div aria-hidden="true" className="mt-2.5 flex h-[3px] gap-0.5">
                    {run.steps.map((entry, index) => (
                        <span key={index} className="min-w-0 flex-1 rounded-full" style={{ background: status[entry.status][1] === null ? line : tone(entry) }} />
                    ))}
                </div>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto px-1.5 py-1.5">
                {run.steps.map((entry, index) => {
                    const id = entry.shell?.sessionId;
                    const active = id !== undefined && activeId === id;
                    const previous = entry.previous.filter(({ shell }) => shell.sessionId !== undefined && sessionIds.includes(shell.sessionId));
                    const label = status[entry.status][0];
                    return (
                        <div key={index} className="mb-0.5 min-w-0">
                            <button className="flex w-full min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-left enabled:hover:bg-white/10 focus-visible:outline-2 focus-visible:outline-offset-[-2px]"
                                style={{ background: active ? withAlpha(scheme.ansi[4], 0.15) : undefined, outlineColor: accent }}
                                title={routineCommand(run.routine, entry.step, entry.attempt)} aria-label={`Focus ${entry.step.name} pane`} aria-pressed={active}
                                disabled={id === undefined || !sessionIds.includes(id)} onClick={() => entry.shell?.focus()}>
                                <span className="size-1.5 shrink-0 rounded-full" style={{ background: tone(entry) }} />
                                <span className="min-w-0 flex-1 truncate text-[12px] font-medium">{entry.step.name}</span>
                                <span className="shrink-0 text-[10px]" style={{ color: tone(entry) }}>{label}</span>
                            </button>
                            {entry.detail || previous.length > 0 ? (
                                <div className="ml-3 mt-0.5 space-y-0.5 border-l pl-2" style={{ borderColor: line, color: muted }}>
                                    {entry.detail ? <div className="px-1 pt-0.5 text-[10px] leading-[14px]">{entry.detail}</div> : null}
                                    {entry.status === 'failed' && !run.stopped ? (
                                        <button className="flex items-center gap-1 rounded px-1 py-0.5 text-[11px] hover:bg-white/10 focus-visible:outline-2" aria-label={`Retry ${entry.step.name}`}
                                            style={{ color: accent }} onClick={() => runner.retry(index)}><Icon name="reload" size={11} /> Retry</button>
                                    ) : null}
                                    {previous.map(({ attempt, shell }) => (
                                        <button key={attempt} className="block w-full truncate rounded px-1 py-0.5 text-left text-[10px] hover:bg-white/10 focus-visible:outline-2"
                                            style={{ color: shell.sessionId === activeId ? accent : muted }} aria-pressed={shell.sessionId === activeId}
                                            aria-label={`View ${entry.step.name} attempt ${attempt}`} onClick={shell.focus}>Attempt {attempt} · failed</button>
                                    ))}
                                </div>
                            ) : null}
                        </div>
                    );
                })}
            </div>
            {runner.busy ? (
                <div className="shrink-0 border-t p-1.5" style={{ borderColor: line, color: muted }}>
                    <button className="flex w-full items-center gap-1.5 rounded px-1.5 py-1 text-left text-[11px] hover:bg-white/10 focus-visible:outline-2"
                        title="Cancel queued steps and leave running commands in their panes" onClick={() => runner.stop()}><Icon name="stop" size={11} /> Stop routine</button>
                </div>
            ) : null}
        </aside>
    );
}
