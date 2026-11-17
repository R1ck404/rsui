// The picture for a file or folder: a folder, or a page with what it holds drawn on it, as Finder draws them, or the icon Windows
// shows for it.

import { memo, useEffect, useState } from 'react';
import { Circle, Path, Rect, Svg } from 'rsui';
import { files } from 'rsui/desktop';
import type { Entry, Kind } from './files';
import * as I from './icons';
import { mix, rgba } from './theme';

const BY_KIND: Record<Kind, any> = {
    folder: I.FolderIcon,
    image: I.ImageIcon,
    design: I.PaletteIcon,
    video: I.VideoIcon,
    audio: I.AudioIcon,
    archive: I.FileZipIcon,
    code: I.CodeIcon,
    web: I.CodeIcon,
    data: I.CodeIcon,
    script: I.TerminalIcon,
    markdown: I.FileTextIcon,
    text: I.FileTextIcon,
    config: I.SlidersIcon,
    doc: I.FileTextIcon,
    sheet: I.FileChartIcon,
    slides: I.FileChartIcon,
    pdf: I.FilePdfIcon,
    ebook: I.FileTextIcon,
    model: I.FileIcon,
    database: I.DriveIcon,
    app: I.AppIcon,
    link: I.FileLinkIcon,
    disk: I.DiskIcon,
    font: I.FontIcon,
    lock: I.FileLockIcon,
    mail: I.FileIcon,
    calendar: I.FileIcon,
    contact: I.FileIcon,
    file: I.FileIcon,
};

const COLORS: Partial<Record<Kind, string>> = {
    image: '#d946ef',
    design: '#ec4899',
    video: '#f43f5e',
    audio: '#f59e0b',
    archive: '#a16207',
    code: '#0ea5e9',
    web: '#f97316',
    data: '#ca8a04',
    script: '#22c55e',
    markdown: '#475569',
    text: '#64748b',
    config: '#64748b',
    doc: '#2563eb',
    sheet: '#16a34a',
    slides: '#ea580c',
    pdf: '#ef4444',
    ebook: '#0d9488',
    model: '#06b6d4',
    database: '#0e7490',
    app: '#6366f1',
    link: '#14b8a6',
    disk: '#78716c',
    font: '#8b5cf6',
    lock: '#eab308',
    mail: '#3b82f6',
    calendar: '#ef4444',
    contact: '#10b981',
};

// Windows' icons, kept by file type (or by file, for what has its own icon), so a folder of 10,000 pictures asks once.
const OWN_ICON = new Set(['exe', 'lnk', 'ico', 'url', 'msi', 'scr', 'cur', 'ani']);
const systemIcons = new Map<string, string>();
const asking = new Map<string, Promise<string | null>>();

function systemIconKey(entry: Entry, shared: boolean): string {
    if (!shared || OWN_ICON.has(entry.ext)) {
        return entry.path;
    }
    return entry.folder ? (entry.link ? ':link-folder' : ':folder') : '.' + entry.ext;
}

function askSystemIcon(entry: Entry, key: string, size: number): Promise<string | null> {
    let pending = asking.get(key + size);
    if (!pending) {
        pending = files
            .icon(entry.path, { size })
            .then((picture) => {
                systemIcons.set(key + size, picture.url);
                return picture.url;
            })
            .catch(() => null);
        asking.set(key + size, pending);
    }
    return pending;
}

const SystemIcon = memo(function SystemIcon(props: { entry: Entry; size: number; shared: boolean; fallback: any }) {
    const { entry, size, shared } = props;
    // Windows' icons come in steps: ask for one a little bigger and let it scale down.
    const asked = size <= 16 ? 16 : size <= 32 ? 32 : size <= 48 ? 48 : 256;
    const key = systemIconKey(entry, shared);
    const [url, setUrl] = useState<string | null>(systemIcons.get(key + asked) ?? null);
    useEffect(() => {
        const known = systemIcons.get(key + asked);
        if (known) {
            setUrl(known);
            return;
        }
        let live = true;
        askSystemIcon(entry, key, asked).then((found) => live && setUrl(found));
        return () => {
            live = false;
        };
    }, [key, asked]);
    if (url === null) {
        const Fallback = props.fallback;
        return <Fallback size={size} strokeWidth={1.5} color="#8a8a8a" className="shrink-0" />;
    }
    return <img src={url} width={size} height={size} className="shrink-0" style={{ width: size, height: size }} />;
});

/** A color with some of it see-through, as `#rrggbbaa`. */
const alpha = (hex: string, amount: number) => rgba(hex, amount);

