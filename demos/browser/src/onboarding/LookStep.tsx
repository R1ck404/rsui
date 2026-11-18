// The tour's look: light, dark or as Windows is, the space's color, and whether the desktop shows through the frame.

import * as I from '../icons';
import { type Settings, SPACE_COLORS, type Theme } from '../model';
import { Toggle } from '../ui/controls';
import { StepHeading } from './StepHeading';

const THEMES: { theme: Theme; label: string }[] = [
    { theme: 'light', label: 'Light' },
    { theme: 'dark', label: 'Dark' },
    { theme: 'system', label: 'Automatic' },
];

export function LookStep(props: { settings: Settings; color: string; onSettings: (changes: Partial<Settings>) => void; onColor: (color: string) => void }) {
    const { settings, color } = props;
    return (
        <>
            <StepHeading title="Make it yours" detail="Pick a look. You can change any of this later in Settings." />
            <div className="mt-8 grid w-full grid-cols-3 gap-3">
                {THEMES.map(({ theme, label }) => (
                    <button
                        key={theme}
                        aria-pressed={settings.theme === theme}
                        onClick={() => props.onSettings({ theme })}
                        className={
                            'flex flex-col items-stretch gap-2.5 rounded-(--b-radius-lg) p-2 transition-all duration-(--b-speed) ' +
                            (settings.theme === theme ? 'bg-(--b-accent-soft) ring-2 ring-(--b-accent)' : 'bg-(--b-hover) hover:bg-(--b-press)')
                        }
                    >
                        <ThemePreview theme={theme} color={color} />
                        <span className="pb-1 text-[13px] font-medium text-(--b-text)">{label}</span>
                    </button>
                ))}
            </div>
            <div className="mt-6 flex items-center gap-2.5">
                {SPACE_COLORS.map((one) => (
                    <button
                        key={one.color}
                        title={one.name}
                        aria-label={one.name}
                        aria-pressed={one.color === color}
                        onClick={() => props.onColor(one.color)}
                        className="flex size-8 items-center justify-center rounded-full text-white transition-transform duration-(--b-speed) hover:scale-110"
                        style={{ background: one.color, boxShadow: one.color === color ? `0 0 0 2px var(--b-card), 0 0 0 4px ${one.color}` : undefined }}
                    >
                        {one.color === color ? <I.CheckIcon size={14} strokeWidth={2.4} /> : null}
                    </button>
                ))}
            </div>
            <div className="mt-6 flex w-full max-w-[360px] items-center justify-between rounded-(--b-radius) bg-(--b-hover) px-4 py-3 text-left">
                <span className="flex flex-col">
                    <span className="text-[13px] font-medium text-(--b-text)">See the desktop through the frame</span>
                    <span className="text-[12px] text-(--b-muted)">Windows 11's Mica, softly tinted</span>
                </span>
                <Toggle label="Translucent frame" on={settings.material !== 'none'} onChange={(on) => props.onSettings({ material: on ? 'mica' : 'none' })} />
            </div>
        </>
    );
}

/** A small drawing of the window in a theme: the tinted frame, a sidebar of tabs, and the page as a card. Automatic is both. */
function ThemePreview(props: { theme: Theme; color: string }) {
    const { theme, color } = props;
    const halves = theme === 'system' ? [false, true] : [theme === 'dark'];
    return (
        <div className="flex h-[78px] overflow-hidden rounded-(--b-radius)">
            {halves.map((dark) => (
                <div
                    key={String(dark)}
                    className="flex h-full flex-1 gap-1 p-1.5"
                    style={{ background: `color-mix(in srgb, ${dark ? '#131316' : '#eceef2'} 80%, ${color})` }}
                >
                    <div className="flex w-[30%] flex-col gap-1 pt-1">
                        {[100, 80, 70, 85].map((width, index) => (
                            <span
                                key={index}
                                className={'h-1.5 rounded-full ' + (index === 1 ? (dark ? 'bg-white/20' : 'bg-white/90') : dark ? 'bg-white/15' : 'bg-black/8')}
                                style={{ width: width + '%' }}
                            />
                        ))}
                    </div>
                    <div className={'flex-1 rounded-[5px] ' + (dark ? 'bg-[#1b1b1f]' : 'bg-white')} />
                </div>
            ))}
        </div>
    );
}
