// What a cell looks like, stored once and pointed at: the terminal keeps a table of styles, and each cell holds its number.
// A color is 0 for the theme's default, 0x01000000 + n for palette color n, or 0x02000000 + 0xRRGGBB.

export const BOLD = 1;
export const DIM = 2;
export const ITALIC = 4;
export const BLINK = 8;
export const REVERSE = 16;
export const HIDDEN = 32;
export const STRIKE = 64;
export const OVERLINE = 128;
/** Bits 8 to 10: 0 none, 1 single, 2 double, 3 curly, 4 dotted, 5 dashed. */
export const UNDERLINE_SHIFT = 8;
export const UNDERLINE_MASK = 7 << UNDERLINE_SHIFT;

export const PALETTE = 0x01000000;
export const RGB = 0x02000000;

export function paletteColor(index: number): number {
    return PALETTE | index;
}

export function rgbColor(r: number, g: number, b: number): number {
    return RGB | ((r & 255) << 16) | ((g & 255) << 8) | (b & 255);
}

export interface Style {
    fg: number;
    bg: number;
    flags: number;
    /** The underline's color, as the others. */
    ul: number;
    /** A hyperlink's number, or 0. */
    link: number;
}

export class StyleTable {
    readonly styles: Style[] = [{ fg: 0, bg: 0, flags: 0, ul: 0, link: 0 }];
    #index = new Map<string, number>([['0,0,0,0,0', 0]]);

    intern(fg: number, bg: number, flags: number, ul: number, link: number): number {
        const key = `${fg},${bg},${flags},${ul},${link}`;
        let id = this.#index.get(key);
        if (id === undefined) {
            id = this.styles.length;
            this.styles.push({ fg, bg, flags, ul, link });
            this.#index.set(key, id);
        }
        return id;
    }

    get(id: number): Style {
        return this.styles[id] ?? this.styles[0];
    }

    copyFrom(source: StyleTable) {
        for (let id = 1; id < source.styles.length; id++) {
            const { fg, bg, flags, ul, link } = source.styles[id];
            this.intern(fg, bg, flags, ul, link);
        }
    }
}
