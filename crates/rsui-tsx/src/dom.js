// A page's DOM as code changes it: elements, text and comments made with `document.createElement()` and the like, put in a page's
// elements with `appendChild()` and the rest, and drawn as the page's elements are, with `MutationObserver`, live `Range`s and the
// document's `Selection`. This is what editors written for a browser, as ProseMirror and Lexical are, build their documents with.
// It's only loaded the first time code makes or moves a node, or asks for the selection, so nothing else pays for it.
//
// What React draws stays React's. A page's element that's drawn by React can be given children by code too, which are drawn after
// what React gave it, as they are in a browser when React has nothing to say about them.

'use strict';

const React = require('react');
const { useState, useLayoutEffect } = React;

/// What `web.js` gives this when it loads it: see `domPack` there.
let web = null;

const ELEMENT = 1;
const TEXT = 3;
const COMMENT = 8;
const DOCUMENT = 9;
const FRAGMENT = 11;

let nextNode = 1;

// Each node keeps its own: its parent, its children, and what's drawn of it, on itself, out of what's enumerable.
function hidden(object, name, value) {
    Object.defineProperty(object, name, { value, writable: true, configurable: true, enumerable: false });
}

/// What an element with nothing in it has as its children, one list for all of them: the first child put in one is in a list of its
/// own just as long as that (see `insert`), as a list that's grown keeps room for more, which most elements here never have.
const NO_KIDS = Object.freeze([]);

/// An element's attributes are a list of names and values, `[name, value, name, value, ...]`, in the order they were set, as a `Map`
/// keeps them: most elements have one or two, which a `Map` takes several times the room for. One with none has this one, and one
/// that's given another has a new list just as long as that, as a list that's grown keeps room for more.
const NO_ATTRS = Object.freeze([]);

function attrAt(list, key) {
    for (let at = 0; at < list.length; at += 2) {
        if (list[at] === key) {
            return at;
        }
    }
    return -1;
}

/// An attribute's value, or undefined.
function attrOf(node, key) {
    const list = node.__attrs;
    const at = attrAt(list, key);
    return at < 0 ? undefined : list[at + 1];
}

function setAttr(node, key, text) {
    const list = node.__attrs;
    const at = attrAt(list, key);
    if (at >= 0) {
        list[at + 1] = text;
        return;
    }
    switch (list.length) {
        case 0:
            node.__attrs = [key, text];
            break;
        case 2:
            node.__attrs = [list[0], list[1], key, text];
            break;
        case 4:
            node.__attrs = [list[0], list[1], list[2], list[3], key, text];
            break;
        default:
            node.__attrs = list.concat(key, text);
    }
}

function removeAttr(node, key) {
    const list = node.__attrs;
    const at = attrAt(list, key);
    if (at >= 0) {
        node.__attrs = list.length === 2 ? NO_ATTRS : list.filter((_, index) => index !== at && index !== at + 1);
    }
}

/// Each attribute as `[name, value]`, as a `Map`'s entries are.
function attrEntries(node) {
    const list = node.__attrs;
    const out = [];
    for (let at = 0; at < list.length; at += 2) {
        out.push([list[at], list[at + 1]]);
    }
    return out;
}

function attrNames(node) {
    const list = node.__attrs;
    const out = [];
    for (let at = 0; at < list.length; at += 2) {
        out.push(list[at]);
    }
    return out;
}

/// Gives a node made here, or a page's element drawn by React, a list of children of its own, if it doesn't have one.
function kidsOf(node) {
    if (node.__kids === undefined) {
        hidden(node, '__kids', []);
    }
    return node.__kids;
}

/// What a node's in, in the DOM: what it was put in here, or for a page's element drawn by React, what it's drawn in.
function parentOf(node) {
    if (node.__parent !== undefined) {
        return node.__parent;
    }
    if (node.__live !== undefined || node.__domKey !== undefined || node.__chain !== undefined) {
        try {
            let at = node.parentElement ?? null;
            // What rsui has only to lay out what's in it, which isn't any element of the page, is passed over, as `drawnChildren`
            // in `web.js` passes over it on the way down.
            while (at !== null && at.__domKey === undefined && at.__live === undefined && at.__chain !== undefined && at.__index > 1) {
                at = at.parentElement ?? null;
            }
            return at;
        } catch {
            return null;
        }
    }
    return null;
}

/// Whether a node is one made here, rather than a page's element drawn by React.
const isOwn = (node) => node !== null && typeof node === 'object' && node.__own === true;

// Drawing: each node that changes says so, and what's drawn of it is drawn again once what's running has finished.

const waiting = new Set();
let drawing = false;

function changed(node) {
    if (node === null || node === undefined) {
        return;
    }
    // Text is drawn as part of what it's in.
    const owner = node.nodeType === TEXT || node.nodeType === COMMENT ? parentOf(node) : node;
    if (owner === null || owner === undefined) {
        return;
    }
    touched(owner);
    waiting.add(owner);
    if (!drawing) {
        drawing = true;
        queueMicrotask(redrawAll);
    }
}

/// Says that code has changed what's in a page's element, and in what that's in, whose text is the DOM's from then on rather than
/// what was laid out: see `textOf`.
function touched(node) {
    for (let at = node; at !== null && at !== undefined && at.__codeTouched !== true; at = parentOf(at)) {
        if (!isOwn(at)) {
            hidden(at, '__codeTouched', true);
        }
    }
}

function redrawAll() {
    drawing = false;
    const now = [...waiting];
    waiting.clear();
    for (const node of now) {
        if (isOwn(node)) {
            node.__redraw?.();
        } else if (node.__live !== undefined) {
            node.__live?.redraw?.();
        } else {
            // The page's `body`, which draws what code put in it.
            node.__redrawPage?.();
        }
    }
}

/// The props an element made here is drawn with, from its attributes.
function propsOf(node) {
    const props = { key: node.__id, __handle: node };
    const attrs = node.__attrs;
    for (let at = 0; at < attrs.length; at += 2) {
        const name = attrs[at];
        const value = attrs[at + 1];
        switch (name) {
            case 'class':
                props.className = value;
                break;
            case 'style':
                props.style = node.__style.__asProps();
                break;
            case 'for':
                props.htmlFor = value;
                break;
            case 'tabindex':
                props.tabIndex = Number(value);
                break;
            case 'contenteditable':
                props.contentEditable = value;
                break;
            case 'colspan':
                props.colSpan = Number(value);
                break;
            case 'rowspan':
                props.rowSpan = Number(value);
                break;
            case 'draggable':
                props.draggable = value === 'true';
                break;
            case 'hidden':
                props.hidden = true;
                break;
            case 'value':
            case 'checked':
            case 'disabled':
                props[name] = name === 'value' ? value : true;
                break;
            default:
                if (!name.startsWith('on')) {
                    props[name] = value;
                }
        }
    }
    if (props.style === undefined && hasStyle(node) && node.__style.length > 0) {
        props.style = node.__style.__asProps();
    }
    if (node.__editing !== undefined) {
        Object.assign(props, node.__editing);
    }
    return props;
}

/// What a node is drawn as: text as it is, and an element as the page's element for its tag.
function render(node) {
    if (node.nodeType === TEXT) {
        return node.__data;
    }
    if (node.nodeType === ELEMENT) {
        if (node.localName === 'br') {
            const parent = node.__parent;
            const last = parent !== null && parent !== undefined && childList(parent).at(-1) === node;
            return last ? '\u200b' : '\n';
        }
        // The same element each time, as nothing it's made from changes, so that drawing again what it's in, as after a paragraph is
        // put in a document of thousands, needn't make them all again, and can tell that they're the same.
        node.__drawnAs ??= React.createElement(web.isInlineTag(node.localName) ? OwnInline : OwnElement, { key: node.__id, node });
        return node.__drawnAs;
    }
    if (node.nodeType === FRAGMENT) {
        return node.__kids.map(render);
    }
    return null;
}

/// An element made here, drawn again whenever it changes.
function OwnElement({ node }) {
    const [, setVersion] = useState(0);
    node.__redraw = () => setVersion((v) => v + 1);
    useLayoutEffect(() => () => {
        // What's in a plain element drawn without a `Styled` of its own can be drawn in it from now on (see `hostInline` in
        // `web.js`), which has it redrawn with that and kept for the page to find, after it's been drawn by this.
        if (!web.hostedNative(node)) {
            node.__redraw = undefined;
            web.forgetNative(node);
        }
        // It's only when it goes, as `node` is always the same for this: it's what the key it's drawn with is made from (see
        // `render`), and an empty list takes no room of its own.
    }, []);
    const kids = node.__kids;
    const props = propsOf(node);
    if (web.nativeElements) {
        // A plain element is drawn as its host element, without a `Styled` of its own: see `drawNative` in `web.js`.
        const drawn = web.drawNative(node, props, kids, render, propsOf);
        if (drawn !== undefined) {
            return drawn;
        }
    }
    web.forgetNative(node);
    const type = web.domElement(node.localName) ?? web.domElement('span');
    if (kids.length === 0) {
        return React.createElement(type, props);
    }
    return React.createElement(type, props, ...kids.map(render));
}

/// The same, for what goes along a line, as a `<strong>` in a paragraph does, which what it's in needs to know before it's drawn.
function OwnInline(props) {
    return OwnElement(props);
}
OwnInline.__webInline = true;
OwnInline.__webFlowSeen = 'inline';
OwnElement.__webFlowSeen = 'block';

/// What a page's element drawn by React has been given by code, drawn after what React gave it.
function renderKids(owner) {
    const kids = owner.__kids;
    if (kids === undefined || kids.length === 0) {
        return null;
    }
    return kids.map(render);
}

// Mutation records, as `MutationObserver` hears them. What code does here is said as it's done. What React draws changes without
// that, so what's watched that React draws is looked at again after the page has rendered, and a record of each kind that changed
// is made then, without the nodes.

const observers = new Set();
let delivering = false;
/// What looks at what React draws after the page has rendered: see `watchers` in `dom_observers.js`.
let reactWatch = null;

/// What's watched of something React draws, to tell when it's changed: the attributes it's asked about, and what's in it.
function reactSeen(target, options) {
    const attributes = options.attributes
        ? JSON.stringify((options.attributeFilter ?? ['class', 'id', 'style', 'hidden', 'aria-expanded', 'aria-selected', 'data-state', 'open'])
            .map((name) => [name, target.getAttribute?.(name) ?? null]))
        : '';
    // What's in something React gave nothing to draw in it is all code's, which is said exactly as it's done, so it isn't looked
    // through: an editor's root, with a document of thousands of paragraphs in it, would be after every key.
    const inside = (options.childList || options.characterData) && reactDrawsIn(target)
        ? (options.subtree ? String(target.textContent ?? '') + '|' + (target.querySelectorAll?.('*').length ?? 0) : String(target.children?.length ?? 0) + '|' + String(target.textContent ?? ''))
        : '';
    return { attributes, inside };
}

/// Whether React draws anything in a page's element: whether it was given children, which code hasn't taken out. What React draws
/// is only ever in what React drew, as a portal into a node code made is drawn where it's made. The page itself, as `body`, has
/// whatever the app draws.
function reactDrawsIn(target) {
    const live = target.__live;
    if (live === undefined || live === null) {
        return true;
    }
    const children = live.children;
    if (target.__replaced === true || children === undefined || children === null || children === false) {
        return false;
    }
    return !(Array.isArray(children) && children.length === 0);
}

