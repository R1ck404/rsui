// The shortcuts, as a list to read.

import { SHOWN } from '../keys';
import { Kbd } from '../ui/controls';

export function Shortcuts() {
    return (
        <div className="flex flex-col">
            {SHOWN.map((row) => (
                <div key={row.keys} className="flex h-10 items-center justify-between border-b border-(--b-line)">
                    <span className="text-(--b-text)">{row.does}</span>
                    <span className="flex gap-1">
                        {row.keys.split('+').map((key) => (
                            <Kbd key={key}>{key}</Kbd>
                        ))}
                    </span>
                </div>
            ))}
        </div>
    );
}
