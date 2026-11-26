// A terminal emulator: what a program prints goes in as text with escape sequences in it, and what comes out is a grid of cells
// with their colors, a scrollback, and what the program said about itself (its title, where it is, which commands it ran).
//
// It knows nothing about windows or fonts and doesn't touch rsui, so the tests run it under Node. It speaks what xterm does,
// which is what programs expect: colors from 16 to 24 bits, the alternate screen, scrolling regions, wide characters, mouse and
// focus reporting, bracketed paste, hyperlinks, synchronized output, the cursor's shapes, titles, the clipboard (OSC 52), and
// shell integration (OSC 133 and OSC 7), from which it keeps the list of commands.

import { cellWidth, widensWithEmojiSelector } from './width';
import { kittyPicture, MAX_IMAGE_DATA, sixelPicture } from './graphics';
import type { Picture } from './graphics';
import {
    BLINK, BOLD, DIM, HIDDEN, ITALIC, OVERLINE, PALETTE, RGB, REVERSE, STRIKE, StyleTable, UNDERLINE_MASK, UNDERLINE_SHIFT,
    paletteColor, rgbColor,
} from './style';

/** What a cell holds past its first character, or in the second cell of a wide one. */
export const WIDE_TAIL = 0xffffffff;

type LineCells = Uint8Array | Uint16Array | Uint32Array;

/** Read-only scrollback needs only enough bits for the largest cell value. */
function compactCells(cells: LineCells, used: number): LineCells {
    let bits = 0;
    for (let x = 0; x < used; x++) bits |= cells[x];
    const largest = bits >>> 0;
    const width = largest <= 0xff ? 1 : largest <= 0xffff ? 2 : 4;
    if (width === cells.BYTES_PER_ELEMENT) return used === cells.length ? cells : cells.slice(0, used);
    const stored = width === 1 ? new Uint8Array(used) : width === 2 ? new Uint16Array(used) : new Uint32Array(used);
    stored.set(cells.subarray(0, used));
    return stored;
}

export class Line {
    ch: LineCells;
    st: LineCells;
    /** Whether the text goes on in the next line, as when it was wrapped rather than ended. */
    wrapped = false;
    /** A number that's new whenever the line changes, for what draws it to tell. */
    ver: number;
    /** Characters that sit on a cell's, as accents do: the whole of what's in the cell, where there are any. */
    extra: Map<number, string> | null = null;

    constructor(cols: number, ver: number, st = 0) {
        this.ch = new Uint32Array(cols);
        this.st = new Uint32Array(cols);
        if (st !== 0) this.st.fill(st);
        this.ver = ver;
    }

    get length(): number {
        return this.ch.length;
    }

    /** How many cells from the left hold anything, or are not the default style. */
    used(): number {
        let n = this.ch.length;
        while (n > 0 && this.ch[n - 1] === 0 && this.st[n - 1] === 0) n--;
        return n;
    }
}

export interface Command {
    id: number;
    /** Lines are numbered from the oldest the terminal still has, as `Terminal.line(n)` and the numbers here go on after it's trimmed. */
    promptLine: number;
    /** Where what was typed starts, if the shell said. */
    inputLine: number | null;
    outputLine: number | null;
    endLine: number | null;
    exit: number | null;
    startedAt: number | null;
    endedAt: number | null;
    text: string;
    cwd: string;
}

export interface Theme {
    fg: number;
    bg: number;
    cursor: number;
    ansi: number[];
}

export type Pointer = 'none' | 'x10' | 'normal' | 'button' | 'any';

export interface Image {
    id: number;
    /** The line it starts on, numbered as commands' are. */
    line: number;
    col: number;
    cols: number;
    rows: number;
    /** What the file said it was, and its bytes as base64. */
    mime: string;
    data: string;
    name: string;
    kittyId?: number;
    placement?: number;
    z?: number;
    stretch?: boolean;
    pixels?: { width: number; height: number };
    viewport?: { x: number; y: number; w: number; h: number; fullWidth: number; fullHeight: number; offsetX: number; offsetY: number };
    clip?: { start: number; end: number };
}

const GROUND = 0;
const ESC = 1;
const ESC_INTERMEDIATE = 2;
const CSI = 3;
const OSC = 4;
const STRING = 5;
const STRING_ESC = 6;

/** The DEC special graphics set, for 0x60 to 0x7e. */
const LINE_DRAWING = '◆▒␉␌␍␊°±␤␋┘┐┌└┼⎺⎻─⎼⎽├┤┴┬│≤≥π≠£·';

const MAX_STRING = 64 * 1024 * 1024;

function blankBuffer(rows: number, cols: number, make: () => Line): Line[] {
    const lines: Line[] = [];
    for (let y = 0; y < rows; y++) lines.push(make());
    return lines;
}

interface Saved {
    x: number;
    y: number;
    fg: number;
    bg: number;
    flags: number;
    ul: number;
    wrapNext: boolean;
    origin: boolean;
    charset: number;
    g: string[];
}

class Buffer {
    lines: Line[];
    x = 0;
    y = 0;
    saved: Saved | null = null;
    keyboard = [0];
    images: Image[] = [];
    constructor(lines: Line[]) {
        this.lines = lines;
    }
}

export class Terminal {
    cols: number;
    rows: number;
    readonly styles = new StyleTable();
    theme: Theme = defaultTheme();

