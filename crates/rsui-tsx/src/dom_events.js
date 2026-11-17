// The browser's event classes, and the small value classes that go with them, as `new MouseEvent('click', {...})` makes them.
// Each is what `instanceof` checks and what a library can make and dispatch itself. They're part of `rsui/dom-globals`, so only
// loaded once something on a page looks for one. What Rust sends is one of these too, by its constructor: see `classify` in
// `web.js`.

const world = globalThis;
const Event = world.Event;

const modifiers = (init) => ({ ctrlKey: !!init.ctrlKey, shiftKey: !!init.shiftKey, altKey: !!init.altKey, metaKey: !!init.metaKey });

function modifierState(event, key) {
    switch (key) {
        case 'Control': return event.ctrlKey;
        case 'Shift': return event.shiftKey;
        case 'Alt': return event.altKey;
        case 'Meta': case 'OS': return event.metaKey;
        default: return event.__modifierStates?.[key] === true;
    }
}

/// Whether this is one of `Kind`, made by it or sent by Rust as one.
function isKind(Kind, value) {
    if (value === null || (typeof value !== 'object' && typeof value !== 'function')) {
        return false;
    }
    for (let at = Object.getPrototypeOf(value); at !== null; at = Object.getPrototypeOf(at)) {
        if (at === Kind.prototype) {
            return true;
        }
    }
    for (let made = value.constructor; typeof made === 'function'; made = Object.getPrototypeOf(made)) {
        if (made === Kind) {
            return true;
        }
    }
    return false;
}

class UIEvent extends Event {
    constructor(type, init = {}) {
        super(type, init);
        this.view = init.view ?? null;
        this.detail = Number(init.detail ?? 0);
        this.which = Number(init.which ?? 0);
    }
    static [Symbol.hasInstance](value) {
        return isKind(this, value);
    }
}

class MouseEvent extends UIEvent {
    constructor(type, init = {}) {
        super(type, init);
        Object.assign(this, modifiers(init));
        this.screenX = Number(init.screenX ?? 0);
        this.screenY = Number(init.screenY ?? 0);
        this.clientX = Number(init.clientX ?? 0);
        this.clientY = Number(init.clientY ?? 0);
        this.movementX = Number(init.movementX ?? 0);
        this.movementY = Number(init.movementY ?? 0);
        this.button = Number(init.button ?? 0);
        this.buttons = Number(init.buttons ?? 0);
        this.relatedTarget = init.relatedTarget ?? null;
        this.which = init.which ?? this.button + 1;
        this.__modifierStates = init.modifierAltGraph ? { AltGraph: true } : undefined;
    }
    get x() {
        return this.clientX;
    }
    get y() {
        return this.clientY;
    }
    get pageX() {
        return this.clientX + (world.scrollX ?? 0);
    }
    get pageY() {
        return this.clientY + (world.scrollY ?? 0);
    }
    get offsetX() {
        return this.clientX - (this.target?.getBoundingClientRect?.().left ?? 0);
    }
    get offsetY() {
        return this.clientY - (this.target?.getBoundingClientRect?.().top ?? 0);
    }
    get layerX() {
        return this.offsetX;
    }
    get layerY() {
        return this.offsetY;
    }
    getModifierState(key) {
        return modifierState(this, key);
    }
    initMouseEvent(type, bubbles, cancelable, view, detail, screenX, screenY, clientX, clientY, ctrlKey, altKey, shiftKey, metaKey, button, relatedTarget) {
        this.initEvent(type, bubbles, cancelable);
        Object.assign(this, { view, detail, screenX, screenY, clientX, clientY, ctrlKey, altKey, shiftKey, metaKey, button, relatedTarget });
    }
}

class PointerEvent extends MouseEvent {
    constructor(type, init = {}) {
        super(type, init);
        this.pointerId = Number(init.pointerId ?? 0);
        this.width = Number(init.width ?? 1);
        this.height = Number(init.height ?? 1);
        this.pressure = Number(init.pressure ?? 0);
        this.tangentialPressure = Number(init.tangentialPressure ?? 0);
        this.tiltX = Number(init.tiltX ?? 0);
        this.tiltY = Number(init.tiltY ?? 0);
        this.twist = Number(init.twist ?? 0);
        this.altitudeAngle = Number(init.altitudeAngle ?? Math.PI / 2);
        this.azimuthAngle = Number(init.azimuthAngle ?? 0);
        this.pointerType = String(init.pointerType ?? '');
        this.isPrimary = !!init.isPrimary;
        this.__coalesced = init.coalescedEvents ?? [];
        this.__predicted = init.predictedEvents ?? [];
    }
    getCoalescedEvents() {
        return [...this.__coalesced];
    }
    getPredictedEvents() {
        return [...this.__predicted];
    }
}

