// More than one window: a settings window that stays above this one, a modal one that has to be answered before this one
// can be used again, and windows of their own that come and go. It's all one app, with the state here and the context from here reaching
// into all of them, so changing something in one shows in the others straight away.
//
// Run it with `cargo run --release -p rsui-tsx -- crates/rsui-tsx/examples/windows.tsx`.

import { createContext, mainWindow, Slider, Switch, useColorScheme, useContext, useState, Window } from 'rsui';

export const title = 'Windows';
export const size = [520, 340];
export const minSize = [360, 240];

const LIGHT = { page: '#ffffff', ink: '#1f2328', muted: '#656d76', side: '#f6f8fa' };
const DARK = { page: '#0d1117', ink: '#e6edf3', muted: '#8b949e', side: '#161b22' };
const Theme = createContext(LIGHT);

function Page({ children, gap = 12 }: { children?: any; gap?: number }) {
    const theme = useContext(Theme);
    return (
        <column flex={1} padding={18} gap={gap} background={theme.page} color={theme.ink}>
            {children}
        </column>
    );
}

function Action({ id, onClick, children, primary }: { id: string; onClick: () => void; children: any; primary?: boolean }) {
    const theme = useContext(Theme);
    return (
        <button key={id} paddingX={14} paddingY={7} radius={6} background={primary ? '#1f6feb' : theme.side} color={primary ? '#ffffff' : theme.ink} onClick={onClick}>
            {children}
        </button>
    );
}

function Settings({ size: textSize, setSize, loud, setLoud }: { size: number; setSize: (size: number) => void; loud: boolean; setLoud: (loud: boolean) => void }) {
    return (
        <Page>
            <text fontWeight="bold" fontSize={18}>Settings</text>
            <text>Text size: {textSize}</text>
            <Slider id="text-size" value={textSize} min={10} max={32} onChange={setSize} />
            <Switch id="loud" label="Bold" checked={loud} onChange={setLoud} />
        </Page>
    );
}

export default function App() {
    const scheme = useColorScheme();
    const theme = scheme === 'dark' ? DARK : LIGHT;
    const [showSettings, setShowSettings] = useState(false);
    const [asking, setAsking] = useState(false);
    const [notes, setNotes] = useState<number[]>([]);
    const [nextNote, setNextNote] = useState(1);
    const [textSize, setTextSize] = useState(16);
    const [loud, setLoud] = useState(false);

    const addNote = () => {
        setNotes([...notes, nextNote]);
        setNextNote(nextNote + 1);
    };

    return (
        <Theme.Provider value={theme}>
            <window key="root" onClose={() => setAsking(true)}>
                <Page>
                    <text fontSize={textSize} fontWeight={loud ? 'bold' : undefined}>This changes from the other window.</text>
                    <row gap={8}>
                        <Action id="settings" primary onClick={() => setShowSettings(!showSettings)}>{showSettings ? 'Close settings' : 'Open settings'}</Action>
                        <Action id="note" onClick={addNote}>New note</Action>
                    </row>
                    <text color={theme.muted}>{notes.length === 0 ? 'No notes open.' : `${notes.length} open, each a window of its own.`}</text>
                    <text color={theme.muted}>Closing this one asks first.</text>
                </Page>

                {showSettings ? (
                    <Window title="Settings" size={[320, 240]} minSize={[260, 200]} onClose={() => setShowSettings(false)}>
                        <Settings size={textSize} setSize={setTextSize} loud={loud} setLoud={setLoud} />
                    </Window>
                ) : null}

                {notes.map((note) => (
                    <Window key={note} title={`Note ${note}`} size={[260, 160]} owned={false} onClose={() => setNotes(notes.filter((other) => other !== note))}>
                        <Page>
                            <text fontSize={textSize} fontWeight={loud ? 'bold' : undefined}>{`Note ${note}`}</text>
                            <text color={theme.muted}>This stays open on its own.</text>
                        </Page>
                    </Window>
                ))}

                {asking ? (
                    <Window title="Close?" size={[300, 130]} modal onClose={() => setAsking(false)}>
                        <Page>
                            <text>Close everything?</text>
                            <row gap={8} justify="end">
                                <Action id="no" onClick={() => setAsking(false)}>Keep it open</Action>
                                <Action id="yes" primary onClick={() => mainWindow.close()}>Close</Action>
                            </row>
                        </Page>
                    </Window>
                ) : null}
            </window>
        </Theme.Provider>
    );
}
