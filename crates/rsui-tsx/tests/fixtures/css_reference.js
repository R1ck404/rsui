// What rsui/web's CSS reading was in JavaScript (web.js at 4c4ca1d), which src/css.rs has to give exactly the same as.
// Only the tests load it. Since then, what's after a backslash is a character as it is outside quotes too, as class names
// from tools have `\:` and `\(` in them, and a rule with no block, as `@layer a, b;` is, is a rule rather than nothing.
let remSize = 16;
/// The text inside a pair of parentheses that starts at `open`, and where it ends.
function insideParens(text, open) {
    let depth = 0;
    for (let at = open; at < text.length; at += 1) {
        if (text[at] === '(') {
            depth += 1;
        } else if (text[at] === ')') {
            depth -= 1;
            if (depth === 0) {
                return [text.slice(open + 1, at), at + 1];
            }
        }
    }
    return [text.slice(open + 1), text.length];
}

/// Splits at what's not in parentheses or quotes.
function splitOutside(text, isSplit) {
    const parts = [];
    let depth = 0;
    let quote = null;
    let start = 0;
    for (let at = 0; at < text.length; at += 1) {
        const c = text[at];
        if (c === '\\') {
            at += 1;
        } else if (quote !== null) {
            if (c === quote) {
                quote = null;
            }
        } else if (c === '"' || c === "'") {
            quote = c;
        } else if (c === '(') {
            depth += 1;
        } else if (c === ')') {
            depth -= 1;
        } else if (depth === 0 && isSplit(c)) {
            parts.push(text.slice(start, at));
            start = at + 1;
        }
    }
    parts.push(text.slice(start));
    return parts.map((part) => part.trim()).filter((part) => part !== '');
}
const words = (text) => splitOutside(text, (c) => c === ' ' || c === '\t' || c === '\n' || c === '\r');
const commas = (text) => splitOutside(text, (c) => c === ',');
function length(text, fontSize = 16) {
    const value = String(text).trim();
    if (value === '0') {
        return 0;
    }
    if (value.startsWith('calc(')) {
        return calculated(insideParens(value, 4)[0], fontSize);
    }
    if (value.startsWith('min(') || value.startsWith('max(') || value.startsWith('clamp(')) {
        const clamp = value.startsWith('clamp(');
        const parts = commas(insideParens(value, clamp ? 5 : 3)[0]).map((part) => length(part.trim(), fontSize));
        // In pixels, it's worked out now. With a share of what it's in among them, that's what it comes to, as it's usually what's meant
        // and the others only keep it in bounds.
        if (parts.every((part) => typeof part === 'number')) {
            if (clamp) {
                return Math.max(parts[0], Math.min(parts[1], parts[2]));
            }
            return value.startsWith('min(') ? Math.min(...parts) : Math.max(...parts);
        }
        if (clamp) {
            return parts[1];
        }
        return parts.find((part) => typeof part === 'string') ?? parts[0];
    }
    const found = /^(-?[0-9]*\.?[0-9]+)([a-z%]*)$/.exec(value);
    if (found === null) {
        return undefined;
    }
    const number = Number(found[1]);
    switch (found[2]) {
        case '':
        case 'px':
            return number;
        case 'rem':
            return number * remSize;
        case 'em':
            return number * fontSize;
        case 'lh':
            return number * fontSize * 1.2;
        case 'rlh':
            return number * remSize * 1.2;
        case 'pt':
            return (number * 96) / 72;
        case '%':
            return number + '%';
        // A share of the window, which is worked out again when it changes size.
        case 'vh':
        case 'vw':
        case 'vmin':
        case 'vmax': {
            globalThis.__rsuiViewportUnits = true;
            const [width, height] = globalThis.__rsuiPageSize ?? [800, 600];
            const side = { vh: height, vw: width, vmin: Math.min(width, height), vmax: Math.max(width, height) }[found[2]];
            return (number * side) / 100;
        }
        default:
            return undefined;
    }
}

