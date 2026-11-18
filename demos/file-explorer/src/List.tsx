// The folder's contents as details, a list or a grid.
//
// This is the one part of the app drawn with rsui's own elements rather than the page's: `<list>` only builds the rows in view and
// scrolls them in Rust, and a row made of `<row>` and `<text>` costs a fraction of one made of styled `<div>`s, which go through the
// page's CSS. Everything around it is Tailwind. The colors come from the same settings, so the two look the same.

import { useEffect, useRef } from 'react';
import { attributesOf, displayName, formatDate, formatSize, kindLabel, type Entry } from './files';
import { FileIcon } from './FileIcon';
import { ChevronDownIcon } from './icons';
import type { Settings, SortKey, View } from './settings';
import type { Colors } from './theme';
import { HAS_THUMBNAIL, useThumbnail } from './thumbs';

/** What every row is drawn with, from the settings. */
export interface Look {
    rowHeight: number;
    iconSize: number;
    stroke: number;
    iconStyle: Settings['iconStyle'];
    tintFolders: boolean;
    colorFiles: boolean;
    sharedIcons: boolean;
    showExtensions: boolean;
    dimHidden: boolean;
    zebra: boolean;
    rowLines: boolean;
    selection: Settings['selection'];
    dateFormat: Settings['dateFormat'];
    clock24: boolean;
    sizeUnits: Settings['sizeUnits'];
    cols: { size: boolean; type: boolean; modified: boolean; created: boolean; attributes: boolean };
    tile: { width: number; height: number; icon: number };
    overscan: number;
    renameSelectsBase: boolean;
    colors: Colors;
    radius: number;
    font: string;
    fontSize: number;
    hoverEffects: boolean;
    gridThumbnails: boolean;
    thumbnailCache: number;
}

export interface Handlers {
    onPick(path: string, mods: { ctrl: boolean; shift: boolean }): void;
    onActivate(path: string, mods: { ctrl: boolean; shift: boolean }): void;
    onOpen(path: string): void;
    onContext(path: string | null): void;
    onDragOut(path: string): void;
    onRename(path: string, name: string | null): void;
    onEmptyClick(): void;
    onEmptyDouble(): void;
}

export const COLUMN_WIDTH = { size: 92, type: 120, modified: 168, created: 168, attributes: 56 };

/** The room around the tiles of a grid, and between them. */
const GRID_PAD = 10;
const GRID_GAP = 8;

/** How a row looks for whether it's selected or has the keyboard. */
function paint(look: Look, selected: boolean, cursor: boolean, odd: boolean) {
    const c = look.colors;
    if (selected) {
        if (look.selection === 'filled') {
            return { background: c.select, color: c.text, muted: c.muted, border: undefined };
        }
        return { background: c['accent-soft'], color: c.text, muted: c.muted, border: look.selection === 'outline' ? c.accent : undefined };
    }
    return {
        background: look.zebra && odd ? c.zebra : undefined,
        color: c.text,
        muted: c.muted,
        border: cursor ? c.line : undefined,
    };
}

function RenameField(props: { entry: Entry; look: Look; handlers: Handlers }) {
    const { entry, look, handlers } = props;
    let done = false;
    const finish = (name: string | null) => {
        if (!done) {
            done = true;
            handlers.onRename(entry.path, name);
        }
    };
    // Focused once, as it shows, with the name selected.
    const field = useRef<any>(null);
    useEffect(() => {
        const element = field.current;
        if (element) {
            element.focus();
            const end = look.renameSelectsBase && !entry.folder && entry.ext ? entry.name.length - entry.ext.length - 1 : entry.name.length;
            if (typeof element.setSelectionRange === 'function') {
                element.setSelectionRange(0, end);
            } else {
                element.select();
            }
        }
    }, []);
    return (
        <input
            ref={field}
            type="text"
            defaultValue={entry.name}
            className="min-w-0 flex-1 rounded-[4px] border border-(--fx-accent) bg-(--fx-panel) px-1 text-(--fx-text) outline-none"
            onKeyDown={(event: any) => {
                event.stopPropagation();
                if (event.key === 'Enter') {
                    finish(event.currentTarget.value);
                } else if (event.key === 'Escape') {
                    finish(null);
                }
            }}
            onBlur={(event: any) => finish(event.currentTarget.value)}
        />
    );
}

