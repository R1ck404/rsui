// What a page has for watching itself and going through itself: `IntersectionObserver`, `TreeWalker` and `NodeIterator`, and the
// watching that `MutationObserver` does of what React draws (see `dom.js`). What watches looks again after the page has been drawn
// again or the window's changed size, and at nothing else, so an app that's still does no work for it. `ResizeObserver` is told
// after the window's laid out instead: see `PageResizeObserver` in `web.js`. Part of `rsui/dom-globals`.

const NodeFilter = Object.freeze({
    FILTER_ACCEPT: 1, FILTER_REJECT: 2, FILTER_SKIP: 3,
    SHOW_ALL: 0xffffffff, SHOW_ELEMENT: 0x1, SHOW_ATTRIBUTE: 0x2, SHOW_TEXT: 0x4, SHOW_CDATA_SECTION: 0x8, SHOW_PROCESSING_INSTRUCTION: 0x40,
    SHOW_COMMENT: 0x80, SHOW_DOCUMENT: 0x100, SHOW_DOCUMENT_TYPE: 0x200, SHOW_DOCUMENT_FRAGMENT: 0x400,
});

/// The ways from one node to the next: between elements only, and between every node, text and comments too.
const BY_ELEMENT = { first: 'firstElementChild', last: 'lastElementChild', next: 'nextElementSibling', previous: 'previousElementSibling', parent: 'parentElement' };
const BY_NODE = { first: 'firstChild', last: 'lastChild', next: 'nextSibling', previous: 'previousSibling', parent: 'parentNode' };

