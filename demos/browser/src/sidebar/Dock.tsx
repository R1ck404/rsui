// The sidebar in its place beside the page. Hiding it closes it up from the page's side, so that it slides out under the
// window's edge as the page takes its room, and showing it opens it back out. While it's hidden, what's left of it is the edge
// that brings it back over the page (see `Peek`).

import { Resizer } from './Resizer';

/** How wide the gap beside it is, which resizes it, and which is all that's left of it while it's hidden. */
const GAP = 8;

export function Dock(props: {
    shown: boolean;
    width: number;
    /** Whether it's on the right, which puts everything the other way round. */
    right: boolean;
    /** Whether it's being dragged wider or narrower, which it follows at once. */
    dragging: boolean;
    children: any;
    onResize: (width: number) => void;
    onResized: (width: number) => void;
    /** The pointer came to the window's edge while it's hidden. */
    onEdge: () => void;
}) {
    const { shown, width, right } = props;
    return (
        // What's in it keeps to the page's side, so that it's the window's side that it goes under.
        <div
            className={
                'flex shrink-0 justify-end overflow-hidden ' +
                (right ? 'flex-row-reverse ' : '') +
                (props.dragging ? '' : 'transition-[width] duration-(--b-move) ease-(--b-ease)')
            }
            style={{ width: shown ? width + GAP : GAP }}
        >
            <div
                className={'flex shrink-0 flex-col transition-[opacity,visibility] duration-(--b-move) ease-(--b-ease) ' + (shown ? '' : 'invisible opacity-0')}
                style={{ width }}
            >
                {props.children}
            </div>
            {shown ? (
                <Resizer width={width} right={right} onMove={props.onResize} onDone={props.onResized} />
            ) : (
                <div className="shrink-0" style={{ width: GAP }} onPointerEnter={props.onEdge} />
            )}
        </div>
    );
}
