// Run with `node --test test/` from the demo's folder (Node 24 reads TypeScript as it is).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { rgbColor } from '../src/vt/style.ts';
import { Terminal, WIDE_TAIL } from '../src/vt/terminal.ts';

const text = (t: Terminal, y: number) => t.textOf(t.lineAt(t.scrollbackLength + y));
const screen = (t: Terminal) => Array.from({ length: t.rows }, (_, y) => text(t, y));

test('text goes in, wraps, and scrolls into the scrollback', () => {
    const t = new Terminal(10, 3);
    t.write('abcdefghijKLM\r\nsecond\r\nthird\r\nfourth');
    assert.deepEqual(screen(t), ['second', 'third', 'fourth']);
    assert.equal(t.scrollbackLength, 2);
    assert.equal(t.textOf(t.lineAt(0)), 'abcdefghij');
    assert.equal(t.lineAt(0).wrapped, true);
    assert.equal(t.lineAt(1).wrapped, false);
    assert.equal(t.textOf(t.lineAt(1)), 'KLM');
    assert.equal(t.cursorY, 2);
});

test('compact scrollback preserves character and style values across copies and recordings', () => {
    const t = new Terminal(20, 1, 4);
    for (let i = 0; i < 260; i++) t.styles.intern(rgbColor(i >> 8, i & 255, 3), 0, 0, 0, 0);
    t.write('caf\u00e9\r\n\u03a9\r\n\x1b[38;2;1;3;3m\u{1f600}e\u0301\x1b[0m\r\n');

    for (const term of [t, t.fork(), Terminal.fromSnapshot(t.snapshot())]) {
        assert.equal(term.scrollbackLength, 3);
        const lines = [term.lineAt(0), term.lineAt(1), term.lineAt(2)];
        assert.deepEqual(lines.map((line) => term.textOf(line)), ['caf\u00e9', '\u03a9', '\u{1f600}e\u0301']);
        assert.deepEqual(lines.map((line) => line.ch.BYTES_PER_ELEMENT), [1, 2, 4]);
        assert.equal(lines[2].ch[1], WIDE_TAIL);
        assert.equal(lines[2].st[0], 260);
        assert.equal(term.styles.get(lines[2].st[0]).fg, rgbColor(1, 3, 3));
    }
});

test('the cursor moves, and erase and insert change a line', () => {
    const t = new Terminal(20, 5);
    t.write('hello world\x1b[1;7H\x1b[KX');
    assert.equal(text(t, 0), 'hello X');
    t.write('\x1b[3;3Hab\x1b[2D\x1b[@Z');
    assert.equal(text(t, 2), '  Zab');
    t.write('\x1b[2J\x1b[H');
    assert.deepEqual(screen(t), ['', '', '', '', '']);
    assert.equal(t.cursorX, 0);
});

test('colors: palette, 256, true color, colon form, and reset', () => {
    const t = new Terminal(10, 2);
    t.write('\x1b[31ma\x1b[38;5;200mb\x1b[38;2;1;2;3mc\x1b[48:2::9:8:7md\x1b[0me');
    const line = t.lineAt(0);
    const style = (x: number) => t.styles.get(line.st[x]);
    assert.equal(style(0).fg, 0x01000001);
    assert.equal(style(1).fg, 0x01000000 | 200);
    assert.equal(style(2).fg, 0x02010203);
    assert.equal(style(3).bg, 0x02090807);
    assert.equal(style(4).fg, 0);
    assert.equal(style(4).bg, 0);
});

test('bold, underline styles and bright colors', () => {
    const t = new Terminal(10, 2);
    t.write('\x1b[1;4:3;93mx');
    const s = t.styles.get(t.lineAt(0).st[0]);
    assert.equal(s.flags & 1, 1);
    assert.equal((s.flags >> 8) & 7, 3);
    assert.equal(s.fg, 0x01000000 | 11);
});

test('wide characters take two cells and are never cut in half', () => {
    const t = new Terminal(6, 2);
    t.write('a漢b');
    assert.equal(t.textOf(t.lineAt(0)), 'a漢b');
    assert.equal(t.cursorX, 4);
    t.write('\x1b[1;3Hx');
    // Writing over the second half leaves the first a blank.
    assert.equal(t.textOf(t.lineAt(0)), 'a xb');
    t.write('\x1b[2;1H12345漢');
    assert.equal(t.textOf(t.lineAt(1)), '12345');
    assert.equal(t.cursorY, 1);
});

