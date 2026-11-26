// Small controls the settings and the bars are made of.

import { Children, createContext, useContext, useId, useLayoutEffect, useRef, useState } from 'react';
import type { Child } from 'rsui';
import { css, mix, tokens, withAlpha } from '../themes';
import type { Scheme } from '../themes';
import { Icon } from './Icon';
import type { IconName } from './Icon';

const FieldContext = createContext<{ label: string; description?: string } | null>(null);

export function useFieldLabel(label?: string) {
    const field = useContext(FieldContext);
    return { 'aria-label': label ?? field?.label, 'aria-describedby': field?.description };
}

export function Switch(props: { on: boolean; onChange: (on: boolean) => void; scheme: Scheme; label?: string; disabled?: boolean }) {
    const { scheme, on } = props;
    const accessibility = useFieldLabel(props.label);
    return (
        <button
            type="button"
            role="switch"
            aria-checked={on}
            disabled={props.disabled}
            {...accessibility}
            className="settings-switch relative h-[20px] w-[36px] shrink-0 rounded-full focus-visible:outline-2 focus-visible:outline-offset-2"
            style={{ background: on ? css(mix(0x101522, scheme.ansi[4], 0.62)) : withAlpha(scheme.fg, 0.16) }}
            onClick={() => props.onChange(!on)}
        >
            <div
                className="settings-switch-thumb pointer-events-none absolute left-[3px] top-[3px] size-[14px] rounded-full"
                style={{ background: on ? '#ffffff' : css(scheme.fg), transform: `translateX(${on ? 16 : 0}px)` }}
            />
        </button>
    );
}

export function Segmented<T extends string>(props: { value: T; options: { value: T; label: string }[]; onChange: (value: T) => void; scheme: Scheme; disabled?: boolean }) {
    const { scheme } = props;
    const accessibility = useFieldLabel();
    const track = useRef<HTMLDivElement>(null);
    const options = useRef(new Map<T, HTMLButtonElement>());
    const [positions, setPositions] = useState(() => new Map<T, { x: number; width: number }>());
    const optionKeys = props.options.map(({ value }) => value).join('\0');
    const indicator = positions.get(props.value);
    useLayoutEffect(() => {
        const container = track.current;
        if (!container) return;
        const buttons = Array.from(options.current);
        const measure = () => {
            const left = container.getBoundingClientRect().left;
            const next = new Map<T, { x: number; width: number }>();
            for (const [value, button] of buttons) {
                const bounds = button.getBoundingClientRect();
                if (bounds.width > 0) next.set(value, { x: bounds.left - left, width: bounds.width });
            }
            setPositions((was) => {
                if (was.size !== next.size) return next;
                for (const [value, { x, width }] of next) {
                    const previous = was.get(value);
                    if (previous?.x !== x || previous.width !== width) return next;
                }
                return was;
            });
        };
        const observer = new ResizeObserver(measure);
        observer.observe(container);
        for (const [, button] of buttons) observer.observe(button);
        measure();
        return () => observer.disconnect();
    }, [optionKeys]);

    return (
        <div ref={track} role={accessibility['aria-label'] ? 'group' : undefined} {...accessibility}
            className="relative flex rounded-lg p-[2px]" style={{ background: withAlpha(scheme.fg, 0.08) }}>
            {indicator ? (
                <div aria-hidden="true" className="segmented-indicator pointer-events-none absolute bottom-[2px] left-0 top-[2px] w-[80px] rounded-md"
                    style={{
                        background: withAlpha(scheme.fg, 0.16),
                        transformOrigin: 'left center',
                        transform: `translateX(${indicator.x}px) scaleX(${indicator.width / 80})`,
                    }} />
            ) : null}
            {props.options.map((option) => {
                const on = option.value === props.value;
                return (
                    <button
                        key={option.value}
                        ref={(element) => {
                            if (element) options.current.set(option.value, element);
                            else options.current.delete(option.value);
                        }}
                        type="button"
                        aria-pressed={on}
                        disabled={props.disabled}
                        className="relative rounded-md px-2.5 py-1 text-[12px] font-medium focus-visible:outline-2 focus-visible:outline-offset-1"
                        style={{
                            background: 'transparent',
                            color: withAlpha(scheme.fg, on ? 1 : 0.6),
                        }}
                        onClick={() => { if (!on) props.onChange(option.value); }}
                    >
                        {option.label}
                    </button>
                );
            })}
        </div>
    );
}

