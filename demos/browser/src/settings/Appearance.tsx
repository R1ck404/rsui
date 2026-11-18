// Appearance: theme, the frame's material and tint, corners, which side the sidebar's on, and animations.

import type { Settings } from '../model';
import { Segmented, Toggle } from '../ui/controls';
import { Row } from './parts';

/** The tints offered, by how much of the space's color the frame takes. */
const TINTS = { none: 0, soft: 45, rich: 90 } as const;

function tintOf(tint: number): keyof typeof TINTS {
    return tint <= 10 ? 'none' : tint <= 55 ? 'soft' : 'rich';
}

export function Appearance(props: { settings: Settings; onSettings: (changes: Partial<Settings>) => void }) {
    const { settings, onSettings: set } = props;
    return (
        <>
            <Row title="Theme">
                <Segmented
                    value={settings.theme}
                    onChange={(theme) => set({ theme })}
                    options={[
                        { value: 'light', label: 'Light' },
                        { value: 'dark', label: 'Dark' },
                        { value: 'system', label: 'Automatic' },
                    ]}
                />
            </Row>
            <Row title="Frame" detail="Mica and Acrylic let the desktop show through, blurred, on Windows 11.">
                <Segmented
                    value={settings.material}
                    onChange={(material) => set({ material })}
                    options={[
                        { value: 'mica', label: 'Mica' },
                        { value: 'acrylic', label: 'Acrylic' },
                        { value: 'none', label: 'Solid' },
                    ]}
                />
            </Row>
            <Row title="Tint" detail="How much of the space's color the frame takes.">
                <Segmented
                    value={tintOf(settings.tint)}
                    onChange={(value) => set({ tint: TINTS[value] })}
                    options={[
                        { value: 'none', label: 'None' },
                        { value: 'soft', label: 'Soft' },
                        { value: 'rich', label: 'Rich' },
                    ]}
                />
            </Row>
            <Row title="Corners">
                <Segmented
                    value={settings.corners}
                    onChange={(corners) => set({ corners })}
                    options={[
                        { value: 'soft', label: 'Soft' },
                        { value: 'round', label: 'Round' },
                    ]}
                />
            </Row>
            <Row title="Sidebar" detail="Which side it's on. Ctrl+S hides it.">
                <Segmented
                    value={settings.sidebarSide}
                    onChange={(sidebarSide) => set({ sidebarSide })}
                    options={[
                        { value: 'left', label: 'Left' },
                        { value: 'right', label: 'Right' },
                    ]}
                />
            </Row>
            <Row title="Animations">
                <Toggle label="Animations" on={settings.animations} onChange={(animations) => set({ animations })} />
            </Row>
        </>
    );
}