// How far the pointer has gone with the button down since it went down on a row, and whether files are being dragged out.
// rsui says `start` as soon as the button goes down, and dragging files out takes the pointer over until it's let go, so a
// click, which always moves a pixel or two in a real hand, would never be a click. Windows waits for 4 pixels; this waits for 6.
const DRAG_AFTER = 6;
let pressed = { path: '', moved: 0, dragging: false };

/** What a row does with the pointer, which is the same in every view. */
function pointer(entry: Entry, handlers: Handlers) {
    return {
        onClick: (event: any) => handlers.onActivate(entry.path, { ctrl: !!event?.ctrlKey, shift: !!event?.shiftKey }),
        onDoubleClick: () => handlers.onOpen(entry.path),
        onContextMenu: () => handlers.onContext(entry.path),
        onDrag: (phase: string, _x: number, _y: number, dx: number, dy: number) => {
            if (phase === 'start') {
                pressed = { path: entry.path, moved: 0, dragging: false };
            } else if (phase === 'move' && pressed.path === entry.path && !pressed.dragging) {
                pressed.moved += Math.abs(dx) + Math.abs(dy);
                if (pressed.moved >= DRAG_AFTER) {
                    pressed.dragging = true;
                    handlers.onDragOut(entry.path);
                }
            } else if (phase === 'end' || phase === 'cancel') {
                pressed = { path: '', moved: 0, dragging: false };
            }
        },
    };
}

function Icon(props: { entry: Entry; look: Look; size: number; inverted: boolean }) {
    const { entry, look, size, inverted } = props;
    return (
        <FileIcon
            entry={entry}
            size={size}
            stroke={size > 30 ? Math.max(1, look.stroke * (24 / size) * 1.6) : look.stroke}
            style={look.iconStyle}
            tintFolders={look.tintFolders}
            colorFiles={look.colorFiles}
            sharedSystemIcons={look.sharedIcons}
            colors={look.colors}
            inverted={inverted}
        />
    );
}

function DetailsRow(props: {
    entry: Entry;
    selected: boolean;
    cursor: boolean;
    renaming: boolean;
    odd: boolean;
    look: Look;
    now: number;
    handlers: Handlers;
    details: boolean;
}) {
    const { entry, selected, cursor, renaming, odd, look, now, handlers, details } = props;
    const p = paint(look, selected, cursor, odd);
    const cell = (width: number, text: string, right = false, mono = false) => (
        <text
            width={width}
            shrink={0}
            color={p.muted}
            fontSize={look.fontSize * 0.92}
            ellipsis
            noWrap
            textAlign={right ? 'right' : 'left'}
            fontFamily={mono ? 'Consolas' : undefined}
        >
            {text}
        </text>
    );
    const body = (
        <row
            height={look.rowHeight - (look.rowLines ? 1 : 0)}
            paddingX={10}
            gap={12}
            align="center"
            radius={look.rowLines ? 0 : look.radius}
            background={p.background}
            hover={selected || !look.hoverEffects ? undefined : look.colors.hover}
            borderColor={p.border}
            borderWidth={p.border ? 1 : 0}
            color={p.color}
            opacity={look.dimHidden && (entry.hidden || entry.system) ? 0.55 : 1}
            {...pointer(entry, handlers)}
        >
            <row grow={1} shrink={1} minWidth={0} gap={10} align="center">
                <Icon entry={entry} look={look} size={look.iconSize} inverted={false} />
                {renaming ? (
                    <RenameField entry={entry} look={look} handlers={handlers} />
                ) : (
                    <text grow={1} shrink={1} ellipsis noWrap fontSize={look.fontSize} color={p.color}>
                        {displayName(entry, look.showExtensions)}
                    </text>
                )}
                {entry.within ? (
                    <text shrink={1} ellipsis noWrap fontSize={look.fontSize * 0.88} color={p.muted}>
                        {entry.within}
                    </text>
                ) : null}
            </row>
            {details && look.cols.modified ? cell(COLUMN_WIDTH.modified, formatDate(entry.modified, look.dateFormat, look.clock24, now)) : null}
            {details && look.cols.created ? cell(COLUMN_WIDTH.created, formatDate(entry.created, look.dateFormat, look.clock24, now)) : null}
            {details && look.cols.type ? cell(COLUMN_WIDTH.type, kindLabel(entry)) : null}
            {details && look.cols.size ? cell(COLUMN_WIDTH.size, entry.folder ? '' : formatSize(entry.size, look.sizeUnits), true) : null}
            {details && look.cols.attributes ? cell(COLUMN_WIDTH.attributes, attributesOf(entry), false, true) : null}
        </row>
    );
    if (!look.rowLines) {
        return body;
    }
    return (
        <column>
            {body}
            <group height={1} background={look.colors.line} />
        </column>
    );
}

