// Workspaces: sets of tabs of their own, one for each thing being worked on, switched between as a whole. Each has a name and
// a color, shown as a small square with its first letter, and can have a folder its new shells start in.

import { useEffect, useRef, useState } from 'react';
import type { Space, Workspace } from '../workspace';
import { SPACE_COLORS } from '../workspace';
import { css, mix, withAlpha, tokens } from '../themes';
import type { Scheme } from '../themes';
import { Icon } from './Icon';
import { Kbd } from './controls';

/** Where a popover opens: under a point, or over it, for one opened from the bottom of the window. */
export interface Anchor {
    x: number;
    y: number;
    up?: boolean;
}

export function anchorOf(e: any, up = false): Anchor {
    const r = e.currentTarget.getBoundingClientRect();
    return { x: r.left, y: up ? r.top - 6 : r.bottom + 6, up };
}

/** A workspace's mark: its first letter on a square of its color. */
export function SpaceMark(props: { space: Space; scheme: Scheme; size?: number; on?: boolean }) {
    const { space, scheme } = props;
    const size = props.size ?? 18;
    const color = scheme.ansi[space.color] ?? scheme.ansi[4];
    return (
        <div
            className="flex shrink-0 items-center justify-center font-semibold"
            style={{
                width: size,
                height: size,
                borderRadius: Math.round(size * 0.3),
                fontSize: Math.round(size * 0.56),
                background: withAlpha(color, props.on === false ? 0.14 : 0.26),
                color: css(mix(color, scheme.fg, 0.25)),
            }}
        >
            {(space.name.trim()[0] ?? '·').toUpperCase()}
        </div>
    );
}

/** The open workspace, by name, as a quiet button that opens the list of them. */
export function SpaceButton(props: { workspace: Workspace; scheme: Scheme; version: number; onOpen: (anchor: Anchor) => void }) {
    const { workspace, scheme } = props;
    void props.version;
    const t = tokens(scheme);
    const [hover, setHover] = useState(false);
    const space = workspace.space;
    return (
        <button
            aria-label="Workspaces"
            title="Workspaces"
            className="flex h-7 min-w-0 max-w-full items-center gap-2 rounded-lg pl-1.5 pr-2 transition-colors duration-150 [app-region:no-drag]"
            style={{ background: hover ? t.hover : 'transparent', color: t.text }}
            onPointerEnter={() => setHover(true)}
            onPointerLeave={() => setHover(false)}
            onPointerDown={(e: any) => e.preventDefault()}
            onClick={(e: any) => props.onOpen(anchorOf(e))}
        >
            <SpaceMark space={space} scheme={scheme} />
            <span className="min-w-0 truncate text-[13px] font-medium">{space.name}</span>
            <span className="flex shrink-0" style={{ color: t.faint }}>
                <Icon name="chevron" size={12} />
            </span>
        </button>
    );
}

/** The list of workspaces, to switch to one, add one, or change one. */
export function SpacePopover(props: { workspace: Workspace; scheme: Scheme; anchor: Anchor; shortcut: (id: string) => string | undefined; onClose: () => void; onEdit: (space: Space) => void }) {
    const { workspace, scheme, anchor } = props;
    const t = tokens(scheme);
    const [hovered, setHovered] = useState(-1);
    const width = 272;
    const left = Math.max(8, Math.min(anchor.x, window.innerWidth - width - 8));
    return (
        <div className="absolute inset-0 z-30" onPointerDown={props.onClose}>
            <div
                className="rise absolute overflow-hidden rounded-xl p-1"
                style={{
                    left,
                    width,
                    ...(anchor.up ? { bottom: window.innerHeight - anchor.y } : { top: anchor.y }),
                    background: withAlpha(mix(scheme.bg, scheme.fg, 0.05), 0.98),
                    border: `1px solid ${withAlpha(scheme.fg, 0.12)}`,
                    boxShadow: `0 16px 48px ${withAlpha(0x000000, 0.5)}`,
                    color: t.text,
                }}
                onPointerDown={(e: any) => e.stopPropagation()}
            >
                <div className="px-2.5 pb-1 pt-1.5 text-[11.5px] font-medium" style={{ color: t.faint }}>
                    Workspaces
                </div>
                {workspace.spaces.map((space, i) => {
                    const on = i === workspace.spaceIndex;
                    const tabs = space.columns.length;
                    return (
                        <div
                            key={space.id}
                            className="flex h-9 cursor-default items-center gap-2.5 rounded-lg pl-2 pr-1.5"
                            style={{ background: on ? t.chosen : hovered === i ? t.hover : 'transparent' }}
                            onPointerEnter={() => setHovered(i)}
                            onPointerLeave={() => setHovered(-1)}
                            onClick={() => {
                                workspace.switchSpace(i);
                                props.onClose();
                            }}
                        >
                            <SpaceMark space={space} scheme={scheme} size={20} />
                            <div className="flex min-w-0 flex-1 flex-col">
                                <span className="truncate text-[13px]" style={{ fontWeight: on ? 500 : 400 }}>
                                    {space.name}
                                </span>
                            </div>
                            {hovered === i ? (
                                <button
                                    aria-label={`Edit ${space.name}`}
                                    title="Edit"
                                    className="flex size-6 items-center justify-center rounded-md hover:bg-white/10"
                                    style={{ color: t.sub }}
                                    onClick={(e: any) => {
                                        e.stopPropagation();
                                        props.onEdit(space);
                                    }}
                                >
                                    <Icon name="dots" size={13} />
                                </button>
                            ) : (
                                <span className="pr-1 text-[11.5px] tabular-nums" style={{ color: t.faint }}>
                                    {tabs} {tabs === 1 ? 'tab' : 'tabs'}
                                </span>
                            )}
                            {props.shortcut('space:' + (i + 1)) ? <Kbd scheme={scheme}>{props.shortcut('space:' + (i + 1))!}</Kbd> : null}
                        </div>
                    );
                })}
                <div className="mx-2 my-1 h-px" style={{ background: t.border }} />
                <button
                    className="flex h-8 w-full items-center gap-2.5 rounded-lg pl-2.5 text-left text-[13px] hover:bg-white/[0.06]"
                    style={{ color: t.sub }}
                    onClick={() => {
                        props.onClose();
                        props.onEdit(workspace.addSpace());
                    }}
                >
                    <Icon name="plus" size={14} />
                    New workspace
                </button>
            </div>
        </div>
    );
}