/// `calc()`, as long as all of it comes to the same kind of thing. A number with no unit can times or divide what has one.
function calculated(text, fontSize) {
    // CSS's constants (`infinity`, `pi`, `e`, `NaN`, in any case) are words of their own.
    const tokens = text.match(/[()*/]|[+-](?=\s)|-?[0-9]*\.?[0-9]+[a-z%]*|-?(?:[Ii][Nn][Ff][Ii][Nn][Ii][Tt][Yy]|[Pp][Ii]|[Ee]|[Nn][Aa][Nn])(?![a-zA-Z0-9-])/g) ?? [];
    let at = 0;
    // A value, as how much of it is a plain number, pixels, and a percentage of what it's in. Only pixels and a percentage can go
    // together, as `100% - 14px` does.
    const atom = () => {
        const token = tokens[at];
        at += 1;
        if (token === '(') {
            const inner = sum();
            at += 1;
            return inner;
        }
        if (token === undefined) {
            return null;
        }
        const word = token.replace(/^-/, '').toLowerCase();
        const sign = token.startsWith('-') ? -1 : 1;
        if (word === 'nan') {
            return null;
        }
        const constant = { infinity: Infinity, pi: Math.PI, e: Math.E }[word];
        if (constant !== undefined) {
            return { number: sign * constant, px: 0, percent: 0, plain: true };
        }
        if (/^-?[0-9.]+$/.test(token)) {
            return { number: parseFloat(token), px: 0, percent: 0, plain: true };
        }
        const size = length(token, fontSize);
        if (size === undefined) {
            return null;
        }
        return typeof size === 'number' ? { number: 0, px: size, percent: 0, plain: false } : { number: 0, px: 0, percent: parseFloat(size), plain: false };
    };
    // What there's none of stays none, even times `infinity`.
    const part = (amount, by) => (amount === 0 ? 0 : amount * by);
    const times = (value, by) => ({ number: part(value.number, by), px: part(value.px, by), percent: part(value.percent, by), plain: value.plain });
    const product = () => {
        let left = atom();
        while (tokens[at] === '*' || tokens[at] === '/') {
            const op = tokens[at];
            at += 1;
            const right = atom();
            if (left === null || right === null || (!left.plain && !right.plain) || (op === '/' && !right.plain)) {
                left = null;
            } else if (op === '*') {
                left = left.plain ? times(right, left.number) : times(left, right.number);
            } else {
                left = times(left, 1 / right.number);
            }
        }
        return left;
    };
    const sum = () => {
        let left = product();
        while (tokens[at] === '+' || tokens[at] === '-') {
            const op = tokens[at];
            at += 1;
            let right = product();
            if (left === null || right === null) {
                left = null;
                continue;
            }
            right = op === '-' ? times(right, -1) : right;
            // A plain number with a length is taken as that many pixels, which is more than CSS lets through.
            const asPx = (value) => (value.plain ? { number: 0, px: value.number, percent: 0, plain: false } : value);
            if (left.plain && right.plain) {
                left = { number: left.number + right.number, px: 0, percent: 0, plain: true };
            } else {
                const [one, other] = [asPx(left), asPx(right)];
                left = { number: 0, px: one.px + other.px, percent: one.percent + other.percent, plain: false };
            }
        }
        return left;
    };
    const result = sum();
    if (result === null) {
        return undefined;
    }
    // What's infinite is as far as a length goes (Chromium's `LayoutUnit`).
    const clamp = (amount) => Math.min(33554400, Math.max(-33554400, amount));
    if (result.plain) {
        return Number.isNaN(result.number) ? undefined : clamp(result.number);
    }
    if (Number.isNaN(result.px) || Number.isNaN(result.percent)) {
        return undefined;
    }
    result.px = clamp(result.px);
    result.percent = clamp(result.percent);
    if (result.percent === 0) {
        return result.px;
    }
    // Both, which what lays it out reads as it's written here.
    return result.px === 0 ? result.percent + '%' : result.percent + '% + ' + result.px + 'px';
}

// Colors

