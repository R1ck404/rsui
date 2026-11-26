import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deflateSync } from 'node:zlib';
import { Terminal } from '../src/vt/terminal.ts';
import { encodeCommittedText, encodeKey, needsPasteConfirmation } from '../src/vt/keys.ts';
import { castText, parseCast } from '../src/term/recording.ts';
import { Playback } from '../src/term/timeline.ts';
import { sshConfigEntries, wslDistributions } from '../src/shells.ts';

const csi = (data: string) => '\x1b[' + data;
const kitty = (data: string) => '\x1b_G' + data + '\x1b\\';
const osc = (data: string) => '\x1b]' + data + '\x07';

test('kitty keyboard flags are negotiated and stacked independently per screen', () => {
    const term = new Terminal(40, 8);
    const replies: string[] = [];
    term.onReply = (text) => replies.push(text);
    term.write(csi('>3u') + csi('?u'));
    term.write(csi('?1049h') + csi('>24u') + csi('?u') + csi('?1049l') + csi('?u'));
    term.write(csi('=8;2u') + csi('=1;3u') + csi('?u') + csi('<u') + csi('?u'));
    assert.deepEqual(replies, [3, 24, 3, 10, 0].map((flags) => csi(`?${flags}u`)));
    term.kittyKeyboard = false;
    term.write(csi('>31u') + csi('?u'));
    assert.equal(replies.length, 5);
});

test('kitty encoding distinguishes shortcuts, repeats, physical keys, and committed text', () => {
    const term = new Terminal(40, 8);
    const key = { key: 'i', code: 'KeyI', ctrlKey: true, shiftKey: false, altKey: false };
    term.write(csi('>3u'));
    assert.equal(encodeKey(key, term), csi('105;5u'));
    assert.equal(encodeKey({ ...key, key: 'Tab', ctrlKey: false }, term), '\t');
    assert.equal(encodeKey({ ...key, repeat: true }, term), csi('105;5:2u'));
    assert.equal(encodeKey({ ...key, type: 'keyup' }, term), csi('105;5:3u'));
    term.write(csi('>31u'));
    assert.equal(encodeKey({ ...key, key: 'X', code: 'KeyX', shiftKey: true }, term), csi('120:88;6u'));
    assert.equal(encodeKey({ ...key, key: 'ф', code: 'KeyA', ctrlKey: false }, term), csi('1092::97;1;1092u'));
    assert.equal(encodeCommittedText('한文', term.kittyFlags), csi('0;1;54620:25991u'));
});

test('kitty graphics accepts chunked compressed RGB, places by ID, and deletes placements', () => {
    const term = new Terminal(40, 8);
    const replies: string[] = [];
    term.onReply = (text) => replies.push(text);
    const encoded = deflateSync(Uint8Array.of(255, 0, 0, 0, 255, 0)).toString('base64');
    term.write(kitty('a=t,i=7,f=24,s=2,v=1,o=z,m=1;' + encoded.slice(0, 4)));
    term.write(kitty('m=0;' + encoded.slice(4)));
    term.write(kitty('a=p,i=7,p=2,x=1,w=1,C=1,c=3,r=2,z=-1;'));
    assert.deepEqual(replies, [kitty('i=7;OK'), kitty('i=7,p=2;OK')]);
    assert.equal(term.cursorY, 0);
    assert.equal(term.images[0].viewport?.x, 1);
    assert.equal(term.images[0].cols, 3);
    assert.equal(term.images[0].z, -1);
    assert.equal(Buffer.from(term.images[0].data, 'base64').readUInt32BE(16), 2);
    term.write(csi('?1049h'));
    assert.equal(term.images.length, 0);
    term.write(csi('?1049l') + kitty('a=d,d=i,i=7,p=2;'));
    assert.equal(term.images.length, 0);
});

