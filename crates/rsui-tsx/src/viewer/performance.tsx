// The Performance pane: each frame, and where its time went.
import { useState, useMemo } from 'rsui';
import { BUDGET, MONO, MOST_LISTED, PARTS, ROW, UI, ms, plural, total } from './theme';
import type { Frame, Theme } from './theme';
import { Across, Action, Cell, Empty, Heading, Toolbar } from './ui';

/// Each frame as a bar, its height how long it took and split into where the time went, with a line at the time there is
/// for one. The newest are on the right.
export function FrameStrip({ frames, chosen, choose, theme }: { frames: Frame[]; chosen: number | undefined; choose: (number: number) => void; theme: Theme }) {
    const height = 80;
    // A frame much longer than the rest, as one the app was stopped in, doesn't flatten the rest: it's cut off.
    const tallest = Math.min(BUDGET * 6, Math.max(BUDGET * 2, ...frames.map(total)));
    const scale = (height - 6) / tallest;
    return (
        <column background={theme.page}>
            <row height={height} align="end" paddingX={8} gap={1}>
                {frames.map((frame) => (
                    <button key={`bar-${frame.number}`} name={`Frame ${frame.number}`} onClick={() => choose(frame.number)} width={6} height={height} direction="column" justify="end" background={frame.number === chosen ? theme.chosen : 'transparent'} hover={theme.hover}>
                        {PARTS.slice()
                            .reverse()
                            .map(([part, , color]) => {
                                const tall = Math.min(height, Math.max(0, (frame[part] as number) * scale));
                                return tall >= 0.5 ? <group key={`${part}`} width={6} height={tall} background={theme[color]} /> : null;
                            })}
                        {total(frame) * scale < 1 ? <group width={6} height={1} background={theme.strong} /> : null}
                    </button>
                ))}
                <group flex={1} />
            </row>
            <group position="absolute" left={0} right={0} top={height - BUDGET * scale} height={1} background={theme.error} opacity={0.6} />
            <Across theme={theme} />
        </column>
    );
}

export function FrameDetail({ frame, theme }: { frame: Frame; theme: Theme }) {
    const took = total(frame);
    return (
        <column gap={12} padding={14}>
            <row gap={10} align="center">
                <text fontSize={14} fontFamily={UI} fontWeight="bold" color={theme.text}>{`Frame ${frame.number}`}</text>
                <text fontSize={12} fontFamily={UI} color={theme.dim}>{`at ${frame.at < 1000 ? `${Math.round(frame.at)} ms` : `${(frame.at / 1000).toFixed(2)} s`}`}</text>
                <text fontSize={12} fontFamily={UI} color={took > BUDGET ? theme.error : theme.ok}>{`${ms(took)}${took > BUDGET ? `, over the ${BUDGET.toFixed(1)} ms there is` : ''}`}</text>
            </row>
            <row height={10} radius={2} width={400} background={theme.bar}>
                {PARTS.map(([part, , color]) => {
                    const share = took > 0 ? (frame[part] as number) / took : 0;
                    return share > 0 ? <group key={`share-${part}`} width={Math.max(1, share * 400)} height={10} background={theme[color]} /> : null;
                })}
            </row>
            <row gap={18}>
                {PARTS.map(([part, label, color]) => (
                    <row key={`legend-${part}`} gap={6} align="center">
                        <group width={10} height={10} radius={2} background={theme[color]} />
                        <text fontSize={12} fontFamily={UI} color={theme.text}>{`${label} ${ms(frame[part] as number)}`}</text>
                    </row>
                ))}
            </row>
            {(frame.present ?? 0) >= 0.1 ? <text fontSize={12} fontFamily={UI} color={theme.dim}>{`Then ${ms(frame.present ?? 0)} waiting for the screen, which isn't counted.`}</text> : null}
            {frame.handled.length > 0 ? (
                <column gap={4}>
                    <text fontSize={11} fontFamily={UI} fontWeight="bold" color={theme.dim}>WHAT CAME IN</text>
                    {frame.handled.map((handled, index) => (
                        <text key={`handled-${index}`} fontSize={12} fontFamily={UI} color={theme.text} selectable>
                            {`${handled.action} on #${handled.key} ran ${handled.note ? handled.note + ' ' : ''}for ${ms(handled.took)}${handled.rejected ? ', and was turned down' : ''}`}
                        </text>
                    ))}
                </column>
            ) : null}
            <column>
                <Heading columns={['Component', 'Self time', 'Total time', 'Why']} widths={[220, 90, 90, 0]} theme={theme} />
                {frame.renders.length === 0 ? <text fontSize={12} fontFamily={UI} color={theme.dim} padding={8}>Nothing rendered.</text> : null}
                {frame.renders.map((render, index) => (
                    <column key={`render-${index}`}>
                        <row height={ROW + 2} align="center" hover={theme.hover}>
                            <row width={220} align="center" paddingLeft={8 + Math.min(render.depth, 8) * 10} gap={6}>
                                <group width={8} height={8} radius={4} background={render.changed ? theme.accent : theme.warn} />
                                <text flex={1} fontSize={12} fontFamily={MONO} color={theme.fn} noWrap ellipsis>{render.name ?? 'a component'}</text>
                            </row>
                            <Cell text={ms(render.own)} width={90} theme={theme} />
                            <Cell text={ms(render.took)} width={90} theme={theme} />
                            <Cell text={render.changed ? render.why ?? render.cause : `changed nothing. ${render.why ?? ''}`} width={0} theme={theme} color={render.changed ? theme.text : theme.warn} />
                        </row>
                        <Across theme={theme} />
                    </column>
                ))}
            </column>
        </column>
    );
}

