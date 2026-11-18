// A site's icon, or until it has one, its first letter on a tile of a color made from its name.

import * as I from '../icons';
import { hostOf } from '../url';

/** A hue from a host's name, the same each time. */
function hueOf(host: string): number {
    let hash = 0;
    for (const char of host) {
        hash = (hash * 31 + char.charCodeAt(0)) | 0;
    }
    return Math.abs(hash) % 360;
}

export function SiteIcon(props: { url: string; icon: string; size?: number }) {
    const { url, icon, size = 16 } = props;
    if (icon !== '') {
        return <img src={icon} alt="" className="shrink-0 rounded-[3px]" style={{ width: size, height: size }} />;
    }
    const host = hostOf(url);
    if (host === '') {
        return (
            <span className="flex shrink-0 text-(--b-faint)">
                <I.GlobeIcon size={size} strokeWidth={1.6} />
            </span>
        );
    }
    return (
        <span
            className="flex shrink-0 items-center justify-center rounded-[4px] font-semibold text-white"
            style={{ width: size, height: size, fontSize: Math.round(size * 0.6), background: `hsl(${hueOf(host)} 45% 58%)` }}
        >
            {host[0].toUpperCase()}
        </span>
    );
}
