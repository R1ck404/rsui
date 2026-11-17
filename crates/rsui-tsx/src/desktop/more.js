// More parts of `rsui/desktop`: `secrets`, `taskbar`, `protocols`, `fileAssociations`, `system`, `network`, `drag` and `menus`.

const { host, action, forget, resolved, topic, pictureBytes, menuSpec } = require('rsui/desktop/core');

const secrets = {
    /// What's kept for this key, in Windows' Credential Manager where only this user can read it, or `undefined`.
    get: (key) => Promise.resolve(host.secretGet(String(key))),
    /// Keeps this text for the key. It can be up to 2560 bytes.
    set: (key, value) => Promise.resolve(host.secretSet(String(key), String(value))),
    /// Forgets what's kept for the key. It's whether there was anything.
    delete: (key) => Promise.resolve(host.secretDelete(String(key))),
};

/// The main window, or the one given, by its id or its controls.
const windowId = (win) => (win === undefined || win === null ? 0 : typeof win === 'number' ? win : win.id ?? 0);

const taskbar = {
    /// How far along something is, on the window's button on the taskbar, from 0 to 1, as `normal`, `paused`, `error` or
    /// `indeterminate`, or taken off with `none`.
    setProgress(value, state = 'normal', win) {
        host.changeWindow(windowId(win), 'progress', Math.min(1, Math.max(0, Number(value) || 0)), String(state));
    },
    /// A small picture over the window's button, as a count or a status, with words for screen readers, or `null` to take it off.
    setOverlayIcon(icon, description = '', win) {
        host.changeWindow(windowId(win), 'overlay', icon === null || icon === undefined ? null : pictureBytes(icon), String(description));
    },
    /// Has the window's button flash to say it wants the user: `{ critical: true }` until they come to it, which is what it
    /// does with nothing said, a few times otherwise, or `{ stop: true }`. Windows 11 stops the flashing then, but keeps the
    /// button lit until the window's next used, as it does once a few flashes are over.
    requestAttention({ critical = true, stop = false } = {}, win) {
        host.changeWindow(windowId(win), 'flash', stop ? 'stop' : critical ? 'critical' : 'informational');
    },
    /// What right-clicking the program's button on the taskbar shows, in place of what it put there before: `tasks` under
    /// Tasks, and each of `categories` under its `name`, with each item `{ title, args, icon, description }`. Choosing one starts
    /// this program again with `args` after it, which goes to the one that's running if it's the only one of it; see
    /// `app.singleInstance`. `icon` is an `.ico` file, or the program's own.
    setJumpList({ tasks = [], categories = [] } = {}) {
        const item = (one) => ({
            title: String(one?.title ?? ''),
            args: String(one?.args ?? ''),
            icon: one?.icon === undefined || one?.icon === null ? undefined : resolved(one.icon),
            description: String(one?.description ?? ''),
        });
        const list = {
            tasks: (tasks ?? []).map(item),
            categories: (categories ?? []).map((category) => ({ name: String(category?.name ?? ''), items: (category?.items ?? []).map(item) })),
        };
        host.jumpListSet(JSON.stringify(list));
    },
    /// Takes everything the program put in its jump list out of it.
    clearJumpList: () => host.jumpListClear(),
};

const protocols = {
    /// Says this program opens links that start `scheme:`, for this user. A link like that then starts it, or goes to the one
    /// that's running if it's the only one of it, which `app.onOpenUrl` hears.
    register: (scheme) => host.protocolRegister(String(scheme).replace(/:$/, '')),
    /// Says it doesn't any more, if it was this program that said it did.
    unregister: (scheme) => host.protocolUnregister(String(scheme).replace(/:$/, '')),
    /// Whether this program is what opens links that start `scheme:`.
    isRegistered: (scheme) => host.protocolIsRegistered(String(scheme).replace(/:$/, '')),
};

const fileAssociations = {
    /// Says this program can open files that end in `extension`, as `{ extension: 'md', description: 'Markdown', icon }`, for this
    /// user. It's one of what "Open with" offers for them. Windows only lets the user choose what opens them unasked, which
    /// `openDefaultAppsSettings` takes them to. Opening one starts this program with it, which `app.onOpenFiles` hears.
    register({ extension, description = '', icon } = {}) {
        if (typeof extension !== 'string' || extension === '') {
            throw new TypeError('fileAssociations.register needs an extension');
        }
        host.fileKindRegister(extension, String(description || extension.toUpperCase() + ' file'), icon === undefined ? undefined : resolved(icon));
    },
    unregister: (extension) => host.fileKindUnregister(String(extension)),
    /// Opens the system's settings for which programs open which kinds of file.
    openDefaultAppsSettings: () => Promise.resolve(host.openDefaultApps()),
};

const systemChanges = topic((tell) => host.systemOnChange(action(tell)));

const system = {
    /// The language and region the user has set, like `en-US`.
    locale: () => host.systemInfo().locale,
    /// The user's time zone, as its IANA name, like `Europe/Amsterdam`, as Windows has it, or as `Intl` guesses it from the time
    /// now where Windows can't say, which can't tell zones with the same time apart.
    timeZone: () => host.systemInfo().timeZone ?? new Intl.DateTimeFormat().resolvedOptions().timeZone,
    /// How long since the user last used the keyboard or the mouse, in milliseconds.
    idleTime: () => host.systemInfo().idleTime,
    /// The color the user's chosen for Windows, as `#rrggbb`, if there is one.
    accentColor: () => host.systemInfo().accentColor ?? null,
    /// What the user's asked of programs to make them easier to use: `{ reducedMotion, highContrast, textScale }`.
    accessibility() {
        const { reducedMotion, highContrast, textScale } = host.systemInfo();
        return { reducedMotion, highContrast, textScale };
    },
    /// Runs when the user changes a setting of the system's: its language, its colors, or what makes it easier to use, with what
    /// all of those are now. What it gives back stops it.
    onChange: (run) => systemChanges.on(() => run({ locale: system.locale(), accentColor: system.accentColor(), accessibility: system.accessibility() })),
};

const networkChanges = topic((tell) => host.networkOnChange(action(() => tell(host.networkStatus()))));

const network = {
    /// `{ online, localOnly, metered }`: whether the internet can be reached, only this network, and whether it costs by how much
    /// is used.
    status: () => host.networkStatus(),
    /// Runs with the status when whether the internet can be reached changes. What it gives back stops it.
    onChange: (run) => networkChanges.on(run),
};

const drag = {
    /// Starts dragging these files out of the program, as Explorer picks them up, to drop in another. Call it while a button's
    /// held, as from `onDrag`'s start. It's what was done with them: `copy`, `move`, `link` or `none`.
    startFiles(paths) {
        try {
            return Promise.resolve(host.dragFiles((Array.isArray(paths) ? paths : [paths]).map(resolved)));
        } catch (error) {
            return Promise.reject(error);
        }
    },
};

let popupActions = [];

const menus = {
    /// Shows a menu where the pointer is, or at `{ x, y }` on the screen in physical pixels, as the menu on something that's
    /// clicked with the other button is. Each item is as a tray's menu's are.
    popup(items, at) {
        forget(popupActions);
        popupActions = [];
        host.popupMenu(menuSpec(items, popupActions), at?.x, at?.y);
    },
};

module.exports = { secrets, taskbar, protocols, fileAssociations, system, network, drag, menus };
