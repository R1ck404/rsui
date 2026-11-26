// Everything the terminal can be asked to do, once: each has a name for the palette, the keys that run it, and what it does.
// The keyboard, the command palette and the menus all read this list.

import type { Session } from './term/Session';
import type { Settings } from './settings';
import type { TabKind, TabView, Workspace } from './workspace';
import { SCHEMES } from './themes';
import { testCard } from './testcard';
import type { Keymap } from './keymap';
import type { Routine } from './routines';

export interface Context {
    workspace: Workspace;
    settings: Settings;
    change: (patch: Partial<Settings>) => void;
    session: Session | undefined;
    copy: (session: Session) => void;
    paste: (session: Session) => void;
    copyText: (text: string) => void;
    openOverlay: (kind: 'palette' | 'settings' | 'studio' | 'recordings') => void;
    /** Opens one of the app's pages as a tab, or goes to it if it's open. */
    openTool: (kind: Exclude<TabKind, 'terminal'>, view?: TabView) => void;
    /** Shows the list of workspaces, or the editor of the open one. */
    openSpaces: () => void;
    editSpace: () => void;
    openSearch: () => void;
    toggleQuake: () => void;
    leaveQuake: () => void;
    toggleFullscreen: () => void;
    toggleTabRecording: () => void;
    toggleRecording: (session: Session) => void;
    instantReplay: (session: Session) => void;
    refreshProfiles: () => void;
    broadcast: { on: boolean; toggle: () => void };
    profiles: { id: string; name: string }[];
    setCapturingShortcut: (capturing: boolean) => void;
    routines: Routine[];
    openRoutines: () => void;
    showRoutineProgress: () => void;
    hasRoutineRun: boolean;
    runRoutine: (routine: Routine) => void;
}

export interface Command {
    id: string;
    title: string;
    /** What it's grouped under in the palette. */
    group: string;
    /** Keys that run it, like `ctrl+shift+t`. More than one can. */
    keys?: string[];
    /** Whether it can be used now. */
    when?: (c: Context) => boolean;
    run: (c: Context) => void;
}

const hasSession = (c: Context) => c.session !== undefined;

