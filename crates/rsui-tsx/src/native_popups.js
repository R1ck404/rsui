// `native` from 'rsui': the system's own popups, each one call and a promise of its answer, the same everywhere. They're the
// system's own windows, Android's dialogs, pickers, share sheet and toasts, and on Windows its task dialogs, its share sheet,
// and a picker around its own calendar or time control; see `rsui::popups`. This is only loaded the first time `native`
// is used. While an app's being checked none comes up: what's asked is said, and the `dialog` step says what the next is
// answered with.

const { host, action, forget } = require('rsui/desktop/core');

/// Asks Rust for the popup, and keeps the promise with what it says: `=` and the answer, `-` for none, or `!` and why it
/// couldn't be asked.
function ask(kind, options) {
    return new Promise((resolve, reject) => {
        const id = action((said) => {
            forget([id]);
            const text = String(said ?? '-');
            if (text.startsWith('!')) {
                reject(new Error(text.slice(1)));
            } else {
                resolve(text.startsWith('=') ? text.slice(1) : null);
            }
        });
        try {
            host.popupAsk(kind, JSON.stringify(options), id);
        } catch (error) {
            forget([id]);
            reject(error);
        }
    });
}

const two = (number) => String(number).padStart(2, '0');

/// A Date as the day it is where this is, as `2026-11-21`.
function dayOf(date, name) {
    if (date === undefined || date === null) {
        return undefined;
    }
    if (!(date instanceof Date) || Number.isNaN(date.getTime())) {
        throw new TypeError(`native.pickDate's ${name} has to be a Date`);
    }
    return `${String(date.getFullYear()).padStart(4, '0')}-${two(date.getMonth() + 1)}-${two(date.getDate())}`;
}

const text = (value) => (value === undefined || value === null ? undefined : String(value));

let toastFailed = false;

const native = {
    /// Asks whether to go ahead: true if the button that goes ahead was pressed, and false otherwise.
    async confirm({ title = '', message = '', confirm, cancel, destructive = false } = {}) {
        const said = await ask('confirm', { title: String(title), message: String(message), confirm: text(confirm), cancel: text(cancel), destructive: destructive === true });
        return said === 'true';
    },
    /// Says something, and is done once it's been seen.
    async alert({ title = '', message = '', ok } = {}) {
        await ask('alert', { title: String(title), message: String(message), ok: text(ok) });
    },
    /// Asks for one of these: its place in the list, or null if none was chosen.
    async sheet({ title = '', options, cancel } = {}) {
        if (!Array.isArray(options) || options.length === 0) {
            throw new TypeError('native.sheet needs options to choose from');
        }
        const said = await ask('sheet', { title: String(title), options: options.map(String), cancel: text(cancel) });
        return said === null ? null : Number(said);
    },
    /// Asks for a day: a Date on it, at the time of day `value` had, or midnight, or null if none was picked.
    async pickDate({ value, min, max } = {}) {
        const said = await ask('date', { value: dayOf(value, 'value'), min: dayOf(min, 'min'), max: dayOf(max, 'max') });
        if (said === null) {
            return null;
        }
        const [year, month, day] = said.split('-').map(Number);
        const picked = value instanceof Date ? new Date(value.getTime()) : new Date(year, month - 1, day);
        picked.setFullYear(year, month - 1, day);
        return picked;
    },
    /// Asks for a time of day, on a 24-hour clock, or null if none was picked.
    async pickTime({ value } = {}) {
        let start;
        if (value !== undefined && value !== null) {
            const hours = Math.trunc(Number(value.hours));
            const minutes = Math.trunc(Number(value.minutes ?? 0));
            if (!(hours >= 0 && hours < 24 && minutes >= 0 && minutes < 60)) {
                throw new TypeError('native.pickTime\'s value has to have hours from 0 to 23 and minutes from 0 to 59');
            }
            start = `${two(hours)}:${two(minutes)}`;
        }
        const said = await ask('time', { value: start });
        if (said === null) {
            return null;
        }
        const [hours, minutes] = said.split(':').map(Number);
        return { hours, minutes };
    },
    /// Brings up the system's share sheet with this text and address. It's done once that's up: where it was shared to,
    /// if it was, isn't said.
    async share({ text: said, url, title } = {}) {
        if (said === undefined && url === undefined) {
            throw new TypeError('native.share needs text or a url to share');
        }
        await ask('share', { title: String(title ?? ''), text: String(said ?? ''), url: String(url ?? '') });
    },
    /// Shows a little message that goes away by itself, for longer with `{ long: true }`.
    toast(message, { long = false } = {}) {
        try {
            host.popupToast(String(message), long === true);
        } catch (error) {
            // Nothing's held up for a toast that can't be shown; it's said once.
            if (!toastFailed) {
                toastFailed = true;
                console.warn(`native.toast: ${error.message}`);
            }
        }
    },
};

module.exports = { native };
