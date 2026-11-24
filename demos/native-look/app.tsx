// rsui's own controls in the platform's look: Material 3 on Android, Fluent on Windows, drawn by rsui. Each control is here
// in each of its states, with what's said for one platform (`android={{ ... }}`, `windows={{ ... }}`) beside the rest.
//
//   rsui-tsx app.tsx                       in a window, in this system's look
//   RSUI_THEME=material rsui-tsx app.tsx   in Material's, whatever the system (`theme` in rsui.config.ts says it too)
//   RSUI_COLOR_SCHEME=dark ...             dark, whatever the system says
//
// `screenshots/` has it in both looks, light and dark, drawn with no window.

import { useState } from 'rsui';

export const title = 'Native look';
export const size = [720, 860];

function Section({ title, children }: { title: string; children?: any }) {
    return (
        <column gap={10}>
            <text fontSize={13} opacity={0.7}>{title}</text>
            {children}
        </column>
    );
}

export default function App() {
    const [count, setCount] = useState(0);
    const [name, setName] = useState('');
    const [email, setEmail] = useState('ada@example.com');
    const [tick, setTick] = useState(true);
    const [other, setOther] = useState(false);
    const [wifi, setWifi] = useState(true);
    const [sound, setSound] = useState(false);
    const [volume, setVolume] = useState(40);
    return (
        <window native padding={24} gap={22} align="start">
            <Section title={'Buttons, pressed ' + count + ' times'}>
                <row gap={12} align="center">
                    <button key="save" native onClick={() => setCount(count + 1)}>Save</button>
                    <button key="second" native android={{ variant: 'tonal' }} windows={{ variant: 'accent' }} onClick={() => setCount(count + 1)}>
                        Send
                    </button>
                    <button key="third" native android={{ variant: 'outlined' }} windows={{ variant: 'subtle' }}>Cancel</button>
                    <button key="text" native android={{ variant: 'text' }}>Learn more</button>
                    <button key="lifted" native android={{ variant: 'elevated' }}>Share</button>
                </row>
                <row gap={12} align="center">
                    <button key="off" native disabled>Disabled</button>
                    <button key="off2" native disabled android={{ variant: 'outlined' }} windows={{ variant: 'accent' }}>Disabled too</button>
                    <button key="wide" native width={220}>A wide one</button>
                </row>
            </Section>
            <Section title="Text fields">
                <input key="name" native width={300} value={name} onChange={setName} placeholder="Name" />
                <input key="email" native width={300} value={email} onChange={setEmail} placeholder="Email" android={{ variant: 'filled' }} />
                <input key="locked" native width={300} value="Can't change this" disabled />
            </Section>
            <row gap={40}>
                <Section title="Checkboxes">
                    <checkbox key="tick" native checked={tick} onChange={setTick}>Remember me</checkbox>
                    <checkbox key="other" native checked={other} onChange={setOther}>Send me news</checkbox>
                    <checkbox key="tick-off" native checked disabled>Can't untick</checkbox>
                </Section>
                <Section title="Switches">
                    <switch key="wifi" native checked={wifi} onChange={setWifi}>Wi-Fi</switch>
                    <switch key="sound" native checked={sound} onChange={setSound}>Sound</switch>
                    <switch key="switch-off" native disabled>Airplane mode</switch>
                </Section>
            </row>
            <Section title={'Slider and progress, at ' + volume}>
                <slider key="volume" native width={300} value={volume} min={0} max={100} step={1} onChange={setVolume} />
                <progress key="done" native width={300} value={volume} max={100} />
            </Section>
        </window>
    );
}
