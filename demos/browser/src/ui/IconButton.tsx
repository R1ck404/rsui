// A button that's only an icon, with nothing behind it until it's hovered.

export type IconType = (props: { size?: number; strokeWidth?: number; className?: string }) => any;

export function IconButton(props: { icon: IconType; label: string; onClick: () => void; disabled?: boolean; size?: number }) {
    const { icon: Icon, label, disabled } = props;
    return (
        <button
            title={label}
            aria-label={label}
            disabled={disabled}
            onClick={props.onClick}
            className="flex size-7 shrink-0 items-center justify-center rounded-(--b-radius-sm) text-(--b-muted) transition-colors duration-(--b-speed) hover:bg-(--b-hover) hover:text-(--b-text) active:bg-(--b-press) disabled:pointer-events-none disabled:opacity-40"
        >
            <Icon size={props.size ?? 16} strokeWidth={1.7} />
        </button>
    );
}
