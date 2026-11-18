// The sidebar over the page while it's hidden, as Arc's is: it slides in from the window's edge while the pointer's there, and
// back out under it once the pointer leaves. Out of sight, it's hidden, shadow and all.

export function Peek(props: { open: boolean; width: number; right: boolean; children: any; onLeave: () => void }) {
    const { open, right } = props;
    const away = right ? 'translate-x-[calc(100%+8px)]' : '-translate-x-[calc(100%+8px)]';
    return (
        <div
            className={
                'absolute top-0 bottom-2 z-30 flex flex-col rounded-(--b-radius-lg) bg-(--b-raised) pt-2 shadow-(--b-pop-shadow) ' +
                'transition-[translate,visibility] duration-(--b-move) ease-(--b-ease) ' +
                (right ? 'right-2 ' : 'left-2 ') +
                (open ? 'translate-x-0' : 'invisible ' + away)
            }
            style={{ width: props.width }}
            onPointerLeave={props.onLeave}
        >
            {props.children}
        </div>
    );
}
