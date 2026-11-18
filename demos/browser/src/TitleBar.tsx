// The strip along the top of the window, which moves it: over the sidebar, its button and back, forward and reload; the page's
// title in the middle while the sidebar's hidden; and the window's buttons in the corner.
//
// The part over the sidebar opens and closes with it, and is never narrower than its buttons, which then sit together by the
// window's edge.

import * as I from './icons';
import { IconButton } from './ui/IconButton';
import { WindowButtons } from './ui/WindowButtons';

export function TitleBar(props: {
    /** How wide the sidebar is, which the part over it is while it's shown. */
    sidebarWidth: number;
    /** Whether the sidebar's in its place, which it isn't while it's hidden or the tour's showing. */
    sidebarShown: boolean;
    sidebarHidden: boolean;
    /** Whether the sidebar's being dragged wider or narrower, which the part over it follows at once. */
    dragging: boolean;
    /** Whether the sidebar's on the right, which puts everything the other way round. */
    right: boolean;
    /** The page's title, shown while the sidebar isn't. */
    title: string;
    /** Whether there's a page to go back and forward in, which the tour hasn't. */
    navigation: boolean;
    canGoBack: boolean;
    canGoForward: boolean;
    loading: boolean;
    canReload: boolean;
    onSidebar: () => void;
    onBack: () => void;
    onForward: () => void;
    onReload: () => void;
    onStop: () => void;
}) {
    const { right, sidebarShown, sidebarHidden } = props;
    return (
        <div className={'flex h-10 shrink-0 items-center [app-region:drag] ' + (right ? 'flex-row-reverse' : '')}>
            <div
                className={
                    'flex h-full min-w-max shrink-0 items-center gap-0.5 px-2 ' +
                    (right ? 'flex-row-reverse ' : '') +
                    (props.dragging ? '' : 'transition-[width] duration-(--b-move) ease-(--b-ease)')
                }
                style={{ width: sidebarShown ? props.sidebarWidth : 0 }}
            >
                <IconButton icon={I.SidebarIcon} label={(sidebarHidden ? 'Show' : 'Hide') + ' sidebar (Ctrl+S)'} onClick={props.onSidebar} />
                <div className="flex-1" />
                {props.navigation ? (
                    <>
                        <IconButton icon={I.BackIcon} label="Back (Alt+←)" disabled={!props.canGoBack} onClick={props.onBack} />
                        <IconButton icon={I.ForwardIcon} label="Forward (Alt+→)" disabled={!props.canGoForward} onClick={props.onForward} />
                        {props.loading ? (
                            <IconButton icon={I.CloseIcon} label="Stop" onClick={props.onStop} />
                        ) : (
                            <IconButton icon={I.ReloadIcon} label="Reload (Ctrl+R)" disabled={!props.canReload} onClick={props.onReload} />
                        )}
                    </>
                ) : null}
            </div>
            <div
                className={
                    'min-w-0 flex-1 truncate px-3 text-center text-[12px] text-(--b-faint) transition-opacity duration-(--b-move) ease-(--b-ease) ' +
                    (sidebarShown ? 'opacity-0' : '')
                }
            >
                {props.title}
            </div>
            <div className={'h-full shrink-0 ' + (right ? 'order-first' : '')}>
                <WindowButtons />
            </div>
        </div>
    );
}