/** Changing a workspace: its name, color and folder, or closing it. */
export function SpaceEditor(props: { workspace: Workspace; space: Space; scheme: Scheme; onClose: () => void }) {
    const { workspace, space, scheme } = props;
    const t = tokens(scheme);
    const [name, setName] = useState(space.name);
    const [folder, setFolder] = useState(space.folder);
    const [color, setColor] = useState(space.color);
    const nameInput = useRef<HTMLInputElement>(null);
    useEffect(() => {
        nameInput.current?.focus();
        nameInput.current?.select?.();
    }, []);
    const here = workspace.currentSession?.cwd ?? '';
    const save = () => {
        workspace.editSpace(space, { name: name.trim() || space.name, folder: folder.trim(), color });
        props.onClose();
    };
    const preview: Space = { ...space, name: name || space.name, color };
    const field = 'h-9 w-full rounded-lg px-3 text-[13px] outline-none';
    const fieldStyle = { background: withAlpha(scheme.fg, 0.06), color: t.text, boxShadow: `inset 0 0 0 1px ${withAlpha(scheme.fg, 0.06)}` };
    const value = (e: any) => (typeof e === 'string' ? e : e.target.value);
    return (
        <div className="fade-in absolute inset-0 z-30 flex items-center justify-center" style={{ background: withAlpha(0x000000, 0.32) }} onPointerDown={props.onClose}>
            <div
                className="rise w-[400px] max-w-[92%] rounded-2xl p-5"
                style={{
                    background: withAlpha(mix(scheme.bg, scheme.fg, 0.04), 0.98),
                    border: `1px solid ${withAlpha(scheme.fg, 0.12)}`,
                    boxShadow: `0 16px 48px ${withAlpha(0x000000, 0.5)}`,
                    color: t.text,
                }}
                onPointerDown={(e: any) => e.stopPropagation()}
                onKeyDown={(e: any) => {
                    if (e.key === 'Enter') save();
                    else if (e.key === 'Escape') props.onClose();
                }}
            >
                <div className="flex items-center gap-3">
                    <SpaceMark space={preview} scheme={scheme} size={32} />
                    <div className="text-[15px] font-semibold">Workspace</div>
                </div>

                <div className="mb-1.5 mt-5 text-[12px] font-medium" style={{ color: t.sub }}>
                    Name
                </div>
                <input ref={nameInput} value={name} className={field} style={fieldStyle} onChange={(e: any) => setName(value(e))} />

                <div className="mb-1.5 mt-4 text-[12px] font-medium" style={{ color: t.sub }}>
                    Color
                </div>
                <div className="flex gap-2">
                    {SPACE_COLORS.map((c) => (
                        <button
                            key={c}
                            aria-label={`Color ${c}`}
                            className="size-6 rounded-full"
                            style={{
                                background: css(scheme.ansi[c] ?? scheme.fg),
                                boxShadow: c === color ? `0 0 0 2px ${css(mix(scheme.bg, scheme.fg, 0.04))}, 0 0 0 3.5px ${withAlpha(scheme.fg, 0.7)}` : 'none',
                            }}
                            onClick={() => setColor(c)}
                        />
                    ))}
                </div>

                <div className="mb-1.5 mt-4 text-[12px] font-medium" style={{ color: t.sub }}>
                    Folder
                </div>
                <input value={folder} placeholder="Where its new shells start; none is where you are" className={field} style={{ ...fieldStyle, fontFamily: 'Cascadia Mono, Consolas, monospace', fontSize: 12 }} onChange={(e: any) => setFolder(value(e))} />
                {here && here !== folder ? (
                    <button className="mt-1.5 truncate text-left text-[12px] hover:underline" style={{ color: t.sub }} onClick={() => setFolder(here)}>
                        Use this shell’s folder: {here}
                    </button>
                ) : null}

                <div className="mt-6 flex items-center gap-2">
                    {workspace.spaces.length > 1 ? (
                        <button
                            className="h-8 rounded-lg px-3 text-[13px] transition-colors hover:bg-white/[0.06]"
                            style={{ color: css(scheme.ansi[1]) }}
                            onClick={() => {
                                workspace.closeSpace(workspace.spaces.indexOf(space));
                                props.onClose();
                            }}
                        >
                            Close workspace
                        </button>
                    ) : null}
                    <div className="flex-1" />
                    <button className="h-8 rounded-lg px-4 text-[13px]" style={{ background: withAlpha(scheme.fg, 0.08) }} onClick={props.onClose}>
                        Cancel
                    </button>
                    <button className="h-8 rounded-lg px-4 text-[13px] font-medium" style={{ background: withAlpha(scheme.fg, 0.88), color: css(scheme.bg) }} onClick={save}>
                        Done
                    </button>
                </div>
            </div>
        </div>
    );
}
