// The browser's colors and sizes, as CSS variables that Tailwind's classes read (`bg-(--b-card)`), so a change of space or
// theme only changes these, and nothing renders again for it.
//
// It's white first: the window is a soft frame, tinted with the open space's color and, with a material, showing the desktop
// through it, and the page sits on it as a white card. Hovers and lines are the text's color, faint, so they look right on
// either.

import type { Settings } from './model';

/** `#rrggbb` with `alpha` of it showing, as `#rrggbbaa`. */
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

/** How light a color looks, from 0 to 1. */
function lightness(hex: string): number {
    const text = hex.replace('#', '');
    const [r, g, b] = [0, 2, 4].map((at) => (parseInt(text.slice(at, at + 2), 16) || 0) / 255);
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function variables(settings: Settings, dark: boolean, accent: string): Record<string, string> {
    const material = settings.material !== 'none';
    const tint = settings.tint / 100;
    const round = settings.corners === 'round';
    const base = dark
        ? {
            frame: mix('#131316', accent, 0.05 + tint * 0.12),
            card: '#1b1b1f',
            raised: '#242429',
            text: '#ececf0',
            muted: '#a0a0aa',
            faint: '#6c6c76',
            ink: '#ffffff',
        }
        : {
            frame: mix('#eceef2', accent, 0.06 + tint * 0.3),
            card: '#ffffff',
            raised: '#ffffff',
            text: '#1a1a1e',
            muted: '#6e6e78',
            faint: '#a3a3ad',
            ink: '#000000',
        };
    const on = dark ? '#ffffff' : '#000000';
    return {
        frame: material ? rgba(base.frame, dark ? 0.72 : 0.66) : base.frame,
        card: base.card,
        raised: base.raised,
        text: base.text,
        muted: base.muted,
        faint: base.faint,
        hover: rgba(on, dark ? 0.07 : 0.05),
        press: rgba(on, dark ? 0.11 : 0.08),
        line: rgba(on, dark ? 0.09 : 0.07),
        // The open tab in the sidebar: a lifted white pill on the frame, as Arc has it.
        tab: dark ? rgba('#ffffff', 0.1) : rgba('#ffffff', 0.86),
        'tab-shadow': dark ? '0 1px 2px #00000040' : '0 1px 2px #0000000f, 0 0 0 0.5px #0000000a',
        accent,
        'accent-soft': rgba(accent, dark ? 0.22 : 0.14),
        // Text on the accent, dark on a light one.
        'on-accent': lightness(accent) > 0.62 ? '#1a1a1e' : '#ffffff',
        // What dims everything behind the command bar and settings.
        scrim: dark ? rgba('#000000', 0.45) : rgba('#14141c', 0.22),
        'card-shadow': dark ? '0 0 0 1px #ffffff0d, 0 8px 24px #00000040' : '0 0 0 1px #0000000a, 0 1px 3px #0000000d, 0 10px 30px #0000000a',
        'pop-shadow': dark
            ? '0 0 0 1px #ffffff12, 0 16px 48px #00000070'
            : '0 0 0 1px #0000000d, 0 4px 12px #00000010, 0 24px 60px #00000024',
        radius: round ? '12px' : '9px',
        'radius-sm': round ? '9px' : '7px',
        'radius-lg': round ? '18px' : '13px',
        speed: settings.animations ? '160ms' : '0ms',
        move: settings.animations ? '320ms' : '0ms',
        ease: 'cubic-bezier(0.22, 1, 0.36, 1)',
    };
}

let sheet: any = null;

/** Puts the variables on the page, replacing what was there. */
export function apply(vars: Record<string, string>): void {
    const text = ':root{' + Object.entries(vars).map(([name, value]) => `--b-${name}:${value};`).join('') + '}';
    if (sheet === null) {
        sheet = (document as any).createElement('style');
        sheet.textContent = text;
        (document as any).head.appendChild(sheet);
    } else if (sheet.textContent !== text) {
        sheet.textContent = text;
    }
}
