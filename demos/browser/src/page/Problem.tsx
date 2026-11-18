// The browser's own page for when a page can't be shown: what went wrong in words, the site, and one thing to do about it.

import * as I from '../icons';

/** What Chromium's network errors mean, for the ones people meet. */
export function explain(code: number): { title: string; detail: string } {
    if (code === -105 || code === -137) {
        return { title: "This site can't be found", detail: 'Check the address for a typo, or try again in a moment.' };
    }
    if (code === -106) {
        return { title: "You're offline", detail: "Your computer isn't connected to the internet." };
    }
    if (code === -102 || code === -7 || code === -118) {
        return { title: "This site isn't answering", detail: 'It may be down, or blocking the connection.' };
    }
    if (code <= -200 && code > -300) {
        return { title: "This connection isn't private", detail: "The site's certificate couldn't be checked, so the page wasn't opened." };
    }
    if (code === -1) {
        return { title: 'This page stopped working', detail: 'Something in it went wrong. Reloading usually brings it back.' };
    }
    return { title: "This page couldn't be opened", detail: 'Something went wrong on the way there.' };
}

export function Problem(props: { title: string; detail: string; code?: string; host?: string; action: string; onAction: () => void }) {
    return (
        <div className="flex size-full flex-1 flex-col items-center justify-center bg-(--b-card) px-8 text-center animate-[fade-in_var(--b-move)_var(--b-ease)]">
            <span className="flex size-14 items-center justify-center rounded-(--b-radius-lg) bg-(--b-hover) text-(--b-muted)">
                <I.GlobeIcon size={26} strokeWidth={1.5} />
            </span>
            <h2 className="mt-5 text-[19px] font-semibold tracking-[-0.01em] text-(--b-text)">{props.title}</h2>
            {props.host ? <p className="mt-1 text-[13px] font-medium text-(--b-muted)">{props.host}</p> : null}
            <p className="mt-2 max-w-[380px] text-[13px] leading-[1.5] text-(--b-muted)">{props.detail}</p>
            <button
                onClick={props.onAction}
                className="mt-6 h-8 rounded-(--b-radius) bg-(--b-accent) px-4 text-[13px] font-medium text-(--b-on-accent) transition-opacity duration-(--b-speed) hover:opacity-90 active:opacity-80"
            >
                {props.action}
            </button>
            {props.code ? <p className="mt-4 font-mono text-[11px] text-(--b-faint)">{props.code}</p> : null}
        </div>
    );
}
