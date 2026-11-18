// What's selected, larger: a picture of it as Explorer would show, the start of a text file, and its details.

import { useEffect, useState } from 'react';
import { createReadStream } from 'fs';
import { attributesOf, formatDate, formatSize, kindLabel, readFolder, type Entry } from './files';
import { FileIcon } from './FileIcon';
import type { Settings } from './settings';
import { useThumbnail } from './thumbs';

function readStart(path: string, bytes: number): Promise<string> {
    return new Promise((resolve, reject) => {
        let text = '';
        const stream = createReadStream(path, { encoding: 'utf8', start: 0, end: Math.max(0, bytes - 1) });
        stream.on('data', (chunk: any) => (text += chunk));
        stream.on('end', () => resolve(text));
        stream.on('error', reject);
    });
}

const TEXT_KINDS = new Set(['text', 'code', 'web', 'data', 'script', 'markdown', 'config']);

function Row(props: { label: string; value: string }) {
    return (
        <div className="flex gap-3 py-1">
            <span className="w-20 shrink-0 text-(--fx-faint)">{props.label}</span>
            <span className="min-w-0 flex-1 break-all text-(--fx-muted)">{props.value}</span>
        </div>
    );
}

export function Preview(props: { entry: Entry | null; count: number; settings: Settings; now: number; width: number; colors: { muted: string; accent: string; [other: string]: string } }) {
    const { entry, count, settings: s, now, colors } = props;
    const picture = useThumbnail(entry && !entry.folder ? entry : null, 512, s.previewImages, s.thumbnailCache, 60);
    const [text, setText] = useState<string | null>(null);
    const [inside, setInside] = useState<number | null>(null);

    useEffect(() => {
        setText(null);
        setInside(null);
        if (!entry) {
            return;
        }
        let live = true;
        // A moment's wait, so holding an arrow key down doesn't read every file it passes.
        const later = setTimeout(() => {
            if (entry.folder) {
                readFolder(entry.path).then((found) => live && setInside(found.length), () => {});
            }
            if (s.previewText && TEXT_KINDS.has(entry.kind) && entry.size > 0) {
                readStart(entry.path, s.previewMaxKB * 1024).then((found) => live && setText(found), () => {});
            }
        }, 60);
        return () => {
            live = false;
            clearTimeout(later);
        };
    }, [entry?.path, entry?.modified, s.previewText, s.previewMaxKB]);

    return (
        <aside className="flex shrink-0 flex-col overflow-y-auto rounded-(--fx-radius-lg) bg-(--fx-card) shadow-(--fx-card-shadow) [scrollbar-width:thin]" style={{ width: props.width }}>
            {!entry ? (
                <div className="flex flex-1 items-center justify-center px-6 text-center text-(--fx-faint)">
                    {count > 1 ? `${count} items selected` : 'Select something to see it here'}
                </div>
            ) : (
                <div className="flex flex-col gap-4 p-4">
                    <div className="flex min-h-40 items-center justify-center overflow-hidden rounded-(--fx-radius-lg) bg-(--fx-input)">
                        {picture ? (
                            <img src={picture} className="max-h-72 max-w-full object-contain" />
                        ) : (
                            <FileIcon
                                entry={entry}
                                size={72}
                                stroke={1.1}
                                style={s.iconStyle}
                                tintFolders={s.tintFolders}
                                colorFiles={s.colorFileIcons}
                                sharedSystemIcons={s.systemIconCache}
                                colors={colors}
                            />
                        )}
                    </div>
                    <div className="flex flex-col gap-0.5">
                        <span className="text-[1.08em] font-semibold break-all text-(--fx-text)">{entry.name}</span>
                        <span className="text-(--fx-muted)">
                            {kindLabel(entry)}
                            {entry.folder ? (inside === null ? '' : ` · ${inside} items`) : ' · ' + formatSize(entry.size, s.sizeUnits)}
                        </span>
                    </div>
                    {text !== null ? (
                        <div className="h-72 shrink-0 overflow-auto rounded-(--fx-radius) bg-(--fx-input) [scrollbar-width:thin]">
                            <pre
                                className={
                                    'm-0 p-3 text-[0.86em] leading-relaxed text-(--fx-muted) ' +
                                    (s.previewMono ? 'font-mono ' : '') +
                                    (s.previewWrap ? 'break-words whitespace-pre-wrap' : 'whitespace-pre')
                                }
                            >
                                {text}
                            </pre>
                        </div>
                    ) : null}
                    {s.previewMeta ? (
                        <div className="flex flex-col pt-1 text-[0.92em]">
                            <Row label="Modified" value={formatDate(entry.modified, s.dateFormat === 'relative' ? 'long' : s.dateFormat, s.clock24, now)} />
                            <Row label="Created" value={formatDate(entry.created, s.dateFormat === 'relative' ? 'long' : s.dateFormat, s.clock24, now)} />
                            {entry.folder ? null : <Row label="Size" value={formatSize(entry.size, 'bytes')} />}
                            {attributesOf(entry) ? <Row label="Attributes" value={attributesOf(entry)} /> : null}
                            <Row label="Where" value={entry.path} />
                        </div>
                    ) : null}
                </div>
            )}
        </aside>
    );
}
