import type { Settings } from '../settings';
import { css, withAlpha } from '../themes';
import type { Scheme } from '../themes';
import { Icon } from './Icon';
import { Segmented } from './controls';

export const playbackTime = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;

export function PlaybackControls({ scheme, at, duration, playing, speed, setSpeed, onToggle, onSeek, disabled = false }: {
    scheme: Scheme; at: number; duration: number; playing: boolean; speed: Settings['replaySpeed'];
    setSpeed: (speed: Settings['replaySpeed']) => void; onToggle: () => void; onSeek: (fraction: number) => void;
    disabled?: boolean;
}) {
    const scrub = (event: any) => {
        const area = event.currentTarget.getBoundingClientRect();
        onSeek(Math.max(0, Math.min(1, (event.clientX - area.left) / Math.max(1, area.width))));
    };
    return (
        <div className="flex items-center gap-3 px-5 pb-4 pt-2" aria-disabled={disabled || undefined} style={{ opacity: disabled ? 0.55 : 1 }}>
            <button aria-label={playing ? 'Pause' : 'Play'} className="flex size-9 items-center justify-center rounded-full"
                style={{ background: css(scheme.ansi[4]), color: css(scheme.bg) }} disabled={disabled} onClick={onToggle}>
                <Icon name={playing ? 'pause' : 'play'} size={15} />
            </button>
            <span className="w-24 text-[12px] tabular-nums" style={{ color: withAlpha(scheme.fg, 0.6) }}>
                {playbackTime(at)} / {playbackTime(duration)}
            </span>
            <div className="relative h-6 flex-1 cursor-pointer"
                onPointerDown={(event: any) => { if (!disabled) { event.currentTarget.setPointerCapture?.(event.pointerId); scrub(event); } }}
                onPointerMove={(event: any) => !disabled && event.buttons === 1 && scrub(event)}>
                <div className="absolute left-0 right-0 top-1/2 h-1 -translate-y-1/2 rounded-full" style={{ background: withAlpha(scheme.fg, 0.15) }} />
                <div className="absolute left-0 top-1/2 h-1 -translate-y-1/2 rounded-full"
                    style={{ width: `${(at / Math.max(0.001, duration)) * 100}%`, background: css(scheme.ansi[4]) }} />
            </div>
            <Segmented scheme={scheme} value={speed} options={[{ value: '1', label: '1×' }, { value: '2', label: '2×' }, { value: '4', label: '4×' }, { value: '8', label: '8×' }]} onChange={setSpeed} disabled={disabled} />
        </div>
    );
}
