// What a page and rsui both have a way into, which is one thing underneath: the window's title, whether it's shown and has the
// keyboard, whether it covers the screen, the screen's scale, the network, the language, and what `matchMedia` asks about. Each
// web way in reads and sets what the rsui way does, and when it changes, by either or by the system, both hear about it.
// Part of `rsui/dom-globals`.

function install(internals) {
    const world = globalThis;
    const { rsui, page, mediaApplies, resizeListeners, motionListeners, PageElement, keyboardOut, focusedNow, tellFocus } = internals;
    const main = rsui.mainWindow;
    const desktop = () => require('rsui/desktop');
    const { FocusEvent, MediaQueryListEvent } = require('rsui/dom-events');
    const Event = world.Event;

    // The title, which is the main window's.
    Object.defineProperty(page, 'title', {
        get: () => main.getState().title,
        set: (value) => main.setTitle(String(value)),
        enumerable: true,
        configurable: true,
    });

    // Whether it's shown, and has the keyboard.
    const shown = (state) => state.visible && !state.minimized;
    Object.defineProperty(page, 'hidden', { get: () => !shown(main.getState()), enumerable: true, configurable: true });
    Object.defineProperty(page, 'visibilityState', { get: () => (shown(main.getState()) ? 'visible' : 'hidden'), enumerable: true, configurable: true });
    // And whether the page has it, which it hasn't while Tab has taken it out of the page (see `keyboardOut` in web.js).
    page.hasFocus = () => main.getState().focused && !keyboardOut();

    // Covering the screen, as the Fullscreen API has it, which is the main window being fullscreen.
    let fullscreenElement = null;
    Object.defineProperty(page, 'fullscreenElement', {
        get: () => (main.getState().fullscreen ? fullscreenElement ?? page.documentElement : null),
        enumerable: true,
        configurable: true,
    });
    Object.defineProperty(page, 'fullscreenEnabled', { value: true, enumerable: true, configurable: true });
    PageElement.prototype.requestFullscreen = function requestFullscreen() {
        fullscreenElement = this;
        main.setFullscreen(true);
        return Promise.resolve();
    };
    // The page itself can cover the screen too, which is what it usually is that does.
    for (const whole of [page.body, page.documentElement]) {
        if (whole !== undefined && whole !== null) {
            whole.requestFullscreen = PageElement.prototype.requestFullscreen;
        }
    }
    page.exitFullscreen = () => {
        main.setFullscreen(false);
        return Promise.resolve();
    };

    // What changes is told the page's way too, once for each change.
    let before = main.getState();
    main.onStateChange((now) => {
        if (shown(now) !== shown(before)) {
            page.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
        }
        if (now.focused !== before.focused && !keyboardOut()) {
            // What has the keyboard keeps it, and hears it go with the window and come back after it, as Chromium tells it
            // (`DispatchEventsOnWindowAndFocusedElement`): `blur` and `focusout` before the window's `blur`, and `focus` and
            // `focusin` after its `focus`.
            const had = focusedNow();
            if (!now.focused && had !== null) {
                tellFocus(had, 'blur');
                tellFocus(had, 'focusout');
            }
            world.dispatchEvent(new FocusEvent(now.focused ? 'focus' : 'blur'));
            if (now.focused && had !== null && focusedNow() === had) {
                tellFocus(had, 'focus');
                tellFocus(had, 'focusin');
            }
        }
        if (now.fullscreen !== before.fullscreen) {
            if (!now.fullscreen) {
                fullscreenElement = null;
            }
            page.dispatchEvent(new Event('fullscreenchange', { bubbles: true }));
        }
        before = now;
    });

    // The scale the window's drawn at, which is its screen's, and what a canvas is made for.
    Object.defineProperty(world, 'devicePixelRatio', {
        get: () => rsui.__host().windowScale(),
        enumerable: true,
        configurable: true,
    });

    // The network and the language, on `navigator`, with `online`, `offline` and `languagechange` as they change.
    const navigator = world.navigator;
    if (navigator !== undefined && navigator !== null) {
        for (const [name, get] of [
            ['onLine', () => desktop().network.status().online],
            ['language', () => desktop().system.locale()],
            ['languages', () => [desktop().system.locale()]],
        ]) {
            Object.defineProperty(navigator, name, { get, enumerable: true, configurable: true });
        }
        // The sound devices and the media keys.
        for (const [name, routed] of [['mediaDevices', 'navigatorMediaDevices'], ['mediaSession', 'navigatorMediaSession']]) {
            Object.defineProperty(navigator, name, { get: () => require('rsui/web-routes')[routed], enumerable: true, configurable: true });
        }
    }
    if (!('MediaMetadata' in world)) {
        Object.defineProperty(world, 'MediaMetadata', { get: () => require('rsui/web-routes').MediaMetadata, configurable: true });
    }
    let online = null;
    let language = null;
    const heard = { network: false, system: false };
    const listenFor = (type) => {
        if ((type === 'online' || type === 'offline') && !heard.network) {
            heard.network = true;
            online = desktop().network.status().online;
            desktop().network.onChange((status) => {
                if (status.online !== online) {
                    online = status.online;
                    world.dispatchEvent(new Event(online ? 'online' : 'offline'));
                }
            });
        }
        if (type === 'languagechange' && !heard.system) {
            heard.system = true;
            language = desktop().system.locale();
            desktop().system.onChange(({ locale }) => {
                if (locale !== language) {
                    language = locale;
                    world.dispatchEvent(new Event('languagechange'));
                }
            });
        }
    };
    const listen = world.addEventListener;
    world.addEventListener = (type, listener, options) => {
        listenFor(String(type));
        return listen(type, listener, options);
    };

    // What `matchMedia` asks, as CSS's `@media` has it, told again when the answer changes.
    const watched = new Set();
    const lookAgain = () => {
        for (const list of watched) {
            list.__lookAgain();
        }
    };
    let watching = false;
    const watch = () => {
        if (watching) {
            return;
        }
        watching = true;
        resizeListeners.add(lookAgain);
        motionListeners?.add(lookAgain);
        rsui.__onColorScheme?.(lookAgain);
        desktop().system.onChange(lookAgain);
    };
    const EventTarget = world.EventTarget;
    class MediaQueryList extends EventTarget {
        #media;
        #matches;
        #listening = 0;
        constructor(media) {
            super();
            this.#media = media;
            this.#matches = mediaApplies(media);
            this.onchange = null;
        }
        get media() {
            return this.#media;
        }
        get matches() {
            return mediaApplies(this.#media);
        }
        addEventListener(type, listener, options) {
            super.addEventListener(type, listener, options);
            if (type === 'change') {
                this.#listening += 1;
                this.#matches = mediaApplies(this.#media);
                watched.add(this);
                watch();
            }
        }
        removeEventListener(type, listener, options) {
            super.removeEventListener(type, listener, options);
            if (type === 'change' && --this.#listening <= 0) {
                watched.delete(this);
            }
        }
        addListener(listener) {
            this.addEventListener('change', listener);
        }
        removeListener(listener) {
            this.removeEventListener('change', listener);
        }
        __lookAgain() {
            const now = mediaApplies(this.#media);
            if (now === this.#matches) {
                return;
            }
            this.#matches = now;
            const event = new MediaQueryListEvent('change', { media: this.#media, matches: now });
            this.onchange?.(event);
            this.dispatchEvent(event);
        }
    }
    world.MediaQueryList = MediaQueryList;
    world.matchMedia = (query) => {
        const list = new MediaQueryList(String(query));
        // Something that only sets `onchange` hears too.
        let own = null;
        Object.defineProperty(list, 'onchange', {
            get: () => own,
            set(listener) {
                own = listener;
                if (typeof listener === 'function') {
                    watched.add(list);
                    watch();
                }
            },
            configurable: true,
        });
        return list;
    };
}

module.exports = { install };
