'use strict';
// `URL` and `URLSearchParams`. None of this is compiled until the first time something looks for one of them.

const SPECIAL = { 'http:': 80, 'https:': 443, 'ftp:': 21, 'ws:': 80, 'wss:': 443, 'file:': null };

const encoder = new TextEncoder();

function percentEncode(text, keep) {
    let out = '';
    for (const character of text) {
        if (keep(character)) {
            out += character;
        } else {
            for (const byte of encoder.encode(character)) {
                out += '%' + byte.toString(16).toUpperCase().padStart(2, '0');
            }
        }
    }
    return out;
}

// What each part of a URL leaves as it is. A `%` is always left, as it's already been encoded.
const isC0 = (c) => c >= '\x21' && c <= '\x7e';
const inFragment = (c) => isC0(c) && !' "<>`'.includes(c);
const inQuery = (c) => isC0(c) && !' "#<>\''.includes(c);
const inPath = (c) => isC0(c) && !' "#<>?`{}^'.includes(c);
const inUser = (c) => inPath(c) && !'/:;=@[\\]|'.includes(c);

function removeDots(segments, special) {
    const out = [];
    segments.forEach((segment, index) => {
        const lower = segment.toLowerCase();
        const last = index === segments.length - 1;
        if (segment === '..' || lower === '.%2e' || lower === '%2e.' || lower === '%2e%2e') {
            out.pop();
            if (last) out.push('');
        } else if (segment === '.' || lower === '%2e') {
            if (last) out.push('');
        } else {
            out.push(segment);
        }
    });
    return out;
}