    // What it says about itself.
    title = '';
    cwd = '';
    bells = 0;
    progress: { state: number; value: number } = { state: 0, value: 0 };
    commands: Command[] = [];
    get images() { return this.#buf.images; }
    set images(images: Image[]) { this.#buf.images = images; }
    host = '';
    /** Palette colors the program has set. */
    palette = new Map<number, number>();

    /** Changes with everything that's drawn differently. */
    version = 0;
    /** Lines the terminal has let go of from the top of its scrollback: what makes line numbers last. */
    evicted = 0;
    scrollbackLimit: number;

    // Modes.
    appCursor = false;
    appKeypad = false;
    bracketedPaste = false;
    pointer: Pointer = 'none';
    pointerEncoding: 'x10' | 'utf8' | 'sgr' | 'urxvt' = 'x10';
    focusEvents = false;
    cursorVisible = true;
    /** 0 or 1 blinking block, 2 block, 3 blinking underline, 4 underline, 5 blinking bar, 6 bar. */
    cursorShape = 1;
    reverseScreen = false;
    synchronized = false;
    kittyKeyboard = true;
    get kittyFlags() { return this.kittyKeyboard ? this.#buf.keyboard[this.#buf.keyboard.length - 1] : 0; }
    graphics: 'all' | 'kitty' | 'sixel' | 'iterm' | 'off' = 'all';
    sixelDisplay = false;
    sixelCursorRight = false;
    get atPrompt(): boolean | null {
        if (this.#alternate) return false;
        const command = this.commands[this.commands.length - 1];
        return command ? command.inputLine !== null && command.outputLine === null && command.endLine === null : null;
    }
    get emptyPrompt(): boolean {
        return this.atPrompt === true && this.#typedText(this.commands[this.commands.length - 1]) === '';
    }
    /** The cell's size in pixels, for what a program asks. */
    cellPx = { w: 9, h: 18 };

    // What the program is told, or what's to be done.
    onReply: (text: string) => void = () => {};
    onTitle: (title: string) => void = () => {};
    onBell: () => void = () => {};
    onClipboard: (text: string) => void = () => {};
    onNotify: (title: string, body: string) => void = () => {};
    onCwd: (cwd: string) => void = () => {};
    onProgress: (state: number, value: number) => void = () => {};
    onCommand: (command: Command, change: 'prompt' | 'input' | 'output' | 'end') => void = () => {};
    onImage: (image: Image) => void = () => {};
    onSync: (on: boolean) => void = () => {};
    onLine: ((line: Line, number: number) => void) | null = null;
    #recordListeners = new Set<(kind: 'o' | 'r', data: string) => void>();
    #writingAt: number | undefined;

    observeRecording(listener: (kind: 'o' | 'r', data: string) => void) {
        this.#recordListeners.add(listener);
        return () => { this.#recordListeners.delete(listener); };
    }

    #clock = 0;
    #main: Buffer;
    #alt: Buffer;
    #buf: Buffer;
    #alternate = false;
    #scrollback: (Line | undefined)[] = [];
    #scrollbackHead = 0;
    #top = 0;
    #bottom: number;
    #tabs: Uint8Array;
    #wrapNext = false;
    #autowrap = true;
    #origin = false;
    #insert = false;
    #newline = false;
    #g: string[] = ['B', 'B', 'B', 'B'];
    #charset = 0;

    // The pen.
    #fg = 0;
    #bg = 0;
    #flags = 0;
    #ul = 0;
    #link = 0;
    #penId = 0;
    #penDirty = false;
    #links: string[] = [''];
    #linkIds = new Map<string, number>();

    // The parser.
    #state = GROUND;
    #params: number[] = [];
    #sub: number[][] = [];
    #current = -1;
    #subs: number[] = [];
    #private = '';
    #intermediate = '';
    #string = '';
    #stringKind = '';
    #lastPrinted = 0;
    #nextCommand = 1;
    #nextImage = 1;
    #utf8Lead = 0;
    /** What each string's introducer was: `]` OSC, `P` DCS, `_` APC, `^` PM, `X` SOS. */
    #introducer = '';
    #kittyImages = new Map<number, Picture>();
    #kittyTransfer: { options: Record<string, string>; chunks: string[]; size: number } | null = null;
    #nextKittyId = 1;
    #kittyNumbers = new Map<number, number>();
    #notificationParts = new Map<string, { title: string; body: string }>();

    constructor(cols: number, rows: number, scrollback = 10000) {
        this.cols = Math.max(1, cols);
        this.rows = Math.max(1, rows);
        this.scrollbackLimit = scrollback;
        this.#main = new Buffer(blankBuffer(this.rows, this.cols, () => this.#blank()));
        // Most shells stay on the main screen. Allocate alternate rows on first use.
        this.#alt = new Buffer([]);
        this.#buf = this.#main;
        this.#bottom = this.rows - 1;
        this.#tabs = new Uint8Array(this.cols);
        this.#resetTabs();
    }

    // ---- What's there, for drawing and for finding things.

    get cursorX(): number {
        return this.#buf.x;
    }
    get cursorY(): number {
        return this.#buf.y;
    }
    get cursorPendingWrap(): boolean {
        return this.#wrapNext;
    }
    get alternate(): boolean {
        return this.#alternate;
    }
    /** How many lines there are above the screen to scroll back to, which the alternate screen has none of. */
    get scrollbackLength(): number {
        return this.#alternate ? 0 : this.#scrollback.length - this.#scrollbackHead;
    }
    /** The lines there are to see, from the oldest to the last on the screen. */
    get lineCount(): number {
        return this.scrollbackLength + this.rows;
    }
    /** The number of the first line the terminal has, as commands and images count. */
    get firstLine(): number {
        return this.evicted;
    }

    /** The `index`th line from the oldest. */
    lineAt(index: number): Line {
        const back = this.scrollbackLength;
        if (index < back) return this.#scrollback[this.#scrollbackHead + index]!;
        return this.#buf.lines[Math.min(this.rows - 1, index - back)];
    }

    /** What's at this line's number, if it's still there. */
    lineNumbered(number: number): Line | null {
        const index = number - this.evicted;
        return index >= 0 && index < this.lineCount ? this.lineAt(index) : null;
    }

    link(id: number): string {
        return this.#links[id] ?? '';
    }

    /** The text of a line between two columns, with the spaces at its end left off when it goes to the end. */
    textOf(line: Line, from = 0, to = line.length): string {
        let out = '';
        const end = Math.min(to, line.length);
        for (let x = Math.max(0, from); x < end; x++) {
            const ch = line.ch[x];
            if (ch === WIDE_TAIL) continue;
            if (ch === 0) {
                out += ' ';
                continue;
            }
            const extra = line.extra?.get(x);
            out += extra ?? String.fromCodePoint(ch);
        }
        return to >= line.length ? out.replace(/ +$/, '') : out;
    }

    /** Bounded plain text from the main screen, including output still on screen. */
    transcript(maxLines: number, maxChars: number): string[] {
        const back = this.#scrollback.length - this.#scrollbackHead;
        const lines = [];
        let left = maxChars;
        let skipEmpty = true;
        for (let index = back + this.rows - 1; index >= 0 && lines.length < maxLines && left > 0; index--) {
            const line = index < back ? this.#scrollback[this.#scrollbackHead + index]! : this.#main.lines[index - back];
            const text = this.textOf(line);
            if (skipEmpty && !text) continue;
            skipEmpty = false;
            const kept = text.slice(0, left);
            lines.push(kept);
            left -= kept.length + 1;
        }
        return lines.reverse();
    }

    restoreTranscript(lines: string[]) {
        const staging = new Terminal(this.cols, 1, this.scrollbackLimit);
        for (const line of lines) staging.write(line.replace(/[\x00-\x1f\x7f-\x9f]/g, '') + '\r\n');
        for (let index = 0; index < staging.scrollbackLength; index++) {
            const line = staging.lineAt(index);
            line.ver = ++this.#clock;
            this.#pushScrollback(line);
        }
    }

    /** An independent emulator state for seeking. Event callbacks belong to the live shell. */
    fork(scrollback = this.scrollbackLimit): Terminal {
        const clone = new Terminal(this.cols, this.rows, this.scrollbackLimit);
        const line = (source: Line) => {
            const copy = new Line(0, source.ver);
            copy.ch = source.ch.slice();
            copy.st = source.st.slice();
            copy.extra = source.extra && new Map(source.extra);
            copy.wrapped = source.wrapped;
            return copy;
        };
        const buffer = (source: Buffer) => {
            const copy = new Buffer(source.lines.map(line));
            copy.x = source.x;
            copy.y = source.y;
            copy.saved = source.saved && { ...source.saved, g: [...source.saved.g] };
            copy.images = [...source.images];
            copy.keyboard = [...source.keyboard];
            return copy;
        };
        clone.#main = buffer(this.#main);
        clone.#alt = buffer(this.#alt);
        clone.#alternate = this.#alternate;
        clone.#buf = this.#alternate ? clone.#alt : clone.#main;
        const head = Math.max(this.#scrollbackHead, this.#scrollback.length - scrollback);
        clone.#scrollback = this.#scrollback.slice(head).map((entry) => line(entry!));
        clone.styles.copyFrom(this.styles);
        clone.commands = this.commands.map((command) => ({ ...command }));
        clone.palette = new Map(this.palette);
        clone.#links = [...this.#links];
        clone.#linkIds = new Map(this.#linkIds);
        clone.#tabs = this.#tabs.slice();
        clone.#g = [...this.#g];
        clone.#params = [...this.#params];
        clone.#sub = this.#sub.map((part) => [...part]);
        clone.#subs = [...this.#subs];
        clone.#kittyImages = new Map(this.#kittyImages);
        clone.#kittyNumbers = new Map(this.#kittyNumbers);
        const transfer = this.#kittyTransfer;
        clone.#kittyTransfer = transfer && { options: transfer.options, chunks: [...transfer.chunks], size: transfer.size };
        clone.#notificationParts = new Map([...this.#notificationParts].map(([id, parts]) => [id, { ...parts }]));
        const properties = ['theme', 'title', 'cwd', 'host', 'bells', 'progress', 'version', 'evicted', 'appCursor',
            'appKeypad', 'bracketedPaste', 'pointer', 'pointerEncoding', 'focusEvents', 'cursorVisible', 'cursorShape',
            'reverseScreen', 'synchronized', 'kittyKeyboard', 'graphics', 'cellPx', 'sixelDisplay', 'sixelCursorRight'] as const;
        Object.assign(clone, Object.fromEntries(properties.map((key) => [key, this[key]])));
        clone.evicted += head - this.#scrollbackHead;
        clone.scrollbackLimit = scrollback;
        if (!clone.alternate) clone.#forget();
        clone.#clock = this.#clock;
        clone.#top = this.#top;
        clone.#bottom = this.#bottom;
        clone.#wrapNext = this.#wrapNext;
        clone.#autowrap = this.#autowrap;
        clone.#origin = this.#origin;
        clone.#insert = this.#insert;
        clone.#newline = this.#newline;
        clone.#charset = this.#charset;
        clone.#fg = this.#fg;
        clone.#bg = this.#bg;
        clone.#flags = this.#flags;
        clone.#ul = this.#ul;
        clone.#link = this.#link;
        clone.#penId = this.#penId;
        clone.#penDirty = this.#penDirty;
        clone.#state = this.#state;
        clone.#current = this.#current;
        clone.#private = this.#private;
        clone.#intermediate = this.#intermediate;
        clone.#string = this.#string;
        clone.#stringKind = this.#stringKind;
        clone.#lastPrinted = this.#lastPrinted;
        clone.#nextCommand = this.#nextCommand;
        clone.#nextImage = this.#nextImage;
        clone.#utf8Lead = this.#utf8Lead;
        clone.#introducer = this.#introducer;
        clone.#nextKittyId = this.#nextKittyId;
        clone.#inputColumn = this.#inputColumn;
        return clone;
    }

    /** Portable emulator state, including an escape sequence that crosses the recording boundary. */
    snapshot(scrollback = this.scrollbackLimit) {
        const contents = (cells: LineCells) => {
            let end = cells.length;
            while (end > 0 && cells[end - 1] === 0) end--;
            return Array.from(cells.subarray(0, end));
        };
        const line = (source: Line) => ({
            size: source.length, ch: contents(source.ch), st: contents(source.st), extra: source.extra && [...source.extra],
            wrapped: source.wrapped, ver: source.ver,
        });
        const buffer = (source: Buffer) => ({
            // A recording always contains a full grid, including a pristine alternate screen.
            lines: source.lines.length ? source.lines.map(line) : Array.from({ length: this.rows }, () => ({
                size: this.cols, ch: [], st: [], extra: null, wrapped: false, ver: 0,
            })),
            x: source.x, y: source.y, saved: source.saved,
            images: source.images, keyboard: source.keyboard,
        });
        const head = Math.max(this.#scrollbackHead, this.#scrollback.length - scrollback);
        const firstLine = this.evicted + head - this.#scrollbackHead;
        return {
            cols: this.cols, rows: this.rows, scrollback: this.scrollbackLimit,
            main: buffer(this.#main), alt: buffer(this.#alt), alternate: this.#alternate,
            back: this.#scrollback.slice(head).map((entry) => line(entry!)),
            styles: this.styles.styles, commands: this.commands.filter((command) => (command.endLine ?? command.promptLine) >= firstLine),
            palette: [...this.palette], links: this.#links, tabs: Array.from(this.#tabs), g: this.#g,
            params: this.#params, sub: this.#sub, subs: this.#subs,
            kittyImages: [...this.#kittyImages], kittyNumbers: [...this.#kittyNumbers], transfer: this.#kittyTransfer,
            notifications: [...this.#notificationParts],
            modes: {
                theme: this.theme, title: this.title, cwd: this.cwd, host: this.host,
                bells: this.bells, progress: this.progress, version: this.version, evicted: firstLine,
                appCursor: this.appCursor, appKeypad: this.appKeypad, bracketedPaste: this.bracketedPaste, pointer: this.pointer,
                pointerEncoding: this.pointerEncoding, focusEvents: this.focusEvents, cursorVisible: this.cursorVisible, cursorShape: this.cursorShape,
                reverseScreen: this.reverseScreen, synchronized: this.synchronized, kittyKeyboard: this.kittyKeyboard, graphics: this.graphics,
                cellPx: this.cellPx, sixelDisplay: this.sixelDisplay, sixelCursorRight: this.sixelCursorRight,
            },
            parser: {
                clock: this.#clock, top: this.#top, bottom: this.#bottom, wrapNext: this.#wrapNext,
                autowrap: this.#autowrap, origin: this.#origin, insert: this.#insert, newline: this.#newline,
                charset: this.#charset, fg: this.#fg, bg: this.#bg, flags: this.#flags,
                ul: this.#ul, link: this.#link, penId: this.#penId, penDirty: this.#penDirty,
                state: this.#state, current: this.#current, private: this.#private, intermediate: this.#intermediate,
                string: this.#string, stringKind: this.#stringKind, lastPrinted: this.#lastPrinted, nextCommand: this.#nextCommand,
                nextImage: this.#nextImage, utf8Lead: this.#utf8Lead, introducer: this.#introducer, nextKittyId: this.#nextKittyId,
                inputColumn: this.#inputColumn,
            },
        };
    }

    static fromSnapshot(state: ReturnType<Terminal['snapshot']>): Terminal {
        const term = new Terminal(state.cols, state.rows, state.scrollback);
        const line = (source: typeof state.back[number]) => {
            if (!Number.isInteger(source.size) || source.size < 1 || source.size > 1000) throw new Error('Invalid recording line width');
            const value = new Line(source.size, source.ver);
            value.ch.set(source.ch);
            value.st.set(source.st);
            value.extra = source.extra && new Map(source.extra);
            value.wrapped = source.wrapped;
            return value;
        };
        const buffer = (source: typeof state.main) => {
            const value = new Buffer(source.lines.map(line));
            value.x = source.x;
            value.y = source.y;
            value.saved = source.saved && { ...source.saved, g: [...source.saved.g] };
            value.images = [...source.images];
            value.keyboard = [...source.keyboard];
            return value;
        };
        term.#main = buffer(state.main);
        term.#alt = buffer(state.alt);
        term.#alternate = state.alternate;
        term.#buf = state.alternate ? term.#alt : term.#main;
        term.#scrollback = state.back.map((source) => {
            const restored = line(source);
            restored.ch = compactCells(restored.ch, restored.length);
            restored.st = compactCells(restored.st, restored.length);
            return restored;
        });
        for (const { fg, bg, flags, ul, link } of state.styles) term.styles.intern(fg, bg, flags, ul, link);
        term.commands = state.commands.map((command) => ({ ...command }));
        term.palette = new Map(state.palette);
        term.#links = [...state.links];
        term.#linkIds = new Map(state.links.map((link, id) => [link, id] as const).slice(1));
        term.#tabs = Uint8Array.from(state.tabs);
        term.#g = [...state.g];
        term.#params = [...state.params];
        term.#sub = state.sub.map((group) => [...group]);
        term.#subs = [...state.subs];
        term.#kittyImages = new Map(state.kittyImages);
        term.#kittyNumbers = new Map(state.kittyNumbers);
        const transfer = state.transfer;
        term.#kittyTransfer = transfer && { ...transfer, chunks: [...transfer.chunks] };
        term.#notificationParts = new Map(state.notifications.map(([key, value]) => [key, { ...value }]));
        for (const key of ['theme', 'title', 'cwd', 'host', 'bells', 'progress', 'version', 'evicted', 'appCursor',
            'appKeypad', 'bracketedPaste', 'pointer', 'pointerEncoding', 'focusEvents', 'cursorVisible', 'cursorShape',
            'reverseScreen', 'synchronized', 'kittyKeyboard', 'graphics', 'cellPx', 'sixelDisplay', 'sixelCursorRight'] as const) {
            Object.assign(term, { [key]: state.modes[key] });
        }
        term.#clock = state.parser.clock;
        term.#top = state.parser.top;
        term.#bottom = state.parser.bottom;
        term.#wrapNext = state.parser.wrapNext;
        term.#autowrap = state.parser.autowrap;
        term.#origin = state.parser.origin;
        term.#insert = state.parser.insert;
        term.#newline = state.parser.newline;
        term.#charset = state.parser.charset;
        term.#fg = state.parser.fg;
        term.#bg = state.parser.bg;
        term.#flags = state.parser.flags;
        term.#ul = state.parser.ul;
        term.#link = state.parser.link;
        term.#penId = state.parser.penId;
        term.#penDirty = state.parser.penDirty;
        term.#state = state.parser.state;
        term.#current = state.parser.current;
        term.#private = state.parser.private;
        term.#intermediate = state.parser.intermediate;
        term.#string = state.parser.string;
        term.#stringKind = state.parser.stringKind;
        term.#lastPrinted = state.parser.lastPrinted;
        term.#nextCommand = state.parser.nextCommand;
        term.#nextImage = state.parser.nextImage;
        term.#utf8Lead = state.parser.utf8Lead;
        term.#introducer = state.parser.introducer;
        term.#nextKittyId = state.parser.nextKittyId;
        term.#inputColumn = state.parser.inputColumn;
        return term;
    }


    // ---- Changing size

    resize(cols: number, rows: number) {
        cols = Math.max(1, Math.floor(cols));
        rows = Math.max(1, Math.floor(rows));
        if (cols === this.cols && rows === this.rows) return;
        for (const record of this.#recordListeners) record('r', `${cols}x${rows}`);
        const wasAlt = this.#alternate;
        for (const buffer of [this.#main, this.#alt]) {
            if (buffer.lines.length === 0) continue;
            const main = buffer === this.#main;
            // Fewer rows: what's below the cursor, if it's empty, goes first, and then the top goes into the scrollback.
            while (buffer.lines.length > rows) {
                const last = buffer.lines[buffer.lines.length - 1];
                if (buffer.y < buffer.lines.length - 1 && last.used() === 0) {
                    buffer.lines.pop();
                } else {
                    const top = buffer.lines.shift()!;
                    if (main) this.#pushScrollback(top);
                    buffer.y = Math.max(0, buffer.y - 1);
                }
            }
            while (buffer.lines.length < rows) buffer.lines.push(this.#blankWith(cols, 0));
            if (cols !== this.cols) {
                buffer.lines = buffer.lines.map((line) => this.#refit(line, cols));
            }
            buffer.x = Math.min(buffer.x, cols - 1);
            buffer.y = Math.min(buffer.y, rows - 1);
        }
        void wasAlt;
        this.cols = cols;
        this.rows = rows;
        this.#top = 0;
        this.#bottom = rows - 1;
        this.#wrapNext = false;
        const tabs = new Uint8Array(cols);
        tabs.set(this.#tabs.subarray(0, Math.min(cols, this.#tabs.length)));
        for (let x = this.#tabs.length; x < cols; x++) tabs[x] = x % 8 === 0 ? 1 : 0;
        this.#tabs = tabs;
        this.version++;
    }

    #refit(line: Line, cols: number): Line {
        if (line.length === cols) return line;
        const next = new Line(cols, ++this.#clock);
        const n = Math.min(cols, line.length);
        next.ch.set(line.ch.subarray(0, n));
        next.st.set(line.st.subarray(0, n));
        next.wrapped = line.wrapped;
        if (line.extra) {
            next.extra = new Map([...line.extra].filter(([x]) => x < cols));
        }
        // A wide character cut in half is its first half alone.
        if (n < line.length && n > 0 && next.ch[n - 1] !== 0 && line.ch[n] === WIDE_TAIL) next.ch[n - 1] = 0;
        return next;
    }

    // ---- Taking in what a program prints

    write(data: string, recordedAt?: number) {
        this.#writingAt = recordedAt;
        const n = data.length;
        let i = 0;
        this.version++;
        while (i < n) {
            if (this.#state === GROUND) {
                i = this.#ground(data, i, n);
                continue;
            }
            const c = data.charCodeAt(i);
            switch (this.#state) {
                case ESC:
                    i++;
                    this.#escape(c);
                    break;
                case ESC_INTERMEDIATE:
                    i++;
                    this.#escapeWithIntermediate(c);
                    break;
                case CSI:
                    i++;
                    this.#csiByte(c);
                    break;
                case OSC:
                case STRING:
                    i = this.#stringBytes(data, i, n);
                    break;
                case STRING_ESC:
                    i++;
                    if (c === 0x5c) {
                        this.#endString();
                    } else {
                        // ESC began something else, and what was being read is lost.
                        this.#state = ESC;
                        this.#string = '';
                        this.#escape(c);
                    }
                    break;
                default:
                    this.#state = GROUND;
            }
        }
        for (const record of this.#recordListeners) record('o', data);
    }

    #ground(data: string, start: number, n: number): number {
        let i = start;
        while (i < n) {
            const c = data.charCodeAt(i);
            if (c >= 0x20 && c !== 0x7f) {
                // A run of printable text goes in without the parser's help while it fits on the line.
                if (c < 0x7f && this.#g[this.#charset] === 'B' && !this.#insert && !this.#wrapNext) {
                    const buffer = this.#buf;
                    const line = buffer.lines[buffer.y];
                    let x = buffer.x;
                    const cols = this.cols;
                    const st = this.#pen();
                    let wrote = false;
                    while (i < n && x < cols) {
                        const d = data.charCodeAt(i);
                        if (d < 0x20 || d >= 0x7f) break;
                        // Overwriting half of a wide character leaves the other half a blank.
                        if (line.ch[x] === WIDE_TAIL || (x + 1 < cols && line.ch[x + 1] === WIDE_TAIL)) this.#breakWide(line, x);
                        line.ch[x] = d;
                        line.st[x] = st;
                        if (line.extra !== null) line.extra.delete(x);
                        this.#lastPrinted = d;
                        x++;
                        i++;
                        wrote = true;
                    }
                    if (wrote) {
                        line.ver = ++this.#clock;
                        if (x >= cols) {
                            buffer.x = cols - 1;
                            this.#wrapNext = true;
                        } else {
                            buffer.x = x;
                        }
                    }
                    if (wrote) continue;
                }
                let cp = c;
                if (c >= 0xd800 && c < 0xdc00 && i + 1 < n) {
                    const low = data.charCodeAt(i + 1);
                    if (low >= 0xdc00 && low < 0xe000) {
                        cp = 0x10000 + ((c - 0xd800) << 10) + (low - 0xdc00);
                        i++;
                    }
                }
                i++;
                this.#print(cp);
                continue;
            }
            i++;
            if (c === 0x1b) {
                this.#state = ESC;
                return i;
            }
            this.#control(c);
        }
        return i;
    }

    #control(c: number) {
        switch (c) {
            case 0x07:
                this.bells++;
                this.onBell();
                break;
            case 0x08:
                if (this.#buf.x > 0) this.#buf.x--;
                this.#wrapNext = false;
                break;
            case 0x09:
                this.#tab(1);
                break;
            case 0x0a:
            case 0x0b:
            case 0x0c:
                this.#lineFeed();
                if (this.#newline) this.#buf.x = 0;
                break;
            case 0x0d:
                this.#buf.x = 0;
                this.#wrapNext = false;
                break;
            case 0x0e:
                this.#charset = 1;
                break;
            case 0x0f:
                this.#charset = 0;
                break;
            case 0x18:
            case 0x1a:
                this.#state = GROUND;
                break;
        }
    }

    // ---- Printing

    #pen(): number {
        if (this.#penDirty) {
            this.#penId = this.styles.intern(this.#fg, this.#bg, this.#flags, this.#ul, this.#link);
            this.#penDirty = false;
        }
        return this.#penId;
    }

    /** The style a cell gets that's cleared: the pen's background alone. */
    #eraseStyle(): number {
        return this.#bg === 0 ? 0 : this.styles.intern(0, this.#bg, 0, 0, 0);
    }

    #blank(): Line {
        return new Line(this.cols, ++this.#clock, this.#eraseStyle());
    }

    #blankWith(cols: number, st: number): Line {
        return new Line(cols, ++this.#clock, st);
    }

    /** Fixes what's left of a wide character when one of its halves is written over. */
    #breakWide(line: Line, x: number) {
        if (line.ch[x] === WIDE_TAIL && x > 0) {
            line.ch[x - 1] = 0;
            line.extra?.delete(x - 1);
        }
        if (x + 1 < line.length && line.ch[x + 1] === WIDE_TAIL) {
            line.ch[x + 1] = 0;
        }
    }

    #print(cp: number) {
        // Charsets: DEC special graphics turns lowercase letters into lines.
        if (cp >= 0x60 && cp <= 0x7e && this.#g[this.#charset] === '0') cp = LINE_DRAWING.codePointAt(cp - 0x60)!;
        else if (cp < 0x7f && this.#g[this.#charset] === 'A' && cp === 0x23) cp = 0xa3;
        const w = cellWidth(cp);
        const buffer = this.#buf;
        if (w === 0) {
            this.#attach(cp);
            return;
        }
        if (this.#wrapNext) {
            if (this.#autowrap) {
                buffer.lines[buffer.y].wrapped = true;
                this.#buf.x = 0;
                this.#lineFeed();
            }
            this.#wrapNext = false;
        }
        let x = buffer.x;
        if (w === 2 && x === this.cols - 1) {
            if (this.#autowrap) {
                // Not room for both halves: the cell's left blank and it goes on the next line.
                const line = buffer.lines[buffer.y];
                this.#breakWide(line, x);
                line.ch[x] = 0;
                line.wrapped = true;
                buffer.x = 0;
                this.#lineFeed();
                x = 0;
            } else {
                return;
            }
        }
        const y = buffer.y;
        const line = buffer.lines[y];
        if (this.#insert) {
            const shift = w;
            for (let k = this.cols - 1; k >= x + shift; k--) {
                line.ch[k] = line.ch[k - shift];
                line.st[k] = line.st[k - shift];
            }
            line.extra = null;
        }
        if (line.ch[x] === WIDE_TAIL || (x + 1 < this.cols && line.ch[x + 1] === WIDE_TAIL)) this.#breakWide(line, x);
        if (w === 2 && x + 2 < this.cols && line.ch[x + 2] === WIDE_TAIL) line.ch[x + 2] = 0;
        const st = this.#pen();
        line.ch[x] = cp;
        line.st[x] = st;
        line.extra?.delete(x);
        if (w === 2) {
            line.ch[x + 1] = WIDE_TAIL;
            line.st[x + 1] = st;
        }
        line.ver = ++this.#clock;
        this.#lastPrinted = cp;
        x += w;
        if (x >= this.cols) {
            buffer.x = this.cols - 1;
            this.#wrapNext = true;
        } else {
            buffer.x = x;
        }
    }

    /** A mark, a joiner or a variation selector: it goes into the cell before. */
    #attach(cp: number) {
        const buffer = this.#buf;
        let x = this.#wrapNext ? buffer.x : buffer.x - 1;
        const line = buffer.lines[buffer.y];
        if (x < 0) return;
        if (line.ch[x] === WIDE_TAIL && x > 0) x--;
        const base = line.ch[x];
        if (base === 0 || base === WIDE_TAIL) return;
        if (!line.extra) line.extra = new Map();
        const was = line.extra.get(x) ?? String.fromCodePoint(base);
        line.extra.set(x, was + String.fromCodePoint(cp));
        line.ver = ++this.#clock;
        // The emoji selector makes a text symbol two cells wide.
        if (cp === 0xfe0f && cellWidth(base) === 1 && widensWithEmojiSelector(base) && x + 1 < this.cols && line.ch[x + 1] !== WIDE_TAIL) {
            const st = line.st[x];
            if (!this.#wrapNext && x + 1 < this.cols) {
                line.ch[x + 1] = WIDE_TAIL;
                line.st[x + 1] = st;
                buffer.x = Math.min(this.cols - 1, x + 2);
                if (x + 2 >= this.cols) this.#wrapNext = true;
            }
        }
    }

    // ---- Moving about

    #tab(count: number) {
        const buffer = this.#buf;
        for (let k = 0; k < count; k++) {
            let x = buffer.x + 1;
            while (x < this.cols - 1 && !this.#tabs[x]) x++;
            buffer.x = Math.min(x, this.cols - 1);
        }
        this.#wrapNext = false;
    }

    #backTab(count: number) {
        const buffer = this.#buf;
        for (let k = 0; k < count; k++) {
            let x = buffer.x - 1;
            while (x > 0 && !this.#tabs[x]) x--;
            buffer.x = Math.max(x, 0);
        }
        this.#wrapNext = false;
    }

    #resetTabs() {
        for (let x = 0; x < this.cols; x++) this.#tabs[x] = x % 8 === 0 ? 1 : 0;
    }

    #lineFeed() {
        const buffer = this.#buf;
        this.onLine?.(buffer.lines[buffer.y], this.#absLine());
        if (buffer.y === this.#bottom) {
            this.#scrollUp(1);
        } else if (buffer.y < this.rows - 1) {
            buffer.y++;
        }
        this.#wrapNext = false;
    }

    #reverseIndex() {
        const buffer = this.#buf;
        if (buffer.y === this.#top) {
            this.#scrollDown(1);
        } else if (buffer.y > 0) {
            buffer.y--;
        }
        this.#wrapNext = false;
    }

    #pushScrollback(line: Line) {
        if (this.scrollbackLimit <= 0) {
            this.evicted++;
            return;
        }
        // Past what's there to be seen, a line is kept in just the cells it uses.
        const used = line.used();
        if (used < line.length && line.extra) line.extra = new Map([...line.extra].filter(([x]) => x < used));
        line.ch = compactCells(line.ch, used);
        line.st = compactCells(line.st, used);
        this.#scrollback.push(line);
        if (this.#scrollback.length - this.#scrollbackHead > this.scrollbackLimit) {
            // The index can wait for compaction, but an evicted line's cell buffers can be freed now.
            this.#scrollback[this.#scrollbackHead++] = undefined;
            this.evicted++;
            if (this.#scrollbackHead > 4096 && this.#scrollbackHead * 2 > this.#scrollback.length) {
                this.#scrollback = this.#scrollback.slice(this.#scrollbackHead);
                this.#scrollbackHead = 0;
            }
            this.#forget();
        }
    }

    /** Lets go of commands and pictures that are beyond the oldest line. */
    #forget() {
        if (this.commands.length > 0 && this.commands[0].promptLine < this.evicted) {
            let k = 0;
            while (k < this.commands.length && (this.commands[k].endLine ?? this.commands[k].promptLine) < this.evicted) k++;
            if (k > 0) this.commands.splice(0, k);
        }
        if (this.images.length > 0) {
            this.images = this.images.filter((image) => image.line + image.rows > this.evicted);
        }
    }

    #scrollUp(count: number) {
        const buffer = this.#buf;
        const region = this.#bottom - this.#top + 1;
        count = Math.min(count, region);
        const whole = this.#top === 0 && this.#bottom === this.rows - 1 && !this.#alternate;
        if (!whole && this.images.length) this.#scrollImages(-count);
        for (let k = 0; k < count; k++) {
            const top = buffer.lines.splice(this.#top, 1)[0];
            if (whole) this.#pushScrollback(top);
            buffer.lines.splice(this.#bottom, 0, this.#blank());
        }
        this.version++;
    }

    #scrollDown(count: number) {
        const buffer = this.#buf;
        const region = this.#bottom - this.#top + 1;
        count = Math.min(count, region);
        if (this.images.length) this.#scrollImages(count);
        for (let k = 0; k < count; k++) {
            buffer.lines.splice(this.#bottom, 1);
            buffer.lines.splice(this.#top, 0, this.#blank());
        }
    }

    #scrollImages(amount: number) {
        const screen = this.evicted + this.scrollbackLength;
        const top = screen + this.#top;
        const bottom = screen + this.#bottom + 1;
        this.images = this.images.flatMap((image) => {
            const start = image.clip?.start ?? 0;
            const end = image.clip?.end ?? image.rows;
            if (image.line + start < top || image.line + end > bottom) return [image];
            const line = image.line + amount;
            const clipped = { start: Math.max(start, top - line), end: Math.min(end, bottom - line) };
            return clipped.start < clipped.end ? [{ ...image, line, clip: clipped }] : [];
        });
    }

    #moveTo(x: number, y: number) {
        const buffer = this.#buf;
        buffer.x = Math.max(0, Math.min(this.cols - 1, x));
        if (this.#origin) {
            buffer.y = Math.max(this.#top, Math.min(this.#bottom, y + this.#top));
        } else {
            buffer.y = Math.max(0, Math.min(this.rows - 1, y));
        }
        this.#wrapNext = false;
    }

    // ---- Escape sequences

    #clearParams() {
        this.#params = [];
        this.#sub = [];
        this.#current = -1;
        this.#subs = [];
        this.#private = '';
        this.#intermediate = '';
    }

    #escape(c: number) {
        const ch = String.fromCharCode(c);
        this.#state = GROUND;
        switch (ch) {
            case '[':
                this.#clearParams();
                this.#state = CSI;
                return;
            case ']':
                this.#string = '';
                this.#introducer = ']';
                this.#state = OSC;
                return;
            case 'P':
            case '_':
            case '^':
            case 'X':
                this.#string = '';
                this.#introducer = ch;
                this.#state = STRING;
                return;
            case '7':
                this.#saveCursor();
                return;
            case '8':
                this.#restoreCursor();
                return;
            case 'D':
                this.#lineFeed();
                return;
            case 'E':
                this.#buf.x = 0;
                this.#lineFeed();
                return;
            case 'H':
                this.#tabs[this.#buf.x] = 1;
                return;
            case 'M':
                this.#reverseIndex();
                return;
            case 'c':
                this.#reset();
                return;
            case '=':
                this.appKeypad = true;
                return;
            case '>':
                this.appKeypad = false;
                return;
            case '\\':
                return;
            case '(':
            case ')':
            case '*':
            case '+':
            case '#':
            case '%':
            case ' ':
                this.#intermediate = ch;
                this.#state = ESC_INTERMEDIATE;
                return;
        }
        if (c < 0x20) this.#control(c);
    }

    #escapeWithIntermediate(c: number) {
        const ch = String.fromCharCode(c);
        const which = this.#intermediate;
        this.#state = GROUND;
        if (which === '(' || which === ')' || which === '*' || which === '+') {
            const slot = '()*+'.indexOf(which);
            this.#g[slot] = ch === '0' || ch === 'A' ? ch : 'B';
        } else if (which === '#' && ch === '8') {
            // The alignment test fills the screen with E.
            for (let y = 0; y < this.rows; y++) {
                const line = this.#buf.lines[y];
                line.ch.fill(0x45);
                line.st.fill(0);
                line.ver = ++this.#clock;
            }
        }
    }

    #saveCursor() {
        const buffer = this.#buf;
        buffer.saved = {
            x: buffer.x, y: buffer.y, fg: this.#fg, bg: this.#bg, flags: this.#flags, ul: this.#ul, wrapNext: this.#wrapNext,
            origin: this.#origin, charset: this.#charset, g: [...this.#g],
        };
    }

    #restoreCursor() {
        const buffer = this.#buf;
        const saved = buffer.saved;
        if (!saved) {
            this.#moveTo(0, 0);
            return;
        }
        buffer.x = Math.min(saved.x, this.cols - 1);
        buffer.y = Math.min(saved.y, this.rows - 1);
        this.#fg = saved.fg;
        this.#bg = saved.bg;
        this.#flags = saved.flags;
        this.#ul = saved.ul;
        this.#penDirty = true;
        this.#wrapNext = saved.wrapNext;
        this.#origin = saved.origin;
        this.#charset = saved.charset;
        this.#g = [...saved.g];
    }

    #reset() {
        this.#fg = this.#bg = this.#flags = this.#ul = this.#link = 0;
        this.#penDirty = true;
        this.#leaveAlternate(false);
        this.#main.keyboard = [0];
        this.#alt.keyboard = [0];
        this.#main.images = [];
        this.#alt.images = [];
        this.#kittyImages.clear();
        this.#kittyNumbers.clear();
        this.#kittyTransfer = null;
        this.#top = 0;
        this.#bottom = this.rows - 1;
        this.#g = ['B', 'B', 'B', 'B'];
        this.#charset = 0;
        this.#origin = false;
        this.#autowrap = true;
        this.#insert = false;
        this.#newline = false;
        this.appCursor = false;
        this.appKeypad = false;
        this.bracketedPaste = false;
        this.pointer = 'none';
        this.pointerEncoding = 'x10';
        this.focusEvents = false;
        this.cursorVisible = true;
        this.cursorShape = 1;
        this.reverseScreen = false;
        this.sixelDisplay = this.sixelCursorRight = false;
        this.#resetTabs();
        this.#buf.saved = null;
        this.#eraseDisplay(2);
        this.#moveTo(0, 0);
    }

    // ---- CSI

    #csiByte(c: number) {
        if (c >= 0x30 && c <= 0x39) {
            this.#current = (this.#current < 0 ? 0 : this.#current) * 10 + (c - 0x30);
            if (this.#current > 0x7fffffff) this.#current = 0x7fffffff;
            return;
        }
        if (c === 0x3b || c === 0x3a) {
            if (c === 0x3a) {
                this.#subs.push(this.#current);
            } else {
                this.#subs.push(this.#current);
                this.#sub.push(this.#subs);
                this.#params.push(this.#subs[0]);
                this.#subs = [];
            }
            this.#current = -1;
            return;
        }
        if (c >= 0x3c && c <= 0x3f) {
            this.#private += String.fromCharCode(c);
            return;
        }
        if (c >= 0x20 && c <= 0x2f) {
            this.#intermediate += String.fromCharCode(c);
            return;
        }
        if (c >= 0x40 && c <= 0x7e) {
            this.#subs.push(this.#current);
            this.#sub.push(this.#subs);
            this.#params.push(this.#subs[0]);
            this.#state = GROUND;
            this.#csi(String.fromCharCode(c));
            return;
        }
        if (c === 0x1b) {
            this.#state = ESC;
            return;
        }
        if (c === 0x18 || c === 0x1a) {
            this.#state = GROUND;
            return;
        }
        if (c < 0x20) this.#control(c);
    }

    /** Parameter `k`, or `or` when it's missing or 0 where 0 means missing. */
    #p(k: number, or: number): number {
        const v = this.#params[k];
        return v === undefined || v < 0 || v === 0 ? or : v;
    }

    /** Parameter `k`, where 0 is 0. */
    #p0(k: number, or: number): number {
        const v = this.#params[k];
        return v === undefined || v < 0 ? or : v;
    }

    #csi(final: string) {
        const buffer = this.#buf;
        const priv = this.#private;
        const inter = this.#intermediate;
        if (priv === '' && inter === '') {
            switch (final) {
                case '@': {
                    const n = Math.min(this.#p(0, 1), this.cols - buffer.x);
                    const line = buffer.lines[buffer.y];
                    for (let x = this.cols - 1; x >= buffer.x + n; x--) {
                        line.ch[x] = line.ch[x - n];
                        line.st[x] = line.st[x - n];
                    }
                    this.#eraseCells(line, buffer.x, buffer.x + n);
                    line.extra = null;
                    this.#wrapNext = false;
                    return;
                }
                case 'A':
                    buffer.y = Math.max(buffer.y >= this.#top ? this.#top : 0, buffer.y - this.#p(0, 1));
                    this.#wrapNext = false;
                    return;
                case 'B':
                case 'e':
                    buffer.y = Math.min(buffer.y <= this.#bottom ? this.#bottom : this.rows - 1, buffer.y + this.#p(0, 1));
                    this.#wrapNext = false;
                    return;
                case 'C':
                case 'a':
                    buffer.x = Math.min(this.cols - 1, buffer.x + this.#p(0, 1));
                    this.#wrapNext = false;
                    return;
                case 'D':
                    buffer.x = Math.max(0, buffer.x - this.#p(0, 1));
                    this.#wrapNext = false;
                    return;
                case 'E':
                    buffer.x = 0;
                    buffer.y = Math.min(this.#bottom, buffer.y + this.#p(0, 1));
                    this.#wrapNext = false;
                    return;
                case 'F':
                    buffer.x = 0;
                    buffer.y = Math.max(this.#top, buffer.y - this.#p(0, 1));
                    this.#wrapNext = false;
                    return;
                case 'G':
                case '`':
                    buffer.x = Math.min(this.cols - 1, this.#p(0, 1) - 1);
                    this.#wrapNext = false;
                    return;
                case 'H':
                case 'f':
                    this.#moveTo(this.#p(1, 1) - 1, this.#p(0, 1) - 1);
                    return;
                case 'I':
                    this.#tab(this.#p(0, 1));
                    return;
                case 'J':
                    this.#eraseDisplay(this.#p0(0, 0));
                    return;
                case 'K':
                    this.#eraseLine(this.#p0(0, 0));
                    return;
                case 'L': {
                    if (buffer.y < this.#top || buffer.y > this.#bottom) return;
                    const n = Math.min(this.#p(0, 1), this.#bottom - buffer.y + 1);
                    for (let k = 0; k < n; k++) {
                        buffer.lines.splice(this.#bottom, 1);
                        buffer.lines.splice(buffer.y, 0, this.#blank());
                    }
                    buffer.x = 0;
                    this.#wrapNext = false;
                    return;
                }
                case 'M': {
                    if (buffer.y < this.#top || buffer.y > this.#bottom) return;
                    const n = Math.min(this.#p(0, 1), this.#bottom - buffer.y + 1);
                    for (let k = 0; k < n; k++) {
                        buffer.lines.splice(buffer.y, 1);
                        buffer.lines.splice(this.#bottom, 0, this.#blank());
                    }
                    buffer.x = 0;
                    this.#wrapNext = false;
                    return;
                }
                case 'P': {
                    const n = Math.min(this.#p(0, 1), this.cols - buffer.x);
                    const line = buffer.lines[buffer.y];
                    for (let x = buffer.x; x < this.cols - n; x++) {
                        line.ch[x] = line.ch[x + n];
                        line.st[x] = line.st[x + n];
                    }
                    this.#eraseCells(line, this.cols - n, this.cols);
                    line.extra = null;
                    this.#wrapNext = false;
                    return;
                }
                case 'S':
                    this.#scrollUp(this.#p(0, 1));
                    return;
                case 'T':
                    this.#scrollDown(this.#p(0, 1));
                    return;
                case 'X': {
                    const line = buffer.lines[buffer.y];
                    this.#eraseCells(line, buffer.x, Math.min(this.cols, buffer.x + this.#p(0, 1)));
                    this.#wrapNext = false;
                    return;
                }
                case 'Z':
                    this.#backTab(this.#p(0, 1));
                    return;
                case 'b': {
                    const n = Math.min(this.#p(0, 1), 65535);
                    const text = String.fromCodePoint(this.#lastPrinted || 0x20);
                    for (let k = 0; k < n; k++) this.#print(text.codePointAt(0)!);
                    return;
                }
                case 'c':
                    this.onReply(this.graphics === 'all' || this.graphics === 'sixel' ? '\x1b[?62;4;22;52c' : '\x1b[?62;22;52c');
                    return;
                case 'd':
                    this.#moveTo(buffer.x, this.#p(0, 1) - 1);
                    return;
                case 'g':
                    if (this.#p0(0, 0) === 0) this.#tabs[buffer.x] = 0;
                    else if (this.#p0(0, 0) === 3) this.#tabs.fill(0);
                    return;
                case 'h':
                    this.#ansiMode(true);
                    return;
                case 'l':
                    this.#ansiMode(false);
                    return;
                case 'm':
                    this.#sgr();
                    return;
                case 'n':
                    this.#report();
                    return;
                case 'r': {
                    const top = this.#p(0, 1) - 1;
                    const bottom = this.#p0(1, 0) === 0 ? this.rows - 1 : Math.min(this.rows - 1, this.#p(1, this.rows) - 1);
                    if (top < bottom) {
                        this.#top = top;
                        this.#bottom = bottom;
                    } else {
                        this.#top = 0;
                        this.#bottom = this.rows - 1;
                    }
                    this.#moveTo(0, 0);
                    return;
                }
                case 's':
                    this.#saveCursor();
                    return;
                case 'u':
                    this.#restoreCursor();
                    return;
                case 't':
                    this.#windowOps();
                    return;
            }
            return;
        }
        if (priv === '?' && inter === '') {
            if (final === 'h') return this.#privateMode(true);
            if (final === 'l') return this.#privateMode(false);
            if (final === 'n') return this.#privateReport();
            if (final === 'u') {
                // A program asks which of the keyboard's extra reports are on.
                if (this.kittyKeyboard) this.onReply(`\x1b[?${this.kittyFlags}u`);
                return;
            }
            if (final === 'J' || final === 'K') {
                // Selective erase: with nothing protected, it's as erase.
                if (final === 'J') this.#eraseDisplay(this.#p0(0, 0));
                else this.#eraseLine(this.#p0(0, 0));
            }
            return;
        }
        if (this.kittyKeyboard && inter === '' && final === 'u' && (priv === '>' || priv === '<' || priv === '=')) {
            const stack = this.#buf.keyboard;
            const flags = this.#p0(0, 0) & 31;
            const current = stack.length - 1;
            if (priv === '>') {
                if (stack.length >= 64) stack.shift();
                stack.push(flags);
            } else if (priv === '<') {
                const pop = Math.max(1, this.#p0(0, 1));
                if (pop >= stack.length) this.#buf.keyboard = [0];
                else stack.length -= pop;
            } else {
                const mode = this.#p0(1, 1);
                if (mode === 1) stack[current] = flags;
                else if (mode === 2) stack[current] |= flags;
                else if (mode === 3) stack[current] &= ~flags;
            }
            return;
        }
        if (priv === '?' && inter === '$' && final === 'p') {
            const mode = this.#p0(0, 0);
            const state = this.#modeState(mode);
            this.onReply(`\x1b[?${mode};${state}$y`);
            return;
        }
        if (priv === '' && inter === '$' && final === 'p') {
            this.onReply(`\x1b[${this.#p0(0, 0)};0$y`);
            return;
        }
        if (priv === '>' && final === 'c') {
            this.onReply('\x1b[>0;10;1c');
            return;
        }
        if (priv === '>' && final === 'q') {
            this.onReply('\x1bP>|rsui terminal 0.1\x1b\\');
            return;
        }
        if (priv === '' && inter === ' ' && final === 'q') {
            this.cursorShape = Math.min(6, this.#p0(0, 0));
            return;
        }
        if (priv === '!' && final === 'p') {
            this.#reset();
            return;
        }
        if (priv === '=' && final === 'c') {
            this.onReply('\x1bP!|00000000\x1b\\');
            return;
        }
        // The keyboard's extra reports are asked for and pushed, but only those that change nothing here are claimed.
        if (final === 'u' && (priv === '>' || priv === '<' || priv === '=')) return;
    }

    #ansiMode(on: boolean) {
        for (const p of this.#params) {
            if (p === 4) this.#insert = on;
            else if (p === 20) this.#newline = on;
        }
    }

    #modeState(mode: number): number {
        const on = (value: boolean) => (value ? 1 : 2);
        switch (mode) {
            case 1: return on(this.appCursor);
            case 5: return on(this.reverseScreen);
            case 6: return on(this.#origin);
            case 7: return on(this.#autowrap);
            case 12: return on(this.cursorShape % 2 === 1 || this.cursorShape === 0);
            case 25: return on(this.cursorVisible);
            case 47: case 1047: case 1049: return on(this.#alternate);
            case 1000: return on(this.pointer === 'normal');
            case 1002: return on(this.pointer === 'button');
            case 1003: return on(this.pointer === 'any');
            case 1004: return on(this.focusEvents);
            case 80: return on(this.sixelDisplay);
            case 8452: return on(this.sixelCursorRight);
            case 1005: return on(this.pointerEncoding === 'utf8');
            case 1006: return on(this.pointerEncoding === 'sgr');
            case 1015: return on(this.pointerEncoding === 'urxvt');
            case 2004: return on(this.bracketedPaste);
            case 2026: return on(this.synchronized);
            case 9001: return 0;
            default: return 0;
        }
    }

    #privateMode(on: boolean) {
        for (const mode of this.#params) {
            switch (mode) {
                case 1:
                    this.appCursor = on;
                    break;
                case 5:
                    this.reverseScreen = on;
                    break;
                case 6:
                    this.#origin = on;
                    this.#moveTo(0, 0);
                    break;
                case 7:
                    this.#autowrap = on;
                    break;
                case 12:
                    // Blinking is the odd shapes; this keeps the shape and changes whether it blinks.
                    if (this.cursorShape === 0) this.cursorShape = 1;
                    if (on && this.cursorShape % 2 === 0) this.cursorShape--;
                    else if (!on && this.cursorShape % 2 === 1) this.cursorShape++;
                    break;
                case 25:
                    this.cursorVisible = on;
                    break;
                case 9:
                    this.pointer = on ? 'x10' : 'none';
                    break;
                case 1000:
                    this.pointer = on ? 'normal' : 'none';
                    break;
                case 1002:
                    this.pointer = on ? 'button' : 'none';
                    break;
                case 1003:
                    this.pointer = on ? 'any' : 'none';
                    break;
                case 1004:
                    this.focusEvents = on;
                    break;
                case 80:
                    this.sixelDisplay = on;
                    break;
                case 8452:
                    this.sixelCursorRight = on;
                    break;
                case 1005:
                    this.pointerEncoding = on ? 'utf8' : 'x10';
                    break;
                case 1006:
                    this.pointerEncoding = on ? 'sgr' : 'x10';
                    break;
                case 1015:
                    this.pointerEncoding = on ? 'urxvt' : 'x10';
                    break;
                case 47:
                case 1047:
                    if (on) this.#enterAlternate(false);
                    else this.#leaveAlternate(false);
                    break;
                case 1048:
                    if (on) this.#saveCursor();
                    else this.#restoreCursor();
                    break;
                case 1049:
                    if (on) this.#enterAlternate(true);
                    else this.#leaveAlternate(true);
                    break;
                case 2004:
                    this.bracketedPaste = on;
                    break;
                case 2026:
                    if (this.synchronized !== on) {
                        this.synchronized = on;
                        this.onSync(on);
                    }
                    break;
            }
        }
    }

    #enterAlternate(save: boolean) {
        if (this.#alternate) return;
        if (save) this.#saveCursor();
        this.#alternate = true;
        this.#buf = this.#alt;
        if (save) {
            this.#alt.lines = blankBuffer(this.rows, this.cols, () => this.#blank());
            this.#alt.x = 0;
            this.#alt.y = 0;
            this.#alt.images = [];
        } else if (this.#alt.lines.length === 0) {
            this.#alt.lines = blankBuffer(this.rows, this.cols, () => this.#blankWith(this.cols, 0));
        }
        this.#wrapNext = false;
    }

    #leaveAlternate(restore: boolean) {
        if (!this.#alternate) return;
        this.#alternate = false;
        this.#buf = this.#main;
        if (restore) this.#restoreCursor();
        this.#wrapNext = false;
        this.#top = 0;
        this.#bottom = this.rows - 1;
    }

    #report() {
        switch (this.#p0(0, 0)) {
            case 5:
                this.onReply('\x1b[0n');
                break;
            case 6: {
                const row = this.#buf.y - (this.#origin ? this.#top : 0) + 1;
                this.onReply(`\x1b[${row};${this.#buf.x + 1}R`);
                break;
            }
        }
    }

    #privateReport() {
        if (this.#p0(0, 0) === 6) {
            const row = this.#buf.y - (this.#origin ? this.#top : 0) + 1;
            this.onReply(`\x1b[?${row};${this.#buf.x + 1};1R`);
        } else if (this.#p0(0, 0) === 996) {
            // Whether the colors are dark or light, as a program asks to follow them.
            this.onReply(`\x1b[?997;${luminance(this.theme.bg) < 0.5 ? 1 : 2}n`);
        }
    }

    #windowOps() {
        switch (this.#p0(0, 0)) {
            case 14:
                this.onReply(`\x1b[4;${this.rows * this.cellPx.h};${this.cols * this.cellPx.w}t`);
                break;
            case 16:
                this.onReply(`\x1b[6;${this.cellPx.h};${this.cellPx.w}t`);
                break;
            case 18:
            case 19:
                this.onReply(`\x1b[${this.#p0(0, 0) === 18 ? 8 : 9};${this.rows};${this.cols}t`);
                break;
            case 22:
            case 23:
                // Pushing and popping the title: it's kept as it is.
                break;
        }
    }

    // ---- Erasing

    #eraseCells(line: Line, from: number, to: number) {
        const st = this.#eraseStyle();
        to = Math.min(to, line.length);
        for (let x = Math.max(0, from); x < to; x++) {
            line.ch[x] = 0;
            line.st[x] = st;
        }
        if (line.extra) for (const x of [...line.extra.keys()]) if (x >= from && x < to) line.extra.delete(x);
        // A wide character that's half gone is gone.
        if (from > 0 && line.ch[from] === 0 && from < line.length) {
            /* the cell before may be the head of one whose tail was just cleared */
            if (line.ch[from - 1] !== 0 && line.ch[from - 1] !== WIDE_TAIL && cellWidth(line.ch[from - 1]) === 2) line.ch[from - 1] = 0;
        }
        if (to < line.length && line.ch[to] === WIDE_TAIL) line.ch[to] = 0;
        line.ver = ++this.#clock;
    }

    #eraseLine(mode: number) {
        const buffer = this.#buf;
        const line = buffer.lines[buffer.y];
        if (mode === 0) this.#eraseCells(line, buffer.x, this.cols);
        else if (mode === 1) this.#eraseCells(line, 0, buffer.x + 1);
        else {
            this.#eraseCells(line, 0, this.cols);
            line.wrapped = false;
        }
        if (mode !== 1) line.wrapped = false;
        this.#wrapNext = false;
    }

    #eraseDisplay(mode: number) {
        const buffer = this.#buf;
        if (mode === 0) {
            this.#eraseLine(0);
            for (let y = buffer.y + 1; y < this.rows; y++) this.#eraseCells(buffer.lines[y], 0, this.cols);
        } else if (mode === 1) {
            for (let y = 0; y < buffer.y; y++) this.#eraseCells(buffer.lines[y], 0, this.cols);
            this.#eraseLine(1);
        } else if (mode === 3 && !this.#alternate) {
            this.evicted += this.scrollbackLength;
            this.#scrollback = [];
            this.#scrollbackHead = 0;
            this.#forget();
        } else if (mode === 2) {
            const screen = this.evicted + this.scrollbackLength;
            this.images = this.images.filter((image) => image.line + (image.clip?.end ?? image.rows) <= screen);
            if (mode === 2 && !this.#alternate && this.#fullScreenContent()) {
                // A clear that wipes real output first keeps it in the scrollback, as most terminals do for Ctrl+L.
                this.#keepScreen();
            }
            for (let y = 0; y < this.rows; y++) {
                this.#eraseCells(buffer.lines[y], 0, this.cols);
                buffer.lines[y].wrapped = false;
            }
        }
        this.#wrapNext = false;
    }

    #fullScreenContent(): boolean {
        return false;
    }

    #keepScreen() {}

    // ---- SGR

    #sgr() {
        const subs = this.#sub;
        const n = subs.length;
        if (n === 0) {
            this.#resetPen();
            return;
        }
        for (let k = 0; k < n; k++) {
            const group = subs[k];
            const code = group[0] < 0 ? 0 : group[0];
            switch (code) {
                case 0:
                    this.#resetPen();
                    break;
                case 1:
                    this.#flags |= BOLD;
                    break;
                case 2:
                    this.#flags |= DIM;
                    break;
                case 3:
                    this.#flags |= ITALIC;
                    break;
                case 4: {
                    const style = group.length > 1 ? Math.max(0, group[1]) : 1;
                    this.#flags = (this.#flags & ~UNDERLINE_MASK) | (Math.min(5, style) << UNDERLINE_SHIFT);
                    break;
                }
                case 5:
                case 6:
                    this.#flags |= BLINK;
                    break;
                case 7:
                    this.#flags |= REVERSE;
                    break;
                case 8:
                    this.#flags |= HIDDEN;
                    break;
                case 9:
                    this.#flags |= STRIKE;
                    break;
                case 21:
                    this.#flags = (this.#flags & ~UNDERLINE_MASK) | (2 << UNDERLINE_SHIFT);
                    break;
                case 22:
                    this.#flags &= ~(BOLD | DIM);
                    break;
                case 23:
                    this.#flags &= ~ITALIC;
                    break;
                case 24:
                    this.#flags &= ~UNDERLINE_MASK;
                    break;
                case 25:
                    this.#flags &= ~BLINK;
                    break;
                case 27:
                    this.#flags &= ~REVERSE;
                    break;
                case 28:
                    this.#flags &= ~HIDDEN;
                    break;
                case 29:
                    this.#flags &= ~STRIKE;
                    break;
                case 53:
                    this.#flags |= OVERLINE;
                    break;
                case 55:
                    this.#flags &= ~OVERLINE;
                    break;
                case 39:
                    this.#fg = 0;
                    break;
                case 49:
                    this.#bg = 0;
                    break;
                case 59:
                    this.#ul = 0;
                    break;
                case 38:
                case 48:
                case 58: {
                    const result = this.#extendedColor(group, subs, k);
                    if (result.color !== null) {
                        if (code === 38) this.#fg = result.color;
                        else if (code === 48) this.#bg = result.color;
                        else this.#ul = result.color;
                    }
                    k = result.next;
                    break;
                }
                default:
                    if (code >= 30 && code <= 37) this.#fg = paletteColor(code - 30);
                    else if (code >= 40 && code <= 47) this.#bg = paletteColor(code - 40);
                    else if (code >= 90 && code <= 97) this.#fg = paletteColor(code - 90 + 8);
                    else if (code >= 100 && code <= 107) this.#bg = paletteColor(code - 100 + 8);
            }
        }
        this.#penDirty = true;
    }

    /** `38;5;n` and `38;2;r;g;b`, and the same with colons (`38:2::r:g:b`), which has a color space's place before the colors. */
    #extendedColor(group: number[], all: number[][], k: number): { color: number | null; next: number } {
        if (group.length > 1) {
            const mode = group[1];
            if (mode === 5 && group.length > 2) return { color: group[2] >= 0 && group[2] < 256 ? paletteColor(group[2]) : null, next: k };
            if (mode === 2 && group.length >= 5) {
                const rest = group.length >= 6 ? group.slice(3, 6) : group.slice(2, 5);
                return { color: rgbColor(rest[0], rest[1], rest[2]), next: k };
            }
            return { color: null, next: k };
        }
        const mode = all[k + 1]?.[0];
        if (mode === 5) {
            const n = all[k + 2]?.[0];
            return { color: n !== undefined && n >= 0 && n < 256 ? paletteColor(n) : null, next: k + 2 };
        }
        if (mode === 2) {
            const [r, g, b] = [all[k + 2]?.[0], all[k + 3]?.[0], all[k + 4]?.[0]];
            if (r === undefined || g === undefined || b === undefined) return { color: null, next: all.length };
            return { color: rgbColor(Math.max(0, r), Math.max(0, g), Math.max(0, b)), next: k + 4 };
        }
        return { color: null, next: k };
    }

    #resetPen() {
        this.#fg = 0;
        this.#bg = 0;
        this.#flags = 0;
        this.#ul = 0;
        this.#penDirty = true;
    }

    // ---- Strings: OSC, DCS and the rest

    #stringBytes(data: string, start: number, n: number): number {
        let i = start;
        const oscLike = this.#state === OSC;
        while (i < n) {
            const c = data.charCodeAt(i);
            if (c === 0x07 && oscLike) {
                this.#endString();
                return i + 1;
            }
            if (c === 0x1b) {
                this.#state = STRING_ESC;
                return i + 1;
            }
            if (c === 0x18 || c === 0x1a) {
                this.#state = GROUND;
                this.#string = '';
                return i + 1;
            }
            // Take a stretch at once.
            let j = i;
            while (j < n) {
                const d = data.charCodeAt(j);
                if (d === 0x1b || d === 0x18 || d === 0x1a || (d === 0x07 && oscLike)) break;
                j++;
            }
            if (this.#string.length < MAX_STRING) this.#string += data.slice(i, j);
            i = j;
        }
        return i;
    }

    #endString() {
        const text = this.#string;
        this.#string = '';
        const kind = this.#introducer;
        this.#state = GROUND;
        if (kind === ']') this.#osc(text);
        else if (kind === 'P') this.#dcs(text);
        else if (kind === '_') this.#apc(text);
    }

    #dcs(text: string) {
        // DECRQSS: a program asks how something's set.
        if (text.startsWith('$q')) {
            const what = text.slice(2);
            if (what === 'm') this.onReply('\x1bP1$r0m\x1b\\');
            else if (what === ' q') this.onReply(`\x1bP1$r${this.cursorShape} q\x1b\\`);
            else if (what === 'r') this.onReply(`\x1bP1$r${this.#top + 1};${this.#bottom + 1}r\x1b\\`);
            else this.onReply('\x1bP0$r\x1b\\');
        } else if (this.graphics === 'all' || this.graphics === 'sixel') {
            const sixel = /^([\d;]*)q/.exec(text);
            if (!sixel) return;
            const [macro, background] = sixel[1].split(';').map(Number);
            const aspect = macro === 2 ? 4 : macro === 3 || macro === 4 ? 3 : macro === 7 || macro === 8 || macro === 9 ? 1 : 2;
            const picture = sixelPicture(text.slice(sixel[0].length), background === 1, this.theme.bg, aspect);
            if (picture) {
                const { x, y } = this.#buf;
                if (this.sixelDisplay) this.#buf.x = this.#buf.y = 0;
                this.#placeImage(picture.data, '', 'auto', 'auto', picture, undefined, !this.sixelDisplay);
                if (this.sixelDisplay) {
                    this.#buf.x = x;
                    this.#buf.y = y;
                }
                else if (this.sixelCursorRight) this.#buf.x = Math.min(this.cols - 1, x + Math.ceil(picture.w / this.cellPx.w));
            }
        }
    }

    #apc(text: string) {
        if (text[0] !== 'G' || (this.graphics !== 'all' && this.graphics !== 'kitty')) return;
        const semi = text.indexOf(';');
        const controls: Record<string, string> = Object.create(null);
        for (const item of text.slice(1, semi < 0 ? undefined : semi).split(',')) {
            if (/^[a-zA-Z]=[a-zA-Z0-9-]+$/.test(item)) controls[item[0]] = item.slice(2);
        }
        const data = semi < 0 ? '' : text.slice(semi + 1);
        const transfer = this.#kittyTransfer ?? { options: controls, chunks: [], size: 0 };
        const options = transfer.options;
        transfer.size += data.length;
        if (transfer.size <= MAX_IMAGE_DATA && data) transfer.chunks.push(data);
        this.#kittyTransfer = controls.m === '1' && transfer.size <= MAX_IMAGE_DATA ? transfer : null;
        if (this.#kittyTransfer) return;
        const action = options.a ?? 't';
        const number = Number(options.I) || 0;
        let id = Number(options.i) || this.#kittyNumbers.get(number) || 0;
        const placement = Number(options.p) || 0;
        const quiet = controls.q ?? options.q;
        const reply = (message: string) => {
            if (quiet === '2' || (quiet === '1' && message === 'OK') || (!id && !number)) return;
            const keys = [id && `i=${id}`, number && `I=${number}`, placement && `p=${placement}`].filter(Boolean);
            this.onReply(`\x1b_G${keys.join(',')};${message}\x1b\\`);
        };
        if (action === 'd') {
            const mode = options.d ?? 'a';
            const byId = mode.toLowerCase() === 'i' || mode.toLowerCase() === 'n';
            const remove = (image: Image) => image.kittyId !== undefined && (byId
                ? image.kittyId === id && (!placement || placement === image.placement)
                : mode.toLowerCase() === 'c'
                  ? this.#absLine() >= image.line && this.#absLine() < image.line + image.rows && this.cursorX >= image.col && this.cursorX < image.col + image.cols
                  : mode.toLowerCase() === 'a');
            this.images = this.images.filter((image) => !remove(image));
            if (mode === 'A') { this.#kittyImages.clear(); this.#kittyNumbers.clear(); }
            else if (byId && mode === mode.toUpperCase() && !placement) {
                this.#kittyImages.delete(id);
                for (const [key, value] of this.#kittyNumbers) if (value === id) this.#kittyNumbers.delete(key);
            }
            return;
        }
        if (options.t && options.t !== 'd') return reply('EINVAL:unsupported transmission medium');
        if (options.U === '1' || !['q', 't', 'T', 'p'].includes(action)) return reply('ENOTSUP:unsupported action');
        const picture = action === 'p' ? this.#kittyImages.get(id) :
            transfer.size <= MAX_IMAGE_DATA && kittyPicture(transfer.chunks.join(''), options);
        if (!picture) return reply(action === 'p' ? 'ENOENT:image not found' : 'EINVAL:invalid image');
        if (action === 'q') return reply('OK');
        if (number && !id) {
            while (this.#kittyImages.has(this.#nextKittyId)) this.#nextKittyId++;
            id = this.#nextKittyId++;
        }
        if (action !== 'p' && id) {
            this.#kittyImages.delete(id);
            this.#kittyImages.set(id, picture);
            if (number) this.#kittyNumbers.set(number, id);
            let size = 0;
            for (const value of this.#kittyImages.values()) size += value.data.length;
            while (size > MAX_IMAGE_DATA * 2 || this.#kittyImages.size > 64) {
                const oldest = this.#kittyImages.keys().next().value!;
                size -= this.#kittyImages.get(oldest)!.data.length;
                this.#kittyImages.delete(oldest);
                for (const [key, value] of this.#kittyNumbers) if (value === oldest) this.#kittyNumbers.delete(key);
            }
        }
        if (action === 'p' || action === 'T') {
            if (id && placement) this.images = this.images.filter((image) => image.kittyId !== id || image.placement !== placement);
            this.#placeImage(picture.data, '', options.c || 'auto', options.r || 'auto', picture, { id, placement, options });
        }
        reply('OK');
    }

    #osc(text: string) {
        const semi = text.indexOf(';');
        const code = semi < 0 ? text : text.slice(0, semi);
        const rest = semi < 0 ? '' : text.slice(semi + 1);
        switch (code) {
            case '0':
            case '2':
                this.title = rest;
                this.onTitle(rest);
                break;
            case '1':
                break;
            case '4':
                this.#oscPalette(rest);
                break;
            case '7':
                this.#setCwd(rest);
                break;
            case '8':
                this.#hyperlink(rest);
                break;
            case '9':
                this.#iterm9(rest);
                break;
            case '99': {
                const split = rest.indexOf(';');
                if (split < 0) break;
                const meta = new Map(rest.slice(0, split).split(':').map((pair) => [pair.slice(0, pair.indexOf('=')), pair.slice(pair.indexOf('=') + 1)]));
                const key = (meta.get('i') ?? '').slice(0, 64);
                const kind = meta.get('p') ?? 'title';
                if (kind !== 'title' && kind !== 'body') break;
                let payload = rest.slice(split + 1, split + 4097);
                if (meta.get('e') === '1') payload = Buffer.from(payload, 'base64').toString('utf8');
                const pending = this.#notificationParts.get(key) ?? { title: '', body: '' };
                pending[kind] = (pending[kind] + payload).slice(0, 4096);
                if (meta.get('d') === '0') {
                    if (this.#notificationParts.size < 32 || this.#notificationParts.has(key)) this.#notificationParts.set(key, pending);
                } else {
                    this.#notificationParts.delete(key);
                    this.onNotify(pending.title, pending.body);
                }
                break;
            }
            case '10':
            case '11':
            case '12':
                this.#oscColors(Number(code), rest);
                break;
            case '52':
                this.#clipboard(rest);
                break;
            case '104':
                if (rest === '') this.palette.clear();
                else for (const part of rest.split(';')) this.palette.delete(Number(part));
                this.version++;
                break;
            case '110':
            case '111':
            case '112':
                break;
            case '133':
                this.#shellMark(rest);
                break;
            case '777': {
                const [kind, title, ...body] = rest.split(';');
                if (kind === 'notify') this.onNotify(title ?? '', body.join(';'));
                break;
            }
            case '1337':
                this.#iterm1337(rest);
                break;
        }
    }

    #setCwd(rest: string) {
        let path = rest;
        const m = /^file:\/\/([^/]*)(\/.*)$/.exec(rest);
        if (m) {
            this.host = m[1];
            try {
                path = decodeURIComponent(m[2]);
            } catch {
                path = m[2];
            }
            // file:///C:/Users is /C:/Users.
            if (/^\/[A-Za-z]:/.test(path)) path = path.slice(1);
        }
        this.cwd = path;
        this.onCwd(path);
    }

    #hyperlink(rest: string) {
        const semi = rest.indexOf(';');
        const uri = semi < 0 ? '' : rest.slice(semi + 1);
        if (uri === '') {
            this.#link = 0;
        } else {
            let id = this.#linkIds.get(uri);
            if (id === undefined) {
                id = this.#links.length;
                this.#links.push(uri);
                this.#linkIds.set(uri, id);
            }
            this.#link = id;
        }
        this.#penDirty = true;
    }

    #iterm9(rest: string) {
        const parts = rest.split(';');
        if (parts[0] === '4') {
            const state = Number(parts[1] ?? 0);
            const value = Math.max(0, Math.min(100, Number(parts[2] ?? 0)));
            this.progress = { state: Number.isFinite(state) ? state : 0, value: Number.isFinite(value) ? value : 0 };
            this.onProgress(this.progress.state, this.progress.value);
        } else if (parts[0] === '9') {
            // ConEmu's way to say where the shell is, as PowerShell can.
            this.cwd = parts.slice(1).join(';').replace(/^"|"$/g, '');
            this.onCwd(this.cwd);
        } else if (rest !== '') {
            this.onNotify('', rest);
        }
    }

    #oscColors(first: number, rest: string) {
        const parts = rest.split(';');
        for (let k = 0; k < parts.length; k++) {
            const slot = first + k;
            const value = parts[k];
            const color = slot === 10 ? this.theme.fg : slot === 11 ? this.theme.bg : this.theme.cursor;
            if (value === '?') {
                this.onReply(`\x1b]${slot};${xparse(color)}\x1b\\`);
            }
        }
    }

    #oscPalette(rest: string) {
        const parts = rest.split(';');
        for (let k = 0; k + 1 < parts.length; k += 2) {
            const index = Number(parts[k]);
            if (!(index >= 0 && index < 256)) continue;
            if (parts[k + 1] === '?') {
                this.onReply(`\x1b]4;${index};${xparse(this.paletteRgb(index))}\x1b\\`);
            } else {
                const color = parseColor(parts[k + 1]);
                if (color !== null) {
                    this.palette.set(index, color);
                    this.version++;
                }
            }
        }
    }

    /** A palette color's 0xRRGGBB, with the program's changes. */
    paletteRgb(index: number): number {
        return this.palette.get(index) ?? xtermColor(index, this.theme.ansi);
    }

    #clipboard(rest: string) {
        const semi = rest.indexOf(';');
        const data = semi < 0 ? '' : rest.slice(semi + 1);
        if (data === '?' || data === '') return;
        try {
            const bytes = Uint8Array.from(atob(data), (ch) => ch.charCodeAt(0));
            this.onClipboard(new TextDecoder().decode(bytes));
        } catch {
            // What isn't base64 is dropped.
        }
    }

    // ---- Shell integration

    #absLine(): number {
        return this.evicted + this.scrollbackLength + this.#buf.y;
    }

    #shellMark(rest: string) {
        if (this.#alternate) return;
        const parts = rest.split(';');
        const kind = parts[0];
        const last = this.commands[this.commands.length - 1];
        const open = last && last.endLine === null ? last : null;
        if (kind === 'A') {
            // A prompt starts. One that never ran is replaced.
            if (open && open.outputLine === null) this.commands.pop();
            const command: Command = {
                id: this.#nextCommand++,
                promptLine: this.#absLine(),
                inputLine: null,
                outputLine: null,
                endLine: null,
                exit: null,
                startedAt: null,
                endedAt: null,
                text: '',
                cwd: this.cwd,
            };
            this.commands.push(command);
            this.onCommand(command, 'prompt');
        } else if (kind === 'B') {
            if (open) {
                open.inputLine = this.#absLine();
                this.#inputColumn = this.#buf.x;
                this.onCommand(open, 'input');
            }
        } else if (kind === 'C') {
            if (open) {
                open.text = this.#typedText(open);
                // Output starts where the cursor is, which is the next line if what was typed is still on this one.
                const here = this.#absLine();
                open.outputLine = open.inputLine !== null && here === open.inputLine && this.#buf.x > 0 ? here + 1 : here;
                open.startedAt = this.#writingAt ?? Date.now();
                open.cwd = this.cwd || open.cwd;
                this.onCommand(open, 'output');
            }
        } else if (kind === 'D') {
            if (open) {
                const code = parts[1] === undefined || parts[1] === '' ? 0 : Number(parts[1]);
                open.exit = Number.isFinite(code) ? code : 0;
                open.endLine = this.#absLine();
                open.endedAt = this.#writingAt ?? Date.now();
                if (open.startedAt === null) open.startedAt = open.endedAt;
                this.onCommand(open, 'end');
            }
        }
    }

    #inputColumn = 0;

    /** What was typed after the prompt, read from the screen when the command starts. */
    #typedText(command: Command): string {
        if (command.inputLine === null) return '';
        const here = this.#absLine();
        let text = '';
        for (let n = command.inputLine; n <= here; n++) {
            const line = this.lineNumbered(n);
            if (!line) continue;
            const from = n === command.inputLine ? this.#inputColumn : 0;
            const to = n === here ? this.#buf.x : line.length;
            const part = this.textOf(line, from, to);
            text += part;
            if (!line.wrapped && n < here) break;
        }
        return text.trim();
    }

    // ---- Pictures

    #iterm1337(rest: string) {
        if (rest.startsWith('File=')) {
            if (this.graphics !== 'all' && this.graphics !== 'iterm') return;
            const colon = rest.indexOf(':');
            if (colon < 0) return;
            const options = new Map<string, string>();
            for (const pair of rest.slice(5, colon).split(';')) {
                const eq = pair.indexOf('=');
                if (eq > 0) options.set(pair.slice(0, eq), pair.slice(eq + 1));
            }
            if (options.get('inline') !== '1') return;
            const data = rest.slice(colon + 1);
            let name = '';
            try {
                name = options.has('name') ? atob(options.get('name')!) : '';
            } catch {
                name = '';
            }
            this.#placeImage(data, name, options.get('width') ?? 'auto', options.get('height') ?? 'auto');
        } else if (rest.startsWith('CurrentDir=')) {
            this.cwd = rest.slice(11);
            this.onCwd(this.cwd);
        } else if (rest.startsWith('RemoteHost=')) {
            this.host = rest.slice(11).split('@').pop() ?? '';
        } else if (rest === 'ClearScrollback') {
            this.#eraseDisplay(3);
        }
    }

    /** Called with a picture's bytes (as base64), and what it asked for as its size, for the cell size it takes up. */
    #placeImage(data: string, name: string, width: string, height: string, picture?: Picture,
        kitty?: { id: number; placement: number; options: Record<string, string> }, advance = true) {
        if (data.length > MAX_IMAGE_DATA) return;
        const mime = picture ? 'image/png' : sniff(data);
        if (mime === null) return;
        const full = picture ?? imageSize(data, mime);
        const { w: cw, h: ch } = this.cellPx;
        const opt = kitty?.options;
        const bounded = (value: string | undefined, fallback: number, low: number, high: number) => {
            const number = value === undefined ? fallback : Number(value);
            return Math.min(high, Math.max(low, number || low));
        };
        const viewport = opt ? {
            x: bounded(opt.x, 0, 0, full.w - 1), y: bounded(opt.y, 0, 0, full.h - 1),
            offsetX: bounded(opt.X, 0, 0, cw - 1), offsetY: bounded(opt.Y, 0, 0, ch - 1),
            w: full.w, h: full.h, fullWidth: full.w, fullHeight: full.h,
        } : undefined;
        if (viewport && opt) {
            viewport.w = bounded(opt.w, full.w - viewport.x, 1, full.w - viewport.x);
            viewport.h = bounded(opt.h, full.h - viewport.y, 1, full.h - viewport.y);
        }
        const size = { w: viewport?.w ?? full.w, h: (viewport?.h ?? full.h) * (picture?.pixelAspect ?? 1) };
        const units = (spec: string, cell: number, whole: number): number | null => {
            if (spec === 'auto') return null;
            if (spec.endsWith('px')) return Math.max(1, Math.ceil(parseFloat(spec) / cell));
            if (spec.endsWith('%')) return Math.max(1, Math.round((parseFloat(spec) / 100) * whole));
            const n = parseFloat(spec);
            return Number.isFinite(n) ? Math.max(1, Math.ceil(n)) : null;
        };
        let cols = units(width, cw, this.cols);
        let rows = units(height, ch, this.rows);
        const naturalCols = Math.max(1, Math.ceil((size.w + (viewport?.offsetX ?? 0)) / cw));
        const naturalRows = Math.max(1, Math.ceil((size.h + (viewport?.offsetY ?? 0)) / ch));
        const natural = cols === null && rows === null;
        let pixelScale = 1;
        if (cols === null && rows === null) {
            cols = naturalCols;
            rows = naturalRows;
        } else if (cols === null) {
            cols = Math.max(1, Math.round((rows! * ch * size.w) / size.h / cw));
        } else if (rows === null) {
            rows = Math.max(1, Math.round((cols * cw * size.h) / size.w / ch));
        }
        // It's never wider than the window, and keeps its shape.
        if (!kitty && cols! > this.cols - this.#buf.x) {
            const scale = (this.cols - this.#buf.x) / cols!;
            pixelScale = scale;
            cols = Math.max(1, this.cols - this.#buf.x);
            rows = Math.max(1, Math.round(rows! * scale));
        }
        const image: Image = {
            id: this.#nextImage++,
            line: this.#absLine(),
            col: this.#buf.x,
            cols: Math.min(4096, cols!),
            rows: Math.min(4096, rows!),
            mime,
            data,
            name,
            ...(picture && natural ? { pixels: { width: size.w * pixelScale, height: size.h * pixelScale } } : {}),
            ...(picture?.pixelAspect ? { stretch: true } : {}),
            ...(kitty ? { kittyId: kitty.id, placement: kitty.placement, z: Number(kitty.options.z) || 0, stretch: true, viewport } : {}),
        };
        this.images.push(image);
        let budget = MAX_IMAGE_DATA * 2;
        for (let i = this.images.length - 1; i >= 0; i--) {
            budget -= this.images[i].data.length;
            if (budget < 0 || this.images.length - i > 128) { this.images.splice(0, i + 1); break; }
        }
        // The cursor goes below it, as text would after the picture.
        const startCol = this.#buf.x;
        if (advance && kitty?.options.C !== '1') {
            for (let k = 0; k < image.rows; k++) this.#lineFeed();
            this.#buf.x = kitty ? Math.min(this.cols - 1, startCol + image.cols) : startCol;
        }
        this.onImage(image);
    }
}

// ---- Colors

export function defaultTheme(): Theme {
    return {
        fg: 0xcccccc,
        bg: 0x0c0c0c,
        cursor: 0xffffff,
        ansi: [
            0x0c0c0c, 0xc50f1f, 0x13a10e, 0xc19c00, 0x0037da, 0x881798, 0x3a96dd, 0xcccccc,
            0x767676, 0xe74856, 0x16c60c, 0xf9f1a5, 0x3b78ff, 0xb4009e, 0x61d6d6, 0xf2f2f2,
        ],
    };
}

/** The 256 colors of xterm, with the first 16 from the theme. */
export function xtermColor(index: number, ansi: number[]): number {
    if (index < 16) return ansi[index] ?? 0;
    if (index < 232) {
        const n = index - 16;
        const level = (v: number) => (v === 0 ? 0 : 55 + v * 40);
        return (level(Math.floor(n / 36)) << 16) | (level(Math.floor(n / 6) % 6) << 8) | level(n % 6);
    }
    const gray = 8 + (index - 232) * 10;
    return (gray << 16) | (gray << 8) | gray;
}

function xparse(color: number): string {
    const part = (v: number) => {
        const h = (v & 255).toString(16).padStart(2, '0');
        return h + h;
    };
    return `rgb:${part(color >> 16)}/${part(color >> 8)}/${part(color)}`;
}

/** `rgb:rr/gg/bb`, `#rrggbb` and `rgb:rrrr/gggg/bbbb`, as a program may set a color. */
export function parseColor(text: string): number | null {
    let m = /^rgb:([0-9a-f]{1,4})\/([0-9a-f]{1,4})\/([0-9a-f]{1,4})$/i.exec(text);
    if (m) {
        const scale = (hex: string) => Math.round((parseInt(hex, 16) / (16 ** hex.length - 1)) * 255);
        return (scale(m[1]) << 16) | (scale(m[2]) << 8) | scale(m[3]);
    }
    m = /^#([0-9a-f]{6})$/i.exec(text);
    if (m) return parseInt(m[1], 16);
    return null;
}

function luminance(color: number): number {
    const r = (color >> 16) & 255;
    const g = (color >> 8) & 255;
    const b = color & 255;
    return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
}

// ---- Pictures' formats

function sniff(base64: string): string | null {
    const head = base64.slice(0, 16);
    if (head.startsWith('iVBOR')) return 'image/png';
    if (head.startsWith('/9j/')) return 'image/jpeg';
    if (head.startsWith('R0lGOD')) return 'image/gif';
    if (head.startsWith('UklGR')) return 'image/webp';
    if (head.startsWith('Qk')) return 'image/bmp';
    return null;
}

function bytesOf(base64: string, count: number): Uint8Array {
    // Enough of the start to read a size from: four characters make three bytes.
    const take = Math.ceil((count * 4) / 3 / 4) * 4;
    try {
        return Uint8Array.from(atob(base64.slice(0, take)), (ch) => ch.charCodeAt(0));
    } catch {
        return new Uint8Array(0);
    }
}

/** A picture's size in pixels, from its header. JPEG's is further in, so a longer start is read for it. */
export function imageSize(base64: string, mime: string): { w: number; h: number } {
    const fallback = { w: 320, h: 240 };
    if (mime === 'image/png') {
        const b = bytesOf(base64, 32);
        if (b.length < 24) return fallback;
        return { w: (b[16] << 24) | (b[17] << 16) | (b[18] << 8) | b[19], h: (b[20] << 24) | (b[21] << 16) | (b[22] << 8) | b[23] };
    }
    if (mime === 'image/gif') {
        const b = bytesOf(base64, 16);
        if (b.length < 10) return fallback;
        return { w: b[6] | (b[7] << 8), h: b[8] | (b[9] << 8) };
    }
    if (mime === 'image/jpeg') {
        const b = bytesOf(base64, Math.min(base64.length, 200000) * 0.75);
        let i = 2;
        while (i + 9 < b.length) {
            if (b[i] !== 0xff) {
                i++;
                continue;
            }
            const marker = b[i + 1];
            if (marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc) {
                return { h: (b[i + 5] << 8) | b[i + 6], w: (b[i + 7] << 8) | b[i + 8] };
            }
            i += 2 + ((b[i + 2] << 8) | b[i + 3]);
        }
        return fallback;
    }
    if (mime === 'image/bmp') {
        const b = bytesOf(base64, 32);
        if (b.length < 26) return fallback;
        return { w: b[18] | (b[19] << 8) | (b[20] << 16), h: Math.abs((b[22] | (b[23] << 8) | (b[24] << 16)) << 0) };
    }
    return fallback;
}

void PALETTE;
void RGB;