const PAGE = '#f6f6f8';
const PAGE_EDGE = '#00000026';
const FOLD = '#d6d6dc';
const FILE_COLOR = '#64748b';

/** A folder as a picture, as Finder's is: its back with the tab, darker, then its front, with a sheen along the top. */
function Folder(props: { size: number; color: string; inverted: boolean }) {
    const { size, color, inverted } = props;
    const base = inverted ? '#ffffff' : color;
    const back = inverted ? '#ffffffb3' : mix(base, '#000000', 0.25);
    const front = inverted ? '#ffffff' : mix(base, '#ffffff', 0.12);
    return (
        <Svg width={size} height={size} shrink={0} viewBox="0 0 24 24">
            <Path fill={back} d="M2.2 6.2C2.2 5 3.1 4.2 4.2 4.2h4.5c.6 0 1.1.2 1.5.6l1.3 1.4h8.3c1.1 0 2 .9 2 2V10H2.2z" />
            <Rect x={2.2} y={7.9} width={19.6} height={12.6} rx={2.1} fill={front} />
            <Rect x={2.2} y={7.9} width={19.6} height={1.1} rx={0.55} fill={inverted ? '#00000014' : alpha('#ffffff', 0.38)} />
            <Rect x={2.2} y={18.4} width={19.6} height={2.1} rx={1} fill="#00000012" />
        </Svg>
    );
}

