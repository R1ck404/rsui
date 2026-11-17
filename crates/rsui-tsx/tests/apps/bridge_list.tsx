import { useState, useAnimated } from 'rsui';

function Row({ index, tick }) {
    return <row height={24} gap={8} paddingX={4} background={tick % 2 ? '#eef3ff' : '#ffffff'}>
        <text width={90} fontSize={12}>{'Row ' + index}</text>
        <text fontSize={12} color="#334455">{tick % 2 ? 'Updated' : 'Ready'}</text>
    </row>;
}

export default function App() {
    const [tick, setTick] = useState(0);
    const width = useAnimated(tick % 2 ? 180 : 90, 120);
    const count = Number(process.env.RSUI_BRIDGE_ROWS || 400);
    return <window gap={4}>
        <button key="advance" height={30} onClick={() => setTick(tick + 1)}>Advance</button>
        <text>{'Tick ' + tick}</text>
        <group key="motion" width={width} height={4} background="#334455" />
        <input key="typing" value={String(tick)} onChange={() => {}} />
        <scroll key="rows" height={500}>
            {Array.from({ length: count }, (_, index) => <Row key={index} index={index} tick={tick} />)}
        </scroll>
    </window>;
}