export function Slider(props: { value: number; min: number; max: number; step: number; onChange: (value: number) => void; scheme: Scheme; format?: (v: number) => string }) {
    const { scheme, min, max, step, value, format, onChange } = props;
    const accessibility = useFieldLabel();
    const set = (next: number) => onChange(Number(Math.min(max, Math.max(min, next)).toFixed(10)));
    const shown = format ? format(value) : value;
    const fraction = Math.min(1, Math.max(0, (value - min) / (max - min)));
    return (
        <div className="flex max-w-full items-center gap-2.5">
            <div className="relative h-5 w-36 min-w-0">
                <div aria-hidden="true" className="pointer-events-none absolute inset-x-[7px] top-1/2 h-1 -translate-y-1/2 rounded-full"
                    style={{ background: withAlpha(scheme.fg, 0.18) }}>
                    <div className="h-full rounded-full" style={{ width: `${fraction * 100}%`, background: withAlpha(scheme.fg, 0.75) }} />
                    <div className="absolute top-1/2 size-[14px] -translate-x-1/2 -translate-y-1/2 rounded-full"
                        style={{ left: `${fraction * 100}%`, background: css(scheme.fg) }} />
                </div>
                <input type="range" {...{ min, max, step }} {...accessibility} aria-valuetext={format ? String(shown) : undefined}
                    className="settings-slider absolute inset-0 size-full cursor-pointer rounded-md bg-transparent focus-visible:outline-2 focus-visible:outline-offset-2"
                    value={String(value)} tabIndex={0}
                    onChange={(event: any) => set(Number(typeof event === 'string' ? event : event.target.value))}
                    onKeyDown={(event: KeyboardEvent) => {
                        if (event.altKey || event.ctrlKey || event.metaKey) return;
                        let next: number;
                        switch (event.key) {
                            case 'ArrowLeft':
                            case 'ArrowDown': next = value - step; break;
                            case 'ArrowRight':
                            case 'ArrowUp': next = value + step; break;
                            case 'Home': next = min; break;
                            case 'End': next = max; break;
                            default: return;
                        }
                        event.preventDefault();
                        set(next);
                    }} />
            </div>
            <span aria-hidden="true" className="w-14 shrink-0 whitespace-nowrap text-right text-[12px] tabular-nums" style={{ color: withAlpha(scheme.fg, 0.6) }}>
                {shown}
            </span>
        </div>
    );
}

export interface FieldProps {
    label: string;
    hint?: string;
    scheme: Scheme;
    children?: Child;
    density?: 'card' | 'compact';
}

export function Field({ label, hint, scheme, children, density = 'compact' }: FieldProps) {
    const hintId = useId();
    const card = density === 'card';
    return (
        <div className={`flex flex-wrap items-center justify-between gap-y-2 ${card ? 'min-h-[52px] gap-x-6 px-4 py-2.5' : 'min-h-9 gap-x-3 py-1.5'}`}>
            <div className={card ? 'min-w-0 flex-[1_1_8rem]' : 'min-w-0 flex-[1_1_4rem]'}>
                <div className={card ? 'text-[13.5px] font-medium' : 'text-[13px]'} style={{ color: card ? withAlpha(scheme.fg, 0.92) : css(scheme.fg) }}>
                    {label}
                </div>
                {hint ? (
                    <div id={hintId} className={card ? 'mt-0.5 text-[12px] leading-[17px]' : 'mt-0.5 text-[11.5px] leading-snug'}
                        style={{ color: withAlpha(scheme.fg, 0.5) }}>
                        {hint}
                    </div>
                ) : null}
            </div>
            {children ? (
                <div className="ml-auto min-w-0 max-w-full">
                    <FieldContext.Provider value={{ label, description: hint ? hintId : undefined }}>{children}</FieldContext.Provider>
                </div>
            ) : null}
        </div>
    );
}

