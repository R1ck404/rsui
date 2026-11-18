// The sidebar, which sits on the window's frame as Arc's does: pinned folders as favorites, then places and drives, and the tabs
// when they're down the side. What's open now is raised, as the open tab is. Everything in it can be dragged into another order,
// the sections too, by their headings, and the order is kept.

import { app } from 'rsui/desktop';
import { COMPUTER, formatSize, nameOf, type Drive } from './files';
import * as I from './icons';
import { moved, ordered, useReorder, type Reorder } from './reorder';
import type { Settings } from './settings';
import { UsageBar } from './ui';

const paths = app.getPaths();

export const PLACES: { name: string; path: string; icon: any }[] = [
    { name: 'Home', path: paths.home ?? '', icon: I.HomeIcon },
    { name: 'Desktop', path: paths.desktop ?? '', icon: I.DesktopIcon },
    { name: 'Documents', path: paths.documents ?? '', icon: I.DocumentsIcon },
    { name: 'Downloads', path: paths.downloads ?? '', icon: I.DownloadIcon },
    { name: 'Pictures', path: paths.pictures ?? '', icon: I.ImageIcon },
    { name: 'Music', path: paths.music ?? '', icon: I.AudioIcon },
    { name: 'Videos', path: paths.videos ?? '', icon: I.VideoIcon },
].filter((place) => place.path !== '');

export const HOME = paths.home ?? 'C:\\';

export function driveIcon(kind: string): any {
    return kind === 'removable' ? I.UsbIcon : kind === 'network' ? I.CloudIcon : kind === 'optical' ? I.DiskIcon : I.DriveIcon;
}

/** What makes something in the sidebar draggable into another place: see `useReorder`. */
function dragging(reorder: Reorder, id: string, index: number) {
    const lifted = reorder.lifted(id);
    return {
        ref: reorder.ref(id),
        onPointerDown: (event: any) => reorder.down(index, event),
        onPointerMove: reorder.move,
        onPointerUp: reorder.up,
        style: lifted ? { transform: `translate(${lifted.dx}px, ${lifted.dy}px)` } : undefined,
        lifted: lifted !== null,
    };
}

function Item(props: {
    name: string;
    icon: any;
    active: boolean;
    onOpen: () => void;
    onContext?: () => void;
    children?: any;
    stroke: number;
    rail: boolean;
    drag: ReturnType<typeof dragging>;
    wasDrag: () => boolean;
}) {
    const { name, icon: Icon, active, onOpen, onContext, children, stroke, rail, drag, wasDrag } = props;
    return (
        <button
            ref={drag.ref}
            title={name}
            onClick={() => !wasDrag() && onOpen()}
            onPointerDown={drag.onPointerDown}
            onPointerMove={drag.onPointerMove}
            onPointerUp={drag.onPointerUp}
            onContextMenu={(event: any) => {
                if (onContext) {
                    event.preventDefault();
                    onContext();
                }
            }}
            style={drag.style}
            className={
                'relative flex w-full flex-col rounded-(--fx-radius-lg) px-2.5 py-[7px] text-left ' +
                (drag.lifted ? 'z-20 bg-(--fx-tab) text-(--fx-text) shadow-(--fx-pop-shadow) ' : 'transition-colors duration-(--fx-speed) ') +
                (active ? 'bg-(--fx-tab) text-(--fx-text) shadow-(--fx-tab-shadow)' : 'text-(--fx-muted) hover:bg-(--fx-tab-hover) hover:text-(--fx-text)')
            }
        >
            <span className="flex w-full items-center gap-2.5">
                <span className={'flex shrink-0 ' + (active ? 'text-(--fx-accent)' : '')}><Icon size={16} strokeWidth={stroke} /></span>
                <span className={'truncate whitespace-nowrap transition-opacity duration-(--fx-speed) ' + (rail ? 'opacity-0' : '')}>{name}</span>
            </span>
            {children ? (
                // What's under the name folds away with it, so a collapsed sidebar is only icons.
                <span
                    className={
                        'flex w-full overflow-hidden transition-[max-height,opacity,padding] duration-(--fx-move) ' +
                        (rail ? 'max-h-0 pt-0 opacity-0' : 'max-h-[40px] pt-1 opacity-100')
                    }
                >
                    {children}
                </span>
            ) : null}
        </button>
    );
}

