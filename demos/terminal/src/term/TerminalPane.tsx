// A shell's screen: its rows, the cursor, the scrollbar with where its commands and matches are, the hints over it, and the mouse.
// The keyboard goes to the shell from `app.tsx`, which knows which column has it.

import { createContext, useContext, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { Row } from './Row';
import { TerminalImage } from './TerminalImage';
import type { Look } from './Row';
import type { Session } from './Session';
import { measureFont } from './metrics';
import { spansByLine } from './search';
import { findHints } from './hints';
import type { Hint } from './hints';
import type { Settings } from '../settings';
import { css, mix, withAlpha } from '../themes';
import type { Scheme } from '../themes';
import { WIDE_TAIL } from '../vt/terminal';
import type { Command } from '../vt/terminal';

const GUTTER = 12;
const SCROLLBAR = 12;
export const terminalInput = 'data-terminal-input';

export function useSession(session: Session): number {
    return useSyncExternalStore(session.subscribe, session.getVersion);
}

export function lookFor(settings: Settings, scheme: Scheme): Look & { cursorColor: number } {
    const m = measureFont(settings.fontFamily, settings.fontSize, settings.lineHeight);
    return {
        family: settings.fontFamily,
        ligatures: settings.ligatures,
        size: settings.fontSize,
        cw: m.cw,
        ch: m.ch,
        offset: m.offset,
        gutter: settings.showMarks ? GUTTER : 4,
        fg: scheme.fg,
        bg: scheme.bg,
        selection: scheme.selection,
        accent: scheme.ansi[4],
        bad: scheme.ansi[1],
        found: mix(scheme.bg, scheme.ansi[3], 0.4),
        foundCurrent: mix(scheme.bg, scheme.ansi[11], 0.75),
        cursorColor: scheme.cursor,
        key: `${settings.fontFamily}|${settings.fontSize}|${settings.lineHeight}|${settings.showMarks}|${settings.ligatures}|${scheme.id}`,
    };
}

const durationText = (ms: number) =>
    ms < 1000 ? `${Math.round(ms)} ms` : ms < 60000 ? `${(ms / 1000).toFixed(ms < 10000 ? 1 : 0)} s` : `${Math.floor(ms / 60000)}m ${Math.round((ms % 60000) / 1000)}s`;

/** For each line in view, what its gutter shows and what's written at the right of it. */
export function marksFor(session: Pick<Session, 'term' | 'commands'>, lines: number[]) {
    const marks = new Map<number, string>();
    const notes = new Map<number, string>();
    const blocks = new Map<number, Command>();
    const top = lines[0] ?? 0;
    const bottom = lines.at(-1) ?? -1;
    const here = session.term.evicted + session.term.scrollbackLength + session.term.cursorY;
    for (let i = session.commands.length - 1; i >= 0; i--) {
        const c = session.commands[i];
        if (c.outputLine === null && c.endLine === null) continue;
        const last = (c.endLine ?? here + 1) - 1;
        if (last < top) break;
        if (c.promptLine > bottom) continue;
        const kind = c.endLine === null ? '1' : c.exit === 0 ? '2' : '3';
        for (const n of lines) {
            if (n < c.promptLine || n > last) continue;
            const first = n === c.promptLine;
            const end = n === last;
            marks.set(n, kind + (first && end ? 'only' : first ? 'start' : end ? 'end' : 'body'));
            if (first) blocks.set(n, c);
        }
        if (c.endLine !== null && c.startedAt !== null && c.endedAt !== null && blocks.has(c.promptLine)) {
            const ms = c.endedAt - c.startedAt;
            const text = (c.exit !== 0 && c.exit !== null ? `exit ${c.exit}  ` : '') + (ms >= 500 ? durationText(ms) : '');
            if (text !== '') notes.set(c.promptLine, text);
        }
    }
    return { marks, notes, blocks };
}

interface LinkTarget {
    line: number;
    from: number;
    to: number;
    text: string;
    kind: string;
    file?: Hint['file'];
}

/** What's under a cell that can be opened: a link a program marked, or an address or file written in the text. */
export function linkAt(session: Session, lineNumber: number, col: number): LinkTarget | null {
    const line = session.lineNumbered(lineNumber);
    if (!line) return null;
    const term = session.term;
    const id = term.styles.get(line.st[Math.min(col, line.length - 1)] ?? 0).link;
    if (id !== 0) {
        let from = col;
        let to = col;
        while (from > 0 && term.styles.get(line.st[from - 1]).link === id) from--;
        while (to + 1 < line.length && term.styles.get(line.st[to + 1]).link === id) to++;
        return { line: lineNumber, from, to: to + 1, text: term.link(id), kind: 'url' };
    }
    const hints = findHints(term, [{ number: lineNumber, line }]);
    const hit = hints.find((h) => h.col <= col && col < h.endCol && (h.kind === 'url' || h.kind === 'path'));
    return hit ? { line: lineNumber, from: hit.col, to: hit.endCol, text: hit.text, kind: hit.kind, file: hit.file } : null;
}

export interface PaneActions {
    openLink: (target: LinkTarget, session: Session) => void;
    paste: (session: Session) => void;
    copy: (session: Session) => void;
    contextMenu: (session: Session, x: number, y: number, link: LinkTarget | null) => void;
    focusMe: () => void;
    blockMenu: (session: Session, command: Command) => void;
    broadcast: boolean;
    terminalFocus: boolean;
    committed: (session: Session, text: string) => void;
}

export const PaneResizePausedContext = createContext(false);

export function TerminalPane(props: {
    session: Session;
    settings: Settings;
    scheme: Scheme;
    active: boolean;
    actions: PaneActions;
}) {
    const { session, settings, scheme, active, actions } = props;
    const pauseResize = useContext(PaneResizePausedContext);
    const version = useSession(session);
    void version;
    const term = session.term;
    const look = useMemo(() => lookFor(settings, scheme), [settings.fontFamily, settings.fontSize, settings.lineHeight, settings.showMarks, settings.ligatures, scheme]);
    const box = useRef<HTMLDivElement>(null);
    const input = useRef<HTMLInputElement>(null);
    const [preedit, setPreedit] = useState('');
    const allowInput = active && actions.terminalFocus && !session.historyOpen && !session.search.open &&
        !session.copyMode && !session.hints && session.exitCode === null;
    const [size, setSize] = useState<{ w: number; h: number } | null>(null);
    const [hover, setHover] = useState<LinkTarget | null>(null);
    const drag = useRef<{ button: number; reported: boolean } | null>(null);
    useEffect(() => {
        if (allowInput) input.current?.focus();
        else if (input.current === document.activeElement) input.current?.blur();
    }, [allowInput]);

    // Keep the PTY dimensions while the sidebar moves, then measure the settled pane.
    useEffect(() => {
        const element = box.current;
        if (!element || pauseResize) return;
        const measure = () => {
            const r = element.getBoundingClientRect();
            setSize((was) => (was && Math.abs(was.w - r.width) < 0.5 && Math.abs(was.h - r.height) < 0.5 ? was : { w: r.width, h: r.height }));
        };
        measure();
        const observer = new ResizeObserver(measure);
        observer.observe(element);
        return () => observer.disconnect();
    }, [pauseResize]);

    const cols = size ? Math.max(2, Math.floor((size.w - look.gutter - SCROLLBAR) / look.cw)) : term.cols;
    const rows = size ? Math.max(1, Math.floor(size.h / look.ch)) : term.rows;
    useEffect(() => {
        term.cellPx = { w: Math.round(look.cw), h: look.ch };
    }, [look.cw, look.ch]);
    useEffect(() => {
        if (size) session.resize(cols, rows);
    }, [cols, rows, size !== null]);

    const top = session.topLine;
    const bottomTop = term.evicted + term.scrollbackLength;
    const atBottom = top >= bottomTop;
    const spans = useMemo(
        () => (session.search.open ? spansByLine(session.search.matches, session.search.current, term.cols) : null),
        [session.search.matches, session.search.current, session.search.open, term.cols],
    );
    const palette = term.palette.size;
    const folds = !term.alternate && session.folds.size > 0
        ? session.commands.filter((command) => session.folds.has(command.id) && command.outputLine !== null && command.endLine !== null && command.endLine > top) : [];
    let foldIndex = 0;
    let nextLine = top;
    const visible: { n: number; fold?: Command }[] = [];
    for (let y = 0; y < term.rows; y++) {
        while (foldIndex < folds.length && folds[foldIndex].endLine! <= nextLine) foldIndex++;
        const fold = folds[foldIndex];
        if (fold && fold.outputLine! <= nextLine && nextLine < fold.endLine!) {
            visible.push({ n: fold.outputLine!, fold });
            nextLine = fold.endLine!;
        } else visible.push({ n: nextLine++ });
    }
    const rowsByLine = new Map(visible.flatMap(({ n, fold }, row) => fold ? [] : [[n, row] as const]));
    const blockMarks = settings.showMarks && !term.alternate ? marksFor(session, [...rowsByLine.keys()]) : null;

    const rowEls = [];
    for (let y = 0; y < term.rows; y++) {
        const { n, fold } = visible[y];
        if (fold) {
            rowEls.push(
                <button key={`fold:${fold.id}`} className="absolute left-3 rounded px-2 text-left text-[11px] hover:bg-white/10"
                    style={{ top: y * look.ch, height: look.ch, color: css(scheme.ansi[4]), background: withAlpha(scheme.ansi[4], 0.08) }}
                    onPointerDown={(e: any) => e.stopPropagation()} onClick={() => session.toggleFold(fold)}>
                    Expand {fold.endLine! - fold.outputLine!} {fold.endLine! - fold.outputLine! === 1 ? 'line' : 'lines'}
                </button>,
            );
            continue;
        }
        const line = term.lineNumbered(n);
        const span = session.selectionSpan(n);
        const found = spans?.get(n);
        const block = blockMarks?.blocks.get(n);
        // Keyed by the line's number and placed by a wrapper, so when output scrolls the rows already drawn only move, and only
        // the new line draws.
        rowEls.push(
            <group key={n} position="absolute" left={0} top={y * look.ch}>
            {line && session.highlights.get(line) === line.ver ? <group position="absolute" left={look.gutter} top={0} width={term.cols * look.cw} height={look.ch} background={withAlpha(scheme.ansi[3], 0.16)} /> : null}
            <Row
                styles={term.styles}
                paletteMap={term.palette}
                ansi={term.theme.ansi}
                line={line}
                ver={line ? line.ver : 0}
                cols={term.cols}
                look={look}
                sel={span ? `${span[0]}-${Math.min(span[1], term.cols)}` : ''}
                found={found ? found.map(([a, b, c]) => `${a}-${Math.min(b, term.cols)}-${c ? 1 : 0}`).join(';') : ''}
                link={hover && hover.line === n ? `${hover.from}-${hover.to}` : ''}
                palette={palette}
                mark={blockMarks?.marks.get(n) ?? ''}
                note={(block && session.bookmarks.has(block.id) ? '★ ' : '') + (blockMarks?.notes.get(n) ?? '')}
            />
            {block ? <button aria-label="Select command output" title="Select output. Right-click for block actions."
                className="absolute left-0 top-0" style={{ width: look.gutter, height: look.ch }}
                onPointerDown={(e: any) => e.stopPropagation()} onClick={() => { actions.focusMe(); session.selectOutput(block); }}
                onContextMenu={(e: any) => { e.stopPropagation(); actions.blockMenu(session, block); }} /> : null}
            </group>,
        );
    }

    // ---- The cursor

    const cursorShapeCode = settings.cursorShape === 'program' ? term.cursorShape : settings.cursorShape === 'block' ? 2 : settings.cursorShape === 'underline' ? 4 : 6;
    const blinking = settings.cursorBlink && (settings.cursorShape === 'program' ? term.cursorShape % 2 === 1 || term.cursorShape === 0 : true) && active;
    const copyCursor = session.copyMode?.cursor;
    const cursorLine = copyCursor ? copyCursor.line : bottomTop + term.cursorY;
    const cursorCol = copyCursor ? copyCursor.col : term.cursorX;
    const cursorRow = rowsByLine.get(cursorLine) ?? -1;
    const cursorVisible = (copyCursor ? true : term.cursorVisible && atBottom) && cursorRow >= 0;
    const under = term.lineNumbered(cursorLine);
    let cursorChar = ' ';
    if (under) {
        const c = under.ch[cursorCol];
        if (c !== 0 && c !== WIDE_TAIL && c !== undefined) cursorChar = under.extra?.get(cursorCol) ?? String.fromCodePoint(c);
    }
    const isWide = under ? under.ch[cursorCol + 1] === WIDE_TAIL : false;
    const cx = look.gutter + cursorCol * look.cw;
    const cy = cursorRow * look.ch;
    const cursorW = (isWide ? 2 : 1) * look.cw;
    const glide = settings.smoothCursor && settings.animations;
    const cursorColor = copyCursor ? scheme.ansi[3] : look.cursorColor;
    const hollow = !active && !copyCursor;

    // ---- The mouse

    const cell = (e: { clientX: number; clientY: number }) => {
        const r = box.current!.getBoundingClientRect();
        const x = e.clientX - r.left - look.gutter;
        const y = e.clientY - r.top;
        return {
            col: Math.max(0, Math.min(term.cols - 1, Math.floor(x / look.cw))),
            row: Math.max(0, Math.min(term.rows - 1, Math.floor(y / look.ch))),
        };
    };
    const reporting = () => term.pointer !== 'none';

    const onPointerDown = (e: any) => {
        actions.focusMe();
        if (allowInput && e.button === 0) input.current?.focus();
        const { col, row } = cell(e);
        const line = visible[row].n;
        if (e.button === 2) {
            if (reporting() && !e.shiftKey) {
                session.mouse({ button: 2, kind: 'press', col, row, shiftKey: e.shiftKey, altKey: e.altKey, ctrlKey: e.ctrlKey });
                drag.current = { button: 2, reported: true };
                return;
            }
            return;
        }
        if (e.button === 1) {
            if (reporting() && !e.shiftKey) {
                session.mouse({ button: 1, kind: 'press', col, row, shiftKey: e.shiftKey, altKey: e.altKey, ctrlKey: e.ctrlKey });
                drag.current = { button: 1, reported: true };
            } else {
                actions.paste(session);
            }
            return;
        }
        if (e.button !== 0) return;
        if (session.hints) return;
        e.currentTarget.setPointerCapture?.(e.pointerId);
        if (reporting() && !e.shiftKey) {
            session.mouse({ button: 0, kind: 'press', col, row, shiftKey: e.shiftKey, altKey: e.altKey, ctrlKey: e.ctrlKey });
            drag.current = { button: 0, reported: true };
            return;
        }
        const openByClick = settings.clickLinks === 'click' || (settings.clickLinks === 'ctrl' && (e.ctrlKey || e.metaKey));
        if (openByClick) {
            const target = linkAt(session, line, col);
            if (target) {
                actions.openLink(target, session);
                return;
            }
        }
        const clicks = e.detail ?? 1;
        session.startSelection({ line, col }, clicks >= 3 ? 'line' : clicks === 2 ? 'word' : e.altKey ? 'rect' : 'cell');
        drag.current = { button: 0, reported: false };
    };

    const onPointerMove = (e: any) => {
        const { col, row } = cell(e);
        const line = visible[row].n;
        const d = drag.current;
        if (d?.reported) {
            session.mouse({ button: d.button as 0 | 1 | 2, kind: 'move', col, row, shiftKey: e.shiftKey, altKey: e.altKey, ctrlKey: e.ctrlKey });
            return;
        }
        if (d && d.button === 0) {
            session.extendSelection({ line, col });
            // Dragging past the top or bottom scrolls.
            const r = box.current!.getBoundingClientRect();
            if (e.clientY < r.top) session.scrollBy(-Math.ceil((r.top - e.clientY) / look.ch));
            else if (e.clientY > r.bottom) session.scrollBy(Math.ceil((e.clientY - r.bottom) / look.ch));
            return;
        }
        if (reporting() && !e.shiftKey && term.pointer === 'any') {
            session.mouse({ button: 'none', kind: 'move', col, row, shiftKey: e.shiftKey, altKey: e.altKey, ctrlKey: e.ctrlKey });
        }
        if (settings.clickLinks !== 'off' && (settings.clickLinks === 'click' || e.ctrlKey || e.metaKey) && !reporting()) {
            const target = linkAt(session, line, col);
            setHover((was) => (target && was && was.line === target.line && was.from === target.from ? was : target));
        } else if (hover) {
            setHover(null);
        }
    };

    const onPointerUp = (e: any) => {
        const { col, row } = cell(e);
        const d = drag.current;
        drag.current = null;
        if (d?.reported) {
            session.mouse({ button: d.button as 0 | 1 | 2, kind: 'release', col, row, shiftKey: e.shiftKey, altKey: e.altKey, ctrlKey: e.ctrlKey });
            return;
        }
        if (d && d.button === 0) {
            const selection = session.selection;
            if (selection && selection.mode === 'cell' && selection.anchor.line === selection.head.line && selection.anchor.col === selection.head.col) {
                session.clearSelection();
            } else if (selection && settings.copyOnSelect) {
                actions.copy(session);
            }
        }
    };

    const onWheel = (e: any) => {
        const { col, row } = cell(e);
        const up = e.deltaY < 0;
        const notches = Math.max(1, Math.round(Math.abs(e.deltaY) / 100)) * (e.ctrlKey ? 0 : 1);
        if (e.ctrlKey || e.metaKey) return;
        if (reporting() && !e.shiftKey) {
            for (let k = 0; k < Math.max(1, notches); k++) {
                session.mouse({ button: 'none', kind: up ? 'wheelUp' : 'wheelDown', col, row, shiftKey: false, altKey: e.altKey, ctrlKey: false });
            }
            return;
        }
        if (term.alternate) {
            // A full screen program with no mouse handling gets the arrow keys, as other terminals send.
            const key = up ? (term.appCursor ? '\x1bOA' : '\x1b[A') : term.appCursor ? '\x1bOB' : '\x1b[B';
            session.send(key.repeat(Math.max(1, notches) * 3));
            return;
        }
        session.scrollBy((up ? -1 : 1) * Math.max(1, notches) * 3);
    };

    const onContextMenu = (e: any) => {
        if (reporting() && !e.shiftKey) return;
        const { col, row } = cell(e);
        actions.contextMenu(session, e.clientX, e.clientY, linkAt(session, visible[row].n, col));
    };

    // ---- The pieces over the rows

    const hintEls = session.hints
        ? session.hints.hints
              .filter((h) => rowsByLine.has(h.line))
              .map((h) => {
                  const typed = session.hints!.typed;
                  if (!h.label.startsWith(typed)) return null;
                  return (
                      <group key={'h' + h.line + '-' + h.col}>
                          <group
                              position="absolute" left={look.gutter + h.col * look.cw} top={rowsByLine.get(h.line)! * look.ch} width={(h.endCol - h.col) * look.cw}
                              height={look.ch} background={withAlpha(scheme.ansi[3], 0.18)} borderColor={withAlpha(scheme.ansi[3], 0.7)} borderWidth={1} radius={3}
                          />
                          <text
                              position="absolute" left={look.gutter + h.col * look.cw - 2} top={rowsByLine.get(h.line)! * look.ch + 1} height={look.ch - 2} paddingX={3}
                              radius={4} background={css(scheme.ansi[3])} color={css(scheme.bg)} fontFamily={look.family} fontSize={look.size - 2} fontWeight="bold"
                          >
                              {h.label.slice(0, typed.length).toUpperCase() === typed.toUpperCase() && typed.length > 0 ? h.label.slice(typed.length) : h.label}
                          </text>
                      </group>
                  );
              })
        : null;

    const imageEls = (settings.graphics === 'off' ? [] : term.images)
        .filter((image) => image.line + image.rows > top && image.line < nextLine && !folds.some((fold) => image.line >= fold.outputLine! && image.line < fold.endLine!))
        .sort((a, b) => (a.z ?? 0) - (b.z ?? 0))
        .map((image) => ({ below: (image.z ?? 0) < 0, element:
            <TerminalImage key={image.id} image={image} look={look} row={rowsByLine.get(image.line) ?? image.line - top} />,
        }));

    const sticky = settings.stickyCommand && !atBottom ? stickyCommand(session, top) : null;

    return (
        <div
            ref={box}
            className="relative size-full overflow-hidden"
            style={{ cursor: hover ? 'pointer' : 'text' }}
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            onPointerLeave={() => hover && setHover(null)}
            onWheel={onWheel}
            onContextMenu={onContextMenu}
        >
            {imageEls.filter((image) => image.below).map((image) => image.element)}
            <group position="absolute" left={0} top={0} width={look.gutter + term.cols * look.cw} height={term.rows * look.ch}>
                {rowEls}
            </group>
            {imageEls.filter((image) => !image.below).map((image) => image.element)}
            {active ? (
                <input ref={input} data-terminal-input="" aria-label="Terminal keyboard input" tabIndex={-1}
                    className="pointer-events-none absolute opacity-0"
                    style={{ left: cx, top: Math.max(0, cy), width: Math.max(2, look.cw), height: look.ch, padding: 0, border: 'none' }}
                    onBlur={() => setPreedit('')}
                    onCompositionUpdate={(event: any) => setPreedit(event.data ?? '')}
                    onCompositionEnd={(event: any) => {
                        setPreedit('');
                        if (allowInput) actions.committed(session, event.data ?? '');
                    }}
                    onChange={() => { if (input.current) input.current.value = ''; }} />
            ) : null}
            {preedit && allowInput ? <span className="pointer-events-none absolute px-1" style={{ left: cx, top: Math.max(0, cy), height: look.ch, fontFamily: look.family, fontSize: look.size, color: css(scheme.bg), background: css(scheme.cursor) }}>{preedit}</span> : null}
            {cursorVisible && settings.cursorTrail && glide && !copyCursor
                ? // Ghosts of the cursor that take longer to get where it goes, so as it glides it leaves a fading trail.
                  [150, 210, 280].map((ms, i) => (
                      <div
                          key={'trail' + i}
                          className="pointer-events-none absolute left-0 top-0 rounded-[2px]"
                          style={{
                              transform: `translate(${cx}px, ${cy}px)`,
                              transition: `transform ${ms}ms cubic-bezier(0.2, 0.9, 0.25, 1)`,
                              width: cursorW,
                              height: look.ch,
                              background: withAlpha(cursorColor, 0.3 - i * 0.08),
                          }}
                      />
                  ))
                : null}
            {cursorVisible ? (
                <div
                    className="pointer-events-none absolute left-0 top-0"
                    style={{
                        transform: `translate(${cx}px, ${cy}px)`,
                        transition: glide ? 'transform 90ms cubic-bezier(0.2, 0.9, 0.25, 1)' : 'none',
                        width: cursorW,
                        height: look.ch,
                    }}
                >
                    <div
                        className={blinking ? 'cursor-blink' : ''}
                        style={{
                            position: 'absolute',
                            left: 0,
                            top: cursorShapeCode >= 3 && cursorShapeCode <= 4 ? look.ch - 2 : 0,
                            width: cursorShapeCode >= 5 ? 2 : cursorW,
                            height: cursorShapeCode >= 3 && cursorShapeCode <= 4 ? 2 : look.ch,
                            background: hollow ? 'transparent' : css(cursorColor),
                            border: hollow ? `1px solid ${css(cursorColor)}` : 'none',
                            borderRadius: cursorShapeCode >= 5 ? 1 : 2,
                        }}
                    />
                    {cursorShapeCode <= 2 && !hollow ? (
                        <text
                            position="absolute" left={0} top={look.offset} noWrap fontFamily={look.family} fontSize={look.size} color={css(scheme.bg)}
                            width={cursorW + 2} height={look.ch - look.offset}
                        >
                            {cursorChar}
                        </text>
                    ) : null}
                </div>
            ) : null}
            {hintEls}
            {sticky ? (
                <div
                    className="absolute left-0 top-0 flex h-[26px] cursor-pointer items-center gap-2 px-3 text-[12px]"
                    style={{ width: look.gutter + term.cols * look.cw, background: withAlpha(mix(scheme.bg, scheme.fg, 0.08), 0.96), color: css(scheme.fg), borderBottom: `1px solid ${withAlpha(scheme.fg, 0.1)}` }}
                    onClick={() => session.scrollTo(sticky.promptLine)}
                >
                    <span style={{ color: css(sticky.exit === null ? scheme.ansi[4] : sticky.exit === 0 ? scheme.ansi[2] : scheme.ansi[1]) }}>❯</span>
                    <span className="truncate" style={{ fontFamily: look.family }}>{sticky.text || 'command'}</span>
                    {sticky.endedAt !== null && sticky.startedAt !== null ? (
                        <span style={{ color: withAlpha(scheme.fg, 0.5) }}>{durationText(sticky.endedAt - sticky.startedAt)}</span>
                    ) : null}
                </div>
            ) : null}
            <Scrollbar session={session} scheme={scheme} look={look} show={settings.scrollMinimap} height={size?.h ?? 0} animate={settings.animations} version={version} />
            {!atBottom && !term.alternate ? (
                <div
                    className="absolute bottom-3 flex h-7 cursor-pointer items-center gap-1.5 rounded-full px-3 text-[12px] shadow-lg"
                    style={{ right: SCROLLBAR + 10, background: css(mix(scheme.bg, scheme.fg, 0.16)), color: css(scheme.fg) }}
                    onClick={() => session.scrollToBottom()}
                >
                    ↓ Latest
                </div>
            ) : null}
            {session.copyMode ? <CopyBadge session={session} scheme={scheme} /> : null}
            {session.historyOpen && !term.alternate ? <CommandHistory session={session} scheme={scheme} open={(command) => actions.blockMenu(session, command)} /> : null}
            {session.exitCode !== null ? (
                <div
                    className="absolute bottom-3 left-4 rounded-md px-3 py-1.5 text-[12px]"
                    style={{ background: css(mix(scheme.bg, scheme.fg, 0.12)), color: css(scheme.fg) }}
                >
                    {session.startError ? session.startError : `Process exited with code ${session.exitCode}.`} Press Enter to restart, or close this column.
                </div>
            ) : null}
        </div>
    );
}

function CommandHistory({ session, scheme, open }: { session: Session; scheme: Scheme; open: (command: Command) => void }) {
    const [bookmarked, setBookmarked] = useState(false);
    const [query, setQuery] = useState('');
    const needle = query.trim().toLowerCase();
    const list = bookmarked ? [...session.bookmarks.values()].map(({ command }) => command) : session.commands;
    const commands = list.filter((c) => c.text && c.text.toLowerCase().includes(needle)).slice(-200).reverse();
    const close = () => { session.historyOpen = false; session.changed(); };
    return (
        <div className="absolute bottom-0 right-3 top-0 flex w-[340px] max-w-[95%] flex-col rounded-lg border p-2 text-[12px]"
            style={{ color: css(scheme.fg), background: css(mix(scheme.bg, scheme.frame, 0.45)), borderColor: withAlpha(scheme.fg, 0.16) }}
            onPointerDown={(e: any) => e.stopPropagation()} onPointerUp={(e: any) => e.stopPropagation()} onWheel={(e: any) => e.stopPropagation()}>
            <div className="flex items-center gap-1 border-b border-white/10 pb-2">
                <button className="rounded px-2 py-1" style={{ background: !bookmarked ? withAlpha(scheme.fg, 0.13) : undefined }} onClick={() => setBookmarked(false)}>History</button>
                <button className="rounded px-2 py-1" style={{ background: bookmarked ? withAlpha(scheme.fg, 0.13) : undefined }} onClick={() => setBookmarked(true)}>Bookmarks</button>
                <button className="ml-auto rounded px-2 py-1 hover:bg-white/10" onClick={close}>Close</button>
            </div>
            <input aria-label="Filter commands" placeholder="Filter commands" value={query} className="my-2 rounded bg-white/10 px-2 py-1.5"
                onChange={(e: any) => setQuery(e.target.value)} onKeyDown={(e: any) => { if (e.key === 'Escape') close(); }} />
            <div className="min-h-0 flex-1 overflow-y-auto">
                {commands.length === 0 ? <div className="px-2 py-2 opacity-65">{needle ? 'No matching commands' : bookmarked ? 'No bookmarks in this pane' : 'No commands marked by the shell'}</div> : null}
                {commands.map((command) => {
                    const retained = command.promptLine >= session.term.evicted && session.commands.some((entry) => entry.id === command.id);
                    return (
                        <div key={command.id} className="flex items-center gap-1 rounded hover:bg-white/10">
                            <button title={command.text} className="min-w-0 flex-1 truncate px-2 py-1.5 text-left" style={{ fontFamily: 'Cascadia Mono' }}
                                onClick={() => { if (!retained) open(command); else { session.scrollTo(command.promptLine); close(); } }}>
                                {session.bookmarks.has(command.id) ? '★ ' : ''}{command.text}
                            </button>
                            <button className="rounded px-2 py-1 hover:bg-white/10" aria-label="Command actions" onClick={() => open(command)}>…</button>
                        </div>
                    );
                })}
            </div>
        </div>
    );
}

function stickyCommand(session: Session, top: number): Command | null {
    const c = session.commandAt(top);
    if (!c || c.outputLine === null) return null;
    if (c.promptLine >= top) return null;
    if (c.endLine !== null && c.endLine <= top + 1) return null;
    return c;
}

function CopyBadge(props: { session: Session; scheme: Scheme }) {
    const mode = props.session.copyMode!;
    const text = mode.searching ? (mode.searching.forward ? '/' : '?') + mode.searching.text : mode.selecting ? `-- ${mode.selecting.kind === 'line' ? 'V-LINE' : mode.selecting.kind === 'block' ? 'V-BLOCK' : 'VISUAL'} --` : '-- COPY MODE --  v select · y copy · / search · q quit';
    return (
        <div
            className="absolute bottom-3 left-4 rounded-md px-3 py-1.5 text-[12px]"
            style={{ background: css(props.scheme.ansi[3]), color: css(props.scheme.bg), fontWeight: 600 }}
        >
            {text}
        </div>
    );
}

/** Scrolls, a frame at a time, to a line; any other scrolling in the meantime (the wheel, a drag, output) takes over. */
const glides = new WeakMap<Session, number>();
function stopGlide(session: Session) {
    const id = glides.get(session);
    if (id !== undefined) cancelAnimationFrame(id);
    glides.delete(session);
}
function glideTo(session: Session, target: number, animate: boolean) {
    stopGlide(session);
    const from = session.topLine;
    if (!animate || Math.abs(target - from) < 3) {
        session.scrollTo(target);
        return;
    }
    const ms = Math.min(360, 150 + Math.sqrt(Math.abs(target - from)) * 8);
    const started = performance.now();
    let last = from;
    const step = (now: number) => {
        if (session.topLine !== last) {
            glides.delete(session);
            return;
        }
        const t = Math.min(1, (now - started) / ms);
        session.scrollTo(Math.round(from + (target - from) * (1 - (1 - t) ** 3)));
        last = session.topLine;
        if (t < 1) glides.set(session, requestAnimationFrame(step));
        else glides.delete(session);
    };
    glides.set(session, requestAnimationFrame(step));
}

const MIN_THUMB = 32;
const EDGE = 2;
const EASE = 'cubic-bezier(0.2, 0.9, 0.25, 1)';

/**
 * The strip down the right edge: how far back the view is, and where commands and matches are in all that there is.
 * It's a quiet pill in the text's own color that grows and takes the accent when it's taken hold of, and it moves, grows and
 * shrinks as the cursor does rather than jumping. Pressing the strip glides to that spot, and the pill follows the pointer
 * from where it was grabbed.
 */
function Scrollbar(props: { session: Session; scheme: Scheme; look: Look; show: boolean; height: number; animate: boolean; version: number }) {
    const { session, scheme, height, animate } = props;
    // It reads the session as it changes, not through a prop, so it's told when that has.
    void props.version;
    const term = session.term;
    const total = term.lineCount;
    const [hover, setHover] = useState(false);
    const [dragging, setDragging] = useState(false);
    const [recent, setRecent] = useState(false);
    const track = useRef<HTMLDivElement>(null);
    const grab = useRef<number | null>(null);
    const pressed = useRef(0);
    const scrolled = session.scrolledTop;
    // It shows a little more for a moment after the view moved, so a wheel turn is seen on it.
    useEffect(() => {
        if (scrolled === null) return;
        setRecent(true);
        const id = setTimeout(() => setRecent(false), 900);
        return () => clearTimeout(id);
    }, [scrolled]);
    if (term.alternate || term.scrollbackLength === 0 || height <= 0) return null;
    const first = term.evicted;
    const rows = term.rows;
    const range = Math.max(1, total - rows);
    const room = Math.max(1, height - EDGE * 2);
    const thumbH = Math.min(room, Math.max(MIN_THUMB, room * (rows / total)));
    const span = Math.max(1, room - thumbH);
    const thumbTop = EDGE + span * Math.max(0, Math.min(1, (session.topLine - first) / range));
    const ticks: { at: number; color: string; h: number; w: number }[] = [];
    if (props.show) {
        for (const c of session.commands) {
            ticks.push({ at: EDGE + room * ((c.promptLine - first) / total), color: c.exit === null || c.exit === 0 ? withAlpha(scheme.ansi[2], 0.7) : withAlpha(scheme.ansi[1], 0.95), h: 2, w: 3 });
        }
        if (session.search.open) {
            for (const m of session.search.matches) ticks.push({ at: EDGE + room * ((m.line - first) / total), color: css(scheme.ansi[3]), h: 3, w: 4 });
        }
    }
    const lineAt = (y: number, offset: number) => first + Math.round(Math.max(0, Math.min(1, (y - offset - EDGE) / span)) * range);
    const localY = (e: any) => e.clientY - track.current!.getBoundingClientRect().top;
    const lit = hover || dragging;
    const thumbColor = dragging ? withAlpha(scheme.ansi[4], 0.85) : hover ? withAlpha(scheme.fg, 0.4) : withAlpha(scheme.fg, recent ? 0.3 : 0.15);
    const following = session.scrolledTop === null;
    const transition = !animate ? 'none' : [
        // While it's dragged it's under the pointer, and while output follows the bottom it only grows, so neither is smoothed.
        ...(dragging || following ? [] : [`top 120ms ${EASE}`, `height 120ms ${EASE}`]),
        `width 140ms ${EASE}`, `right 140ms ${EASE}`, 'background-color 180ms ease-out',
    ].join(', ');
    return (
        <div
            ref={track}
            className="absolute right-0 top-0 h-full"
            style={{ width: SCROLLBAR, cursor: 'default' }}
            onPointerEnter={() => setHover(true)}
            onPointerLeave={() => setHover(false)}
            onPointerDown={(e: any) => {
                e.stopPropagation();
                if (e.button !== 0) return;
                e.currentTarget.setPointerCapture?.(e.pointerId);
                const y = localY(e);
                pressed.current = y;
                stopGlide(session);
                setDragging(true);
                if (y >= thumbTop && y <= thumbTop + thumbH) {
                    // On the pill: it stays where it is under the pointer.
                    grab.current = y - thumbTop;
                } else {
                    // Beside it: the pill goes there, centered on the pointer, and can be dragged on from it.
                    grab.current = thumbH / 2;
                    glideTo(session, lineAt(y, grab.current), animate);
                }
            }}
            onPointerMove={(e: any) => {
                if (grab.current === null || e.buttons !== 1) return;
                const y = localY(e);
                // The glide a press beside the pill started goes on until the pointer moves from where it pressed.
                if (glides.has(session) && Math.abs(y - pressed.current) < 4) return;
                const to = lineAt(y, grab.current);
                if (to !== session.topLine) {
                    stopGlide(session);
                    session.scrollTo(to);
                }
            }}
            onPointerUp={() => { grab.current = null; setDragging(false); }}
            onPointerCancel={() => { grab.current = null; setDragging(false); }}
        >
            <div
                className="absolute rounded-full"
                style={{ right: 2, top: EDGE, width: 8, height: room, background: withAlpha(scheme.fg, lit ? 0.06 : 0), transition: animate ? 'background-color 180ms ease-out' : 'none' }}
            />
            {ticks.map((t, i) => (
                <div key={i} className="absolute rounded-full" style={{ right: 0, top: t.at - t.h / 2, width: t.w, height: t.h, background: t.color }} />
            ))}
            <div
                className="absolute rounded-full"
                style={{
                    right: lit ? 3 : 4, width: lit ? 6 : 4, top: thumbTop, height: thumbH,
                    background: thumbColor, transition,
                }}
            />
        </div>
    );
}