/** What's drawn on a page for its kind: a little picture of what it holds, or a band in its color. */
function PageMark(props: { kind: Kind; color: string }) {
    const { kind, color } = props;
    const soft = alpha(color, 0.16);
    const line = (y: number, width: number, fill = '#b4b4bc') => <Rect key={y} x={7.3} y={y} width={width} height={1.05} rx={0.52} fill={fill} />;
    const stroke = { fill: 'none', stroke: color, strokeWidth: 1.3, strokeLinecap: 'round' as const };
    switch (kind) {
        case 'text':
            return <>{[10.4, 12.7, 15, 17.3].map((y, at) => line(y, at === 3 ? 5.8 : 9.4))}</>;
        case 'markdown':
            return (
                <>
                    <Rect x={6.8} y={10.6} width={10.4} height={7.4} rx={1.3} fill={soft} />
                    <Path {...stroke} d="M8.6 16.4v-4l1.7 2.1 1.7-2.1v4" />
                    <Path {...stroke} d="M15 12.4v3.8m-1.5-1.5 1.5 1.5 1.5-1.5" />
                </>
            );
        case 'config':
            return (
                <>
                    {[11.2, 14.2, 17.2].map((y) => <Rect key={y} x={7.4} y={y - 0.5} width={9.2} height={1} rx={0.5} fill="#c4c4cc" />)}
                    <Circle cx={10} cy={11.2} r={1.25} fill={color} />
                    <Circle cx={14.2} cy={14.2} r={1.25} fill={color} />
                    <Circle cx={11.4} cy={17.2} r={1.25} fill={color} />
                </>
            );
        case 'doc':
            return (
                <>
                    <Rect x={6.8} y={9.4} width={10.4} height={2.6} rx={0.7} fill={color} />
                    {[13.4, 15.5, 17.6].map((y, at) => line(y, at === 2 ? 6 : 9.4))}
                </>
            );
        case 'sheet':
            return (
                <>
                    <Rect x={6.8} y={9.8} width={10.4} height={8.8} rx={1} fill={soft} />
                    <Rect x={6.8} y={9.8} width={10.4} height={2.6} rx={1} fill={alpha(color, 0.55)} />
                    {[10.3, 13.8].map((x) => <Rect key={x} x={x} y={9.8} width={0.55} height={8.8} fill={alpha(color, 0.6)} />)}
                    {[12.4, 15.5].map((y) => <Rect key={y} x={6.8} y={y} width={10.4} height={0.55} fill={alpha(color, 0.6)} />)}
                </>
            );
        case 'slides':
            return (
                <>
                    <Rect x={6.6} y={10} width={10.8} height={7.2} rx={1.1} fill={soft} />
                    <Rect x={8.4} y={13.6} width={1.6} height={2.4} rx={0.4} fill={color} />
                    <Rect x={11.2} y={11.8} width={1.6} height={4.2} rx={0.4} fill={color} />
                    <Rect x={14} y={12.8} width={1.6} height={3.2} rx={0.4} fill={alpha(color, 0.6)} />
                    <Rect x={9.6} y={18.2} width={4.8} height={0.9} rx={0.45} fill="#c4c4cc" />
                </>
            );
        case 'pdf':
            return (
                <>
                    {line(9.6, 9.4)}
                    {line(11.7, 6.4)}
                    <Rect x={6.3} y={14.1} width={11.4} height={4.9} rx={1.2} fill={color} />
                </>
            );
        case 'image':
            return (
                <>
                    <Rect x={6.8} y={10.2} width={10.4} height={8.4} rx={1.3} fill={soft} />
                    <Path fill={alpha(color, 0.55)} d="M7.6 17.8l2.4-2.8 1.6 1.8z" />
                    <Path fill={color} d="M9.2 17.8l3.4-4 3.8 4z" />
                    <Circle cx={14.9} cy={12.3} r={1.15} fill={color} />
                </>
            );
        case 'design':
            return (
                <>
                    <Circle cx={10.4} cy={13.6} r={2.7} fill={alpha(color, 0.85)} />
                    <Rect x={11.3} y={13.5} width={5} height={5} rx={1} fill={alpha('#8b5cf6', 0.8)} />
                    <Path fill="#f59e0b" d="M8.2 18.6l1.6-3 1.6 3z" />
                </>
            );
        case 'video':
            return (
                <>
                    <Rect x={6.8} y={10.2} width={10.4} height={8.4} rx={1.3} fill={soft} />
                    {[11, 13.1, 15.2, 17.3].map((y) => (
                        <Rect key={y} x={7.4} y={y} width={1} height={0.9} rx={0.2} fill={alpha(color, 0.5)} />
                    ))}
                    {[11, 13.1, 15.2, 17.3].map((y) => (
                        <Rect key={'r' + y} x={15.6} y={y} width={1} height={0.9} rx={0.2} fill={alpha(color, 0.5)} />
                    ))}
                    <Path fill={color} d="M10.8 12.2v4.4l3.8-2.2z" />
                </>
            );
        case 'audio':
            return (
                <>
                    <Path {...stroke} strokeWidth={1.2} d="M11.3 16.6v-5.4l4.7-1v5.2" />
                    <Circle cx={10.2} cy={16.6} r={1.35} fill={color} />
                    <Circle cx={14.9} cy={15.4} r={1.35} fill={color} />
                </>
            );
        case 'code':
            return (
                <>
                    <Path {...stroke} d="M10 11.6 7.8 14.1l2.2 2.5m4-5 2.2 2.5-2.2 2.5" />
                    <Path {...stroke} strokeWidth={1.1} stroke={alpha(color, 0.6)} d="M12.7 11.2l-1.4 5.8" />
                </>
            );
        case 'web':
            return (
                <>
                    <Circle cx={12} cy={14.2} r={3.7} fill={soft} stroke={color} strokeWidth={1.1} />
                    <Path fill="none" stroke={color} strokeWidth={0.9} d="M8.3 14.2h7.4M12 10.5c1.3 1.3 1.8 2.5 1.8 3.7S13.3 16.6 12 17.9c-1.3-1.3-1.8-2.5-1.8-3.7s.5-2.4 1.8-3.7" />
                </>
            );
        case 'data':
            return (
                <>
                    <Path {...stroke} d="M10.3 10.9c-1 0-1.4.5-1.4 1.3v.9c0 .7-.4 1-1.1 1 .7 0 1.1.3 1.1 1v.9c0 .8.4 1.3 1.4 1.3" />
                    <Path {...stroke} d="M13.7 10.9c1 0 1.4.5 1.4 1.3v.9c0 .7.4 1 1.1 1-.7 0-1.1.3-1.1 1v.9c0 .8-.4 1.3-1.4 1.3" />
                    <Circle cx={12} cy={14.1} r={0.75} fill={color} />
                </>
            );
        case 'script':
            return (
                <>
                    <Rect x={6.6} y={10} width={10.8} height={8.4} rx={1.4} fill="#27272a" />
                    <Path fill="none" stroke="#4ade80" strokeWidth={1.2} strokeLinecap="round" d="M8.5 12.6l1.8 1.5-1.8 1.5" />
                    <Rect x={11.2} y={15.1} width={3.4} height={1} rx={0.5} fill="#4ade80" />
                </>
            );
        case 'archive':
            return (
                <>
                    {[3.4, 5.4, 7.4, 9.4, 11.4].map((y, at) => (
                        <Rect key={y} x={at % 2 === 0 ? 11 : 12} y={y} width={1} height={1.4} rx={0.3} fill={color} />
                    ))}
                    <Rect x={6.3} y={14.1} width={11.4} height={4.9} rx={1.2} fill={color} />
                </>
            );
        case 'ebook':
            return (
                <>
                    <Path fill={alpha(color, 0.85)} d="M6.9 11.2c1.8-.6 3.5-.4 5.1.7v6.6c-1.6-1.1-3.3-1.3-5.1-.7z" />
                    <Path fill={color} d="M17.1 11.2c-1.8-.6-3.5-.4-5.1.7v6.6c1.6-1.1 3.3-1.3 5.1-.7z" />
                </>
            );
        case 'model':
            return (
                <>
                    <Path fill={alpha(color, 0.45)} d="M12 10.2l4.2 2.3L12 14.8l-4.2-2.3z" />
                    <Path fill={alpha(color, 0.75)} d="M7.8 12.5 12 14.8v4.6l-4.2-2.3z" />
                    <Path fill={color} d="M16.2 12.5 12 14.8v4.6l4.2-2.3z" />
                </>
            );
        case 'database':
            return (
                <>
                    <Path fill={alpha(color, 0.75)} d="M8.2 11.4v6c0 .9 1.7 1.7 3.8 1.7s3.8-.8 3.8-1.7v-6z" />
                    <Path fill="none" stroke="#ffffff99" strokeWidth={0.7} d="M8.2 13.4c0 .9 1.7 1.7 3.8 1.7s3.8-.8 3.8-1.7M8.2 15.4c0 .9 1.7 1.7 3.8 1.7s3.8-.8 3.8-1.7" />
                    <Circle cx={12} cy={11.4} r={0.1} fill={color} />
                    <Path fill={color} d="M8.2 11.4c0-.9 1.7-1.7 3.8-1.7s3.8.8 3.8 1.7-1.7 1.7-3.8 1.7-3.8-.8-3.8-1.7" />
                </>
            );
        case 'app':
            return (
                <>
                    <Rect x={6.8} y={10.2} width={10.4} height={8} rx={1.3} fill={soft} stroke={color} strokeWidth={0.8} />
                    <Rect x={6.8} y={10.2} width={10.4} height={2.2} rx={1.1} fill={color} />
                    <Rect x={8.4} y={13.8} width={3.2} height={3} rx={0.6} fill={alpha(color, 0.6)} />
                    <Rect x={12.4} y={13.8} width={3.2} height={1.1} rx={0.5} fill="#c4c4cc" />
                    <Rect x={12.4} y={15.7} width={2.4} height={1.1} rx={0.5} fill="#c4c4cc" />
                </>
            );
        case 'link':
            return (
                <>
                    <Circle cx={12} cy={14.2} r={3.8} fill={soft} />
                    <Path {...stroke} d="M10 16.4c.2-2.4 1.6-3.7 4-3.7m-1.6-1.7 1.8 1.7-1.8 1.7" />
                </>
            );
        case 'disk':
            return (
                <>
                    <Circle cx={12} cy={14.2} r={3.9} fill={alpha(color, 0.3)} stroke={color} strokeWidth={0.9} />
                    <Circle cx={12} cy={14.2} r={1.2} fill={color} />
                    <Path fill="none" stroke="#ffffffaa" strokeWidth={0.7} strokeLinecap="round" d="M9.7 12.6c.5-.7 1.2-1.1 2-1.3" />
                </>
            );
        case 'font':
            return (
                <>
                    <Path {...stroke} strokeWidth={1.5} d="M8.4 17.8l3-7.6 3 7.6M9.5 15.3h3.8" />
                    <Circle cx={15.7} cy={16.6} r={1.25} fill="none" stroke={color} strokeWidth={1.1} />
                    <Rect x={16.6} y={15} width={0.9} height={2.8} rx={0.45} fill={color} />
                </>
            );
        case 'lock':
            return (
                <>
                    <Path fill="none" stroke={color} strokeWidth={1.2} d="M10.2 13.5v-1.5a1.8 1.8 0 0 1 3.6 0v1.5" />
                    <Rect x={8.8} y={13.4} width={6.4} height={5} rx={1.1} fill={color} />
                    <Rect x={11.5} y={15.1} width={1} height={1.7} rx={0.5} fill="#ffffffcc" />
                </>
            );
        case 'mail':
            return (
                <>
                    <Rect x={6.8} y={10.8} width={10.4} height={7.2} rx={1.2} fill={soft} stroke={color} strokeWidth={0.9} />
                    <Path fill="none" stroke={color} strokeWidth={1} d="M7.3 11.5l4.7 3.6 4.7-3.6" />
                </>
            );
        case 'calendar':
            return (
                <>
                    <Rect x={7.2} y={10} width={9.6} height={8.6} rx={1.3} fill="#ffffff" stroke="#d4d4d8" strokeWidth={0.6} />
                    <Rect x={7.2} y={10} width={9.6} height={2.6} rx={1.3} fill={color} />
                    {[0, 1, 2].map((row) =>
                        [0, 1, 2].map((col) => (
                            <Rect key={row + '-' + col} x={8.6 + col * 2.5} y={13.6 + row * 1.6} width={1.4} height={0.9} rx={0.3} fill={row === 1 && col === 1 ? color : '#c4c4cc'} />
                        )),
                    )}
                </>
            );
        case 'contact':
            return (
                <>
                    <Circle cx={12} cy={14} r={4} fill={soft} />
                    <Circle cx={12} cy={12.9} r={1.6} fill={color} />
                    <Path fill={color} d="M9 17.2c.4-1.6 1.6-2.5 3-2.5s2.6.9 3 2.5c-.8.6-1.8 1-3 1s-2.2-.4-3-1" />
                </>
            );
        default:
            return <Rect x={6.3} y={14.1} width={11.4} height={4.9} rx={1.2} fill={color} />;
    }
}

