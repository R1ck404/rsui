// Colors and sizes from the settings, as CSS variables. Tailwind's classes use them as `bg-(--fx-hover)` and the like, so a
// change of theme or accent only changes these variables, and nothing has to render again for it.
//
// The window is a frame with panels on it, as Arc's is: the frame is tinted with the accent and, with a material, lets the desktop
// show through, and the panels on it are solid cards. Hovers and lines are the text color at a low opacity, so they look the same
// on the frame and on a card, in either theme.

import type { Settings } from './settings';

const PALETTES: Record<Settings['palette'], string[]> = {
    // 50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950, as Tailwind has them.
    zinc: ['#fafafa', '#f4f4f5', '#e4e4e7', '#d4d4d8', '#a1a1aa', '#71717a', '#52525b', '#3f3f46', '#27272a', '#18181b', '#09090b'],
    slate: ['#f8fafc', '#f1f5f9', '#e2e8f0', '#cbd5e1', '#94a3b8', '#64748b', '#475569', '#334155', '#1e293b', '#0f172a', '#020617'],
    stone: ['#fafaf9', '#f5f5f4', '#e7e5e4', '#d6d3d1', '#a8a29e', '#78716c', '#57534e', '#44403c', '#292524', '#1c1917', '#0c0a09'],
    neutral: ['#fafafa', '#f5f5f5', '#e5e5e5', '#d4d4d4', '#a3a3a3', '#737373', '#525252', '#404040', '#262626', '#171717', '#0a0a0a'],
    gray: ['#f9fafb', '#f3f4f6', '#e5e7eb', '#d1d5db', '#9ca3af', '#6b7280', '#4b5563', '#374151', '#1f2937', '#111827', '#030712'],
};

const RADII = { none: 0, small: 5, medium: 7, large: 10 };
const ROWS = { compact: 26, cozy: 30, comfortable: 36 };

/** A color with some of it see-through, as `#rrggbbaa`, which both CSS and rsui's own elements read. */
export function rgba(hex: string, alpha: number): string {
    const base = hex.replace('#', '').padEnd(6, '0').slice(0, 6);
    return '#' + base + Math.round(Math.max(0, Math.min(1, alpha)) * 255).toString(16).padStart(2, '0');
}

/** `from` with `share` of `to` in it. */
export function mix(from: string, to: string, share: number): string {
    const read = (hex: string) => {
        const text = hex.replace('#', '');
        return [0, 2, 4].map((at) => parseInt(text.slice(at, at + 2), 16) || 0);
    };
    const [a, b] = [read(from), read(to)];
    return '#' + a.map((value, at) => Math.round(value + (b[at] - value) * share).toString(16).padStart(2, '0')).join('');
}

/** How tall a row of the list is, in pixels. */
export function rowHeight(settings: Settings): number {
    return Math.round(ROWS[settings.density] + (settings.fontSize - 13) * 1.5);
}

export function gridTile(settings: Settings): { width: number; height: number; icon: number } {
    const icon = { small: 32, medium: 48, large: 72, huge: 112 }[settings.gridSize];
    return { width: icon + 56, height: icon + Math.round(settings.fontSize * 3.4) + 20, icon };
}

export type Colors = Record<
    | 'frame' | 'card' | 'pop' | 'app' | 'side' | 'panel' | 'text' | 'muted' | 'faint' | 'line' | 'card-line' | 'hover' | 'press' | 'zebra'
    | 'input' | 'accent' | 'accent-soft' | 'accent-text' | 'shadow' | 'tab' | 'tab-hover' | 'chip' | 'select',
    string
>;

/** The neutral grays the panels are, with only a little of the palette's own hue in them, so that a blue-gray palette doesn't make
 * them navy. */
const DARK_CARD = '#1d1d20';
const DARK_FRAME = '#111113';
const LIGHT_FRAME = '#e8e8eb';

