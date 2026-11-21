// What the types allow of `native` on rsui's own elements, and what they don't: `tsc -p crates/rsui-tsx/tests/types` has
// no errors when each line marked `@ts-expect-error` is one.

import { useState } from 'rsui';

export function Allowed() {
    const [name, setName] = useState('');
    const [on, setOn] = useState(false);
    const [level, setLevel] = useState(3);
    return (
        <window native padding={16} gap={8}>
            <button native onClick={() => setOn(!on)}>Save</button>
            <button native android={{ variant: 'tonal' }} windows={{ variant: 'accent' }} style={{ marginTop: 8 }}>Send</button>
            <button native disabled width={200} margin={4} flex={1} alignSelf="start">Wide</button>
            <input native value={name} onChange={setName} placeholder="Name" android={{ variant: 'filled' }} />
            <input native value={name} onInput={(value) => setName(value.trim())} />
            <checkbox native checked={on} onChange={(checked) => setOn(checked)}>Remember me</checkbox>
            <switch native checked={on} onChange={setOn}>Wi-Fi</switch>
            <slider native value={level} min={0} max={10} onChange={setLevel} width={200} />
            <progress native value={0.4} />
            {/* Without `native`, they're as they were. */}
            <button background="#123456" radius={6} className="x">Plain</button>
            <button native={false} background="#123456">Plain too</button>
            <checkbox checked={on} onToggle={() => setOn(!on)}>Plain</checkbox>
        </window>
    );
}

export function NotAllowed() {
    return (
        <window native>
            {/* @ts-expect-error a native control's colors are the platform's */}
            <button native background="#ff0000">Red</button>
            {/* @ts-expect-error and so are its corners */}
            <button native radius={2}>Square</button>
            {/* @ts-expect-error and its classes */}
            <button native className="rounded bg-blue-600">Classes</button>
            {/* @ts-expect-error and what's in its style */}
            <button native style={{ color: 'red' }}>Style</button>
            {/* @ts-expect-error a variant one platform doesn't have */}
            <button native android={{ variant: 'accent' }}>Wrong platform</button>
            {/* @ts-expect-error `native` says on or off, never a platform */}
            <button native="android">Platform</button>
            {/* @ts-expect-error a native text field's type is the platform's */}
            <input native fontSize={20} />
            {/* @ts-expect-error a checkbox says whether it's ticked, not its text */}
            <checkbox native onChange={(text: string) => text}>Tick</checkbox>
            {/* @ts-expect-error a slider needs `native` for now */}
            <slider value={3} />
            {/* @ts-expect-error macOS's look has nothing to say yet */}
            <button native macos={{ variant: 'push' }}>Mac</button>
        </window>
    );
}