function soonDelivered() {
    if (!delivering) {
        delivering = true;
        queueMicrotask(deliver);
    }
}

function record(type, target, extra) {
    if (observers.size === 0) {
        return;
    }
    for (const observer of observers) {
        for (const [watched, options] of observer.__targets) {
            let reached = target === watched;
            if (!reached && options.subtree) {
                for (let at = parentOf(target); at !== null && at !== undefined; at = parentOf(at)) {
                    if (at === watched) {
                        reached = true;
                        break;
                    }
                }
            }
            if (!reached) {
                continue;
            }
            if (type === 'childList' && !options.childList) {
                continue;
            }
            if (type === 'attributes' && !(options.attributes || options.attributeOldValue || options.attributeFilter)) {
                continue;
            }
            if (type === 'attributes' && options.attributeFilter && !options.attributeFilter.includes(extra.attributeName)) {
                continue;
            }
            if (type === 'characterData' && !(options.characterData || options.characterDataOldValue)) {
                continue;
            }
            const keepOld = (type === 'attributes' && options.attributeOldValue) || (type === 'characterData' && options.characterDataOldValue);
            observer.__records.push({
                type,
                target,
                addedNodes: extra.addedNodes ?? [],
                removedNodes: extra.removedNodes ?? [],
                previousSibling: extra.previousSibling ?? null,
                nextSibling: extra.nextSibling ?? null,
                attributeName: extra.attributeName ?? null,
                attributeNamespace: null,
                oldValue: keepOld ? extra.oldValue ?? null : null,
            });
            // What's been said of it isn't said again when what React draws is next looked at.
            options.__told = true;
            break;
        }
    }
    soonDelivered();
}

function deliver() {
    delivering = false;
    for (const observer of [...observers]) {
        if (observer.__records.length === 0) {
            continue;
        }
        const records = observer.__records;
        observer.__records = [];
        try {
            observer.__callback.call(observer, records, observer);
        } catch (error) {
            console.error(error);
        }
    }
}

class MutationObserver {
    constructor(callback) {
        if (typeof callback !== 'function') {
            throw new TypeError("Failed to construct 'MutationObserver': The callback provided as parameter 1 is not a function.");
        }
        hidden(this, '__callback', callback);
        hidden(this, '__targets', new Map());
        hidden(this, '__records', []);
    }
    observe(target, options = {}) {
        const said = { ...options };
        if (said.attributeOldValue || said.attributeFilter) {
            said.attributes ??= true;
        }
        if (said.characterDataOldValue) {
            said.characterData ??= true;
        }
        if (!said.childList && !said.attributes && !said.characterData) {
            throw new TypeError("Failed to execute 'observe' on 'MutationObserver': The options object must set at least one of 'attributes', 'characterData', or 'childList' to true.");
        }
        this.__targets.set(target, said);
        observers.add(this);
        if (!isOwn(target)) {
            said.__seen = reactSeen(target, said);
            reactWatch ??= require('rsui/dom-observers').watchers(web.onPageRendered, web.resizeListeners);
            reactWatch.add(this);
        }
    }
    disconnect() {
        this.__targets.clear();
        this.__records = [];
        observers.delete(this);
        reactWatch?.remove(this);
    }
    /// Whether looking at what React draws that's watched could find anything: not when no attribute is watched and React draws
    /// nothing in any of it, now or when it was last looked at, which leaves what code does, which is said as it's done.
    __wantsLook() {
        for (const [target, options] of this.__targets) {
            if (options.__seen !== undefined && (options.__seen.attributes !== '' || options.__seen.inside !== '' || reactDrawsIn(target))) {
                return true;
            }
        }
        return false;
    }
    /// Looks at what React draws that's watched, after the page has rendered.
    __look() {
        for (const [target, options] of this.__targets) {
            if (options.__seen === undefined) {
                continue;
            }
            const now = reactSeen(target, options);
            if (options.__told === true) {
                options.__told = false;
            } else {
                const made = (type) => ({ type, target, addedNodes: [], removedNodes: [], previousSibling: null, nextSibling: null, attributeName: null, attributeNamespace: null, oldValue: null });
                if (now.inside !== options.__seen.inside) {
                    this.__records.push(made('childList'));
                }
                if (now.attributes !== options.__seen.attributes) {
                    this.__records.push(made('attributes'));
                }
            }
            options.__seen = now;
        }
        if (this.__records.length > 0) {
            soonDelivered();
        }
    }
    takeRecords() {
        const records = this.__records;
        this.__records = [];
        return records;
    }
}

// Ranges, which move with what they're around as it changes, as live ranges do.

const liveRanges = new Set();

function nodeLength(node) {
    if (node.nodeType === TEXT || node.nodeType === COMMENT) {
        return node.__data.length;
    }
    return childList(node).length;
}

/// Every child of a node, in order: what React drew, then what code gave it, for a page's element, or its own for one made here.
function childList(node) {
    if (isOwn(node)) {
        return node.__kids ?? [];
    }
    if (node === web.document) {
        return [web.document.documentElement];
    }
    const drawn = typeof node.__drawnChildren === 'function' ? node.__drawnChildren() : [];
    const own = node.__kids ?? [];
    return drawn.length === 0 ? own : [...drawn, ...own];
}

/// Where `child` is in `kids`, the children code gave what it's in, or -1. Each child keeps where it was last found, which is
/// looked at first, then the places either side, as one put in or taken out before it moves it by one. Otherwise every child's
/// place is found again at once. So walking along thousands of children is a look at each, rather than a search for each.
function ownIndex(kids, child) {
    const hint = child.__at;
    if (kids[hint] === child) {
        return hint;
    }
    if (kids[hint + 1] === child) {
        child.__at = hint + 1;
        return hint + 1;
    }
    if (kids[hint - 1] === child) {
        child.__at = hint - 1;
        return hint - 1;
    }
    let found = -1;
    for (let at = 0; at < kids.length; at += 1) {
        const one = kids[at];
        one.__at = at;
        if (one === child) {
            found = at;
        }
    }
    return found;
}

/// Where a node code put in what it's in is among that one's own children, or -1 for anything else, as what React drew is.
function ownPlace(parent, child) {
    const kids = parent.__kids;
    if (kids === undefined || child.__parent !== parent || child.__drawn === true || child.__own !== true) {
        return -1;
    }
    return ownIndex(kids, child);
}

/// How many children what React drew in a page's element has before what code gave it.
function drawnCount(parent) {
    return isOwn(parent) || typeof parent.__drawnChildren !== 'function' ? 0 : parent.__drawnChildren().length;
}

function indexIn(node) {
    const parent = parentOf(node);
    if (parent === null || parent === undefined) {
        return -1;
    }
    const own = ownPlace(parent, node);
    if (own >= 0) {
        return isOwn(parent) ? own : drawnCount(parent) + own;
    }
    const kids = childList(parent);
    const at = kids.indexOf(node);
    return at >= 0 || node.__domKey === undefined ? at : kids.findIndex((one) => sameNode(one, node));
}

/// The same node, as far as that can be told: a page's element can be found more than once as a different object, but by the
/// same key.
const sameNode = (one, other) => one === other || (one?.__domKey !== undefined && one.__domKey === other?.__domKey);
const identity = (node) => node?.__domKey ?? node;

/// A node and what it's in, up to the top.
function ancestry(node) {
    const out = [];
    for (let at = node; at !== null && at !== undefined; at = parentOf(at)) {
        out.push(at);
    }
    return out;
}

/// Where `a` is against `b`, as boundary points: -1 before, 0 the same, 1 after.
function comparePoints(nodeA, offsetA, nodeB, offsetB) {
    if (sameNode(nodeA, nodeB)) {
        return Math.sign(offsetA - offsetB);
    }
    const a = ancestry(nodeA);
    const b = ancestry(nodeB);
    const inB = new Map(b.map((node, at) => [identity(node), at]));
    for (let i = 0; i < a.length; i += 1) {
        const j = inB.get(identity(a[i]));
        if (j === undefined) {
            continue;
        }
        if (i === 0) {
            // A contains B: A's offset against where what B's in is in A.
            return offsetA <= indexIn(b[j - 1]) ? -1 : 1;
        }
        if (j === 0) {
            return indexIn(a[i - 1]) < offsetB ? -1 : 1;
        }
        return Math.sign(indexIn(a[i - 1]) - indexIn(b[j - 1]));
    }
    return 0;
}

class StaticRange {
    constructor(init) {
        this.startContainer = init.startContainer;
        this.startOffset = init.startOffset;
        this.endContainer = init.endContainer;
        this.endOffset = init.endOffset;
    }
    get collapsed() {
        return this.startContainer === this.endContainer && this.startOffset === this.endOffset;
    }
}

