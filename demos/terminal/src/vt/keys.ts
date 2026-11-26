// What a key sends to the program: the bytes a terminal's keyboard has always sent, and how the mouse is reported.

import type { Terminal } from './terminal';

export interface KeyInfo {
    key: string;
    ctrlKey: boolean;
    altKey: boolean;
    shiftKey: boolean;
    metaKey?: boolean;
    code?: string;
    location?: number;
    repeat?: boolean;
    type?: string;
    getModifierState?: (key: string) => boolean;
}

const CSI_KEYS: Record<string, string> = {
    ArrowUp: 'A', ArrowDown: 'B', ArrowRight: 'C', ArrowLeft: 'D', Home: 'H', End: 'F',
};

const TILDE_KEYS: Record<string, number> = { Insert: 2, Delete: 3, PageUp: 5, PageDown: 6 };

const FUNCTION_KEYS: Record<string, number> = {
    F5: 15, F6: 17, F7: 18, F8: 19, F9: 20, F10: 21, F11: 23, F12: 24, F13: 25, F14: 26, F15: 28, F16: 29, F17: 31, F18: 32,
    F19: 33, F20: 34,
};

const SS3_KEYS: Record<string, string> = { F1: 'P', F2: 'Q', F3: 'R', F4: 'S' };

/** The number a modifier adds to a key's report: 1 plus shift 1, alt 2, ctrl 4 and meta 8. */
function modifier(e: KeyInfo): number {
    return 1 + (e.shiftKey ? 1 : 0) + (e.altKey ? 2 : 0) + (e.ctrlKey ? 4 : 0) + (e.metaKey ? 8 : 0);
}

/** What to send for a key, or null if it sends nothing (a modifier on its own, a key with no meaning here). */
export function encodeKey(e: KeyInfo, term: { appCursor: boolean; kittyFlags?: number }): string | null {
    const flags = term.kittyFlags ?? 0;
    if (!flags) return e.type === 'keyup' ? null : legacyKey(e, term);
    const { key, code = '' } = e;
    const all = !!(flags & 8);
    const release = e.type === 'keyup';
    if (release && !(flags & 2)) return null;
    const special = { Enter: 13, Tab: 9, Backspace: 127, Escape: 27 }[key];
    if (!all && special && special !== 27) return release ? null : legacyKey(e, term);
    const modifierCode = MODIFIER_CODES[code] ?? MODIFIER_CODES[key + (e.location === 2 ? 'Right' : 'Left')];
    if (!all && modifierCode) return null;
    const printable = Array.from(key).length === 1 && key.codePointAt(0)! >= 32;
    const altGraph = e.getModifierState?.('AltGraph');
    const modified = (e.ctrlKey || e.altKey || e.metaKey) && !altGraph;
    if (printable && !all && (!(flags & 1) || !modified)) {
        return release ? null : altGraph ? key : legacyKey(e, term);
    }
    const keypad = code.startsWith('Numpad') || e.location === 3;
    const keypadDigit = /^Numpad([0-9])$/.exec(code);
    const keypadNumber = keypad && ((key.length !== 1 && KEYPAD_KEYS[key]) || KEYPAD_KEYS[code] ||
        (keypadDigit ? 57399 + Number(keypadDigit[1]) : /^\d$/.test(key) ? 57399 + Number(key) : 0));
    const fn = /^F(\d+)$/.exec(key);
    const tilde = key === 'F3' ? 13 : TILDE_KEYS[key] ?? FUNCTION_KEYS[key];
    const letter = CSI_KEYS[key] ?? SS3_KEYS[key];
    const physical = /^Key([A-Z])$/.exec(code)?.[1].toLowerCase() ?? /^Digit([0-9])$/.exec(code)?.[1] ?? US_KEYS[code];
    const unshifted = printable ? (e.shiftKey ? UNSHIFTED[key] ?? key.toLowerCase() : key.toLowerCase()) : '';
    const privateKey = keypadNumber || modifierCode || KITTY_KEYS[key] ||
        (fn && Number(fn[1]) >= 13 && Number(fn[1]) <= 35 ? 57363 + Number(fn[1]) : 0);
    const number = privateKey || special || (printable ? unshifted.codePointAt(0)! : tilde ?? (letter ? 1 : 0));
    if (!number) return null;
    const final = privateKey || special || printable ? 'u' : tilde ? '~' : letter;
    let keys = String(number);
    if (printable && !privateKey && flags & 4) {
        const shifted = e.shiftKey && key !== unshifted ? key.codePointAt(0) : undefined;
        const base = physical && physical !== unshifted ? physical.codePointAt(0) : undefined;
        if (shifted || base) keys += ':' + (shifted ?? '') + (base ? ':' + base : '');
    }
    let mod = modifier(e);
    if (all || !printable) mod += (e.getModifierState?.('CapsLock') ? 64 : 0) + (e.getModifierState?.('NumLock') ? 128 : 0);
    const event = flags & 2 ? release ? 3 : e.repeat ? 2 : 1 : 1;
    const associated = all && flags & 16 && printable && !release && !modified ? key.codePointAt(0) : 0;
    const suffix = mod !== 1 || event !== 1 || associated ? `;${mod}${event === 1 ? '' : ':' + event}${associated ? ';' + associated : ''}` : '';
    return `\x1b[${keys === '1' && letter && !suffix ? '' : keys}${suffix}${final}`;
}

