// A long list built the way a page builds one: rows of Tailwind-styled `<div>`s in a container that scrolls, with name, size
// and date columns as the file explorer's details view has them. `LONG_LIST_N` says how many rows (1,000 unless it says),
// `LONG_LIST_MODE=cv` puts `content-visibility: auto` on each row, the explicit form of skipping what's off screen, and
// `LONG_LIST_MODE=native` has the same rows made of rsui's own `<row>` and `<text>` in a `<list>`, which only builds what shows,
// for comparing with.
//
// The keyboard moves the selection, as a file list's does: Up and Down, Home and End, each bringing the selected row into view.

import { memo, useCallback, useLayoutEffect, useRef, useState } from 'react';
import { PageWindow } from 'rsui/web';
import './app.css';

export const title = 'Long list';
export const size = [1000, 700];

const COUNT = Number(process.env.LONG_LIST_N || 1000);
const EXPLICIT = process.env.LONG_LIST_MODE === 'cv';
const NATIVE = process.env.LONG_LIST_MODE === 'native';

interface Item {
    name: string;
    size: string;
    date: string;
    folder: boolean;
}

const ITEMS: Item[] = Array.from({ length: COUNT }, (_, index) => ({
    name: index % 7 === 0 ? `Folder ${index}` : `file-${index}.${['txt', 'png', 'dll', 'json', 'md'][index % 5]}`,
    size: index % 7 === 0 ? '' : `${((index * 37) % 9000) + 1} KB`,
    date: `2026-${String((index % 12) + 1).padStart(2, '0')}-${String((index % 28) + 1).padStart(2, '0')} 12:${String(index % 60).padStart(2, '0')}`,
    folder: index % 7 === 0,
}));

const Row = memo(function Row({ index, selected, onPick }: { index: number; selected: boolean; onPick: (index: number) => void }) {
    const item = ITEMS[index];
    const ref = useRef<HTMLDivElement>(null);
    useLayoutEffect(() => {
        if (selected) {
            ref.current?.scrollIntoView({ block: 'nearest' });
        }
    }, [selected]);
    return (
        <div
            ref={ref}
            data-index={index}
            onMouseDown={() => onPick(index)}
            className={`flex h-7 items-center gap-2 border-b border-transparent px-3 text-sm ${EXPLICIT ? 'row-cv' : ''} ${
                selected ? 'bg-blue-100 text-blue-950' : index % 2 === 1 ? 'bg-zinc-50 hover:bg-zinc-100' : 'hover:bg-zinc-100'
            }`}
        >
            <span className={`size-4 shrink-0 rounded-sm ${item.folder ? 'bg-amber-400' : 'bg-zinc-300'}`} />
            <span className="min-w-0 flex-1 truncate">{item.name}</span>
            <span className="w-24 shrink-0 text-right text-zinc-500 tabular-nums">{item.size}</span>
            <span className="w-40 shrink-0 text-zinc-500 tabular-nums">{item.date}</span>
        </div>
    );
});

/// The same row, made of rsui's own elements.
const NativeRow = memo(function NativeRow({ index, selected }: { index: number; selected: boolean }) {
    const item = ITEMS[index];
    const muted = '#71717a';
    return (
        <row height={28} paddingX={12} gap={8} align="center" borderWidth={0} background={selected ? '#dbeafe' : index % 2 === 1 ? '#fafafa' : undefined}>
            <group width={16} height={16} shrink={0} radius={2} background={item.folder ? '#fbbf24' : '#d4d4d8'} />
            <text grow={1} shrink={1} minWidth={0} ellipsis noWrap fontSize={14} color={selected ? '#172554' : '#18181b'}>
                {item.name}
            </text>
            <text width={96} shrink={0} textAlign="right" fontSize={14} color={muted}>
                {item.size}
            </text>
            <text width={160} shrink={0} fontSize={14} color={muted}>
                {item.date}
            </text>
        </row>
    );
});

function List() {
    const [selected, setSelected] = useState(0);
    const onPick = useCallback((index: number) => setSelected(index), []);
    const onKeyDown = (event: React.KeyboardEvent) => {
        const page = 20;
        const next = { ArrowDown: selected + 1, ArrowUp: selected - 1, End: COUNT - 1, Home: 0, PageDown: selected + page, PageUp: selected - page }[event.key];
        if (next !== undefined) {
            event.preventDefault();
            setSelected(Math.max(0, Math.min(COUNT - 1, next)));
        }
    };
    const renderRow = useCallback((index: number) => <NativeRow key={index} index={index} selected={index === selected} />, [selected]);
    const rows = [];
    for (let index = 0; index < (NATIVE ? 0 : COUNT); index++) {
        rows.push(<Row key={index} index={index} selected={index === selected} onPick={onPick} />);
    }
    return (
        <div className="flex h-screen flex-col bg-white text-zinc-900">
            <div className="flex h-8 shrink-0 items-center gap-2 border-b border-zinc-200 px-3 text-xs font-medium text-zinc-500">
                <span className="w-4 shrink-0" />
                <span className="flex-1">Name</span>
                <span className="w-24 shrink-0 text-right">Size</span>
                <span className="w-40 shrink-0">Modified</span>
            </div>
            {NATIVE ? (
                <div id="rows" tabIndex={0} onKeyDown={onKeyDown} className="relative min-h-0 flex-1 outline-none">
                    <list count={COUNT} rowHeight={28} reveal={selected} renderRow={renderRow} position="absolute" top={0} left={0} right={0} bottom={0} />
                </div>
            ) : (
                <div id="rows" tabIndex={0} onKeyDown={onKeyDown} className="min-h-0 flex-1 overflow-y-auto outline-none">
                    {rows}
                </div>
            )}
            <div className="flex h-6 shrink-0 items-center border-t border-zinc-200 px-3 text-xs text-zinc-500">
                {COUNT} items, row {selected + 1} selected
            </div>
        </div>
    );
}

export default function App() {
    return (
        <PageWindow>
            <List />
        </PageWindow>
    );
}