/// Every color CSS has a name for.
const NAMED = {
    aliceblue: '#f0f8ff', antiquewhite: '#faebd7', aqua: '#00ffff', aquamarine: '#7fffd4', azure: '#f0ffff', beige: '#f5f5dc',
    bisque: '#ffe4c4', black: '#000000', blanchedalmond: '#ffebcd', blue: '#0000ff', blueviolet: '#8a2be2', brown: '#a52a2a',
    burlywood: '#deb887', cadetblue: '#5f9ea0', chartreuse: '#7fff00', chocolate: '#d2691e', coral: '#ff7f50', cornflowerblue: '#6495ed',
    cornsilk: '#fff8dc', crimson: '#dc143c', cyan: '#00ffff', darkblue: '#00008b', darkcyan: '#008b8b', darkgoldenrod: '#b8860b',
    darkgray: '#a9a9a9', darkgreen: '#006400', darkgrey: '#a9a9a9', darkkhaki: '#bdb76b', darkmagenta: '#8b008b', darkolivegreen: '#556b2f',
    darkorange: '#ff8c00', darkorchid: '#9932cc', darkred: '#8b0000', darksalmon: '#e9967a', darkseagreen: '#8fbc8f',
    darkslateblue: '#483d8b', darkslategray: '#2f4f4f', darkslategrey: '#2f4f4f', darkturquoise: '#00ced1', darkviolet: '#9400d3',
    deeppink: '#ff1493', deepskyblue: '#00bfff', dimgray: '#696969', dimgrey: '#696969', dodgerblue: '#1e90ff', firebrick: '#b22222',
    floralwhite: '#fffaf0', forestgreen: '#228b22', fuchsia: '#ff00ff', gainsboro: '#dcdcdc', ghostwhite: '#f8f8ff', gold: '#ffd700',
    goldenrod: '#daa520', gray: '#808080', green: '#008000', greenyellow: '#adff2f', grey: '#808080', honeydew: '#f0fff0',
    hotpink: '#ff69b4', indianred: '#cd5c5c', indigo: '#4b0082', ivory: '#fffff0', khaki: '#f0e68c', lavender: '#e6e6fa',
    lavenderblush: '#fff0f5', lawngreen: '#7cfc00', lemonchiffon: '#fffacd', lightblue: '#add8e6', lightcoral: '#f08080',
    lightcyan: '#e0ffff', lightgoldenrodyellow: '#fafad2', lightgray: '#d3d3d3', lightgreen: '#90ee90', lightgrey: '#d3d3d3',
    lightpink: '#ffb6c1', lightsalmon: '#ffa07a', lightseagreen: '#20b2aa', lightskyblue: '#87cefa', lightslategray: '#778899',
    lightslategrey: '#778899', lightsteelblue: '#b0c4de', lightyellow: '#ffffe0', lime: '#00ff00', limegreen: '#32cd32', linen: '#faf0e6',
    magenta: '#ff00ff', maroon: '#800000', mediumaquamarine: '#66cdaa', mediumblue: '#0000cd', mediumorchid: '#ba55d3',
    mediumpurple: '#9370db', mediumseagreen: '#3cb371', mediumslateblue: '#7b68ee', mediumspringgreen: '#00fa9a',
    mediumturquoise: '#48d1cc', mediumvioletred: '#c71585', midnightblue: '#191970', mintcream: '#f5fffa', mistyrose: '#ffe4e1',
    moccasin: '#ffe4b5', navajowhite: '#ffdead', navy: '#000080', oldlace: '#fdf5e6', olive: '#808000', olivedrab: '#6b8e23',
    orange: '#ffa500', orangered: '#ff4500', orchid: '#da70d6', palegoldenrod: '#eee8aa', palegreen: '#98fb98', paleturquoise: '#afeeee',
    palevioletred: '#db7093', papayawhip: '#ffefd5', peachpuff: '#ffdab9', peru: '#cd853f', pink: '#ffc0cb', plum: '#dda0dd',
    powderblue: '#b0e0e6', purple: '#800080', rebeccapurple: '#663399', red: '#ff0000', rosybrown: '#bc8f8f', royalblue: '#4169e1',
    saddlebrown: '#8b4513', salmon: '#fa8072', sandybrown: '#f4a460', seagreen: '#2e8b57', seashell: '#fff5ee', sienna: '#a0522d',
    silver: '#c0c0c0', skyblue: '#87ceeb', slateblue: '#6a5acd', slategray: '#708090', slategrey: '#708090', snow: '#fffafa',
    springgreen: '#00ff7f', steelblue: '#4682b4', tan: '#d2b48c', teal: '#008080', thistle: '#d8bfd8', tomato: '#ff6347',
    turquoise: '#40e0d0', violet: '#ee82ee', wheat: '#f5deb3', white: '#ffffff', whitesmoke: '#f5f5f5', yellow: '#ffff00',
    yellowgreen: '#9acd32', transparent: '#00000000',
};

const hexPart = (value) => Math.round(Math.max(0, Math.min(255, value))).toString(16).padStart(2, '0');

function hslToRgb(h, s, l) {
    const a = s * Math.min(l, 1 - l);
    const f = (n) => {
        const k = (n + h / 30) % 12;
        return l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
    };
    return [f(0) * 255, f(8) * 255, f(4) * 255];
}