export function Performance({ frames, live, summary, theme }: { frames: Frame[]; live: boolean; summary: () => Promise<string>; theme: Theme }) {
    const busy = useMemo(() => frames.filter((frame) => frame.renders.length > 0 || frame.handled.length > 0), [frames]);
    const [chosen, setChosen] = useState<number | null>(live ? null : busy.length > 0 ? busy[busy.length - 1].number : null);
    const [text, setText] = useState<string | null>(null);
    const shown = chosen ?? busy[busy.length - 1]?.number;
    const frame = busy.find((one) => one.number === shown);
    const renders = frames.reduce((sum, one) => sum + one.renders.length, 0);
    const wasted = frames.reduce((sum, one) => sum + one.renders.filter((render) => !render.changed).length, 0);
    const over = frames.filter((one) => total(one) > BUDGET).length;
    return (
        <column flex={1}>
            <Toolbar theme={theme}>
                <text fontSize={12} fontFamily={UI} color={theme.dim} paddingX={6}>
                    {`${plural(frames.length, 'frame')} · ${plural(renders, 'render')} · ${wasted} changing nothing · ${over} over budget`}
                </text>
                <group flex={1} />
                {live && chosen !== null ? <Action label="Show the newest" onClick={() => setChosen(null)} theme={theme} /> : null}
                <Action label={text === null ? 'Summary' : 'Frames'} onClick={async () => setText(text === null ? await summary() : null)} theme={theme} />
            </Toolbar>
            {text !== null ? (
                <scroll key="summary" flex={1} padding={12}>
                    <text fontSize={12} fontFamily={MONO} color={theme.text} selectable>{text || 'Working it out.'}</text>
                </scroll>
            ) : (
                <column flex={1}>
                    <FrameStrip frames={busy.slice(-MOST_LISTED)} chosen={shown} choose={setChosen} theme={theme} />
                    <scroll key="frame" flex={1}>
                        {frame ? <FrameDetail frame={frame} theme={theme} /> : <Empty text={live ? 'Nothing has happened yet. Use the app, and each frame where something did shows here.' : 'Nothing happened in what was kept.'} theme={theme} />}
                    </scroll>
                </column>
            )}
        </column>
    );
}
