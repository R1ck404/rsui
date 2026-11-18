// What rsui/web made of CSS declarations in JavaScript (web.js at f43de18), which src/declare.rs has to give exactly the same as.
// Only the tests load it, after css_reference.js, whose `length`, `color`, `words` and `commas` it uses.
//
// One thing is changed: the tables of words below have nothing behind them, where they were plain objects, so a word that's the
// name of something every object has, as `constructor` or `toString` is, isn't found in them. In web.js such a word found that
// function, as `align-items: constructor` gave `align` the function `Object`, which isn't anything; declare.rs has them as words
// that aren't in the table, which is what they are.

// Sizes and colors are what web.js had at f43de18, which is css.rs; its words and commas are css_reference.js's, which are the same.
length = (text, fontSize = 16) => natives.cssLength(text, fontSize, remSize);
color = (text) => natives.cssColor(text);

/// What's been said to be left out, every time, and what the page's variables are, which the test sets.
let leftOut = [];
function notHere(what) {
    leftOut.push(what);
}
const variables = new Map();
function variableValue(name) {
    return variables.get(name);
}

/// Puts in what `var(--name)` stands for.
function withVariables(text, depth = 0) {
    let at = text.indexOf('var(');
    if (at < 0 || depth > 16) {
        return text;
    }
    let out = '';
    let from = 0;
    while (at >= 0) {
        out += text.slice(from, at);
        const [inside, end] = insideParens(text, at + 3);
        const comma = inside.indexOf(',');
        const name = (comma < 0 ? inside : inside.slice(0, comma)).trim();
        const fallback = comma < 0 ? '' : inside.slice(comma + 1).trim();
        const found = variableValue(name);
        out += withVariables(found !== undefined ? found : fallback, depth + 1);
        from = end;
        at = text.indexOf('var(', from);
    }
    return out + text.slice(from);
}

function lookFrom(declarations) {
    const look = {};
    declareAll(look, declarations, 16);
    return look;
}

/// Makes each declaration part of `props`, with what says `!important` after the rest, and which props that made kept in
/// `__important`, as nothing that doesn't say it goes over it, as CSS has it.
function declareAll(props, declarations, fontSize) {
    let any = false;
    for (const [name, value, important] of declarations) {
        if (important === true) {
            any = true;
        } else {
            declare(props, name, value, fontSize);
        }
    }
    if (!any) {
        return;
    }
    const before = { ...props };
    for (const [name, value, important] of declarations) {
        if (important === true) {
            declare(props, name, value, fontSize);
        }
    }
    // What it made, which is also what it set that wasn't there before, as `undefined`.
    const kept = Object.keys(props).filter((key) => key !== '__important' && (!(key in before) || props[key] !== before[key]));
    props.__important = [...(props.__important ?? []), ...kept];
}

/// Puts what `props` says over what `look` says, apart from what `look` has that was `!important`, unless `props` says that's

/// As a number of pixels, or as text if it's a percentage. It's `undefined` if it's nothing that can be worked out here.
/// Grid tracks as CSS writes them, with each size that's in something other than pixels in pixels, which is what a grid here takes.
function gridTracks(value, fontSize) {
    return value.replace(/(-?[0-9]*\.?[0-9]+)(rem|em|vw|vh|vmin|vmax|pt)\b/g, (whole) => {
        const size = length(whole, fontSize);
        return typeof size === 'number' ? size + 'px' : whole;
    });
}


// Declarations into props

const ALIGN = Object.assign(Object.create(null), {
    'flex-start': 'start', start: 'start', 'self-start': 'start', left: 'start', top: 'start', baseline: 'start',
    center: 'center', 'flex-end': 'end', end: 'end', 'self-end': 'end', right: 'end', bottom: 'end', stretch: 'stretch', normal: 'stretch',
});
const JUSTIFY = Object.assign(Object.create(null), {
    'flex-start': 'start', start: 'start', left: 'start', normal: 'start', center: 'center', 'flex-end': 'end', end: 'end', right: 'end',
    'space-between': 'space_between', 'space-around': 'space_around', 'space-evenly': 'space_evenly',
});
const SILENT = new Set([
    'will-change', '-webkit-font-smoothing', '-moz-osx-font-smoothing',
    'text-rendering', 'font-display', '-moz-user-select', '-ms-user-select',
    'overflow-wrap', 'word-wrap', 'word-break', 'overscroll-behavior', 'scroll-behavior', 'isolation',
    'backface-visibility', 'perspective', 'contain', 'tab-size', 'caret-color', 'resize', 'field-sizing', 'vertical-align',
]);

const FROM_INSIDE = new Set(['auto', 'none', 'unset', 'initial', 'inherit', 'fit-content', 'max-content', 'min-content']);

/// A `background-position` with each length in it that isn't in pixels in pixels, or as it was if there's none.
function positionInPixels(said, fontSize) {
    let any = false;
    const out = words(said).map((word) => {
        const size = !/px$/.test(word) && /[a-zA-Z]/.test(word) ? length(word, fontSize) : undefined;
        if (typeof size === 'number') {
            any = true;
            return size + 'px';
        }
        return word;
    });
    return any ? out.join(' ') : said;
}

function setSides(props, name, value, fontSize) {
    const parts = words(value);
    const read = parts.map((part) => (part === 'auto' ? 'auto' : length(part, fontSize)));
    const [top, right = top, bottom = top, left = right] = read;
    const put = (side, size) => {
        if (typeof size === 'number') {
            props[name + side] = size;
            if (name + side + 'Share' in props) {
                props[name + side + 'Share'] = undefined;
            }
        } else if (typeof size === 'string' && size.includes('%')) {
            props[name + side] = 0;
            props[name + side + 'Share'] = size;
        } else if (size !== 'auto') {
            notHere(name + ' as ' + value);
        }
    };
    if (name === 'margin') {
        // What's `auto` takes what's left over, which depends on what it's in, so it's only worked out when that's known.
        for (const [side, size] of [['Top', top], ['Right', right], ['Bottom', bottom], ['Left', left]]) {
            props['__auto' + side] = size === 'auto';
            if (size === 'auto') {
                props['margin' + side] = undefined;
            }
        }
    }
    put('Top', top);
    put('Right', right);
    put('Bottom', bottom);
    put('Left', left);
}

/// The ways a border can be drawn, as CSS names them. Only dashes and dots look different here.
const DECORATION_STYLES = new Set(['solid', 'double', 'dotted', 'dashed', 'wavy']);
const BORDER_STYLES = new Set(['solid', 'dashed', 'dotted', 'double', 'groove', 'ridge', 'inset', 'outset']);

/// What an `outline` says: a line around the outside of something, which takes no room. It's only there with a style that isn't
/// `none`, as CSS has it, 3 pixels wide unless it says, in the color of the text unless it says.
function setOutline(props, value) {
    let style = 'none';
    let width = 3;
    let paint;
    for (const part of words(value)) {
        const size = length(part);
        const found = color(part);
        if (typeof size === 'number') {
            width = size;
        } else if (part === 'none' || part === 'hidden' || BORDER_STYLES.has(part) || part === 'auto') {
            style = part;
        } else if (found !== undefined) {
            paint = found;
        }
    }
    props.outlineWidth = style === 'none' || style === 'hidden' ? 0 : width;
    props.cssOutlineWidth = width;
    // A shorthand sets each of its longhands, to its initial value where it doesn't say: the color of the text, for the color.
    props.outlineColor = paint ?? 'currentcolor';
    setOutlineStyle(props, style);
}

/// What's drawn differently from a solid outline: dashes, dots, or `auto`, the browser's focus ring.
function setOutlineStyle(props, style) {
    if (style === 'dashed' || style === 'dotted' || style === 'auto') {
        props.outlineStyle = style;
    } else {
        delete props.outlineStyle;
    }
}

/// What `font-feature-settings` says of `tnum`, if anything: on, unless it's `0` or `off`.
function tnumSetting(value) {
    for (const part of value.split(',').map((one) => one.trim())) {
        for (const quoted of ['"tnum"', "'tnum'"]) {
            if (part.startsWith(quoted)) {
                const rest = part.slice(quoted.length).trim();
                return !(rest === '0' || rest === 'off');
            }
        }
    }
    return undefined;
}

/// What's said of each side's border width, taken away, as one width for all of them says over it.
function withoutBorderSides(props) {
    for (const side of ['Top', 'Right', 'Bottom', 'Left']) {
        props['border' + side + 'Width'] = undefined;
    }
}

/// What a shorthand that says every side's color puts back over what was said of one side: each side has the color they all have.
function withoutBorderSideColors(props) {
    for (const side of ['Top', 'Right', 'Bottom', 'Left']) {
        props['border' + side + 'Color'] = undefined;
    }
}

/// What the `background` shorthand puts back to how it is if nothing says, besides its color.
function withoutBackgroundLayers(props) {
    for (const key of ['backgroundImage', 'backgroundGradient', 'backgroundRepeat', 'backgroundPosition', 'backgroundSize', 'backgroundClip']) {
        props[key] = undefined;
    }
}

function setBorder(props, value) {
    if (value === 'none' || value === '0') {
        withoutBorderSides(props);
        withoutBorderSideColors(props);
        props.borderWidth = 0;
        return;
    }
    let found = false;
    if (value === '') {
        return;
    }
    for (const part of words(value)) {
        const size = length(part);
        const paint = color(part);
        if (typeof size === 'number') {
            withoutBorderSides(props);
            props.borderWidth = size;
            found = true;
        } else if (paint !== undefined) {
            props.borderColor = paint;
            found = true;
        } else if (part === 'none' || part === 'hidden') {
            withoutBorderSides(props);
            props.borderWidth = 0;
            found = true;
        } else if (BORDER_STYLES.has(part)) {
            props.borderStyle = DRAWN_BORDERS.has(part) ? part : 'solid';
            found = true;
        }
    }
    if (found && props.borderWidth === undefined) {
        // It's 3 as CSS has it if it's not said, which is more than anyone meant.
        props.borderWidth = 1;
    }
    if (found) {
        withoutBorderSideColors(props);
    }
}

