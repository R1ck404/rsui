// The Replay pane: what came in to an app, to start it again from any of that.
import { useState, useEffect } from 'rsui';
import { MOST_LISTED, ROW, UI } from './theme';
import type { Ask, Input, Theme } from './theme';
import { Across, Action, Cell, Down, Empty, Heading, IconButton, Toolbar } from './ui';

/// The inputs that are worth starting a replay at: those that do something, rather than the pointer only moving.
export const DOING = ['down', 'up', 'buttondown', 'buttonup', 'key', 'text', 'composition', 'scroll', 'files', 'drag', 'access', 'close'];

/// What's come in to the app, each of which it can be started again from, replaying everything before it.
export function Replay({ count, ask, onStarted, theme }: { count: number; ask: Ask; onStarted: (pid: number) => void; theme: Theme }) {
    const [inputs, setInputs] = useState<Input[]>([]);
    const [all, setAll] = useState(false);
    const [said, setSaid] = useState<string | null>(null);
    useEffect(() => {
        let gone = false;
        ask({ ask: 'inputs', after: inputs.length }).then((answer) => {
            if (!gone && !answer.error && answer.inputs.length > 0) {
                setInputs([...inputs, ...answer.inputs]);
            }
        });
        return () => {
            gone = true;
        };
    }, [count]);
    const again = async (to: number, pause: boolean) => {
        const answer = await ask({ ask: 'replay', to, pause });
        setSaid(answer.error ?? answer.said ?? null);
        if (answer.pid) {
            onStarted(answer.pid);
        }
    };
    const shown = (all ? inputs : inputs.filter((one) => DOING.includes(one.kind))).slice(-MOST_LISTED);
    return (
        <column flex={1}>
            <Toolbar theme={theme}>
                <IconButton name={all ? 'Only what does something' : 'Show moves too'} icon="eye" on={all} onClick={() => setAll(!all)} theme={theme} />
                <Down theme={theme} />
                <text flex={1} fontSize={12} fontFamily={UI} color={said ? theme.text : theme.dim} paddingX={6} noWrap ellipsis selectable>
                    {said ?? 'Starts the app again in a window of its own, giving it everything up to the input chosen at the times it came, with the same random numbers.'}
                </text>
            </Toolbar>
            <Heading columns={['#', 'Time', 'Input', '']} widths={[60, 80, 0, 330]} theme={theme} />
            <scroll key="inputs" flex={1}>
                {shown.length === 0 ? <Empty text="Nothing has come in yet. Use the app, and what you do shows here." theme={theme} /> : null}
                {shown.map((one) => (
                    <column key={`input-${one.number}`}>
                        <row height={ROW + 8} align="center" hover={theme.hover}>
                            <Cell text={`${one.number}`} width={60} theme={theme} color={theme.dim} />
                            <Cell text={`${(one.at / 1000).toFixed(2)} s`} width={80} theme={theme} color={theme.dim} />
                            <Cell text={one.said} width={0} theme={theme} />
                            <row width={330} justify="end" gap={6} paddingRight={8}>
                                <Action label="Replay to here" onClick={() => again(one.number, false)} theme={theme} />
                                <Action label="Replay and stop in it" onClick={() => again(one.number, true)} theme={theme} strong />
                            </row>
                        </row>
                        <Across theme={theme} />
                    </column>
                ))}
            </scroll>
        </column>
    );
}