export function encodeCommittedText(text: string, flags: number) {
    if ((flags & 24) !== 24) return flags & 8 ? '' : text;
    const points = Array.from(text, (character) => character.codePointAt(0)!).filter((point) => point >= 32 && point !== 127);
    return points.length ? `\x1b[0;1;${points.join(':')}u` : '';
}

const MODIFIER_CODES: Record<string, number> = {
    ShiftLeft: 57441, ControlLeft: 57442, AltLeft: 57443, MetaLeft: 57444,
    ShiftRight: 57447, ControlRight: 57448, AltRight: 57449, MetaRight: 57450, AltGraphLeft: 57453, AltGraphRight: 57453,
};
const KITTY_KEYS: Record<string, number> = {
    CapsLock: 57358, ScrollLock: 57359, NumLock: 57360, PrintScreen: 57361, Pause: 57362, ContextMenu: 57363,
    MediaPlay: 57428, MediaPause: 57429, MediaPlayPause: 57430, MediaStop: 57432, MediaFastForward: 57433,
    MediaRewind: 57434, MediaTrackNext: 57435, MediaTrackPrevious: 57436, MediaRecord: 57437,
    AudioVolumeDown: 57438, AudioVolumeUp: 57439, AudioVolumeMute: 57440,
};
const KEYPAD_KEYS: Record<string, number> = {
    NumpadDecimal: 57409, NumpadDivide: 57410, NumpadMultiply: 57411, NumpadSubtract: 57412, NumpadAdd: 57413,
    NumpadEnter: 57414, NumpadEqual: 57415, NumpadComma: 57416, ArrowLeft: 57417, ArrowRight: 57418, ArrowUp: 57419,
    ArrowDown: 57420, PageUp: 57421, PageDown: 57422, Home: 57423, End: 57424, Insert: 57425, Delete: 57426, Clear: 57427,
};
const US_KEYS: Record<string, string> = {
    Backquote: '`', Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']', Backslash: '\\',
    Semicolon: ';', Quote: "'", Comma: ',', Period: '.', Slash: '/', Space: ' ',
};
const UNSHIFTED: Record<string, string> = Object.fromEntries(
    Array.from('~!@#$%^&*()_+{}|:"<>?', (character, idx) => [character, '`1234567890-=[]\\;\',./'[idx]]),
);

