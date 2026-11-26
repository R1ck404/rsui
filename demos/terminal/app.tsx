// A terminal whose columns are a strip: near-full-width shells side by side in one row, with the edges of the ones next to the
// open one showing at the sides. Slide between them with Ctrl+Alt+Left and Right, or shrink them all into a grid to pick one.
//
// What a shell is, how it's drawn, and what each key does is in `src/`. This is the window that holds them and routes the keys.

import './app.css';
import * as fs from 'fs';
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { mainWindow, useWindowState } from 'rsui';
import { PageWindow } from 'rsui/web';
import { clipboard, menus, notifications, shell, shortcuts, sound, taskbar } from 'rsui/desktop';
import { hostname } from 'os';
import { buildCommands, commandKeys, indexKeys } from './src/commands';
import type { Context } from './src/commands';
import { chord, isNativeShortcut } from './src/keymap';
import { allProfiles, discoverProfiles } from './src/launch';
import { changeSettings, settingsStore, storageGet, storageSet, useWindowSettings } from './src/settings';
import { DEMO_ROUTINE, integratedProfile, readRoutines, RoutineRunner, singleLine, windowsFolder } from './src/routines';
import type { Routine, RoutineStep } from './src/routines';
import { routineShells } from './src/routine-shell';
import { copyKey } from './src/term/copymode';
import type { Session } from './src/term/Session';
import { linkAt, terminalInput } from './src/term/TerminalPane';
import type { PaneActions } from './src/term/TerminalPane';
import { Palette, SearchBar, Toasts } from './src/ui/Overlays';
import { keyLabel, withShortcut } from './src/ui/controls';
import { Chrome } from './src/ui/Chrome';
import { load as loadMedia } from './src/studio/media';
import { PAGE_KINDS, pageEnv, pageLabel } from './src/ui/Pages';
import { SpaceEditor, SpacePopover } from './src/ui/Spaces';
import type { Anchor } from './src/ui/Spaces';
import { Quake } from './src/quake';
import type { QuakeState } from './src/quake';
import { saveRecording, tabRecordingPath } from './src/term/recording';
import { TabSessionRecorder } from './src/term/tab-session';
import type { Cast } from './src/term/recording';
import { Player } from './src/ui/Recordings';
import { RoutinesContext } from './src/ui/Routines';
import { Modal } from './src/ui/Modal';
import { resolveScheme, css, withAlpha } from './src/themes';
import { themeOf, Workspace } from './src/workspace';
import type { Column } from './src/workspace';
import type { Command as ShellCommand } from './src/vt/terminal';
import { needsPasteConfirmation } from './src/vt/keys';

export const title = 'Terminal';
export const size = [1360, 860];
export const minSize = [620, 420];
// It draws its own title bar.
export const frameless = true;

export default function Root() {
    return (
        <PageWindow>
            <App />
        </PageWindow>
    );
}

type Overlay = 'none' | 'palette' | 'paste' | 'spaces' | 'space' | 'replay';
type RecordingState = 'idle' | 'starting' | 'recording' | 'saving';

