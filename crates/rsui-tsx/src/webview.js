// What draws `<webview>`: web pages from elsewhere, in a real browser view. rsui draws an app's own UI itself; someone else's
// site is written for a browser, so it gets one: WebView2, Edge's runtime, through `webview.rs`. Nothing of it is loaded
// until an app has a `<webview>`, and the browser only starts once one is on screen.
//
// With `--do`, `--agent` and `--png` nothing's started: a load comes back as one that worked, titled with the address's host,
// and the `webview` steps make the page do the rest. What's here is the same either way.

const React = require('react');
const { createElement, useEffect, useState } = React;
const { host, action } = require('rsui/desktop/core');

const views = new Map();
let nextId = 1;
let heard = 0;

/// The number what happens in every view is told to, made the first time one's opened.
function hearing() {
    if (heard === 0) {
        heard = action((said) => {
            let event;
            try {
                event = JSON.parse(said);
            } catch {
                return;
            }
            views.get(event.id)?.hear(event);
        });
    }
    return heard;
}

/// Finds the page's icon, the best of what its `<link rel=icon>`s say or `/favicon.ico`, and sends it back drawn as a 64-pixel
/// PNG, which every picture the page can show can be drawn as, whatever it was: `.ico` and `.svg` included. One from another
/// site that doesn't allow it to be read is skipped for the next.
const ICON_SCRIPT = `(async () => {
    const tried = new Set();
    let shown = null;
    const loaded = document.readyState === 'complete' ? null : new Promise(done => window.addEventListener('load', done, { once: true }));
    const find = async () => {
        const links = [...document.querySelectorAll('link[rel~="icon" i], link[rel="apple-touch-icon" i]')];
        const size = (link) => Math.max(0, ...String(link.getAttribute('sizes') || '').split(/\\s+/).map((one) => parseInt(one, 10) || 0));
        const fit = (link) => /svg/i.test(link.type || link.href) ? 1000 : size(link) >= 32 && size(link) <= 192 ? 500 + size(link) : size(link);
        links.sort((a, b) => fit(b) - fit(a));
        const tries = links.map((link) => link.href);
        if (/^https?:/.test(location.protocol)) tries.push(location.origin + '/favicon.ico');
        for (const href of tries) {
            if (href === shown) return;
            if (tried.has(href)) continue;
            tried.add(href);
            try {
                const picture = new Image();
                picture.crossOrigin = new URL(href).origin === location.origin ? '' : 'anonymous';
                await new Promise((done, failed) => { picture.onload = done; picture.onerror = failed; picture.src = href; });
                const canvas = document.createElement('canvas');
                canvas.width = canvas.height = 64;
                canvas.getContext('2d').drawImage(picture, 0, 0, 64, 64);
                window.chrome.webview.postMessage({ rsuiIcon: canvas.toDataURL('image/png'), at: location.href });
                shown = href;
                return;
            } catch {}
        }
    };
    await find();
    if (loaded) {
        await loaded;
        await find();
    }
})(); null`;

/// What a view says about itself, as `onState` is given it.
function emptyState() {
    return { url: '', title: '', loading: false, canGoBack: false, canGoForward: false, icon: '', error: null, unavailable: null, fullScreen: false };
}

/// One browser view. What its element's handlers are is set each time it renders, so what's told is always the latest.
class View {
    constructor({ profile, private: inPrivate, userAgent, keys }) {
        this.id = nextId++;
        this.state = emptyState();
        this.on = {};
        this.closed = false;
        this.asked = '';
        this.calls = new Map();
        this.nextCall = 1;
        views.set(this.id, this);
        this.keys = keyList(keys);
        const settings = {
            profile: typeof profile === 'string' ? profile : '',
            private: inPrivate === true,
            userAgent: typeof userAgent === 'string' ? userAgent : '',
            keys: this.keys,
        };
        host.webviewOpen(this.id, hearing(), JSON.stringify(settings));
    }

