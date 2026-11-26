// One shell in one column: the program running in a pseudo terminal, the emulator that reads what it prints, and everything the
// column keeps about how it's being looked at: how far it's scrolled, what's selected, what's searched for, and the commands
// it has run. It's a store: components read it and hear when it changes, and a burst of output makes one change a frame.

import { pty } from 'rsui/desktop';
import type { Pty } from 'rsui/desktop';
import { encodeCommittedText, encodeKey, encodeMouse, encodePaste } from '../vt/keys';
import type { KeyInfo, MouseEvent } from '../vt/keys';
import { Terminal } from '../vt/terminal';
import type { Command, Line, Theme } from '../vt/terminal';
import { findHints } from './hints';
import type { Hint } from './hints';
import { searchLines } from './search';
import type { Match, SearchOptions } from './search';
import { appendEvent, RecentTimeline, replayClock } from './timeline';
import type { Recorded } from './timeline';
import { compileTrigger } from './triggers';
import type { Settings } from '../settings';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

export interface Pos {
    /** A line's number, as the terminal counts them from the first it has ever kept, so it stays put as output scrolls. */
    line: number;
    col: number;
}

export type SelectionMode = 'cell' | 'word' | 'line' | 'rect';

export interface Selection {
    anchor: Pos;
    head: Pos;
    mode: SelectionMode;
}

export interface SearchState {
    open: boolean;
    query: string;
    caseSensitive: boolean;
    regex: boolean;
    matches: Match[];
    current: number;
    error: string | null;
}

export interface CopyMode {
    cursor: Pos;
    /** Selecting: by character, by line or by block, from where `v` was pressed. */
    selecting: null | { kind: 'char' | 'line' | 'block'; from: Pos };
    /** What's typed after `/` or `?` while it's being typed. */
    searching: null | { forward: boolean; text: string };
    pending: string;
}

export interface HintState {
    hints: Hint[];
    typed: string;
}

export interface SessionOptions {
    id: number;
    profileId: string;
    profileName: string;
    label?: string;
    command: string;
    cwd?: string;
    initialCwd?: string;
    env?: Record<string, string>;
    cols: number;
    rows: number;
    scrollback: number;
    theme: Theme;
    onFinished?: (session: Session, command: Command) => void;
    onBell?: (session: Session) => void;
    onClipboard?: (text: string) => void;
    onNotify?: (session: Session, title: string, body: string) => void;
    onExit?: (session: Session, code: number) => void;
    onProgress?: (session: Session) => void;
    settings?: () => Settings;
    transcript?: string[];
    savedBookmarks?: { text: string; output: string }[];
    host?: string;
    onOutput?: () => void;
    onAttention?: () => void;
}

export interface ShellObserver {
    command: (command: Command, change: 'output' | 'end') => void;
    line?: (line: Line, number: number, partial: boolean) => void;
    updated: () => void;
    closed: () => void;
}

let nextId = 1;
export const newId = () => nextId++;

export class Session {
    readonly id: number;
    readonly profileId: string;
    readonly profileName: string;
    readonly term: Terminal;
    pty: Pty | null = null;
    hasInput = false;
    /** The shell's own exit code once it's over. */
    exitCode: number | null = null;
    startError: string | null = null;
    title = '';
    cwd = '';
    /** Bumps whenever anything to draw changes. */
    version = 0;

    /** The first line shown when scrolled back, or null while it follows the bottom. */
    scrolledTop: number | null = null;
    selection: Selection | null = null;
    search: SearchState = { open: false, query: '', caseSensitive: false, regex: false, matches: [], current: -1, error: null };
    copyMode: CopyMode | null = null;
    hints: HintState | null = null;
    bellUntil = 0;
    focused = false;
    attention = '';
    historyOpen = false;
    bookmarks = new Map<number, { command: Command; output: string }>();
    folds = new Set<number>();
    highlights = new WeakMap<Line, number>();
    gitBranch = '';
    #gitRequest = '';
    #gitEpoch = 0;
    #replay: RecentTimeline | null = null;
    #recordStart = 0;
    #rules?: Settings['triggers'];
    #triggers: { rule: Settings['triggers'][number]; regex: RegExp }[] = [];
    #seen = new WeakMap<Line, { version: number; rules: Set<number> }>();
    #hasLineHook = false;
    #notifiedAt = -Infinity;
    recording: { startedAt: number; events: Recorded[]; cols: number; rows: number } | null = null;
    /** Everything that's been typed in a run of this, for the shell's finished marker. */
    lastActivity = Date.now();
    /** Wide characters' images and the like are decoded elsewhere; this says which have been seen. */
    readonly options: SessionOptions;

