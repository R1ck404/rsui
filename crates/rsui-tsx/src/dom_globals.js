// The globals a page has, for code and libraries written for a browser: `document`, `window`, `location`, `history`, the event
// classes, `getComputedStyle`, `navigator` and the rest. Nothing here is loaded until one of them is first looked at, so an app that
// draws a page's tags with CSS and never touches the DOM doesn't pay for it. What it needs from the page is `rsui/web`'s, and
// it's given that. The nodes code makes, ranges and the selection are `rsui/dom`'s, which is only loaded once code asks for one.

function install(internals) {
    const { rsui, elementClasses, computedStyleOf, fillFormData, domPack, page, listenOn, unlistenOn, tellWindow, pageScrollTo,
        pageScrollBy, pageScrollState, pageScrollOffsets, location, history, ResizeObserver } = internals;
    const world = globalThis;
    // Code that has the page asks where things are, from now on, so what's laid out is kept for it.
    rsui.__host().keepLaidOut();
    // There are never any of these, and code that's checking what something is shouldn't stop because of that. What's there already
    // stays, as `DocumentFragment` does, which is `rsui/dom`'s once that's loaded: looking at it here would load that.
    for (const [name, made] of Object.entries(elementClasses)) {
        if (!Object.prototype.hasOwnProperty.call(world, name)) {
            world[name] = made;
        }
    }
    world.getComputedStyle = (element) => computedStyleOf(element);
    // `new FormData(form)` reads what a page's form has in it, as a browser's does. The engine's is only made when it's wanted.
    const engineFormData = Object.getOwnPropertyDescriptor(world, 'FormData');
    let pageFormData = null;
    Object.defineProperty(world, 'FormData', {
        get() {
            if (pageFormData === null) {
                const Base = typeof engineFormData?.get === 'function' ? engineFormData.get.call(world) : engineFormData?.value;
                pageFormData = class FormData extends Base {
                    constructor(form = undefined, submitter = undefined) {
                        super();
                        if (form === undefined || form === null) {
                            return;
                        }
                        const sending = form.__live?.sending;
                        if (sending === undefined) {
                            throw new TypeError("Failed to construct 'FormData': parameter 1 is not of type 'HTMLFormElement'.");
                        }
                        fillFormData(this, sending, submitter ?? null);
                    }
                };
            }
            // The engine's own, once it's made, is what it is from then on, which would leave this behind.
            Object.defineProperty(world, 'FormData', { value: pageFormData, writable: true, enumerable: false, configurable: true });
            return pageFormData;
        },
        set(value) {
            Object.defineProperty(world, 'FormData', { value, writable: true, enumerable: false, configurable: true });
        },
        enumerable: false,
        configurable: true,
    });
    world.getSelection ??= () => domPack().selection;
    // Told about each element's size after the window's laid out: see `PageResizeObserver` in `web.js`.
    world.ResizeObserver ??= ResizeObserver;
    // What the engine only makes the first time it's looked at, as `Event` and `navigator` are, is left until then, as making it
    // means compiling what it's in, which takes longer than the rest of this.
    const madeWhenWanted = (name) => typeof Object.getOwnPropertyDescriptor(world, name)?.get === 'function';
    // What it would have been set to, as `world.name = value` sets it.
    const settleAs = (name, value) => Object.defineProperty(world, name, { value, writable: true, enumerable: true, configurable: true });
    if (!madeWhenWanted('Event')) {
        world.Event ??= class {
            constructor(type, options) {
                this.type = type;
                Object.assign(this, options);
            }
            preventDefault() {}
            stopPropagation() {}
        };
    }
    if (world.CustomEvent === undefined || world.CustomEvent === null) {
        if (madeWhenWanted('Event')) {
            Object.defineProperty(world, 'CustomEvent', {
                get() {
                    const made = world.Event;
                    settleAs('CustomEvent', made);
                    return made;
                },
                set(value) {
                    settleAs('CustomEvent', value);
                },
                enumerable: true,
                configurable: true,
            });
        } else {
            world.CustomEvent = world.Event;
        }
    }
    world.document ??= page;
    world.window ??= world;

    world.matchMedia ??= (query) => ({
        matches: String(query).includes('dark') && rsui.__colorScheme() === 'dark',
        media: String(query),
        addEventListener() {},
        removeEventListener() {},
        addListener() {},
        removeListener() {},
    });
    // The window hears what `document` does, and its changing size.
    world.addEventListener ??= (type, listener, options) => listenOn('window', String(type), listener, options);
    world.removeEventListener ??= (type, listener, options) => unlistenOn('window', String(type), listener, options);
    world.dispatchEvent ??= tellWindow;
    // The page's own scrolling, as the window's.
    world.scrollTo = pageScrollTo;
    world.scroll = pageScrollTo;
    world.scrollBy = pageScrollBy;
    for (const [name, along] of [['scrollX', 'x'], ['scrollY', 'y'], ['pageXOffset', 'x'], ['pageYOffset', 'y']]) {
        Object.defineProperty(world, name, { get: () => pageScrollOffsets()[along], set() {}, enumerable: true, configurable: true });
    }
    // What's asked for the next frame is done together at the next frame, as a browser does it: asking again, or cancelling,
    // doesn't move that frame later. Each with its own 16 ms timer, a drag that cancels and asks again at each move, as
    // Mantine's `useMove` does, had the frame put off by every move, so a ColorPicker's or Slider's thumb only moved once the
    // pointer slowed to under 60 moves a second.
    if (world.requestAnimationFrame === undefined) {
        let wanted = new Map();
        let lastId = 0;
        let timer = null;
        const runFrame = () => {
            timer = null;
            const due = wanted;
            wanted = new Map();
            // The frame's time, on `performance.now()`'s clock, as a browser tells it.
            const now = performance.now();
            for (const run of due.values()) {
                try {
                    run(now);
                } catch (error) {
                    // Told as it would have been from a timer of its own, without stopping the rest.
                    setTimeout(() => {
                        throw error;
                    }, 0);
                }
            }
        };
        world.requestAnimationFrame = (run) => {
            const id = ++lastId;
            wanted.set(id, run);
            if (timer === null) {
                timer = setTimeout(runFrame, 16);
            }
            return id;
        };
        world.cancelAnimationFrame = (id) => {
            wanted.delete(id);
        };
    }
    // The engine has some of this, and code looks at the rest.
    // A page is told it's in Edge, as `layer.js` says from the start: see `PAGE_AGENT` there.
    const agent = rsui.__pageAgent;
    const wanted = { userAgent: agent, appVersion: agent.slice('Mozilla/'.length), vendor: 'Google Inc.', platform: 'Win32', language: 'en-US', languages: ['en-US'], onLine: true, maxTouchPoints: 0, hardwareConcurrency: 1 };
    const filledIn = (navigator) => {
        for (const name of Object.keys(wanted)) {
            // What's worked out when it's asked for, as `hardwareConcurrency` is, is there, and asking would load what works it out.
            if (typeof Object.getOwnPropertyDescriptor(navigator, name)?.get !== 'function' && navigator[name] === undefined) {
                navigator[name] = wanted[name];
            }
        }
        if (navigator.clipboard === undefined) {
            Object.defineProperty(navigator, 'clipboard', { get: () => require('rsui/web-routes').navigatorClipboard, configurable: true, enumerable: true });
        }
        return navigator;
    };
    const made = Object.getOwnPropertyDescriptor(world, 'navigator');
    if (typeof made?.get === 'function' && typeof made.set === 'function') {
        // The engine's, filled in when it's first looked at.
        Object.defineProperty(world, 'navigator', {
            get() {
                const navigator = filledIn(made.get.call(world) ?? {});
                made.set.call(world, navigator);
                return navigator;
            },
            set(value) {
                made.set.call(world, value);
            },
            configurable: true,
        });
    } else {
        const navigator = filledIn(world.navigator ?? {});
        world.navigator = navigator;
    }
    world.location ??= location;
    world.history ??= history;
    for (const name of ['PopStateEvent', 'HashChangeEvent']) {
        world[name] ??= class extends (typeof world.Event === 'function' ? world.Event : Object) {
            constructor(type, options = {}) {
                super(type, options);
                Object.assign(this, { type, state: null, oldURL: '', newURL: '' }, options);
            }
        };
    }
    require('rsui/dom-events').install(internals);
    require('rsui/dom-observers').install(internals);
    require('rsui/dom-state').install(internals);
}

module.exports = install;