test('sixel draws pixels and respects its display mode and protocol setting', () => {
    const term = new Terminal(40, 8);
    term.write(csi('3;4H') + csi('?80h') + '\x1bP0;1q"1;1;2;6#1;2;100;0;0!2~\x1b\\');
    const [picture] = term.images;
    assert.equal(picture.line, 0);
    assert.equal(Buffer.from(picture.data, 'base64').readUInt32BE(20), 6);
    assert.equal(term.cursorY, 2);
    term.graphics = 'off';
    term.write('\x1bPq~\x1b\\');
    assert.equal(term.images.length, 1);
});

test('prompt context and multipart notification payloads follow the shell and OSC marks', () => {
    const term = new Terminal(40, 8);
    let notice: string[] = [];
    term.onNotify = (...args) => { notice = args; };
    assert.equal(term.atPrompt, null);
    term.write(osc('133;A') + '$ ' + osc('133;B'));
    assert.equal(term.atPrompt, true);
    term.write(osc('133;C'));
    assert.equal(term.atPrompt, false);
    term.write(osc('99;i=job:d=0:p=title;Waiting') + osc('99;i=job:p=body;Continue?'));
    assert.deepEqual(notice, ['Waiting', 'Continue?']);
});

test('multiline paste checks every recipient and keeps asking for shells with unknown context', () => {
    const editor = { bracketedPaste: false, atPrompt: false };
    const unknown = { bracketedPaste: false, atPrompt: null };
    const safePrompt = { bracketedPaste: true, atPrompt: true };
    assert.equal(needsPasteConfirmation('first\nsecond', [editor, safePrompt], true), false);
    assert.equal(needsPasteConfirmation('first\r', [editor, unknown, safePrompt], true), true);
    assert.equal(needsPasteConfirmation('first\nsecond', [editor], false), true);
    assert.equal(needsPasteConfirmation('first', [unknown], false), false);
});

test('backward replay preserves resize events, modes and parser state across checkpoints', () => {
    const events = Array.from({ length: 720 }, (_, index) => ({ t: index * 10, data: `\r\nline ${index}`, kind: 'o' as const }));
    events[127].data = '\x1b[38;2;10;';
    events[128].data = '20;30mcolor';
    const header = { version: 2 as const, width: 40, height: 8, timestamp: 0 };
    const cast = parseCast(castText(header, [...events.slice(0, 355), { t: 3541, data: '60x12', kind: 'r' }, ...events.slice(355)]))!;
    const replay = new Playback(cast);
    replay.seek(7190);
    assert.equal(replay.term.cols, 60);
    replay.seek(1280);
    const fresh = new Playback(cast);
    fresh.seek(1280);
    assert.equal(replay.term.cols, 40);
    assert.deepEqual(replay.term.transcript(30, 2048), fresh.term.transcript(30, 2048));
    const row = replay.term.lineAt(replay.term.scrollbackLength + replay.term.cursorY);
    assert.equal(replay.term.styles.get(row.st[replay.term.cursorX - 1]).fg, 0x020a141e);
});

test('restored transcripts are plain text, bounded by scrollback and independent of live shell modes', () => {
    const term = new Terminal(8, 4, 2);
    term.restoreTranscript(['old', 'recent', '\x1b[?2004h中']);
    assert.equal(term.scrollbackLength, 2);
    assert.deepEqual(term.transcript(5, 100), ['[?2004h', '中']);
    assert.equal(term.bracketedPaste, false);
});

test('discovered profiles use concrete SSH aliases and UTF-16 WSL names', () => {
    assert.deepEqual(sshConfigEntries('Host one TWO *.test !local # note\n Host=one work-dev\nHost -x "lab_1"\nInclude conf.d/*.conf "work config"'), {
        hosts: ['one', 'TWO', 'work-dev', 'lab_1'], includes: ['conf.d/*.conf', 'work config'],
    });
    assert.deepEqual(wslDistributions(Buffer.from('\ufeffUbuntu\r\nDebian Work\r\nUbuntu\r\n', 'utf16le')), ['Ubuntu', 'Debian Work']);
});