/// The picture a `background` or `background-image` shows, from its first `url()`, and for the `background` shorthand, what it says
/// about its color and where the picture goes. Says whether there was one.
function setBackgroundUrl(props, value, shorthand) {
    const found = /url\(\s*(?:"([^"]*)"|'([^']*)'|([^)]*?))\s*\)/.exec(value);
    if (found === null) {
        return false;
    }
    // Without quotes, a URL can't have spaces, quotes or brackets in it, and CSS leaves out all of what says one does.
    if (found[3] !== undefined && /[\s"'(]/.test(found[3])) {
        return true;
    }
    props.backgroundImage = found[1] ?? found[2] ?? found[3];
    if (!shorthand) {
        return true;
    }
    // What's left, without the picture, which can have anything in it.
    const rest = value.slice(0, found.index) + ' ' + value.slice(found.index + found[0].length);
    const layers = commas(rest);
    const first = layers[0] ?? '';
    const [placed, sized] = first.split('/');
    const position = [];
    const repeats = [];
    for (const word of words(placed ?? '')) {
        if (/^(repeat|no-repeat|repeat-x|repeat-y|space|round)$/.test(word)) {
            repeats.push(word);
        } else if (/^(left|right|top|bottom|center)$/.test(word) || typeof length(word) === 'number' || /%$/.test(word)) {
            position.push(word);
        } else if (color(word) !== undefined) {
            props.background = color(word);
        }
    }
    if (repeats.length > 0) {
        props.backgroundRepeat = repeats.join(' ');
    }
    if (position.length > 0) {
        props.backgroundPosition = positionInPixels(position.join(' '), 16);
    }
    if (sized !== undefined && sized.trim() !== '') {
        props.backgroundSize = sized.trim().split(/\s+/).filter((word) => color(word) === undefined).join(' ');
        const paint = words(sized).map(color).find((one) => one !== undefined);
        if (paint !== undefined) {
            props.background = paint;
        }
    }
    // The color goes with the last of them.
    const last = layers.length > 1 ? layers[layers.length - 1] : '';
    const paint = words(last).map(color).find((one) => one !== undefined);
    if (paint !== undefined) {
        props.background = paint;
    }
    return true;
}

function setShadow(props, value) {
    delete props.shadows;
    if (value === 'none') {
        props.shadowBlur = 0;
        props.shadows = '';
        return;
    }
    // Each shadow, in the order CSS lists them, which is from the one on top.
    const all = commas(value).map((one) => {
        const sizes = [];
        let paint;
        let inset = false;
        for (const part of words(one)) {
            const size = length(part);
            const found = color(part);
            if (part === 'inset') {
                inset = true;
            } else if (typeof size === 'number') {
                sizes.push(size);
            } else if (found !== undefined) {
                paint = found;
            }
        }
        return { x: sizes[0] ?? 0, y: sizes[1] ?? 0, blur: sizes[2] ?? 0, spread: sizes[3] ?? 0, paint, inset };
    });
    const first = all[0];
    if (first === undefined) {
        return;
    }
    props.shadowX = first.x;
    props.shadowY = first.y;
    props.shadowBlur = first.blur;
    props.shadowSpread = first.spread;
    if (first.paint !== undefined) {
        props.shadowColor = first.paint;
    }
    {
        // All of them as a list, which moves as a whole when `box-shadow` changes.
        props.shadows = all
            .map((one) => `${one.x} ${one.y} ${one.blur} ${one.spread} ${colorNumber(one.paint ?? 'rgba(0,0,0,0.25)') ?? 1073741824}${one.inset ? ' inset' : ''}`)
            .join('\n');
    }
}

function fontWeight(value) {
    return value === 'normal' || value === 'bold' ? value : String(value);
}

// `monospace` stays as it is, as how big it's drawn depends on it (see `monospaceSize`), and rsui finds Consolas for it.
const GENERIC_FONTS = Object.assign(Object.create(null), { 'sans-serif': 'Arial', 'san-serif': 'Arial', serif: 'Times New Roman', 'system-ui': 'Segoe UI', cursive: 'Comic Sans MS', fantasy: 'Impact' });

function fontFamily(value) {
    const names = commas(value)
        .map((one) => one.trim().replace(/^["']|["']$/g, ''))
        .map((name) => GENERIC_FONTS[name.toLowerCase()] ?? name)
        .filter((name) => name !== '');
    return names.join(',');
}

/// What font props come to here: a file, for a family that CSS has given files for, with the nearest weight there is, and otherwise a
/// system font in regular or bold. These stay as CSS has them until the end, so that a weight can go down without the family, as it does.
function finalFont(given) {
    let props = given;
    if (props.textDirection !== undefined || props.textAlign === 'start' || props.textAlign === 'end') {
        // Text starts at the right, and ends at the left, when it goes right to left, as `direction: rtl` says.
        props = { ...props };
        const backwards = props.textDirection === 'rtl';
        if (props.textAlign === undefined || props.textAlign === 'start') {
            props.textAlign = backwards ? 'right' : props.textAlign === undefined ? undefined : 'left';
        } else if (props.textAlign === 'end') {
            props.textAlign = backwards ? 'left' : 'right';
        }
        if (props.textAlign === undefined) {
            delete props.textAlign;
        }
    }
    if (props.__whiteSpace !== undefined || props.__sizeSaid !== undefined) {
        props = { ...props };
        // Text that's in something that keeps it to one line is kept to one line too.
        if ((props.__whiteSpace === 'nowrap' || props.__whiteSpace === 'pre') && props.noWrap === undefined) {
            props.noWrap = true;
        }
        delete props.__whiteSpace;
        delete props.__sizeSaid;
    }
    if (props.__lineScale !== undefined) {
        props = { ...props };
        if (typeof props.__lineScale === 'number' && typeof props.fontSize === 'number') {
            props.lineHeight = props.__lineScale * props.fontSize;
        }
        delete props.__lineScale;
    }
    const { fontFamily: family, fontWeight: weight } = props;
    if (family === undefined && weight === undefined) {
        return props;
    }
    const wanted = weight === undefined || weight === 'normal' ? 400 : weight === 'bold' ? 700 : Number(weight);
    // The fonts CSS gave files for are found by the first of the family's names.
    const files = family === undefined ? undefined : fontFiles.get(typeof family === 'string' ? family.split(',')[0] : family);
    const out = { ...props };
    if (files !== undefined) {
        const italic = props.fontStyle === 'italic';
        const fitting = files.some((file) => file.italic === italic) ? files.filter((file) => file.italic === italic) : files;
        let best = fitting[0];
        for (const file of fitting) {
            if (Math.abs(file.weight - wanted) < Math.abs(best.weight - wanted)) {
                best = file;
            }
        }
        out.fontFamily = best.path;
        // It's only heavier than the file is if there's nothing near that, which CSS would make up, and this makes bold.
        out.fontWeight = wanted >= 600 && best.weight < 600 ? 'bold' : 'normal';
        if (best.italic) {
            delete out.fontStyle;
        }
    } else {
        // A system font's own weights, which rsui looks for by name, as `Segoe UI Semibold` is.
        out.fontWeight = wanted === 700 ? 'bold' : wanted === 400 ? 'normal' : String(wanted);
    }
    return out;
}

function setFlex(props, value, fontSize) {
    const parts = words(value);
    if (parts[0] === 'none') {
        props.grow = 0;
        props.shrink = 0;
        props.basis = undefined;
        return;
    }
    if (parts[0] === 'auto') {
        props.grow = 1;
        props.shrink = undefined;
        props.basis = undefined;
        return;
    }
    if (parts[0] === 'initial') {
        props.grow = undefined;
        props.shrink = undefined;
        props.basis = undefined;
        return;
    }
    // Up to two numbers, for how much it grows and shrinks, and what it starts from, in any order after them.
    const numbers = [];
    let basis;
    for (const part of parts) {
        if (numbers.length < 2 && basis === undefined && /^[0-9]*\.?[0-9]+$/.test(part)) {
            numbers.push(parseFloat(part));
        } else {
            basis = part;
        }
    }
    const grow = numbers[0] ?? 1;
    const shrink = numbers[1] ?? 1;
    if (basis === undefined) {
        // With only numbers, it starts from nothing, so what each gets is only its share.
        if (grow > 0) {
            // From nothing, as a share of what it's in, which is from what's in it when how big that is depends on this.
            props.grow = grow;
            props.shrink = 1;
            props.basis = '0%';
        } else {
            props.grow = 0;
        }
    } else {
        // It starts from how big it is, or what it says, and what's left over is shared out.
        props.grow = grow;
        setBasis(props, basis, fontSize);
    }
    if (shrink === 0) {
        props.shrink = 0;
    }
}

/// What a flex item starts from before what's left is shared out: how big it is, for `auto` and `content`, or a length.
function setBasis(props, basis, fontSize) {
    const word = basis.trim().toLowerCase();
    if (word === 'auto' || word === 'content' || word === 'max-content' || word === 'fit-content') {
        // As big as it is, over what any rule before said.
        props.basis = undefined;
        return;
    }
    const size = length(word, fontSize);
    if (typeof size === 'number') {
        props.basis = size;
    } else if (typeof size === 'string' && size.includes('% +')) {
        props.basis = size;
    } else if (word.endsWith('%') && parseFloat(word) === 0) {
        props.basis = 0;
    } else if (word.endsWith('%') && Number.isFinite(parseFloat(word)) && parseFloat(word) > 0) {
        // A share of the flex box, which the layout works out once it knows how long that is.
        props.basis = word;
    } else {
        notHere('flex-basis as ' + basis);
    }
}

/// Makes `name: value` part of `props`. A value that isn't known is said once and left out.
/// What each property that's passed down is called here, as `inherit` on one of those means having none of its own.
const INHERITED_KEYS = Object.assign(Object.create(null), {
    color: 'color',
    'font-family': 'fontFamily',
    'font-size': 'fontSize',
    'font-weight': 'fontWeight',
    'font-style': 'fontStyle',
    'line-height': 'lineHeight',
    'text-align': 'textAlign',
    cursor: 'cursor',
    visibility: '__visibility',
    'text-transform': '__case',
});

/// Makes `name: value` part of `props`, keeping what's in `em` as a share of the element's own text size, which is only known when
/// it's drawn: `__em_marginTop` and the like, and `__fontScale` for a text size that's a share of what's around it.
// What part of a table `display` says something is.
const TABLE_PARTS = { table: 'table', 'inline-table': 'table', 'table-row-group': 'group', 'table-header-group': 'group', 'table-footer-group': 'group', 'table-row': 'row', 'table-cell': 'cell', 'table-caption': 'caption', 'table-column': 'column', 'table-column-group': 'columns' };

function declare(props, name, given, fontSize) {
    let value = withVariables(given);
    if (value.trim() === '' && given.includes('var(') && INHERITED_KEYS[name] !== undefined) {
        value = 'unset';
    }
    if (name === 'font-size') {
        const found = /^(-?[0-9]*\.?[0-9]+)(em|%)$/.exec(value.trim());
        props.__fontScale = found === null ? null : Number(found[1]) / (found[2] === '%' ? 100 : 1);
        const named = FONT_SIZE_WORDS[value.trim()];
        if (named !== undefined) {
            props.__fontScale = named;
        }
    }
    if (name === 'font-size' || name.startsWith('--') || !/[0-9.](em|lh)\b/.test(value)) {
        // Nothing's been said in `em` yet, which is nearly always, so there's nothing to look out for.
        if (EM_KEYS_SEEN.length === 0) {
            declareOne(props, name, value, fontSize, given);
            return;
        }
        const before = EM_KEYS_SEEN.map((key) => props[key]);
        declareOne(props, name, value, fontSize, given);
        // What's said in pixels now isn't a share of anything any more, which goes over anything said before, wherever that was.
        for (let at = 0; at < EM_KEYS_SEEN.length; at += 1) {
            const key = EM_KEYS_SEEN[at];
            if (props[key] !== before[at]) {
                props['__em_' + key] = undefined;
            }
        }
        return;
    }
    const before = snapshot(props);
    declareOne(props, name, value, fontSize, given);
    for (const key of Object.keys(props)) {
        const now = props[key];
        if (!key.startsWith('__') && now !== before.get(key) && typeof now === 'number' && fontSize > 0) {
            props['__em_' + key] = now / fontSize;
            if (!EM_KEYS_SEEN.includes(key)) {
                EM_KEYS_SEEN.push(key);
            }
        }
    }
}

/// What every prop's been, to see what a declaration changed.
function snapshot(props) {
    return new Map(Object.entries(props));
}

/// Which props have ever been said in `em`, so there's nothing to look out for until something has.
const EM_KEYS_SEEN = [];

/// `linear-gradient(...)` as what draws it takes it: `"angle|#rrggbbaa 0%|#rrggbbaa 20px|..."`, or `null` if it isn't one, or can't be.
function linearGradient(value) {
    const found = /^\s*(repeating-)?(linear|radial)-gradient\((.*)\)\s*$/is.exec(value);
    if (found === null) {
        return null;
    }
    const parts = commas(found[3]).map((part) => part.trim());
    // One with nothing in it, as one whose colors are variables nothing's set comes to, is none.
    if (parts.length === 0) {
        return null;
    }
    const kind = found[2].toLowerCase() === 'radial' ? radialKind(parts) : linearKind(parts);
    const stops = [];
    for (const part of parts) {
        const pieces = splitOutside(part, (c) => c === ' ');
        const paint = color(pieces[0]);
        if (paint === undefined) {
            return null;
        }
        // Always all eight digits, which is what's read.
        let digits = paint.slice(1);
        if (digits.length === 3 || digits.length === 4) {
            digits = [...digits].map((digit) => digit + digit).join('');
        }
        const hex = '#' + (digits.length === 6 ? digits + 'ff' : digits);
        // A stop with two places is two, of the same color.
        for (const place of pieces.length > 1 ? pieces.slice(1, 3) : [null]) {
            stops.push(place === null ? hex : hex + ' ' + (typeof length(place) === 'number' ? length(place) + 'px' : place));
        }
    }
    return stops.length === 0 ? null : (found[1] === undefined ? '' : '*') + kind + '|' + stops.join('|');
}

/// Where a radial gradient's middle is and how big it is, as `radial ellipse farthest-corner 50% 50%`, from what it says before its
/// colors, which is taken out of `parts` if it's there.
function radialKind(parts) {
    let shape = null;
    const size = [];
    let middle = ['50%', '50%'];
    const said = words(parts[0] ?? '');
    if (said.length > 0 && color(said[0]) === undefined) {
        parts.shift();
        const at = said.indexOf('at');
        for (const word of at < 0 ? said : said.slice(0, at)) {
            if (word === 'circle' || word === 'ellipse') {
                shape = word;
            } else {
                size.push(/^(closest|farthest)-/.test(word) || word.endsWith('%') ? word : (length(word) ?? 0) + 'px');
            }
        }
        if (at >= 0) {
            middle = placeOf(said.slice(at + 1));
        }
    }
    shape ??= size.length === 1 && !/^(closest|farthest)-/.test(size[0]) ? 'circle' : 'ellipse';
    return ['radial', shape, ...(size.length === 0 ? ['farthest-corner'] : size), ...middle].join(' ');
}

/// Where something is across and down, as `left top` or `30% 10px` says it, as a percentage or pixels each way.
function placeOf(said) {
    const one = (word) => Object.assign(Object.create(null), { left: '0%', top: '0%', center: '50%', right: '100%', bottom: '100%' })[word] ?? (word.endsWith('%') ? word : (length(word) ?? 0) + 'px');
    const upright = (word) => word === 'top' || word === 'bottom';
    if (said.length === 0) {
        return ['50%', '50%'];
    }
    if (said.length === 1) {
        return upright(said[0]) ? ['50%', one(said[0])] : [one(said[0]), '50%'];
    }
    const [first, second] = said;
    return upright(first) || second === 'left' || second === 'right' ? [one(second), one(first)] : [one(first), one(second)];
}

/// The angle a linear gradient goes at, in degrees, from what it says before its colors, which is taken out of `parts` if it's there.
function linearKind(parts) {
    let angle = 180;
    const first = parts[0].toLowerCase();
    const sides = Object.assign(Object.create(null), { top: 0, right: 90, bottom: 180, left: 270 });
    if (first.startsWith('to ')) {
        const where = first.slice(3).trim().split(/\s+/);
        if (where.length === 1) {
            angle = sides[where[0]] ?? 180;
        } else {
            // A corner, which is the angle between the sides it names.
            const [a, b] = where.map((side) => sides[side]);
            angle = a === 0 && b === 270 ? 315 : b === 0 && a === 270 ? 315 : (a + b) / 2;
        }
        parts.shift();
    } else if (/^-?[0-9.]+(deg|turn|rad|grad)$/.test(first) || first === '0') {
        const number = parseFloat(first);
        angle = first.endsWith('turn') ? number * 360 : first.endsWith('grad') ? number * 0.9 : first.endsWith('rad') ? (number * 180) / Math.PI : number;
        parts.shift();
    }
    return String(angle);
}

/// What a `background` or `background-image` with gradients or a picture in it says: each gradient, from the one on top, in
/// `backgroundGradient`, a line each, the first picture, and for the `background` shorthand, the color at the end. Says whether
/// there was a gradient or a picture.
function setBackgroundLayers(props, value, shorthand) {
    const layers = commas(value);
    const gradients = [];
    let found = false;
    let pictured = false;
    layers.forEach((layer, at) => {
        let rest = layer.trim();
        const start = /^(repeating-)?(linear|radial)-gradient\(/i.exec(rest);
        if (start !== null) {
            // Up to the bracket that closes it.
            let depth = 0;
            let end = rest.length - 1;
            for (let index = start[0].length - 1; index < rest.length; index += 1) {
                depth += rest[index] === '(' ? 1 : rest[index] === ')' ? -1 : 0;
                if (depth === 0) {
                    end = index;
                    break;
                }
            }
            const spec = linearGradient(rest.slice(0, end + 1));
            if (spec !== null) {
                gradients.push(spec);
            } else {
                notHere('background as ' + rest.slice(0, end + 1));
            }
            found = true;
            rest = rest.slice(end + 1);
        } else if (!pictured && /url\(/i.test(rest)) {
            pictured = true;
            found = setBackgroundUrl(props, rest, shorthand) || found;
            rest = '';
        }
        if (shorthand && at === layers.length - 1) {
            const paint = words(rest).map(color).find((one) => one !== undefined);
            if (paint !== undefined) {
                props.background = paint;
            }
        }
    });
    if (gradients.length > 0) {
        props.backgroundGradient = gradients.join('\n');
    }
    return found;
}

/// How much softer a `drop-shadow()` is than a `box-shadow` that says the same, as Chromium has them.
const DROP_SHADOW_BLUR = 2;

/// One to four values for the sides of something, as CSS writes them, as four: top, right, bottom and left, or for corners, top
/// left, top right, bottom right and bottom left.
function fourSides(values) {
    const [a, b = a, c = a, d = b] = values;
    return [a, b, c, d];
}

/// How big a corner is one way, as pixels, or a percentage of the box as text, which is only known when it's drawn.
function cornerSize(value, fontSize) {
    if (value === undefined) {
        return 0;
    }
    if (value.endsWith('%')) {
        const share = parseFloat(value);
        return Number.isFinite(share) ? share + '%' : 0;
    }
    const size = length(value, fontSize);
    return typeof size === 'number' ? Math.max(0, size) : 0;
}

/// The corners something has said, as `[across, down]` for each, from the top left going around.
function cornersOf(props) {
    if (typeof props.cornerRadii === 'string') {
        const parts = props.cornerRadii.split(' ').map((part) => (part.endsWith('%') ? part : Number(part)));
        return [0, 1, 2, 3].map((at) => [parts[at * 2], parts[at * 2 + 1]]);
    }
    const radius = typeof props.radius === 'number' ? props.radius : 0;
    return [0, 1, 2, 3].map(() => [radius, radius]);
}

/// Says what corners something has: one `radius` if they're all the same circle in pixels, which is nearly always and is drawn
/// most simply, or each of them in `cornerRadii` otherwise.
function setCorners(props, corners) {
    const flat = corners.flat();
    if (flat.every((one) => typeof one === 'number' && one === flat[0])) {
        props.radius = flat[0];
        delete props.cornerRadii;
        return;
    }
    props.cornerRadii = flat.join(' ');
    // What only takes one radius, as a picture does, takes the biggest that's in pixels, or is as round as it can be if every corner
    // is half of it or more.
    const allHalf = flat.every((one) => typeof one === 'string' && parseFloat(one) >= 50);
    props.radius = allHalf ? 9999 : Math.max(0, ...flat.filter((one) => typeof one === 'number'));
}

/// The kinds of border that are drawn other than as a line.
const DRAWN_BORDERS = new Set(['dashed', 'dotted', 'double', 'groove', 'ridge', 'inset', 'outset']);

/// The kinds of marker a list's items can have.
const LIST_TYPES = new Set(['none', 'disc', 'circle', 'square', 'decimal', 'decimal-leading-zero', 'lower-alpha', 'lower-latin', 'upper-alpha', 'upper-latin', 'lower-roman', 'upper-roman', 'disclosure-closed', 'disclosure-open']);

/// The words for a text size, as shares of the usual size, or of what's around it.
const FONT_SIZE_WORDS = Object.assign(Object.create(null), { 'xx-small': 0.5625, 'x-small': 0.625, small: 0.8125, medium: 1, large: 1.125, 'x-large': 1.5, 'xx-large': 2, smaller: 0.8333, larger: 1.2 });

/// Puts in what was said as a share of the size of the text, now that it's known: `parent` is the size of the text around it.
function resolveRelative(look, parent) {
    if (typeof look.__fontScale === 'number') {
        look.fontSize = look.__fontScale * parent;
    }
    delete look.__fontScale;
    const own = typeof look.fontSize === 'number' ? look.fontSize : parent;
    for (const key of Object.keys(look)) {
        if (key.startsWith('__em_')) {
            const share = look[key];
            if (typeof share === 'number') {
                look[key.slice(5)] = share * own;
            }
            delete look[key];
        }
    }
    return look;
}

/// What `inherit` is marked on for web.js, as `inherit_marker` in declare.rs has it.
const SHORTHAND_MARKERS = {
    'text-decoration-line': '__inherit_text-decoration', 'text-decoration-style': '__inherit_text-decoration',
    'text-decoration-color': '__inherit_text-decoration', 'text-decoration-thickness': '__inherit_text-decoration',
    'background-color': '__inherit_background', 'background-image': '__inherit_background',
    'outline-width': '__inherit_outline', 'outline-style': '__inherit_outline', 'outline-color': '__inherit_outline',
};
const INHERIT_MARKED = [
    'flex-wrap', 'flex-direction', 'flex-flow', 'flex', 'flex-grow', 'flex-shrink', 'flex-basis', 'align-items', 'align-self', 'align-content',
    'justify-content', 'gap', 'row-gap', 'column-gap', 'width', 'height', 'min-width', 'min-height', 'max-width', 'max-height', 'background',
    'background-color', 'background-image', 'border', 'border-color', 'border-width', 'border-style', 'border-radius', 'padding', 'margin',
    'box-shadow', 'opacity', 'overflow', 'overflow-x', 'overflow-y', 'position', 'top', 'right', 'bottom', 'left', 'z-index', 'text-decoration',
    'outline', 'box-sizing', 'vertical-align', 'transition', 'text-overflow',
];

/// The props a border's or an outline's width is said in, which are whole pixels, as a browser draws them.
const LINE_WIDTH_KEYS = ['borderWidth', 'borderTopWidth', 'borderRightWidth', 'borderBottomWidth', 'borderLeftWidth', 'outlineWidth'];

function declareOne(props, name, value, fontSize, given) {
    declareOneSaid(props, name, value, fontSize, given);
    if ((name.startsWith('border') || name.startsWith('outline')) && !name.includes('radius')) {
        for (const key of LINE_WIDTH_KEYS) {
            const width = props[key];
            if (typeof width === 'number') {
                const whole = width > 0 && width < 1 ? 1 : width >= 1 ? Math.floor(width + 1e-6) : width;
                if (whole !== width) {
                    props[key] = whole;
                }
            }
        }
    }
}

function declareOneSaid(props, name, value, fontSize, given) {
    // A border's or an outline's `thin`, `medium` and `thick` are 1, 3 and 5 pixels, as a browser has them.
    const LINE_WIDTHS = { thin: '1px', medium: '3px', thick: '5px' };
    if ((name.startsWith('border') || name.startsWith('outline')) && words(value).some((word) => Object.hasOwn(LINE_WIDTHS, word))) {
        value = words(value).map((word) => (Object.hasOwn(LINE_WIDTHS, word) ? LINE_WIDTHS[word] : word)).join(' ');
    }
    if (name.startsWith('border') && !name.includes('radius') && name !== 'border-collapse' && name !== 'border-spacing') {
        // A border that's said is what it is, rather than one drawn as the system draws a control.
        props.__ownBorder = true;
    }
    if (INHERIT_MARKED.includes(name)) {
        // The parent's own value, which web.js puts in when the element is drawn (see `withInherited` there).
        if (value === 'inherit') {
            props['__inherit_' + name] = name;
            return;
        }
        // What's said goes over an `inherit` before it, in the same block or in a rule it goes over.
        props['__inherit_' + name] = undefined;
    }
    // A part said after its shorthand's `inherit` goes over that too.
    const shorthand = Object.hasOwn(SHORTHAND_MARKERS, name) ? SHORTHAND_MARKERS[name] : undefined;
    if (shorthand !== undefined && value !== 'inherit') {
        props[shorthand] = undefined;
    }
    const passedDown = INHERITED_KEYS[name];
    if (passedDown !== undefined && (value === 'inherit' || value === 'unset' || (name === 'color' && value.toLowerCase() === 'currentcolor'))) {
        props[passedDown] = undefined;
        return;
    }
    const setSize = (key) => {
        const size = length(value, fontSize);
        const fit = key === 'width' ? 'fitWidth' : key === 'height' ? 'fitHeight' : null;
        if (fit !== null) {
            delete props[fit];
        }
        if (size !== undefined) {
            props[key] = size;
        } else if (fit !== null && (value === 'fit-content' || value === 'max-content' || value === 'min-content')) {
            // As big as what's in it, and not stretched to what it's in.
            delete props[key];
            props[fit] = true;
        } else if (FROM_INSIDE.has(value)) {
            // Its own size, over anything said before.
            props[key] = undefined;
        } else {
            notHere(name + ' as ' + value);
        }
    };
    const setNumber = (key) => {
        const size = length(value, fontSize);
        if (typeof size === 'number') {
            props[key] = size;
        } else if (size !== undefined) {
            notHere(name + ' as a percentage');
        } else if (FROM_INSIDE.has(value)) {
            props[key] = undefined;
        } else {
            notHere(name + ' as ' + value);
        }
    };
    const setColor = (key, said = value) => {
        if (said === '') {
            return;
        }
        if (said.toLowerCase() === 'currentcolor' && key !== 'color') {
            // The color of its text, which is only known once it's all been worked out.
            props[key] = 'currentcolor';
            return;
        }
        if (said === 'inherit' || said.toLowerCase() === 'currentcolor' || said === 'unset' || said === 'initial') {
            // What it's in decides, which is what having none here does.
            props[key] = undefined;
            return;
        }
        const paint = color(said);
        if (paint !== undefined) {
            props[key] = paint;
        } else {
            notHere(name + ' as ' + said);
        }
    };
    switch (name) {
        case 'display':
            if (value !== 'none' && TABLE_PARTS[value] !== undefined) {
                // A part of a table is a block, with what's in it laid out by the table it's in.
                props.tablePart = TABLE_PARTS[value];
                props.__block = true;
                props.__inline = false;
                props.__hidden = false;
                break;
            }
            // What's said of something that's a part of a table by itself, as a `<td>` is, goes over that.
            if (value !== 'none') {
                props.tablePart = undefined;
            }
            switch (value) {
                case 'none':
                    props.__hidden = true;
                    break;
                // One display takes the place of another, so each says what the others would have said.
                case 'flex':
                case 'inline-flex':
                    props.__flex = true;
                    props.__inline = value === 'inline-flex';
                    props.display = undefined;
                    props.__lineBox = undefined;
                    props.__listItem = false;
                    if (props.direction === undefined) {
                        props.direction = 'row';
                        props.__directionByDisplay = true;
                    }
                    props.__hidden = false;
                    break;
                case 'inline-block':
                case 'inline':
                    props.__inline = true;
                    // Only what's `inline-block` is a box of its own on its line, as tall as it is.
                    props.__lineBox = value === 'inline-block';
                    delete props.__block;
                    props.__flex = undefined;
                    props.display = undefined;
                    props.__listItem = false;
                    props.__hidden = false;
                    break;
                case 'grid':
                case 'inline-grid':
                    props.display = 'grid';
                    props.__flex = undefined;
                    props.__listItem = false;
                    props.__hidden = false;
                    break;
                case 'contents':
                    props.display = 'contents';
                    props.__flex = undefined;
                    props.__listItem = false;
                    props.__hidden = false;
                    break;
                case 'block':
                case 'flow-root':
                case 'list-item':
                case 'table':
                    // Not on a line with what's around it, which matters for a picture.
                    props.__block = true;
                    props.__inline = false;
                    props.__flex = undefined;
                    props.display = undefined;
                    props.__lineBox = undefined;
                    // Only a list item has a marker, as `display: block` on an `<li>` takes it away.
                    props.__listItem = value === 'list-item';
                    props.__hidden = false;
                    if (value === 'flow-root') {
                        // What's first and last in it keeps its margins in it.
                        props.__ownMargins = true;
                    }
                    break;
                default:
                    props.__hidden = false;
            }
            break;
        case 'flex-direction':
            props.__directionByDisplay = undefined;
            props.direction = value.startsWith('column') ? 'column' : 'row';
            // From the end to the start, which is the other way along the same line. Saying it isn't is said too, so that
            // `sm:flex-row` after `flex-col-reverse` (shadcn/ui's dialog footer) turns it back, as each rule's props are
            // put together after they're worked out.
            props.reverse = value.endsWith('-reverse');
            break;
        case 'order': {
            const order = parseInt(value, 10);
            if (!Number.isNaN(order)) {
                props.__order = order;
            }
            break;
        }
        case 'align-items':
            props.align = ALIGN[value] ?? 'stretch';
            break;
        case 'justify-self':
            props.justifySelf = ALIGN[value] ?? 'stretch';
            break;
        case 'align-self':
            props.alignSelf = ALIGN[value] ?? 'stretch';
            break;
        case 'justify-content':
            props.justify = value === 'normal' || value === 'stretch' ? undefined : JUSTIFY[value] ?? 'start';
            break;
        case 'justify-items':
            props.justifyItems = ALIGN[value] ?? 'stretch';
            break;
        case 'place-items': {
            const [across, along = across] = words(value);
            props.align = ALIGN[across] ?? 'stretch';
            props.justifyItems = ALIGN[along] ?? 'stretch';
            break;
        }
        case 'flex':
            setFlex(props, value, fontSize);
            break;
        case 'flex-grow':
            props.grow = parseFloat(value);
            break;
        case 'flex-shrink':
            props.shrink = parseFloat(value);
            break;
        case 'flex-basis':
            setBasis(props, value, fontSize);
            break;
        case 'width':
        case 'height':
            setSize(name);
            break;
        case 'min-width': {
            // A share of what it's in, as `max-width`'s can be, which is only known once that's laid out.
            const size = length(value, fontSize);
            if (typeof size === 'string' && size.includes('%')) {
                props.minWidth = size;
            } else {
                setNumber('minWidth');
            }
            break;
        }
        case 'max-width': {
            // A share of what it's in, which is only known once that's laid out.
            const size = length(value, fontSize);
            if (typeof size === 'string' && size.includes('%')) {
                props.maxWidth = size;
            } else {
                setNumber('maxWidth');
            }
            break;
        }
        case 'min-height':
            setNumber('minHeight');
            break;
        case 'max-height':
            setNumber('maxHeight');
            break;
        case 'margin':
        case 'padding':
            setSides(props, name, value, fontSize);
            break;
        case 'margin-top':
        case 'margin-right':
        case 'margin-bottom':
        case 'margin-left':
        case 'padding-top':
        case 'padding-right':
        case 'padding-bottom':
        case 'padding-left': {
            const [kind, side] = name.split('-');
            const key = side[0].toUpperCase() + side.slice(1);
            const size = length(value, fontSize);
            if (typeof size === 'string' && size.includes('%')) {
                props[kind + key] = 0;
                props[kind + key + 'Share'] = size;
            } else {
                setNumber(kind + key);
                if (kind + key + 'Share' in props) {
                    props[kind + key + 'Share'] = undefined;
                }
            }
            if (kind === 'margin') {
                props['__auto' + key] = value === 'auto';
            }
            break;
        }
        case 'gap':
        case 'grid-gap': {
            const [rows, columns = rows] = words(value);
            const row = length(rows, fontSize);
            const column = length(columns, fontSize);
            if (typeof row === 'number') {
                props.gap = row;
                props.rowGap = row;
            }
            if (typeof column === 'number') {
                props.columnGap = column;
            }
            break;
        }
        case 'row-gap':
        case 'grid-row-gap':
            setNumber('rowGap');
            break;
        case 'column-gap':
        case 'grid-column-gap':
            setNumber('columnGap');
            break;
        case 'position':
            delete props.__sticky;
            props.__fixed = value === 'fixed';
            if (value === 'absolute' || value === 'fixed') {
                props.position = 'absolute';
            } else if (value === 'relative') {
                props.position = 'relative';
            } else if (value === 'static') {
                // Where it would be, and drawn with what isn't placed, which is what nothing saying anything comes to.
                delete props.position;
            } else if (value === 'sticky' || value === '-webkit-sticky') {
                props.position = 'relative';
                props.__sticky = true;
            }
            break;
        case 'top':
        case 'left':
        case 'right':
        case 'bottom':
            setSize(name);
            break;
        case 'inset': {
            const [top, right = top, bottom = top, left = right] = words(value).map((part) => length(part, fontSize));
            Object.assign(props, { top, right, bottom, left });
            break;
        }
        case 'overflow':
        case 'overflow-x':
        case 'overflow-y': {
            const moves = (one) => one === 'auto' || one === 'scroll' || one === 'overlay';
            const [first, second = first] = words(value);
            const across = name === 'overflow-y' ? false : moves(first);
            // `overflow` is across and then down, and each of the others is only that one.
            const down = name === 'overflow-x' ? false : moves(name === 'overflow' ? second : first);
            const was = props.__scroll;
            const wasAcross = was === 'horizontal' || was === 'both';
            const wasDown = was === 'vertical' || was === 'both';
            const isAcross = name === 'overflow-y' ? wasAcross : across;
            const isDown = name === 'overflow-x' ? wasDown : down;
            props.__scroll = isAcross && isDown ? 'both' : isAcross ? 'horizontal' : isDown ? 'vertical' : undefined;
            // `scroll` has a scrollbar whether there's more to see or not, which takes room.
            props.__scrollAlways = words(value).includes('scroll');
            // What doesn't scroll and is hidden is cut off at its edges, as what scrolls is.
            const cut = words(value).some((one) => one === 'hidden' || one === 'clip');
            if (props.__scroll === undefined && cut) {
                props.clip = true;
            } else if (name === 'overflow') {
                delete props.clip;
            }
            break;
        }
        case 'background':
        case 'background-color': {
            if (value === '') {
                break;
            }
            if (name === 'background') {
                withoutBackgroundLayers(props);
            }
            if (value === 'none' || value === 'inherit' || value === 'unset' || value === 'initial') {
                props.background = '#00000000';
                break;
            }
            if (value.toLowerCase() === 'currentcolor') {
                props.background = 'currentcolor';
                break;
            }
            if (name === 'background') {
                if (setBackgroundLayers(props, value, true)) {
                    break;
                }
            }
            const paint = color(value);
            if (paint !== undefined) {
                props.background = paint;
                break;
            }
            const gradient = linearGradient(value);
            if (gradient !== null) {
                props.backgroundGradient = gradient;
                break;
            }
            // An image, or a gradient that can't be drawn, with the first color there is in it, if it has one, as the nearest thing to that.
            notHere('background as ' + value);
            const inside = /(#[0-9a-fA-F]{3,8}|rgba?\([^)]*\)|hsla?\([^)]*\))/.exec(value);
            if (inside !== null) {
                props.background = color(inside[1]);
            }
            break;
        }
        case 'background-image': {
            props.backgroundImage = undefined;
            props.backgroundGradient = undefined;
            if (!setBackgroundLayers(props, value, false) && value !== 'none') {
                notHere(name + ' as ' + value);
            }
            break;
        }
        case 'grid-template-areas':
            if (value === 'none') {
                delete props.gridAreas;
            } else {
                props.gridAreas = value;
            }
            break;
        case 'grid-area': {
            // A name, or where it starts and ends, as `row / column / row / column`.
            const parts = value.split('/').map((part) => part.trim());
            if (parts.length === 1 && /^[A-Za-z_-][\w-]*$/.test(parts[0]) && parts[0] !== 'auto') {
                props.gridArea = parts[0];
            } else {
                const [rowStart, columnStart = 'auto', rowEnd, columnEnd] = parts;
                props.gridRow = rowEnd === undefined ? rowStart : rowStart + ' / ' + rowEnd;
                props.gridColumn = columnEnd === undefined ? columnStart : columnStart + ' / ' + columnEnd;
            }
            break;
        }
        case '-rsui-own-margins':
            // What's first and last in it keeps its margins in it, as `<fieldset>`'s do: for the browser's own CSS.
            props.__ownMargins = true;
            break;
        case 'all':
            // Everything as if nothing had said anything about it, and what comes after it in the same rule says what it says.
            if (value === 'unset' || value === 'initial' || value === 'inherit') {
                for (const key of allKeys()) {
                    props[key] = undefined;
                }
            }
            break;
        case '-rsui-native-border':
            // How much more room a control's border takes than it looks like it does, as a browser's take 2 pixels and draw
            // 1, which only counts while nothing says what its border is.
            props.__nativeBorder = length(value);
            delete props.__ownBorder;
            break;
        case '-rsui-natural-width':
            // How wide a field is if nothing says, which stretching across what it's in still changes, as a browser's does.
            props.naturalWidth = length(value);
            break;
        case 'resize':
            props.__resize = commas(value)[0]?.trim() ?? value;
            break;
        case 'field-sizing':
            if ((commas(value)[0]?.trim() ?? value) === 'content') {
                props.fieldSizing = 'content';
            } else {
                delete props.fieldSizing;
            }
            break;
        case 'accent-color':
            if (value === 'auto') {
                delete props.accentColor;
            } else {
                setColor('accentColor');
            }
            break;
        case 'scrollbar-width':
            if (value === 'auto' || value === 'thin' || value === 'none') {
                props.__scrollbarWidth = value;
            } else {
                notHere(name + ' as ' + value);
            }
            break;
        case 'scrollbar-color': {
            const parts = words(value);
            if (value === 'auto') {
                props.__scrollbarThumb = undefined;
                props.__scrollbarTrack = undefined;
                props.__scrollbarColor = 'auto';
            } else if (parts.length === 2) {
                const whole = value;
                value = parts[0];
                setColor('__scrollbarThumb');
                value = parts[1];
                setColor('__scrollbarTrack');
                value = whole;
                props.__scrollbarColor = 'said';
            } else {
                notHere(name + ' as ' + value);
            }
            break;
        }
        case 'background-clip':
        case '-webkit-background-clip':
            // Text that shows its background through it, as gradient text does.
            if (value === 'text') {
                props.backgroundClip = 'text';
            } else {
                props.backgroundClip = undefined;
            }
            break;
        case 'clip-path':
            if (value === 'none') {
                delete props.clipPath;
            } else {
                props.clipPath = value;
                if (value.startsWith('inset(')) {
                    props.clip = true;
                }
            }
            break;
        case 'filter':
            if (value === 'none') {
                delete props.cssFilter;
            } else {
                props.cssFilter = value;
                // A shadow of a box is the shadow of what it shows, which is what it is if its background covers it.
                const shadow = /drop-shadow\(([^()]*(?:\([^()]*\)[^()]*)*)\)/.exec(value);
                if (shadow !== null && props.shadowBlur === undefined && props.shadows === undefined) {
                    const parts = words(shadow[1]);
                    const sizes = parts.map((part) => length(part)).filter((size) => typeof size === 'number');
                    const paint = parts.map(color).find((one) => one !== undefined);
                    setShadow(props, `${sizes[0] ?? 0}px ${sizes[1] ?? 0}px ${(sizes[2] ?? 0) * DROP_SHADOW_BLUR}px ${paint ?? 'black'}`);
                }
                if (/(blur|url)\(/.test(value)) {
                    notHere('filter as ' + value);
                }
            }
            break;
        case 'background-size':
            {
                const said = (commas(value)[0] ?? value).trim();
                props.backgroundSize = said.split(/\s+/).some((word) => word.endsWith('em') && !word.endsWith('rem')) ? said : words(said).map((word) => {
                    const unitless = word.endsWith('%') || /^[a-zA-Z-]*$/.test(word);
                    const size = unitless ? undefined : length(word, fontSize);
                    return typeof size === 'number' ? size + 'px' : word;
                }).join(' ');
            }
            break;
        case 'background-position':
            props.backgroundPosition = positionInPixels(commas(value)[0]?.trim() ?? value, fontSize);
            break;
        case 'background-repeat':
            props.backgroundRepeat = commas(value)[0]?.trim() ?? value;
            break;
        case 'color':
            setColor('color');
            break;
        case 'fill':
            setColor('fill');
            break;
        case 'stroke':
            setColor('stroke');
            break;
        case 'stroke-width':
            setNumber('strokeWidth');
            break;
        case 'stroke-linecap':
            props.strokeLinecap = value;
            break;
        case 'stroke-linejoin':
            props.strokeLinejoin = value;
            break;
        case 'stroke-dasharray':
            props.strokeDasharray = value;
            break;
        case 'stroke-dashoffset':
            setNumber('strokeDashoffset');
            break;
        case 'opacity': {
            const opacity = value.endsWith('%') ? parseFloat(value) / 100 : parseFloat(value);
            if (Number.isFinite(opacity)) {
                props.opacity = opacity;
            } else {
                delete props.opacity;
            }
            break;
        }
        case 'border':
            setBorder(props, value);
            break;
        case 'border-top':
        case 'border-right':
        case 'border-bottom':
        case 'border-left': {
            const side = name.slice(7);
            const key = side[0].toUpperCase() + side.slice(1);
            const one = {};
            setBorder(one, value);
            if (one.borderWidth !== undefined) {
                props['border' + key + 'Width'] = one.borderWidth;
            }
            if (one.borderColor !== undefined) {
                props['border' + key + 'Color'] = one.borderColor;
            }
            if (typeof one.borderStyle === 'string' && one.borderStyle !== 'solid') {
                props.borderStyle = one.borderStyle;
            }
            break;
        }
        case 'border-top-width':
        case 'border-right-width':
        case 'border-bottom-width':
        case 'border-left-width': {
            const side = name.split('-')[1];
            setNumber('border' + side[0].toUpperCase() + side.slice(1) + 'Width');
            break;
        }
        case 'border-top-color':
        case 'border-right-color':
        case 'border-bottom-color':
        case 'border-left-color': {
            const side = name.split('-')[1];
            setColor('border' + side[0].toUpperCase() + side.slice(1) + 'Color');
            break;
        }
        case 'border-width': {
            withoutBorderSides(props);
            const parts = words(value);
            if (parts.length < 2) {
                setNumber('borderWidth');
                break;
            }
            // Each side as it says, in the order CSS has them, the ones not said taking the opposite one's.
            const [top, right = top, bottom = top, left = right] = parts.map((part) => length(part, fontSize));
            for (const [side, size] of [['Top', top], ['Right', right], ['Bottom', bottom], ['Left', left]]) {
                if (typeof size === 'number') {
                    props['border' + side + 'Width'] = size;
                }
            }
            props.borderWidth = 0;
            break;
        }
        case 'border-color': {
            const parts = words(value);
            if (parts.length < 2) {
                withoutBorderSideColors(props);
                setColor('borderColor');
                break;
            }
            // Each side's own, the ones not said taking the opposite one's.
            const [top, right = top, bottom = top, left = right] = parts;
            setColor('borderColor', top);
            for (const [side, paint] of [['Top', top], ['Right', right], ['Bottom', bottom], ['Left', left]]) {
                setColor('border' + side + 'Color', paint);
            }
            break;
        }
        case 'border-style':
            if (value === 'none' || value === 'hidden') {
                withoutBorderSides(props);
                props.borderWidth = 0;
            } else {
                const first = words(value)[0];
                props.borderStyle = DRAWN_BORDERS.has(first) ? first : 'solid';
            }
            break;
        case 'outline':
            setOutline(props, value);
            break;
        case 'outline-width':
            setNumber('outlineWidth');
            if (typeof length(value) === 'number') {
                props.cssOutlineWidth = length(value);
            }
            break;
        case 'outline-color':
            setColor('outlineColor');
            break;
        case 'outline-offset':
            setNumber('outlineOffset');
            break;
        case 'outline-style':
            if (value === 'none' || value === 'hidden') {
                props.outlineWidth = 0;
            } else if (props.outlineWidth === undefined) {
                props.outlineWidth = 3;
            }
            setOutlineStyle(props, value);
            break;
        case 'border-radius': {
            // Up to four across, then up to four down after a `/`, from the top left going around, as CSS has it.
            const [across, down = across] = value.split('/').map((part) => fourSides(words(part.trim())));
            setCorners(props, [0, 1, 2, 3].map((at) => [cornerSize(across[at], fontSize), cornerSize(down[at], fontSize)]));
            break;
        }
        case 'border-top-left-radius':
        case 'border-top-right-radius':
        case 'border-bottom-right-radius':
        case 'border-bottom-left-radius': {
            const at = ['border-top-left-radius', 'border-top-right-radius', 'border-bottom-right-radius', 'border-bottom-left-radius'].indexOf(name);
            const [first, second = first] = words(value);
            const corners = cornersOf(props);
            corners[at] = [cornerSize(first, fontSize), cornerSize(second, fontSize)];
            setCorners(props, corners);
            break;
        }
        case 'box-shadow':
            setShadow(props, value);
            break;
        case 'font-size': {
            const size = length(value, fontSize);
            if (typeof size === 'number') {
                props.fontSize = size;
            }
            break;
        }
        case 'font-weight':
            props.fontWeight = fontWeight(value);
            break;
        case 'font-family':
            props.fontFamily = fontFamily(value);
            break;
        case 'font-style':
            props.fontStyle = value === 'italic' || value === 'oblique' ? 'italic' : 'normal';
            break;
        case 'font': {
            if (value === 'inherit' || value === 'unset') {
                for (const key of ['fontFamily', 'fontSize', 'fontWeight', 'fontStyle', 'lineHeight', '__lineScale', '__fontScale']) {
                    props[key] = undefined;
                }
                break;
            }
            const parts = words(value);
            let weight = 'normal';
            let italic = false;
            const STRETCH = ['normal', 'small-caps', 'ultra-condensed', 'extra-condensed', 'condensed', 'semi-condensed', 'semi-expanded', 'expanded', 'extra-expanded', 'ultra-expanded'];
            for (const [at, part] of parts.entries()) {
                const lower = part.toLowerCase();
                if (STRETCH.includes(lower)) {
                    continue;
                }
                if (lower === 'bold' || lower === 'bolder' || lower === 'lighter' || /^[1-9]00$/.test(part)) {
                    weight = part;
                    continue;
                }
                if (lower === 'italic' || lower === 'oblique') {
                    italic = true;
                    continue;
                }
                const slash = part.indexOf('/');
                const sizeText = slash < 0 ? part : part.slice(0, slash);
                let line = slash < 0 || slash === part.length - 1 ? null : part.slice(slash + 1);
                let familyAt = at + 1;
                const next = parts[at + 1];
                if (line === null && (part.endsWith('/') || (next !== undefined && next.startsWith('/')))) {
                    if (next === '/') {
                        line = parts[at + 2] ?? null;
                        familyAt = at + 3;
                    } else if (next !== undefined && next.startsWith('/')) {
                        line = next.slice(1);
                        familyAt = at + 2;
                    } else {
                        line = next ?? null;
                        familyAt = at + 2;
                    }
                }
                const size = length(sizeText, fontSize);
                if (typeof size !== 'number') {
                    break;
                }
                props.fontWeight = fontWeight(weight);
                props.fontStyle = italic ? 'italic' : 'normal';
                props.fontSize = size;
                const share = /^(-?[0-9]*\.?[0-9]+)(em|%)$/.exec(sizeText.trim());
                props.__fontScale = share === null ? null : Number(share[1]) / (share[2] === '%' ? 100 : 1);
                if (FONT_SIZE_WORDS[sizeText.trim()] !== undefined) {
                    props.__fontScale = FONT_SIZE_WORDS[sizeText.trim()];
                }
                if (line !== null) {
                    declareOne(props, 'line-height', line, size, given);
                }
                if (parts.length > familyAt) {
                    props.fontFamily = fontFamily(parts.slice(familyAt).join(' '));
                }
                break;
            }
            break;
        }
        case 'text-align':
            if (value === 'left' || value === 'center' || value === 'right') {
                props.textAlign = value;
            } else if (value === 'start' || value === 'end') {
                // Which side that is depends on which way the text goes, which is only known where the text is.
                props.textAlign = value;
            }
            break;
        case 'direction':
            props.textDirection = value === 'rtl' ? 'rtl' : 'ltr';
            break;
        case 'unicode-bidi':
            if (['normal', 'embed', 'isolate', 'bidi-override', 'isolate-override', 'plaintext'].includes(value)) {
                props.unicodeBidi = value;
            }
            break;
        case '-webkit-app-region':
        case 'app-region':
            // Taking hold of it moves the window, as a title bar does in a window with no frame of its own, apart from what
            // says it doesn't, and what can be clicked.
            if (value === 'drag' || value === 'no-drag') {
                props.dragsWindow = value === 'drag';
            }
            break;
        case 'text-decoration':
        case 'text-decoration-line': {
            // Which lines, and, for all of it at once, how they look, with what isn't said going back to how it is by itself.
            const said = words(value.replace(/,/g, ' '));
            props.underline = said.includes('underline');
            props.lineThrough = said.includes('line-through');
            props.overline = said.includes('overline');
            if (name === 'text-decoration') {
                const style = said.find((word) => DECORATION_STYLES.has(word));
                const paint = said.map((word) => color(word)).find((found) => found !== undefined);
                const thick = said.map((word) => length(word, fontSize)).find((found) => typeof found === 'number');
                props.decorationStyle = style ?? 'solid';
                props.decorationColor = paint;
                props.decorationThickness = thick;
            }
            break;
        }
        case 'text-decoration-style':
            props.decorationStyle = value;
            break;
        case 'text-decoration-color':
            setColor('decorationColor');
            break;
        case 'text-decoration-thickness':
            setNumber('decorationThickness');
            break;
        case 'text-underline-offset':
            setNumber('underlineOffset');
            break;
        case 'content':
            // What's in `::before` and `::after`: it's only there at all if it says this. Text, counters and attributes together
            // are worked out for each element it's on.
            if (/(counter|attr)\(/.test(value)) {
                props.__content = { template: contentTemplate(value) };
            } else if (value !== 'none' && value !== 'normal') {
                // What's in quotes, with what CSS writes as `\201C` as the character it is.
                props.__content = value
                    .replace(/^["']|["']$/g, '')
                    .replace(/\\([0-9a-fA-F]{1,6})\s?/g, (_, hex) => String.fromCodePoint(parseInt(hex, 16)))
                    .replace(/\\(.)/g, '$1');
            }
            break;
        case 'line-height': {
            if (value === 'normal' || value === 'inherit' || value === 'unset' || value === 'initial') {
                const said = value === 'normal' || value === 'initial' ? null : undefined;
                props.lineHeight = said;
                props.__lineScale = said;
                break;
            }
            const plain = /^[0-9.]+$/.test(value);
            // A `calc()` of numbers without units is a number too, as Tailwind's `text-sm` says `calc(1.25 / .875)`.
            const unitlessCalc = value.startsWith('calc(') && !/[a-zA-Z%]/.test(value.slice(5));
            const calced = unitlessCalc ? length(value, 1) : undefined;
            const scale = plain ? parseFloat(value) : typeof calced === 'number' ? calced : null;
            const size = scale !== null ? scale * fontSize : length(value, fontSize);
            // A number on its own is times the size of the text, whatever that turns out to be, here and in everything inside it.
            props.__lineScale = scale;
            if (typeof size === 'number') {
                props.lineHeight = size;
            } else if (typeof size === 'string' && size.endsWith('%')) {
                props.lineHeight = (parseFloat(size) * fontSize) / 100;
            }
            break;
        }
        case 'list-style':
        case 'list-style-type': {
            const type = words(value).find((word) => LIST_TYPES.has(word));
            if (type !== undefined) {
                props.__listStyle = type;
            } else if (name === 'list-style-type' || words(value).includes('none')) {
                props.__listStyle = 'none';
            }
            break;
        }
        case 'vertical-align':
            // Above or below the text around it, which is the only way text inside text goes.
            if (value === 'super' || value === 'sub') {
                props.verticalAlign = value;
            } else if (value === 'baseline') {
                delete props.verticalAlign;
                delete props.__lineAlign;
            }
            // A box on a line at its top, middle or bottom, rather than on the line's text.
            if (value === 'top' || value === 'text-top') {
                props.__lineAlign = 'start';
            } else if (value === 'bottom' || value === 'text-bottom') {
                props.__lineAlign = 'end';
            } else if (value === 'middle') {
                props.__lineAlign = 'center';
            }
            break;
        case '-webkit-line-clamp':
        case 'line-clamp': {
            // Only so many lines, the last of which ends in `…` if there's more.
            const lines = parseInt(value, 10);
            if (lines > 0) {
                props.maxLines = lines;
            } else {
                delete props.maxLines;
            }
            break;
        }
        case '-webkit-box-orient':
            break;
        case 'overflow-wrap':
        case 'word-wrap':
            // A word too wide for a line is broken, rather than going past the edge.
            props.keepWords = !(value === 'break-word' || value === 'anywhere');
            break;
        case 'word-break':
            if (value === 'break-all' || value === 'break-word') {
                props.keepWords = false;
            } else if (value === 'keep-all' || value === 'normal') {
                props.keepWords = true;
            }
            // Lines end anywhere, not only where a word's too long.
            if (value === 'break-all') {
                props.wordBreak = 'break-all';
            } else {
                delete props.wordBreak;
            }
            break;
        case 'letter-spacing': {
            // `normal` is none, and so is anything that isn't a length.
            const spacing = length(value, fontSize);
            props.letterSpacing = typeof spacing === 'number' ? spacing : 0;
            break;
        }
        case 'word-spacing': {
            // `normal` is none, and so is anything that isn't a length.
            const spacing = length(value, fontSize);
            props.wordSpacing = typeof spacing === 'number' ? spacing : 0;
            break;
        }
        case 'text-indent': {
            const indent = length(value, fontSize);
            if (typeof indent === 'number') {
                props.textIndent = indent;
            } else {
                notHere(name + ' as ' + value);
            }
            break;
        }
        case 'text-transform':
            if (value === 'uppercase' || value === 'lowercase' || value === 'capitalize' || value === 'none') {
                props.__case = value;
            }
            break;
        case 'text-overflow':
            if (value === 'ellipsis') {
                props.ellipsis = true;
            }
            break;
        case 'white-space':
            props.__whiteSpace = value;
            if (value === 'nowrap' || value === 'pre') {
                props.noWrap = true;
            } else {
                delete props.noWrap;
            }
            break;
        case 'visibility':
            props.__visibility = value;
            break;
        case 'content-visibility':
            props.__contentVisibility = value;
            break;
        case 'contain-intrinsic-size':
            props.__intrinsicSize = value;
            break;
        case 'contain-intrinsic-height':
        case 'contain-intrinsic-block-size':
            props.__intrinsicHeight = value;
            break;
        case 'contain-intrinsic-width':
        case 'contain-intrinsic-inline-size':
            props.__intrinsicWidth = value;
            break;
        case 'user-select':
        case '-webkit-user-select':
            props.__userSelect = value;
            break;
        case 'appearance':
        case '-webkit-appearance':
        case '-moz-appearance':
            props.__appearance = value;
            break;
        case 'cursor':
            props.cursor = value === 'pointer' || value === 'text' ? value : 'default';
            break;
        case 'pointer-events':
            props.inert = value === 'none';
            break;
        case 'object-fit':
            if (value === 'contain' || value === 'cover' || value === 'fill') {
                props.fit = value;
            }
            break;
        case 'transform':
            if (value !== 'none') {
                props.transform = value;
            }
            break;
        case 'transform-origin':
            props.transformOrigin = value;
            break;
        case 'translate':
            props.__translate = value;
            break;
        case 'rotate':
            props.__rotate = value;
            break;
        case 'scale':
            props.__scale = value;
            break;
        case 'border-collapse':
            props.__borderCollapse = value === 'collapse';
            break;
        case 'border-spacing': {
            const spacing = length(words(value)[0], fontSize);
            if (typeof spacing === 'number') {
                props.__borderSpacing = spacing;
            }
            break;
        }
        case 'box-sizing':
            // `inherit` is nearly always from `html { box-sizing: border-box }`, which is what it comes to.
            props.__boxSizing = value === 'inherit' ? 'border-box' : value;
            break;
        case 'aspect-ratio': {
            const [wide, high = '1'] = value.split('/').map((part) => part.trim());
            const ratio = Number(wide) / Number(high);
            if (Number.isFinite(ratio) && ratio > 0) {
                props.__aspect = ratio;
            } else if (value !== 'auto') {
                notHere('aspect-ratio as ' + value);
            }
            break;
        }
        case 'flex-wrap':
            props.flexWrap = value === 'wrap' || value === 'wrap-reverse';
            if (value === 'wrap-reverse') {
                notHere('flex-wrap: wrap-reverse, which wraps the usual way instead,');
            }
            break;
        case 'flex-flow':
            for (const word of words(value)) {
                if (word === 'wrap' || word === 'wrap-reverse' || word === 'nowrap') {
                    declare(props, 'flex-wrap', word, fontSize);
                } else {
                    declare(props, 'flex-direction', word, fontSize);
                }
            }
            break;
        case 'grid-auto-flow':
            props.__flow = value;
            break;
        case 'align-content':
            props.__alignContent = value;
            break;
        case 'grid-template-columns':
            props.columns = gridTracks(value, fontSize);
            break;
        case 'grid-template-rows':
            props.rows = gridTracks(value, fontSize);
            break;
        case 'grid-auto-rows':
            props.gridAutoRows = gridTracks(value, fontSize);
            break;
        case 'grid-auto-columns':
            props.gridAutoColumns = gridTracks(value, fontSize);
            break;
        case 'grid-column':
            props.gridColumn = value;
            break;
        case 'grid-row':
            props.gridRow = value;
            break;
        case 'grid-column-start':
            props.gridColumnStart = value;
            break;
        case 'grid-column-end':
            props.gridColumnEnd = value;
            break;
        case 'grid-row-start':
            props.gridRowStart = value;
            break;
        case 'grid-row-end':
            props.gridRowEnd = value;
            break;
        case 'z-index': {
            const order = parseInt(value, 10);
            if (!Number.isNaN(order)) {
                props.zIndex = order;
            }
            break;
        }
        case 'transition':
            props.__transition = value;
            delete props.__transitionParts;
            break;
        case 'transition-property':
        case 'transition-duration':
        case 'transition-delay':
        case 'transition-timing-function':
            props.__transitionParts = { ...props.__transitionParts, [name]: value };
            break;
        case 'text-shadow': {
            // The first shadow, as `x y blur color`, with the color as a number. It goes on to what's inside, as a font does.
            const first = commas(value)[0]?.trim();
            if (value === 'none' || first === undefined) {
                props.textShadow = 'none';
                break;
            }
            const sizes = [];
            let paint;
            for (const part of words(first)) {
                const size = length(part, fontSize);
                if (typeof size === 'number') {
                    sizes.push(size);
                } else if (color(part) !== undefined) {
                    paint = color(part);
                }
            }
            props.textShadow = `${sizes[0] ?? 0} ${sizes[1] ?? 0} ${sizes[2] ?? 0} ${colorNumber(paint ?? 'currentcolor') ?? colorNumber('#000000')}`;
            break;
        }
        case 'table-layout':
            props.__tableLayout = value;
            break;
        case 'caption-side':
            props.captionSide = value;
            break;
        case 'float':
            if (value === 'left' || value === 'right' || value === 'inline-start' || value === 'inline-end') {
                props.__float = value;
            } else {
                delete props.__float;
            }
            break;
        case 'clear':
            if (value === 'none') {
                delete props.__clear;
            } else {
                props.__clear = true;
            }
            break;
        case 'animation':
            props.__animation = value;
            delete props.__animationParts;
            break;
        case 'animation-name':
        case 'animation-duration':
        case 'animation-delay':
        case 'animation-iteration-count':
        case 'animation-timing-function':
        case 'animation-direction':
        case 'animation-fill-mode':
        case 'animation-play-state':
            props.__animationParts = { ...props.__animationParts, [name]: value };
            break;
        case 'font-variant-numeric': {
            const said = words(value);
            if (said.includes('tabular-nums')) {
                props.tabularNums = true;
            } else if (said.some((word) => word === 'proportional-nums' || word === 'normal')) {
                props.tabularNums = false;
            } else if (value === 'inherit' || value === 'unset') {
                props.tabularNums = undefined;
            }
            break;
        }
        default: {
            if (name === 'font-feature-settings' && tnumSetting(value) !== undefined) {
                props.tabularNums = tnumSetting(value);
                break;
            }
            if (/^(font-feature-settings|font-variation-settings|font-kerning|font-optical-sizing)$/.test(name) && /^(normal|auto|inherit|initial|unset)$/.test(value)) {
                break;
            }
            const prefix = /^-(webkit|moz|ms|o)-/.exec(name);
            if (prefix !== null) {
                const before = leftOut.length;
                declare(props, name.slice(prefix[0].length), value, fontSize);
                leftOut.length = before;
                break;
            }
            if (!SILENT.has(name) && !name.startsWith('--') && !name.startsWith('_')) {
                notHere(name);
            }
        }
    }
}

/// A color as rsui keeps it: red, green, blue and alpha in one number.
function colorNumber(text) {
    let hex = color(String(text));
    if (hex === undefined) {
        return undefined;
    }
    hex = hex.slice(1);
    if (hex.length <= 4) {
        hex = [...hex].map((digit) => digit + digit).join('');
    }
    if (hex.length === 6) {
        hex += 'ff';
    }
    return parseInt(hex, 16) >>> 0;
}

/// What `content` says, as text, counters and attributes, in order.
function contentTemplate(value) {
    const out = [];
    const each = /"((?:[^"\\]|\\.)*)"|'((?:[^'\\]|\\.)*)'|counters?\(\s*([\w-]+)\s*(?:,\s*(?:"[^"]*"\s*,\s*)?([\w-]+))?\s*\)|attr\(\s*([\w-]+)\s*\)|(open-quote|close-quote)/g;
    for (const found of value.matchAll(each)) {
        if (found[1] !== undefined || found[2] !== undefined) {
            const text = (found[1] ?? found[2])
                .replace(/\\([0-9a-fA-F]{1,6})\s?/g, (_, hex) => String.fromCodePoint(parseInt(hex, 16)))
                .replace(/\\(.)/g, '$1');
            out.push({ text });
        } else if (found[3] !== undefined) {
            out.push({ counter: found[3], style: found[4] ?? 'decimal' });
        } else if (found[5] !== undefined) {
            out.push({ attr: found[5] });
        } else {
            out.push({ text: found[6] === 'open-quote' ? '“' : '”' });
        }
    }
    return out;
}

/// What `content` comes to for one element: each counter as how many of its kind come before it and it, which is what a counter

/// Goes on an element's `style`, for a `style` object that's written for the web.
/// What a `style` object written for the web says, as it is before anything's been worked out from it. `fontSize` is what an `em`
/// is, if the object doesn't say.
function inlineLook(given, fontSize = 16) {
    const out = {};
    for (const name of Object.keys(given)) {
        const value = given[name];
        if (value === null || value === undefined || value === false) {
            continue;
        }
        const css = name.replace(/[A-Z]/g, (letter) => '-' + letter.toLowerCase());
        // A number's in pixels, as React has it, apart from these.
        const plain = typeof value === 'number' && !['opacity', 'flex', 'flex-grow', 'flex-shrink', 'z-index', 'font-weight', 'line-height', 'zoom', 'order'].includes(css);
        declare(out, css, plain ? value + 'px' : String(value), Number(given.fontSize) || fontSize);
    }
    return out;
}

/// What a browser says each element looks like by itself, which a page's own CSS goes over. It's Chromium's, as far as it goes.
const BROWSER_CSS = `
h1 { font-size: 2em; margin-top: 0.67em; margin-bottom: 0.67em; font-weight: bold; }
h2 { font-size: 1.5em; margin-top: 0.83em; margin-bottom: 0.83em; font-weight: bold; }
h3 { font-size: 1.17em; margin-top: 1em; margin-bottom: 1em; font-weight: bold; }
h4 { margin-top: 1.33em; margin-bottom: 1.33em; font-weight: bold; }
h5 { font-size: 0.83em; margin-top: 1.67em; margin-bottom: 1.67em; font-weight: bold; }
h6 { font-size: 0.67em; margin-top: 2.33em; margin-bottom: 2.33em; font-weight: bold; }
p, dl { margin-top: 1em; margin-bottom: 1em; }
blockquote, figure { margin: 1em 40px; }
ul { margin-top: 1em; margin-bottom: 1em; padding-left: 40px; list-style-type: disc; }
ol { margin-top: 1em; margin-bottom: 1em; padding-left: 40px; list-style-type: decimal; }
dd { margin-left: 40px; }
pre { font-family: monospace; white-space: pre; margin-top: 1em; margin-bottom: 1em; }
code, kbd, samp { font-family: monospace; }
b, strong, th { font-weight: bold; }
i, em, cite, dfn, var, address { font-style: italic; }
u, ins { text-decoration: underline; }
s, del { text-decoration: line-through; }
small, sub, sup { font-size: smaller; }
sub { vertical-align: sub; }
sup { vertical-align: super; }
samp, var { font-family: monospace; }
var, dfn { font-style: italic; }
mark { background-color: yellow; color: black; }
abbr { text-decoration: underline dotted; }
a { color: #0000ee; text-decoration: underline; cursor: pointer; }
hr { margin-top: 0.5em; margin-bottom: 0.5em; border-top: 1px solid #9a9a9a; border-bottom: 1px solid #eeeeee; border-left: 1px solid #9a9a9a; border-right: 1px solid #eeeeee; }
table { display: table; border-collapse: separate; border-spacing: 2px; }
caption { display: table-caption; text-align: center; }
thead, tbody, tfoot { display: table-row-group; }
tr { display: table-row; }
td, th { display: table-cell; }
col { display: table-column; }
colgroup { display: table-column-group; }
dialog { position: absolute; left: 0; right: 0; width: fit-content; height: fit-content; margin: auto; border: 3px solid black; padding: 1em; background-color: white; color: black; }
th { text-align: center; padding: 1px; }
td { padding: 1px; }
input:disabled, textarea:disabled, select:disabled, button:disabled { color: rgba(16, 16, 16, 0.3); }
input:disabled, textarea:disabled { background-color: rgba(239, 239, 239, 0.3); border-color: rgba(118, 118, 118, 0.3); }
button:disabled { border-color: rgba(118, 118, 118, 0.3); -rsui-native-border: 1px; }
fieldset { -rsui-own-margins: 1; margin-left: 2px; margin-right: 2px; padding: 0.35em 0.75em 0.625em; border: 2px groove #f0f0f0; }
legend { padding-left: 2px; padding-right: 2px; }
button { box-sizing: border-box; font-family: Arial; font-size: 13.3333px; line-height: normal; color: black; text-align: center; padding: 1px 6px; border: 1px solid #767676; border-radius: 2px; background-color: #efefef; -rsui-native-border: 1px; }
select { box-sizing: border-box; }
input { -rsui-natural-width: 169px; }
input, select { font-family: Arial; font-size: 13.3333px; color: black; padding: 2px 3px; border: 1px solid #767676; border-radius: 2px; background-color: white; }
select { padding: 0; }
checkbox { margin: 3px 3px 3px 4px; }
radio { margin: 3px 3px 0 5px; }
summary { list-style-type: disclosure-closed; }
progress { display: inline-flex; align-items: center; width: 10em; height: 1em; }
meter { display: inline-flex; align-items: center; width: 5em; height: 1em; }
textarea { font-family: monospace; font-size: 13.3333px; color: black; padding: 2px; border: 1px solid #767676; border-radius: 2px; background-color: white; }
`;

let browserLooks = null;
let allKeysMade = null;

/// What `all` says something about: every prop the browser's sheet says for anything, then how text is written.
function allKeys() {
    if (allKeysMade === null) {
        browserLook('div');
        const keys = [];
        for (const look of browserLooks.values()) {
            for (const key of Object.keys(look)) {
                if (!keys.includes(key)) {
                    keys.push(key);
                }
            }
        }
        for (const key of ['color', 'fontFamily', 'fontSize', 'fontWeight', 'fontStyle', 'lineHeight', '__lineScale', 'textAlign', 'letterSpacing',
            'wordSpacing', '__whiteSpace', '__case', 'textIndent', '__listStyle']) {
            if (!keys.includes(key)) {
                keys.push(key);
            }
        }
        allKeysMade = keys;
    }
    return allKeysMade;
}

/// What a browser says this element looks like by itself, as it is before anything's been worked out from it. An input's depends on
/// its `type`, as a checkbox isn't drawn as a field.
function browserLook(tag, type, disabled = false) {
    if (tag === 'input' && (type === 'checkbox' || type === 'radio')) {
        tag = type;
    } else if (tag === 'input' && (type === 'range' || type === 'hidden' || type === 'file' || type === 'color')) {
        return undefined;
    }
    if (browserLooks === null) {
        browserLooks = new Map();
        for (const rule of readCss(BROWSER_CSS).rules) {
            const look = lookFrom(rule.block.declarations);
            for (const name of commas(rule.selector)) {
                browserLooks.set(name.trim(), { ...browserLooks.get(name.trim()), ...look });
            }
        }
    }
    const own = browserLooks.get(tag);
    // What can't be used is fainter, as a browser's own CSS says, which a page's goes over.
    const faint = disabled ? browserLooks.get(tag + ':disabled') : undefined;
    return faint === undefined ? own : { ...own, ...faint };
}
