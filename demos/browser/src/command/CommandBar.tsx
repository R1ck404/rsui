// The command bar: where an address is typed, or a search, with suggestions from the open tabs, the favorites and where the
// browser's been. It floats over the page, dimmed behind it: the page shows as a still picture of itself there, as a window
// can't be drawn over.

import { useEffect, useRef, useState } from 'react';
import * as I from '../icons';
import type { Favorite, Tab, Visit } from '../model';
import { Kbd } from '../ui/controls';
import { SiteIcon } from '../ui/SiteIcon';
import { type Engine, shortUrl } from '../url';
import { type Suggestion, suggest } from './suggest';

export function CommandBar(props: {
    /** What's in it as it opens: the page's address, when it's opened to change it. */
    initial: string;
    /** Whether what's chosen replaces the open page, rather than opening in a new tab. */
    replace: boolean;
    engine: Engine;
    tabs: Tab[];
    favorites: Favorite[];
    history: Visit[];
    onChoose: (suggestion: Suggestion, inNewTab: boolean) => void;
    onClose: () => void;
}) {
    const { initial, replace, engine, tabs, favorites, history } = props;
    const [text, setText] = useState(initial);
    const [picked, setPicked] = useState(0);
    const input = useRef<any>(null);
    useEffect(() => {
        input.current?.focus();
        input.current?.select?.();
    }, []);
    const shown = suggest(text, initial, { engine, tabs, favorites, history });
    const at = Math.min(picked, shown.length - 1);
    const choose = (suggestion: Suggestion | undefined, newTab: boolean) => {
        if (suggestion !== undefined && suggestion.url !== '') {
            props.onChoose(suggestion, newTab);
        }
    };
    return (
        <div className="absolute inset-0 z-40 flex justify-center bg-(--b-scrim) px-6 pt-[12vh] animate-[fade-in_var(--b-speed)_ease-out]" onPointerDown={props.onClose}>
            <div
                onPointerDown={(event: any) => event.stopPropagation()}
                className="flex h-fit max-h-[70vh] w-full max-w-[640px] flex-col overflow-hidden rounded-(--b-radius-lg) bg-(--b-raised) shadow-(--b-pop-shadow) animate-[rise_var(--b-move)_var(--b-ease)]"
            >
                <div className="flex h-14 shrink-0 items-center gap-3 px-4">
                    <span className="flex text-(--b-faint)"><I.SearchIcon size={18} strokeWidth={1.8} /></span>
                    <input
                        ref={input}
                        value={text}
                        placeholder={'Search ' + engine.name + ' or enter address'}
                        onChange={(event: any) => {
                            setText(event.target.value);
                            setPicked(0);
                        }}
                        onKeyDown={(event: any) => {
                            if (event.key === 'ArrowDown') {
                                event.preventDefault();
                                setPicked((at + 1) % shown.length);
                            } else if (event.key === 'ArrowUp') {
                                event.preventDefault();
                                setPicked((at - 1 + shown.length) % shown.length);
                            } else if (event.key === 'Enter') {
                                event.preventDefault();
                                // Alt+Enter opens what's chosen in a new tab, even when it's changing the open page.
                                choose(shown[at], !replace || event.altKey);
                            } else if (event.key === 'Escape') {
                                event.preventDefault();
                                props.onClose();
                            }
                        }}
                        className="h-full min-w-0 flex-1 bg-transparent text-[16px] text-(--b-text) outline-none"
                    />
                    <Kbd>Esc</Kbd>
                </div>
                {shown.length > 0 ? <div className="h-px shrink-0 bg-(--b-line)" /> : null}
                <div className="flex min-h-0 flex-col overflow-y-auto p-1.5">
                    {shown.map((suggestion, index) => (
                        <Row
                            key={suggestion.kind + suggestion.url}
                            suggestion={suggestion}
                            picked={index === at}
                            engine={engine}
                            onHover={() => setPicked(index)}
                            onChoose={() => choose(suggestion, !replace)}
                        />
                    ))}
                </div>
            </div>
        </div>
    );
}

function Row(props: { suggestion: Suggestion; picked: boolean; engine: Engine; onHover: () => void; onChoose: () => void }) {
    const { suggestion, picked, engine } = props;
    const what = {
        go: 'Go',
        search: engine.name,
        tab: 'Switch to tab',
        favorite: 'Favorite',
        history: shortUrl(suggestion.url),
    }[suggestion.kind];
    return (
        <button
            onPointerMove={props.onHover}
            onClick={props.onChoose}
            className={
                'flex h-11 shrink-0 items-center gap-3 rounded-(--b-radius) px-3 text-left transition-colors duration-[60ms] ' +
                (picked ? 'bg-(--b-accent-soft)' : '')
            }
        >
            <span className={'flex w-5 shrink-0 justify-center ' + (picked ? 'text-(--b-accent)' : 'text-(--b-muted)')}>
                {suggestion.kind === 'search' ? (
                    <I.SearchIcon size={16} strokeWidth={1.8} />
                ) : suggestion.kind === 'go' ? (
                    <I.GlobeIcon size={16} strokeWidth={1.7} />
                ) : (
                    <SiteIcon url={suggestion.url} icon={suggestion.icon} size={16} />
                )}
            </span>
            <span className="min-w-0 flex-1 truncate text-(--b-text)">{suggestion.title}</span>
            <span className={'flex max-w-[40%] shrink-0 items-center gap-1 text-[12px] ' + (picked ? 'text-(--b-accent)' : 'text-(--b-faint)')}>
                <span className="min-w-0 truncate">{what}</span>
                {suggestion.kind === 'tab' ? <I.ChevronRightIcon size={11} strokeWidth={2} /> : null}
            </span>
        </button>
    );
}
