import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { mainWindow } from 'rsui';
import { dialogs, shell } from 'rsui/desktop';
import type { Settings } from '../settings';
import { exportVideo } from '../term/export-video';
import { clamp, FULL_FRAME, normalizeCrop, trimTime } from '../term/recording-edits';
import type { Crop } from '../term/recording-edits';
import { KEYSTROKE_ANIMATIONS, KEYSTROKE_CATEGORIES, KEYSTROKE_DURATIONS, KEYSTROKE_LAYOUTS, KEYSTROKE_LIMITS,
    KEYSTROKE_POSITIONS, filterKeystrokes, keystrokeOption, nextKeyIndex } from '../term/recording-keys';
import { saveTabSession, stateAt } from '../term/tab-session';
import type { RecordingEdits, TabRecording } from '../term/tab-session';
import { lookFor } from '../term/TerminalPane';
import { replayClock } from '../term/timeline';
import { css, resolveScheme, tokens } from '../themes';
import type { Scheme } from '../themes';
import { Icon } from './Icon';
import { PlaybackControls } from './PlaybackControls';
import { RecordedPreview, recordingAspect, recordingTracks } from './RecordedPreview';
import { RecordingCrop } from './RecordingCrop';
import { RecordingLayout } from './RecordingLayout';
import type { RecordingNavigation } from './RecordingLayout';
import { RecordingTimeline } from './RecordingTimeline';
import { RecordingCategories, RecordingInspector, editorButton } from './RecordingTools';
import type { RecordingTool } from './RecordingTools';

type TrimEdge = 'from' | 'to';
type Props = { file: string; title: string; data: TabRecording; settings: Settings; scheme: Scheme; nav: RecordingNavigation; active: boolean };

