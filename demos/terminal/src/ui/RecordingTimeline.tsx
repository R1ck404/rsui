import { useMemo, useRef } from 'react';
import { clamp, timecode, VIDEO_FRAME_MS } from '../term/recording-edits';
import type { TabRecording } from '../term/tab-session';
import { css, tokens, withAlpha } from '../themes';
import type { Scheme } from '../themes';
import { Icon } from './Icon';
import { useRecordingDrag } from './recording-drag';
import { editorButton } from './RecordingTools';

type TrimEdge = 'from' | 'to';
type Drag = { edge?: TrimEdge; offset: number };

/** At most one tick per time bucket, regardless of how much terminal output was recorded. */
function activityTicks({ panes, duration }: TabRecording) {
    const buckets = 72;
    if (duration <= 0) return [];
    const active = new Set<number>();
    for (const { events } of panes) {
        let low = 0;
        for (let bucket = 0; bucket < buckets && low < events.length; bucket++) {
            const end = (bucket + 1) * duration / buckets;
            if (events[low].t > end) continue;
            active.add(bucket);
            let high = events.length;
            while (low < high) {
                const mid = (low + high) >>> 1;
                if (events[mid].t <= end) low = mid + 1;
                else high = mid;
            }
        }
    }
    return [...active].map((bucket) => (bucket + 0.5) * 100 / buckets);
}

export function RecordingTimeline({ data, at, start, end, playing, busy, scheme, onSeek, onTrim, onToggle }: {
    data: TabRecording;
    at: number;
    start: number;
    end: number;
    playing: boolean;
    busy: boolean;
    scheme: Scheme;
    onSeek: (at: number) => void;
    onTrim: (edge: TrimEdge, at: number) => void;
    onToggle: () => void;
}) {
    const { duration } = data;
    const track = useRef<HTMLDivElement>(null);
    const t = tokens(scheme);
    const accent = css(scheme.ansi[4]);
    const ticks = useMemo(() => activityTicks(data), [data]);
    const percent = (time: number) => `${time / Math.max(1, duration) * 100}%`;
    const rulerTime = (time: number) => timecode(time).slice(0, duration < 4000 ? -2 : -4);
    const drag = useRecordingDrag<Drag>((intent, point) => {
        const rect = track.current?.getBoundingClientRect();
        if (!rect || rect.width <= 0 || busy) return;
        const next = clamp((point.x - rect.left - intent.offset) * duration / rect.width, 0, duration);
        if (intent.edge) onTrim(intent.edge, next);
        else onSeek(next);
    });
    const keyMove = (event: KeyboardEvent, current: number, move: (next: number) => void) => {
        if (busy || event.ctrlKey || event.metaKey || event.altKey) return;
        const step = event.shiftKey ? 1000 : VIDEO_FRAME_MS;
        if (event.key === 'ArrowLeft') move(current - step);
        else if (event.key === 'ArrowRight') move(current + step);
        else if (event.key === 'Home') move(0);
        else if (event.key === 'End') move(duration);
        else return;
        event.preventDefault();
    };
    return (
        <section aria-label="Timeline" aria-disabled={busy || undefined} className="shrink-0 px-3 pb-3" style={{ background: t.panel }}>
            <div className="flex h-10 items-center gap-1">
                <button className={editorButton + ' flex size-7 items-center justify-center px-0'} aria-label="Previous frame"
                    disabled={busy || at <= 0} onClick={() => onSeek(at - VIDEO_FRAME_MS)}><Icon name="back" size={14} /></button>
                <button className={editorButton + ' flex size-7 items-center justify-center px-0'} aria-label={playing ? 'Pause' : 'Play'}
                    style={{ background: t.chosen, color: accent }} disabled={busy} onClick={onToggle}><Icon name={playing ? 'pause' : 'play'} size={14} /></button>
                <button className={editorButton + ' flex size-7 items-center justify-center px-0'} aria-label="Next frame"
                    disabled={busy || at >= duration} onClick={() => onSeek(at + VIDEO_FRAME_MS)}><Icon name="forward" size={14} /></button>
                <span className="ml-2 text-[11px] tabular-nums font-mono">{timecode(at)} <span style={{ color: t.sub }}>/ {timecode(duration)}</span></span>
            </div>
            <div className="flex items-end">
                <div className="mr-3 flex h-8 shrink-0 items-center gap-1.5 text-[11px]" style={{ color: t.sub }}>
                    <Icon name="terminal" size={13} /><span>Terminal</span>
                </div>
                <div ref={track} className="relative mx-2 min-w-0 flex-1">
                    <div aria-hidden="true" className="relative flex h-5 items-start justify-between text-[10px] tabular-nums font-mono" style={{ color: t.sub }}>
                        {[0, 0.5, 1].map((fraction) => (
                            <span key={fraction} className="whitespace-nowrap">{rulerTime(duration * fraction)}</span>
                        ))}
                    </div>
                    <div className="relative h-8 rounded" style={{ background: t.hover }}>
                        <div aria-hidden="true" className="pointer-events-none absolute inset-y-0 rounded border"
                            style={{ left: percent(start), right: percent(duration - end), borderColor: withAlpha(scheme.ansi[4], 0.6), background: withAlpha(scheme.ansi[4], 0.17) }} />
                        {ticks.map((left) => (
                            <div key={left} aria-hidden="true" className="pointer-events-none absolute top-2.5 h-3 w-px"
                                style={{ left: `${left}%`, background: withAlpha(scheme.fg, 0.28) }} />
                        ))}
                    </div>
                    <div role="slider" aria-label="Playhead" aria-valuetext={timecode(at)} aria-valuemin={0} aria-valuemax={duration}
                        aria-valuenow={Math.round(at)} tabIndex={busy ? -1 : 0} className="absolute inset-0 cursor-pointer rounded focus-visible:outline-2"
                        style={{ touchAction: 'none' }} onKeyDown={(event) => keyMove(event, at, onSeek)}
                        onPointerDown={(event) => { if (!busy) drag.begin(event, { offset: 0 }); }} {...drag.events} />
                    <div aria-hidden="true" className="pointer-events-none absolute bottom-0 top-0 w-px" style={{ left: percent(at), background: accent }}>
                        <div className="absolute left-[-3px] top-0 h-1.5 w-[7px] rounded-sm" style={{ background: accent }} />
                    </div>
                    {(['from', 'to'] as const).map((edge) => {
                        const value = edge === 'from' ? start : end;
                        return <div key={edge} role="slider" aria-label={edge === 'from' ? 'Trim start' : 'Trim end'} aria-valuetext={timecode(value)}
                            aria-valuemin={0} aria-valuemax={duration} aria-valuenow={Math.round(value)} tabIndex={busy ? -1 : 0}
                            className="absolute bottom-0 flex h-8 w-4 cursor-ew-resize items-center justify-center rounded focus-visible:outline-2"
                            style={{ left: percent(value), transform: edge === 'from' ? 'translateX(-75%)' : 'translateX(-25%)', touchAction: 'none' }}
                            onKeyDown={(event) => keyMove(event, value, (next) => onTrim(edge, next))}
                            onPointerDown={(event) => {
                                const rect = track.current?.getBoundingClientRect();
                                if (!busy && rect) drag.begin(event, { edge, offset: event.clientX - rect.left - value / Math.max(1, duration) * rect.width });
                            }} {...drag.events}>
                            <div aria-hidden="true" className="pointer-events-none h-5 w-[5px] rounded-sm" style={{ background: accent }} />
                        </div>;
                    })}
                </div>
            </div>
        </section>
    );
}
