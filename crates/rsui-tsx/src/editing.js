// Editing a page's elements, as `contenteditable` has it: the caret and what's selected, drawn; keys, typing and ways of typing
// turned into `beforeinput` and `input` with the `inputType` a browser gives, and what a browser does by itself when nothing
// stops it; the pointer selecting; the clipboard; undo; and `document.execCommand()`. It's loaded the first time a page has an
// element it can edit, so nothing else pays for it. What's edited is the DOM `dom.js` keeps.

'use strict';

const rsui = require('rsui');
const React = require('react');
const { useState, useLayoutEffect, useRef } = React;

let web = null;
let dom = null;
/// The web's event classes.
const events = require('rsui/dom-events');

const ELEMENT = 1;
const TEXT = 3;

const BLOCKS = new Set(['p', 'div', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'li', 'ul', 'ol', 'blockquote', 'pre', 'section', 'article', 'header', 'footer', 'td', 'th', 'tr', 'table', 'tbody', 'thead', 'figure', 'figcaption', 'dd', 'dt', 'dl', 'hr', 'main', 'nav', 'aside', 'address']);

// Which element can be edited, and which has the keyboard.

/// What a node's own `contenteditable` says: `true`, `false`, or nothing.
function editableSaid(node) {
    if (node === null || node === undefined || node.nodeType !== ELEMENT) {
        return null;
    }
    const said = dom.isOwn(node) ? node.getAttribute('contenteditable') : node.__live?.attributes?.contentEditable ?? node.__live?.given?.contentEditable;
    if (said === undefined || said === null) {
        return null;
    }
    const text = String(said).toLowerCase();
    if (text === 'false') {
        return false;
    }
    return text === '' || text === 'true' || text === 'plaintext-only';
}

/// The element a node is edited in, which is the outermost one of those around it that can be edited, or `null`.
function editingHost(node) {
    let host = null;
    for (let at = node; at !== null && at !== undefined; at = dom.parentOf(at)) {
        const said = editableSaid(at);
        if (said === false) {
            break;
        }
        if (said === true) {
            host = at;
        }
    }
    if (host === null && designMode) {
        return web.document.body;
    }
    return host;
}

let designMode = false;
/// What can be edited that has the keyboard, which is the editing host of what the page says has it.
let active = null;
/// Whether what can be edited that's given the keyboard has the caret put at its start, if what's selected isn't in it already.
let placing = true;

/// Gives the keyboard to what can be edited, or takes it from it, as the page's focus does it: see `moveFocus` in `web.js`, which
/// tells this with `focusMoved`.
function focusHost(host, place = true) {
    if (host === null) {
        const now = web.document.activeElement;
        if (active !== null && (now === active || active.contains(now))) {
            web.moveFocus(null);
        }
        return;
    }
    placing = place;
    try {
        web.moveFocus(host);
    } finally {
        placing = true;
    }
}

/// What has the keyboard has changed, which the page says once what had it has heard `blur`, and before what has it hears
/// `focus`, as a browser has the caret in place by then.
function focusMoved(next) {
    const host = next === null ? null : editingHost(next);
    if (active === host) {
        return;
    }
    active = host;
    if (host !== null && placing && (dom.selectionRange === null || !host.contains(dom.selectionRange.startContainer))) {
        const [node, offset] = firstPlace(host);
        dom.selection.collapse(node, offset);
    }
    blinkFrom();
    redrawCarets();
}

// Events, as a page has them.

/// An `InputEvent`, with what one has.
function inputEvent(type, init) {
    const { targetRanges: _ranges, ...rest } = init;
    const made = dom.makeEvent(type, { bubbles: true, composed: true, ...rest }, events.InputEvent);
    const ranges = init.targetRanges ?? [];
    made.getTargetRanges = () => ranges.map((one) => new dom.StaticRange(one));
    made.isTrusted = true;
    return made;
}

/// Says what's about to change, and does it unless something says not to, as a browser does with `beforeinput` and `input`.
function edit(inputType, data, perform, { cancelable = true, dataTransfer = null, ranges = null } = {}) {
    const host = active;
    if (host === null) {
        return false;
    }
    const range = dom.selectionRange;
    const targetRanges = ranges ?? (range === null ? [] : [range]);
    const before = inputEvent('beforeinput', { inputType, data, dataTransfer, isComposing: composing !== null, cancelable, targetRanges });
    if (!dom.dispatch(host, before) && cancelable) {
        return false;
    }
    remember(host, inputType);
    perform();
    dom.dispatch(host, inputEvent('input', { inputType, data, dataTransfer, isComposing: composing !== null, cancelable: false }));
    blinkFrom();
    redrawCarets();
    return true;
}

// Positions in the DOM, and what's next to them.

/// Where the first text in something is, or its start.
function firstPlace(node) {
    for (let at = node; ;) {
        const kids = dom.childList(at);
        if (at.nodeType === TEXT) {
            return [at, 0];
        }
        const next = kids.find((one) => one.nodeType === TEXT || one.nodeType === ELEMENT);
        if (next === undefined) {
            return [at, 0];
        }
        if (next.nodeType === ELEMENT && next.localName === 'br') {
            return [at, kids.indexOf(next)];
        }
        at = next;
    }
}

function lastPlace(node) {
    for (let at = node; ;) {
        if (at.nodeType === TEXT) {
            return [at, at.__data.length];
        }
        const kids = dom.childList(at).filter((one) => one.nodeType === TEXT || (one.nodeType === ELEMENT && one.localName !== 'br'));
        const last = kids.at(-1);
        if (last === undefined) {
            const all = dom.childList(at);
            const br = all.findIndex((one) => one.localName === 'br');
            return [at, br >= 0 ? br : all.length];
        }
        at = last;
    }
}

const isBlock = (node) => node?.nodeType === ELEMENT && BLOCKS.has(String(node.localName));

/// The block a place is in, inside its host: what's drawn as one paragraph.
function blockOf(node, host) {
    for (let at = node; at !== null && at !== undefined; at = dom.parentOf(at)) {
        if (at === host || isBlock(at)) {
            return at;
        }
    }
    return host;
}

/// The text a block has, in order, as its pieces: each text, and each `<br>` as a line break, with where each starts.
function piecesOf(block) {
    const out = [];
    let at = 0;
    const visit = (node) => {
        for (const child of dom.childList(node)) {
            if (child.nodeType === TEXT) {
                out.push({ node: child, start: at, length: child.__data.length });
                at += child.__data.length;
            } else if (child.nodeType === ELEMENT) {
                if (child.localName === 'br') {
                    out.push({ node: child, start: at, length: 1, br: true });
                    at += 1;
                } else if (!isBlock(child)) {
                    visit(child);
                }
            }
        }
    };
    visit(block);
    return out;
}

/// How far into its block's text a place is.
function indexInBlock(block, node, offset) {
    const pieces = piecesOf(block);
    if (node.nodeType === TEXT) {
        const piece = pieces.find((one) => one.node === node);
        return piece === undefined ? 0 : piece.start + Math.min(offset, piece.length);
    }
    // Between children: what's before that child.
    const child = dom.childList(node)[offset];
    if (child === undefined) {
        const last = pieces.filter((one) => node.contains(one.node)).at(-1);
        return last === undefined ? 0 : last.start + last.length;
    }
    const first = pieces.find((one) => one.node === child || child.contains?.(one.node));
    if (first !== undefined) {
        return first.start;
    }
    let before = 0;
    for (const piece of pieces) {
        if (dom.comparePoints(piece.node, 0, node, offset) < 0) {
            before = piece.start + piece.length;
        }
    }
    return before;
}

/// The place `index` into a block's text is, in a text node where there's one, preferring the text before at a boundary.
function placeInBlock(block, index, after = false) {
    const pieces = piecesOf(block);
    if (pieces.length === 0) {
        // No text of its own: at the start of the first thing in it that isn't a block, as Chromium puts a place in an empty
        // `<output>` below the paragraphs around it.
        const own = dom.childList(block).find((child) => child.nodeType === ELEMENT && !isBlock(child));
        if (own !== undefined) {
            return [own, 0];
        }
    }
    for (let i = 0; i < pieces.length; i += 1) {
        const piece = pieces[i];
        const end = piece.start + piece.length;
        if (index < end || (index === end && !after && !piece.br) || i === pieces.length - 1) {
            if (piece.br) {
                const parent = dom.parentOf(piece.node);
                return [parent, dom.indexIn(piece.node) + (index > piece.start ? 1 : 0)];
            }
            return [piece.node, Math.max(0, Math.min(piece.length, index - piece.start))];
        }
    }
    return firstPlace(block);
}

