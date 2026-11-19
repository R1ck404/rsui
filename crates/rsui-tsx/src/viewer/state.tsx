// The State pane: every change of state, to go back to any of them.
import { ROW, UI, plural } from './theme';
import type { Change, Theme } from './theme';
import { Across, Action, Cell, Empty, Heading, Toolbar } from './ui';

/// Every change of state kept. Looking at an app, each can be gone back to, and what's after where things are now is greyed,
/// as it'll be gone through again going forward, or replaced by what changes next.
export function History({ history, now, goTo, said, theme }: { history: Change[]; now: number | null; goTo: ((change: number) => void) | null; said: string | null; theme: Theme }) {
    const oldest = history[0]?.number ?? 1;
    const row = (number: number, component: string, change: string, ahead: boolean, key: string) => (
        <column key={key}>
            <row height={ROW + 6} align="center" hover={theme.hover} background={now === number ? theme.chosen : 'transparent'}>
                <Cell text={`${number}`} width={60} theme={theme} color={theme.dim} />
                <Cell text={component} width={180} theme={theme} color={ahead ? theme.dim : theme.fn} mono />
                <Cell text={change} width={0} theme={theme} color={ahead ? theme.dim : theme.text} mono />
                <row width={170} justify="end" paddingRight={8}>
                    {goTo ? now === number ? <text fontSize={12} fontFamily={UI} color={theme.ok} paddingRight={6}>now</text> : <Action label={ahead ? 'Go forward to here' : 'Go back to here'} onClick={() => goTo(number)} theme={theme} /> : null}
                </row>
            </row>
            <Across theme={theme} />
        </column>
    );
    return (
        <column flex={1}>
            <Toolbar theme={theme}>
                <text fontSize={12} fontFamily={UI} color={theme.dim} paddingX={6}>{plural(history.length, 'change') + ' of state'}</text>
                <group flex={1} />
                {said ? <text fontSize={12} fontFamily={UI} color={theme.text} paddingRight={8} selectable>{said}</text> : null}
            </Toolbar>
            <Heading columns={['#', 'Component', 'Change', '']} widths={[60, 180, 0, 170]} theme={theme} />
            <scroll key="history" flex={1}>
                {history.length === 0 ? <Empty text={goTo ? 'No state has changed yet. Use the app, and each change shows here.' : 'No state changed in what was kept.'} theme={theme} /> : null}
                {goTo && history.length > 0 ? row(oldest - 1, '', oldest > 1 ? 'Before the oldest change kept' : 'Before anything changed', false, 'change-start') : null}
                {history.map((change) => row(change.number, change.component, change.said, now !== null && change.number > now, `change-${change.number}`))}
            </scroll>
        </column>
    );
}
