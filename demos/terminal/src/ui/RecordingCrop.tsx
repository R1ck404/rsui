import { useRef } from 'react';
import { cropEdge, moveCrop } from '../term/recording-edits';
import type { Crop, CropEdge } from '../term/recording-edits';
import { css } from '../themes';
import type { Scheme } from '../themes';
import { useRecordingDrag } from './recording-drag';

type Drag = { edge: CropEdge; offset: number } | { crop: Crop; x: number; y: number };
const horizontal = (edge: CropEdge) => edge === 'left' || edge === 'right';
const percent = (fraction: number) => `${fraction * 100}%`;

export function RecordingCrop({ crop, scheme, onChange }: { crop: Crop; scheme: Scheme; onChange: (crop: Crop) => void }) {
    const box = useRef<HTMLDivElement>(null);
    const accent = css(scheme.ansi[4]);
    const drag = useRecordingDrag<Drag>((intent, point) => {
        const area = box.current?.getBoundingClientRect();
        if (!area || area.width <= 0 || area.height <= 0) return;
        if ('edge' in intent) {
            const { edge, offset } = intent;
            const fraction = horizontal(edge) ? (point.x - area.left) / area.width : (point.y - area.top) / area.height;
            onChange(cropEdge(crop, edge, fraction - offset));
        } else {
            onChange(moveCrop(intent.crop, (point.x - intent.x) / area.width, (point.y - intent.y) / area.height));
        }
    });
    const frame = { left: percent(crop.left), top: percent(crop.top), right: percent(1 - crop.right), bottom: percent(1 - crop.bottom) };
    return (
        <div ref={box} className="pointer-events-none absolute inset-0" aria-label="Crop frame">
            <div aria-hidden="true" className="absolute inset-0 overflow-hidden">
                <div className="absolute border-2"
                    style={{ ...frame, borderColor: accent, boxShadow: '0 0 0 4096px rgba(0,0,0,0.56)' }}>
                    {[1 / 3, 2 / 3].map((part) => (
                        <div key={part} className="pointer-events-none absolute inset-0">
                            <div className="absolute inset-y-0 w-px bg-white/35" style={{ left: percent(part) }} />
                            <div className="absolute inset-x-0 h-px bg-white/35" style={{ top: percent(part) }} />
                        </div>
                    ))}
                </div>
            </div>
            <div role="group" aria-label="Move crop area" tabIndex={0}
                className="pointer-events-auto absolute cursor-move focus-visible:outline-2"
                style={{ ...frame, touchAction: 'none' }}
                onKeyDown={(event) => {
                    if (event.ctrlKey || event.altKey || event.metaKey) return;
                    const step = event.shiftKey ? 0.05 : 0.01;
                    if (event.key === 'ArrowLeft') onChange(moveCrop(crop, -step, 0));
                    else if (event.key === 'ArrowRight') onChange(moveCrop(crop, step, 0));
                    else if (event.key === 'ArrowUp') onChange(moveCrop(crop, 0, -step));
                    else if (event.key === 'ArrowDown') onChange(moveCrop(crop, 0, step));
                    else return;
                    event.preventDefault();
                }}
                onPointerDown={(event) => drag.begin(event, { crop, x: event.clientX, y: event.clientY })} {...drag.events} />
            {(['left', 'top', 'right', 'bottom'] as const).map((edge) => {
                const across = horizontal(edge);
                return <div key={edge} role="slider" aria-label={`Crop ${edge} edge`} aria-orientation={across ? 'horizontal' : 'vertical'}
                    aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(crop[edge] * 100)} tabIndex={0}
                    className="pointer-events-auto absolute flex -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded focus-visible:outline-2"
                    style={{ left: percent(across ? crop[edge] : (crop.left + crop.right) / 2),
                        top: percent(across ? (crop.top + crop.bottom) / 2 : crop[edge]),
                        width: across ? 24 : 44, height: across ? 44 : 24, cursor: across ? 'ew-resize' : 'ns-resize', touchAction: 'none' }}
                    onKeyDown={(event) => {
                        if (event.ctrlKey || event.altKey || event.metaKey) return;
                        const before = across ? 'ArrowLeft' : 'ArrowUp';
                        const after = across ? 'ArrowRight' : 'ArrowDown';
                        if (event.key !== before && event.key !== after) return;
                        event.preventDefault();
                        const step = event.shiftKey ? 0.05 : 0.01;
                        onChange(cropEdge(crop, edge, crop[edge] + (event.key === before ? -step : step)));
                    }}
                    onPointerDown={(event) => {
                        const area = box.current?.getBoundingClientRect();
                        if (!area || area.width <= 0 || area.height <= 0) return;
                        const fraction = across ? (event.clientX - area.left) / area.width : (event.clientY - area.top) / area.height;
                        drag.begin(event, { edge, offset: fraction - crop[edge] });
                    }} {...drag.events}>
                    <div aria-hidden="true" className="pointer-events-none rounded-sm" style={{ width: across ? 5 : 24, height: across ? 24 : 5, background: accent }} />
                </div>;
            })}
        </div>
    );
}
