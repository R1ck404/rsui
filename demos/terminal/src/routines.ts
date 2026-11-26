export interface RoutineStep {
    name: string;
    command: string;
    profile: string;
    cwd: string;
    phase: number;
    ready: string;
}

export interface Routine {
    id: string;
    name: string;
    steps: RoutineStep[];
}

export const DEMO_ROUTINE: Routine = {
    id: 'local-development',
    name: 'Start local development',
    steps: [
        { name: 'Web', command: "Start-Sleep -Milliseconds 700; Write-Output 'Web ready'; Start-Sleep -Seconds 30", profile: 'powershell', cwd: '', phase: 1, ready: 'Web ready' },
        { name: 'API', command: "Start-Sleep -Milliseconds 1400; throw 'Demo API failed to start. Retry from Routines.'", profile: 'powershell', cwd: '', phase: 1, ready: 'API ready' },
        { name: 'Tests', command: "Write-Output 'Running smoke tests'; Start-Sleep -Milliseconds 900; Write-Output '3 tests passed'", profile: 'powershell', cwd: '', phase: 2, ready: '' },
    ],
};

export function routineCommand(routine: Routine, step: RoutineStep, attempt: number): string {
    if (routine.id === DEMO_ROUTINE.id && step.name === 'API' && attempt > 1) {
        return "Start-Sleep -Milliseconds 700; Write-Output 'API ready'; Start-Sleep -Seconds 30";
    }
    return step.command;
}

export const integratedProfile = (id: string) => id === 'powershell' || id === 'pwsh' || id === 'gitbash';
export const singleLine = (text: string) => !!text.trim() && !/[\x00-\x1f\x7f-\x9f\u2028\u2029]/.test(text);
export const windowsFolder = (path: string) => /^(?:[A-Za-z]:[\\/]|\\\\)/.test(path) && singleLine(path);

export function readRoutines(raw: string | null): Routine[] {
    try {
        const items: unknown = JSON.parse(raw ?? '[]');
        if (!Array.isArray(items)) return [];
        const ids = new Set([DEMO_ROUTINE.id]);
        return items.filter((item): item is Routine => {
            if (!item || typeof item.id !== 'string' || !item.id || ids.has(item.id) || typeof item.name !== 'string' || !singleLine(item.name)) return false;
            if (!Array.isArray(item.steps) || !item.steps.length || !item.steps.every((step: unknown) => {
                if (!step || typeof step !== 'object') return false;
                const { name, command, profile, cwd, phase, ready } = step as RoutineStep;
                return typeof name === 'string' && singleLine(name) && typeof command === 'string' && singleLine(command) &&
                    integratedProfile(profile) && typeof cwd === 'string' && (cwd === '' || windowsFolder(cwd)) &&
                    Number.isSafeInteger(phase) && phase > 0 && typeof ready === 'string' && (ready === '' || singleLine(ready));
            })) return false;
            ids.add(item.id);
            return true;
        });
    } catch {
        return [];
    }
}

type Status = 'waiting' | 'starting' | 'running' | 'ready' | 'passed' | 'failed' | 'stopped';

export interface RoutineShell {
    sessionId?: number;
    start: () => void;
    detach: () => void;
    focus: () => void;
}

export interface RoutineEvents {
    started: () => void;
    ready: () => void;
    finished: (exit: number | null) => void;
    failed: (message: string) => void;
}

export interface StepRun {
    step: RoutineStep;
    attempt: number;
    status: Status;
    detail?: string;
    shell?: RoutineShell;
    previous: { attempt: number; shell: RoutineShell }[];
}

type Prepare = (step: RoutineStep, attempt: number, events: RoutineEvents) => RoutineShell;

interface Run {
    routine: Routine;
    steps: StepRun[];
    stopped: boolean;
    prepare: Prepare;
}

const satisfied = (step: StepRun) => step.status === 'ready' || step.status === 'passed';
const pending = (step: StepRun) => step.status === 'waiting' || step.status === 'starting' || step.status === 'running';

export class RoutineRunner {
    run: Run | null = null;
    #version = 0;
    #listeners = new Set<() => void>();
    #advanceTimer: ReturnType<typeof setTimeout> | null = null;

    subscribe = (listener: () => void) => {
        this.#listeners.add(listener);
        return () => this.#listeners.delete(listener);
    };

    getVersion = () => this.#version;

    get busy() {
        return this.run?.steps.some(pending) ?? false;
    }

    get tabStatus(): 'failed' | 'busy' | null {
        if (this.run?.steps.some(({ status }) => status === 'failed')) return 'failed';
        return this.busy ? 'busy' : null;
    }

    launch(routine: Routine, prepare: Prepare) {
        if (this.busy) return false;
        this.stop();
        const steps: StepRun[] = routine.steps.map((step) => ({ step, attempt: 1, status: 'waiting', previous: [] }));
        const run = this.run = { routine, steps, stopped: false, prepare };
        for (const step of steps) this.#prepare(run, step);
        this.#advance();
        this.#changed();
        return true;
    }

    retry(index: number) {
        const run = this.run;
        const step = run?.steps[index];
        if (!run || run.stopped || step?.status !== 'failed') return;
        const previous = step.shell ? [...step.previous, { attempt: step.attempt, shell: step.shell }] : step.previous;
        const next: StepRun = { step: step.step, attempt: step.attempt + 1, status: 'waiting', previous };
        run.steps[index] = next;
        this.#prepare(run, next);
        next.shell?.focus();
        this.#advance();
        this.#changed();
    }

    stop() {
        if (this.#advanceTimer !== null) clearTimeout(this.#advanceTimer);
        this.#advanceTimer = null;
        if (!this.run || this.run.stopped) return;
        this.run.stopped = true;
        for (const step of this.run.steps) {
            step.shell?.detach();
            if (pending(step)) step.status = 'stopped';
        }
        this.#changed();
    }

    #prepare(run: Run, entry: StepRun) {
        const update = (status: Status, detail?: string) => {
            if (this.run !== run || run.stopped || entry.status === 'failed' || entry.status === 'passed') return;
            entry.status = status;
            entry.detail = detail;
            if (status === 'failed' || status === 'passed') entry.shell?.detach();
            this.#changed();
            // Check again after all events from the current shell output have arrived.
            if (satisfied(entry) && this.#advanceTimer === null) {
                this.#advanceTimer = setTimeout(() => {
                    this.#advanceTimer = null;
                    this.#advance();
                }, 0);
            }
        };
        try {
            entry.shell = run.prepare(entry.step, entry.attempt, {
                started: () => { if (entry.status === 'starting') update('running'); },
                ready: () => { if (entry.status === 'running') update('ready'); },
                finished: (exit) => {
                    if (exit !== 0) update('failed', exit === null ? 'The shell did not report an exit code' : `Command exited with code ${exit}`);
                    else if (entry.step.ready && entry.status !== 'ready') update('failed', 'Command ended before printing its ready text');
                    else update('passed');
                },
                failed: (message) => update('failed', message),
            });
            if (entry.status === 'failed') entry.shell.detach();
        } catch (error) {
            update('failed', error instanceof Error ? error.message : String(error));
        }
    }

    #advance() {
        const run = this.run;
        if (!run || run.stopped) return;
        for (const entry of run.steps) {
            if (entry.status !== 'waiting' || run.steps.some((other) => other.step.phase < entry.step.phase && !satisfied(other))) continue;
            entry.status = 'starting';
            entry.shell?.start();
            this.#changed();
        }
    }

    #changed() {
        this.#version++;
        for (const listener of this.#listeners) listener();
    }
}