/** Every color the app is drawn in, from the theme, the palette, the accent and the window's material. */
export function colorsOf(settings: Settings, dark: boolean): Colors {
    const g = PALETTES[settings.palette];
    const material = settings.backdrop !== 'none';
    const tint = settings.frameTint / 100;
    const see = settings.translucency / 100;
    // With a material, the desktop shows through the frame, and through the panels too, less, so the blur runs through the whole
    // window rather than stopping at the panels' edges. Neither is ever quite gone, so text on them stays readable.
    const frameAlpha = material ? 1 - see * 0.82 : 1;
    const cardAlpha = material ? 1 - see * 0.5 : 1;
    if (dark) {
        const card = mix(DARK_CARD, g[9], 0.3);
        const frame = mix(mix(DARK_FRAME, g[10], 0.3), settings.accent, tint * 0.55);
        return {
            frame: rgba(frame, frameAlpha),
            card: rgba(card, cardAlpha),
            // Popovers are solid, as what's under them showing through them faintly only looks like a mistake.
            pop: mix(card, '#ffffff', 0.03),
            app: card,
            side: '#00000000',
            panel: mix(card, '#ffffff', 0.04),
            text: g[1],
            muted: g[4],
            faint: g[5],
            line: '#ffffff12',
            'card-line': '#ffffff0f',
            hover: '#ffffff0f',
            press: '#ffffff17',
            zebra: '#ffffff06',
            input: '#ffffff0d',
            accent: settings.accent,
            'accent-soft': rgba(settings.accent, 0.22),
            // What's selected, as a tint of the accent rather than all of it, so it isn't heavier than what's around it.
            select: rgba(settings.accent, 0.4),
            'accent-text': '#ffffff',
            shadow: '#00000073',
            tab: rgba(mix(card, '#ffffff', 0.07), material ? Math.min(1, cardAlpha + 0.15) : 1),
            'tab-hover': '#ffffff0d',
            // What's chosen in a row of choices, which shows on a panel however see-through that is.
            chip: '#ffffff21',
        };
    }
    const frame = mix(mix(LIGHT_FRAME, g[2], 0.3), settings.accent, tint * 0.6);
    return {
        frame: rgba(frame, frameAlpha),
        card: rgba('#ffffff', cardAlpha),
        pop: '#ffffff',
        app: '#ffffff',
        side: '#ffffff00',
        panel: g[0],
        text: g[9],
        muted: g[5],
        faint: g[4],
        line: '#0000000f',
        'card-line': '#0000000d',
        hover: '#0000000a',
        press: '#00000012',
        zebra: '#00000005',
        input: '#0000000a',
        accent: settings.accent,
        'accent-soft': rgba(settings.accent, 0.13),
        select: rgba(settings.accent, 0.28),
        'accent-text': '#ffffff',
        shadow: '#0f172a24',
        tab: rgba('#ffffff', material ? Math.min(1, cardAlpha + 0.15) : 1),
        chip: '#ffffff',
        'tab-hover': '#ffffff8c',
    };
}

export const radiusOf = (settings: Settings): number => RADII[settings.radius];

export function cssVariables(settings: Settings, colors: Colors): string {
    const vars: Record<string, string> = { ...colors };
    const radius = RADII[settings.radius];
    vars.radius = radius + 'px';
    vars['radius-lg'] = Math.round(radius * 1.6) + 'px';
    vars.row = rowHeight(settings) + 'px';
    vars.font = fontStack(settings);
    vars['font-size'] = settings.fontSize + 'px';
    vars.speed = settings.animations ? '140ms' : '0ms';
    // How long panels take to move to new places and sizes, as when a pane's split or the sidebar collapses, and how they ease
    // into them: quickly away, then settling, as things on a Mac do.
    vars.move = settings.animations ? '260ms' : '0ms';
    vars.ease = 'cubic-bezier(0.22, 1, 0.36, 1)';
    vars.gap = settings.panelGap + 'px';
    vars['card-shadow'] = settings.panelShadow ? `0 1px 2px ${colors.shadow}, 0 0 0 1px ${colors['card-line']}` : `0 0 0 1px ${colors['card-line']}`;
    vars['tab-shadow'] = settings.panelShadow ? `0 1px 2px ${colors.shadow}` : 'none';
    vars['pop-shadow'] = `0 12px 32px ${colors.shadow}, 0 0 0 1px ${colors['card-line']}`;
    vars.body = settings.backdrop === 'none' ? colors.frame : 'transparent';
    let text = ':root {\n';
    for (const [name, value] of Object.entries(vars)) {
        text += `  --fx-${name}: ${value};\n`;
    }
    return text + '}\n';
}

export const fontStack = (settings: Settings) => `"${settings.fontFamily}", "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif`;

let sheet: { textContent: string } | null = null;

/** Puts the variables in one `<style>` of their own, changing what it says from then on. */
export function applyVariables(text: string): void {
    if (sheet === null) {
        const element = (document as any).createElement('style');
        element.textContent = text;
        (document as any).head.appendChild(element);
        sheet = element;
    } else if (sheet.textContent !== text) {
        sheet.textContent = text;
    }
}