// A domain as a URL has it: in lower case, with the other kinds of full stop as dots, and each label that isn't ASCII
// written in Punycode, which is what UTS #46 does to most names, without the tables for the rest. It's '' for a name
// that can't be one. `toUnicode` goes the other way.
let punycode = null;
const forbiddenInDomain = /[\x00-\x20#%/:<>?@[\\\]^|\x7f]/;

function domainLabels(domain) {
    return String(domain)
        .replace(/[­͏᠋-᠍᠏​⁠︀-️﻿]/g, '')
        .replace(/[。．｡]/g, '.')
        .toLowerCase()
        .split('.');
}

// A label in Punycode only counts if it's the one way of writing something that isn't all ASCII.
function decodeLabel(label) {
    const encoded = label.slice(4);
    if (!/^[a-z0-9-]+$/.test(encoded)) return null;
    punycode ??= require('punycode');
    let decoded;
    try {
        decoded = punycode.decode(encoded);
    } catch (error) {
        return null;
    }
    if (!/[^\x00-\x7f]/.test(decoded) || punycode.encode(decoded) !== encoded) return null;
    return decoded;
}

function toASCII(domain) {
    const labels = domainLabels(domain);
    for (let at = 0; at < labels.length; at++) {
        const label = labels[at];
        if (label.startsWith('xn--')) {
            if (decodeLabel(label) === null) return '';
        } else if (/[^\x00-\x7f]/.test(label)) {
            punycode ??= require('punycode');
            labels[at] = 'xn--' + punycode.encode(label.normalize('NFC'));
        }
    }
    const out = labels.join('.');
    return forbiddenInDomain.test(out) ? '' : out;
}

function toUnicode(domain) {
    const labels = domainLabels(domain);
    for (let at = 0; at < labels.length; at++) {
        if (labels[at].startsWith('xn--')) {
            const decoded = decodeLabel(labels[at]);
            if (decoded === null) return '';
            labels[at] = decoded;
        }
    }
    const out = labels.join('.');
    return forbiddenInDomain.test(out.replace(/[^\x00-\x7f]/g, '')) ? '' : out;
}

function parseHost(host, special) {
    if (!special) return host;
    let decoded;
    try {
        decoded = decodeURIComponent(host);
    } catch (error) {
        return null;
    }
    const lower = decoded.toLowerCase();
    if (lower === '' || /[\x00-\x20#%/:<>?@[\\\]^|]/.test(lower.replace(/^\[.*\]$/, ''))) {
        return null;
    }
    // Numbers that make an address are written the one way there is.
    if (/^\d+(\.\d+){3}$/.test(lower)) {
        const parts = lower.split('.').map(Number);
        if (parts.some((part) => part > 255)) return null;
        return parts.join('.');
    }
    // A name in another alphabet is written in letters a name can have, as every URL has it.
    if (/[^\x00-\x7f]/.test(lower) || lower.includes('xn--')) {
        const ascii = toASCII(lower);
        return ascii === '' ? null : ascii;
    }
    return lower;
}

function parseUrl(input, base) {
    let text = String(input).replace(/^[\x00-\x20]+|[\x00-\x20]+$/g, '').replace(/[\t\n\r]/g, '');
    const parts = { protocol: '', username: '', password: '', host: null, port: '', path: [], opaque: null, search: '', hash: '' };
    const hashAt = text.indexOf('#');
    let hash = null;
    if (hashAt >= 0) {
        hash = text.slice(hashAt + 1);
        text = text.slice(0, hashAt);
    }
    const queryAt = text.indexOf('?');
    let query = null;
    if (queryAt >= 0) {
        query = text.slice(queryAt + 1);
        text = text.slice(0, queryAt);
    }
    const scheme = /^([a-zA-Z][a-zA-Z0-9+.-]*):/.exec(text);
    let rest;
    if (scheme) {
        parts.protocol = scheme[1].toLowerCase() + ':';
        rest = text.slice(scheme[0].length);
    } else {
        if (base === undefined) return null;
        parts.protocol = base.protocol;
        rest = text;
    }
    const special = parts.protocol in SPECIAL;
    if (special) rest = rest.replace(/\\/g, '/');
    if (!scheme) {
        const from = base.__parts();
        if (from.opaque !== null) return null;
        if (rest.startsWith('//')) {
            // Only the scheme comes from the base.
        } else {
            parts.username = from.username;
            parts.password = from.password;
            parts.host = from.host;
            parts.port = from.port;
            if (rest === '') {
                parts.path = from.path;
                if (query === null) parts.search = from.search;
            } else if (rest.startsWith('/')) {
                parts.path = removeDots(rest.slice(1).split('/').map((part) => percentEncode(part, inPath)), special);
            } else {
                const dir = from.path.slice(0, -1);
                parts.path = removeDots(dir.concat(rest.split('/').map((part) => percentEncode(part, inPath))), special);
            }
            finish(parts, query, hash);
            return parts;
        }
    }
    if (special && scheme && base !== undefined && !rest.startsWith('/') && parts.protocol === base.protocol) {
        return parseUrl(rest + (query === null ? '' : '?' + query) + (hash === null ? '' : '#' + hash), base);
    }
    if (special || rest.startsWith('//')) {
        if (special) {
            rest = rest.replace(/^\/*/, '');
            if (parts.protocol === 'file:') {
                // A path with no host, as on most file URLs, starts with a slash, or a drive letter on Windows.
                const original = text.slice(scheme ? scheme[0].length : 0).replace(/\\/g, '/');
                const afterSlashes = original.startsWith('//') ? original.slice(2) : original;
                const slash = afterSlashes.indexOf('/');
                const possibleHost = original.startsWith('//') ? (slash < 0 ? afterSlashes : afterSlashes.slice(0, slash)) : '';
                if (possibleHost === '' || /^[a-zA-Z]:$/.test(possibleHost)) {
                    parts.host = '';
                    const pathText = possibleHost === '' ? afterSlashes.replace(/^\/*/, '') : afterSlashes;
                    parts.path = removeDots(pathText.split('/').map((part) => percentEncode(part, inPath)), true);
                    finish(parts, query, hash);
                    return parts;
                }
            }
        } else {
            rest = rest.slice(2);
        }
        const slash = rest.indexOf('/');
        const authority = slash < 0 ? rest : rest.slice(0, slash);
        const pathText = slash < 0 ? '' : rest.slice(slash + 1);
        const at = authority.lastIndexOf('@');
        let hostPort = authority;
        if (at >= 0) {
            const credentials = authority.slice(0, at);
            hostPort = authority.slice(at + 1);
            const colon = credentials.indexOf(':');
            parts.username = percentEncode(colon < 0 ? credentials : credentials.slice(0, colon), inUser);
            parts.password = colon < 0 ? '' : percentEncode(credentials.slice(colon + 1), inUser);
        }
        const portMatch = /:(\d*)$/.exec(hostPort);
        let host = hostPort;
        if (portMatch && !hostPort.endsWith(']')) {
            host = hostPort.slice(0, portMatch.index);
            if (portMatch[1] !== '') {
                const port = Number(portMatch[1]);
                if (port > 65535) return null;
                parts.port = port === SPECIAL[parts.protocol] ? '' : String(port);
            }
        } else if (/:[^\]]*$/.test(hostPort) && !hostPort.endsWith(']')) {
            return null;
        }
        const parsed = parseHost(host, special);
        if (parsed === null) return null;
        // `file://localhost/x` is the file `/x` on this computer.
        parts.host = parts.protocol === 'file:' && parsed === 'localhost' ? '' : parsed;
        const segments = pathText === '' && !special && slash < 0 ? [] : pathText.split('/');
        parts.path = removeDots(segments.map((part) => percentEncode(part, inPath)), special);
    } else if (rest.startsWith('/')) {
        parts.path = removeDots(rest.slice(1).split('/').map((part) => percentEncode(part, inPath)), false);
    } else {
        parts.opaque = percentEncode(rest, (c) => c >= '\x20' && c <= '\x7e');
    }
    finish(parts, query, hash);
    return parts;
}