function App() {
    const settings = useWindowSettings();
    const overrides = settings.schemeOverrides[settings.scheme];
    const scheme = useMemo(() => resolveScheme(settings.scheme, overrides), [settings.scheme, overrides]);
    const windowState = useWindowState();
    const [overlay, setOverlay] = useState<Overlay>('none');
    const [broadcast, setBroadcast] = useState(false);
    const [spaceAnchor, setSpaceAnchor] = useState<Anchor>({ x: 12, y: 48 });
    const [editing, setEditing] = useState<number>(0);
    const [pasting, setPasting] = useState<{ targets: Session[]; text: string } | null>(null);
    const [replay, setReplay] = useState<Cast | null>(null);
    const [quakeState, setQuakeState] = useState<QuakeState>('off');
    const [quake] = useState(() => new Quake(() => settingsStore.get().quakeHeight, setQuakeState));
    const [capturingShortcut, setCapturingShortcut] = useState(false);
    const [savedRoutines, setSavedRoutines] = useState(() => readRoutines(storageGet('terminal.routines')));
    const [routineEditor, setRoutineEditor] = useState<Routine | null>(null);
    const [routineDraft, setRoutineDraft] = useState<{ key: string; step: RoutineStep }[]>([]);
    const started = useRef(false);
    const [tabRecording, setTabRecording] = useState<RecordingState>('idle');
    const recordingState = useRef<RecordingState>('idle');
    const recordingTab = useRef<{ tab: Column; recorder: TabSessionRecorder } | null>(null);
    const recordingJob = useRef<Promise<void> | null>(null);

    // Everything a callback needs that changes as the app does, so the callbacks themselves can stay the same.
    const latest = useRef({} as any);

    const [workspace] = useState(
        () =>
            new Workspace({
                onBell: (session) => latest.current.onBell?.(session),
                onFinished: (session, command) => latest.current.onFinished?.(session, command),
                onClipboard: (text) => latest.current.onClipboard?.(text),
                onNotify: (session, title, body) => latest.current.onNotify?.(session, title, body),
                onProgress: (session) => latest.current.onProgress?.(session),
            }),
    );
    const workspaceVersion = useSyncExternalStore(workspace.subscribe, workspace.getVersion);
    useEffect(() => () => workspace.allColumns.forEach((column) => column.routine?.close()), [workspace]);
    const session = workspace.currentSession;
    const routineTab = workspace.current?.routine ? workspace.current : workspace.allColumns.findLast((column) => column.routine);
    useSessionVersion(session);
    // A handle for checks that drive the app with `eval` steps.
    (globalThis as any).__terminal = {
        workspace,
        latest,
        runner: routineTab?.routine?.runner,
        /** A file's bytes as base64, for checks that send a picture. */
        base64: (file: string) => fs.readFileSync(file).toString('base64'),
        /** The studio's loader, for checks that look at what a file decodes to. */
        media: loadMedia,
        run: (id: string) => latest.current.commands.find((c: any) => c.id === id)?.run(latest.current.context),
    };

    // ---- What the shell and the system say

    const copyText = (text: string) => {
        if (text !== '') void clipboard.write(text);
    };
    const copy = (target: Session) => {
        const text = target.selectionText();
        if (text === '') return false;
        copyText(text);
        return true;
    };
    const paste = async (target: Session) => {
        const contents = await clipboard.read();
        const text = contents.text ?? '';
        if (text === '') return;
        const targets = inputTargets(target);
        if (settings.warnOnMultilinePaste && needsPasteConfirmation(text, targets.map(({ term }) => term), settings.pasteAtPromptOnly)) {
            setPasting({ targets, text });
            setOverlay('paste');
            return;
        }
        send(targets, text);
    };
    const inputTargets = (target: Session) => (broadcast ? workspace.columns.flatMap((c) => c.sessions) : [target])
        .filter((session) => session.exitCode === null);
    const send = (targets: Session[], text: string) => {
        const live = new Set(workspace.allColumns.flatMap((column) => column.sessions));
        for (const s of targets) if (live.has(s)) s.paste(text);
    };

    latest.current.onBell = (target: Session) => {
        void target;
        if (settings.bell === 'sound' || settings.bell === 'both') sound.beep('info');
    };
    latest.current.onClipboard = (text: string) => {
        if (!settings.allowClipboardWrite) return;
        copyText(text);
        workspace.toast('A program copied text', 'info', text.length > 60 ? text.slice(0, 60) + '…' : text);
    };
    latest.current.onNotify = (target: Session, title: string, body: string) => {
        notifications.show({ title: title || target.title || 'Terminal', body, onClick: () => { workspace.activate(target); mainWindow.focus(); } });
    };
    const instantReplay = (target: Session) => {
        const cast = target.instantReplay();
        if (!cast) {
            workspace.toast(settings.replaySeconds ? 'No recent output to replay' : 'Instant replay is off', 'info');
            return;
        }
        setReplay(cast);
        setOverlay('replay');
    };
    latest.current.onProgress = (target: Session) => {
        // Whatever the open shell reports is what the taskbar shows.
        const shown = workspace.currentSession ?? target;
        const { state, value } = shown.term.progress;
        try {
            if (state === 0) taskbar.setProgress(0, 'none');
            else if (state === 3) taskbar.setProgress(0, 'indeterminate');
            else taskbar.setProgress(value / 100, state === 2 ? 'error' : state === 4 ? 'paused' : 'normal');
        } catch {
            // No taskbar to show it on.
        }
    };
    latest.current.onFinished = (target: Session, command: ShellCommand) => {
        if (command.startedAt === null || command.endedAt === null) return;
        const seconds = (command.endedAt - command.startedAt) / 1000;
        const seen = windowState.focused && workspace.currentSession === target;
        if (seen) return;
        const ok = command.exit === 0;
        if (settings.notifyAfter > 0 && seconds >= settings.notifyAfter) {
            if (!windowState.focused) {
                notifications.show({
                    title: ok ? 'Command finished' : `Command failed (${command.exit})`,
                    body: `${command.text || 'A command'} · ${seconds.toFixed(seconds < 10 ? 1 : 0)} s`,
                    onClick: () => { workspace.activate(target); mainWindow.focus(); },
                });
                taskbar.requestAttention({ critical: false });
            }
        }
        if (windowState.focused) {
            workspace.toast(ok ? 'Finished' : `Failed (${command.exit})`, ok ? 'ok' : 'bad', command.text);
        }
    };

    const openLink = (target: { text: string; kind: string; file?: { path: string; line: number | null; col: number | null } }, from: Session) => {
        const webLink = /^https?:\/\//i.test(target.text);
        if (target.kind === 'url' || webLink) {
            if (webLink && settingsStore.get().linkBrowser === 'builtIn') {
                workspace.openTool('browser', { url: target.text });
                return;
            }
            void shell.open(target.text).catch(() => workspace.toast('That link can’t be opened from here', 'bad', target.text));
            return;
        }
        const file = target.file;
        if (file && (!from.term.host || from.term.host.toLowerCase() === hostname().toLowerCase())) openFile(file.path, file.line, file.col, from.cwd);
    };

    const openFile = (path: string, line: number | null, col: number | null, cwd: string) => {
        const absolute = /^[A-Za-z]:[\\/]/.test(path) || path.startsWith('\\\\') ? path : `${cwd.replace(/[\\/]$/, '')}\\${path.replace(/^\.[\\/]/, '').replace(/\//g, '\\')}`;
        const editor = settingsStore.get().editor.trim();
        if (editor === '') {
            void shell.open(absolute);
            return;
        }
        const command = editor.replace('{file}', `"${absolute}"`).replace('{line}', String(line ?? 1)).replace('{col}', String(col ?? 1));
        void import('child_process')
            .then(({ exec }) => exec(command, (error: unknown) => error && void shell.open(absolute)))
            .catch(() => shell.open(absolute));
    };

    const showContextMenu = (target: Session, x: number, y: number, link: ReturnType<typeof linkAt>) => {
        const hasSelection = target.selection !== null;
        const finished = target.lastFinished();
        const items: any[] = [];
        if (link) {
            items.push({ label: link.kind === 'url' ? 'Open link' : 'Open in editor', onClick: () => openLink(link, target) });
            items.push({ label: 'Copy ' + (link.kind === 'url' ? 'link' : 'path'), onClick: () => copyText(link.text) });
            items.push({ separator: true });
        }
        items.push({ label: 'Copy', shortcut: menuShortcut('copy'), disabled: !hasSelection, onClick: () => copy(target) });
        items.push({ label: 'Paste', shortcut: menuShortcut('paste'), onClick: () => void paste(target) });
        items.push({ label: 'Select all', onClick: () => target.selectAll() });
        items.push({ separator: true });
        items.push({ label: 'Copy last output', disabled: !finished, onClick: () => finished && copyText(target.outputOf(finished)) });
        items.push({ label: 'Find…', shortcut: menuShortcut('find'), onClick: () => target.openSearch() });
        items.push({ label: 'Hints…', shortcut: menuShortcut('hints'), onClick: () => target.startHints() });
        items.push({ separator: true });
        const routinePane = workspace.spaceOf(target)?.columns.some((column) => column.routine && column.sessions.includes(target));
        if (!routinePane) items.push({ label: 'Split down', onClick: () => { workspace.activate(target); workspace.split(); } });
        items.push({ label: 'Close', onClick: () => workspace.closeSession(target) });
        try {
            menus.popup(items);
        } catch {
            // Nowhere to show one.
        }
        void x;
        void y;
    };

    const showBlockMenu = (target: Session, block: ShellCommand) => {
        const bookmark = target.bookmarks.get(block.id);
        const retained = target.commands.some((command) => command.id === block.id) && block.promptLine >= target.term.evicted;
        const output = () => retained ? target.outputOf(block) : bookmark?.output ?? '';
        const hasOutput = block.outputLine !== null && (block.endLine === null || block.endLine > block.outputLine);
        const canRun = () => target.exitCode === null && target.term.emptyPrompt && !!block.text && !/[\x00-\x1f\x7f-\x9f]/.test(block.text);
        const canSave = block.endLine !== null && singleLine(block.text) && integratedProfile(target.profileId) && windowsFolder(block.cwd);
        const draftKey = `${target.id}:${block.id}`;
        const staged = routineDraft.some(({ key }) => key === draftKey);
        const capture = () => {
            if (staged) return routineDraft;
            const name = singleLine(target.displayName) ? target.displayName : target.profileName;
            const phase = Math.max(0, ...routineDraft.map(({ step }) => step.phase)) + 1;
            const step = { name, command: block.text, profile: target.profileId, cwd: block.cwd, phase, ready: '' };
            return [...routineDraft, { key: draftKey, step }];
        };
        menus.popup([
            { label: 'Jump to command', disabled: !retained, onClick: () => { target.scrollTo(block.promptLine); target.historyOpen = false; target.changed(); } },
            { label: 'Select output', disabled: !retained || !hasOutput, onClick: () => target.selectOutput(block) },
            { label: target.folds.has(block.id) ? 'Expand output' : 'Collapse output', disabled: !retained || !hasOutput || block.endLine === null, onClick: () => target.toggleFold(block) },
            { label: 'Copy command', disabled: !block.text, onClick: () => copyText(block.text) },
            { label: 'Copy output', disabled: !hasOutput && !bookmark?.output, onClick: () => copyText(output()) },
            { label: 'Copy command and output', onClick: () => copyText([block.text, output()].filter(Boolean).join('\n')) },
            { label: bookmark ? 'Remove bookmark' : 'Bookmark command', disabled: !bookmark && block.endLine === null,
                onClick: () => { target.toggleBookmark(block); workspace.scheduleSave(); } },
            { label: 'Rerun in this pane', disabled: !canRun(), onClick: () => {
                if (!canRun()) return;
                target.paste(block.text);
                target.send('\r');
            } },
            { label: staged ? 'Remove from routine draft' : 'Add to routine draft', disabled: !canSave, onClick: () => {
                setRoutineDraft(staged ? routineDraft.filter(({ key }) => key !== draftKey) : capture());
                workspace.toast(staged ? 'Removed from routine draft' : 'Added to routine draft', 'info');
            } },
            { label: 'Save as routine…', disabled: !canSave, onClick: () => {
                const draft = capture();
                setRoutineDraft(draft);
                target.historyOpen = false;
                target.changed();
                setRoutineEditor({ id: '', name: '', steps: draft.map(({ step }) => step) });
                workspace.openTool('routines');
            } },
        ]);
    };

    const actions: PaneActions = {
        openLink,
        paste: (target) => void paste(target),
        copy: (target) => void copy(target),
        contextMenu: showContextMenu,
        focusMe: () => {},
        blockMenu: showBlockMenu,
        broadcast,
        terminalFocus: overlay === 'none',
        committed: (target, text) => {
            for (const s of inputTargets(target)) s.commitText(text);
        },
    };

    // ---- Commands

    const profiles = allProfiles(settings).map((p) => ({ id: p.id, name: p.name }));
    const routines = useMemo(() => [DEMO_ROUTINE, ...savedRoutines], [savedRoutines]);
    const openRoutines = () => {
        workspace.openTool('routines');
    };
    const runRoutine = (routine: Routine) => {
        const session = workspace.current?.kind === 'routines' ? workspace.lastTerminalSession : workspace.currentSession;
        const folder = session?.cwd || workspace.space.folder;
        const runner = new RoutineRunner();
        const tab = workspace.openRoutine(routine.name, runner);
        runner.launch(routine, routineShells(workspace, tab, routine, folder));
        setOverlay('none');
    };
    const persistRoutines = (next: Routine[]) => {
        storageSet('terminal.routines', JSON.stringify(next));
        setSavedRoutines(next);
    };
    const saveRoutine = (routine: Routine) => {
        if (routine.id) {
            persistRoutines(savedRoutines.map((item) => item.id === routine.id ? routine : item));
        } else {
            const id = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
            persistRoutines([...savedRoutines, { ...routine, id }]);
            if (routineEditor?.steps.length) setRoutineDraft([]);
        }
        setRoutineEditor(null);
    };
    const context: Context = {
        workspace,
        settings,
        change: changeSettings,
        session,
        copy: (target) => void copy(target),
        paste: (target) => void paste(target),
        copyText,
        openOverlay: (kind) => (kind === 'palette' ? setOverlay('palette') : workspace.openTool(kind)),
        openTool: (kind, view) => workspace.openTool(kind, view),
        openSpaces: () => {
            setSpaceAnchor({ x: 12, y: 48 });
            setOverlay('spaces');
        },
        editSpace: () => {
            setEditing(workspace.space.id);
            setOverlay('space');
        },
        openSearch: () => session?.openSearch(),
        toggleQuake: () => latest.current.quake?.(),
        leaveQuake: () => latest.current.leaveQuake?.(),
        toggleFullscreen: () => mainWindow.setFullscreen(!windowState.fullscreen),
        toggleRecording: (target) => latest.current.record?.(target),
        toggleTabRecording: () => latest.current.recordTab?.(),
        instantReplay,
        refreshProfiles: () => void discoverProfiles(settingsStore.get()).then(() => workspace.paneChanged()),
        broadcast: { on: broadcast, toggle: () => setBroadcast((on) => !on) },
        profiles,
        setCapturingShortcut,
        routines,
        openRoutines,
        runRoutine,
        hasRoutineRun: !!routineTab,
        showRoutineProgress: () => { if (routineTab) workspace.focusTab(routineTab); },
    };
    const spaceKey = workspace.spaces.map((s) => s.id + ':' + s.name).join(',');
    const commands = useMemo(() => buildCommands(context), [profiles.map((p) => p.id).join(','), spaceKey, routines]);
    const keymap = useMemo(() => indexKeys(commands, settings.keymap), [commands, settings.keymap]);
    const primaryKey = (id: string) => {
        const command = commands.find((entry) => entry.id === id);
        return command && commandKeys(command, settings.keymap).find((key) => keymap.get(key) === command);
    };
    const shortcut = (id: string) => {
        const key = primaryKey(id);
        return key ? keyLabel(key) : undefined;
    };
    const menuShortcut = (id: string) => {
        const key = primaryKey(id);
        return isNativeShortcut(key) ? key : undefined;
    };
    latest.current.context = context;
    latest.current.keymap = keymap;
    latest.current.commands = commands;
    latest.current.overlay = overlay;
    pageEnv.context = context;
    pageEnv.keys = keymap;
    pageEnv.runKey = (key: string) => {
        const command = keymap.get(key);
        if (!command || (command.when && !command.when(context))) return false;
        command.run(context);
        return true;
    };
    latest.current.broadcast = broadcast;
    latest.current.quake = () => void quake.toggle(settings.animations);
    latest.current.leaveQuake = () => quake.leave();
    const captureSource = windowState.visible && !windowState.minimized && !workspace.overview && overlay === 'none' ? workspace.current : null;
    latest.current.captureSource = captureSource;
    const recordTab = async (openResult = true) => {
        const before = recordingState.current;
        if (before !== 'idle' && before !== 'recording') return;
        const update = (state: RecordingState) => {
            recordingState.current = state;
            setTabRecording(state);
        };
        const tab = workspace.current;
        if (before === 'idle' && tab?.kind !== 'terminal') {
            workspace.toast('Open the terminal tab you want to record', 'info');
            return;
        }
        update(before === 'idle' ? 'starting' : 'saving');
        try {
            if (before === 'idle' && tab) {
                if (latest.current.overlay !== 'none') {
                    setOverlay('none');
                    await new Promise((resolve) => setTimeout(resolve, 25));
                }
                const recorder = new TabSessionRecorder(await tabRecordingPath());
                if (!workspace.allColumns.includes(tab) || !tab.sessions.length) {
                    update('idle');
                    return;
                }
                recorder.update(tab, settingsStore.get(), latest.current.captureSource === tab);
                recordingTab.current = { tab, recorder };
                update('recording');
            } else {
                const file = await recordingTab.current?.recorder.stop();
                recordingTab.current = null;
                update('idle');
                if (file && openResult) workspace.openTool('recordings', { recording: file });
                else if (file) workspace.toast('Tab recording saved', 'ok', 'Open Recordings to play it');
            }
        } catch (error) {
            recordingTab.current = null;
            update('idle');
            workspace.toast(before === 'idle' ? 'Tab recording could not start' : 'The tab recording could not be saved', 'bad', String(error));
        }
    };
    latest.current.recordTab = (openResult = true) => {
        if (recordingState.current === 'starting' || recordingState.current === 'saving') return recordingJob.current;
        const job = recordTab(openResult);
        recordingJob.current = job;
        return job;
    };
    latest.current.record = async (target: Session) => {
        if (!target.recording) {
            target.startRecording();
            workspace.toast('Recording shell output', 'info', 'Use the command palette to stop the .cast recording');
            return;
        }
        const recording = target.stopRecording();
        if (!recording || recording.events.length === 0) {
            workspace.toast('Nothing was recorded', 'info');
            return;
        }
        try {
            const file = await saveRecording(recording, target.title || target.profileName);
            if (workspace.currentSession === target) workspace.openTool('recordings', { recording: file });
            else workspace.toast('Recording saved', 'ok', file);
        } catch (error) {
            workspace.toast('The recording couldn’t be saved', 'bad', String(error));
        }
    };

    // ---- Starting, and what follows the settings

    useEffect(() => {
        let cancelled = false;
        const begin = () => {
            if (cancelled) return;
            const restored = settingsStore.get().restoreSession && workspace.restore();
            if (!restored && !workspace.current) workspace.addColumn(undefined, undefined, 'end');
            started.current = true;
        };
        if ((settings.restoreSession && workspace.savedLayoutUsesDiscoveredProfiles()) || /^(ssh|wsl):/.test(settings.profile)) {
            void discoverProfiles(settings).then(begin, begin);
        } else {
            begin();
            void discoverProfiles(settings).then(() => { if (!cancelled) workspace.paneChanged(); }, () => {});
        }
        return () => { cancelled = true; };
    }, []);
    useEffect(() => () => { if (started.current) workspace.save(); }, [workspace]);
    useEffect(() => {
        const active = recordingTab.current;
        if (!active || tabRecording !== 'recording') return;
        if (!workspace.allColumns.includes(active.tab)) void latest.current.recordTab(false);
        else active.recorder.update(active.tab, settingsStore.get(), captureSource === active.tab);
    }, [workspaceVersion, tabRecording, captureSource, settings]);
    useEffect(() => {
        if (tabRecording !== 'recording') return;
        return settingsStore.subscribe(() => {
            const active = recordingTab.current;
            if (active && workspace.allColumns.includes(active.tab)) active.recorder.update(active.tab, settingsStore.get(), latest.current.captureSource === active.tab);
        });
    }, [tabRecording, workspace]);
    useEffect(() => {
        for (const column of workspace.allColumns) for (const s of column.sessions) s.refreshOptions();
    }, [settings.graphics, settings.kittyKeyboard, settings.triggers, settings.paneStatus, settings.replaySeconds]);
    useEffect(() => { if (started.current) workspace.scheduleSave(); }, [settings.restoreLines, settings.restoreSession]);
    useEffect(() => {
        workspace.setTheme(scheme);
    }, [scheme]);
    useEffect(() => {
        try {
            mainWindow.setBackdrop(settings.material);
        } catch {
            // Windows 10 has no materials.
        }
    }, [settings.material]);
    useEffect(() => {
        const column = workspace.current;
        const name = column?.routine?.name ?? (session ? shortTitle(session) : column && column.kind !== 'terminal' ? pageLabel(column).title : '');
        mainWindow.setTitle(name ? `${name} – Terminal` : 'Terminal');
    }, [session?.title, session?.profileName, workspace.current?.id, workspace.current?.view.title]);
    useEffect(() => {
        for (const column of workspace.allColumns) for (const s of column.sessions) s.term.scrollbackLimit = settings.scrollback;
    }, [settings.scrollback]);

    // The dropdown terminal's key, from anywhere.
    useEffect(() => {
        if (!settings.quakeEnabled || capturingShortcut) return;
        try {
            return shortcuts.register(settings.quakeKey, () => latest.current.quake?.());
        } catch {
            workspace.toast(`${settings.quakeKey} is taken by another program`, 'bad', 'Choose another key for the dropdown terminal in settings');
        }
    }, [settings.quakeEnabled, settings.quakeKey, capturingShortcut]);

    // ---- The keyboard

    useEffect(() => {
        const pressed = new Map<string, Session[]>();
        const onKeyDown = (e: KeyboardEvent) => {
            const state = latest.current;
            if (e.isComposing || e.key === 'Process' || e.key === 'Dead') return;
            const key = chord(e);
            const command = state.keymap.get(key);
            if (command?.id === 'record') {
                e.preventDefault();
                if (!e.repeat) state.recordTab?.();
                return;
            }
            if (state.overlay !== 'none') return;
            const target = e.target as HTMLElement | null;
            if ((target?.tagName === 'INPUT' && !target.hasAttribute(terminalInput)) || target?.tagName === 'TEXTAREA') return;
            const ctx: Context = state.context;
            const active: Session | undefined = ctx.workspace.currentSession;
            const recording = recordingTab.current;
            if (active?.exitCode === null && recording && recordingState.current === 'recording' && state.captureSource === recording.tab) {
                recording.recorder.recordKey(e);
            }
            const appKeysFirst = ctx.settings.kittyKeysFirst && (active?.term.kittyFlags ?? 0) !== 0 && command?.id !== 'palette';
            if (active?.hints) {
                e.preventDefault();
                handleHintKey(active, e, ctx, openLink);
                return;
            }
            if (active?.copyMode) {
                e.preventDefault();
                copyKey(active, e, { copy: ctx.copyText });
                return;
            }
            if (!appKeysFirst && command && (!command.when || command.when(ctx))) {
                e.preventDefault();
                command.run(ctx);
                return;
            }
            if (!active) return;
            if (active.historyOpen) {
                if (e.key === 'Escape') {
                    e.preventDefault();
                    active.historyOpen = false;
                    active.changed();
                }
                return;
            }
            if (active.exitCode !== null && e.key === 'Enter') {
                e.preventDefault();
                ctx.workspace.restart(active);
                return;
            }
            if (!appKeysFirst && key === 'ctrl+c' && active.selection) {
                e.preventDefault();
                copy(active);
                active.clearSelection();
                return;
            }
            if (e.key === 'Escape' && active.search.open) {
                active.closeSearch();
                return;
            }
            const targets = state.broadcast ? ctx.workspace.columns.flatMap((c) => c.sessions).filter((session) => session.exitCode === null) : [active];
            const releases = [];
            let handled = false;
            for (const s of targets) if (s.key(e)) {
                handled = true;
                if (s.term.kittyFlags & 2) releases.push(s);
            }
            if (releases.length) pressed.set(e.code || e.key, releases);
            if (handled) e.preventDefault();
            else if (appKeysFirst && command && (!command.when || command.when(ctx))) {
                e.preventDefault();
                command.run(ctx);
            }
        };
        const onKeyUp = (e: KeyboardEvent) => {
            const code = e.code || e.key;
            const targets = pressed.get(code);
            pressed.delete(code);
            if (targets) for (const s of targets) s.key(e);
        };
        const onBlur = () => pressed.clear();
        window.addEventListener('keydown', onKeyDown, true);
        window.addEventListener('keyup', onKeyUp, true);
        window.addEventListener('blur', onBlur);
        return () => {
            window.removeEventListener('keydown', onKeyDown, true);
            window.removeEventListener('keyup', onKeyUp, true);
            window.removeEventListener('blur', onBlur);
        };
    }, []);

    // Ctrl and the wheel changes the size of the text, anywhere.
    useEffect(() => {
        const onWheel = (e: WheelEvent) => {
            if (!e.ctrlKey) return;
            e.preventDefault();
            const s = settingsStore.get();
            changeSettings({ fontSize: Math.max(7, Math.min(40, s.fontSize + (e.deltaY < 0 ? 1 : -1))) });
        };
        window.addEventListener('wheel', onWheel, { passive: false } as any);
        return () => window.removeEventListener('wheel', onWheel);
    }, []);

    const closeOverlay = useCallback(() => {
        setOverlay('none');
        setPasting(null);
        setReplay(null);
    }, []);

    const frameTint = withAlpha(scheme.frame, settings.material === 'none' ? 1 : settings.frameOpacity);
    const profileMenu = () => {
        try {
            menus.popup([
                ...profiles.map((p) => ({ label: p.name, onClick: () => void workspace.addColumn(p.id) })),
                { separator: true },
                ...PAGE_KINDS.map((k) => ({ label: k.label, onClick: () => workspace.openTool(k.kind) })),
            ]);
        } catch {
            // Nowhere to show it.
        }
    };

    return (
        <div
            className="relative h-screen w-screen overflow-hidden"
            data-motion={settings.animations ? 'on' : 'off'}
            style={{ background: frameTint, color: css(scheme.fg), fontFamily: settings.uiFont === 'mono' ? `"${settings.fontFamily}", Consolas, monospace` : undefined }}
        >
            <RoutinesContext.Provider value={{
                routines, draft: routineDraft.map(({ step }) => step), editor: routineEditor,
                profiles: profiles.filter(({ id }) => integratedProfile(id)), defaultProfile: settings.profile, shortcut,
                onRun: runRoutine, onEdit: setRoutineEditor, onSave: saveRoutine,
                onDelete: (routine) => persistRoutines(savedRoutines.filter(({ id }) => id !== routine.id)),
            }}>
                <Chrome
                    workspace={workspace}
                    settings={settings}
                    scheme={scheme}
                    actions={actions}
                    broadcast={broadcast}
                    tabRecording={tabRecording === 'recording' && captureSource !== recordingTab.current?.tab ? 'paused' : tabRecording}
                    version={workspace.version}
                    onClosePane={(target) => workspace.closeSession(target)}
                    handlers={{
                        shortcut,
                        onNew: () => void workspace.addColumn(),
                        onProfiles: profileMenu,
                        onSearch: () => session?.openSearch(),
                        onPalette: () => setOverlay('palette'),
                        onSettings: () => workspace.openTool('settings'),
                        onQuake: () => latest.current.quake?.(),
                        onStudio: () => workspace.openTool('studio'),
                        onRecord: () => latest.current.recordTab?.(),
                        onRecordings: () => workspace.openTool('recordings'),
                        beforeClose: async () => {
                            await recordingJob.current;
                            if (recordingState.current === 'recording') await latest.current.recordTab(false);
                            workspace.save();
                        },
                        onBrowser: () => workspace.openTool('browser'),
                        onRoutines: openRoutines,
                        onMore: () => {
                            try {
                                menus.popup([
                                    ...PAGE_KINDS.map((k) => ({ label: k.kind === 'browser' ? 'New browser tab' : k.label, shortcut: menuShortcut(k.kind), onClick: () => workspace.openTool(k.kind) })),
                                    { separator: true },
                                    { label: 'Overview', shortcut: menuShortcut('overview'), checked: workspace.overview, onClick: () => workspace.toggleOverview() },
                                    { label: 'Workspaces…', shortcut: menuShortcut('spaces'), onClick: () => setOverlay('spaces') },
                                    { label: 'Command palette', shortcut: menuShortcut('palette'), onClick: () => setOverlay('palette') },
                                ]);
                            } catch {
                                // Nowhere to show it.
                            }
                        },
                        onSpaces: (anchor) => {
                            setSpaceAnchor(anchor);
                            setOverlay('spaces');
                        },
                    }}
                />
            </RoutinesContext.Provider>
            {session?.search.open ? <SearchBar session={session} scheme={scheme} onClose={() => session.closeSearch()} /> : null}
            {quakeState !== 'off' ? (
                <button
                    className="absolute bottom-9 left-1/2 z-20 -translate-x-1/2 rounded-full px-3 py-1 text-[11.5px]"
                    style={{ background: withAlpha(scheme.fg, 0.1), color: withAlpha(scheme.fg, 0.7) }}
                    onClick={() => quake.leave()}
                >
                    {settings.quakeEnabled ? `Dropdown mode · ${keyLabel(settings.quakeKey)} hides it · click to leave` : 'Click to leave dropdown mode'}
                </button>
            ) : null}
            {overlay === 'spaces' ? (
                <SpacePopover
                    workspace={workspace}
                    scheme={scheme}
                    anchor={spaceAnchor}
                    shortcut={shortcut}
                    onClose={closeOverlay}
                    onEdit={(space) => {
                        setEditing(space.id);
                        setOverlay('space');
                    }}
                />
            ) : null}
            {overlay === 'space' && workspace.spaces.some((s) => s.id === editing) ? (
                <SpaceEditor key={editing} workspace={workspace} space={workspace.spaces.find((s) => s.id === editing)!} scheme={scheme} onClose={closeOverlay} />
            ) : null}
            {overlay === 'palette' ? <Palette commands={commands} context={context} scheme={scheme} shortcut={shortcut} onClose={closeOverlay} /> : null}
            {overlay === 'paste' && pasting ? (
                <PasteConfirm
                    text={pasting.text}
                    targetCount={pasting.targets.length}
                    scheme={scheme}
                    onCancel={closeOverlay}
                    onPaste={() => {
                        const { targets, text } = pasting;
                        closeOverlay();
                        send(targets, text);
                    }}
                />
            ) : null}
            {overlay === 'replay' && replay ? <ReplayDialog cast={replay} settings={settings} scheme={scheme} close={closeOverlay} /> : null}
            <Toasts toasts={workspace.toasts} scheme={scheme} />
        </div>
    );
}

