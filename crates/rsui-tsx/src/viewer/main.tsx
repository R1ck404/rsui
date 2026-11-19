// rsui's inspector, laid out as a browser's is. `rsui-tsx --view file.rsuitrace` shows a trace that was saved, with the file in
// RSUI_TRACE: each frame as what came in, what rendered and why, and where the time went, with the components that took the
// most, and every change of state. `rsui-tsx --view` looks at an app while it runs, over the pipe each app run with rsui-tsx
// listens on, as said in the files in RSUI_LIVE: then there's its elements, what it says and a console for it, its code with
// breakpoints and logpoints, and what came in to it, to start it again from any of that and stop in what it did.
import { useState, useEffect, useRef, useCallback, useColorScheme } from 'rsui';
import { readFileSync, unlinkSync } from 'fs';
import { ASK_EVERY, DARK, LIGHT, MOST_FRAMES, MOST_HEARD, PANEL_LEAST, UI } from './theme';
import type { Ask, Breakpoint, Change, Frame, Heard, Logpoint, Running, Stopped, Theme, Trace } from './theme';
import { Link, runningApps } from './link';
import { Across, Action, Down, Icon, IconButton, TabStrip } from './ui';
import { Elements } from './elements';
import { Console } from './console';
import type { Worked } from './console';
import { Sources, placeOf } from './sources';
import { History } from './state';
import { Replay } from './replay';
import { Performance } from './performance';
import { Rendering } from './rendering';
import { Spans } from './spans';
import { Timeline } from './timeline';
import { StatusBar, Palette } from './palette';
import type { Command } from './palette';

export const title = 'rsui inspector';
export const size = [1280, 800];
export const minSize = [720, 420];

function load(): Trace | string | null {
    const file = process.env.RSUI_TRACE;
    if (!file) {
        return process.env.RSUI_LIVE ? null : 'There is no trace to show. Open one with rsui-tsx --view file.rsuitrace.';
    }
    try {
        return JSON.parse(readFileSync(file, 'utf8'));
    } catch (error) {
        return `${file} couldn't be read: ${error}`;
    }
}

const TRACE = load();

/// A trace that was saved.
function Saved({ trace, theme }: { trace: Trace; theme: Theme }) {
    const [tab, setTab] = useState('Performance');
    // A trace saved since version 2 has where the time went, as spans, for the same Performance tab as live.
    const spans = useRef<Spans | null>(null);
    if (spans.current === null) {
        spans.current = new Spans();
        spans.current.absorb((trace as any).spans);
    }
    const timed = spans.current.frames.length > 0 || spans.current.spans.length > 0;
    return (
        <window background={theme.page} color={theme.text} fontFamily={UI}>
            <TabStrip
                tabs={['Performance', 'Rendering', 'State']}
                value={tab}
                onChange={setTab}
                theme={theme}
                right={<text fontSize={12} fontFamily={UI} color={theme.dim} paddingRight={10}>{trace.file ?? trace.app}</text>}
            />
            {tab === 'Performance' && timed ? <Timeline spans={spans.current} frames={trace.frames} live={false} summary={() => Promise.resolve(trace.summary)} theme={theme} /> : null}
            {tab === 'Performance' && !timed ? <Performance frames={trace.frames} live={false} summary={() => Promise.resolve(trace.summary)} theme={theme} /> : null}
            {tab === 'Rendering' ? <Rendering frames={trace.frames} theme={theme} /> : null}
            {tab === 'State' ? <History history={trace.history} now={null} goTo={null} said={null} theme={theme} /> : null}
        </window>
    );
}

type News = { frames: Frame[]; history: Change[]; now: number; heard: Heard[]; logpoints: Logpoint[]; showing: boolean; breakpoints: Breakpoint[]; paused: Stopped | null; inputs: number };
const NO_NEWS: News = { frames: [], history: [], now: 0, heard: [], logpoints: [], showing: false, breakpoints: [], paused: null, inputs: 0 };
const TABS = ['Elements', 'Console', 'Sources', 'Performance', 'Rendering', 'State', 'Replay'];

