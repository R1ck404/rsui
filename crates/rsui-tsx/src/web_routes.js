// The browser's names for what `rsui/desktop` does, for code and libraries written for a page. Each one is a thin front over the
// same native call or the same value as the rsui way in, so the two never disagree: what `localStorage` sets, `storage` reads,
// and the other way round. Nothing here is loaded until a page's name is first used.

const desktop = require('rsui/desktop');
const { Playback } = require('rsui/desktop/sound');
const { Store } = require('rsui/desktop/storage');
const host = require('rsui').__host();

/// A notification, as the web has it. Permission is always granted, as the program's already on the desktop.
class Notification {
    static permission = 'granted';

    static requestPermission(done) {
        done?.('granted');
        return Promise.resolve('granted');
    }

    #listeners = new Map();

    constructor(title, options = {}) {
        this.title = String(title);
        this.body = options.body ?? '';
        this.tag = options.tag ?? '';
        this.data = options.data ?? null;
        this.onclick = null;
        this.onclose = null;
        this.onshow = null;
        this.onerror = null;
        desktop.notifications.show({ title: this.title, body: this.body, onClick: () => this.#fire('click') });
        queueMicrotask(() => this.#fire('show'));
    }

    #fire(type) {
        const event = { type, target: this, currentTarget: this, preventDefault() {} };
        this['on' + type]?.(event);
        for (const listener of this.#listeners.get(type) ?? []) {
            listener(event);
        }
    }

    addEventListener(type, listener) {
        if (!this.#listeners.has(type)) {
            this.#listeners.set(type, new Set());
        }
        this.#listeners.get(type).add(listener);
    }

    removeEventListener(type, listener) {
        this.#listeners.get(type)?.delete(listener);
    }

    close() {
        this.#fire('close');
    }
}

/// A sound, as the web has it, over `sound`'s playback. MP3 and WAV files can both be played.
class Audio {
    #playback = null;
    #src = '';
    #listeners = new Map();

    constructor(src) {
        this.src = src ?? '';
        this.loop = false;
        this.paused = true;
        this.ended = false;
        this.onended = null;
        this.onplay = null;
        this.onpause = null;
        this.preload = 'auto';
        this.autoplay = false;
    }

    get src() {
        return this.#src;
    }

    set src(value) {
        this.#playback?.stop();
        this.#playback = null;
        this.#src = String(value);
    }

    #made() {
        if (this.#playback === null) {
            this.#playback = new Playback(this.#src, { onEnded: () => this.#reachedEnd() });
        }
        this.#playback.loop = this.loop;
        return this.#playback;
    }

    get volume() {
        return this.#playback?.volume ?? this._volume ?? 1;
    }

    set volume(value) {
        this._volume = Math.min(1, Math.max(0, Number(value)));
        if (this.#playback !== null) {
            this.#playback.volume = this._volume;
        }
    }

    get muted() {
        return this.#playback?.muted ?? this._muted ?? false;
    }

    set muted(on) {
        this._muted = on === true;
        if (this.#playback !== null) {
            this.#playback.muted = this._muted;
        }
    }

    get duration() {
        const length = this.#playback?.duration;
        return length === null || length === undefined ? NaN : length / 1000;
    }

    get currentTime() {
        return (this.#playback?.position ?? 0) / 1000;
    }

    set currentTime(seconds) {
        this.#made().seek(Number(seconds) * 1000);
    }

    #fire(type) {
        const event = { type, target: this, currentTarget: this };
        this['on' + type]?.(event);
        for (const listener of this.#listeners.get(type) ?? []) {
            listener(event);
        }
    }

    #reachedEnd() {
        this.paused = true;
        this.ended = true;
        this.#fire('ended');
    }

    play() {
        const playback = this.#made();
        if (this._volume !== undefined) {
            playback.volume = this._volume;
        }
        if (this._muted !== undefined) {
            playback.muted = this._muted;
        }
        playback.play();
        this.paused = false;
        this.ended = false;
        this.#fire('play');
        return Promise.resolve();
    }

