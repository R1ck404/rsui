import { memo, useEffect, useRef, useState } from 'react';
import type { Child } from 'rsui';
import type { Settings } from '../settings';
import type { PaneRect } from '../strip/panes';
import { paneStyle } from '../strip/panes';
import { FULL_FRAME, clamp } from '../term/recording-edits';
import type { Crop } from '../term/recording-edits';
import { keystrokeMotion, visibleKeystrokes } from '../term/recording-keys';
import type { KeystrokeAnimation, KeystrokeLayout, KeystrokePosition, RecordedKeystroke } from '../term/recording-keys';
import { Row } from '../term/Row';
import { spanOf } from '../term/Session';
import { TerminalImage } from '../term/TerminalImage';
import { lookFor, marksFor } from '../term/TerminalPane';
import { stateAt } from '../term/tab-session';
import type { RecordedView, TabRecording } from '../term/tab-session';
import { Playback } from '../term/timeline';
import { css, resolveScheme, tokens, withAlpha } from '../themes';
import type { Scheme } from '../themes';
import { Terminal, WIDE_TAIL } from '../vt/terminal';
import { Background, backgroundOptions } from './Background';

const ReplayRow = memo(Row);
type Look = ReturnType<typeof lookFor>;
const offset = (value: number) => Math.round(value * 1000) / 1000;

export function recordingTracks(data: TabRecording) {
    try {
        return new Map(data.panes.map((pane) => {
            const initial = Terminal.fromSnapshot(pane.initial);
            const playback = new Playback({
                header: { version: 2, width: initial.cols, height: initial.rows, timestamp: 0 },
                initial, events: pane.events, duration: data.duration / 1000,
            });
            return [pane.id, { pane, playback }] as const;
        }));
    } catch {
        return null;
    }
}

function terminalGeometry(panes: { term: { cols: number; rows: number }; rect: PaneRect }[], look: Look, settings: Settings, showHeader: boolean) {
    const padLeft = Math.max(0, settings.textPadding - (settings.showMarks ? 12 : 4));
    const padTop = showHeader ? 4 : Math.round(settings.textPadding * 0.75);
    const padBottom = Math.round(settings.textPadding / 2);
    const width = Math.max(1, ...panes.map(({ term, rect }) => Math.ceil((term.cols * look.cw + look.gutter + padLeft + 14) / rect.w)));
    const height = Math.max(1, ...panes.map(({ term, rect }) => Math.ceil((term.rows * look.ch + padTop + padBottom + (showHeader ? 32 : 0)) / rect.h)));
    return { width, height, padLeft, padTop };
}

/** The source canvas keeps the first frame's aspect ratio through recorded terminal resizes. */
export function recordingAspect(data: TabRecording, settings: Settings) {
    const layout = stateAt(data.layout, 0);
    const style = stateAt(data.styles, 0);
    const appearance = { ...settings, ...style };
    const look = lookFor(appearance, resolveScheme(style.scheme, style.schemeOverrides?.[style.scheme]));
    const panes = layout.panes.flatMap(({ id, rect }) => {
        const initial = data.panes.find((pane) => pane.id === id)?.initial;
        return initial ? [{ term: initial, rect }] : [];
    });
    const { width, height } = terminalGeometry(panes, look, appearance, layout.showHeader);
    const aspect = width / height;
    return Number.isFinite(aspect) && aspect > 0 ? aspect : 1;
}

const keyPlacement: Record<KeystrokePosition, { row: string; stack: string }> = {
    'bottom-center': { row: 'bottom-4 justify-center', stack: 'bottom-4 flex-col items-center' },
    'bottom-left': { row: 'bottom-4 justify-start', stack: 'bottom-4 flex-col items-start' },
    'bottom-right': { row: 'bottom-4 justify-end', stack: 'bottom-4 flex-col items-end' },
    'top-center': { row: 'top-4 justify-center', stack: 'top-4 flex-col-reverse items-center' },
};