/// Going through what's in `root`, as `document.createTreeWalker()` does. One that only shows elements goes from element to element,
/// which doesn't need the DOM's text nodes (`rsui/dom`), and one that shows more goes through every node.
class TreeWalker {
    #by;
    constructor(root, whatToShow = NodeFilter.SHOW_ALL, filter = null) {
        this.root = root;
        this.whatToShow = whatToShow >>> 0;
        this.filter = filter;
        this.currentNode = root;
        this.#by = (this.whatToShow & ~(NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_DOCUMENT | NodeFilter.SHOW_DOCUMENT_FRAGMENT)) === 0 ? BY_ELEMENT : BY_NODE;
    }
    #accept(node) {
        const type = node?.nodeType ?? 1;
        if (((1 << (type - 1)) & this.whatToShow) === 0) {
            return NodeFilter.FILTER_SKIP;
        }
        const filter = this.filter;
        if (filter === null || filter === undefined) {
            return NodeFilter.FILTER_ACCEPT;
        }
        return typeof filter === 'function' ? filter(node) : filter.acceptNode(node);
    }
    parentNode() {
        let node = this.currentNode;
        while (node !== null && node !== undefined && node !== this.root) {
            node = node[this.#by.parent] ?? null;
            if (node !== null && this.#accept(node) === NodeFilter.FILTER_ACCEPT) {
                this.currentNode = node;
                return node;
            }
        }
        return null;
    }
    #children(first) {
        let node = first ? this.currentNode[this.#by.first] : this.currentNode[this.#by.last];
        while (node !== null && node !== undefined) {
            const result = this.#accept(node);
            if (result === NodeFilter.FILTER_ACCEPT) {
                this.currentNode = node;
                return node;
            }
            if (result === NodeFilter.FILTER_SKIP) {
                const inside = first ? node[this.#by.first] : node[this.#by.last];
                if (inside !== null && inside !== undefined) {
                    node = inside;
                    continue;
                }
            }
            for (;;) {
                const beside = first ? node[this.#by.next] : node[this.#by.previous];
                if (beside !== null && beside !== undefined) {
                    node = beside;
                    break;
                }
                const parent = node[this.#by.parent] ?? null;
                if (parent === null || parent === this.root || parent === this.currentNode) {
                    return null;
                }
                node = parent;
            }
        }
        return null;
    }
    firstChild() {
        return this.#children(true);
    }
    lastChild() {
        return this.#children(false);
    }
    #siblings(next) {
        let node = this.currentNode;
        if (node === this.root) {
            return null;
        }
        for (;;) {
            let beside = next ? node[this.#by.next] : node[this.#by.previous];
            while (beside !== null && beside !== undefined) {
                node = beside;
                const result = this.#accept(node);
                if (result === NodeFilter.FILTER_ACCEPT) {
                    this.currentNode = node;
                    return node;
                }
                beside = next ? node[this.#by.first] : node[this.#by.last];
                if (result === NodeFilter.FILTER_REJECT || beside === null || beside === undefined) {
                    beside = next ? node[this.#by.next] : node[this.#by.previous];
                }
            }
            node = node[this.#by.parent] ?? null;
            if (node === null || node === this.root) {
                return null;
            }
            if (this.#accept(node) === NodeFilter.FILTER_ACCEPT) {
                return null;
            }
        }
    }
    nextSibling() {
        return this.#siblings(true);
    }
    previousSibling() {
        return this.#siblings(false);
    }
    nextNode() {
        let node = this.currentNode;
        let result = NodeFilter.FILTER_ACCEPT;
        for (;;) {
            while (result !== NodeFilter.FILTER_REJECT) {
                const first = node[this.#by.first];
                if (first === null || first === undefined) {
                    break;
                }
                node = first;
                result = this.#accept(node);
                if (result === NodeFilter.FILTER_ACCEPT) {
                    this.currentNode = node;
                    return node;
                }
            }
            let beside = null;
            let at = node;
            while (at !== null && at !== undefined) {
                if (at === this.root) {
                    return null;
                }
                beside = at[this.#by.next];
                if (beside !== null && beside !== undefined) {
                    break;
                }
                at = at[this.#by.parent] ?? null;
            }
            if (beside === null || beside === undefined) {
                return null;
            }
            node = beside;
            result = this.#accept(node);
            if (result === NodeFilter.FILTER_ACCEPT) {
                this.currentNode = node;
                return node;
            }
        }
    }
    previousNode() {
        let node = this.currentNode;
        while (node !== this.root) {
            let beside = node[this.#by.previous];
            while (beside !== null && beside !== undefined) {
                node = beside;
                let result = this.#accept(node);
                while (result !== NodeFilter.FILTER_REJECT && node[this.#by.last]) {
                    node = node[this.#by.last];
                    result = this.#accept(node);
                }
                if (result === NodeFilter.FILTER_ACCEPT) {
                    this.currentNode = node;
                    return node;
                }
                beside = node[this.#by.previous];
            }
            if (node === this.root || node[this.#by.parent] === null || node[this.#by.parent] === undefined) {
                return null;
            }
            node = node[this.#by.parent];
            if (this.#accept(node) === NodeFilter.FILTER_ACCEPT) {
                this.currentNode = node;
                return node;
            }
        }
        return null;
    }
}

/// `document.createNodeIterator()`, which is a tree walker that only goes forwards and back.
class NodeIterator {
    #walker;
    constructor(root, whatToShow = NodeFilter.SHOW_ALL, filter = null) {
        this.#walker = new TreeWalker(root, whatToShow, filter);
        this.root = root;
        this.whatToShow = whatToShow;
        this.filter = filter;
        this.referenceNode = root;
        this.pointerBeforeReferenceNode = true;
    }
    nextNode() {
        if (this.pointerBeforeReferenceNode) {
            this.pointerBeforeReferenceNode = false;
            const accepted = this.filter === null || (typeof this.filter === 'function' ? this.filter(this.root) : this.filter.acceptNode(this.root)) === NodeFilter.FILTER_ACCEPT;
            if (this.referenceNode === this.root && accepted) {
                return this.root;
            }
        }
        this.#walker.currentNode = this.referenceNode;
        const next = this.#walker.nextNode();
        if (next !== null) {
            this.referenceNode = next;
        }
        return next;
    }
    previousNode() {
        this.#walker.currentNode = this.referenceNode;
        const before = this.#walker.previousNode();
        if (before !== null) {
            this.referenceNode = before;
        }
        return before;
    }
    detach() {}
}

/// What watches the page, told when it's been drawn again, or the window's changed size.
function watchers(onPageRendered, resizeListeners) {
    const watching = new Set();
    let soon = null;
    const look = () => {
        soon = null;
        for (const one of [...watching]) {
            try {
                one.__look();
            } catch (error) {
                console.error(error);
            }
        }
    };
    // After whatever's changing now has been drawn, which is when where things are is known. What can't have anything to find,
    // as a `MutationObserver` on what only code changes, says so, and isn't waited for.
    const lookSoon = () => {
        if (soon === null && watching.size > 0) {
            for (const one of watching) {
                if (one.__wantsLook?.() ?? true) {
                    soon = setTimeout(look, 16);
                    return;
                }
            }
        }
    };
    let hooked = false;
    return {
        add(one) {
            watching.add(one);
            if (!hooked) {
                hooked = true;
                onPageRendered(lookSoon);
                resizeListeners.add(lookSoon);
            }
            lookSoon();
        },
        remove(one) {
            watching.delete(one);
        },
        lookSoon,
    };
}

/// What an `IntersectionObserver`'s `rootMargin` adds to each side of its root, as CSS's `margin` says them: one to four
/// lengths, top, right, bottom and left, in pixels or as a share of the root's width (across) or height (down).
function rootMargins(text, width, height) {
    const parts = String(text ?? '').trim().split(/\s+/).filter((part) => part !== '');
    const [top = '0px', right = top, bottom = top, left = right] = parts;
    const length = (part, of) => (part.endsWith('%') ? ((parseFloat(part) || 0) / 100) * of : parseFloat(part) || 0);
    return { top: length(top, height), right: length(right, width), bottom: length(bottom, height), left: length(left, width) };
}

function install(internals) {
    const world = globalThis;
    const { onPageRendered, resizeListeners } = internals;
    const { DOMRectReadOnly } = require('rsui/dom-events');
    const watched = watchers(onPageRendered, resizeListeners);

    class IntersectionObserver {
        #run;
        #targets = new Map();
        constructor(run, options = {}) {
            this.#run = run;
            this.root = options.root ?? null;
            this.rootMargin = options.rootMargin ?? '0px';
            const thresholds = options.threshold ?? 0;
            this.thresholds = Object.freeze((Array.isArray(thresholds) ? thresholds : [thresholds]).map(Number).sort((a, b) => a - b));
        }
        observe(target) {
            if (!this.#targets.has(target)) {
                this.#targets.set(target, { band: -1 });
            }
            watched.add(this);
        }
        unobserve(target) {
            this.#targets.delete(target);
        }
        disconnect() {
            this.#targets.clear();
            watched.remove(this);
        }
        takeRecords() {
            return [];
        }
        __look() {
            // A document as the root is its window, as having none is. floating-ui gives one, with a margin on each side that
            // makes the root the element's own box, and looks again whenever what it's told isn't all of it: with the margin
            // read as one number for every side, it never was, and it looked again every second for each popover, for good.
            const viewport = this.root === null || this.root === undefined || this.root.nodeType === 9 || typeof this.root.getBoundingClientRect !== 'function';
            const outer = viewport
                ? { left: 0, top: 0, right: world.innerWidth ?? 0, bottom: world.innerHeight ?? 0 }
                : this.root.getBoundingClientRect();
            const margin = rootMargins(this.rootMargin, outer.right - outer.left, outer.bottom - outer.top);
            const root = { left: outer.left - margin.left, top: outer.top - margin.top, right: outer.right + margin.right, bottom: outer.bottom + margin.bottom };
            const entries = [];
            for (const [target, last] of this.#targets) {
                const box = target.getBoundingClientRect?.() ?? { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 };
                const left = Math.max(box.left, root.left);
                const top = Math.max(box.top, root.top);
                const right = Math.min(box.right, root.right);
                const bottom = Math.min(box.bottom, root.bottom);
                const width = Math.max(0, right - left);
                const height = Math.max(0, bottom - top);
                const area = box.width * box.height;
                const touching = right >= left && bottom >= top && (target.isConnected !== false);
                const ratio = area > 0 ? (width * height) / area : touching ? 1 : 0;
                // Which of the thresholds it's past, which is what it's told about when that changes.
                const band = touching ? this.thresholds.filter((one) => ratio >= one).length : 0;
                if (band === last.band && last.touching === touching) {
                    continue;
                }
                last.band = band;
                last.touching = touching;
                entries.push({
                    target,
                    time: performance.now(),
                    isIntersecting: touching && (ratio > 0 || this.thresholds.includes(0)),
                    intersectionRatio: ratio,
                    boundingClientRect: new DOMRectReadOnly(box.left, box.top, box.width, box.height),
                    intersectionRect: new DOMRectReadOnly(left, top, width, height),
                    rootBounds: new DOMRectReadOnly(root.left, root.top, root.right - root.left, root.bottom - root.top),
                });
            }
            if (entries.length > 0) {
                this.#run(entries, this);
            }
        }
    }

    for (const [name, value] of Object.entries({ IntersectionObserver, TreeWalker, NodeIterator, NodeFilter })) {
        Object.defineProperty(world, name, { value, writable: true, enumerable: false, configurable: true });
    }
}

module.exports = { install, watchers, TreeWalker, NodeIterator, NodeFilter };
