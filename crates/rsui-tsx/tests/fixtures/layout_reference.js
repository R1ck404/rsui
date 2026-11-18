// What rsui/web did in JavaScript to each element's look once CSS had said everything about it (web.js at f323b46), which
// src/layout.rs has to give exactly the same as: where it goes in what it's in, as `Styled` worked it out, how text is written in it, `settle` and `finalFont`.
// Only the tests load it. Everything's inside `layoutReference`, as web.js's own `settle` and `finalFont` are loaded beside it.
// It uses `declaring` and `motionProps`, which the test gives it.
//
// One thing is changed, as in declare_reference.js: the tables of words have nothing behind them, where they were plain objects, so a
// word that's the name of something every object has, as `constructor` is, isn't found in them.

const layoutReference = (() => {
    const INLINE_TEXT_TAGS = new Set(['span', 'a', 'b', 'i', 'em', 'strong', 'small', 'code', 'u', 's', 'sub', 'sup', 'mark', 'abbr', 'cite', 'kbd', 'time', 'label', 'samp', 'var', 'dfn', 'ins', 'del', 'data', 'output', 'bdi', 'bdo']);
    const INLINE_BOXES = new Set(['button', 'input', 'select', 'textarea', 'img', 'svg', 'video', 'canvas', 'iframe', 'progress', 'meter']);
    const COLOR_KEYS = ['background', 'borderColor', 'borderTopColor', 'borderRightColor', 'borderBottomColor', 'borderLeftColor', 'shadowColor', 'outlineColor', 'fill', 'stroke', 'hover', 'pressed'];
    const fontFiles = new Map();

    const ALIGN = Object.assign(Object.create(null), {
        'flex-start': 'start', start: 'start', 'self-start': 'start', left: 'start', top: 'start', baseline: 'start',
        center: 'center', 'flex-end': 'end', end: 'end', 'self-end': 'end', right: 'end', bottom: 'end', stretch: 'stretch', normal: 'stretch',
    });
    const JUSTIFY = Object.assign(Object.create(null), {
        'flex-start': 'start', start: 'start', left: 'start', normal: 'start', center: 'center', 'flex-end': 'end', end: 'end', right: 'end',
        'space-between': 'space_between', 'space-around': 'space_around', 'space-evenly': 'space_evenly',
    });

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
            // Spaces at the end of a line are part of it where they're kept as written.
            if (props.__whiteSpace === 'pre' || props.__whiteSpace === 'pre-wrap' || props.__whiteSpace === 'break-spaces') {
                props.keepSpaces = true;
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
        // The first of the family's names that CSS gave files for, past any that only a Mac has, with the names after it.
        let files;
        let rest = '';
        if (typeof family === 'string') {
            const names = family.split(',');
            for (let at = 0; at < names.length; at += 1) {
                if (fontFiles.has(names[at])) {
                    files = fontFiles.get(names[at]);
                    rest = names.slice(at + 1).join(',');
                    break;
                }
                if (names[at].toLowerCase() !== '-apple-system' && names[at].toLowerCase() !== 'blinkmacsystemfont') {
                    break;
                }
            }
        }
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
            if (best.metrics !== undefined) {
                out.fontMetrics = String(best.metrics[0]) + ' ' + String(best.metrics[1]);
            } else {
                delete out.fontMetrics;
            }
            if (best.local !== undefined) {
                // The system's font it named, in its own weight, or what's after it in the family if that isn't there.
                out.fontFamily = rest === '' ? best.path : best.path + ',' + rest;
                out.fontWeight = best.local.weight === 700 ? 'bold' : best.local.weight === 400 ? 'normal' : String(best.local.weight);
                if (best.local.italic) {
                    out.fontStyle = 'italic';
                }
            } else {
                out.fontFamily = best.path;
                // It's only heavier than the file is if there's nothing near that, which CSS would make up, and this makes bold.
                out.fontWeight = wanted >= 600 && best.weight < 600 ? 'bold' : 'normal';
                if (best.italic) {
                    delete out.fontStyle;
                }
            }
            if (best.adjust !== 1 && typeof out.fontSize === 'number') {
                out.fontSize = out.fontSize * best.adjust;
            }
        } else {
            if (typeof family === 'string') {
                delete out.fontMetrics;
            }
            // A system font's own weights, which rsui looks for by name, as `Segoe UI Semibold` is.
            out.fontWeight = wanted === 700 ? 'bold' : wanted === 400 ? 'normal' : String(wanted);
        }
        return out;
    }

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

    function settle(look) {
        motionProps(look);
        if (look.__fontScale !== undefined || declaring.em !== '') {
            // Anything that's still a share of the size of its text has nothing better than the usual size to go by.
            resolveRelative(look, 16);
        }
        const flow = look.__flow;
        const content = look.__alignContent;
        delete look.__flow;
        delete look.__alignContent;
        if (look.__sticky === true) {
            // Where it sticks is from the edges of what it scrolls in, rather than where it's moved to.
            if (typeof look.top === 'number') {
                look.stickyTop = look.top;
            }
            if (typeof look.bottom === 'number') {
                look.stickyBottom = look.bottom;
            }
            delete look.top;
            delete look.bottom;
            delete look.left;
            delete look.right;
        }
        delete look.__sticky;
        if (look.__visibility !== undefined) {
            if (look.__visibility === 'hidden') {
                look.opacity = 0;
                look.inert = true;
            }
            delete look.__visibility;
        }
        const room = typeof look.__nativeBorder === 'number' && look.__ownBorder !== true ? look.__nativeBorder : 0;
        delete look.__nativeBorder;
        delete look.__ownBorder;
        for (const side of ['Top', 'Right', 'Bottom', 'Left']) {
            const thick = look['border' + side + 'Width'] ?? look.borderWidth;
            if (typeof thick === 'number' && thick > 0) {
                look['padding' + side] = (look['padding' + side] ?? 0) + thick + room;
            }
        }
        // A size is what's inside the padding and border, as CSS has it, unless it says it's the whole thing. Here it's always the whole.
        if (look.__boxSizing !== 'border-box') {
            const across = (look.paddingLeft ?? 0) + (look.paddingRight ?? 0);
            const down = (look.paddingTop ?? 0) + (look.paddingBottom ?? 0);
            for (const [key, more] of [['width', across], ['minWidth', across], ['maxWidth', across], ['height', down], ['minHeight', down], ['maxHeight', down]]) {
                look[key] = withMore(look[key], more);
            }
        }
        delete look.__boxSizing;
        if (look.__fillWidth === true) {
            // All of what it's in, which is the whole of it, padding and all.
            look.width = '100%';
        }
        delete look.__fillWidth;
        // Things make room for each other, in CSS, unless they say they don't. It stops at what's in them there, and doesn't here, so this is
        // only for what's a share of what it's in, which is what relies on it.
        if (look.shrink === undefined) {
            look.shrink = 1;
            look.minAuto = true;
        }
        // What says how small it can be is that small along a flex box, rather than as small as what's in it.
        if (look.minAuto === true) {
            if (look.minWidth !== undefined) {
                look.minWidthSaid = true;
            }
            if (look.minHeight !== undefined) {
                look.minHeightSaid = true;
            }
        }
        // A block's children are one under another, and their margins go together; a flex box's or a grid's don't.
        if (look.direction === undefined && look.display === undefined && look.__flex !== true) {
            look.collapseMargins = true;
            if (look.__ownMargins === true) {
                look.keepMargins = true;
            }
            // Nor is there any space between them.
            delete look.gap;
        } else if (look.display !== 'grid') {
            // A flex box's gap is the one that goes the way it does.
            const along = look.direction === 'row' ? look.columnGap : look.rowGap;
            if (along !== undefined) {
                look.gap = along;
            }
        }
        delete look.__ownMargins;
        // A flex box's row starts on the right in right-to-left text, turned around as `row-reverse` is.
        if (look.__flex === true && look.direction === 'row' && look.textDirection === 'rtl') {
            look.reverse = !(look.reverse === true);
        }
        delete look.__flex;
        delete look.__block;
        delete look.__listStyle;
        if (look.__autoLeft === true && look.__autoRight === true) {
            // With nothing to say what it's in, `margin: 0 auto` is the middle, which is what it nearly always is.
            look.alignSelf ??= 'center';
        }
        delete look.__autoTop;
        delete look.__autoRight;
        delete look.__autoBottom;
        delete look.__autoLeft;
        if (look.__fixed === true) {
            // Where it is in the window rather than in what it's in, which is on top of everything else, as rsui has it.
            const { top, right, bottom, left } = look;
            if (top === 0 && left === 0 && ((right === 0 && bottom === 0) || (look.width === '100%' && look.height === '100%'))) {
                look.overlay = 'fill';
            } else {
                // Placed in the window by its own sides, as CSS has what's fixed (rsui-layout's `Placement::Inset`). A side says where
                // it is with a length or a share; `auto` says nothing, and saying nothing along a way leaves it where it would be.
                const said = (side) => typeof side === 'number' || (typeof side === 'string' && side.trim() !== 'auto');
                if ((said(left) || said(right)) && (said(top) || said(bottom))) {
                    look.overlay = 'inset';
                }
            }
            if (look.overlay !== undefined) {
                delete look.position;
                if (look.overlay === 'fill') {
                    for (const key of ['top', 'right', 'bottom', 'left', 'width', 'height']) {
                        delete look[key];
                    }
                }
            }
        }
        delete look.__fixed;
        if (typeof look.__aspect === 'number') {
            // One size and the shape it's said to be gives the other, as long as that one isn't said too.
            if (typeof look.width === 'number' && look.height === undefined) {
                look.height = look.width / look.__aspect;
            } else if (typeof look.height === 'number' && look.width === undefined) {
                look.width = look.height * look.__aspect;
            } else if (look.height === undefined || look.width === undefined) {
                // A size that's only known once it's laid out, as a block's width is, gives the other then.
                look.aspect = look.__aspect;
            }
        }
        delete look.__aspect;
        if (look.display === 'grid' && flow !== undefined && flow.startsWith('column')) {
            // Each thing in a column of its own, which a grid does itself.
            look.gridFlow = 'column';
        }
        if (look.display === 'grid' && look.gridFlow === undefined && look.columns === undefined && look.rows === undefined) {
            delete look.display;
            look.direction = 'column';
            if (look.rowGap !== undefined) {
                look.gap = look.rowGap;
            }
            // Its rows share what room is left, and what's in each is in the middle of its row, which comes to the same as room around
            // each, as long as nothing says where the rows go.
            const middled = look.align === 'center' && content === undefined;
            // Its one column is only as wide as what's in it when `justify-content` says where it goes, and that's where what's
            // in it is.
            const placed = { start: 'start', center: 'center', end: 'end', space_between: 'start', space_around: 'center', space_evenly: 'center' }[look.justify];
            if (look.justifyItems !== undefined) {
                look.align = look.justifyItems;
            } else if (placed !== undefined) {
                look.align = placed;
            } else if (look.align !== undefined) {
                delete look.align;
            }
            // `align-content` is along its one column.
            if (content !== undefined) {
                look.justify = JUSTIFY[content] ?? 'start';
            } else if (middled) {
                look.justify = 'space_around';
            } else {
                delete look.justify;
            }
            delete look.justifyItems;
        } else if (look.display === 'grid' && content !== undefined) {
            look.alignContent = content;
        } else if (look.display !== 'grid' && look.flexWrap === true && content !== undefined) {
            // Where the lines of a flex box that wraps go.
            look.alignContent = content;
        } else if (content !== undefined && look.display !== 'grid' && look.direction === 'row' && look.align === undefined) {
            // Only for things that wrap, which these don't, unless it's the only thing that's saying where they go.
            look.align = ALIGN[content] ?? 'stretch';
        }
        return look;
    }

    function autoMargins(look, flow) {
        const [top, right, bottom, left] = ['Top', 'Right', 'Bottom', 'Left'].map((side) => look['__auto' + side] === true);
        for (const side of ['Top', 'Right', 'Bottom', 'Left']) {
            delete look['__auto' + side];
        }
        if (look.position === 'absolute' || look.position === 'fixed') {
            for (const [name, on] of [['marginTopAuto', top], ['marginRightAuto', right], ['marginBottomAuto', bottom], ['marginLeftAuto', left]]) {
                if (on) {
                    look[name] = true;
                }
            }
            return;
        }
        const across = flow === 'row' ? [top, bottom] : [left, right];
        const along = flow === 'row' ? [['marginLeftAuto', left], ['marginRightAuto', right]] : [['marginTopAuto', top], ['marginBottomAuto', bottom]];
        if (across[0] || across[1]) {
            look.alignSelf ??= across[0] && across[1] ? 'center' : across[0] ? 'end' : 'start';
            if (flow === 'block' && look.width === undefined && look.fitWidth !== true) {
                // A block is as wide as what it's in, as far as its max-width lets it, and auto margins put it in the middle of that.
                look.__fillWidth = true;
            } else if (flow !== 'row') {
                // As wide as it is, rather than as wide as what it's in.
                look.fitWidth ??= look.width === undefined;
            }
        }
        if (flow !== 'block') {
            for (const [name, on] of along) {
                if (on) {
                    look[name] = true;
                }
            }
        }
    }

    function fitInFlow(look, tag, flow, around = undefined) {
        // What goes on a line is as wide as it is, as a span with a border or a background shows.
        const inline = look.__inline ?? (INLINE_BOXES.has(tag) || (INLINE_TEXT_TAGS.has(tag) && look.__block !== true && look.position !== 'absolute'));
        delete look.__inline;
        if (inline && flow === 'block' && look.width === undefined && look.display === undefined) {
            look.fitWidth = true;
            // Where it is on its line, which is as the text of what it's in is aligned, `around`.
            // A block where text goes from right to left is laid out from the left and turned around: its start is at the right.
            const backwards = look.textDirection === 'rtl';
            look.alignSelf ??= around === 'center' ? 'center' : around === 'end' || around === (backwards ? 'left' : 'right') ? 'end' : 'start';
        }
        return inline;
    }

    function withCurrentColor(look, current) {
        for (const key of COLOR_KEYS) {
            if (look[key] === 'currentcolor') {
                if (current === undefined) {
                    delete look[key];
                } else {
                    look[key] = current;
                }
            }
        }
        return look;
    }

    function withMore(size, more) {
        if (more === 0 || size === undefined) {
            return size;
        }
        if (typeof size === 'number') {
            return size + more;
        }
        const both = /^(-?[0-9.]+)% \+ (-?[0-9.]+)px$/.exec(size);
        if (both !== null) {
            return both[1] + '% + ' + (Number(both[2]) + more) + 'px';
        }
        if (typeof size === 'string' && size.endsWith('%')) {
            return parseFloat(size) + '% + ' + more + 'px';
        }
        return size;
    }

    function sitsOnLine(look) {
        if (look.__block !== true && look.__flex !== true && look.display === undefined && look.direction === undefined && look.position !== 'absolute') {
            look.onLine = true;
        }
    }

    function alongLine(textAlign, backwards) {
        return textAlign === 'center' ? 'center'
            : textAlign === 'right' || textAlign === (backwards ? 'start' : 'end') || (backwards && textAlign !== 'left' && textAlign !== 'end') ? 'end' : undefined;
    }

    /// What's in something laid out from the left and turned around where text goes from right to left has its own margins
    /// swapped, so that each is on the side it says once it's turned around.
    function mirroredMargins(look) {
        if (look.position === 'absolute' || look.position === 'fixed' || look.__fixed === true) {
            return;
        }
        for (const [one, other] of [['marginLeft', 'marginRight'], ['__autoLeft', '__autoRight'], ['marginLeftShare', 'marginRightShare']]) {
            const [a, b] = [look[one], look[other]];
            if (a === undefined && b === undefined) {
                continue;
            }
            if (b === undefined) {
                delete look[one];
            } else {
                look[one] = b;
            }
            if (a === undefined) {
                delete look[other];
            } else {
                look[other] = a;
            }
        }
    }

    /// What `Styled` did to the look of an element made for a tag, from just after what's in it had been worked out to the props it
    /// was made with, as `makeHost` gave them to `finalFont`. Only `along` and `blockChild` (whether anything right in it is a
    /// block) are about what's in it. `withLegend` went where a fieldset's `settle` is; it isn't here.
    function place(look, tag, givenFlow, along, blockChild, fontAbove, host, shapeParts) {
        const onLine = givenFlow === 'line';
        const flow = onLine ? 'row' : givenFlow;
        delete look.__hidden;
        delete look.__scroll;
        delete look.__case;
        if (!onLine && flow !== 'line' && look.textDirection === 'rtl') {
            mirroredMargins(look);
        }
        if (look.display === undefined && look.direction === undefined && along) {
            look.direction = 'row';
            look.align ??= 'baseline';
            look.flexWrap ??= !(look.__whiteSpace === 'nowrap' || look.__whiteSpace === 'pre');
            look.lineStrut = true;
            look.justify ??= alongLine(look.textAlign, look.textDirection === 'rtl');
        }
        if (tag === 'button' && look.display === undefined && look.direction === undefined) {
            look.justify ??= 'center';
        }
        if (tag === 'svg' || tag === 'img') {
            sitsOnLine(look);
        }
        const inside = look.display === 'grid' || look.__flex === true || look.direction !== undefined ? (look.direction === 'row' ? (look.lineStrut === true ? 'line' : 'row') : 'column') : 'block';
        if (INLINE_TEXT_TAGS.has(tag) && look.__inline === undefined && blockChild) {
            look.__inline = false;
        }
        const inline = fitInFlow(look, tag, flow, fontAbove?.textAlign);
        if (look.__lineAlign !== undefined) {
            if (flow === 'row') {
                look.alignSelf ??= look.__lineAlign;
            }
            delete look.__lineAlign;
        }
        if ((onLine || flow === 'block') && INLINE_TEXT_TAGS.has(tag) && inline && look.__lineBox !== true && look.__flex !== true && look.display !== 'grid' && look.position !== 'absolute') {
            for (const side of ['Top', 'Bottom']) {
                const more = (look['padding' + side] ?? 0) + (look['border' + side + 'Width'] ?? look.borderWidth ?? 0);
                if (typeof more === 'number' && more > 0) {
                    look['margin' + side] = (typeof look['margin' + side] === 'number' ? look['margin' + side] : 0) - more;
                }
            }
        }
        delete look.__lineBox;
        if (onLine && look.shrink === undefined && typeof look.width === 'number') {
            look.shrink = 0;
        }
        // A block's children don't shrink to fit it, as flex items do, whatever height they say: `h-full` in a grid's cell that's
        // stretched is as tall as the cell, with what's after it going past its end (shadcn/ui's Command in its card).
        if (flow === 'block' && look.shrink === undefined && (typeof look.height === 'number' || typeof look.height === 'string')) {
            look.shrink = 0;
            look.minAuto = true;
        }
        if (flow === 'block') {
            delete look.grow;
            delete look.basis;
        }
        autoMargins(look, flow);
        const current = look.color ?? fontAbove?.color;
        withCurrentColor(look, current);
        const out = { ...settle(look), ...host };
        delete out.__lineScale;
        if (tag === 'svg') {
            out.__parts = shapeParts;
        }
        for (const [key, said] of [['width', '__ownWidth'], ['height', '__ownHeight']]) {
            if (out[said] !== undefined) {
                if (look[key] === undefined) {
                    out[key] = out[said];
                }
                delete out[said];
            }
        }
        return { inside, props: finalFont(out) };
    }

    const FONT_PROPS = ['fontSize', 'fontWeight', 'fontFamily', 'fontStyle', 'textAlign', 'lineHeight', '__lineScale', 'color', '__listStyle', 'keepWords', '__whiteSpace', '__sizeSaid', 'letterSpacing', 'wordSpacing', 'textShadow', 'textIndent', 'textDirection', 'tabularNums', '__case', 'textWrap'];

    function onlyMonospace(family) {
        return typeof family === 'string' && family.replace(/["']/g, '').trim().toLowerCase() === 'monospace';
    }

    function monospaceSize(look, fontAbove) {
        const was = onlyMonospace(fontAbove?.fontFamily);
        const now = onlyMonospace(look.fontFamily);
        if (was !== now) {
            const above = typeof fontAbove?.fontSize === 'number' ? fontAbove.fontSize : 16;
            look.fontSize = above * (now ? 13 / 16 : 16 / 13);
        }
    }

    function listInList(look, tag, above) {
        const em = typeof look.fontSize === 'number' ? look.fontSize : 16;
        for (const side of ['marginTop', 'marginBottom']) {
            if (look[side] === em) {
                look[side] = 0;
            }
        }
        if (tag === 'ul' && look.__listStyle === 'disc') {
            look.__listStyle = above === 'circle' || above === 'square' ? 'square' : 'circle';
        }
    }

    /// What `Styled` did to how text is written in what it made, before what's in it: from just after `passedDown` to `fontKey`.
    function ownFont(look, fontAbove, tag, isHost) {
        const ownSize = look.fontSize !== undefined || look.__fontScale !== undefined;
        if (isHost) {
            if (ownSize && typeof look.__fontScale !== 'number') {
                look.__sizeSaid = true;
            }
            // With nothing around it saying, rsui's own text size, which is what it's drawn at (`NATIVE_TEXT_SIZE` in layout.rs).
            resolveRelative(look, typeof fontAbove?.fontSize === 'number' ? fontAbove.fontSize : 14);
        }
        for (const name of FONT_PROPS) {
            if (look[name] === undefined && fontAbove !== null && fontAbove[name] !== undefined) {
                look[name] = fontAbove[name];
            }
        }
        // `bolder` and `lighter` are a step from the weight around it, as CSS has them.
        if (look.fontWeight === 'bolder' || look.fontWeight === 'lighter') {
            const around = fontAbove?.fontWeight;
            const from = around === undefined || around === null || around === 'normal' ? 400 : around === 'bold' ? 700 : Number(around);
            look.fontWeight = look.fontWeight === 'bolder'
                ? (from < 350 ? 400 : from < 550 ? 700 : 900)
                : (from < 550 ? 100 : from < 750 ? 400 : 700);
        }
        if (isHost && !ownSize && look.__sizeSaid !== true) {
            monospaceSize(look, fontAbove);
        }
        if (isHost && (tag === 'ul' || tag === 'ol') && fontAbove?.__listStyle !== undefined) {
            listInList(look, tag, fontAbove.__listStyle);
        }
        if (typeof look.__lineScale === 'number' && typeof look.fontSize === 'number') {
            look.lineHeight = look.__lineScale * look.fontSize;
        }
        const fontKey = FONT_PROPS.map((name) => look[name]).join('|');
        return { look, fontKey };
    }

    return { place, ownFont, settle, finalFont, resolveRelative, fontFiles };
})();

// What rsui/web asked in JavaScript about what's in an element (web.js at f323b46), which src/layout.rs has to answer exactly the same:
// what goes along a line, what's a block, what floats, what's placed by itself, and the order of what's in a flex box. The test gives
// it what it looked things up with: React's `isValidElement` and `Children`, `tagLook`, `looksOfClasses`, `classLook`, the
// contexts that hand something down, `SVG_TAGS`, `rsui`, `Select` and `Option`.

const childrenReference = (({ NestedRules, InheritedFont, FlowContext, NthContext }) => {
    const TEXT_TAGS = new Set(['span', 'p', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'label', 'small', 'strong', 'em', 'b', 'i', 'pre', 'code', 'legend', 'blockquote', 'sup', 'sub', 'u', 's', 'mark', 'abbr', 'cite', 'kbd', 'time', 'del', 'ins', 'samp', 'var', 'dfn', 'q', 'data', 'output', 'bdi', 'bdo']);

    const INLINE_BOXES = new Set(['button', 'input', 'select', 'textarea', 'img', 'svg', 'video', 'canvas', 'iframe', 'progress', 'meter']);

    const INLINE_TAGS = new Set(['a', 'abbr', 'b', 'button', 'cite', 'code', 'em', 'i', 'img', 'input', 'kbd', 'label', 'mark', 'select', 'small', 'span', 'strong', 'sub', 'sup', 'svg', 'textarea', 'time', 'u', 's']);

    const INLINE_TEXT_TAGS = new Set(['span', 'a', 'b', 'i', 'em', 'strong', 'small', 'code', 'u', 's', 'sub', 'sup', 'mark', 'abbr', 'cite', 'kbd', 'time', 'label', 'samp', 'var', 'dfn', 'ins', 'del', 'data', 'output', 'bdi', 'bdo']);

    const BLOCK_TAGS = new Set(['div', 'hgroup', 'search', 'section', 'main', 'nav', 'header', 'footer', 'aside', 'article', 'form', 'fieldset', 'figure', 'li', 'td', 'th', 'dd', 'dt', 'details', 'summary', 'address', 'figcaption']);

    const OTHER_BLOCKS = new Set(['p', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'ul', 'ol', 'table', 'pre', 'blockquote', 'hr']);

    const isInline = (child) =>
        typeof child === 'string' ||
        typeof child === 'number' ||
        child === null ||
        child === undefined ||
        typeof child === 'boolean' ||
        (Array.isArray(child) && child.every(isInline)) ||
        (isValidElement(child) && !placedApart(child) && (child.type === 'text' || child.type === 'span' || child.type?.__webInline === true
            // A page's inline tag written as itself, as a file with no tag rsui has too writes `<a>`.
            || (typeof child.type === 'string' && INLINE_TEXT_TAGS.has(child.props?.webTag ?? child.type)))) ||
        // What's only handing something down to what's in it is as inline as that is.
        (isValidElement(child) && passesDown(child) && isInline(child.kids ?? child.props?.children));

    function passesDown(child) {
        return child.type === NestedRules.Provider || child.type === InheritedFont.Provider || child.type === FlowContext.Provider || child.type === NthContext.Provider
            || (typeof child.type === 'function' && child.type.context !== undefined && child.type.context !== null);
    }

    function placedApart(child) {
        const position = child.props?.position ?? child.props?.style?.position ?? child.type?.__webLook?.()?.position;
        return position === 'absolute' || position === 'fixed';
    }

    function hostType(tag, props, children) {
        const flowing = BLOCK_TAGS.has(tag) && children !== undefined && Children.toArray(children).some((child) => typeof child === 'string' || typeof child === 'number');
        if (TEXT_TAGS.has(tag) || flowing) {
            const inline = children === undefined || Children.toArray(children).every(isInline);
            // If it says how what's in it goes, as a flex box or a grid does, what's in it isn't one piece of text.
            const arranged = props.display !== undefined || props.direction !== undefined;
            return inline && !arranged ? 'text' : 'group';
        }
        if (SVG_TAGS[tag] !== undefined) {
            return SVG_TAGS[tag];
        }
        switch (tag) {
            case 'a':
                // An `<a>` with no `href` is drawn as a `<span>` would be.
                return typeof props.href === 'string' ? 'link' : hostType('span', props, children);
            case 'img':
                return 'image';
            case 'button':
                return 'button';
            case 'li':
                return 'item';
            case 'input':
            case 'textarea':
                if (props.type === 'checkbox' || props.type === 'radio') {
                    return rsui.Checkbox;
                }
                if (tag === 'input' && (props.type === 'submit' || props.type === 'reset' || props.type === 'button')) {
                    return 'button';
                }
                return props.type === 'range' ? rsui.Slider : 'input';
            case 'select':
                return Select;
            case 'option':
                return Option;
            default:
                return 'group';
        }
    }

    function tagOf(element) {
        const type = element.type;
        if (typeof type === 'function') {
            return type.__webTag ?? null;
        }
        return element.props?.webTag ?? (typeof type === 'string' ? type : null);
    }

    function elementsIn(children, out = []) {
        for (const one of Children.toArray(children)) {
            if (!isValidElement(one)) {
                continue;
            }
            if (one.type === globalThis.__Fragment) {
                elementsIn(one.kids ?? one.props?.children, out);
            } else {
                out.push(one);
            }
        }
        return out;
    }

    function goesAlong(children) {
        const elements = elementsIn(children);
        // What floats is placed as a block's is, beside what comes after it, rather than along a line.
        if (elements.length < 2 || elements.some((one) => floatOf(one) !== undefined)) {
            return false;
        }
        return elements.every((one) => {
            const said = saidFlow(one);
            if (said === 'block') {
                return false;
            }
            // What CSS makes go along a line, as `display: inline-block` does, goes along one, whatever kind of element it is.
            if (said === 'inline') {
                return true;
            }
            const tag = tagOf(one);
            // A component that's been found to make something that goes along a line goes along one: see `learnFlow`.
            return (tag !== null && INLINE_TAGS.has(tag)) || one.type === 'text' || one.type === 'span' || one.type === 'link' || one.type === 'image' || one.type === 'button' || one.type === 'input'
                || one.type?.__webInline === true;
        });
    }

    function textWithBoxes(children) {
        const all = Children.toArray(children);
        const text = all.some((child) => typeof child === 'string' || typeof child === 'number');
        const boxes = all.some((child) => isValidElement(child) && INLINE_BOXES.has(tagOf(child)));
        return text && boxes && all.every((child) => !isValidElement(child) || (saidFlow(child) !== 'block' && (isInline(child) || INLINE_BOXES.has(tagOf(child)))));
    }

    function goesOnLine(child) {
        if (typeof child === 'string' || typeof child === 'number') {
            return true;
        }
        if (!isValidElement(child)) {
            return false;
        }
        const tag = tagOf(child);
        // Pictures are left where they are, as they're so often placed by themselves, which a line around them would get in the way of.
        if (tag === 'svg' || tag === 'img') {
            return false;
        }
        const said = saidFlow(child);
        if (said !== undefined) {
            return said === 'inline';
        }
        // Something on a line with a block in it is broken around that block, as CSS has it, so it's a block itself.
        if (INLINE_TEXT_TAGS.has(tag) && holdsBlock(child)) {
            return false;
        }
        return isInline(child) || INLINE_BOXES.has(tag) || INLINE_TEXT_TAGS.has(tag);
    }

    function holdsBlock(element) {
        return Children.toArray(element.props?.children).some(
            (child) => isValidElement(child) && (saidFlow(child) === 'block' || BLOCK_TAGS.has(tagOf(child)) || OTHER_BLOCKS.has(tagOf(child))),
        );
    }

    function saidFlow(child) {
        const props = child.props ?? {};
        const own = props.style?.display;
        if (typeof own === 'string') {
            return flowOfDisplay(own);
        }
        const tag = tagOf(child);
        const look = { ...(tag === null ? undefined : tagLook(tag)), ...child.type?.__webLook?.(), ...looksOfClasses(props.className) };
        if (look.__inline === true) {
            return 'inline';
        }
        if (look.__block === true || look.__flex === true || look.display === 'grid') {
            return 'block';
        }
        return undefined;
    }

    /// Whether CSS makes something a box of its own on a line, as `display: inline-block` and `inline-flex` do.
    function saidLineBox(child) {
        const props = child.props ?? {};
        const own = props.style?.display;
        if (typeof own === 'string') {
            return own === 'inline-block' || own === 'inline-flex' || own === 'inline-grid';
        }
        const tag = tagOf(child);
        const look = { ...(tag === null ? undefined : tagLook(tag)), ...child.type?.__webLook?.(), ...looksOfClasses(props.className) };
        return look.__lineBox === true || (look.__inline === true && look.__flex === true);
    }

    function flowOfDisplay(display) {
        if (display.startsWith('inline')) {
            return 'inline';
        }
        if (display === 'none' || display === 'contents') {
            return undefined;
        }
        return 'block';
    }

    function floatOf(child) {
        if (!isValidElement(child)) {
            return undefined;
        }
        const props = child.props ?? {};
        const own = props.style?.float;
        if (own === 'left' || own === 'right' || own === 'inline-start' || own === 'inline-end') {
            return own;
        }
        const classes = looksOfClasses(props.className);
        if ('__float' in classes) {
            return classes.__float;
        }
        const tag = tagOf(child);
        return tag === null ? undefined : tagLook(tag)?.__float;
    }

    function clearsFloats(child) {
        if (!isValidElement(child)) {
            return false;
        }
        const props = child.props ?? {};
        if (props.style?.clear !== undefined) {
            return props.style.clear !== 'none';
        }
        return looksOfClasses(props.className).__clear === true;
    }

    function orderOf(child) {
        if (!isValidElement(child)) {
            return 0;
        }
        const props = child.props ?? {};
        const own = props.style?.order ?? props.order;
        if (own !== undefined && !Number.isNaN(Number(own))) {
            return Number(own);
        }
        const look = child.type?.__webLook?.();
        if (typeof look?.__order === 'number') {
            return look.__order;
        }
        let order = 0;
        for (const name of typeof props.className === 'string' ? props.className.split(/\s+/) : []) {
            const found = name === '' ? undefined : classLook(name)?.__order;
            if (typeof found === 'number') {
                order = found;
            }
        }
        return order;
    }

    /// Whether what's in an element made for `tag` went along a line, as `Styled` worked it out.
    function along(children, tag) {
        const onlyText = Children.toArray(children).some((child) => typeof child === 'string' || typeof child === 'number')
            && elementsIn(children).every((one) => INLINE_TEXT_TAGS.has(tagOf(one)) && saidFlow(one) !== 'inline');
        return (BLOCK_TAGS.has(tag) && !onlyText && (goesAlong(children) || textWithBoxes(children))) || (INLINE_TEXT_TAGS.has(tag) && textWithBoxes(children));
    }

    /// Which of what's in something `intoLines` put on lines, or nothing if it left it as it was.
    function lines(children, always) {
        const all = Children.toArray(children);
        const apart = all.map((child) => isValidElement(child) && placedApart(child));
        const inline = all.map((child, at) => (apart[at] ? null : goesOnLine(child)));
        const picture = all.map((child, at) => inline[at] === false && isValidElement(child) && (tagOf(child) === 'img' || tagOf(child) === 'svg') && saidFlow(child) === undefined);
        const besideLine = (at, step) => {
            for (let next = at + step; next >= 0 && next < all.length; next += step) {
                if (apart[next]) {
                    continue;
                }
                return inline[next] === true || picture[next];
            }
            return false;
        };
        const joined = picture.map((one, at) => one && (besideLine(at, -1) || besideLine(at, 1)));
        joined.forEach((one, at) => {
            if (one) {
                inline[at] = true;
            }
        });
        if (!inline.includes(true) || (!inline.includes(false) && !always)) {
            return undefined;
        }
        return inline;
    }

    /// The `order` of each of what's in a flex box, as `inOrder` found it, or nothing when they were all 0.
    function orders(children) {
        const found = Children.toArray(children).map(orderOf);
        return found.every((order) => order === 0) ? undefined : found;
    }

    return {
        along,
        lines,
        orders,
        hostType,
        holdsBlock: (children) => holdsBlock({ props: { children } }),
        lineBoxes: (children) => Children.toArray(children).some((child) => isValidElement(child) && saidLineBox(child)),
        blockChild: (children) => Children.toArray(children).some((child) => isValidElement(child) && saidFlow(child) === 'block'),
        anyFloat: (children) => Children.toArray(children).some((child) => floatOf(child) !== undefined),
        everyInline: (children) => Children.toArray(children).every(isInline),
        saidFlow,
        floatOf,
        clearsFloats,
        goesOnLine,
        isInline,
    };
});
