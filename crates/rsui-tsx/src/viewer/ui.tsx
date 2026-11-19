// The pieces the inspector's panes are made of: icons, buttons, splitters, tabs, toolbars, sections, fields and tables.
import { TextInput, Svg, Path, Circle } from 'rsui';
import { MONO, UI } from './theme';
import type { Theme } from './theme';

export const ICONS: Record<string, string> = {
    resume: 'M8 5v14l11-7z',
    pause: 'M6 5h4v14H6zM14 5h4v14h-4z',
    right: 'M9 6l6 6-6 6z',
    down: 'M6 9l6 6 6-6z',
    close: 'M6 6l12 12M18 6L6 18',
    clear: 'M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18zM6 6l12 12',
    save: 'M12 3v12M7 10l5 5 5-5M5 20h14',
    eye: 'M2 12c3-6 17-6 20 0c-3 6-17 6-20 0zM12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6z',
    freeze: 'M12 2v20M4 7l16 10M20 7L4 17',
    file: 'M6 2h8l4 4v16H6zM14 2v4h4',
    folder: 'M3 6h6l2 2h10v11H3z',
    back: 'M15 6l-6 6 6 6',
    replay: 'M4 12a8 8 0 1 0 3-6.2M4 4v5h5',
    stop: 'M7 7h10v10H7z',
    dockRight: 'M3 5h18v14H3zM15 5v14',
    dockBottom: 'M3 5h18v14H3zM3 14h18',
    undock: 'M9 9h11v10H9zM4 15V5h11',
    record: 'M12 6a6 6 0 1 0 0 12a6 6 0 1 0 0-12z',
    live: 'M5 5l7 7-7 7M13 5l7 7-7 7',
    zoomOut: 'M10 4a6 6 0 1 0 0 12a6 6 0 1 0 0-12zM7 10h6M15 15l5 5',
    fit: 'M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5',
    pick: 'M4 4h7M4 4v7M20 20l-9-9M11 11v6l2-2 3 4M11 11h6l-2 2',
};

/// An icon, drawn with lines, or filled for those that are shapes.
export function Icon({ name, color, size = 16 }: { name: string; color: string; size?: number }) {
    const filled = name === 'record' || name === 'resume' || name === 'pause' || name === 'right' || name === 'down' || name === 'stop';
    if (name === 'over' || name === 'into' || name === 'out') {
        return (
            <Svg width={size} height={size} viewBox="0 0 24 24" color={color} fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
                {name === 'over' ? <Path d="M4 13a8 8 0 0 1 15-3.5M20 4v6h-6" /> : null}
                {name === 'into' ? <Path d="M12 3v10M8 9l4 4 4-4" /> : null}
                {name === 'out' ? <Path d="M12 14V4M8 8l4-4 4 4" /> : null}
                <Circle cx={12} cy={19} r={2} fill="currentColor" />
            </Svg>
        );
    }
    return (
        <Svg width={size} height={size} viewBox="0 0 24 24" color={color} fill={filled ? 'currentColor' : 'none'} stroke={filled ? 'none' : 'currentColor'} strokeWidth={2} strokeLinecap="round">
            <Path d={ICONS[name] ?? ''} />
        </Svg>
    );
}

/// A button that's only an icon, named for what it does, which is what shows when the pointer's on it.
export function IconButton({ name, icon, onClick, theme, color, disabled, on }: { name: string; icon: string; onClick: () => void; theme: Theme; color?: string; disabled?: boolean; on?: boolean }) {
    return (
        <button name={name} tooltip={name} onClick={onClick} disabled={disabled} width={26} height={24} radius={4} align="center" justify="center" background={on ? theme.chosen : 'transparent'} hover={theme.raised}>
            <Icon name={icon} color={color ?? (on ? theme.accent : theme.dim)} />
        </button>
    );
}

/// A small button with words on it, as a browser's inspector has in its toolbars.
export function Action({ label, onClick, theme, disabled, strong }: { label: string; onClick: () => void; theme: Theme; disabled?: boolean; strong?: boolean }) {
    return (
        <button name={label} onClick={onClick} disabled={disabled} direction="row" paddingX={10} height={24} radius={4} align="center" justify="center" background={strong ? theme.accent : 'transparent'} borderWidth={1} borderColor={strong ? theme.accent : theme.line} hover={strong ? theme.accent : theme.raised}>
            <text fontSize={12} fontFamily={UI} color={strong ? theme.onAccent : theme.accent} noWrap>{label}</text>
        </button>
    );
}

export function Across({ theme }: { theme: Theme }) {
    return <group height={1} background={theme.line} />;
}

export function Down({ theme }: { theme: Theme }) {
    return <group width={1} background={theme.line} />;
}

