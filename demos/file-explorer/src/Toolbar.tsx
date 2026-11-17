// The bar along the top of a panel: back, forward and refresh, where it is as breadcrumbs (`This PC / Users / Rick`), the search,
// and how the folder's shown. It's quiet until it's used, as a browser's toolbar is.

import { memo, useEffect, useRef, useState } from 'react';
import { COMPUTER, crumbsOf, normalize } from './files';
import * as I from './icons';
import type { Settings, View } from './settings';
import { IconButton, IconSegmented } from './ui';

const Address = memo(function Address(props: { folder: string; mode: Settings['addressMode']; editAt: number; narrow: boolean; onGo: (path: string) => void }) {
    const { folder, mode, editAt, narrow, onGo } = props;
    const [editing, setEditing] = useState(false);
    const field = useRef<any>(null);
    useEffect(() => {
        if (editAt > 0) {
            setEditing(true);
        }
    }, [editAt]);
    useEffect(() => {
        if (editing && field.current) {
            field.current.focus();
            field.current.select();
        }
    }, [editing]);

    if (editing || mode === 'path') {
        return (
            <input
                ref={field}
                key={folder + (editing ? ':editing' : '')}
                defaultValue={folder === COMPUTER ? 'This PC' : folder}
                className="h-[28px] min-w-0 flex-1 rounded-(--fx-radius) bg-(--fx-input) px-2.5 text-(--fx-text) outline-none"
                onKeyDown={(event: any) => {
                    event.stopPropagation();
                    if (event.key === 'Enter') {
                        const typed = String(event.currentTarget.value);
                        setEditing(false);
                        onGo(typed.trim().toLowerCase() === 'this pc' ? COMPUTER : normalize(typed));
                    } else if (event.key === 'Escape') {
                        setEditing(false);
                    }
                }}
                onBlur={() => setEditing(false)}
            />
        );
    }
    const all = crumbsOf(folder);
    // A deep folder shows where it starts and its last few folders, with the rest behind "…", which goes to the nearest of them.
    // In a narrow panel, only the last two, after "…".
    const most = narrow ? 2 : 4;
    const crumbs = all.length > most + 1 ? [{ name: '…', path: all[all.length - most - 1].path }, ...all.slice(-most)] : all;
    return (
        <div className="flex h-[28px] min-w-0 flex-1 cursor-text items-center overflow-hidden" onClick={() => setEditing(true)}>
            {crumbs.map((crumb, at) => {
                const last = at === crumbs.length - 1;
                return (
                    <span key={crumb.path + at} className={'flex items-center ' + (last ? 'min-w-0 shrink' : 'shrink-0')}>
                        {at > 0 ? <span className="px-0.5 text-(--fx-faint)">/</span> : null}
                        <button
                            className={
                                'truncate rounded-[5px] px-1.5 py-0.5 transition-colors duration-(--fx-speed) ' +
                                (last ? 'font-medium text-(--fx-text)' : 'text-(--fx-muted) hover:bg-(--fx-hover) hover:text-(--fx-text)')
                            }
                            onClick={(event: any) => {
                                event.stopPropagation();
                                onGo(crumb.path);
                            }}
                        >
                            {crumb.name}
                        </button>
                    </span>
                );
            })}
        </div>
    );
});

