import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rmdir, stat, unlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { loadTabFilm, readTabFrame, tabFrameAt } from '../src/term/tab-recording.ts';
import { loadTabSession, saveTabSession, stateAt, TabSessionRecorder } from '../src/term/tab-session.ts';
import { Playback } from '../src/term/timeline.ts';
import { cropEdge, fitCrop, FULL_FRAME, moveCrop, normalizeCrop, trimTime } from '../src/term/recording-edits.ts';
import { filterKeystrokes, KEYSTROKE_CATEGORIES, keystrokeMotion, nextKeyIndex, recordingKey, visibleKeystrokes } from '../src/term/recording-keys.ts';
import type { KeyInfo } from '../src/vt/keys.ts';
import { Terminal, WIDE_TAIL } from '../src/vt/terminal.ts';
import type { Settings } from '../src/settings.ts';
import type { Session } from '../src/term/Session.ts';
import type { Column } from '../src/workspace.ts';

async function withRecording(run: (path: string) => Promise<void>) {
    const folder = await mkdtemp(join(tmpdir(), 'rsui-tabrec-test-'));
    const file = join(folder, 'check.tabrec');
    try {
        await run(file);
    } finally {
        await unlink(file).catch(() => {});
        await rmdir(folder);
    }
}

const key = (key: string, modifiers: Partial<KeyInfo> = {}): KeyInfo => ({ key, ctrlKey: false, shiftKey: false, altKey: false, ...modifiers });

test('existing PNG recordings remain seekable', () => withRecording(async (file) => {
    const frames = [{ at: 0, offset: 0, bytes: 6, background: 0x112233 }, { at: 40, offset: 6, bytes: 6, background: 0x334455 }];
    const index = Buffer.from(JSON.stringify({ duration: 45, frames }));
    const trailer = Buffer.alloc(8);
    trailer.writeUInt32BE(index.length);
    trailer.writeUInt32BE(0x52535431, 4);
    await writeFile(file, Buffer.concat([Buffer.from('112233334455'), index, trailer]));
    const film = loadTabFilm(file);
    assert.ok(film);
    assert.equal(loadTabSession(file), null);
    assert.equal(tabFrameAt(film, 0), 0);
    assert.equal(tabFrameAt(film, film.duration), 1);
    assert.equal(Buffer.from(readTabFrame(film, 1).split(',')[1], 'base64').toString(), '334455');
}));