export function SettingsCard({ scheme, title, children }: { scheme: Scheme; title?: string; children: Child }) {
    const divider = `1px solid ${withAlpha(scheme.fg, 0.06)}`;
    return (
        <section className={title ? 'mb-7' : undefined}>
            {title ? (
                <h3 className="mb-2 px-1 text-[12.5px] font-medium" style={{ color: withAlpha(scheme.fg, 0.45) }}>{title}</h3>
            ) : null}
            <div className="settings-card overflow-hidden rounded-xl" style={{ boxShadow: `inset 0 0 0 1px ${withAlpha(scheme.fg, 0.05)}` }}>
                {Children.map(Children.toArray(children), (row, index) => (
                    <div style={{ borderTop: index > 0 ? divider : undefined }}>{row}</div>
                ))}
            </div>
        </section>
    );
}

export function Section(props: { title: string; scheme: Scheme; children: any }) {
    return (
        <div className="mb-5">
            <div className="mb-2 text-[12.5px] font-medium" style={{ color: withAlpha(props.scheme.fg, 0.42) }}>
                {props.title}
            </div>
            {props.children}
        </div>
    );
}

/** A shortcut as keycaps, a small rounded square for each key: `Ctrl+Shift+T` is three. */
export function Kbd(props: { children: string; scheme: Scheme }) {
    const keys = props.children.split('+').filter(Boolean);
    return (
        <div className="flex items-center gap-[3px]">
            {keys.map((key, i) => (
                <div
                    key={i}
                    className="h-[19px] min-w-[19px] rounded-[5px] px-[5px] text-center text-[10.5px] font-medium leading-[19px]"
                    style={{ background: withAlpha(props.scheme.fg, 0.1), color: withAlpha(props.scheme.fg, 0.62) }}
                >
                    {key}
                </div>
            ))}
        </div>
    );
}

/** `ctrl+shift+t` as it's written on a key: `Ctrl+Shift+T`. */
export function keyLabel(keys: string): string {
    return keys
        .split('+')
        .map((part) => (part.length === 1 ? part.toUpperCase() : part[0].toUpperCase() + part.slice(1)))
        .join('+')
        .replace('Pagedown', 'PgDn')
        .replace('Pageup', 'PgUp');
}

export function withShortcut(label: string, key?: string): string {
    return key ? `${label} (${key})` : label;
}

export interface IconButtonProps {
    icon: IconName;
    label: string;
    scheme: Scheme;
    size?: number;
    iconSize?: number;
    pressed?: boolean;
    disabled?: boolean;
    keepFocus?: boolean;
    noDrag?: boolean;
    onClick: () => void;
    onContextMenu?: () => void;
}

export function IconButton({ icon, label, scheme, size = 32, iconSize, pressed, disabled, keepFocus, noDrag, onClick, onContextMenu }: IconButtonProps) {
    const t = tokens(scheme);
    const [hover, setHover] = useState(false);
    return (
        <button type="button" aria-label={label} title={label} aria-pressed={pressed} disabled={disabled}
            className={`flex shrink-0 items-center justify-center transition-colors duration-150 focus-visible:outline-2 focus-visible:outline-offset-[-2px] ${noDrag ? '[app-region:no-drag]' : ''}`}
            style={{
                width: size, height: size, borderRadius: size < 30 ? 6 : 8,
                color: disabled ? t.faint : pressed || hover ? t.text : t.sub,
                background: disabled ? 'transparent' : pressed ? t.chosen : hover ? t.hover : 'transparent',
            }}
            onPointerEnter={() => setHover(true)} onPointerLeave={() => setHover(false)}
            onPointerDown={keepFocus ? (event) => event.preventDefault() : undefined}
            onClick={onClick} onContextMenu={onContextMenu}>
            <Icon name={icon} size={iconSize ?? (size < 28 ? 13 : size < 32 ? 15 : 16)} strokeWidth={size === 28 ? 1.5 : size === 32 ? 1.6 : undefined} />
        </button>
    );
}

export function SettingsButton(props: { scheme: Scheme; children: any; onClick: () => void; primary?: boolean; disabled?: boolean; label?: string }) {
    const { fg, bg } = props.scheme;
    return (
        <button
            type="button"
            aria-label={props.label}
            disabled={props.disabled}
            className="min-h-8 shrink-0 rounded-md px-2.5 text-[12px] font-medium focus-visible:outline-2 focus-visible:outline-offset-2 disabled:opacity-40"
            style={{ background: props.primary ? css(fg) : withAlpha(fg, 0.08), color: css(props.primary ? bg : fg) }}
            onClick={props.onClick}
        >
            {props.children}
        </button>
    );
}