/// An app being looked at while it runs, until it stops, when what was kept stays to be looked at.
function Watching({ app, onLeave, onReplay, theme }: { app: Running; onLeave: () => void; onReplay: (pid: number) => void; theme: Theme }) {
    const [news, setNews] = useState<News>(NO_NEWS);
    const [gone, setGone] = useState<string | null>(null);
    // Whether what's shown has stopped being brought up to date, to read it, which doesn't stop the app.
    const [frozen, setFrozen] = useState(false);
    const [tab, setTab] = useState('Elements');
    const [said, setSaid] = useState<string | null>(null);
    const [frame, setFrame] = useState(0);
    const [wanted, setWanted] = useState<string | null>(null);
    const [worked, setWorked] = useState<Worked[]>([]);
    const [cleared, setCleared] = useState(0);
    // Which edge of the app's window this is kept along, if it's docked in it, as a browser's inspector can be.
    const [docked, setDocked] = useState<string | null>(null);
    const link = useRef<Link | null>(null);
    // Where the app's time went, as spans, which only grows, so it's kept here and told about with a number.
    const spans = useRef(new Spans());
    const [, setSpansVersion] = useState(0);
    const [picking, setPicking] = useState(false);
    const [picked, setPicked] = useState<{ path: number[]; at: number } | null>(null);
    const [palette, setPalette] = useState(false);
    const frozenNow = useRef(false);
    frozenNow.current = frozen;
    // What's said about something done goes after a while, as a browser's inspector's notes do.
    useEffect(() => {
        if (said === null) {
            return undefined;
        }
        const timer = setTimeout(() => setSaid(null), 6000);
        return () => clearTimeout(timer);
    }, [said]);

    useEffect(() => {
        const made = new Link(app.pipe, (why) => {
            setGone(why);
            if (why.includes('no longer running')) {
                try {
                    unlinkSync(app.place);
                } catch {
                    // It's gone already.
                }
            }
        });
        link.current = made;
        let kept = NO_NEWS;
        let heardUpTo = 0;
        let timer: any = null;
        const askAgain = async () => {
            if (made.gone) {
                return;
            }
            if (!frozenNow.current) {
                const after = kept.frames[kept.frames.length - 1]?.number ?? 0;
                const answer = await made.ask({ ask: 'news', after, heard: heardUpTo, ...spans.current.asking() });
                if (answer.error) {
                    if (!made.gone) {
                        setSaid(answer.error);
                    }
                } else {
                    // F12 in the app again: it's given its window back already, and this goes.
                    if (answer.close) {
                        made.close();
                        process.exit(0);
                    }
                    if (spans.current.absorb(answer.spans)) {
                        setSpansVersion(spans.current.version);
                    }
                    setPicking(Boolean(answer.picking));
                    if (answer.picked) {
                        setPicked({ path: answer.picked, at: Date.now() });
                        setTab('Elements');
                    }
                    // Frames are numbered again from 1 when the app starts keeping them again.
                    const fresh: Frame[] = answer.frames;
                    const again = answer.started || (fresh.length > 0 && fresh[0].number <= after);
                    const frames = fresh.length === 0 && !again ? kept.frames : [...(again ? [] : kept.frames), ...fresh].slice(-MOST_FRAMES);
                    const heard = answer.heard.length === 0 ? kept.heard : [...kept.heard, ...answer.heard].slice(-MOST_HEARD);
                    heardUpTo = answer.said;
                    const history: Change[] = answer.history;
                    const showing = answer.showing ?? kept.showing;
                    const inputs = answer.inputs ?? kept.inputs;
                    const stopped: Stopped | null = answer.paused ?? null;
                    const same =
                        frames === kept.frames &&
                        heard === kept.heard &&
                        answer.now === kept.now &&
                        showing === kept.showing &&
                        inputs === kept.inputs &&
                        history.length === kept.history.length &&
                        history[history.length - 1]?.number === kept.history[kept.history.length - 1]?.number &&
                        JSON.stringify(answer.logpoints) === JSON.stringify(kept.logpoints) &&
                        JSON.stringify(answer.breakpoints) === JSON.stringify(kept.breakpoints) &&
                        JSON.stringify(stopped) === JSON.stringify(kept.paused);
                    if (!same) {
                        // Where it's just stopped is shown, in its code, whatever was being looked at.
                        if (stopped && (!kept.paused || kept.paused.at !== stopped.at)) {
                            setTab('Sources');
                            setFrame(0);
                            setWanted(placeOf(stopped.at)[0]);
                        }
                        kept = { frames, history, now: answer.now, heard, logpoints: answer.logpoints, showing, breakpoints: answer.breakpoints ?? [], paused: stopped, inputs };
                        setNews(kept);
                    }
                }
            }
            timer = setTimeout(askAgain, ASK_EVERY);
        };
        askAgain();
        return () => {
            clearTimeout(timer);
            made.close();
        };
    }, [app.pipe]);

    const ask: Ask = useCallback((request: object) => link.current?.ask(request) ?? Promise.resolve({ error: 'The app has stopped.' }), []);
    const goTo = async (change: number) => {
        const answer = await ask({ ask: 'goto', change });
        setSaid(answer.error ?? answer.said ?? null);
    };
    const save = async () => {
        const answer = await ask({ ask: 'save' });
        setSaid(answer.error ?? answer.said ?? null);
    };
    const showInApp = async () => {
        const answer = await ask({ ask: 'show', on: !news.showing });
        if (answer.error) {
            setSaid(answer.error);
        }
    };
    const resume = async (how: string) => {
        const answer = await ask({ ask: 'resume', how });
        setSaid(answer.error ?? null);
    };
    const dock = async (side: string | null) => {
        const answer = await ask({ ask: 'dock', pid: process.pid, side, size: side === 'bottom' ? 320 : 520 });
        if (answer.error) {
            setSaid(answer.error);
        } else {
            setDocked(side);
        }
    };
    // Opened by F12 in the app, it docks itself along the app's bottom, or where it was docked last.
    useEffect(() => {
        if (process.env.RSUI_VIEW_DOCK) {
            let side = 'bottom';
            try {
                side = localStorage.getItem('dock') ?? 'bottom';
            } catch {
                // Nothing's kept, so it goes along the bottom.
            }
            dock(side === 'none' ? null : side);
        }
    }, []);
    useEffect(() => {
        if (docked !== null) {
            try {
                localStorage.setItem('dock', docked);
            } catch {
                // It's only a convenience.
            }
        }
    }, [docked]);
    const pick = async () => {
        const answer = await ask({ ask: 'pick', on: !picking });
        if (answer.error) {
            setSaid(answer.error);
        } else {
            setPicking(Boolean(answer.picking));
        }
    };
    const openSource = (file: string) => {
        setWanted(file);
        setTab('Sources');
    };
    const commands: Command[] = [
        ...TABS.map((name) => ({ name: `Show ${name}`, hint: 'tab', run: () => setTab(name) })),
        { name: 'Pick an element in the app', hint: 'Elements', run: () => { setTab('Elements'); pick(); } },
        { name: spans.current.recording ? 'Stop recording JavaScript' : 'Record JavaScript', hint: 'Performance', run: async () => { setTab('Performance'); await ask({ ask: 'record', on: !spans.current.recording }); } },
        { name: news.showing ? 'Hide flashes in the app' : 'Show flashes in the app', hint: 'ctrl+shift+F', run: showInApp },
        { name: 'Save a trace', run: save },
        { name: 'Dock to the bottom of the app', run: () => dock('bottom') },
        { name: 'Dock to the right of the app', run: () => dock('right') },
        { name: 'Undock into a separate window', run: () => dock(null) },
        { name: frozen ? 'Bring up to date' : 'Stop updating', run: () => setFrozen(!frozen) },
        { name: 'Pause the app', hint: 'Sources', run: () => ask({ ask: 'pause' }) },
    ];
    // When the app's gone, so is where it was docked, as the app gives it back as it closes.
    useEffect(() => {
        if (gone) {
            setDocked(null);
        }
    }, [gone]);
    const run = async (expression: string) => {
        const after = news.heard[news.heard.length - 1]?.number ?? 0;
        const stopped = news.paused;
        const answer = await ask(stopped ? { ask: 'evaluate', frame, expression } : { ask: 'evaluate', expression });
        const output = String(answer.error ?? answer.value ?? '');
        setWorked((was) => [...was, { after, input: expression, output, threw: Boolean(answer.error || answer.threw) }].slice(-200));
    };
    const stopped = gone ? null : news.paused;
    const status = gone ?? (stopped ? `paused, process ${app.pid}` : frozen ? 'not brought up to date' : `running, process ${app.pid}`);

    return (
        <window
            background={theme.page}
            color={theme.text}
            fontFamily={UI}
            onKeyPress={(key: string) => {
                if (key === 'ctrl+shift+p' || key === 'f1') {
                    setPalette(true);
                    return true;
                }
                return false;
            }}
        >
            <TabStrip
                tabs={TABS}
                value={tab}
                onChange={setTab}
                theme={theme}
                right={
                    <row align="center" gap={2} paddingRight={6} shrink={1}>
                        <group width={8} height={8} radius={4} background={gone ? theme.strong : stopped ? theme.paused : theme.ok} />
                        <text fontSize={12} fontFamily={UI} color={theme.text} paddingLeft={4} noWrap ellipsis shrink={6} minWidth={0} maxWidth={140}>{app.app}</text>
                        <text fontSize={12} fontFamily={UI} color={theme.dim} paddingLeft={6} paddingRight={6} noWrap ellipsis shrink={12} minWidth={0} maxWidth={180}>{status}</text>
                        <Down theme={theme} />
                        {gone ? <Action label="Look at another" onClick={onLeave} theme={theme} /> : null}
                        {gone ? null : <IconButton name={picking ? 'Stop picking' : 'Pick an element in the app'} icon="pick" on={picking} onClick={pick} theme={theme} />}
                        {gone ? null : <Down theme={theme} />}
                        {gone ? null : <IconButton name="Undock into a separate window" icon="undock" on={docked === null} onClick={() => dock(null)} theme={theme} />}
                        {gone ? null : <IconButton name="Dock to the bottom of the app" icon="dockBottom" on={docked === 'bottom'} onClick={() => dock('bottom')} theme={theme} />}
                        {gone ? null : <IconButton name="Dock to the right of the app" icon="dockRight" on={docked === 'right'} onClick={() => dock('right')} theme={theme} />}
                        {gone ? null : <Down theme={theme} />}
                        {gone ? null : <IconButton name={news.showing ? 'Hide in the app' : 'Show in the app'} icon="eye" on={news.showing} onClick={showInApp} theme={theme} />}
                        {gone ? null : <IconButton name={frozen ? 'Bring up to date' : 'Stop updating'} icon="freeze" on={frozen} onClick={() => setFrozen(!frozen)} theme={theme} />}
                        {gone ? null : <IconButton name="Save a trace" icon="save" onClick={save} theme={theme} />}
                    </row>
                }
            />
            {stopped ? (
                <column background={theme.pausedBack}>
                    <row height={30} align="center" paddingX={10} gap={8}>
                        <Icon name="pause" color={theme.paused} size={14} />
                        <text fontSize={12} fontFamily={UI} fontWeight="bold" color={theme.paused}>Paused in debugger</text>
                        <text flex={1} fontSize={12} fontFamily={UI} color={theme.text} noWrap ellipsis>{`${stopped.function || 'a function with no name'}, ${stopped.at}`}</text>
                        <IconButton name="Resume script" icon="resume" onClick={() => resume('continue')} theme={theme} color={theme.accent} />
                        <IconButton name="Step over the next call" icon="over" onClick={() => resume('over')} theme={theme} color={theme.accent} />
                    </row>
                    <Across theme={theme} />
                </column>
            ) : null}
            {said && tab !== 'State' ? (
                <column background={theme.bar}>
                    <row height={24} align="center" paddingX={10}>
                        <text flex={1} fontSize={12} fontFamily={UI} color={theme.text} noWrap ellipsis selectable>{said}</text>
                        <IconButton name="Dismiss" icon="close" onClick={() => setSaid(null)} theme={theme} />
                    </row>
                    <Across theme={theme} />
                </column>
            ) : null}
            <scroll key="panel" axis="horizontal" flex={1}>
            <column flex={1} minWidth={PANEL_LEAST} height="100%">
            {tab === 'Elements' ? <Elements ask={ask} theme={theme} gone={Boolean(gone)} picked={picked} /> : null}
            {tab === 'Console' ? <Console heard={news.heard} worked={worked} run={run} clear={() => setCleared(news.heard[news.heard.length - 1]?.number ?? 0)} cleared={cleared} stopped={stopped} theme={theme} /> : null}
            {tab === 'Sources' ? <Sources app={app} news={news} ask={ask} frame={frame} setFrame={setFrame} wanted={wanted} theme={theme} gone={Boolean(gone)} /> : null}
            {tab === 'Performance' ? (
                <Timeline
                    spans={spans.current}
                    frames={news.frames}
                    live={!gone}
                    ask={gone ? undefined : ask}
                    summary={async () => {
                        const answer = await ask({ ask: 'summary' });
                        return answer.error ?? answer.summary ?? '';
                    }}
                    openSource={openSource}
                    theme={theme}
                />
            ) : null}
            {tab === 'Rendering' ? <Rendering frames={news.frames} theme={theme} /> : null}
            {tab === 'State' ? <History history={news.history} now={news.now} goTo={gone ? null : goTo} said={said} theme={theme} /> : null}
            {tab === 'Replay' ? <Replay count={news.inputs} ask={ask} onStarted={onReplay} theme={theme} /> : null}
            </column>
            </scroll>
            <StatusBar spans={spans.current} status={picking ? 'Click anything in the app to pick it.' : `${app.app}: ${status}`} said={null} theme={theme} onOpenPalette={() => setPalette(true)} />
            {palette ? <Palette commands={commands} close={() => setPalette(false)} theme={theme} /> : null}
        </window>
    );
}