/// A line between two panes, which can be dragged to make the one it's for wider or narrower.
export function Splitter({ width, setWidth, least, most, theme, fromRight }: { width: number; setWidth: (width: number) => void; least: number; most: number; theme: Theme; fromRight?: boolean }) {
    return (
        <group
            width={5}
            align="center"
            hover={theme.line}
            onDrag={(phase: string, x: number, y: number, dx: number) => {
                if (phase === 'move') {
                    setWidth(Math.max(least, Math.min(most, width + (fromRight ? -dx : dx))));
                }
            }}
        >
            <group width={1} flex={1} background={theme.line} />
        </group>
    );
}

/// A row of tabs across the top of what they're for, the chosen one underlined.
export function TabStrip({ tabs, value, onChange, theme, right }: { tabs: string[]; value: string; onChange: (tab: string) => void; theme: Theme; right?: any }) {
    return (
        <column background={theme.bar}>
            <row height={29} align="center" paddingX={4}>
                <scroll key="tabs" axis="horizontal" flex={1} shrink={1} minWidth={180} height={29}>
                    <row height={29}>
                        {tabs.map((tab) => (
                            <button key={`tab-${tab}`} name={tab} onClick={() => onChange(tab)} direction="column" height={29} paddingX={10} justify="center" hover={theme.hover}>
                                <group flex={1} />
                                <text fontSize={12} fontFamily={UI} color={tab === value ? theme.text : theme.dim} noWrap>{tab}</text>
                                <group flex={1} />
                                <group height={2} width="100%" radius={1} background={tab === value ? theme.accent : 'transparent'} />
                            </button>
                        ))}
                    </row>
                </scroll>
                {right ?? null}
            </row>
            <Across theme={theme} />
        </column>
    );
}

/// A strip of tools at the top of a pane.
export function Toolbar({ theme, children }: { theme: Theme; children?: any }) {
    return (
        <column background={theme.bar}>
            <row height={28} align="center" gap={2} paddingX={4}>
                {children}
            </row>
            <Across theme={theme} />
        </column>
    );
}

/// A part of a sidebar, which can be folded away.
export function Section({ title, open, onToggle, theme, children, right }: { title: string; open: boolean; onToggle: () => void; theme: Theme; children?: any; right?: any }) {
    return (
        <column>
            <row height={24} align="center" background={theme.bar}>
                <button direction="row" justify="start" name={title} onClick={onToggle} flex={1} height={24} align="center" gap={4} paddingX={6} hover={theme.hover}>
                    <Icon name={open ? 'down' : 'right'} color={theme.dim} size={12} />
                    <text fontSize={12} fontFamily={UI} fontWeight="bold" color={theme.text}>{title}</text>
                </button>
                {right ?? null}
            </row>
            <Across theme={theme} />
            {open ? <column paddingY={2}>{children}</column> : null}
        </column>
    );
}

/// What a pane says when it has nothing to show.
export function Empty({ text, theme }: { text: string; theme: Theme }) {
    return (
        <column flex={1} align="center" justify="center" padding={24}>
            <text fontSize={12} fontFamily={UI} color={theme.dim} textAlign="center" maxWidth={420}>{text}</text>
        </column>
    );
}

export function Field({ name, value, onChange, placeholder, theme, width, flex, onKeyPress, mono }: { name: string; value: string; onChange: (value: string) => void; placeholder?: string; theme: Theme; width?: number; flex?: number; onKeyPress?: (key: string) => unknown; mono?: boolean }) {
    return (
        <TextInput
            key={`field-${name}`}
            name={name}
            value={value}
            onChange={onChange}
            placeholder={placeholder}
            width={width}
            flex={flex}
            onKeyPress={onKeyPress}
            fontSize={12}
            fontFamily={mono ? MONO : UI}
            paddingX={6}
            paddingY={3}
            radius={3}
            background={theme.field}
            color={theme.text}
            borderColor={theme.line}
        />
    );
}

/// A table's heading, its columns as wide as `widths` says, with 0 for the one that takes what's left.
export function Heading({ columns, widths, theme, sorted, onSort }: { columns: string[]; widths: number[]; theme: Theme; sorted?: number; onSort?: (column: number) => void }) {
    return (
        <column background={theme.bar}>
            <row height={22} align="center">
                {columns.map((column, index) => (
                    <button direction="row" justify="start" key={`heading-${column}`} name={`Sort by ${column}`} onClick={() => onSort?.(index)} width={widths[index] || undefined} flex={widths[index] ? undefined : 1} height={22} paddingX={8} align="center" hover={onSort ? theme.hover : undefined}>
                        <text fontSize={12} fontFamily={UI} color={theme.text} noWrap ellipsis>{`${column}${sorted === index ? ' ▾' : ''}`}</text>
                    </button>
                ))}
            </row>
            <Across theme={theme} />
        </column>
    );
}

export function Cell({ text, width, theme, color, mono, bold }: { text: string; width: number; theme: Theme; color?: string; mono?: boolean; bold?: boolean }) {
    return (
        <text width={width || undefined} flex={width ? undefined : 1} paddingX={8} fontSize={12} fontFamily={mono ? MONO : UI} fontWeight={bold ? 'bold' : 'normal'} color={color ?? theme.text} noWrap ellipsis selectable>
            {text}
        </text>
    );
}

// Elements.