/** Draws again as a shell changes, if there is one. */
function useSessionVersion(session: Session | undefined) {
    useSyncExternalStore(session ? session.subscribe : noSubscribe, session ? session.getVersion : zero);
}
const noSubscribe = () => () => {};
const zero = () => 0;

function shortTitle(session: Session): string {
    return session.displayName;
}

function ReplayDialog({ cast, settings, scheme, close }: { cast: Cast; settings: ReturnType<typeof useWindowSettings>; scheme: ReturnType<typeof resolveScheme>; close: () => void }) {
    return (
        <Modal label="Instant replay" backdrop={0.55} inset="p-8" onClose={close}
            className="flex h-full w-full max-w-[1100px] flex-col overflow-hidden rounded-xl"
            style={{ background: css(scheme.bg), border: `1px solid ${withAlpha(scheme.fg, 0.18)}` }}>
            <div className="flex h-11 shrink-0 items-center justify-between px-4 text-[13px]">
                <span>Instant replay · {cast.header.title}</span>
                <button type="button" data-dialog-focus className="rounded px-2 py-1 hover:bg-white/10 focus-visible:outline-2" onClick={close}>Back to live</button>
            </div>
            <Player cast={cast} settings={settings} scheme={scheme} theme={themeOf(scheme)} live />
        </Modal>
    );
}

