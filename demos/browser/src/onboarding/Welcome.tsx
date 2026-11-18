// The tour's first page: the browser's mark, and what it's about.

import * as I from '../icons';

export function Welcome(props: { color: string }) {
    return (
        <>
            <Mark color={props.color} />
            <h1 className="mt-8 text-[34px] font-semibold tracking-[-0.025em] text-(--b-text)">A calmer way to browse</h1>
            <p className="mt-3 max-w-[400px] text-[15px] leading-[1.55] text-(--b-muted)">
                Your tabs live in the sidebar. Pages get the whole window. Everything else stays out of the way.
            </p>
        </>
    );
}

/** The browser's mark: a rounded square in the space's color, deepening towards its corner, with a soft sheen on top. */
function Mark(props: { color: string }) {
    return (
        <div
            className="relative flex size-[88px] items-center justify-center overflow-hidden rounded-[26px] shadow-(--b-pop-shadow)"
            style={{ background: `linear-gradient(145deg, ${props.color}, color-mix(in srgb, ${props.color} 55%, #1a1a2e))` }}
        >
            <span className="absolute inset-0 bg-linear-to-b from-white/25 to-transparent to-55%" />
            <span className="relative flex text-white">
                <I.CompassIcon size={40} strokeWidth={1.6} />
            </span>
        </div>
    );
}
