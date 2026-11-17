// The small controls everything else is made of: quiet icon buttons, switches, segmented choices and a slider-like stepper, drawn
// in the app's variables so they look the same on the frame and on a panel.

import { memo } from 'react';

type IconType = (props: { size?: number; strokeWidth?: number; className?: string }) => any;

/** A button that's only an icon, with nothing behind it until it's hovered, as Arc's and the Mac's toolbar buttons are. */
export const IconButton = memo(function IconButton(props: {
    icon: IconType;
    label: string;
    onClick: () => void;
    active?: boolean;
    disabled?: boolean;
    showLabel?: boolean;
    stroke?: number;
    size?: number;
    /** How big the button is, square, when it isn't the usual 28px. */
    box?: number;
}) {
    const { icon: Icon, label, onClick, active, disabled, showLabel, stroke, size, box } = props;
    return (
        <button
            title={label}
            aria-label={label}
            disabled={disabled}
            onClick={onClick}
            style={box ? { width: box, height: box } : undefined}
            className={
                'flex h-[28px] shrink-0 items-center justify-center gap-1.5 transition-colors duration-(--fx-speed) ' +
                (box ? 'rounded-(--fx-radius-lg) ' : 'rounded-(--fx-radius) ') +
                (showLabel ? 'px-2 ' : 'w-[28px] ') +
                (disabled
                    ? 'text-(--fx-faint) opacity-40'
                    : active
                      ? 'bg-(--fx-accent-soft) text-(--fx-accent)'
                      : 'text-(--fx-muted) hover:bg-(--fx-hover) hover:text-(--fx-text) active:bg-(--fx-press)')
            }
        >
            <Icon size={size ?? 16} strokeWidth={stroke ?? 1.6} />
            {showLabel ? <span className="text-[0.92em] whitespace-nowrap">{label}</span> : null}
        </button>
    );
});

export const Toggle = memo(function Toggle(props: { on: boolean; onChange: (on: boolean) => void; label: string }) {
    const { on, onChange, label } = props;
    return (
        <button
            title={label}
            aria-label={label}
            role="switch"
            onClick={() => onChange(!on)}
            className={
                'relative h-[18px] w-[32px] shrink-0 rounded-full transition-colors duration-(--fx-speed) ' +
                (on ? 'bg-(--fx-accent)' : 'bg-(--fx-press)')
            }
        >
            <span
                className={
                    'absolute top-[2px] h-[14px] w-[14px] rounded-full bg-white shadow-sm transition-all duration-(--fx-speed) ' +
                    (on ? 'left-[16px]' : 'left-[2px]')
                }
            />
        </button>
    );
});

function SegmentedView<T extends string | number>(props: { value: T; options: [T, string][]; onChange: (value: T) => void; small?: boolean }) {
    const { value, options, onChange, small } = props;
    return (
        <div className="flex flex-wrap gap-0.5 rounded-(--fx-radius) bg-(--fx-input) p-[3px]">
            {options.map(([option, label]) => (
                <button
                    key={String(option)}
                    onClick={() => onChange(option)}
                    className={
                        'rounded-[calc(var(--fx-radius)-2px)] whitespace-nowrap transition-colors duration-(--fx-speed) ' +
                        (small ? 'px-2 py-0.5 text-[0.88em] ' : 'px-2.5 py-1 text-[0.92em] ') +
                        (option === value ? 'bg-(--fx-chip) text-(--fx-text) shadow-(--fx-tab-shadow)' : 'text-(--fx-muted) hover:text-(--fx-text)')
                    }
                >
                    {label}
                </button>
            ))}
        </div>
    );
}

/** A row of choices, one of them chosen. */
export const Segmented = memo(SegmentedView) as typeof SegmentedView;

/** Choices that are icons, as the view switcher is. */
export const IconSegmented = memo(function IconSegmented(props: {
    value: string;
    options: [string, IconType, string][];
    onChange: (value: any) => void;
}) {
    const { value, options, onChange } = props;
    return (
        <div className="flex shrink-0 gap-0.5 rounded-(--fx-radius) bg-(--fx-input) p-[3px]">
            {options.map(([option, Icon, label]) => (
                <button
                    key={option}
                    title={label}
                    aria-label={label}
                    onClick={() => onChange(option)}
                    className={
                        'flex h-[22px] w-[28px] items-center justify-center rounded-[calc(var(--fx-radius)-2px)] transition-colors duration-(--fx-speed) ' +
                        (option === value ? 'bg-(--fx-chip) text-(--fx-text) shadow-(--fx-tab-shadow)' : 'text-(--fx-faint) hover:text-(--fx-text)')
                    }
                >
                    <Icon size={14} strokeWidth={1.7} />
                </button>
            ))}
        </div>
    );
});

export const Stepper = memo(function Stepper(props: { value: number; min: number; max: number; step: number; unit?: string; onChange: (value: number) => void }) {
    const { value, min, max, step, unit, onChange } = props;
    const set = (next: number) => onChange(Math.min(max, Math.max(min, Math.round(next / step) * step)));
    const share = (value - min) / (max - min);
    return (
        <div className="flex items-center gap-2">
            <button
                onClick={() => set(value - step)}
                className="flex h-[24px] w-[24px] items-center justify-center rounded-full text-(--fx-muted) hover:bg-(--fx-hover) hover:text-(--fx-text)"
            >
                −
            </button>
            <div
                className="relative h-[4px] w-[112px] cursor-pointer rounded-full bg-(--fx-press)"
                onClick={(event: any) => set(min + (event.offsetX / Math.max(1, event.currentTarget.offsetWidth)) * (max - min))}
            >
                <div className="absolute top-0 left-0 h-[4px] rounded-full bg-(--fx-accent)" style={{ width: `${share * 100}%` }} />
                <div
                    className="absolute top-[-5px] h-[14px] w-[14px] rounded-full bg-white shadow-sm"
                    style={{ left: `calc(${share * 100}% - 7px)` }}
                />
            </div>
            <button
                onClick={() => set(value + step)}
                className="flex h-[24px] w-[24px] items-center justify-center rounded-full text-(--fx-muted) hover:bg-(--fx-hover) hover:text-(--fx-text)"
            >
                +
            </button>
            <span className="w-[48px] text-right text-[0.88em] text-(--fx-muted) tabular-nums">
                {value}
                {unit ? ' ' + unit : ''}
            </span>
        </div>
    );
});

export const Kbd = memo(function Kbd(props: { children: any }) {
    return (
        <span className="rounded-[4px] bg-(--fx-input) px-1.5 py-px font-mono text-[0.85em] text-(--fx-muted)">
            {props.children}
        </span>
    );
});

/** How full something is, as a bar: a drive, say. Red past 90%. */
export const UsageBar = memo(function UsageBar(props: { used: number; accent: string; height: number }) {
    const { used, accent, height } = props;
    return (
        <div className="flex w-full overflow-hidden rounded-full bg-(--fx-press)" style={{ height }}>
            <div className="rounded-full" style={{ width: `${Math.round(used * 100)}%`, height, background: used > 0.9 ? '#ef4444' : accent }} />
        </div>
    );
});