/** A pinned folder, as a tile in a grid of them, as Arc's favorites are. */
function Favorite(props: {
    path: string;
    active: boolean;
    stroke: number;
    rail: boolean;
    drag: ReturnType<typeof dragging>;
    wasDrag: () => boolean;
    onOpen: (path: string) => void;
    onUnpin: (path: string) => void;
}) {
    const { path, active, stroke, rail, drag, wasDrag, onOpen, onUnpin } = props;
    return (
        <button
            ref={drag.ref}
            title={path}
            onClick={() => !wasDrag() && onOpen(path)}
            onPointerDown={drag.onPointerDown}
            onPointerMove={drag.onPointerMove}
            onPointerUp={drag.onPointerUp}
            onContextMenu={(event: any) => {
                event.preventDefault();
                onUnpin(path);
            }}
            style={drag.style}
            className={
                'relative flex min-w-0 rounded-(--fx-radius-lg) ' +
                // Collapsed, the tiles are a column of icons, lined up with the rest.
                (rail ? 'h-[33px] w-full flex-row items-center px-2.5 ' : 'h-[54px] flex-col items-center justify-center gap-1 px-1.5 ') +
                (drag.lifted ? 'z-20 shadow-(--fx-pop-shadow) ' : 'transition-colors duration-(--fx-speed) ') +
                (active || drag.lifted
                    ? 'bg-(--fx-tab) text-(--fx-text) shadow-(--fx-tab-shadow)'
                    : 'bg-(--fx-tab-hover) text-(--fx-muted) hover:bg-(--fx-tab) hover:text-(--fx-text)')
            }
        >
            <span className={'flex ' + (active ? 'text-(--fx-accent)' : '')}><I.FolderIcon size={17} strokeWidth={stroke} /></span>
            {rail ? null : <span className="w-full truncate text-center text-[0.8em]">{nameOf(path)}</span>}
        </button>
    );
}

/** A section's heading, which is what the section's dragged by. */
function Heading(props: { children: string; rail: boolean; drag: ReturnType<typeof dragging> }) {
    const { drag, rail } = props;
    return (
        <div
            onPointerDown={drag.onPointerDown}
            onPointerMove={drag.onPointerMove}
            onPointerUp={drag.onPointerUp}
            title={rail ? props.children : undefined}
            className="relative overflow-hidden px-2.5 pt-4 pb-1.5 text-[0.8em] font-medium whitespace-nowrap text-(--fx-faint) hover:text-(--fx-muted)"
        >
            <span className={'transition-opacity duration-(--fx-speed) ' + (rail ? 'opacity-0' : '')}>{props.children}</span>
            {/* Collapsed, a heading's a short line between the sections, where its name was. */}
            <span
                className={
                    'absolute top-[23px] left-[8px] h-px w-[16px] rounded-full bg-current transition-opacity duration-(--fx-speed) ' + (rail ? 'opacity-40' : 'opacity-0')
                }
            />
        </div>
    );
}

const SECTIONS = ['favorites', 'places', 'drives'];

export interface SidebarOrder {
    sections?: string[];
    places?: string[];
    drives?: string[];
}

