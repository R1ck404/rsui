// A first app for phones: a list to tick things off on, a count, and a field to add to the list, sized for fingers. It's
// what the Android build carries until real apps are written for phones.
//
//   rsui-tsx app.tsx                        on a computer, in a narrow window
//   tools/android/build-apk.ps1 -App demos/mobile-hello   for a phone
//
// Everything is plain React with inline styles, so it needs no packages. The row of buttons under the title tries each of
// the system's own popups, from `native`, and says what came back.

import { useState } from 'react';
import { native } from 'rsui';
import { PageWindow } from 'rsui/web';

export const title = 'Hello';

type Item = { id: number; text: string; done: boolean };

const START: Item[] = [
    { id: 1, text: 'Open the app on the phone', done: true },
    { id: 2, text: 'Tap a row to tick it off', done: false },
    { id: 3, text: 'Add one of your own below', done: false },
    { id: 4, text: 'Scroll when the list is long', done: false },
];

const accent = '#3b6ef5';

function Row({ item, toggle }: { item: Item; toggle: () => void }) {
    return (
        <div
            onClick={toggle}
            style={{
                display: 'flex',
                alignItems: 'center',
                gap: 14,
                padding: '14px 18px',
                borderBottom: '1px solid #e6e8ef',
                background: '#fff',
            }}
        >
            <div
                style={{
                    width: 24,
                    height: 24,
                    borderRadius: 12,
                    border: item.done ? 'none' : '2px solid #b8bdcc',
                    background: item.done ? accent : 'transparent',
                    color: '#fff',
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    fontSize: 14,
                }}
            >
                {item.done ? '✓' : ''}
            </div>
            <div style={{ fontSize: 17, color: item.done ? '#9aa0b1' : '#1d2230', textDecoration: item.done ? 'line-through' : 'none' }}>
                {item.text}
            </div>
        </div>
    );
}

/// Each of the system's popups, and what it says when it's answered.
const POPUPS: [string, () => Promise<string>][] = [
    ['Confirm', async () => {
        const ok = await native.confirm({ title: 'Clear the list?', message: 'Everything on it goes.', confirm: 'Clear', destructive: true });
        return ok ? 'Clear it' : 'Kept it';
    }],
    ['Alert', async () => {
        await native.alert({ title: 'Saved', message: 'Your list is safe.' });
        return 'Seen';
    }],
    ['Sheet', async () => {
        const options = ['By name', 'By date', 'Done last'];
        const chosen = await native.sheet({ title: 'Sort the list', options });
        return chosen === null ? 'No sort' : options[chosen];
    }],
    ['Date', async () => {
        const day = await native.pickDate({ value: new Date() });
        return day === null ? 'No day' : day.toDateString();
    }],
    ['Time', async () => {
        const time = await native.pickTime({ value: { hours: 9, minutes: 0 } });
        return time === null ? 'No time' : `${time.hours}:${String(time.minutes).padStart(2, '0')}`;
    }],
    ['Share', async () => {
        await native.share({ title: 'My list', text: 'Things to do, from rsui' });
        return 'Shared';
    }],
    ['Toast', async () => {
        native.toast('Hello from rsui');
        return 'Toasted';
    }],
];

function Popups() {
    const [said, setSaid] = useState('Try the system\'s own popups');
    const run = (name: string, ask: () => Promise<string>) => {
        ask().then((answer) => setSaid(`${name}: ${answer}`), (error: Error) => setSaid(`${name} failed: ${error.message}`));
    };
    return (
        <div style={{ background: '#fff', borderBottom: '1px solid #e6e8ef' }}>
            <div style={{ display: 'flex', gap: 8, padding: '12px 14px 6px', overflowX: 'auto' }}>
                {POPUPS.map(([name, ask]) => (
                    <button
                        key={name}
                        onClick={() => run(name, ask)}
                        style={{ fontSize: 15, padding: '8px 14px', borderRadius: 18, border: '1px solid #d5d9e4', background: '#fff', flexShrink: 0 }}
                    >
                        {name}
                    </button>
                ))}
            </div>
            <div style={{ fontSize: 14, color: '#6b7183', padding: '0 18px 10px' }}>{said}</div>
        </div>
    );
}

function App() {
    const [items, setItems] = useState(START);
    const [count, setCount] = useState(0);
    const [draft, setDraft] = useState('');
    const left = items.filter((item) => !item.done).length;
    const add = () => {
        const text = draft.trim();
        if (text) {
            setItems([...items, { id: Date.now(), text, done: false }]);
            setDraft('');
        }
    };
    return (
        <div
            style={{
                position: 'fixed',
                inset: 0,
                display: 'flex',
                flexDirection: 'column',
                background: '#f4f5f9',
                fontFamily: 'system-ui',
            }}
        >
            <div style={{ padding: '28px 20px 16px', background: accent, color: '#fff' }}>
                <div style={{ fontSize: 28, fontWeight: 700 }}>Hello from rsui</div>
                <div style={{ fontSize: 15, opacity: 0.85, marginTop: 4 }}>
                    {left === 0 ? 'All done' : `${left} left to do`}
                </div>
            </div>
            <Popups />
            <div style={{ flex: 1, overflowY: 'auto' }}>
                {items.map((item) => (
                    <Row
                        key={item.id}
                        item={item}
                        toggle={() => setItems(items.map((one) => (one.id === item.id ? { ...one, done: !one.done } : one)))}
                    />
                ))}
            </div>
            <div style={{ display: 'flex', gap: 10, padding: 14, background: '#fff', borderTop: '1px solid #e6e8ef' }}>
                <input
                    value={draft}
                    placeholder="Something to do"
                    onChange={(event: { target: { value: string } }) => setDraft(event.target.value)}
                    onKeyDown={(event: { key: string }) => event.key === 'Enter' && add()}
                    style={{ flex: 1, fontSize: 17, padding: '12px 14px', borderRadius: 12, border: '1px solid #d5d9e4' }}
                />
                <button
                    onClick={add}
                    style={{ fontSize: 17, padding: '12px 18px', borderRadius: 12, border: 'none', background: accent, color: '#fff' }}
                >
                    Add
                </button>
            </div>
            <div style={{ display: 'flex', justifyContent: 'center', padding: '10px 0 18px', background: '#fff' }}>
                <button
                    onClick={() => setCount(count + 1)}
                    style={{ fontSize: 15, padding: '8px 16px', borderRadius: 20, border: '1px solid #d5d9e4', background: '#fff' }}
                >
                    Tapped {count} {count === 1 ? 'time' : 'times'}
                </button>
            </div>
        </div>
    );
}

export default function Root() {
    return (
        <PageWindow>
            <App />
        </PageWindow>
    );
}
