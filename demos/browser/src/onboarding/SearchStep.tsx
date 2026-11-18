// The tour's search engine: where what's typed that isn't an address goes.

import * as I from '../icons';
import { SiteIcon } from '../ui/SiteIcon';
import { ENGINES } from '../url';
import { StepHeading } from './StepHeading';

export function SearchStep(props: { engine: string; onEngine: (engine: string) => void }) {
    return (
        <>
            <StepHeading title="Search with" detail="What you type that isn't an address goes here." />
            <div className="mt-8 grid w-full grid-cols-2 gap-2">
                {ENGINES.map((engine) => {
                    const picked = props.engine === engine.id;
                    return (
                        <button
                            key={engine.id}
                            aria-pressed={picked}
                            onClick={() => props.onEngine(engine.id)}
                            className={
                                'flex h-12 items-center gap-3 rounded-(--b-radius) px-4 text-left transition-all duration-(--b-speed) ' +
                                (picked ? 'bg-(--b-accent-soft) ring-2 ring-(--b-accent)' : 'bg-(--b-hover) hover:bg-(--b-press)')
                            }
                        >
                            <SiteIcon url={engine.search} icon="" size={18} />
                            <span className="flex-1 text-[13.5px] font-medium text-(--b-text)">{engine.name}</span>
                            {picked ? (
                                <span className="flex text-(--b-accent)">
                                    <I.CheckIcon size={15} strokeWidth={2.2} />
                                </span>
                            ) : null}
                        </button>
                    );
                })}
            </div>
        </>
    );
}
