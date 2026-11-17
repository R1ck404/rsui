// The small parts of `rsui/desktop`, each its own object: `shell`, `clipboard`, `dialogs`, `notifications`, `tray`, `shortcuts`,
// `displays` and `power`. They're together as they're each only a few lines over what Rust does.

const { host, rsui, action, forget, resolved, topic, pictureBytes, menuSpec } = require('rsui/desktop/core');
const { useEffect, useRef } = rsui;

/// Whether this is a web address or a mail one, rather than a file's path, as `C:\notes.txt` isn't.
const isAddress = (target) => /^[a-z][a-z0-9+.-]*:/i.test(target) && !/^[a-z]:[\\/]/i.test(target);

// Each is a promise of it being done, which the system does on a thread of its own. What isn't there throws straight away.
const shell = {
    /// Opens a web page in the browser, an address in the mail program, or a file or folder with what the system has for it.
    /// Only http, https and mailto addresses can be opened this way, as what a program shows can come from anywhere.
    open(target) {
        const text = String(target);
        return Promise.resolve(isAddress(text) ? host.openUrl(text) : host.shellOpenPath(resolved(text)));
    },
    /// Shows a file in its folder in Explorer, picked out.
    reveal: (path) => Promise.resolve(host.shellReveal(resolved(path))),
    /// Puts files in the Recycle Bin, from where they can be brought back.
    trash: (paths) => Promise.resolve(host.shellTrash((Array.isArray(paths) ? paths : [paths]).map(resolved))),
    /// Asks which program to open a file with.
    openWith: (path) => Promise.resolve(host.shellOpenWith(resolved(path))),
    /// Shows the system's properties of a file or folder.
    properties: (path) => Promise.resolve(host.shellProperties(resolved(path))),
};

const clipboardChanges = topic((tell) => host.clipboardOnChange(action(tell)));

/// A PNG's bytes, from the hex Rust gives them as.
const fromHex = (hex) => Uint8Array.from(hex.match(/../g) ?? [], (pair) => parseInt(pair, 16));
const toHex = (bytes) => Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');

const waitingForPaste = [];
rsui.__onPasted((text) => {
    for (const resolve of waitingForPaste.splice(0)) {
        resolve(text);
    }
});

/// The text on the clipboard that what's typed into is pasted from, which is the system's, or, while an app's being checked,
/// the one the steps have.
const pastedText = () => new Promise((resolve) => {
    waitingForPaste.push(resolve);
    host.paste();
});

const clipboard = {
    /// What's on the clipboard: `{ text, html, files, image }`, each that's there, `image` being a PNG's bytes.
    async read() {
        const got = host.clipboardRead();
        const out = {};
        const text = host.desktopReal() ? got.text : await pastedText();
        if (text !== undefined && text !== null && (host.desktopReal() || text !== '' || got.html === undefined)) {
            out.text = text;
        }
        if (got.html !== undefined) {
            out.html = got.html;
        }
        if (got.files !== undefined) {
            out.files = got.files;
        }
        if (got.pngHex !== undefined) {
            out.image = fromHex(got.pngHex);
        }
        return out;
    },
    /// Puts this on the clipboard in place of what's there: `{ text, html, files, image }`, each that's given, `image` as a PNG's bytes.
    write(contents) {
        const given = typeof contents === 'string' ? { text: contents } : contents ?? {};
        if (given.text === undefined && given.html === undefined && given.files === undefined && given.image === undefined) {
            return Promise.reject(new TypeError('clipboard.write needs text, html, files or an image'));
        }
        try {
            host.clipboardWrite(
                given.text === undefined ? undefined : String(given.text),
                given.html === undefined ? undefined : String(given.html),
                (given.files ?? []).map(resolved),
                given.image === undefined ? undefined : toHex(given.image),
            );
            if (!host.desktopReal() && given.text !== undefined) {
                // What's typed into pastes it too.
                host.copy(String(given.text));
            }
            return Promise.resolve();
        } catch (error) {
            return Promise.reject(error);
        }
    },
    clear: () => Promise.resolve(host.clipboardClear()),
    /// What kinds of thing are on it now: `text/plain`, `text/html`, `image/png`, `files`, and others by the name they have.
    formats: () => host.clipboardFormats(),
    /// Runs whenever something's put on the clipboard, by this program or another. What it gives back stops it.
    onChange: (run) => clipboardChanges.on(run),
};

/// `{ name: 'Pictures', extensions: ['png', 'jpg'] }` is what Windows wants as `*.png;*.jpg`.
function askForPath(kind, { title, start, filters = [] } = {}) {
    const patterns = filters.map(({ name, extensions }) => [name, extensions.map((extension) => '*.' + extension).join(';')]);
    return host.dialog(kind, title, start, patterns);
}

const dialogs = {
    /// Asks for a file to open, or a folder with `{ folders: true }`. It's the path, or `null` if none was chosen.
    open(options = {}) {
        return askForPath(options.folders === true ? 'folder' : 'open', options);
    },
    /// Asks where to save. It's the path, or `null`.
    save(options) {
        return askForPath('save', options);
    },
    /// Asks with the system's message box: 'ok', 'cancel', 'yes', 'no' or 'retry'.
    message({ title = '', message = '', kind = 'info', buttons = 'ok' } = {}) {
        return host.messageBoxLater(String(title), String(message), kind, buttons);
    },
    /// Asks for a color with the system's color dialog, starting from `initial`, as `#rrggbb`. It's the color chosen, as
    /// `#rrggbb`, or `null`. While an app's being checked, the `dialog` step answers it.
    pickColor({ initial } = {}) {
        try {
            return host.pickColor(initial === undefined || initial === null ? '' : String(initial));
        } catch (error) {
            return Promise.reject(error);
        }
    },
    /// Asks for a font with the system's font dialog, starting from `initial`. It's `{ family, size, weight, italic, color }`,
    /// with `size` in points, `weight` from 100 to 900 and `color` as `#rrggbb`, or `null`. While an app's being checked, the
    /// `dialog` step answers it with JSON of that.
    pickFont({ initial } = {}) {
        let asked;
        try {
            asked = host.pickFont(initial === undefined || initial === null ? '' : JSON.stringify(initial));
        } catch (error) {
            return Promise.reject(error);
        }
        return asked.then((text) => {
            if (text === null || text === undefined) {
                return null;
            }
            const font = JSON.parse(text);
            return {
                family: String(font.family ?? ''),
                size: Number(font.size ?? 12),
                weight: Number(font.weight ?? 400),
                italic: font.italic === true,
                color: String(font.color ?? '#000000'),
            };
        });
    },
};

