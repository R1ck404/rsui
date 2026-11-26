// Recorded tab frames and shell output share the recordings view and playback controls.

import { useEffect, useMemo, useRef, useState } from 'react';
import { shell } from 'rsui/desktop';
import { Row } from '../term/Row';
import { TerminalImage } from '../term/TerminalImage';
import { lookFor } from '../term/TerminalPane';
import { listRecordings, loadRecording } from '../term/recording';
import type { Cast } from '../term/recording';
import { loadTabFilm, readTabFrame, tabFrameAt } from '../term/tab-recording';
import type { TabFilm } from '../term/tab-recording';
import { Playback, replayClock } from '../term/timeline';
import type { Settings } from '../settings';
import type { Theme } from '../vt/terminal';
import { css, withAlpha } from '../themes';
import type { Scheme } from '../themes';
import { Icon } from './Icon';
import { PlaybackControls } from './PlaybackControls';
import { RecordedTab } from './RecordedTab';
import { loadTabSession } from '../term/tab-session';
import { RecordingLayout } from './RecordingLayout';
import type { RecordingNavigation } from './RecordingLayout';

const nameOf = (name: string) => name.replace(/(?:-\d{3}(?:-\d+)?)?\.(?:tabrec|cast|mp4)$/, '').replace('_', ' ').replace(/-(\d\d)-(\d\d)$/, ':$1:$2');

/** What's been recorded, newest first, to pick one from: in the window's sidebar, or at the side of the recordings tab. */
export function RecordingsNav(props: { chosen: string | null; scheme: Scheme; onChoose: (file: string) => void; version?: number }) {
    const { scheme } = props;
    const files = useMemo(() => listRecordings(), [props.version]);
    return (
        <div className="flex flex-col gap-px">
            <h2 className="px-1.5 pb-2 pt-1 text-[15px] font-semibold tracking-tight" style={{ color: css(scheme.fg) }}>Recordings</h2>
            {files.length === 0 ? (
                <div className="px-2 py-3 text-[12px] leading-[18px]" style={{ color: withAlpha(scheme.fg, 0.45) }}>
                    Nothing recorded yet. Use Record terminal tab to start.
                </div>
            ) : null}
            {files.map((f) => {
                const on = f.file === props.chosen;
                return (
                    <button
                        key={f.file}
                        className="flex flex-col items-start rounded-md px-2 py-1.5 text-left transition-colors duration-150"
                        style={{ background: on ? withAlpha(scheme.fg, 0.085) : 'transparent' }}
                        onClick={() => props.onChoose(f.file)}
                    >
                        <span className="text-[13px]" style={{ color: withAlpha(scheme.fg, on ? 0.92 : 0.72) }}>
                            {nameOf(f.name)}
                        </span>
                        <span className="text-[11.5px]" style={{ color: withAlpha(scheme.fg, 0.38) }}>
                            {f.name.endsWith('.tabrec') ? 'Terminal tab' : f.name.endsWith('.mp4') ? 'Window video' : 'Shell output'} · {f.size >= 1024 ** 2
                                ? `${(f.size / 1024 ** 2).toFixed(1)} MB` : `${(f.size / 1024).toFixed(0)} KB`}
                        </span>
                    </button>
                );
            })}
        </div>
    );
}