    #listeners = new Set<() => void>();
    #pending = false;
    #syncSince = 0;
    #disposed = false;
    #shellObservers = new Set<ShellObserver>();
    #searchTimer: ReturnType<typeof setTimeout> | null = null;
    #lastCols: number;
    #lastRows: number;

    constructor(options: SessionOptions) {
        this.options = options;
        this.id = options.id;
        this.profileId = options.profileId;
        this.profileName = options.profileName;
        this.cwd = options.initialCwd ?? options.cwd ?? '';
        this.#lastCols = options.cols;
        this.#lastRows = options.rows;
        const term = new Terminal(options.cols, options.rows, options.scrollback);
        term.theme = options.theme;
        term.host = options.host ?? '';
        if (options.transcript?.length) term.restoreTranscript(options.transcript);
        this.term = term;
        options.savedBookmarks?.forEach(({ text, output }, index) => {
            const id = -index - 1;
            this.bookmarks.set(id, { output, command: {
                id, text, promptLine: -1, inputLine: null, outputLine: null, endLine: -1,
                exit: null, startedAt: null, endedAt: null, cwd: '',
            } });
        });
        this.refreshOptions();
        term.onReply = (text) => this.#write(text);
        term.onTitle = (title) => {
            this.title = title;
            this.changed();
        };
        term.onCwd = (cwd) => {
            this.cwd = cwd;
            this.updateGit();
            this.changed();
        };
        term.onBell = () => {
            if (options.settings?.().attentionOn === 'notificationsAndBell') this.requestAttention('Bell');
            this.bellUntil = Date.now() + 160;
            options.onBell?.(this);
            this.changed();
        };
        term.onClipboard = (text) => options.onClipboard?.(text);
        term.onNotify = (title, body) => {
            const attention = options.settings?.().attentionOn;
            if (attention && attention !== 'off') this.requestAttention(title || body || 'Notification');
            options.onNotify?.(this, title, body);
        };
        term.onProgress = () => options.onProgress?.(this);
        term.onCommand = (command, change) => {
            if (change === 'output' || change === 'end') {
                for (const observer of this.#shellObservers) observer.command(command, change);
            }
            if (change === 'end') options.onFinished?.(this, command);
            if (change === 'prompt') this.updateGit(true);
            this.changed();
        };
        term.onSync = (on) => {
            if (on) this.#syncSince = Date.now();
            else this.changed();
        };
        this.#start();
    }

    #start() {
        const { command, cwd, env, cols, rows } = this.options;
        try {
            this.pty = pty.spawn({
                command,
                cwd,
                env: {
                    TERM: 'xterm-256color',
                    COLORTERM: 'truecolor',
                    TERM_PROGRAM: 'rsui-terminal',
                    TERM_PROGRAM_VERSION: '0.1.0',
                    RSUI_TERMINAL_RESTORED: this.options.transcript?.length ? '1' : '',
                    ...env,
                },
                cols,
                rows,
                onData: (text) => this.#data(text),
                onExit: (code) => {
                    this.exitCode = code;
                    for (const observer of this.#shellObservers) observer.updated();
                    this.options.onExit?.(this, code);
                    this.changed();
                },
            });
        } catch (error) {
            this.startError = error instanceof Error ? error.message : String(error);
            this.exitCode = -1;
            this.term.write(`\x1b[31m${this.startError}\x1b[0m\r\n`);
            this.changed();
        }
    }

    /** What it's called: what its program said, unless that's only the path of the program, which says less than its name. */
    get displayName(): string {
        if (this.options.label) return this.options.label;
        const title = this.title.trim();
        if (title === '' || /\.exe$/i.test(title) || /^[A-Za-z]:\\[^ ]*$/.test(title)) return this.profileName;
        return title;
    }

    // ---- Changes, and hearing of them

    subscribe = (listener: () => void) => {
        this.#listeners.add(listener);
        return () => this.#listeners.delete(listener);
    };

    getVersion = () => this.version;

    observeShell(observer: ShellObserver) {
        this.#shellObservers.add(observer);
        this.#updateLineHook();
        return () => {
            this.#shellObservers.delete(observer);
            this.#updateLineHook();
        };
    }

    /** Says something changed, once per frame however often it's said. */
    changed() {
        if (this.#pending || this.#disposed) return;
        this.#pending = true;
        const run = () => {
            // A program that's drawing a whole screen at once asked to be shown when it's done, within a moment.
            if (this.term.synchronized && Date.now() - this.#syncSince < 150) {
                setTimeout(run, 16);
                return;
            }
            this.#pending = false;
            this.version++;
            for (const listener of [...this.#listeners]) listener();
        };
        setTimeout(run, 0);
    }

    #data(text: string) {
        this.lastActivity = Date.now();
        const term = this.term;
        this.#capture(text);
        term.write(text);
        if (term.onLine && !term.alternate) {
            const number = term.evicted + term.scrollbackLength + term.cursorY;
            const line = term.lineNumbered(number);
            if (line) this.#observeLine(line, number, true);
        }
        for (const observer of this.#shellObservers) observer.updated();
        if (this.scrolledTop !== null) {
            // Scrolled back, the view stays on the same lines, unless they've gone from the top of the scrollback.
            if (this.scrolledTop < term.evicted) this.scrolledTop = term.evicted;
        }
        if (this.selection && this.selection.anchor.line < term.evicted && this.selection.head.line < term.evicted) this.selection = null;
        if (this.search.open && this.search.query !== '') this.#researchLater();
        this.options.onOutput?.();
        this.changed();
    }

    // ---- Size

    resize(cols: number, rows: number) {
        cols = Math.max(2, cols);
        rows = Math.max(1, rows);
        if (cols === this.#lastCols && rows === this.#lastRows) return;
        this.#lastCols = cols;
        this.#lastRows = rows;
        this.#capture(`${cols}x${rows}`, 'r');
        this.term.resize(cols, rows);
        this.pty?.resize(cols, rows);
        this.changed();
    }

    // ---- Typing

    /** Sends this to the program as if typed. */
    send(text: string) {
        if (!text || this.exitCode !== null || !this.pty) return;
        this.hasInput = true;
        this.#write(text);
    }

    #write(text: string) {
        if (text === '' || this.exitCode !== null) return;
        this.pty?.write(text);
    }

    key(e: KeyInfo): boolean {
        const bytes = encodeKey(e, this.term);
        if (bytes === null) return false;
        if (e.type === 'keyup') {
            this.send(bytes);
            return true;
        }
        this.acknowledge();
        if (this.scrolledTop !== null) this.scrollToBottom();
        if (this.selection && !this.copyMode) this.clearSelection();
        this.send(bytes);
        return true;
    }

    paste(text: string) {
        if (text === '') return;
        if (this.scrolledTop !== null) this.scrollToBottom();
        this.acknowledge();
        this.send(encodePaste(text, this.term));
    }

    commitText(text: string) {
        if (!text) return;
        this.scrollToBottom();
        this.acknowledge();
        this.send(encodeCommittedText(text, this.term.kittyFlags));
    }

    mouse(e: MouseEvent): boolean {
        const text = encodeMouse(this.term, e);
        if (text === null) return false;
        this.send(text);
        return true;
    }

    focus(on: boolean) {
        if (this.focused === on) return;
        this.focused = on;
        if (on) this.acknowledge();
        if (this.term.focusEvents) this.#write(on ? '\x1b[I' : '\x1b[O');
        this.changed();
    }

    requestAttention(message: string) {
        this.attention = message.slice(0, 120);
        this.changed();
        this.options.onAttention?.();
    }

    acknowledge() {
        if (!this.attention) return;
        this.attention = '';
        this.changed();
        this.options.onAttention?.();
    }

    refreshOptions() {
        const settings = this.options.settings?.();
        if (!settings) return;
        this.term.graphics = settings.graphics;
        this.term.kittyKeyboard = settings.kittyKeyboard;
        if (settings.replaySeconds === 0) this.#replay = null;
        if (!settings.paneStatus) this.#gitRequest = '';
        else this.updateGit();
        if (settings.triggers !== this.#rules) {
            this.#rules = settings.triggers;
            this.#triggers = settings.triggers.flatMap((rule) => {
                const regex = compileTrigger(rule);
                return regex ? [{ rule, regex }] : [];
            });
            this.#hasLineHook = this.#triggers.length > 0;
            this.#seen = new WeakMap();
            this.#updateLineHook();
        }
        this.changed();
    }

    #updateLineHook() {
        this.term.onLine = this.#hasLineHook || [...this.#shellObservers].some((observer) => observer.line)
            ? (line, number) => this.#observeLine(line, number, false) : null;
    }

    #observeLine(line: Line, number: number, partial: boolean) {
        if (this.term.alternate) return;
        if (this.#hasLineHook) this.#match(line, partial);
        for (const observer of this.#shellObservers) observer.line?.(line, number, partial);
    }

    #match(line: Line, partial: boolean) {
        const text = this.term.textOf(line).slice(0, 1024);
        if (!text) return;
        let seen = this.#seen.get(line);
        if (!seen) {
            seen = { version: -1, rules: new Set() };
            this.#seen.set(line, seen);
        }
        if (partial && seen.version === line.ver) return;
        seen.version = line.ver;
        this.#triggers.forEach(({ rule, regex }, index) => {
            if ((partial && !rule.partial) || seen.rules.has(index) || !regex.test(text)) return;
            seen.rules.add(index);
            if (rule.action === 'highlight') this.highlights.set(line, line.ver);
            else if (rule.action === 'attention') this.requestAttention(text);
            else if (Date.now() - this.#notifiedAt > 2500) {
                this.#notifiedAt = Date.now();
                if (this.options.settings?.().attentionOn !== 'off') this.requestAttention(text);
                this.options.onNotify?.(this, 'Terminal trigger', text);
            }
        });
    }

    updateGit(refresh = false) {
        if (!this.options.settings?.().paneStatus) return;
        const cwd = this.cwd;
        const host = this.term.host;
        const request = host + '\0' + cwd;
        if (request === this.#gitRequest && !refresh) return;
        if (request !== this.#gitRequest) this.gitBranch = '';
        this.#gitRequest = request;
        const epoch = ++this.#gitEpoch;
        if (!/^(?:[A-Za-z]:[\\/]|\\\\|\/)/.test(cwd) || (this.options.host || (host && host.toLowerCase() !== os.hostname().toLowerCase()))) return;
        void branchAt(cwd).then((branch) => {
            if (this.#disposed || this.#gitRequest !== request || this.#gitEpoch !== epoch) return;
            this.gitBranch = branch;
            this.changed();
        });
    }

    // ---- Scrolling

    get viewRows(): number {
        return this.term.rows;
    }

    /** The number of the first line in view. */
    get topLine(): number {
        const term = this.term;
        const bottomTop = term.evicted + term.scrollbackLength;
        if (term.alternate) return bottomTop;
        return this.scrolledTop === null ? bottomTop : Math.max(term.evicted, Math.min(this.scrolledTop, bottomTop));
    }

    get atBottom(): boolean {
        return this.scrolledTop === null || this.topLine >= this.term.evicted + this.term.scrollbackLength;
    }

    /** How many lines back from the bottom the view is. */
    get offset(): number {
        return this.term.evicted + this.term.scrollbackLength - this.topLine;
    }

    scrollTo(top: number) {
        const term = this.term;
        const bottomTop = term.evicted + term.scrollbackLength;
        const clamped = Math.max(term.evicted, Math.min(top, bottomTop));
        this.scrolledTop = clamped >= bottomTop ? null : clamped;
        this.changed();
    }

    scrollBy(lines: number) {
        if (this.term.alternate) return;
        this.scrollTo(this.topLine + lines);
    }

    scrollToBottom() {
        if (this.scrolledTop !== null) {
            this.scrolledTop = null;
            this.changed();
        }
    }

    /** Scrolls so this line is in view, in the middle if it was off screen. */
    reveal(line: number, middle = true) {
        const top = this.topLine;
        const rows = this.term.rows;
        if (line >= top && line < top + rows) return;
        this.scrollTo(middle ? line - Math.floor(rows / 2) : line);
    }

    // ---- Lines

    lineNumbered(line: number): Line | null {
        return this.term.lineNumbered(line);
    }

    // ---- Selection

    clearSelection() {
        if (this.selection) {
            this.selection = null;
            this.changed();
        }
    }

    startSelection(pos: Pos, mode: SelectionMode) {
        this.selection = { anchor: pos, head: pos, mode };
        if (mode === 'word') this.#snapWord();
        this.changed();
    }

    extendSelection(pos: Pos) {
        const selection = this.selection;
        if (!selection) return;
        selection.head = pos;
        this.changed();
    }

    selectAll() {
        const term = this.term;
        const first = term.evicted;
        const last = term.evicted + term.lineCount - 1;
        this.selection = { anchor: { line: first, col: 0 }, head: { line: last, col: term.cols - 1 }, mode: 'cell' };
        this.changed();
    }

    #snapWord() {
        const selection = this.selection;
        if (!selection) return;
        const line = this.lineNumbered(selection.anchor.line);
        if (!line) return;
        const [from, to] = wordAround(this.term, line, selection.anchor.col);
        selection.anchor = { line: selection.anchor.line, col: from };
        selection.head = { line: selection.anchor.line, col: to };
    }

    /** What's selected in a line: the columns from and up to, or null. */
    selectionSpan(lineNumber: number): [number, number] | null {
        const selection = this.selection;
        if (!selection) return null;
        return spanOf(selection, lineNumber, this.term.cols);
    }

    selectionText(): string {
        const selection = this.selection;
        if (!selection) return '';
        return this.textBetween(selection);
    }

    /** The text of a selection or any range, with a line break where a line ended rather than wrapped. */
    textBetween(selection: Selection): string {
        const [a, b] = ordered(selection.anchor, selection.head);
        const parts: string[] = [];
        for (let n = a.line; n <= b.line; n++) {
            const line = this.lineNumbered(n);
            if (!line) continue;
            const span = spanOf(selection, n, this.term.cols);
            if (!span) continue;
            const reachesEnd = span[1] >= this.term.cols;
            let text = this.term.textOf(line, span[0], span[1]);
            // The cells past a line's text aren't part of what's copied.
            if (reachesEnd) text = text.replace(/ +$/, '');
            parts.push(text);
            if (n < b.line && !(line.wrapped && selection.mode !== 'rect')) parts.push('\n');
        }
        return parts.join('');
    }

    // ---- Commands

    get commands(): Command[] {
        return this.term.commands;
    }

    /** The command whose prompt is nearest above this line, or null. */
    commandAt(line: number): Command | null {
        const list = this.term.commands;
        let found: Command | null = null;
        for (const command of list) {
            if (command.promptLine <= line) found = command;
            else break;
        }
        return found;
    }

    jumpPrompt(direction: -1 | 1) {
        const list = this.term.commands;
        const top = this.topLine;
        let target: Command | undefined;
        if (direction < 0) {
            for (let i = list.length - 1; i >= 0; i--) {
                if (list[i].promptLine < top) {
                    target = list[i];
                    break;
                }
            }
        } else {
            target = list.find((command) => command.promptLine > top);
        }
        if (!target) {
            if (direction > 0) this.scrollToBottom();
            return;
        }
        this.scrollTo(target.promptLine);
    }

    /** The text a command printed, which is what's wanted from "copy last output". */
    outputOf(command: Command): string {
        if (command.outputLine === null) return '';
        const end = command.endLine ?? this.term.evicted + this.term.lineCount;
        const parts: string[] = [];
        for (let n = command.outputLine; n < end; n++) {
            const line = this.lineNumbered(n);
            if (!line) continue;
            parts.push(this.term.textOf(line));
            if (!line.wrapped) parts.push('\n');
        }
        return parts.join('').replace(/\n+$/, '');
    }

    lastFinished(): Command | null {
        const list = this.term.commands;
        for (let i = list.length - 1; i >= 0; i--) if (list[i].endLine !== null && list[i].outputLine !== null) return list[i];
        return null;
    }

    selectOutput(command: Command) {
        if (command.outputLine === null) return;
        const end = (command.endLine ?? this.term.evicted + this.term.lineCount) - 1;
        const start = Math.max(this.term.evicted, command.outputLine);
        if (end < start) return;
        this.selection = { anchor: { line: start, col: 0 }, head: { line: end, col: this.term.cols }, mode: 'line' };
        this.changed();
    }

    toggleBookmark(command: Command) {
        if (this.bookmarks.has(command.id)) this.bookmarks.delete(command.id);
        else {
            if (this.bookmarks.size >= 32) this.bookmarks.delete(this.bookmarks.keys().next().value!);
            this.bookmarks.set(command.id, { command: { ...command }, output: this.outputOf(command).slice(0, 16384) });
        }
        this.changed();
    }

    toggleFold(command: Command) {
        if (command.endLine === null || command.outputLine === null || command.outputLine >= command.endLine) return;
        if (this.folds.has(command.id)) this.folds.delete(command.id);
        else this.folds.add(command.id);
        this.changed();
    }

    // ---- Search

    openSearch() {
        this.search = { ...this.search, open: true };
        if (this.search.query !== '') this.runSearch(true);
        this.changed();
    }

    closeSearch() {
        this.search = { ...this.search, open: false, matches: [], current: -1 };
        this.changed();
    }

    setSearch(patch: Partial<Pick<SearchState, 'query' | 'caseSensitive' | 'regex'>>) {
        this.search = { ...this.search, ...patch };
        this.runSearch(true);
    }

    #researchLater() {
        if (this.#searchTimer !== null) return;
        this.#searchTimer = setTimeout(() => {
            this.#searchTimer = null;
            this.runSearch(false);
        }, 250);
    }

    runSearch(jump: boolean) {
        const state = this.search;
        const options: SearchOptions = { query: state.query, caseSensitive: state.caseSensitive, regex: state.regex };
        const term = this.term;
        const result = searchLines(term, options);
        const was = state.matches[state.current];
        let current = -1;
        if (result.matches.length > 0) {
            if (!jump && was) {
                current = result.matches.findIndex((m) => m.line === was.line && m.col === was.col);
            }
            if (current < 0) {
                // The match nearest the bottom of what's in view, which is where a search usually wants to start.
                const bottom = this.topLine + term.rows - 1;
                current = result.matches.length - 1;
                for (let i = 0; i < result.matches.length; i++) {
                    if (result.matches[i].line > bottom) {
                        current = Math.max(0, i - 1);
                        break;
                    }
                }
            }
        }
        this.search = { ...state, matches: result.matches, current, error: result.error };
        if (jump && current >= 0) this.reveal(result.matches[current].line);
        this.changed();
    }

    stepSearch(direction: 1 | -1) {
        const state = this.search;
        if (state.matches.length === 0) return;
        const current = (state.current + direction + state.matches.length) % state.matches.length;
        this.search = { ...state, current };
        this.reveal(state.matches[current].line);
        this.changed();
    }

    // ---- Hints

    startHints() {
        const term = this.term;
        const top = this.topLine;
        const lines: { number: number; line: Line }[] = [];
        for (let n = top; n < top + term.rows; n++) {
            const line = term.lineNumbered(n);
            if (line) lines.push({ number: n, line });
        }
        const hints = findHints(term, lines);
        if (hints.length === 0) return false;
        this.hints = { hints, typed: '' };
        this.changed();
        return true;
    }

    stopHints() {
        if (this.hints) {
            this.hints = null;
            this.changed();
        }
    }

    // ---- Copy mode

    startCopyMode() {
        const term = this.term;
        const line = term.evicted + term.scrollbackLength + term.cursorY;
        this.copyMode = { cursor: { line, col: term.cursorX }, selecting: null, searching: null, pending: '' };
        this.changed();
    }

    stopCopyMode() {
        this.copyMode = null;
        this.selection = null;
        this.changed();
    }

    // ---- Recording

    startRecording() {
        this.#recordStart = replayClock();
        this.recording = { startedAt: Date.now(), events: [], cols: this.term.cols, rows: this.term.rows };
        this.changed();
    }

    stopRecording() {
        const recording = this.recording;
        this.recording = null;
        this.changed();
        return recording;
    }

    #capture(data: string, kind: Recorded['kind'] = 'o') {
        if (this.recording) appendEvent(this.recording.events, { t: replayClock() - this.#recordStart, data, kind }, true);
        const seconds = this.options.settings?.().replaySeconds ?? 0;
        if (!seconds) return;
        this.#replay ??= new RecentTimeline();
        this.#replay.add(this.term, data, seconds, kind);
    }

    instantReplay() {
        const seconds = this.options.settings?.().replaySeconds ?? 0;
        return seconds && this.#replay ? this.#replay.cast(this.displayName, seconds) : null;
    }

    // ---- Ending

    kill() {
        this.pty?.kill();
    }

    dispose() {
        this.#disposed = true;
        for (const observer of this.#shellObservers) observer.closed();
        this.#shellObservers.clear();
        if (this.#searchTimer !== null) clearTimeout(this.#searchTimer);
        this.pty?.dispose();
        this.pty = null;
        this.#listeners.clear();
    }
}

async function branchAt(cwd: string): Promise<string> {
    let folder = cwd;
    while (folder) {
        let git = path.join(folder, '.git');
        try {
            if ((await fs.promises.stat(git)).isFile()) {
                const manifest = (await fs.promises.readFile(git, 'utf8')).trim();
                if (!manifest.startsWith('gitdir: ')) return '';
                git = path.resolve(folder, manifest.slice(8));
            }
            const head = (await fs.promises.readFile(path.join(git, 'HEAD'), 'utf8')).trim();
            return head.startsWith('ref: refs/heads/') ? head.slice(16, 156) : /^[a-f\d]{40,64}$/i.test(head) ? head.slice(0, 8) : '';
        } catch { /* Look in the parent for the repository. */ }
        const parent = path.dirname(folder);
        if (parent === folder) break;
        folder = parent;
    }
    return '';
}

// ---- Selections, as plain functions

export function ordered(a: Pos, b: Pos): [Pos, Pos] {
    return a.line < b.line || (a.line === b.line && a.col <= b.col) ? [a, b] : [b, a];
}

/** The columns of a line a selection covers, from and up to. */
export function spanOf(selection: Selection, lineNumber: number, cols: number): [number, number] | null {
    const [a, b] = ordered(selection.anchor, selection.head);
    if (lineNumber < a.line || lineNumber > b.line) return null;
    if (selection.mode === 'rect') {
        const from = Math.min(selection.anchor.col, selection.head.col);
        const to = Math.max(selection.anchor.col, selection.head.col) + 1;
        return [from, to];
    }
    if (selection.mode === 'line') return [0, cols];
    const from = lineNumber === a.line ? a.col : 0;
    const to = lineNumber === b.line ? b.col + 1 : cols;
    return from < to ? [from, to] : null;
}

const NOT_WORD = new Set(' \t"\'`<>()[]{}|;,*'.split(''));

/** The cells of the word around a column: runs of anything that isn't a space or a bracket, so paths and addresses stay whole. */
export function wordAround(term: Terminal, line: Line, col: number): [number, number] {
    const isWord = (x: number) => {
        const ch = line.ch[x];
        if (ch === 0) return false;
        if (ch === 0xffffffff) return true;
        return !NOT_WORD.has(String.fromCodePoint(ch));
    };
    void term;
    const n = line.length;
    col = Math.max(0, Math.min(col, n - 1));
    if (!isWord(col)) {
        let to = col;
        while (to + 1 < n && !isWord(to + 1) && line.ch[to + 1] === line.ch[col]) to++;
        return [col, to];
    }
    let from = col;
    let to = col;
    while (from > 0 && isWord(from - 1)) from--;
    while (to + 1 < n && isWord(to + 1)) to++;
    return [from, to];
}
