// The Console pane: what an app says, and what's run in it.
import { useState, TextInput } from 'rsui';
import { MONO, MOST_LISTED, UI } from './theme';
import type { Heard, Stopped, Theme } from './theme';
import { Down, Field, IconButton, Toolbar } from './ui';

export type Worked = { after: number; input: string; output: string; threw: boolean };

/// What a line that's been said is, for how it's shown.
export function kindOf(line: string): string {
    if (line.startsWith('logpoint ')) {
        return 'logpoint';
    }
    if (/^(breakpoint|paused|step) at /.test(line)) {
        return 'debugger';
    }
    if (/^(Uncaught|error|Error|TypeError|ReferenceError|RangeError|SyntaxError)\b/.test(line) || line.includes(' threw ')) {
        return 'error';
    }
    if (/^(warning|Warning|rsui\/web:)/.test(line)) {
        return 'warn';
    }
    return 'log';
}

export function ConsoleLine({ text, kind, theme, prefix }: { text: string; kind: string; theme: Theme; prefix?: string }) {
    const back = kind === 'error' ? theme.errorBack : kind === 'warn' ? theme.warnBack : 'transparent';
    const color = kind === 'error' ? theme.error : kind === 'warn' ? theme.warn : kind === 'logpoint' ? theme.logpoint : kind === 'debugger' ? theme.accent : kind === 'input' ? theme.accent : theme.text;
    return (
        <column background={back}>
            <row paddingX={8} paddingY={3} gap={6}>
                <text width={12} fontSize={12} fontFamily={MONO} color={theme.dim}>{prefix ?? ''}</text>
                <text flex={1} fontSize={12} fontFamily={MONO} color={color} selectable>{text}</text>
            </row>
            <group height={1} background={kind === 'error' || kind === 'warn' ? back : theme.line} />
        </column>
    );
}

export function Console({ heard, worked, run, clear, cleared, stopped, theme }: { heard: Heard[]; worked: Worked[]; run: (expression: string) => void; clear: () => void; cleared: number; stopped: Stopped | null; theme: Theme }) {
    const [typed, setTyped] = useState('');
    const [filter, setFilter] = useState('');
    const [history, setHistory] = useState<string[]>([]);
    const [back, setBack] = useState(-1);
    const shown = heard.filter((one) => one.number > cleared && (filter === '' || one.line.toLowerCase().includes(filter.toLowerCase()))).slice(-MOST_LISTED);
    const items: any[] = [];
    const evaluations = worked.filter((one) => one.after >= cleared);
    let next = 0;
    const evaluated = (upTo: number) => {
        while (next < evaluations.length && evaluations[next].after <= upTo) {
            const one = evaluations[next];
            items.push(<ConsoleLine key={`in-${next}`} text={one.input} kind="input" prefix="›" theme={theme} />);
            items.push(<ConsoleLine key={`out-${next}`} text={one.output} kind={one.threw ? 'error' : 'log'} prefix="‹" theme={theme} />);
            next += 1;
        }
    };
    for (const one of shown) {
        evaluated(one.number - 1);
        items.push(<ConsoleLine key={`heard-${one.number}`} text={one.line} kind={kindOf(one.line)} theme={theme} />);
    }
    evaluated(Number.MAX_SAFE_INTEGER);
    const submit = () => {
        const expression = typed.trim();
        if (expression === '') {
            return;
        }
        run(expression);
        setHistory([...history, expression].slice(-50));
        setBack(-1);
        setTyped('');
    };
    return (
        <column flex={1}>
            <Toolbar theme={theme}>
                <IconButton name="Clear the console" icon="clear" onClick={clear} theme={theme} />
                <Down theme={theme} />
                <group width={4} />
                <Field name="Filter" value={filter} onChange={setFilter} placeholder="Filter" width={220} theme={theme} />
                <group flex={1} />
                <text fontSize={12} fontFamily={UI} color={stopped ? theme.paused : theme.dim} paddingRight={8}>
                    {stopped ? `Working things out in ${stopped.frames[0]?.function || 'the function'} at ${stopped.at}` : 'Working things out at the top of the app'}
                </text>
            </Toolbar>
            <scroll key="console" flex={1}>
                {items.length === 0 ? <text fontSize={12} fontFamily={UI} color={theme.dim} padding={10}>What console.log and logpoints say shows here.</text> : null}
                {items}
                <row paddingX={8} paddingY={4} gap={6} align="center">
                    <text width={12} fontSize={13} fontFamily={MONO} color={theme.accent}>›</text>
                    <TextInput
                        key="console-prompt"
                        name="Console prompt"
                        flex={1}
                        value={typed}
                        onChange={setTyped}
                        onKeyPress={(key: string) => {
                            if (key === 'enter') {
                                submit();
                                return true;
                            }
                            if (key === 'up' && history.length > 0) {
                                const at = back < 0 ? history.length - 1 : Math.max(0, back - 1);
                                setBack(at);
                                setTyped(history[at]);
                                return true;
                            }
                            if (key === 'down' && back >= 0) {
                                const at = back + 1;
                                setBack(at >= history.length ? -1 : at);
                                setTyped(at >= history.length ? '' : history[at]);
                                return true;
                            }
                            return false;
                        }}
                        fontSize={12}
                        fontFamily={MONO}
                        paddingX={0}
                        paddingY={2}
                        borderWidth={0}
                        background="transparent"
                        color={theme.text}
                    />
                </row>
            </scroll>
        </column>
    );
}

// Sources.
