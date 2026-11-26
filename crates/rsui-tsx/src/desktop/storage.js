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

// A caller's input is snapshotted on set. Keep that JSON until someone needs the value,
// so later saves can copy it and the store owns only one of the two representations.
class StoredJSON {
    constructor(text) {
        this.text = text;
    }
}

function storedValue(value) {
    if (typeof value === 'string') return value;
    const text = JSON.stringify(value);
    if (value !== null && typeof value === 'object' && text !== undefined) return new StoredJSON(text);
    return JSON.parse(text);
}

function storeText(values, encoded) {
    if (!encoded) return JSON.stringify(values);
    const parts = [];
    let group = null;
    const finishGroup = () => {
        if (group === null) return;
        const text = JSON.stringify(group);
        if (text !== '{}') parts.push(text.slice(1, -1));
        group = null;
    };
    // Serialize adjacent ordinary values together. This honors mutations through get(), named
    // toJSON calls, omitted values, and property ordering, while reusing unobserved snapshots.
    for (const key of Object.keys(values)) {
        const value = values[key];
        if (value instanceof StoredJSON) {
            finishGroup();
            parts.push(JSON.stringify(key) + ':' + value.text);
        } else {
            if (group === null) group = Object.create(null);
            group[key] = value;
        }
    }
    finishGroup();
    return '{' + parts.join(',') + '}';
}

/// One store. What's in it is read the first time it's asked for, and written straight away after each change, to another file
/// first, so that stopping halfway can't lose what's there.
class Store {
    #file;
    #items = null;
    #listeners = new Set();
    #lastLength = -1;
    #encoded = 0;

    constructor(file) {
        this.#file = file;
    }

    #load() {
        if (this.#items === null) {
            this.#items = new Map();
            if (this.#file !== null) {
                try {
                    const fs = require('fs');
                    const text = fs.readFileSync(this.#file, 'utf8');
                    const read = JSON.parse(text);
                    for (const [key, value] of Object.entries(read)) {
                        this.#items.set(key, value);
                    }
                    this.#lastLength = text.length;
                } catch {
                    // There's nothing there yet, or it isn't something this can read, so it starts empty.
                }
            }
        }
        return this.#items;
    }

    #commit(key, value) {
        const items = this.#load();
        const old = items.get(key);
        const encoded = key === null ? 0 : this.#encoded - Number(old instanceof StoredJSON) + Number(value instanceof StoredJSON);
        if (this.#file !== null) {
            let text = '{}';
            if (key !== null) {
                const values = Object.fromEntries(items);
                if (value === undefined) {
                    delete values[key];
                } else {
                    // An own data property even when the key happens to be "__proto__".
                    Object.defineProperty(values, key, { value, enumerable: true, configurable: true, writable: true });
                }
                text = storeText(values, encoded);
            }
            const maybeSame = old === value || (value instanceof StoredJSON && (
                old instanceof StoredJSON ? value.text === old.text : old !== null && typeof old === 'object'
            ));
            this.#write(text, maybeSame);
        }
        if (key === null) {
            items.clear();
            this.#encoded = 0;
        } else {
            // Serializing an exposed value can call toJSON, which can read other keys.
            this.#encoded += Number(value instanceof StoredJSON) - Number(items.get(key) instanceof StoredJSON);
            if (value === undefined) items.delete(key);
            else items.set(key, value);
        }
    }

    #write(text, maybeSame) {
        const fs = require('fs');
        if (maybeSame && text.length === this.#lastLength) {
            // An external replacement can have the same size and timestamp. Verify the
            // exact file before skipping a synchronous commit, without retaining its bytes.
            try {
                if (fs.readFileSync(this.#file, 'utf8') === text) return;
            } catch {
                // Recreate a missing file, or surface the write error below.
            }
        }
        fs.mkdirSync(require('path').dirname(this.#file), { recursive: true });
        const temp = this.#file + '.' + process.pid + '.new';
        try {
            fs.writeFileSync(temp, text);
            fs.renameSync(temp, this.#file);
        } catch (error) {
            try { fs.unlinkSync(temp); } catch {}
            throw error;
        }
        this.#lastLength = text.length;
    }

    #changed(key, value, old, from) {
        for (const listener of [...this.#listeners]) {
            listener({ key, value, old, from });
        }
    }

    /// What's kept for this key, or `undefined`.
    get(key) {
        key = String(key);
        const items = this.#load();
        const stored = items.get(key);
        if (!(stored instanceof StoredJSON)) return stored;
        const value = JSON.parse(stored.text);
        items.set(key, value);
        this.#encoded--;
        return value;
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
        const old = this.#listeners.size ? this.get(key) : undefined;
        this.#commit(key, storedValue(value));
        if (this.#listeners.size) this.#changed(key, this.get(key), old, from);
    }

    delete(key, from = 'storage') {
        key = String(key);
        const items = this.#load();
        if (!items.has(key)) {
            return false;
        }
        const old = this.#listeners.size ? this.get(key) : undefined;
        this.#commit(key, undefined);
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
        this.#commit(null, undefined);
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
