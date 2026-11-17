'use strict';
// `Event`, `EventTarget`, `AbortController` and `AbortSignal`, only compiled when something looks for one of them.

class Event {
    constructor(type, options = {}) {
        if (arguments.length === 0) {
            throw new TypeError("Failed to construct 'Event': 1 argument required, but only 0 present.");
        }
        this.type = String(type);
        this.bubbles = !!options?.bubbles;
        this.cancelable = !!options?.cancelable;
        this.composed = !!options?.composed;
        this.defaultPrevented = false;
        this.target = null;
        this.currentTarget = null;
        this.eventPhase = 0;
        this.isTrusted = false;
        this.timeStamp = performance.now();
        // Whether nothing further along should hear it, and whether nothing else here should either.
        this.__stopped = false;
        this.__immediate = false;
    }
    get srcElement() {
        return this.target;
    }
    get returnValue() {
        return !this.defaultPrevented;
    }
    set returnValue(value) {
        if (value === false) this.preventDefault();
    }
    get cancelBubble() {
        return this.__stopped;
    }
    set cancelBubble(value) {
        if (value) this.stopPropagation();
    }
    preventDefault() {
        if (this.cancelable && !this.__passive) this.defaultPrevented = true;
    }
    stopPropagation() {
        this.__stopped = true;
    }
    stopImmediatePropagation() {
        this.__stopped = true;
        this.__immediate = true;
    }
    /// What it goes through, as the page dispatching it says, nearest first.
    composedPath() {
        return this.__path ?? (this.currentTarget === null ? [] : [this.currentTarget]);
    }
    initEvent(type, bubbles = false, cancelable = false) {
        this.type = String(type);
        this.bubbles = !!bubbles;
        this.cancelable = !!cancelable;
    }
}
for (const [name, value] of [['NONE', 0], ['CAPTURING_PHASE', 1], ['AT_TARGET', 2], ['BUBBLING_PHASE', 3]]) {
    Object.defineProperty(Event, name, { value, enumerable: true });
    Object.defineProperty(Event.prototype, name, { value, enumerable: true });
}

class EventTarget {
    #listeners = new Map();
    addEventListener(type, listener, options) {
        if (listener === null || listener === undefined) return;
        const list = this.#listeners.get(type) ?? [];
        if (list.some((item) => item.listener === listener)) return;
        const once = typeof options === 'object' && options !== null && !!options.once;
        list.push({ listener, once });
        this.#listeners.set(type, list);
    }
    removeEventListener(type, listener) {
        const list = this.#listeners.get(type);
        if (list !== undefined) this.#listeners.set(type, list.filter((item) => item.listener !== listener));
    }
    dispatchEvent(event) {
        event.target = this;
        event.currentTarget = this;
        event.eventPhase = 2;
        for (const item of [...(this.#listeners.get(event.type) ?? [])]) {
            if (item.once) this.removeEventListener(event.type, item.listener);
            if (typeof item.listener === 'function') {
                item.listener.call(this, event);
            } else {
                item.listener.handleEvent(event);
            }
            if (event.__immediate) break;
        }
        event.currentTarget = null;
        event.eventPhase = 0;
        return !event.defaultPrevented;
    }
}

// As in Node and browsers, it's a `DOMException`, whose code for this is 20.
function abortError(message = 'This operation was aborted') {
    return new DOMException(message, 'AbortError');
}

class AbortSignal extends EventTarget {
    aborted = false;
    reason = undefined;
    onabort = null;
    static abort(reason) {
        const signal = new AbortSignal();
        signal.__abort(reason);
        return signal;
    }
    static timeout(milliseconds) {
        const signal = new AbortSignal();
        setTimeout(() => {
            signal.__abort(new DOMException('The operation was aborted due to timeout', 'TimeoutError'));
        }, milliseconds);
        return signal;
    }
    __abort(reason) {
        if (this.aborted) return;
        this.aborted = true;
        this.reason = reason === undefined ? abortError() : reason;
        const event = new Event('abort');
        if (typeof this.onabort === 'function') this.onabort(event);
        this.dispatchEvent(event);
    }
    throwIfAborted() {
        if (this.aborted) throw this.reason;
    }
}

class AbortController {
    signal = new AbortSignal();
    abort(reason) {
        this.signal.__abort(reason);
    }
}

module.exports = { Event, EventTarget, AbortController, AbortSignal };
