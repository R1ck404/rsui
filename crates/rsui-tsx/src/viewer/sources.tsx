// The Sources pane: an app's code, with breakpoints and logpoints, and where it stopped.
import { useState, useMemo, useEffect, useCallback, TextInput, focus } from 'rsui';
import { readFileSync, readdirSync } from 'fs';
import { MONO, ROW, UI } from './theme';
import type { Ask, Breakpoint, Logpoint, Running, Stopped, Theme } from './theme';
import { Across, Field, Icon, IconButton, Section, Splitter, Toolbar } from './ui';
import type { News } from './main';

export const KEYWORDS = new Set(
    'const let var function return if else for while do switch case break continue new class extends import export from default async await try catch finally throw typeof instanceof in of this null undefined true false yield delete void super static get set as type interface'.split(' '),
);

/// A line of code in pieces, each with what it is, carrying on from a comment the line before left open.
export function tokens(line: string, inComment: boolean): { pieces: [string, string][]; inComment: boolean } {
    const pieces: [string, string][] = [];
    let at = 0;
    const push = (kind: string, text: string) => {
        if (text !== '') {
            pieces.push([kind, text]);
        }
    };
    while (at < line.length) {
        if (inComment) {
            const end = line.indexOf('*/', at);
            const stop = end < 0 ? line.length : end + 2;
            push('comment', line.slice(at, stop));
            at = stop;
            inComment = end < 0;
            continue;
        }
        const rest = line.slice(at);
        if (rest.startsWith('//')) {
            push('comment', rest);
            break;
        }
        if (rest.startsWith('/*')) {
            inComment = true;
            continue;
        }
        const c = line[at];
        if (c === '"' || c === "'" || c === '`') {
            let end = at + 1;
            while (end < line.length && line[end] !== c) {
                end += line[end] === '\\' ? 2 : 1;
            }
            push('str', line.slice(at, end + 1));
            at = end + 1;
            continue;
        }
        if (/[0-9]/.test(c) && (at === 0 || !/[\w$]/.test(line[at - 1]))) {
            const match = /^[0-9][0-9_.xXa-fA-FeEn]*/.exec(rest);
            push('num', match ? match[0] : c);
            at += match ? match[0].length : 1;
            continue;
        }
        if (/[A-Za-z_$]/.test(c)) {
            const match = /^[\w$]+/.exec(rest);
            const word = match ? match[0] : c;
            const before = line.slice(0, at).trimEnd();
            const kind = KEYWORDS.has(word) ? 'kw' : before.endsWith('<') || before.endsWith('</') ? 'tag' : rest.slice(word.length).trimStart().startsWith('(') || /^[A-Z]/.test(word) ? 'fn' : 'plain';
            push(kind, word);
            at += word.length;
            continue;
        }
        const match = /^[^\w$"'`/0-9]+|^\//.exec(rest);
        const text = match ? match[0] : c;
        push('plain', text);
        at += text.length;
    }
    return { pieces, inComment };
}

/// The app's own source files, from its folder in, but not what's in packages or what's built.
export function sourceFiles(folder: string): string[] {
    const found: string[] = [];
    const walk = (dir: string, from: string, depth: number) => {
        let entries: any[] = [];
        try {
            entries = readdirSync(dir, { withFileTypes: true });
        } catch {
            return;
        }
        for (const entry of entries) {
            const name = entry.name;
            if (name.startsWith('.') || name === 'node_modules' || name === 'target' || name === 'dist' || name === 'build') {
                continue;
            }
            if (entry.isDirectory()) {
                if (depth < 4) {
                    walk(`${dir}\\${name}`, `${from}${name}/`, depth + 1);
                }
            } else if (/\.(tsx|ts|jsx|js|mjs|cjs|css)$/.test(name)) {
                found.push(`${from}${name}`);
            }
            if (found.length >= 400) {
                return;
            }
        }
    };
    walk(folder, '', 0);
    return found.sort((a, b) => (a.includes('/') === b.includes('/') ? a.localeCompare(b) : a.includes('/') ? 1 : -1));
}

export function placeOf(at: string): [string, number] {
    const cut = at.lastIndexOf(':');
    return [at.slice(0, cut), Number(at.slice(cut + 1))];
}

/// What each kind of piece of code is colored.
export function colorOf(kind: string, theme: Theme): string {
    return kind === 'kw' ? theme.kw : kind === 'str' ? theme.str : kind === 'num' ? theme.num : kind === 'comment' ? theme.comment : kind === 'fn' ? theme.fn : kind === 'tag' ? theme.tag : theme.text;
}

/// A line of code, with its number, which is where a breakpoint goes.
export function CodeLine({ number, pieces, marked, logs, here, gutter, toggle, menu, theme }: { number: number; pieces: [string, string][]; marked: boolean; logs: boolean; here: boolean; gutter: number; toggle: (line: number, marked: boolean) => void; menu: (line: number) => void; theme: Theme }) {
    return (
        <row height={18} align="center" background={here ? theme.exec : 'transparent'}>
            <button
                key={`src-line-${number}`}
                name={`Line ${number}`}
                tooltip={marked ? 'Take the breakpoint away' : 'Stop here. Right-click for a logpoint'}
                onClick={() => toggle(number, marked)}
                onContextMenu={() => menu(number)}
                direction="row"
                width={gutter}
                height={18}
                align="center"
                justify="end"
                paddingRight={8}
                hover={theme.hover}
            >
                <text fontSize={12} fontFamily={MONO} paddingX={4} radius={2} background={marked ? theme.breakpoint : logs ? theme.logpoint : 'transparent'} color={marked || logs ? '#ffffff' : theme.dim}>
                    {String(number)}
                </text>
            </button>
            <text fontSize={12} fontFamily={MONO} noWrap paddingLeft={4}>
                {pieces.length === 0 ? ' ' : null}
                {pieces.map(([kind, piece], at) => (
                    <span key={`piece-${at}`} color={colorOf(kind, theme)}>{piece}</span>
                ))}
            </text>
        </row>
    );
}

export function CodeView({ file, folder, breakpoints, logpoints, stopped, frame, ask, onSaid, theme }: { file: string; folder: string; breakpoints: Breakpoint[]; logpoints: Logpoint[]; stopped: Stopped | null; frame: number; ask: Ask; onSaid: (said: string | null) => void; theme: Theme }) {
    const [editing, setEditing] = useState<number | null>(null);
    const [expression, setExpression] = useState('');
    const text = useMemo(() => {
        try {
            return readFileSync(`${folder}\\${file.replace(/\//g, '\\')}`, 'utf8') as string;
        } catch (error) {
            return `// ${file} couldn't be read: ${error}`;
        }
    }, [file, folder, stopped?.at]);
    const lines = useMemo(() => text.replace(/\r/g, '').split('\n'), [text]);
    const coloured = useMemo(() => {
        let inComment = false;
        return lines.map((line) => {
            const done = tokens(line, inComment);
            inComment = done.inComment;
            return done.pieces;
        });
    }, [lines]);
    const marks = new Set(breakpoints.map((one) => one.at));
    const logged = new Set(logpoints.map((one) => one.at));
    const chosenFrame = stopped?.frames[frame];
    const [stoppedFile, stoppedLine] = chosenFrame ? placeOf(chosenFrame.at) : ['', 0];
    const here = stoppedFile === file ? stoppedLine : 0;
    // The line it's stopped at is brought into view, with some of what's around it.
    useEffect(() => {
        if (here > 0) {
            focus(`src-line-${Math.max(1, here - 6)}`);
            focus(`src-line-${Math.min(lines.length, here + 8)}`);
        }
    }, [here, file, stopped?.at]);
    const toggle = useCallback(
        async (line: number, marked: boolean) => {
            const at = `${file}:${line}`;
            const answer = await ask(marked ? { ask: 'breakpoint', at, take: true } : { ask: 'breakpoint', at });
            onSaid(answer.error ?? null);
        },
        [file],
    );
    const menu = useCallback((line: number) => {
        setEditing((was: number | null) => (was === line ? null : line));
        setExpression('');
    }, []);
    const addLogpoint = async (line: number) => {
        const at = `${file}:${line}`;
        const answer = await ask({ ask: 'logpoint', at, expression: expression.trim() });
        onSaid(answer.error ?? null);
        if (!answer.error) {
            setEditing(null);
            setExpression('');
        }
    };
    const gutter = String(lines.length).length * 8 + 22;
    return (
        <scroll key={`code-${file}`} flex={1} axis="both" paddingY={4}>
            {lines.map((line, index) => {
                const number = index + 1;
                const at = `${file}:${number}`;
                return [
                    <CodeLine key={`line-${number}`} number={number} pieces={coloured[index]} marked={marks.has(at)} logs={logged.has(at)} here={number === here} gutter={gutter} toggle={toggle} menu={menu} theme={theme} />,
                    editing === number ? (
                            <row key={`logpoint-row-${number}`} paddingLeft={gutter} paddingY={4} paddingRight={8} gap={6} align="center" background={theme.bar}>
                                <text fontSize={12} fontFamily={UI} color={theme.logpoint}>Logpoint</text>
                                <TextInput
                                    key={`logpoint-${number}`}
                                    name="Logpoint expression"
                                    autoFocus
                                    flex={1}
                                    value={expression}
                                    onChange={setExpression}
                                    placeholder="What to say each time this line runs, as items.length"
                                    onKeyPress={(key: string) => {
                                        if (key === 'enter' && expression.trim() !== '') {
                                            addLogpoint(number);
                                            return true;
                                        }
                                        if (key === 'escape') {
                                            setEditing(null);
                                            return true;
                                        }
                                        return false;
                                    }}
                                    fontSize={12}
                                    fontFamily={MONO}
                                    paddingX={6}
                                    paddingY={3}
                                    radius={3}
                                    background={theme.field}
                                    color={theme.text}
                                    borderColor={theme.logpoint}
                                />
                            </row>
                        ) : null,
                ];
            })}
        </scroll>
    );
}

export function Sources({ app, news, ask, frame, setFrame, wanted, theme, gone }: { app: Running; news: News; ask: Ask; frame: number; setFrame: (frame: number) => void; wanted: string | null; theme: Theme; gone: boolean }) {
    const folder = app.file.slice(0, app.file.lastIndexOf('\\'));
    const main = app.file.slice(folder.length + 1);
    const files = useMemo(() => sourceFiles(folder), [folder]);
    const [file, setFile] = useState(wanted ?? main);
    const [left, setLeft] = useState(220);
    const [right, setRight] = useState(320);
    const [said, setSaid] = useState<string | null>(null);
    const [open, setOpen] = useState<Record<string, boolean>>({ watch: true, breakpoints: true, scope: true, stack: true });
    const [watches, setWatches] = useState<string[]>([]);
    const [watching, setWatching] = useState('');
    const [watched, setWatched] = useState<Record<string, { value: string; threw: boolean }>>({});
    const stopped = gone ? null : news.paused;
    useEffect(() => {
        if (wanted) {
            setFile(wanted);
        }
    }, [wanted]);
    // What's watched is worked out again each time it stops, in the function chosen.
    useEffect(() => {
        if (!stopped) {
            return;
        }
        Promise.all(watches.map((one) => ask({ ask: 'evaluate', frame, expression: one }))).then((answers) => {
            const next: Record<string, { value: string; threw: boolean }> = {};
            answers.forEach((answer, index) => {
                next[watches[index]] = { value: String(answer.error ?? answer.value ?? ''), threw: Boolean(answer.error || answer.threw) };
            });
            setWatched(next);
        });
    }, [stopped?.at, frame, watches.join('\n')]);
    const toggle = (name: string) => setOpen({ ...open, [name]: !open[name] });
    const resume = async (how: string) => {
        const answer = await ask({ ask: 'resume', how });
        setSaid(answer.error ?? null);
    };
    const chosen = stopped?.frames[Math.min(frame, (stopped?.frames.length ?? 1) - 1)];
    const reason = stopped ? (stopped.reason === 'breakpoint' ? 'Paused on breakpoint' : stopped.reason === 'step' ? 'Paused after a step' : 'Paused') : null;
    const note = (text: string) => <text fontSize={12} fontFamily={UI} color={theme.dim} paddingX={12} paddingY={4}>{text}</text>;
    return (
        <row flex={1}>
            <column width={left}>
                <Toolbar theme={theme}>
                    <text fontSize={12} fontFamily={UI} color={theme.text} paddingX={6}>Files</text>
                </Toolbar>
                <scroll key="files" flex={1} paddingY={4}>
                    {files.map((one) => {
                        const depth = one.split('/').length - 1;
                        return (
                            <button direction="row" justify="start" key={`file-${one}`} name={`File ${one}`} onClick={() => setFile(one)} height={ROW} align="center" gap={6} paddingLeft={8 + depth * 12} paddingRight={8} background={one === file ? theme.selected : 'transparent'} hover={one === file ? theme.selected : theme.hover}>
                                <Icon name="file" color={one.endsWith('.css') ? theme.kw : theme.tag} size={14} />
                                <text fontSize={12} fontFamily={UI} color={theme.text} noWrap ellipsis>{one.slice(one.lastIndexOf('/') + 1)}</text>
                            </button>
                        );
                    })}
                    {files.length === 0 ? note('No source files were found next to the app.') : null}
                </scroll>
            </column>
            <Splitter width={left} setWidth={setLeft} least={140} most={420} theme={theme} />
            <column flex={1}>
                <column background={theme.bar}>
                    <row height={28} align="center" paddingX={4}>
                        <row height={28} paddingX={10} align="center" gap={6} background={theme.page}>
                            <Icon name="file" color={theme.tag} size={14} />
                            <text fontSize={12} fontFamily={UI} color={theme.text} noWrap>{file}</text>
                        </row>
                        <group flex={1} />
                        {said ? <text fontSize={12} fontFamily={UI} color={theme.error} paddingRight={8} selectable>{said}</text> : null}
                    </row>
                    <Across theme={theme} />
                </column>
                <CodeView file={file} folder={folder} breakpoints={news.breakpoints} logpoints={news.logpoints} stopped={stopped} frame={frame} ask={ask} onSaid={setSaid} theme={theme} />
            </column>
            <Splitter width={right} setWidth={setRight} least={220} most={560} theme={theme} fromRight />
            <column width={right}>
                <Toolbar theme={theme}>
                    {stopped ? (
                        <IconButton name="Resume" icon="resume" onClick={() => resume('continue')} theme={theme} color={theme.accent} />
                    ) : (
                        <IconButton name="Pause the app" icon="pause" onClick={() => ask({ ask: 'pause' }).then((answer) => setSaid(answer.error ?? null))} theme={theme} disabled={gone} />
                    )}
                    <IconButton name="Step over" icon="over" onClick={() => resume('over')} theme={theme} disabled={!stopped} color={stopped ? theme.accent : theme.strong} />
                    <IconButton name="Step into" icon="into" onClick={() => resume('into')} theme={theme} disabled={!stopped} color={stopped ? theme.accent : theme.strong} />
                    <IconButton name="Step out" icon="out" onClick={() => resume('out')} theme={theme} disabled={!stopped} color={stopped ? theme.accent : theme.strong} />
                </Toolbar>
                <scroll key="debugger" flex={1}>
                    {reason ? (
                        <column background={theme.pausedBack} paddingX={10} paddingY={8} gap={2}>
                            <text fontSize={12} fontFamily={UI} fontWeight="bold" color={theme.paused}>{reason}</text>
                            <text fontSize={12} fontFamily={UI} color={theme.text} selectable>{`${stopped?.function || 'a function with no name'}, ${stopped?.at}`}</text>
                        </column>
                    ) : null}
                    <Section title="Watch" open={open.watch} onToggle={() => toggle('watch')} theme={theme}>
                        {watches.map((one) => (
                            <row key={`watch-${one}`} height={ROW} align="center" paddingLeft={12} paddingRight={2} hover={theme.hover}>
                                <text fontSize={12} fontFamily={MONO} color={theme.attr} noWrap>{one}</text>
                                <text fontSize={12} fontFamily={MONO} color={theme.dim}>{': '}</text>
                                <text flex={1} fontSize={12} fontFamily={MONO} color={watched[one]?.threw ? theme.error : theme.text} noWrap ellipsis selectable>
                                    {stopped ? watched[one]?.value ?? '…' : '<not available>'}
                                </text>
                                <IconButton name={`Stop watching ${one}`} icon="close" onClick={() => setWatches(watches.filter((other) => other !== one))} theme={theme} />
                            </row>
                        ))}
                        <row paddingX={8} paddingY={4}>
                            <Field
                                name="Watch expression"
                                value={watching}
                                onChange={setWatching}
                                placeholder="Add an expression to watch"
                                flex={1}
                                mono
                                theme={theme}
                                onKeyPress={(key: string) => {
                                    if (key === 'enter' && watching.trim() !== '') {
                                        setWatches([...watches.filter((one) => one !== watching.trim()), watching.trim()]);
                                        setWatching('');
                                        return true;
                                    }
                                    return false;
                                }}
                            />
                        </row>
                    </Section>
                    <Section title="Breakpoints" open={open.breakpoints} onToggle={() => toggle('breakpoints')} theme={theme}>
                        {news.breakpoints.length === 0 && news.logpoints.length === 0 ? note("Click a line's number to stop there, or right-click it for a logpoint.") : null}
                        {news.breakpoints.map((one) => (
                            <row key={`bp-${one.at}`} height={ROW + 2} align="center" paddingLeft={12} gap={6} hover={theme.hover}>
                                <group width={10} height={10} radius={2} background={theme.breakpoint} />
                                <button direction="row" justify="start" name={`Show ${one.at}`} flex={1} height={ROW} align="center" onClick={() => setFile(placeOf(one.at)[0])}>
                                    <text fontSize={12} fontFamily={UI} color={theme.text} noWrap ellipsis>{one.at}</text>
                                </button>
                                <text fontSize={11} fontFamily={UI} color={theme.dim}>{one.hits === 0 ? '' : one.hits === 1 ? 'stopped once' : `stopped ${one.hits}×`}</text>
                                <IconButton name={`Take away the breakpoint at ${one.at}`} icon="close" onClick={() => ask({ ask: 'breakpoint', at: one.at, take: true })} theme={theme} />
                            </row>
                        ))}
                        {news.logpoints.map((one) => (
                            <row key={`lp-${one.at}-${one.expression}`} height={ROW + 2} align="center" paddingLeft={12} gap={6} hover={theme.hover}>
                                <group width={10} height={10} radius={2} background={theme.logpoint} />
                                <button direction="row" justify="start" name={`Show ${one.at}`} flex={1} height={ROW} align="center" onClick={() => setFile(placeOf(one.at)[0])}>
                                    <text fontSize={12} fontFamily={UI} color={theme.text} noWrap ellipsis>
                                        {`${one.at}  `}
                                        <span color={theme.logpoint}>{one.expression}</span>
                                    </text>
                                </button>
                                <text fontSize={11} fontFamily={UI} color={theme.dim}>{one.hits === 0 ? '' : `ran ${one.hits}×`}</text>
                                <IconButton name={`Take away the logpoint at ${one.at}`} icon="close" onClick={() => ask({ ask: 'logpoint', at: one.at })} theme={theme} />
                            </row>
                        ))}
                    </Section>
                    <Section title="Scope" open={open.scope} onToggle={() => toggle('scope')} theme={theme}>
                        {!chosen ? note('Not paused') : null}
                        {chosen && chosen.locals.length === 0 ? note(chosen.debugged ? 'Nothing in its variables here.' : "It's not one of the app's own files, so its variables aren't known.") : null}
                        {chosen?.locals.map((local) => (
                            <row key={`local-${local.name}`} paddingLeft={12} paddingRight={8} paddingY={1} hover={theme.hover}>
                                <text fontSize={12} fontFamily={MONO} color={local.name === 'this' ? theme.kw : theme.attr} noWrap>{local.name}</text>
                                <text fontSize={12} fontFamily={MONO} color={theme.dim}>{': '}</text>
                                <text flex={1} fontSize={12} fontFamily={MONO} color={/^-?[\d.]+$/.test(local.value) ? theme.num : /^['"]/.test(local.value) ? theme.str : theme.text} selectable>
                                    {local.value}
                                </text>
                            </row>
                        ))}
                    </Section>
                    <Section title="Call Stack" open={open.stack} onToggle={() => toggle('stack')} theme={theme}>
                        {!stopped ? note('Not paused') : null}
                        {stopped?.frames.map((one, index) => (
                            <button
                                key={`stack-${index}`}
                                name={`Call ${index}`}
                                direction="row" justify="start"
                                onClick={() => {
                                    setFrame(index);
                                    setFile(placeOf(one.at)[0]);
                                }}
                                height={ROW + 2}
                                align="center"
                                paddingLeft={6}
                                paddingRight={8}
                                gap={4}
                                background={index === frame ? theme.selected : 'transparent'}
                                hover={index === frame ? theme.selected : theme.hover}
                            >
                                <group width={12}>{index === frame ? <Icon name="right" color={theme.accent} size={12} /> : null}</group>
                                <text flex={1} fontSize={12} fontFamily={UI} color={one.debugged ? theme.text : theme.dim} noWrap ellipsis>{one.function || '(anonymous)'}</text>
                                <text fontSize={11} fontFamily={UI} color={theme.dim} noWrap>{one.at}</text>
                            </button>
                        ))}
                    </Section>
                </scroll>
            </column>
        </row>
    );
}

// Performance.
