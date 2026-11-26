import assert from 'node:assert/strict';
import { setTimeout as later } from 'node:timers/promises';
import test from 'node:test';
import { DEMO_ROUTINE, readRoutines, RoutineRunner } from '../src/routines';
import type { RoutineEvents } from '../src/routines';

function launch() {
    const runner = new RoutineRunner();
    const started: string[] = [];
    const attempts = new Map<string, RoutineEvents>();
    const detached: string[] = [];
    runner.launch(DEMO_ROUTINE, (step, attempt, events) => {
        const key = `${step.name}:${attempt}`;
        attempts.set(key, events);
        return {
            start: () => { started.push(key); events.started(); },
            detach: () => { detached.push(key); },
            focus: () => {},
        };
    });
    const events = (key: string) => attempts.get(key)!;
    return { runner, started, events, detached };
}

test('parallel steps gate later phases and retry replaces only the failed attempt', async () => {
    const { runner, started, events } = launch();
    assert.deepEqual(started, ['Web:1', 'API:1']);
    events('Web:1').ready();
    events('API:1').finished(1);
    await later(0);
    assert.deepEqual(runner.run?.steps.map(({ status }) => status), ['ready', 'failed', 'waiting']);

    runner.retry(1);
    assert.equal(runner.run?.steps[1].previous[0]?.attempt, 1);
    events('API:1').ready();
    await later(0);
    assert.deepEqual(started, ['Web:1', 'API:1', 'API:2']);
    events('API:2').ready();
    await later(0);
    assert.deepEqual(started, ['Web:1', 'API:1', 'API:2', 'Tests:1']);
    events('Tests:1').finished(0);
    assert.equal(runner.run?.steps[2].status, 'passed');
    runner.stop();
});

test('failure in a readiness batch blocks advancement and stop releases the queued panes', async () => {
    const { runner, started, events, detached } = launch();
    events('Web:1').ready();
    events('API:1').ready();
    events('API:1').finished(2);
    await later(0);
    assert.deepEqual(started, ['Web:1', 'API:1']);
    runner.stop();
    assert.ok(detached.includes('Tests:1'));
    assert.equal(runner.run?.steps[2].status, 'stopped');
    assert.equal(runner.busy, false);
    events('API:1').ready();
    await later(0);
    assert.deepEqual(started, ['Web:1', 'API:1']);
});

test('restored routines reject duplicate IDs, invalid phases and commands with terminal controls', () => {
    const original = { ...DEMO_ROUTINE, id: 'saved' };
    const step = original.steps[0];
    const invalidCommand = { ...original, id: 'control', steps: [{ ...step, command: 'echo one\recho two' }] };
    const invalidPhase = { ...original, id: 'phase', steps: [{ ...step, phase: -1 }] };
    assert.deepEqual(readRoutines(JSON.stringify([original, original, DEMO_ROUTINE, invalidCommand, invalidPhase])), [original]);
});