export function RecordedPreview({ data, tracks, time, settings, look, colors, scheme, aspect, crop, cropOverlay, output, keystrokes }: {
    data: TabRecording;
    tracks: NonNullable<ReturnType<typeof recordingTracks>>;
    time: number;
    settings: Settings;
    look: Look;
    colors: Scheme;
    scheme: Scheme;
    aspect: number;
    crop: Crop;
    cropOverlay?: Child;
    output: { current: HTMLDivElement | null };
    keystrokes?: { events: readonly RecordedKeystroke[]; from: number; position: KeystrokePosition; hold: number;
        layout: KeystrokeLayout; limit: number; animation: KeystrokeAnimation };
}) {
    const stage = useRef<HTMLDivElement>(null);
    const [room, setRoom] = useState({ w: 800, h: 480 });
    useEffect(() => {
        const element = stage.current;
        if (!element) return;
        const measure = () => {
            const { width: w, height: h } = element.getBoundingClientRect();
            setRoom((previous) => previous.w === w && previous.h === h ? previous : { w, h });
        };
        measure();
        const observer = new ResizeObserver(measure);
        observer.observe(element);
        return () => observer.disconnect();
    }, []);

    const layout = stateAt(data.layout, time);
    const panes = layout.panes.flatMap(({ id, rect }) => {
        const track = tracks.get(id);
        if (!track) return [];
        track.playback.seek(time);
        return [{ id, rect, term: track.playback.term, view: stateAt(track.pane.views, time) }];
    });
    const { width, height, padLeft, padTop } = terminalGeometry(panes, look, settings, layout.showHeader);
    const clip = cropOverlay ? FULL_FRAME : crop;
    const clipWidth = clip.right - clip.left;
    const clipHeight = clip.bottom - clip.top;
    const units = Math.max(1, Math.min((room.w - 36) / (aspect * clipWidth), (room.h - 36) / clipHeight));
    const sourceWidth = units * aspect;
    const sourceHeight = units;
    const scale = Math.min(sourceWidth / width, sourceHeight / height);
    const t = tokens(scheme);
    const keys = keystrokes && !cropOverlay
        ? visibleKeystrokes(keystrokes.events, time, keystrokes.from, keystrokes.hold, keystrokes.limit) : [];

    return (
        <div ref={stage} className="relative flex min-h-0 min-w-0 flex-1 items-center justify-center overflow-hidden" style={{ background: t.frame }}>
            <div className="relative shrink-0" style={{ width: sourceWidth * clipWidth, height: sourceHeight * clipHeight }}>
                <div ref={output} className="absolute inset-0 overflow-hidden" aria-label="Video preview"
                    style={{ background: css(colors.frame), boxShadow: `0 0 0 1px ${withAlpha(scheme.fg, 0.18)}` }}>
                    <div className="absolute" style={{ left: offset(-sourceWidth * clip.left), top: offset(-sourceHeight * clip.top), width: sourceWidth, height: sourceHeight }}>
                        <div className="absolute overflow-hidden" style={{ width, height, left: offset((sourceWidth - width * scale) / 2),
                            top: offset((sourceHeight - height * scale) / 2), transform: `scale(${scale})`, transformOrigin: 'top left',
                            background: css(colors.frame), borderRadius: settings.radius }}>
                            <div className="absolute inset-0" style={{ background: withAlpha(colors.bg, settings.columnOpacity) }} />
                            <Background options={backgroundOptions(settings, colors)} time={time / 1000} />
                            {panes.map(({ id, term, view, rect }) => {
                                const divider = `1px solid ${withAlpha(colors.fg, 0.1)}`;
                                return (
                                    <div key={id} className="absolute overflow-hidden" style={{
                                        ...paneStyle(rect), borderLeft: rect.x > 0 ? divider : undefined, borderTop: rect.y > 0 ? divider : undefined,
                                    }}>
                                        {layout.showHeader ? <div className="flex h-8 items-center px-3 text-[12px]"
                                            style={{ color: withAlpha(colors.fg, layout.active === id ? 0.85 : 0.5) }}>{view.title}</div> : null}
                                        <div style={{ paddingLeft: padLeft, paddingTop: padTop }}>
                                            <RecordedPane term={term} view={view} look={look} settings={settings} scheme={colors} time={time} active={layout.active === id} />
                                        </div>
                                    </div>
                                );
                            })}
                        </div>
                    </div>
                    {keystrokes && keys.length > 0 ? (
                        <div aria-label="Recorded keystrokes"
                            className={`pointer-events-none absolute inset-x-2 flex gap-1.5 ${keystrokes.layout === 'stack'
                                ? keyPlacement[keystrokes.position].stack : 'flex-wrap items-center ' + keyPlacement[keystrokes.position].row}`}>
                            {keys.map(({ t, keys }, index) => (
                                <div key={`${t}:${index}`} className="flex shrink-0 items-center gap-0.5 rounded-md px-1.5 py-1"
                                    style={{ background: '#101114', color: '#f4f5f8', ...keystrokeMotion(keystrokes.animation, time - t, keystrokes.hold) }}>
                                    {keys.map((key, i) => (
                                        <kbd key={i} className="rounded-sm px-2 py-1 text-[14px] font-medium leading-none"
                                            style={{ background: '#33363e' }}>{key}</kbd>
                                    ))}
                                </div>
                            ))}
                        </div>
                    ) : null}
                </div>
                {cropOverlay}
            </div>
        </div>
    );
}

