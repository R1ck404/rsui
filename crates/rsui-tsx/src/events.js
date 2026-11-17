// What events as the web has them need beyond what every click does: handlers written for the web on rsui's own elements, and
// what's being dragged. It's only loaded the first time either is wanted, so an app that has neither pays nothing for it.

const rsui = require('rsui');
const host = rsui.__host();

// Handlers for events as the web has them, on rsui's own elements, by what hears each of them in rsui. A pointer's events and the
// mouse's that go with them are each sent by themselves, pointer first, as on a page.
const RSUI_EVENTS = [
    ['__onPointerDown', [['onPointerDown', 'pointerdown']]],
    ['__onPointerDownCapture', [['onPointerDownCapture', 'pointerdown']]],
    ['__onPointerUp', [['onPointerUp', 'pointerup']]],
    ['__onPointerUpCapture', [['onPointerUpCapture', 'pointerup']]],
    ['__onPointerMove', [['onPointerMove', 'pointermove']]],
    ['__onPointerMoveCapture', [['onPointerMoveCapture', 'pointermove']]],
    ['__onMouseDown', [['onMouseDown', 'mousedown']]],
    ['__onMouseDownCapture', [['onMouseDownCapture', 'mousedown']]],
    ['__onMouseUp', [['onMouseUp', 'mouseup']]],
    ['__onMouseUpCapture', [['onMouseUpCapture', 'mouseup']]],
    ['__onMouseMove', [['onMouseMove', 'mousemove']]],
    ['__onMouseMoveCapture', [['onMouseMoveCapture', 'mousemove']]],
    ['__onWheel', [['onWheel', 'wheel']]],
    ['__onWheelCapture', [['onWheelCapture', 'wheel']]],
    ['__onKeyDown', [['onKeyDown', 'keydown']]],
    ['__onKeyDownCapture', [['onKeyDownCapture', 'keydown']]],
    ['__onKeyUp', [['onKeyUp', 'keyup']]],
    ['__onKeyUpCapture', [['onKeyUpCapture', 'keyup']]],
    ['__onAuxClick', [['onAuxClick', 'auxclick']]],
    ['__onClickCapture', [['onClickCapture', 'click']]],
];
const RSUI_ENTERS = [['onMouseEnter', 'mouseenter'], ['onPointerEnter', 'pointerenter'], ['onMouseOver', 'mouseover'], ['onPointerOver', 'pointerover']];
const RSUI_LEAVES = [['onMouseLeave', 'mouseleave'], ['onPointerLeave', 'pointerleave'], ['onMouseOut', 'mouseout'], ['onPointerOut', 'pointerout']];

// What the web calls each handler for a part of dragging.
const DRAG_HANDLERS = { dragstart: 'onDragStart', drag: 'onDrag', dragenter: 'onDragEnter', dragover: 'onDragOver', dragleave: 'onDragLeave', drop: 'onDrop', dragend: 'onDragEnd' };

// The event that came with something, which is last of what a handler's given, or one with nothing in it if nothing came.
function eventIn(args) {
    const last = args[args.length - 1];
    return typeof last === 'object' && last !== null ? last : Object.create(rsui.__newEventBase());
}

