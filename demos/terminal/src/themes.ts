// Color schemes. Each is the terminal's colors (background, text, cursor, selection and the 16 ANSI colors) and the shade the
// window's frame takes, which sits behind the columns and shows through Mica or Acrylic.

export interface Scheme {
    id: string;
    name: string;
    dark: boolean;
    bg: number;
    fg: number;
    cursor: number;
    selection: number;
    /** The frame behind the columns. */
    frame: number;
    ansi: number[];
}

export function tokens(scheme: Scheme) {
    return {
        frame: css(scheme.frame),
        panel: css(scheme.bg),
        text: withAlpha(scheme.fg, 0.92),
        sub: withAlpha(scheme.fg, 0.56),
        faint: withAlpha(scheme.fg, 0.34),
        hover: withAlpha(scheme.fg, 0.055),
        chosen: withAlpha(scheme.fg, 0.085),
        border: withAlpha(scheme.fg, 0.07),
    };
}

const s = (
    id: string, name: string, dark: boolean, bg: number, fg: number, cursor: number, selection: number, frame: number, ansi: number[],
): Scheme => ({ id, name, dark, bg, fg, cursor, selection, frame, ansi });

export const SCHEMES: Scheme[] = [
    // Neutral greys with soft colors: the panel a step lighter than the frame, as a quiet app's is.
    s('graphite', 'Graphite', true, 0x161618, 0xe4e4e7, 0xe4e4e7, 0x34343a, 0x0a0a0b, [
        0x232326, 0xf07178, 0x9fd39b, 0xe6c07b, 0x82aaff, 0xc3a6ff, 0x89ddff, 0xd4d4d8,
        0x5c5c63, 0xff8a91, 0xb5e8b0, 0xf2d49b, 0xa3c0ff, 0xd5bfff, 0xa8e8ff, 0xfafafa,
    ]),
    s('ink', 'Ink', true, 0x0f1017, 0xdcdfe8, 0xdcdfe8, 0x2a2f45, 0x07080c, [
        0x1b1d27, 0xf28b82, 0x8fd19e, 0xe8c27a, 0x7ea6ff, 0xb69cff, 0x7fd0f0, 0xc9cdd8,
        0x4d5263, 0xff9e96, 0xa8e6b4, 0xf4d595, 0x9dbbff, 0xcab4ff, 0x9fe0f7, 0xf2f4f8,
    ]),
    s('midnight', 'Midnight', true, 0x0f1117, 0xd7dae0, 0x7aa2f7, 0x2a3350, 0x0a0c11, [
        0x1b1e28, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
        0x414868, 0xff899d, 0xb9f27c, 0xffc777, 0x8db7ff, 0xcaa9fa, 0xa4daff, 0xe5e9ff,
    ]),
    s('mocha', 'Catppuccin Mocha', true, 0x1e1e2e, 0xcdd6f4, 0xf5e0dc, 0x45475a, 0x151521, [
        0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
        0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
    ]),
    s('latte', 'Catppuccin Latte', false, 0xeff1f5, 0x4c4f69, 0xdc8a78, 0xccd0da, 0xe3e6ec, [
        0x5c5f77, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xacb0be,
        0x6c6f85, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xbcc0cc,
    ]),
    s('tokyo', 'Tokyo Night', true, 0x1a1b26, 0xc0caf5, 0xc0caf5, 0x33467c, 0x14151e, [
        0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
        0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
    ]),
    s('dracula', 'Dracula', true, 0x282a36, 0xf8f8f2, 0xf8f8f2, 0x44475a, 0x1e1f29, [
        0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2,
        0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
    ]),
    s('gruvbox', 'Gruvbox Dark', true, 0x282828, 0xebdbb2, 0xebdbb2, 0x504945, 0x1d2021, [
        0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984,
        0x928374, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
    ]),
    s('nord', 'Nord', true, 0x2e3440, 0xd8dee9, 0xd8dee9, 0x434c5e, 0x242933, [
        0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0,
        0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
    ]),
    s('rosepine', 'Rosé Pine', true, 0x191724, 0xe0def4, 0x524f67, 0x403d52, 0x12101c, [
        0x26233a, 0xeb6f92, 0x31748f, 0xf6c177, 0x9ccfd8, 0xc4a7e7, 0xebbcba, 0xe0def4,
        0x6e6a86, 0xeb6f92, 0x31748f, 0xf6c177, 0x9ccfd8, 0xc4a7e7, 0xebbcba, 0xe0def4,
    ]),
    s('onedark', 'One Dark', true, 0x282c34, 0xabb2bf, 0x528bff, 0x3e4451, 0x1e2127, [
        0x282c34, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf,
        0x5c6370, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xffffff,
    ]),
    s('solarized', 'Solarized Dark', true, 0x002b36, 0x93a1a1, 0x93a1a1, 0x073642, 0x00212b, [
        0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
        0x586e75, 0xcb4b16, 0x859900, 0xb58900, 0x268bd2, 0x6c71c4, 0x2aa198, 0xfdf6e3,
    ]),
    s('paper', 'Paper', false, 0xfbfbfa, 0x2b2b2b, 0x2b2b2b, 0xd9e2f3, 0xeeeeec, [
        0x2b2b2b, 0xc41a16, 0x1b7f2a, 0x9a6700, 0x1f5fbf, 0x8e3b9b, 0x0e7c86, 0xbfbfbf,
        0x6b6b6b, 0xde3a36, 0x2a9d3c, 0xb8860b, 0x3b7bdc, 0xa855b6, 0x1895a1, 0xffffff,
    ]),
    s('matrix', 'Matrix', true, 0x020a04, 0x7dff9a, 0x7dff9a, 0x0f3a1a, 0x010603, [
        0x020a04, 0x0f8a3a, 0x1fd25a, 0x9aff6a, 0x1a9c5a, 0x2fd28a, 0x5affc8, 0x9bffb4,
        0x0e3b1c, 0x2fe06a, 0x4dff85, 0xc6ff8a, 0x4fd68a, 0x6bffb0, 0x8affde, 0xd3ffdf,
    ]),
];

export function schemeById(id: string): Scheme {
    return SCHEMES.find((scheme) => scheme.id === id) ?? SCHEMES[0];
}

export const SCHEME_COLORS = { bg: 'Background', fg: 'Text', cursor: 'Cursor', selection: 'Selection', frame: 'Window frame' };
export type SchemeColor = keyof typeof SCHEME_COLORS;
export type SchemeOverrides = Partial<Record<SchemeColor, number>> & { ansi?: Partial<Record<number, number>> };

export function resolveScheme(id: string, overrides?: SchemeOverrides): Scheme {
    const scheme = schemeById(id);
    if (!overrides) return scheme;
    const { ansi, ...colors } = overrides;
    return { ...scheme, ...colors, ansi: scheme.ansi.map((color, i) => ansi?.[i] ?? color) };
}

export const css = (rgb: number): string => '#' + (rgb & 0xffffff).toString(16).padStart(6, '0');

/** A color mixed toward another, `amount` from 0 to 1. */
export function mix(a: number, b: number, amount: number): number {
    const ch = (shift: number) => Math.round(((a >> shift) & 255) * (1 - amount) + ((b >> shift) & 255) * amount);
    return (ch(16) << 16) | (ch(8) << 8) | ch(0);
}

/** A color with an opacity, as the `#rrggbbaa` rsui and CSS both read. */
export function withAlpha(rgb: number, alpha: number): string {
    return css(rgb) + Math.round(Math.max(0, Math.min(1, alpha)) * 255).toString(16).padStart(2, '0');
}
