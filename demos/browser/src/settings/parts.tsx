// What settings are laid out with: a row with a title, a line on what it does and its control; a chip of a few; and a plain
// button for doing something once.

export function Row(props: { title: string; detail?: string; children: any }) {
    return (
        <div className="flex min-h-14 items-center justify-between gap-6 border-b border-(--b-line) py-3">
            <div className="flex min-w-0 flex-col gap-0.5">
                <span className="text-[13px] font-medium text-(--b-text)">{props.title}</span>
                {props.detail ? <span className="text-[12px] leading-[1.45] text-(--b-muted)">{props.detail}</span> : null}
            </div>
            {props.children}
        </div>
    );
}

export function Chip(props: { on: boolean; onClick: () => void; children: any }) {
    return (
        <button
            aria-pressed={props.on}
            onClick={props.onClick}
            className={
                'h-6 rounded-full px-2.5 text-[12px] transition-colors duration-(--b-speed) ' +
                (props.on ? 'bg-(--b-accent) text-(--b-on-accent)' : 'bg-(--b-hover) text-(--b-muted) hover:text-(--b-text)')
            }
        >
            {props.children}
        </button>
    );
}

export function Plain(props: { onClick: () => void; children: any }) {
    return (
        <button
            onClick={props.onClick}
            className="h-7 shrink-0 rounded-(--b-radius-sm) bg-(--b-hover) px-3 text-[12.5px] text-(--b-text) transition-colors duration-(--b-speed) hover:bg-(--b-press)"
        >
            {props.children}
        </button>
    );
}