/// The blocks inside a host, in order, that have text of their own, kept until what's in the page changes in a way that could
/// change them. Typing into a paragraph of a document of 10,000 changes what's in that paragraph, and React draws the page
/// again, but leaves the blocks as they were, so they aren't all found again for each key.
const blocksKept = new WeakMap();
function blocksIn(host) {
    const tree = web.treeNow();
    const structure = dom.structure;
    const kept = blocksKept.get(host);
    if (kept !== undefined && (kept.tree === tree || pageKidsStay(kept)) && (kept.structure === structure || blocksStay(host, kept))) {
        kept.tree = tree;
        kept.structure = structure;
        return kept.blocks;
    }
    const pages = [];
    const blocks = findBlocks(host, pages);
    blocksKept.set(host, { structure, tree, blocks, pages, listed: null });
    return blocks;
}

/// What `findBlocks` found in what it looked in, `pages` being each page's element among those, which only has children of its
/// own when what's in it is code's: what React draws in one comes and goes with the tree, rather than `dom.structure`.
function findBlocks(host, pages) {
    const out = [];
    const visit = (node) => {
        let own = false;
        const kids = dom.childList(node);
        if (!dom.isOwn(node)) {
            pages.push(node, kids.length - (node.__kids?.length ?? 0));
        }
        for (const child of kids) {
            if (isBlock(child)) {
                visit(child);
            } else {
                own = true;
            }
        }
        if (own || kids.length === 0) {
            out.push(node);
        }
    };
    visit(host);
    return out;
}

/// Whether the tree changing leaves what `findBlocks` looked in as it was: none of the page's elements it looked in had or has
/// anything React drew in it.
function pageKidsStay(kept) {
    for (let at = 0; at < kept.pages.length; at += 2) {
        const node = kept.pages[at];
        if (kept.pages[at + 1] !== 0 || dom.childList(node).length !== (node.__kids?.length ?? 0)) {
            return false;
        }
    }
    return true;
}

/// Whether what's been put in or taken out since the blocks were found leaves them as they were: nothing put in or taken out of
/// the host or a block `findBlocks` looks in was a block, and each of those it changed is still a block found, or not one, as it was.
function blocksStay(host, kept) {
    const changes = dom.changesSince(kept.structure);
    if (changes === null) {
        return false;
    }
    for (let at = 0; at < changes.length; at += 2) {
        const parent = changes[at];
        if (!lookedIn(parent, host)) {
            continue;
        }
        if (isBlock(changes[at + 1])) {
            return false;
        }
        kept.listed ??= new Set(kept.blocks);
        const kids = dom.childList(parent);
        if (kept.listed.has(parent) !== (kids.length === 0 || kids.some((child) => !isBlock(child)))) {
            return false;
        }
    }
    return true;
}

/// Whether `findBlocks` looks at what's in a node: the host's, and each block's in blocks all the way up to it.
function lookedIn(node, host) {
    for (let at = node; at !== null && at !== undefined; at = dom.parentOf(at)) {
        if (at === host) {
            return true;
        }
        if (!isBlock(at)) {
            return false;
        }
    }
    return false;
}

// Geometry, from how the text is laid out: see `textCaret` and `textIndexAt` in `geometry.rs`.

/// The element a block is drawn as, whose key `textCaret` takes.
function drawnKey(node) {
    for (let at = node; at !== null && at !== undefined; at = dom.parentOf(at)) {
        if (at.__domKey !== undefined) {
            return at.__domKey;
        }
    }
    return undefined;
}

/// Where a caret at this place goes on the window, as `[x, y, height]`.
function caretAt(node, offset) {
    const host = editingHost(node) ?? node;
    const block = blockOf(node, host);
    const key = drawnKey(block);
    if (key === undefined) {
        return null;
    }
    return rsui.__host().textCaret(key, indexInBlock(block, node, offset)) ?? null;
}