class Range {
    constructor() {
        const root = web.document;
        this.startContainer = root;
        this.startOffset = 0;
        this.endContainer = root;
        this.endOffset = 0;
        liveRanges.add(new WeakRef(this));
    }
    get collapsed() {
        return this.startContainer === this.endContainer && this.startOffset === this.endOffset;
    }
    get commonAncestorContainer() {
        const ancestors = new Set(ancestry(this.startContainer).map(identity));
        for (let at = this.endContainer; at !== null && at !== undefined; at = parentOf(at)) {
            if (ancestors.has(identity(at))) {
                return at;
            }
        }
        return this.startContainer;
    }
    setStart(node, offset) {
        checkPoint(node, offset);
        this.startContainer = node;
        this.startOffset = offset;
        if (comparePoints(node, offset, this.endContainer, this.endOffset) > 0) {
            this.endContainer = node;
            this.endOffset = offset;
        }
        selectionTouched(this);
    }
    setEnd(node, offset) {
        checkPoint(node, offset);
        this.endContainer = node;
        this.endOffset = offset;
        if (comparePoints(node, offset, this.startContainer, this.startOffset) < 0) {
            this.startContainer = node;
            this.startOffset = offset;
        }
        selectionTouched(this);
    }
    setStartBefore(node) {
        this.setStart(parentOf(node), indexIn(node));
    }
    setStartAfter(node) {
        this.setStart(parentOf(node), indexIn(node) + 1);
    }
    setEndBefore(node) {
        this.setEnd(parentOf(node), indexIn(node));
    }
    setEndAfter(node) {
        this.setEnd(parentOf(node), indexIn(node) + 1);
    }
    collapse(toStart = false) {
        if (toStart) {
            this.endContainer = this.startContainer;
            this.endOffset = this.startOffset;
        } else {
            this.startContainer = this.endContainer;
            this.startOffset = this.endOffset;
        }
        selectionTouched(this);
    }
    selectNode(node) {
        const parent = parentOf(node);
        const at = indexIn(node);
        this.startContainer = parent;
        this.startOffset = at;
        this.endContainer = parent;
        this.endOffset = at + 1;
        selectionTouched(this);
    }
    selectNodeContents(node) {
        this.startContainer = node;
        this.startOffset = 0;
        this.endContainer = node;
        this.endOffset = nodeLength(node);
        selectionTouched(this);
    }
    cloneRange() {
        const out = new Range();
        out.startContainer = this.startContainer;
        out.startOffset = this.startOffset;
        out.endContainer = this.endContainer;
        out.endOffset = this.endOffset;
        return out;
    }
    compareBoundaryPoints(how, other) {
        const [a, b] = [
            [[this.startContainer, this.startOffset], [other.startContainer, other.startOffset]],
            [[this.endContainer, this.endOffset], [other.startContainer, other.startOffset]],
            [[this.endContainer, this.endOffset], [other.endContainer, other.endOffset]],
            [[this.startContainer, this.startOffset], [other.endContainer, other.endOffset]],
        ][how];
        return comparePoints(a[0], a[1], b[0], b[1]);
    }
    comparePoint(node, offset) {
        if (comparePoints(node, offset, this.startContainer, this.startOffset) < 0) {
            return -1;
        }
        if (comparePoints(node, offset, this.endContainer, this.endOffset) > 0) {
            return 1;
        }
        return 0;
    }
    isPointInRange(node, offset) {
        return this.comparePoint(node, offset) === 0;
    }
    intersectsNode(node) {
        const parent = parentOf(node);
        if (parent === null) {
            return true;
        }
        const at = indexIn(node);
        return comparePoints(parent, at, this.endContainer, this.endOffset) < 0 && comparePoints(parent, at + 1, this.startContainer, this.startOffset) > 0;
    }
    toString() {
        let out = '';
        walkRange(this, (node, from, to) => {
            if (node.nodeType === TEXT) {
                out += node.__data.slice(from, to);
            }
        });
        return out;
    }
    deleteContents() {
        deleteRange(this);
    }
    extractContents() {
        const fragment = this.cloneContents();
        deleteRange(this);
        return fragment;
    }
    cloneContents() {
        const fragment = web.document.createDocumentFragment();
        const common = this.commonAncestorContainer;
        if (this.startContainer === this.endContainer && (this.startContainer.nodeType === TEXT || this.startContainer.nodeType === COMMENT)) {
            fragment.appendChild(web.document.createTextNode(this.startContainer.__data.slice(this.startOffset, this.endOffset)));
            return fragment;
        }
        const range = this;
        const top = common.nodeType === TEXT ? parentOf(common) : common;
        const reach = reachOf(range, top);
        // What's wholly in the range is copied whole, without asking where each of what's in it is.
        const copy = (node, into, whole = false) => {
            const kids = childList(node);
            const [first, last] = node === top && reach !== null ? [Math.max(0, reach.lo), Math.min(kids.length - 1, reach.hi)] : [0, kids.length - 1];
            for (let index = first; index <= last; index += 1) {
                const child = kids[index];
                const inside = whole || (node === top && reach !== null && index >= reach.wlo && index <= reach.whi);
                if (!inside && !range.intersectsNode(child)) {
                    continue;
                }
                if (inside && child.nodeType !== TEXT) {
                    const shallow = child.cloneNode(false);
                    into.appendChild(shallow);
                    copy(child, shallow, true);
                    continue;
                }
                if (child.nodeType === TEXT) {
                    const from = child === range.startContainer ? range.startOffset : 0;
                    const to = child === range.endContainer ? range.endOffset : child.__data.length;
                    into.appendChild(web.document.createTextNode(child.__data.slice(from, to)));
                } else {
                    const shallow = child.cloneNode(false);
                    into.appendChild(shallow);
                    copy(child, shallow);
                }
            }
        };
        copy(top, fragment);
        return fragment;
    }
    insertNode(node) {
        let container = this.startContainer;
        let offset = this.startOffset;
        if (container.nodeType === TEXT) {
            const after = container.splitText(offset);
            container = parentOf(after);
            offset = indexIn(after);
        }
        const before = childList(container)[offset] ?? null;
        container.insertBefore(node, before);
        if (this.collapsed) {
            this.setEnd(container, offset + (node.nodeType === FRAGMENT ? 0 : 1));
        }
    }
    surroundContents(parent) {
        const contents = this.extractContents();
        this.insertNode(parent);
        parent.appendChild(contents);
        this.selectNode(parent);
    }
    getBoundingClientRect() {
        const rects = this.getClientRects();
        if (rects.length === 0) {
            return rect(0, 0, 0, 0);
        }
        let [x0, y0, x1, y1] = [Infinity, Infinity, -Infinity, -Infinity];
        for (const one of rects) {
            x0 = Math.min(x0, one.left);
            y0 = Math.min(y0, one.top);
            x1 = Math.max(x1, one.right);
            y1 = Math.max(y1, one.bottom);
        }
        return rect(x0, y0, x1 - x0, y1 - y0);
    }
    getClientRects() {
        return web.editingGeometry?.rangeRects(this) ?? [];
    }
    detach() {}
}
Object.assign(Range, { START_TO_START: 0, START_TO_END: 1, END_TO_END: 2, END_TO_START: 3 });
Object.assign(Range.prototype, { START_TO_START: 0, START_TO_END: 1, END_TO_END: 2, END_TO_START: 3 });

function rect(x, y, width, height) {
    return { x, y, left: x, top: y, right: x + width, bottom: y + height, width, height, toJSON: () => ({ x, y, width, height, top: y, right: x + width, bottom: y + height, left: x }) };
}

function checkPoint(node, offset) {
    if (node === null || node === undefined || typeof node !== 'object') {
        throw new TypeError("Failed to execute on 'Range': parameter 1 is not of type 'Node'.");
    }
    if (offset > nodeLength(node) || offset < 0) {
        const error = new Error(`The offset ${offset} is larger than the node's length (${nodeLength(node)}).`);
        error.name = 'IndexSizeError';
        throw error;
    }
}

/// Which of the children of `top`, which both of a range's ends are in, the range reaches: those from `lo` to `hi` at least
/// partly, and those from `wlo` to `whi` wholly, worked out from where its ends are, as `comparePoints` would have it, rather than
/// by comparing each child with them, which took most of a second for a selection of thousands of paragraphs. It's null when an
/// end can't be found in it, which leaves each to be asked about.
function reachOf(range, top) {
    const where = (container) => {
        if (sameNode(container, top)) {
            return { between: true, index: 0 };
        }
        for (let at = container; at !== null && at !== undefined; at = parentOf(at)) {
            const parent = parentOf(at);
            if (parent === null || parent === undefined) {
                return null;
            }
            if (sameNode(parent, top)) {
                const index = indexIn(at);
                return index < 0 ? null : { between: false, index };
            }
        }
        return null;
    };
    const start = where(range.startContainer);
    const end = where(range.endContainer);
    if (start === null || end === null) {
        return null;
    }
    return {
        lo: start.between ? range.startOffset : start.index,
        hi: end.between ? range.endOffset - 1 : end.index,
        wlo: start.between ? range.startOffset : start.index + 1,
        whi: end.between ? range.endOffset - 1 : end.index - 1,
    };
}

/// Calls `each(node, from, to)` for each text and element the range has in it, in order, with what part of a text it covers.
function walkRange(range, each) {
    const { startContainer: sc, startOffset: so, endContainer: ec, endOffset: eo } = range;
    if (sc === ec && (sc.nodeType === TEXT || sc.nodeType === COMMENT)) {
        each(sc, so, eo);
        return;
    }
    const visit = (node) => {
        if (node.nodeType === TEXT) {
            const from = node === sc ? so : 0;
            const to = node === ec ? eo : node.__data.length;
            each(node, from, to);
            return;
        }
        each(node, 0, 0);
        for (const child of childList(node)) {
            if (range.intersectsNode(child) || child === sc || child === ec) {
                visit(child);
            }
        }
    };
    // What's wholly in it has all of what's in it in it too.
    const whole = (node) => {
        if (node.nodeType === TEXT) {
            each(node, 0, node.__data.length);
            return;
        }
        each(node, 0, 0);
        for (const child of childList(node)) {
            whole(child);
        }
    };
    const common = range.commonAncestorContainer;
    if (common.nodeType === TEXT) {
        visit(common);
        return;
    }
    const kids = childList(common);
    const reach = reachOf(range, common);
    if (reach === null) {
        for (const child of kids) {
            if (range.intersectsNode(child)) {
                visit(child);
            }
        }
        return;
    }
    for (let index = Math.max(0, reach.lo); index <= Math.min(kids.length - 1, reach.hi); index += 1) {
        const child = kids[index];
        if (index >= reach.wlo && index <= reach.whi) {
            whole(child);
        } else if (range.intersectsNode(child)) {
            visit(child);
        }
    }
}

/// Takes what a range has out of the document, as `deleteContents()` does, leaving it collapsed where it started.
function deleteRange(range) {
    if (range.collapsed) {
        return;
    }
    const { startContainer: sc, startOffset: so, endContainer: ec, endOffset: eo } = range;
    if (sc === ec && sc.nodeType === TEXT) {
        sc.deleteData(so, eo - so);
        range.setEnd(sc, so);
        return;
    }
    const remove = [];
    const common = range.commonAncestorContainer;
    const top = common.nodeType === TEXT ? parentOf(common) : common;
    const reach = reachOf(range, top);
    const visit = (node) => {
        const kids = [...childList(node)];
        const [first, last] = node === top && reach !== null ? [Math.max(0, reach.lo), Math.min(kids.length - 1, reach.hi)] : [0, kids.length - 1];
        for (let index = first; index <= last; index += 1) {
            const child = kids[index];
            if (node === top && reach !== null && index >= reach.wlo && index <= reach.whi) {
                remove.push(child);
                continue;
            }
            const parent = node;
            const at = indexIn(child);
            const entirely = comparePoints(parent, at, sc, so) >= 0 && comparePoints(parent, at + 1, ec, eo) <= 0;
            if (entirely) {
                remove.push(child);
            } else if (range.intersectsNode(child) && child.nodeType === ELEMENT) {
                visit(child);
            }
        }
    };
    visit(top);
    if (ec.nodeType === TEXT && ec !== sc) {
        ec.deleteData(0, eo);
    }
    if (sc.nodeType === TEXT && sc !== ec) {
        sc.deleteData(so, sc.__data.length - so);
    }
    for (const node of remove) {
        parentOf(node)?.removeChild(node);
    }
    range.setStart(sc, Math.min(so, nodeLength(sc)));
    range.collapse(true);
}

/// Keeps every live range where it was when what it's around changes, as the DOM spec says.
function rangesAfter(change) {
    for (const weak of liveRanges) {
        const range = weak.deref();
        if (range === undefined) {
            liveRanges.delete(weak);
            continue;
        }
        for (const end of ['start', 'end']) {
            const container = range[end + 'Container'];
            const offset = range[end + 'Offset'];
            const [nextContainer, nextOffset] = change(container, offset);
            range[end + 'Container'] = nextContainer;
            range[end + 'Offset'] = nextOffset;
        }
    }
}

// The document's selection: one range, with which way it was made.

let selectionRange = null;
let selectionBackward = false;
let selectionNotice = false;

function selectionTouched(range) {
    if (range === selectionRange) {
        noticeSelection();
    }
}

function noticeSelection() {
    web.editingGeometry?.selectionMoved?.();
    if (selectionNotice) {
        return;
    }
    selectionNotice = true;
    setTimeout(() => {
        selectionNotice = false;
        dispatch(web.document, makeEvent('selectionchange', { bubbles: false, cancelable: false }));
    }, 0);
}

