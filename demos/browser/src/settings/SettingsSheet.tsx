// Settings, as a sheet over the page: its sections down the side, and each setting applied as it's changed. Escape or a click
// beside it closes it.

import { useEffect, useState } from 'react';
import * as I from '../icons';
import type { Browser } from '../useBrowser';
import { type IconType, IconButton } from '../ui/IconButton';
import { Appearance } from './Appearance';
import { General } from './General';
import { Privacy } from './Privacy';
import { Shortcuts } from './Shortcuts';
import { Spaces } from './Spaces';

type Section = 'general' | 'appearance' | 'spaces' | 'privacy' | 'shortcuts';

const SECTIONS: { id: Section; label: string; icon: IconType }[] = [
    { id: 'general', label: 'General', icon: I.SettingsIcon },
    { id: 'appearance', label: 'Appearance', icon: I.SparkleIcon },
    { id: 'spaces', label: 'Spaces', icon: I.SpacesIcon },
    { id: 'privacy', label: 'Privacy', icon: I.ShieldIcon },
    { id: 'shortcuts', label: 'Shortcuts', icon: I.KeyboardIcon },
];

export function SettingsSheet(props: { browser: Browser; onTour: () => void; onClose: () => void }) {
    const { browser } = props;
    const [section, setSection] = useState<Section>('general');
    useEffect(() => {
        const heard = (event: any) => {
            if (event.key === 'Escape') props.onClose();
        };
        document.addEventListener('keydown', heard);
        return () => document.removeEventListener('keydown', heard);
    });
    const label = SECTIONS.find((one) => one.id === section)?.label;
    return (
        <div className="absolute inset-0 z-40 flex items-center justify-center bg-(--b-scrim) p-6 animate-[fade-in_var(--b-speed)_ease-out]" onPointerDown={props.onClose}>
            <div
                role="dialog"
                aria-label="Settings"
                onPointerDown={(event: any) => event.stopPropagation()}
                className="flex h-full max-h-[560px] w-full max-w-[760px] overflow-hidden rounded-(--b-radius-lg) bg-(--b-raised) shadow-(--b-pop-shadow) animate-[rise_var(--b-move)_var(--b-ease)]"
            >
                <nav className="flex w-[188px] shrink-0 flex-col gap-0.5 bg-(--b-hover) p-2.5 pt-4">
                    <span className="px-2.5 pb-3 text-[15px] font-semibold tracking-[-0.01em] text-(--b-text)">Settings</span>
                    {SECTIONS.map((one) => (
                        <button
                            key={one.id}
                            aria-current={section === one.id ? 'page' : undefined}
                            onClick={() => setSection(one.id)}
                            className={
                                'flex h-8 items-center gap-2.5 rounded-(--b-radius-sm) px-2.5 text-left transition-colors duration-(--b-speed) ' +
                                (section === one.id ? 'bg-(--b-raised) text-(--b-text) shadow-(--b-tab-shadow)' : 'text-(--b-muted) hover:text-(--b-text)')
                            }
                        >
                            <one.icon size={15} strokeWidth={1.7} />
                            {one.label}
                        </button>
                    ))}
                </nav>
                <div className="flex min-w-0 flex-1 flex-col">
                    <div className="flex h-12 shrink-0 items-center justify-between pr-2.5 pl-6">
                        <span className="text-[14px] font-semibold text-(--b-text)">{label}</span>
                        <IconButton icon={I.CloseIcon} label="Close" onClick={props.onClose} />
                    </div>
                    <div key={section} className="flex min-h-0 flex-1 flex-col overflow-y-auto px-6 pb-6 animate-[fade-in_var(--b-speed)_var(--b-ease)]">
                        {section === 'general' ? (
                            <General settings={browser.settings} onSettings={browser.change} />
                        ) : section === 'appearance' ? (
                            <Appearance settings={browser.settings} onSettings={browser.change} />
                        ) : section === 'spaces' ? (
                            <Spaces spaces={browser.session.spaces} onChange={browser.changeSpace} onDelete={browser.removeSpace} />
                        ) : section === 'privacy' ? (
                            <Privacy
                                settings={browser.settings}
                                historyCount={browser.history.length}
                                onSettings={browser.change}
                                onClearHistory={browser.clearHistory}
                                onClearCookies={() => browser.activeView()?.clearCookies()}
                                onTour={props.onTour}
                            />
                        ) : (
                            <Shortcuts />
                        )}
                    </div>
                </div>
            </div>
        </div>
    );
}