/// The apps there are to look at, until one's chosen, as a browser lists what it can inspect. With only one, that's the one.
function Live({ theme }: { theme: Theme }) {
    const [apps, setApps] = useState<Running[]>(runningApps);
    const [chosen, setChosen] = useState<Running | null>(null);
    const [left, setLeft] = useState<number[]>([]);
    // A replay that's been started, which is looked at once it says it's there.
    const [awaited, setAwaited] = useState<number | null>(null);
    useEffect(() => {
        if (chosen) {
            return undefined;
        }
        const timer = setInterval(() => setApps(runningApps()), awaited ? 200 : 1000);
        return () => clearInterval(timer);
    }, [chosen, awaited]);
    const choosable = apps.filter((one) => !left.includes(one.pid));
    const only = choosable.length === 1 ? choosable[0] : null;
    const replayed = awaited ? apps.find((one) => one.pid === awaited) : undefined;
    // With only one, that's chosen, so it stays when it stops, for what was kept to be looked at. A replay that's been
    // started is chosen as soon as it's there.
    useEffect(() => {
        if (replayed) {
            setAwaited(null);
            setChosen(replayed);
        } else if (!chosen && only && !awaited) {
            setChosen(only);
        }
    }, [chosen, only?.pid, replayed?.pid]);
    if (chosen) {
        return (
            <Watching
                key={`app-${chosen.pid}`}
                app={chosen}
                onLeave={() => {
                    setLeft([...left, chosen.pid]);
                    setChosen(null);
                    setApps(runningApps());
                }}
                onReplay={(pid) => {
                    setAwaited(pid);
                    setChosen(null);
                }}
                theme={theme}
            />
        );
    }
    return (
        <window background={theme.page} color={theme.text} fontFamily={UI}>
            <TabStrip tabs={['Apps']} value="Apps" onChange={() => {}} theme={theme} />
            <column padding={24} gap={16} maxWidth={760}>
                <text fontSize={18} fontFamily={UI} color={theme.text}>{awaited ? `Starting the replay, process ${awaited}` : 'Apps running'}</text>
                <text fontSize={12} fontFamily={UI} color={theme.dim}>
                    {choosable.length === 0 ? 'Run one with rsui-tsx app.tsx and it shows up here. A packed app can’t be looked at.' : 'These are running, and can be inspected while they run.'}
                </text>
                <column borderWidth={1} borderColor={theme.line} radius={4}>
                    {choosable.map((one, index) => (
                        <column key={`app-${one.pid}`}>
                            {index > 0 ? <Across theme={theme} /> : null}
                            <row padding={12} gap={12} align="center" hover={theme.hover}>
                                <column flex={1} gap={2}>
                                    <text fontSize={13} fontFamily={UI} fontWeight="bold" color={theme.text}>{one.app}</text>
                                    <text fontSize={12} fontFamily={UI} color={theme.dim} noWrap ellipsis>{`${one.file} · process ${one.pid}`}</text>
                                </column>
                                <button name={one.app} onClick={() => setChosen(one)} direction="row" paddingX={8} height={24} align="center" hover={theme.hover} radius={4}>
                                    <text fontSize={12} fontFamily={UI} color={theme.accent}>inspect</text>
                                </button>
                            </row>
                        </column>
                    ))}
                    {choosable.length === 0 ? <text fontSize={12} fontFamily={UI} color={theme.dim} padding={12}>Nothing yet.</text> : null}
                </column>
            </column>
        </window>
    );
}

export default function Viewer() {
    const theme = useColorScheme() === 'dark' ? DARK : LIGHT;
    if (TRACE === null) {
        return <Live theme={theme} />;
    }
    if (typeof TRACE === 'string') {
        return (
            <window padding={24} background={theme.page}>
                <text color={theme.error} selectable>{TRACE}</text>
            </window>
        );
    }
    return <Saved trace={TRACE} theme={theme} />;
}