class Selection {
    get rangeCount() {
        return selectionRange === null ? 0 : 1;
    }
    get anchorNode() {
        return selectionRange === null ? null : selectionBackward ? selectionRange.endContainer : selectionRange.startContainer;
    }
    get anchorOffset() {
        return selectionRange === null ? 0 : selectionBackward ? selectionRange.endOffset : selectionRange.startOffset;
    }
    get focusNode() {
        return selectionRange === null ? null : selectionBackward ? selectionRange.startContainer : selectionRange.endContainer;
    }
    get focusOffset() {
        return selectionRange === null ? 0 : selectionBackward ? selectionRange.startOffset : selectionRange.endOffset;
    }
    get isCollapsed() {
        return selectionRange === null || selectionRange.collapsed;
    }
    get type() {
        return selectionRange === null ? 'None' : selectionRange.collapsed ? 'Caret' : 'Range';
    }
    get direction() {
        return selectionRange === null || selectionRange.collapsed ? 'none' : selectionBackward ? 'backward' : 'forward';
    }
    getRangeAt(index) {
        if (index !== 0 || selectionRange === null) {
            const error = new Error(`The index provided (${index}) is greater than or equal to the maximum bound (${this.rangeCount}).`);
            error.name = 'IndexSizeError';
            throw error;
        }
        return selectionRange;
    }
    addRange(range) {
        if (selectionRange === null) {
            selectionRange = range;
            selectionBackward = false;
            noticeSelection();
        }
    }
    removeRange(range) {
        if (range === selectionRange) {
            this.removeAllRanges();
        }
    }
    removeAllRanges() {
        if (selectionRange !== null) {
            selectionRange = null;
            selectionBackward = false;
            noticeSelection();
        }
    }
    empty() {
        this.removeAllRanges();
    }
    collapse(node, offset = 0) {
        if (node === null || node === undefined) {
            this.removeAllRanges();
            return;
        }
        const range = new Range();
        range.setStart(node, offset);
        range.collapse(true);
        selectionRange = range;
        selectionBackward = false;
        noticeSelection();
    }
    setPosition(node, offset = 0) {
        this.collapse(node, offset);
    }
    collapseToStart() {
        if (selectionRange !== null) {
            this.collapse(selectionRange.startContainer, selectionRange.startOffset);
        }
    }
    collapseToEnd() {
        if (selectionRange !== null) {
            this.collapse(selectionRange.endContainer, selectionRange.endOffset);
        }
    }
    extend(node, offset = 0) {
        if (selectionRange === null) {
            this.collapse(node, offset);
            return;
        }
        const anchor = [this.anchorNode, this.anchorOffset];
        this.setBaseAndExtent(anchor[0], anchor[1], node, offset);
    }
    setBaseAndExtent(anchorNode, anchorOffset, focusNode, focusOffset) {
        const range = new Range();
        const backward = comparePoints(anchorNode, anchorOffset, focusNode, focusOffset) > 0;
        if (backward) {
            range.setStart(focusNode, focusOffset);
            range.setEnd(anchorNode, anchorOffset);
        } else {
            range.setStart(anchorNode, anchorOffset);
            range.setEnd(focusNode, focusOffset);
        }
        selectionRange = range;
        selectionBackward = backward;
        noticeSelection();
    }
    selectAllChildren(node) {
        this.setBaseAndExtent(node, 0, node, nodeLength(node));
    }
    deleteFromDocument() {
        if (selectionRange !== null) {
            selectionRange.deleteContents();
            noticeSelection();
        }
    }
    containsNode(node, partly = false) {
        if (selectionRange === null) {
            return false;
        }
        if (partly) {
            return selectionRange.intersectsNode(node);
        }
        const parent = parentOf(node);
        const at = indexIn(node);
        return comparePoints(parent, at, selectionRange.startContainer, selectionRange.startOffset) >= 0
            && comparePoints(parent, at + 1, selectionRange.endContainer, selectionRange.endOffset) <= 0;
    }
    modify(alter, direction, granularity) {
        web.editingGeometry?.modify?.(this, String(alter), String(direction), String(granularity));
    }
    getComposedRanges() {
        return selectionRange === null ? [] : [new StaticRange(selectionRange)];
    }
    toString() {
        return selectionRange === null ? '' : selectionText(selectionRange);
    }
}

/// How many line breaks each block puts around itself in what's selected as text, as Chromium's selection says it: a paragraph a
/// blank line, the rest one.
const SELECTION_BREAKS = new Set(['div', 'li', 'ul', 'ol', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'blockquote', 'pre', 'section', 'article',
    'header', 'footer', 'tr', 'table', 'figure', 'figcaption', 'dd', 'dt', 'dl', 'hr', 'main', 'nav', 'aside', 'address', 'form', 'fieldset']);

/// What's selected as text, as `getSelection().toString()` says it: the text in it, with line breaks between blocks (two around a
/// paragraph, collapsing where blocks meet), and nothing of what's `user-select: none`, which is a line of its own.
function selectionText(range) {
    const { startContainer: sc, startOffset: so, endContainer: ec, endOffset: eo } = range;
    if (sc === ec && sc.nodeType === TEXT) {
        return sc.__data.slice(so, eo);
    }
    let out = '';
    let pending = 0;
    let started = false;
    const flush = () => {
        if (started && pending > 0) {
            out += '\n'.repeat(pending);
        }
        pending = 0;
    };
    const visit = (node) => {
        if (node.nodeType === TEXT) {
            const text = node.__data.slice(node === sc ? so : 0, node === ec ? eo : node.__data.length);
            if (text !== '' || node === ec) {
                flush();
            }
            if (text !== '') {
                out += text;
                started = true;
            }
            return;
        }
        if (node.nodeType !== ELEMENT && node.nodeType !== DOCUMENT && node.nodeType !== FRAGMENT) {
            return;
        }
        const tag = String(node.localName ?? '');
        if (tag === 'br') {
            flush();
            out += '\n';
            started = true;
            return;
        }
        const breaks = tag === 'p' ? 2 : SELECTION_BREAKS.has(tag) ? 1 : 0;
        if (started) {
            pending = Math.max(pending, breaks);
        }
        if (node.__live?.look?.__userSelect === 'none') {
            if (breaks > 0) {
                flush();
                pending = started ? 1 : 0;
            }
            return;
        }
        for (const child of childList(node)) {
            if (range.intersectsNode(child) || child === sc || child === ec) {
                visit(child);
            }
        }
        if (started) {
            pending = Math.max(pending, breaks);
        }
    };
    visit(range.commonAncestorContainer);
    return out;
}

const selection = new Selection();

// Events, which are the web's classes (`rsui/dom-events`), and go along the DOM from the window down to what they're for and back
// up, as any of the page's events do: see `dispatchOn` in `web.js`.

/// The web's event classes.
const events = require('rsui/dom-events');

function makeEvent(type, init = {}, Kind = null) {
    const Made = Kind ?? globalThis.Event;
    return new Made(type, init);
}

/// The way from the top down to `target`: its DOM parents, and for a page's element, what it's drawn in.
function pathTo(target) {
    const path = [];
    for (let at = target; at !== null && at !== undefined && at !== web.document; at = parentOf(at)) {
        path.unshift(at);
        if (path.length > 512) {
            break;
        }
    }
    return path;
}

/// Sends an event made by code, or by editing, to what it's for, with what's listening on the way down and back up hearing it.
function dispatch(target, event) {
    return web.dispatch(target, event);
}

// Nodes.

