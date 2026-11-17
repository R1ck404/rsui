// A small app to change into yours. Run it with `rsui-tsx app.tsx`, and it changes as you save.

import { Checkbox, TextInput, useColorScheme, useState } from 'rsui';

export const title = '{{TITLE}}';
export const size = [480, 400];

interface Task {
    id: number;
    text: string;
    done: boolean;
}

export default function App() {
    const dark = useColorScheme() === 'dark';
    const page = dark ? '#14171c' : '#ffffff';
    const ink = dark ? '#e6e9ed' : '#1c2024';
    const muted = dark ? '#98a1ab' : '#60676f';

    const [tasks, setTasks] = useState<Task[]>([{ id: 1, text: 'Change this into your app', done: false }]);
    const [draft, setDraft] = useState('');
    const [nextId, setNextId] = useState(2);

    const add = () => {
        if (draft.trim() === '') {
            return;
        }
        setTasks([...tasks, { id: nextId, text: draft.trim(), done: false }]);
        setNextId(nextId + 1);
        setDraft('');
    };
    const left = tasks.filter((task) => !task.done).length;

    return (
        <window background={page} color={ink} padding={24} gap={12}>
            <text fontSize={22} fontWeight="bold">{title}</text>
            <row
                gap={8}
                onKeyPress={(key: string) => {
                    if (key === 'enter') {
                        add();
                        return true;
                    }
                    return false;
                }}
            >
                <TextInput key="draft" name="New task" flex={1} placeholder="What needs doing?" value={draft} onChange={setDraft} />
                <button key="add" paddingX={14} paddingY={7} radius={6} background="#1f6feb" color="#ffffff" onClick={add}>Add</button>
            </row>
            <scroll key="tasks" flex={1} gap={8}>
                {tasks.map((task) => (
                    <Checkbox
                        key={'task-' + task.id}
                        label={task.text}
                        checked={task.done}
                        onChange={(done: boolean) => setTasks(tasks.map((other) => (other.id === task.id ? { ...other, done } : other)))}
                    />
                ))}
            </scroll>
            <text key="left" color={muted}>{left + ' left'}</text>
        </window>
    );
}