/// The first of `blocks` whose bottom is at or below `y`, as they go down the page in order, found by halves, so that a document
/// of thousands of paragraphs is asked about a dozen of them.
function firstReaching(blocks, y) {
    let low = 0;
    let high = blocks.length;
    while (low < high) {
        const middle = (low + high) >> 1;
        const box = blockBox(blocks[middle]);
        if (box !== null && box.bottom < y) {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    return low;
}

/// The place nearest a point on the window, in what can be edited there.
function placeAt(x, y, host) {
    const blocks = blocksIn(host);
    let best = null;
    // Near where the point is down the page, and the ones either side, in case the nearest is the one before.
    const from = Math.max(0, firstReaching(blocks, y) - 1);
    for (let at = from; at < blocks.length; at += 1) {
        const block = blocks[at];
        const key = drawnKey(block);
        if (key === undefined) {
            continue;
        }
        const box = blockBox(block);
        if (box === null) {
            continue;
        }
        const away = y < box.top ? box.top - y : y > box.bottom ? y - box.bottom : 0;
        if (best === null || away < best.away) {
            best = { block, away, key };
        }
        if (away === 0 || box.top > y) {
            break;
        }
    }
    // Below everything there's a box for, it's the last one above that has one.
    for (let at = from - 1; best === null && at >= 0; at -= 1) {
        const key = drawnKey(blocks[at]);
        const box = key === undefined ? null : blockBox(blocks[at]);
        if (box !== null && box.top <= y) {
            best = { block: blocks[at], away: y - box.bottom, key };
        }
    }
    if (best === null) {
        return firstPlace(host);
    }
    const index = rsui.__host().textIndexAt(best.key, x, y) ?? 0;
    return placeInBlock(best.block, index);
}

function blockBox(block) {
    const rect = block.getBoundingClientRect?.();
    if (rect === undefined || (rect.width === 0 && rect.height === 0)) {
        return null;
    }
    return rect;
}

// Moving the caret.

function graphemeBefore(text, at) {
    if (at <= 0) return 0;
    const code = text.charCodeAt(at - 1);
    return code >= 0xdc00 && code <= 0xdfff && at >= 2 ? 2 : 1;
}

function graphemeAfter(text, at) {
    if (at >= text.length) return 0;
    const code = text.charCodeAt(at);
    return code >= 0xd800 && code <= 0xdbff && at + 1 < text.length ? 2 : 1;
}

let wordCharacter;
try {
    wordCharacter = /[\p{L}\p{N}_]/u;
} catch {
    wordCharacter = /\w/;
}
const isWordAt = (text, at) => at >= 0 && at < text.length && wordCharacter.test(text[at]);

/// A block's text as one string.
function blockText(block) {
    return piecesOf(block).map((piece) => (piece.br ? '\n' : piece.node.__data)).join('');
}

/// Where a place moves to, one step of `granularity` in `direction` (`forward` or `backward`), inside `host`.
function step(host, node, offset, direction, granularity) {
    const forward = direction === 'forward' || direction === 'right';
    const block = blockOf(node, host);
    const text = blockText(block);
    let index = indexInBlock(block, node, offset);
    const blocks = blocksIn(host);
    const at = blocks.indexOf(block);
    if (granularity === 'line' || granularity === 'lineboundary') {
        return lineBoundary(host, node, offset, forward);
    }
    if (granularity === 'paragraph' || granularity === 'paragraphboundary') {
        return forward ? lastPlace(block) : firstPlace(block);
    }
    if (granularity === 'documentboundary') {
        return forward ? lastPlace(blocks.at(-1) ?? host) : firstPlace(blocks[0] ?? host);
    }
    if (forward && index >= text.length) {
        const next = blocks[at + 1];
        return next === undefined ? [node, offset] : firstPlace(next);
    }
    if (!forward && index <= 0) {
        const previous = blocks[at - 1];
        return previous === undefined ? [node, offset] : lastPlace(previous);
    }
    if (granularity === 'word') {
        if (forward) {
            while (index < text.length && !isWordAt(text, index)) index += 1;
            while (index < text.length && isWordAt(text, index)) index += 1;
        } else {
            while (index > 0 && !isWordAt(text, index - 1)) index -= 1;
            while (index > 0 && isWordAt(text, index - 1)) index -= 1;
        }
    } else {
        index += forward ? graphemeAfter(text, index) : -graphemeBefore(text, index);
    }
    return placeInBlock(block, index, forward);
}

/// The start or end of the line a place is on, from where the layout put it.
function lineBoundary(host, node, offset, forward) {
    const caret = caretAt(node, offset);
    const block = blockOf(node, host);
    if (caret === null) {
        return forward ? lastPlace(block) : firstPlace(block);
    }
    const box = blockBox(block);
    const x = forward ? (box?.right ?? caret[0] + 10000) + 1000 : (box?.left ?? 0) - 1000;
    const key = drawnKey(block);
    const index = rsui.__host().textIndexAt(key, x, caret[1] + caret[2] / 2) ?? 0;
    return placeInBlock(block, index, !forward);
}

/// Where a place moves to a line up or down, as near the same distance across as it can.
let goalX = null;
function vertical(host, node, offset, down) {
    const caret = caretAt(node, offset);
    if (caret === null) {
        return [node, offset];
    }
    goalX ??= caret[0];
    const y = down ? caret[1] + caret[2] * 1.5 : caret[1] - caret[2] * 0.5;
    const hostBox = host.getBoundingClientRect?.();
    if (hostBox !== undefined && (y < hostBox.top || y > hostBox.bottom)) {
        const blocks = blocksIn(host);
        return down ? lastPlace(blocks.at(-1) ?? host) : firstPlace(blocks[0] ?? host);
    }
    return placeAt(goalX, y, host);
}

// What a browser does by itself, to the DOM.

function collapseSelectionTo(node, offset) {
    dom.selection.collapse(node, offset);
}

/// Takes what's selected out, and leaves the caret there, joining the blocks at each end.
function deleteSelection(host) {
    const range = dom.selectionRange;
    if (range === null || range.collapsed) {
        return;
    }
    const startBlock = blockOf(range.startContainer, host);
    const endBlock = blockOf(range.endContainer, host);
    const start = [range.startContainer, range.startOffset];
    range.deleteContents();
    if (startBlock !== endBlock && endBlock !== host && dom.parentOf(endBlock) !== null && startBlock !== host) {
        mergeBlocks(startBlock, endBlock);
    }
    const node = start[0].isConnected === false && range.startContainer ? range.startContainer : start[0];
    collapseSelectionTo(node, Math.min(start[1], dom.nodeLength(node)));
    tidyBlock(startBlock);
}

/// Moves what's in `second` to the end of `first`, and takes `second` away.
function mergeBlocks(first, second) {
    for (const br of [...dom.childList(first)].filter((one) => one.localName === 'br' && one === dom.childList(first).at(-1))) {
        br.remove();
    }
    // What's only there to give an empty block a line goes with it, as a browser joining them leaves it out.
    const placeholder = blockText(second) === '\n' ? dom.childList(second).find((one) => one.localName === 'br') : undefined;
    for (const child of [...dom.childList(second)]) {
        if (child !== placeholder && !(child.nodeType === TEXT && child.__data === '')) {
            first.appendChild(child);
        }
    }
    second.remove();
}

/// A block left with nothing in it gets a `<br>`, as a browser leaves one, so that it still has a line.
function tidyBlock(block) {
    if (block === null || block === undefined || !dom.isOwn(block)) {
        return;
    }
    const text = blockText(block);
    if (text === '' && dom.childList(block).every((one) => one.nodeType !== ELEMENT || one.localName !== 'br')) {
        for (const empty of dom.childList(block).filter((one) => one.nodeType === TEXT)) {
            empty.remove();
        }
        block.appendChild(web.document.createElement('br'));
        collapseSelectionTo(block, 0);
    }
}

/// Where text typed now goes: a text node, made if there isn't one there.
function textPlace(host) {
    const range = dom.selectionRange;
    let node = range?.startContainer ?? host;
    let offset = range?.startOffset ?? 0;
    if (node.nodeType === TEXT) {
        return [node, offset];
    }
    const kids = dom.childList(node);
    const before = kids[offset - 1];
    const after = kids[offset];
    if (before?.nodeType === TEXT) {
        return [before, before.__data.length];
    }
    if (after?.nodeType === TEXT) {
        return [after, 0];
    }
    // Into the text of what's next to it on the line, as a browser types into a `<span>` the caret's at the edge of.
    const inline = (one) => one?.nodeType === ELEMENT && !isBlock(one) && one.localName !== 'br';
    if (inline(before)) {
        const [at, where] = lastPlace(before);
        if (at.nodeType === TEXT) {
            return [at, where];
        }
    }
    if (inline(after)) {
        const [at, where] = firstPlace(after);
        if (at.nodeType === TEXT) {
            return [at, where];
        }
    }
    if (!dom.isOwn(node) && node === host && kids.length === 0) {
        // A host with nothing in it gets a paragraph, as a browser's first line is.
        const block = web.document.createElement('div');
        node.appendChild(block);
        node = block;
        offset = 0;
    }
    const text = web.document.createTextNode('');
    const brAfter = after?.localName === 'br' && dom.childList(node).length === 1;
    node.insertBefore(text, dom.childList(node)[offset] ?? null);
    if (brAfter) {
        after.remove();
    }
    return [text, 0];
}

function insertText(host, data) {
    deleteSelection(host);
    const [node, offset] = textPlace(host);
    node.insertData(offset, data);
    collapseSelectionTo(node, offset + data.length);
}

function insertParagraph(host) {
    deleteSelection(host);
    const range = dom.selectionRange;
    if (range === null) {
        return;
    }
    let block = blockOf(range.startContainer, host);
    if (block === host) {
        // Text right in the host: what's before the caret becomes a block of its own, as a browser's first `<div>`.
        const wrap = web.document.createElement('div');
        for (const child of [...dom.childList(host)]) {
            wrap.appendChild(child);
        }
        host.appendChild(wrap);
        block = wrap;
    }
    const after = block.cloneNode(false);
    if (after.localName === 'h1' || after.localName === 'h2' || after.localName === 'h3') {
        // At the end of a heading, a browser goes on with a paragraph.
    }
    const split = new dom.Range();
    split.setStart(range.startContainer, range.startOffset);
    split.setEnd(...lastPlaceEnd(block));
    const moved = split.extractContents();
    after.appendChild(moved);
    block.parentNode.insertBefore(after, block.nextSibling);
    tidyBlock(block);
    tidyBlock(after);
    const [node, offset] = firstPlace(after);
    collapseSelectionTo(node, offset);
}

/// The very end of a block, after everything in it.
function lastPlaceEnd(block) {
    return [block, dom.childList(block).length];
}

function insertLineBreak(host) {
    deleteSelection(host);
    const range = dom.selectionRange;
    if (range === null) {
        return;
    }
    let node = range.startContainer;
    let offset = range.startOffset;
    if (node.nodeType === TEXT) {
        const tail = node.splitText(offset);
        node = dom.parentOf(tail);
        offset = dom.indexIn(tail);
    }
    const br = web.document.createElement('br');
    node.insertBefore(br, dom.childList(node)[offset] ?? null);
    const block = blockOf(br, host);
    // A break at the end of a block needs another to have a line after it, as a browser puts in.
    if (blockText(block).endsWith('\n')) {
        const pieces = piecesOf(block);
        if (pieces.at(-1)?.node === br) {
            br.parentNode.insertBefore(web.document.createElement('br'), br.nextSibling);
        }
    }
    collapseSelectionTo(dom.parentOf(br), dom.indexIn(br) + 1);
}

function deleteBy(host, direction, granularity) {
    const range = dom.selectionRange;
    if (range === null) {
        return;
    }
    if (!range.collapsed) {
        deleteSelection(host);
        return;
    }
    const [node, offset] = [range.startContainer, range.startOffset];
    const block = blockOf(node, host);
    const index = indexInBlock(block, node, offset);
    const forward = direction === 'forward';
    const text = blockText(block);
    if ((!forward && index === 0) || (forward && index >= text.replace(/\n$/, '').length && blockText(block).length === index)) {
        // Joining with the block before or after.
        const blocks = blocksIn(host);
        const at = blocks.indexOf(block);
        const [first, second] = forward ? [block, blocks[at + 1]] : [blocks[at - 1], block];
        if (first === undefined || second === undefined || first === host || second === host) {
            return;
        }
        const join = lastPlace(first);
        const firstWasEmpty = blockText(first) === '';
        mergeBlocks(first, second);
        if (firstWasEmpty) {
            const [n, o] = firstPlace(first);
            collapseSelectionTo(n, o);
        } else {
            collapseSelectionTo(join[0], join[1]);
        }
        tidyBlock(first);
        return;
    }
    const [toNode, toOffset] = step(host, node, offset, direction, granularity);
    const span = new dom.Range();
    if (forward) {
        span.setStart(node, offset);
        span.setEnd(toNode, toOffset);
    } else {
        span.setStart(toNode, toOffset);
        span.setEnd(node, offset);
    }
    dom.selection.setBaseAndExtent(span.startContainer, span.startOffset, span.endContainer, span.endOffset);
    deleteSelection(host);
}

/// What deleting would take, for `getTargetRanges()`.
function deletionRange(host, direction, granularity) {
    const range = dom.selectionRange;
    if (range === null || !range.collapsed) {
        return range === null ? [] : [range.cloneRange()];
    }
    const [toNode, toOffset] = step(host, range.startContainer, range.startOffset, direction, granularity);
    const out = new dom.Range();
    if (direction === 'forward') {
        out.setStart(range.startContainer, range.startOffset);
        out.setEnd(toNode, toOffset);
    } else {
        out.setStart(toNode, toOffset);
        out.setEnd(range.startContainer, range.startOffset);
    }
    return [out];
}

function insertFragment(host, fragment) {
    deleteSelection(host);
    const range = dom.selectionRange;
    if (range === null) {
        return;
    }
    const last = fragment.lastChild;
    range.insertNode(fragment);
    if (last !== null) {
        const [node, offset] = last.nodeType === TEXT ? [last, last.__data.length] : [dom.parentOf(last), dom.indexIn(last) + 1];
        collapseSelectionTo(node, offset);
    }
}

/// Puts what's selected in `tag`, or takes it out of one, as bold and italic do.
function toggleFormat(host, tag) {
    const range = dom.selectionRange;
    if (range === null || range.collapsed) {
        return;
    }
    const inside = range.startContainer.nodeType === TEXT ? dom.parentOf(range.startContainer) : range.startContainer;
    const wrapper = inside?.closest?.(tag);
    if (wrapper !== null && wrapper !== undefined && host.contains(wrapper)) {
        // Taken out of it.
        const parent = dom.parentOf(wrapper);
        for (const child of [...dom.childList(wrapper)]) {
            parent.insertBefore(child, wrapper);
        }
        wrapper.remove();
        return;
    }
    const element = web.document.createElement(tag);
    const contents = range.extractContents();
    element.appendChild(contents);
    range.insertNode(element);
    dom.selection.setBaseAndExtent(element, 0, element, dom.childList(element).length);
}

// Undo, as a browser keeps it for what it did by itself: the host's content and where the selection was, before each change.

const histories = new WeakMap();

/// The host's content and where the selection is, or with `only`, a block's index, just that block's: text typed into a
/// block leaves the blocks as they were, so undoing it only needs that block back, and the whole of a big document (Lexical's
/// 10,000 paragraphs are ~900 KB of HTML) isn't written out each time someone starts typing again.
function snapshot(host, only = null) {
    const range = dom.selectionRange;
    const block = range === null ? null : blockOf(range.startContainer, host);
    const blocks = blocksIn(host);
    const at = block === null ? null : [blocks.indexOf(block), indexInBlock(block, range.startContainer, range.startOffset)];
    if (only !== null) {
        return { block: only, html: blocks[only]?.innerHTML ?? '', at };
    }
    return { block: null, html: host.innerHTML ?? '', at };
}

/// The block text typed now goes into, if that's all it changes: the selection is inside one block, not the host itself.
function typedBlock(host, inputType) {
    const range = dom.selectionRange;
    if (inputType !== 'insertText' || range === null) {
        return null;
    }
    const block = blockOf(range.startContainer, host);
    if (block === host || (!dom.isOwn(block) && block.__kids === undefined) || (!range.collapsed && blockOf(range.endContainer, host) !== block)) {
        return null;
    }
    const index = blocksIn(host).indexOf(block);
    return index < 0 ? null : index;
}

function remember(host, inputType) {
    if (inputType === 'historyUndo' || inputType === 'historyRedo' || !dom.isOwn(host) && host.__kids === undefined) {
        return;
    }
    let history = histories.get(host);
    if (history === undefined) {
        history = { undo: [], redo: [], last: null, at: 0, block: null };
        histories.set(host, history);
    }
    // Typing goes together, as a browser groups it, while it's into the same block.
    const now = Date.now();
    const block = typedBlock(host, inputType);
    if (!(inputType === 'insertText' && history.last === 'insertText' && now - history.at < 1000 && block === history.block)) {
        history.undo.push(snapshot(host, block));
        if (history.undo.length > 100) {
            history.undo.shift();
        }
    }
    history.redo = [];
    history.last = inputType;
    history.at = now;
    history.block = block;
}

function replaceContent(node, html) {
    if (dom.isOwn(node)) {
        node.innerHTML = html;
    } else {
        for (const child of [...(node.__kids ?? [])]) {
            child.remove();
        }
        node.appendChild(dom.parseHTML(html));
    }
}

function restore(host, state) {
    if (state.block !== null) {
        // Only text was typed into this block since, so the blocks are as they were.
        const block = blocksIn(host)[state.block];
        if (block === undefined) {
            return;
        }
        replaceContent(block, state.html);
    } else {
        replaceContent(host, state.html);
    }
    if (state.at !== null) {
        const block = blocksIn(host)[state.at[0]];
        if (block !== undefined) {
            const [node, offset] = placeInBlock(block, state.at[1]);
            collapseSelectionTo(node, offset);
        }
    }
}

function undo(host, redo = false) {
    const history = histories.get(host);
    if (history === undefined) {
        return;
    }
    const from = redo ? history.redo : history.undo;
    const to = redo ? history.undo : history.redo;
    const state = from.pop();
    if (state === undefined) {
        return;
    }
    to.push(snapshot(host, state.block));
    history.last = null;
    restore(host, state);
}

// Keys.

function keyDown(event) {
    const host = active;
    committed = null;
    if (host === null || event.defaultPrevented || event.isTrusted !== true) {
        return;
    }
    const key = event.key;
    const ctrl = event.ctrlKey || event.metaKey;
    const shift = event.shiftKey;
    const range = dom.selectionRange;
    if (key !== 'ArrowUp' && key !== 'ArrowDown') {
        goalX = null;
    }
    const move = (granularity, direction) => {
        if (range === null) return;
        const focus = [dom.selection.focusNode, dom.selection.focusOffset];
        let target;
        if (!shift && !range.collapsed && granularity === 'character') {
            target = direction === 'forward' ? [range.endContainer, range.endOffset] : [range.startContainer, range.startOffset];
        } else if (granularity === 'vertical') {
            target = vertical(host, focus[0], focus[1], direction === 'forward');
        } else {
            target = step(host, focus[0], focus[1], direction, granularity);
        }
        if (shift) {
            dom.selection.extend(target[0], target[1]);
        } else {
            collapseSelectionTo(target[0], target[1]);
        }
        blinkFrom();
        redrawCarets();
        event.preventDefault?.();
    };
    switch (key) {
        case 'ArrowLeft':
            return move(ctrl ? 'word' : 'character', 'backward');
        case 'ArrowRight':
            return move(ctrl ? 'word' : 'character', 'forward');
        case 'ArrowUp':
            return move(ctrl ? 'paragraph' : 'vertical', 'backward');
        case 'ArrowDown':
            return move(ctrl ? 'paragraph' : 'vertical', 'forward');
        case 'Home':
            return move(ctrl ? 'documentboundary' : 'lineboundary', 'backward');
        case 'End':
            return move(ctrl ? 'documentboundary' : 'lineboundary', 'forward');
        case 'Backspace': {
            const granularity = ctrl ? 'word' : 'character';
            const type = range !== null && !range.collapsed ? 'deleteContentBackward' : ctrl ? 'deleteWordBackward' : 'deleteContentBackward';
            edit(type, null, () => deleteBy(host, 'backward', granularity), { ranges: deletionRange(host, 'backward', granularity) });
            return;
        }
        case 'Delete': {
            const granularity = ctrl ? 'word' : 'character';
            const type = range !== null && !range.collapsed ? 'deleteContentForward' : ctrl ? 'deleteWordForward' : 'deleteContentForward';
            edit(type, null, () => deleteBy(host, 'forward', granularity), { ranges: deletionRange(host, 'forward', granularity) });
            return;
        }
        case 'Enter':
            if (shift) {
                edit('insertLineBreak', null, () => insertLineBreak(host));
            } else {
                edit('insertParagraph', null, () => insertParagraph(host));
            }
            return;
        default:
    }
    if (ctrl && !event.altKey) {
        switch (String(key).toLowerCase()) {
            case 'a':
                selectAll(host);
                event.preventDefault?.();
                return;
            case 'z':
                edit(shift ? 'historyRedo' : 'historyUndo', null, () => undo(host, shift));
                event.preventDefault?.();
                return;
            case 'y':
                edit('historyRedo', null, () => undo(host, true));
                event.preventDefault?.();
                return;
            case 'b':
                edit('formatBold', null, () => toggleFormat(host, 'b'));
                return;
            case 'i':
                edit('formatItalic', null, () => toggleFormat(host, 'i'));
                return;
            case 'u':
                edit('formatUnderline', null, () => toggleFormat(host, 'u'));
                return;
            default:
        }
    }
}

function selectAll(host) {
    const blocks = blocksIn(host);
    const [startNode, startOffset] = firstPlace(blocks[0] ?? host);
    const [endNode, endOffset] = lastPlace(blocks.at(-1) ?? host);
    dom.selection.setBaseAndExtent(startNode, startOffset, endNode, endOffset);
    redrawCarets();
}

function typed(event) {
    const host = active;
    const text = event.data ?? '';
    if (host === null || text === '' || event.defaultPrevented || event.isTrusted !== true) {
        return;
    }
    if (committed !== null) {
        // What a way of typing put together is typed when it's done, which it already is.
        const same = committed === text;
        committed = null;
        if (same) {
            return;
        }
    }
    edit('insertText', text, () => insertText(host, text));
}

// Ways of typing: what's being put together is in the text as it changes, as a browser shows it, and what it came to stays
// when it's done, said as one last change of it before `compositionend`, as Chromium says it. What's then typed, which is that
// again, isn't typed twice.

let composing = null;
let committed = null;

function compositionStart(event) {
    const host = active;
    committed = null;
    if (host === null || event.isTrusted !== true) {
        return;
    }
    // Where it goes is worked out when there's first something to put there, as what's listening may change the page first, as
    // an editor taking out what was selected does.
    composing = { node: null, offset: 0, length: 0 };
}

function compositionUpdate(event) {
    const host = active;
    if (host === null || composing === null || event.isTrusted !== true) {
        return;
    }
    compose(event.data ?? '');
}

function compose(data) {
    const host = active;
    edit('insertCompositionText', data, () => {
        if (composing.node === null || composing.node.isConnected === false || !host.contains(composing.node)) {
            const range = dom.selectionRange;
            if (range === null || !host.contains(range.startContainer)) {
                // With no caret in it, there's nowhere for it to go, as what was selected went with what was drawn there.
                return;
            }
            // Into what's selected now, in place of it, as a way of typing starts.
            deleteSelection(host);
            const [at, where] = textPlace(host);
            Object.assign(composing, { node: at, offset: where, length: 0 });
        }
        const { node, offset, length } = composing;
        if (node.isConnected !== false) {
            node.replaceData(offset, Math.min(length, node.__data.length - offset), data);
            collapseSelectionTo(node, offset + data.length);
        }
        composing.length = data.length;
    }, { cancelable: false });
}

/// Before the page hears it's done.
function compositionEnd(event) {
    if (composing === null || event.isTrusted !== true) {
        return;
    }
    const data = event.data ?? '';
    if (active !== null) {
        compose(data);
    }
    composing = null;
    committed = data === '' ? null : data;
}

// The pointer: a press puts the caret where it is, two select a word and three a paragraph, and moving with it down selects.

let pressed = null;
let lastPress = { at: 0, x: 0, y: 0, count: 0 };
let moveListening = false;
const POINTER_MOVE_DEFAULT = { __last: true };
// Same-program diagnostic control for the former permanent subscription.
const KEEP_IDLE_MOVES = (() => {
    try {
        return globalThis.process?.env?.RSUI_EDITING_IDLE_MOVES === '1';
    } catch {
        return false;
    }
})();

/// Selection only observes movement during an accepted press. Merely loading
/// editing (for example, reading getSelection()) must not make every later mouse
/// movement allocate and dispatch a page event for an otherwise idle editor.
function syncMoveListener() {
    const wanted = KEEP_IDLE_MOVES || pressed !== null || pagePressed !== null;
    if (wanted === moveListening) {
        return;
    }
    moveListening = wanted;
    if (wanted) {
        web.listenOn('window', 'mousemove', pointerMove, POINTER_MOVE_DEFAULT);
    } else {
        web.unlistenOn('window', 'mousemove', pointerMove, POINTER_MOVE_DEFAULT);
    }
}

/// A move can be the first observation that the button was released outside
/// the page. Stop tracking without collapsing a selection or synthesizing an up.
function cancelPointerPress() {
    pressed = null;
    pagePressed = null;
    syncMoveListener();
}

function pointerDown(event) {
    if ((event.button !== undefined && event.button !== 0) || event.defaultPrevented || event.isTrusted !== true) {
        return;
    }
    const target = event.target;
    const host = target === null || target === undefined ? null : editingHost(target);
    if (host === null) {
        if (active !== null) {
            focusHost(null);
        }
        pagePress(event);
        return;
    }
    pagePressed = null;
    const now = Date.now();
    const near = Math.abs(event.clientX - lastPress.x) < 4 && Math.abs(event.clientY - lastPress.y) < 4;
    const count = event.detail > 1 ? event.detail : near && now - lastPress.at < 500 ? lastPress.count + 1 : 1;
    lastPress = { at: now, x: event.clientX, y: event.clientY, count };
    focusHost(host, false);
    const [node, offset] = placeAt(event.clientX, event.clientY, host);
    goalX = null;
    if (count === 2 || count === 3) {
        const block = blockOf(node, host);
        if (count === 3) {
            // The paragraph, which a line break ends as the block's end does.
            const index = indexInBlock(block, node, offset);
            let from = 0;
            let to = blockText(block).length;
            for (const piece of piecesOf(block)) {
                if (!piece.br) {
                    continue;
                }
                if (piece.start < index) {
                    from = piece.start + 1;
                } else {
                    // Up to the start of the next line, after the break, as Chromium selects it.
                    to = piece.start + 1;
                    break;
                }
            }
            const [a, b] = from === 0 ? firstPlace(block) : placeInBlock(block, from, true);
            const [c, d] = placeInBlock(block, to, true);
            dom.selection.setBaseAndExtent(a, b, c, d);
        } else {
            const text = blockText(block);
            let from = indexInBlock(block, node, offset);
            let to = from;
            if (isWordAt(text, from) || isWordAt(text, from - 1)) {
                while (from > 0 && isWordAt(text, from - 1)) from -= 1;
                while (to < text.length && isWordAt(text, to)) to += 1;
                // With the spaces after it, as on Windows.
                while (to < text.length && /[ \t ]/.test(text[to])) to += 1;
            } else if (to < text.length) {
                to += 1;
            }
            const [a, b] = placeInBlock(block, from, true);
            const [c, d] = placeInBlock(block, to);
            dom.selection.setBaseAndExtent(a, b, c, d);
        }
    } else if (event.shiftKey && dom.selectionRange !== null) {
        dom.selection.extend(node, offset);
    } else {
        collapseSelectionTo(node, offset);
    }
    pressed = { host };
    syncMoveListener();
    blinkFrom();
    redrawCarets();
}

function pointerMove(event) {
    if (pressed === null) {
        pageMove(event);
        return;
    }
    if (event.buttons !== undefined && (event.buttons & 1) === 0) {
        cancelPointerPress();
        return;
    }
    const [node, offset] = placeAt(event.clientX, event.clientY, pressed.host);
    dom.selection.extend(node, offset);
    redrawCarets();
}

function pointerUp() {
    pressed = null;
    pageRelease();
}

// Page text that isn't edited, selected with the pointer as a browser does it: a press leaves a collapsed selection where it is, two
// select a word (with the spaces after it, as on Windows) and three the paragraph (to where the next text that can be selected
// starts, as Chromium has it), Shift extends, and moving with the button down selects. A press on what's selected may drag it, so
// it only collapses the selection if the button comes up where it went down. A press in a control (a button, a field) or on what's
// `user-select: none` leaves the selection as it is.

let pagePressed = null;

/// Whether the text in `node` can be selected: not in what says `user-select: none`, unless something in that says `text` again,
/// and with the pointer (`pressed`), not in a control, which has the press for itself.
function selectsText(node, pressed = true) {
    for (let at = node; at !== null && at !== undefined; at = dom.parentOf(at)) {
        if (at.nodeType !== ELEMENT) {
            continue;
        }
        const said = at.__live?.look?.__userSelect;
        if (said === 'none') {
            return false;
        }
        if (said === 'text' || said === 'all') {
            return true;
        }
        if (pressed && /^(button|input|textarea|select|option)$/.test(String(at.localName))) {
            return false;
        }
    }
    return true;
}

/// Whether a place is inside what a range covers, ends and all.
function insideRange(node, offset, range) {
    return dom.comparePoints(node, offset, range.startContainer, range.startOffset) >= 0
        && dom.comparePoints(node, offset, range.endContainer, range.endOffset) <= 0;
}

/// Puts the document's selection here, collapsed, unless it's there already, so that nothing hears it change when it hasn't.
function collapsePageSelection(node, offset) {
    const selection = dom.selection;
    if (selection.rangeCount === 1 && selection.anchorNode === node && selection.anchorOffset === offset && selection.isCollapsed) {
        return;
    }
    selection.collapse(node, offset);
}

/// The first place in a block after `block`, in the page's order, whose text can be selected, or null.
function nextSelectablePlace(block, host) {
    const blocks = blocksIn(host);
    for (let at = blocks.indexOf(block) + 1; at > 0 && at < blocks.length; at += 1) {
        const place = firstPlace(blocks[at]);
        if (selectsText(place[0], false)) {
            return place;
        }
    }
    return null;
}

/// The place on the page nearest a point: in the text of the block under it, if it has any there (a toast fixed over the page
/// isn't where it comes in the page's order), or else the nearest block down the page.
function pagePlaceAt(x, y, host) {
    const under = web.document.elementFromPoint?.(x, y);
    if (under !== null && under !== undefined && under !== host) {
        const block = blockOf(under, host);
        const key = drawnKey(block);
        const box = key === undefined ? null : blockBox(block);
        if (box !== null && y >= box.top && y <= box.bottom && piecesOf(block).length > 0) {
            return placeInBlock(block, rsui.__host().textIndexAt(key, x, y) ?? 0);
        }
        // Over something with no text of its own there: the nearest place in the nearest thing around it that has text.
        for (let at = under; at !== null && at !== undefined && at !== host; at = dom.parentOf(at)) {
            if (at.nodeType === ELEMENT && blocksIn(at).some((one) => drawnKey(one) !== undefined && piecesOf(one).length > 0)) {
                return nearestPlaceIn(x, y, at);
            }
        }
    }
    return nearestPlaceIn(x, y, host);
}

/// The place nearest a point in the blocks inside `root`, down and then across, wherever each is drawn: what's fixed, as a toast,
/// isn't where it comes in the page's order, which `placeAt` goes by.
function nearestPlaceIn(x, y, root) {
    let best = null;
    for (const block of blocksIn(root)) {
        const key = drawnKey(block);
        const box = key === undefined ? null : blockBox(block);
        if (box === null) {
            continue;
        }
        const away = y < box.top ? box.top - y : y > box.bottom ? y - box.bottom : 0;
        const across = x < box.left ? box.left - x : x > box.right ? x - box.right : 0;
        // A block with no text of its own (one with only boxes in it, around paragraphs) is a place only from outside it, as below
        // everything: inside it, the text nearest is.
        if (away === 0 && across === 0 && piecesOf(block).length === 0) {
            continue;
        }
        if (best === null || away < best.away || (away === best.away && across < best.across)) {
            best = { block, key, away, across };
        }
    }
    if (best === null) {
        return firstPlace(root);
    }
    return placeInBlock(best.block, rsui.__host().textIndexAt(best.key, x, y) ?? 0);
}

/// A press on the page, after everything's heard it, where it isn't in something that's edited.
function pagePress(event) {
    if ((event.button !== undefined && event.button !== 0) || event.defaultPrevented || event.isTrusted !== true) {
        return;
    }
    const target = event.target;
    if (target === null || target === undefined || !selectsText(target)) {
        return;
    }
    const host = web.document.body;
    const [node, offset] = pagePlaceAt(event.clientX, event.clientY, host);
    if (!selectsText(node)) {
        return;
    }
    const count = Math.max(1, Number(event.detail) || 1);
    const range = dom.selectionRange;
    if (count === 1 && !event.shiftKey && range !== null && !range.collapsed && insideRange(node, offset, range)) {
        pagePressed = { host, at: [node, offset], x: event.clientX, y: event.clientY, dragging: true };
        syncMoveListener();
        return;
    }
    const block = blockOf(node, host);
    if (count === 2) {
        const text = blockText(block);
        let from = indexInBlock(block, node, offset);
        let to = from;
        if (isWordAt(text, from) || isWordAt(text, from - 1)) {
            while (from > 0 && isWordAt(text, from - 1)) from -= 1;
            while (to < text.length && isWordAt(text, to)) to += 1;
            // With the spaces after it, as on Windows.
            while (to < text.length && /[ \t ]/.test(text[to])) to += 1;
        } else if (to < text.length) {
            to += 1;
        }
        const [a, b] = placeInBlock(block, from, true);
        const [c, d] = placeInBlock(block, to);
        dom.selection.setBaseAndExtent(a, b, c, d);
    } else if (count >= 3) {
        const [a, b] = firstPlace(block);
        const [c, d] = nextSelectablePlace(block, host) ?? lastPlace(block);
        dom.selection.setBaseAndExtent(a, b, c, d);
    } else if (event.shiftKey && range !== null) {
        dom.selection.extend(node, offset);
    } else {
        collapsePageSelection(node, offset);
    }
    pagePressed = { host, at: null, dragging: false };
    syncMoveListener();
}

/// Moving with the button down from a press on the page selects up to where it is, unless the press may be dragging what was
/// selected.
function pageMove(event) {
    if (event.buttons !== undefined && (event.buttons & 1) === 0) {
        cancelPointerPress();
        return;
    }
    if (pagePressed === null) {
        return;
    }
    if (pagePressed.dragging) {
        pagePressed.moved ||= Math.abs(event.clientX - pagePressed.x) > 3 || Math.abs(event.clientY - pagePressed.y) > 3;
        return;
    }
    const [node, offset] = pagePlaceAt(event.clientX, event.clientY, pagePressed.host);
    if (!selectsText(node)) {
        return;
    }
    const selection = dom.selection;
    if (selection.focusNode !== node || selection.focusOffset !== offset) {
        selection.extend(node, offset);
    }
}

/// The button coming up where it went down on what was selected leaves the selection collapsed there.
function pageRelease() {
    const was = pagePressed;
    pagePressed = null;
    syncMoveListener();
    if (was?.dragging && !was.moved && was.at !== null) {
        collapsePageSelection(was.at[0], was.at[1]);
    }
}

// The clipboard. Text goes to the system's; the HTML that came with it is kept here, for pasting what was copied in the app.

function selectedHTML() {
    const range = dom.selectionRange;
    if (range === null || range.collapsed) {
        return '';
    }
    return dom.serialize(range.cloneContents());
}

function copied(event) {
    const host = active;
    const range = dom.selectionRange;
    if (event.defaultPrevented || event.isTrusted !== true || range === null || range.collapsed || host === null || !host.contains(range.startContainer)) {
        return;
    }
    event.clipboardData?.setData('text/html', selectedHTML());
    event.clipboardData?.setData('text/plain', range.toString());
    if (event.type === 'cut') {
        edit('deleteByCut', null, () => deleteSelection(host));
    }
}

function pasted(event) {
    const host = active;
    if (host === null || event.defaultPrevented || event.isTrusted !== true) {
        return;
    }
    const data = event.clipboardData;
    const html = data?.getData('text/html') ?? '';
    const text = data?.getData('text/plain') ?? '';
    const transfer = data ?? null;
    edit('insertFromPaste', null, () => {
        if (html !== '') {
            insertFragment(host, dom.parseHTML(html));
        } else if (text !== '') {
            insertPlain(host, text);
        }
    }, { dataTransfer: transfer });
}

/// Text with line breaks, as blocks.
function insertPlain(host, text) {
    const lines = text.replace(/\r\n?/g, '\n').split('\n');
    lines.forEach((line, index) => {
        if (index > 0) {
            insertParagraph(host);
        }
        if (line !== '') {
            insertText(host, line);
        }
    });
}

// `document.execCommand()`.

function execCommand(command, _showUI, value) {
    const host = active ?? (dom.selectionRange === null ? null : editingHost(dom.selectionRange.startContainer));
    if (host === null) {
        return false;
    }
    if (active === null) {
        focusHost(host, false);
    }
    const name = String(command).toLowerCase();
    const run = (type, data, perform) => {
        edit(type, data, perform, { cancelable: false });
        return true;
    };
    switch (name) {
        case 'inserttext':
            return run('insertText', String(value ?? ''), () => insertText(host, String(value ?? '')));
        case 'inserthtml':
            return run('insertFromPaste', null, () => insertFragment(host, dom.parseHTML(String(value ?? ''))));
        case 'delete':
            return run('deleteContentBackward', null, () => deleteBy(host, 'backward', 'character'));
        case 'forwarddelete':
            return run('deleteContentForward', null, () => deleteBy(host, 'forward', 'character'));
        case 'insertparagraph':
            return run('insertParagraph', null, () => insertParagraph(host));
        case 'insertlinebreak':
            return run('insertLineBreak', null, () => insertLineBreak(host));
        case 'bold':
            return run('formatBold', null, () => toggleFormat(host, 'b'));
        case 'italic':
            return run('formatItalic', null, () => toggleFormat(host, 'i'));
        case 'underline':
            return run('formatUnderline', null, () => toggleFormat(host, 'u'));
        case 'strikethrough':
            return run('formatStrikeThrough', null, () => toggleFormat(host, 's'));
        case 'selectall':
            selectAll(host);
            return true;
        case 'undo':
            return run('historyUndo', null, () => undo(host));
        case 'redo':
            return run('historyRedo', null, () => undo(host, true));
        case 'createlink':
            return run('insertLink', String(value ?? ''), () => {
                toggleFormat(host, 'a');
                const range = dom.selectionRange;
                const link = range?.startContainer?.nodeType === ELEMENT ? range.startContainer.closest?.('a') ?? range.startContainer : null;
                link?.setAttribute?.('href', String(value ?? ''));
            });
        case 'unlink':
            return run('formatRemove', null, () => toggleFormat(host, 'a'));
        case 'removeformat':
            return run('formatRemove', null, () => {
                for (const tag of ['b', 'i', 'u', 's', 'strong', 'em']) {
                    const inside = dom.selectionRange?.startContainer;
                    const wrapper = (inside?.nodeType === TEXT ? dom.parentOf(inside) : inside)?.closest?.(tag);
                    if (wrapper && host.contains(wrapper)) {
                        toggleFormat(host, tag);
                    }
                }
            });
        case 'formatblock': {
            const tag = String(value ?? 'p').replace(/[<>]/g, '').toLowerCase();
            return run('formatBlock', tag, () => {
                const range = dom.selectionRange;
                const block = range === null ? null : blockOf(range.startContainer, host);
                if (block === null || block === host) {
                    return;
                }
                const replaced = web.document.createElement(tag);
                for (const child of [...dom.childList(block)]) {
                    replaced.appendChild(child);
                }
                block.replaceWith(replaced);
                const [node, offset] = firstPlace(replaced);
                collapseSelectionTo(node, offset);
            });
        }
        default:
            return false;
    }
}

function queryCommandState(command) {
    const tag = { bold: 'b', italic: 'i', underline: 'u', strikethrough: 's' }[String(command).toLowerCase()];
    const range = dom.selectionRange;
    if (tag === undefined || range === null) {
        return false;
    }
    const inside = range.startContainer.nodeType === TEXT ? dom.parentOf(range.startContainer) : range.startContainer;
    return inside?.closest?.(`${tag},${tag === 'b' ? 'strong' : tag === 'i' ? 'em' : tag}`) != null;
}

const SUPPORTED = new Set(['inserttext', 'inserthtml', 'delete', 'forwarddelete', 'insertparagraph', 'insertlinebreak', 'bold', 'italic', 'underline', 'strikethrough', 'selectall', 'undo', 'redo', 'createlink', 'unlink', 'removeformat', 'formatblock']);

// Drawing the caret and what's selected: boxes inside the host, drawn over its text.

const overlays = new Set();
let caretShown = true;
let blink = 0;
let blinks = 0;
// How many times the caret changes before it stops blinking and stays shown, as a native text box's does (rsui's
// `CARET_CHANGES`): each change draws a frame, so a caret left blinking would keep the window from ever going quiet, which is
// when it gives back memory and lets go of the GPU.
const CARET_CHANGES = 20;

/// Shows the caret and starts its blinking over, as focus moving to an editor, typing, or moving the caret does, so it holds
/// still while that's happening.
function blinkFrom() {
    clearInterval(blink);
    blink = 0;
    blinks = 0;
    caretShown = true;
    if (active !== null && !globalThis.__rsuiNoBlink) {
        blink = setInterval(() => {
            blinks += 1;
            caretShown = blinks >= CARET_CHANGES || !caretShown;
            if (blinks >= CARET_CHANGES) {
                clearInterval(blink);
                blink = 0;
            }
            for (const one of overlays) {
                if (one.host === active) {
                    one.redraw();
                }
            }
        }, 530);
    }
}

// Where the caret and what's selected are drawn is worked out once the page has been laid out with whatever changed, as it's
// asked of that layout (rather than one made for asking, a change behind), and then they're drawn. Saying that it's to be
// worked out changes what's drawn of it, so that the page is laid out again even when only the selection moved.
let redrawing = false;
let askedFor = 0;
function redrawCarets() {
    if (redrawing) {
        return;
    }
    redrawing = true;
    queueMicrotask(() => {
        redrawing = false;
        askedFor += 1;
        web.afterLayout(placeCarets);
        for (const one of overlays) {
            one.redraw();
        }
    });
}

function placeCarets() {
    for (const one of overlays) {
        one.found = boxesFor(one.host);
        one.placed = askedFor;
        one.redraw();
    }
}

/// The boxes to draw for what's selected in a host, from its top left, as `[x, y, width, height]`, and the caret.
function boxesFor(host) {
    const range = dom.selectionRange;
    const own = host.getBoundingClientRect?.();
    if (range === null || own === undefined || !host.contains(range.startContainer)) {
        return null;
    }
    const at = (place) => {
        const caret = caretAt(place[0], place[1]);
        return caret === null ? null : [caret[0] - own.left, caret[1] - own.top, caret[2]];
    };
    if (range.collapsed) {
        const caret = active === host ? at([range.startContainer, range.startOffset]) : null;
        return { caret, boxes: [] };
    }
    const boxes = [];
    const blocks = blocksIn(host);
    const startBlock = blockOf(range.startContainer, host);
    const endBlock = blockOf(range.endContainer, host);
    // Only what's in the window is drawn, as what's selected can be thousands of paragraphs: from the first that shows.
    const first = Math.max(blocks.indexOf(startBlock), firstReaching(blocks, 0));
    const last = blocks.indexOf(endBlock);
    if (last !== -1 && last < first) {
        return { caret: null, boxes };
    }
    const bottom = globalThis.innerHeight ?? 10000;
    let inside = false;
    for (let index = Math.max(0, first); index < blocks.length; index += 1) {
        const block = blocks[index];
        if (block === startBlock || index === first) inside = true;
        if (!inside) continue;
        const shown = blockBox(block);
        if (shown !== null && shown.top > bottom) break;
        const from = block === startBlock ? at([range.startContainer, range.startOffset]) : at(firstPlace(block));
        const to = block === endBlock ? at([range.endContainer, range.endOffset]) : at(lastPlace(block));
        const box = blockBox(block);
        if (from !== null && to !== null) {
            const right = (box?.right ?? own.right) - own.left;
            const left = (box?.left ?? own.left) - own.left;
            if (Math.abs(from[1] - to[1]) < 1) {
                boxes.push([from[0], from[1], Math.max(2, to[0] - from[0] + (block !== endBlock ? 6 : 0)), from[2]]);
            } else {
                boxes.push([from[0], from[1], Math.max(2, right - from[0]), from[2]]);
                if (to[1] > from[1] + from[2]) {
                    boxes.push([left, from[1] + from[2], right - left, to[1] - from[1] - from[2]]);
                }
                boxes.push([left, to[1], Math.max(2, to[0] - left), to[2]]);
            }
        }
        if (block === endBlock) break;
    }
    return { caret: null, boxes };
}

/// What's selected is drawn over its text in Windows' highlight blue, see-through, as rsui writes colors.
const SELECTED = '#0078d755';

/// What's drawn over a host: the caret while it has the keyboard, and what's selected.
function Overlay({ host }) {
    const [, setVersion] = useState(0);
    const kept = useRef(null);
    kept.current ??= { host, found: null, placed: 0, redraw: () => setVersion((v) => v + 1) };
    const mine = kept.current;
    mine.host = host;
    useLayoutEffect(() => {
        overlays.add(mine);
        return () => overlays.delete(mine);
    }, [mine]);
    const found = mine.found;
    // Waiting to be placed, which says so, so that what it's drawn on is laid out again.
    const waiting = mine.placed !== askedFor;
    if (found === null) {
        return waiting ? React.createElement('group', { key: '__editing', position: 'absolute', left: 0, top: 0, right: 0, bottom: 0, inert: true, overflow: 'visible', asked: askedFor }) : null;
    }
    // As the page says its text or caret is colored, which rsui takes as a number, as CSS's colors come to.
    const said = host.__live?.look?.caretColor ?? host.__live?.look?.color;
    const color = typeof said === 'number' ? said : typeof said === 'string' ? globalThis.__rsuiColor?.(said) ?? '#000000' : '#000000';
    const parts = found.boxes.map(([x, y, width, height], index) => React.createElement('group', {
        key: 's' + index, position: 'absolute', left: x, top: y, width, height, background: SELECTED, inert: true,
    }));
    if (found.caret !== null && caretShown) {
        const [x, y, height] = found.caret;
        parts.push(React.createElement('group', { key: 'caret', position: 'absolute', left: Math.round(x), top: y, width: 1, height, background: color ?? '#000000', inert: true }));
    }
    // As big as the host by where its sides are, rather than by a size, which would change as the host grows, and have the host's
    // thousands of paragraphs laid out again only for the caret to move.
    return React.createElement('group', { key: '__editing', position: 'absolute', left: 0, top: 0, right: 0, bottom: 0, inert: true, overflow: 'visible', asked: waiting ? askedFor : 0 }, ...parts);
}

/// What `Styled` adds to an element that can be edited: an overlay, and the props it needs.
function hostExtras(handle) {
    return React.createElement(Overlay, { key: '__editing', host: handle });
}

// What `dom.js` asks about geometry and editing.

const geometry = {
    editingHost,
    selectionMoved() {
        redrawCarets();
    },
    rangeRects(range) {
        const start = caretAt(range.startContainer, range.startOffset);
        if (start === null) {
            return [];
        }
        const end = range.collapsed ? start : caretAt(range.endContainer, range.endOffset) ?? start;
        const rect = (x, y, width, height) => ({ x, y, left: x, top: y, right: x + width, bottom: y + height, width, height });
        if (Math.abs(start[1] - end[1]) < 1) {
            return [rect(Math.min(start[0], end[0]), start[1], Math.abs(end[0] - start[0]), start[2])];
        }
        return [rect(start[0], start[1], 0, start[2]), rect(end[0], end[1], 0, end[2])];
    },
    modify(selection, alter, direction, granularity) {
        const host = editingHost(selection.focusNode);
        if (host === null) {
            return;
        }
        const dir = direction === 'left' || direction === 'backward' ? 'backward' : 'forward';
        const [node, offset] = granularity === 'line'
            ? vertical(host, selection.focusNode, selection.focusOffset, dir === 'forward')
            : step(host, selection.focusNode, selection.focusOffset, dir, granularity);
        if (alter === 'extend') {
            selection.extend(node, offset);
        } else {
            selection.collapse(node, offset);
        }
    },
};

/// Where in the DOM a point on the window is, as `caretPositionFromPoint()` says.
function caretPositionFromPoint(x, y) {
    const target = web.document.elementFromPoint?.(x, y);
    const host = target === null || target === undefined ? null : editingHost(target) ?? target;
    if (host === null) {
        return null;
    }
    const [node, offset] = placeAt(x, y, host);
    return { offsetNode: node, offset, getClientRect: () => geometry.rangeRects(rangeAt(node, offset))[0] ?? null };
}

function rangeAt(node, offset) {
    const range = new dom.Range();
    range.setStart(node, offset);
    range.collapse(true);
    return range;
}

function install(hooks, domPack) {
    web = hooks;
    dom = domPack;
    // What already has the keyboard may be something that can now be edited.
    const now = web.document.activeElement;
    active = now === web.document.body ? null : editingHost(now);
    // What a browser does when nothing's said not to, after everything else has heard it.
    const last = { __last: true };
    web.listenOn('window', 'keydown', keyDown, last);
    web.listenOn('window', 'rsuitext', typed, last);
    web.listenOn('window', 'compositionstart', compositionStart, last);
    web.listenOn('window', 'compositionupdate', compositionUpdate, last);
    web.listenOn('window', 'compositionend', compositionEnd, { capture: true });
    web.listenOn('window', 'mousedown', pointerDown, last);
    syncMoveListener();
    web.listenOn('window', 'mouseup', pointerUp, last);
    web.listenOn('window', 'copy', copied, last);
    web.listenOn('window', 'cut', copied, last);
    web.listenOn('window', 'paste', pasted, last);
    // What's selected that scrolls into the window is drawn then.
    web.listenOn('window', 'scroll', () => redrawCarets(), { capture: true });
}

module.exports = {
    install,
    /// What's selected in what has the keyboard, as `compositionstart` says it's what the way of typing replaces.
    selectedText() {
        const range = dom.selectionRange;
        return active === null || range === null || range.collapsed ? '' : range.toString();
    },
    geometry,
    hostExtras,
    editingHost,
    execCommand,
    queryCommandState,
    queryCommandSupported: (command) => SUPPORTED.has(String(command).toLowerCase()),
    queryCommandEnabled: (command) => SUPPORTED.has(String(command).toLowerCase()) && active !== null,
    caretPositionFromPoint,
    pagePress,
    pageMove,
    pageRelease,
    selectsText,
    caretRangeFromPoint(x, y) {
        const found = caretPositionFromPoint(x, y);
        return found === null ? null : rangeAt(found.offsetNode, found.offset);
    },
    get designMode() {
        return designMode ? 'on' : 'off';
    },
    set designMode(value) {
        designMode = String(value).toLowerCase() === 'on';
    },
    focusHost,
    focusMoved,
};
