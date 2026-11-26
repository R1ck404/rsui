// One row of the terminal: the cells' backgrounds, the selection and search matches under the text, the text in runs that each
// have a width of their own so a glyph from another font can't push the rest along, and underlines and strikes over it.
//
// A row only draws again when its line changed, or what's drawn on it did: those are all the props, and each is a number or a
// string, so rsui can tell for itself that a row that's the same needs nothing.

import { Path, Polyline, Svg } from 'rsui';
import { WIDE_TAIL } from '../vt/terminal';
import { xtermColor } from '../vt/terminal';
import type { Line } from '../vt/terminal';
import type { StyleTable } from '../vt/style';
import { BLINK, BOLD, DIM, HIDDEN, ITALIC, OVERLINE, REVERSE, STRIKE, UNDERLINE_MASK, UNDERLINE_SHIFT } from '../vt/style';
import { css, mix, withAlpha } from '../themes';
import { boxRects, boxShape } from './boxdraw';
import type { Rect } from './boxdraw';

export interface Look {
    family: string;
    ligatures: boolean;
    size: number;
    cw: number;
    ch: number;
    offset: number;
    /** Room at the left of each row for the marks. */
    gutter: number;
    fg: number;
    bg: number;
    selection: number;
    accent: number;
    /** The color of what failed. */
    bad: number;
    found: number;
    foundCurrent: number;
    /** A key for what the look depends on, which changes when any of it does. */
    key: string;
}

export interface RowProps {
    /** The terminal's styles and palette, passed apart from the terminal itself, which holds callbacks, so rsui can tell a row
     * given the same ones needs nothing. */
    styles: StyleTable;
    paletteMap: Map<number, number>;
    ansi: number[];
    line: Line | null;
    /** The line's version: what makes this draw again when the line changes in place. */
    ver: number;
    cols: number;
    look: Look;
    /** The selected columns as `from-to`, or ''. */
    sel: string;
    /** Search matches as `from-to-current;...`, or ''. */
    found: string;
    /** The link under the pointer, as `from-to`, or ''. */
    link: string;
    /** The palette's version, so colors a program changed redraw. */
    palette: number;
    /** What's in the gutter: 0 nothing, then the command's state (1 running, 2 done, 3 failed) and where in it this row is. */
    mark: string;
    /** The command's time, drawn at the right of its first row. */
    note: string;
}

interface Run {
    x: number;
    n: number;
    text: string;
    fg: number;
    bold: boolean;
    italic: boolean;
    flags: number;
    ul: number;
    link: number;
}

function resolve(colors: { paletteMap: Map<number, number>; ansi: number[] }, color: number, fallback: number): number {
    if (color === 0) return fallback;
    const tag = color >>> 24;
    if (tag === 2) return color & 0xffffff;
    const index = color & 0xff;
    return colors.paletteMap.get(index) ?? xtermColor(index, colors.ansi);
}

const isWideHead = (line: Line, x: number) => x + 1 < line.length && line.ch[x + 1] === WIDE_TAIL;

// Powerline's solid and outline separators fit the whole cell, including its line spacing.
const POWERLINE_PATHS = [
    'M0 0 L10 10 L0 20 Z', 'M0.75 0.75 L9.25 10 L0.75 19.25',
    'M10 0 L0 10 L10 20 Z', 'M9.25 0.75 L0.75 10 L9.25 19.25',
    'M0 0 C13.33 0 13.33 20 0 20 Z', 'M0.75 0.75 C12.08 0.75 12.08 19.25 0.75 19.25',
    'M10 0 C-3.33 0 -3.33 20 10 20 Z', 'M9.25 0.75 C-2.08 0.75 -2.08 19.25 9.25 19.25',
];

