// Spaces: each one's name and color, whether its sign-ins are kept apart, and deleting it.

import * as I from '../icons';
import { type Space, SPACE_COLORS } from '../model';
import { Toggle } from '../ui/controls';
import { IconButton } from '../ui/IconButton';

export function Spaces(props: { spaces: Space[]; onChange: (id: string, changes: Partial<Space>) => void; onDelete: (id: string) => void }) {
    const { spaces } = props;
    return (
        <>
            <p className="mb-2 text-[12.5px] leading-[1.5] text-(--b-muted)">
                Each space has its own tabs and color. One kept apart has its own cookies and sign-ins too, so a work account stays out of a
                personal one.
            </p>
            {spaces.map((space) => (
                <SpaceRow
                    key={space.id}
                    space={space}
                    only={spaces.length === 1}
                    onChange={(changes) => props.onChange(space.id, changes)}
                    onDelete={() => props.onDelete(space.id)}
                />
            ))}
        </>
    );
}

function SpaceRow(props: { space: Space; only: boolean; onChange: (changes: Partial<Space>) => void; onDelete: () => void }) {
    const { space, only } = props;
    return (
        <div className="flex flex-col gap-3 border-b border-(--b-line) py-4">
            <div className="flex items-center gap-3">
                <span className="size-3 shrink-0 rounded-full" style={{ background: space.color }} />
                <input
                    value={space.name}
                    aria-label="Name"
                    onChange={(event: any) => props.onChange({ name: event.target.value })}
                    className="h-8 min-w-0 flex-1 rounded-(--b-radius-sm) bg-(--b-hover) px-2.5 text-[13px] font-medium text-(--b-text) outline-none focus:bg-(--b-press)"
                />
                {only ? null : <IconButton icon={I.TrashIcon} label={'Delete ' + space.name} onClick={props.onDelete} />}
            </div>
            <div className="flex items-center justify-between gap-4 pl-6">
                <div className="flex gap-1.5">
                    {SPACE_COLORS.map((one) => (
                        <button
                            key={one.color}
                            title={one.name}
                            aria-label={one.name}
                            aria-pressed={one.color === space.color}
                            onClick={() => props.onChange({ color: one.color })}
                            className="size-5 rounded-full transition-transform duration-(--b-speed) hover:scale-110"
                            style={{
                                background: one.color,
                                boxShadow: one.color === space.color ? `0 0 0 2px var(--b-raised), 0 0 0 3.5px ${one.color}` : undefined,
                            }}
                        />
                    ))}
                </div>
                <label className="flex items-center gap-2 text-[12px] text-(--b-muted)">
                    Keep sign-ins apart
                    <Toggle label="Keep sign-ins apart" on={space.apart} onChange={(apart) => props.onChange({ apart })} />
                </label>
            </div>
        </div>
    );
}