    pause() {
        this.#playback?.pause();
        this.paused = true;
        this.#fire('pause');
    }

    load() {
        this.#playback?.stop();
        this.#playback = null;
    }

    addEventListener(type, listener) {
        if (!this.#listeners.has(type)) {
            this.#listeners.set(type, new Set());
        }
        this.#listeners.get(type).add(listener);
    }

    removeEventListener(type, listener) {
        this.#listeners.get(type)?.delete(listener);
    }
}

/// The system's message box, waiting for it, as a page's `alert` does.
function alert(message = '') {
    host.messageBoxNow('', String(message), 'info', 'ok');
}

function confirm(message = '') {
    const answer = host.messageBoxNow('', String(message), 'question', 'ok-cancel');
    return answer !== 'cancel' && answer !== 'no';
}

/// The main screen, as the web has it, which is in logical pixels.
function primary() {
    const one = desktop.displays.primary();
    if (one === null) {
        return { width: 0, height: 0, workWidth: 0, workHeight: 0, workX: 0, workY: 0 };
    }
    const logical = (value) => Math.round(value / one.scale);
    return {
        width: logical(one.width),
        height: logical(one.height),
        workWidth: logical(one.workWidth),
        workHeight: logical(one.workHeight),
        workX: logical(one.workX),
        workY: logical(one.workY),
    };
}

const screen = {
    get width() {
        return primary().width;
    },
    get height() {
        return primary().height;
    },
    get availWidth() {
        return primary().workWidth;
    },
    get availHeight() {
        return primary().workHeight;
    },
    get availLeft() {
        return primary().workX;
    },
    get availTop() {
        return primary().workY;
    },
    colorDepth: 24,
    pixelDepth: 24,
    orientation: { type: 'landscape-primary', angle: 0, addEventListener() {}, removeEventListener() {} },
};

/// `localStorage` and `sessionStorage` as the web has them, over a store. Values are strings there: what `storage` set that
/// isn't one is read as its JSON.
function webStorage(store, from) {
    const text = (value) => (value === undefined ? null : typeof value === 'string' ? value : JSON.stringify(value));
    const methods = {
        get length() {
            return store.keys().length;
        },
        key: (at) => store.keys()[at] ?? null,
        getItem: (key) => text(store.get(key)),
        setItem: (key, value) => store.set(key, String(value), from),
        removeItem: (key) => {
            store.delete(key, from);
        },
        clear: () => store.clear(from),
    };
    return new Proxy(methods, {
        get: (target, key) => (typeof key === 'symbol' || key in target ? target[key] : text(store.get(key)) ?? undefined),
        set(target, key, value) {
            target.setItem(key, value);
            return true;
        },
        has: (target, key) => key in target || store.has(key),
        deleteProperty(target, key) {
            target.removeItem(key);
            return true;
        },
        ownKeys: () => store.keys(),
        getOwnPropertyDescriptor(target, key) {
            const value = text(store.get(key));
            return value === null ? undefined : { value, enumerable: true, configurable: true, writable: true };
        },
    });
}

let local = null;
let session = null;

/// The app's own store, as `localStorage`. What `storage` changes is told to the page as a `storage` event, as a change from
/// another tab would be, which is how libraries that keep state there hear about it.
function localStorage() {
    if (local === null) {
        local = webStorage(require('rsui/desktop/storage').storage.open(), 'localStorage');
        desktop.storage.onChange(({ key, value, old, from }) => {
            if (from === 'localStorage' || typeof globalThis.dispatchEvent !== 'function') {
                return;
            }
            const text = (v) => (v === undefined ? null : typeof v === 'string' ? v : JSON.stringify(v));
            const event = typeof globalThis.StorageEvent === 'function'
                ? new globalThis.StorageEvent('storage', { key, newValue: text(value), oldValue: text(old), storageArea: local, url: '' })
                : Object.assign(new Event('storage'), { key, newValue: text(value), oldValue: text(old), storageArea: local, url: '' });
            globalThis.dispatchEvent(event);
        });
    }
    return local;
}