// Notifications. The system shows one at a time, as a balloon from the tray, so showing another takes the place of the last.
let notificationAction = 0;
let nextNotification = 1;
let showing = null;
const clickListeners = new Set();
const actionListeners = new Set();

function clicked(id, own) {
    if (showing?.id === id) {
        showing = null;
    }
    own?.();
    for (const listener of [...clickListeners]) {
        listener({ id });
    }
}

const notifications = {
    /// Shows a notification, in place of any that's showing. It's the notification's id.
    show({ title = '', body = '', onClick } = {}) {
        const id = nextNotification++;
        forget([notificationAction]);
        notificationAction = action(() => clicked(id, onClick));
        host.notify(String(title), String(body), notificationAction);
        showing = { id, title: String(title), body: String(body), onClick };
        return id;
    },
    /// Changes what it says, if it's still the one showing.
    update(id, { title, body } = {}) {
        if (showing?.id !== id) {
            return false;
        }
        const next = { title: title ?? showing.title, body: body ?? showing.body };
        forget([notificationAction]);
        const own = showing.onClick;
        notificationAction = action(() => clicked(id, own));
        host.notify(next.title, next.body, notificationAction);
        Object.assign(showing, next);
        return true;
    },
    /// Takes it away, if it's still the one showing.
    dismiss(id) {
        if (showing?.id === id) {
            notifications.dismissAll();
        }
    },
    dismissAll() {
        forget([notificationAction]);
        notificationAction = 0;
        showing = null;
        host.notificationDismiss();
    },
    /// Runs with `{ id }` when any notification's clicked. What it gives back stops it.
    onClick(run) {
        clickListeners.add(run);
        return () => clickListeners.delete(run);
    },
    /// Runs with `{ id, action }` when one of a notification's buttons is pressed. The system's notifications that come from the
    /// tray have no buttons, so this is never told yet. What it gives back stops it.
    onAction(run) {
        actionListeners.add(run);
        return () => actionListeners.delete(run);
    },
};

let trayActions = [];

const tray = {
    /// Puts an icon in the tray, or changes the one that's there. Clicking on it with the other button opens its menu.
    set({ tooltip = '', icon, onClick, menu = [] } = {}) {
        forget(trayActions);
        trayActions = [];
        const bytes = pictureBytes(icon);
        const click = action(onClick);
        if (click !== 0) {
            trayActions.push(click);
        }
        host.setTray(String(tooltip), bytes, click, menuSpec(menu, trayActions));
    },
    remove() {
        forget(trayActions);
        trayActions = [];
        host.removeTray();
    },
    /// Where the icon is on the screen, in physical pixels, as `{ x, y, width, height }`, to put something next to it, or `null`.
    getBounds: () => host.trayBounds(),
};

/// Runs this when these keys are pressed, like `ctrl+shift+k`, whatever's being used at the time. It throws if something else has
/// them. What it gives back stops it.
function register(keys, run) {
    const id = action(run);
    let handle;
    try {
        handle = host.globalShortcut(String(keys), id);
    } catch (error) {
        forget([id]);
        throw error;
    }
    return function stop() {
        if (handle !== null) {
            host.removeGlobalShortcut(handle);
            forget([id]);
            handle = null;
        }
    };
}

const shortcuts = {
    register,
    /// `register`, for as long as this component's there.
    useShortcut(keys, run) {
        const latest = useRef(run);
        latest.current = run;
        useEffect(() => register(keys, () => latest.current()), [keys]);
    },
};

const displayChanges = topic((tell) => host.displaysOnChange(action(tell)));

const displays = {
    /// The screens there are, in physical pixels.
    list: () => host.displays(),
    /// Where the pointer is on the screen, in physical pixels.
    pointer: () => host.pointerOnScreen(),
    /// The main screen.
    primary() {
        const list = host.displays();
        return list.find((display) => display.primary) ?? list[0] ?? null;
    },
    /// The screen a point is on, in physical pixels, or the nearest one.
    forPoint: (x, y) => host.displayAt(Number(x), Number(y)),
    /// The screen most of a window is on, by its id, or the main window's.
    forWindow: (windowId = 0) => host.displayOfWindow(Number(windowId)),
    /// Runs when a screen's added, taken away or changed. What it gives back stops it.
    onChange: (run) => displayChanges.on(() => run(host.displays())),
};

const powerTopic = topic((tell) => host.onPower(action(tell)));

const power = {
    /// Stops the machine going to sleep and the screen going off, until it's turned off.
    keepAwake: (on = true) => host.keepAwake(on === true),
    /// `{ onBattery, level, charging }`, with `level` from 0 to 1, or `null` with no battery.
    battery: () => host.battery(),
    /// Runs with 'suspend', 'resume' or 'power-changed'. What it gives back stops it.
    onChange: (run) => powerTopic.on(run),
};

module.exports = { shell, clipboard, dialogs, notifications, tray, shortcuts, displays, power };
