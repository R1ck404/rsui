// What opens over the strip: the command palette, the search bar, and the messages that come up in a corner.

import { useEffect, useMemo, useRef, useState } from 'react';
import { fuzzy } from '../commands';
import type { Command, Context } from '../commands';
import type { Session } from '../term/Session';
import { useSession } from '../term/TerminalPane';
import type { Toast } from '../workspace';
import { Icon } from './Icon';
import { Kbd } from './controls';
import { css, mix, withAlpha } from '../themes';
import type { Scheme } from '../themes';

export function Palette(props: { commands: Command[]; context: Context; scheme: Scheme; shortcut: (id: string) => string | undefined; onClose: () => void }) {
    const { scheme, context } = props;
    const [query, setQuery] = useState('');
    const [at, setAt] = useState(0);
    const input = useRef<HTMLInputElement>(null);
    useEffect(() => {
        input.current?.focus();
    }, []);
    const results = useMemo(() => {
        const scored: { command: Command; score: number }[] = [];
        for (const command of props.commands) {
            if (command.when && !command.when(context)) continue;
            const score = fuzzy(query, command.title);
            if (score !== null) scored.push({ command, score });
        }
        scored.sort((a, b) => b.score - a.score);
        return scored.slice(0, 10).map((s) => s.command);
    }, [query, props.commands]);
    const chosen = Math.min(at, Math.max(0, results.length - 1));
    const run = (command: Command | undefined) => {
        if (!command) return;
        props.onClose();
        // After the palette has gone, so what it does isn't undone by focus returning.
        setTimeout(() => command.run(context), 0);
    };
    return (
        <div className="absolute inset-0 z-30 flex justify-center pt-[12vh]" style={{ background: withAlpha(0x000000, 0.28) }} onPointerDown={props.onClose}>
            <div
                className="rise h-fit w-[560px] max-w-[92%] overflow-hidden rounded-2xl"
                style={{
                    background: withAlpha(mix(scheme.bg, scheme.fg, 0.05), 0.97),
                    border: `1px solid ${withAlpha(scheme.fg, 0.14)}`,
                    boxShadow: `0 24px 80px ${withAlpha(0x000000, 0.6)}`,
                    color: css(scheme.fg),
                }}
                onPointerDown={(e: any) => e.stopPropagation()}
            >
                <div className="flex items-center gap-3 px-4 py-3" style={{ borderBottom: `1px solid ${withAlpha(scheme.fg, 0.08)}` }}>
                    <span style={{ color: withAlpha(scheme.fg, 0.5) }}>
                        <Icon name="command" size={17} />
                    </span>
                    <input
                        ref={input}
                        value={query}
                        placeholder="Type a command"
                        className="min-w-0 flex-1 bg-transparent text-[15px] outline-none"
                        style={{ color: css(scheme.fg) }}
                        onChange={(e: any) => {
                            setQuery(typeof e === 'string' ? e : e.target.value);
                            setAt(0);
                        }}
                        onKeyDown={(e: any) => {
                            if (e.key === 'ArrowDown') {
                                setAt(Math.min(results.length - 1, chosen + 1));
                                e.preventDefault();
                            } else if (e.key === 'ArrowUp') {
                                setAt(Math.max(0, chosen - 1));
                                e.preventDefault();
                            } else if (e.key === 'Enter') {
                                run(results[chosen]);
                                e.preventDefault();
                            } else if (e.key === 'Escape') {
                                props.onClose();
                                e.preventDefault();
                            }
                        }}
                    />
                </div>
                <div className="max-h-[380px] overflow-y-auto p-1.5">
                    {results.length === 0 ? (
                        <div className="px-3 py-6 text-center text-[13px]" style={{ color: withAlpha(scheme.fg, 0.5) }}>
                            Nothing matches
                        </div>
                    ) : null}
                    {results.map((command, i) => {
                        const key = props.shortcut(command.id);
                        return (
                            <div
                                key={command.id}
                                className="flex cursor-pointer items-center gap-3 rounded-lg px-3 py-2"
                                style={{ background: i === chosen ? withAlpha(scheme.fg, 0.1) : 'transparent' }}
                                onPointerEnter={() => setAt(i)}
                                onClick={() => run(command)}
                            >
                                <span className="min-w-0 flex-1 truncate text-[13.5px]">{command.title}</span>
                                <span className="shrink-0 text-[11px]" style={{ color: withAlpha(scheme.fg, 0.4) }}>
                                    {command.group}
                                </span>
                                {key ? <Kbd scheme={scheme}>{key}</Kbd> : null}
                            </div>
                        );
                    })}
                </div>
            </div>
        </div>
    );
}

