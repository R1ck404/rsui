// The same gallery as rsui's `examples/gallery.rs`, written in TSX.
//
// Run it with `cargo run --release -p rsui-tsx -- crates/rsui-tsx/examples/gallery.tsx`,
// then change something in this file and save it. What's on screen changes, and keeps its state.

import { Dialog, MenuButton, Select, TextInput, useAnimated, useState } from 'rsui';

export const title = 'rsui gallery, in TSX';

const INK = '#1f2328';
const MUTED = '#656d76';
const BLUE = '#1f6feb';
const CARD = '#ffffff';
const LINE = '#d8dee4';

const cardStyle = { padding: 16, gap: 10, background: CARD, radius: 10 };

function Heading({ children }) {
    return <text color={MUTED} fontWeight="bold">{children}</text>;
}

function Card({ name, children, flexible = false }) {
    return (
        <group {...cardStyle} align={flexible ? undefined : 'start'} flex={flexible ? 1 : undefined}>
            <Heading>{name}</Heading>
            {children}
        </group>
    );
}

function Button({ primary = false, children, ...rest }) {
    const paint = primary
        ? { background: BLUE, hover: '#388bfd', pressed: '#1558c0', color: '#ffffff' }
        : { background: LINE, hover: '#c9d1d9', pressed: '#afb8c1', color: INK };
    return <button paddingX={14} paddingY={7} radius={6} {...paint} {...rest}>{children}</button>;
}

function Counter() {
    const [count, setCount] = useState(0);
    // Each click moves this along, smoothly, a tenth of the way to full each time.
    const along = useAnimated(((count % 11) + 11) % 11 * 40, 250);
    return (
        <Card name="Counter">
            <text fontSize={20} fontWeight="bold">Clicked {count} times</text>
            <group width={along} height={4} background={BLUE} radius={2} />
            <row gap={8}>
                <Button primary key="add" onClick={() => setCount(count + 1)}>Add one</Button>
                <Button key="reset" onClick={() => setCount(0)}>Reset</Button>
            </row>
        </Card>
    );
}

function Loader() {
    const [status, setStatus] = useState('Nothing loaded yet.');
    const start = () => {
        setStatus('Loading...');
        setTimeout(() => setStatus('Loaded, without the window stuttering.'), 800);
    };
    return (
        <Card name="Background work">
            <text color={MUTED}>{status}</text>
            <Button key="load" onClick={start}>Load in the background</Button>
        </Card>
    );
}

/// Tab to the input or click it, then type. Enter anywhere in the row sends what's there, which shows a shortcut
/// belonging to one part of the UI.
function Typing() {
    const [draft, setDraft] = useState('');
    const [sent, setSent] = useState('Tab here, type, and press enter.');
    const [notes, setNotes] = useState('');
    const send = () => {
        if (draft !== '') {
            setSent('You said: ' + draft);
            setDraft('');
        }
    };
    const onKey = (key: string) => {
        if (key === 'enter') {
            send();
        }
    };
    return (
        <Card name="Typing">
            <row key="message" gap={8} onKeyPress={onKey}>
                <TextInput key="draft" name="Something to say" placeholder="Type here" value={draft} onChange={setDraft} />
                <Button primary key="send" onClick={send}>Send</Button>
            </row>
            <text color={MUTED}>{sent}</text>
            <TextInput key="notes" name="Notes" placeholder="Several lines go here" rows={3} value={notes} onChange={setNotes} />
        </Card>
    );
}

function Paragraph() {
    return (
        <Card name="Text">
            <text>
                Every glyph here came from code in this project: it read the font file, kerned the pairs (AVATAR, Type,
                Wave), broke the lines, drew the outlines and placed them in a texture. Resize the window to watch this
                paragraph wrap.
            </text>
        </Card>
    );
}

/// Letters that join, text that runs from the right, marks, and both directions on one line.
function Languages() {
    const [hebrew, setHebrew] = useState('שלום עולם');
    return (
        <group padding={16} gap={6} background={CARD} radius={10}>
            <Heading>Every direction</Heading>
            <text>العربية لغة جميلة لا إله</text>
            <text>بِسْمِ اللَّهِ الرَّحْمَنِ</text>
            <text>בְּרֵאשִׁית בָּרָא (שלום)</text>
            <text>Mixed: שלום and مرحبا 123, office, Việt</text>
            <text>漢字 かな 한글 हिन्दी தமிழ் ไทย ខ្មែរ မြန်မာ</text>
            <text>😀 👍🏽 ❤️ 1️⃣ 👨‍👩‍👧 🏳️‍🌈</text>
            <TextInput key="hebrew" name="Hebrew" value={hebrew} onChange={setHebrew} />
        </group>
    );
}

/// A menu, a dialog and a tooltip, all of which go on top of everything else.
function Popups() {
    const [dialogOpen, setDialogOpen] = useState(false);
    const [said, setSaid] = useState('Nothing chosen yet.');
    const [chosen, setChosen] = useState(0);
    const say = (text: string) => ({ label: text, onSelect: () => setSaid('You chose ' + text.toLowerCase() + '.') });
    return (
        <Card name="On top of everything">
            <row gap={8}>
                <MenuButton id="menu" label="Open a menu" items={[say('Cut'), say('Copy'), say('Paste')]} />
                <Select id="fruit" options={['Apple', 'Pear', 'Plum']} value={chosen} onChange={setChosen} />
                <Button key="open" tooltip="Dims the rest until you close it" onClick={() => setDialogOpen(true)}>
                    Open a dialog
                </Button>
                <Dialog id="about" name="About dialogs" open={dialogOpen} onClose={() => setDialogOpen(false)}>
                    <group gap={12} align="start">
                        <text>This takes the keyboard while it's open. Tab stays in here, and escape closes it.</text>
                        <Button primary key="close" onClick={() => setDialogOpen(false)}>Close</Button>
                    </group>
                </Dialog>
            </row>
            <text color={MUTED}>{said}</text>
        </Card>
    );
}

/// A large photo, which only takes as much memory as this small strip of it needs.
function Photo() {
    return (
        <image src="C:/Windows/Web/Wallpaper/Windows/img0.jpg" name="The Windows wallpaper" height={72} radius={10} fit="cover" />
    );
}

function Row({ index }) {
    return (
        <item paddingX={12} paddingY={6} background={index % 2 === 0 ? CARD : '#f6f8fa'}>
            Row number {index}
        </item>
    );
}

function renderRow(index: number) {
    return <Row index={index} />;
}

function LongList() {
    return (
        <Card name="A million rows: scroll with the wheel" flexible>
            <list key="rows" count={1000000} visible={24} rowHeight={30} flex={1} renderRow={renderRow} />
        </Card>
    );
}

export default function App() {
    return (
        <window direction="row" padding={16} gap={16}>
            {/* Make the window short to see this scroll. */}
            <scroll key="left" gap={16} flex={1}>
                <Counter />
                <Loader />
                <Typing />
                <Popups />
                <Paragraph />
            </scroll>
            <group gap={16} flex={1}>
                <Photo />
                <Languages />
                <LongList />
            </group>
        </window>
    );
}
