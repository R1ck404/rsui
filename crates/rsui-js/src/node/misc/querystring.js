'use strict';
// Node's `querystring`, as `node/misc.js` makes it, the first time it's asked for.
module.exports = () => {
    const unescape = (text) => {
        try {
            return decodeURIComponent(text.replace(/\+/g, ' '));
        } catch {
            return text;
        }
    };
    const value = (item) => (typeof item === 'string' ? item : typeof item === 'number' && Number.isFinite(item) ? String(item) : typeof item === 'bigint' || typeof item === 'boolean' ? String(item) : '');
    const stringify = (object, separator = '&', equals = '=') => {
        if (object === null || typeof object !== 'object') {
            return '';
        }
        const out = [];
        for (const name of Object.keys(object)) {
            const items = Array.isArray(object[name]) ? object[name] : [object[name]];
            for (const item of items) {
                out.push(encodeURIComponent(name) + equals + encodeURIComponent(value(item)));
            }
        }
        return out.join(separator);
    };
    const parse = (text, separator = '&', equals = '=') => {
        const out = Object.create(null);
        if (typeof text !== 'string' || text === '') {
            return out;
        }
        for (const part of text.split(separator)) {
            if (part === '') {
                continue;
            }
            const at = part.indexOf(equals);
            const name = unescape(at < 0 ? part : part.slice(0, at));
            const item = at < 0 ? '' : unescape(part.slice(at + equals.length));
            if (name in out) {
                out[name] = Array.isArray(out[name]) ? [...out[name], item] : [out[name], item];
            } else {
                out[name] = item;
            }
        }
        return out;
    };
    return { stringify, parse, encode: stringify, decode: parse, escape: encodeURIComponent, unescape };
};
