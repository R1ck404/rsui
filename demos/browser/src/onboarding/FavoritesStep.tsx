// The tour's favorites: a few well-known sites to start the sidebar's tiles with. Their icons come once they've been visited.

import * as I from '../icons';
import { SiteIcon } from '../ui/SiteIcon';
import { StepHeading } from './StepHeading';

export const SUGGESTED = [
    { title: 'Wikipedia', url: 'https://www.wikipedia.org/' },
    { title: 'YouTube', url: 'https://www.youtube.com/' },
    { title: 'GitHub', url: 'https://github.com/' },
    { title: 'Gmail', url: 'https://mail.google.com/' },
    { title: 'Reddit', url: 'https://www.reddit.com/' },
    { title: 'Hacker News', url: 'https://news.ycombinator.com/' },
    { title: 'Maps', url: 'https://www.openstreetmap.org/' },
    { title: 'Spotify', url: 'https://open.spotify.com/' },
    { title: 'Figma', url: 'https://www.figma.com/' },
    { title: 'Notion', url: 'https://www.notion.so/' },
    { title: 'BBC News', url: 'https://www.bbc.com/news' },
    { title: 'MDN', url: 'https://developer.mozilla.org/' },
];

export function FavoritesStep(props: { picked: Set<string>; onPicked: (picked: Set<string>) => void }) {
    const { picked } = props;
    const flip = (url: string) => {
        const next = new Set(picked);
        if (next.has(url)) next.delete(url);
        else next.add(url);
        props.onPicked(next);
    };
    return (
        <>
            <StepHeading title="Keep a few close" detail="Favorites sit at the top of the sidebar, one click away in every space." />
            <div className="mt-8 grid w-full grid-cols-4 gap-2">
                {SUGGESTED.map((site) => {
                    const on = picked.has(site.url);
                    return (
                        <button
                            key={site.url}
                            aria-pressed={on}
                            onClick={() => flip(site.url)}
                            className={
                                'relative flex flex-col items-center gap-2 rounded-(--b-radius-lg) px-2 pt-4 pb-3 transition-all duration-(--b-speed) ' +
                                (on ? 'bg-(--b-accent-soft) ring-2 ring-(--b-accent)' : 'bg-(--b-hover) hover:bg-(--b-press)')
                            }
                        >
                            <SiteIcon url={site.url} icon="" size={26} />
                            <span className="w-full truncate text-[12px] text-(--b-text)">{site.title}</span>
                            {on ? (
                                <span className="absolute top-1.5 right-1.5 flex size-4 items-center justify-center rounded-full bg-(--b-accent) text-(--b-on-accent)">
                                    <I.CheckIcon size={10} strokeWidth={2.6} />
                                </span>
                            ) : null}
                        </button>
                    );
                })}
            </div>
        </>
    );
}