test('compact tab recordings preserve positions, a partial escape, colors, and settings on rewind', () => withRecording(async (file) => {
    const term = new Terminal(30, 8, 256);
    const session = { id: 1, displayName: 'Shell', term, scrolledTop: null, selection: null, folds: new Set(),
        bookmarks: new Map(), subscribe: () => () => {} } as unknown as Session;
    const settings = { scheme: 'graphite', cursorShape: 'bar', background: 'dither', fontSize: 15 } as Settings;
    const tab = { sessions: [session], active: 0 } as Column;
    term.write('start\x1b[4;9');
    const recorder = new TabSessionRecorder(file);
    recorder.update(tab, settings, true);
    recorder.recordKey(key('c', { ctrlKey: true }));
    recorder.recordKey(key('a'));
    recorder.update(tab, settings, false);
    recorder.recordKey(key('F2'));
    term.write('H\x1b[31m界\x1b[0m');
    await new Promise((resolve) => setTimeout(resolve, 20));
    term.write('\x1b[2;1Hhidden');
    recorder.update(tab, settings, true);
    await new Promise((resolve) => setTimeout(resolve, 20));
    term.cellPx = { w: 11, h: 21 };
    term.resize(36, 9);
    term.write('\x1b[?1049h\x1b[2;3Hfullscreen');
    recorder.update(tab, { ...settings, scheme: 'nord' }, true);
    await new Promise((resolve) => setTimeout(resolve, 20));
    term.write('\x1b[?1049l');
    const second = { ...session, id: 2, term: new Terminal(18, 9, 256) } as unknown as Session;
    tab.sessions.push(second);
    tab.paneLayout = { axis: 'x', children: [0, 1] };
    recorder.update(tab, { ...settings, scheme: 'nord' }, true);
    for (let i = 0; i < 320; i++) term.write('\r\n' + 'repeated output '.repeat(12));
    await recorder.stop();
    const data = loadTabSession(file);
    assert.ok(data);
    assert.equal(loadTabFilm(file), null);
    assert.deepEqual(data.keystrokes?.map(({ keys }) => keys), [['Ctrl', 'C'], ['a']]);
    assert.ok((await stat(file)).size < 8 * 1024);
    const track = data.panes[0];
    const hidden = track.events.find((event) => event.data.includes('hidden'));
    assert.equal(hidden?.t, track.events[0].t);
    assert.ok(track.events.find((event) => event.kind === 'r')!.t > track.events[0].t);
    assert.deepEqual(stateAt(data.layout, data.duration).panes[1], { id: 2, rect: { x: 0.5, y: 0, w: 0.5, h: 1 } });
    assert.equal(stateAt(data.styles, 0).cursorShape, 'bar');
    assert.equal(stateAt(data.styles, data.duration).scheme, 'nord');
    const initial = Terminal.fromSnapshot(track.initial);
    const playback = new Playback({ initial, events: track.events, duration: data.duration / 1000,
        header: { version: 2, width: 30, height: 8, timestamp: 0 } });
    playback.seek(data.duration);
    assert.equal(playback.term.alternate, false);
    assert.equal(playback.term.cols, 36);
    const alternate = track.events.find((event) => event.data.includes('1049h'))!;
    playback.seek(alternate.t);
    assert.equal(playback.term.alternate, true);
    assert.deepEqual(playback.term.cellPx, { w: 11, h: 21 });
    assert.equal(playback.term.lineAt(1).ch[2], 'f'.codePointAt(0));
    playback.seek(track.events[0].t);
    const line = playback.term.lineAt(3);
    assert.equal(line.ch[8], '界'.codePointAt(0));
    assert.equal(line.ch[9], WIDE_TAIL);
    assert.equal(playback.term.styles.get(line.st[8]).fg, 0x01000001);
    const crop = fitCrop(1, 16 / 9);
    await saveTabSession(file, { ...data, edits: { scheme: 'nord', fg: 0xabc123, from: track.events[0].t, to: alternate.t, crop,
        showKeystrokes: true, keystrokeCategories: ['special'], keystrokePosition: 'top-center', keystrokeDuration: '2000',
        keystrokeLayout: 'stack', keystrokeLimit: '6', keystrokeAnimation: 'pop' } });
    assert.equal(loadTabSession(file)?.edits?.fg, 0xabc123);
    assert.deepEqual(loadTabSession(file)?.edits?.crop, crop);
    assert.deepEqual(loadTabSession(file)?.edits?.keystrokeCategories, ['special']);
    assert.equal(loadTabSession(file)?.edits?.keystrokePosition, 'top-center');
    assert.equal(loadTabSession(file)?.edits?.keystrokeDuration, '2000');
    assert.equal(loadTabSession(file)?.edits?.keystrokeLayout, 'stack');
    assert.equal(loadTabSession(file)?.edits?.keystrokeLimit, '6');
    assert.equal(loadTabSession(file)?.edits?.keystrokeAnimation, 'pop');
    assert.deepEqual(loadTabSession(file)?.panes[0].events, data.panes[0].events);
}));

test('crop presets fit the source and crop edges stay inside it', () => {
    assert.deepEqual(fitCrop(1, 2), { left: 0.25, top: 0, right: 0.75, bottom: 1 });
    assert.deepEqual(fitCrop(2, 1), { left: 0, top: 0.25, right: 1, bottom: 0.75 });
    assert.deepEqual(normalizeCrop(undefined), FULL_FRAME);
    assert.deepEqual(normalizeCrop({ ...FULL_FRAME, left: Infinity }), FULL_FRAME);
    assert.equal(cropEdge(FULL_FRAME, 'left', 2).left, 0.9);
    assert.equal(cropEdge(FULL_FRAME, 'bottom', -1).bottom, 0.1);
    assert.deepEqual(moveCrop(fitCrop(1, 2), -5, 5), { left: 0, top: 0, right: 0.5, bottom: 1 });
});