/** The chosen recording, played back. */
export function RecordingsBody(props: { chosen: string | null; settings: Settings; scheme: Scheme; theme: Theme; nav: RecordingNavigation; active: boolean }) {
    const { scheme, chosen } = props;
    const isVideo = chosen?.endsWith('.mp4');
    const isTab = chosen?.endsWith('.tabrec');
    const tab = useMemo(() => chosen && isTab ? loadTabSession(chosen) : null, [chosen, isTab]);
    const tabFilm = useMemo(() => chosen && isTab && !tab ? loadTabFilm(chosen) : null, [chosen, isTab, tab]);
    const [loaded, setLoaded] = useState<{ file: string; cast: Cast | null } | null>(null);
    useEffect(() => {
        if (!chosen || isVideo || isTab) return;
        let current = true;
        void loadRecording(chosen).then((cast) => { if (current) setLoaded({ file: chosen, cast }); });
        return () => { current = false; };
    }, [chosen, isVideo, isTab]);
    const cast = loaded?.file === chosen ? loaded.cast : null;
    if (!chosen || (!isVideo && !cast && !tab && !tabFilm)) {
        return (
            <RecordingLayout nav={props.nav} scheme={scheme}>
                <div className="flex size-full flex-col items-center justify-center gap-2" style={{ color: withAlpha(scheme.fg, 0.5) }}>
                    <Icon name="video" size={22} />
                    <div className="text-[13px]">{!chosen ? 'Choose a recording' : isTab || loaded?.file === chosen ? 'Could not read this recording' : 'Loading recording…'}</div>
                </div>
            </RecordingLayout>
        );
    }
    const title = nameOf(chosen.split(/[\\/]/).pop() ?? '');
    if (tab) return <RecordedTab key={chosen} file={chosen} data={tab} scheme={scheme} settings={props.settings}
        nav={props.nav} title={title} active={props.active} />;
    return (
        <RecordingLayout nav={props.nav} scheme={scheme}>
            <div className="flex size-full flex-col">
                <div className="flex h-11 shrink-0 items-center justify-between px-4">
                    <span className="text-[13px] font-medium" style={{ color: withAlpha(scheme.fg, 0.9) }}>
                        {title}
                    </span>
                    <button className="rounded-md px-2.5 py-1 text-[12px] hover:bg-white/10" style={{ color: withAlpha(scheme.fg, 0.6) }} onClick={() => void shell.reveal(chosen)}>
                        Show in folder
                    </button>
                </div>
                {isVideo ? <WindowVideo key={chosen} file={chosen} scheme={scheme} />
                    : tabFilm ? <TabFilmPlayer key={chosen} film={tabFilm} scheme={scheme} settings={props.settings} />
                    : cast ? <Player key={chosen} cast={cast} settings={props.settings} scheme={scheme} theme={props.theme} /> : null}
            </div>
        </RecordingLayout>
    );
}

function TabFilmPlayer({ film, scheme, settings }: { film: TabFilm; scheme: Scheme; settings: Settings }) {
    const duration = film.duration / 1000;
    const [at, setAt] = useState(0);
    const [playing, setPlaying] = useState(duration > 0);
    const [speed, setSpeed] = useState(settings.replaySpeed);
    const clock = useRef(0);
    const seek = (next: number) => { clock.current = next; setAt(next); };
    useEffect(() => {
        if (!playing) return;
        let last = replayClock();
        const timer = setInterval(() => {
            const now = replayClock();
            const next = Math.min(duration, clock.current + ((now - last) / 1000) * Number(speed));
            last = now;
            seek(next);
            if (next >= duration) setPlaying(false);
        }, 33);
        return () => clearInterval(timer);
    }, [playing, speed, duration]);
    const frame = tabFrameAt(film, at * 1000);
    const src = useMemo(() => {
        try { return readTabFrame(film, frame); }
        catch { return ''; }
    }, [film, frame]);
    return (
        <div className="flex min-h-0 flex-1 flex-col">
            <div className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden" style={{ background: css(film.frames[frame].background) }}>
                {src ? <img className="h-full w-full object-contain" src={src} alt="Recorded terminal tab" />
                    : <span className="text-[13px]">Could not read this frame</span>}
            </div>
            <PlaybackControls scheme={scheme} at={at} duration={duration} playing={playing} speed={speed} setSpeed={setSpeed}
                onToggle={() => {
                    if (!playing && at >= duration) seek(0);
                    setPlaying(!playing);
                }}
                onSeek={(fraction) => { setPlaying(false); seek(duration * fraction); }} />
        </div>
    );
}

function WindowVideo({ file, scheme }: { file: string; scheme: Scheme }) {
    const [error, setError] = useState(false);
    return (
        <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 px-4">
            <button className="flex items-center gap-2 rounded-md px-4 py-2 text-[13px] font-medium"
                style={{ background: css(scheme.ansi[4]), color: css(scheme.bg) }}
                onClick={() => { setError(false); void shell.open(file).catch(() => setError(true)); }}>
                <Icon name="play" size={14} />
                Open in video player
            </button>
            {error ? <p role="status" className="text-[12px]" style={{ color: css(scheme.ansi[1]) }}>
                Windows could not open this video. Use Show in folder to open the file from Explorer.
            </p> : null}
        </div>
    );
}

/** The newest recording, which a recordings tab opens on. */
export function newestRecording(): string | null {
    return listRecordings()[0]?.file ?? null;
}

