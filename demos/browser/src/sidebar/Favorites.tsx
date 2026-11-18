// The favorites, as tiles along the top of the sidebar, the same in every space. The site that's open is lifted.

import type { Favorite } from '../model';
import { SiteIcon } from '../ui/SiteIcon';
import { hostOf } from '../url';

export function Favorites(props: {
    favorites: Favorite[];
    /** The open page's address, whose site's tile is lifted. */
    openUrl: string;
    onOpen: (favorite: Favorite) => void;
    onMenu: (favorite: Favorite) => void;
}) {
    const { favorites, openUrl } = props;
    if (favorites.length === 0) return null;
    const open = openUrl === '' ? '' : hostOf(openUrl);
    return (
        <div className="grid grid-cols-4 gap-1.5 pt-1">
            {favorites.map((favorite) => (
                <button
                    key={favorite.id}
                    title={favorite.title || hostOf(favorite.url)}
                    onClick={() => props.onOpen(favorite)}
                    onContextMenu={() => props.onMenu(favorite)}
                    className={
                        'flex h-11 items-center justify-center rounded-(--b-radius) transition-colors duration-(--b-speed) ' +
                        (hostOf(favorite.url) === open ? 'bg-(--b-tab) shadow-(--b-tab-shadow)' : 'bg-(--b-hover) hover:bg-(--b-press)')
                    }
                >
                    <SiteIcon url={favorite.url} icon={favorite.icon} size={18} />
                </button>
            ))}
        </div>
    );
}