class WheelEvent extends MouseEvent {
    constructor(type, init = {}) {
        super(type, init);
        this.deltaX = Number(init.deltaX ?? 0);
        this.deltaY = Number(init.deltaY ?? 0);
        this.deltaZ = Number(init.deltaZ ?? 0);
        this.deltaMode = Number(init.deltaMode ?? 0);
    }
}
Object.assign(WheelEvent, { DOM_DELTA_PIXEL: 0, DOM_DELTA_LINE: 1, DOM_DELTA_PAGE: 2 });

class KeyboardEvent extends UIEvent {
    constructor(type, init = {}) {
        super(type, init);
        Object.assign(this, modifiers(init));
        this.key = String(init.key ?? '');
        this.code = String(init.code ?? '');
        this.location = Number(init.location ?? 0);
        this.repeat = !!init.repeat;
        this.isComposing = !!init.isComposing;
        // What older code reads, which a page lets an event be made with.
        this.charCode = Number(init.charCode ?? 0);
        this.keyCode = Number(init.keyCode ?? 0);
        this.which = Number(init.which ?? (this.keyCode || this.charCode));
    }
    getModifierState(key) {
        return modifierState(this, key);
    }
    initKeyboardEvent(type, bubbles, cancelable, view, key, location, ctrlKey, altKey, shiftKey, metaKey) {
        this.initEvent(type, bubbles, cancelable);
        Object.assign(this, { view, key, location, ctrlKey, altKey, shiftKey, metaKey });
    }
}
Object.assign(KeyboardEvent, { DOM_KEY_LOCATION_STANDARD: 0, DOM_KEY_LOCATION_LEFT: 1, DOM_KEY_LOCATION_RIGHT: 2, DOM_KEY_LOCATION_NUMPAD: 3 });

class FocusEvent extends UIEvent {
    constructor(type, init = {}) {
        super(type, init);
        this.relatedTarget = init.relatedTarget ?? null;
    }
}

class InputEvent extends UIEvent {
    constructor(type, init = {}) {
        super(type, init);
        this.data = init.data ?? null;
        this.inputType = String(init.inputType ?? '');
        this.isComposing = !!init.isComposing;
        this.dataTransfer = init.dataTransfer ?? null;
        this.__ranges = init.targetRanges ?? [];
    }
    getTargetRanges() {
        return [...this.__ranges];
    }
}

class CompositionEvent extends UIEvent {
    constructor(type, init = {}) {
        super(type, init);
        this.data = String(init.data ?? '');
    }
}

class DragEvent extends MouseEvent {
    constructor(type, init = {}) {
        super(type, init);
        this.dataTransfer = init.dataTransfer ?? null;
    }
}

class ClipboardEvent extends Event {
    constructor(type, init = {}) {
        super(type, init);
        this.clipboardData = init.clipboardData ?? null;
    }
    static [Symbol.hasInstance](value) {
        return isKind(this, value);
    }
}

/// What a form hears as it's sent, with the button that sent it, if one did.
class SubmitEvent extends Event {
    constructor(type, init = {}) {
        super(type, init);
        this.submitter = init.submitter ?? null;
    }
    static [Symbol.hasInstance](value) {
        return isKind(this, value);
    }
}

class StorageEvent extends Event {
    constructor(type, init = {}) {
        super(type, init);
        this.key = init.key ?? null;
        this.oldValue = init.oldValue ?? null;
        this.newValue = init.newValue ?? null;
        this.url = String(init.url ?? '');
        this.storageArea = init.storageArea ?? null;
    }
}

class AnimationEvent extends Event {
    constructor(type, init = {}) {
        super(type, init);
        this.animationName = String(init.animationName ?? '');
        this.elapsedTime = Number(init.elapsedTime ?? 0);
        this.pseudoElement = String(init.pseudoElement ?? '');
    }
}

class TransitionEvent extends Event {
    constructor(type, init = {}) {
        super(type, init);
        this.propertyName = String(init.propertyName ?? '');
        this.elapsedTime = Number(init.elapsedTime ?? 0);
        this.pseudoElement = String(init.pseudoElement ?? '');
    }
}

class ErrorEvent extends Event {
    constructor(type, init = {}) {
        super(type, init);
        this.message = String(init.message ?? '');
        this.filename = String(init.filename ?? '');
        this.lineno = Number(init.lineno ?? 0);
        this.colno = Number(init.colno ?? 0);
        this.error = init.error;
    }
}

