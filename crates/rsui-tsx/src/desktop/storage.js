// `storage` in `rsui/desktop`: what the app keeps between runs, as a JSON file in its data folder. `localStorage` is the same store,
// seen as a page sees it, so what either sets the other reads.

const { host } = require('rsui/desktop/core');

function folder() {
    if (process.env.RSUI_STORAGE_FILE) {
        return require('path').dirname(process.env.RSUI_STORAGE_FILE);
    }
    return require('path').join(process.env.APPDATA ?? '.', 'rsui-storage');
}

function fileFor(name) {
    if (name === null && process.env.RSUI_STORAGE_FILE) {
        return process.env.RSUI_STORAGE_FILE;
    }
    const base = host.appName() + (name === null ? '' : '.' + name);
    return require('path').join(folder(), base + '.json');
}

/// One store. What's in it is read the first time it's asked for, and written straight away after each change, to another file
/// first, so that stopping halfway can't lose what's there.
class Store {
    #file;
    #items = null;
    #listeners = new Set();

    constructor(file) {
        this.#file = file;
    }

    #load() {
        if (this.#items === null) {
            this.#items = new Map();
            if (this.#file !== null) {
                try {
                    const read = JSON.parse(require('fs').readFileSync(this.#file, 'utf8'));
                    for (const [key, value] of Object.entries(read)) {
                        this.#items.set(key, value);
                    }
                } catch {
                    // There's nothing there yet, or it isn't something this can read, so it starts empty.
                }
            }
        }
        return this.#items;
    }

    #keep() {
        if (this.#file === null) {
            return;
        }
        const fs = require('fs');
        fs.mkdirSync(require('path').dirname(this.#file), { recursive: true });
        fs.writeFileSync(this.#file + '.new', JSON.stringify(Object.fromEntries(this.#load())));
        fs.renameSync(this.#file + '.new', this.#file);
    }

    #changed(key, value, old, from) {
        for (const listener of [...this.#listeners]) {
            listener({ key, value, old, from });
        }
    }

    /// What's kept for this key, or `undefined`.
    get(key) {
        return this.#load().get(String(key));
    }

    has(key) {
        return this.#load().has(String(key));
    }

    /// Keeps this for the key. It has to be something JSON can hold.
    set(key, value, from = 'storage') {
        if (value === undefined) {
            return this.delete(key, from);
        }
        key = String(key);
        const items = this.#load();
        const old = items.get(key);
        // What JSON makes of it is what's kept, so what comes back is the same after the program starts again.
        const kept = typeof value === 'string' ? value : JSON.parse(JSON.stringify(value));
        items.set(key, kept);
        this.#keep();
        this.#changed(key, kept, old, from);
    }

    delete(key, from = 'storage') {
        key = String(key);
        const items = this.#load();
        if (!items.has(key)) {
            return false;
        }
        const old = items.get(key);
        items.delete(key);
        this.#keep();
        this.#changed(key, undefined, old, from);
        return true;
    }

    keys() {
        return [...this.#load().keys()];
    }

    clear(from = 'storage') {
        const items = this.#load();
        if (items.size === 0) {
            return;
        }
        items.clear();
        this.#keep();
        this.#changed(null, undefined, undefined, from);
    }

    /// Runs with `{ key, value, old, from }` after each change, `from` being 'storage' or 'localStorage'. `key` is null for
    /// `clear()`. What it gives back stops it.
    onChange(run) {
        this.#listeners.add(run);
        return () => this.#listeners.delete(run);
    }
}

const opened = new Map();

function open(name = null) {
    if (name !== null && !/^[A-Za-z0-9_.-]+$/.test(String(name))) {
        throw new TypeError('a store is named with letters, digits, `_`, `-` and `.`: ' + name);
    }
    const key = name === null ? null : String(name);
    if (!opened.has(key)) {
        opened.set(key, new Store(fileFor(key)));
    }
    return opened.get(key);
}

const main = () => open(null);

/// The app's own store, and `open(name)` for others.
const storage = {
    get: (key) => main().get(key),
    has: (key) => main().has(key),
    set: (key, value) => main().set(key, value),
    delete: (key) => main().delete(key),
    keys: () => main().keys(),
    clear: () => main().clear(),
    onChange: (run) => main().onChange(run),
    open,
};

module.exports = { storage, Store };
