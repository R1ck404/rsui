// What every part of `rsui/desktop` shares: the numbers that what the system says back is sent to, and finding the app's files.
// With `--do` and `--agent` nothing really happens, and it's said instead; the steps make what the system would say back happen.

const rsui = require('rsui');
const host = rsui.__host();

const actions = new Map();
let nextAction = 1;

/// A number for the system to say back to, which runs this. It's 0 for nothing.
function action(run) {
    if (typeof run !== 'function') {
        return 0;
    }
    const id = nextAction++;
    actions.set(id, run);
    return id;
}

function forget(ids) {
    for (const id of ids) {
        actions.delete(id);
    }
}

rsui.__desktopEvents((id, value) => {
    const run = actions.get(id);
    if (run !== undefined) {
        run(value);
    }
});

/// A file the app names, from the folder its main file is in.
function resolved(file) {
    const path = require('path');
    return path.resolve(path.dirname(host.appFile()), String(file));
}

/// Listeners for something the system tells about, asked for from it the first time one's added. What `on` gives back stops one.
function topic(start) {
    const listeners = new Set();
    let started = false;
    const tell = (value) => {
        for (const listener of [...listeners]) {
            listener(value);
        }
    };
    return {
        on(run) {
            if (typeof run !== 'function') {
                throw new TypeError('a listener has to be a function');
            }
            if (!started) {
                started = true;
                start(tell);
            }
            listeners.add(run);
            return () => listeners.delete(run);
        },
        tell,
        get heard() {
            return listeners.size > 0;
        },
    };
}

/// A picture for the system: a file the app names, or its bytes, as hex. Nothing is ''.
function pictureBytes(icon) {
    if (icon === undefined || icon === null) {
        return '';
    }
    if (typeof icon === 'string') {
        return require('fs').readFileSync(resolved(icon)).toString('hex');
    }
    return Buffer.from(icon).toString('hex');
}

/// A menu as it goes to the system, with what each item does as a number, and those numbers listed in `used`.
function menuSpec(items, used) {
    return (items ?? []).map((item) => {
        const inner = item.items;
        const id = action(item.onClick);
        if (id !== 0) {
            used.push(id);
        }
        return {
            label: String(item.label ?? ''),
            shortcut: item.shortcut ?? null,
            disabled: item.disabled === true,
            checked: item.checked === true,
            separator: item.separator === true,
            items: inner === undefined ? [] : menuSpec(inner, used),
            action: id,
        };
    });
}

module.exports = { host, rsui, action, forget, resolved, topic, pictureBytes, menuSpec };