// An empty `?` or `#` stays in `href`, though `search` and `hash` say there's nothing there.
function finish(parts, query, hash) {
    if (query !== null) parts.search = '?' + percentEncode(query, inQuery);
    if (hash !== null) parts.hash = '#' + percentEncode(hash, inFragment);
}

function formEncode(text) {
    return percentEncode(String(text), (c) => /[A-Za-z0-9*\-._]/.test(c)).replace(/%20/g, '+');
}

function formDecode(text) {
    const spaced = text.replace(/\+/g, ' ');
    const bytes = [];
    for (let at = 0; at < spaced.length; at++) {
        const code = spaced.charCodeAt(at);
        if (code === 0x25 && /^[0-9a-fA-F]{2}$/.test(spaced.slice(at + 1, at + 3))) {
            bytes.push(parseInt(spaced.slice(at + 1, at + 3), 16));
            at += 2;
        } else {
            bytes.push(...encoder.encode(spaced[at]));
        }
    }
    return new TextDecoder().decode(new Uint8Array(bytes));
}

class URLSearchParams {
    #items = [];
    #owner = null;
    constructor(from = '') {
        if (from !== null && typeof from === 'object' && typeof from[Symbol.iterator] === 'function') {
            for (const pair of from) {
                const [name, value, ...more] = pair;
                if (value === undefined || more.length > 0) {
                    throw new TypeError('Each pair must have a name and a value');
                }
                this.#items.push([String(name), String(value)]);
            }
        } else if (from !== null && typeof from === 'object') {
            for (const name of Object.keys(from)) {
                this.#items.push([name, String(from[name])]);
            }
        } else {
            this.__read(String(from));
        }
    }
    __read(text) {
        this.#items = [];
        const clean = text.startsWith('?') ? text.slice(1) : text;
        for (const piece of clean.split('&')) {
            if (piece === '') continue;
            const at = piece.indexOf('=');
            const name = at < 0 ? piece : piece.slice(0, at);
            const value = at < 0 ? '' : piece.slice(at + 1);
            this.#items.push([formDecode(name), formDecode(value)]);
        }
    }
    __own(owner) {
        this.#owner = owner;
    }
    #changed() {
        if (this.#owner !== null) this.#owner.__paramsChanged(this.toString());
    }
    get size() {
        return this.#items.length;
    }
    append(name, value) {
        this.#items.push([String(name), String(value)]);
        this.#changed();
    }
    delete(name, value) {
        const wanted = String(name);
        this.#items = this.#items.filter(([found, foundValue]) => found !== wanted || (value !== undefined && foundValue !== String(value)));
        this.#changed();
    }
    get(name) {
        const found = this.#items.find(([item]) => item === String(name));
        return found === undefined ? null : found[1];
    }
    getAll(name) {
        return this.#items.filter(([item]) => item === String(name)).map(([, value]) => value);
    }
    has(name, value) {
        return this.#items.some(([item, itemValue]) => item === String(name) && (value === undefined || itemValue === String(value)));
    }
    set(name, value) {
        const wanted = String(name);
        const first = this.#items.findIndex(([item]) => item === wanted);
        if (first < 0) {
            this.#items.push([wanted, String(value)]);
        } else {
            this.#items[first][1] = String(value);
            this.#items = this.#items.filter(([item], index) => item !== wanted || index === first);
        }
        this.#changed();
    }
    sort() {
        this.#items = this.#items
            .map((item, index) => [item, index])
            .sort(([a, aAt], [b, bAt]) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : aAt - bAt))
            .map(([item]) => item);
        this.#changed();
    }
    forEach(run, self) {
        for (const [name, value] of this.#items) run.call(self, value, name, this);
    }
    *entries() {
        for (const [name, value] of this.#items) yield [name, value];
    }
    *keys() {
        for (const [name] of this.#items) yield name;
    }
    *values() {
        for (const [, value] of this.#items) yield value;
    }
    [Symbol.iterator]() {
        return this.entries();
    }
    toString() {
        return this.#items.map(([name, value]) => formEncode(name) + '=' + formEncode(value)).join('&');
    }
}

class URL {
    #parts;
    #params = null;
    constructor(input, base) {
        let from;
        if (base !== undefined) {
            from = base instanceof URL ? base : new URL(base);
        }
        const parts = parseUrl(input, from);
        if (parts === null) {
            const error = new TypeError('Invalid URL');
            error.code = 'ERR_INVALID_URL';
            error.input = String(input);
            throw error;
        }
        this.#parts = parts;
    }
    static canParse(input, base) {
        try {
            new URL(input, base);
            return true;
        } catch (error) {
            return false;
        }
    }
    // What `new URL` makes, or null where it would throw.
    static parse(input, base) {
        try {
            return new URL(input, base);
        } catch (error) {
            return null;
        }
    }
    __parts() {
        return this.#parts;
    }
    __paramsChanged(text) {
        this.#parts.search = text === '' ? '' : '?' + text;
    }
    get protocol() {
        return this.#parts.protocol;
    }
    set protocol(value) {
        const clean = String(value).replace(/:.*$/, '').toLowerCase() + ':';
        if (clean in SPECIAL === this.#parts.protocol in SPECIAL) this.#parts.protocol = clean;
    }
    get username() {
        return this.#parts.username;
    }
    set username(value) {
        this.#parts.username = percentEncode(String(value), inUser);
    }
    get password() {
        return this.#parts.password;
    }
    set password(value) {
        this.#parts.password = percentEncode(String(value), inUser);
    }
    get hostname() {
        return this.#parts.host ?? '';
    }
    set hostname(value) {
        const parsed = parseHost(String(value), this.#parts.protocol in SPECIAL);
        if (parsed !== null) this.#parts.host = parsed;
    }
    get port() {
        return this.#parts.port;
    }
    set port(value) {
        const text = String(value);
        if (text === '') {
            this.#parts.port = '';
        } else if (/^\d+/.test(text)) {
            const port = parseInt(text, 10);
            if (port <= 65535) this.#parts.port = port === SPECIAL[this.#parts.protocol] ? '' : String(port);
        }
    }
    get host() {
        return this.hostname + (this.#parts.port === '' ? '' : ':' + this.#parts.port);
    }
    set host(value) {
        const match = /^([^:]*)(?::(\d+))?/.exec(String(value));
        if (match) {
            this.hostname = match[1];
            if (match[2] !== undefined) this.port = match[2];
        }
    }
    get origin() {
        const { protocol } = this.#parts;
        return protocol in SPECIAL && protocol !== 'file:' ? protocol + '//' + this.host : 'null';
    }
    get pathname() {
        const { opaque, path } = this.#parts;
        if (opaque !== null) return opaque;
        return path.length === 0 && !(this.#parts.protocol in SPECIAL) ? '' : '/' + path.join('/');
    }
    set pathname(value) {
        if (this.#parts.opaque === null) {
            const clean = String(value).replace(/^\//, '');
            this.#parts.path = removeDots(clean.split('/').map((part) => percentEncode(part, inPath)));
        }
    }
    get search() {
        return this.#parts.search === '?' ? '' : this.#parts.search;
    }
    set search(value) {
        const text = String(value).replace(/^\?/, '');
        this.#parts.search = text === '' ? '' : '?' + percentEncode(text, inQuery);
        if (this.#params !== null) this.#params.__read(this.#parts.search);
    }
    get searchParams() {
        if (this.#params === null) {
            this.#params = new URLSearchParams(this.#parts.search);
            this.#params.__own(this);
        }
        return this.#params;
    }
    get hash() {
        return this.#parts.hash === '#' ? '' : this.#parts.hash;
    }
    set hash(value) {
        const text = String(value).replace(/^#/, '');
        this.#parts.hash = text === '' ? '' : '#' + percentEncode(text, inFragment);
    }
    get href() {
        const parts = this.#parts;
        let out = parts.protocol;
        if (parts.host !== null) {
            out += '//';
            if (parts.username !== '' || parts.password !== '') {
                out += parts.username;
                if (parts.password !== '') out += ':' + parts.password;
                out += '@';
            }
            out += this.host;
        }
        return out + this.pathname + parts.search + parts.hash;
    }
    set href(value) {
        const parts = parseUrl(value);
        if (parts === null) throw new TypeError('Invalid URL');
        this.#parts = parts;
        if (this.#params !== null) this.#params.__read(parts.search);
    }
    toString() {
        return this.href;
    }
    toJSON() {
        return this.href;
    }
}

module.exports = { URL, URLSearchParams, toASCII, toUnicode };