function Tile(props: { entry: Entry; selected: boolean; cursor: boolean; renaming: boolean; look: Look; handlers: Handlers }) {
    const { entry, selected, cursor, renaming, look, handlers } = props;
    const p = paint(look, selected, cursor, false);
    const picture = useThumbnail(entry, look.tile.icon <= 48 ? 96 : 256, look.gridThumbnails && HAS_THUMBNAIL.has(entry.kind), look.thumbnailCache, 30);
    return (
        <column
            width={look.tile.width}
            height={look.tile.height}
            shrink={0}
            paddingX={6}
            paddingTop={10}
            gap={6}
            align="center"
            radius={look.radius * 1.6}
            background={p.background}
            hover={selected || !look.hoverEffects ? undefined : look.colors.hover}
            borderColor={p.border}
            borderWidth={p.border ? 1 : 0}
            opacity={look.dimHidden && (entry.hidden || entry.system) ? 0.55 : 1}
            {...pointer(entry, handlers)}
        >
            {picture ? (
                <image src={picture} width={look.tile.icon + 16} height={look.tile.icon} fit="contain" radius={4} />
            ) : (
                <Icon entry={entry} look={look} size={look.tile.icon} inverted={false} />
            )}
            {renaming ? (
                <RenameField entry={entry} look={look} handlers={handlers} />
            ) : (
                // Names that don't fit go on to a second line, as Explorer's do, and only then end in "…".
                <text width={look.tile.width - 12} textAlign="center" fontSize={look.fontSize * 0.92} color={p.color} maxLines={2}>
                    {displayName(entry, look.showExtensions)}
                </text>
            )}
        </column>
    );
}

function SortHeader(props: {
    label: string;
    sortKey: SortKey;
    current: SortKey;
    descending: boolean;
    width?: number;
    align?: string;
    onSort: (key: SortKey) => void;
}) {
    const { label, sortKey, current, descending, width, align, onSort } = props;
    const active = sortKey === current;
    return (
        <button
            className={
                'flex shrink-0 items-center gap-1 text-[0.85em] font-medium whitespace-nowrap hover:text-(--fx-text) ' +
                (active ? 'text-(--fx-text) ' : 'text-(--fx-muted) ') +
                (align === 'right' ? 'justify-end ' : '') +
                (width ? '' : 'min-w-0 flex-1')
            }
            style={width ? { width } : undefined}
            onClick={() => onSort(sortKey)}
        >
            {label}
            {active ? (
                <span className={'flex ' + (descending ? '' : 'rotate-180')}>
                    <ChevronDownIcon size={12} strokeWidth={2} />
                </span>
            ) : null}
        </button>
    );
}