export function Sidebar(props: {
    settings: Settings;
    folder: string;
    pinned: string[];
    drives: Drive[];
    width: number;
    /** Collapsed to a rail of icons. */
    rail: boolean;
    /** Whether its width moves to a new one, rather than going there at once, as it does while it's dragged. */
    animate: boolean;
    order: SidebarOrder;
    tabs?: any;
    onOpen: (path: string) => void;
    onUnpin: (path: string) => void;
    onPinned: (pinned: string[]) => void;
    onOrder: (order: SidebarOrder) => void;
}) {
    const { settings, folder, pinned, drives, width, rail, animate, order, tabs, onOpen, onUnpin, onPinned, onOrder } = props;
    const here = folder.toLowerCase();
    const stroke = settings.iconStroke;

    // What's in each, in the order it's kept in.
    const places = ordered(PLACES, order.places, (place) => place.path);
    const driveItems = ordered(
        [{ id: COMPUTER, drive: null as Drive | null }, ...drives.map((drive) => ({ id: drive.path, drive }))],
        order.drives,
        (item) => item.id,
    );
    const shown = {
        favorites: settings.showPinned && pinned.length > 0,
        places: settings.showQuickAccess,
        drives: settings.showDrives,
    } as Record<string, boolean>;
    const sections = ordered(SECTIONS, order.sections, (id) => id).filter((id) => shown[id]);

    const placeIds = places.map((place) => place.path);
    const driveIds = driveItems.map((item) => item.id);
    const reorder = {
        sections: useReorder(sections, 'y', (from, to) => {
            const hidden = SECTIONS.filter((id) => !sections.includes(id));
            onOrder({ ...order, sections: [...moved(sections, from, to), ...hidden] });
        }),
        favorites: useReorder(pinned, 'both', (from, to) => onPinned(moved(pinned, from, to))),
        places: useReorder(placeIds, 'y', (from, to) => onOrder({ ...order, places: moved(placeIds, from, to) })),
        drives: useReorder(driveIds, 'y', (from, to) => onOrder({ ...order, drives: moved(driveIds, from, to) })),
    };

    const content = (id: string) => {
        switch (id) {
            case 'favorites':
                return (
                    <div className={rail ? 'flex flex-col' : 'grid grid-cols-3 gap-1.5'}>
                        {pinned.map((path, index) => (
                            <Favorite
                                key={path}
                                path={path}
                                active={here === path.toLowerCase()}
                                stroke={stroke}
                                rail={rail}
                                drag={dragging(reorder.favorites, path, index)}
                                wasDrag={reorder.favorites.wasDrag}
                                onOpen={onOpen}
                                onUnpin={onUnpin}
                            />
                        ))}
                    </div>
                );
            case 'places':
                return places.map((place, index) => (
                    <Item
                        key={place.path}
                        name={place.name}
                        icon={place.icon}
                        stroke={stroke}
                                rail={rail}
                        active={here === place.path.toLowerCase()}
                        drag={dragging(reorder.places, place.path, index)}
                        wasDrag={reorder.places.wasDrag}
                        onOpen={() => onOpen(place.path)}
                    />
                ));
            default:
                return driveItems.map(({ id: key, drive }, index) => {
                    const drag = dragging(reorder.drives, key, index);
                    if (drive === null) {
                        return (
                            <Item
                                key={key}
                                name="This PC"
                                icon={I.ComputerIcon}
                                stroke={stroke}
                                rail={rail}
                                active={folder === COMPUTER}
                                drag={drag}
                                wasDrag={reorder.drives.wasDrag}
                                onOpen={() => onOpen(COMPUTER)}
                            />
                        );
                    }
                    const used = drive.total > 0 ? 1 - drive.free / drive.total : 0;
                    return (
                        <Item
                            key={key}
                            name={(drive.label || 'Local Disk') + ' (' + drive.path.slice(0, 2) + ')'}
                            icon={driveIcon(drive.kind)}
                            stroke={stroke}
                                rail={rail}
                            active={here === drive.path.toLowerCase()}
                            drag={drag}
                            wasDrag={reorder.drives.wasDrag}
                            onOpen={() => onOpen(drive.path)}
                        >
                            {settings.showDriveUsage && drive.total > 0 ? (
                                <div className="flex w-full flex-col gap-1 pl-[26px]">
                                    <UsageBar used={used} accent={settings.accent} height={3} />
                                    <span className="text-[0.78em] text-(--fx-faint)">
                                        {formatSize(drive.free, settings.sizeUnits)} free of {formatSize(drive.total, settings.sizeUnits)}
                                    </span>
                                </div>
                            ) : null}
                        </Item>
                    );
                });
        }
    };
    const titles: Record<string, string> = { favorites: 'Favorites', places: 'Places', drives: 'Drives' };

    return (
        <nav
            className={
                'flex shrink-0 flex-col overflow-x-hidden overflow-y-auto pb-3 pl-2 [scrollbar-width:none] ' +
                (animate ? 'transition-[width,padding] duration-(--fx-move) ease-(--fx-ease)' : '')
            }
            // Hidden, it's no width at all, its padding too.
            style={width > 0 ? { width } : { width: 0, paddingLeft: 0 }}
        >
            {tabs ? <div className="pt-1 pb-1">{tabs}</div> : null}
            {sections.map((id, index) => {
                const drag = dragging(reorder.sections, id, index);
                return (
                    <div
                        key={id}
                        ref={drag.ref}
                        style={drag.style}
                        className={'relative flex flex-col ' + (drag.lifted ? 'z-20 rounded-(--fx-radius-lg) bg-(--fx-frame) shadow-(--fx-pop-shadow)' : '')}
                    >
                        <Heading drag={drag} rail={rail}>{titles[id]}</Heading>
                        {content(id)}
                    </div>
                );
            })}
        </nav>
    );
}
