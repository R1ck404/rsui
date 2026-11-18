// A tab's panes, laid out by the page: each split in src/layout.ts is a flex row or column of what's in it, each part as big as
// its share of it (`flex-grow`, from nothing), with a handle in each gap between them. Nothing here knows how big the window is:
// layout works that out, and when it changes, everything follows it at once, as no size is said anywhere to move. What moves is a
// share: when a pane's split off, closed or evened out, the shares glide to their new ones, and a pane just split off grows from
// nothing. Only the split whose handle is held follows it at once.
//
// A pane that ends up in another split, as one split the other way is, is a new element there, so a pane keeps what it's showing
// by its id rather than by where it is: see `Pane`.

import { useEffect, useRef, useState } from 'react';
import type { Handle as HandleBox, Layout } from './layout';
import { Handle } from './Panes';

export interface SplitsProps {
    layout: Layout;
    gap: number;
    /** The split whose handle is held, whose shares follow the pointer rather than gliding. */
    held: string | null;
    /** Whether a pane's just been split off, which grows from nothing. */
    fresh: (id: string) => boolean;
    /** The pane with this id, drawn. */
    pane: (id: string) => any;
    /** Told the element each pane is laid out in as it's made (`there`), and as it goes, for where the panes are. */
    placed: (id: string, element: any, there: boolean) => void;
    onStart: (handle: HandleBox) => void;
    onMove: (handle: HandleBox, by: number) => void;
    onDone: () => void;
    onReset: (handle: HandleBox) => void;
}

/** How shares move: see `--fx-move` and `--fx-ease` in theme.ts. */
const GLIDES = 'transition-[flex-grow] duration-(--fx-move) ease-(--fx-ease) ';

/** One part of a split: its share of it, which a part that's just been made starts from nothing to grow to. */
function Part(props: {
    share: number;
    glides: boolean;
    grows: boolean;
    dir: 'row' | 'column';
    placed?: (element: any, there: boolean) => void;
    children: any;
}) {
    const { share, glides, grows, dir, placed, children } = props;
    const [started, setStarted] = useState(!grows);
    const own = usePlaced(placed);
    useEffect(() => {
        if (started) {
            return;
        }
        // Laid out once with no share, so there's something for it to grow from.
        const timer = setTimeout(() => setStarted(true), 16);
        return () => clearTimeout(timer);
    }, []);
    return (
        <div
            ref={own}
            className={'flex min-h-0 min-w-0 basis-0 ' + (dir === 'row' ? 'flex-row ' : 'flex-col ') + (glides ? GLIDES : '')}
            style={{ flexGrow: started ? share : 0 }}
        >
            {children}
        </div>
    );
}

/** What's in `layout`, which is all of what it's in. */
function Inside(props: SplitsProps & { node: Layout }) {
    const { node, gap, held } = props;
    if (node.kind === 'pane') {
        return props.pane(node.id);
    }
    const dir = node.dir;
    const parts: any[] = [];
    node.children.forEach((child, index) => {
        if (index > 0) {
            parts.push(
                <Handle
                    key={'h' + index}
                    handle={{ split: node.id, index: index - 1, dir, room: 0 }}
                    gap={gap}
                    onStart={props.onStart}
                    onMove={props.onMove}
                    onDone={props.onDone}
                    onReset={props.onReset}
                />,
            );
        }
        // A part is known by what's in it, so the same pane or split keeps its element as others come and go beside it.
        parts.push(
            <Part
                key={child.id}
                share={node.sizes[index]}
                glides={held !== node.id}
                grows={child.kind === 'pane' && props.fresh(child.id)}
                // What's inside a part goes across it as its split goes, unless it's a split of its own.
                dir={child.kind === 'split' ? child.dir : 'row'}
                placed={child.kind === 'pane' ? (element: any, there: boolean) => props.placed(child.id, element, there) : undefined}
            >
                <Inside {...props} node={child} />
            </Part>,
        );
    });
    return parts;
}

/** A ref that tells `placed` the element it's given, and which element it was when it's let go. */
function usePlaced(placed?: (element: any, there: boolean) => void) {
    const element = useRef<any>(null);
    return (made: any) => {
        if (made) {
            element.current = made;
            placed?.(made, true);
        } else if (element.current) {
            placed?.(element.current, false);
        }
    };
}

/** All of a tab's panes, as its layout has them, filling what they're in. A tab of one pane is a split of one, so that when a
 * pane beside it closes, the one that's left is the same element, and glides to all of the room from its share of it. */
export function Splits(props: SplitsProps) {
    const { layout } = props;
    const node: Layout = layout.kind === 'pane' ? { kind: 'split', id: '~whole', dir: 'row', children: [layout], sizes: [1] } : layout;
    return (
        <div className={'flex min-h-0 min-w-0 flex-1 ' + (node.dir === 'row' ? 'flex-row' : 'flex-col')}>
            <Inside {...props} node={node} />
        </div>
    );
}