function legacyKey(e: KeyInfo, term: { appCursor: boolean }): string | null {
    const key = e.key;
    const mod = modifier(e);
    const plain = mod === 1;
    const arrow = CSI_KEYS[key];
    if (arrow !== undefined) {
        if (!plain) return `\x1b[1;${mod}${arrow}`;
        return term.appCursor ? `\x1bO${arrow}` : `\x1b[${arrow}`;
    }
    const tilde = TILDE_KEYS[key];
    if (tilde !== undefined) return plain ? `\x1b[${tilde}~` : `\x1b[${tilde};${mod}~`;
    const fn = FUNCTION_KEYS[key];
    if (fn !== undefined) return plain ? `\x1b[${fn}~` : `\x1b[${fn};${mod}~`;
    const ss3 = SS3_KEYS[key];
    if (ss3 !== undefined) return plain ? `\x1bO${ss3}` : `\x1b[1;${mod}${ss3}`;
    switch (key) {
        case 'Enter':
            return e.altKey ? '\x1b\r' : '\r';
        case 'Backspace':
            return e.ctrlKey ? '\x08' : e.altKey ? '\x1b\x7f' : '\x7f';
        case 'Tab':
            return e.shiftKey ? '\x1b[Z' : e.altKey ? '\x1b\t' : '\t';
        case 'Escape':
            return '\x1b';
        case 'Shift':
        case 'Control':
        case 'Alt':
        case 'Meta':
        case 'CapsLock':
        case 'NumLock':
        case 'ScrollLock':
        case 'ContextMenu':
        case 'Dead':
        case 'Process':
        case 'Unidentified':
            return null;
    }
    // A character, or a pair of them for what's outside the first 64K.
    if (key.length === 0 || (key.length > 2 && !/^[\ud800-\udbff][\udc00-\udfff]$/.test(key))) return null;
    if (e.ctrlKey && !e.metaKey) {
        const c = key.length === 1 ? key.charCodeAt(0) : 0;
        let code = -1;
        if (c >= 0x61 && c <= 0x7a) code = c - 0x60;
        else if (c >= 0x41 && c <= 0x5a) code = c - 0x40;
        else if (c === 0x20 || c === 0x40 || c === 0x32) code = 0;
        else if (c === 0x5b || c === 0x33) code = 27;
        else if (c === 0x5c || c === 0x34) code = 28;
        else if (c === 0x5d || c === 0x35) code = 29;
        else if (c === 0x5e || c === 0x36) code = 30;
        else if (c === 0x5f || c === 0x2d || c === 0x2f || c === 0x37) code = 31;
        else if (c === 0x38) code = 127;
        if (code >= 0) return (e.altKey ? '\x1b' : '') + String.fromCharCode(code);
        return null;
    }
    return (e.altKey ? '\x1b' : '') + key;
}

/** What's sent when text is pasted: wrapped in the markers the program asked for, with no marker in the text itself. */
export function encodePaste(text: string, term: { bracketedPaste: boolean }): string {
    const clean = text.replace(/\r?\n/g, '\r');
    if (!term.bracketedPaste) return clean;
    return '\x1b[200~' + clean.replace(/\x1b\[20[01]~/g, '') + '\x1b[201~';
}

export function needsPasteConfirmation(text: string, terminals: { bracketedPaste: boolean; atPrompt: boolean | null }[], promptOnly: boolean): boolean {
    return /[\r\n]/.test(text) && terminals.some((term) => !term.bracketedPaste && (!promptOnly || term.atPrompt !== false));
}

export interface MouseEvent {
    /** 0 left, 1 middle, 2 right, or 'none' for movement with no button held. */
    button: 0 | 1 | 2 | 'none';
    kind: 'press' | 'release' | 'move' | 'wheelUp' | 'wheelDown' | 'wheelLeft' | 'wheelRight';
    /** The cell, counting from 0. */
    col: number;
    row: number;
    shiftKey: boolean;
    altKey: boolean;
    ctrlKey: boolean;
}

/** What the mouse does as the program has asked to hear it, or null when it didn't ask for this. */
export function encodeMouse(term: Pick<Terminal, 'pointer' | 'pointerEncoding'>, e: MouseEvent): string | null {
    if (term.pointer === 'none') return null;
    const wheel = e.kind.startsWith('wheel');
    if (e.kind === 'move' && term.pointer !== 'any' && !(term.pointer === 'button' && e.button !== 'none')) return null;
    if (e.kind === 'release' && term.pointer === 'x10') return null;
    if (wheel && term.pointer === 'x10') return null;
    let b: number;
    if (e.kind === 'wheelUp') b = 64;
    else if (e.kind === 'wheelDown') b = 65;
    else if (e.kind === 'wheelLeft') b = 66;
    else if (e.kind === 'wheelRight') b = 67;
    else b = e.button === 'none' ? 3 : e.button;
    if (term.pointer !== 'x10') {
        if (e.shiftKey) b += 4;
        if (e.altKey) b += 8;
        if (e.ctrlKey) b += 16;
    }
    if (e.kind === 'move') b += 32;
    const x = e.col + 1;
    const y = e.row + 1;
    switch (term.pointerEncoding) {
        case 'sgr':
            return `\x1b[<${b};${x};${y}${e.kind === 'release' ? 'm' : 'M'}`;
        case 'urxvt':
            return `\x1b[${(e.kind === 'release' ? 3 : b) + 32};${x};${y}M`;
        case 'utf8':
            return `\x1b[M${String.fromCodePoint((e.kind === 'release' ? 3 : b) + 32)}${String.fromCodePoint(32 + x)}${String.fromCodePoint(32 + y)}`;
        default:
            if (x > 223 || y > 223) return null;
            return `\x1b[M${String.fromCharCode((e.kind === 'release' ? 3 : b) + 32, 32 + x, 32 + y)}`;
    }
}