export function ColumnHeaders(props: { look: Look; sortKey: SortKey; descending: boolean; onSort: (key: SortKey) => void }) {
    const { look, sortKey, descending, onSort } = props;
    return (
        <div className="flex h-[30px] shrink-0 items-center gap-3 border-b border-(--fx-line) pr-[26px] pl-[18px]">
            <div className="flex min-w-0 flex-1" style={{ paddingLeft: look.iconSize + 10 }}>
                <SortHeader label="Name" sortKey="name" current={sortKey} descending={descending} onSort={onSort} />
            </div>
            {look.cols.modified ? <SortHeader label="Modified" sortKey="modified" current={sortKey} descending={descending} width={COLUMN_WIDTH.modified} onSort={onSort} /> : null}
            {look.cols.created ? <SortHeader label="Created" sortKey="created" current={sortKey} descending={descending} width={COLUMN_WIDTH.created} onSort={onSort} /> : null}
            {look.cols.type ? <SortHeader label="Type" sortKey="type" current={sortKey} descending={descending} width={COLUMN_WIDTH.type} onSort={onSort} /> : null}
            {look.cols.size ? <SortHeader label="Size" sortKey="size" current={sortKey} descending={descending} width={COLUMN_WIDTH.size} align="right" onSort={onSort} /> : null}
            {look.cols.attributes ? (
                <span className="shrink-0 text-[0.85em] font-medium text-(--fx-muted)" style={{ width: COLUMN_WIDTH.attributes }}>
                    Attr
                </span>
            ) : null}
        </div>
    );
}

export function FileList(props: {
    items: Entry[];
    view: View;
    look: Look;
    selected: Set<string>;
    cursor: string | null;
    renaming: string | null;
    reveal: { path: string | null; at: number };
    now: number;
    empty: string;
    handlers: Handlers;
    folder: string;
    /** Told how many rows fit, and how many tiles across, as layout has them, for the keyboard. */
    onShape: (rows: number, columns: number) => void;
}) {
    const { items, view, look, selected, cursor, renaming, reveal, now, empty, handlers, folder, onShape } = props;
    const grid = view === 'grid';
    // A grid's rows are as far apart as a tile is tall and the gap under it, and it puts as many tiles across as fit itself.
    const stride = grid ? look.tile.height + GRID_GAP : look.rowHeight + 1;

    // Draws one row; rows whose props didn't change aren't drawn again.
    const renderRow = (row: number) => {
        if (grid) {
            const entry = items[row];
            return (
                <Tile
                    key={entry.path}
                    entry={entry}
                    selected={selected.has(entry.path)}
                    cursor={cursor === entry.path}
                    renaming={renaming === entry.path}
                    look={look}
                    handlers={handlers}
                />
            );
        }
        const entry = items[row];
        return (
            <group key={entry.path} paddingX={8} height={stride} paddingBottom={1}>
                <DetailsRow
                    entry={entry}
                    selected={selected.has(entry.path)}
                    cursor={cursor === entry.path}
                    renaming={renaming === entry.path}
                    odd={row % 2 === 1}
                    look={look}
                    now={now}
                    handlers={handlers}
                    details={view === 'details'}
                />
            </group>
        );
    };

    if (items.length === 0) {
        return (
            <div
                className="flex min-h-0 flex-1 items-center justify-center text-(--fx-faint)"
                onClick={() => handlers.onEmptyClick()}
                onContextMenu={(event: any) => {
                    event.preventDefault();
                    handlers.onContext(null);
                }}
            >
                {empty}
            </div>
        );
    }
    const index = reveal.path === null ? -1 : items.findIndex((item) => item.path === reveal.path);
    // The list fills what's around it exactly, which is how big layout makes it, and works out how much of it shows from that.
    return (
        <div className="relative min-h-0 flex-1">
            <list
            key={folder + ':' + view}
            count={items.length}
            rowHeight={stride}
            itemWidth={grid ? look.tile.width : undefined}
            gap={grid ? GRID_GAP : undefined}
            paddingX={grid ? GRID_PAD : undefined}
            overscan={look.overscan}
            reveal={index < 0 ? undefined : index}
            renderRow={renderRow}
            position="absolute"
            top={0}
            left={0}
            right={0}
            bottom={0}
            paddingTop={6}
            onResize={onShape}
            onClick={() => handlers.onEmptyClick()}
            onDoubleClick={() => handlers.onEmptyDouble()}
            onContextMenu={() => handlers.onContext(null)}
            />
        </div>
    );
}