// What Rust asks for an element of rsui's own that has handlers for events as the web has them: rsui's handlers for them,
// each of which gives the event to what it was written for. See `gather_props` in `bridge.rs`.
function rsuiEvents(given) {
    const out = {};
    const has = (names) => names.some(([name]) => typeof given[name] === 'function');
    const each = (names, event) => {
        for (const [name, type] of names) {
            if (typeof given[name] === 'function') {
                event.type = type;
                given[name](event);
                if (event.__immediate === true) {
                    return;
                }
            }
        }
    };
    for (const [hears, names] of RSUI_EVENTS) {
        if (has(names)) {
            out[hears] = (...args) => each(names, eventIn(args));
        }
    }
    if (has(RSUI_ENTERS) || has(RSUI_LEAVES)) {
        out.__onPointerHover = (on, event) => each(on ? RSUI_ENTERS : RSUI_LEAVES, eventIn([event]));
    }
    if (typeof given.onPaste === 'function' || typeof given.onCopy === 'function' || typeof given.onCut === 'function') {
        out.__onClipboard = (what, event) => {
            const text = event?.data ?? '';
            const plain = (type) => /^text(\/plain)?$/i.test(String(type));
            event = eventIn([event]);
            event.type = what;
            event.clipboardData = {
                types: what === 'paste' && text !== '' ? ['text/plain'] : [],
                files: [],
                items: [],
                getData: (type) => (what === 'paste' && plain(type) ? text : ''),
                setData: (type, value) => plain(type) && host.copy(String(value)),
                clearData() {},
            };
            given[{ paste: 'onPaste', copy: 'onCopy', cut: 'onCut' }[what]]?.(event);
        };
    }
    if (typeof given.onCompositionStart === 'function' || typeof given.onCompositionUpdate === 'function' || typeof given.onCompositionEnd === 'function') {
        out.__onComposition = (phase, event) => {
            event = eventIn([event]);
            event.type = 'composition' + phase;
            given[{ start: 'onCompositionStart', update: 'onCompositionUpdate', end: 'onCompositionEnd' }[phase]]?.(event);
        };
    }
    // Dragging, as a page has it. `onDrag` is rsui's own, for being dragged with the pointer, so it's left as it is here.
    const dragNames = ['onDragStart', 'onDragEnter', 'onDragOver', 'onDragLeave', 'onDrop', 'onDragEnd'];
    if (dragNames.some((name) => typeof given[name] === 'function')) {
        if (typeof given.onDrop === 'function' || typeof given.onDragOver === 'function' || typeof given.onDragEnter === 'function') {
            host.wantDrops();
        }
        out.__onDrag = (type, event) => {
            const name = DRAG_HANDLERS[type];
            if (name !== 'onDrag' && typeof given[name] === 'function') {
                event = withDataTransfer(eventIn([event]), type);
                event.type = type;
                given[name](event);
            }
        };
    }
    if (typeof given.onBlur === 'function') {
        // `onFocus` is told as it always is, and `onBlur` when it's let go of.
        out.onFocus = (on) => {
            given.onFocus?.(on);
            if (on === false) {
                const event = Object.create(rsui.__newEventBase());
                event.type = 'blur';
                given.onBlur(event);
            }
        };
    }
    return out;
}

// What's being dragged inside the app, as a page's `dataTransfer` has it, from its `dragstart` until the next one.
let dragging = null;

// What a format's called, as `getData('text')` means plain text.
function dataFormat(format) {
    const lower = String(format).toLowerCase();
    return lower === 'text' ? 'text/plain' : lower === 'url' ? 'text/uri-list' : lower;
}

// Gives an event about dragging its `dataTransfer`: what was set at `dragstart` for something dragged inside the app, and the
// files or text from another program, for what's dragged in from there. Getting what's dragged works all through it.
function withDataTransfer(event, type) {
    const files = event.__files ?? [];
    const outside = files.length > 0 || typeof event.data === 'string';
    let held;
    if (outside) {
        held = { store: new Map(), effectAllowed: 'copy', dropEffect: 'copy', files: files.map((path) => ({ name: String(path).split(/[\\/]/).pop(), path, type: '', size: 0 })) };
        if (typeof event.data === 'string') {
            held.store.set('text/plain', event.data);
            if (/^[a-z][a-z0-9+.-]*:\/\//i.test(event.data.trim())) {
                held.store.set('text/uri-list', event.data.trim());
            }
        }
    } else {
        if (type === 'dragstart' || dragging === null) {
            dragging = { store: new Map(), effectAllowed: 'all', dropEffect: 'move', files: [] };
        }
        held = dragging;
    }
    event.dataTransfer = {
        get dropEffect() {
            return held.dropEffect;
        },
        set dropEffect(value) {
            held.dropEffect = String(value);
        },
        get effectAllowed() {
            return held.effectAllowed;
        },
        set effectAllowed(value) {
            held.effectAllowed = String(value);
        },
        get types() {
            return [...held.store.keys(), ...(held.files.length > 0 ? ['Files'] : [])];
        },
        files: held.files,
        items: [],
        getData: (format) => held.store.get(dataFormat(format)) ?? '',
        setData: (format, value) => {
            held.store.set(dataFormat(format), String(value));
        },
        clearData: (format) => (format === undefined ? held.store.clear() : held.store.delete(dataFormat(format))),
        setDragImage() {},
    };
    return event;
}

module.exports = { rsuiEvents, withDataTransfer, DRAG_HANDLERS };