test('keystroke recording distinguishes shortcuts, printable keys, and AltGr', () => {
    assert.deepEqual(recordingKey(key('Enter')), ['Enter']);
    assert.deepEqual(recordingKey(key('ArrowLeft', { shiftKey: true })), ['Shift', '←']);
    assert.deepEqual(recordingKey(key('@', { ctrlKey: true, altKey: true, getModifierState: () => true })), ['@']);
    assert.deepEqual(recordingKey(key('A', { shiftKey: true })), ['A']);
    assert.deepEqual(recordingKey(key(' ')), ['Space']);
    assert.equal(recordingKey(key('Shift', { shiftKey: true })), null);
});

test('key navigation and overlays use recording time, including after seeking backwards', () => {
    const events = [0, 100, 200, 300].map((t) => ({ t, keys: ['Enter'] }));
    assert.equal(nextKeyIndex(events, 200), 3);
    assert.deepEqual(visibleKeystrokes(events, 300), events.slice(1));
    assert.deepEqual(visibleKeystrokes(events, 300, 250), events.slice(3));
    assert.deepEqual(visibleKeystrokes(events, 1050), events.slice(1));
    assert.deepEqual(visibleKeystrokes(events, 0), events.slice(0, 1));
    assert.deepEqual(visibleKeystrokes(events, 1400), []);
});

test('key filters keep shortcuts, special keys, and typed characters distinct', () => {
    const labels = [['Ctrl', 'C'], ['Enter'], ['a'], ['Esc'], ['F1'], ['←'], ['Space'], ['Shift', 'Tab']];
    const events = labels.map((keys, index) => ({ t: index * 100, keys }));
    const specials = filterKeystrokes(events, ['special']);
    assert.deepEqual(specials.map(({ keys }) => keys), [['Enter'], ['Esc'], ['F1'], ['←']]);
    assert.deepEqual(filterKeystrokes(events, ['typed']).map(({ keys }) => keys), [['a'], ['Space']]);
    assert.deepEqual(filterKeystrokes(events, ['shortcuts']).map(({ keys }) => keys), [['Ctrl', 'C'], ['Shift', 'Tab']]);
    assert.deepEqual(filterKeystrokes(events, []), []);
    assert.equal(filterKeystrokes(events, KEYSTROKE_CATEGORIES), events);
    assert.deepEqual(visibleKeystrokes(specials, 950, 0, 500), specials.slice(-1));
    assert.deepEqual(visibleKeystrokes(specials, 950, 0, 2000), specials.slice(-3));
});

test('overlapping presses keep their own timers and animation samples recording time', () => {
    const events = ['Enter', 'Esc', 'F1'].map((key, index) => ({ t: index * 50, keys: [key] }));
    assert.deepEqual(visibleKeystrokes(events, 100, 0, 1000, 1), [events[2]]);
    assert.deepEqual(visibleKeystrokes(events, 100, 0, 1000, 6), events);
    assert.deepEqual(visibleKeystrokes(events, 1000, 0, 1000, 6), events.slice(1));
    assert.deepEqual(visibleKeystrokes(events, 1050, 0, 1000, 6), events.slice(2));
    assert.deepEqual(visibleKeystrokes(events, 1100, 0, 1000, 6), []);
    assert.deepEqual(keystrokeMotion('pop', 0, 1000), { opacity: 0.35, transform: 'scale(0.940)' });
    assert.deepEqual(keystrokeMotion('fade', 500, 1000), { opacity: 1, transform: undefined });
    assert.equal(keystrokeMotion('fade', 950, 1000)?.opacity, 0.4);
    assert.equal(keystrokeMotion('none', 0, 1000), undefined);
});

test('trimming keeps at least one frame when the recording is long enough', () => {
    assert.equal(trimTime('from', 900, 0, 500, 1000), 458);
    assert.equal(trimTime('to', 0, 300, 500, 1000), 342);
    assert.equal(trimTime('from', 900, 0, 25, 25), 0);
    assert.equal(trimTime('to', 0, 0, 25, 25), 25);
    assert.equal(trimTime('to', 500, 0, 0, 0), 0);
});