test('combining marks stay with their letter', () => {
    const t = new Terminal(10, 2);
    t.write('e\u0301x');
    assert.equal(t.textOf(t.lineAt(0)), 'e\u0301x');
    assert.equal(t.cursorX, 2);
});

test('scrolling regions scroll only inside, and reverse index scrolls down', () => {
    const t = new Terminal(10, 5);
    t.write('1\r\n2\r\n3\r\n4\r\n5');
    t.write('\x1b[2;4r\x1b[4;1H\nX');
    assert.deepEqual(screen(t), ['1', '3', '4', 'X', '5']);
    t.write('\x1b[2;1H\x1bM');
    assert.deepEqual(screen(t), ['1', '', '3', '4', '5']);
    assert.equal(t.scrollbackLength, 0);
});

test('the alternate screen comes back as it was', () => {
    const t = new Terminal(10, 3);
    t.write('main text\x1b[?1049h');
    assert.equal(t.alternate, true);
    t.write('\x1b[2JALT');
    assert.equal(text(t, 0), 'ALT');
    t.write('\x1b[?1049l');
    assert.equal(t.alternate, false);
    assert.equal(text(t, 0), 'main text');
    assert.equal(t.cursorX, 9);
});

test('an unused alternate screen survives resize, fork and recording restore', () => {
    const t = new Terminal(4, 2);
    t.write('main\x1b[42m');
    t.resize(9, 3);
    const fork = t.fork();
    const recording = t.snapshot();
    assert.deepEqual(recording.alt.lines.map(line => line.size), [9, 9, 9]);
    for (const term of [fork, Terminal.fromSnapshot(recording), t]) {
        term.write('\x1b[?47h');
        assert.deepEqual(screen(term), ['', '', '']);
        assert.deepEqual([...term.lineAt(0).st], Array(9).fill(0));
        term.write('\x1b[3;8HX\x1b[?47l');
        assert.equal(text(term, 0), 'main');
        term.write('\x1b[?47h');
        assert.equal(text(term, 2), '       X');
        term.write('\x1b[?47l\x1b[?1049h');
        assert.notEqual(term.lineAt(0).st[0], 0);
        assert.equal(text(term, 2).trim(), '');
    }
});

test('modes: bracketed paste, application cursor keys, mouse, focus, cursor', () => {
    const t = new Terminal(10, 3);
    t.write('\x1b[?2004h\x1b[?1h\x1b[?1002h\x1b[?1006h\x1b[?1004h\x1b[?25l\x1b[5 q');
    assert.equal(t.bracketedPaste, true);
    assert.equal(t.appCursor, true);
    assert.equal(t.pointer, 'button');
    assert.equal(t.pointerEncoding, 'sgr');
    assert.equal(t.focusEvents, true);
    assert.equal(t.cursorVisible, false);
    assert.equal(t.cursorShape, 5);
    t.write('\x1b[?2004l\x1b[?1l\x1b[?1002l');
    assert.equal(t.bracketedPaste, false);
    assert.equal(t.pointer, 'none');
});

