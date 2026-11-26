import { useState } from 'react';
import { dialogs } from 'rsui/desktop';
import { css, withAlpha } from '../themes';
import type { Scheme } from '../themes';

export function ColorEditor({ label, value, onChange, scheme, compact = false, disabled = false }: {
    label: string;
    value: number;
    onChange: (value: number) => void;
    scheme: Scheme;
    compact?: boolean;
    disabled?: boolean;
}) {
    const [hex, setHex] = useState(css(value));
    const digits = hex.trim().replace(/^#/, '');
    const color = /^[0-9a-f]{6}$/i.test(digits) ? Number.parseInt(digits, 16) : null;
    const apply = () => {
        if (!disabled && color !== null) {
            setHex(css(color));
            if (color !== value) onChange(color);
        }
    };
    return (
        <div className="flex flex-wrap items-center gap-2 text-[12px]">
            <span className={compact ? 'w-full' : 'min-w-[76px]'}>{label}</span>
            <button
                aria-label={`Pick ${label.toLowerCase()} color`}
                disabled={disabled}
                title="Open color picker"
                className="size-7 rounded-md"
                style={{ background: css(color ?? value), border: `1px solid ${withAlpha(scheme.fg, 0.26)}` }}
                onClick={async () => {
                    const picked = await dialogs.pickColor({ initial: css(color ?? value) });
                    if (picked) {
                        setHex(picked);
                        const next = Number.parseInt(picked.slice(1), 16);
                        if (next !== value) onChange(next);
                    }
                }}
            />
            <input
                aria-label={label + ' hex color'}
                value={hex}
                disabled={disabled}
                className="h-7 w-[84px] rounded-md px-2 font-mono"
                style={{ background: withAlpha(scheme.fg, 0.09), color: css(scheme.fg) }}
                onChange={(e: any) => setHex(typeof e === 'string' ? e : e.target.value)}
                onKeyDown={(e: any) => { if (e.key === 'Enter') apply(); }}
            />
            <button
                disabled={disabled || color === null || color === value}
                className="h-7 rounded-md px-2.5"
                style={{ background: withAlpha(scheme.fg, 0.09), opacity: disabled || color === null || color === value ? 0.5 : 1 }}
                onClick={apply}
            >
                Apply
            </button>
            {color === null ? <span style={{ color: css(scheme.ansi[1]) }}>Use 6 hex digits</span> : null}
        </div>
    );
}
