// The Rendering pane: the components that rendered, and why, and what that took.
import { useState, useMemo } from 'rsui';
import { ROW, UI, ms } from './theme';
import type { Frame, Theme } from './theme';
import { Across, Cell, Empty, Heading } from './ui';

/// Every component that rendered, with how long it took and how often it rendered for nothing, as a profiler shows it.
export function Rendering({ frames, theme }: { frames: Frame[]; theme: Theme }) {
    const [sorted, setSorted] = useState(1);
    const rows = useMemo(() => {
        const by = new Map<string, { renders: number; wasted: number; own: number; took: number }>();
        for (const frame of frames) {
            for (const render of frame.renders) {
                const name = render.name ?? 'a component';
                const kept = by.get(name) ?? { renders: 0, wasted: 0, own: 0, took: 0 };
                kept.renders += 1;
                kept.wasted += render.changed ? 0 : 1;
                kept.own += render.own;
                kept.took += render.took;
                by.set(name, kept);
            }
        }
        return [...by.entries()];
    }, [frames]);
    const keyOf = (row: [string, { renders: number; wasted: number; own: number; took: number }]) => [0, row[1].own, row[1].took, row[1].renders, row[1].wasted][sorted];
    const ordered = sorted === 0 ? [...rows].sort((a, b) => a[0].localeCompare(b[0])) : [...rows].sort((a, b) => keyOf(b) - keyOf(a));
    const most = Math.max(0.001, ...rows.map((row) => row[1].own));
    return (
        <column flex={1}>
            <Heading columns={['Component', 'Self time', 'Total time', 'Renders', 'Changing nothing']} widths={[0, 160, 110, 90, 130]} theme={theme} sorted={sorted} onSort={setSorted} />
            <scroll key="components" flex={1}>
                {ordered.length === 0 ? <Empty text="Nothing has rendered yet." theme={theme} /> : null}
                {ordered.map(([name, kept]) => (
                    <column key={`component-${name}`}>
                        <row height={ROW + 2} align="center" hover={theme.hover}>
                            <Cell text={name} width={0} theme={theme} color={theme.fn} mono />
                            <row width={160} align="center" paddingX={8} gap={6}>
                                <text width={64} fontSize={12} fontFamily={UI} color={theme.text} noWrap>{ms(kept.own)}</text>
                                <group width={Math.max(1, (kept.own / most) * 70)} height={8} radius={2} background={theme.script} />
                            </row>
                            <Cell text={ms(kept.took)} width={110} theme={theme} />
                            <Cell text={String(kept.renders)} width={90} theme={theme} />
                            <Cell text={String(kept.wasted)} width={130} theme={theme} color={kept.wasted > 0 ? theme.warn : theme.dim} />
                        </row>
                        <Across theme={theme} />
                    </column>
                ))}
            </scroll>
        </column>
    );
}