class PromiseRejectionEvent extends Event {
    constructor(type, init = {}) {
        super(type, init);
        this.promise = init.promise;
        this.reason = init.reason;
    }
}

class CustomEvent extends Event {
    constructor(type, init = {}) {
        super(type, init);
        this.detail = init.detail ?? null;
    }
    initCustomEvent(type, bubbles, cancelable, detail) {
        this.initEvent(type, bubbles, cancelable);
        this.detail = detail;
    }
}

class MediaQueryListEvent extends Event {
    constructor(type, init = {}) {
        super(type, init);
        this.media = String(init.media ?? '');
        this.matches = !!init.matches;
    }
}

class DOMRectReadOnly {
    #x;
    #y;
    #width;
    #height;
    constructor(x = 0, y = 0, width = 0, height = 0) {
        this.#x = Number(x);
        this.#y = Number(y);
        this.#width = Number(width);
        this.#height = Number(height);
    }
    static fromRect(rect = {}) {
        return new this(rect.x, rect.y, rect.width, rect.height);
    }
    get x() {
        return this.#x;
    }
    get y() {
        return this.#y;
    }
    get width() {
        return this.#width;
    }
    get height() {
        return this.#height;
    }
    get top() {
        return Math.min(this.#y, this.#y + this.#height);
    }
    get bottom() {
        return Math.max(this.#y, this.#y + this.#height);
    }
    get left() {
        return Math.min(this.#x, this.#x + this.#width);
    }
    get right() {
        return Math.max(this.#x, this.#x + this.#width);
    }
    toJSON() {
        return { x: this.x, y: this.y, width: this.width, height: this.height, top: this.top, right: this.right, bottom: this.bottom, left: this.left };
    }
    // What's set on one goes where a `DOMRect`'s would.
    __set(name, value) {
        if (name === 'x') this.#x = Number(value);
        else if (name === 'y') this.#y = Number(value);
        else if (name === 'width') this.#width = Number(value);
        else if (name === 'height') this.#height = Number(value);
    }
}

class DOMRect extends DOMRectReadOnly {}
for (const name of ['x', 'y', 'width', 'height']) {
    const get = Object.getOwnPropertyDescriptor(DOMRectReadOnly.prototype, name).get;
    Object.defineProperty(DOMRect.prototype, name, {
        get,
        set(value) {
            this.__set(name, value);
        },
        configurable: true,
        enumerable: true,
    });
}

/// `CSS`: `escape` as CSSOM has it, for building a selector out of an id, and `supports`, which says yes to what's written as
/// CSS is, as a browser of now does for nearly all of it, and no to what another engine's prefix is on.
const CSS = {
    escape(value) {
        const text = String(value);
        let out = '';
        for (let at = 0; at < text.length; at += 1) {
            const code = text.charCodeAt(at);
            if (code === 0) {
                out += '\uFFFD';
            } else if ((code >= 0x1 && code <= 0x1f) || code === 0x7f || (at === 0 && code >= 0x30 && code <= 0x39)
                || (at === 1 && code >= 0x30 && code <= 0x39 && text.charCodeAt(0) === 0x2d)) {
                out += '\\' + code.toString(16) + ' ';
            } else if (at === 0 && text.length === 1 && code === 0x2d) {
                out += '\\' + text[at];
            } else if (code >= 0x80 || code === 0x2d || code === 0x5f || (code >= 0x30 && code <= 0x39) || (code >= 0x41 && code <= 0x5a) || (code >= 0x61 && code <= 0x7a)) {
                out += text[at];
            } else {
                out += '\\' + text[at];
            }
        }
        return out;
    },
    supports(property, value) {
        const said = value === undefined ? String(property) : String(property) + ':' + String(value);
        if (value === undefined && !/^\s*(\(|not\s|selector\()/i.test(said)) {
            return false;
        }
        return !/(^|[(\s])-(moz|ms|o)-/i.test(said) && (value === undefined || String(value).trim() !== '');
    },
    registerProperty() {},
};

const made = {
    UIEvent, MouseEvent, PointerEvent, WheelEvent, KeyboardEvent, FocusEvent, InputEvent, CompositionEvent, DragEvent, ClipboardEvent,
    StorageEvent, SubmitEvent, AnimationEvent, TransitionEvent, ErrorEvent, PromiseRejectionEvent, CustomEvent, MediaQueryListEvent, DOMRectReadOnly,
    DOMRect, CSS,
};

/// Puts them on the page's globals, in place of any stand-in that was there.
function install() {
    for (const [name, value] of Object.entries(made)) {
        Object.defineProperty(world, name, { value, writable: true, enumerable: false, configurable: true });
    }
}

module.exports = { install, ...made };
