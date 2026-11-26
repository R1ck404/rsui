// Copy mode: a cursor of its own that moves over everything the terminal has, with the keys vi has, to select text and copy it
// without the mouse. Esc or q leaves it.

import type { Session } from './Session';
import type { Pos } from './Session';
import { WIDE_TAIL } from '../vt/terminal';
import type { Line } from '../vt/terminal';

type Key = { key: string; ctrlKey: boolean; shiftKey: boolean; altKey: boolean };

/** 0 blank, 1 letters and digits, 2 anything else: what a word is made of. */
function kindAt(line: Line | null, col: number): number {
    if (!line || col < 0 || col >= line.length) return 0;
    let ch = line.ch[col];
    if (ch === WIDE_TAIL && col > 0) ch = line.ch[col - 1];
    if (ch === 0 || ch === 0x20) return 0;
    const c = String.fromCodePoint(ch);
    return /[\p{L}\p{N}_]/u.test(c) ? 1 : 2;
}

export function copyKey(session: Session, e: Key, done: { copy: (text: string) => void }): boolean {
    const mode = session.copyMode;
    if (!mode) return false;
    const term = session.term;
    const first = term.evicted;
    const last = term.evicted + term.lineCount - 1;
    const cols = term.cols;
    const cursor = mode.cursor;
    const at = (line: number, col: number): Pos => ({ line: Math.max(first, Math.min(last, line)), col: Math.max(0, Math.min(cols - 1, col)) });

    // What's being typed after / or ?
    if (mode.searching) {
        const s = mode.searching;
        if (e.key === 'Escape') {
            mode.searching = null;
        } else if (e.key === 'Enter') {
            mode.searching = null;
            session.setSearch({ query: s.text });
            session.search = { ...session.search, open: false };
            jumpToMatch(session, s.forward ? 1 : -1, true);
        } else if (e.key === 'Backspace') {
            s.text = s.text.slice(0, -1);
        } else if (e.key.length === 1 && !e.ctrlKey) {
            s.text += e.key;
        }
        session.changed();
        return true;
    }

    const move = (to: Pos) => {
        mode.cursor = to;
        session.reveal(to.line, false);
        // Keep it in view without jumping about.
        const top = session.topLine;
        if (to.line < top) session.scrollTo(to.line);
        else if (to.line >= top + term.rows) session.scrollTo(to.line - term.rows + 1);
        if (mode.selecting) {
            const kind = mode.selecting.kind;
            session.selection = { anchor: mode.selecting.from, head: to, mode: kind === 'block' ? 'rect' : kind === 'line' ? 'line' : 'cell' };
        }
        session.changed();
    };

    const prior = mode.pending;
    const count = parseInt(prior, 10);
    const n = Number.isFinite(count) && count > 0 ? count : 1;
    const key = e.key;
    if (/^[1-9]$/.test(key) || (key === '0' && /^\d+$/.test(prior))) {
        mode.pending = (prior === 'g' ? '' : prior) + key;
        session.changed();
        return true;
    }
    mode.pending = '';
    const lineOf = (line: number) => term.lineNumbered(line);

    if (e.ctrlKey) {
        const half = Math.max(1, Math.floor(term.rows / 2));
        switch (key.toLowerCase()) {
            case 'u': move(at(cursor.line - half, cursor.col)); return true;
            case 'd': move(at(cursor.line + half, cursor.col)); return true;
            case 'b': move(at(cursor.line - term.rows, cursor.col)); return true;
            case 'f': move(at(cursor.line + term.rows, cursor.col)); return true;
            case 'v':
                mode.selecting = mode.selecting?.kind === 'block' ? null : { kind: 'block', from: { ...cursor } };
                move(cursor);
                if (!mode.selecting) session.selection = null;
                return true;
            case 'c':
                session.stopCopyMode();
                return true;
        }
        return true;
    }

    switch (key) {
        case 'Escape':
        case 'q':
            if (mode.selecting) {
                mode.selecting = null;
                session.selection = null;
                session.changed();
            } else {
                session.stopCopyMode();
            }
            return true;
        case 'h':
        case 'ArrowLeft':
            move(at(cursor.line, cursor.col - n));
            return true;
        case 'l':
        case 'ArrowRight':
            move(at(cursor.line, cursor.col + n));
            return true;
        case 'j':
        case 'ArrowDown':
            move(at(cursor.line + n, cursor.col));
            return true;
        case 'k':
        case 'ArrowUp':
            move(at(cursor.line - n, cursor.col));
            return true;
        case '0':
        case 'Home':
            move(at(cursor.line, 0));
            return true;
        case '$':
        case 'End': {
            const line = lineOf(cursor.line);
            move(at(cursor.line, Math.max(0, (line?.used() ?? 1) - 1)));
            return true;
        }
        case '^': {
            const line = lineOf(cursor.line);
            let col = 0;
            while (col < cols - 1 && kindAt(line, col) === 0) col++;
            move(at(cursor.line, col));
            return true;
        }
        case 'g':
            // `gg` goes to the top: the first g waits for the second.
            if (prior === 'g') {
                move(at(first, 0));
            } else {
                mode.pending = 'g';
                session.changed();
            }
            return true;
        case 'G':
            move(at(last, 0));
            return true;
        case 'H':
            move(at(session.topLine, 0));
            return true;
        case 'M':
            move(at(session.topLine + Math.floor(term.rows / 2), 0));
            return true;
        case 'L':
            move(at(session.topLine + term.rows - 1, 0));
            return true;
        case 'w':
        case 'W': {
            let p = { ...cursor };
            for (let k = 0; k < n; k++) p = nextWord(session, p, key === 'W');
            move(at(p.line, p.col));
            return true;
        }
        case 'b':
        case 'B': {
            let p = { ...cursor };
            for (let k = 0; k < n; k++) p = prevWord(session, p, key === 'B');
            move(at(p.line, p.col));
            return true;
        }
        case 'e':
        case 'E': {
            let p = { ...cursor };
            for (let k = 0; k < n; k++) p = endWord(session, p, key === 'E');
            move(at(p.line, p.col));
            return true;
        }
        case 'v':
            mode.selecting = mode.selecting?.kind === 'char' ? null : { kind: 'char', from: { ...cursor } };
            if (!mode.selecting) session.selection = null;
            move(cursor);
            return true;
        case 'V':
            mode.selecting = mode.selecting?.kind === 'line' ? null : { kind: 'line', from: { ...cursor } };
            if (!mode.selecting) session.selection = null;
            move(cursor);
            return true;
        case 'y':
        case 'Enter': {
            const text = session.selection ? session.selectionText() : '';
            if (text !== '') done.copy(text);
            session.stopCopyMode();
            return true;
        }
        case '/':
            mode.searching = { forward: true, text: '' };
            session.changed();
            return true;
        case '?':
            mode.searching = { forward: false, text: '' };
            session.changed();
            return true;
        case 'n':
            jumpToMatch(session, 1, false);
            return true;
        case 'N':
            jumpToMatch(session, -1, false);
            return true;
    }
    return true;
}