const VOID = new Set(['area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'source', 'track', 'wbr']);
const SVG_NS = 'http://www.w3.org/2000/svg';

/// What each kind of node has, on top of what a page's element has.
const nodeMethods = {
    get childNodes() {
        return nodeListOf(childList(this));
    },
    get firstChild() {
        return childList(this)[0] ?? null;
    },
    get lastChild() {
        return childList(this).at(-1) ?? null;
    },
    get nextSibling() {
        const parent = parentOf(this);
        if (parent === null) {
            return null;
        }
        const own = ownPlace(parent, this);
        if (own >= 0) {
            // What code gave it comes after what React drew in it.
            return parent.__kids[own + 1] ?? null;
        }
        const at = indexIn(this);
        return at < 0 ? null : childList(parent)[at + 1] ?? null;
    },
    get previousSibling() {
        const parent = parentOf(this);
        if (parent === null) {
            return null;
        }
        const own = ownPlace(parent, this);
        if (own > 0 || (own === 0 && isOwn(parent))) {
            return own > 0 ? parent.__kids[own - 1] : null;
        }
        const at = indexIn(this);
        return at > 0 ? childList(parent)[at - 1] : null;
    },
    hasChildNodes() {
        return childList(this).length > 0;
    },
    appendChild(child) {
        return insert(this, child, null);
    },
    insertBefore(child, before) {
        return insert(this, child, before ?? null);
    },
    removeChild(child) {
        if (child === null || child === undefined || parentOf(child) !== this || ownPlace(this, child) < 0) {
            // What React drew isn't code's to take away, and is left as it is, as is what isn't here.
            if (child !== null && child !== undefined && parentOf(child) === this) {
                return child;
            }
            const error = new Error("Failed to execute 'removeChild' on 'Node': The node to be removed is not a child of this node.");
            error.name = 'NotFoundError';
            throw error;
        }
        take(child);
        return child;
    },
    replaceChild(child, old) {
        insert(this, child, old);
        if (child !== old) {
            this.removeChild(old);
        }
        return old;
    },
    contains(other) {
        const own = isOwn(this);
        for (let at = other; at !== null && at !== undefined; at = parentOf(at)) {
            if (sameNode(at, this)) {
                return true;
            }
            // A node code made has only what code put in it in it, so once what's looked at isn't one of those, nothing further up
            // is this. Going on up would ask where each of the page's elements above is.
            if (own && !isOwn(at)) {
                return false;
            }
        }
        return false;
    },
    getRootNode() {
        let at = this;
        while (parentOf(at) !== null && parentOf(at) !== undefined) {
            at = parentOf(at);
        }
        return isConnected(at) ? web.document : at;
    },
    compareDocumentPosition(other) {
        if (other === this) {
            return 0;
        }
        if (this.contains(other)) {
            return 16 | 4;
        }
        if (other.contains?.(this)) {
            return 8 | 2;
        }
        const order = comparePoints(parentOf(this) ?? this, Math.max(0, indexIn(this)), parentOf(other) ?? other, Math.max(0, indexIn(other)));
        return order < 0 ? 4 : order > 0 ? 2 : 1 | 32;
    },
    isSameNode(other) {
        return other === this;
    },
    normalize() {
        const list = childList(this);
        for (let i = 0; i < list.length; i += 1) {
            const node = list[i];
            if (node.nodeType === TEXT) {
                while (list[i + 1]?.nodeType === TEXT) {
                    const next = list[i + 1];
                    node.appendData(next.__data);
                    take(next);
                }
                if (node.__data === '') {
                    take(node);
                    i -= 1;
                }
            } else {
                node.normalize?.();
            }
        }
    },
};

/// A list of nodes, as `childNodes` gives it.
function nodeListOf(list) {
    const out = list.slice();
    out.item = (index) => out[index] ?? null;
    return out;
}

function isConnected(node) {
    for (let at = node; at !== null && at !== undefined; at = parentOf(at)) {
        if (!isOwn(at) && at.nodeType !== FRAGMENT) {
            // A page's element is on the page.
            return at.__domKey !== undefined || at === web.document || at === web.document.body || at === web.document.documentElement;
        }
    }
    return false;
}

/// Puts `child` in `parent` before `before`, or at the end, taking it out of where it was first, as `insertBefore()` does.
/// How many times what's in what has changed, for what keeps something it worked out from that (see `blocksIn` in editing.js).
let structure = 0;
/// And what the last of those changes were, each as the node something was put in or taken out of and that something, from when
/// `structure` was `changesFrom`: what's kept can look at what changed since, rather than work everything out again, as typing
/// into one paragraph of 10,000 changes what's in that one alone. Only the last few are kept.
const CHANGES_KEPT = 64;
let changes = [];
let changesFrom = 0;

function changedStructure(parent, child) {
    if (changes.length === CHANGES_KEPT * 2) {
        changes = [];
        changesFrom = structure;
    }
    structure += 1;
    changes.push(parent ?? null, child);
}

/// What's been put in or taken out of what since `structure` was `at`, as a flat list of what it was in and what it was, or
/// null when that's further back than what's kept.
function changesSince(at) {
    if (at < changesFrom || at > structure) {
        return null;
    }
    return changes.slice((at - changesFrom) * 2);
}

function insert(parent, child, before) {
    changedStructure(parent, child);
    if (child === null || typeof child !== 'object') {
        throw new TypeError("Failed to execute 'insertBefore' on 'Node': parameter 1 is not of type 'Node'.");
    }
    if (child.nodeType === FRAGMENT) {
        for (const one of [...child.__kids]) {
            insert(parent, one, before);
        }
        return child;
    }
    if (child === parent || child.contains?.(parent)) {
        const error = new Error("Failed to execute 'insertBefore' on 'Node': The new child element contains the parent.");
        error.name = 'HierarchyRequestError';
        throw error;
    }
    if (!isOwn(child)) {
        // A page's element drawn by React, moved among what React drew in what it's in, is drawn where it's put, as on a page,
        // until React changes what's in that (see `moveDrawn` in web.js), as cmdk sorts its items. Anywhere else, it stays
        // where React put it.
        const previous = child.previousSibling ?? null;
        const next = child.nextSibling ?? null;
        if (!isOwn(parent) && web.moveDrawn?.(parent, child, before) === true && next !== before && !sameNode(next, before)) {
            changedStructure(parent, child);
            record('childList', parent, { removedNodes: [child], previousSibling: previous, nextSibling: next });
            record('childList', parent, { addedNodes: [child], previousSibling: before === null ? parent.lastChild : before.previousSibling ?? null, nextSibling: before });
        }
        return child;
    }
    if (parentOf(child) !== null) {
        take(child);
    }
    const kids = kidsOf(parent);
    let at = before === null ? kids.length : ownPlace(parent, before);
    if (at < 0) {
        if (childList(parent).includes(before)) {
            // Before something React drew: the first of code's own.
            at = 0;
        } else {
            const error = new Error("Failed to execute 'insertBefore' on 'Node': The node before which the new node is to be inserted is not a child of this node.");
            error.name = 'NotFoundError';
            throw error;
        }
    }
    const previous = at > 0 ? kids[at - 1] : null;
    const next = kids[at] ?? null;
    if (kids.length === 0) {
        // Just as long as what's in it: see `NO_KIDS`.
        parent.__kids = [child];
    } else {
        kids.splice(at, 0, child);
    }
    child.__parent = parent;
    child.__at = at;
    const index = drawnCount(parent) + at;
    rangesAfter((container, offset) => (container === parent && offset > index ? [container, offset + 1] : [container, offset]));
    record('childList', parent, { addedNodes: [child], previousSibling: previous, nextSibling: next });
    changed(parent);
    web.editingGeometry?.inserted?.(child);
    return child;
}

/// Takes a node out of what it's in.
function take(child) {
    const parent = child.__parent;
    changedStructure(parent, child);
    if (parent === null || parent === undefined) {
        return;
    }
    const kids = parent.__kids;
    const at = ownPlace(parent, child);
    if (at < 0) {
        return;
    }
    const index = drawnCount(parent) + at;
    const previous = at > 0 ? kids[at - 1] : null;
    const next = kids[at + 1] ?? null;
    kids.splice(at, 1);
    child.__parent = null;
    // What was in it, or at it, is where it was.
    rangesAfter((container, offset) => {
        if (child === container || child.contains(container)) {
            return [parent, index];
        }
        if (container === parent && offset > index) {
            return [container, offset - 1];
        }
        return [container, offset];
    });
    record('childList', parent, { removedNodes: [child], previousSibling: previous, nextSibling: next });
    changed(parent);
}

/// What a page's element and one made here have, as elements.
const elementMethods = {
    get children() {
        return nodeListOf(childList(this).filter((one) => one.nodeType === ELEMENT));
    },
    get childElementCount() {
        return this.children.length;
    },
    get firstElementChild() {
        return this.children[0] ?? null;
    },
    get lastElementChild() {
        return this.children.at(-1) ?? null;
    },
    get nextElementSibling() {
        for (let at = this.nextSibling; at !== null; at = at.nextSibling) {
            if (at.nodeType === ELEMENT) {
                return at;
            }
        }
        return null;
    },
    get previousElementSibling() {
        for (let at = this.previousSibling; at !== null; at = at.previousSibling) {
            if (at.nodeType === ELEMENT) {
                return at;
            }
        }
        return null;
    },
    append(...nodes) {
        for (const one of nodes) {
            this.appendChild(typeof one === 'string' ? web.document.createTextNode(one) : one);
        }
    },
    prepend(...nodes) {
        const first = this.firstChild;
        for (const one of nodes) {
            this.insertBefore(typeof one === 'string' ? web.document.createTextNode(one) : one, first);
        }
    },
    replaceChildren(...nodes) {
        for (const one of [...(this.__kids ?? [])]) {
            take(one);
        }
        this.append(...nodes);
    },
    insertAdjacentElement(where, node) {
        return adjacent(this, where, node);
    },
    insertAdjacentText(where, text) {
        adjacent(this, where, web.document.createTextNode(String(text)));
    },
    insertAdjacentHTML(where, html) {
        const fragment = parseHTML(String(html));
        adjacent(this, where, fragment);
    },
    get textContent() {
        return textOf(this);
    },
    set textContent(value) {
        replaceDrawn(this);
        for (const one of [...(this.__kids ?? [])]) {
            take(one);
        }
        const text = value === null || value === undefined ? '' : String(value);
        if (text !== '') {
            insert(this, new TextNode(text), null);
        }
    },
    get innerHTML() {
        return childList(this).map(serialize).join('');
    },
    set innerHTML(value) {
        replaceDrawn(this);
        for (const one of [...(this.__kids ?? [])]) {
            take(one);
        }
        insert(this, parseHTML(String(value ?? '')), null);
    },
};

/// What React drew in a page's element goes, as code's said what's in it instead, as on a page, where it's taken out of the DOM
/// and React's changes to it come to nothing from then on.
function replaceDrawn(node) {
    if (!isOwn(node) && node.__replaced !== true) {
        hidden(node, '__replaced', true);
        // Drawn again with everything else that changed, once: drawing it again twice at once would come to nothing.
        changed(node);
    }
}

/// What's around a node and in it, as `insertAdjacentElement()` puts things.
function adjacent(node, where, child) {
    switch (String(where).toLowerCase()) {
        case 'beforebegin':
            parentOf(node)?.insertBefore(child, node);
            break;
        case 'afterbegin':
            node.insertBefore(child, node.firstChild);
            break;
        case 'beforeend':
            node.appendChild(child);
            break;
        case 'afterend':
            parentOf(node)?.insertBefore(child, node.nextSibling);
            break;
        default:
            throw new SyntaxError(`The value provided ('${where}') is not one of 'beforeBegin', 'afterBegin', 'beforeEnd', or 'afterEnd'.`);
    }
    return child;
}

/// What a `style` attribute says, given to the element's `style` without saying it again as the attribute.
function styleSaid(node, text) {
    hidden(node, '__styleQuiet', true);
    try {
        node.__style.cssText = text;
    } finally {
        node.__styleQuiet = false;
    }
}

function kebab(name) {
    const text = String(name);
    if (text.startsWith('--')) {
        return text;
    }
    return text.replace(/[A-Z]/g, (c) => '-' + c.toLowerCase()).replace(/^ms-/, '-ms-');
}

/// The classes on an element made here, as `classList` gives them.
function classListOf(node) {
    const list = () => (node.getAttribute('class') ?? '').split(/\s+/).filter((one) => one !== '');
    const write = (classes) => node.setAttribute('class', classes.join(' '));
    const tokens = {
        add: (...names) => {
            const now = list();
            const added = names.filter((name) => !now.includes(name));
            if (added.length > 0 || node.getAttribute('class') === null) {
                write([...now, ...added]);
            }
        },
        remove: (...names) => {
            const now = list();
            if (names.some((name) => now.includes(name))) {
                write(now.filter((one) => !names.includes(one)));
            }
        },
        toggle: (name, force) => {
            const has = list().includes(name);
            const on = force ?? !has;
            if (on && !has) tokens.add(name);
            if (!on && has) tokens.remove(name);
            return on;
        },
        replace: (old, next) => {
            const now = list();
            if (!now.includes(old)) return false;
            write(now.map((one) => (one === old ? next : one)));
            return true;
        },
        contains: (name) => list().includes(name),
        item: (index) => list()[index] ?? null,
        get length() {
            return list().length;
        },
        get value() {
            return node.getAttribute('class') ?? '';
        },
        toString: () => node.getAttribute('class') ?? '',
        forEach: (each) => list().forEach(each),
        [Symbol.iterator]: () => list()[Symbol.iterator](),
    };
    return tokens;
}

/// What an element made here has, on top of what every element has.
const ownElementProps = {
    get tagName() {
        return this.__ns === SVG_NS ? this.localName : this.localName.toUpperCase();
    },
    get nodeName() {
        return this.tagName;
    },
    get id() {
        return this.getAttribute('id') ?? '';
    },
    set id(value) {
        this.setAttribute('id', value);
    },
    get className() {
        return this.getAttribute('class') ?? '';
    },
    set className(value) {
        this.setAttribute('class', value);
    },
    get classList() {
        if (this.__classList === undefined) {
            hidden(this, '__classList', classListOf(this));
        }
        return this.__classList;
    },
    get style() {
        return this.__style;
    },
    set style(value) {
        this.__style.cssText = value;
    },
    get dataset() {
        const node = this;
        return new Proxy({}, {
            get: (_, name) => (typeof name === 'string' ? node.getAttribute('data-' + kebab(name)) ?? undefined : undefined),
            set: (_, name, value) => {
                node.setAttribute('data-' + kebab(name), value);
                return true;
            },
            deleteProperty: (_, name) => {
                node.removeAttribute('data-' + kebab(name));
                return true;
            },
            has: (_, name) => node.hasAttribute('data-' + kebab(name)),
            ownKeys: () => attrNames(node).filter((key) => key.startsWith('data-')).map((key) => key.slice(5).replace(/-([a-z])/g, (_, c) => c.toUpperCase())),
            getOwnPropertyDescriptor: () => ({ enumerable: true, configurable: true }),
        });
    },
    get attributes() {
        const list = attrEntries(this).map(([name, value]) => ({ name, value, localName: name, nodeName: name, namespaceURI: null, specified: true }));
        list.getNamedItem = (name) => list.find((one) => one.name === String(name).toLowerCase()) ?? null;
        list.item = (index) => list[index] ?? null;
        return list;
    },
    getAttribute(name) {
        const key = this.__ns === SVG_NS ? String(name) : String(name).toLowerCase();
        return attrOf(this, key) ?? null;
    },
    getAttributeNS(_space, name) {
        return this.getAttribute(name);
    },
    getAttributeNames() {
        return attrNames(this);
    },
    hasAttribute(name) {
        return this.getAttribute(name) !== null;
    },
    hasAttributes() {
        return this.__attrs.length > 0;
    },
    setAttribute(name, value) {
        const key = this.__ns === SVG_NS ? String(name) : String(name).toLowerCase();
        const text = String(value);
        const before = attrOf(this, key) ?? null;
        if (before === text) {
            return;
        }
        setAttr(this, key, text);
        if (key === 'style') {
            styleSaid(this, text);
        }
        record('attributes', this, { attributeName: key, oldValue: before });
        this.__attributeChanged(key);
    },
    setAttributeNS(_space, name, value) {
        this.setAttribute(String(name).replace(/^.*:/, ''), value);
    },
    removeAttribute(name) {
        const key = this.__ns === SVG_NS ? String(name) : String(name).toLowerCase();
        const before = attrOf(this, key);
        if (before === undefined) {
            return;
        }
        removeAttr(this, key);
        if (key === 'style') {
            styleSaid(this, '');
        }
        record('attributes', this, { attributeName: key, oldValue: before });
        this.__attributeChanged(key);
    },
    removeAttributeNS(_space, name) {
        this.removeAttribute(name);
    },
    toggleAttribute(name, force) {
        const on = force ?? !this.hasAttribute(name);
        if (on) {
            if (!this.hasAttribute(name)) this.setAttribute(name, '');
        } else {
            this.removeAttribute(name);
        }
        return on;
    },
    __attributeChanged(key) {
        if (key === 'contenteditable') {
            web.editingChanged?.(this);
        }
        this.__attributesChanged?.(key);
        changed(this);
    },
    __syncStyle() {
        const text = this.__style.cssText;
        const before = attrOf(this, 'style') ?? null;
        if (text === '') {
            removeAttr(this, 'style');
        } else {
            setAttr(this, 'style', text);
        }
        record('attributes', this, { attributeName: 'style', oldValue: before });
        changed(this);
    },
    get textContent() {
        return textOf(this);
    },
    set textContent(value) {
        for (const one of [...this.__kids]) {
            take(one);
        }
        const text = value === null || value === undefined ? '' : String(value);
        if (text !== '') {
            this.appendChild(web.document.createTextNode(text));
        }
    },
    get innerText() {
        return textOf(this);
    },
    set innerText(value) {
        this.textContent = value;
    },
    get innerHTML() {
        return this.__kids.map(serialize).join('');
    },
    set innerHTML(value) {
        for (const one of [...this.__kids]) {
            take(one);
        }
        this.appendChild(parseHTML(String(value ?? '')));
    },
    get outerHTML() {
        return serialize(this);
    },
    get isConnected() {
        return isConnected(this);
    },
    get parentNode() {
        return this.__parent;
    },
    get parentElement() {
        const parent = this.__parent;
        return parent !== null && parent !== undefined && parent.nodeType === ELEMENT ? parent : null;
    },
    get ownerDocument() {
        return web.document;
    },
    get contentEditable() {
        const said = this.getAttribute('contenteditable');
        return said === null ? 'inherit' : said === '' ? 'true' : said;
    },
    set contentEditable(value) {
        if (value === 'inherit') {
            this.removeAttribute('contenteditable');
        } else {
            this.setAttribute('contenteditable', String(value));
        }
    },
    get isContentEditable() {
        return web.editingGeometry?.editingHost?.(this) !== null && web.editingGeometry?.editingHost?.(this) !== undefined;
    },
    get tabIndex() {
        const said = this.getAttribute('tabindex');
        return said === null ? (this.getAttribute('contenteditable') !== null ? 0 : -1) : Number(said);
    },
    set tabIndex(value) {
        this.setAttribute('tabindex', String(value));
    },
    get spellcheck() {
        return this.getAttribute('spellcheck') !== 'false';
    },
    set spellcheck(value) {
        this.setAttribute('spellcheck', value ? 'true' : 'false');
    },
    get title() {
        return this.getAttribute('title') ?? '';
    },
    set title(value) {
        this.setAttribute('title', value);
    },
    get hidden() {
        return this.hasAttribute('hidden');
    },
    set hidden(value) {
        this.toggleAttribute('hidden', Boolean(value));
    },
    get href() {
        return this.getAttribute('href') ?? '';
    },
    set href(value) {
        this.setAttribute('href', value);
    },
    get src() {
        return this.getAttribute('src') ?? '';
    },
    set src(value) {
        this.setAttribute('src', value);
    },
    get draggable() {
        return this.getAttribute('draggable') === 'true';
    },
    set draggable(value) {
        this.setAttribute('draggable', value ? 'true' : 'false');
    },
    get dir() {
        return this.getAttribute('dir') ?? '';
    },
    set dir(value) {
        this.setAttribute('dir', value);
    },
    get lang() {
        return this.getAttribute('lang') ?? '';
    },
    set lang(value) {
        this.setAttribute('lang', value);
    },
    get namespaceURI() {
        return this.__ns;
    },
    cloneNode(deep = false) {
        const copy = this.__ns === SVG_NS ? web.document.createElementNS(SVG_NS, this.localName) : web.document.createElement(this.localName);
        for (const [name, value] of attrEntries(this)) {
            copy.setAttribute(name, value);
        }
        if (deep) {
            for (const one of this.__kids) {
                copy.appendChild(one.cloneNode(true));
            }
        }
        return copy;
    },
    isEqualNode(other) {
        return other !== null && other !== undefined && serialize(this) === serialize(other);
    },
    remove() {
        if (this.__parent !== null && this.__parent !== undefined) {
            take(this);
        }
    },
    before(...nodes) {
        const parent = this.__parent;
        for (const one of nodes) {
            parent?.insertBefore(typeof one === 'string' ? web.document.createTextNode(one) : one, this);
        }
    },
    after(...nodes) {
        const parent = this.__parent;
        let next = this.nextSibling;
        for (const one of nodes) {
            parent?.insertBefore(typeof one === 'string' ? web.document.createTextNode(one) : one, next);
        }
    },
    replaceWith(...nodes) {
        const parent = this.__parent;
        if (parent === null || parent === undefined) {
            return;
        }
        const next = this.nextSibling;
        take(this);
        for (const one of nodes) {
            parent.insertBefore(typeof one === 'string' ? web.document.createTextNode(one) : one, next);
        }
    },
    matches(selector) {
        return matchesSelector(this, String(selector));
    },
    webkitMatchesSelector(selector) {
        return this.matches(selector);
    },
    closest(selector) {
        for (let at = this; at !== null && at !== undefined; at = parentOf(at)) {
            if (at.nodeType === ELEMENT && at.matches?.(selector)) {
                return at;
            }
        }
        return null;
    },
    querySelector(selector) {
        return querySelectorIn(this, String(selector), true)[0] ?? null;
    },
    querySelectorAll(selector) {
        return nodeListOf(querySelectorIn(this, String(selector), false));
    },
    getElementsByTagName(name) {
        return nodeListOf(querySelectorIn(this, String(name), false));
    },
    getElementsByClassName(names) {
        return nodeListOf(querySelectorIn(this, String(names).trim().split(/\s+/).map((one) => '.' + one).join(''), false));
    },
    addEventListener(type, listener, options) {
        web.listenOnElement(this, String(type), listener, options);
    },
    removeEventListener(type, listener, options) {
        web.unlistenOnElement(this, String(type), listener, options);
    },
    dispatchEvent(event) {
        return dispatch(this, event);
    },
    focus(options) {
        web.focusNode(this, options);
    },
    blur() {
        web.blurNode(this);
    },
    click() {
        dispatch(this, new events.MouseEvent('click', { bubbles: true, cancelable: true, composed: true, view: globalThis }));
    },
    getBoundingClientRect() {
        return web.rectOfOwn(this) ?? rect(0, 0, 0, 0);
    },
    getClientRects() {
        const one = this.getBoundingClientRect();
        return one.width === 0 && one.height === 0 ? [] : [one];
    },
    get offsetWidth() {
        return this.getBoundingClientRect().width;
    },
    get offsetHeight() {
        return this.getBoundingClientRect().height;
    },
    get clientWidth() {
        return this.getBoundingClientRect().width;
    },
    get clientHeight() {
        return this.getBoundingClientRect().height;
    },
    get offsetLeft() {
        return this.getBoundingClientRect().x;
    },
    get offsetTop() {
        return this.getBoundingClientRect().y;
    },
    get offsetParent() {
        return this.parentElement;
    },
    get scrollTop() {
        return 0;
    },
    set scrollTop(_) {},
    get scrollLeft() {
        return 0;
    },
    set scrollLeft(_) {},
    get scrollHeight() {
        return this.getBoundingClientRect().height;
    },
    get scrollWidth() {
        return this.getBoundingClientRect().width;
    },
    scrollIntoView() {},
    scrollTo() {},
    scrollBy() {},
    getAnimations: () => [],
    animate: () => ({ cancel() {}, finish() {}, finished: Promise.resolve(), onfinish: null }),
    attachShadow() {
        throw new Error('Shadow roots are not supported here.');
    },
    get shadowRoot() {
        return null;
    },
    get assignedSlot() {
        return null;
    },
};

function textOf(node) {
    if (node.nodeType === TEXT) {
        return node.__data;
    }
    if (node.nodeType === COMMENT) {
        return '';
    }
    const kids = childList(node);
    if (kids.length === 0 && !isOwn(node) && node.__live !== undefined && node.__codeTouched !== true) {
        // What React drew with nothing in it that's a node, which is as it was laid out, with what its components drew in it. Text
        // with nodes beside it is as React gave it, not as laid out: a flex box's text has no spaces at its ends when it's drawn,
        // but `New Tab <kbd>` still has its space.
        return web.drawnText(node);
    }
    let out = '';
    for (const one of kids) {
        out += one.nodeType === TEXT ? one.__data : one.nodeType === ELEMENT || one.nodeType === FRAGMENT ? textOf(one) : '';
    }
    return out;
}

/// What elements made here are made from, by the page's class for their tag: that class's prototype, with what an element made
/// here has on top of it, once for all of them, rather than on each, as an editor's document has thousands of them.
const ownPrototypes = new Map();

function ownPrototypeFor(name) {
    const page = web.prototypeFor(name);
    let made = ownPrototypes.get(page);
    if (made === undefined) {
        made = Object.create(page);
        for (const [key, descriptor] of Object.entries(Object.getOwnPropertyDescriptors(ownElementProps))) {
            Object.defineProperty(made, key, { ...descriptor, enumerable: false, configurable: true });
        }
        // Its `style` is what any element's is, which says what it says as its `style` attribute. It's made the first time it's
        // wanted, as most of a document's elements never have one.
        Object.defineProperty(made, '__style', {
            get() {
                const node = this;
                if (node === made) {
                    // Asked of what they're made from, which has no style of its own.
                    return undefined;
                }
                const style = web.styleFor({ live: null, changed: () => node.__styleQuiet !== true && node.__syncStyle() });
                hidden(node, '__style', style);
                return style;
            },
            enumerable: false,
            configurable: true,
        });
        ownPrototypes.set(page, made);
    }
    return made;
}

/// Whether an element made here has had its `style` made, which it hasn't if nothing's asked for it.
const hasStyle = (node) => Object.prototype.hasOwnProperty.call(node, '__style');

/// Makes an element of this tag.
function makeElement(tag, ns = null) {
    const name = ns === SVG_NS ? String(tag) : String(tag).toLowerCase();
    const node = Object.create(ownPrototypeFor(name));
    hidden(node, '__own', true);
    hidden(node, '__id', '__domn' + nextNode++);
    hidden(node, '__parent', null);
    // Where it was last found among what it's in: see `ownIndex`.
    hidden(node, '__at', 0);
    hidden(node, '__kids', NO_KIDS);
    // What it's drawn as, made the first time: see `render`.
    hidden(node, '__drawnAs', null);
    hidden(node, '__attrs', NO_ATTRS);
    hidden(node, '__ns', ns ?? 'http://www.w3.org/1999/xhtml');
    hidden(node, 'localName', name);
    hidden(node, 'nodeType', ELEMENT);
    return node;
}

/// What text and comments have.
class CharacterData {
    get data() {
        return this.__data;
    }
    set data(value) {
        this.replaceData(0, this.__data.length, String(value ?? ''));
    }
    get nodeValue() {
        return this.__data;
    }
    set nodeValue(value) {
        this.data = value;
    }
    get textContent() {
        return this.__data;
    }
    set textContent(value) {
        this.data = value;
    }
    get wholeText() {
        return this.__data;
    }
    get length() {
        return this.__data.length;
    }
    get parentNode() {
        return this.__parent;
    }
    get parentElement() {
        const parent = this.__parent;
        return parent !== null && parent !== undefined && parent.nodeType === ELEMENT ? parent : null;
    }
    get ownerDocument() {
        return web.document;
    }
    get isConnected() {
        return isConnected(this);
    }
    get nodeName() {
        return this.nodeType === TEXT ? '#text' : '#comment';
    }
    get childNodes() {
        return nodeListOf([]);
    }
    get firstChild() {
        return null;
    }
    get lastChild() {
        return null;
    }
    hasChildNodes() {
        return false;
    }
    get nextSibling() {
        return Object.getOwnPropertyDescriptor(nodeMethods, 'nextSibling').get.call(this);
    }
    get previousSibling() {
        return Object.getOwnPropertyDescriptor(nodeMethods, 'previousSibling').get.call(this);
    }
    get nextElementSibling() {
        return Object.getOwnPropertyDescriptor(elementMethods, 'nextElementSibling').get.call(this);
    }
    get previousElementSibling() {
        return Object.getOwnPropertyDescriptor(elementMethods, 'previousElementSibling').get.call(this);
    }
    substringData(offset, count) {
        return this.__data.substr(offset, count);
    }
    appendData(text) {
        this.replaceData(this.__data.length, 0, String(text));
    }
    insertData(offset, text) {
        this.replaceData(offset, 0, String(text));
    }
    deleteData(offset, count) {
        this.replaceData(offset, count, '');
    }
    replaceData(offset, count, text) {
        const before = this.__data;
        if (offset > before.length) {
            const error = new Error(`The offset ${offset} is greater than the node's length (${before.length}).`);
            error.name = 'IndexSizeError';
            throw error;
        }
        const end = Math.min(before.length, offset + count);
        const after = before.slice(0, offset) + text + before.slice(end);
        if (after === before) {
            return;
        }
        this.__data = after;
        const node = this;
        const removed = end - offset;
        rangesAfter((container, at) => {
            if (container !== node) {
                return [container, at];
            }
            if (at > offset && at <= end) {
                return [container, offset];
            }
            if (at > end) {
                return [container, at - removed + text.length];
            }
            return [container, at];
        });
        record('characterData', this, { oldValue: before });
        changed(this);
    }
    splitText(offset) {
        const tail = web.document.createTextNode(this.__data.slice(offset));
        const parent = this.__parent;
        if (parent !== null && parent !== undefined) {
            parent.insertBefore(tail, this.nextSibling);
            // What was in the part that went is in the new node, as the spec has live ranges.
            const node = this;
            rangesAfter((container, at) => (container === node && at > offset ? [tail, at - offset] : [container, at]));
        }
        this.replaceData(offset, this.__data.length - offset, '');
        return tail;
    }
    cloneNode() {
        return this.nodeType === TEXT ? web.document.createTextNode(this.__data) : web.document.createComment(this.__data);
    }
    isEqualNode(other) {
        return other?.nodeType === this.nodeType && other.__data === this.__data;
    }
    isSameNode(other) {
        return other === this;
    }
    contains(other) {
        return other === this;
    }
    compareDocumentPosition(other) {
        return nodeMethods.compareDocumentPosition.call(this, other);
    }
    getRootNode() {
        return nodeMethods.getRootNode.call(this);
    }
    remove() {
        if (this.__parent !== null && this.__parent !== undefined) {
            take(this);
        }
    }
    before(...nodes) {
        ownElementProps.before.call(this, ...nodes);
    }
    after(...nodes) {
        ownElementProps.after.call(this, ...nodes);
    }
    replaceWith(...nodes) {
        ownElementProps.replaceWith.call(this, ...nodes);
    }
    addEventListener(type, listener, options) {
        web.listenOnElement(this, String(type), listener, options);
    }
    removeEventListener(type, listener, options) {
        web.unlistenOnElement(this, String(type), listener, options);
    }
    dispatchEvent(event) {
        return dispatch(this, event);
    }
}

let TextNode = null;
let CommentNode = null;
let FragmentNode = null;

function makeClasses() {
    const Base = web.elementClasses.Node;
    // Text is what code checks for with `instanceof Text`, and a node as any is.
    class CharacterDataNode extends Base {}
    for (const [key, descriptor] of Object.entries(Object.getOwnPropertyDescriptors(CharacterData.prototype))) {
        if (key !== 'constructor') {
            Object.defineProperty(CharacterDataNode.prototype, key, descriptor);
        }
    }
    TextNode = class Text extends CharacterDataNode {
        constructor(data = '') {
            super();
            hidden(this, '__own', true);
            hidden(this, '__id', '__domn' + nextNode++);
            hidden(this, '__parent', null);
            hidden(this, '__at', 0);
            hidden(this, '__data', String(data));
            hidden(this, 'nodeType', TEXT);
        }
    };
    CommentNode = class Comment extends CharacterDataNode {
        constructor(data = '') {
            super();
            hidden(this, '__own', true);
            hidden(this, '__id', '__domn' + nextNode++);
            hidden(this, '__parent', null);
            hidden(this, '__at', 0);
            hidden(this, '__data', String(data));
            hidden(this, 'nodeType', COMMENT);
        }
    };
    FragmentNode = class DocumentFragment extends (web.elementClasses.DocumentFragment ?? Base) {
        constructor() {
            super();
            hidden(this, '__own', true);
            hidden(this, '__id', '__domn' + nextNode++);
            hidden(this, '__parent', null);
            hidden(this, '__at', 0);
            hidden(this, '__kids', NO_KIDS);
            hidden(this, 'nodeType', FRAGMENT);
        }
        get nodeName() {
            return '#document-fragment';
        }
        get textContent() {
            return textOf(this);
        }
        get ownerDocument() {
            return web.document;
        }
        cloneNode(deep = false) {
            const copy = new FragmentNode();
            if (deep) {
                for (const one of this.__kids) {
                    copy.appendChild(one.cloneNode(true));
                }
            }
            return copy;
        }
        querySelector(selector) {
            return querySelectorIn(this, String(selector), true)[0] ?? null;
        }
        querySelectorAll(selector) {
            return nodeListOf(querySelectorIn(this, String(selector), false));
        }
        getElementById(id) {
            return querySelectorIn(this, '#' + id, true)[0] ?? null;
        }
    };
    for (const Kind of [FragmentNode]) {
        for (const source of [nodeMethods, elementMethods]) {
            for (const [key, descriptor] of Object.entries(Object.getOwnPropertyDescriptors(source))) {
                if (!Object.prototype.hasOwnProperty.call(Kind.prototype, key)) {
                    Object.defineProperty(Kind.prototype, key, descriptor);
                }
            }
        }
    }
}

// HTML, read and written, as `innerHTML` and the clipboard have it.

const ENTITIES = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: '\u00a0', copy: '©', reg: '®', hellip: '…', mdash: '—', ndash: '–', lsquo: '‘', rsquo: '’', ldquo: '“', rdquo: '”', bull: '•', middot: '·', zwj: '‍', zwnj: '‌' };

function decode(text) {
    return text.replace(/&(#x[0-9a-f]+|#[0-9]+|[a-z]+);/gi, (whole, name) => {
        if (name[0] === '#') {
            const code = name[1] === 'x' || name[1] === 'X' ? parseInt(name.slice(2), 16) : parseInt(name.slice(1), 10);
            return Number.isFinite(code) ? String.fromCodePoint(code) : whole;
        }
        return ENTITIES[name.toLowerCase()] ?? whole;
    });
}

function escapeText(text) {
    return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/\u00a0/g, '&nbsp;');
}

