// The first run: a short tour that sets the browser up as it goes. Each step changes the real settings, so what's picked shows
// at once, behind it and in it. Pages come in from the side they're on, and the dots say how far along it is.

import { useState } from 'react';
import * as I from '../icons';
import { type Favorite, newId, type Settings } from '../model';
import { FavoritesStep, SUGGESTED } from './FavoritesStep';
import { KeysStep } from './KeysStep';
import { LookStep } from './LookStep';
import { SearchStep } from './SearchStep';
import { Welcome } from './Welcome';

const STEPS = ['welcome', 'look', 'search', 'favorites', 'keys'] as const;

/** The favorites picked to start with, until they're changed. */
const FIRST_PICKS = ['https://www.wikipedia.org/', 'https://github.com/', 'https://www.youtube.com/'];

export function Onboarding(props: {
    settings: Settings;
    color: string;
    onSettings: (changes: Partial<Settings>) => void;
    onColor: (color: string) => void;
    onDone: (favorites: Favorite[]) => void;
}) {
    const { settings, color } = props;
    const [step, setStep] = useState(0);
    const [forward, setForward] = useState(true);
    const [picked, setPicked] = useState<Set<string>>(() => new Set(FIRST_PICKS));
    const go = (to: number) => {
        setForward(to > step);
        setStep(to);
    };
    const last = step === STEPS.length - 1;
    const finish = () =>
        props.onDone(SUGGESTED.filter((site) => picked.has(site.url)).map((site) => ({ id: newId(), url: site.url, title: site.title, icon: '' })));
    const name = STEPS[step];
    return (
        <div className="absolute inset-0 z-50 flex flex-col items-center justify-center px-6">
            <div
                key={name}
                className={
                    'flex w-full max-w-[520px] flex-col items-center text-center ' +
                    (forward ? 'animate-[enter-from-right_var(--b-move)_var(--b-ease)]' : 'animate-[enter-from-left_var(--b-move)_var(--b-ease)]')
                }
            >
                {name === 'welcome' ? (
                    <Welcome color={color} />
                ) : name === 'look' ? (
                    <LookStep settings={settings} color={color} onSettings={props.onSettings} onColor={props.onColor} />
                ) : name === 'search' ? (
                    <SearchStep engine={settings.engine} onEngine={(engine) => props.onSettings({ engine })} />
                ) : name === 'favorites' ? (
                    <FavoritesStep picked={picked} onPicked={setPicked} />
                ) : (
                    <KeysStep />
                )}
            </div>
            <div className="absolute bottom-10 grid w-full max-w-[520px] grid-cols-[1fr_auto_1fr] items-center px-6">
                <button
                    onClick={() => go(step - 1)}
                    className={
                        'h-9 justify-self-start rounded-(--b-radius) px-3 text-[13px] text-(--b-muted) transition-colors duration-(--b-speed) hover:bg-(--b-hover) hover:text-(--b-text) ' +
                        (step === 0 ? 'invisible' : '')
                    }
                >
                    Back
                </button>
                <div className="flex items-center gap-1.5">
                    {STEPS.map((one, index) => (
                        <span
                            key={one}
                            className={'h-1.5 rounded-full transition-all duration-(--b-move) ' + (index === step ? 'w-[18px]' : 'w-1.5 bg-(--b-press)')}
                            style={index === step ? { background: color } : undefined}
                        />
                    ))}
                </div>
                <button
                    onClick={() => (last ? finish() : go(step + 1))}
                    className="flex h-9 items-center gap-1.5 justify-self-end rounded-(--b-radius) bg-(--b-text) px-4 text-[13px] font-medium text-(--b-card) transition-opacity duration-(--b-speed) hover:opacity-90 active:opacity-80"
                >
                    {step === 0 ? 'Get started' : last ? 'Start browsing' : 'Continue'}
                    <I.ChevronRightIcon size={13} strokeWidth={2.2} />
                </button>
            </div>
        </div>
    );
}
