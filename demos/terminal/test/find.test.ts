import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Terminal } from '../src/vt/terminal.ts';
import { searchLines, spansByLine } from '../src/term/search.ts';
import { findHints } from '../src/term/hints.ts';

function visible(t: Terminal) {
    const out = [];
    for (let n = t.evicted; n < t.evicted + t.lineCount; n++) out.push({ number: n, line: t.lineNumbered(n)! });
    return out;
}

test('search finds text in the scrollback and the screen, in any case', () => {
    const t = new Terminal(20, 3);
    t.write('Alpha beta\r\nbeta gamma\r\nBETA delta\r\nepsilon beta');
    const result = searchLines(t, { query: 'beta', caseSensitive: false, regex: false });
    assert.equal(result.matches.length, 4);
    assert.deepEqual(result.matches.map((m) => [m.line, m.col]), [[0, 6], [1, 0], [2, 0], [3, 8]]);
    const exact = searchLines(t, { query: 'beta', caseSensitive: true, regex: false });
    assert.equal(exact.matches.length, 3);
});

test('search can use a pattern, and says when it is not one', () => {
    const t = new Terminal(30, 2);
    t.write('error 404 and error 500');
    const result = searchLines(t, { query: 'error \\d+', caseSensitive: false, regex: true });
    assert.equal(result.matches.length, 2);
    assert.equal(result.matches[1].col, 14);
    assert.equal(result.matches[1].endCol, 23);
    assert.notEqual(searchLines(t, { query: '(', caseSensitive: false, regex: true }).error, null);
});

test('a match can run over a wrapped line', () => {
    const t = new Terminal(8, 3);
    t.write('abcdefghijkl');
    const result = searchLines(t, { query: 'ghij', caseSensitive: false, regex: false });
    assert.equal(result.matches.length, 1);
    const m = result.matches[0];
    assert.deepEqual([m.line, m.col, m.endLine, m.endCol], [0, 6, 1, 2]);
    const spans = spansByLine(result.matches, 0, 8);
    assert.deepEqual(spans.get(0), [[6, 8, true]]);
    assert.deepEqual(spans.get(1), [[0, 2, true]]);
});

test('search columns count wide characters as two cells', () => {
    const t = new Terminal(20, 2);
    t.write('日本語 abc');
    const result = searchLines(t, { query: 'abc', caseSensitive: false, regex: false });
    assert.equal(result.matches[0].col, 7);
});

test('hints find links, files with a line, hashes and addresses, and give each a letter', () => {
    const t = new Terminal(100, 8);
    t.write('see https://example.com/a?b=1, and src/main.ts:42:7 or C:\\Users\\me\\x.txt\r\n');
    t.write('commit 3f2a9c1d host 192.168.1.20:8080 id 123e4567-e89b-12d3-a456-426614174000\r\n');
    t.write('\x1b]8;;https://linked.example\x1b\\click me\x1b]8;;\x1b\\ done');
    const hints = findHints(t, visible(t));
    const by = (kind: string) => hints.filter((h) => h.kind === kind).map((h) => h.text);
    assert.deepEqual(by('url'), ['https://example.com/a?b=1']);
    assert.ok(by('path').includes('src/main.ts:42:7'));
    assert.ok(by('path').includes('C:\\Users\\me\\x.txt'));
    assert.deepEqual(by('hash'), ['3f2a9c1d']);
    assert.deepEqual(by('ip'), ['192.168.1.20:8080']);
    assert.deepEqual(by('uuid'), ['123e4567-e89b-12d3-a456-426614174000']);
    assert.deepEqual(by('link'), ['https://linked.example']);
    const file = hints.find((h) => h.text === 'src/main.ts:42:7')!;
    assert.deepEqual(file.file, { path: 'src/main.ts', line: 42, col: 7 });
    assert.equal(new Set(hints.map((h) => h.label)).size, hints.length);
    // The nearest the bottom comes first.
    assert.equal(hints[0].kind, 'link');
    assert.equal(hints[0].label, 'a');
});
