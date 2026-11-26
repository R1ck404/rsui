// The strip along the top, which moves the window: the name, a button for a new terminal with the shells it can open, the
// overview, search, the palette and settings, and the window's own buttons.

import { mainWindow, useWindowState } from 'rsui';
import { Icon } from './Icon';
import type { IconName } from './Icon';
import { css, withAlpha } from '../themes';
import type { Scheme } from '../themes';

function Button(props: { icon: IconName; label: string; scheme: Scheme; on?: boolean; onClick: () => void }) {
    return (
        <button
            aria-label={props.label}
            title={props.label}
            className="flex size-8 items-center justify-center rounded-lg transition-colors duration-150 hover:bg-white/10"
            style={{ color: withAlpha(props.scheme.fg, props.on ? 1 : 0.62), background: props.on ? withAlpha(props.scheme.fg, 0.12) : undefined }}
            onPointerDown={(e: any) => e.preventDefault()}
            onClick={props.onClick}
        >
            <Icon name={props.icon} size={16} />
        </button>
    );
}

export function TitleBar(props: {
    scheme: Scheme;
    overview: boolean;
    onNew: () => void;
    onProfiles: () => void;
    onOverview: () => void;
    onSearch: () => void;
    onPalette: () => void;
    onSettings: () => void;
    onQuake: () => void;
    title: string;
}) {
    const { scheme } = props;
    const { maximized } = useWindowState();
    return (
        <div className="flex h-11 shrink-0 items-center gap-1 pl-4 [app-region:drag]">
            <div className="mr-3 flex items-center gap-2.5" style={{ color: css(scheme.fg) }}>
                <span className="flex size-[22px] items-center justify-center rounded-md" style={{ background: css(scheme.ansi[4]), color: css(scheme.bg) }}>
                    <Icon name="terminal" size={14} strokeWidth={2.2} />
                </span>
                <span className="text-[13px] font-semibold tracking-tight">Terminal</span>
            </div>
            <div className="flex items-center [app-region:no-drag]">
                <Button icon="plus" label="New terminal (Ctrl+Shift+T)" scheme={scheme} onClick={props.onNew} />
                <button
                    aria-label="Choose a shell"
                    title="Choose a shell"
                    className="-ml-1 flex h-8 w-5 items-center justify-center rounded-lg hover:bg-white/10"
                    style={{ color: withAlpha(scheme.fg, 0.55) }}
                    onPointerDown={(e: any) => e.preventDefault()}
                    onClick={props.onProfiles}
                >
                    <Icon name="chevron" size={12} />
                </button>
                <Button icon="grid" label="Overview (Ctrl+Shift+O)" scheme={scheme} on={props.overview} onClick={props.onOverview} />
            </div>
            <div className="min-w-0 flex-1 truncate px-4 text-center text-[12px]" style={{ color: withAlpha(scheme.fg, 0.45) }}>
                {props.title}
            </div>
            <div className="flex items-center [app-region:no-drag]">
                <Button icon="search" label="Find (Ctrl+Shift+F)" scheme={scheme} onClick={props.onSearch} />
                <Button icon="command" label="Command palette (Ctrl+Shift+P)" scheme={scheme} onClick={props.onPalette} />
                <Button icon="quake" label="Dropdown terminal" scheme={scheme} onClick={props.onQuake} />
                <Button icon="settings" label="Settings (Ctrl+,)" scheme={scheme} onClick={props.onSettings} />
            </div>
            <div className="ml-1 flex h-full shrink-0 items-stretch [app-region:no-drag]">
                <button
                    aria-label="Minimize"
                    className="flex w-11 items-center justify-center hover:bg-white/10"
                    style={{ color: withAlpha(scheme.fg, 0.7) }}
                    onClick={() => mainWindow.minimize()}
                >
                    <span className="h-px w-2.5 bg-current" />
                </button>
                <button
                    aria-label={maximized ? 'Restore' : 'Maximize'}
                    className="flex w-11 items-center justify-center hover:bg-white/10"
                    style={{ color: withAlpha(scheme.fg, 0.7) }}
                    onClick={() => (maximized ? mainWindow.restore() : mainWindow.maximize())}
                >
                    <span className="size-2.5 rounded-[2px] border border-current" />
                </button>
                <button
                    aria-label="Close"
                    className="flex w-11 items-center justify-center hover:bg-[#e81123] hover:text-white"
                    style={{ color: withAlpha(scheme.fg, 0.7) }}
                    onClick={() => mainWindow.close()}
                >
                    <Icon name="close" size={14} />
                </button>
            </div>
        </div>
    );
}