export function buildCommands(c: Context): Command[] {
    const list: Command[] = [
        { id: 'new', title: 'New terminal', group: 'Terminals', keys: ['ctrl+shift+t'], run: ({ workspace }) => void workspace.addColumn() },
        { id: 'newBefore', title: 'New terminal on the left', group: 'Terminals', run: ({ workspace }) => void workspace.addColumn(undefined, undefined, 'before') },
        { id: 'browser', title: 'New browser tab', group: 'Tabs', keys: ['ctrl+alt+b'], run: ({ openTool }) => openTool('browser') },
        { id: 'back', title: 'Back to the terminal', group: 'Tabs', run: ({ workspace }) => workspace.back() },
        { id: 'newEnd', title: 'New terminal at the end', group: 'Terminals', run: ({ workspace }) => void workspace.addColumn(undefined, undefined, 'end') },
        { id: 'close', title: 'Close this shell or tab', group: 'Terminals', keys: ['ctrl+shift+w'], run: ({ workspace, session }) => (session && !workspace.current?.routine ? workspace.closeSession(session) : workspace.closeColumn()) },
        { id: 'closeColumn', title: 'Close this column', group: 'Terminals', run: ({ workspace }) => workspace.closeColumn() },
        { id: 'next', title: 'Next column', group: 'Terminals', keys: ['ctrl+alt+right', 'ctrl+pagedown', 'ctrl+tab'], run: ({ workspace }) => workspace.moveFocus(1) },
        { id: 'prev', title: 'Previous column', group: 'Terminals', keys: ['ctrl+alt+left', 'ctrl+pageup', 'ctrl+shift+tab'], run: ({ workspace }) => workspace.moveFocus(-1) },
        { id: 'moveRight', title: 'Move column right', group: 'Terminals', keys: ['ctrl+alt+shift+right'], run: ({ workspace }) => workspace.moveColumn(1) },
        { id: 'moveLeft', title: 'Move column left', group: 'Terminals', keys: ['ctrl+alt+shift+left'], run: ({ workspace }) => workspace.moveColumn(-1) },
        { id: 'paneDown', title: 'Focus the shell below', group: 'Terminals', keys: ['ctrl+alt+down'], run: ({ workspace }) => workspace.focusPane(1) },
        { id: 'paneUp', title: 'Focus the shell above', group: 'Terminals', keys: ['ctrl+alt+up'], run: ({ workspace }) => workspace.focusPane(-1) },
        { id: 'split', title: 'Split this column: another shell below', group: 'Terminals', keys: ['ctrl+shift+d'], when: ({ workspace }) => !workspace.current?.routine, run: ({ workspace }) => workspace.split() },
        { id: 'paneZoom', title: 'Zoom or unzoom the active pane', group: 'Terminals', when: ({ workspace }) => !workspace.current?.routine && (workspace.current?.sessions.length ?? 0) > 1, run: ({ workspace }) => workspace.togglePaneZoom() },
        { id: 'refreshProfiles', title: 'Refresh detected SSH and WSL profiles', group: 'Terminals', run: ({ refreshProfiles }) => refreshProfiles() },
        { id: 'overview', title: 'Overview: all columns at once', group: 'Terminals', keys: ['ctrl+shift+o', 'ctrl+alt+o'], run: ({ workspace }) => workspace.toggleOverview() },
        { id: 'width', title: 'Change how wide this column is', group: 'Terminals', keys: ['ctrl+alt+w'], run: ({ workspace }) => workspace.cycleWidth() },
        { id: 'widthFull', title: 'Column: full width', group: 'Terminals', run: ({ workspace }) => workspace.setWidth('full') },
        { id: 'widthWide', title: 'Column: wide, neighbors showing', group: 'Terminals', run: ({ workspace }) => workspace.setWidth('wide') },
        { id: 'widthTwo', title: 'Column: two thirds', group: 'Terminals', run: ({ workspace }) => workspace.setWidth('twothirds') },
        { id: 'widthHalf', title: 'Column: half', group: 'Terminals', run: ({ workspace }) => workspace.setWidth('half') },
        { id: 'widthThird', title: 'Column: a third', group: 'Terminals', run: ({ workspace }) => workspace.setWidth('third') },

        { id: 'palette', title: 'Command palette', group: 'App', keys: ['ctrl+shift+p'], run: ({ openOverlay }) => openOverlay('palette') },
        { id: 'routines', title: 'Routines', group: 'Routines', run: ({ openRoutines }) => openRoutines() },
        { id: 'routineProgress', title: 'Show routine tab', group: 'Routines', when: ({ hasRoutineRun }) => hasRoutineRun, run: ({ showRoutineProgress }) => showRoutineProgress() },
        { id: 'studio', title: 'Background studio: make your own from a picture or video', group: 'Look', run: ({ openOverlay }) => openOverlay('studio') },
        { id: 'settings', title: 'Settings', group: 'App', keys: ['ctrl+,'], run: ({ openOverlay }) => openOverlay('settings') },
        { id: 'prompt', title: 'Customize prompt', group: 'Look', run: ({ openTool }) => openTool('settings', { page: 'prompt' }) },
        { id: 'quake', title: 'Show or hide the dropdown terminal', group: 'App', run: ({ toggleQuake }) => toggleQuake() },
        { id: 'quakeLeave', title: 'Leave dropdown mode', group: 'App', run: ({ leaveQuake }) => leaveQuake() },
        { id: 'testCard', title: 'Show the test card: colors, styles and Unicode', group: 'App', when: hasSession, run: ({ session }) => {
            if (!session) return;
            session.term.write('\r\n' + testCard(session.term.cols));
            session.changed();
        } },
        { id: 'fullscreen', title: 'Full screen', group: 'App', keys: ['f11'], run: ({ toggleFullscreen }) => toggleFullscreen() },

        { id: 'copy', title: 'Copy', group: 'Text', keys: ['ctrl+shift+c', 'ctrl+insert'], when: hasSession, run: ({ session, copy }) => session && copy(session) },
        { id: 'paste', title: 'Paste', group: 'Text', keys: ['ctrl+shift+v', 'shift+insert'], when: hasSession, run: ({ session, paste }) => session && paste(session) },
        { id: 'selectAll', title: 'Select all', group: 'Text', keys: ['ctrl+shift+a'], when: hasSession, run: ({ session }) => session?.selectAll() },
        { id: 'find', title: 'Find in scrollback', group: 'Text', keys: ['ctrl+shift+f'], when: hasSession, run: ({ openSearch }) => openSearch() },
        { id: 'hints', title: 'Hints: pick a link, path or hash from the screen', group: 'Text', keys: ['ctrl+shift+space', 'ctrl+shift+e'], when: hasSession, run: ({ session }) => session?.startHints() },
        { id: 'copyMode', title: 'Copy mode: move and select with the keyboard', group: 'Text', keys: ['ctrl+shift+x'], when: hasSession, run: ({ session }) => session?.startCopyMode() },
        { id: 'copyOutput', title: 'Copy the last command’s output', group: 'Commands', keys: ['ctrl+shift+y'], when: hasSession, run: ({ session, copyText, workspace }) => {
            const command = session?.lastFinished();
            if (session && command) {
                copyText(session.outputOf(command));
                workspace.toast('Copied the output of', 'ok', command.text || 'the last command');
            } else workspace.toast('No finished command to copy', 'info', 'Shell integration marks where output starts and ends');
        } },
        { id: 'selectOutput', title: 'Select the last command’s output', group: 'Commands', keys: ['ctrl+shift+u'], when: hasSession, run: ({ session }) => {
            const command = session?.lastFinished();
            if (session && command) session.selectOutput(command);
        } },
        { id: 'copyCommand', title: 'Copy the last command', group: 'Commands', when: hasSession, run: ({ session, copyText, workspace }) => {
            const command = session?.lastFinished();
            if (command) {
                copyText(command.text);
                workspace.toast('Copied the command', 'ok', command.text);
            }
        } },
        { id: 'promptUp', title: 'Jump to the previous prompt', group: 'Commands', keys: ['ctrl+shift+up'], when: hasSession, run: ({ session }) => session?.jumpPrompt(-1) },
        { id: 'promptDown', title: 'Jump to the next prompt', group: 'Commands', keys: ['ctrl+shift+down'], when: hasSession, run: ({ session }) => session?.jumpPrompt(1) },
        { id: 'scrollTop', title: 'Scroll to the top', group: 'Commands', keys: ['ctrl+shift+home'], when: hasSession, run: ({ session }) => session && session.scrollTo(session.term.evicted) },
        { id: 'scrollBottom', title: 'Scroll to the bottom', group: 'Commands', keys: ['ctrl+shift+end'], when: hasSession, run: ({ session }) => session?.scrollToBottom() },
        { id: 'pageUp', title: 'Scroll a page up', group: 'Commands', keys: ['shift+pageup'], when: hasSession, run: ({ session }) => session?.scrollBy(-(session.term.rows - 1)) },
        { id: 'pageDown', title: 'Scroll a page down', group: 'Commands', keys: ['shift+pagedown'], when: hasSession, run: ({ session }) => session?.scrollBy(session.term.rows - 1) },
        { id: 'clearScrollback', title: 'Clear the scrollback', group: 'Commands', when: hasSession, run: ({ session }) => session?.term.write('\x1b[3J') },
        { id: 'record', title: 'Start or stop recording the terminal tab', group: 'Commands', keys: ['ctrl+shift+r'], run: ({ toggleTabRecording }) => toggleTabRecording() },
        { id: 'recordOutput', title: 'Start or stop recording shell output (.cast)', group: 'Commands', when: hasSession, run: ({ session, toggleRecording }) => session && toggleRecording(session) },
        { id: 'recordings', title: 'Recordings: play back what was recorded', group: 'Commands', run: ({ openOverlay }) => openOverlay('recordings') },
        { id: 'instantReplay', title: 'Instant replay of this pane', group: 'Commands', when: hasSession, run: ({ session, instantReplay }) => session && instantReplay(session) },
        { id: 'commandHistory', title: 'Command history and bookmarks', group: 'Commands', when: hasSession, run: ({ session }) => {
            if (!session) return;
            session.historyOpen = !session.historyOpen;
            session.changed();
        } },
        { id: 'broadcast', title: 'Type into every shell at once', group: 'Commands', keys: ['ctrl+shift+b'], run: ({ broadcast }) => broadcast.toggle() },

        { id: 'zoomIn', title: 'Bigger text', group: 'Look', keys: ['ctrl+=', 'ctrl+plus', 'ctrl+shift+plus'], run: ({ settings, change }) => change({ fontSize: Math.min(40, settings.fontSize + 1) }) },
        { id: 'zoomOut', title: 'Smaller text', group: 'Look', keys: ['ctrl+-'], run: ({ settings, change }) => change({ fontSize: Math.max(7, settings.fontSize - 1) }) },
        { id: 'zoomReset', title: 'Reset the text size', group: 'Look', keys: ['ctrl+0'], run: ({ change }) => change({ fontSize: 14 }) },
        { id: 'ligatures', title: 'Toggle ligatures', group: 'Look', run: ({ settings, change }) => change({ ligatures: !settings.ligatures }) },
        { id: 'smoothCursor', title: 'Toggle the gliding cursor', group: 'Look', run: ({ settings, change }) => change({ smoothCursor: !settings.smoothCursor }) },
        { id: 'cursorTrail', title: 'Toggle the cursor’s trail', group: 'Look', run: ({ settings, change }) => change({ cursorTrail: !settings.cursorTrail }) },
        { id: 'marks', title: 'Toggle the marks beside commands', group: 'Look', run: ({ settings, change }) => change({ showMarks: !settings.showMarks }) },
    ];
    for (const routine of c.routines) {
        list.push({ id: `routine:${routine.id}`, title: `Run routine: ${routine.name}`, group: 'Routines', run: ({ runRoutine }) => runRoutine(routine) });
    }
    for (const scheme of SCHEMES) {
        list.push({ id: 'scheme:' + scheme.id, title: `Color scheme: ${scheme.name}`, group: 'Look', run: ({ change }) => change({ scheme: scheme.id }) });
    }
    const layouts = [['normal', 'Normal'], ['tabs', 'Tabs along the top'], ['sidebar', 'Sidebar'], ['strip', 'Strip of columns'], ['minimal', 'Minimal']] as const;
    for (const [layout, name] of layouts) {
        list.push({ id: 'layout:' + layout, title: `Layout: ${name}`, group: 'Look', run: ({ change }) => change({ layout }) });
    }
    list.push({
        id: 'nextLayout', title: 'Next layout', group: 'Look', keys: ['ctrl+shift+l'],
        run: ({ settings, change }) => change({ layout: layouts[(layouts.findIndex(([l]) => l === settings.layout) + 1) % layouts.length][0] }),
    });
    for (const [background, name] of [['none', 'none'], ['dither', 'dither'], ['stars', 'stars'], ['creature', 'creature'], ['ascii', 'ASCII'], ['halftone', 'halftone'], ['blobs', 'blobs'], ['rain', 'rain']] as const) {
        list.push({ id: 'background:' + background, title: `Background: ${name}`, group: 'Look', run: ({ change }) => change({ background }) });
    }
    for (const material of ['mica', 'acrylic', 'tabbed', 'none'] as const) {
        list.push({ id: 'material:' + material, title: `Window material: ${material === 'none' ? 'solid' : material}`, group: 'Look', run: ({ change }) => change({ material }) });
    }
    for (const profile of c.profiles) {
        list.push({ id: 'profile:' + profile.id, title: `New ${profile.name}`, group: 'Terminals', run: ({ workspace }) => void workspace.addColumn(profile.id) });
        list.push({ id: 'profileBelow:' + profile.id, title: `Split with ${profile.name}`, group: 'Terminals', when: ({ workspace }) => !workspace.current?.routine, run: ({ workspace }) => workspace.split(profile.id) });
    }
    list.push(
        { id: 'spaces', title: 'Workspaces…', group: 'Workspaces', keys: ['ctrl+alt+space'], run: ({ openSpaces }) => openSpaces() },
        { id: 'spaceNew', title: 'New workspace', group: 'Workspaces', keys: ['ctrl+alt+n'], run: ({ workspace, editSpace }) => { workspace.addSpace(); editSpace(); } },
        { id: 'spaceEdit', title: 'Rename or recolor this workspace', group: 'Workspaces', run: ({ editSpace }) => editSpace() },
        { id: 'spaceNext', title: 'Next workspace', group: 'Workspaces', keys: ['ctrl+alt+pagedown'], run: ({ workspace }) => workspace.moveSpace(1) },
        { id: 'spacePrev', title: 'Previous workspace', group: 'Workspaces', keys: ['ctrl+alt+pageup'], run: ({ workspace }) => workspace.moveSpace(-1) },
        { id: 'spaceClose', title: 'Close this workspace and its shells', group: 'Workspaces', when: ({ workspace }) => workspace.spaces.length > 1, run: ({ workspace }) => workspace.closeSpace() },
        { id: 'spaceFolder', title: 'Start this workspace’s shells in this folder', group: 'Workspaces', when: hasSession, run: ({ workspace, session }) => {
            if (!session?.cwd) return;
            workspace.editSpace(workspace.space, { folder: session.cwd });
            workspace.toast('New shells here start in', 'ok', session.cwd);
        } },
    );
    c.workspace.spaces.forEach((space, i) => {
        list.push({ id: 'moveTo:' + space.id, title: `Move this tab to ${space.name}`, group: 'Workspaces', when: ({ workspace }) => workspace.space !== space && workspace.current !== undefined, run: ({ workspace }) => workspace.moveColumnToSpace(space) });
        list.push({ id: 'space:' + (i + 1), title: `Switch to workspace ${i + 1}: ${space.name}`, group: 'Workspaces', keys: i < 9 ? [`ctrl+${i + 1}`] : undefined, run: ({ workspace }) => workspace.switchSpace(i) });
    });
    for (let n = 1; n <= 9; n++) {
        list.push({ id: 'goto' + n, title: `Go to column ${n}`, group: 'Terminals', keys: [`ctrl+alt+${n}`], run: ({ workspace }) => workspace.focusColumn(n - 1) });
    }
    return list;
}

