// The tabs that aren't shells: settings, the background studio, recordings and a browser. Each is a page in a tab of its own,
// as a terminal is, and the ones with a list of their own to pick from (settings' sections, the recordings) show it at the left
// of their panel, or, in the sidebar layout, in place of the window's sidebar, which slides away for it.

import { memo, useContext, useMemo, useRef, useState } from 'react';
import type { WebViewHandle, WebViewKey, WebViewState } from 'rsui/webview';
import type { Command, Context } from '../commands';
import { isNativeShortcut } from '../keymap';
import type { Column, TabKind, Workspace } from '../workspace';
import { themeOf } from '../workspace';
import type { Settings } from '../settings';
import { withAlpha, tokens } from '../themes';
import type { Scheme } from '../themes';
import { Studio } from '../studio/Studio';
import { Icon } from './Icon';
import { IconButton } from './controls';
import type { IconName } from './Icon';
import { SettingsBody, SettingsNav } from './SettingsPage';
import type { PageId } from './SettingsPage';
import { RecordingsBody, RecordingsNav, newestRecording } from './Recordings';
import { RecordingSidebarContext } from './RecordingLayout';
import { RoutinesPage } from './Routines';

/** What the pages need of the app that changes as it does: kept here, so a tab is given only what says it changed. */
export const pageEnv: { context: Context | null; keys: ReadonlyMap<string, Command>; runKey: (chord: string) => boolean } = {
    context: null, keys: new Map(), runKey: () => false,
};

/** The app's keys that still work while a web page has the keyboard. */
const BROWSER_APP_KEYS = new Set(['ctrl+shift+t', 'ctrl+shift+w', 'ctrl+shift+p', 'ctrl+alt+b', 'ctrl+alt+n', 'ctrl+alt+pageup', 'ctrl+alt+pagedown', 'ctrl+1', 'ctrl+2', 'ctrl+3', 'ctrl+4', 'ctrl+5', 'ctrl+6', 'ctrl+7', 'ctrl+8', 'ctrl+9', 'ctrl+shift+o', 'ctrl+tab', 'ctrl+shift+tab', 'ctrl+pageup', 'ctrl+pagedown', 'ctrl+alt+left', 'ctrl+alt+right', 'ctrl+,']);

export const PAGE_KINDS: { kind: Exclude<TabKind, 'terminal'>; label: string; icon: IconName }[] = [
    { kind: 'browser', label: 'Browser', icon: 'globe' },
    { kind: 'studio', label: 'Background studio', icon: 'studio' },
    { kind: 'recordings', label: 'Recordings', icon: 'video' },
    { kind: 'routines', label: 'Routines', icon: 'split' },
    { kind: 'settings', label: 'Settings', icon: 'gear' },
];

/** Whether a kind of page has a list of its own at its side. */
export const hasNav = (kind: TabKind) => kind === 'settings' || kind === 'recordings';

export const HOME = 'https://duckduckgo.com/';

/** A page tab's name and icon, for its tab and the sidebar. */
export function pageLabel(column: Column): { title: string; icon: IconName } {
    switch (column.kind) {
        case 'settings':
            return { title: 'Settings', icon: 'gear' };
        case 'studio':
            return { title: 'Studio', icon: 'studio' };
        case 'recordings':
            return { title: 'Recordings', icon: 'video' };
        case 'routines':
            return { title: 'Routines', icon: 'split' };
        case 'browser':
            return { title: column.view.title || hostOf(column.view.at ?? column.view.url ?? '') || 'New tab', icon: 'globe' };
        default:
            return { title: 'Terminal', icon: 'terminal' };
    }
}