/** Moves to the next or previous match of the last search from where the cursor is. */
function jumpToMatch(session: Session, direction: 1 | -1, fresh: boolean) {
    const mode = session.copyMode;
    if (!mode) return;
    const matches = session.search.matches;
    if (matches.length === 0) return;
    const c = mode.cursor;
    let target = matches.findIndex((m) => (direction > 0 ? m.line > c.line || (m.line === c.line && m.col > c.col) : false));
    if (direction > 0) {
        if (target < 0) target = 0;
    } else {
        target = -1;
        for (let i = matches.length - 1; i >= 0; i--) {
            const m = matches[i];
            if (m.line < c.line || (m.line === c.line && m.col < c.col)) {
                target = i;
                break;
            }
        }
        if (target < 0) target = matches.length - 1;
    }
    void fresh;
    const m = matches[target];
    session.search = { ...session.search, current: target };
    mode.cursor = { line: m.line, col: m.col };
    session.reveal(m.line);
    if (mode.selecting) {
        session.selection = { anchor: mode.selecting.from, head: mode.cursor, mode: mode.selecting.kind === 'block' ? 'rect' : mode.selecting.kind === 'line' ? 'line' : 'cell' };
    }
    session.changed();
}

function classOf(session: Session, p: Pos, big: boolean): number {
    const k = kindAt(session.lineNumbered(p.line), p.col);
    return big && k !== 0 ? 1 : k;
}

function step(session: Session, p: Pos, direction: 1 | -1): Pos | null {
    const term = session.term;
    let { line, col } = p;
    col += direction;
    if (col >= term.cols) {
        line++;
        col = 0;
    } else if (col < 0) {
        line--;
        col = term.cols - 1;
    }
    if (line < term.evicted || line > term.evicted + term.lineCount - 1) return null;
    return { line, col };
}

function nextWord(session: Session, from: Pos, big: boolean): Pos {
    let p: Pos | null = from;
    const start = classOf(session, p, big);
    while (p && start !== 0 && classOf(session, p, big) === start) p = step(session, p, 1);
    while (p && classOf(session, p, big) === 0) p = step(session, p, 1);
    return p ?? from;
}

function prevWord(session: Session, from: Pos, big: boolean): Pos {
    let p: Pos | null = step(session, from, -1);
    while (p && classOf(session, p, big) === 0) p = step(session, p, -1);
    if (!p) return from;
    const kind = classOf(session, p, big);
    for (;;) {
        const before = step(session, p, -1);
        if (!before || before.line !== p.line || classOf(session, before, big) !== kind) break;
        p = before;
    }
    return p;
}

function endWord(session: Session, from: Pos, big: boolean): Pos {
    let p: Pos | null = step(session, from, 1);
    while (p && classOf(session, p, big) === 0) p = step(session, p, 1);
    if (!p) return from;
    const kind = classOf(session, p, big);
    for (;;) {
        const after = step(session, p, 1);
        if (!after || after.line !== p.line || classOf(session, after, big) !== kind) break;
        p = after;
    }
    return p;
}