test('what a program asks is answered', () => {
    const t = new Terminal(80, 24);
    const said: string[] = [];
    t.onReply = (s) => said.push(s);
    t.write('\x1b[5;9H\x1b[6n\x1b[c\x1b[5n\x1b[18t\x1b[?2004$p\x1b[>q');
    assert.equal(said[0], '\x1b[5;9R');
    assert.match(said[1], /^\x1b\[\?62;/);
    assert.equal(said[2], '\x1b[0n');
    assert.equal(said[3], '\x1b[8;24;80t');
    assert.equal(said[4], '\x1b[?2004;2$y');
    assert.match(said[5], /rsui terminal/);
    t.write('\x1b]11;?\x07');
    assert.equal(said[6], '\x1b]11;rgb:0c0c/0c0c/0c0c\x1b\\');
});

test('titles, the working folder, hyperlinks, the clipboard and notifications', () => {
    const t = new Terminal(30, 3);
    let title = '';
    let clip = '';
    let note = '';
    t.onTitle = (s) => (title = s);
    t.onClipboard = (s) => (clip = s);
    t.onNotify = (a, b) => (note = `${a}|${b}`);
    t.write('\x1b]0;my title\x07\x1b]7;file://PC/C:/Users/me/a%20b\x1b\\');
    assert.equal(title, 'my title');
    assert.equal(t.cwd, 'C:/Users/me/a b');
    t.write('\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ plain');
    const line = t.lineAt(0);
    assert.equal(t.link(t.styles.get(line.st[0]).link), 'https://example.com');
    assert.equal(t.styles.get(line.st[6]).link, 0);
    t.write(`\x1b]52;c;${btoa('copied')}\x07`);
    assert.equal(clip, 'copied');
    t.write('\x1b]777;notify;Build;done\x07');
    assert.equal(note, 'Build|done');
});

test('progress as the taskbar shows it', () => {
    const t = new Terminal(10, 2);
    t.write('\x1b]9;4;1;42\x07');
    assert.deepEqual(t.progress, { state: 1, value: 42 });
    t.write('\x1b]9;4;0;0\x07');
    assert.equal(t.progress.state, 0);
});

test('shell integration lists commands with their text, output and exit code', () => {
    const t = new Terminal(40, 6);
    t.write('\x1b]133;A\x07PS> \x1b]133;B\x07ls -l\x1b]133;C\x07\r\nfile1\r\nfile2\r\n\x1b]133;D;0\x07');
    t.write('\x1b]133;A\x07PS> \x1b]133;B\x07bad\x1b]133;C\x07\r\nboom\r\n\x1b]133;D;3\x07\x1b]133;A\x07PS> ');
    assert.equal(t.commands.length, 3);
    const [a, b, c] = t.commands;
    assert.equal(a.text, 'ls -l');
    assert.equal(a.exit, 0);
    assert.equal(a.outputLine, a.promptLine + 1);
    assert.equal(a.endLine, a.promptLine + 3);
    assert.equal(b.text, 'bad');
    assert.equal(b.exit, 3);
    assert.equal(c.exit, null);
});

test('command numbers last while the scrollback is trimmed', () => {
    const t = new Terminal(10, 3, 5);
    t.write('\x1b]133;A\x07x\x1b]133;B\x07\x1b]133;C\x07\r\n\x1b]133;D;0\x07');
    const first = t.commands[0];
    assert.equal(t.lineNumbered(first.promptLine) !== null, true);
    for (let i = 0; i < 20; i++) t.write(`line ${i}\r\n`);
    assert.equal(t.commands.length, 0);
    assert.ok(t.evicted > 0);
    assert.equal(t.lineNumbered(first.promptLine), null);
});

test('the line drawing set', () => {
    const t = new Terminal(10, 2);
    t.write('\x1b(0lqk\x1b(B');
    assert.equal(t.textOf(t.lineAt(0)), '┌─┐');
});

test('repeating, tabs and backspace', () => {
    const t = new Terminal(20, 2);
    t.write('a\x1b[3b\tb\x08c');
    assert.equal(text(t, 0), 'aaaa    c');
});

test('resizing keeps what is there', () => {
    const t = new Terminal(10, 4);
    t.write('one\r\ntwo\r\nthree');
    t.resize(6, 2);
    assert.deepEqual(screen(t), ['two', 'three']);
    assert.equal(t.scrollbackLength, 1);
    t.resize(12, 5);
    assert.equal(t.rows, 5);
    assert.equal(text(t, 0), 'two');
    assert.equal(text(t, 1), 'three');
});

test('a picture from iTerm2 takes its cells and moves the cursor below it', () => {
    // 1x1 PNG
    const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==';
    const t = new Terminal(20, 10);
    t.cellPx = { w: 10, h: 20 };
    let got = 0;
    t.onImage = () => got++;
    t.write(`\x1b]1337;File=inline=1;width=4;height=2:${png}\x07`);
    assert.equal(got, 1);
    assert.equal(t.images[0].cols, 4);
    assert.equal(t.images[0].rows, 2);
    assert.equal(t.cursorY, 2);
});

test('output cut anywhere in a sequence still parses', () => {
    const t = new Terminal(20, 2);
    const whole = '\x1b[1;31mred\x1b[0m \x1b]0;t\x07 ok';
    for (const ch of whole) t.write(ch);
    assert.equal(text(t, 0), 'red  ok');
    assert.equal(t.title, 't');
    assert.equal(t.styles.get(t.lineAt(0).st[0]).fg, 0x01000001);
});

test('a flood of output is quick', () => {
    const t = new Terminal(120, 40);
    const chunk = ('x'.repeat(100) + '\r\n').repeat(500);
    const start = performance.now();
    for (let i = 0; i < 40; i++) t.write(chunk);
    const ms = performance.now() - start;
    // 2 MB of text.
    assert.ok(ms < 1500, `took ${ms} ms`);
    assert.equal(t.scrollbackLength, 10000);
});
