// General settings: the search engine, what a new tab opens, how the address shows, and when tabs sleep.

import type { Settings } from '../model';
import { Segmented, Toggle } from '../ui/controls';
import { ENGINES } from '../url';
import { Chip, Row } from './parts';

export function General(props: { settings: Settings; onSettings: (changes: Partial<Settings>) => void }) {
    const { settings, onSettings: set } = props;
    return (
        <>
            <Row title="Search engine" detail="What's typed that isn't an address is searched for here.">
                <div className="flex max-w-[300px] flex-wrap justify-end gap-1">
                    {ENGINES.map((engine) => (
                        <Chip key={engine.id} on={settings.engine === engine.id} onClick={() => set({ engine: engine.id })}>
                            {engine.name}
                        </Chip>
                    ))}
                </div>
            </Row>
            <Row title="New tab" detail="What Ctrl+T and the sidebar's New Tab open.">
                <Segmented
                    value={settings.newTab}
                    onChange={(newTab) => set({ newTab })}
                    options={[
                        { value: 'command', label: 'Command bar' },
                        { value: 'page', label: 'Start page' },
                    ]}
                />
            </Row>
            <Row title="Show the whole address" detail="Rather than only the site's name, in the sidebar.">
                <Toggle label="Show the whole address" on={settings.showFullUrl} onChange={(showFullUrl) => set({ showFullUrl })} />
            </Row>
            <Row title="Put tabs to sleep" detail="A tab that hasn't been looked at for this long lets its page go, and loads it again when it's opened.">
                <Segmented
                    value={String(settings.sleepAfter)}
                    onChange={(value) => set({ sleepAfter: Number(value) })}
                    options={[
                        { value: '15', label: '15 min' },
                        { value: '30', label: '30 min' },
                        { value: '120', label: '2 h' },
                        { value: '0', label: 'Never' },
                    ]}
                />
            </Row>
        </>
    );
}
