// What's around the inspector's tabs: the status bar along its bottom, with how the app's doing now, and the command palette,
// ctrl+shift+P, which finds any tab or action by typing part of its name.
import { useState, useMemo } from 'rsui';
import { Spans, NS, formatNs } from './spans';
import { Theme, UI, MONO } from './theme';
import { Field, Across } from './ui';

/// How the app's doing now, from the frames of the last second.
export function StatusBar({ spans, status, said, theme, onOpenPalette }: { spans: Spans; status: string; said: string | null; theme: Theme; onOpenPalette: () => void }) {
    const now = useMemo(() => {
        const frames = spans.frames;
        const last = frames[frames.length - 1];
        if (!last) {
            return null;
        }
        const recent = frames.filter((frame) => last.end - frame.end < 1000 * NS);
        const worst = recent.reduce((most, frame) => Math.max(most, frame.end - frame.start), 0);
        const slow = recent.filter((frame) => frame.end - frame.start > 16.667 * NS).length;
        return { took: last.end - last.start, fps: recent.length, worst, slow, idle: spans.now - last.end > 1000 * NS };
    }, [spans.version]);
    const color = (took: number) => (took > 16.667 * NS ? theme.error : took > 8.333 * NS ? theme.warn : theme.ok);
    return (
        <column background={theme.bar}>
            <Across theme={theme} />
            <row height={22} align="center" paddingX={8} gap={14}>
                <text fontSize={11} fontFamily={UI} color={theme.dim} noWrap ellipsis shrink={1} minWidth={0}>{said ?? status}</text>
                <group flex={1} />
                {spans.recording ? (
                    <row gap={5} align="center">
                        <group width={7} height={7} radius={4} background={theme.error} />
                        <text fontSize={11} fontFamily={UI} color={theme.error}>Recording JavaScript</text>
                    </row>
                ) : null}
                {now ? (
                    <row gap={5} align="center">
                        <group width={7} height={7} radius={4} background={color(now.took)} />
                        <text fontSize={11} fontFamily={MONO} color={theme.text}>{`last frame ${formatNs(now.took)}`}</text>
                    </row>
                ) : null}
                {now && !now.idle ? <text fontSize={11} fontFamily={MONO} color={theme.dim}>{`${now.fps} fps`}</text> : null}
                {now && now.slow > 0 && !now.idle ? <text fontSize={11} fontFamily={MONO} color={theme.error}>{`${now.slow} slow in the last second`}</text> : null}
                {now?.idle ? <text fontSize={11} fontFamily={UI} color={theme.dim}>idle</text> : null}
                <text fontSize={11} fontFamily={UI} color={theme.dim}>{spans.level >= 2 ? 'full detail' : spans.level === 1 ? 'light detail' : ''}</text>
                <button name="Command palette (ctrl+shift+P)" tooltip="Command palette (ctrl+shift+P)" onClick={onOpenPalette} height={18} paddingX={6} radius={3} direction="row" align="center" hover={theme.raised}>
                    <text fontSize={11} fontFamily={UI} color={theme.accent}>⌘ commands</text>
                </button>
            </row>
        </column>
    );
}

export type Command = { name: string; hint?: string; run: () => void };

/// A list of everything that can be done, found by typing any part of what it's called, as an editor's command palette.
export function Palette({ commands, close, theme }: { commands: Command[]; close: () => void; theme: Theme }) {
    const [typed, setTyped] = useState('');
    const [at, setAt] = useState(0);
    const found = useMemo(() => {
        const words = typed.toLowerCase().split(/\s+/).filter(Boolean);
        return commands.filter((command) => words.every((word) => command.name.toLowerCase().includes(word) || (command.hint ?? '').toLowerCase().includes(word)));
    }, [typed, commands]);
    const chosen = Math.min(at, Math.max(0, found.length - 1));
    const run = (command: Command | undefined) => {
        if (command) {
            close();
            command.run();
        }
    };
    return (
        <column position="absolute" left={0} right={0} top={0} bottom={0} background="#00000055" align="center" paddingTop={60} onClick={close}>
            <column width={520} background={theme.raised} radius={8} borderWidth={1} borderColor={theme.line} padding={8} gap={6} onClick={(event: any) => event?.stopPropagation?.()}>
                <Field
                    name="Command"
                    value={typed}
                    onChange={(value) => {
                        setTyped(value);
                        setAt(0);
                    }}
                    placeholder="Type a command, a tab, or an action"
                    theme={theme}
                    onKeyPress={(key) => {
                        if (key === 'enter') {
                            run(found[chosen]);
                            return true;
                        }
                        if (key === 'escape') {
                            close();
                            return true;
                        }
                        if (key === 'down' || key === 'up') {
                            setAt(Math.max(0, Math.min(found.length - 1, chosen + (key === 'down' ? 1 : -1))));
                            return true;
                        }
                        return false;
                    }}
                />
                <scroll key="commands" maxHeight={360}>
                    {found.length === 0 ? <text fontSize={12} fontFamily={UI} color={theme.dim} padding={8}>Nothing's called that.</text> : null}
                    {found.slice(0, 40).map((command, index) => (
                        <button key={`command-${command.name}`} name={command.name} onClick={() => run(command)} height={26} paddingX={8} radius={4} direction="row" align="center" background={index === chosen ? theme.selected : 'transparent'} hover={theme.hover}>
                            <text flex={1} fontSize={12} fontFamily={UI} color={theme.text} noWrap ellipsis>{command.name}</text>
                            {command.hint ? <text fontSize={11} fontFamily={UI} color={theme.dim} noWrap>{command.hint}</text> : null}
                        </button>
                    ))}
                </scroll>
            </column>
        </column>
    );
}
