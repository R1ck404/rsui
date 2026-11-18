// The window's own minimize, maximize and close buttons, as it has no title bar of the system's.

import { mainWindow, useWindowState } from 'rsui';
import * as I from '../icons';

const BUTTON = 'flex h-full w-11 items-center justify-center text-(--b-muted) transition-colors duration-(--b-speed) ';

export function WindowButtons() {
    const { maximized } = useWindowState();
    return (
        <div className="flex h-full shrink-0 items-stretch">
            <button title="Minimize" aria-label="Minimize" className={BUTTON + 'hover:bg-(--b-hover) hover:text-(--b-text)'} onClick={() => mainWindow.minimize()}>
                <span className="h-px w-2.5 bg-current" />
            </button>
            <button
                title={maximized ? 'Restore' : 'Maximize'}
                aria-label={maximized ? 'Restore' : 'Maximize'}
                className={BUTTON + 'hover:bg-(--b-hover) hover:text-(--b-text)'}
                onClick={() => (maximized ? mainWindow.restore() : mainWindow.maximize())}
            >
                {maximized ? (
                    <span className="relative size-2.5">
                        <span className="absolute top-0 right-0 size-2 rounded-[2px] border border-current" />
                        <span className="absolute bottom-0 left-0 size-2 rounded-[2px] border border-current bg-(--b-frame)" />
                    </span>
                ) : (
                    <span className="size-2.5 rounded-[2px] border border-current" />
                )}
            </button>
            <button title="Close" aria-label="Close" className={BUTTON + 'hover:bg-[#e81123] hover:text-white'} onClick={() => mainWindow.close()}>
                <I.CloseIcon size={13} strokeWidth={1.6} />
            </button>
        </div>
    );
}