export function RecordedTab({ file, title, data, settings, scheme, nav, active }: Props) {
    const [edits, setEdits] = useState<RecordingEdits>(data.edits ?? {});
    const [saved, setSaved] = useState(edits);
    const [editing, setEditing] = useState(false);
    const [inspectorOpen, setInspectorOpen] = useState(true);
    const [tool, setTool] = useState<RecordingTool>('trim');
    const [playing, setPlaying] = useState(data.duration > 0);
    const [speed, setSpeed] = useState(settings.replaySpeed);
    const [at, setAt] = useState(Number.isFinite(edits.from) ? edits.from! : 0);
    const atRef = useRef(at);
    const viewport = useRef<HTMLDivElement>(null);
    const [progress, setProgress] = useState<number | null>(null);
    const [status, setStatus] = useState('');
    const [saving, setSaving] = useState(false);
    const [exporting, setExporting] = useState(false);
    const [exportResult, setExportResult] = useState<{ file: string } | { error: string } | null>(null);
    const exportPending = useRef(false);
    const activeRef = useRef(active);
    activeRef.current = active;
    const tracks = useMemo(() => recordingTracks(data), [data]);
    const aspect = useMemo(() => recordingAspect(data, settings), [data, settings]);
    const { duration } = data;
    const start = clamp(Number.isFinite(edits.from) ? edits.from! : 0, 0, duration);
    const end = clamp(Number.isFinite(edits.to) ? edits.to! : duration, start, duration);
    const time = editing ? clamp(at, 0, duration) : clamp(at, start, end);
    const selectedCategories = edits.keystrokeCategories;
    const keyCategories = useMemo(() => Array.isArray(selectedCategories)
        ? KEYSTROKE_CATEGORIES.filter((category) => selectedCategories.includes(category))
        : KEYSTROKE_CATEGORIES, [selectedCategories]);
    const keys = useMemo(() => data.keystrokes && filterKeystrokes(data.keystrokes, keyCategories), [data.keystrokes, keyCategories]);
    const position = edits.keystrokePosition && KEYSTROKE_POSITIONS.includes(edits.keystrokePosition) ? edits.keystrokePosition : 'bottom-center';
    const hold = edits.keystrokeDuration && KEYSTROKE_DURATIONS.includes(edits.keystrokeDuration) ? edits.keystrokeDuration : '1000';
    const layout = keystrokeOption(KEYSTROKE_LAYOUTS, edits.keystrokeLayout);
    const limit = keystrokeOption(KEYSTROKE_LIMITS, edits.keystrokeLimit);
    const animation = keystrokeOption(KEYSTROKE_ANIMATIONS, edits.keystrokeAnimation);
    const hasKeys = !!data.keystrokes?.length;
    const firstKey = keys?.[0];
    const lastKey = keys?.at(-1);
    const canStepBack = firstKey !== undefined && time > firstKey.t;
    const canStepForward = lastKey !== undefined && time < lastKey.t;
    const crop = useMemo(() => normalizeCrop(edits.crop), [edits.crop]);
    const busy = exporting || saving;
    const dirty = useMemo(() => JSON.stringify(edits) !== JSON.stringify(saved), [edits, saved]);
    const seek = useCallback((next: number) => {
        atRef.current = next;
        setAt(next);
    }, []);
    const scrub = useCallback((next: number) => {
        setPlaying(false);
        seek(clamp(next, 0, duration));
    }, [duration, seek]);
    const togglePlayback = () => {
        if (!playing && (atRef.current < start || atRef.current >= end)) seek(start);
        setPlaying(!playing);
    };

    useEffect(() => {
        if (!active || !playing) return;
        let last = replayClock();
        const timer = setInterval(() => {
            const now = replayClock();
            const next = clamp(atRef.current + (now - last) * (editing ? 1 : Number(speed)), start, end);
            last = now;
            seek(next);
            if (next >= end) setPlaying(false);
        }, 33);
        return () => clearInterval(timer);
    }, [active, playing, start, end, speed, editing, seek]);

    const style = stateAt(data.styles, time);
    const appearance = useMemo(() => ({ ...settings, ...style, background: edits.background ?? style.background }), [settings, style, edits.background]);
    const themeId = edits.scheme ?? style.scheme;
    const colors = useMemo(() => {
        const overrides = { ...style.schemeOverrides?.[themeId] };
        if (edits.fg !== undefined) overrides.fg = edits.fg;
        if (edits.bg !== undefined) overrides.bg = edits.bg;
        if (edits.cursor !== undefined) overrides.cursor = edits.cursor;
        return resolveScheme(themeId, overrides);
    }, [themeId, style.schemeOverrides, edits.fg, edits.bg, edits.cursor]);
    const look = useMemo(() => lookFor(appearance, colors), [appearance, colors]);

    const change = useCallback((patch: RecordingEdits) => {
        if (busy) return;
        setEdits((previous) => ({ ...previous, ...patch }));
        setStatus('');
    }, [busy]);
    const changeCrop = (next: Crop) => {
        const { left, top, right, bottom } = next;
        if (left === crop.left && top === crop.top && right === crop.right && bottom === crop.bottom) return;
        const fullFrame = left === FULL_FRAME.left && top === FULL_FRAME.top && right === FULL_FRAME.right && bottom === FULL_FRAME.bottom;
        change({ crop: fullFrame ? undefined : next });
    };
    const trim = useCallback((edge: TrimEdge, next: number) => {
        const point = trimTime(edge, next, start, end, duration);
        if (point !== (edge === 'from' ? start : end)) change({ [edge]: point });
        scrub(point);
    }, [start, end, duration, change, scrub]);
    const stepKey = useCallback((direction: -1 | 1) => {
        if (!keys?.length) return;
        const at = atRef.current;
        const index = direction === 1 ? nextKeyIndex(keys, at) : nextKeyIndex(keys, Math.ceil(at) - 1) - 1;
        const event = keys[index];
        if (event) scrub(event.t);
    }, [keys, scrub]);
    const categories = useMemo(() => editing && tracks ? (
        <RecordingCategories scheme={scheme} tool={tool} busy={busy}
            onTool={(next) => { setPlaying(false); setTool(next); setInspectorOpen(true); }} />
    ) : undefined, [editing, tracks, scheme, tool, busy]);
    const inspector = useMemo(() => editing && inspectorOpen && tracks ? (
        <RecordingInspector scheme={scheme} colors={colors} tool={tool} edits={edits} start={start} end={end} aspect={aspect}
            busy={busy} onChange={change} onMark={(edge) => trim(edge, atRef.current)}
            hasKeys={hasKeys} hasMatchingKeys={firstKey !== undefined} categories={keyCategories} position={position} hold={hold}
            presentation={{ layout, limit, animation }}
            canStepBack={canStepBack} canStepForward={canStepForward} onStepKey={stepKey}
            onRevert={dirty ? () => { setEdits(saved); setStatus(''); } : undefined}
            onHide={() => setInspectorOpen(false)} />
    ) : undefined, [editing, inspectorOpen, tracks, scheme, colors, tool, edits, start, end, aspect, busy, change, trim, hasKeys,
        firstKey, keyCategories, position, hold, layout, limit, animation, canStepBack, canStepForward, stepKey, dirty, saved]);
    const toggleEditor = () => {
        setPlaying(false);
        setEditing(!editing);
        if (editing) seek(clamp(atRef.current, start, end));
        else setInspectorOpen(true);
    };
    const save = async () => {
        setSaving(true);
        try {
            await saveTabSession(file, { ...data, edits });
            setSaved(edits);
            setStatus('Edits saved');
        } catch (error) {
            setStatus(`Could not save edits. ${error instanceof Error ? error.message : String(error)}`);
        } finally {
            setSaving(false);
        }
    };
    const exportMp4 = async () => {
        if (exportPending.current) return;
        exportPending.current = true;
        const before = atRef.current;
        setPlaying(false);
        setExporting(true);
        setStatus('');
        setExportResult(null);
        try {
            const destination = await dialogs.save({
                title: 'Export terminal video', start: 'terminal.mp4',
                filters: [{ name: 'MP4 video', extensions: ['mp4'] }],
            });
            if (!destination) return;
            const file = /\.mp4$/i.test(destination) ? destination : destination + '.mp4';
            let size: { width: number; height: number } | undefined;
            await exportVideo(file, start, end, async (frame, done) => {
                seek(frame);
                setProgress(done);
                // Capture the committed output surface, after the crop handles have been hidden.
                await new Promise<void>((resolve) => setTimeout(resolve, 0));
                const rect = viewport.current?.getBoundingClientRect();
                if (!rect || !activeRef.current) throw new Error('Keep the recording open during export.');
                if (rect.width < 2 || rect.height < 2) throw new Error('The video preview needs more room.');
                size ??= { width: rect.width, height: rect.height };
                if (Math.abs(rect.width - size.width) > 0.1 || Math.abs(rect.height - size.height) > 0.1) {
                    throw new Error('The video preview resized during export.');
                }
                const region = { x: rect.left, y: rect.top, ...size };
                if (typeof mainWindow.capturePixels === 'function') {
                    return { format: 'bgra', ...await mainWindow.capturePixels(region) };
                }
                // Older runtimes can still capture the preview as PNG.
                return { format: 'png', data: await mainWindow.capture(region) };
            });
            setExportResult({ file });
        } catch (error) {
            setExportResult({ error: error instanceof Error ? error.message : String(error) });
        } finally {
            exportPending.current = false;
            setExporting(false);
            setProgress(null);
            seek(before);
        }
    };
    const t = tokens(scheme);
    const message = exporting ? progress === null ? 'Preparing export' : progress === 1 ? 'Finishing MP4' : `Exporting ${Math.floor(progress * 100)}%`
        : saving ? 'Saving edits' : status || (dirty ? 'Unsaved edits' : '');
    const cropOverlay = editing && tool === 'crop' && !exporting ? (
        <RecordingCrop crop={crop} scheme={scheme} onChange={changeCrop} />
    ) : undefined;
    const previewAnimation = playing && settings.animations
        ? (animation === 'pop' && window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'fade' : animation)
        : 'none';

    return (
        <RecordingLayout nav={nav} sidebar={categories} scheme={scheme}>
            <div className="flex min-h-12 shrink-0 flex-wrap items-center gap-x-2 gap-y-1 px-2.5 py-1" style={{ background: t.panel }}>
                <div className="flex h-9 min-w-0 flex-1 basis-[100px] flex-col justify-center pl-1">
                    <div className="truncate text-[13px] font-semibold" title={title}>{title}</div>
                    <div role="status" className="truncate text-[11px]" style={{ color: t.sub }} title={message}>{message}</div>
                </div>
                {tracks ? (
                    <div className="ml-auto flex shrink-0 items-center gap-1.5">
                        {editing ? (
                            <>
                                {!inspectorOpen ? (
                                    <button className={editorButton + ' flex size-7 items-center justify-center px-0'}
                                        title="Show editing controls" aria-label="Show editing controls" disabled={busy}
                                        onClick={() => setInspectorOpen(true)}><Icon name="gear" size={15} /></button>
                                ) : null}
                                <button className={editorButton} disabled={busy || !dirty} style={{ background: t.chosen }}
                                    onClick={() => void save()}>Save edits</button>
                            </>
                        ) : (
                            <button className={editorButton + ' flex size-7 items-center justify-center px-0'} aria-label="Show in folder" title="Show in folder"
                                disabled={busy} onClick={() => void shell.reveal(file)}><Icon name="folder" size={15} /></button>
                        )}
                        <button className={editorButton} disabled={busy} style={{ background: t.chosen }} onClick={toggleEditor}>
                            {editing ? 'Done' : 'Edit recording'}
                        </button>
                        <button className={editorButton} style={{ background: css(scheme.ansi[4]), color: css(scheme.bg) }} disabled={busy || !active}
                            onClick={() => void exportMp4()}>Export MP4</button>
                    </div>
                ) : null}
            </div>
            {exportResult ? (
                <div className="flex max-h-24 shrink-0 flex-wrap items-center gap-x-2 gap-y-1 overflow-y-auto px-3 py-2 text-[12px]" style={{ background: t.panel }}>
                    {'error' in exportResult ? (
                        <p role="alert" className="min-w-0 break-words" style={{ color: css(scheme.ansi[1]) }}>
                            Could not export video. {exportResult.error}
                        </p>
                    ) : (
                        <>
                            <p role="status" className="min-w-0 flex-1 break-words" title={exportResult.file}>
                                Video exported to {exportResult.file}
                            </p>
                            <button className={editorButton} style={{ background: t.chosen }}
                                onClick={() => void shell.reveal(exportResult.file)}>Show exported video</button>
                        </>
                    )}
                    <button className={editorButton + ' flex size-7 items-center justify-center px-0'} aria-label="Dismiss export result"
                        onClick={() => setExportResult(null)}><Icon name="close" size={13} /></button>
                </div>
            ) : null}
            {tracks ? (
                <>
                    <div className="flex min-h-0 flex-1">
                        <RecordedPreview data={data} tracks={tracks} time={time} settings={appearance} look={look} colors={colors} scheme={scheme}
                            aspect={aspect} crop={crop} cropOverlay={cropOverlay} output={viewport}
                            keystrokes={edits.showKeystrokes === true && keys?.length
                                ? { events: keys, from: editing && !exporting ? 0 : start, position, hold: Number(hold), layout,
                                    limit: layout === 'latest' ? 1 : Number(limit),
                                    animation: exporting ? animation : previewAnimation }
                                : undefined} />
                        {inspector}
                    </div>
                    {editing ? (
                        <RecordingTimeline data={data} at={time} start={start} end={end} playing={playing} busy={busy}
                            scheme={scheme} onSeek={scrub} onTrim={trim} onToggle={togglePlayback} />
                    ) : (
                        <div style={{ background: t.panel }}>
                            <PlaybackControls scheme={scheme} at={(time - start) / 1000} duration={(end - start) / 1000}
                                playing={playing} speed={speed} setSpeed={setSpeed} onToggle={togglePlayback} disabled={busy}
                                onSeek={(fraction) => scrub(start + fraction * (end - start))} />
                        </div>
                    )}
                </>
            ) : <div className="p-4 text-[13px]">Could not reconstruct this recording.</div>}
        </RecordingLayout>
    );
}