export function commandKeys(command: Command, keymap: Keymap): string[] {
    return (Object.hasOwn(keymap, command.id) ? keymap[command.id] : command.keys) ?? [];
}

/** Explicit bindings take precedence if a newly available command introduces a conflicting default. */
export function indexKeys(commands: Command[], keymap: Keymap): Map<string, Command> {
    const map = new Map<string, Command>();
    for (const command of commands) {
        for (const key of commandKeys(command, keymap)) {
            if (!map.has(key) || Object.hasOwn(keymap, command.id)) map.set(key, command);
        }
    }
    return map;
}

export const GLOBAL_SHORTCUT_ID = 'globalQuake';

export function shortcutConflict(commands: Command[], settings: Settings, id: string, keys: string[]): string | undefined {
    if (id !== GLOBAL_SHORTCUT_ID && keys.includes(settings.quakeKey)) return 'the global dropdown terminal';
    return commands.find((command) => command.id !== id && commandKeys(command, settings.keymap).some((key) => keys.includes(key)))?.title;
}

/** How well a query matches a title: higher is better, and null is no match. Letters have to come in order. */
export function fuzzy(query: string, title: string): number | null {
    if (query === '') return 0;
    const q = query.toLowerCase();
    const t = title.toLowerCase();
    let score = 0;
    let at = 0;
    let previous = -2;
    for (const ch of q) {
        if (ch === ' ') continue;
        const found = t.indexOf(ch, at);
        if (found < 0) return null;
        // Letters together, and at the start of a word, count for more.
        score += found === previous + 1 ? 6 : 1;
        if (found === 0 || t[found - 1] === ' ' || t[found - 1] === ':') score += 5;
        previous = found;
        at = found + 1;
    }
    return score - t.length * 0.05;
}