/** A file as a picture: a page with its corner folded over and a shadow under the fold, with what it holds drawn on it. */
function Page(props: { size: number; kind: Kind; color: string }) {
    const { size, kind, color } = props;
    return (
        <Svg width={size} height={size} shrink={0} viewBox="0 0 24 24">
            <Path fill={PAGE} stroke={PAGE_EDGE} strokeWidth={0.7} d="M6.2 2.4h8.1l5.1 5.1v12.6c0 .9-.7 1.5-1.5 1.5H6.2c-.9 0-1.5-.7-1.5-1.5V3.9c0-.9.7-1.5 1.5-1.5z" />
            <Path fill="#0000000a" d="M17.9 7.5h1.5v12.6c0 .9-.7 1.5-1.5 1.5z" />
            <Path fill="#00000012" d="M14.3 6c0 .8.7 1.5 1.5 1.5h3.6l-.9.9h-2.9c-.9 0-1.5-.7-1.5-1.5z" />
            <Path fill={FOLD} d="M14.3 2.4v3.6c0 .8.7 1.5 1.5 1.5h3.6z" />
            <PageMark kind={kind} color={color} />
        </Svg>
    );
}

/** The kinds whose band says what they are, in letters, when there's room. */
const LABELLED = new Set<Kind>(['pdf', 'archive', 'file']);