function escapeAttribute(text) {
    return text.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/\u00a0/g, '&nbsp;');
}

function serialize(node) {
    if (node.nodeType === TEXT) {
        return escapeText(node.__data);
    }
    if (node.nodeType === COMMENT) {
        return `<!--${node.__data}-->`;
    }
    if (node.nodeType === FRAGMENT) {
        return node.__kids.map(serialize).join('');
    }
    if (node.__own !== true) {
        // A page's element drawn by React, with what's in it, without what React gave it to draw it with.
        const tag = String(node.localName ?? 'div');
        return VOID.has(tag) ? `<${tag}>` : `<${tag}>${childList(node).map(serialize).join('')}</${tag}>`;
    }
    const attributes = attrEntries(node).map(([name, value]) => ` ${name}="${escapeAttribute(value)}"`).join('');
    if (VOID.has(node.localName)) {
        return `<${node.localName}${attributes}>`;
    }
    return `<${node.localName}${attributes}>${node.__kids.map(serialize).join('')}</${node.localName}>`;
}

/// Reads HTML into a fragment, as `innerHTML` does, leaving out scripts.
function parseHTML(html) {
    const fragment = new FragmentNode();
    const stack = [fragment];
    const top = () => stack[stack.length - 1];
    const tagPattern = /<!--([\s\S]*?)-->|<!\[CDATA\[[\s\S]*?\]\]>|<!doctype[^>]*>|<\/\s*([a-zA-Z][\w:-]*)\s*>|<([a-zA-Z][\w:-]*)((?:\s+[^\s"'>\/=]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s"'=<>`]+))?)*)\s*(\/?)>/gi;
    let at = 0;
    let match;
    let skipping = null;
    while ((match = tagPattern.exec(html)) !== null) {
        if (match.index > at && skipping === null) {
            const text = decode(html.slice(at, match.index));
            if (text !== '') {
                top().appendChild(new TextNode(text));
            }
        }
        at = tagPattern.lastIndex;
        if (match[1] !== undefined) {
            if (skipping === null) {
                top().appendChild(new CommentNode(match[1]));
            }
            continue;
        }
        if (match[2] !== undefined) {
            const name = match[2].toLowerCase();
            if (skipping !== null) {
                if (name === skipping) {
                    skipping = null;
                }
                continue;
            }
            for (let i = stack.length - 1; i > 0; i -= 1) {
                if (stack[i].localName === name) {
                    stack.length = i;
                    break;
                }
            }
            continue;
        }
        if (match[3] !== undefined) {
            const name = match[3].toLowerCase();
            if (skipping !== null) {
                continue;
            }
            if (name === 'script' || name === 'style' || name === 'meta' || name === 'link' || name === 'title') {
                if (match[5] !== '/') {
                    skipping = name;
                }
                continue;
            }
            const element = makeElement(name);
            for (const [, key, , v1, v2, v3] of match[4].matchAll(/([^\s"'>\/=]+)(\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g)) {
                if (!/^on/i.test(key)) {
                    element.setAttribute(key, decode(v1 ?? v2 ?? v3 ?? ''));
                }
            }
            top().appendChild(element);
            if (!VOID.has(name) && match[5] !== '/') {
                stack.push(element);
            }
        }
    }
    if (at < html.length && skipping === null) {
        const text = decode(html.slice(at));
        if (text !== '') {
            top().appendChild(new TextNode(text));
        }
    }
    return fragment;
}

/// A document read from HTML, apart from the page's, as editors make one to read what's pasted or what they start with.
class ParsedDocument {
    constructor(html) {
        const parsed = parseHTML(html);
        const find = (parent, name) => Array.from(kidsOf(parent)).find((kid) => kid.localName === name) ?? null;
        let root = find(parsed, 'html');
        let body = find(root ?? parsed, 'body');
        let head = find(root ?? parsed, 'head');
        if (root === null) {
            root = makeElement('html');
        }
        if (head === null) {
            head = makeElement('head');
            root.insertBefore(head, root.firstChild);
        }
        if (body === null) {
            body = makeElement('body');
            for (const kid of Array.from(kidsOf(parsed))) {
                if (kid !== root && kid !== head) {
                    body.appendChild(kid);
                }
            }
        }
        for (const part of [head, body]) {
            if (parentOf(part) !== root) {
                root.appendChild(part);
            }
        }
        hidden(this, 'documentElement', root);
        hidden(this, 'head', head);
        hidden(this, 'body', body);
    }
    get nodeType() {
        return DOCUMENT;
    }
    get nodeName() {
        return '#document';
    }
    get childNodes() {
        return nodeListOf([this.documentElement]);
    }
    get firstChild() {
        return this.documentElement;
    }
    /// Its `<style>`s aren't read: what's read into it isn't shown.
    get styleSheets() {
        return [];
    }
    createElement(tag) {
        return makeElement(String(tag).toLowerCase());
    }
    createElementNS(ns, tag) {
        return makeElement(String(tag), ns);
    }
    createTextNode(data) {
        return new TextNode(String(data));
    }
    createComment(data) {
        return new CommentNode(String(data));
    }
    createDocumentFragment() {
        return new FragmentNode();
    }
    createRange() {
        return new Range();
    }
    querySelector(selector) {
        return querySelectorIn(this.documentElement, selector, true)[0] ?? null;
    }
    querySelectorAll(selector) {
        return nodeListOf(querySelectorIn(this.documentElement, selector, false));
    }
    getElementsByTagName(name) {
        return this.documentElement.getElementsByTagName(name);
    }
    getElementById(id) {
        return this.querySelector(`#${id}`);
    }
}

class DOMParser {
    parseFromString(text, _type) {
        return new ParsedDocument(String(text));
    }
}

class XMLSerializer {
    serializeToString(node) {
        return node.nodeType === DOCUMENT ? serialize(node.documentElement) : serialize(node);
    }
}

// Selectors, for what's made here: kinds of element, ids, classes, attributes, `:not()`, `:first-child` and the like, and the
// usual ways of saying what's in what.

function parseSelector(text) {
    return text.split(',').map((one) => {
        const parts = [];
        const tokens = one.trim().match(/(\s*[>+~]\s*|\s+|[^\s>+~]+(?:\([^)]*\))?(?:[^\s>+~]*)?)/g) ?? [];
        let combinator = ' ';
        for (const token of tokens) {
            const trimmed = token.trim();
            if (trimmed === '') {
                if (parts.length > 0) combinator = ' ';
                continue;
            }
            if (trimmed === '>' || trimmed === '+' || trimmed === '~') {
                combinator = trimmed;
                continue;
            }
            parts.push({ combinator: parts.length === 0 ? null : combinator, test: compound(trimmed) });
            combinator = ' ';
        }
        return parts;
    });
}

function compound(text) {
    const checks = [];
    const pattern = /([a-zA-Z*][\w-]*)|#([\w-]+)|\.([\w-]+)|\[\s*([\w:-]+)\s*(?:([~^$*|]?=)\s*(?:"([^"]*)"|'([^']*)'|([^\]\s]*)))?\s*\]|:(not|first-child|last-child|only-child|empty|focus|focus-within|checked|disabled|nth-child)(?:\(([^)]*)\))?/g;
    let match;
    while ((match = pattern.exec(text)) !== null) {
        const [, tag, id, cls, attr, op, a1, a2, a3, pseudo, arg] = match;
        if (tag !== undefined) {
            const name = tag.toLowerCase();
            if (name !== '*') checks.push((node) => String(node.localName ?? '').toLowerCase() === name);
        } else if (id !== undefined) {
            checks.push((node) => node.getAttribute?.('id') === id || node.id === id);
        } else if (cls !== undefined) {
            checks.push((node) => ` ${node.getAttribute?.('class') ?? node.className ?? ''} `.replace(/\s+/g, ' ').includes(` ${cls} `));
        } else if (attr !== undefined) {
            const want = a1 ?? a2 ?? a3;
            checks.push((node) => {
                const value = node.getAttribute?.(attr);
                if (value === null || value === undefined) return false;
                switch (op) {
                    case undefined: return true;
                    case '=': return value === want;
                    case '~=': return value.split(/\s+/).includes(want);
                    case '^=': return value.startsWith(want);
                    case '$=': return value.endsWith(want);
                    case '*=': return value.includes(want);
                    case '|=': return value === want || value.startsWith(want + '-');
                    default: return false;
                }
            });
        } else if (pseudo !== undefined) {
            const siblings = (node) => {
                const parent = parentOf(node);
                return parent === null ? [node] : childList(parent).filter((one) => one.nodeType === ELEMENT);
            };
            switch (pseudo) {
                case 'not': {
                    const inner = parseSelector(arg ?? '');
                    checks.push((node) => !inner.some((parts) => parts.length === 1 && parts[0].test(node)));
                    break;
                }
                case 'first-child': checks.push((node) => siblings(node)[0] === node); break;
                case 'last-child': checks.push((node) => siblings(node).at(-1) === node); break;
                case 'only-child': checks.push((node) => siblings(node).length === 1); break;
                case 'empty': checks.push((node) => childList(node).length === 0); break;
                case 'focus': checks.push((node) => web.document.activeElement === node); break;
                case 'focus-within': checks.push((node) => node.contains(web.document.activeElement)); break;
                case 'checked': checks.push((node) => node.hasAttribute?.('checked')); break;
                case 'disabled': checks.push((node) => node.hasAttribute?.('disabled')); break;
                case 'nth-child': {
                    const n = Number(arg);
                    checks.push((node) => siblings(node).indexOf(node) + 1 === n);
                    break;
                }
                default: checks.push(() => false);
            }
        }
    }
    return (node) => node.nodeType === ELEMENT && checks.every((check) => check(node));
}

function matchesParts(node, parts, index) {
    if (!parts[index].test(node)) {
        return false;
    }
    if (index === 0) {
        return true;
    }
    const combinator = parts[index].combinator;
    if (combinator === '>') {
        const parent = parentOf(node);
        return parent !== null && matchesParts(parent, parts, index - 1);
    }
    if (combinator === '+' || combinator === '~') {
        for (let at = node.previousSibling; at !== null && at !== undefined; at = at.previousSibling) {
            if (at.nodeType !== ELEMENT) continue;
            if (matchesParts(at, parts, index - 1)) return true;
            if (combinator === '+') return false;
        }
        return false;
    }
    for (let at = parentOf(node); at !== null && at !== undefined; at = parentOf(at)) {
        if (at.nodeType === ELEMENT && matchesParts(at, parts, index - 1)) {
            return true;
        }
    }
    return false;
}

const selectorCache = new Map();
function matchesSelector(node, selector) {
    let parsed = selectorCache.get(selector);
    if (parsed === undefined) {
        parsed = parseSelector(selector);
        selectorCache.set(selector, parsed);
    }
    return parsed.some((parts) => parts.length > 0 && matchesParts(node, parts, parts.length - 1));
}

function querySelectorIn(root, selector, first) {
    const out = [];
    const visit = (node) => {
        for (const child of childList(node)) {
            if (child.nodeType !== ELEMENT) {
                continue;
            }
            if (matchesSelector(child, selector)) {
                out.push(child);
                if (first) return true;
            }
            if (visit(child)) return true;
        }
        return false;
    };
    visit(root);
    return out;
}

// What `web.js` uses.

function install(hooks) {
    web = hooks;
    makeClasses();
    // What every node and element has, which a page's element drawn by React has too, as far as it doesn't have its own.
    for (const [target, source] of [[web.elementClasses.Node.prototype, nodeMethods], [web.elementClasses.Element.prototype, elementMethods]]) {
        for (const [key, descriptor] of Object.entries(Object.getOwnPropertyDescriptors(source))) {
            if (!Object.prototype.hasOwnProperty.call(target, key)) {
                Object.defineProperty(target, key, { ...descriptor, enumerable: false, configurable: true });
            }
        }
    }
    // These replace what `web.js` had stand in for them until now, which made this be loaded.
    const settle = (name, value) => Object.defineProperty(globalThis, name, { value, writable: true, enumerable: false, configurable: true });
    settle('Text', TextNode);
    settle('Comment', CommentNode);
    settle('CharacterData', Object.getPrototypeOf(TextNode));
    settle('MutationObserver', MutationObserver);
    settle('Range', Range);
    settle('StaticRange', StaticRange);
    settle('Selection', Selection);
    settle('DocumentFragment', FragmentNode);
    settle('DOMParser', DOMParser);
    settle('XMLSerializer', XMLSerializer);
}

module.exports = {
    install,
    ParsedDocument,
    makeElement,
    makeText: (data) => new TextNode(data),
    makeComment: (data) => new CommentNode(data),
    makeFragment: () => new FragmentNode(),
    renderKids,
    selection,
    Range,
    StaticRange,
    MutationObserver,
    dispatch,
    pathTo,
    makeEvent,
    parseHTML,
    serialize,
    childList,
    parentOf,
    indexIn,
    nodeLength,
    comparePoints,
    insert,
    take,
    textOf,
    isOwn,
    walkRange,
    deleteRange,
    noticeSelection,
    nodeMethods,
    elementMethods,
    get structure() {
        return structure;
    },
    changesSince,
    get selectionRange() {
        return selectionRange;
    },
    setSelection(anchorNode, anchorOffset, focusNode, focusOffset) {
        selection.setBaseAndExtent(anchorNode, anchorOffset, focusNode, focusOffset);
    },
};