function PasteConfirm(props: { text: string; targetCount: number; scheme: ReturnType<typeof resolveScheme>; onCancel: () => void; onPaste: () => void }) {
    const { scheme } = props;
    const lines = props.text.split(/\r\n|\r|\n/);
    const title = `Paste ${lines.length} lines${props.targetCount > 1 ? ` into ${props.targetCount} panes` : ''}?`;
    return (
        <Modal label={title} onClose={props.onCancel} className="rise w-[520px] max-w-[92%] rounded-2xl p-5"
            style={{ background: css(scheme.bg), border: `1px solid ${withAlpha(scheme.fg, 0.16)}`, color: css(scheme.fg), boxShadow: `0 24px 80px ${withAlpha(0x000000, 0.6)}` }}>
            <h2 className="text-[15px] font-semibold">{title}</h2>
            <div className="mt-1 text-[12.5px]" style={{ color: withAlpha(scheme.fg, 0.6) }}>
                A line break runs what's before it. Check this is what you meant to run.
            </div>
            <pre
                className="mt-3 max-h-48 overflow-auto rounded-lg p-3 text-[12px]"
                style={{ background: withAlpha(scheme.fg, 0.07), fontFamily: 'Cascadia Code, Consolas, monospace' }}
            >
                {lines.slice(0, 12).join('\n') + (lines.length > 12 ? `\n… ${lines.length - 12} more` : '')}
            </pre>
            <div className="mt-4 flex justify-end gap-2">
                <button type="button" data-dialog-focus className="rounded-lg px-4 py-2 text-[13px] focus-visible:outline-2"
                    style={{ background: withAlpha(scheme.fg, 0.1) }} onClick={props.onCancel}>
                    Cancel
                </button>
                <button type="button" className="rounded-lg px-4 py-2 text-[13px] font-semibold focus-visible:outline-2"
                    style={{ background: css(scheme.ansi[4]), color: css(scheme.bg) }} onClick={props.onPaste}>
                    Paste
                </button>
            </div>
        </Modal>
    );
}

/** A letter or two typed while hints are showing: the hint they name is picked up, or opened when it's typed with shift. */
function handleHintKey(session: Session, e: KeyboardEvent, ctx: Context, open: (t: any, s: Session) => void) {
    const state = session.hints!;
    if (e.key === 'Escape') {
        session.stopHints();
        return;
    }
    if (e.key === 'Backspace') {
        state.typed = state.typed.slice(0, -1);
        session.changed();
        return;
    }
    if (e.key.length !== 1 || e.ctrlKey || e.altKey) return;
    const typed = state.typed + e.key.toLowerCase();
    const matching = state.hints.filter((h) => h.label.startsWith(typed));
    if (matching.length === 0) return;
    state.typed = typed;
    const exact = matching.find((h) => h.label === typed);
    if (exact && matching.length === 1) {
        session.stopHints();
        if (e.shiftKey) {
            if (exact.kind === 'url' || exact.kind === 'link' || exact.kind === 'path') open({ text: exact.text, kind: exact.kind === 'path' ? 'path' : 'url', file: exact.file }, session);
            else session.send(exact.text);
        } else {
            ctx.copyText(exact.text);
            ctx.workspace.toast('Copied', 'ok', exact.text);
        }
        return;
    }
    session.changed();
}