function RecordedPane({ term, view, look, settings, scheme, time, active }: {
    term: Terminal; view: RecordedView; look: ReturnType<typeof lookFor>; settings: Settings;
    scheme: Scheme; time: number; active: boolean;
}) {
    const bottom = term.evicted + term.scrollbackLength;
    const top = term.alternate ? bottom : clamp(view.top ?? bottom, term.evicted, bottom);
    const folds = !term.alternate && view.folds.length ? term.commands.filter((c) => view.folds.includes(c.id) && c.outputLine !== null && c.endLine !== null) : [];
    let lineNumber = top;
    const rows = Array.from({ length: term.rows }, () => {
        const fold = folds.find((c) => c.outputLine! <= lineNumber && lineNumber < c.endLine!);
        if (!fold) return { n: lineNumber++, fold };
        const n = fold.outputLine!;
        lineNumber = fold.endLine!;
        return { n, fold };
    });
    const marks = settings.showMarks && !term.alternate ? marksFor({ term, commands: term.commands }, rows.filter(({ fold }) => !fold).map(({ n }) => n)) : null;
    const cursorLine = view.cursor?.line ?? bottom + term.cursorY;
    const x = view.cursor?.col ?? term.cursorX;
    const y = rows.findIndex(({ n, fold }) => n === cursorLine && !fold);
    const under = term.lineNumbered(cursorLine);
    const char = under?.ch[x];
    const cursorWidth = look.cw * (under?.ch[x + 1] === WIDE_TAIL ? 2 : 1);
    const shape = settings.cursorShape === 'program' ? term.cursorShape : settings.cursorShape === 'bar' ? 6 : settings.cursorShape === 'underline' ? 4 : 2;
    const blink = active && settings.cursorBlink && (settings.cursorShape !== 'program' || term.cursorShape === 0 || term.cursorShape % 2 === 1);
    const cursor = y >= 0 && (view.cursor || (top === bottom && term.cursorVisible)) && (!blink || Math.floor(time / 500) % 2 === 0);
    const cursorColor = view.cursor ? scheme.ansi[3] : look.cursorColor;
    let paletteVersion = 0;
    for (const [index, color] of term.palette) paletteVersion = (31 * paletteVersion + index + 17 * color) | 0;
    const images = settings.graphics === 'off' ? [] : term.images.filter((image) => image.line + image.rows > top && image.line < lineNumber)
        .sort((a, b) => (a.z ?? 0) - (b.z ?? 0));
    const imageRow = (line: number) => line < top ? line - top : rows.findIndex(({ n, fold }) => n === line && !fold);
    const drawImages = (below: boolean) => images.filter((image) => ((image.z ?? 0) < 0) === below && (image.line < top || imageRow(image.line) >= 0))
        .map((image) => <TerminalImage key={image.id} image={image} look={look} row={imageRow(image.line)} />);
    return (
        <div className="relative overflow-hidden" style={{ height: term.rows * look.ch }}>
            {drawImages(true)}
            {rows.map(({ n, fold }, i) => {
                const line = term.lineNumbered(n);
                const selection = view.selection && spanOf(view.selection, n, term.cols);
                const block = marks?.blocks.get(n);
                return <group key={i} position="absolute" left={0} top={i * look.ch} width={term.cols * look.cw + look.gutter} height={look.ch}>
                    {fold ? <span className="absolute left-3 rounded px-2 text-[11px]" style={{ color: css(scheme.ansi[4]), background: withAlpha(scheme.ansi[4], 0.08) }}>Expand {fold.endLine! - fold.outputLine!} lines</span>
                        : <ReplayRow styles={term.styles} paletteMap={term.palette} ansi={scheme.ansi}
                            line={line} ver={line?.ver ?? 0} cols={term.cols} look={look}
                            sel={selection ? `${selection[0]}-${selection[1]}` : ''} found="" link="" palette={paletteVersion}
                            mark={marks?.marks.get(n) ?? ''} note={(block && view.bookmarks.includes(block.id) ? '★ ' : '') + (marks?.notes.get(n) ?? '')} />}
                </group>;
            })}
            {drawImages(false)}
            {cursor ? <div className="absolute" style={{ left: look.gutter + x * look.cw, top: y * look.ch, width: cursorWidth, height: look.ch }}>
                <div className="absolute left-0" style={{
                    top: shape === 3 || shape === 4 ? look.ch - 2 : 0,
                    width: shape >= 5 ? 2 : cursorWidth, height: shape === 3 || shape === 4 ? 2 : look.ch,
                    background: active || view.cursor ? css(cursorColor) : 'transparent',
                    border: active || view.cursor ? 'none' : `1px solid ${css(cursorColor)}`, borderRadius: 2,
                }} />
                {shape <= 2 && (active || view.cursor) && char && char !== WIDE_TAIL ? <text position="absolute" left={0} top={look.offset} noWrap
                    fontFamily={look.family} fontSize={look.size} color={css(scheme.bg)} width={cursorWidth + 2} height={look.ch - look.offset}>
                    {under?.extra?.get(x) ?? String.fromCodePoint(char)}
                </text> : null}
            </div> : null}
        </div>
    );
}