export const FileIcon = memo(function FileIcon(props: {
    entry: Entry;
    size: number;
    stroke: number;
    style: 'blode' | 'system';
    tintFolders: boolean;
    colorFiles: boolean;
    sharedSystemIcons: boolean;
    colors: { muted: string; accent: string; [other: string]: string };
    /** On a filled selection, where a folder's drawn in white. */
    inverted?: boolean;
    open?: boolean;
}) {
    const { entry, size, style, tintFolders, colorFiles } = props;
    if (style === 'system') {
        return <SystemIcon entry={entry} size={size} shared={props.sharedSystemIcons} fallback={BY_KIND[entry.kind]} />;
    }
    if (entry.folder) {
        return <Folder size={size} color={tintFolders ? props.colors.accent : '#7aa7d9'} inverted={props.inverted === true} />;
    }
    const color = (colorFiles ? COLORS[entry.kind] : undefined) ?? FILE_COLOR;
    const label = size >= 40 && LABELLED.has(entry.kind) && entry.ext ? entry.ext.toUpperCase().slice(0, 4) : null;
    if (label === null) {
        return <Page size={size} kind={entry.kind} color={color} />;
    }
    // The band says what kind of file it is, as a page in Finder's grid does.
    return (
        <group width={size} height={size} shrink={0}>
            <Page size={size} kind={entry.kind} color={color} />
            <text
                position="absolute"
                left={0}
                width={size}
                top={size * 0.585}
                height={size * 0.2}
                textAlign="center"
                fontSize={Math.max(7, size * 0.14)}
                fontWeight="bold"
                color="#ffffff"
                noWrap
            >
                {label}
            </text>
        </group>
    );
});

/** How many icons and thumbnails are kept, for the status bar. */
export function cachedIconCount(): number {
    return systemIcons.size;
}