export function Row(props: RowProps) {
    const { line, cols, look } = props;
    const styles = props.styles;
    const { cw, ch, offset, gutter } = look;
    const width = gutter + cols * cw;
    const kids: any[] = [];
    // The line changes in place, so its version is what says it did. A prop that's never read can't make a row draw again.
    void props.ver;
    void props.palette;
    if (line) {
        const used = Math.min(cols, line.length);
        // Background rectangles and text runs, in one pass over the cells.
        let bgStart = -1;
        let bgColor = 0;
        let run: Run | null = null;
        const runs: Run[] = [];
        const boxes: (Rect & { color: number })[] = [];
        const separators: { x: number; path: number; fg: number }[] = [];
        const closeBg = (end: number) => {
            if (bgStart >= 0) {
                kids.push(
                    <group key={'b' + bgStart} position="absolute" left={gutter + bgStart * cw} top={0} width={(end - bgStart) * cw} height={ch} background={css(bgColor)} />,
                );
            }
            bgStart = -1;
        };
        for (let x = 0; x < used; x++) {
            const code = line.ch[x];
            const style = styles.get(line.st[x]);
            let fg = resolve(props, style.fg, look.fg);
            let bg: number | null = style.bg === 0 ? null : resolve(props, style.bg, look.bg);
            if (style.flags & REVERSE) {
                const was = fg;
                fg = bg ?? look.bg;
                bg = was;
            }
            if (style.flags & DIM) fg = mix(fg, bg ?? look.bg, 0.45);
            if (bg !== null && bg !== look.bg) {
                if (bgStart >= 0 && bgColor === bg) {
                    /* the same stretch goes on */
                } else {
                    closeBg(x);
                    bgStart = x;
                    bgColor = bg;
                }
            } else {
                closeBg(x);
            }
            if (code === WIDE_TAIL) {
                if (run) run.n++;
                continue;
            }
            const hidden = (style.flags & HIDDEN) !== 0;
            if (code >= 0xe0b0 && code <= 0xe0b7 && !hidden) {
                separators.push({ x: gutter + x * cw, path: code - 0xe0b0, fg });
                run = null;
                continue;
            }
            if (code >= 0x2500 && code <= 0x259f && !hidden) {
                const shape = boxShape(code);
                if (shape) {
                    // Lines and blocks are drawn here, so they meet the cells beside and above them.
                    for (const rect of boxRects(shape, gutter + x * cw, cw, ch)) boxes.push({ ...rect, color: fg });
                    run = null;
                    continue;
                }
            }
            const text = code === 0 || hidden ? ' ' : (line.extra?.get(x) ?? String.fromCodePoint(code));
            const wide = isWideHead(line, x);
            const bold = (style.flags & BOLD) !== 0;
            const italic = (style.flags & ITALIC) !== 0;
            const decoration = style.flags & (UNDERLINE_MASK | STRIKE | OVERLINE);
            const ul = style.ul === 0 ? fg : resolve(props, style.ul, fg);
            const plain = code === 0 || code === 0x20;
            // A run goes on while the look stays the same; a space can join any run that has no decoration.
            if (
                run && !wide && (look.ligatures || code > 0x7f) && run.link === style.link && run.fg === fg && run.bold === bold && run.italic === italic &&
                run.flags === decoration && (run.ul === ul || !decoration)
            ) {
                run.text += text;
                run.n++;
                continue;
            }
            if (plain && !decoration && style.link === 0) {
                // A blank starts nothing: it's a gap between runs.
                run = null;
                continue;
            }
            run = { x, n: wide ? 2 : 1, text, fg, bold, italic, flags: decoration, ul, link: style.link };
            if (wide) {
                // A wide character is a run of its own, so what's after it starts where it should.
                runs.push(run);
                run = null;
                x++;
                continue;
            }
            runs.push(run);
        }
        closeBg(used);
        void BLINK;

        // The selection and the search matches go under the text.
        if (props.sel !== '') {
            const [a, b] = props.sel.split('-').map(Number);
            kids.push(
                <group key="sel" position="absolute" left={gutter + a * cw} top={0} width={Math.max(1, (b - a) * cw)} height={ch} background={css(look.selection)} />,
            );
        }
        if (props.found !== '') {
            for (const part of props.found.split(';')) {
                const [a, b, c] = part.split('-').map(Number);
                kids.push(
                    <group
                        key={'f' + a} position="absolute" left={gutter + a * cw} top={0} width={Math.max(1, (b - a) * cw)} height={ch}
                        radius={3} background={css(c === 1 ? look.foundCurrent : look.found)}
                    />,
                );
            }
        }
        // Rectangles that touch, in the same color, as a line across many cells is, are one.
        const merged: (Rect & { color: number })[] = [];
        for (const rect of boxes) {
            const last = merged[merged.length - 1];
            if (last && last.color === rect.color && last.alpha === rect.alpha && last.y === rect.y && last.h === rect.h && last.x + last.w === rect.x) {
                last.w += rect.w;
            } else {
                merged.push({ ...rect });
            }
        }
        let k = 0;
        for (const rect of merged) {
            kids.push(
                <group key={'x' + k++} position="absolute" left={rect.x} top={rect.y} width={rect.w} height={rect.h} background={rect.alpha < 1 ? withAlpha(rect.color, rect.alpha) : css(rect.color)} />,
            );
        }
        for (const { x, path, fg } of separators) {
            const outline = path % 2 === 1;
            kids.push(
                <group key={'powerline' + x} position="absolute" left={x} top={0} width={cw} height={ch}>
                    <Svg width={cw} height={ch} viewBox="0 0 10 20" stretch>
                        <Path d={POWERLINE_PATHS[path]} fill={outline ? 'none' : css(fg)} stroke={outline ? css(fg) : 'none'} strokeWidth={1.3} />
                    </Svg>
                </group>,
            );
        }
        for (const r of runs) {
            const w = r.n * cw;
            const left = gutter + r.x * cw;
            if (r.text.trim() !== '' || r.text.length > 1) {
                kids.push(
                    <text
                        key={'t' + r.x} position="absolute" left={left} top={offset} width={w + (r.text.charCodeAt(r.text.length - 1) > 0x7f ? cw : 2)} height={look.ch - offset} noWrap
                        fontFamily={look.family} fontSize={look.size} color={css(r.fg)} fontWeight={r.bold ? 'bold' : 'normal'}
                        fontStyle={r.italic ? 'italic' : 'normal'}
                    >
                        {r.text}
                    </text>,
                );
            }
            const flags = r.flags;
            const kind = (flags & UNDERLINE_MASK) >> UNDERLINE_SHIFT;
            const color = css(r.ul);
            if (kind === 1 || kind === 2) {
                kids.push(<group key={'u' + r.x} position="absolute" left={left} top={ch - 2} width={w} height={1} background={color} />);
                if (kind === 2) kids.push(<group key={'u2' + r.x} position="absolute" left={left} top={ch - 4} width={w} height={1} background={color} />);
            } else if (kind === 3) {
                const points: string[] = [];
                for (let k = 0; k <= Math.ceil(w / 2); k++) points.push(`${k * 2},${k % 2 === 0 ? 3 : 0.5}`);
                kids.push(
                    <group key={'uc' + r.x} position="absolute" left={left} top={ch - 5} width={w} height={4}>
                        <Svg width={w} height={4} viewBox={`0 0 ${w} 4`} fill="none" stroke={color} strokeWidth={1}>
                            <Polyline points={points.join(' ')} />
                        </Svg>
                    </group>,
                );
            } else if (kind === 4 || kind === 5) {
                const step = kind === 4 ? 2 : 5;
                for (let k = 0; k < Math.min(r.n * 4, w); k += step * 2) {
                    kids.push(<group key={'ud' + r.x + '-' + k} position="absolute" left={left + k} top={ch - 2} width={step} height={1} background={color} />);
                }
            }
            if (flags & STRIKE) {
                kids.push(<group key={'s' + r.x} position="absolute" left={left} top={Math.floor(ch / 2)} width={w} height={1} background={css(r.fg)} />);
            }
            if (flags & OVERLINE) {
                kids.push(<group key={'o' + r.x} position="absolute" left={left} top={0} width={w} height={1} background={css(r.fg)} />);
            }
            if (r.link !== 0 && kind === 0) {
                // A link a program marked is underlined faintly, and plainly when the pointer's on it.
                for (let k = 0; k < Math.min(r.n * 4, w); k += 4) {
                    kids.push(<group key={'ul' + r.x + '-' + k} position="absolute" left={left + k} top={ch - 2} width={2} height={1} background={withAlpha(r.fg, 0.6)} />);
                }
            }
        }
        if (props.link !== '') {
            const [a, b] = props.link.split('-').map(Number);
            kids.push(<group key="hl" position="absolute" left={gutter + a * cw} top={ch - 2} width={(b - a) * cw} height={1} background={css(look.accent)} />);
        }
    }
    // A command's first line, where it was typed, is a soft band with a thin bar at its left: grey once it worked, red when it
    // failed, the accent while it runs. Its output is left plain. The band goes under everything else in the row.
    let band: any = null;
    if (props.mark !== '' && props.mark !== '0') {
        const kind = props.mark[0];
        const part = props.mark.slice(1);
        if (part === 'start' || part === 'only') {
            const tone = kind === '3' ? look.bad : kind === '1' ? look.accent : look.fg;
            band = (
                <group key="band" position="absolute" left={0} top={0} width={width} height={ch} background={withAlpha(kind === '3' ? look.bad : look.fg, kind === '3' ? 0.07 : 0.045)}>
                    <group position="absolute" left={0} top={0} width={2} height={ch} background={withAlpha(tone, kind === '2' ? 0.4 : 0.9)} />
                </group>
            );
        }
    }
    if (props.note !== '') {
        kids.push(
            <text
                key="note" position="absolute" right={4} top={Math.max(0, offset)} height={ch} noWrap fontFamily={look.family}
                fontSize={Math.max(9, look.size - 3)} color={withAlpha(look.fg, 0.38)} paddingX={6}
            >
                {props.note}
            </text>,
        );
    }
    return (
        <group width={width} height={ch} position="relative">
            {band}
            {kids}
        </group>
    );
}

