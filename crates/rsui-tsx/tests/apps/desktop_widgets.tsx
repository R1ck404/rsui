// Desktop widgets from plain window options: a clock on the desktop itself, behind the icons, and a bar along an edge of the
// screen that keeps its strip the way the taskbar does. This little window is what turns them on and off.
//
// It's for trying by hand, as it changes the real desktop.
// Run it with
// `cargo run --release -p rsui-tsx -- crates/rsui-tsx/examples/desktop_widgets.tsx`.

import { mainWindow, useEffect, useState, Window } from 'rsui';

export const title = 'Desktop widgets';
export const size = [360, 300];

type Layer = 'desktop' | 'bottom' | 'top' | 'normal';
type Edge = 'top' | 'bottom' | 'left' | 'right';

function useNow() {
    const [now, setNow] = useState(() => new Date());
    useEffect(() => {
        const timer = setInterval(() => setNow(new Date()), 1000);
        return () => clearInterval(timer);
    }, []);
    return now;
}

const two = (n: number) => String(n).padStart(2, '0');

/// Only what's drawn here shows: a card with a see-through background, and the time on it. Nothing's behind the root.
function Clock() {
    const now = useNow();
    return (
        <column flex={1} align="center" justify="center">
            <column padding={16} radius={18} background="rgba(15, 20, 30, 0.55)" align="center" gap={2} dragsWindow>
                <text fontSize={44} fontWeight="bold" color="#ffffff">{`${two(now.getHours())}:${two(now.getMinutes())}:${two(now.getSeconds())}`}</text>
                <text fontSize={14} color="rgba(255, 255, 255, 0.8)">{now.toDateString()}</text>
            </column>
        </column>
    );
}

function Bar({ edge, onMove }: { edge: Edge; onMove: () => void }) {
    const now = useNow();
    const across = edge === 'top' || edge === 'bottom';
    return (
        <group flex={1} direction={across ? 'row' : 'column'} align="center" gap={12} paddingX={12} paddingY={6} background="#1b2230" color="#e6edf3">
            <text fontWeight="bold">rsui bar</text>
            <text>{`${two(now.getHours())}:${two(now.getMinutes())}`}</text>
            <row flex={1} />
            <button key="bar-move" id="bar-move" paddingX={10} paddingY={4} radius={4} background="#2f3b52" onClick={onMove}>
                Move ({edge})
            </button>
        </group>
    );
}

function Choice({ id, chosen, onClick, children }: { id: string; chosen: boolean; onClick: () => void; children: any }) {
    return (
        <button key={id} id={id} paddingX={10} paddingY={5} radius={5} background={chosen ? '#1f6feb' : '#eaeef2'} color={chosen ? '#ffffff' : '#1f2328'} onClick={onClick}>
            {children}
        </button>
    );
}

const EDGES: Edge[] = ['top', 'right', 'bottom', 'left'];

export default function App() {
    const [clock, setClock] = useState(true);
    const [layer, setLayer] = useState<Layer>('desktop');
    const [transparent, setTransparent] = useState(true);
    const [bar, setBar] = useState(false);
    const [edge, setEdge] = useState<Edge>('top');
    const moveBar = () => setEdge(EDGES[(EDGES.indexOf(edge) + 1) % EDGES.length]);
    return (
        <window>
            <column flex={1} padding={16} gap={10} background="#ffffff" color="#1f2328">
                <text fontWeight="bold" fontSize={16}>Desktop widgets</text>
                <text>{`Clock: ${clock ? `on, ${layer} layer${transparent ? ', transparent' : ''}` : 'off'}`}</text>
                <row gap={6}>
                    <Choice id="clock" chosen={clock} onClick={() => setClock(!clock)}>{clock ? 'Hide clock' : 'Show clock'}</Choice>
                    <Choice id="see-through" chosen={transparent} onClick={() => setTransparent(!transparent)}>Transparent</Choice>
                </row>
                <row gap={6}>
                    {(['desktop', 'bottom', 'normal', 'top'] as Layer[]).map((one) => (
                        <Choice key={one} id={`layer-${one}`} chosen={layer === one} onClick={() => setLayer(one)}>{one}</Choice>
                    ))}
                </row>
                <text>{`App bar: ${bar ? `on the ${edge} edge` : 'off'}`}</text>
                <row gap={6}>
                    <Choice id="bar-toggle" chosen={bar} onClick={() => setBar(!bar)}>{bar ? 'Remove bar' : 'Add bar'}</Choice>
                    <Choice id="bar-edge" chosen={false} onClick={moveBar}>Next edge</Choice>
                </row>
                <row gap={6}>
                    <Choice id="quit" chosen={false} onClick={() => mainWindow.close()}>Quit, putting everything back</Choice>
                </row>
            </column>
            {clock ? (
                <Window title="Clock" size={[320, 160]} owned={false} frameless transparent={transparent} layer={layer} skipTaskbar allDesktops>
                    <Clock />
                </Window>
            ) : null}
            {bar ? (
                <Window title="Bar" owned={false} frameless alwaysOnTop skipTaskbar appBar={{ edge, size: edge === 'top' || edge === 'bottom' ? 36 : 140 }}>
                    <Bar edge={edge} onMove={moveBar} />
                </Window>
            ) : null}
        </window>
    );
}