    #set(changes) {
        this.state = { ...this.state, ...changes };
        this.on.onState?.(this.state);
    }

    #history() {
        return { canGoBack: host.webviewAsk(this.id, 'back') === true, canGoForward: host.webviewAsk(this.id, 'forward') === true };
    }

    #do(what, given = '', call = 0) {
        if (!this.closed) {
            host.webviewDo(this.id, what, String(given), call);
        }
    }

    /// What `webview.rs` says happened.
    hear(event) {
        switch (event.type) {
            case 'unavailable':
                this.#set({ unavailable: String(event.message ?? ''), loading: false });
                this.on.onUnavailable?.(this.state.unavailable);
                break;
            case 'navigating':
                this.#set({ loading: true, error: null, url: event.url });
                break;
            case 'navigated':
                this.iconRequest = null;
                this.#set({ url: event.url, icon: '', ...this.#history() });
                break;
            case 'inPage':
                this.#set({ url: event.url, ...this.#history() });
                break;
            case 'history':
                this.#set(this.#history());
                break;
            case 'title':
                this.#set({ title: String(event.title ?? '') });
                break;
            case 'domReady':
                this.#findIcon();
                break;
            case 'finished':
                this.#set({ loading: false, ...this.#history() });
                if (this.state.icon === '') {
                    this.#findIcon();
                }
                break;
            case 'failed':
                // A stop, or going somewhere else before this one arrived, isn't something that went wrong.
                if (event.code === -3) {
                    this.#set({ loading: false });
                } else {
                    this.#set({ loading: false, error: { url: event.url, code: event.code, description: event.description }, url: event.url, title: '', icon: '' });
                }
                break;
            case 'newWindow':
                this.on.onNewWindow?.({ url: event.url, user: event.user === true });
                break;
            case 'key':
                this.on.onKey?.(event.key);
                break;
            case 'fullScreen':
                this.#set({ fullScreen: event.on === true });
                break;
            case 'still':
                // A picture of the page, drawn where it is while something see-through is over it, which a window can't be.
                this.onStill?.(typeof event.url === 'string' ? event.url : null);
                break;
            case 'message': {
                let value = event.json;
                try {
                    value = JSON.parse(event.json);
                } catch {
                    // A message that isn't JSON is given as it is.
                }
                if (value !== null && typeof value === 'object' && typeof value.rsuiIcon === 'string') {
                    if (value.at === this.state.url || this.state.icon === '') {
                        this.#set({ icon: value.rsuiIcon });
                    }
                    break;
                }
                this.on.onMessage?.(value);
                break;
            }
            case 'focus':
                this.on.onFocus?.(event.on === true);
                break;
            case 'close':
                this.on.onClose?.();
                break;
            case 'crashed':
                this.#set({ loading: false, error: { url: this.state.url, code: -1, description: 'The page stopped working' } });
                break;
            case 'script': {
                const waiting = this.calls.get(event.call);
                if (waiting !== undefined) {
                    this.calls.delete(event.call);
                    if (event.error !== undefined) {
                        waiting.reject(new Error(event.error));
                    } else {
                        let value = null;
                        try {
                            value = JSON.parse(event.json);
                        } catch {
                            value = event.json;
                        }
                        waiting.resolve(value);
                    }
                }
                break;
            }
        }
    }

    #findIcon() {
        if (this.iconRequest) return;
        const request = this.run(ICON_SCRIPT);
        this.iconRequest = request;
        request.catch(() => {
            if (this.iconRequest === request) this.iconRequest = null;
        });
    }

    go(url) {
        this.asked = String(url);
        this.#set({ url: this.asked, loading: true, error: null });
        this.#do('navigate', this.asked);
    }

    back() {
        this.#do('back');
    }

    forward() {
        this.#do('forward');
    }

    reload() {
        this.#do('reload');
    }

    stop() {
        this.#do('stop');
    }

    focus() {
        this.#do('focus');
    }

    /// Gives the keyboard back to the app's window, if the page has it.
    blur() {
        this.#do('blur');
    }

    devTools() {
        this.#do('devTools');
    }

    zoom(factor) {
        this.#do('zoom', Number(factor) || 1);
    }

    setKeys(keys) {
        const next = keyList(keys);
        if (next.length === this.keys.length && next.every((key, i) => key === this.keys[i])) return;
        this.keys = next;
        this.#do('keys', JSON.stringify(next));
    }

    post(value) {
        this.#do('post', JSON.stringify(value ?? null));
    }

    /// Forgets the cookies of this view's profile.
    clearCookies() {
        this.#do('clearCookies');
    }

    /// Runs `script` in the page, and gives what it comes to, as JSON would have it.
    run(script) {
        if (this.closed) {
            return Promise.reject(new Error('the view has closed'));
        }
        const call = this.nextCall++;
        return new Promise((resolve, reject) => {
            this.calls.set(call, { resolve, reject });
            this.#do('script', script, call);
        });
    }

    close() {
        if (this.closed) {
            return;
        }
        this.closed = true;
        for (const waiting of this.calls.values()) {
            waiting.reject(new Error('the view has closed'));
        }
        this.calls.clear();
        views.delete(this.id);
        host.webviewClose(this.id);
    }
}