export function Player(props: { cast: Cast; settings: Settings; scheme: Scheme; theme: Theme; live?: boolean }) {
    const { cast, scheme } = props;
    const look = useMemo(() => lookFor({ ...props.settings, showMarks: false }, scheme), [props.settings.fontFamily, props.settings.fontSize, props.settings.lineHeight, props.settings.ligatures, scheme]);
    const playback = useMemo(() => {
        const value = new Playback(cast, (term) => {
            term.theme = props.theme;
            term.graphics = props.settings.graphics;
            term.cellPx = { w: look.cw, h: look.ch };
        });
        value.seek(0);
        return value;
    }, [cast]);
    const term = playback.term;
    const [at, setAt] = useState(0);
    const atRef = useRef(0);
    const [playing, setPlaying] = useState(!props.live && cast.duration > 0);
    const [speed, setSpeed] = useState(props.settings.replaySpeed);
    const lastPaint = useRef(0);
    const pendingSeek = useRef<number | null>(null);
    const frame = useRef<ReturnType<typeof setTimeout> | null>(null);
    const box = useRef<HTMLDivElement>(null);
    const [room, setRoom] = useState({ w: 800, h: 500 });

    useEffect(() => {
        const element = box.current;
        if (!element) return;
        const measure = () => {
            const r = element.getBoundingClientRect();
            setRoom({ w: r.width, h: r.height });
        };
        measure();
        const observer = new ResizeObserver(measure);
        observer.observe(element);
        return () => observer.disconnect();
    }, []);

    const seek = (t: number) => {
        atRef.current = t;
        playback.seek(t * 1000);
        playback.term.theme = props.theme;
        setAt(t);
    };

    useEffect(() => {
        if (!playing) return;
        let last = replayClock();
        const timer = setInterval(() => {
            const now = replayClock();
            const next = Math.min(cast.duration, atRef.current + ((now - last) / 1000) * Number(speed));
            last = now;
            atRef.current = next;
            const changed = playback.seek(next * 1000);
            playback.term.theme = props.theme;
            if (changed || now - lastPaint.current > 120 || next >= cast.duration) {
                lastPaint.current = now;
                setAt(next);
            }
            if (next >= cast.duration) setPlaying(false);
        }, 33);
        return () => clearInterval(timer);
    }, [playing, speed, playback, props.theme]);
    useEffect(() => () => { if (frame.current !== null) clearTimeout(frame.current); }, []);

    const width = term.cols * look.cw + look.gutter;
    const height = term.rows * look.ch;
    const scale = Math.max(0.01, Math.min(1, (room.w - 32) / width, (room.h - 32) / height));
    const rows = [];
    for (let y = 0; y < term.rows; y++) {
        const line = term.lineAt(term.scrollbackLength + y);
        rows.push(<Row key={y} styles={term.styles} paletteMap={term.palette} ansi={term.theme.ansi} line={line} ver={line.ver} cols={term.cols} look={look} sel="" found="" link="" palette={0} mark="" note="" />);
    }
    const cursor = term.cursorVisible ? { x: look.gutter + term.cursorX * look.cw, y: term.cursorY * look.ch } : null;
    const top = term.evicted + term.scrollbackLength;
    const images = props.settings.graphics === 'off' ? [] : term.images.filter((image) => image.line + image.rows > top && image.line < top + term.rows)
        .sort((a, b) => (a.z ?? 0) - (b.z ?? 0));
    const scrub = (fraction: number) => {
        setPlaying(false);
        pendingSeek.current = fraction * cast.duration;
        if (frame.current !== null) return;
        frame.current = setTimeout(() => {
            frame.current = null;
            if (pendingSeek.current !== null) seek(pendingSeek.current);
        }, 16);
    };
    return (
        <div className="flex min-h-0 flex-1 flex-col">
            <div ref={box} className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden">
                <div
                    className="relative overflow-hidden rounded-xl"
                    style={{
                        width,
                        height,
                        transform: `scale(${scale})`,
                        background: props.settings.background === 'none' ? css(scheme.bg) : undefined,
                        boxShadow: `0 0 0 1px ${withAlpha(scheme.fg, 0.1)}`,
                    }}
                >
                    {images.filter((image) => (image.z ?? 0) < 0).map((image) => <TerminalImage key={image.id} image={image} look={look} row={image.line - top} />)}
                    <column position="absolute" left={0} top={0}>
                        {rows}
                    </column>
                    {images.filter((image) => (image.z ?? 0) >= 0).map((image) => <TerminalImage key={image.id} image={image} look={look} row={image.line - top} />)}
                    {cursor ? <div className="absolute" style={{ left: cursor.x, top: cursor.y, width: look.cw, height: look.ch, background: withAlpha(scheme.cursor, 0.7) }} /> : null}
                </div>
            </div>
            <PlaybackControls scheme={scheme} at={at} duration={cast.duration} playing={playing} speed={speed} setSpeed={setSpeed}
                onToggle={() => {
                    if (!playing && at >= cast.duration) seek(0);
                    setPlaying(!playing);
                }}
                onSeek={scrub} />
        </div>
    );
}

