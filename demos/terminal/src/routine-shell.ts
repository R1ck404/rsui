import { allProfiles } from './launch';
import { integratedProfile, routineCommand, windowsFolder } from './routines';
import type { Routine, RoutineEvents, RoutineStep } from './routines';
import { settingsStore } from './settings';
import type { Line } from './vt/terminal';
import type { Column, Workspace } from './workspace';

const normalFolder = (folder: string) => folder.replace(/\//g, '\\').replace(/\\+$/, '').toLowerCase();

export function routineShells(workspace: Workspace, column: Column, routine: Routine, currentFolder: string) {
    const launchFolder = windowsFolder(currentFolder) ? currentFolder : undefined;

    return (step: RoutineStep, attempt: number, events: RoutineEvents) => {
        if (!workspace.allColumns.includes(column)) throw new Error('The routine tab was closed');
        if (!integratedProfile(step.profile) || !allProfiles(settingsStore.get()).some(({ id }) => id === step.profile)) {
            throw new Error('This step needs its saved shell profile to be installed');
        }
        const cwd = step.cwd || launchFolder;
        const label = attempt === 1 ? step.name : `${step.name} · attempt ${attempt}`;
        const session = workspace.spawn(step.profile, cwd, { label });
        column.sessions.push(session);
        workspace.paneChanged();
        const term = session.term;
        const text = routineCommand(routine, step, attempt);
        let armed = false;
        let submitted: number | undefined;
        let outputLine: number | null = null;
        let sawReady = false;
        let wrap = '';
        let wrappedLine = -1;
        let timer: ReturnType<typeof setTimeout> | undefined;

        const stopTimer = () => {
            clearTimeout(timer);
            timer = undefined;
        };
        const fail = (message: string) => {
            session.requestAttention(message);
            events.failed(message);
        };
        const line = (line: Line, number: number, partial: boolean) => {
            if (sawReady || outputLine === null || number < outputLine) return;
            const prior = wrappedLine === number - 1 ? wrap : '';
            const lastCell = line.wrapped && line.ch[line.length - 1] === 32 ? ' ' : '';
            const printed = prior + term.textOf(line) + lastCell;
            if (!partial && line.wrapped) {
                wrap = step.ready.length > 1 ? printed.slice(1 - step.ready.length) : '';
                wrappedLine = number;
            }
            if (printed.includes(step.ready)) {
                sawReady = true;
                events.ready();
            }
        };
        const update = () => {
            if (session.exitCode !== null) {
                fail(session.startError ?? `Shell exited with code ${session.exitCode}`);
                return;
            }
            if (!armed || submitted !== undefined) return;
            if (session.hasInput) {
                fail('This pane received input before its routine step could start');
                return;
            }
            if (!term.emptyPrompt) return;
            if (cwd && normalFolder(session.cwd) !== normalFolder(cwd)) {
                fail(`Shell opened in ${session.cwd || 'an unknown folder'} instead of ${cwd}`);
                return;
            }
            submitted = term.commands.at(-1)?.id;
            try {
                session.paste(text);
                session.send('\r');
            } catch (error) {
                fail(error instanceof Error ? error.message : String(error));
            }
        };
        const unobserve = session.observeShell({
            updated: update,
            closed: () => fail('This pane was closed'),
            line: step.ready ? line : undefined,
            command: (command, change) => {
                if (change === 'output') {
                    if (command.id !== submitted) {
                        fail('Another command started in this pane');
                        return;
                    }
                    outputLine = command.outputLine;
                    stopTimer();
                    events.started();
                    return;
                }
                if (command.id !== submitted) return;
                if (step.ready) {
                    const number = term.evicted + term.scrollbackLength + term.cursorY;
                    const current = term.lineNumbered(number);
                    if (current) line(current, number, true);
                }
                if (command.exit !== 0) session.requestAttention(`Command exited with code ${command.exit ?? 'unknown'}`);
                else if (step.ready && !sawReady) session.requestAttention('Command ended before printing its ready text');
                events.finished(command.exit);
            },
        });
        update();

        return {
            sessionId: session.id,
            start: () => {
                armed = true;
                timer = setTimeout(() => fail(submitted === undefined ? 'No empty shell prompt appeared' : 'The shell did not mark the command as started'), 15000);
                update();
            },
            detach: () => { stopTimer(); unobserve(); },
            focus: () => { workspace.activate(session); session.acknowledge(); },
        };
    };
}
