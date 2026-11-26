import { test } from 'node:test';
import assert from 'node:assert/strict';
import { encodeKey, encodeMouse, encodePaste } from '../src/vt/keys.ts';

const k = (key: string, mods: Partial<{ ctrlKey: boolean; altKey: boolean; shiftKey: boolean }> = {}) =>
    encodeKey({ key, ctrlKey: false, altKey: false, shiftKey: false, ...mods }, { appCursor: false });

test('keys send what a terminal always has', () => {
    assert.equal(k('a'), 'a');
    assert.equal(k('A', { shiftKey: true }), 'A');
    assert.equal(k('c', { ctrlKey: true }), '\x03');
    assert.equal(k('[', { ctrlKey: true }), '\x1b');
    assert.equal(k('x', { altKey: true }), '\x1bx');
    assert.equal(k('Enter'), '\r');
    assert.equal(k('Backspace'), '\x7f');
    assert.equal(k('Tab', { shiftKey: true }), '\x1b[Z');
    assert.equal(k('ArrowUp'), '\x1b[A');
    assert.equal(k('ArrowLeft', { ctrlKey: true }), '\x1b[1;5D');
    assert.equal(k('Delete'), '\x1b[3~');
    assert.equal(k('PageUp', { shiftKey: true }), '\x1b[5;2~');
    assert.equal(k('F1'), '\x1bOP');
    assert.equal(k('F5'), '\x1b[15~');
    assert.equal(k('F12', { ctrlKey: true }), '\x1b[24;5~');
    assert.equal(k('Shift'), null);
    assert.equal(k('😀'), '😀');
    assert.equal(encodeKey({ key: 'ArrowUp', ctrlKey: false, altKey: false, shiftKey: false }, { appCursor: true }), '\x1bOA');
});

test('paste is wrapped only when asked, and cannot end itself early', () => {
    assert.equal(encodePaste('a\nb', { bracketedPaste: false }), 'a\rb');
    assert.equal(encodePaste('a\x1b[201~b', { bracketedPaste: true }), '\x1b[200~ab\x1b[201~');
});

test('the mouse is reported as asked for', () => {
    const press = { button: 0 as const, kind: 'press' as const, col: 4, row: 2, shiftKey: false, altKey: false, ctrlKey: false };
    assert.equal(encodeMouse({ pointer: 'none', pointerEncoding: 'x10' }, press), null);
    assert.equal(encodeMouse({ pointer: 'normal', pointerEncoding: 'sgr' }, press), '\x1b[<0;5;3M');
    assert.equal(encodeMouse({ pointer: 'normal', pointerEncoding: 'sgr' }, { ...press, kind: 'release' }), '\x1b[<0;5;3m');
    assert.equal(encodeMouse({ pointer: 'normal', pointerEncoding: 'x10' }, press), '\x1b[M %#');
    assert.equal(encodeMouse({ pointer: 'normal', pointerEncoding: 'sgr' }, { ...press, kind: 'wheelUp', ctrlKey: true }), '\x1b[<80;5;3M');
    assert.equal(encodeMouse({ pointer: 'normal', pointerEncoding: 'sgr' }, { ...press, kind: 'move', button: 'none' }), null);
    assert.equal(encodeMouse({ pointer: 'any', pointerEncoding: 'sgr' }, { ...press, kind: 'move', button: 'none' }), '\x1b[<35;5;3M');
});