function sessionStorage() {
    session ??= webStorage(new Store(null), 'sessionStorage');
    return session;
}

/// `navigator.clipboard`, over `clipboard`.
const navigatorClipboard = {
    readText: () => desktop.clipboard.read().then((contents) => contents.text ?? ''),
    writeText: (text) => desktop.clipboard.write({ text: String(text) }),
    read: () => desktop.clipboard.read().then((contents) => [{
        types: contents.text === undefined ? [] : ['text/plain'],
        getType: (type) => Promise.resolve(new Blob([type === 'text/plain' ? contents.text ?? '' : ''], { type })),
    }]),
    async write(items) {
        for (const item of items) {
            if (item.types.includes('text/plain')) {
                const blob = await item.getType('text/plain');
                return desktop.clipboard.write({ text: await blob.text() });
            }
        }
    },
    addEventListener() {},
    removeEventListener() {},
};

/// `navigator.mediaDevices`, for what it can say without asking for the camera or the microphone: the devices that play and
/// record sound, as `audio.devices()` lists them, with the system's own first of each kind, as a browser lists them.
/// `getUserMedia` isn't here.
const deviceListeners = new Set();
let devicesHeard = false;
const navigatorMediaDevices = {
    ondevicechange: null,
    enumerateDevices() {
        const devices = [...desktop.audio.devices()].sort((one, other) => (one.kind === other.kind ? 0 : one.kind === 'output' ? -1 : 1) || Number(other.isDefault) - Number(one.isDefault));
        return Promise.resolve(devices.map(({ id, name, kind }) => {
            const info = { deviceId: id, groupId: '', kind: kind === 'output' ? 'audiooutput' : 'audioinput', label: name };
            return { ...info, toJSON: () => info };
        }));
    },
    getSupportedConstraints: () => ({}),
    getUserMedia: () => Promise.reject(Object.assign(new Error("there are no cameras or microphones here"), { name: "NotSupportedError" })),
    addEventListener(type, listener) {
        if (type !== 'devicechange' || typeof listener !== 'function') {
            return;
        }
        deviceListeners.add(listener);
        if (!devicesHeard) {
            devicesHeard = true;
            desktop.audio.onChange(() => {
                const event = { type: 'devicechange', target: navigatorMediaDevices };
                navigatorMediaDevices.ondevicechange?.(event);
                for (const one of [...deviceListeners]) {
                    one(event);
                }
            });
        }
    },
    removeEventListener(type, listener) {
        deviceListeners.delete(listener);
    },
};

/// What's playing, as `navigator.mediaSession.metadata` takes it.
class MediaMetadata {
    constructor({ title = '', artist = '', album = '', artwork = [] } = {}) {
        this.title = String(title);
        this.artist = String(artist);
        this.album = String(album);
        this.artwork = [...artwork];
    }
}

/// `navigator.mediaSession`, over `media`: what the media keys do, as `setActionHandler` says, and what's playing. The key
/// that plays and pauses is `play` or `pause` by `playbackState`, as in a browser.
const mediaHandlers = new Map();
let mediaHeard = false;
const fromKey = { play: 'play', pause: 'pause', next: 'nexttrack', previous: 'previoustrack', stop: 'stop' };
const navigatorMediaSession = {
    metadata: null,
    playbackState: 'none',
    setActionHandler(type, handler) {
        if (handler === null || handler === undefined) {
            mediaHandlers.delete(String(type));
            return;
        }
        mediaHandlers.set(String(type), handler);
        if (!mediaHeard) {
            mediaHeard = true;
            desktop.media.onKey((key) => {
                const wanted = key === 'playPause' ? (navigatorMediaSession.playbackState === 'playing' ? 'pause' : 'play') : fromKey[key];
                mediaHandlers.get(wanted)?.({ action: wanted });
            });
        }
    },
    setPositionState() {},
    setMicrophoneActive() {},
    setCameraActive() {},
};

module.exports = { Notification, Audio, alert, confirm, screen, localStorage, sessionStorage, navigatorClipboard, navigatorMediaDevices, navigatorMediaSession, MediaMetadata };
