// Hints: the things on the screen worth picking up, each given a letter or two to press, as Kitty's hints and WezTerm's quick
// select do: addresses, files (with the line and column after them), git hashes, IP addresses, UUIDs, colors and links a program
// marked itself.

import type { Line, Terminal } from '../vt/terminal';
import { lineText } from './search';

export type HintKind = 'url' | 'path' | 'hash' | 'ip' | 'uuid' | 'color' | 'link';

export interface Hint {
    label: string;
    kind: HintKind;
    text: string;
    line: number;
    col: number;
    endCol: number;
    /** For a file, where in it: `path:12:5`. */
    file?: { path: string; line: number | null; col: number | null };
}

/**
 * What to look for. A pattern with a prefix group (`(^|[^...])`) has its match in the second group, so that a path starts at a
 * boundary and not in the middle of another word, without a lookbehind.
 */
const PATTERNS: [HintKind, RegExp, boolean][] = [
    ['url', /\b(?:https?|ftp|file|ssh|git):\/\/[^\s<>"'`)\]}]+/g, false],
    ['uuid', /\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/gi, false],
    ['path', /[A-Za-z]:[\\/][^\s:*?"<>|'`]+(?::\d+(?::\d+)?)?/g, false],
    ['path', /(^|[^\w@%+=.~/\\:-])((?:~|\.{1,2})?(?:[\\/][\w@%+=.,~-]+)+(?::\d+(?::\d+)?)?)/g, true],
    ['path', /(^|[^\w@%+=.~/\\:-])((?:[\w@%+=.~-]+[\\/])+[\w@%+=.~-]*\.\w+(?::\d+(?::\d+)?)?)/g, true],
    ['path', /(^|[^\w@%+=.~/\\:-])([\w.-]+\.(?:tsx?|jsx?|json|rs|py|go|md|toml|ya?ml|css|html|txt|log|cs|cpp|hpp|c|h|java|kt|sh|ps1|sql|lock|ini|xml)(?::\d+(?::\d+)?)?)/g, true],
    ['ip', /\b\d{1,3}(?:\.\d{1,3}){3}(?::\d{2,5})?\b/g, false],
    ['hash', /\b(?=[0-9a-f]*[a-f])(?=[0-9a-f]*\d)[0-9a-f]{7,40}\b/g, false],
    ['color', /#[0-9a-fA-F]{6}\b/g, false],
];

const LABELS = 'asdfghjklqwertyuiopzxcvbnm';

/** Takes the punctuation a sentence puts after an address off its end. */
function trimmed(text: string): string {
    return text.replace(/[.,;:!?'")\]}]+$/, '');
}

function fileOf(text: string): Hint['file'] {
    const m = /^(.*?)(?::(\d+)(?::(\d+))?)?$/.exec(text);
    if (!m) return { path: text, line: null, col: null };
    return { path: m[1], line: m[2] ? Number(m[2]) : null, col: m[3] ? Number(m[3]) : null };
}

export function findHints(term: Terminal, lines: { number: number; line: Line }[]): Hint[] {
    const found: Omit<Hint, 'label'>[] = [];
    for (const { number, line } of lines) {
        const { text, cols } = lineText(term, line);
        const taken: [number, number][] = [];
        const free = (a: number, b: number) => taken.every(([x, y]) => b <= x || a >= y);
        // What a program marked as a link.
        let runStart = -1;
        let runLink = 0;
        const closeRun = (end: number) => {
            if (runStart >= 0 && runLink !== 0) {
                found.push({ kind: 'link', text: term.link(runLink), line: number, col: runStart, endCol: end });
                taken.push([runStart, end]);
            }
            runStart = -1;
            runLink = 0;
        };
        const used = line.used();
        for (let x = 0; x < used; x++) {
            const link = term.styles.get(line.st[x]).link;
            if (link !== runLink) {
                closeRun(x);
                if (link !== 0) {
                    runStart = x;
                    runLink = link;
                }
            }
        }
        closeRun(used);
        for (const [kind, pattern, prefixed] of PATTERNS) {
            pattern.lastIndex = 0;
            let m: RegExpExecArray | null;
            while ((m = pattern.exec(text)) !== null) {
                const raw = prefixed ? m[2] : m[0];
                const at = prefixed ? m.index + m[1].length : m.index;
                const value = trimmed(raw);
                if (value.length < 2) continue;
                const from = cols[at] ?? 0;
                const to = (cols[at + value.length - 1] ?? from) + 1;
                if (!free(from, to)) continue;
                taken.push([from, to]);
                const hint: Omit<Hint, 'label'> = { kind, text: value, line: number, col: from, endCol: to };
                if (kind === 'path') hint.file = fileOf(value);
                found.push(hint);
            }
        }
    }
    // The nearest the bottom gets the shortest labels, as that's where what's wanted usually is.
    found.sort((a, b) => b.line - a.line || a.col - b.col);
    const two = found.length > LABELS.length;
    return found.slice(0, LABELS.length * LABELS.length).map((hint, i) => ({
        ...hint,
        label: two ? LABELS[Math.floor(i / LABELS.length)] + LABELS[i % LABELS.length] : LABELS[i],
    }));
}
