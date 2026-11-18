// One tab's page: its `<webview>`, with what shows until the page arrives, and the browser's own page for when it can't be
// shown. A tab that isn't open keeps its view, hidden, so going back to it is instant, until it's put to sleep.

import { shell } from 'rsui/desktop';
import type { WebViewHandle, WebViewKey, WebViewState } from 'rsui/webview';
import type { Tab } from '../model';
import { SiteIcon } from '../ui/SiteIcon';
import { hostOf } from '../url';
import { explain, Problem } from './Problem';

const GET_WEBVIEW2 = 'https://developer.microsoft.com/microsoft-edge/webview2/';

export function Page(props: {
    tab: Tab;
    shown: boolean;
    /** Whether its corners are rounded as the card's are, which they aren't full screen. */
    rounded: boolean;
    profile: string;
    state: WebViewState | undefined;
    keys: string[];
    view: (view: WebViewHandle | null) => void;
    onState: (state: WebViewState) => void;
    onNewWindow: (url: string) => void;
    onKey: (key: WebViewKey) => void;
    onClose: () => void;
    /** Goes to the address that failed again. */
    onRetry: (url: string) => void;
}) {
    const { tab, shown, state } = props;
    const failed = state?.error ?? null;
    const unavailable = state?.unavailable ?? null;
    const hidden = !shown || failed !== null || unavailable !== null;
    return (
        <>
            <webview
                src={tab.url}
                profile={props.profile}
                keys={props.keys}
                ref={props.view}
                className={hidden ? 'hidden' : 'flex size-full flex-1 bg-(--b-card) ' + (props.rounded ? 'rounded-(--b-radius-lg)' : '')}
                onState={props.onState}
                onNewWindow={(request) => props.onNewWindow(request.url)}
                onKey={props.onKey}
                onClose={props.onClose}
            >
                {/* Until the page has started arriving: where it's going, quietly. */}
                <div className="flex size-full flex-col items-center justify-center gap-3 bg-(--b-card) animate-[fade-in_var(--b-move)_var(--b-ease)]">
                    <SiteIcon url={tab.url} icon={tab.icon} size={28} />
                    <span className="text-[12px] text-(--b-faint)">{hostOf(tab.url)}</span>
                </div>
            </webview>
            {shown && unavailable !== null ? (
                <Problem title="Pages need Microsoft Edge WebView2" detail={unavailable} action="Get WebView2" onAction={() => void shell.open(GET_WEBVIEW2)} />
            ) : shown && failed !== null ? (
                <Problem {...explain(failed.code)} code={failed.description} host={hostOf(failed.url)} action="Try again" onAction={() => props.onRetry(failed.url)} />
            ) : null}
        </>
    );
}