const Search = memo(function Search(props: {
    query: string;
    recursive: boolean;
    focusAt: number;
    narrow: boolean;
    tiny?: boolean;
    onQuery: (query: string) => void;
    onRecursive: (on: boolean) => void;
    busy: boolean;
}) {
    const { query, recursive, focusAt, narrow, onQuery, onRecursive, busy } = props;
    const field = useRef<any>(null);
    useEffect(() => {
        if (focusAt > 0 && field.current) {
            field.current.focus();
            field.current.select();
        }
    }, [focusAt]);
    return (
        <div
            className={
                'flex h-[28px] shrink items-center gap-1.5 rounded-(--fx-radius) bg-(--fx-input) pr-1 pl-2 transition-colors duration-(--fx-speed) focus-within:bg-(--fx-press) ' +
                (props.tiny ? 'w-[104px] min-w-[30px]' : narrow ? 'w-36' : 'w-52')
            }
        >
            <span className={'flex shrink-0 ' + (busy ? 'animate-pulse text-(--fx-accent)' : 'text-(--fx-faint)')}><I.SearchIcon size={14} strokeWidth={1.8} /></span>
            <input
                ref={field}
                aria-label="Search"
                value={query}
                placeholder={recursive ? 'Search below' : 'Search'}
                className="min-w-0 flex-1 bg-transparent text-(--fx-text) outline-none placeholder:text-(--fx-faint)"
                onChange={(event: any) => onQuery(event.target.value)}
                onKeyDown={(event: any) => {
                    if (event.key === 'Escape') {
                        event.stopPropagation();
                        onQuery('');
                        event.currentTarget.blur();
                    } else if (event.key === 'ArrowDown' || event.key === 'Enter') {
                        event.currentTarget.blur();
                    } else {
                        event.stopPropagation();
                    }
                }}
            />
            {query ? (
                <button className="rounded-[4px] p-0.5 text-(--fx-faint) hover:text-(--fx-text)" onClick={() => onQuery('')}>
                    <I.XIcon size={12} strokeWidth={2} />
                </button>
            ) : null}
            <button
                title="Search subfolders"
                aria-label="Search subfolders"
                onClick={() => onRecursive(!recursive)}
                className={'rounded-[5px] p-1 ' + (recursive ? 'bg-(--fx-accent-soft) text-(--fx-accent)' : 'text-(--fx-faint) hover:text-(--fx-text)')}
            >
                <I.FoldersIcon size={13} strokeWidth={1.8} />
            </button>
        </div>
    );
});

const VIEWS: [string, any, string][] = [
    ['details', I.TableIcon, 'Details'],
    ['list', I.ListIcon, 'List'],
    ['grid', I.GridIcon, 'Grid'],
];

export const PaneBar = memo(function PaneBar(props: {
    settings: Settings;
    folder: string;
    canBack: boolean;
    canForward: boolean;
    view: View;
    query: string;
    recursive: boolean;
    searching: boolean;
    editAddressAt: number;
    focusSearchAt: number;
    narrow: boolean;
    /** Very narrow, as a pane in a row of three is: only what's needed most. */
    tiny?: boolean;
    /** Closes the pane, when there's more than one. */
    onClose?: () => void;
    onBack: () => void;
    onForward: () => void;
    onRefresh: () => void;
    onGo: (path: string) => void;
    onQuery: (query: string) => void;
    onRecursive: (on: boolean) => void;
    onView: (view: View) => void;
    onHidden: () => void;
    onNewFolder: () => void;
}) {
    const s = props.settings;
    const stroke = s.iconStroke + 0.15;
    const tiny = props.tiny === true;
    return (
        <header className="flex h-[46px] min-w-0 shrink-0 items-center gap-1 overflow-hidden px-2.5">
            <IconButton icon={I.ArrowLeftIcon} label="Back" stroke={stroke} disabled={!props.canBack} onClick={props.onBack} />
            <IconButton icon={I.ArrowRightIcon} label="Forward" stroke={stroke} disabled={!props.canForward} onClick={props.onForward} />
            {tiny ? null : <IconButton icon={I.RefreshIcon} label="Refresh" stroke={stroke} size={15} onClick={props.onRefresh} />}
            <div className={'ml-1.5 flex flex-1 items-center ' + (tiny ? 'min-w-[64px]' : 'min-w-0')}>
                <Address folder={props.folder} mode={s.addressMode} editAt={props.editAddressAt} narrow={props.narrow} onGo={props.onGo} />
            </div>
            <div className="ml-2 flex shrink-0 items-center gap-1.5">
                <Search
                    query={props.query}
                    recursive={props.recursive}
                    busy={props.searching}
                    focusAt={props.focusSearchAt}
                    narrow={props.narrow}
                    tiny={tiny}
                    onQuery={props.onQuery}
                    onRecursive={props.onRecursive}
                />
                {props.narrow ? null : <IconSegmented value={props.view} options={VIEWS} onChange={props.onView} />}
                {tiny ? null : (
                    <>
                        <IconButton icon={I.PlusIcon} label="New folder" stroke={stroke} disabled={props.folder === COMPUTER} onClick={props.onNewFolder} />
                        <IconButton icon={s.showHidden ? I.EyeIcon : I.EyeOffIcon} label="Hidden items" stroke={stroke} active={s.showHidden} onClick={props.onHidden} />
                    </>
                )}
                {props.onClose ? <IconButton icon={I.XIcon} label="Close pane" stroke={stroke} size={14} onClick={props.onClose} /> : null}
            </div>
        </header>
    );
});