function keyList(keys) {
    return Array.isArray(keys) ? keys.map(String) : [];
}

/// The view an element has, made the first time it renders and closed when it goes, told what's handed to it each time.
function useView(props) {
    const { src, profile, private: inPrivate, userAgent, keys, zoom, ref, onState, onNewWindow, onKey, onMessage, onFocus, onClose, onUnavailable } = props;
    const [view] = useState(() => new View({ profile, private: inPrivate, userAgent, keys }));
    const [still, setStill] = useState(null);
    view.on = { onState, onNewWindow, onKey, onMessage, onFocus, onClose, onUnavailable };
    view.onStill = setStill;
    view.still = still;
    useEffect(() => () => view.close(), []);
    useEffect(() => view.setKeys(keys), [keys]);
    useEffect(() => {
        if (typeof src === 'string' && src !== '' && src !== view.asked && src !== view.state.url) {
            view.go(src);
        }
    }, [src]);
    useEffect(() => {
        if (typeof zoom === 'number') {
            view.zoom(zoom);
        }
    }, [zoom]);
    useEffect(() => {
        if (ref == null) {
            return undefined;
        }
        if (typeof ref === 'function') {
            ref(view);
            return () => ref(null);
        }
        ref.current = view;
        return () => {
            ref.current = null;
        };
    }, [ref]);
    return view;
}

/// What's the view's own rather than its element's.
const OWN = new Set(['children', 'src', 'profile', 'private', 'userAgent', 'keys', 'zoom', 'ref', 'onState', 'onNewWindow', 'onKey', 'onMessage',
    'onFocus', 'onClose', 'onUnavailable']);

/// What the element is given: its own props, and the view's number. While it shows a still picture of the page, it says so
/// (`embed_still`), for the view to be cut where something see-through is over it, and the picture fills it.
function elementProps(props, view) {
    const out = { embed: view.id };
    if (view.still !== null) {
        out.embed_still = true;
    }
    for (const name in props) {
        if (!OWN.has(name)) {
            out[name] = props[name];
        }
    }
    return out;
}

/// `<webview>`: a web page from elsewhere, where this element is, placed and sized like a `<group>`. What's in it shows until
/// the page has started arriving, and instead of it if it can't be shown. It's given back through `ref` as a handle with `go`,
/// `back`, `forward`, `reload`, `stop`, `focus`, `blur`, `devTools`, `zoom`, `post`, `run` and `clearCookies`.
function WebView(props) {
    const view = useView(props);
    const shown = view.still === null ? props.children : createElement('image', { src: view.still, fit: 'fill', grow: 1, alignSelf: 'stretch' });
    return createElement('group', elementProps(props, view), shown);
}

/// `<webview>` in a page, placed and sized by the page's CSS like a `<div>`.
function PageWebView(props) {
    const view = useView(props);
    if (view.still === null) {
        return createElement('div', elementProps(props, view), props.children);
    }
    const style = { ...props.style, backgroundImage: `url("${view.still}")`, backgroundSize: '100% 100%', backgroundRepeat: 'no-repeat' };
    return createElement('div', { ...elementProps(props, view), style });
}
PageWebView.__webTag = 'webview';

/// The version of WebView2 on this computer, or '' if there's none.
function runtime() {
    return host.webviewRuntime();
}

module.exports = { WebView, PageWebView, runtime };
