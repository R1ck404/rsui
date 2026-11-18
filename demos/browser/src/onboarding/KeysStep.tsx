// The tour's last page: the few shortcuts worth knowing from the start.

import { Kbd } from '../ui/controls';
import { StepHeading } from './StepHeading';

const KEYS = [
    { keys: ['Ctrl', 'T'], does: 'Search, or open anything' },
    { keys: ['Ctrl', 'L'], does: 'Change where this tab goes' },
    { keys: ['Ctrl', 'S'], does: 'Hide the sidebar for more room' },
    { keys: ['Ctrl', 'D'], does: 'Pin a tab so it stays' },
    { keys: ['Ctrl', 'Alt', '→'], does: 'Move to your next space' },
];

export function KeysStep() {
    return (
        <>
            <StepHeading title="Everything's a keystroke away" detail="These work wherever you are, even inside a page." />
            <div className="mt-8 flex w-full flex-col gap-1.5">
                {KEYS.map((row) => (
                    <div key={row.does} className="flex h-11 items-center justify-between rounded-(--b-radius) bg-(--b-hover) px-4">
                        <span className="text-[13.5px] text-(--b-text)">{row.does}</span>
                        <span className="flex gap-1">
                            {row.keys.map((key) => (
                                <Kbd key={key}>{key}</Kbd>
                            ))}
                        </span>
                    </div>
                ))}
            </div>
        </>
    );
}