export function SearchBar(props: { session: Session; scheme: Scheme; onClose: () => void }) {
    const { session, scheme } = props;
    useSession(session);
    const state = session.search;
    const input = useRef<HTMLInputElement>(null);
    useEffect(() => {
        input.current?.focus();
        input.current?.select?.();
    }, []);
    const toggle = (label: string, on: boolean, title: string, run: () => void) => (
        <button
            title={title}
            className="flex h-6 min-w-6 items-center justify-center rounded-md px-1 text-[11.5px] font-semibold"
            style={{ background: on ? withAlpha(scheme.ansi[4], 0.35) : 'transparent', color: on ? css(scheme.fg) : withAlpha(scheme.fg, 0.55) }}
            onPointerDown={(e: any) => e.preventDefault()}
            onClick={run}
        >
            {label}
        </button>
    );
    const count = state.query === '' ? '' : state.error ? 'bad pattern' : state.matches.length === 0 ? 'no matches' : `${state.current + 1} of ${state.matches.length}`;
    return (
        <div
            className="rise absolute right-5 top-14 z-20 flex w-[400px] items-center gap-1.5 rounded-xl px-2.5 py-2"
            style={{
                background: withAlpha(mix(scheme.bg, scheme.fg, 0.07), 0.97),
                border: `1px solid ${withAlpha(state.error ? scheme.ansi[1] : scheme.fg, state.error ? 0.7 : 0.14)}`,
                boxShadow: `0 12px 40px ${withAlpha(0x000000, 0.5)}`,
                color: css(scheme.fg),
            }}
            onPointerDown={(e: any) => e.stopPropagation()}
        >
            <span className="px-1" style={{ color: withAlpha(scheme.fg, 0.5) }}>
                <Icon name="search" size={15} />
            </span>
            <input
                ref={input}
                value={state.query}
                placeholder="Find"
                className="min-w-0 flex-1 bg-transparent text-[13.5px] outline-none"
                style={{ color: css(scheme.fg) }}
                onChange={(e: any) => session.setSearch({ query: typeof e === 'string' ? e : e.target.value })}
                onKeyDown={(e: any) => {
                    if (e.key === 'Enter') {
                        session.stepSearch(e.shiftKey ? -1 : 1);
                        e.preventDefault();
                    } else if (e.key === 'Escape') {
                        props.onClose();
                        e.preventDefault();
                    } else if (e.altKey && e.key.toLowerCase() === 'c') {
                        session.setSearch({ caseSensitive: !state.caseSensitive });
                        e.preventDefault();
                    } else if (e.altKey && e.key.toLowerCase() === 'r') {
                        session.setSearch({ regex: !state.regex });
                        e.preventDefault();
                    }
                }}
            />
            <span className="min-w-[58px] text-right text-[11.5px] tabular-nums" style={{ color: withAlpha(scheme.fg, 0.55) }}>
                {count}
            </span>
            {toggle('Aa', state.caseSensitive, 'Match case (Alt+C)', () => session.setSearch({ caseSensitive: !state.caseSensitive }))}
            {toggle('.*', state.regex, 'Regular expression (Alt+R)', () => session.setSearch({ regex: !state.regex }))}
            {toggle('↑', false, 'Previous (Shift+Enter)', () => session.stepSearch(-1))}
            {toggle('↓', false, 'Next (Enter)', () => session.stepSearch(1))}
            {toggle('✕', false, 'Close (Esc)', props.onClose)}
        </div>
    );
}

export function Toasts(props: { toasts: Toast[]; scheme: Scheme }) {
    const { scheme } = props;
    return (
        <div className="pointer-events-none absolute bottom-5 right-5 z-40 flex flex-col items-end gap-2">
            {props.toasts.map((toast) => {
                const dot = toast.tone === 'bad' ? scheme.ansi[1] : toast.tone === 'ok' ? scheme.ansi[2] : scheme.ansi[4];
                return (
                    <div
                        key={toast.id}
                        className="toast-in flex max-w-[380px] items-start gap-3 rounded-xl py-2.5 pl-3.5 pr-4"
                        style={{
                            background: css(mix(scheme.bg, scheme.fg, 0.08)),
                            boxShadow: `0 12px 32px ${withAlpha(0x000000, 0.45)}, 0 0 0 1px ${withAlpha(scheme.fg, 0.06)}`,
                            color: css(scheme.fg),
                        }}
                    >
                        <div className="mt-[6px] size-[7px] shrink-0 rounded-full" style={{ background: css(dot) }} />
                        <div className="min-w-0">
                            <div className="text-[13px] font-medium">{toast.text}</div>
                            {toast.detail ? (
                                <div className="mt-0.5 truncate text-[12px]" style={{ color: withAlpha(scheme.fg, 0.55) }}>
                                    {toast.detail}
                                </div>
                            ) : null}
                        </div>
                    </div>
                );
            })}
        </div>
    );
}
