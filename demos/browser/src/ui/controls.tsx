// Small controls for settings and the tour: a switch, a segmented choice as a Mac has, and a key as a keyboard shows it.
// The switch is in pixels, as its knob's place is, so the two stay in step whatever the page's text size is.

export function Toggle(props: { on: boolean; onChange: (on: boolean) => void; label: string }) {
    const { on, label } = props;
    return (
        <button
            title={label}
            aria-label={label}
            role="switch"
            aria-checked={on}
            onClick={() => props.onChange(!on)}
            className={'relative h-[20px] w-[34px] shrink-0 rounded-full transition-colors duration-(--b-speed) ' + (on ? 'bg-(--b-accent)' : 'bg-(--b-press)')}
        >
            <span
                className={
                    'absolute top-[3px] size-[14px] rounded-full bg-white shadow-sm transition-[left] duration-(--b-speed) ' + (on ? 'left-[17px]' : 'left-[3px]')
                }
            />
        </button>
    );
}

export function Segmented<T extends string>(props: { value: T; options: { value: T; label: string }[]; onChange: (value: T) => void }) {
    const { value, options } = props;
    return (
        <div role="radiogroup" className="flex shrink-0 gap-0.5 rounded-(--b-radius-sm) bg-(--b-hover) p-0.5">
            {options.map((option) => (
                <button
                    key={option.value}
                    role="radio"
                    aria-checked={option.value === value}
                    onClick={() => props.onChange(option.value)}
                    className={
                        'h-6 rounded-[6px] px-2.5 text-[12px] transition-colors duration-(--b-speed) ' +
                        (option.value === value ? 'bg-(--b-raised) text-(--b-text) shadow-(--b-tab-shadow)' : 'text-(--b-muted) hover:text-(--b-text)')
                    }
                >
                    {option.label}
                </button>
            ))}
        </div>
    );
}

export function Kbd(props: { children: any }) {
    return (
        <kbd className="inline-flex h-5 min-w-5 items-center justify-center rounded-[5px] border border-(--b-line) bg-(--b-raised) px-1.5 font-sans text-[11px] text-(--b-muted)">
            {props.children}
        </kbd>
    );
}