/// A color as rsui takes it, or `undefined` if it isn't one.
function color(text) {
    const value = String(text).trim().toLowerCase();
    if (value.startsWith('#')) {
        return /^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/.test(value) ? value : undefined;
    }
    const named = NAMED[value];
    if (named !== undefined) {
        return named;
    }
    const found = /^(rgba?|hsla?)\(/.exec(value);
    if (found === null) {
        return undefined;
    }
    const parts = insideParens(value, found[0].length - 1)[0]
        .split(/[\s,/]+/)
        .filter((part) => part !== '');
    if (parts.length < 3) {
        return undefined;
    }
    const part = (at, whole) => (parts[at].endsWith('%') ? (parseFloat(parts[at]) * whole) / 100 : parseFloat(parts[at]));
    const alpha = parts.length > 3 ? part(3, 1) : 1;
    const rgb = found[1].startsWith('rgb')
        ? [part(0, 255), part(1, 255), part(2, 255)]
        : hslToRgb(parseFloat(parts[0]), parseFloat(parts[1]) / 100, parseFloat(parts[2]) / 100);
    if (rgb.some(Number.isNaN) || Number.isNaN(alpha)) {
        return undefined;
    }
    const start = '#' + hexPart(rgb[0]) + hexPart(rgb[1]) + hexPart(rgb[2]);
    return alpha >= 1 ? start : start + hexPart(alpha * 255);
}

// CSS, as text, into rules

/// The characters that mean something to [`readBlock`].
const CSS_SPECIAL = /["'(){};\\]/g;

/// Reads the inside of a block, as far as what closes it. What's before a `{` is what the rule is for, and `name: value;` is the rest.
function readBlock(text, start, top = false) {
    const block = { declarations: [], rules: [] };
    let at = start;
    let from = start;
    let quote = null;
    let parens = 0;
    while (at < text.length) {
        // Straight to the next character that means anything, as going through them all one at a time is slow.
        CSS_SPECIAL.lastIndex = at;
        const next = CSS_SPECIAL.exec(text);
        if (next === null) {
            at = text.length;
            break;
        }
        at = next.index;
        const c = text[at];
        if (quote !== null) {
            if (c === '\\') {
                at += 1;
            } else if (c === quote) {
                quote = null;
            }
        } else if (c === '\\') {
            at += 1;
        } else if (c === '"' || c === "'") {
            quote = c;
        } else if (c === '(') {
            parens += 1;
        } else if (c === ')') {
            parens -= 1;
        } else if (parens <= 0) {
            if (c === ';') {
                addDeclaration(block, text.slice(from, at));
                from = at + 1;
            } else if (c === '{') {
                const [inner, end] = readBlock(text, at + 1);
                block.rules.push({ selector: text.slice(from, at).trim(), block: inner });
                at = end;
                from = at;
                continue;
            } else if (c === '}') {
                addDeclaration(block, text.slice(from, at));
                if (!top) {
                    return [block, at + 1];
                }
                // One that closes nothing is a mistake, which a browser gets past, as this does.
                from = at + 1;
            }
        }
        at += 1;
    }
    addDeclaration(block, text.slice(from, at));
    return [block, at];
}

function addDeclaration(block, text) {
    const trimmed = text.trim();
    if (trimmed.startsWith('@')) {
        block.rules.push({ selector: trimmed, block: { declarations: [], rules: [] } });
        return;
    }
    const colon = text.indexOf(':');
    if (colon < 0) {
        return;
    }
    const said = text.slice(0, colon).trim();
    const name = said.startsWith('--') ? said : said.toLowerCase();
    // What's after a backslash is any character, as `--chakra-spacing-1\.5` has a full stop in its name.
    if (/^([\p{L}_-]|\\[\s\S]|\\$|[^\x00-\x7f])([\p{L}\p{N}_-]|\\[\s\S]|\\$|[^\x00-\x7f])*$/u.test(name.startsWith('-') ? name.slice(1) : name)) {
        const value = text.slice(colon + 1);
        const important = /!\s*important/i.test(value);
        const plain = value.replace(/!\s*important/gi, '').trim();
        block.declarations.push(important ? [name, plain, true] : [name, plain]);
    }
}

const withoutComments = (text) => text.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
const readCss = (text) => readBlock(withoutComments(text), 0, true)[0];
