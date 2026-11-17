// `app` in `rsui/desktop`: this program as a whole, from what it's called and where its files go to starting at login, being
// the only one of it running, and the links and files it's asked to open.

const { host, action, forget, resolved } = require('rsui/desktop/core');

let secondAction = 0;
let secondHeard = null;

/// Whether this is a link of a scheme a program can be said to open, rather than a file's path, as `C:\notes.txt` isn't.
const isLink = (given) => /^[a-z][a-z0-9+.-]*:/i.test(given) && !/^[a-z]:[\\/]/i.test(given);

const urlListeners = new Set();
const fileListeners = new Set();
let startedSaid = false;

/// Tells what's listening about links and files the program's been asked to open, as it was started with or as another of it was.
function opened(given) {
    const links = given.filter(isLink);
    const files = given.filter((one) => !isLink(one) && !one.startsWith('-')).map((one) => resolved(one));
    for (const link of links) {
        for (const listener of [...urlListeners]) {
            listener(link);
        }
    }
    if (files.length > 0) {
        for (const listener of [...fileListeners]) {
            listener(files);
        }
    }
}

/// What it was started with is told to the first that listens, once, as that's what it was for.
function sayWhatItStartedWith() {
    if (startedSaid) {
        return;
    }
    startedSaid = true;
    queueMicrotask(() => opened(host.startedWith()));
}

/// This program's `package.json`, as it's found from where its main file is, if it has one.
function packageOf() {
    const fs = require('fs');
    const path = require('path');
    for (let folder = path.dirname(host.appFile()); ; folder = path.dirname(folder)) {
        try {
            return JSON.parse(fs.readFileSync(path.join(folder, 'package.json'), 'utf8'));
        } catch {
            // Not here.
        }
        if (path.dirname(folder) === folder) {
            return null;
        }
    }
}

const quitListeners = new Set();

const app = {
    /// What this program is: `{ name, version, packed, path }`, from its `package.json` if it has one.
    getInfo() {
        const made = packageOf();
        return {
            name: made?.productName ?? made?.name ?? host.appName(),
            version: typeof made?.version === 'string' ? made.version : '0.0.0',
            packed: host.isPacked(),
            path: host.appPaths().exe,
        };
    },
    /// Where things are: `home`, `appData`, `localAppData`, `userData` (this program's own), `temp`, `desktop`, `documents`,
    /// `downloads`, `music`, `pictures`, `videos`, `exe` and `app`, its main file.
    getPaths: () => host.appPaths(),
    /// Runs with `{ preventDefault() }` before the program ends because `quit()` was called, which can stop it.
    onBeforeQuit(run) {
        quitListeners.add(run);
        return () => quitListeners.delete(run);
    },
    /// Runs with each link the program's been asked to open: what it was started with, and what another start of it was, once
    /// it's the only one of it running. See `protocols.register`.
    onOpenUrl(run) {
        urlListeners.add(run);
        sayWhatItStartedWith();
        return () => urlListeners.delete(run);
    },
    /// Runs with the files the program's been asked to open, the same way. See `fileAssociations.register`.
    onOpenFiles(run) {
        fileListeners.add(run);
        sayWhatItStartedWith();
        return () => fileListeners.delete(run);
    },
    /// Says these files were opened, for the system's lists of what's been used lately, as in the program's jump list.
    setRecentDocuments: (paths) => host.recentDocuments((Array.isArray(paths) ? paths : [paths]).map(resolved)),
    clearRecentDocuments: () => host.clearRecentDocuments(),
    /// Says whether this program starts when someone logs in.
    setStartAtLogin(on = true, { args = [] } = {}) {
        host.setStartAtLogin(on === true, args.map(String));
    },
    startsAtLogin: () => host.startsAtLogin(),
    /// Says whether this is the only one of this program running. If it isn't, it should end, and what it was started with has gone
    /// to the one that's running, to `onSecond`, and to `onOpenUrl` and `onOpenFiles`.
    singleInstance(onSecond) {
        forget([secondAction]);
        secondHeard = onSecond ?? null;
        secondAction = action((text) => {
            const given = text === '' ? [] : String(text).split('\n');
            secondHeard?.(given);
            opened(given);
        });
        return host.singleInstance(secondAction);
    },
    /// Starts this program again, and closes this one.
    relaunch: () => host.relaunch(),
    /// Closes the main window, which ends the program, unless something that's listening says not to.
    quit() {
        let stopped = false;
        const event = { preventDefault: () => (stopped = true) };
        for (const listener of [...quitListeners]) {
            listener(event);
        }
        if (!stopped) {
            host.closeWindow(0);
        }
        return !stopped;
    },
};

module.exports = { app };
