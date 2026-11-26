// Searching everything the terminal has: the scrollback and the screen. Lines that wrapped are searched as one, so a match can
// run on from the end of a line to the start of the next.

import { WIDE_TAIL } from '../vt/terminal';
import type { Line, Terminal } from '../vt/terminal';

export interface Match {
    /** Where it starts, as line numbers and columns, and where it ends (not included). */
    line: number;
    col: number;
    endLine: number;
    endCol: number;
}

export interface SearchOptions {
    query: string;
    caseSensitive: boolean;
    regex: boolean;
}

export interface SearchResult {
    matches: Match[];
    error: string | null;
}

const MOST = 5000;

/** A line's text with, for each character in it, the column it's in. */
export function lineText(term: Terminal, line: Line): { text: string; cols: Int32Array } {
    const n = line.used();
    let text = '';
    const cols = new Int32Array(n * 2 + 2);
    let at = 0;
    for (let x = 0; x < n; x++) {
        const ch = line.ch[x];
        if (ch === WIDE_TAIL) continue;
        const part = ch === 0 ? ' ' : (line.extra?.get(x) ?? String.fromCodePoint(ch));
        for (let k = 0; k < part.length; k++) cols[at + k] = x;
        at += part.length;
        text += part;
    }
    cols[at] = n;
    void term;
    return { text, cols: cols.subarray(0, at + 1) };
}

export function searchLines(term: Terminal, options: SearchOptions): SearchResult {
    if (options.query === '') return { matches: [], error: null };
    let pattern: RegExp;
    try {
        const source = options.regex ? options.query : options.query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
        pattern = new RegExp(source, options.caseSensitive ? 'g' : 'gi');
    } catch (error) {
        return { matches: [], error: error instanceof Error ? error.message.replace(/^Invalid regular expression: /, '') : 'not a pattern' };
    }
    const matches: Match[] = [];
    const first = term.evicted;
    const last = term.evicted + term.lineCount;
    let n = first;
    while (n < last && matches.length < MOST) {
        // One logical line: this and each that follows while the one before wrapped.
        const segments: { number: number; text: string; cols: Int32Array; start: number }[] = [];
        let joined = '';
        let m = n;
        for (;;) {
            const line = term.lineNumbered(m);
            if (!line) break;
            const part = lineText(term, line);
            segments.push({ number: m, text: part.text, cols: part.cols, start: joined.length });
            joined += line.wrapped ? part.text.padEnd(line.length, ' ') : part.text;
            m++;
            if (!line.wrapped || m >= last) break;
        }
        if (joined !== '') {
            pattern.lastIndex = 0;
            let found: RegExpExecArray | null;
            while ((found = pattern.exec(joined)) !== null && matches.length < MOST) {
                if (found[0].length === 0) {
                    pattern.lastIndex++;
                    continue;
                }
                const from = locate(segments, found.index, joined.length, false);
                const to = locate(segments, found.index + found[0].length - 1, joined.length, true);
                if (from && to) matches.push({ line: from.line, col: from.col, endLine: to.line, endCol: to.col });
            }
        }
        n = m;
    }
    return { matches, error: null };
}

function locate(
    segments: { number: number; text: string; cols: Int32Array; start: number }[], index: number, total: number, end: boolean,
): { line: number; col: number } | null {
    void total;
    for (let i = segments.length - 1; i >= 0; i--) {
        const segment = segments[i];
        if (index >= segment.start) {
            const local = index - segment.start;
            const cols = segment.cols;
            if (local < cols.length - 1) return { line: segment.number, col: end ? cols[local] + 1 : cols[local] };
            // Past the text of a wrapped line: its padding, which is blank cells.
            return { line: segment.number, col: end ? local + 1 : local };
        }
    }
    return null;
}

/** What to highlight in each line: the columns of every match there, and whether each is the current one. */
export function spansByLine(matches: Match[], current: number, cols: number): Map<number, [number, number, boolean][]> {
    const map = new Map<number, [number, number, boolean][]>();
    const add = (line: number, from: number, to: number, isCurrent: boolean) => {
        let list = map.get(line);
        if (!list) map.set(line, (list = []));
        list.push([from, to, isCurrent]);
    };
    matches.forEach((match, i) => {
        const isCurrent = i === current;
        if (match.line === match.endLine) {
            add(match.line, match.col, match.endCol, isCurrent);
        } else {
            add(match.line, match.col, cols, isCurrent);
            for (let n = match.line + 1; n < match.endLine; n++) add(n, 0, cols, isCurrent);
            add(match.endLine, 0, match.endCol, isCurrent);
        }
    });
    return map;
}