export function hostOf(url: string): string {
    const m = /^[a-z]+:\/\/([^/?#]+)/i.exec(url);
    return m ? m[1].replace(/^www\./, '') : '';
}

/** What's typed in the address bar, as somewhere to go: an address as it is, or else a search for it. */
export function addressFor(typed: string): string {
    const text = typed.trim();
    if (text === '') return HOME;
    if (/^[a-z]+:\/\//i.test(text) || text.startsWith('about:')) return text;
    if (!/\s/.test(text) && /^[\w-]+(\.[\w-]+)+(:\d+)?(\/.*)?$/.test(text)) return 'https://' + text;
    if (/^localhost(:\d+)?(\/.*)?$/.test(text)) return 'http://' + text;
    return 'https://duckduckgo.com/?q=' + encodeURIComponent(text);
}

interface PageProps {
    column: Column;
    workspace: Workspace;
    settings: Settings;
    scheme: Scheme;
    /** Its own list shows at its side, as it does everywhere but the sidebar layout, which shows it in the sidebar. */
    withNav: boolean;
    focused: boolean;
    /** The workspace's version: a page's view changes in place. */
    version: number;
}

/** A page tab's panel. */
export function PageBody(props: PageProps) {
    const { column } = props;
    void props.version;
    const t = tokens(props.scheme);
    const sidebar = useContext(RecordingSidebarContext);
    if (column.kind === 'recordings') {
        const chosen = chosenRecording(column);
        const library = (
            <RecordingsNav chosen={chosen} scheme={props.scheme} version={props.version}
                onChoose={(file) => props.workspace.setView(column, { recording: file })} />
        );
        return <RecordingsBody chosen={chosen} settings={props.settings} scheme={props.scheme} theme={themeOf(props.scheme)}
            nav={{ inline: props.withNav || !sidebar, sidebar, library }} active={props.focused} />;
    }
    const body =
        column.kind === 'settings' ? (
            <SettingsBody key={column.view.page ?? 'general'} settings={props.settings} scheme={props.scheme} context={pageEnv.context!} change={pageEnv.context!.change} page={(column.view.page as PageId) ?? 'general'} />
        ) : column.kind === 'studio' ? (
            props.focused ? (
                <Studio settings={props.settings} scheme={props.scheme} change={pageEnv.context!.change} onApplied={() => props.workspace.back()} />
            ) : (
                <Resting icon="studio" scheme={props.scheme} />
            )
        ) : column.kind === 'routines' ? (
            props.focused ? <RoutinesPage scheme={props.scheme} /> : <Resting icon="split" scheme={props.scheme} />
        ) : column.kind === 'browser' ? (
            <BrowserPage {...props} />
        ) : null;
    if (!props.withNav || !hasNav(column.kind)) return <div className="relative size-full overflow-hidden" style={{ color: t.text }}>{body}</div>;
    return (
        <div className="flex size-full" style={{ color: t.text }}>
            <div className="w-[216px] shrink-0 overflow-y-auto px-2 pt-3" style={{ borderRight: `1px solid ${t.border}` }}>
                <PageNav {...props} />
            </div>
            <div className="relative min-w-0 flex-1 overflow-hidden">{body}</div>
        </div>
    );
}

/** A page's own list: settings' sections, or what's been recorded. */
export const PageNav = memo(function PageNav(props: { column: Column; workspace: Workspace; scheme: Scheme; version: number }) {
    const { column, workspace } = props;
    void props.version;
    if (column.kind === 'settings') {
        return <SettingsNav page={(column.view.page as PageId) ?? 'general'} scheme={props.scheme} onPage={(page) => workspace.setView(column, { page })} />;
    }
    return null;
});

function chosenRecording(column: Column): string | null {
    if (column.view.recording === undefined) column.view.recording = newestRecording();
    return column.view.recording ?? null;
}

/** What a page that's out of sight shows in the strip, in place of what it would cost to keep drawing. */
function Resting(props: { icon: IconName; scheme: Scheme }) {
    return (
        <div className="flex size-full items-center justify-center" style={{ color: withAlpha(props.scheme.fg, 0.3) }}>
            <Icon name={props.icon} size={28} />
        </div>
    );
}

// ---- Browser

function BrowserPage(props: PageProps) {
    const { column, workspace, scheme } = props;
    const { keys, context } = pageEnv;
    const keymap = props.settings.keymap;
    const keptKeys = useMemo(() => {
        const kept = ['ctrl+l', 'f5'];
        for (const [key, command] of keys) {
            if (command.id !== 'record' && !BROWSER_APP_KEYS.has(key) && !Object.hasOwn(keymap, command.id)) continue;
            if (key.includes('meta+') || !isNativeShortcut(key) || (command.when && (!context || !command.when(context)))) continue;
            if (!kept.includes(key)) kept.push(key);
        }
        return kept;
    }, [keys, keymap, context?.session, props.version]);
    const t = tokens(scheme);
    const view = useRef<WebViewHandle | null>(null);
    const [state, setState] = useState<WebViewState | null>(null);
    const [typed, setTyped] = useState<string | null>(null);
    const input = useRef<HTMLInputElement>(null);
    const src = column.view.url ?? HOME;
    const shownUrl = state?.url || column.view.at || src;

    const onState = (next: WebViewState) => {
        setState(next);
        if (next.title !== column.view.title || next.url !== column.view.at) workspace.setView(column, { title: next.title, at: next.url });
    };
    const go = (text: string) => {
        const url = addressFor(text);
        setTyped(null);
        if (url === src) view.current?.go(url);
        else workspace.setView(column, { url, at: url, title: '' });
        view.current?.focus();
    };
    const onKey = (key: WebViewKey) => {
        const name = key.name.toLowerCase();
        if (pageEnv.runKey(name)) return;
        if (name === 'ctrl+l') {
            view.current?.blur();
            input.current?.focus();
            return;
        }
        if (name === 'f5') {
            view.current?.reload();
            return;
        }
    };

    return (
        <div className="flex size-full flex-col">
            <div className="flex h-11 shrink-0 items-center gap-1 px-2" style={{ borderBottom: `1px solid ${t.border}` }}>
                <IconButton size={30} keepFocus icon="back" label="Back" scheme={scheme} disabled={!state?.canGoBack} onClick={() => view.current?.back()} />
                <IconButton size={30} keepFocus icon="forward" label="Forward" scheme={scheme} disabled={!state?.canGoForward} onClick={() => view.current?.forward()} />
                <IconButton size={30} keepFocus icon={state?.loading ? 'close' : 'reload'} label={state?.loading ? 'Stop' : 'Reload (F5)'} scheme={scheme} onClick={() => (state?.loading ? view.current?.stop() : view.current?.reload())} />
                <div className="ml-1 flex h-[30px] min-w-0 flex-1 items-center gap-2 rounded-lg px-3" style={{ background: withAlpha(scheme.fg, 0.06) }}>
                    {state?.icon ? <img src={state.icon} className="size-[14px] shrink-0 rounded-[3px]" /> : <span style={{ color: t.faint }}><Icon name="globe" size={14} /></span>}
                    <input
                        ref={input}
                        value={typed ?? shownUrl}
                        placeholder="Search or type an address"
                        className="min-w-0 flex-1 bg-transparent text-[12.5px] outline-none"
                        style={{ color: typed === null ? t.sub : t.text }}
                        onFocus={() => setTyped(shownUrl)}
                        onBlur={() => setTyped(null)}
                        onChange={(e: any) => setTyped(typeof e === 'string' ? e : e.target.value)}
                        onKeyDown={(e: any) => {
                            if (e.key === 'Enter') go(typed ?? shownUrl);
                            else if (e.key === 'Escape') {
                                setTyped(null);
                                view.current?.focus();
                            }
                        }}
                    />
                </div>
            </div>
            <div className="relative min-h-0 flex-1">
                <webview
                    src={src}
                    profile="terminal"
                    keys={keptKeys}
                    ref={(handle: WebViewHandle | null) => (view.current = handle)}
                    className="flex size-full"
                    onState={onState}
                    onNewWindow={(request) => workspace.openTool('browser', { url: request.url })}
                    onKey={onKey}
                    onClose={() => workspace.closeColumn(workspace.columns.indexOf(column))}
                >
                    <div className="flex size-full flex-col items-center justify-center gap-2" style={{ color: t.faint }}>
                        <Icon name="globe" size={24} />
                        <span className="text-[12px]">{hostOf(src)}</span>
                    </div>
                </webview>
                {state?.unavailable ? (
                    <Problem scheme={scheme} title="Pages need Microsoft Edge WebView2" detail={state.unavailable} />
                ) : state?.error ? (
                    <Problem scheme={scheme} title="This page didn’t load" detail={`${hostOf(state.error.url)} · ${state.error.description}`} />
                ) : null}
            </div>
        </div>
    );
}

function Problem(props: { scheme: Scheme; title: string; detail: string }) {
    const t = tokens(props.scheme);
    return (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-1.5" style={{ background: t.panel }}>
            <div className="text-[14px] font-medium" style={{ color: t.text }}>
                {props.title}
            </div>
            <div className="max-w-[420px] text-center text-[12.5px]" style={{ color: t.sub }}>
                {props.detail}
            </div>
        </div>
    );
}
